//! The recording session state machine (plan IN-1).
//!
//! `Idle → Starting → Recording → Stopping → Finalized`; a fault leads to `Interrupted`, which is
//! preserved as `Recoverable` and then `Recovered` or `Quarantined`. Every transition is journaled
//! with the caller's time, so a recovery or quarantine is auditable. No other path exists: an
//! invalid transition is an error and changes nothing.

use cfa4_time::Tick;
use serde::{Deserialize, Serialize};

use crate::error::IngestError;
use crate::events::{EventKind, IngestEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Idle,
    Starting,
    Recording,
    Stopping,
    Finalized,
    Interrupted,
    Recoverable,
    Recovered,
    Quarantined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    Begin,
    Started,
    Stop,
    Stopped,
    Fault,
    Preserve,
    Recover,
    Quarantine,
    Release,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub seq: u64,
    pub at: Tick,
    pub from: State,
    pub to: State,
    pub trigger: Trigger,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct Session {
    id: u64,
    state: State,
    journal: Vec<JournalEntry>,
}

/// The transition table; `None` = not allowed.
fn next(from: State, t: Trigger) -> Option<State> {
    use State::*;
    use Trigger::*;
    Some(match (from, t) {
        (Idle, Begin) => Starting,
        (Starting, Started) => Recording,
        (Starting, Fault) | (Recording, Fault) | (Stopping, Fault) => Interrupted,
        (Recording, Stop) => Stopping,
        (Stopping, Stopped) => Finalized,
        (Interrupted, Preserve) => Recoverable,
        (Recoverable, Recover) => Recovered,
        (Recoverable, Quarantine) | (Interrupted, Quarantine) | (Finalized, Quarantine) | (Recovered, Quarantine) => Quarantined,
        // Released media is not trusted as finalized: it goes back to `Recoverable` for a new check.
        (Quarantined, Release) => Recoverable,
        _ => return None,
    })
}

impl Session {
    pub fn new(id: u64) -> Self {
        Session { id, state: State::Idle, journal: Vec::new() }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Apply `trigger` at `at`. `note` is the reason; `Fault`, `Quarantine` and `Release` need one.
    /// Returns the event for the transition (the caller publishes it).
    pub fn apply(&mut self, trigger: Trigger, at: Tick, note: &str) -> Result<IngestEvent, IngestError> {
        if matches!(trigger, Trigger::Fault | Trigger::Quarantine | Trigger::Release) && note.trim().is_empty() {
            return Err(IngestError::ReasonRequired("fault, quarantine and release must say why"));
        }
        let to = next(self.state, trigger).ok_or(IngestError::InvalidTransition { from: self.state, trigger })?;
        let seq = self.journal.len() as u64;
        self.journal.push(JournalEntry { seq, at, from: self.state, to, trigger, note: note.to_string() });
        self.state = to;
        let kind = match to {
            State::Recording => Some(EventKind::RecordingStarted),
            State::Finalized => Some(EventKind::MediaFinalized),
            State::Interrupted => Some(EventKind::RecordingInterrupted { reason: note.to_string() }),
            State::Quarantined => Some(EventKind::MediaQuarantined { reason: note.to_string() }),
            State::Stopping => Some(EventKind::RecordingStopped),
            _ => None,
        };
        Ok(IngestEvent { session: self.id, seq, at, kind: kind.unwrap_or(EventKind::StateChanged { to }) })
    }
}
