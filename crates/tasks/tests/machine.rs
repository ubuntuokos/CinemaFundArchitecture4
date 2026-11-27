//! ACS-7 on a real machine (ignored by default; `scripts/dev/acs7-stability.sh` runs it inside
//! the reference-machine limits). It takes memory itself, in steps, up to at most 60 % of what is
//! free, and checks that the readings and the resource manager follow: the budget comes from the
//! cgroup limit, the free amount drops with the load, a request that no longer fits waits instead
//! of starting, and everything comes back once the memory is released. Under the cgroup limit the
//! host is never at risk.

use std::time::Duration;

use cfa4_tasks::budget::{Budget, BudgetSettings};
use cfa4_tasks::manager::{Class, Denied, Request, Resource, WaitReason};
use cfa4_tasks::{Manager, Monitor, sample};

const MIB: u64 = 1 << 20;
const CHUNK: usize = 256 << 20;

fn ram_in_use(denied: &Denied) -> bool {
    matches!(denied, Denied::Wait { reasons } if reasons.iter().any(|r| matches!(r, WaitReason::InUse { resource: Resource::Ram, .. })))
}

#[test]
#[ignore = "takes real memory; run through scripts/dev/acs7-stability.sh"]
fn the_budget_follows_real_memory_use_on_this_machine() {
    let before = sample();
    let (Some(effective), Some(free)) = (before.nominal.ram_effective_bytes(), before.ram_free_effective_bytes()) else {
        eprintln!("acs7: RAM cannot be read here; skipped");
        return;
    };
    if let Ok(limit) = std::env::var("CFA4_EXPECT_RAM_LIMIT_MIB").map(|v| v.parse::<u64>().unwrap_or(0)) {
        assert!(effective <= limit * MIB, "the cgroup limit ({limit} MiB) is the effective RAM, got {} MiB", effective / MIB);
    }
    let manager = Manager::new(Budget::new(&before.nominal, &BudgetSettings::default()));
    let monitor = Monitor::default();
    let request = Request { class: Class::Background, ram_bytes: free / 2, cpu_milli: 0, gpu: None };
    let first = manager.check(&request, &before, &monitor);
    assert!(!first.as_ref().is_err_and(ram_in_use), "half of the free memory fits at first: {first:?}");

    // take 60 % of the free memory, touching every page
    let target = usize::try_from(free / 10 * 6).unwrap_or(usize::MAX);
    let mut held: Vec<Vec<u8>> = Vec::new();
    while held.len() * CHUNK < target {
        held.push(vec![1u8; CHUNK]);
    }
    let loaded = sample();
    let free_loaded = loaded.ram_free_effective_bytes().unwrap_or(0);
    eprintln!(
        "acs7: effective {} MiB, free {} MiB, holding {} MiB, free now {} MiB",
        effective / MIB,
        free / MIB,
        (held.len() * CHUNK) >> 20,
        free_loaded / MIB
    );
    assert!(free_loaded + (target as u64) / 2 < free, "the load shows in the free memory");
    let denied = manager.check(&request, &loaded, &monitor);
    assert!(denied.as_ref().is_err_and(ram_in_use), "it waits for memory instead of starting: {denied:?}");

    drop(held);
    std::thread::sleep(Duration::from_millis(500));
    let after = sample();
    assert!(after.ram_free_effective_bytes().unwrap_or(0) > free_loaded, "the memory came back");
    let again = manager.check(&request, &after, &monitor);
    assert!(!again.as_ref().is_err_and(ram_in_use), "and the request fits again: {again:?}");
}
