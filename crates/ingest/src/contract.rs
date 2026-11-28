//! The common recording contract (LIVE-STUDIO-001 annex X, DECKLINK-READY R2).

use std::path::PathBuf;

use cfa4_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};

use crate::error::IngestError;
use crate::extent::ExtentView;

/// Upper bounds: a job comes from a UI, a CLI or an agent, so it is checked like any hostile input.
pub const MAX_TRACKS: usize = 64;
pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_CHANNELS: u16 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Container {
    Mkv,
    Mp4,
    FragmentedMp4,
    Mov,
    Mxf,
}

impl Container {
    /// A container an editor can read while it is still being written (first IN-2 round).
    pub fn growable(self) -> bool {
        matches!(self, Container::Mkv | Container::FragmentedMp4)
    }
}

/// Stand-in for File Services' `StagedFile` (FS-1 does not exist yet).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedTarget {
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoTrackSpec {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrackSpec {
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SplitRule {
    BySize(u64),
    ByDuration(Tick),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordJob {
    pub target: StagedTarget,
    pub container: Container,
    pub video: Vec<VideoTrackSpec>,
    pub audio: Vec<AudioTrackSpec>,
    pub split: Option<SplitRule>,
    /// Start time code as an offset from midnight (display only; all maths is in ticks).
    pub start_timecode: Option<Tick>,
}

impl RecordJob {
    pub fn validate(&self) -> Result<(), IngestError> {
        let bad = |m: &str| Err(IngestError::InvalidJob(m.to_string()));
        if self.video.is_empty() && self.audio.is_empty() {
            return bad("no tracks");
        }
        if self.video.len() > MAX_TRACKS || self.audio.len() > MAX_TRACKS {
            return bad("too many tracks");
        }
        for v in &self.video {
            if v.width == 0 || v.height == 0 || v.width > MAX_DIMENSION || v.height > MAX_DIMENSION {
                return bad("video size out of range");
            }
            if v.frame_rate.num <= 0 || v.frame_rate.den <= 0 {
                return bad("frame rate must be positive");
            }
            if v.codec.is_empty() {
                return bad("empty video codec");
            }
        }
        for a in &self.audio {
            if a.channels == 0 || a.channels > MAX_CHANNELS || a.sample_rate == 0 || a.codec.is_empty() {
                return bad("audio track out of range");
            }
        }
        match self.split {
            Some(SplitRule::BySize(0)) => return bad("split size is zero"),
            Some(SplitRule::ByDuration(d)) if d <= Tick::ZERO => return bad("split duration must be positive"),
            _ => {}
        }
        if matches!(self.start_timecode, Some(t) if t < Tick::ZERO) {
            return bad("negative start time code");
        }
        if self.target.path.as_os_str().is_empty() {
            return bad("empty target");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordHandle(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataKind {
    Timecode,
    Markers,
    ColorInfo,
    AudioLayout,
    Other(String),
}

/// What a finished recording says about itself. `dropped` is the measurable proof of "no dropped
/// frames"; `lost` makes a metadata kind the container cannot keep visible (no silent loss).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordReport {
    /// File identifiers (File Services `FileId`s once FS-1 exists).
    pub files: Vec<String>,
    pub frames: u64,
    pub dropped: u64,
    pub preserved: Vec<MetadataKind>,
    pub lost: Vec<(MetadataKind, String)>,
}

/// A recording backend (libobs, DeckLink, ...). `extent` is the read-only view the editor uses.
pub trait Recorder: Send {
    fn start(&mut self, job: RecordJob) -> Result<RecordHandle, IngestError>;
    fn marker(&mut self, handle: RecordHandle, at: Tick, label: &str) -> Result<(), IngestError>;
    fn stop(&mut self, handle: RecordHandle) -> Result<RecordReport, IngestError>;
    fn extent(&self, handle: RecordHandle) -> Result<ExtentView, IngestError>;
}
