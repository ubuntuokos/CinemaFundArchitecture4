//! The timeline as the Smart Render planner sees it (CFA4-SCE-001 v1.5, `edit_decision.rs`): an
//! ordered list of output time ranges, each filled by one source clip or by nothing (a gap), with
//! the reasons the timeline changes its pictures.
//!
//! The engine builds this list from a project snapshot; the planner never reads the project. The
//! list says only what the planner needs to decide copy against render: which source range each
//! output range shows and whether anything (effect, transition, title, speed, scaling, frame rate
//! or colour conversion…) changes those pictures. Every time is a [`Tick`].

use serde::{Deserialize, Serialize};

use cfa4_time::Tick;

use crate::capabilities::{Container, StreamFormat};
use crate::timestamp::Rational;

/// Why the timeline changes the pictures of an item, so they must be rendered (SMART-RENDER
/// plan, "Render-required").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderCause {
    /// A video effect, or a colour correction.
    Effect,
    /// A transition touches the range.
    Transition,
    /// Other tracks, alpha or a blend mode are composited over or under the clip.
    Composite,
    /// A title, caption burn-in or export overlay.
    Overlay,
    /// Position, scale, rotation, crop, or a frame size different from the output.
    Transform,
    /// Speed, reverse, freeze frame or time remapping.
    Speed,
    /// The source frame rate differs from the output frame rate.
    FrameRate,
    /// A colour space, transfer, range or bit depth conversion.
    ColorConversion,
    /// The clip is shown through a proxy or a nested sequence.
    Indirect,
    /// The pictures do not come from a stream the export can copy: a still, a generator, a
    /// graphic, an adjustment layer, or a file in another codec or container.
    Source,
}

/// A source clip on the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditClip {
    /// The source file, as in [`crate::media_index::MediaIndex::source_id`].
    pub source_id: String,
    /// The video stream of the source.
    pub stream_id: u32,
    pub container: Container,
    /// Source time (on the stream's PTS timeline) shown at the item's `output_in`.
    pub source_in: Tick,
    /// An identifier of each codec configuration of the stream, by
    /// [`crate::media_index::FrameIndexEntry::config_id`] (for example a hash of the `avcC` or
    /// the sequence header). Two copied runs can only share one output stream when these are
    /// equal; an unknown configuration is never copied.
    #[serde(default)]
    pub configs: Vec<String>,
    /// Profile, chroma and bit depth of the stream, for the encoder that re-encodes its boundaries.
    #[serde(default)]
    pub format: Option<StreamFormat>,
}

/// What an item shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum EditContent {
    Clip(EditClip),
    /// Black (and silence).
    Gap,
}

/// One output time range of the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditItem {
    /// Where the item starts in the output, from 0.
    pub output_in: Tick,
    pub duration: Tick,
    pub content: EditContent,
    /// What changes the pictures of this item; empty when the source pictures are shown as they are.
    #[serde(default)]
    pub render_causes: Vec<RenderCause>,
}

impl EditItem {
    /// Where the item ends in the output (exclusive); `None` on overflow.
    pub fn output_out(&self) -> Option<Tick> {
        self.output_in.0.checked_add(self.duration.0).map(Tick)
    }
}

/// The video of a timeline, in output order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditDecisionList {
    /// The timeline (sequence) the list was made from.
    pub timeline_id: String,
    /// Seconds per output frame (`1/25`, `1001/30000`…). Every item starts and ends on this grid.
    pub frame_duration: Rational,
    /// Sorted by `output_in`, without overlaps; a hole between items is a gap.
    pub items: Vec<EditItem>,
}
