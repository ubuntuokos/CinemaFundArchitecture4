//! Timeline Smart Render executor (WP-06 of `CFA4-SMART-CUT-RENDER-R1`): runs the
//! [`ExportExecutionPlan`] of a whole timeline (`cfa4_smart_edit::smart_render`) into one H.264
//! MP4 / MOV stream.
//!
//! - **Copied runs** are read from their own source and written packet for packet. Every copied
//!   run must use the same codec configuration (one `avcC`), the same timescale and the same
//!   composition delay, and starts at an IDR picture, so decoding never reaches across a splice.
//! - **Re-encoded runs** (boundaries, and runs whose stream does not match the output) are decoded
//!   from their source with the CFA4 H.264 decoder and encoded with the CFA4 H.264 encoder.
//! - **Rendered segments** (gaps, effects, transitions…) are encoded from the pictures the caller
//!   renders ([`RenderedFrames`]); this module knows nothing about the timeline's effects.
//!
//! Re-encoded and rendered pictures use parameter-set ids the copied configuration does not, so the
//! output `avcC` holds the copied SPS/PPS unchanged followed by the encoder's (as in
//! [`crate::boundary_reencode`]). Every encoded run starts with an IDR; neighbouring IDRs get
//! different `idr_pic_id`s. The output is constant frame rate: every source picture that is copied
//! or re-encoded must last exactly one output frame (the planner renders a clip at another frame
//! rate).
//!
//! Incompatible input is refused before anything is written ([`check_mp4_timeline`] makes the same
//! checks without decoding). [`verify_mp4_timeline`] reopens the output, compares the copied
//! samples with their sources, checks the presentation timeline, decodes the whole output and
//! compares every re-encoded picture with its source picture (luma PSNR). Rendered pictures are
//! checked to decode; comparing them would mean rendering the timeline twice.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Seek, Write};

use cfa4_h264enc::{Encoder, EncoderConfig, Packet as EncodedPacket, PacketFormat, Profile, RateControl, YuvFrame};
use cfa4_isobmff::{AvcConfig, ByteSource, CodecConfig, Mp4Writer, SampleEntry, Track, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::Codec;
use cfa4_smart_edit::media_index::StreamIndex;
use cfa4_smart_edit::plan::{ExportExecutionPlan, PLAN_VERSION, PlanSegment, SegmentMode};
use cfa4_smart_edit::smart_render::TICK_TIMEBASE;
use cfa4_smart_edit::timestamp::Rational;
use cfa4_time::TICKS_PER_SECOND;
use serde::{Deserialize, Serialize};

use crate::boundary_reencode::{
    BoundaryDecoder, BoundaryEncoding, SourceParams, SourceTiming, check_reencoded_sei, encoder_config, idr_pic_id, incompatible, luma_psnr, nals,
    parse_params, planes, source_timing,
};
use crate::smart_cut::{Mp4CopySource, SmartCutError, brand, check_access_point, io};

/// One opened source of a timeline plan: its WP-02 index and the MP4 / MOV track the plan reads.
pub struct TimelineSource<'a> {
    pub index: &'a StreamIndex,
    pub source: Mp4CopySource<'a, dyn ByteSource + 'a>,
}

/// One rendered output picture: 8-bit 4:2:0, tightly packed planes (`width × height` luma,
/// `width/2 × height/2` for each chroma plane).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedPicture {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

/// The caller's renderer: the pictures of the plan's `Render` segments.
pub trait RenderedFrames {
    /// The picture at output frame `output_frame` (a position in `segment.presentation`).
    fn picture(&mut self, segment: &PlanSegment, output_frame: u64) -> Result<RenderedPicture, String>;
}

/// What the timeline export writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineOutput {
    /// Output picture size; copied and re-encoded pictures must have it.
    pub width: u32,
    pub height: u32,
    /// Seconds per output frame, as in the timeline's edit decision list.
    pub frame_duration: Rational,
    pub encoding: BoundaryEncoding,
}

/// What a timeline export wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineReport {
    pub samples: u64,
    /// Copied unchanged from a source.
    pub copied: u64,
    /// Decoded from a source and re-encoded.
    pub reencoded: u64,
    /// Encoded from rendered pictures.
    pub rendered: u64,
    pub payload_bytes: u64,
    /// Output duration in the track timescale.
    pub duration: u64,
    pub timescale: u32,
    /// Lowest luma PSNR of a re-encoded picture against its source picture (dB; `None` when
    /// nothing was re-encoded or the output was not verified).
    pub min_psnr: Option<f64>,
}

/// A source the plan reads, resolved.
struct Src<'s, 'a> {
    ts: &'s TimelineSource<'a>,
    track: &'a Track,
    timing: SourceTiming,
    entry: &'a SampleEntry,
    avc: &'a AvcConfig,
    params: SourceParams,
}

/// One planned segment, resolved.
enum Seg {
    /// Packets `first..=last` (decode order) of `src`, copied.
    Copy { src: String, first: usize, last: usize },
    /// Packets `decode_first..=decode_last` of `src` decoded; `pictures` (decode indices, in
    /// presentation order) re-encoded.
    Reencode { src: String, decode_first: usize, decode_last: usize, pictures: Vec<usize> },
    /// Output frames `first..=last` of plan segment `segment`, rendered by the caller.
    Render { segment: usize, first: u64, last: u64 },
}

struct Prepared<'s, 'a> {
    srcs: BTreeMap<String, Src<'s, 'a>>,
    segs: Vec<Seg>,
    timescale: u32,
    /// Output frame duration in the timescale.
    frame: u32,
    /// Composition delay of every picture that is not copied: the copied runs' delay.
    delay: i64,
    /// The source whose configuration the copied runs share.
    base: Option<String>,
}

fn stale(detail: String) -> SmartCutError {
    SmartCutError::StaleIndex { detail }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `frame_duration` seconds in ticks, when it is a whole positive number of them.
fn frame_ticks(frame_duration: Rational) -> Option<i64> {
    let (num, den) = (frame_duration.num, frame_duration.den);
    if num <= 0 || den <= 0 {
        return None;
    }
    let ticks = i128::from(num).checked_mul(i128::from(TICKS_PER_SECOND))?;
    (ticks % i128::from(den) == 0).then(|| i64::try_from(ticks / i128::from(den)).ok()).flatten()
}

impl<'s, 'a> Src<'s, 'a> {
    fn open(ts: &'s TimelineSource<'a>, stream_id: u32) -> Result<Self, SmartCutError> {
        let track = ts.source.file.tracks.get(ts.source.track).ok_or(SmartCutError::NoTrack { track: ts.source.track })?;
        if ts.index.stream_id != track.id || stream_id != track.id {
            return Err(SmartCutError::WrongStream { expected: track.id, found: stream_id });
        }
        if ts.index.codec != Codec::H264 {
            return Err(SmartCutError::Unsupported {
                codec: ts.index.codec,
                source: cfa4_smart_edit::capabilities::Container::Mp4,
                output: cfa4_smart_edit::capabilities::Container::Mp4,
            });
        }
        let timing = source_timing(ts.index, track)?;
        // the sample entry of the first packet; every planned packet must use it
        let entry = track
            .samples
            .first()
            .and_then(|s| s.description_index.checked_sub(1))
            .and_then(|x| track.entries.get(usize::try_from(x).ok()?))
            .ok_or_else(|| stale(format!("track {} has no sample entry", track.id)))?;
        let CodecConfig::Avc(avc) = &entry.codec else {
            return Err(incompatible(format!("track {} is not H.264 in an avcC sample entry", track.id)));
        };
        let params = parse_params(avc)?;
        Ok(Src { ts, track, timing, entry, avc, params })
    }

    fn read(&self, d: usize) -> Result<Vec<u8>, SmartCutError> {
        self.ts.source.file.read_sample(self.ts.source.bytes, self.ts.source.track, d).map_err(io)
    }

    /// Packets `first..=last` exist and use the source's sample entry.
    fn check_range(&self, segment: usize, first: u64, last: u64) -> Result<(usize, usize), SmartCutError> {
        let n = self.track.samples.len();
        let as_index = |d: u64| usize::try_from(d).ok().filter(|&d| d < n);
        let (Some(first), Some(last)) = (as_index(first), as_index(last)) else {
            return Err(stale(format!("segment {segment}: packets {first}–{last} are past the end of track {}", self.track.id)));
        };
        let description = self.track.samples.first().map(|s| s.description_index);
        if self.track.samples.get(first..=last).unwrap_or_default().iter().any(|s| Some(s.description_index) != description) {
            return Err(SmartCutError::MixedSampleEntries);
        }
        Ok((first, last))
    }
}

fn prepare<'s, 'a>(
    plan: &ExportExecutionPlan,
    sources: &'s BTreeMap<String, TimelineSource<'a>>,
    output: &TimelineOutput,
) -> Result<Prepared<'s, 'a>, SmartCutError> {
    if plan.plan_version != PLAN_VERSION {
        return Err(SmartCutError::PlanVersion { found: plan.plan_version });
    }
    if plan.codec != Codec::H264 || brand(plan.output_container).is_none() {
        return Err(SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container });
    }
    let (w, h) = (output.width, output.height);
    if w == 0 || h == 0 || !w.is_multiple_of(2) || !h.is_multiple_of(2) || w > u32::from(u16::MAX) || h > u32::from(u16::MAX) {
        return Err(incompatible(format!("output size {w}×{h} is not an even size the sample entry can hold")));
    }
    let step = frame_ticks(output.frame_duration).ok_or_else(|| incompatible("the output frame duration is not a whole number of ticks"))?;
    // the segments follow each other on the output frame grid, and the plan's length is theirs
    let mut at: i64 = 0;
    let mut srcs: BTreeMap<String, Src<'s, 'a>> = BTreeMap::new();
    let mut segs = Vec::with_capacity(plan.segments.len());
    for (i, s) in plan.segments.iter().enumerate() {
        if s.output_in.timebase != TICK_TIMEBASE || s.output_in.value != at || s.presentation.last < s.presentation.first || s.decode.last < s.decode.first {
            return Err(SmartCutError::NotContiguous { segment: i });
        }
        let frames = i64::try_from(s.frames()).ok().and_then(|f| f.checked_mul(step)).ok_or(SmartCutError::NotContiguous { segment: i })?;
        at = at.checked_add(frames).ok_or(SmartCutError::NotContiguous { segment: i })?;
        if s.mode == SegmentMode::Render {
            segs.push(Seg::Render { segment: i, first: s.presentation.first, last: s.presentation.last });
            continue;
        }
        let Some(source) = &s.source else { return Err(stale(format!("segment {i} names no source"))) };
        let id = source.source_id.clone();
        if !srcs.contains_key(&id) {
            let ts = sources.get(&id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
            srcs.insert(id.clone(), Src::open(ts, s.stream_id)?);
        }
        let src = srcs.get(&id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
        if s.stream_id != src.track.id {
            return Err(SmartCutError::WrongStream { expected: src.track.id, found: s.stream_id });
        }
        let (first, last) = src.check_range(i, s.decode.first, s.decode.last)?;
        if s.mode.is_copy() {
            let shown = (first..=last).all(|d| src.timing.position.get(d).is_some_and(|p| (s.presentation.first..=s.presentation.last).contains(p)));
            if !shown || s.decode.count() != s.presentation.count() {
                return Err(SmartCutError::NotContiguous { segment: i });
            }
            segs.push(Seg::Copy { src: id, first, last });
        } else {
            let mut pictures = Vec::new();
            for p in s.presentation.first..=s.presentation.last {
                match usize::try_from(p).ok().and_then(|p| src.timing.order.get(p)).copied() {
                    Some(d) if (first..=last).contains(&d) => pictures.push(d),
                    _ => return Err(stale(format!("picture {p} of segment {i} is not decoded by it"))),
                }
            }
            if !src.track.samples.get(first).is_some_and(|x| x.is_sync) {
                return Err(stale(format!("segment {i} does not decode from an access point")));
            }
            segs.push(Seg::Reencode { src: id, decode_first: first, decode_last: last, pictures });
        }
    }
    if segs.is_empty() {
        return Err(SmartCutError::NotLossless);
    }
    if plan.expected_duration.timebase != TICK_TIMEBASE || plan.expected_duration.value != at {
        return Err(stale(format!("the segments last {at} ticks, the plan says {}", plan.expected_duration.value)));
    }
    // the copied runs share one configuration, timescale and composition delay
    let mut base: Option<String> = None;
    let mut delay: Option<i64> = None;
    for seg in &segs {
        let Seg::Copy { src: id, first, last } = seg else { continue };
        let src = srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
        match base.as_ref().and_then(|b| srcs.get(b)) {
            None => base = Some(id.clone()),
            Some(b) => {
                if b.avc.to_bytes() != src.avc.to_bytes() {
                    return Err(incompatible(format!("{id} has another codec configuration than the other copied runs")));
                }
                if b.track.timescale != src.track.timescale {
                    return Err(incompatible(format!("{id} has another timescale than the other copied runs")));
                }
            }
        }
        let run = src.track.samples.get(*first..=*last).unwrap_or_default();
        let first_dts = run.first().map_or(0, |s| s.dts);
        let min_pts = run.iter().map(|s| s.pts).min().unwrap_or(first_dts);
        let d = min_pts.checked_sub(first_dts).filter(|d| *d >= 0).ok_or(SmartCutError::Timestamps { decode_index: *first as u64 })?;
        if delay.is_some_and(|x| x != d) {
            return Err(incompatible(format!("the copied run of {id} has another composition delay than the runs before it")));
        }
        delay = Some(d);
    }
    // the output timescale: the copied runs', else the frame rate's own
    let (timescale, frame) = match base.as_ref().and_then(|b| srcs.get(b)) {
        Some(b) => {
            let ts = i128::from(b.track.timescale);
            let units = i128::from(output.frame_duration.num).checked_mul(ts);
            let den = i128::from(output.frame_duration.den);
            match units.filter(|u| den > 0 && u % den == 0).and_then(|u| u32::try_from(u / den).ok()).filter(|f| *f > 0) {
                Some(frame) => (b.track.timescale, frame),
                None => return Err(incompatible(format!("the output frame duration is not a whole number of 1/{} s units", b.track.timescale))),
            }
        }
        None => {
            let (num, den) = (u64::try_from(output.frame_duration.num).unwrap_or(0), u64::try_from(output.frame_duration.den).unwrap_or(0));
            let g = gcd(num, den).max(1);
            match (u32::try_from(den / g), u32::try_from(num / g)) {
                (Ok(timescale), Ok(frame)) if timescale > 0 && frame > 0 => (timescale, frame),
                _ => return Err(incompatible("the output frame duration does not fit an MP4 timescale")),
            }
        }
    };
    // constant frame rate: every source picture shown lasts one output frame
    for seg in &segs {
        let (id, shown): (&String, Vec<usize>) = match seg {
            Seg::Copy { src, first, last } => (src, (*first..=*last).collect()),
            Seg::Reencode { src, pictures, .. } => (src, pictures.clone()),
            Seg::Render { .. } => continue,
        };
        let src = srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
        let source_frame = u64::from(frame).checked_mul(u64::from(src.track.timescale));
        for d in shown {
            let shown_for = src.timing.durations.get(d).map(|&x| u64::from(x).checked_mul(u64::from(timescale)));
            if shown_for != Some(source_frame) || source_frame.is_none() {
                return Err(incompatible(format!("picture {d} of {id} does not last one output frame")));
            }
        }
    }
    Ok(Prepared { srcs, segs, timescale, frame, delay: delay.unwrap_or(0), base })
}

/// The encoder settings when nothing is copied: the output frame and rate, High profile.
fn plain_config(output: &TimelineOutput, rate: (u32, u32)) -> Result<EncoderConfig, SmartCutError> {
    let e = &output.encoding;
    if !e.crf.is_finite() || !(0.0..=51.0).contains(&e.crf) || !e.min_psnr.is_finite() {
        return Err(SmartCutError::Encode { detail: format!("crf {} (0–51) and min_psnr {} must be finite", e.crf, e.min_psnr) });
    }
    let mut cfg = EncoderConfig::new(output.width, output.height, rate.0.max(1), rate.1.max(1));
    cfg.profile = Profile::High;
    cfg.rate = RateControl::Crf(e.crf);
    cfg.bframes = 0;
    cfg.keyint = u32::MAX;
    cfg.scenecut = false;
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.aud = false;
    cfg.threads = e.threads;
    Ok(cfg)
}

struct Setup<'s, 'a> {
    p: Prepared<'s, 'a>,
    /// The encoder settings, when something is re-encoded or rendered.
    cfg: Option<EncoderConfig>,
}

fn setup<'s, 'a>(
    plan: &ExportExecutionPlan,
    sources: &'s BTreeMap<String, TimelineSource<'a>>,
    output: &TimelineOutput,
) -> Result<Setup<'s, 'a>, SmartCutError> {
    let p = prepare(plan, sources, output)?;
    let base = p.base.as_ref().and_then(|b| p.srcs.get(b));
    if let Some(b) = base {
        let video = b.entry.video.as_ref().map(|v| (u32::from(v.width), u32::from(v.height)));
        if video != Some((output.width, output.height)) {
            return Err(incompatible(format!("the copied pictures are {video:?}, the output is {}×{}", output.width, output.height)));
        }
    }
    // in-band parameter sets would redefine the ids; every planned packet is also checked against
    // the index's access points
    for seg in &p.segs {
        let (id, first, last) = match seg {
            Seg::Copy { src, first, last } => (src, *first, *last),
            Seg::Reencode { src, decode_first, decode_last, .. } => (src, *decode_first, *decode_last),
            Seg::Render { .. } => continue,
        };
        let src = p.srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
        for d in first..=last {
            let data = src.read(d)?;
            if nals(&data, src.avc.length_size)?.iter().any(|nal| matches!(nal.first().map(|b| b & 0x1f), Some(7 | 8))) {
                return Err(incompatible(format!("packet {d} of {id} carries in-band parameter sets")));
            }
            if let Some(frame) = src.ts.index.frames.get(d) {
                check_access_point(d, &data, src.avc.length_size, frame)?;
            }
        }
    }
    // a copied run starts at an IDR (decoding never reaches across a splice), and two IDRs in a row
    // have different idr_pic_ids
    let mut last_idr: Option<u32> = None;
    let mut previous: Option<(&String, usize)> = None;
    for seg in &p.segs {
        match seg {
            Seg::Copy { src: id, first, last } => {
                let src = p.srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
                let continues = previous.is_some_and(|(pid, plast)| pid == id && plast.checked_add(1) == Some(*first));
                if !continues {
                    let idr = idr_pic_id(&src.read(*first)?, src.avc.length_size, &src.params)?;
                    match idr {
                        None => return Err(incompatible(format!("the copied run of {id} starts at packet {first}, which is not an IDR picture"))),
                        Some(x) if last_idr == Some(x) && first == last => {}
                        Some(x) if last_idr == Some(x) => {
                            return Err(incompatible(format!("two IDR pictures in a row share idr_pic_id {x} at packet {first} of {id}")));
                        }
                        Some(_) => {}
                    }
                }
                last_idr = idr_pic_id(&src.read(*last)?, src.avc.length_size, &src.params)?;
                previous = Some((id, *last));
            }
            Seg::Reencode { .. } | Seg::Render { .. } => {
                last_idr = None;
                previous = None;
            }
        }
    }
    let encodes = p.segs.iter().any(|s| !matches!(s, Seg::Copy { .. }));
    if !encodes {
        return Ok(Setup { p, cfg: None });
    }
    let g = gcd(u64::from(p.timescale), u64::from(p.frame)).max(1);
    let (num, den) = (u64::from(p.timescale) / g, u64::from(p.frame) / g);
    let (Ok(num), Ok(den)) = (u32::try_from(num), u32::try_from(den)) else { return Err(incompatible("the frame rate does not fit the encoder")) };
    if num > u32::MAX / 2 {
        return Err(incompatible(format!("the frame rate {num}/{den} does not fit the encoder's timing fields")));
    }
    let mut cfg = match base {
        Some(b) => encoder_config(b.avc, &b.params, output.width, output.height, (num, den), &output.encoding)?,
        None => plain_config(output, (num, den))?,
    };
    // re-encoded sources: decodable by the boundary path, shown as the output signals
    let looks = |c: &EncoderConfig| (c.sar, c.color.primaries, c.color.transfer, c.color.matrix, c.color.full_range);
    for seg in &p.segs {
        let Seg::Reencode { src: id, decode_first, decode_last, .. } = seg else { continue };
        let src = p.srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
        let theirs = encoder_config(src.avc, &src.params, output.width, output.height, (num, den), &output.encoding)?;
        if base.is_some() && looks(&theirs) != looks(&cfg) {
            return Err(incompatible(format!("{id} has another aspect ratio or colour than the copied stream")));
        }
        if base.is_none() && p.segs.iter().all(|s| !matches!(s, Seg::Render { .. })) && looks(&cfg) != looks(&theirs) {
            // nothing copied or rendered: the first re-encoded source sets how the output is shown
            cfg.sar = theirs.sar;
            cfg.color = theirs.color;
        }
        for d in *decode_first..=*decode_last {
            check_reencoded_sei(&src.read(d)?, src.avc.length_size, d, &mut cfg)?;
        }
    }
    Ok(Setup { p, cfg: Some(cfg) })
}

/// Check, without decoding, rendering or writing anything, that [`execute_mp4_timeline`] can run
/// `plan`: the same refusals it makes before it writes.
pub fn check_mp4_timeline(plan: &ExportExecutionPlan, sources: &BTreeMap<String, TimelineSource<'_>>, output: &TimelineOutput) -> Result<(), SmartCutError> {
    setup(plan, sources, output).map(|_| ())
}

/// Encodes one run of pictures and writes each packet as the encoder emits it.
struct Run {
    encoder: Encoder,
    written: u64,
    expected: u64,
    frame: u32,
    delay: i32,
}

impl Run {
    fn push<W: Write + Seek>(
        &mut self,
        w: &mut Mp4Writer<W>,
        t: usize,
        report: &mut TimelineReport,
        picture: &YuvFrame<'_>,
        rendered: bool,
    ) -> Result<(), SmartCutError> {
        let k = i64::try_from(self.written).unwrap_or(i64::MAX);
        let packets = self.encoder.encode(picture, k).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
        self.write(w, t, report, &packets, rendered)
    }

    fn finish<W: Write + Seek>(mut self, w: &mut Mp4Writer<W>, t: usize, report: &mut TimelineReport, rendered: bool) -> Result<(), SmartCutError> {
        let packets = self.encoder.flush();
        self.write(w, t, report, &packets, rendered)?;
        if self.written != self.expected {
            return Err(SmartCutError::Encode { detail: format!("{} packets for {} pictures", self.written, self.expected) });
        }
        Ok(())
    }

    fn write<W: Write + Seek>(
        &mut self,
        w: &mut Mp4Writer<W>,
        t: usize,
        report: &mut TimelineReport,
        packets: &[EncodedPacket],
        rendered: bool,
    ) -> Result<(), SmartCutError> {
        for packet in packets {
            let k = i64::try_from(self.written).unwrap_or(i64::MAX);
            if self.written >= self.expected || packet.pts != k || packet.dts != k || (k == 0) != packet.keyframe {
                return Err(SmartCutError::Encode { detail: format!("unexpected packet order at picture {k}") });
            }
            w.write_sample(t, WriteSample { data: &packet.data, duration: self.frame, composition_offset: self.delay, is_sync: packet.keyframe })
                .map_err(io)?;
            self.written += 1;
            if rendered {
                report.rendered += 1;
            } else {
                report.reencoded += 1;
            }
            report.payload_bytes = report.payload_bytes.saturating_add(packet.data.len() as u64);
            report.duration = report.duration.saturating_add(u64::from(self.frame));
        }
        Ok(())
    }
}

/// Write the timeline `plan` into `out`: copied runs from `sources`, re-encoded runs decoded from
/// them, rendered segments from `frames`.
///
/// Incompatible plans are refused before anything is written; on a later error `out` may hold a
/// partial file, so write to a temporary file and keep it only after [`verify_mp4_timeline`]
/// passes.
pub fn execute_mp4_timeline<W: Write + Seek>(
    plan: &ExportExecutionPlan,
    sources: &BTreeMap<String, TimelineSource<'_>>,
    frames: &mut dyn RenderedFrames,
    output: &TimelineOutput,
    out: W,
) -> Result<(W, TimelineReport), SmartCutError> {
    let Setup { p, cfg } = setup(plan, sources, output)?;
    let base = p.base.as_ref().and_then(|b| p.srcs.get(b));
    // the output avcC: the copied configuration, then the encoder's
    let mut avc = base.map(|b| b.avc.clone());
    if let Some(cfg) = &cfg {
        let probe = Encoder::new(cfg.clone()).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
        let enc = AvcConfig::parse(&probe.avcc()).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
        avc = Some(match avc {
            Some(mut a) => {
                a.sps.extend(enc.sps);
                a.pps.extend(enc.pps);
                a.level = a.level.max(enc.level);
                a
            }
            None => enc,
        });
    }
    let avc = avc.ok_or(SmartCutError::NotLossless)?;
    if avc.sps.len() > 31 || avc.pps.len() > 255 {
        return Err(incompatible("too many parameter sets for one avcC"));
    }
    let (w16, h16) = (u16::try_from(output.width).unwrap_or(u16::MAX), u16::try_from(output.height).unwrap_or(u16::MAX));
    let entry = match base {
        Some(b) => SampleEntry { codec: CodecConfig::Avc(avc), ..b.entry.clone() },
        None => SampleEntry::avc(avc, w16, h16),
    };
    let brand = brand(plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let mut w = Mp4Writer::new(out, WriterOptions::new(brand)).map_err(io)?;
    let mut tcfg = TrackConfig::new(entry, p.timescale);
    if let Some(b) = base {
        tcfg.language = b.track.language.clone();
        tcfg.matrix = Some(b.track.matrix);
    }
    tcfg.media_start = (p.delay > 0).then_some(p.delay);
    let t = w.add_track(tcfg).map_err(io)?;
    let delay = i32::try_from(p.delay).map_err(|_| SmartCutError::Timestamps { decode_index: 0 })?;
    let mut report = TimelineReport { samples: 0, copied: 0, reencoded: 0, rendered: 0, payload_bytes: 0, duration: 0, timescale: p.timescale, min_psnr: None };
    // idr_pic_id of the last written sample when it was an IDR
    let mut last_idr: Option<u32> = None;
    for (i, seg) in p.segs.iter().enumerate() {
        match seg {
            Seg::Copy { src: id, first, last } => {
                let src = p.srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
                for d in *first..=*last {
                    let s = src.track.samples.get(d).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                    let data = src.read(d)?;
                    if data.len() as u64 != u64::from(s.size) {
                        return Err(stale(format!("packet {d} of {id}: read {} bytes, expected {}", data.len(), s.size)));
                    }
                    let offset = s.pts.checked_sub(s.dts).and_then(|o| i32::try_from(o).ok()).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })?;
                    w.write_sample(t, WriteSample { data: &data, duration: p.frame, composition_offset: offset, is_sync: s.is_sync }).map_err(io)?;
                    if d == *last {
                        last_idr = idr_pic_id(&data, src.avc.length_size, &src.params)?;
                    }
                    report.copied += 1;
                    report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
                    report.duration = report.duration.saturating_add(u64::from(p.frame));
                }
            }
            Seg::Reencode { .. } | Seg::Render { .. } => {
                let Some(mut cfg) = cfg.clone() else { return Err(SmartCutError::NotLossless) };
                // the IDR ids next to this run
                let mut taken = BTreeSet::new();
                taken.extend(last_idr);
                if let Some(Seg::Copy { src: id, first, .. }) = p.segs.get(i + 1) {
                    let src = p.srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
                    taken.extend(idr_pic_id(&src.read(*first)?, src.avc.length_size, &src.params)?);
                }
                let idr = (0u16..4).find(|x| !taken.contains(&u32::from(*x))).unwrap_or(0);
                cfg.first_idr_pic_id = idr;
                let (width, height) = (cfg.width, cfg.height);
                let encoder = Encoder::new(cfg).map_err(|e| SmartCutError::Encode { detail: e.to_string() })?;
                match seg {
                    Seg::Reencode { src: id, decode_first, decode_last, pictures } => {
                        let src = p.srcs.get(id).ok_or_else(|| SmartCutError::MissingSource { source_id: id.clone() })?;
                        let mut run = Run { encoder, written: 0, expected: pictures.len() as u64, frame: p.frame, delay };
                        let mut decoder = BoundaryDecoder::new(&src.ts.source, src.avc, *decode_first, *decode_last, pictures)?;
                        while let Some(pic) = decoder.next_picture()? {
                            if (pic.width, pic.height) != (width, height) {
                                return Err(incompatible(format!("the pictures of {id} are {}×{}, the output is {width}×{height}", pic.width, pic.height)));
                            }
                            run.push(&mut w, t, &mut report, &planes(&pic)?, false)?;
                        }
                        run.finish(&mut w, t, &mut report, false)?;
                        last_idr = (pictures.len() == 1).then_some(u32::from(idr));
                    }
                    Seg::Render { segment, first, last } => {
                        let plan_segment = plan.segments.get(*segment).ok_or(SmartCutError::NotContiguous { segment: *segment })?;
                        let count = last.saturating_sub(*first).saturating_add(1);
                        let mut run = Run { encoder, written: 0, expected: count, frame: p.frame, delay };
                        for f in *first..=*last {
                            let pic = frames.picture(plan_segment, f).map_err(|detail| SmartCutError::Render { segment: *segment, detail })?;
                            let (cw, ch) = (width as usize / 2, height as usize / 2);
                            let luma = (width as usize).checked_mul(height as usize);
                            if (pic.width, pic.height) != (width, height) || Some(pic.y.len()) != luma || pic.u.len() != cw * ch || pic.v.len() != cw * ch {
                                return Err(SmartCutError::Render { segment: *segment, detail: format!("frame {f} is not a {width}×{height} 4:2:0 picture") });
                            }
                            let yuv = YuvFrame { y: &pic.y, u: &pic.u, v: &pic.v, y_stride: width as usize, uv_stride: cw };
                            run.push(&mut w, t, &mut report, &yuv, true)?;
                        }
                        run.finish(&mut w, t, &mut report, true)?;
                        last_idr = (count == 1).then_some(u32::from(idr));
                    }
                    Seg::Copy { .. } => {}
                }
            }
        }
    }
    report.samples = report.copied + report.reencoded + report.rendered;
    let out = w.finish().map_err(io)?;
    Ok((out, report))
}

/// What each output sample should be.
enum Expect<'p> {
    Copied { src: &'p str, d: usize },
    Reencoded { src: &'p str, d: usize, boundary: usize },
    Rendered,
}

/// Read back a file written by [`execute_mp4_timeline`] and check it: the copied samples are the
/// source samples, the presentation timeline is the planned one without gaps, the whole output
/// decodes, and every re-encoded picture reaches `output.encoding.min_psnr` against its source
/// picture.
pub fn verify_mp4_timeline<O: ByteSource>(
    plan: &ExportExecutionPlan,
    sources: &BTreeMap<String, TimelineSource<'_>>,
    output: &TimelineOutput,
    written: O,
) -> Result<TimelineReport, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    if !output.encoding.min_psnr.is_finite() {
        return Err(fail(format!("min_psnr {} is not a finite threshold", output.encoding.min_psnr)));
    }
    let p = prepare(plan, sources, output)?;
    let file = cfa4_isobmff::open(&written).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let [track] = file.tracks.as_slice() else {
        return Err(fail(format!("the output has {} tracks, expected 1", file.tracks.len())));
    };
    if track.timescale != p.timescale {
        return Err(fail(format!("timescale {} instead of {}", track.timescale, p.timescale)));
    }
    let [entry] = track.entries.as_slice() else { return Err(fail("the output has more than one sample entry".into())) };
    let CodecConfig::Avc(avc) = &entry.codec else { return Err(fail("the output is not H.264".into())) };
    if let Some(b) = p.base.as_ref().and_then(|b| p.srcs.get(b))
        && (!avc.sps.starts_with(&b.avc.sps) || !avc.pps.starts_with(&b.avc.pps))
    {
        return Err(fail("the copied parameter sets changed".into()));
    }
    // the re-encoded segments, for decoding their source pictures one boundary at a time
    let boundaries: Vec<(&str, usize, usize, &Vec<usize>)> = p
        .segs
        .iter()
        .filter_map(|s| match s {
            Seg::Reencode { src, decode_first, decode_last, pictures } => Some((src.as_str(), *decode_first, *decode_last, pictures)),
            _ => None,
        })
        .collect();
    let mut expected: Vec<Expect<'_>> = Vec::new();
    let mut boundary = 0usize;
    for seg in &p.segs {
        match seg {
            Seg::Copy { src, first, last } => expected.extend((*first..=*last).map(|d| Expect::Copied { src: src.as_str(), d })),
            Seg::Reencode { src, pictures, .. } => {
                expected.extend(pictures.iter().map(|&d| Expect::Reencoded { src: src.as_str(), d, boundary }));
                boundary += 1;
            }
            Seg::Render { first, last, .. } => {
                for _ in *first..=*last {
                    expected.push(Expect::Rendered);
                }
            }
        }
    }
    if track.samples.len() != expected.len() {
        return Err(fail(format!("{} samples written, {} planned", track.samples.len(), expected.len())));
    }
    let mut report =
        TimelineReport { samples: 0, copied: 0, reencoded: 0, rendered: 0, payload_bytes: 0, duration: 0, timescale: track.timescale, min_psnr: None };
    for (k, (out, exp)) in track.samples.iter().zip(&expected).enumerate() {
        let data = file.read_sample(&written, 0, k).map_err(|e| fail(format!("sample {k}: {e}")))?;
        if out.duration != p.frame {
            return Err(fail(format!("sample {k} lasts {}, one frame is {}", out.duration, p.frame)));
        }
        match exp {
            Expect::Copied { src: id, d } => {
                let src = p.srcs.get(*id).ok_or_else(|| SmartCutError::MissingSource { source_id: (*id).to_string() })?;
                let s = src.track.samples.get(*d).ok_or_else(|| fail(format!("packet {d} of {id} is missing")))?;
                if src.read(*d)? != data || out.is_sync != s.is_sync || out.pts.checked_sub(out.dts) != s.pts.checked_sub(s.dts) {
                    return Err(fail(format!("sample {k} differs from packet {d} of {id}")));
                }
                report.copied += 1;
            }
            Expect::Reencoded { .. } | Expect::Rendered => {
                if out.pts.checked_sub(out.dts) != Some(p.delay) {
                    return Err(fail(format!("sample {k} has another composition delay than the copied runs")));
                }
                if matches!(exp, Expect::Rendered) {
                    report.rendered += 1;
                } else {
                    report.reencoded += 1;
                }
            }
        }
        report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
        report.duration = report.duration.saturating_add(u64::from(out.duration));
    }
    // the presentation timeline: one picture per output frame, from the composition delay on
    let mut pts: Vec<i64> = track.samples.iter().map(|s| s.pts).collect();
    pts.sort_unstable();
    for (k, &t) in pts.iter().enumerate() {
        let at = i64::try_from(k).ok().and_then(|k| k.checked_mul(i64::from(p.frame))).and_then(|x| x.checked_add(p.delay));
        if Some(t) != at {
            return Err(fail(format!("picture {k} is shown at {t}, expected {at:?}")));
        }
    }
    // decode the whole output; compare the re-encoded pictures with their source pictures
    let decode_err = |e: cfa4_h264::Error| fail(format!("the output does not decode: {e}"));
    let by_pts: BTreeMap<i64, &Expect<'_>> = track.samples.iter().zip(&expected).map(|(s, e)| (s.pts, e)).collect();
    let mut decoder = cfa4_h264::Decoder::from_avcc(&avc.to_bytes()).map_err(decode_err)?;
    let mut decoded = 0usize;
    let mut min_psnr: Option<f64> = None;
    let mut current: Option<(usize, BoundaryDecoder<'_, '_, dyn ByteSource + '_>)> = None;
    let mut check = |pic: cfa4_h264::Picture| -> Result<(), SmartCutError> {
        decoded += 1;
        let Some(Expect::Reencoded { src: id, d, boundary: b }) = by_pts.get(&pic.pts) else { return Ok(()) };
        let src = p.srcs.get(*id).ok_or_else(|| SmartCutError::MissingSource { source_id: (*id).to_string() })?;
        if current.as_ref().is_none_or(|(c, _)| c != b) {
            let (_, first, last, pictures) = boundaries.get(*b).copied().ok_or_else(|| fail(format!("no source picture for packet {d} of {id}")))?;
            current = Some((*b, BoundaryDecoder::new(&src.ts.source, src.avc, first, last, pictures)?));
        }
        let want = src.track.samples.get(*d).map(|s| s.pts);
        let reference = match current.as_mut() {
            Some((_, decoder)) => decoder.next_picture()?,
            None => None,
        };
        let reference = reference.filter(|r| Some(r.pts) == want).ok_or_else(|| fail(format!("no source picture for packet {d} of {id}")))?;
        let psnr = luma_psnr(&pic, &reference).ok_or_else(|| fail(format!("the re-encoded picture of packet {d} of {id} has another size")))?;
        if psnr < output.encoding.min_psnr {
            return Err(fail(format!("re-encoded picture of packet {d} of {id}: {psnr:.1} dB < {:.1} dB", output.encoding.min_psnr)));
        }
        min_psnr = Some(min_psnr.map_or(psnr, |m: f64| m.min(psnr)));
        Ok(())
    };
    for (k, s) in track.samples.iter().enumerate() {
        let data = file.read_sample(&written, 0, k).map_err(|e| fail(format!("sample {k}: {e}")))?;
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
    report.samples = report.copied + report.reencoded + report.rendered;
    report.min_psnr = min_psnr;
    Ok(report)
}
