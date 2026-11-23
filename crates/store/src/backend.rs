//! The storage trait of the task queue, and its in-memory implementation.
//!
//! [`TaskBackend`] is what the scheduler and the workers call. [`TaskStore`](crate::TaskStore)
//! implements it on PostgreSQL; [`MemoryTaskStore`] implements it on a [`cfa4_tasks::Queue`] held
//! in memory, with exactly the same rules. The memory store needs no server: cutting and playback
//! never depend on the database (`AGENTS.md` §0), tests run without PostgreSQL, and a machine
//! without the CFA4 cluster still has a working queue (it just does not survive a restart).

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use cfa4_tasks::{Manager, Monitor, NewTask, Queue, QueueConfig, QueueError, QueueEvent, Request, Snapshot, State, Task, UtcMs};

use crate::{StoreError, TaskStore};

/// The operations on a task queue that survives, or does not survive, a restart. Every method is
/// one atomic step: it happens completely or not at all.
pub trait TaskBackend: Send + Sync {
    /// Add a task (`scheduled` or `queued`).
    fn submit(&self, new: NewTask, now: UtcMs) -> impl Future<Output = Result<Task, StoreError>> + Send;
    /// One scheduler step ([`Queue::tick`]).
    fn tick(
        &self,
        now: UtcMs,
        owner: &str,
        manager: &mut Manager,
        snap: &Snapshot,
        monitor: &Monitor,
    ) -> impl Future<Output = Result<Vec<QueueEvent>, StoreError>> + Send;
    fn get(&self, id: &str) -> impl Future<Output = Result<Option<Task>, StoreError>> + Send;
    fn renew(&self, id: &str, owner: &str, now: UtcMs) -> impl Future<Output = Result<UtcMs, StoreError>> + Send;
    fn checkpoint(&self, id: &str, owner: &str, stage: u64) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn finish(&self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn cancel(&self, id: &str, expected_version: u64, now: UtcMs) -> impl Future<Output = Result<State, StoreError>> + Send;
    fn cancelled(&self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn pause(&self, id: &str, expected_version: u64, now: UtcMs) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn resume(&self, id: &str, expected_version: u64, now: UtcMs) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn reschedule(&self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn unblock(&self, id: &str, expected_version: u64, request: Request, now: UtcMs) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// The queue in memory: the reference implementation of [`TaskBackend`].
pub struct MemoryTaskStore {
    queue: Mutex<Queue>,
}

impl MemoryTaskStore {
    pub fn new(config: QueueConfig) -> Self {
        MemoryTaskStore { queue: Mutex::new(Queue::new(config)) }
    }

    /// Run one operation on a copy and keep the copy only when it succeeded, so a refused
    /// operation leaves the queue as it was (the same all-or-nothing rule as a rolled-back
    /// transaction). A poisoned lock is recovered: the state it guards is only ever replaced whole.
    fn run<R>(&self, op: impl FnOnce(&mut Queue) -> Result<R, QueueError>) -> Result<R, StoreError> {
        let mut guard = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let mut scratch = Queue::from_tasks(guard.config().clone(), guard.tasks().to_vec());
        let result = op(&mut scratch)?;
        *guard = scratch;
        Ok(result)
    }

    /// Like [`Self::run`], and the resource manager changes on a copy that replaces the caller's
    /// only when the operation succeeded.
    fn run_managed<R>(&self, manager: &mut Manager, op: impl FnOnce(&mut Queue, &mut Manager) -> Result<R, QueueError>) -> Result<R, StoreError> {
        let mut scratch = manager.clone();
        let result = self.run(|q| op(q, &mut scratch))?;
        *manager = scratch;
        Ok(result)
    }
}

impl TaskBackend for MemoryTaskStore {
    async fn submit(&self, new: NewTask, now: UtcMs) -> Result<Task, StoreError> {
        self.run(|q| q.submit(new, now).cloned())
    }

    async fn tick(&self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor) -> Result<Vec<QueueEvent>, StoreError> {
        self.run_managed(manager, |q, m| q.tick(now, owner, m, snap, monitor))
    }

    async fn get(&self, id: &str) -> Result<Option<Task>, StoreError> {
        let guard = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(guard.get(id).cloned())
    }

    async fn renew(&self, id: &str, owner: &str, now: UtcMs) -> Result<UtcMs, StoreError> {
        self.run(|q| q.renew(id, owner, now))
    }

    async fn checkpoint(&self, id: &str, owner: &str, stage: u64) -> Result<(), StoreError> {
        self.run(|q| q.checkpoint(id, owner, stage))
    }

    async fn finish(&self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> Result<(), StoreError> {
        self.run_managed(manager, |q, m| q.finish(id, owner, to, now, reason, m))
    }

    async fn cancel(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<State, StoreError> {
        self.run(|q| q.cancel(id, expected_version, now))
    }

    async fn cancelled(&self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> Result<(), StoreError> {
        self.run_managed(manager, |q, m| q.cancelled(id, owner, now, m))
    }

    async fn pause(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), StoreError> {
        self.run(|q| q.pause(id, expected_version, now))
    }

    async fn resume(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), StoreError> {
        self.run(|q| q.resume(id, expected_version, now))
    }

    async fn reschedule(&self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> Result<(), StoreError> {
        self.run(|q| q.reschedule(id, expected_version, at, now))
    }

    async fn unblock(&self, id: &str, expected_version: u64, request: Request, now: UtcMs) -> Result<(), StoreError> {
        self.run(|q| q.unblock(id, expected_version, request, now))
    }
}

impl TaskBackend for TaskStore {
    async fn submit(&self, new: NewTask, now: UtcMs) -> Result<Task, StoreError> {
        TaskStore::submit(self, new, now).await
    }
    async fn tick(&self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor) -> Result<Vec<QueueEvent>, StoreError> {
        TaskStore::tick(self, now, owner, manager, snap, monitor).await
    }
    async fn get(&self, id: &str) -> Result<Option<Task>, StoreError> {
        TaskStore::get(self, id).await
    }
    async fn renew(&self, id: &str, owner: &str, now: UtcMs) -> Result<UtcMs, StoreError> {
        TaskStore::renew(self, id, owner, now).await
    }
    async fn checkpoint(&self, id: &str, owner: &str, stage: u64) -> Result<(), StoreError> {
        TaskStore::checkpoint(self, id, owner, stage).await
    }
    async fn finish(&self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> Result<(), StoreError> {
        TaskStore::finish(self, id, owner, to, now, reason, manager).await
    }
    async fn cancel(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<State, StoreError> {
        TaskStore::cancel(self, id, expected_version, now).await
    }
    async fn cancelled(&self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> Result<(), StoreError> {
        TaskStore::cancelled(self, id, owner, now, manager).await
    }
    async fn pause(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), StoreError> {
        TaskStore::pause(self, id, expected_version, now).await
    }
    async fn resume(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), StoreError> {
        TaskStore::resume(self, id, expected_version, now).await
    }
    async fn reschedule(&self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> Result<(), StoreError> {
        TaskStore::reschedule(self, id, expected_version, at, now).await
    }
    async fn unblock(&self, id: &str, expected_version: u64, request: Request, now: UtcMs) -> Result<(), StoreError> {
        TaskStore::unblock(self, id, expected_version, request, now).await
    }
}
