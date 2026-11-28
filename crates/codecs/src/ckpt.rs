//! Decoder states to restart a video from.
//!
//! A video whose every picture depends on the one before (id RoQ, Interplay MVE) has no sync
//! samples after its first. A decoder that passes a multiple of the spacing keeps a copy of its
//! state here; the sample table then reports those pictures as sync samples, so a seek decodes
//! from the nearest one passed once instead of from the start every time. The first jump forward
//! still decodes from the start (or the last checkpoint). A decoder is free to ignore the store:
//! it restores a state only when it is handed the sample the state was taken before.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Most bytes of states one video keeps.
const BUDGET: usize = 96 << 20;

pub(crate) struct Checkpoints<S> {
    spacing: usize,
    cap: usize,
    map: Mutex<BTreeMap<usize, S>>,
}

impl<S: Clone> Checkpoints<S> {
    /// A store for states of `state_bytes` each, one every `spacing` samples.
    pub(crate) fn new(spacing: usize, state_bytes: usize) -> Self {
        Self { spacing: spacing.max(1), cap: (BUDGET / state_bytes.max(1)).clamp(4, 64), map: Mutex::new(BTreeMap::new()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<usize, S>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The last sample at or before `i` a state is kept for (0, where every decode can start).
    pub(crate) fn sync_before(&self, i: usize) -> usize {
        self.lock().range(..=i).next_back().map_or(0, |(&k, _)| k)
    }

    pub(crate) fn get(&self, i: usize) -> Option<S> {
        self.lock().get(&i).cloned()
    }

    /// Keep `state` (the decoder's state before sample `i`) when `i` is on the spacing and the
    /// store has room.
    pub(crate) fn offer(&self, i: usize, state: &S) {
        if i == 0 || !i.is_multiple_of(self.spacing) {
            return;
        }
        let mut m = self.lock();
        if m.len() < self.cap && !m.contains_key(&i) {
            m.insert(i, state.clone());
        }
    }
}
