//! Runtime resource monitor (ACS-1): turns a series of [`Snapshot`]s into events.
//!
//! - **Nominal changes** (a GPU appears or disappears, a cgroup limit or a card's VRAM changes)
//!   are reported once, when they happen.
//! - **Current load** is watched per signal (memory, CPU and I/O pressure, low memory, each GPU's
//!   VRAM) with hysteresis, so a short spike is told apart from a lasting limit: one sample over
//!   the raise threshold is a [`Level::Spike`]; `sustain_samples` samples in a row make it
//!   [`Level::Sustained`]; it clears only after it stays below the lower clear threshold for as
//!   many samples. A spike that falls back clears at once. Each change of level is one event, so
//!   a planner (ACS-3) waits out spikes and plans around sustained limits.
//!
//! The monitor is a pure state machine: the caller samples ([`crate::sample`]) at its own pace
//! (the resource manager in ACS-2 owns the timer), so tests drive it with synthetic snapshots.
//! An unknown value leaves the signal's state as it is.

use serde::{Deserialize, Serialize};

use crate::resources::{Nominal, Snapshot};

/// Basis points: hundredths of a percent, 10 000 = 100 %.
pub type Bp = u32;
const FULL: Bp = 10_000;

/// What is watched.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "node")]
pub enum Signal {
    /// PSI memory `some avg10`.
    MemoryPressure,
    /// PSI cpu `some avg10`.
    CpuPressure,
    /// PSI io `some avg10`.
    IoPressure,
    /// Share of the effective RAM in use (cgroup limit or physical, see [`Snapshot::ram_free_effective_bytes`]).
    LowMemory,
    /// Share of a GPU's VRAM in use, by render node.
    Vram(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Normal,
    /// Over the threshold, not yet for long: wait before acting on it.
    Spike,
    /// Over the threshold for `sustain_samples` samples: a real limit, plan around it.
    Sustained,
}

/// A change of the nominal resources.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum NominalChange {
    RamPhysical { from: Option<u64>, to: Option<u64> },
    RamLimit { from: Option<u64>, to: Option<u64> },
    RamLimitUnknown { from: bool, to: bool },
    CpusOnline { from: Option<u32>, to: Option<u32> },
    CpusAllowed { from: Option<u32>, to: Option<u32> },
    CpuQuota { from: Option<u64>, to: Option<u64> },
    GpuAdded { node: String },
    GpuRemoved { node: String },
    GpuVram { node: String, from: Option<u64>, to: Option<u64> },
    GpuDisplay { node: String, from: Option<bool>, to: Option<bool> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event")]
pub enum Event {
    Nominal {
        changes: Vec<NominalChange>,
    },
    /// A signal changed level; `value_bp` is the sample that changed it.
    Level {
        signal: Signal,
        from: Level,
        to: Level,
        value_bp: Bp,
    },
}

/// Raise thresholds in basis points; a signal clears below `raise × (1 − hysteresis)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MonitorConfig {
    pub memory_pressure_bp: Bp,
    pub cpu_pressure_bp: Bp,
    pub io_pressure_bp: Bp,
    /// Raise when this share of the effective RAM is in use (9000 = less than 10 % free).
    pub low_memory_used_bp: Bp,
    pub vram_used_bp: Bp,
    /// How far below the raise threshold a signal must fall to clear, as a share of it.
    pub hysteresis_bp: Bp,
    /// Samples in a row that make a spike sustained, and that clear a sustained signal.
    pub sustain_samples: u32,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        MonitorConfig {
            memory_pressure_bp: 1_000,
            cpu_pressure_bp: 5_000,
            io_pressure_bp: 2_000,
            low_memory_used_bp: 9_000,
            vram_used_bp: 9_000,
            hysteresis_bp: 2_500,
            sustain_samples: 3,
        }
    }
}

#[derive(Clone, Debug)]
struct State {
    signal: Signal,
    level: Level,
    /// Consecutive samples over (Spike) or under (Sustained) the relevant threshold.
    run: u32,
}

/// The monitor. Feed it every sample; it returns what changed.
#[derive(Clone, Debug, Default)]
pub struct Monitor {
    config: MonitorConfig,
    last_nominal: Option<Nominal>,
    states: Vec<State>,
}

impl Monitor {
    pub fn new(config: MonitorConfig) -> Self {
        Monitor { config, last_nominal: None, states: Vec::new() }
    }

    /// The current level of a signal (`Normal` when never raised).
    pub fn level(&self, signal: &Signal) -> Level {
        self.states.iter().find(|s| s.signal == *signal).map_or(Level::Normal, |s| s.level)
    }

    /// Observe one sample. The first sample only sets the nominal baseline.
    pub fn observe(&mut self, snap: &Snapshot) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some(last) = &self.last_nominal {
            let changes = nominal_changes(last, &snap.nominal);
            if !changes.is_empty() {
                events.push(Event::Nominal { changes });
            }
        }
        // a GPU that is gone cannot stay raised: drop its state silently, its removal is reported
        self.states.retain(|s| match &s.signal {
            Signal::Vram(node) => snap.nominal.gpu(node).is_some(),
            _ => true,
        });
        self.last_nominal = Some(snap.nominal.clone());
        let c = self.config.clone();
        let cur = &snap.current;
        let mut values: Vec<(Signal, Option<Bp>, Bp)> = vec![
            (Signal::MemoryPressure, cur.memory_pressure.map(|p| p.some_avg10_bp), c.memory_pressure_bp),
            (Signal::CpuPressure, cur.cpu_pressure.map(|p| p.some_avg10_bp), c.cpu_pressure_bp),
            (Signal::IoPressure, cur.io_pressure.map(|p| p.some_avg10_bp), c.io_pressure_bp),
            (Signal::LowMemory, used_share(snap.nominal.ram_effective_bytes(), snap.ram_free_effective_bytes()), c.low_memory_used_bp),
        ];
        for gpu in &snap.nominal.gpus {
            values.push((Signal::Vram(gpu.node.clone()), used_share(gpu.vram_total_bytes, snap.vram_free_bytes(&gpu.node)), c.vram_used_bp));
        }
        for (signal, value, raise) in values {
            let Some(value) = value else {
                continue;
            };
            if let Some(event) = self.step(signal, value, raise) {
                events.push(event);
            }
        }
        events
    }

    fn step(&mut self, signal: Signal, value: Bp, raise: Bp) -> Option<Event> {
        let sustain = self.config.sustain_samples.max(1);
        let clear = clear_threshold(raise, self.config.hysteresis_bp);
        let index = match self.states.iter().position(|s| s.signal == signal) {
            Some(i) => i,
            None => {
                self.states.push(State { signal: signal.clone(), level: Level::Normal, run: 0 });
                self.states.len().saturating_sub(1)
            }
        };
        let state = self.states.get_mut(index)?;
        let from = state.level;
        match state.level {
            Level::Normal if value >= raise => {
                state.run = 1;
                state.level = if sustain <= 1 { Level::Sustained } else { Level::Spike };
            }
            Level::Normal => {}
            Level::Spike if value < clear => {
                state.level = Level::Normal;
                state.run = 0;
            }
            Level::Spike => {
                state.run = state.run.saturating_add(1);
                if state.run >= sustain {
                    state.level = Level::Sustained;
                    state.run = 0;
                }
            }
            Level::Sustained if value < clear => {
                state.run = state.run.saturating_add(1);
                if state.run >= sustain {
                    state.level = Level::Normal;
                    state.run = 0;
                }
            }
            Level::Sustained => state.run = 0,
        }
        (state.level != from).then_some(Event::Level { signal, from, to: state.level, value_bp: value })
    }
}

fn clear_threshold(raise: Bp, hysteresis: Bp) -> Bp {
    let cut = u64::from(raise).saturating_mul(u64::from(hysteresis.min(FULL))) / u64::from(FULL);
    raise.saturating_sub(u32::try_from(cut).unwrap_or(raise))
}

/// Used share in basis points from a total and what is free of it; `None` when unknown or the
/// total is zero.
fn used_share(total: Option<u64>, free: Option<u64>) -> Option<Bp> {
    let total = total?;
    if total == 0 {
        return None;
    }
    let used = total.saturating_sub(free?.min(total));
    let bp = u128::from(used).saturating_mul(u128::from(FULL)) / u128::from(total);
    u32::try_from(bp).ok()
}

fn nominal_changes(a: &Nominal, b: &Nominal) -> Vec<NominalChange> {
    let mut out = Vec::new();
    if a.ram_physical_bytes != b.ram_physical_bytes {
        out.push(NominalChange::RamPhysical { from: a.ram_physical_bytes, to: b.ram_physical_bytes });
    }
    if a.ram_cgroup_limit_bytes != b.ram_cgroup_limit_bytes {
        out.push(NominalChange::RamLimit { from: a.ram_cgroup_limit_bytes, to: b.ram_cgroup_limit_bytes });
    }
    if a.ram_cgroup_limit_unknown != b.ram_cgroup_limit_unknown {
        out.push(NominalChange::RamLimitUnknown { from: a.ram_cgroup_limit_unknown, to: b.ram_cgroup_limit_unknown });
    }
    if a.cpus_online != b.cpus_online {
        out.push(NominalChange::CpusOnline { from: a.cpus_online, to: b.cpus_online });
    }
    if a.cpus_allowed != b.cpus_allowed {
        out.push(NominalChange::CpusAllowed { from: a.cpus_allowed, to: b.cpus_allowed });
    }
    if a.cpu_quota_milli != b.cpu_quota_milli {
        out.push(NominalChange::CpuQuota { from: a.cpu_quota_milli, to: b.cpu_quota_milli });
    }
    for old in &a.gpus {
        match b.gpu(&old.node) {
            None => out.push(NominalChange::GpuRemoved { node: old.node.clone() }),
            Some(new) => {
                if old.vram_total_bytes != new.vram_total_bytes {
                    out.push(NominalChange::GpuVram { node: old.node.clone(), from: old.vram_total_bytes, to: new.vram_total_bytes });
                }
                if old.drives_display != new.drives_display {
                    out.push(NominalChange::GpuDisplay { node: old.node.clone(), from: old.drives_display, to: new.drives_display });
                }
            }
        }
    }
    for new in &b.gpus {
        if a.gpu(&new.node).is_none() {
            out.push(NominalChange::GpuAdded { node: new.node.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::{Current, GpuCurrent, GpuNominal};
    use crate::system::Pressure;
    use cfa4_route::Vendor;

    const GIB: u64 = 1 << 30;

    fn snap(mem_pressure_bp: Option<u32>, free_gib: Option<u64>, vram_used_gib: Option<u64>) -> Snapshot {
        Snapshot {
            nominal: Nominal {
                ram_physical_bytes: Some(32 * GIB),
                cpus_online: Some(16),
                gpus: vec![GpuNominal {
                    node: "renderD128".into(),
                    vendor: Vendor::Amd,
                    driver: "amdgpu".into(),
                    vram_total_bytes: Some(12 * GIB),
                    drives_display: Some(false),
                }],
                ..Nominal::default()
            },
            current: Current {
                ram_available_bytes: free_gib.map(|g| g * GIB),
                memory_pressure: mem_pressure_bp.map(|b| Pressure { some_avg10_bp: b, full_avg10_bp: 0 }),
                gpus: vec![GpuCurrent { node: "renderD128".into(), vram_used_bytes: vram_used_gib.map(|g| g * GIB), busy_percent: None }],
                ..Current::default()
            },
            notes: vec![],
        }
    }

    fn levels(events: &[Event]) -> Vec<(Signal, Level)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Level { signal, to, .. } => Some((signal.clone(), *to)),
                Event::Nominal { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_short_spike_is_not_a_limit() {
        let mut m = Monitor::default();
        assert!(m.observe(&snap(Some(0), Some(20), Some(1))).is_empty());
        assert_eq!(levels(&m.observe(&snap(Some(4000), Some(20), Some(1)))), vec![(Signal::MemoryPressure, Level::Spike)]);
        assert_eq!(levels(&m.observe(&snap(Some(100), Some(20), Some(1)))), vec![(Signal::MemoryPressure, Level::Normal)], "falls back: cleared at once");
        assert_eq!(m.level(&Signal::MemoryPressure), Level::Normal);
    }

    #[test]
    fn a_lasting_load_becomes_sustained_and_clears_with_hysteresis() {
        let mut m = Monitor::default();
        m.observe(&snap(Some(0), Some(20), Some(1)));
        assert_eq!(levels(&m.observe(&snap(Some(1500), Some(20), Some(1)))), vec![(Signal::MemoryPressure, Level::Spike)]);
        assert!(levels(&m.observe(&snap(Some(1500), Some(20), Some(1)))).is_empty());
        // between the clear (750) and raise (1000) thresholds still counts while spiking
        assert_eq!(levels(&m.observe(&snap(Some(800), Some(20), Some(1)))), vec![(Signal::MemoryPressure, Level::Sustained)]);
        // sustained: needs three samples below 750 in a row; one above resets the count
        assert!(levels(&m.observe(&snap(Some(500), Some(20), Some(1)))).is_empty());
        assert!(levels(&m.observe(&snap(Some(900), Some(20), Some(1)))).is_empty());
        assert!(levels(&m.observe(&snap(Some(500), Some(20), Some(1)))).is_empty());
        assert!(levels(&m.observe(&snap(Some(500), Some(20), Some(1)))).is_empty());
        assert_eq!(levels(&m.observe(&snap(Some(500), Some(20), Some(1)))), vec![(Signal::MemoryPressure, Level::Normal)]);
    }

    #[test]
    fn low_memory_and_full_vram_are_signals_unknown_keeps_the_state() {
        let mut m = Monitor::new(MonitorConfig { sustain_samples: 1, ..MonitorConfig::default() });
        m.observe(&snap(None, Some(20), Some(1)));
        let e = m.observe(&snap(None, Some(2), Some(11)));
        assert_eq!(levels(&e), vec![(Signal::LowMemory, Level::Sustained), (Signal::Vram("renderD128".into()), Level::Sustained)]);
        assert!(m.observe(&snap(None, None, None)).is_empty(), "unknown values change nothing");
        assert_eq!(m.level(&Signal::LowMemory), Level::Sustained);
    }

    #[test]
    fn nominal_changes_are_reported_once() {
        let mut m = Monitor::default();
        let base = snap(Some(0), Some(20), Some(1));
        assert!(m.observe(&base).is_empty(), "the first sample is the baseline");
        let mut limited = base.clone();
        limited.nominal.ram_cgroup_limit_bytes = Some(16 * GIB);
        limited.nominal.gpus.clear();
        limited.nominal.gpus.push(GpuNominal {
            node: "renderD129".into(),
            vendor: Vendor::Nvidia,
            driver: "nvidia".into(),
            vram_total_bytes: None,
            drives_display: None,
        });
        let e = m.observe(&limited);
        assert_eq!(
            e,
            vec![Event::Nominal {
                changes: vec![
                    NominalChange::RamLimit { from: None, to: Some(16 * GIB) },
                    NominalChange::GpuRemoved { node: "renderD128".into() },
                    NominalChange::GpuAdded { node: "renderD129".into() },
                ]
            }]
        );
        assert!(m.observe(&limited).is_empty());
    }

    #[test]
    fn a_removed_gpu_drops_its_raised_state() {
        let mut m = Monitor::new(MonitorConfig { sustain_samples: 1, ..MonitorConfig::default() });
        m.observe(&snap(None, None, Some(1)));
        m.observe(&snap(None, None, Some(12)));
        assert_eq!(m.level(&Signal::Vram("renderD128".into())), Level::Sustained);
        let mut gone = snap(None, None, None);
        gone.nominal.gpus.clear();
        gone.current.gpus.clear();
        m.observe(&gone);
        assert_eq!(m.level(&Signal::Vram("renderD128".into())), Level::Normal);
    }

    #[test]
    fn hostile_numbers_never_panic() {
        assert_eq!(used_share(Some(0), Some(5)), None);
        assert_eq!(used_share(Some(10), Some(50)), Some(0));
        assert_eq!(used_share(Some(u64::MAX), Some(0)), Some(10_000));
        assert_eq!(clear_threshold(u32::MAX, u32::MAX), 0);
        assert_eq!(clear_threshold(1000, 2500), 750);
        let mut m = Monitor::new(MonitorConfig { sustain_samples: 0, hysteresis_bp: u32::MAX, ..MonitorConfig::default() });
        m.observe(&snap(Some(u32::MAX), Some(u64::MAX / GIB), Some(u64::MAX / GIB)));
        m.observe(&snap(Some(0), Some(0), Some(0)));
    }

    #[test]
    fn events_are_serde_for_the_control_channel() {
        let e = Event::Level { signal: Signal::Vram("renderD128".into()), from: Level::Normal, to: Level::Spike, value_bp: 9100 };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"{"event":"level","signal":{"kind":"vram","node":"renderD128"},"from":"normal","to":"spike","valueBp":9100}"#);
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }
}
