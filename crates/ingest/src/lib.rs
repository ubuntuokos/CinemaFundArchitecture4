//! CFA4 shared recorder core (layer L3). See `README.md` and the plan
//! `CFA4-TV-INGEST-RUNDOWN-001-RESZLETES-TERV` (phase IN-1).
//!
//! No device, encoder or SDK code lives here: backends implement [`Recorder`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod contract;
pub mod error;
pub mod events;
pub mod extent;
pub mod session;
pub mod testsrc;

pub use contract::{AudioTrackSpec, Container, MetadataKind, RecordHandle, RecordJob, RecordReport, Recorder, SplitRule, StagedTarget, VideoTrackSpec};
pub use error::IngestError;
pub use events::{EventKind, IngestEvent};
pub use extent::{Extent, ExtentState, ExtentView, SeekCheck};
pub use session::{JournalEntry, Session, State, Trigger};
