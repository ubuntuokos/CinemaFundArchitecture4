//! Smart Render planner (WP-05): the [`ExportExecutionPlan`] of a whole timeline, made of copied
//! source runs, re-encoded boundaries and rendered ranges (CFA4-SCE-001 v1.5 §8; the timeline
//! hybrid of `docs/plans/SMART-RENDER-2026-10-27.md`).
//!
//! The planner takes the [`EditDecisionList`] of a timeline and the WP-02 index of every source,
//! and sorts each output range into one of three kinds:
//!
//! - **Render:** a gap, or a clip whose pictures the timeline changes (effect, transition,
//!   overlay, transform, speed, frame rate, colour…). Nothing of it is copied.
//! - **Copy, with re-encoded boundaries:** a clip shown as it is. It is planned like a Smart Cut
//!   in frame-accurate mode: the longest run that starts at a clean access point and ends where
//!   decode and presentation order meet is copied, the pictures before and after it are
//!   re-encoded from the latest clean access point before them (the extra pictures decoded only
//!   as references are the segment's `decode` span).
//! - **Re-encode for the seam:** a run that could be copied but whose stream does not match the
//!   output stream (another codec, another codec configuration, or an unknown one). The output
//!   is one stream, so such a run is re-encoded rather than copied.
//!
//! Clips that continue each other in the same source (`A[0, 2 s)` then `A[2 s, 4 s)`) are joined
//! before planning, so a split without a change costs no re-encode. A clip whose planned pictures
//! do not fill its output range exactly (another source frame rate) is rendered, so no frame is
//! duplicated or lost.
//!
//! Times: the plan's output times and expected duration are on the tick timebase (one unit per
//! [`Tick`]); the source times of copied and re-encoded segments stay on their stream's timebase.
//! For a `Render` segment the `presentation` and `decode` spans are the output frame positions it
//! fills.

use serde::{Deserialize, Serialize};

use cfa4_time::{TICKS_PER_SECOND, Tick};

use crate::capabilities::{Codec, Container, Key, Operation, Policy, Registry};
use crate::copy_planner::{CutMode, PlanRefusal, SmartCutRequest, guarantee_blocker, plan_smart_cut};
use crate::edit_decision::{EditClip, EditContent, EditDecisionList, RenderCause};
use crate::media_index::{MediaIndex, StreamIndex};
use crate::plan::{CopyGuarantee, ExportExecutionPlan, PLAN_VERSION, PlanSegment, SegmentMode, SegmentReason, SegmentSource, Span};
use crate::timestamp::{MediaTime, Rational};

/// The timebase of the plan's output times: one unit per [`Tick`].
pub const TICK_TIMEBASE: Rational = Rational { num: 1, den: TICKS_PER_SECOND };

/// What the export writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartRenderRequest {
    /// Codec of the output video stream.
    pub codec: Codec,
    pub output_container: Container,
    pub guarantee: CopyGuarantee,
    /// The configuration identifier (as in [`EditClip::configs`]) the output stream must have, if
    /// the export settings fix one; otherwise the first copied run sets it.
    #[serde(default)]
    pub output_config: Option<String>,
}

/// Why no plan can be made for the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "reason")]
pub enum RenderRefusal {
    /// The timeline has no items.
    EmptyTimeline,
    /// The frame duration is not positive, or not a whole number of ticks.
    InvalidFrameRate { frame_duration: Rational },
    /// The item is empty or does not start and end on the frame grid.
    OffGrid { item: u32 },
    /// The item starts before the previous one ends.
    Overlap { item: u32 },
    /// No index was given for the clip's source.
    UnknownSource { item: u32, source_id: String },
    /// The source index has no such stream.
    UnknownStream { item: u32, stream_id: u32 },
    /// The clip's source range cannot be planned.
    Clip { item: u32, refusal: PlanRefusal },
    /// A time does not fit the tick range.
    Overflow { item: u32 },
}

impl std::fmt::Display for RenderRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderRefusal::EmptyTimeline => write!(f, "the timeline is empty"),
            RenderRefusal::InvalidFrameRate { frame_duration: d } => write!(f, "the frame duration {}/{} s is not a whole number of ticks", d.num, d.den),
            RenderRefusal::OffGrid { item } => write!(f, "item {item} is empty or not on the frame grid"),
            RenderRefusal::Overlap { item } => write!(f, "item {item} overlaps the item before it"),
            RenderRefusal::UnknownSource { item, source_id } => write!(f, "item {item}: no index for source {source_id}"),
            RenderRefusal::UnknownStream { item, stream_id } => write!(f, "item {item}: the source has no stream {stream_id}"),
            RenderRefusal::Clip { item, refusal } => write!(f, "item {item}: {refusal}"),
            RenderRefusal::Overflow { item } => write!(f, "item {item}: a time is out of range"),
        }
    }
}

impl std::error::Error for RenderRefusal {}

/// A timeline plan, with whether it can run now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartRenderPlan {
    pub plan: ExportExecutionPlan,
    /// The registry allows every operation the plan needs.
    pub executable: bool,
    /// What keeps the plan from running, in a stable order without repeats.
    pub blockers: Vec<String>,
}

/// One output range after gaps are filled and continuing clips joined.
struct Part<'a> {
    item: u32,
    output_in: i64,
    duration: i64,
    kind: PartKind<'a>,
}

enum PartKind<'a> {
    Gap,
    Changed(&'a EditClip, Vec<RenderCause>),
    AsIs(&'a EditClip),
}

/// Whether `next` shows the pictures right after `prev` from the same source, unchanged.
fn continues(prev: &Part<'_>, next: &Part<'_>) -> bool {
    let (PartKind::AsIs(a), PartKind::AsIs(b)) = (&prev.kind, &next.kind) else { return false };
    let (Some(prev_end), Some(prev_src_end)) = (prev.output_in.checked_add(prev.duration), a.source_in.0.checked_add(prev.duration)) else {
        return false;
    };
    prev_end == next.output_in
        && prev_src_end == b.source_in.0
        && a.source_id == b.source_id
        && a.stream_id == b.stream_id
        && a.container == b.container
        && a.configs == b.configs
        && a.format == b.format
}

/// The output ranges of `edl`, in order, with the holes as gaps.
fn parts(edl: &EditDecisionList, frame: i64) -> Result<Vec<Part<'_>>, RenderRefusal> {
    let mut out: Vec<Part<'_>> = Vec::new();
    let mut cursor = 0i64;
    for (i, it) in edl.items.iter().enumerate() {
        let item = u32::try_from(i).map_err(|_| RenderRefusal::Overflow { item: u32::MAX })?;
        let (start, duration) = (it.output_in.0, it.duration.0);
        if start < 0 || duration <= 0 || start % frame != 0 || duration % frame != 0 {
            return Err(RenderRefusal::OffGrid { item });
        }
        if start < cursor {
            return Err(RenderRefusal::Overlap { item });
        }
        if start > cursor {
            out.push(Part { item, output_in: cursor, duration: start - cursor, kind: PartKind::Gap });
        }
        let kind = match &it.content {
            EditContent::Gap => PartKind::Gap,
            EditContent::Clip(c) if it.render_causes.is_empty() => PartKind::AsIs(c),
            EditContent::Clip(c) => {
                let mut causes = it.render_causes.clone();
                causes.sort();
                causes.dedup();
                PartKind::Changed(c, causes)
            }
        };
        let part = Part { item, output_in: start, duration, kind };
        match out.last_mut() {
            Some(prev) if continues(prev, &part) => prev.duration = prev.duration.checked_add(duration).ok_or(RenderRefusal::Overflow { item })?,
            _ => out.push(part),
        }
        cursor = it.output_out().ok_or(RenderRefusal::Overflow { item })?.0;
    }
    Ok(out)
}

/// Ticks per output frame.
fn frame_ticks(d: Rational) -> Option<i64> {
    if !d.is_valid() {
        return None;
    }
    let n = i128::from(TICKS_PER_SECOND).checked_mul(i128::from(d.num))?;
    let den = i128::from(d.den);
    (n % den == 0).then(|| i64::try_from(n / den).ok()).flatten().filter(|&t| t > 0)
}

fn ticks(t: i64) -> MediaTime {
    MediaTime::new(t, TICK_TIMEBASE)
}

/// A `Render` segment over output frames.
fn render_segment(part: &Part<'_>, frame: i64, reason: SegmentReason, causes: Vec<RenderCause>) -> Result<PlanSegment, RenderRefusal> {
    let overflow = RenderRefusal::Overflow { item: part.item };
    let first = u64::try_from(part.output_in / frame).map_err(|_| overflow.clone())?;
    let count = u64::try_from(part.duration / frame).map_err(|_| overflow.clone())?;
    let span = Span { first, last: first.checked_add(count.saturating_sub(1)).ok_or(overflow.clone())? };
    let (stream_id, source, source_in) = match &part.kind {
        PartKind::Gap => (0, None, part.output_in),
        PartKind::Changed(c, _) | PartKind::AsIs(c) => {
            (c.stream_id, Some(SegmentSource { source_id: c.source_id.clone(), container: c.container, item: part.item }), c.source_in.0)
        }
    };
    let source_out = source_in.checked_add(part.duration).ok_or(overflow)?;
    Ok(PlanSegment {
        stream_id,
        mode: SegmentMode::Render,
        reason,
        presentation: span,
        decode: span,
        source_in: ticks(source_in),
        source_out: ticks(source_out),
        output_in: ticks(part.output_in),
        source,
        render_causes: causes,
    })
}

/// The configuration identifier of the pictures a segment reads.
fn segment_config<'a>(stream: &StreamIndex, clip: &'a EditClip, s: &PlanSegment) -> Option<&'a String> {
    let i = stream.frames.binary_search_by_key(&s.decode.first, |f| f.decode_index).ok()?;
    let id = stream.frames.get(i)?.config_id;
    clip.configs.get(usize::try_from(id).ok()?)
}

/// Collects blocker lines without repeats.
#[derive(Default)]
struct Blockers(Vec<String>);

impl Blockers {
    fn add(&mut self, lines: impl IntoIterator<Item = String>) {
        for l in lines {
            if !self.0.contains(&l) {
                self.0.push(l);
            }
        }
    }
}

/// Plan the export of a timeline. `indexes` holds the index of every source a clip names.
pub fn plan_smart_render(
    registry: &Registry,
    indexes: &[MediaIndex],
    edl: &EditDecisionList,
    request: &SmartRenderRequest,
    policy: Policy,
) -> Result<SmartRenderPlan, RenderRefusal> {
    let frame = frame_ticks(edl.frame_duration).ok_or(RenderRefusal::InvalidFrameRate { frame_duration: edl.frame_duration })?;
    if edl.items.is_empty() {
        return Err(RenderRefusal::EmptyTimeline);
    }
    let parts = parts(edl, frame)?;
    let mut segments: Vec<PlanSegment> = Vec::new();
    let mut blockers = Blockers::default();
    let mut output_config = request.output_config.clone();
    let mut rendered = false;
    let mut first_container = None;

    for part in &parts {
        let clip = match &part.kind {
            PartKind::Gap => {
                segments.push(render_segment(part, frame, SegmentReason::Gap, Vec::new())?);
                rendered = true;
                continue;
            }
            PartKind::Changed(c, causes) => {
                first_container.get_or_insert(c.container);
                segments.push(render_segment(part, frame, SegmentReason::Transformed, causes.clone())?);
                rendered = true;
                continue;
            }
            PartKind::AsIs(c) => c,
        };
        first_container.get_or_insert(clip.container);
        let item = part.item;
        let index =
            indexes.iter().find(|m| m.source_id == clip.source_id).ok_or_else(|| RenderRefusal::UnknownSource { item, source_id: clip.source_id.clone() })?;
        let stream = index.streams.iter().find(|s| s.stream_id == clip.stream_id).ok_or(RenderRefusal::UnknownStream { item, stream_id: clip.stream_id })?;
        if stream.codec != request.codec {
            segments.push(render_segment(part, frame, SegmentReason::IncompatibleStream, Vec::new())?);
            rendered = true;
            continue;
        }
        let cut = SmartCutRequest {
            stream_id: clip.stream_id,
            in_point: clip.source_in,
            out_point: Tick(clip.source_in.0.checked_add(part.duration).ok_or(RenderRefusal::Overflow { item })?),
            mode: CutMode::FrameAccurate,
            guarantee: request.guarantee,
            source_container: clip.container,
            output_container: request.output_container,
            format: clip.format.clone(),
        };
        let planned = plan_smart_cut(registry, index, &cut, policy).map_err(|refusal| RenderRefusal::Clip { item, refusal })?;
        // the planned pictures must fill the item exactly at the output frame rate
        let frames: u64 = planned.plan.segments.iter().map(PlanSegment::frames).fold(0, u64::saturating_add);
        if i64::try_from(frames).ok().and_then(|n| n.checked_mul(frame)) != Some(part.duration) {
            segments.push(render_segment(part, frame, SegmentReason::Transformed, vec![RenderCause::FrameRate])?);
            rendered = true;
            continue;
        }
        let (mut copied, mut reencoded) = (false, false);
        for mut s in planned.plan.segments {
            if s.mode.is_copy() {
                let config = segment_config(stream, clip, &s);
                let fits = match (&output_config, config) {
                    (_, None) => false,
                    (None, Some(c)) => {
                        output_config = Some(c.clone());
                        true
                    }
                    (Some(o), Some(c)) => o == c,
                };
                if !fits {
                    s.mode = SegmentMode::BoundaryReencode;
                    s.reason = SegmentReason::IncompatibleStream;
                }
            }
            copied |= s.mode.is_copy();
            reencoded |= !s.mode.is_copy();
            let offset = s.output_in.to_tick().ok_or(RenderRefusal::Overflow { item })?;
            s.output_in = ticks(part.output_in.checked_add(offset.0).ok_or(RenderRefusal::Overflow { item })?);
            s.source = Some(SegmentSource { source_id: clip.source_id.clone(), container: clip.container, item });
            segments.push(s);
        }
        let (codec, source, output) = (request.codec, clip.container, request.output_container);
        if copied {
            blockers.add(registry.smart_cut(codec, source, output, policy).blockers());
            blockers.add(guarantee_blocker(codec, source, output, request.guarantee));
        }
        if reencoded {
            match &clip.format {
                Some(format) => blockers.add(registry.smart_render(codec, source, output, format, policy).blockers()),
                None => blockers.add([format!("item {item}: boundary re-encode: the {} stream format (profile, chroma, bit depth) is unknown", codec.label())]),
            }
        }
    }
    if rendered {
        for key in [Key::codec(Operation::Encode, request.codec), Key::in_container(Operation::Mux, request.codec, request.output_container)] {
            let d = registry.decide(key, policy);
            if !d.allowed {
                blockers.add(d.reasons);
            }
        }
    }
    let end = parts.last().and_then(|p| p.output_in.checked_add(p.duration)).ok_or(RenderRefusal::EmptyTimeline)?;
    let plan = ExportExecutionPlan {
        plan_version: PLAN_VERSION,
        source_id: edl.timeline_id.clone(),
        codec: request.codec,
        source_container: first_container.unwrap_or(request.output_container),
        output_container: request.output_container,
        guarantee: request.guarantee,
        segments,
        expected_duration: ticks(end),
        requires_validation: true,
    };
    let blockers = blockers.0;
    Ok(SmartRenderPlan { executable: blockers.is_empty(), blockers, plan })
}
