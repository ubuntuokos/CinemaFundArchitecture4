//! The `tasks` schema (PG-2): the `cfa4-tasks` queue kept in PostgreSQL.
//!
//! Every operation is one transaction: lock the rows it needs, run the queue's own rule on them
//! ([`cfa4_tasks::Queue`]), then write back the tasks whose version changed, their new
//! transitions and one outbox event per transition, and commit. The resource manager is changed
//! on a copy that replaces the caller's only after the commit, so a failed commit leaves both the
//! database and the reservations as they were.

use std::collections::HashMap;

use cfa4_tasks::manager::Class;
use cfa4_tasks::{Manager, Monitor, NewTask, Priority, Queue, QueueConfig, QueueError, QueueEvent, Snapshot, State, Task, UtcMs};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

use crate::{Store, StoreError};

/// At most this many open tasks are locked by one scheduler step.
const MAX_LOCKED: i64 = cfa4_tasks::queue::MAX_TASKS as i64;

/// The NOTIFY channel the scheduler listens on for new or changed tasks.
pub const CHANNEL: &str = "cfa4_tasks";

fn class_rank(class: Class) -> i16 {
    match class {
        Class::Interactive => 0,
        Class::FinalRender => 1,
        Class::Background => 2,
        Class::Speculative => 3,
    }
}

fn priority_rank(p: Priority) -> i16 {
    match p {
        Priority::Low => 0,
        Priority::Normal => 1,
        Priority::High => 2,
    }
}

pub(crate) fn state_name(s: State) -> String {
    match serde_json::to_value(s) {
        Ok(Value::String(name)) => name,
        _ => format!("{s:?}"),
    }
}

fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// The tasks of a store.
pub struct TaskStore {
    store: Store,
    config: QueueConfig,
}

/// What a task looked like when it was loaded.
struct Before {
    version: u64,
    last_seq: Option<u64>,
}

impl TaskStore {
    pub(crate) fn new(store: Store, config: QueueConfig) -> Self {
        TaskStore { store, config }
    }

    async fn load(tx: &Transaction<'_>, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Result<Vec<Task>, StoreError> {
        let rows = tx.query(sql, params).await.map_err(StoreError::query)?;
        let mut tasks = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id").map_err(StoreError::query)?;
            let doc: Value = row.try_get("doc").map_err(|e| StoreError::Corrupt { id: id.clone(), detail: e.to_string() })?;
            let task: Task = serde_json::from_value(doc).map_err(|e| StoreError::Corrupt { id: id.clone(), detail: e.to_string() })?;
            if task.id != id {
                return Err(StoreError::Corrupt { id, detail: format!("document carries id {}", task.id) });
            }
            tasks.push(task);
        }
        Ok(tasks)
    }

    /// Write back what changed since `before`, with the new transitions and outbox events.
    async fn save(tx: &Transaction<'_>, before: &HashMap<String, Before>, tasks: &[Task], events: &[QueueEvent]) -> Result<(), StoreError> {
        for t in tasks {
            let Some(b) = before.get(&t.id) else {
                continue;
            };
            if b.version == t.version {
                continue;
            }
            let doc = serde_json::to_value(t).map_err(StoreError::query)?;
            let n = tx
                .execute(
                    "UPDATE tasks.task SET state = $2, class_rank = $3, priority = $4, start_at = to_timestamp($5::bigint / 1000.0), lease_owner = $6, lease_until = to_timestamp($7::bigint / 1000.0), version = $8, doc = $9, updated_at = now() WHERE id = $1 AND version = $10",
                    &[
                        &t.id,
                        &state_name(t.state),
                        &class_rank(t.request.class),
                        &priority_rank(t.priority),
                        &start_at(t),
                        &t.lease_owner,
                        &t.lease_until.map(ms),
                        &ms(t.version),
                        &doc,
                        &ms(b.version),
                    ],
                )
                .await
                .map_err(StoreError::query)?;
            if n != 1 {
                // the row is locked, so this means someone bypassed the lock: refuse, roll back
                return Err(StoreError::Queue { error: QueueError::Stale { id: t.id.clone(), expected: b.version, current: t.version } });
            }
            Self::write_schedule(tx, t).await?;
            // the scheduler wakes on every state change, after the commit
            tx.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &t.id]).await.map_err(StoreError::query)?;
            Self::log(tx, t, b.last_seq).await?;
        }
        for e in events {
            let event = serde_json::to_value(e).map_err(StoreError::query)?;
            let id = event.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
            // a scheduler decision (waiting, blocked, missed start, ...) is progress of the job
            let payload = json!({ "taskId": id, "queueEvent": event });
            crate::outbox::emit(tx, &id, "cfa4.execution.job.progress", &payload).await?;
        }
        Ok(())
    }

    /// Keep `tasks.schedule` equal to the task's start rule (it changes on `reschedule` and when a
    /// missed start moves to the next slot).
    async fn write_schedule(tx: &Transaction<'_>, t: &Task) -> Result<(), StoreError> {
        let (mode, at) = match t.start {
            cfa4_tasks::StartMode::Now => ("now", None),
            cfa4_tasks::StartMode::WhenResources => ("resource", None),
            cfa4_tasks::StartMode::At { at } => ("at", Some(ms(at))),
            cfa4_tasks::StartMode::AtWithResources { at } => ("at_resource", Some(ms(at))),
        };
        let if_missed = match t.missed {
            cfa4_tasks::MissedStart::RunLate => "delay",
            cfa4_tasks::MissedStart::NextSlot => "next_slot",
            cfa4_tasks::MissedStart::AskUser => "ask",
        };
        tx.execute(
            "INSERT INTO tasks.schedule (task_id, mode, start_at, time_zone, if_missed) VALUES ($1, $2, to_timestamp($3::bigint / 1000.0), $4, $5) \
             ON CONFLICT (task_id) DO UPDATE SET mode = EXCLUDED.mode, start_at = EXCLUDED.start_at, time_zone = EXCLUDED.time_zone, if_missed = EXCLUDED.if_missed",
            &[&t.id, &mode, &at, &t.time_zone, &if_missed],
        )
        .await
        .map_err(StoreError::query)?;
        Ok(())
    }

    async fn log(tx: &Transaction<'_>, t: &Task, after: Option<u64>) -> Result<(), StoreError> {
        for l in t.log.iter().filter(|l| after.is_none_or(|a| l.seq > a)) {
            let (from, to) = (state_name(l.from), state_name(l.to));
            let by = match serde_json::to_value(l.by) {
                Ok(Value::String(s)) => s,
                _ => String::new(),
            };
            tx.execute(
                "INSERT INTO tasks.transition (task_id, seq, from_state, to_state, at, by_actor, reason) VALUES ($1, $2, $3, $4, to_timestamp($5::bigint / 1000.0), $6, $7)",
                &[&t.id, &ms(l.seq), &from, &to, &ms(l.at), &by, &l.reason],
            )
            .await
            .map_err(StoreError::query)?;
            // the detailed state and the task's own sequence number ride in the payload (D-3, Q10)
            let payload = json!({ "taskId": t.id, "state": to, "from": from, "to": to, "entity_seq": l.seq, "at": l.at, "version": t.version });
            crate::outbox::emit(tx, &t.id, crate::outbox::subject_for(&to), &payload).await?;
        }
        Ok(())
    }

    fn snapshot(tasks: &[Task]) -> HashMap<String, Before> {
        tasks.iter().map(|t| (t.id.clone(), Before { version: t.version, last_seq: t.log.last().map(|l| l.seq) })).collect()
    }

    /// Run `op` on one locked task.
    async fn one<R>(
        &self,
        id: &str,
        manager: Option<&mut Manager>,
        op: impl FnOnce(&mut Queue, Option<&mut Manager>) -> Result<R, QueueError>,
    ) -> Result<R, StoreError> {
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::query)?;
        let tasks = Self::load(&tx, "SELECT id, doc FROM tasks.task WHERE id = $1 FOR UPDATE", &[&id]).await?;
        if tasks.is_empty() {
            return Err(QueueError::NotFound { id: id.to_owned() }.into());
        }
        let before = Self::snapshot(&tasks);
        let mut queue = Queue::from_tasks(self.config.clone(), tasks);
        // the manager changes on a copy that replaces the caller's only after the commit
        let mut scratch = manager.as_deref().cloned();
        let r = op(&mut queue, scratch.as_mut())?;
        Self::save(&tx, &before, queue.tasks(), &[]).await?;
        tx.commit().await.map_err(StoreError::query)?;
        if let (Some(m), Some(s)) = (manager, scratch) {
            *m = s;
        }
        Ok(r)
    }

    /// Add a task (`scheduled` or `queued`) and wake the scheduler.
    pub async fn submit(&self, new: NewTask, now: UtcMs) -> Result<Task, StoreError> {
        let mut queue = Queue::new(self.config.clone());
        let task = queue.submit(new, now)?.clone();
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::query)?;
        let open: i64 = tx.query_one("SELECT count(*) FROM tasks.task", &[]).await.and_then(|r| r.try_get(0)).map_err(StoreError::query)?;
        if open >= MAX_LOCKED {
            return Err(QueueError::Full.into());
        }
        let doc = serde_json::to_value(&task).map_err(StoreError::query)?;
        let n = tx
            .execute(
                "INSERT INTO tasks.task (id, state, class_rank, priority, submitted_at, start_at, version, doc) VALUES ($1, $2, $3, $4, to_timestamp($5::bigint / 1000.0), to_timestamp($6::bigint / 1000.0), $7, $8) ON CONFLICT (id) DO NOTHING",
                &[&task.id, &state_name(task.state), &class_rank(task.request.class), &priority_rank(task.priority), &ms(task.submitted_at), &start_at(&task), &ms(task.version), &doc],
            )
            .await
            .map_err(StoreError::query)?;
        if n != 1 {
            return Err(QueueError::DuplicateId { id: task.id }.into());
        }
        Self::write_schedule(&tx, &task).await?;
        // the job exists: `created` goes out with the commit (the task's own sequence starts at 0)
        let state = state_name(task.state);
        let payload =
            json!({ "taskId": task.id, "state": state, "from": Value::Null, "to": state, "entity_seq": 0, "at": task.submitted_at, "version": task.version });
        crate::outbox::emit(&tx, &task.id, crate::outbox::subject_for(&state), &payload).await?;
        tx.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &task.id]).await.map_err(StoreError::query)?;
        tx.commit().await.map_err(StoreError::query)?;
        Ok(task)
    }

    /// One scheduler step (see [`Queue::tick`]): locks the open tasks no other scheduler holds
    /// (`SKIP LOCKED`), and starts, waits, blocks or expires them in one transaction.
    pub async fn tick(&self, now: UtcMs, owner: &str, manager: &mut Manager, snap: &Snapshot, monitor: &Monitor) -> Result<Vec<QueueEvent>, StoreError> {
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::query)?;
        let tasks = Self::load(
            &tx,
            "SELECT id, doc FROM tasks.task WHERE state IN ('scheduled', 'queued', 'waiting', 'running', 'cancelling') ORDER BY class_rank, priority DESC, submitted_at LIMIT $1 FOR UPDATE SKIP LOCKED",
            &[&MAX_LOCKED],
        )
        .await?;
        let before = Self::snapshot(&tasks);
        let mut queue = Queue::from_tasks(self.config.clone(), tasks);
        let mut scratch = manager.clone();
        let events = queue.tick(now, owner, &mut scratch, snap, monitor)?;
        Self::save(&tx, &before, queue.tasks(), &events).await?;
        tx.commit().await.map_err(StoreError::query)?;
        *manager = scratch;
        Ok(events)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Task>, StoreError> {
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::query)?;
        let mut tasks = Self::load(&tx, "SELECT id, doc FROM tasks.task WHERE id = $1", &[&id]).await?;
        Ok(tasks.pop())
    }

    /// Renew a running task's lease (no state change, no new version).
    pub async fn renew(&self, id: &str, owner: &str, now: UtcMs) -> Result<UtcMs, StoreError> {
        self.in_place(id, |q| q.renew(id, owner, now), None).await
    }

    /// Record a finished stage (no state change, no new version).
    pub async fn checkpoint(&self, id: &str, owner: &str, stage: u64) -> Result<(), StoreError> {
        self.checkpoint_keyed(id, owner, stage, None).await
    }

    /// Record a finished stage together with the cache key of its result (`tasks.checkpoint`, one
    /// row per stage; a stage recorded twice keeps its first row, so a repeated stage is harmless).
    pub async fn checkpoint_keyed(&self, id: &str, owner: &str, stage: u64, cache_key: Option<&str>) -> Result<(), StoreError> {
        if let Some(k) = cache_key {
            crate::check_text("cache key", k, 1, 256)?;
        }
        self.in_place(id, |q| q.checkpoint(id, owner, stage), Some((stage, cache_key))).await
    }

    /// Run `op` on one locked task that changes neither its state nor its version, and write the
    /// task back.
    async fn in_place<R>(&self, id: &str, op: impl FnOnce(&mut Queue) -> Result<R, QueueError>, stage: Option<(u64, Option<&str>)>) -> Result<R, StoreError> {
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::query)?;
        let tasks = Self::load(&tx, "SELECT id, doc FROM tasks.task WHERE id = $1 FOR UPDATE", &[&id]).await?;
        let mut queue = Queue::from_tasks(self.config.clone(), tasks);
        let r = op(&mut queue)?;
        let task = queue.get(id).ok_or_else(|| StoreError::from(QueueError::NotFound { id: id.to_owned() }))?;
        let doc = serde_json::to_value(task).map_err(StoreError::query)?;
        tx.execute(
            "UPDATE tasks.task SET lease_until = to_timestamp($2::bigint / 1000.0), doc = $3, updated_at = now() WHERE id = $1",
            &[&id, &task.lease_until.map(ms), &doc],
        )
        .await
        .map_err(StoreError::query)?;
        if let Some((stage, cache_key)) = stage {
            tx.execute(
                "INSERT INTO tasks.checkpoint (task_id, stage, cache_key) VALUES ($1, $2, $3) ON CONFLICT (task_id, stage) DO NOTHING",
                &[&id, &ms(stage), &cache_key],
            )
            .await
            .map_err(StoreError::query)?;
        }
        tx.commit().await.map_err(StoreError::query)?;
        Ok(r)
    }

    pub async fn finish(&self, id: &str, owner: &str, to: State, now: UtcMs, reason: &str, manager: &mut Manager) -> Result<(), StoreError> {
        self.one(id, Some(manager), |q, m| match m {
            Some(m) => q.finish(id, owner, to, now, reason, m),
            None => Err(QueueError::Invalid { reason: "no manager".into() }),
        })
        .await
    }

    pub async fn cancel(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<State, StoreError> {
        self.one(id, None, |q, _| q.cancel(id, expected_version, now)).await
    }

    pub async fn cancelled(&self, id: &str, owner: &str, now: UtcMs, manager: &mut Manager) -> Result<(), StoreError> {
        self.one(id, Some(manager), |q, m| match m {
            Some(m) => q.cancelled(id, owner, now, m),
            None => Err(QueueError::Invalid { reason: "no manager".into() }),
        })
        .await
    }

    pub async fn pause(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), StoreError> {
        self.one(id, None, |q, _| q.pause(id, expected_version, now)).await
    }

    pub async fn resume(&self, id: &str, expected_version: u64, now: UtcMs) -> Result<(), StoreError> {
        self.one(id, None, |q, _| q.resume(id, expected_version, now)).await
    }

    pub async fn reschedule(&self, id: &str, expected_version: u64, at: UtcMs, now: UtcMs) -> Result<(), StoreError> {
        self.one(id, None, |q, _| q.reschedule(id, expected_version, at, now)).await
    }

    pub async fn unblock(&self, id: &str, expected_version: u64, request: cfa4_tasks::Request, now: UtcMs) -> Result<(), StoreError> {
        self.one(id, None, |q, _| q.unblock(id, expected_version, request, now)).await
    }
}

fn start_at(t: &Task) -> Option<i64> {
    match t.start {
        cfa4_tasks::StartMode::At { at } | cfa4_tasks::StartMode::AtWithResources { at } => Some(ms(at)),
        cfa4_tasks::StartMode::Now | cfa4_tasks::StartMode::WhenResources => None,
    }
}

mod history;
pub use history::{EstimateBasis, EstimateRecord, RunRecord};

#[cfg(test)]
mod tests;
