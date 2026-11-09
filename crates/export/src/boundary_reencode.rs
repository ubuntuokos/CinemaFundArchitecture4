//! Boundary re-encode executor (WP-06 of `CFA4-SMART-CUT-RENDER-R1`, *határ-újrakódolás*): runs a
//! frame-accurate Smart Cut plan of one H.264 MP4 / MOV source. The whole GOPs inside the range
//! are copied packet for packet, as [`crate::smart_cut`] does; only the pictures between a cut and
//! the nearest clean access point are decoded and re-encoded with the CFA4 H.264 encoder.
//!
//! - **One stream, one sample entry:** the re-encoded pictures use parameter-set ids the source
//!   does not, so the output `avcC` carries the source SPS/PPS unchanged followed by the
//!   encoder's. Every re-encoded segment starts with an IDR, and a copied segment that follows one
//!   must start with an IDR too, so decoding never reaches across a splice. Neighbouring IDRs get
//!   different `idr_pic_id`s.
//! - **Compatibility first (SCE-001 §8):** the splice is made only when the encoder can match the
//!   source stream: Baseline, Main or High profile, 8-bit 4:2:0, progressive frames, 4-byte NAL
//!   lengths, an even picture size and free parameter-set ids. Otherwise the plan is refused
//!   ([`SmartCutError::Incompatible`]) before anything is written, never spliced unsafely.
//! - **Timing:** the output keeps the source durations. Re-encoded pictures have no reordering;
//!   they get the composition delay of the copied run, so the presentation timeline has no gap at
//!   a splice, and an edit list starts the presentation at the first picture.
//! - **Validation:** [`verify_mp4_boundary_reencode`] reopens the output, checks that the copied
//!   samples are the source samples, that the presentation timeline is the planned one, decodes
//!   the whole output with the CFA4 H.264 decoder and compares every re-encoded picture with the
//!   source picture (luma PSNR).
//!
//! Rendered segments (gaps, effects; the timeline Smart Render) are refused here
//! ([`SmartCutError::RenderSegment`], [`SmartCutError::TimelinePlan`]).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Seek, Write};

use cfa4_bitstream::{length_prefixed_nals, unescape_rbsp};
use cfa4_h264::params::{Pps, Sps};
use cfa4_h264::slice::{NalHeader, SliceHeader};
use cfa4_h264enc::{ColorConfig, Encoder, EncoderConfig, PacketFormat, Profile, RateControl, YuvFrame};
use cfa4_isobmff::{AvcConfig, ByteSource, CodecConfig, Mp4Writer, SampleEntry, Track, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::Codec;
use cfa4_smart_edit::media_index::StreamIndex;
use cfa4_smart_edit::plan::{ExportExecutionPlan, PLAN_VERSION, SegmentMode};
use serde::{Deserialize, Serialize};

use crate::smart_cut::{Mp4CopySource, SmartCutError, brand, io};

const NAL_IDR: u8 = 5;
const NAL_SLICE: u8 = 1;

/// How the boundary pictures are encoded and checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryEncoding {
    /// Constant rate factor of the re-encoded pictures (lower is better; 16 is visually lossless
    /// for most material).
    pub crf: f32,
    /// Encoder worker threads (0 = all cores).
    pub threads: usize,
    /// Lowest luma PSNR (dB) a re-encoded picture may have against its source picture when the
    /// output is verified.
    pub min_psnr: f64,
}

impl Default for BoundaryEncoding {
    fn default() -> Self {
        BoundaryEncoding { crf: 16.0, threads: 0, min_psnr: 30.0 }
    }
}

/// What a boundary re-encode wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryReport {
    /// Samples (pictures) written.
    pub samples: u64,
    /// Of those, copied unchanged from the source.
    pub copied: u64,
    /// Of those, decoded and re-encoded.
    pub reencoded: u64,
    /// Payload bytes written (sample data only).
    pub payload_bytes: u64,
    /// Output duration in the track timescale.
    pub duration: u64,
    pub timescale: u32,
    /// Lowest luma PSNR of a re-encoded picture (dB; `None` when nothing was re-encoded or the
    /// output was not verified). Identical pictures count as 99 dB.
    pub min_psnr: Option<f64>,
}

/// One planned segment, resolved against the source track.
enum Seg {
    /// Source samples `first..=last` (decode order), copied.
    Copy { first: usize, last: usize },
    /// Source samples `decode_first..=decode_last` decoded; `pictures` (decode indices, in
    /// presentation order) re-encoded.
    Reencode { decode_first: usize, decode_last: usize, pictures: Vec<usize> },
}

/// The source parameter sets, parsed.
struct SourceParams {
    spss: Vec<Option<Sps>>,
    ppss: Vec<Option<Pps>>,
}

struct Prepared<'a> {
    track: &'a Track,
    entry: &'a SampleEntry,
    avc: &'a AvcConfig,
    segs: Vec<Seg>,
    /// Composition delay (PTS − DTS) of the re-encoded pictures: the delay of the copied run.
    delay: i64,
    /// Presentation duration of each source picture, by decode index.
    durations: Vec<u32>,
}

fn incompatible(detail: impl Into<String>) -> SmartCutError {
    SmartCutError::Incompatible { detail: detail.into() }
}

fn prepare<'a, S: ByteSource + ?Sized>(plan: &ExportExecutionPlan, index: &StreamIndex, source: &Mp4CopySource<'a, S>) -> Result<Prepared<'a>, SmartCutError> {
    if plan.plan_version != PLAN_VERSION {
        return Err(SmartCutError::PlanVersion { found: plan.plan_version });
    }
    if plan.segments.iter().any(|s| s.source.is_some()) {
        return Err(SmartCutError::TimelinePlan);
    }
    let unsupported = SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container };
    if plan.codec != Codec::H264 || brand(plan.source_container).is_none() || brand(plan.output_container).is_none() || index.codec != Codec::H264 {
        return Err(unsupported);
    }
    let track = source.file.tracks.get(source.track).ok_or(SmartCutError::NoTrack { track: source.track })?;
    if index.stream_id != track.id {
        return Err(SmartCutError::WrongStream { expected: track.id, found: index.stream_id });
    }
    let n = track.samples.len();
    if index.frames.len() != n {
        return Err(SmartCutError::StaleIndex { detail: format!("{} indexed pictures, {n} samples in track {}", index.frames.len(), track.id) });
    }
    for (d, (sample, frame)) in track.samples.iter().zip(&index.frames).enumerate() {
        if sample.size != frame.size {
            return Err(SmartCutError::StaleIndex { detail: format!("packet {d} is {} bytes, the index says {}", sample.size, frame.size) });
        }
    }
    // presentation order of the source pictures, and how long each one is shown
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&d| (track.samples.get(d).map_or(i64::MAX, |s| s.pts), d));
    let mut position = vec![0u64; n];
    let mut durations = vec![0u32; n];
    for (p, &d) in order.iter().enumerate() {
        if let Some(slot) = position.get_mut(d) {
            *slot = p as u64;
        }
        if let Some(Some(expected)) = index.frames.get(d).map(|f| f.presentation_index)
            && expected != p as u64
        {
            return Err(SmartCutError::StaleIndex { detail: format!("packet {d} is shown at {p}, the index says {expected}") });
        }
        let this = track.samples.get(d).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
        let shown = match order.get(p + 1).and_then(|&next| track.samples.get(next)) {
            Some(next) => next.pts.checked_sub(this.pts).and_then(|x| u32::try_from(x).ok()),
            None => Some(this.duration),
        };
        match (shown, durations.get_mut(d)) {
            (Some(x), Some(slot)) if x > 0 => *slot = x,
            _ => return Err(SmartCutError::Timestamps { decode_index: d as u64 }),
        }
    }
    let mut segs = Vec::with_capacity(plan.segments.len());
    let mut used = BTreeSet::new();
    let mut next_position: Option<u64> = None;
    for (i, s) in plan.segments.iter().enumerate() {
        if s.stream_id != track.id {
            return Err(SmartCutError::WrongStream { expected: track.id, found: s.stream_id });
        }
        if s.decode.last < s.decode.first || s.presentation.last < s.presentation.first || next_position.is_some_and(|p| p != s.presentation.first) {
            return Err(SmartCutError::NotContiguous { segment: i });
        }
        next_position = s.presentation.last.checked_add(1);
        let as_index = |d: u64| usize::try_from(d).ok().filter(|&d| d < n);
        let (Some(first), Some(last)) = (as_index(s.decode.first), as_index(s.decode.last)) else {
            return Err(SmartCutError::StaleIndex { detail: format!("packets {}–{} are past the end of track {}", s.decode.first, s.decode.last, track.id) });
        };
        used.extend(first..=last);
        match s.mode {
            SegmentMode::StrictCopy | SegmentMode::CompatibleCopy => {
                // a copied run shows exactly the pictures it decodes
                let shown = (first..=last).all(|d| position.get(d).is_some_and(|p| (s.presentation.first..=s.presentation.last).contains(p)));
                if !shown || s.decode.count() != s.presentation.count() {
                    return Err(SmartCutError::NotContiguous { segment: i });
                }
                segs.push(Seg::Copy { first, last });
            }
            SegmentMode::BoundaryReencode => {
                let mut pictures = Vec::new();
                for p in s.presentation.first..=s.presentation.last {
                    let d = usize::try_from(p).ok().and_then(|p| order.get(p)).copied();
                    match d {
                        Some(d) if (first..=last).contains(&d) => pictures.push(d),
                        _ => return Err(SmartCutError::StaleIndex { detail: format!("picture {p} of segment {i} is not decoded by it") }),
                    }
                }
                if !track.samples.get(first).is_some_and(|x| x.is_sync) {
                    return Err(SmartCutError::StaleIndex { detail: format!("segment {i} does not decode from an access point") });
                }
                segs.push(Seg::Reencode { decode_first: first, decode_last: last, pictures });
            }
            SegmentMode::Render => return Err(SmartCutError::RenderSegment { segment: i }),
        }
    }
    if segs.is_empty() {
        return Err(SmartCutError::NotLossless);
    }
    // one sample entry for everything read
    let mut description = None;
    for &d in &used {
        let x = track.samples.get(d).map(|s| s.description_index);
        match description {
            None => description = x,
            Some(_) if description == x => {}
            Some(_) => return Err(SmartCutError::MixedSampleEntries),
        }
    }
    let entry = description
        .and_then(|x| x.checked_sub(1))
        .and_then(|x| track.entries.get(usize::try_from(x).ok()?))
        .ok_or_else(|| SmartCutError::StaleIndex { detail: format!("track {} has no sample entry for the planned packets", track.id) })?;
    let CodecConfig::Avc(avc) = &entry.codec else { return Err(unsupported) };
    // the copied runs set the composition delay; they must agree on it
    let mut delay = None;
    for seg in &segs {
        if let Seg::Copy { first, last } = *seg {
            let run = track.samples.get(first..=last).unwrap_or_default();
            let first_dts = run.first().map_or(0, |s| s.dts);
            let min_pts = run.iter().map(|s| s.pts).min().unwrap_or(first_dts);
            let d = min_pts.checked_sub(first_dts).filter(|d| *d >= 0).ok_or(SmartCutError::Timestamps { decode_index: first as u64 })?;
            if delay.is_some_and(|x| x != d) {
                return Err(SmartCutError::Timestamps { decode_index: first as u64 });
            }
            delay = Some(d);
        }
    }
    Ok(Prepared { track, entry, avc, segs, delay: delay.unwrap_or(0), durations })
}

fn parse_params(avc: &AvcConfig) -> Result<SourceParams, SmartCutError> {
    let mut spss: Vec<Option<Sps>> = vec![None; 32];
    let mut ppss: Vec<Option<Pps>> = vec![None; 256];
    for nal in &avc.sps {
        let sps = Sps::parse(&unescape_rbsp(nal.get(1..).unwrap_or_default())).map_err(|e| incompatible(format!("source SPS: {e}")))?;
        let id = sps.id as usize;
        *spss.get_mut(id).ok_or_else(|| incompatible("source SPS id out of range"))? = Some(sps);
    }
    for nal in &avc.pps {
        let pps = Pps::parse(&unescape_rbsp(nal.get(1..).unwrap_or_default()), &spss).map_err(|e| incompatible(format!("source PPS: {e}")))?;
        let id = pps.id as usize;
        *ppss.get_mut(id).ok_or_else(|| incompatible("source PPS id out of range"))? = Some(pps);
    }
    Ok(SourceParams { spss, ppss })
}

/// The NAL units of a length-prefixed sample.
fn nals(data: &[u8], length_size: u8) -> Result<Vec<&[u8]>, SmartCutError> {
    length_prefixed_nals(data, usize::from(length_size)).map_err(|e| SmartCutError::Decode { detail: e.to_string() })
}

/// `Some(idr_pic_id)` when the sample is an IDR picture, `None` for any other picture.
fn idr_pic_id(data: &[u8], length_size: u8, params: &SourceParams) -> Result<Option<u32>, SmartCutError> {
    for nal in nals(data, length_size)? {
        let Some(&first) = nal.first() else { continue };
        let kind = first & 0x1f;
        if kind == NAL_SLICE {
            return Ok(None);
        }
        if kind != NAL_IDR {
            continue;
        }
        let header = NalHeader::parse(first).map_err(|e| SmartCutError::Decode { detail: e.to_string() })?;
        let rbsp = unescape_rbsp(nal.get(1..).unwrap_or_default());
        let (sh, _, _) = SliceHeader::parse(&rbsp, header, |id| {
            let pps = params.ppss.get(id as usize).and_then(Option::as_ref).ok_or_else(|| cfa4_h264::Error::MissingParameterSet(format!("PPS {id}")))?;
            let sps = params
                .spss
                .get(pps.sps_id as usize)
                .and_then(Option::as_ref)
                .ok_or_else(|| cfa4_h264::Error::MissingParameterSet(format!("SPS {}", pps.sps_id)))?;
            Ok((pps, sps))
        })
        .map_err(|e| SmartCutError::Decode { detail: e.to_string() })?;
        return Ok(Some(sh.idr_pic_id));
    }
    Ok(None)
}

/// The encoder settings that match the source stream, or why there are none.
fn encoder_config(
    avc: &AvcConfig,
    params: &SourceParams,
    width: u32,
    height: u32,
    rate: (u32, u32),
    encoding: &BoundaryEncoding,
) -> Result<EncoderConfig, SmartCutError> {
    if avc.length_size != 4 {
        return Err(incompatible(format!("{}-byte NAL lengths (the encoder writes 4)", avc.length_size)));
    }
    let used: Vec<&Sps> = params.spss.iter().flatten().collect();
    let Some(sps) = used.first() else { return Err(incompatible("the source has no SPS")) };
    for s in &used {
        if s.chroma_format_idc != 1 || s.bit_depth_luma != 8 || s.bit_depth_chroma != 8 || s.separate_colour_plane {
            return Err(incompatible("only 8-bit 4:2:0 sources can be re-encoded at a boundary"));
        }
        if !s.frame_mbs_only {
            return Err(incompatible("interlaced (field or MBAFF) sources cannot be re-encoded at a boundary yet"));
        }
    }
    let profile = match sps.profile_idc {
        66 => Profile::Baseline,
        77 => Profile::Main,
        100 => Profile::High,
        other => return Err(incompatible(format!("profile_idc {other} has no matching encoder profile"))),
    };
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(incompatible(format!("picture size {width}×{height} is not even")));
    }
    let sps_id = (0u8..32).find(|&i| params.spss.get(usize::from(i)).is_some_and(Option::is_none));
    let pps_id = (0u8..=255).find(|&i| params.ppss.get(usize::from(i)).is_some_and(Option::is_none));
    let (Some(sps_id), Some(pps_id)) = (sps_id, pps_id) else { return Err(incompatible("no free parameter-set id")) };
    let mut cfg = EncoderConfig::new(width, height, rate.0.max(1), rate.1.max(1));
    cfg.profile = profile;
    cfg.level = Some(avc.level);
    cfg.rate = RateControl::Crf(encoding.crf);
    cfg.bframes = 0;
    cfg.keyint = u32::MAX;
    cfg.scenecut = false;
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.aud = false;
    cfg.threads = encoding.threads;
    cfg.sps_id = sps_id;
    cfg.pps_id = pps_id;
    Ok(cfg)
}

/// Decode the source samples of a re-encoded segment and return its pictures, in presentation
/// order.
fn decode_boundary<S: ByteSource + ?Sized>(
    source: &Mp4CopySource<'_, S>,
    avc: &AvcConfig,
    decode_first: usize,
    decode_last: usize,
    pictures: &[usize],
) -> Result<Vec<cfa4_h264::Picture>, SmartCutError> {
    let decode_err = |e: cfa4_h264::Error| SmartCutError::Decode { detail: e.to_string() };
    let mut decoder = cfa4_h264::Decoder::from_avcc(&avc.to_bytes()).map_err(decode_err)?;
    let track = source.file.tracks.get(source.track).ok_or(SmartCutError::NoTrack { track: source.track })?;
    let wanted: BTreeSet<i64> = pictures.iter().filter_map(|&d| track.samples.get(d).map(|s| s.pts)).collect();
    let mut got = BTreeMap::new();
    for d in decode_first..=decode_last {
        let pts = track.samples.get(d).map(|s| s.pts).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
        let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
        for p in decoder.decode(&data, pts).map_err(decode_err)? {
            if wanted.contains(&p.pts) {
                got.insert(p.pts, p);
            }
        }
    }
    for p in decoder.flush() {
        if wanted.contains(&p.pts) {
            got.insert(p.pts, p);
        }
    }
    if let Some(e) = decoder.take_error() {
        return Err(decode_err(e));
    }
    if got.len() != wanted.len() {
        return Err(SmartCutError::Decode { detail: format!("{} of {} boundary pictures decoded", got.len(), wanted.len()) });
    }
    Ok(got.into_values().collect())
}

fn planes(p: &cfa4_h264::Picture) -> Result<YuvFrame<'_>, SmartCutError> {
    let (Some(y), Some(u), Some(v)) = (p.y.as_u8(), p.u.as_u8(), p.v.as_u8()) else {
        return Err(incompatible("the boundary pictures are not 8-bit"));
    };
    if p.chroma_format != 1 {
        return Err(incompatible("the boundary pictures are not 4:2:0"));
    }
    Ok(YuvFrame { y, u, v, y_stride: p.y_stride, uv_stride: p.uv_stride })
}

/// Luma PSNR of two 8-bit pictures of the same size (99 dB when identical).
fn luma_psnr(a: &cfa4_h264::Picture, b: &cfa4_h264::Picture) -> Option<f64> {
    if (a.width, a.height) != (b.width, b.height) {
        return None;
    }
    let (ay, by) = (a.y.as_u8()?, b.y.as_u8()?);
    let (w, h) = (a.width as usize, a.height as usize);
    let mut sse = 0u64;
    for row in 0..h {
        let ra = ay.get(row * a.y_stride..row * a.y_stride + w)?;
        let rb = by.get(row * b.y_stride..row * b.y_stride + w)?;
        sse += ra.iter().zip(rb).map(|(x, y)| u64::from(x.abs_diff(*y)).pow(2)).sum::<u64>();
    }
    if sse == 0 {
        return Some(99.0);
    }
    let mse = sse as f64 / (w * h).max(1) as f64;
    Some(10.0 * (255.0 * 255.0 / mse).log10())
}

struct Setup<'a> {
    p: Prepared<'a>,
    params: SourceParams,
    /// The encoder settings, when something is re-encoded.
    cfg: Option<EncoderConfig>,
}

/// SEI payload types carried through a re-encoded boundary (HDR static metadata) and refused
/// (user data: closed captions and other per-picture data the encoder cannot reproduce).
const SEI_USER_DATA_REGISTERED: u32 = 4;
const SEI_USER_DATA_UNREGISTERED: u32 = 5;
const SEI_MASTERING_DISPLAY: u32 = 137;
const SEI_CONTENT_LIGHT: u32 = 144;

/// The SEI messages (payload type, payload) of a length-prefixed sample.
fn sei_messages(data: &[u8], length_size: u8) -> Result<Vec<(u32, Vec<u8>)>, SmartCutError> {
    let mut out = Vec::new();
    for nal in nals(data, length_size)? {
        if nal.first().map(|b| b & 0x1f) != Some(6) {
            continue;
        }
        let rbsp = unescape_rbsp(nal.get(1..).unwrap_or_default());
        let mut at = 0usize;
        // stop at the rbsp trailing bits
        while rbsp.get(at).is_some_and(|&b| b != 0x80) {
            let mut read = || -> Option<u32> {
                let mut v = 0u32;
                loop {
                    let b = *rbsp.get(at)?;
                    at += 1;
                    v = v.checked_add(u32::from(b))?;
                    if b != 0xff {
                        return Some(v);
                    }
                }
            };
            let (Some(kind), Some(size)) = (read(), read()) else { break };
            let end = at.checked_add(size as usize).filter(|&e| e <= rbsp.len());
            let Some(end) = end else { break };
            out.push((kind, rbsp.get(at..end).unwrap_or_default().to_vec()));
            at = end;
        }
    }
    Ok(out)
}

/// Everything that can be checked without decoding: the plan against the source, the encoder
/// settings that match the source stream, the frame rate of the re-encoded pictures, their SEI
/// metadata and the IDR after every boundary.
fn setup<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'a, S>,
    encoding: &BoundaryEncoding,
) -> Result<Setup<'a>, SmartCutError> {
    let p = prepare(plan, index, source)?;
    let params = parse_params(p.avc)?;
    let mut reencoded = p.segs.iter().filter_map(|s| match s {
        Seg::Reencode { decode_first, decode_last, pictures } => Some((*decode_first, *decode_last, pictures)),
        Seg::Copy { .. } => None,
    });
    let Some(first) = reencoded.next() else { return Ok(Setup { p, params, cfg: None }) };
    // one frame rate: the encoder signals a fixed one in its VUI
    let frame = first.2.first().and_then(|&d| p.durations.get(d)).copied().unwrap_or(1);
    for (_, _, pictures) in std::iter::once(first).chain(reencoded) {
        if pictures.iter().any(|&d| p.durations.get(d) != Some(&frame)) {
            return Err(incompatible("the re-encoded pictures have a variable frame rate"));
        }
    }
    let video = p.entry.video.as_ref();
    let (width, height) = video.map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let mut cfg = encoder_config(p.avc, &params, width, height, (p.track.timescale, frame), encoding)?;
    // SEI metadata of the decoded packets: HDR is carried over, user data refuses the plan
    for seg in &p.segs {
        let Seg::Reencode { decode_first, decode_last, .. } = *seg else { continue };
        for d in decode_first..=decode_last {
            let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
            for (kind, payload) in sei_messages(&data, p.avc.length_size)? {
                match kind {
                    SEI_USER_DATA_REGISTERED => {
                        return Err(incompatible(format!("packet {d} carries SEI user data (captions or other per-picture metadata) a re-encode would drop")));
                    }
                    SEI_USER_DATA_UNREGISTERED if !is_encoder_note(&payload) => {
                        return Err(incompatible(format!("packet {d} carries unregistered SEI user data a re-encode would drop")));
                    }
                    SEI_MASTERING_DISPLAY => cfg.mastering_display = <[u8; 24]>::try_from(payload.as_slice()).ok(),
                    SEI_CONTENT_LIGHT => {
                        if let [a, b, c, e] = payload[..] {
                            cfg.content_light = Some((u16::from_be_bytes([a, b]), u16::from_be_bytes([c, e])));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // a copied run after a re-encoded one must start at an IDR
    for pair in p.segs.windows(2) {
        if let [Seg::Reencode { .. }, Seg::Copy { first, .. }] = pair {
            let data = source.file.read_sample(source.bytes, source.track, *first).map_err(io)?;
            if idr_pic_id(&data, p.avc.length_size, &params)?.is_none() {
                return Err(incompatible(format!("the copy after a boundary starts at packet {first}, which is not an IDR picture")));
            }
        }
    }
    Ok(Setup { p, params, cfg: Some(cfg) })
}

/// An unregistered SEI that only names the encoder (x264, CFA4 and others write one): nothing a
/// viewer needs, so a re-encode may drop it.
fn is_encoder_note(payload: &[u8]) -> bool {
    payload.get(16..).is_some_and(|text| text.iter().all(|&b| b == 0 || b == b'\n' || (0x20..0x7f).contains(&b)))
}

/// Check, without decoding or writing anything, that [`execute_mp4_boundary_reencode`] can run
/// `plan` on `source`: the same refusals the executor makes before it writes, so a planner or a
/// user interface can show them first (an interlaced, 10-bit, variable-frame-rate or captioned
/// source…).
pub fn check_mp4_boundary_reencode<S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    encoding: &BoundaryEncoding,
) -> Result<(), SmartCutError> {
    setup(plan, index, source, encoding).map(|_| ())
}

/// Write a frame-accurate Smart Cut `plan` (copy and boundary re-encode segments) of one H.264
/// MP4 / MOV source into `out`.
///
/// `index` is the stream index the plan was built from. Incompatible sources are refused before
/// anything is written; on any later error `out` may hold a partial file, so write to a
/// temporary file and keep it only after [`verify_mp4_boundary_reencode`] passes.
pub fn execute_mp4_boundary_reencode<W: Write + Seek, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    encoding: &BoundaryEncoding,
    out: W,
) -> Result<(W, BoundaryReport), SmartCutError> {
    let Setup { p, params, cfg } = setup(plan, index, source, encoding)?;
    let track = p.track;
    // decode every boundary first: an incompatible picture refuses the plan before writing
    let mut boundary = Vec::new();
    for seg in &p.segs {
        if let Seg::Reencode { decode_first, decode_last, pictures } = seg {
            boundary.push(decode_boundary(source, p.avc, *decode_first, *decode_last, pictures)?);
        }
    }
    let mut cfg = cfg;
    if let (Some(cfg), Some(pic)) = (cfg.as_mut(), boundary.iter().flatten().next()) {
        if (pic.width, pic.height) != (cfg.width, cfg.height) {
            return Err(incompatible(format!("the pictures are {}×{}, the sample entry says {}×{}", pic.width, pic.height, cfg.width, cfg.height)));
        }
        // signal the source colour and aspect in the encoder's VUI
        cfg.sar = pic.sar;
        cfg.color = ColorConfig { primaries: pic.color.primaries, transfer: pic.color.transfer, matrix: pic.color.matrix, full_range: pic.color.full_range };
    }
    for pic in boundary.iter().flatten() {
        planes(pic)?;
        if cfg.as_ref().is_some_and(|c| (pic.width, pic.height) != (c.width, c.height)) {
            return Err(incompatible("the boundary pictures change size"));
        }
    }
    // the output avcC: the source parameter sets, then the encoder's
    let mut avc = p.avc.clone();
    if let Some(cfg) = &cfg {
        let probe = Encoder::new(cfg.clone()).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
        let enc = AvcConfig::parse(&probe.avcc()).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
        avc.sps.extend(enc.sps);
        avc.pps.extend(enc.pps);
        avc.level = avc.level.max(enc.level);
        // avcC counts SPSs in 5 bits and PPSs in 8
        if avc.sps.len() > 31 || avc.pps.len() > 255 {
            return Err(incompatible("too many parameter sets for one avcC"));
        }
    }
    let mut entry = p.entry.clone();
    entry.codec = CodecConfig::Avc(avc);
    let brand = brand(plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let mut w = Mp4Writer::new(out, WriterOptions::new(brand)).map_err(io)?;
    let mut tcfg = TrackConfig::new(entry, track.timescale);
    tcfg.language = track.language.clone();
    tcfg.media_start = (p.delay > 0).then_some(p.delay);
    let t = w.add_track(tcfg).map_err(io)?;
    let delay = i32::try_from(p.delay).map_err(|_| SmartCutError::Timestamps { decode_index: 0 })?;
    let mut report = BoundaryReport { samples: 0, copied: 0, reencoded: 0, payload_bytes: 0, duration: 0, timescale: track.timescale, min_psnr: None };
    let mut boundary = boundary.into_iter();
    // idr_pic_id of the last written sample when it was an IDR
    let mut last_idr: Option<u32> = None;
    for (i, seg) in p.segs.iter().enumerate() {
        match seg {
            Seg::Copy { first, last } => {
                for d in *first..=*last {
                    let s = track.samples.get(d).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                    let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
                    if data.len() as u64 != u64::from(s.size) {
                        return Err(SmartCutError::StaleIndex { detail: format!("packet {d}: read {} bytes, expected {}", data.len(), s.size) });
                    }
                    let offset = s.pts.checked_sub(s.dts).and_then(|o| i32::try_from(o).ok()).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                    w.write_sample(t, WriteSample { data: &data, duration: s.duration, composition_offset: offset, is_sync: s.is_sync }).map_err(io)?;
                    last_idr = if d == *last { idr_pic_id(&data, p.avc.length_size, &params)? } else { None };
                    report.copied += 1;
                    report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
                    report.duration = report.duration.saturating_add(u64::from(s.duration));
                }
            }
            Seg::Reencode { pictures, .. } => {
                let Some(mut cfg) = cfg.clone() else { return Err(SmartCutError::NotLossless) };
                let decoded = boundary.next().ok_or(SmartCutError::NotLossless)?;
                // the IDR ids next to this segment
                let mut taken = BTreeSet::new();
                taken.extend(last_idr);
                if let Some(Seg::Copy { first, .. }) = p.segs.get(i + 1) {
                    let data = source.file.read_sample(source.bytes, source.track, *first).map_err(io)?;
                    taken.extend(idr_pic_id(&data, p.avc.length_size, &params)?);
                }
                let id = (0u16..4).find(|x| !taken.contains(&u32::from(*x))).unwrap_or(0);
                cfg.first_idr_pic_id = id;
                let mut encoder = Encoder::new(cfg).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
                let mut packets = Vec::new();
                for (k, pic) in decoded.iter().enumerate() {
                    packets.extend(encoder.encode(&planes(pic)?, k as i64).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?);
                }
                packets.extend(encoder.flush());
                if packets.len() != pictures.len() {
                    return Err(SmartCutError::Encode { detail: format!("{} packets for {} pictures", packets.len(), pictures.len()) });
                }
                for (k, (packet, &d)) in packets.iter().zip(pictures).enumerate() {
                    let duration = p.durations.get(d).copied().ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                    if packet.pts != k as i64 || packet.dts != k as i64 || (k == 0) != packet.keyframe {
                        return Err(SmartCutError::Encode { detail: format!("unexpected packet order at picture {k}") });
                    }
                    w.write_sample(t, WriteSample { data: &packet.data, duration, composition_offset: delay, is_sync: packet.keyframe }).map_err(io)?;
                    report.reencoded += 1;
                    report.payload_bytes = report.payload_bytes.saturating_add(packet.data.len() as u64);
                    report.duration = report.duration.saturating_add(u64::from(duration));
                }
                last_idr = (pictures.len() == 1).then_some(u32::from(id));
            }
        }
    }
    report.samples = report.copied + report.reencoded;
    let out = w.finish().map_err(io)?;
    Ok((out, report))
}

/// Read back a file written by [`execute_mp4_boundary_reencode`] and check it: the copied samples
/// are the source samples, the presentation timeline is the planned one without gaps, the whole
/// output decodes, and every re-encoded picture reaches `encoding.min_psnr` against its source
/// picture.
pub fn verify_mp4_boundary_reencode<S: ByteSource + ?Sized, O: ByteSource>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    encoding: &BoundaryEncoding,
    output: O,
) -> Result<BoundaryReport, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let p = prepare(plan, index, source)?;
    let file = cfa4_isobmff::open(&output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let [track] = file.tracks.as_slice() else {
        return Err(fail(format!("the output has {} tracks, expected 1", file.tracks.len())));
    };
    if track.timescale != p.track.timescale {
        return Err(fail(format!("timescale {} instead of {}", track.timescale, p.track.timescale)));
    }
    let [entry] = track.entries.as_slice() else { return Err(fail("the output has more than one sample entry".into())) };
    let CodecConfig::Avc(avc) = &entry.codec else { return Err(fail("the output is not H.264".into())) };
    if !avc.sps.starts_with(&p.avc.sps) || !avc.pps.starts_with(&p.avc.pps) {
        return Err(fail("the source parameter sets changed".into()));
    }
    // what each output sample should be: (copied source packet or None, source picture shown)
    let mut expected: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
    let mut shown = Vec::new();
    for seg in &p.segs {
        match seg {
            Seg::Copy { first, last } => {
                let mut run: Vec<usize> = (*first..=*last).collect();
                expected.extend(run.iter().map(|&d| (Some(d), Vec::new())));
                run.sort_by_key(|&d| p.track.samples.get(d).map_or(i64::MAX, |s| s.pts));
                shown.extend(run);
            }
            Seg::Reencode { pictures, .. } => {
                expected.extend(pictures.iter().map(|&d| (None, vec![d])));
                shown.extend(pictures.iter().copied());
            }
        }
    }
    if track.samples.len() != expected.len() || plan.copied_frames().saturating_add(plan.reencoded_frames()) != expected.len() as u64 {
        return Err(fail(format!("{} samples written, {} planned", track.samples.len(), expected.len())));
    }
    let mut report = BoundaryReport { samples: 0, copied: 0, reencoded: 0, payload_bytes: 0, duration: 0, timescale: track.timescale, min_psnr: None };
    for (k, (out, (copied, shown_by))) in track.samples.iter().zip(&expected).enumerate() {
        let data = file.read_sample(&output, 0, k).map_err(|e| fail(format!("sample {k}: {e}")))?;
        if let Some(d) = *copied {
            let src = p.track.samples.get(d).ok_or_else(|| fail(format!("source packet {d} is missing")))?;
            let same = source.file.read_sample(source.bytes, source.track, d).map_err(io)? == data;
            if !same || out.is_sync != src.is_sync || out.duration != src.duration || out.pts.checked_sub(out.dts) != src.pts.checked_sub(src.dts) {
                return Err(fail(format!("sample {k} differs from source packet {d}")));
            }
            report.copied += 1;
        } else {
            let planned = shown_by.first().and_then(|d| p.durations.get(*d)).copied();
            if Some(out.duration) != planned {
                return Err(fail(format!("re-encoded sample {k} lasts {}, the plan says {planned:?}", out.duration)));
            }
            report.reencoded += 1;
        }
        report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
        report.duration = report.duration.saturating_add(u64::from(out.duration));
    }
    // the presentation timeline: the planned pictures, in order, each shown as long as in the source
    let mut pts: Vec<i64> = track.samples.iter().map(|s| s.pts).collect();
    pts.sort_unstable();
    let mut at = pts.first().copied().unwrap_or(0);
    if at != p.delay {
        return Err(fail(format!("the presentation starts at {at}, expected {}", p.delay)));
    }
    for (k, (&t, &d)) in pts.iter().zip(&shown).enumerate() {
        if t != at {
            return Err(fail(format!("picture {k} is shown at {t}, expected {at}")));
        }
        at = at.saturating_add(i64::from(p.durations.get(d).copied().unwrap_or(0)));
    }
    // decode the whole output; compare the re-encoded pictures with the source
    let decode_err = |e: cfa4_h264::Error| fail(format!("the output does not decode: {e}"));
    let mut source_pictures = BTreeMap::new();
    for seg in &p.segs {
        if let Seg::Reencode { decode_first, decode_last, pictures } = seg {
            for (pic, &d) in decode_boundary(source, p.avc, *decode_first, *decode_last, pictures)?.into_iter().zip(pictures) {
                source_pictures.insert(d, pic);
            }
        }
    }
    let by_pts: BTreeMap<i64, Option<usize>> = track.samples.iter().zip(&expected).map(|(s, (_, re))| (s.pts, re.first().copied())).collect();
    let mut decoder = cfa4_h264::Decoder::from_avcc(&avc.to_bytes()).map_err(decode_err)?;
    let mut decoded = 0usize;
    let mut min_psnr: Option<f64> = None;
    let mut check = |pic: cfa4_h264::Picture| -> Result<(), SmartCutError> {
        decoded += 1;
        if let Some(Some(d)) = by_pts.get(&pic.pts) {
            let reference = source_pictures.get(d).ok_or_else(|| fail(format!("no source picture for packet {d}")))?;
            let psnr = luma_psnr(&pic, reference).ok_or_else(|| fail(format!("re-encoded picture of packet {d} has another size")))?;
            if psnr < encoding.min_psnr {
                return Err(fail(format!("re-encoded picture of packet {d}: {psnr:.1} dB < {:.1} dB", encoding.min_psnr)));
            }
            min_psnr = Some(min_psnr.map_or(psnr, |m: f64| m.min(psnr)));
        }
        Ok(())
    };
    for (k, s) in track.samples.iter().enumerate() {
        let data = file.read_sample(&output, 0, k).map_err(|e| fail(format!("sample {k}: {e}")))?;
        for pic in decoder.decode(&data, s.pts).map_err(decode_err)? {
            check(pic)?;
        }
    }
    for pic in decoder.flush() {
        check(pic)?;
    }
    if let Some(e) = decoder.take_error() {
        return Err(decode_err(e));
    }
    if decoded != expected.len() {
        return Err(fail(format!("{decoded} pictures decoded, {} written", expected.len())));
    }
    report.samples = report.copied + report.reencoded;
    report.min_psnr = min_psnr;
    Ok(report)
}
