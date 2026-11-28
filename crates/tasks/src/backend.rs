//! Where the queue lives: the synchronous [`QueueBackend`] the engine's `TaskHost` talks to.
//!
//! The rules stay in [`Queue`]; this trait is the seam that lets the same host run on the
//! in-memory queue (this crate) or, later, on a PostgreSQL-backed queue that the application
//! wires in from the L5 store (PG-C). Reads return owned values because a backend that lives
//! elsewhere cannot lend references. Every operation can fail with [`QueueError::Unavailable`]
//! when the backend cannot be reached; callers never wait for it on the editing or playback path
//! and never switch to another backend silently.

use crate::manager::{Manager, Request};
use crate::monitor::Monitor;
use crate::queue::{NewTask, Queue, QueueConfig, QueueError, QueueEvent, State, Task, UtcMs};
use crate::resources::Snapshot;

/// The queue operations the engine needs; the same set as the store's async `TaskBackend`.
pub trait QueueBackend: Send {
    fn config(&self) -> QueueConfig;
    /// Every task the queue holds.
    fn tasks(&self) -> Result<Vec<Task>, QueueError>;
    fn get(&self, id: &str) -> Result<Option<Task>, QueueError>;
    fn submit(&mut self, new: NewTask, now: UtcMs) -> Result<Task, QueueError>;
    fn tick(&mut self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor) -> Result<Vec<QueueEvent>, QueueError>;
    fn renew(&mut self, id: &str, owner: &str, now: UtcMs) -> Result<UtcMs, QueueError>;
    fn checkpoint(&mut self, id: &str, owner: &str, stage: u64) -> Result<(), QueueError>;
    fn finish(&mut self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> Result<(), QueueError>;
    fn cancel(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<State, QueueError>;
    fn cancelled(&mut self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> Result<(), QueueError>;
    fn pause(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError>;
    fn resume(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError>;
    fn reschedule(&mut self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> Result<(), QueueError>;
    fn unblock(&mut self, id: &str, expected_version: u64, request: Request, now: UtcMs) -> Result<(), QueueError>;
    /// Drop finished tasks older than `before`; says how many.
    fn prune(&mut self, before: UtcMs) -> Result<usize, QueueError>;
}

/// The in-memory queue: today's behaviour, unchanged.
impl QueueBackend for Queue {
    fn config(&self) -> QueueConfig {
        Queue::config(self).clone()
    }
    fn tasks(&self) -> Result<Vec<Task>, QueueError> {
        Ok(Queue::tasks(self).to_vec())
    }
    fn get(&self, id: &str) -> Result<Option<Task>, QueueError> {
        Ok(Queue::get(self, id).cloned())
    }
    fn submit(&mut self, new: NewTask, now: UtcMs) -> Result<Task, QueueError> {
        Queue::submit(self, new, now).cloned()
    }
    fn tick(&mut self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor) -> Result<Vec<QueueEvent>, QueueError> {
        Queue::tick(self, now, owner, manager, snap, monitor)
    }
    fn renew(&mut self, id: &str, owner: &str, now: UtcMs) -> Result<UtcMs, QueueError> {
        Queue::renew(self, id, owner, now)
    }
    fn checkpoint(&mut self, id: &str, owner: &str, stage: u64) -> Result<(), QueueError> {
        Queue::checkpoint(self, id, owner, stage)
    }
    fn finish(&mut self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> Result<(), QueueError> {
        Queue::finish(self, id, owner, to, now, reason, manager)
    }
    fn cancel(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<State, QueueError> {
        Queue::cancel(self, id, expected_version, now)
    }
    fn cancelled(&mut self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> Result<(), QueueError> {
        Queue::cancelled(self, id, owner, now, manager)
    }
    fn pause(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError> {
        Queue::pause(self, id, expected_version, now)
    }
    fn resume(&mut self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), QueueError> {
        Queue::resume(self, id, expected_version, now)
    }
    fn reschedule(&mut self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> Result<(), QueueError> {
        Queue::reschedule(self, id, expected_version, at, now)
    }
    fn unblock(&mut self, id: &str, expected_version: u64, request: Request, now: UtcMs) -> Result<(), QueueError> {
        Queue::unblock(self, id, expected_version, request, now)
    }
    fn prune(&mut self, before: UtcMs) -> Result<usize, QueueError> {
        Ok(Queue::prune(self, before))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::Class;
    use crate::queue::{MissedStart, Priority, StartMode};

    fn new(id: &str) -> NewTask {
        NewTask {
            id: id.to_owned(),
            request: Request { class: Class::Background, ram_bytes: 0, cpu_milli: 0, gpu: None },
            priority: Priority::Normal,
            start: StartMode::Now,
            missed: MissedStart::RunLate,
            resumable: false,
            time_zone: None,
        }
    }

    #[test]
    fn memory_backend_follows_the_queue_rules_through_the_trait() {
        let mut q: Box<dyn QueueBackend> = Box::new(Queue::new(QueueConfig::default()));
        let t = q.submit(new("a"), 0).expect("submit");
        assert_eq!(t.id, "a");
        assert_eq!(q.get("a").expect("get").map(|t| t.state), Some(t.state));
        assert!(q.get("nope").expect("get").is_none());
        assert!(matches!(q.submit(new("a"), 1), Err(QueueError::DuplicateId { .. })));
        assert!(matches!(q.cancel("a", 99, 2), Err(QueueError::Stale { .. })));
        assert_eq!(q.cancel("a", t.version, 2).expect("cancel"), State::Cancelled);
        assert_eq!(q.tasks().expect("tasks").len(), 1);
        assert_eq!(q.prune(u64::MAX).expect("prune"), 1);
        assert!(q.tasks().expect("tasks").is_empty());
    }
}
