//! Smart Cut executor (WP-04 of `CFA4-SMART-CUT-RENDER-R1`): writes the copy segments of an
//! [`ExportExecutionPlan`] from an MP4 / MOV source into a new MP4 / MOV file, packet for packet,
//! and checks the result by reading it back.
//!
//! The plan comes from `cfa4_smart_edit::copy_planner` (WP-03). This executor runs only plans in
//! which every segment is copied ([`ExportExecutionPlan::is_lossless`]); boundary re-encode and
//! render segments belong to the Smart Render executor (WP-06) and are refused here, never
//! dropped or re-encoded silently.
//!
//! - **Payloads:** every copied sample is written with the source bytes unchanged. MP4 → MP4 / MOV
//!   keeps the length-prefixed NAL framing and the source sample entry (`avcC` for H.264, `hvcC`
//!   for HEVC), so a `compatible` plan needs no representation change either and runs the same way.
//! - **Timing:** the output starts at decode time 0; sample durations and composition offsets
//!   (PTS − DTS) are copied, and an edit list starts the presentation at the first copied picture.
//! - **Consistency:** the executor refuses when the index the plan was built from no longer
//!   matches the source track (sample count or sizes), when the copied samples are not one run in
//!   decode order, or when they use more than one sample entry.
//! - **Validation:** [`verify_mp4_copy`] reopens the written file and compares every sample with
//!   its source sample: payload bytes, sync flag, duration, composition offset and sample entry
//!   (SCE-001 §18).
//!
//! The output goes to any `Write + Seek`; the caller writes it to a temporary file and renames it
//! into place, as every export does.

use std::fmt;
use std::io::{Seek, Write};

use cfa4_isobmff::{Brand, ByteSource, CodecConfig, Mp4File, Mp4Writer, Sample, SampleEntry, Track, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::{Codec, Container};
use cfa4_smart_edit::media_index::{FrameIndexEntry, RandomAccessKind, StreamIndex};
use cfa4_smart_edit::plan::{ExportExecutionPlan, PLAN_VERSION};
use cfa4_smart_edit::timestamp::Rational;
use serde::{Deserialize, Serialize};

/// Why a plan could not be executed or its output did not verify.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum SmartCutError {
    /// The plan was written by another plan version.
    PlanVersion { found: u32 },
    /// Some segment must be re-encoded or rendered (WP-06), or the plan has no segments.
    NotLossless,
    /// A timeline plan, whose segments name their own sources: the Smart Render executor (WP-06)
    /// runs those.
    TimelinePlan,
    /// Only H.264 is copied from MP4 / MOV into MP4 / MOV in this release.
    Unsupported { codec: Codec, source: Container, output: Container },
    /// The source has no track at this index.
    NoTrack { track: usize },
    /// A segment names another stream than the source track.
    WrongStream { expected: u32, found: u32 },
    /// The index no longer matches the source track (it was built from another file or version).
    StaleIndex { detail: String },
    /// The copied samples are not one run in decode order.
    NotContiguous { segment: usize },
    /// The copied samples use more than one sample entry (codec configuration).
    MixedSampleEntries,
    /// A timestamp does not fit the output tables.
    Timestamps { decode_index: u64 },
    /// Reading the source, writing the output or reading it back failed.
    Io { detail: String },
    /// The written file does not hold the copied samples unchanged.
    Verification { detail: String },
    /// A segment the boundary re-encode executor does not run (a rendered segment: gap, effect…).
    RenderSegment { segment: usize },
    /// The re-encoded pictures cannot be spliced with the copied stream safely (profile, chroma
    /// format, bit depth, interlacing, parameter-set ids, a copy that does not start at an IDR…).
    /// Nothing is spliced; the export needs a full render instead (SCE-001 §8).
    Incompatible { detail: String },
    /// Decoding the source pictures of a boundary segment failed.
    Decode { detail: String },
    /// Encoding the boundary pictures failed.
    Encode { detail: String },
    /// A timeline plan names a source the executor was not given.
    MissingSource { source_id: String },
    /// Rendering a picture of a rendered segment failed.
    Render { segment: usize, detail: String },
}

impl fmt::Display for SmartCutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmartCutError::PlanVersion { found } => write!(f, "plan version {found} is not {PLAN_VERSION}"),
            SmartCutError::NotLossless => write!(f, "the plan re-encodes or renders a segment; only copy plans run here"),
            SmartCutError::TimelinePlan => write!(f, "a timeline plan cannot run as a Smart Cut of one source"),
            SmartCutError::Unsupported { codec, source, output } => {
                write!(f, "packet copy of {} from {} to {} is not supported", codec.label(), source.label(), output.label())
            }
            SmartCutError::NoTrack { track } => write!(f, "the source has no track {track}"),
            SmartCutError::WrongStream { expected, found } => write!(f, "the plan is for stream {found}, the source track is {expected}"),
            SmartCutError::StaleIndex { detail } => write!(f, "the index does not match the source: {detail}"),
            SmartCutError::NotContiguous { segment } => write!(f, "copy segment {segment} does not follow the previous one in decode order"),
            SmartCutError::MixedSampleEntries => write!(f, "the copied samples use more than one sample entry"),
            SmartCutError::Timestamps { decode_index } => write!(f, "the timestamps of packet {decode_index} do not fit the output"),
            SmartCutError::Io { detail } => write!(f, "I/O: {detail}"),
            SmartCutError::Verification { detail } => write!(f, "verification failed: {detail}"),
            SmartCutError::RenderSegment { segment } => write!(f, "segment {segment} must be rendered; only copy and boundary re-encode segments run here"),
            SmartCutError::Incompatible { detail } => write!(f, "the boundary re-encode cannot be spliced with the copied stream: {detail}"),
            SmartCutError::Decode { detail } => write!(f, "decoding the boundary pictures failed: {detail}"),
            SmartCutError::Encode { detail } => write!(f, "encoding the boundary pictures failed: {detail}"),
            SmartCutError::MissingSource { source_id } => write!(f, "the plan reads {source_id}, which was not opened"),
            SmartCutError::Render { segment, detail } => write!(f, "rendering segment {segment} failed: {detail}"),
        }
    }
}

impl std::error::Error for SmartCutError {}

impl From<SmartCutError> for crate::ExportError {
    fn from(e: SmartCutError) -> Self {
        match e {
            SmartCutError::Io { detail } => crate::ExportError::Io(detail),
            other => crate::ExportError::Unsupported(other.to_string()),
        }
    }
}

/// What a copy wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyReport {
    /// Samples (pictures) written.
    pub samples: u64,
    /// Payload bytes written (sample data only).
    pub payload_bytes: u64,
    /// Output duration in the track timescale (sum of the sample durations).
    pub duration: u64,
    /// The track timescale.
    pub timescale: u32,
}

/// The source of a copy: an opened MP4 / MOV file, its bytes and the video track the plan cuts.
pub struct Mp4CopySource<'a, S: ByteSource + ?Sized> {
    pub file: &'a Mp4File,
    pub bytes: &'a S,
    /// Index into [`Mp4File::tracks`]; the plan's `stream_id` is that track's id.
    pub track: usize,
}

/// The source sample range a lossless plan copies, checked against the index and the track.
struct CopyRange<'a> {
    track: &'a cfa4_isobmff::Track,
    first: usize,
    last: usize,
    entry: &'a SampleEntry,
}

pub(crate) fn io(e: impl fmt::Display) -> SmartCutError {
    SmartCutError::Io { detail: e.to_string() }
}

pub(crate) fn brand(c: Container) -> Option<Brand> {
    match c {
        Container::Mp4 => Some(Brand::Mp4),
        Container::Mov => Some(Brand::Mov),
        _ => None,
    }
}

fn copy_range<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'a, S>,
) -> Result<CopyRange<'a>, SmartCutError> {
    if plan.plan_version != PLAN_VERSION {
        return Err(SmartCutError::PlanVersion { found: plan.plan_version });
    }
    if !plan.is_lossless() {
        return Err(SmartCutError::NotLossless);
    }
    // a timeline plan (Smart Render, several sources) runs in the WP-06 executor
    if plan.segments.iter().any(|s| s.source.is_some()) {
        return Err(SmartCutError::TimelinePlan);
    }
    let unsupported = SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container };
    if !matches!(plan.codec, Codec::H264 | Codec::Hevc)
        || brand(plan.source_container).is_none()
        || brand(plan.output_container).is_none()
        || index.codec != plan.codec
    {
        return Err(unsupported);
    }
    let track = source.file.tracks.get(source.track).ok_or(SmartCutError::NoTrack { track: source.track })?;
    if index.stream_id != track.id {
        return Err(SmartCutError::WrongStream { expected: track.id, found: index.stream_id });
    }
    if index.frames.len() != track.samples.len() {
        return Err(SmartCutError::StaleIndex {
            detail: format!("{} indexed pictures, {} samples in track {}", index.frames.len(), track.samples.len(), track.id),
        });
    }
    check_timebase(index, track)?;
    // the copy segments, in output order, must read one run of packets in decode order
    let mut range: Option<(u64, u64)> = None;
    for (i, s) in plan.segments.iter().enumerate() {
        if s.stream_id != track.id {
            return Err(SmartCutError::WrongStream { expected: track.id, found: s.stream_id });
        }
        if s.decode.last < s.decode.first {
            return Err(SmartCutError::NotContiguous { segment: i });
        }
        range = match range {
            None => Some((s.decode.first, s.decode.last)),
            Some((first, last)) if last.checked_add(1) == Some(s.decode.first) => Some((first, s.decode.last)),
            Some(_) => return Err(SmartCutError::NotContiguous { segment: i }),
        };
    }
    let (first, last) = range.ok_or(SmartCutError::NotLossless)?;
    let as_index = |d: u64| usize::try_from(d).ok().filter(|&d| d < track.samples.len());
    let (Some(first), Some(last)) = (as_index(first), as_index(last)) else {
        return Err(SmartCutError::StaleIndex { detail: format!("packets {first}–{last} are past the end of track {}", track.id) });
    };
    let mut description = None;
    for d in first..=last {
        let (Some(sample), Some(frame)) = (track.samples.get(d), index.frames.get(d)) else {
            return Err(SmartCutError::StaleIndex { detail: format!("packet {d} is missing") });
        };
        check_sample(d, sample, frame)?;
        match description {
            None => description = Some(sample.description_index),
            Some(x) if x == sample.description_index => {}
            Some(_) => return Err(SmartCutError::MixedSampleEntries),
        }
    }
    let entry = description
        .and_then(|x| x.checked_sub(1))
        .and_then(|x| track.entries.get(usize::try_from(x).ok()?))
        .ok_or_else(|| SmartCutError::StaleIndex { detail: format!("track {} has no sample entry for the copied packets", track.id) })?;
    // the sample entry carries the plan's codec (`avcC` for H.264, `hvcC` for HEVC)
    match (&entry.codec, plan.codec) {
        (CodecConfig::Avc(_), Codec::H264) | (CodecConfig::Hevc(_), Codec::Hevc) => {}
        _ => return Err(unsupported),
    }
    Ok(CopyRange { track, first, last, entry })
}

/// Composition offset (PTS − DTS) of a source sample, as the output tables store it.
fn composition_offset(s: &cfa4_isobmff::Sample, decode_index: usize) -> Result<i32, SmartCutError> {
    s.pts.checked_sub(s.dts).and_then(|o| i32::try_from(o).ok()).ok_or(SmartCutError::Timestamps { decode_index: decode_index as u64 })
}

/// Write the copy segments of a lossless `plan` from `source` into `out` as a new MP4 / MOV file.
///
/// `index` is the stream index the plan was built from (WP-02); it must still describe the source
/// track. Nothing is re-encoded: a plan with any other segment is refused before anything is
/// written. On an error `out` may hold a partial file, so write to a temporary file and keep it
/// only on success.
pub fn execute_mp4_copy<W: Write + Seek, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    out: W,
) -> Result<(W, CopyReport), SmartCutError> {
    let r = copy_range(plan, index, source)?;
    let samples = r.track.samples.get(r.first..=r.last).unwrap_or_default();
    // the copied run must still last as long as planned (a replaced file with another final
    // sample duration is refused; check_timebase made the units the same)
    let total = samples.iter().fold(0i64, |t, s| t.saturating_add(i64::from(s.duration)));
    if total != plan.expected_duration.value || plan.expected_duration.timebase != index.timebase {
        return Err(SmartCutError::StaleIndex { detail: format!("the copied packets last {total}, the plan says {}", plan.expected_duration.value) });
    }
    let base_dts = samples.first().map(|s| s.dts).unwrap_or(0);
    // the presentation starts at the earliest copied picture
    let mut start: i64 = i64::MAX;
    for (k, s) in samples.iter().enumerate() {
        let rel = s.pts.checked_sub(base_dts).ok_or(SmartCutError::Timestamps { decode_index: (r.first + k) as u64 })?;
        start = start.min(rel);
    }
    if start < 0 {
        return Err(SmartCutError::Timestamps { decode_index: r.first as u64 });
    }
    let brand = brand(plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let mut w = Mp4Writer::new(out, WriterOptions::new(brand)).map_err(io)?;
    let mut cfg = TrackConfig::new(r.entry.clone(), r.track.timescale);
    cfg.language = r.track.language.clone();
    cfg.media_start = (start > 0).then_some(start);
    // keep the display transform (rotated phone footage…)
    cfg.matrix = Some(r.track.matrix);
    let t = w.add_track(cfg).map_err(io)?;
    let mut report = CopyReport { samples: 0, payload_bytes: 0, duration: 0, timescale: r.track.timescale };
    for (k, s) in samples.iter().enumerate() {
        let d = r.first + k;
        let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
        if data.len() as u64 != u64::from(s.size) {
            return Err(SmartCutError::StaleIndex { detail: format!("packet {d}: read {} bytes, expected {}", data.len(), s.size) });
        }
        match (&r.entry.codec, index.frames.get(d)) {
            (CodecConfig::Avc(avc), Some(frame)) => check_access_point(d, &data, avc.length_size, frame)?,
            (CodecConfig::Hevc(hevc), Some(frame)) => check_hevc_access_point(d, &data, hevc.length_size, frame)?,
            _ => {}
        }
        let ws = WriteSample { data: &data, duration: s.duration, composition_offset: composition_offset(s, d)?, is_sync: s.is_sync };
        w.write_sample(t, ws).map_err(io)?;
        report.samples += 1;
        report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
        report.duration = report.duration.saturating_add(u64::from(s.duration));
    }
    let out = w.finish().map_err(io)?;
    Ok((out, report))
}

/// Read back a file written by [`execute_mp4_copy`] and check that it holds exactly the copied
/// source samples: same count, payload bytes, sync flags, durations, composition offsets and
/// sample entry. Returns the same report as the copy.
pub fn verify_mp4_copy<S: ByteSource + ?Sized, O: ByteSource>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    output: O,
) -> Result<CopyReport, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let r = copy_range(plan, index, source)?;
    let file = cfa4_isobmff::open(&output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let [track] = file.tracks.as_slice() else {
        return Err(fail(format!("the output has {} tracks, expected 1", file.tracks.len())));
    };
    let expected = r.last - r.first + 1;
    if track.samples.len() != expected || plan.copied_frames() != expected as u64 {
        return Err(fail(format!("{} samples written, {expected} copied, {} planned", track.samples.len(), plan.copied_frames())));
    }
    if track.timescale != r.track.timescale {
        return Err(fail(format!("timescale {} instead of {}", track.timescale, r.track.timescale)));
    }
    if track.entries.len() != 1 || track.entries.first().map(|e| &e.codec) != Some(&r.entry.codec) {
        return Err(fail("the sample entry (codec configuration) changed".into()));
    }
    let mut report = CopyReport { samples: 0, payload_bytes: 0, duration: 0, timescale: track.timescale };
    for (k, out) in track.samples.iter().enumerate() {
        let d = r.first + k;
        let src = r.track.samples.get(d).ok_or_else(|| fail(format!("source packet {d} is missing")))?;
        if out.is_sync != src.is_sync || out.duration != src.duration || out.pts.checked_sub(out.dts) != src.pts.checked_sub(src.dts) {
            return Err(fail(format!("sample {k} (source packet {d}) has other timing or sync flag")));
        }
        let a = file.read_sample(&output, 0, k).map_err(|e| fail(format!("sample {k}: {e}")))?;
        let b = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
        if a != b {
            return Err(fail(format!("sample {k} (source packet {d}) payload differs")));
        }
        report.samples += 1;
        report.payload_bytes = report.payload_bytes.saturating_add(a.len() as u64);
        report.duration = report.duration.saturating_add(u64::from(out.duration));
    }
    Ok(report)
}

/// The index must be on the track's timebase: plan positions and durations are read from both.
pub(crate) fn check_timebase(index: &StreamIndex, track: &Track) -> Result<(), SmartCutError> {
    let timebase = Rational::new(1, i64::from(track.timescale));
    if index.timebase != timebase {
        return Err(SmartCutError::StaleIndex {
            detail: format!("the index timebase is {}/{}, track {} counts 1/{}", index.timebase.num, index.timebase.den, track.id, track.timescale),
        });
    }
    Ok(())
}

/// Packet `d` of the source must be the one that was indexed: same size, timestamps and sync flag, so a
/// remuxed or replaced file (another frame rate, the same access units) is not cut by a stale plan.
pub(crate) fn check_sample(d: usize, sample: &Sample, frame: &FrameIndexEntry) -> Result<(), SmartCutError> {
    let stale = |detail: String| Err(SmartCutError::StaleIndex { detail });
    if u64::from(sample.size) != u64::from(frame.size) {
        return stale(format!("packet {d} is {} bytes, the index says {}", sample.size, frame.size));
    }
    if frame.pts != Some(sample.pts) || frame.dts != Some(sample.dts) {
        return stale(format!("packet {d} has PTS {} / DTS {}, the index says {:?} / {:?}", sample.pts, sample.dts, frame.pts, frame.dts));
    }
    // the plan's access points come from the index: the file must mark the same sync samples
    if frame.keyframe != sample.is_sync {
        return stale(format!("packet {d} is {}a sync sample, the index says otherwise", if sample.is_sync { "" } else { "not " }));
    }
    Ok(())
}

/// Packet `d` must be the kind of picture the index says where the plan relies on it: an IDR in
/// the index is an IDR in the bitstream and the other way round, so a source re-encoded with
/// other access points (same sizes, timestamps and sync table) is not cut by a stale plan.
pub(crate) fn check_access_point(d: usize, data: &[u8], length_size: u8, frame: &FrameIndexEntry) -> Result<(), SmartCutError> {
    let nals = cfa4_bitstream::length_prefixed_nals(data, usize::from(length_size))
        .map_err(|e| SmartCutError::StaleIndex { detail: format!("packet {d} does not split into NAL units: {e}") })?;
    let idr = nals.iter().any(|n| n.first().map(|b| b & 0x1f) == Some(5));
    if idr != (frame.random_access == RandomAccessKind::Idr) {
        let is = if idr { "is" } else { "is not" };
        return Err(SmartCutError::StaleIndex { detail: format!("packet {d} {is} an IDR picture, the index says otherwise") });
    }
    Ok(())
}

/// [`check_access_point`] for HEVC: an IRAP picture in the index (IDR, CRA, BLA) is one in the
/// bitstream and the other way round, and an IDR stays an IDR.
pub(crate) fn check_hevc_access_point(d: usize, data: &[u8], length_size: u8, frame: &FrameIndexEntry) -> Result<(), SmartCutError> {
    let nals = cfa4_bitstream::length_prefixed_nals(data, usize::from(length_size))
        .map_err(|e| SmartCutError::StaleIndex { detail: format!("packet {d} does not split into NAL units: {e}") })?;
    // VCL NAL unit types of the base layer
    let types: Vec<u8> = nals
        .iter()
        .filter_map(|n| match &n[..] {
            [h0, h1, ..] if h0 & 1 == 0 && h1 >> 3 == 0 => Some((h0 >> 1) & 0x3F),
            _ => None,
        })
        .filter(|t| *t < 32)
        .collect();
    let irap = types.iter().any(|t| (16..=23).contains(t));
    let idr = types.iter().any(|t| matches!(t, 19 | 20));
    let kind = frame.random_access;
    if irap != kind.is_access_point() || idr != (kind == RandomAccessKind::Idr) {
        let is = if idr {
            "an IDR"
        } else if irap {
            "a CRA / BLA"
        } else {
            "not an IRAP"
        };
        return Err(SmartCutError::StaleIndex { detail: format!("packet {d} is {is} picture, the index says {kind:?}") });
    }
    Ok(())
}
