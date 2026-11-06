//! Smart Cut planner (WP-03): the [`ExportExecutionPlan`] for cutting one source range out of one
//! video stream, without writing anything (CFA4-SCE-001 v1.5 §13, P4 phase 2).
//!
//! The planner works on the [`StreamIndex`] of WP-02 and the [`Registry`] of WP-01:
//!
//! - **Copy spans** are decided from the bitstream: a span starts at a clean access point (IDR,
//!   closed GOP) and ends where the pictures read so far in decode order are exactly a run of
//!   consecutive presentation positions, so copying those packets outputs those pictures and no
//!   other. Every picture in it must be independent or refer only within its GOP, and no codec
//!   configuration change may fall inside.
//! - **GOP-boundary mode** copies only. A range that does not start and end at such points is
//!   refused with the nearest safe points before and after; the planner never moves a cut
//!   silently (plan R1 §7).
//! - **Frame-accurate mode** copies the longest such span inside the range and re-encodes the
//!   pictures before and after it (határ-újrakódolás), or the whole range when it holds no copy
//!   span. Re-encoding runs only after the user confirms the plan
//!   ([`ExportExecutionPlan::requires_confirmation`]).
//! - **Capabilities:** the registry says whether the plan can run now. A plan that needs an
//!   operation the registry does not allow is still returned, for the preview, with
//!   `executable: false` and the reasons; nothing unproven counts as lossless.
//!
//! Times: the request names source times in [`Tick`]s on the stream's own PTS timeline. The
//! output keeps every picture shown during `[in_point, out_point)`: the picture on screen at
//! `in_point` (even if it started earlier) up to the last one that starts before `out_point`.

use serde::{Deserialize, Serialize};

use cfa4_time::Tick;

use crate::capabilities::{Codec, Container, Policy, Registry, StreamFormat};
use crate::media_index::{DependencyStatus, FrameIndexEntry, MediaIndex, StreamIndex};
use crate::plan::{CopyGuarantee, ExportExecutionPlan, PLAN_VERSION, PlanSegment, SegmentMode, SegmentReason, Span};
use crate::timestamp::{MediaTime, Rational};

/// How exactly the cut must follow the requested points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CutMode {
    /// The output holds exactly the requested pictures; the boundaries are re-encoded when they
    /// do not fall on safe copy points.
    FrameAccurate,
    /// Copy only: the requested points must be safe copy points, or the plan is refused with the
    /// nearest ones.
    GopBoundary,
}

/// What to cut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartCutRequest {
    pub stream_id: u32,
    /// Source time of the cut-in (inclusive), on the stream's PTS timeline.
    pub in_point: Tick,
    /// Source time of the cut-out (exclusive).
    pub out_point: Tick,
    pub mode: CutMode,
    pub guarantee: CopyGuarantee,
    pub source_container: Container,
    pub output_container: Container,
    /// Profile, chroma subsampling and bit depth of the stream, for the boundary encoder's limits.
    /// `None`: unknown, so a boundary re-encode cannot be shown to fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<StreamFormat>,
}

/// A picture where a lossless cut can be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafePoint {
    /// Presentation position of the picture.
    pub presentation: u64,
    /// Its PTS (for a cut-in) or the time just after it (for a cut-out).
    pub time: MediaTime,
}

/// The safe copy points nearest to a requested range, for the user to choose from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafePoints {
    /// The last safe cut-in at or before the requested in picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_before: Option<SafePoint>,
    /// The first safe cut-in after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_after: Option<SafePoint>,
    /// The last safe cut-out at or before the requested out picture (from the safe cut-in at or
    /// before the requested one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_before: Option<SafePoint>,
    /// The first safe cut-out after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_after: Option<SafePoint>,
}

/// Why no plan can be made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "reason")]
pub enum PlanRefusal {
    UnknownStream {
        stream_id: u32,
    },
    /// The stream holds no picture.
    EmptyStream,
    /// The cut-out is not after the cut-in.
    EmptyRange,
    /// The range starts before the first picture or ends after the last one.
    OutOfRange,
    /// A picture has no PTS or no presentation position; cut points cannot be resolved.
    MissingTimestamps {
        decode_index: u64,
    },
    /// Presentation timestamps repeat or do not fit the tick range.
    InvalidTimestamps {
        decode_index: u64,
    },
    /// GOP-boundary mode: the requested points are not safe copy points.
    NotAtSafePoints {
        first: u64,
        last: u64,
        safe: Box<SafePoints>,
    },
    /// Pictures that must be re-encoded cannot be decoded (no clean access point before them, a
    /// missing reference, an unanalysed picture).
    UndecodablePictures {
        first: u64,
        last: u64,
    },
}

impl std::fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanRefusal::UnknownStream { stream_id } => write!(f, "no stream {stream_id} in the index"),
            PlanRefusal::EmptyStream => write!(f, "the stream holds no picture"),
            PlanRefusal::EmptyRange => write!(f, "the cut-out is not after the cut-in"),
            PlanRefusal::OutOfRange => write!(f, "the range lies outside the stream"),
            PlanRefusal::MissingTimestamps { decode_index } => write!(f, "picture {decode_index} has no presentation timestamp"),
            PlanRefusal::InvalidTimestamps { decode_index } => write!(f, "picture {decode_index} has a repeated or out-of-range timestamp"),
            PlanRefusal::NotAtSafePoints { first, last, .. } => {
                write!(f, "pictures {first}–{last} cannot be copied losslessly: the cut points are not at safe GOP boundaries")
            }
            PlanRefusal::UndecodablePictures { first, last } => write!(f, "pictures {first}–{last} cannot be decoded for re-encoding"),
        }
    }
}

impl std::error::Error for PlanRefusal {}

/// A Smart Cut plan and whether it can run now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartCutPlan {
    pub plan: ExportExecutionPlan,
    /// Every operation the plan needs is allowed by the registry.
    pub executable: bool,
    /// Why not, one line each (empty when executable).
    pub blockers: Vec<String>,
    /// The nearest safe copy points around the requested range.
    pub safe_points: SafePoints,
}

/// The stream as the planner sees it: pictures by presentation position, with their source ticks.
struct Timeline<'a> {
    stream: &'a StreamIndex,
    /// Frame (index into `stream.frames`) at each presentation position.
    by_presentation: Vec<usize>,
    /// PTS tick at each presentation position.
    ticks: Vec<i64>,
}

impl<'a> Timeline<'a> {
    fn new(stream: &'a StreamIndex) -> Result<Timeline<'a>, PlanRefusal> {
        let n = stream.frames.len();
        if n == 0 {
            return Err(PlanRefusal::EmptyStream);
        }
        let mut by_presentation: Vec<Option<usize>> = vec![None; n];
        for (i, f) in stream.frames.iter().enumerate() {
            let missing = PlanRefusal::MissingTimestamps { decode_index: f.decode_index };
            let (Some(p), Some(_)) = (f.presentation_index, f.pts) else { return Err(missing) };
            let slot =
                usize::try_from(p).ok().and_then(|p| by_presentation.get_mut(p)).ok_or(PlanRefusal::InvalidTimestamps { decode_index: f.decode_index })?;
            if slot.is_some() {
                return Err(PlanRefusal::InvalidTimestamps { decode_index: f.decode_index });
            }
            *slot = Some(i);
        }
        // presentation positions run 0..n without gaps
        let by_presentation: Vec<usize> = by_presentation.into_iter().collect::<Option<_>>().ok_or(PlanRefusal::InvalidTimestamps { decode_index: 0 })?;
        let mut ticks = Vec::with_capacity(n);
        for &i in &by_presentation {
            let f = stream.frames.get(i).ok_or(PlanRefusal::EmptyStream)?;
            let invalid = PlanRefusal::InvalidTimestamps { decode_index: f.decode_index };
            let t = f.pts.and_then(|v| MediaTime::new(v, stream.timebase).to_tick()).ok_or(invalid.clone())?;
            if ticks.last().is_some_and(|&prev: &i64| t.0 <= prev) {
                return Err(invalid);
            }
            ticks.push(t.0);
        }
        Ok(Timeline { stream, by_presentation, ticks })
    }

    fn frame(&self, p: u64) -> Option<&'a FrameIndexEntry> {
        let i = *self.by_presentation.get(usize::try_from(p).ok()?)?;
        self.stream.frames.get(i)
    }

    fn len(&self) -> u64 {
        self.ticks.len() as u64
    }

    fn pts(&self, p: u64) -> Option<i64> {
        self.frame(p)?.pts
    }

    /// The source time just after presentation position `p` (the next PTS, or the last PTS plus
    /// the last frame interval).
    fn end_pts(&self, p: u64) -> Option<i64> {
        if let Some(next) = p.checked_add(1).and_then(|q| self.pts(q)) {
            return Some(next);
        }
        let last = self.pts(p)?;
        let step = match p.checked_sub(1).and_then(|q| self.pts(q)) {
            Some(prev) => last.checked_sub(prev)?,
            None => 0,
        };
        last.checked_add(step)
    }

    fn end_tick(&self, p: u64) -> Option<i64> {
        Some(MediaTime::new(self.end_pts(p)?, self.stream.timebase).to_tick()?.0)
    }

    /// The pictures shown during `[in_point, out_point)`.
    fn resolve(&self, in_point: Tick, out_point: Tick) -> Result<Span, PlanRefusal> {
        if out_point.0 <= in_point.0 {
            return Err(PlanRefusal::EmptyRange);
        }
        let last = self.len().saturating_sub(1);
        let (Some(&start), Some(end)) = (self.ticks.first(), self.end_tick(last)) else { return Err(PlanRefusal::EmptyStream) };
        if in_point.0 < start || out_point.0 > end {
            return Err(PlanRefusal::OutOfRange);
        }
        // the picture on screen at in_point; the last one starting before out_point
        let first = self.ticks.partition_point(|&t| t <= in_point.0).saturating_sub(1) as u64;
        let last = self.ticks.partition_point(|&t| t < out_point.0).saturating_sub(1) as u64;
        if last < first {
            return Err(PlanRefusal::EmptyRange);
        }
        Ok(Span { first, last })
    }
}

/// A copyable span: decode positions `decode` output exactly presentation positions
/// `presentation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CopySpan {
    decode: Span,
    presentation: Span,
}

/// A picture may sit inside a copied span.
fn copyable(f: &FrameIndexEntry) -> bool {
    matches!(f.dependency_status, DependencyStatus::Independent | DependencyStatus::WithinGop)
}

/// The copy spans starting at the clean access point at decode position `start`, shortest first,
/// while the pictures stay copyable within one codec configuration. Stops after the first span
/// that ends past presentation position `until` (a longer one cannot be wanted), or at decode
/// position `stop` (exclusive).
fn copy_spans(stream: &StreamIndex, start: usize, until: u64, stop: usize) -> Vec<CopySpan> {
    let mut out = Vec::new();
    let Some(head) = stream.frames.get(start) else { return out };
    if !head.random_access.is_clean() {
        return out;
    }
    let (mut lo, mut hi, mut count) = (u64::MAX, 0u64, 0u64);
    for f in stream.frames.iter().take(stop).skip(start) {
        let Some(p) = f.presentation_index else { break };
        if !copyable(f) || f.config_id != head.config_id {
            break;
        }
        lo = lo.min(p);
        hi = hi.max(p);
        count = count.saturating_add(1);
        if hi.checked_sub(lo).and_then(|d| d.checked_add(1)) == Some(count) {
            out.push(CopySpan { decode: Span { first: head.decode_index, last: f.decode_index }, presentation: Span { first: lo, last: hi } });
        }
        if out.last().is_some_and(|s| s.presentation.last > until) {
            break;
        }
    }
    out
}

/// Decode positions of the clean access points.
fn clean_starts(stream: &StreamIndex) -> Vec<usize> {
    stream.frames.iter().enumerate().filter(|(_, f)| f.random_access.is_clean()).map(|(i, _)| i).collect()
}

/// Where a copy from clean access point `start` begins in presentation order: with every leading
/// picture of its GOP (those shown before it), when they can be copied.
fn safe_in(stream: &StreamIndex, start: usize, starts: &[usize]) -> Option<u64> {
    let next = starts.iter().copied().find(|&s| s > start).unwrap_or(stream.frames.len());
    copy_spans(stream, start, u64::MAX, next).iter().map(|s| s.presentation.first).min()
}

/// The latest clean access point at or before decode position `decode`.
fn start_before(starts: &[usize], stream: &StreamIndex, decode: u64) -> Option<usize> {
    starts.iter().copied().filter(|&s| stream.frames.get(s).is_some_and(|f| f.decode_index <= decode)).max()
}

struct Planner<'a> {
    timeline: Timeline<'a>,
    timebase: Rational,
}

impl Planner<'_> {
    fn point(&self, p: u64, end: bool) -> Option<SafePoint> {
        let value = if end { self.timeline.end_pts(p)? } else { self.timeline.pts(p)? };
        Some(SafePoint { presentation: p, time: MediaTime::new(value, self.timebase) })
    }

    /// The safe cut-ins (with their access points) around `wanted`.
    fn safe_points(&self, wanted: Span) -> SafePoints {
        let stream = self.timeline.stream;
        let mut before: Option<(usize, u64)> = None;
        let mut after: Option<(usize, u64)> = None;
        let starts = clean_starts(stream);
        for &start in &starts {
            let Some(p) = safe_in(stream, start, &starts) else { continue };
            if p <= wanted.first {
                if before.is_none_or(|(_, b)| p >= b) {
                    before = Some((start, p));
                }
            } else if after.is_none_or(|(_, a)| p < a) {
                after = Some((start, p));
            }
        }
        let mut safe = SafePoints {
            in_before: before.and_then(|(_, p)| self.point(p, false)),
            in_after: after.and_then(|(_, p)| self.point(p, false)),
            ..SafePoints::default()
        };
        if let Some((start, _)) = before.or(after) {
            let spans = copy_spans(stream, start, wanted.last, stream.frames.len());
            safe.out_before = spans.iter().rev().find(|s| s.presentation.last <= wanted.last).and_then(|s| self.point(s.presentation.last, true));
            safe.out_after = spans.iter().find(|s| s.presentation.last > wanted.last).and_then(|s| self.point(s.presentation.last, true));
        }
        safe
    }

    /// The copy span outputting exactly `wanted`, if there is one.
    fn exact_copy(&self, wanted: Span) -> Option<CopySpan> {
        let stream = self.timeline.stream;
        let first = self.timeline.frame(wanted.first)?;
        // the copy must start at the GOP the first picture belongs to
        let start = start_before(&clean_starts(stream), stream, first.decode_index)?;
        copy_spans(stream, start, wanted.last, stream.frames.len()).into_iter().find(|s| s.presentation == wanted)
    }

    /// The longest copy span inside `wanted`. Each clean start inside the range is tried once,
    /// except those already inside a span found (they can only give a part of it).
    fn inner_copy(&self, wanted: Span) -> Option<CopySpan> {
        let stream = self.timeline.stream;
        let mut best: Option<CopySpan> = None;
        let mut covered: Option<u64> = None;
        for start in clean_starts(stream) {
            let Some(head) = stream.frames.get(start) else { continue };
            if covered.is_some_and(|last| head.decode_index <= last) {
                continue;
            }
            // every span from here begins at or before the access point's own picture
            let Some(own) = head.presentation_index else { continue };
            if own < wanted.first || own > wanted.last {
                continue;
            }
            let found = copy_spans(stream, start, wanted.last, stream.frames.len())
                .into_iter()
                .filter(|s| s.presentation.first >= wanted.first && s.presentation.last <= wanted.last)
                .max_by_key(|s| s.presentation.count());
            if let Some(s) = found {
                covered = Some(s.decode.last);
                if best.is_none_or(|b| s.presentation.count() > b.presentation.count()) {
                    best = Some(s);
                }
            }
        }
        best
    }

    /// A re-encoded segment for the pictures `wanted`: decoded from the latest clean access point
    /// before them (in decode order).
    fn reencode_span(&self, wanted: Span) -> Result<Span, PlanRefusal> {
        let stream = self.timeline.stream;
        let undecodable = PlanRefusal::UndecodablePictures { first: wanted.first, last: wanted.last };
        let mut lowest = u64::MAX;
        let mut highest = 0u64;
        for p in wanted.first..=wanted.last {
            let f = self.timeline.frame(p).ok_or(undecodable.clone())?;
            if matches!(f.dependency_status, DependencyStatus::Undecodable | DependencyStatus::Unknown) {
                return Err(undecodable);
            }
            lowest = lowest.min(f.decode_index);
            highest = highest.max(f.decode_index);
        }
        // the pictures it needs come after a clean start; the latest one at or before them
        let start = stream.frames.iter().filter(|f| f.random_access.is_clean() && f.decode_index <= lowest).map(|f| f.decode_index).max().ok_or(undecodable)?;
        Ok(Span { first: start, last: highest })
    }

    fn segment(&self, stream_id: u32, mode: SegmentMode, reason: SegmentReason, presentation: Span, decode: Span, origin: i64) -> Option<PlanSegment> {
        let source_in = self.timeline.pts(presentation.first)?;
        let source_out = self.timeline.end_pts(presentation.last)?;
        Some(PlanSegment {
            stream_id,
            mode,
            reason,
            presentation,
            decode,
            source_in: MediaTime::new(source_in, self.timebase),
            source_out: MediaTime::new(source_out, self.timebase),
            output_in: MediaTime::new(source_in.checked_sub(origin)?, self.timebase),
        })
    }
}

/// How the codec's packets are framed in a container, when it is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    /// Start codes (Annex B), as in MPEG-TS / PS.
    AnnexB,
    /// Length prefixes (`avcC` / `hvcC`), as in MP4, QuickTime and Matroska.
    LengthPrefixed,
    /// The same elementary stream in every container (MPEG-2 video).
    Elementary,
}

fn framing(codec: Codec, container: Container) -> Option<Framing> {
    match codec {
        Codec::Mpeg2Video => Some(Framing::Elementary),
        Codec::H264 | Codec::Hevc => match container {
            Container::MpegTs | Container::MpegPs => Some(Framing::AnnexB),
            Container::Mp4 | Container::Mov | Container::Matroska | Container::WebM => Some(Framing::LengthPrefixed),
            Container::Mxf => None,
        },
        Codec::Av1 | Codec::Aac | Codec::Pcm => None,
    }
}

/// Why a copy under `guarantee` cannot keep the payload, if it cannot.
///
/// A bit-exact copy keeps the payload bytes, so the framing must be the same in both containers;
/// a compatible copy may change it, but only through a proven lossless conversion (SCE-001
/// §2.2, §11), and none is registered yet (the native bitstream adapters come with phase P3).
fn guarantee_blocker(codec: Codec, source: Container, output: Container, guarantee: CopyGuarantee) -> Option<String> {
    let (c, s, o) = (codec.label(), source.label(), output.label());
    match (framing(codec, source), framing(codec, output)) {
        (Some(a), Some(b)) if a == b => None,
        (Some(_), Some(_)) => Some(match guarantee {
            CopyGuarantee::BitExact => {
                format!("bit-exact copy: {c} from {s} into {o} changes the packet framing; only the compatible copy guarantee allows that")
            }
            CopyGuarantee::Compatible => format!("compatible copy: no proven lossless framing conversion of {c} from {s} into {o} is registered yet"),
        }),
        _ => Some(format!("copy: the packet framing of {c} in {s} and {o} is not known to match")),
    }
}

/// Plan a Smart Cut of `request` out of the indexed source. Reads nothing but its arguments.
pub fn plan_smart_cut(registry: &Registry, index: &MediaIndex, request: &SmartCutRequest, policy: Policy) -> Result<SmartCutPlan, PlanRefusal> {
    let stream = index.streams.iter().find(|s| s.stream_id == request.stream_id).ok_or(PlanRefusal::UnknownStream { stream_id: request.stream_id })?;
    let timeline = Timeline::new(stream)?;
    let wanted = timeline.resolve(request.in_point, request.out_point)?;
    let planner = Planner { timeline, timebase: stream.timebase };
    let safe_points = planner.safe_points(wanted);
    let origin = planner.timeline.pts(wanted.first).ok_or(PlanRefusal::EmptyRange)?;
    let copy_mode = request.guarantee.segment_mode();
    let sid = request.stream_id;
    let invalid = PlanRefusal::InvalidTimestamps { decode_index: 0 };

    let mut segments = Vec::new();
    if let Some(span) = planner.exact_copy(wanted) {
        segments.push(planner.segment(sid, copy_mode, SegmentReason::WholeGops, span.presentation, span.decode, origin).ok_or(invalid.clone())?);
    } else {
        if request.mode == CutMode::GopBoundary {
            return Err(PlanRefusal::NotAtSafePoints { first: wanted.first, last: wanted.last, safe: Box::new(safe_points) });
        }
        match planner.inner_copy(wanted) {
            Some(core) => {
                if core.presentation.first > wanted.first {
                    let head = Span { first: wanted.first, last: core.presentation.first.saturating_sub(1) };
                    let decode = planner.reencode_span(head)?;
                    segments
                        .push(planner.segment(sid, SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary, head, decode, origin).ok_or(invalid.clone())?);
                }
                segments.push(planner.segment(sid, copy_mode, SegmentReason::WholeGops, core.presentation, core.decode, origin).ok_or(invalid.clone())?);
                if core.presentation.last < wanted.last {
                    let tail = Span { first: core.presentation.last.saturating_add(1), last: wanted.last };
                    let decode = planner.reencode_span(tail)?;
                    segments
                        .push(planner.segment(sid, SegmentMode::BoundaryReencode, SegmentReason::TailBoundary, tail, decode, origin).ok_or(invalid.clone())?);
                }
            }
            None => {
                let decode = planner.reencode_span(wanted)?;
                segments
                    .push(planner.segment(sid, SegmentMode::BoundaryReencode, SegmentReason::NoCopyableRange, wanted, decode, origin).ok_or(invalid.clone())?);
            }
        }
    }

    let end = planner.timeline.end_pts(wanted.last).ok_or(invalid.clone())?;
    let plan = ExportExecutionPlan {
        plan_version: PLAN_VERSION,
        source_id: index.source_id.clone(),
        codec: stream.codec,
        source_container: request.source_container,
        output_container: request.output_container,
        guarantee: request.guarantee,
        segments,
        expected_duration: MediaTime::new(end.checked_sub(origin).ok_or(invalid)?, stream.timebase),
        requires_validation: true,
    };
    let blockers = blockers(registry, &plan, request, policy);
    Ok(SmartCutPlan { executable: blockers.is_empty(), blockers, plan, safe_points })
}

/// What keeps `plan` from running now, in a stable order without repeats.
fn blockers(registry: &Registry, plan: &ExportExecutionPlan, request: &SmartCutRequest, policy: Policy) -> Vec<String> {
    let (codec, source, output) = (plan.codec, request.source_container, request.output_container);
    let mut out: Vec<String> = Vec::new();
    let mut add = |lines: Vec<String>| {
        for l in lines {
            if !out.contains(&l) {
                out.push(l);
            }
        }
    };
    if plan.segments.iter().any(|s| s.mode.is_copy()) {
        add(registry.smart_cut(codec, source, output, policy).blockers());
        add(guarantee_blocker(codec, source, output, plan.guarantee).into_iter().collect());
    }
    if plan.requires_confirmation() {
        match &request.format {
            Some(format) => add(registry.smart_render(codec, source, output, format, policy).blockers()),
            None => add(vec![format!("boundary re-encode: the {} stream format (profile, chroma, bit depth) is unknown", codec.label())]),
        }
    }
    out
}
