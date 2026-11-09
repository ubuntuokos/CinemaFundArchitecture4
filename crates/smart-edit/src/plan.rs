//! The common export execution plan (CFA4-SCE-001 v1.5 §8): one plan type for the GOP Smart Cut
//! (one clip, WP-03) and, later, the Smart Render of a timeline (WP-05). Every segment is copied
//! (`StrictCopy`, `CompatibleCopy`), re-encoded at a cut boundary (`BoundaryReencode`) or rendered
//! (`Render`); one executor runs them all (WP-04, WP-06).
//!
//! A plan is data: building one reads no file and writes none, and the same index, registry and
//! request always give the same plan.
//!
//! A plan of one clip names its source in [`ExportExecutionPlan::source_id`]. A timeline plan
//! (Smart Render) names the timeline there, and each segment that reads a source names it in
//! [`PlanSegment::source`].

use serde::{Deserialize, Serialize};

use crate::capabilities::{Codec, Container};
use crate::edit_decision::RenderCause;
use crate::timestamp::MediaTime;

/// Version of the plan data model.
pub const PLAN_VERSION: u32 = 1;

/// How a segment reaches the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentMode {
    /// The compressed packets are copied with unchanged payload bytes (Strict Bit-Exact Stream
    /// Copy, SCE-001 §2.1).
    StrictCopy,
    /// The packets are copied without re-encoding; only a proven lossless representation change
    /// (Annex B ↔ length-prefixed…) is allowed (Compatible Stream Copy, §2.2).
    CompatibleCopy,
    /// Decoded from an access point and re-encoded, so that a cut lands on the requested picture
    /// (határ-újrakódolás).
    BoundaryReencode,
    /// Rendered and encoded in full.
    Render,
}

impl SegmentMode {
    /// The compressed packets reach the output without re-encoding.
    pub fn is_copy(self) -> bool {
        matches!(self, SegmentMode::StrictCopy | SegmentMode::CompatibleCopy)
    }
}

/// Which copy guarantee the user chose (SCE-001 §13.3). There is no automatic switch between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CopyGuarantee {
    /// Copied packets keep their payload bytes and order; otherwise the export is refused.
    BitExact,
    /// Copied packets decode to the same pictures; a proven lossless representation change is
    /// allowed.
    Compatible,
}

impl CopyGuarantee {
    /// The mode of the copied segments under this guarantee.
    pub fn segment_mode(self) -> SegmentMode {
        match self {
            CopyGuarantee::BitExact => SegmentMode::StrictCopy,
            CopyGuarantee::Compatible => SegmentMode::CompatibleCopy,
        }
    }
}

/// Why a segment has its mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentReason {
    /// Starts at a clean access point and ends where decode and presentation order meet: the
    /// packets can be copied unchanged.
    WholeGops,
    /// The pictures from the requested in point up to the first clean copy start.
    HeadBoundary,
    /// The pictures after the last clean copy end up to the requested out point.
    TailBoundary,
    /// The range holds no copyable span (it is shorter than a GOP, or no clean access point lies
    /// inside it): re-encoded as a whole.
    NoCopyableRange,
    /// A timeline gap: nothing to copy, the output is rendered (black and silence).
    Gap,
    /// The timeline changes the pictures (effect, transition, speed…; see
    /// [`PlanSegment::render_causes`]): rendered.
    Transformed,
    /// The pictures could be copied, but their stream does not match the output stream (another
    /// codec configuration, or one that is not known): re-encoded so that the output stays one
    /// stream.
    IncompatibleStream,
}

/// Inclusive span of positions (presentation or decode order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub first: u64,
    pub last: u64,
}

impl Span {
    /// Positions in the span (at least one).
    pub fn count(self) -> u64 {
        self.last.saturating_sub(self.first).saturating_add(1)
    }
}

/// Where a segment of a timeline plan reads its pictures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentSource {
    pub source_id: String,
    pub container: Container,
    /// Index of the timeline item ([`crate::edit_decision::EditItem`]) the segment comes from.
    pub item: u32,
}

/// One segment of a plan, for one stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSegment {
    pub stream_id: u32,
    pub mode: SegmentMode,
    pub reason: SegmentReason,
    /// The pictures the segment puts in the output, by presentation position.
    pub presentation: Span,
    /// The packets the executor reads, in decode order: copied as they are for a copy segment;
    /// decoded from the access point at `first` for a re-encoded one (pictures outside
    /// `presentation` are decoded only as references and not output).
    pub decode: Span,
    /// Source time of the first output picture (its PTS).
    pub source_in: MediaTime,
    /// Source time just after the last output picture (the next picture's PTS, or the last PTS
    /// plus one frame interval at the end of the stream).
    pub source_out: MediaTime,
    /// Where the segment starts in the output, from 0 at the first output picture.
    pub output_in: MediaTime,
    /// The source of a timeline segment; `None` in a plan of one clip (the plan's source) and for
    /// a gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SegmentSource>,
    /// Why a `Render` segment is rendered, as the timeline says.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_causes: Vec<RenderCause>,
}

impl PlanSegment {
    /// Output pictures of the segment.
    pub fn frames(&self) -> u64 {
        self.presentation.count()
    }
}

/// What an export does, segment by segment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportExecutionPlan {
    pub plan_version: u32,
    pub source_id: String,
    pub codec: Codec,
    pub source_container: Container,
    pub output_container: Container,
    pub guarantee: CopyGuarantee,
    /// In output (presentation) order, without gaps or overlaps.
    pub segments: Vec<PlanSegment>,
    pub expected_duration: MediaTime,
    /// The output is checked after writing (copied payloads against the source; SCE-001 §18).
    pub requires_validation: bool,
}

impl ExportExecutionPlan {
    /// Every segment is copied: nothing is re-encoded.
    pub fn is_lossless(&self) -> bool {
        !self.segments.is_empty() && self.segments.iter().all(|s| s.mode.is_copy())
    }

    /// Some segment is re-encoded or rendered: the plan runs only after the user confirms it
    /// (SCE-001 v1.5, decision 2).
    pub fn requires_confirmation(&self) -> bool {
        self.segments.iter().any(|s| !s.mode.is_copy())
    }

    /// Output pictures that are copied.
    pub fn copied_frames(&self) -> u64 {
        self.segments.iter().filter(|s| s.mode.is_copy()).map(PlanSegment::frames).fold(0, u64::saturating_add)
    }

    /// Output pictures that are re-encoded or rendered.
    pub fn reencoded_frames(&self) -> u64 {
        self.segments.iter().filter(|s| !s.mode.is_copy()).map(PlanSegment::frames).fold(0, u64::saturating_add)
    }
}
