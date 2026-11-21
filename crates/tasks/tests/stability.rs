//! ACS-7 stability run: thousands of random queue operations against a machine whose free memory
//! and pressure keep changing (background load), with leases lapsing as if CFA4 had stopped. After
//! every step the invariants that keep work safe must hold:
//!
//! - the reservations never exceed the budget (RAM, CPU, every GPU's VRAM);
//! - a reservation is held exactly by the running and cancelling tasks, one each;
//! - a task that cannot resume is never started a second time;
//! - nothing panics, whatever the caller sends (stale versions, wrong owners, unknown ids).

use cfa4_route::Vendor;
use cfa4_tasks::budget::{Budget, BudgetSettings};
use cfa4_tasks::manager::{Class, GpuRequest, Request};
use cfa4_tasks::resources::{Current, GpuCurrent, GpuNominal, Nominal};
use cfa4_tasks::{Manager, MissedStart, Monitor, NewTask, Priority, Queue, QueueConfig, Snapshot, StartMode, State};

const GIB: u64 = 1 << 30;

/// A small deterministic generator (the run is the same every time).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn machine(free_ram: Option<u64>, vram_used: Option<u64>, psi_some_bp: Option<u32>) -> Snapshot {
    let mut current = Current {
        ram_available_bytes: free_ram,
        gpus: vec![GpuCurrent { node: "renderD128".into(), vram_used_bytes: vram_used, busy_percent: None }],
        ..Current::default()
    };
    if let Some(bp) = psi_some_bp {
        current.memory_pressure = Some(cfa4_tasks::Pressure { some_avg10_bp: bp, full_avg10_bp: bp / 2 });
    }
    Snapshot {
        nominal: Nominal {
            ram_physical_bytes: Some(16 * GIB),
            cpus_online: Some(8),
            gpus: vec![GpuNominal {
                node: "renderD128".into(),
                vendor: Vendor::Nvidia,
                driver: "nvidia".into(),
                vram_total_bytes: Some(12 * GIB),
                drives_display: Some(false),
            }],
            ..Nominal::default()
        },
        current,
        notes: vec![],
    }
}

fn check(q: &Queue, m: &Manager, step: usize) {
    let b = m.budget();
    assert!(b.ram_bytes.is_some_and(|r| m.reserved_ram_bytes() <= r), "step {step}: RAM over budget");
    assert!(b.cpu_milli.is_some_and(|c| m.reserved_cpu_milli() <= c), "step {step}: CPU over budget");
    for g in &b.gpus {
        assert!(m.reserved_vram_bytes(&g.node) <= g.vram_bytes, "step {step}: VRAM over budget on {}", g.node);
    }
    let holders = q.tasks().iter().filter(|t| t.reservation.is_some()).count();
    let active = q.tasks().iter().filter(|t| matches!(t.state, State::Running | State::Cancelling)).count();
    assert_eq!(holders, m.held_count(), "step {step}: reservations and holders differ");
    assert_eq!(
        holders,
        active,
        "step {step}: a running task without a reservation or the other way round: {:?}",
        q.tasks()
            .iter()
            .filter(|t| t.reservation.is_some())
            .map(|t| (&t.id, t.state, t.log.iter().map(|l| (l.from, l.to, l.reason.clone())).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
    for t in q.tasks() {
        if !t.resumable {
            assert!(t.attempts <= 1, "step {step}: {} started twice", t.id);
        }
        if t.reservation.is_some() {
            assert!(matches!(t.state, State::Running | State::Cancelling), "step {step}: {} holds a reservation in {:?}", t.id, t.state);
        }
    }
}

#[test]
fn thousands_of_random_operations_keep_the_invariants() {
    for seed in [0x00C0_FFEE, 1, 2, 3, 0xACE7, 0xDEAD_BEEF, 42, 7_777_777] {
        run(seed);
    }
}

fn run(seed: u64) {
    let mut rng = Lcg(seed);
    let mut q = Queue::new(QueueConfig::default());
    let base = machine(Some(14 * GIB), Some(0), None);
    let mut m = Manager::new(Budget::new(&base.nominal, &BudgetSettings::default()));
    let mut mon = Monitor::default();
    let mut now: u64 = 1_000;
    let mut submitted = 0u32;
    let (mut started, mut finished) = (0usize, 0usize);
    let owners = ["sup-a", "sup-b"];
    // a longer soak on the reference machine: CFA4_ACS7_STEPS=1000000 (scripts/dev/acs7-stability.sh)
    let steps: usize = std::env::var("CFA4_ACS7_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000);
    for step in 0..steps {
        // background load: other programs take memory and VRAM, sometimes unreadable, sometimes under pressure
        let free = match rng.below(10) {
            0 => None,
            _ => Some((6 + rng.below(9)) * GIB),
        };
        let vram = if rng.below(10) == 0 { None } else { Some(rng.below(6) * GIB) };
        let psi = (rng.below(8) == 0).then(|| u32::try_from(rng.below(6_000)).unwrap_or(0));
        let snap = machine(free, vram, psi);
        mon.observe(&snap);
        m.observe_sample();

        match rng.below(12) {
            0..=2 => {
                submitted += 1;
                let class = [Class::Interactive, Class::FinalRender, Class::Background, Class::Speculative][rng.below(4) as usize];
                // now and then one that never fits this machine
                let scale = if rng.below(10) == 0 { 4 } else { 1 };
                let gpu = (rng.below(3) == 0).then(|| GpuRequest { vram_bytes: rng.below(5) * scale * GIB, node: None, vendor: None });
                let new = NewTask {
                    id: format!("t{submitted}"),
                    request: Request { class, ram_bytes: rng.below(4) * scale * GIB, cpu_milli: rng.below(3_000) * scale, gpu },
                    priority: [Priority::Low, Priority::Normal, Priority::High][rng.below(3) as usize],
                    start: if rng.below(4) == 0 { StartMode::At { at: now + rng.below(120_000) } } else { StartMode::Now },
                    missed: [MissedStart::RunLate, MissedStart::NextSlot, MissedStart::AskUser][rng.below(3) as usize],
                    resumable: rng.below(2) == 0,
                    time_zone: [None, Some("Europe/Budapest".to_owned()), Some("America/New_York".to_owned())][rng.below(3) as usize].clone(),
                };
                let _ = q.submit(new, now);
            }
            3..=5 => {
                let owner = owners[rng.below(2) as usize];
                if let Ok(events) = q.tick(now, owner, &mut m, &snap, &mon) {
                    started += events.iter().filter(|e| matches!(e, cfa4_tasks::QueueEvent::Started { .. })).count();
                }
            }
            n => {
                // act on a random task, often with the wrong owner or a stale version
                // half the time one that runs, as a worker would
                let running: Vec<usize> = (0..q.tasks().len()).filter(|&i| q.tasks().get(i).is_some_and(|t| t.state == State::Running)).collect();
                let pick = match running.get(rng.below(running.len() as u64) as usize) {
                    Some(&i) if rng.below(2) == 0 => i,
                    _ => rng.below(q.tasks().len() as u64) as usize,
                };
                let Some(t) = q.tasks().get(pick).cloned() else { continue };
                let owner = t.lease_owner.clone().filter(|_| rng.below(4) != 0).unwrap_or_else(|| "intruder".into());
                let version = if rng.below(4) == 0 { t.version.wrapping_add(1) } else { t.version };
                let r = match n {
                    6 => q.renew(&t.id, &owner, now).map(|_| ()),
                    7 => q.checkpoint(&t.id, &owner, rng.below(100)),
                    8 => q.finish(&t.id, &owner, [State::Succeeded, State::Failed, State::Paused][rng.below(3) as usize], now, "worker", &mut m),
                    9 => q.cancel(&t.id, version, now).map(|_| ()),
                    10 => q.cancelled(&t.id, &owner, now, &mut m),
                    _ => match rng.below(3) {
                        0 => q.pause(&t.id, version, now),
                        1 => q.resume(&t.id, version, now),
                        _ => q.reschedule(&t.id, version, now + 60_000, now),
                    },
                };
                if n == 8 && r.is_ok() {
                    finished += 1;
                }
            }
        }
        // time passes; now and then far enough for every lease to lapse (CFA4 stopped)
        now += if rng.below(50) == 0 { 120_000 } else { rng.below(3_000) };
        if step % 500 == 499 {
            q.prune(now.saturating_sub(10 * 60_000));
        }
        check(&q, &m, step);
    }
    assert!(steps < 5_000 || submitted > 1_000, "the run exercised the queue");
    assert!(steps < 5_000 || (started > 100 && finished > 50), "and work started ({started}) and ended ({finished})");
}
