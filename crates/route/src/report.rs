//! The routing decisions of this process, for `perf.stats` and the control channel.
//!
//! The crates that route their work (`cfa4-platform` for the hardware decoder and encoder) publish
//! the chain they chose and the device inventory here; `cfa4-engine`'s `perf.stats` reads it back
//! together with [`crate::coverage`]. Process-wide, bounded, and a poisoned lock is recovered: the
//! report is diagnostics and never stops work.

use std::collections::BTreeMap;
use std::sync::{OnceLock, PoisonError, RwLock};

use serde::Serialize;

use crate::{Chain, Inventory, TaskId};

/// Most tasks the report keeps.
pub const MAX_TASKS: usize = 256;

#[derive(Default)]
struct State {
    chains: BTreeMap<TaskId, Chain>,
    inventory: Option<Inventory>,
}

fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(State::default()))
}

/// Records the chain of a task (replacing an earlier one of the same task). Past [`MAX_TASKS`]
/// tasks a new task is dropped.
pub fn publish(chain: Chain) {
    let mut s = state().write().unwrap_or_else(PoisonError::into_inner);
    if s.chains.len() < MAX_TASKS || s.chains.contains_key(&chain.task) {
        s.chains.insert(chain.task.clone(), chain);
    }
}

/// Records the device inventory the chains were chosen on.
pub fn publish_inventory(inventory: Inventory) {
    state().write().unwrap_or_else(PoisonError::into_inner).inventory = Some(inventory);
}

/// What `perf.stats` shows under `route`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// The chain of every routed task, by task id.
    pub chains: Vec<Chain>,
    /// The devices found, `None` before anything was routed.
    pub inventory: Option<Inventory>,
    /// The vendor-neutrality report of the workspace.
    pub coverage: Vec<crate::Coverage>,
}

/// The published chains and inventory, with the coverage report.
pub fn snapshot() -> Snapshot {
    let s = state().read().unwrap_or_else(PoisonError::into_inner);
    Snapshot { chains: s.chains.values().cloned().collect(), inventory: s.inventory.clone(), coverage: crate::coverage() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackendId, DeviceId, Route};

    #[test]
    fn chains_are_kept_per_task_and_bounded() {
        let chain = |task: &str, backend: &str| Chain {
            task: TaskId::new(task).unwrap(),
            routes: vec![Route { backend: BackendId::new(backend).unwrap(), device: DeviceId::new("cpu").unwrap() }],
            declined: vec![],
        };
        publish(chain("report.test.a", "software"));
        publish(chain("report.test.a", "vaapi"));
        let snap = snapshot();
        let a: Vec<_> = snap.chains.iter().filter(|c| c.task.as_str() == "report.test.a").collect();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].routes[0].backend.as_str(), "vaapi", "the later chain replaces the earlier");
        assert_eq!(snap.coverage.len(), crate::Family::ALL.len());

        for i in 0..(MAX_TASKS * 2) {
            publish(chain(&format!("report.test.many{i}"), "cpu"));
        }
        assert!(snapshot().chains.len() <= MAX_TASKS);
        publish_inventory(Inventory::default());
        assert!(snapshot().inventory.is_some());
    }
}
