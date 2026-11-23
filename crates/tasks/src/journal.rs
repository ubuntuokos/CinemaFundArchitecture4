//! The write-behind journal of a task queue (PG-001 §6).
//!
//! The engine keeps its queue in memory, so editing, playback and saving never wait on a
//! database (`AGENTS.md` §0). A [`TaskJournal`] is told, after the fact, about every task whose
//! state moved, and the application decides where that goes (the PostgreSQL store, a log, nothing).
//! The contract is that `record` never blocks and never fails the caller: a journal that cannot
//! keep up drops, and says so itself.

use crate::queue::Task;

/// Told about every task the in-memory queue changed.
pub trait TaskJournal: Send {
    /// `task` is the whole task as it is now (state, version, transitions). Called once per change,
    /// in the order the changes happened. Must return quickly.
    fn record(&mut self, task: &Task);
}
