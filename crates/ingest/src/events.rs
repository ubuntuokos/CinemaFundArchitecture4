//! Typed events of a recording (TV plan section 6, recorder part). The names are **provisional**:
//! the Event Fabric subject catalogue owner confirms them; until the Event Fabric exists the engine
//! publishes these as its own in-process events.

use cfa4_time::Tick;
use serde::{Deserialize, Serialize};

use crate::session::State;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventKind {
    RecordingStarted,
    RecordingStopped,
    RecordingInterrupted {
        reason: String,
    },
    SourceLost {
        source: String,
    },
    SourceRecovered {
        source: String,
    },
    GrowingMediaUpdated {
        frames: u64,
        available: Tick,
    },
    MediaFinalized,
    IntegrityCheckCompleted {
        ok: bool,
    },
    MediaQuarantined {
        reason: String,
    },
    /// A transition without a catalogue event of its own (for example `Starting`, `Recoverable`).
    StateChanged {
        to: State,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestEvent {
    pub session: u64,
    pub seq: u64,
    pub at: Tick,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl IngestEvent {
    /// Provisional subject name.
    pub fn name(&self) -> &'static str {
        match self.kind {
            EventKind::RecordingStarted => "ingest.recording.started",
            EventKind::RecordingStopped => "ingest.recording.stopped",
            EventKind::RecordingInterrupted { .. } => "ingest.recording.interrupted",
            EventKind::SourceLost { .. } => "ingest.source.lost",
            EventKind::SourceRecovered { .. } => "ingest.source.recovered",
            EventKind::GrowingMediaUpdated { .. } => "ingest.media.growing.updated",
            EventKind::MediaFinalized => "ingest.media.finalized",
            EventKind::IntegrityCheckCompleted { .. } => "ingest.media.integrity.completed",
            EventKind::MediaQuarantined { .. } => "ingest.media.quarantined",
            EventKind::StateChanged { .. } => "ingest.session.state.changed",
        }
    }
}
