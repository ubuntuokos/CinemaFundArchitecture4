//! Boundary re-encode of HEVC (WP-07 of `CFA4-SMART-CUT-RENDER-R1`): the HEVC side of
//! [`crate::boundary_reencode`]. Whole GOPs are copied packet for packet; only the pictures
//! between a cut and the nearest clean access point are decoded with the CFA4 HEVC decoder and
//! re-encoded with the CFA4 HEVC encoder (Main or Main 10, I and P pictures).
//!
//! - **One stream, one sample entry:** the re-encoded pictures use VPS / SPS / PPS ids the source
//!   does not, so the output `hvcC` carries the source parameter sets unchanged followed by the
//!   encoder's. Every re-encoded segment starts with an IDR picture. A copied segment that
//!   follows one starts at a clean access point (an IDR, or a CRA / BLA picture without RASL
//!   pictures); when that is not an IDR, the last re-encoded sample ends with an end-of-sequence
//!   NAL unit, so the copied picture starts a new coded video sequence and its picture order
//!   count does not depend on the re-encoded pictures.
//! - **Compatibility first (SCE-001 §8):** Main or Main 10, 4:2:0 at 8 or 10 bits, progressive
//!   frames, 4-byte NAL lengths, an even picture size, free parameter-set ids and no parameter
//!   sets inside the samples. Anything else is refused before writing.
//! - **Validation:** [`verify`] reopens the output, checks the copied samples and the timeline,
//!   decodes the whole output and compares every re-encoded picture with its source picture
//!   (luma PSNR).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Seek, Write};

use cfa4_bitstream::unescape_rbsp;
use cfa4_hevc::encode::{ColorConfig, Encoder, EncoderConfig, Packet as EncodedPacket, PacketFormat, RateControl, YuvFrame};
use cfa4_hevc::params::{Pps, Sps, Vps};
use cfa4_isobmff::{ByteSource, CodecConfig, HevcConfig, HevcNalArray, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::{Codec, StreamFormat};
use cfa4_smart_edit::media_index::{RandomAccessKind, StreamIndex};
use cfa4_smart_edit::plan::ExportExecutionPlan;

use crate::boundary_reencode::{BoundaryEncoding, BoundaryReport, HdrMetadata, Prepared, Seg, check_output_samples, check_sei, incompatible, nals, prepare};
use crate::smart_cut::{Mp4CopySource, SmartCutError, brand, check_hevc_access_point, io};

const NAL_VPS: u8 = 32;
const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;
/// An end-of-sequence NAL unit (type 36, layer 0, temporal id 0), with its 4-byte length.
const END_OF_SEQUENCE: [u8; 6] = [0, 0, 0, 2, 36 << 1, 1];

/// The NAL unit type of `nal` (its 2-byte header).
fn nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|h| (h >> 1) & 0x3f)
}

/// The source parameter sets of an `hvcC`, by id.
struct SourceParams {
    vps: [bool; 16],
    sps: Vec<Option<Sps>>,
    pps: [bool; 64],
}

fn parse_params(hvcc: &HevcConfig) -> Result<SourceParams, SmartCutError> {
    let mut params = SourceParams { vps: [false; 16], sps: vec![None; 16], pps: [false; 64] };
    for array in &hvcc.arrays {
        for nal in &array.nalus {
            let rbsp = unescape_rbsp(nal.get(2..).unwrap_or_default());
            match nal_type(nal) {
                Some(NAL_VPS) => {
                    let vps = Vps::parse(&rbsp).map_err(|e| incompatible(format!("source VPS: {e}")))?;
                    *params.vps.get_mut(usize::from(vps.id)).ok_or_else(|| incompatible("source VPS id out of range"))? = true;
                }
                Some(NAL_SPS) => {
                    let sps = Sps::parse(&rbsp).map_err(|e| incompatible(format!("source SPS: {e}")))?;
                    let id = sps.id as usize;
                    *params.sps.get_mut(id).ok_or_else(|| incompatible("source SPS id out of range"))? = Some(sps);
                }
                Some(NAL_PPS) => {
                    let pps = Pps::parse(&rbsp).map_err(|e| incompatible(format!("source PPS: {e}")))?;
                    *params.pps.get_mut(pps.id as usize).ok_or_else(|| incompatible("source PPS id out of range"))? = true;
                }
                _ => {}
            }
        }
    }
    Ok(params)
}

/// Main (1) or Main 10 (2), by `general_profile_idc` or a compatibility flag.
fn main_profile(sps: &Sps) -> bool {
    let compatible = |j: u32| sps.ptl.compatibility & (1u32 << (31 - j)) != 0;
    matches!(sps.ptl.profile_idc, 1 | 2) || compatible(1) || compatible(2)
}

/// The encoder settings that match the source stream, or why there are none.
fn encoder_config(
    hvcc: &HevcConfig,
    params: &SourceParams,
    width: u32,
    height: u32,
    rate: (u32, u32),
    encoding: &BoundaryEncoding,
) -> Result<EncoderConfig, SmartCutError> {
    if !encoding.crf.is_finite() || !(0.0..=51.0).contains(&encoding.crf) || !encoding.min_psnr.is_finite() {
        return Err(SmartCutError::Encode { detail: format!("crf {} (0–51) and min_psnr {} must be finite", encoding.crf, encoding.min_psnr) });
    }
    if hvcc.length_size != 4 {
        return Err(incompatible(format!("{}-byte NAL lengths (the encoder writes 4)", hvcc.length_size)));
    }
    let used: Vec<&Sps> = params.sps.iter().flatten().collect();
    let Some(first) = used.first() else { return Err(incompatible("the source has no SPS")) };
    let bit_depth = first.bit_depth_luma;
    for s in &used {
        if !main_profile(s) {
            return Err(incompatible(format!("HEVC profile {} (the encoder writes Main and Main 10)", s.ptl.profile_idc)));
        }
        if s.chroma_format_idc != 1 || s.separate_colour_plane || s.bit_depth_luma != s.bit_depth_chroma || !matches!(s.bit_depth_luma, 8 | 10) {
            return Err(incompatible("only 4:2:0 sources of 8 or 10 bits can be re-encoded at a boundary"));
        }
        if s.bit_depth_luma != bit_depth {
            return Err(incompatible("the source parameter sets differ in bit depth"));
        }
        if s.ptl.interlaced_source || s.vui.as_ref().is_some_and(|v| v.field_seq) {
            return Err(incompatible("interlaced (field) sources cannot be re-encoded at a boundary yet"));
        }
        let (_, _, w, h) = s.crop_rect();
        if (w, h) != (width, height) {
            return Err(incompatible(format!("the pictures are {w}×{h}, the sample entry says {width}×{height}")));
        }
    }
    // the encoder's VUI repeats the source's aspect and colour, which every SPS must share
    let looks =
        |s: &Sps| s.vui.as_ref().map(|v| (v.sar, v.colour_primaries, v.transfer_characteristics, v.matrix_coefficients, v.full_range && v.range_signalled));
    let look = looks(first);
    if used.iter().any(|s| looks(s) != look) {
        return Err(incompatible("the source parameter sets differ in aspect ratio or colour"));
    }
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(incompatible(format!("picture size {width}×{height} is not even")));
    }
    let free = |taken: &[bool]| taken.iter().position(|t| !t).and_then(|i| u8::try_from(i).ok());
    let sps_taken: Vec<bool> = params.sps.iter().map(Option::is_some).collect();
    let (Some(vps_id), Some(sps_id), Some(pps_id)) = (free(&params.vps), free(&sps_taken), free(&params.pps)) else {
        return Err(incompatible("no free parameter-set id"));
    };
    let mut cfg = EncoderConfig::new(width, height, rate.0.max(1), rate.1.max(1));
    cfg.bit_depth = u8::try_from(bit_depth).unwrap_or(8);
    cfg.rate = RateControl::Crf(encoding.crf);
    cfg.keyint = 0;
    cfg.format = PacketFormat::LengthPrefixed;
    // one slice per worker; a single-threaded export gets one slice, so its output is repeatable
    cfg.slices = if encoding.threads == 1 { 1 } else { 0 };
    (cfg.vps_id, cfg.sps_id, cfg.pps_id) = (vps_id, sps_id, pps_id);
    match look {
        Some((sar, primaries, transfer, matrix, full_range)) => {
            cfg.sar = if sar.0 > 0 && sar.1 > 0 { sar } else { (1, 1) };
            cfg.color = ColorConfig { primaries, transfer, matrix, full_range };
        }
        // no VUI: the colour description says "unspecified"
        None => cfg.color = ColorConfig { primaries: 2, transfer: 2, matrix: 2, full_range: false },
    }
    Ok(cfg)
}

/// The stream format a planner judges an HEVC boundary re-encode by (profile, chroma format,
/// bit depth), read from the first SPS of `hvcc`. `None` when it has no SPS or it does not parse.
pub fn hevc_stream_format(hvcc: &HevcConfig) -> Option<StreamFormat> {
    let nal = hvcc.arrays.iter().flat_map(|a| a.nalus.iter()).find(|n| nal_type(n) == Some(NAL_SPS))?;
    let sps = Sps::parse(&unescape_rbsp(nal.get(2..)?)).ok()?;
    let profile = match sps.ptl.profile_idc {
        1 => "main",
        2 => "main10",
        3 => "mainstillpicture",
        4 => "rext",
        _ => "other",
    };
    let chroma = match sps.chroma_format_idc {
        0 => "4:0:0",
        1 => "4:2:0",
        2 => "4:2:2",
        _ => "4:4:4",
    };
    let bit_depth = u8::try_from(sps.bit_depth_luma.max(sps.bit_depth_chroma)).unwrap_or(u8::MAX);
    Some(StreamFormat { profile: profile.into(), chroma: chroma.into(), bit_depth })
}

/// The source pictures of a re-encoded segment, decoded on demand in presentation order.
struct BoundaryDecoder<'s, 'a, S: ByteSource + ?Sized> {
    source: &'s Mp4CopySource<'a, S>,
    decoder: cfa4_hevc::Decoder,
    next: usize,
    last: usize,
    wanted: BTreeSet<i64>,
    ready: VecDeque<cfa4_hevc::Picture>,
    flushed: bool,
    delivered: usize,
}

impl<'s, 'a, S: ByteSource + ?Sized> BoundaryDecoder<'s, 'a, S> {
    fn new(source: &'s Mp4CopySource<'a, S>, hvcc: &HevcConfig, decode_first: usize, decode_last: usize, pictures: &[usize]) -> Result<Self, SmartCutError> {
        let mut decoder = cfa4_hevc::Decoder::with_threads(1);
        decoder.configure_hvcc(&hvcc.to_bytes()).map_err(|e| SmartCutError::Decode { detail: e.to_string() })?;
        let track = source.file.tracks.get(source.track).ok_or(SmartCutError::NoTrack { track: source.track })?;
        let mut wanted = BTreeSet::new();
        for &d in pictures {
            wanted.insert(track.samples.get(d).map(|s| s.pts).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?);
        }
        Ok(BoundaryDecoder { source, decoder, next: decode_first, last: decode_last, wanted, ready: VecDeque::new(), flushed: false, delivered: 0 })
    }

    /// The next wanted picture, or `None` after the last one.
    fn next_picture(&mut self) -> Result<Option<cfa4_hevc::Picture>, SmartCutError> {
        let decode_err = |e: cfa4_hevc::Error| SmartCutError::Decode { detail: e.to_string() };
        loop {
            if let Some(p) = self.ready.pop_front() {
                self.delivered += 1;
                return Ok(Some(p));
            }
            if self.flushed {
                if let Some(e) = self.decoder.take_error() {
                    return Err(decode_err(e));
                }
                if self.delivered != self.wanted.len() {
                    return Err(SmartCutError::Decode { detail: format!("{} of {} boundary pictures decoded", self.delivered, self.wanted.len()) });
                }
                return Ok(None);
            }
            let pictures = if self.next <= self.last {
                let d = self.next;
                self.next += 1;
                let track = self.source.file.tracks.get(self.source.track).ok_or(SmartCutError::NoTrack { track: self.source.track })?;
                let pts = track.samples.get(d).map(|s| s.pts).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                let data = self.source.file.read_sample(self.source.bytes, self.source.track, d).map_err(io)?;
                self.decoder.decode(&data, pts).map_err(decode_err)?
            } else {
                self.flushed = true;
                self.decoder.flush()
            };
            self.ready.extend(pictures.into_iter().filter(|p| self.wanted.contains(&p.pts)));
        }
    }
}

fn planes(p: &cfa4_hevc::Picture) -> Result<YuvFrame<'_>, SmartCutError> {
    if p.chroma_format != 1 {
        return Err(incompatible("the boundary pictures are not 4:2:0"));
    }
    let (y_stride, uv_stride) = (p.y_stride, p.uv_stride);
    match (&p.y, &p.u, &p.v) {
        (cfa4_hevc::Plane::U8(y), cfa4_hevc::Plane::U8(u), cfa4_hevc::Plane::U8(v)) => Ok(YuvFrame::U8 { y, u, v, y_stride, uv_stride }),
        (cfa4_hevc::Plane::U16(y), cfa4_hevc::Plane::U16(u), cfa4_hevc::Plane::U16(v)) => Ok(YuvFrame::U16 { y, u, v, y_stride, uv_stride }),
        _ => Err(incompatible("the boundary pictures mix sample sizes")),
    }
}

/// Luma PSNR of two pictures of the same size and bit depth (99 dB when identical).
fn luma_psnr(a: &cfa4_hevc::Picture, b: &cfa4_hevc::Picture) -> Option<f64> {
    if (a.width, a.height, a.bit_depth) != (b.width, b.height, b.bit_depth) || a.y.len() < a.y_stride.saturating_mul(a.height as usize) {
        return None;
    }
    let (w, h) = (a.width as usize, a.height as usize);
    let mut sse = 0u64;
    for row in 0..h {
        for x in 0..w {
            let (i, j) = (row * a.y_stride + x, row * b.y_stride + x);
            if i >= a.y.len() || j >= b.y.len() {
                return None;
            }
            sse += u64::from(a.y.get(i).abs_diff(b.y.get(j))).pow(2);
        }
    }
    if sse == 0 {
        return Some(99.0);
    }
    let peak = f64::from((1u32 << a.bit_depth.min(16)) - 1);
    let mse = sse as f64 / (w * h).max(1) as f64;
    Some(10.0 * (peak * peak / mse).log10())
}

struct Setup<'a> {
    p: Prepared<'a>,
    hvcc: &'a HevcConfig,
    cfg: Option<EncoderConfig>,
    /// Decode indices of the copied samples that follow a re-encoded segment without being IDR
    /// pictures: the re-encoded segment before them ends with an end-of-sequence NAL unit.
    needs_eos: BTreeSet<usize>,
}

fn setup<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'a, S>,
    encoding: &BoundaryEncoding,
) -> Result<Setup<'a>, SmartCutError> {
    let p = prepare(plan, index, source)?;
    let CodecConfig::Hevc(hvcc) = &p.entry.codec else {
        return Err(SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container });
    };
    let params = parse_params(hvcc)?;
    // every planned packet: no in-band parameter sets (they could reuse the encoder's ids) and
    // the planned access points are still there
    for seg in &p.segs {
        let (first, last) = match *seg {
            Seg::Copy { first, last } => (first, last),
            Seg::Reencode { decode_first, decode_last, .. } => (decode_first, decode_last),
        };
        for d in first..=last {
            let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
            if nals(&data, hvcc.length_size)?.iter().any(|nal| matches!(nal_type(nal), Some(NAL_VPS | NAL_SPS | NAL_PPS))) {
                return Err(incompatible(format!("packet {d} carries in-band parameter sets")));
            }
            if let Some(frame) = index.frames.get(d) {
                check_hevc_access_point(d, &data, hvcc.length_size, frame)?;
            }
        }
    }
    // a copied run after a re-encoded one starts at a clean access point
    let mut needs_eos = BTreeSet::new();
    for pair in p.segs.windows(2) {
        if let [Seg::Reencode { .. }, Seg::Copy { first, .. }] = pair {
            let kind = index.frames.get(*first).map(|f| f.random_access);
            if !kind.is_some_and(RandomAccessKind::is_clean) {
                return Err(incompatible(format!("the copy after a boundary starts at packet {first}, which is not a clean access point")));
            }
            if kind != Some(RandomAccessKind::Idr) {
                needs_eos.insert(*first);
            }
        }
    }
    let mut reencoded = p.segs.iter().filter_map(|s| match s {
        Seg::Reencode { pictures, .. } => Some(pictures),
        Seg::Copy { .. } => None,
    });
    let Some(first) = reencoded.next() else { return Ok(Setup { p, hvcc, cfg: None, needs_eos }) };
    // one frame rate: the encoder signals a fixed one in its VUI
    let frame = first.first().and_then(|&d| p.durations.get(d)).copied().unwrap_or(1);
    for pictures in std::iter::once(first).chain(reencoded) {
        if pictures.iter().any(|&d| p.durations.get(d) != Some(&frame)) {
            return Err(incompatible("the re-encoded pictures have a variable frame rate"));
        }
    }
    let (width, height) = p.entry.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let (mut num, mut den) = (p.track.timescale, frame.max(1));
    let (mut a, mut b) = (num, den);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    if a > 1 {
        (num, den) = (num / a, den / a);
    }
    let mut cfg = encoder_config(hvcc, &params, width, height, (num, den), encoding)?;
    // SEI metadata of the decoded packets: HDR is carried over, user data refuses the plan
    let mut hdr = HdrMetadata::default();
    for seg in &p.segs {
        let Seg::Reencode { decode_first, decode_last, .. } = *seg else { continue };
        for d in decode_first..=decode_last {
            let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
            check_sei(&data, hvcc.length_size, Codec::Hevc, d, &mut hdr)?;
        }
    }
    (cfg.mastering_display, cfg.content_light) = (hdr.mastering_display, hdr.content_light);
    Ok(Setup { p, hvcc, cfg: Some(cfg), needs_eos })
}

/// The refusals [`execute`] makes before it writes.
pub(crate) fn check<S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    encoding: &BoundaryEncoding,
) -> Result<(), SmartCutError> {
    setup(plan, index, source, encoding).map(|_| ())
}

/// The output `hvcC`: the source's, with the encoder's parameter sets appended.
fn output_hvcc(source: &HevcConfig, encoder: &Encoder) -> Result<HevcConfig, SmartCutError> {
    let mut hvcc = source.clone();
    let enc = HevcConfig::parse(&encoder.hvcc()).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
    hvcc.general_level_idc = hvcc.general_level_idc.max(enc.general_level_idc);
    for array in enc.arrays {
        match hvcc.arrays.iter_mut().find(|a| a.nal_type == array.nal_type) {
            Some(existing) => {
                existing.nalus.extend(array.nalus);
                // the samples carry no parameter sets: every one is in the sample entry
                existing.completeness = true;
            }
            None => hvcc.arrays.push(HevcNalArray { completeness: true, ..array }),
        }
    }
    if hvcc.arrays.len() > 255 || hvcc.arrays.iter().any(|a| a.nalus.len() > usize::from(u16::MAX)) {
        return Err(incompatible("too many parameter sets for one hvcC"));
    }
    Ok(hvcc)
}

/// [`crate::boundary_reencode::execute_mp4_boundary_reencode`] of an HEVC plan.
pub(crate) fn execute<W: Write + Seek, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    encoding: &BoundaryEncoding,
    out: W,
) -> Result<(W, BoundaryReport), SmartCutError> {
    let Setup { p, hvcc: source_hvcc, cfg, needs_eos } = setup(plan, index, source, encoding)?;
    let track = p.track;
    let mut entry = p.entry.clone();
    if let Some(cfg) = &cfg {
        let probe = Encoder::new(cfg.clone()).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
        entry.codec = CodecConfig::Hevc(output_hvcc(source_hvcc, &probe)?);
    }
    let brand = brand(plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let mut w = Mp4Writer::new(out, WriterOptions::new(brand)).map_err(io)?;
    let mut tcfg = TrackConfig::new(entry, track.timescale);
    tcfg.language = track.language.clone();
    tcfg.media_start = (p.delay > 0).then_some(p.delay);
    tcfg.matrix = Some(track.matrix);
    let t = w.add_track(tcfg).map_err(io)?;
    let delay = i32::try_from(p.delay).map_err(|_| SmartCutError::Timestamps { decode_index: 0 })?;
    let mut report = BoundaryReport { samples: 0, copied: 0, reencoded: 0, payload_bytes: 0, duration: 0, timescale: track.timescale, min_psnr: None };
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
                    report.copied += 1;
                    report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
                    report.duration = report.duration.saturating_add(u64::from(s.duration));
                }
            }
            Seg::Reencode { decode_first, decode_last, pictures } => {
                let Some(cfg) = cfg.clone() else { return Err(SmartCutError::NotLossless) };
                let eos = matches!(p.segs.get(i + 1), Some(Seg::Copy { first, .. }) if needs_eos.contains(first));
                let mut decoder = BoundaryDecoder::new(source, source_hvcc, *decode_first, *decode_last, pictures)?;
                let (width, height) = (cfg.width, cfg.height);
                let mut encoder = Encoder::new(cfg).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
                let mut written = 0usize;
                let mut write = |w: &mut Mp4Writer<W>, report: &mut BoundaryReport, packet: EncodedPacket| -> Result<(), SmartCutError> {
                    let k = written;
                    let d = pictures
                        .get(k)
                        .copied()
                        .ok_or_else(|| SmartCutError::Encode { detail: format!("more packets than the {} pictures", pictures.len()) })?;
                    let duration = p.durations.get(d).copied().ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                    if packet.pts != k as i64 || packet.dts != k as i64 || (k == 0) != packet.keyframe {
                        return Err(SmartCutError::Encode { detail: format!("unexpected packet order at picture {k}") });
                    }
                    let mut data = packet.data;
                    if eos && k + 1 == pictures.len() {
                        data.extend_from_slice(&END_OF_SEQUENCE);
                    }
                    w.write_sample(t, WriteSample { data: &data, duration, composition_offset: delay, is_sync: packet.keyframe }).map_err(io)?;
                    report.reencoded += 1;
                    report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
                    report.duration = report.duration.saturating_add(u64::from(duration));
                    written += 1;
                    Ok(())
                };
                let mut k = 0i64;
                while let Some(pic) = decoder.next_picture()? {
                    if (pic.width, pic.height) != (width, height) {
                        return Err(incompatible("the boundary pictures change size"));
                    }
                    for packet in encoder.encode(&planes(&pic)?, k).map_err(|e| SmartCutError::Encode { detail: e.to_string() })? {
                        write(&mut w, &mut report, packet)?;
                    }
                    k += 1;
                }
                for packet in encoder.flush().map_err(|e| SmartCutError::Encode { detail: e.to_string() })? {
                    write(&mut w, &mut report, packet)?;
                }
                if written != pictures.len() {
                    return Err(SmartCutError::Encode { detail: format!("{written} packets for {} pictures", pictures.len()) });
                }
            }
        }
    }
    report.samples = report.copied + report.reencoded;
    let out = w.finish().map_err(io)?;
    Ok((out, report))
}

/// [`crate::boundary_reencode::verify_mp4_boundary_reencode`] of an HEVC plan.
pub(crate) fn verify<S: ByteSource + ?Sized, O: ByteSource>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    encoding: &BoundaryEncoding,
    output: O,
) -> Result<BoundaryReport, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    if !encoding.min_psnr.is_finite() {
        return Err(fail(format!("min_psnr {} is not a finite threshold", encoding.min_psnr)));
    }
    let p = prepare(plan, index, source)?;
    let CodecConfig::Hevc(source_hvcc) = &p.entry.codec else {
        return Err(SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container });
    };
    let file = cfa4_isobmff::open(&output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let [track] = file.tracks.as_slice() else {
        return Err(fail(format!("the output has {} tracks, expected 1", file.tracks.len())));
    };
    if track.timescale != p.track.timescale {
        return Err(fail(format!("timescale {} instead of {}", track.timescale, p.track.timescale)));
    }
    let [entry] = track.entries.as_slice() else { return Err(fail("the output has more than one sample entry".into())) };
    let CodecConfig::Hevc(hvcc) = &entry.codec else { return Err(fail("the output is not HEVC".into())) };
    // the source parameter sets are all still there, unchanged and first in their arrays
    let kept = source_hvcc.arrays.iter().all(|a| hvcc.arrays.iter().any(|b| b.nal_type == a.nal_type && b.nalus.starts_with(&a.nalus)));
    if !kept {
        return Err(fail("the source parameter sets changed".into()));
    }
    let (expected, mut report) = check_output_samples(plan, &p, source, &file, &output)?;
    // decode the whole output; compare the re-encoded pictures with the source pictures, which
    // are decoded picture by picture as the output reaches them
    let decode_err = |e: cfa4_hevc::Error| fail(format!("the output does not decode: {e}"));
    let boundaries: Vec<(usize, usize, &Vec<usize>)> = p
        .segs
        .iter()
        .filter_map(|seg| match seg {
            Seg::Reencode { decode_first, decode_last, pictures } => Some((*decode_first, *decode_last, pictures)),
            Seg::Copy { .. } => None,
        })
        .collect();
    let boundary_of: BTreeMap<usize, usize> = boundaries.iter().enumerate().flat_map(|(b, (_, _, pictures))| pictures.iter().map(move |&d| (d, b))).collect();
    let mut source_pictures: Option<(usize, BoundaryDecoder<'_, '_, S>)> = None;
    let by_pts: BTreeMap<i64, Option<usize>> = track.samples.iter().zip(&expected).map(|(s, (_, re))| (s.pts, re.first().copied())).collect();
    let mut decoder = cfa4_hevc::Decoder::with_threads(1);
    decoder.configure_hvcc(&hvcc.to_bytes()).map_err(decode_err)?;
    let mut decoded = 0usize;
    let mut min_psnr: Option<f64> = None;
    let mut check = |pic: cfa4_hevc::Picture| -> Result<(), SmartCutError> {
        decoded += 1;
        if let Some(Some(d)) = by_pts.get(&pic.pts) {
            let b = boundary_of.get(d).copied().ok_or_else(|| fail(format!("no source picture for packet {d}")))?;
            if source_pictures.as_ref().is_none_or(|(current, _)| *current != b) {
                let (first, last, pictures) = boundaries.get(b).copied().ok_or_else(|| fail(format!("no source picture for packet {d}")))?;
                source_pictures = Some((b, BoundaryDecoder::new(source, source_hvcc, first, last, pictures)?));
            }
            let want = p.track.samples.get(*d).map(|s| s.pts);
            let reference = match source_pictures.as_mut() {
                Some((_, decoder)) => decoder.next_picture()?,
                None => None,
            };
            let reference = reference.filter(|r| Some(r.pts) == want).ok_or_else(|| fail(format!("no source picture for packet {d}")))?;
            let psnr = luma_psnr(&pic, &reference).ok_or_else(|| fail(format!("re-encoded picture of packet {d} has another size")))?;
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
    report.min_psnr = min_psnr;
    Ok(report)
}
