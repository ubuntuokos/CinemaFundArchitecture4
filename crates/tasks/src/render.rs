//! The render job contract of CFA4-RENDER-INTEGRATION-PLAN-001 phase C0 (`docs/audit/render-c0/`).
//!
//! A render is a [`crate::Class::FinalRender`] task in the one [`crate::Queue`]: there is no second
//! queue or state machine. This module holds only what the render client adds on top: the plan's
//! four job states as a view of [`State`], and the event a render job reports. Both are plain data
//! (serde, no dependency on the renderer), so the control channel, the CLI and the Qt GUI read the
//! same shapes. The adapter that fills [`RenderEvent::progress`] from an export's progress lives
//! above this layer.

use serde::{Deserialize, Serialize};

use crate::queue::State;

/// The states a client shows for a render job: the plan's Queued / Running / Completed / Failed /
/// Cancelled, plus Cancelling (a running job asked to stop) and Paused, which the queue has and
/// the plan lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderState {
    Queued,
    Running,
    Paused,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl RenderState {
    /// The client's view of a queue state. Scheduled, Waiting and Blocked are kinds of "not
    /// running yet"; the queue state stays available to a client that wants the reason.
    pub fn of(state: State) -> RenderState {
        match state {
            State::Scheduled | State::Queued | State::Waiting | State::Blocked => RenderState::Queued,
            State::Running => RenderState::Running,
            State::Paused => RenderState::Paused,
            State::Cancelling => RenderState::Cancelling,
            State::Succeeded => RenderState::Completed,
            State::Failed => RenderState::Failed,
            State::Cancelled => RenderState::Cancelled,
        }
    }

    /// Completed, Failed or Cancelled: the job will not change again.
    pub fn is_terminal(self) -> bool {
        matches!(self, RenderState::Completed | RenderState::Failed | RenderState::Cancelled)
    }
}

/// What a [`RenderEvent`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderEventKind {
    Progress,
    Log,
    Warning,
    /// A backend declined the work and another one took it (never silent: the plan's A4/A9).
    Fallback,
}

/// Frames done of a known total, with the estimated time left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderProgress {
    pub done: u64,
    pub total: u64,
    /// Estimated milliseconds left; `None` until there is enough to estimate from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_ms: Option<u64>,
}

impl RenderProgress {
    /// Done share in 0..=1 (0 when the total is not known yet); never above 1.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { (self.done.min(self.total) as f64) / (self.total as f64) }
    }
}

/// Which backend gave way, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderFallback {
    pub from: String,
    pub to: String,
    pub reason: String,
}

/// One report from a render job. `job` is the queue's [`crate::Task::id`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderEvent {
    pub job: String,
    pub kind: RenderEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<RenderProgress>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<RenderFallback>,
}

impl RenderEvent {
    pub fn progress(job: &str, done: u64, total: u64, eta_ms: Option<u64>, message: &str) -> RenderEvent {
        RenderEvent {
            job: job.to_owned(),
            kind: RenderEventKind::Progress,
            progress: Some(RenderProgress { done, total, eta_ms }),
            message: message.to_owned(),
            fallback: None,
        }
    }

    pub fn fallback(job: &str, from: &str, to: &str, reason: &str) -> RenderEvent {
        RenderEvent {
            job: job.to_owned(),
            kind: RenderEventKind::Fallback,
            progress: None,
            message: String::new(),
            fallback: Some(RenderFallback { from: from.to_owned(), to: to.to_owned(), reason: reason.to_owned() }),
        }
    }

    /// An event is well formed when its kind and its payload agree: a progress event carries
    /// progress, a fallback event carries a fallback, and no other kind carries either.
    pub fn is_consistent(&self) -> bool {
        match self.kind {
            RenderEventKind::Progress => self.progress.is_some() && self.fallback.is_none(),
            RenderEventKind::Fallback => self.fallback.is_some() && self.progress.is_none(),
            RenderEventKind::Log | RenderEventKind::Warning => self.progress.is_none() && self.fallback.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::transition;

    const ALL: [State; 10] = [
        State::Scheduled,
        State::Queued,
        State::Waiting,
        State::Blocked,
        State::Running,
        State::Paused,
        State::Cancelling,
        State::Succeeded,
        State::Failed,
        State::Cancelled,
    ];

    /// The contract table of `C0-contracts-spec.md` §1, pinned: a change of the mapping must
    /// change this test and the spec together.
    #[test]
    fn the_state_mapping_is_the_documented_one() {
        let expected = [
            (State::Scheduled, RenderState::Queued),
            (State::Queued, RenderState::Queued),
            (State::Waiting, RenderState::Queued),
            (State::Blocked, RenderState::Queued),
            (State::Running, RenderState::Running),
            (State::Paused, RenderState::Paused),
            (State::Cancelling, RenderState::Cancelling),
            (State::Succeeded, RenderState::Completed),
            (State::Failed, RenderState::Failed),
            (State::Cancelled, RenderState::Cancelled),
        ];
        assert_eq!(expected.len(), ALL.len());
        for (s, r) in expected {
            assert_eq!(RenderState::of(s), r, "{s:?}");
        }
    }

    #[test]
    fn terminal_in_the_queue_is_terminal_in_the_view_and_nothing_leaves_it() {
        for s in ALL {
            assert_eq!(RenderState::of(s).is_terminal(), s.is_terminal(), "{s:?}");
            if s.is_terminal() {
                assert!(ALL.iter().all(|&t| !transition(s, t)), "{s:?} must not allow any transition");
            }
        }
    }

    #[test]
    fn a_queue_transition_never_moves_the_view_out_of_a_terminal_state() {
        // The view is a function of the queue state, so the queue's transition table is the
        // only source of allowed changes: C0 adds none.
        for from in ALL {
            for to in ALL {
                if transition(from, to) {
                    assert!(!RenderState::of(from).is_terminal(), "{from:?} -> {to:?}");
                }
            }
        }
    }

    #[test]
    fn the_wire_names_are_camel_case_and_stable() {
        let e = RenderEvent::progress("job-7", 12, 48, Some(3000), "encoding");
        let j = serde_json::to_string(&e).unwrap();
        assert_eq!(j, r#"{"job":"job-7","kind":"progress","progress":{"done":12,"total":48,"etaMs":3000},"message":"encoding"}"#);
        let back: RenderEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
        let f = RenderEvent::fallback("job-7", "gpu", "cpu", "no adapter");
        let j = serde_json::to_string(&f).unwrap();
        assert_eq!(j, r#"{"job":"job-7","kind":"fallback","fallback":{"from":"gpu","to":"cpu","reason":"no adapter"}}"#);
        assert_eq!(serde_json::to_string(&RenderState::Cancelling).unwrap(), r#""cancelling""#);
    }

    #[test]
    fn a_kind_without_its_payload_is_inconsistent() {
        let mut e = RenderEvent::progress("j", 1, 2, None, "");
        assert!(e.is_consistent());
        e.progress = None;
        assert!(!e.is_consistent());
        let mut f = RenderEvent::fallback("j", "a", "b", "r");
        assert!(f.is_consistent());
        f.progress = Some(RenderProgress { done: 0, total: 0, eta_ms: None });
        assert!(!f.is_consistent());
    }

    #[test]
    fn the_fraction_is_bounded() {
        assert_eq!(RenderProgress { done: 5, total: 0, eta_ms: None }.fraction(), 0.0);
        assert_eq!(RenderProgress { done: 60, total: 48, eta_ms: None }.fraction(), 1.0);
        assert_eq!(RenderProgress { done: 12, total: 48, eta_ms: None }.fraction(), 0.25);
    }
}
