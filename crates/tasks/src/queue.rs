//! Task queue and scheduler core (ACS-4), without storage: the rules every store must follow.
//!
//! This module is the reference behaviour of the CFA4-wide queue. It holds the tasks in memory;
//! the PostgreSQL store (`tasks` schema, PG-2, through `cfa4-store`) applies the same operations
//! inside transactions (`FOR UPDATE SKIP LOCKED`, the lease, the version check), and the tests
//! here are the contract it must pass. Time is passed in by the caller (UTC milliseconds), so the
//! scheduler is testable and never reads a clock itself.
//!
//! - **States** follow the SDK host report (version 2): `scheduled`, `queued`, `waiting` (for
//!   resources), `blocked` (no path fits: the client plans again or the user approves another
//!   path), `running`, `paused`, `cancelling`, `succeeded`, `failed`, `cancelled`. AI sub-phases
//!   (`preparing_model`, `loading`) belong to `cfa4-ai-jobs`. Every change is checked against the
//!   allowed [`transition`]s and logged.
//! - **Order:** class first (P0 interactive, R final render, P1 background, P2 speculative), then
//!   the user's priority, then submission time. A task further down may start while one above
//!   waits for resources ("backfill"), but not once the one above has waited longer than
//!   [`QueueConfig::starvation_ms`]: then it goes first, so a big task is never starved.
//! - **Four start modes** ([`StartMode`]): now, when resources allow, at a time, at a time when
//!   resources allow. "Now" and "at a time" tell the user when the task has to wait; the other
//!   two wait quietly.
//! - **Exactly once:** a running task holds a lease ([`QueueConfig::lease_ms`]) that its
//!   supervisor renews. When it lapses (CFA4 or the machine stopped), a resumable task with a
//!   checkpoint is queued again from that checkpoint; any other task fails. A task never starts
//!   from the beginning a second time on its own.
//! - **Versions:** every change bumps the task's version; a change the user asks for names the
//!   version it saw, and a stale one is refused ([`QueueError::Stale`]).
//! - **Missed starts** (the machine was off at the scheduled time): run late, move to the next
//!   slot (the same local time in the task's IANA time zone, [`crate::schedule`]), or ask the
//!   user ([`MissedStart`]).

use serde::{Deserialize, Serialize};

use crate::manager::{Class, Denied, Manager, Request, ReservationId, WaitReason};
use crate::monitor::Monitor;
use crate::resources::Snapshot;

/// Milliseconds since the Unix epoch, UTC.
pub type UtcMs = u64;

/// At most this many tasks are held (a hostile client cannot grow the queue without bound).
pub const MAX_TASKS: usize = 100_000;
/// At most this many transitions are kept per task (the oldest are dropped first).
pub const MAX_LOG: usize = 256;
/// Longest accepted id, lease owner or reason text.
pub const MAX_TEXT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Priority {
    Low,
    Normal,
    High,
}

/// When a task may start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "mode")]
pub enum StartMode {
    /// As soon as possible; the user is told if it has to wait.
    Now,
    /// When the resources allow; waits quietly.
    WhenResources,
    /// At `at`; the user is told if it then has to wait.
    At { at: UtcMs },
    /// At `at` or later, when the resources allow; waits quietly.
    AtWithResources { at: UtcMs },
}

impl StartMode {
    /// The scheduled start time, if any.
    pub fn at(self) -> Option<UtcMs> {
        match self {
            StartMode::At { at } | StartMode::AtWithResources { at } => Some(at),
            StartMode::Now | StartMode::WhenResources => None,
        }
    }

    /// Whether waiting for resources is reported to the user.
    pub fn notifies_waiting(self) -> bool {
        matches!(self, StartMode::Now | StartMode::At { .. })
    }
}

/// What to do when a scheduled start was missed (the machine or CFA4 was off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MissedStart {
    /// Start as soon as possible.
    RunLate,
    /// Move to the next slot: the same local time on the next day it is still ahead, in the
    /// task's IANA time zone.
    NextSlot,
    /// Wait for the user.
    AskUser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Scheduled,
    Queued,
    Waiting,
    Blocked,
    Running,
    Paused,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl State {
    pub fn is_terminal(self) -> bool {
        matches!(self, State::Succeeded | State::Failed | State::Cancelled)
    }
}

/// Whether `from → to` is an allowed change.
pub fn transition(from: State, to: State) -> bool {
    use State::*;
    matches!(
        (from, to),
        (Scheduled, Queued | Blocked | Paused | Cancelled)
            | (Queued | Waiting, Running | Waiting | Blocked | Paused | Cancelled)
            | (Blocked, Queued | Cancelled)
            | (Running, Succeeded | Failed | Paused | Cancelling | Queued)
            | (Paused, Queued | Scheduled | Cancelled)
            | (Cancelling, Cancelled | Failed)
    )
}

/// Who made a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Actor {
    User,
    Scheduler,
    Worker,
}

/// One logged change (the `tasks.transition` row).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub seq: u64,
    pub from: State,
    pub to: State,
    pub at: UtcMs,
    pub by: Actor,
    pub reason: String,
}

/// What a `blocked` task waits for from the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum Awaiting {
    /// The one-time acceptance of the model's terms: per implementation and weights hash. The CFA4
    /// does not distribute the model, so this warning is the user's own check (ADR 0008 §2).
    LicenseAcceptance { model: String, weights_sha256: String },
}

/// What the client submits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTask {
    /// Client-generated (UUIDv7 text in the store).
    pub id: String,
    /// What the planner chose for it (ACS-3); `request.class` is the task's class.
    pub request: Request,
    pub priority: Priority,
    pub start: StartMode,
    pub missed: MissedStart,
    /// It saves checkpoints and can continue from the last one.
    pub resumable: bool,
    /// The IANA time zone its start time was chosen in (`Europe/Budapest`); a missed start moves
    /// to the next slot there ([`crate::schedule::next_slot`]). `None`: UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// Tasks with the same key use the same resident model (for example the weights hash): of two
    /// equally ranked tasks the one that matches the model already loaded goes first, so the model
    /// is loaded once for the batch (ACS-8, [`QueueConfig::starvation_ms`] bounds the preference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_key: Option<String>,
}

/// A task in the queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub request: Request,
    pub priority: Priority,
    pub start: StartMode,
    pub missed: MissedStart,
    pub resumable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_key: Option<String>,
    /// Set while the task is `blocked` on something only the user can give, not on resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting: Option<Awaiting>,
    pub state: State,
    pub version: u64,
    pub submitted_at: UtcMs,
    /// When it last entered `queued` or `waiting` (for the starvation rule).
    pub ready_since: Option<UtcMs>,
    /// Starts so far (a resume counts).
    pub attempts: u32,
    /// The last finished stage, if any.
    pub checkpoint: Option<u64>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<UtcMs>,
    pub reservation: Option<ReservationId>,
    pub gpu_node: Option<String>,
    /// Why it waits or is blocked, as of the last attempt.
    pub wait_reasons: Vec<WaitReason>,
    pub blocked_by: Option<Denied>,
    pub log: Vec<Transition>,
    next_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueueConfig {
    /// How long a lease lasts without renewal.
    pub lease_ms: u64,
    /// A start more than this late counts as missed.
    pub missed_grace_ms: u64,
    /// After waiting this long, a task no longer lets lower ones start before it.
    pub starvation_ms: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig { lease_ms: 30_000, missed_grace_ms: 5 * 60_000, starvation_ms: 10 * 60_000 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "error")]
pub enum QueueError {
    Full,
    DuplicateId {
        id: String,
    },
    NotFound {
        id: String,
    },
    /// The version the caller saw is not the current one.
    Stale {
        id: String,
        expected: u64,
        current: u64,
    },
    NotAllowed {
        id: String,
        from: State,
        to: State,
    },
    /// The caller does not hold the task's lease.
    NotLeaseOwner {
        id: String,
    },
    Invalid {
        reason: String,
    },
}

/// What a [`Queue::tick`] did, for the UI, the notifications (ACS-5) and the outbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event")]
pub enum QueueEvent {
    Started {
        id: String,
        gpu_node: Option<String>,
        resumed_from: Option<u64>,
    },
    /// Has to wait; `notify` when the user asked for it now or at a time.
    Waiting {
        id: String,
        reasons: Vec<WaitReason>,
        notify: bool,
    },
    /// No path fits: plan again or ask the user.
    Blocked {
        id: String,
        denied: Denied,
    },
    /// A scheduled start was missed; with [`MissedStart::NextSlot`] it moved to `next_at`.
    Missed {
        id: String,
        scheduled_at: UtcMs,
        action: MissedStart,
        /// With [`MissedStart::NextSlot`]: the new start time (stays `scheduled`).
        next_at: Option<UtcMs>,
    },
    /// The lease lapsed: resumed from the checkpoint (queued) or failed.
    LeaseLost {
        id: String,
        resumed: bool,
    },
}

/// The queue: tasks and the rules over them. Plain data; the caller keeps it behind one lock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    config: QueueConfig,
    tasks: Vec<Task>,
    /// The `batch_key` of the task started last: the model that is loaded now.
    #[serde(default)]
    resident: Option<String>,
}

/// Start order of the classes (lower goes first): interactive, final render, background, speculative.
pub fn class_rank(class: Class) -> u8 {
    match class {
        Class::Interactive => 0,
        Class::FinalRender => 1,
        Class::Background => 2,
        Class::Speculative => 3,
    }
}

fn check_text(what: &str, text: &str) -> Result<(), QueueError> {
    if text.is_empty() || text.len() > MAX_TEXT {
        return Err(QueueError::Invalid { reason: format!("{what}: empty or longer than {MAX_TEXT} bytes") });
    }
    Ok(())
}

fn clip(text: &str) -> String {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default().to_owned()
}

impl Task {
    fn set_state(&mut self, to: State, at: UtcMs, by: Actor, reason: &str) -> Result<(), QueueError> {
        if !transition(self.state, to) {
            return Err(QueueError::NotAllowed { id: self.id.clone(), from: self.state, to });
        }
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(Transition { seq: self.next_seq, from: self.state, to, at, by, reason: clip(reason) });
        self.next_seq = self.next_seq.saturating_add(1);
        if matches!(to, State::Queued | State::Waiting) && !matches!(self.state, State::Queued | State::Waiting) {
            self.ready_since = Some(at);
        }
        if !matches!(to, State::Queued | State::Waiting) {
            self.ready_since = None;
        }
        self.state = to;
        self.version = self.version.saturating_add(1);
        Ok(())
    }

    fn check_version(&self, expected: u64) -> Result<(), QueueError> {
        if self.version != expected {
            return Err(QueueError::Stale { id: self.id.clone(), expected, current: self.version });
        }
        Ok(())
    }

    fn check_owner(&self, owner: &str) -> Result<(), QueueError> {
        if self.state != State::Running || self.lease_owner.as_deref() != Some(owner) {
            return Err(QueueError::NotLeaseOwner { id: self.id.clone() });
        }
        Ok(())
    }
}

impl Queue {
    pub fn new(config: QueueConfig) -> Self {
        Queue { config, tasks: Vec::new(), resident: None }
    }

    /// A queue over tasks loaded from a store (the store runs one operation on them and writes
    /// back what changed).
    pub fn from_tasks(config: QueueConfig, tasks: Vec<Task>) -> Self {
        Queue { config, tasks, resident: None }
    }

    pub fn into_tasks(self) -> Vec<Task> {
        self.tasks
    }

    pub fn config(&self) -> &QueueConfig {
        &self.config
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut Task, QueueError> {
        self.tasks.iter_mut().find(|t| t.id == id).ok_or_else(|| QueueError::NotFound { id: id.to_owned() })
    }

    /// Add a task: `scheduled` when it has a start time, `queued` otherwise.
    pub fn submit(&mut self, new: NewTask, now: UtcMs) -> Result<&Task, QueueError> {
        check_text("id", &new.id)?;
        if self.tasks.len() >= MAX_TASKS {
            return Err(QueueError::Full);
        }
        if self.get(&new.id).is_some() {
            return Err(QueueError::DuplicateId { id: new.id });
        }
        if let Some(zone) = &new.time_zone {
            crate::schedule::check_zone(zone).map_err(|e| QueueError::Invalid { reason: format!("time zone: {e:?}") })?;
        }
        let scheduled = new.start.at().is_some();
        let task = Task {
            id: new.id,
            request: new.request,
            priority: new.priority,
            start: new.start,
            missed: new.missed,
            resumable: new.resumable,
            time_zone: new.time_zone,
            batch_key: new.batch_key,
            awaiting: None,
            state: if scheduled { State::Scheduled } else { State::Queued },
            version: 1,
            submitted_at: now,
            ready_since: (!scheduled).then_some(now),
            attempts: 0,
            checkpoint: None,
            lease_owner: None,
            lease_until: None,
            reservation: None,
            gpu_node: None,
            wait_reasons: Vec::new(),
            blocked_by: None,
            log: Vec::new(),
            next_seq: 1,
        };
        self.tasks.push(task);
        self.tasks.last().ok_or(QueueError::Full)
    }

    /// One scheduler step at `now`: lapsed leases, due and missed starts, then start what fits,
    /// in order. Granted tasks get a lease for `owner` (the supervisor that runs them).
    pub fn tick(&mut self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor) -> Result<Vec<QueueEvent>, QueueError> {
        check_text("owner", owner)?;
        let mut events = Vec::new();
        self.expire_leases(now, manager, &mut events);
        self.due_starts(now, &mut events);
        self.launch(now, owner, manager, snap, monitor, &mut events);
        Ok(events)
    }

    fn expire_leases(&mut self, now: UtcMs, manager: &mut Manager, events: &mut Vec<QueueEvent>) {
        for t in self.tasks.iter_mut().filter(|t| matches!(t.state, State::Running | State::Cancelling) && t.lease_until.is_some_and(|u| u < now)) {
            if let Some(r) = t.reservation.take() {
                manager.release(r);
            }
            t.lease_owner = None;
            t.lease_until = None;
            t.gpu_node = None;
            // its worker died while stopping: it is cancelled, as the user asked
            if t.state == State::Cancelling {
                if t.set_state(State::Cancelled, now, Actor::Scheduler, "lease lapsed while cancelling").is_ok() {
                    events.push(QueueEvent::LeaseLost { id: t.id.clone(), resumed: false });
                }
                continue;
            }
            // never from the beginning a second time: only a checkpointed resumable task continues
            let resumed = t.resumable && t.checkpoint.is_some();
            let (to, reason) = if resumed { (State::Queued, "lease lapsed, resume from checkpoint") } else { (State::Failed, "lease lapsed, not resumable") };
            if t.set_state(to, now, Actor::Scheduler, reason).is_ok() {
                events.push(QueueEvent::LeaseLost { id: t.id.clone(), resumed });
            }
        }
    }

    fn due_starts(&mut self, now: UtcMs, events: &mut Vec<QueueEvent>) {
        let grace = self.config.missed_grace_ms;
        for t in self.tasks.iter_mut().filter(|t| t.state == State::Scheduled) {
            let Some(at) = t.start.at() else {
                continue;
            };
            if at > now {
                continue;
            }
            if now.saturating_sub(at) <= grace {
                let _ = t.set_state(State::Queued, now, Actor::Scheduler, "start time reached");
                continue;
            }
            let next_at = match t.missed {
                MissedStart::RunLate => {
                    let _ = t.set_state(State::Queued, now, Actor::Scheduler, "missed start, running late");
                    None
                }
                // the same local time on the next day it is still ahead, in the task's time zone
                MissedStart::NextSlot => match crate::schedule::next_slot(at, t.time_zone.as_deref().unwrap_or("UTC"), now) {
                    Ok(next) => {
                        t.start = match t.start {
                            StartMode::AtWithResources { .. } => StartMode::AtWithResources { at: next },
                            _ => StartMode::At { at: next },
                        };
                        t.version = t.version.saturating_add(1);
                        Some(next)
                    }
                    // no zone to compute it in: the user decides
                    Err(_) => {
                        let _ = t.set_state(State::Paused, now, Actor::Scheduler, "missed start, next slot unknown");
                        None
                    }
                },
                MissedStart::AskUser => {
                    let _ = t.set_state(State::Paused, now, Actor::Scheduler, "missed start");
                    None
                }
            };
            events.push(QueueEvent::Missed { id: t.id.clone(), scheduled_at: at, action: t.missed, next_at });
        }
    }

    /// Ready tasks in start order.
    fn order(&self, now: UtcMs) -> Vec<usize> {
        let mut ready: Vec<usize> =
            (0..self.tasks.len()).filter(|&i| self.tasks.get(i).is_some_and(|t| matches!(t.state, State::Queued | State::Waiting))).collect();
        let starvation = self.config.starvation_ms;
        // among equals the task that matches the resident model goes first, unless an older one
        // has already waited too long (a model is not kept hot at the cost of starving others)
        let waited_too_long = |t: &Task| t.ready_since.is_some_and(|s| now.saturating_sub(s) >= starvation);
        // `false` sorts first: matches the resident model and no starved rival of the same rank
        let behind_resident = |t: &Task| -> bool {
            let matches = t.batch_key.is_some() && t.batch_key == self.resident;
            let rival_starved = self.tasks.iter().any(|o| {
                matches!(o.state, State::Queued | State::Waiting)
                    && o.id != t.id
                    && o.request.class == t.request.class
                    && o.priority == t.priority
                    && waited_too_long(o)
            });
            !matches || rival_starved
        };
        ready.sort_by_key(|&i| {
            let t = self.tasks.get(i);
            (t.map(|t| class_rank(t.request.class)), t.map(|t| std::cmp::Reverse(t.priority)), t.map(behind_resident), t.map(|t| t.submitted_at), i)
        });
        ready
    }

    fn launch(&mut self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor, events: &mut Vec<QueueEvent>) {
        let starvation = self.config.starvation_ms;
        let lease = self.config.lease_ms;
        let mut resident = self.resident.clone();
        for i in self.order(now) {
            let Some(t) = self.tasks.get_mut(i) else {
                continue;
            };
            match manager.try_reserve(&t.request, snap, monitor) {
                Ok(grant) => {
                    let resumed_from = if t.attempts > 0 { t.checkpoint } else { None };
                    if t.set_state(State::Running, now, Actor::Scheduler, "resources reserved").is_err() {
                        manager.release(grant.id);
                        continue;
                    }
                    t.attempts = t.attempts.saturating_add(1);
                    t.reservation = Some(grant.id);
                    t.gpu_node = grant.gpu_node.clone();
                    t.lease_owner = Some(owner.to_owned());
                    t.lease_until = Some(now.saturating_add(lease));
                    t.wait_reasons.clear();
                    events.push(QueueEvent::Started { id: t.id.clone(), gpu_node: grant.gpu_node, resumed_from });
                    if t.batch_key.is_some() {
                        resident = t.batch_key.clone();
                    }
                }
                Err(Denied::Wait { reasons }) => {
                    let first_wait = t.state != State::Waiting;
                    if first_wait {
                        let _ = t.set_state(State::Waiting, now, Actor::Scheduler, "waiting for resources");
                    }
                    if first_wait || t.wait_reasons != reasons {
                        events.push(QueueEvent::Waiting { id: t.id.clone(), reasons: reasons.clone(), notify: t.start.notifies_waiting() });
                    }
                    t.wait_reasons = reasons;
                    // a task that has waited too long keeps the ones below from starting first
                    if t.ready_since.is_some_and(|s| now.saturating_sub(s) >= starvation) {
                        break;
                    }
                }
                Err(denied) => {
                    let _ = t.set_state(State::Blocked, now, Actor::Scheduler, "no path fits");
                    t.blocked_by = Some(denied.clone());
                    events.push(QueueEvent::Blocked { id: t.id.clone(), denied });
                }
            }
        }
        self.resident = resident;
    }

    /// The model loaded now (the `batch_key` of the task started last).
    pub fn resident(&self) -> Option<&str> {
        self.resident.as_deref()
    }

    /// The model is gone from memory (unloaded, or the worker ended): no task is preferred for it.
    pub fn clear_resident(&mut self) {
        self.resident = None;
    }

    /// Block a task until the user has accepted the model's terms (per implementation and weights
    /// hash). The one-time acceptance itself is stored by the caller; a task whose acceptance is
    /// already on record never gets here.
    pub fn await_license(&mut self, id: &str, expected_version: u64, model: &str, weights_sha256: &str, now: UtcMs) -> Result<(), QueueError> {
        check_text("model", model)?;
        check_text("weights_sha256", weights_sha256)?;
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        t.set_state(State::Blocked, now, Actor::Scheduler, "license acceptance needed")?;
        t.awaiting = Some(Awaiting::LicenseAcceptance { model: model.to_owned(), weights_sha256: weights_sha256.to_owned() });
        t.blocked_by = None;
        Ok(())
    }

    /// The user accepted the terms: the task is queued again, behind nothing it did not wait for.
    pub fn accept_license(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        if !matches!(t.awaiting, Some(Awaiting::LicenseAcceptance { .. })) {
            return Err(QueueError::Invalid { reason: "the task does not wait for a license acceptance".into() });
        }
        t.set_state(State::Queued, now, Actor::User, "license accepted")?;
        t.awaiting = None;
        Ok(())
    }

    /// The supervisor renews the lease of a running task.
    pub fn renew(&mut self, id: &str, owner: &str, now: UtcMs) -> Result<UtcMs, QueueError> {
        let lease = self.config.lease_ms;
        let t = self.get_mut(id)?;
        t.check_owner(owner)?;
        let until = now.saturating_add(lease);
        t.lease_until = Some(until);
        Ok(until)
    }

    /// A stage finished: the task can continue from here after a restart.
    pub fn checkpoint(&mut self, id: &str, owner: &str, stage: u64) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_owner(owner)?;
        t.checkpoint = Some(t.checkpoint.map_or(stage, |c| c.max(stage)));
        Ok(())
    }

    /// The task ended (`Succeeded`, `Failed`, or `Paused` after saving a checkpoint). Its
    /// resources are released.
    pub fn finish(&mut self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_owner(owner)?;
        if !matches!(to, State::Succeeded | State::Failed | State::Paused) {
            return Err(QueueError::NotAllowed { id: id.to_owned(), from: t.state, to });
        }
        if to == State::Paused && !(t.resumable && t.checkpoint.is_some()) {
            return Err(QueueError::Invalid { reason: format!("{id}: pausing needs a checkpoint") });
        }
        t.set_state(to, now, Actor::Worker, reason)?;
        if let Some(r) = t.reservation.take() {
            manager.release(r);
        }
        t.lease_owner = None;
        t.lease_until = None;
        t.gpu_node = None;
        Ok(())
    }

    /// The user cancels: a running task goes to `cancelling` (its worker stops and reports
    /// [`Queue::cancelled`]); anything else not finished is cancelled at once.
    pub fn cancel(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<State, QueueError> {
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        // already stopping: its worker (or the lapsed lease) ends it and releases the resources;
        // cancelling it again must not skip that (ACS-7 stability run)
        if t.state == State::Cancelling {
            return Ok(State::Cancelling);
        }
        let to = if t.state == State::Running { State::Cancelling } else { State::Cancelled };
        t.set_state(to, now, Actor::User, "cancelled by the user")?;
        Ok(to)
    }

    /// The worker of a cancelling task stopped.
    pub fn cancelled(&mut self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        if t.state != State::Cancelling || t.lease_owner.as_deref() != Some(owner) {
            return Err(QueueError::NotLeaseOwner { id: id.to_owned() });
        }
        t.set_state(State::Cancelled, now, Actor::Worker, "stopped")?;
        if let Some(r) = t.reservation.take() {
            manager.release(r);
        }
        t.lease_owner = None;
        t.lease_until = None;
        Ok(())
    }

    /// The user pauses a task that has not started (a running one pauses through its worker).
    pub fn pause(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        if t.state == State::Running {
            return Err(QueueError::NotAllowed { id: id.to_owned(), from: State::Running, to: State::Paused });
        }
        t.set_state(State::Paused, now, Actor::User, "paused by the user")
    }

    /// The user resumes a paused task, or lets a missed one run now.
    pub fn resume(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        // only a paused task: a running one would be queued again with its reservation still held
        // and start a second time (ACS-7 stability run)
        if t.state != State::Paused {
            return Err(QueueError::NotAllowed { id: id.to_owned(), from: t.state, to: State::Queued });
        }
        t.set_state(State::Queued, now, Actor::User, "resumed by the user")
    }

    /// A new start time for a scheduled or paused task (a missed start's next slot, or the user
    /// moving it).
    pub fn reschedule(&mut self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        if !matches!(t.state, State::Scheduled | State::Paused) {
            return Err(QueueError::NotAllowed { id: id.to_owned(), from: t.state, to: State::Scheduled });
        }
        t.start = match t.start {
            StartMode::Now | StartMode::At { .. } => StartMode::At { at },
            StartMode::WhenResources | StartMode::AtWithResources { .. } => StartMode::AtWithResources { at },
        };
        if t.state == State::Paused {
            t.set_state(State::Scheduled, now, Actor::User, "rescheduled")
        } else {
            t.version = t.version.saturating_add(1);
            Ok(())
        }
    }

    /// A blocked task gets a new request (the client planned again, or the user approved a path
    /// that changes the result) and is queued again.
    pub fn unblock(&mut self, id: &str, expected_version: u64, request: Request, now: UtcMs) -> Result<(), QueueError> {
        let t = self.get_mut(id)?;
        t.check_version(expected_version)?;
        t.set_state(State::Queued, now, Actor::User, "new path")?;
        t.request = request;
        t.blocked_by = None;
        t.awaiting = None;
        Ok(())
    }

    /// Drop finished tasks older than `before` (their history lives in the store).
    pub fn prune(&mut self, before: UtcMs) -> usize {
        let len = self.tasks.len();
        self.tasks.retain(|t| !(t.state.is_terminal() && t.log.last().is_some_and(|l| l.at < before)));
        len.saturating_sub(self.tasks.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{Budget, BudgetSettings};
    use crate::manager::GpuRequest;
    use crate::resources::{Current, GpuCurrent, GpuNominal, Nominal};
    use cfa4_route::Vendor;

    const GIB: u64 = 1 << 30;
    const MIN: u64 = 60_000;

    fn snap() -> Snapshot {
        Snapshot {
            nominal: Nominal {
                ram_physical_bytes: Some(32 * GIB),
                cpus_online: Some(16),
                gpus: vec![GpuNominal {
                    node: "renderD128".into(),
                    vendor: Vendor::Nvidia,
                    driver: "nvidia".into(),
                    vram_total_bytes: Some(12 * GIB),
                    drives_display: Some(false),
                }],
                ..Nominal::default()
            },
            current: Current {
                ram_available_bytes: Some(28 * GIB),
                gpus: vec![GpuCurrent { node: "renderD128".into(), vram_used_bytes: Some(0), busy_percent: None }],
                ..Current::default()
            },
            notes: vec![],
        }
    }

    fn manager(s: &Snapshot) -> Manager {
        let mut m = Manager::new(Budget::new(&s.nominal, &BudgetSettings::default()));
        m.set_settle_samples(1);
        m
    }

    fn task(id: &str, class: Class, priority: Priority, vram_gib: u64, start: StartMode) -> NewTask {
        NewTask {
            id: id.into(),
            request: Request {
                class,
                ram_bytes: GIB,
                cpu_milli: 1000,
                gpu: (vram_gib > 0).then(|| GpuRequest { vram_bytes: vram_gib * GIB, node: None, vendor: None }),
            },
            priority,
            start,
            missed: MissedStart::RunLate,
            resumable: true,
            time_zone: None,
            batch_key: None,
        }
    }

    fn state(q: &Queue, id: &str) -> Option<State> {
        q.get(id).map(|t| t.state)
    }

    #[test]
    fn class_then_priority_then_age_and_atomic_starts() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        q.submit(task("p1-low", Class::Background, Priority::Low, 8, StartMode::Now), 1).unwrap();
        q.submit(task("p1-high", Class::Background, Priority::High, 8, StartMode::Now), 2).unwrap();
        q.submit(task("p0", Class::Interactive, Priority::Low, 2, StartMode::Now), 3).unwrap();
        let e = q.tick(10, "sup", &mut m, &s, &mon).unwrap();
        // 11.5 GiB of VRAM: the interactive task and the high-priority background one fit
        assert!(matches!(e.first(), Some(QueueEvent::Started { id, .. }) if id == "p0"), "{e:?}");
        assert_eq!((state(&q, "p0"), state(&q, "p1-high"), state(&q, "p1-low")), (Some(State::Running), Some(State::Running), Some(State::Waiting)));
        assert!(matches!(e.last(), Some(QueueEvent::Waiting { notify: true, .. })), "started now: the user is told");
    }

    fn keyed(id: &str, key: &str, vram_gib: u64) -> NewTask {
        NewTask { batch_key: Some(key.into()), ..task(id, Class::Background, Priority::Normal, vram_gib, StartMode::Now) }
    }

    #[test]
    fn tasks_of_the_resident_model_go_first_but_never_starve_an_older_one() {
        let s = snap();
        let mon = Monitor::default();
        // the large task takes all VRAM, so only one task runs at a time and the order is visible
        let mut m = manager(&s);
        let mut q = Queue::default();
        q.submit(keyed("a1", "model-a", 8), 1).unwrap();
        q.tick(2, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(q.resident(), Some("model-a"));
        q.submit(keyed("b1", "model-b", 8), 3).unwrap();
        q.submit(keyed("a2", "model-a", 8), 4).unwrap();
        q.finish("a1", "sup", State::Succeeded, 5, "done", &mut m).unwrap();
        q.tick(6, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!((state(&q, "a2"), state(&q, "b1")), (Some(State::Running), Some(State::Waiting)), "the loaded model is reused: one load for the batch");
        // after the starvation limit the older task of another model is not passed again
        let mut m = manager(&s);
        let mut q = Queue::default();
        q.submit(keyed("a1", "model-a", 8), 1).unwrap();
        q.tick(2, "sup", &mut m, &s, &mon).unwrap();
        q.submit(keyed("b1", "model-b", 8), 3).unwrap();
        let late = 3 + QueueConfig::default().starvation_ms + 1;
        q.submit(keyed("a2", "model-a", 8), late).unwrap();
        q.finish("a1", "sup", State::Succeeded, late, "done", &mut m).unwrap();
        q.tick(late + 1, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!((state(&q, "b1"), state(&q, "a2")), (Some(State::Running), Some(State::Waiting)), "b1 waited too long");
        q.clear_resident();
        assert_eq!(q.resident(), None);
    }

    #[test]
    fn a_task_waits_in_blocked_until_the_license_is_accepted() {
        let mut q = Queue::default();
        q.submit(keyed("lic", "model-x", 4), 1).unwrap();
        let v = q.get("lic").map(|t| t.version).unwrap_or(0);
        assert!(q.accept_license("lic", v, 2).is_err(), "nothing was awaited");
        q.await_license("lic", v, "model-x", "ab12", 2).unwrap();
        let t = q.get("lic").unwrap();
        assert_eq!(t.state, State::Blocked);
        assert_eq!(t.awaiting, Some(Awaiting::LicenseAcceptance { model: "model-x".into(), weights_sha256: "ab12".into() }));
        // blocked tasks are never started, whatever the resources
        let s = snap();
        let mut m = manager(&s);
        q.tick(3, "sup", &mut m, &s, &Monitor::default()).unwrap();
        assert_eq!(state(&q, "lic"), Some(State::Blocked));
        let v = q.get("lic").map(|t| t.version).unwrap_or(0);
        assert!(matches!(q.accept_license("lic", v.saturating_add(9), 4), Err(QueueError::Stale { .. })));
        q.accept_license("lic", v, 4).unwrap();
        assert_eq!((state(&q, "lic"), q.get("lic").and_then(|t| t.awaiting.clone())), (Some(State::Queued), None));
        q.tick(5, "sup", &mut m, &s, &Monitor::default()).unwrap();
        assert_eq!(state(&q, "lic"), Some(State::Running));
        assert!(q.await_license("lic", 1, "", "x", 6).is_err(), "hostile input is refused");
    }

    #[test]
    fn cancelling_twice_or_losing_the_worker_never_leaks_the_reservation() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        q.submit(task("a", Class::Background, Priority::Normal, 2, StartMode::Now), 0).unwrap();
        q.submit(task("b", Class::Background, Priority::Normal, 2, StartMode::Now), 0).unwrap();
        q.tick(1, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(m.held_count(), 2);
        // a second cancel of a stopping task changes nothing (it used to cancel it and keep its reservation)
        let v = q.get("a").unwrap().version;
        assert_eq!(q.cancel("a", v, 2).unwrap(), State::Cancelling);
        let v = q.get("a").unwrap().version;
        assert_eq!(q.cancel("a", v, 3).unwrap(), State::Cancelling);
        assert_eq!(m.held_count(), 2);
        // nor does resuming a running task: it would start twice
        let v = q.get("b").unwrap().version;
        assert!(matches!(q.resume("b", v, 3), Err(QueueError::NotAllowed { .. })));
        q.cancelled("a", "sup", 4, &mut m).unwrap();
        assert_eq!(m.held_count(), 1);
        // the worker of a stopping task dies: the lapsed lease cancels it and frees its resources
        let v = q.get("b").unwrap().version;
        q.cancel("b", v, 5).unwrap();
        let e = q.tick(10 * MIN, "sup", &mut m, &s, &mon).unwrap();
        assert!(e.contains(&QueueEvent::LeaseLost { id: "b".into(), resumed: false }), "{e:?}");
        assert_eq!((state(&q, "b"), m.held_count()), (Some(State::Cancelled), 0));
    }

    #[test]
    fn a_starved_task_stops_the_backfill() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::new(QueueConfig { lease_ms: 60 * MIN, ..QueueConfig::default() });
        q.submit(task("hog", Class::Background, Priority::Normal, 6, StartMode::Now), 0).unwrap();
        q.tick(0, "sup", &mut m, &s, &mon).unwrap();
        q.submit(task("big", Class::Background, Priority::High, 8, StartMode::WhenResources), 1).unwrap();
        q.submit(task("small", Class::Background, Priority::Low, 2, StartMode::WhenResources), 2).unwrap();
        q.tick(MIN, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!((state(&q, "big"), state(&q, "small")), (Some(State::Waiting), Some(State::Running)), "backfill while the big one is young");
        q.finish("small", "sup", State::Succeeded, 2 * MIN, "done", &mut m).unwrap();
        q.submit(task("small2", Class::Background, Priority::Low, 2, StartMode::WhenResources), 3 * MIN).unwrap();
        q.tick(20 * MIN, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "small2"), Some(State::Queued), "the big task waited 10+ minutes: no more backfill");
        q.finish("hog", "sup", State::Succeeded, 21 * MIN, "done", &mut m).unwrap();
        q.tick(22 * MIN, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "big"), Some(State::Running));
    }

    #[test]
    fn a_lapsed_lease_resumes_from_a_checkpoint_or_fails_never_restarts() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        q.submit(task("resumable", Class::Background, Priority::Normal, 2, StartMode::Now), 0).unwrap();
        q.submit(NewTask { resumable: false, ..task("oneshot", Class::Background, Priority::Normal, 2, StartMode::Now) }, 0).unwrap();
        q.tick(0, "sup-a", &mut m, &s, &mon).unwrap();
        q.checkpoint("resumable", "sup-a", 3).unwrap();
        assert_eq!(q.checkpoint("resumable", "sup-b", 4), Err(QueueError::NotLeaseOwner { id: "resumable".into() }));
        q.renew("resumable", "sup-a", 20_000).unwrap();
        // CFA4 stops; a new supervisor ticks after the leases lapsed
        let e = q.tick(60_000, "sup-b", &mut m, &s, &mon).unwrap();
        assert!(e.contains(&QueueEvent::LeaseLost { id: "oneshot".into(), resumed: false }));
        assert!(e.contains(&QueueEvent::LeaseLost { id: "resumable".into(), resumed: true }));
        assert!(e.contains(&QueueEvent::Started { id: "resumable".into(), gpu_node: Some("renderD128".into()), resumed_from: Some(3) }));
        assert_eq!(state(&q, "oneshot"), Some(State::Failed));
        assert_eq!(q.get("resumable").map(|t| (t.attempts, t.lease_owner.as_deref())), Some((2, Some("sup-b"))));
        assert_eq!(m.held_count(), 1, "the lapsed reservations were released");
    }

    #[test]
    fn scheduled_starts_and_missed_ones() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        q.submit(task("night", Class::Background, Priority::Normal, 2, StartMode::AtWithResources { at: 100 * MIN }), 0).unwrap();
        q.submit(NewTask { missed: MissedStart::NextSlot, ..task("slot", Class::Background, Priority::Normal, 2, StartMode::At { at: 10 * MIN }) }, 0).unwrap();
        q.submit(NewTask { missed: MissedStart::AskUser, ..task("ask", Class::Background, Priority::Normal, 2, StartMode::At { at: 10 * MIN }) }, 0).unwrap();
        q.submit(task("late", Class::Background, Priority::Normal, 2, StartMode::At { at: 10 * MIN }), 0).unwrap();
        assert_eq!(state(&q, "night"), Some(State::Scheduled));
        q.tick(MIN, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "night"), Some(State::Scheduled), "not yet");
        // the machine was off from minute 1 to minute 60
        let e = q.tick(60 * MIN, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "late"), Some(State::Running), "run late");
        // the next slot: the same time of day tomorrow (no zone given: UTC), still scheduled
        assert_eq!((state(&q, "slot"), state(&q, "ask")), (Some(State::Scheduled), Some(State::Paused)));
        let next = 24 * 60 * MIN + 10 * MIN;
        assert!(e.contains(&QueueEvent::Missed { id: "slot".into(), scheduled_at: 10 * MIN, action: MissedStart::NextSlot, next_at: Some(next) }), "{e:?}");
        assert_eq!(q.get("slot").map(|t| t.start), Some(StartMode::At { at: next }));
        // the user can still move it
        let v = q.get("slot").map_or(0, |t| t.version);
        q.reschedule("slot", v, 2 * next, 60 * MIN).unwrap();
        assert_eq!(state(&q, "slot"), Some(State::Scheduled));
        // an unknown zone is refused when the task is submitted
        let bad = NewTask { time_zone: Some("Nowhere/Atlantis".into()), ..task("tz", Class::Background, Priority::Normal, 0, StartMode::Now) };
        assert!(matches!(q.submit(bad, 0), Err(QueueError::Invalid { .. })));
        // on time (within the grace)
        q.tick(102 * MIN, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "night"), Some(State::Running));
    }

    #[test]
    fn stale_versions_and_forbidden_changes_are_refused() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        q.submit(task("a", Class::Background, Priority::Normal, 2, StartMode::Now), 0).unwrap();
        assert_eq!(q.submit(task("a", Class::Background, Priority::Normal, 2, StartMode::Now), 0).err(), Some(QueueError::DuplicateId { id: "a".into() }));
        q.tick(0, "sup", &mut m, &s, &mon).unwrap();
        assert!(matches!(q.pause("a", 1, 1), Err(QueueError::Stale { expected: 1, current: 2, .. })), "the user saw the queued version");
        assert!(matches!(q.pause("a", 2, 1), Err(QueueError::NotAllowed { .. })), "a running task pauses through its worker");
        assert_eq!(q.cancel("a", 2, 2), Ok(State::Cancelling));
        assert!(q.cancelled("a", "other", 3, &mut m).is_err());
        q.cancelled("a", "sup", 3, &mut m).unwrap();
        assert_eq!(state(&q, "a"), Some(State::Cancelled));
        assert_eq!(m.held_count(), 0);
        let v = q.get("a").map_or(0, |t| t.version);
        assert!(matches!(q.resume("a", v, 4), Err(QueueError::NotAllowed { .. })), "cancelled is final");
        let log: Vec<_> = q.get("a").map(|t| t.log.iter().map(|l| (l.from, l.to)).collect()).unwrap_or_default();
        assert_eq!(log, vec![(State::Queued, State::Running), (State::Running, State::Cancelling), (State::Cancelling, State::Cancelled)]);
    }

    #[test]
    fn pausing_needs_a_checkpoint_and_blocked_tasks_wait_for_a_new_path() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        q.submit(task("a", Class::Background, Priority::Normal, 2, StartMode::Now), 0).unwrap();
        q.submit(task("huge", Class::Background, Priority::Normal, 40, StartMode::Now), 0).unwrap();
        let e = q.tick(0, "sup", &mut m, &s, &mon).unwrap();
        assert!(matches!(e.last(), Some(QueueEvent::Blocked { id, .. }) if id == "huge"));
        assert!(matches!(q.finish("a", "sup", State::Paused, 1, "pause", &mut m), Err(QueueError::Invalid { .. })));
        q.checkpoint("a", "sup", 1).unwrap();
        q.finish("a", "sup", State::Paused, 2, "pause", &mut m).unwrap();
        assert_eq!(m.held_count(), 0, "a paused task frees its resources");
        let v = q.get("huge").map_or(0, |t| t.version);
        let smaller = task("x", Class::Background, Priority::Normal, 4, StartMode::Now).request;
        q.unblock("huge", v, smaller, 3).unwrap();
        q.tick(4, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "huge"), Some(State::Running));
    }

    #[test]
    fn hostile_input_is_an_error_never_a_panic() {
        let s = snap();
        let mut m = manager(&s);
        let mon = Monitor::default();
        let mut q = Queue::default();
        assert!(matches!(q.submit(task("", Class::Background, Priority::Normal, 0, StartMode::Now), 0), Err(QueueError::Invalid { .. })));
        assert!(matches!(
            q.submit(task(&"x".repeat(MAX_TEXT + 1), Class::Background, Priority::Normal, 0, StartMode::Now), 0),
            Err(QueueError::Invalid { .. })
        ));
        assert!(q.tick(0, "", &mut m, &s, &mon).is_err());
        assert!(matches!(q.renew("nope", "sup", 0), Err(QueueError::NotFound { .. })));
        // u64::MAX times saturate
        q.submit(task("t", Class::Background, Priority::Normal, 0, StartMode::At { at: u64::MAX }), u64::MAX).unwrap();
        q.tick(u64::MAX, "sup", &mut m, &s, &mon).unwrap();
        assert_eq!(state(&q, "t"), Some(State::Running));
        assert_eq!(q.get("t").and_then(|t| t.lease_until), Some(u64::MAX));
        // the log is bounded
        let mut q = Queue::default();
        q.submit(task("p", Class::Background, Priority::Normal, 0, StartMode::Now), 0).unwrap();
        for i in 0..(MAX_LOG as u64 + 10) {
            let v = q.get("p").map_or(0, |t| t.version);
            if i % 2 == 0 { q.pause("p", v, i).unwrap() } else { q.resume("p", v, i).unwrap() }
        }
        assert_eq!(q.get("p").map(|t| t.log.len()), Some(MAX_LOG));
    }

    #[test]
    fn transitions_table() {
        assert!(transition(State::Queued, State::Running));
        assert!(!transition(State::Succeeded, State::Queued));
        assert!(!transition(State::Scheduled, State::Running), "a scheduled task is queued first");
        for s in [State::Succeeded, State::Failed, State::Cancelled] {
            assert!(s.is_terminal());
        }
    }

    #[test]
    fn the_queue_is_serde() {
        let s = snap();
        let mut m = manager(&s);
        let mut q = Queue::default();
        q.submit(task("a", Class::FinalRender, Priority::High, 2, StartMode::AtWithResources { at: 5 }), 0).unwrap();
        q.tick(5, "sup", &mut m, &s, &Monitor::default()).unwrap();
        let json = serde_json::to_string(&q).unwrap();
        let back: Queue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, q);
        assert!(json.contains(r#""start":{"mode":"atWithResources","at":5}"#), "{json}");
    }
}
