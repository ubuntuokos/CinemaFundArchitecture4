//! The engine's task queue, written behind to PostgreSQL (PG-001 §6).
//!
//! The engine keeps its queue in memory and never waits for the database. The application hands
//! it a [`SessionMirror`] as its [`TaskJournal`]: every task change goes into a bounded channel
//! (a full channel drops the change and counts it, the next change of that task rewrites the whole
//! document anyway), and a thread of its own writes batches to `tasks.session_task`. These rows are
//! a record for other tools and for the next start; the database scheduler (`tasks.task`) never
//! reads them. A session that died leaves open rows: the next start closes them as `interrupted`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cfa4_tasks::{Task, TaskJournal};
use serde_json::Value;

use crate::tasks::state_name;
use crate::{Store, StoreConfig, StoreError};

#[cfg(test)]
mod tests;

/// Changes waiting for the writer thread.
const QUEUE: usize = 1024;
/// Most rows written in one transaction.
const BATCH: usize = 128;
/// Rows of ended sessions are kept this long.
pub const KEEP_DAYS: u32 = 30;
/// States of a task that has not ended.
const OPEN: [&str; 7] = ["scheduled", "queued", "waiting", "running", "cancelling", "paused", "blocked"];

/// The session rows of a store.
pub struct SessionTasks {
    store: Store,
}

/// One row, as read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub state: String,
    pub version: u64,
}

impl Store {
    /// The record of the engine's in-memory tasks.
    pub fn session_tasks(&self) -> SessionTasks {
        SessionTasks { store: self.clone() }
    }
}

impl SessionTasks {
    /// Write `rows` (task id, state, version, document) of `session`; an older version never
    /// replaces a newer one.
    pub async fn write(&self, session: &str, rows: &[(String, String, u64, Value)]) -> Result<(), StoreError> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::query)?;
        for (id, state, version, doc) in rows {
            let version = i64::try_from(*version).unwrap_or(i64::MAX);
            tx.execute(
                "INSERT INTO tasks.session_task (session, id, state, version, doc) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (session, id) DO UPDATE SET state = EXCLUDED.state, version = EXCLUDED.version, doc = EXCLUDED.doc, updated_at = now() WHERE tasks.session_task.version <= EXCLUDED.version",
                &[&session, id, state, &version, doc],
            )
            .await
            .map_err(StoreError::query)?;
        }
        tx.commit().await.map_err(StoreError::query)
    }

    /// The rows of `session`, by id.
    pub async fn list(&self, session: &str) -> Result<Vec<SessionRow>, StoreError> {
        let client = self.store.client().await?;
        let rows =
            client.query("SELECT id, state, version FROM tasks.session_task WHERE session = $1 ORDER BY id", &[&session]).await.map_err(StoreError::query)?;
        rows.iter()
            .map(|r| {
                let version: i64 = r.try_get("version").map_err(StoreError::query)?;
                Ok(SessionRow {
                    id: r.try_get("id").map_err(StoreError::query)?,
                    state: r.try_get("state").map_err(StoreError::query)?,
                    version: u64::try_from(version).unwrap_or(0),
                })
            })
            .collect()
    }

    /// Close the open tasks of every session other than `current` for which `alive` says no:
    /// they died with their process. Returns how many rows were closed.
    pub async fn close_dead(&self, current: &str, alive: impl Fn(&str) -> bool) -> Result<u64, StoreError> {
        let client = self.store.client().await?;
        let open: Vec<String> = OPEN.iter().map(|s| (*s).to_owned()).collect();
        let rows = client
            .query("SELECT DISTINCT session FROM tasks.session_task WHERE session <> $1 AND state = ANY($2)", &[&current, &open])
            .await
            .map_err(StoreError::query)?;
        let mut closed = 0;
        for r in rows {
            let session: String = r.try_get(0).map_err(StoreError::query)?;
            if alive(&session) {
                continue;
            }
            closed += client
                .execute("UPDATE tasks.session_task SET state = 'interrupted', updated_at = now() WHERE session = $1 AND state = ANY($2)", &[&session, &open])
                .await
                .map_err(StoreError::query)?;
        }
        Ok(closed)
    }

    /// Delete rows not written for `days` days.
    pub async fn prune(&self, days: u32) -> Result<u64, StoreError> {
        let client = self.store.client().await?;
        let days = i32::try_from(days).unwrap_or(i32::MAX);
        client.execute("DELETE FROM tasks.session_task WHERE updated_at < now() - make_interval(days => $1)", &[&days]).await.map_err(StoreError::query)
    }
}

/// A session id that a later start can check for life: `<pid>-<start ms>`.
pub fn new_session_id() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("{}-{ms}", std::process::id())
}

/// Whether the process of a [`new_session_id`] still runs on this machine (Linux `/proc`).
pub fn session_alive(session: &str) -> bool {
    session.split('-').next().and_then(|p| p.parse::<u32>().ok()).is_none_or(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
}

type Row = (String, String, u64, Value);

/// The journal the engine writes into: changes go to a thread that writes them to PostgreSQL.
pub struct SessionMirror {
    tx: SyncSender<Row>,
    dropped: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    session: String,
    writer: Option<std::thread::JoinHandle<()>>,
}

impl SessionMirror {
    /// Connect, bring the schema up to date and start the writer thread; waits at most
    /// `startup` for that. An error means there is no mirror and the engine goes on without one.
    pub fn start(config: StoreConfig, startup: Duration) -> Result<SessionMirror, StoreError> {
        let session = new_session_id();
        let (tx, rx) = sync_channel::<Row>(QUEUE);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), StoreError>>();
        let failed = Arc::new(AtomicU64::new(0));
        let (thread_session, thread_failed) = (session.clone(), failed.clone());
        let writer = std::thread::Builder::new()
            .name("cfa4-task-mirror".into())
            .spawn(move || {
                // the writer must never take the process down
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(config, thread_session, rx, ready_tx, thread_failed)));
            })
            .map_err(StoreError::unavailable)?;
        match ready_rx.recv_timeout(startup) {
            Ok(Ok(())) => Ok(SessionMirror { tx, dropped: Arc::new(AtomicU64::new(0)), failed, session, writer: Some(writer) }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(StoreError::unavailable(std::io::Error::new(std::io::ErrorKind::TimedOut, "the task mirror did not start"))),
        }
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    /// Changes dropped because the writer could not keep up.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Batches the database refused.
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }
}

impl TaskJournal for SessionMirror {
    fn record(&mut self, task: &Task) {
        let Ok(doc) = serde_json::to_value(task) else { return };
        if self.tx.try_send((task.id.clone(), state_name(task.state), task.version, doc)).is_err() && self.dropped.fetch_add(1, Ordering::Relaxed) == 0 {
            log::warn!("task mirror: the database writer is behind; changes are dropped until it catches up");
        }
    }
}

impl Drop for SessionMirror {
    fn drop(&mut self) {
        // closing the channel lets the writer write what is left and end; wait for it briefly
        let (dead, _) = sync_channel(1);
        drop(std::mem::replace(&mut self.tx, dead));
        if let Some(h) = self.writer.take() {
            let start = std::time::Instant::now();
            while !h.is_finished() && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn run(config: StoreConfig, session: String, rx: Receiver<Row>, ready: std::sync::mpsc::Sender<Result<(), StoreError>>, failed: Arc<AtomicU64>) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ready.send(Err(StoreError::unavailable(e)));
            return;
        }
    };
    let opened = rt.block_on(async {
        let store = Store::connect(&config).await?;
        store.migrate().await?;
        let tasks = store.session_tasks();
        match tasks.close_dead(&session, session_alive).await {
            Ok(n) if n > 0 => log::info!("task mirror: {n} task(s) of an ended session closed as interrupted"),
            Ok(_) => {}
            Err(e) => log::warn!("task mirror: could not close the tasks of ended sessions: {e}"),
        }
        if let Err(e) = tasks.prune(KEEP_DAYS).await {
            log::warn!("task mirror: could not prune old rows: {e}");
        }
        Ok::<_, StoreError>(tasks)
    });
    let tasks = match opened {
        Ok(t) => {
            let _ = ready.send(Ok(()));
            t
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let last_error = Mutex::new(String::new());
    loop {
        // wait for a change, then take what has piled up
        let first = match rx.recv_timeout(Duration::from_secs(3600)) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut batch = vec![first];
        let mut closed = false;
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(r) => batch.push(r),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if let Err(e) = rt.block_on(tasks.write(&session, &batch)) {
            failed.fetch_add(1, Ordering::Relaxed);
            // say it once per kind of error, not once per batch
            let text = e.to_string();
            let mut last = last_error.lock().unwrap_or_else(PoisonError::into_inner);
            if *last != text {
                log::warn!("task mirror: could not write {} change(s): {text}", batch.len());
                *last = text;
            }
        }
        if closed {
            return;
        }
    }
}
