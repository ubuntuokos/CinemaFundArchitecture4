//! Resource availability manager (ACS-2): atomic reservations against the budget and the current
//! load, so CFA4's heavy work never endangers the host's stability.
//!
//! [`Manager::try_reserve`] grants a [`Request`] (RAM, CPU, optionally VRAM on one GPU) **all at
//! once or not at all** (ADR-0008 §3: no "hold and wait", so no deadlock). Every check runs before
//! anything is recorded:
//!
//! 1. **Budget** ([`Budget`], from the nominal resources): a request larger than the whole budget
//!    can never run on this machine as asked → [`Denied::NeverFits`]; the planner (ACS-3) must pick
//!    another validated path. An unknown budget never fits (unknown is never enough).
//! 2. **Ledger:** what other reservations already hold plus this request must stay within the
//!    budget → otherwise [`Denied::Wait`].
//! 3. **Current load** ([`Snapshot`]): other programs use memory and VRAM too, so the free amount
//!    now, minus the reserve, must cover the request → otherwise wait. A free amount that cannot be
//!    read (a cgroup's usage, a GPU without a VRAM counter) waits as well: unknown is never enough.
//!    A grant may not show in the load yet (the task is still allocating), so for
//!    [`DEFAULT_SETTLE_SAMPLES`] samples ([`Manager::observe_sample`]) it is also taken off the
//!    current free amount; two requests checked against the same snapshot never share its room.
//! 4. **Pressure** ([`Monitor`] levels): memory pressure or low memory, spike or sustained, makes
//!    every class but [`Class::Interactive`] wait; sustained CPU or I/O pressure makes
//!    [`Class::Speculative`] wait. A GPU whose VRAM signal is sustained takes no new work.
//!
//! A `Wait` lists every reason and is retried when something changes (a release, a monitor
//! event); the queue (ACS-4) owns the retry. The manager is plain data: the caller keeps it
//! behind one lock.

use cfa4_route::Vendor;
use serde::{Deserialize, Serialize};

use crate::budget::Budget;
use crate::monitor::{Level, Monitor, Signal};
use crate::resources::Snapshot;

/// The task classes of ADR-0008 (P0, P1, P2, R).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Class {
    /// P0: the user is waiting on it (preview, scrubbing).
    Interactive,
    /// P1: started by the user, runs in the background.
    Background,
    /// P2: speculative pre-work that may be thrown away.
    Speculative,
    /// R: the final render / export.
    FinalRender,
}

/// VRAM on one GPU.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuRequest {
    pub vram_bytes: u64,
    /// A specific render node; `None`: any budgeted GPU, the manager picks.
    pub node: Option<String>,
    /// Only GPUs of this vendor (the runtime's: CUDA needs NVIDIA); `None`: any.
    #[serde(default)]
    pub vendor: Option<Vendor>,
}

/// What one task needs, all at once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub class: Class,
    pub ram_bytes: u64,
    /// Thousandths of a CPU.
    pub cpu_milli: u64,
    pub gpu: Option<GpuRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReservationId(pub u64);

/// A granted reservation: release it with [`Manager::release`] when the task ends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub id: ReservationId,
    /// The GPU chosen for the request's VRAM.
    pub gpu_node: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "resource")]
pub enum Resource {
    Ram,
    Cpu,
    /// No budgeted GPU (or not the requested one).
    Gpu {
        node: Option<String>,
    },
    Vram {
        node: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "reason")]
pub enum WaitReason {
    /// Other reservations hold too much of the budget.
    Reserved {
        resource: Resource,
        free_in_budget: u64,
    },
    /// Other programs use it now.
    InUse {
        resource: Resource,
        free_now: u64,
    },
    Pressure {
        signal: Signal,
        level: Level,
    },
    /// The current free amount cannot be read (cgroup usage, VRAM counter): unknown is never
    /// enough, so the request waits until it is known.
    Unknown {
        resource: Resource,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "denied")]
pub enum Denied {
    /// More than the whole budget: will not run on this machine as asked.
    NeverFits { resource: Resource, needed: u64, budget: Option<u64> },
    /// Fits the budget, not now.
    Wait { reasons: Vec<WaitReason> },
    /// Every reservation id is used up (only after 2^64 grants or a hostile restored state);
    /// nothing is granted rather than handing out an id that is already held.
    IdsExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Held {
    id: ReservationId,
    request: Request,
    gpu_node: Option<String>,
    /// Samples still to come before this grant's use is assumed to show in the current load;
    /// until then it is also taken off the current free amount.
    #[serde(default)]
    unsettled_samples: u32,
}

/// Samples after a grant before its use is assumed to show in [`Snapshot`]'s current load.
pub const DEFAULT_SETTLE_SAMPLES: u32 = 3;

/// The manager: budget plus the reservations held against it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manager {
    budget: Budget,
    held: Vec<Held>,
    next_id: u64,
    #[serde(default = "default_settle_samples")]
    settle_samples: u32,
}

fn default_settle_samples() -> u32 {
    DEFAULT_SETTLE_SAMPLES
}

impl Manager {
    pub fn new(budget: Budget) -> Self {
        Manager { budget, held: Vec::new(), next_id: 1, settle_samples: DEFAULT_SETTLE_SAMPLES }
    }

    /// How many samples a new grant stays deducted from the current free amount (the time a task
    /// needs to actually allocate what it reserved). At least 1.
    pub fn set_settle_samples(&mut self, samples: u32) {
        self.settle_samples = samples.max(1);
    }

    /// A new snapshot was taken: grants made before it age by one sample. Call it once per sample,
    /// with the same cadence the [`Monitor`] observes.
    pub fn observe_sample(&mut self) {
        for h in &mut self.held {
            h.unsettled_samples = h.unsettled_samples.saturating_sub(1);
        }
    }

    /// RAM granted recently enough that the current load may not show it yet.
    fn unsettled_ram_bytes(&self) -> u64 {
        self.held.iter().filter(|h| h.unsettled_samples > 0).fold(0u64, |a, h| a.saturating_add(h.request.ram_bytes))
    }

    fn unsettled_vram_bytes(&self, node: &str) -> u64 {
        self.held
            .iter()
            .filter(|h| h.unsettled_samples > 0 && h.gpu_node.as_deref() == Some(node))
            .fold(0u64, |a, h| a.saturating_add(h.request.gpu.as_ref().map_or(0, |g| g.vram_bytes)))
    }

    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    /// A new budget (the nominal resources or the settings changed). Held reservations stay; if
    /// they now exceed the budget, new requests wait until enough are released.
    pub fn set_budget(&mut self, budget: Budget) {
        self.budget = budget;
    }

    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    pub fn reserved_ram_bytes(&self) -> u64 {
        self.held.iter().fold(0u64, |a, h| a.saturating_add(h.request.ram_bytes))
    }

    pub fn reserved_cpu_milli(&self) -> u64 {
        self.held.iter().fold(0u64, |a, h| a.saturating_add(h.request.cpu_milli))
    }

    pub fn reserved_vram_bytes(&self, node: &str) -> u64 {
        self.held.iter().filter(|h| h.gpu_node.as_deref() == Some(node)).fold(0u64, |a, h| a.saturating_add(h.request.gpu.as_ref().map_or(0, |g| g.vram_bytes)))
    }

    /// Grant everything `request` needs, or nothing.
    pub fn try_reserve(&mut self, request: &Request, snap: &Snapshot, monitor: &Monitor) -> Result<Grant, Denied> {
        let gpu_node = self.check(request, snap, monitor)?;
        let id = ReservationId(self.next_id);
        if self.held.iter().any(|h| h.id == id) {
            return Err(Denied::IdsExhausted);
        }
        let Some(next) = self.next_id.checked_add(1) else {
            return Err(Denied::IdsExhausted);
        };
        self.next_id = next;
        self.held.push(Held { id, request: request.clone(), gpu_node: gpu_node.clone(), unsettled_samples: self.settle_samples.max(1) });
        Ok(Grant { id, gpu_node })
    }

    /// What [`Manager::try_reserve`] would answer now, without reserving anything (the planner
    /// compares paths with it). `Ok` carries the GPU that would be chosen.
    pub fn check(&self, request: &Request, snap: &Snapshot, monitor: &Monitor) -> Result<Option<String>, Denied> {
        let mut waits = Vec::new();

        // RAM
        if request.ram_bytes > 0 {
            let budget = self.budget.ram_bytes;
            match budget {
                Some(b) if request.ram_bytes <= b => {
                    let free_in_budget = b.saturating_sub(self.reserved_ram_bytes());
                    if request.ram_bytes > free_in_budget {
                        waits.push(WaitReason::Reserved { resource: Resource::Ram, free_in_budget });
                    }
                    match snap.ram_free_effective_bytes().map(|f| f.saturating_sub(self.budget.ram_reserve_bytes).saturating_sub(self.unsettled_ram_bytes())) {
                        Some(free_now) if request.ram_bytes > free_now => waits.push(WaitReason::InUse { resource: Resource::Ram, free_now }),
                        Some(_) => {}
                        None => waits.push(WaitReason::Unknown { resource: Resource::Ram }),
                    }
                }
                _ => return Err(Denied::NeverFits { resource: Resource::Ram, needed: request.ram_bytes, budget }),
            }
        }

        // CPU
        if request.cpu_milli > 0 {
            let budget = self.budget.cpu_milli;
            match budget {
                Some(b) if request.cpu_milli <= b => {
                    let free_in_budget = b.saturating_sub(self.reserved_cpu_milli());
                    if request.cpu_milli > free_in_budget {
                        waits.push(WaitReason::Reserved { resource: Resource::Cpu, free_in_budget });
                    }
                }
                _ => return Err(Denied::NeverFits { resource: Resource::Cpu, needed: request.cpu_milli, budget }),
            }
        }

        // GPU: among the budgeted candidates that can ever hold it, the one with the most real room
        // (the smaller of what the budget and the card's current use leave)
        let mut gpu_node = None;
        if let Some(gpu) = &request.gpu {
            let candidates: Vec<_> =
                self.budget.gpus.iter().filter(|g| gpu.node.as_ref().is_none_or(|n| *n == g.node) && gpu.vendor.is_none_or(|v| g.vendor == Some(v))).collect();
            if candidates.is_empty() {
                return Err(Denied::NeverFits { resource: Resource::Gpu { node: gpu.node.clone() }, needed: gpu.vram_bytes, budget: None });
            }
            let fitting: Vec<_> = candidates.iter().filter(|g| gpu.vram_bytes <= g.vram_bytes).collect();
            if fitting.is_empty() {
                let largest = candidates.iter().map(|g| g.vram_bytes).max();
                return Err(Denied::NeverFits { resource: Resource::Vram { node: gpu.node.clone() }, needed: gpu.vram_bytes, budget: largest });
            }
            let mut best: Option<(u64, &str)> = None;
            let mut gpu_waits = Vec::new();
            for g in fitting {
                let signal = Signal::Vram(g.node.clone());
                let level = monitor.level(&signal);
                if level == Level::Sustained {
                    gpu_waits.push(WaitReason::Pressure { signal, level });
                    continue;
                }
                let free_in_budget = g.vram_bytes.saturating_sub(self.reserved_vram_bytes(&g.node));
                if gpu.vram_bytes > free_in_budget {
                    gpu_waits.push(WaitReason::Reserved { resource: Resource::Vram { node: Some(g.node.clone()) }, free_in_budget });
                    continue;
                }
                let Some(free_now) =
                    snap.vram_free_bytes(&g.node).map(|f| f.saturating_sub(g.vram_reserve_bytes).saturating_sub(self.unsettled_vram_bytes(&g.node)))
                else {
                    gpu_waits.push(WaitReason::Unknown { resource: Resource::Vram { node: Some(g.node.clone()) } });
                    continue;
                };
                if gpu.vram_bytes > free_now {
                    gpu_waits.push(WaitReason::InUse { resource: Resource::Vram { node: Some(g.node.clone()) }, free_now });
                    continue;
                }
                let room = free_in_budget.min(free_now);
                let better = match best {
                    None => true,
                    Some((best_room, node)) => room > best_room || (room == best_room && g.node.as_str() < node),
                };
                if better {
                    best = Some((room, g.node.as_str()));
                }
            }
            match best {
                Some((_, node)) => gpu_node = Some(node.to_owned()),
                None => waits.extend(gpu_waits),
            }
        }

        // pressure, by class
        if request.class != Class::Interactive {
            for signal in [Signal::MemoryPressure, Signal::LowMemory] {
                let level = monitor.level(&signal);
                if level != Level::Normal {
                    waits.push(WaitReason::Pressure { signal, level });
                }
            }
        }
        if request.class == Class::Speculative {
            for signal in [Signal::CpuPressure, Signal::IoPressure] {
                let level = monitor.level(&signal);
                if level == Level::Sustained {
                    waits.push(WaitReason::Pressure { signal, level });
                }
            }
        }

        if !waits.is_empty() {
            return Err(Denied::Wait { reasons: waits });
        }
        Ok(gpu_node)
    }

    /// Release a reservation; `false` when it was not held (already released, unknown id).
    pub fn release(&mut self, id: ReservationId) -> bool {
        let before = self.held.len();
        self.held.retain(|h| h.id != id);
        self.held.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::BudgetSettings;
    use crate::monitor::MonitorConfig;
    use crate::resources::{Current, GpuCurrent, GpuNominal, Nominal};
    use crate::system::Pressure;
    use cfa4_route::Vendor;

    const GIB: u64 = 1 << 30;

    fn gpu(node: &str, vram_gib: u64, display: bool) -> GpuNominal {
        GpuNominal { node: node.into(), vendor: Vendor::Nvidia, driver: "nvidia".into(), vram_total_bytes: Some(vram_gib * GIB), drives_display: Some(display) }
    }

    /// 32 GiB, 16 CPUs, two 12 GiB compute cards and an 8 GiB display card; `free_gib` RAM free now,
    /// `used_gib` VRAM in use now on renderD129.
    fn machine(free_gib: u64, used_gib: u64) -> Snapshot {
        Snapshot {
            nominal: Nominal {
                ram_physical_bytes: Some(32 * GIB),
                cpus_online: Some(16),
                gpus: vec![gpu("renderD128", 8, true), gpu("renderD129", 12, false), gpu("renderD130", 12, false)],
                ..Nominal::default()
            },
            current: Current {
                ram_available_bytes: Some(free_gib * GIB),
                gpus: vec![
                    GpuCurrent { node: "renderD128".into(), vram_used_bytes: Some(GIB), busy_percent: None },
                    GpuCurrent { node: "renderD129".into(), vram_used_bytes: Some(used_gib * GIB), busy_percent: None },
                    GpuCurrent { node: "renderD130".into(), vram_used_bytes: Some(0), busy_percent: None },
                ],
                ..Current::default()
            },
            notes: vec![],
        }
    }

    fn manager(snap: &Snapshot) -> Manager {
        Manager::new(Budget::new(&snap.nominal, &BudgetSettings::default()))
    }

    fn req(class: Class, ram_gib: u64, cpus: u64, vram_gib: Option<u64>) -> Request {
        Request { class, ram_bytes: ram_gib * GIB, cpu_milli: cpus * 1000, gpu: vram_gib.map(|v| GpuRequest { vram_bytes: v * GIB, node: None, vendor: None }) }
    }

    #[test]
    fn grants_are_atomic_and_spread_over_the_gpus() {
        let snap = machine(28, 0);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        // budget: RAM 32 − 3.2 GiB reserve, CPU 15, two 11.5 GiB compute cards (the display card is out)
        let a = m.try_reserve(&req(Class::Background, 8, 4, Some(10)), &snap, &mon).unwrap();
        let b = m.try_reserve(&req(Class::Background, 8, 4, Some(10)), &snap, &mon).unwrap();
        assert_eq!((a.gpu_node.as_deref(), b.gpu_node.as_deref()), (Some("renderD129"), Some("renderD130")));
        // RAM and CPU would fit, VRAM not: nothing is held for it (no hold and wait)
        let c = m.try_reserve(&req(Class::Background, 4, 2, Some(10)), &snap, &mon);
        assert!(matches!(c, Err(Denied::Wait { .. })), "{c:?}");
        assert_eq!((m.held_count(), m.reserved_ram_bytes(), m.reserved_cpu_milli()), (2, 16 * GIB, 8000));
        assert!(m.release(a.id));
        assert!(!m.release(a.id), "a second release is a no-op");
        let c = m.try_reserve(&req(Class::Background, 4, 2, Some(10)), &snap, &mon).unwrap();
        assert_eq!(c.gpu_node.as_deref(), Some("renderD129"));
    }

    #[test]
    fn never_fits_is_told_apart_from_wait() {
        let snap = machine(28, 0);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        assert_eq!(
            m.try_reserve(&req(Class::Background, 0, 0, Some(16)), &snap, &mon),
            Err(Denied::NeverFits { resource: Resource::Vram { node: None }, needed: 16 * GIB, budget: Some(12 * GIB - 512 * (1 << 20)) })
        );
        assert!(matches!(m.try_reserve(&req(Class::Background, 64, 0, None), &snap, &mon), Err(Denied::NeverFits { resource: Resource::Ram, .. })));
        assert!(matches!(m.try_reserve(&req(Class::Background, 0, 32, None), &snap, &mon), Err(Denied::NeverFits { resource: Resource::Cpu, .. })));
        let display =
            Request { gpu: Some(GpuRequest { vram_bytes: GIB, node: Some("renderD128".into()), vendor: None }), ..req(Class::Background, 0, 0, None) };
        assert!(
            matches!(m.try_reserve(&display, &snap, &mon), Err(Denied::NeverFits { resource: Resource::Gpu { .. }, .. })),
            "the display card has no budget"
        );
        let mut unknown = Manager::new(Budget::default());
        assert!(matches!(unknown.try_reserve(&req(Class::Interactive, 1, 0, None), &snap, &mon), Err(Denied::NeverFits { budget: None, .. })));
        assert_eq!(unknown.try_reserve(&req(Class::Interactive, 0, 0, None), &snap, &mon).map(|g| g.gpu_node), Ok(None), "an empty request needs nothing");
    }

    #[test]
    fn other_programs_current_use_counts() {
        // only 6 GiB RAM free now, of which the 3.2 GiB reserve stays free; renderD129 has 10 GiB in use by someone else
        let snap = machine(6, 10);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        let r = m.try_reserve(&req(Class::Background, 4, 1, None), &snap, &mon);
        assert!(matches!(&r, Err(Denied::Wait { reasons }) if matches!(reasons.as_slice(), [WaitReason::InUse { resource: Resource::Ram, .. }])), "{r:?}");
        let pinned =
            Request { gpu: Some(GpuRequest { vram_bytes: 4 * GIB, node: Some("renderD129".into()), vendor: None }), ..req(Class::Background, 1, 1, None) };
        let r = m.try_reserve(&pinned, &snap, &mon);
        assert!(
            matches!(&r, Err(Denied::Wait { reasons }) if matches!(reasons.as_slice(), [WaitReason::InUse { resource: Resource::Vram { .. }, .. }])),
            "{r:?}"
        );
        let any = m.try_reserve(&req(Class::Background, 1, 1, Some(4)), &snap, &mon).unwrap();
        assert_eq!(any.gpu_node.as_deref(), Some("renderD130"), "the busy card is skipped");
    }

    #[test]
    fn pressure_holds_back_by_class() {
        let mut snap = machine(28, 0);
        let mut m = manager(&snap);
        let mut mon = Monitor::new(MonitorConfig { sustain_samples: 2, ..MonitorConfig::default() });
        mon.observe(&snap);
        snap.current.memory_pressure = Some(Pressure { some_avg10_bp: 2000, full_avg10_bp: 0 });
        mon.observe(&snap); // spike
        assert!(matches!(m.try_reserve(&req(Class::Background, 1, 1, None), &snap, &mon), Err(Denied::Wait { .. })), "background waits out a spike");
        assert!(m.try_reserve(&req(Class::Interactive, 1, 1, None), &snap, &mon).is_ok(), "the user is waiting: interactive goes");
        snap.current.memory_pressure = Some(Pressure::default());
        snap.current.cpu_pressure = Some(Pressure { some_avg10_bp: 9000, full_avg10_bp: 0 });
        mon.observe(&snap);
        mon.observe(&snap);
        mon.observe(&snap); // memory cleared, CPU sustained
        assert!(m.try_reserve(&req(Class::FinalRender, 1, 1, None), &snap, &mon).is_ok(), "CPU pressure only slows: the render goes");
        let r = m.try_reserve(&req(Class::Speculative, 1, 1, None), &snap, &mon);
        assert_eq!(r, Err(Denied::Wait { reasons: vec![WaitReason::Pressure { signal: Signal::CpuPressure, level: Level::Sustained }] }));
    }

    #[test]
    fn a_gpu_with_sustained_vram_pressure_takes_no_new_work() {
        let mut snap = machine(28, 0);
        let mut m = manager(&snap);
        let mut mon = Monitor::new(MonitorConfig { sustain_samples: 1, ..MonitorConfig::default() });
        if let Some(g) = snap.current.gpus.iter_mut().find(|g| g.node == "renderD130") {
            g.vram_used_bytes = Some(11 * GIB);
        }
        mon.observe(&snap);
        mon.observe(&snap);
        let pinned =
            Request { gpu: Some(GpuRequest { vram_bytes: GIB / 4, node: Some("renderD130".into()), vendor: None }), ..req(Class::Interactive, 0, 0, None) };
        let r = m.try_reserve(&pinned, &snap, &mon);
        assert!(matches!(&r, Err(Denied::Wait { reasons }) if matches!(reasons.first(), Some(WaitReason::Pressure { .. }))), "{r:?}");
    }

    #[test]
    fn hostile_requests_never_panic_or_overflow() {
        let snap = machine(28, 0);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        let huge = Request {
            class: Class::Background,
            ram_bytes: u64::MAX,
            cpu_milli: u64::MAX,
            gpu: Some(GpuRequest { vram_bytes: u64::MAX, node: None, vendor: None }),
        };
        assert!(matches!(m.try_reserve(&huge, &snap, &mon), Err(Denied::NeverFits { .. })));
        let mut m = Manager { next_id: u64::MAX - 1, ..manager(&snap) };
        let a = m.try_reserve(&req(Class::Interactive, 0, 0, None), &snap, &mon).unwrap();
        assert_eq!(a.id, ReservationId(u64::MAX - 1));
        assert_eq!(m.try_reserve(&req(Class::Interactive, 0, 0, None), &snap, &mon), Err(Denied::IdsExhausted), "never wraps, never repeats");
        assert_eq!(m.held_count(), 1);
        // a restored state whose counter points at a held id: refused, not duplicated
        m.next_id = a.id.0;
        assert_eq!(m.try_reserve(&req(Class::Interactive, 0, 0, None), &snap, &mon), Err(Denied::IdsExhausted));
        assert!(m.release(a.id));
        assert!(m.try_reserve(&req(Class::Interactive, 0, 0, None), &snap, &mon).is_ok());
    }

    #[test]
    fn recent_grants_are_taken_off_the_current_free_amount() {
        // 14 GiB free now minus the 3.2 GiB reserve: one 8 GiB task fits, a second one against the
        // same snapshot must not (budget alone would allow it); renderD130 alone, 11.5 GiB free
        let snap = machine(14, 12);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        let a = m.try_reserve(&req(Class::Background, 8, 1, Some(8)), &snap, &mon).unwrap();
        assert_eq!(a.gpu_node.as_deref(), Some("renderD130"));
        let r = m.try_reserve(&req(Class::Background, 1, 1, Some(8)), &snap, &mon);
        assert!(
            matches!(&r, Err(Denied::Wait { reasons }) if reasons.iter().any(|w| matches!(w, WaitReason::InUse { resource: Resource::Vram { .. }, .. }))),
            "{r:?}"
        );
        let r = m.try_reserve(&req(Class::Background, 8, 1, None), &snap, &mon);
        assert!(matches!(&r, Err(Denied::Wait { reasons }) if matches!(reasons.as_slice(), [WaitReason::InUse { resource: Resource::Ram, .. }])), "{r:?}");
        // once the samples show the task's own use, it is no longer counted twice
        let mut later = machine(6, 12);
        if let Some(g) = later.current.gpus.iter_mut().find(|g| g.node == "renderD130") {
            g.vram_used_bytes = Some(8 * GIB);
        }
        for _ in 0..DEFAULT_SETTLE_SAMPLES {
            m.observe_sample();
        }
        assert!(m.try_reserve(&req(Class::Background, 2, 1, Some(3)), &later, &mon).is_ok());
    }

    #[test]
    fn unknown_current_use_waits() {
        // a cgroup limit whose usage cannot be read: free RAM is unknown, not the limit
        let mut snap = machine(28, 0);
        snap.nominal.ram_cgroup_limit_bytes = Some(32 * GIB);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        let r = m.try_reserve(&req(Class::Interactive, 1, 1, None), &snap, &mon);
        assert_eq!(r, Err(Denied::Wait { reasons: vec![WaitReason::Unknown { resource: Resource::Ram }] }));
        // a GPU whose VRAM use is unknown gets no work
        let mut snap = machine(28, 0);
        for g in &mut snap.current.gpus {
            g.vram_used_bytes = None;
        }
        let r = m.try_reserve(&req(Class::Interactive, 1, 1, Some(1)), &snap, &mon);
        assert!(
            matches!(&r, Err(Denied::Wait { reasons }) if reasons.len() == 2 && reasons.iter().all(|w| matches!(w, WaitReason::Unknown { resource: Resource::Vram { .. } }))),
            "{r:?}"
        );
        assert_eq!(m.held_count(), 0);
    }

    #[test]
    fn the_gpu_with_the_most_real_room_wins() {
        // renderD129 has 9 GiB in use by another program: its budget room is larger on paper
        // than renderD130's after a 2 GiB reservation there, but its real room is smaller
        let snap = machine(28, 9);
        let mut m = manager(&snap);
        let mon = Monitor::default();
        let pin =
            Request { gpu: Some(GpuRequest { vram_bytes: 2 * GIB, node: Some("renderD130".into()), vendor: None }), ..req(Class::Background, 0, 0, None) };
        m.try_reserve(&pin, &snap, &mon).unwrap();
        let any = m.try_reserve(&req(Class::Background, 0, 0, Some(1)), &snap, &mon).unwrap();
        assert_eq!(any.gpu_node.as_deref(), Some("renderD130"));
    }

    #[test]
    fn the_manager_and_denials_are_serde() {
        let snap = machine(28, 0);
        let mut m = manager(&snap);
        let _ = m.try_reserve(&req(Class::FinalRender, 1, 1, Some(1)), &snap, &Monitor::default());
        let json = serde_json::to_string(&m).unwrap();
        let back: Manager = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        let d = Denied::Wait { reasons: vec![WaitReason::Reserved { resource: Resource::Vram { node: Some("renderD129".into()) }, free_in_budget: 5 }] };
        assert_eq!(
            serde_json::to_string(&d).unwrap(),
            r#"{"denied":"wait","reasons":[{"reason":"reserved","resource":{"resource":"vram","node":"renderD129"},"freeInBudget":5}]}"#
        );
    }
}
