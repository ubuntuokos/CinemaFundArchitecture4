//! Resource budgets (ACS-2): how much of the machine CFA4's heavy work may take at most.
//!
//! A budget is computed from the **nominal** resources ([`Nominal`]) and the user's
//! [`BudgetSettings`]: the effective RAM (physical or the cgroup limit) minus a reserve for the
//! desktop and other programs, optionally capped by the user; the effective CPUs minus a reserve;
//! and per GPU its VRAM minus a reserve. The **current** load is not part of the budget: the
//! [`crate::manager::Manager`] checks it at every reservation, so a budget stays stable while the
//! load moves.
//!
//! GPUs (ADR-0008 §3):
//! - a GPU whose VRAM is unknown gets no budget (unknown is never enough);
//! - a GPU that drives a display, or whose display use is unknown, gets no budget when a usable
//!   GPU known to drive no display exists; otherwise only if the user allows it
//!   ([`BudgetSettings::allow_display_gpu`]);
//! - the user may restrict heavy work to some GPUs ([`BudgetSettings::gpus`]).

use cfa4_route::Vendor;
use serde::{Deserialize, Serialize};

use crate::resources::Nominal;

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

/// The user's settings (serde, so the UI, the control channel and MCP read and set them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BudgetSettings {
    /// RAM kept free for the desktop and other programs. `None`: the larger of 2 GiB and 10 % of
    /// the effective RAM.
    pub ram_reserve_bytes: Option<u64>,
    /// At most this much RAM for CFA4's heavy work, on top of the reserve. `None`: no cap.
    pub ram_limit_bytes: Option<u64>,
    /// CPU kept free, in thousandths of a CPU (1000 = one CPU).
    pub cpu_reserve_milli: u64,
    /// VRAM kept free on every GPU (driver, compositor, other programs).
    pub vram_reserve_bytes: u64,
    /// Let heavy work use a GPU that drives a display when it is the only usable one.
    pub allow_display_gpu: bool,
    /// Only these render nodes take heavy work. `None`: every usable GPU.
    pub gpus: Option<Vec<String>>,
}

impl Default for BudgetSettings {
    fn default() -> Self {
        BudgetSettings {
            ram_reserve_bytes: None,
            ram_limit_bytes: None,
            cpu_reserve_milli: 1000,
            vram_reserve_bytes: 512 * MIB,
            allow_display_gpu: false,
            gpus: None,
        }
    }
}

/// One GPU's budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuBudget {
    pub node: String,
    /// Which runtimes can use it (CUDA: NVIDIA, ROCm: AMD, OpenVINO: Intel).
    #[serde(default)]
    pub vendor: Option<Vendor>,
    pub vram_bytes: u64,
    /// Kept free on the card, also against other programs' current use.
    pub vram_reserve_bytes: u64,
}

/// Why a GPU got no budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "reason")]
pub enum GpuExcluded {
    VramUnknown {
        node: String,
    },
    DrivesDisplay {
        node: String,
    },
    /// Whether it drives a display could not be read; treated as if it did.
    DisplayUnknown {
        node: String,
    },
    NotSelected {
        node: String,
    },
    TooSmall {
        node: String,
        vram_bytes: u64,
    },
}

/// What CFA4's heavy work may take at most.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    /// `None`: the effective RAM is unknown, so no RAM can be promised.
    pub ram_bytes: Option<u64>,
    /// Kept free, also against other programs' current use.
    pub ram_reserve_bytes: u64,
    pub cpu_milli: Option<u64>,
    pub gpus: Vec<GpuBudget>,
    pub excluded: Vec<GpuExcluded>,
}

impl Budget {
    pub fn new(nominal: &Nominal, settings: &BudgetSettings) -> Self {
        let effective = nominal.ram_effective_bytes();
        let ram_reserve = settings.ram_reserve_bytes.unwrap_or_else(|| effective.map_or(2 * GIB, |e| (e / 10).max(2 * GIB)));
        let ram_bytes = effective.map(|e| {
            let after_reserve = e.saturating_sub(ram_reserve);
            settings.ram_limit_bytes.map_or(after_reserve, |cap| after_reserve.min(cap))
        });
        let cpu_milli = nominal.cpus_effective_milli().map(|c| c.saturating_sub(settings.cpu_reserve_milli));

        let mut excluded = Vec::new();
        let mut usable = Vec::new();
        for gpu in &nominal.gpus {
            if settings.gpus.as_ref().is_some_and(|only| !only.contains(&gpu.node)) {
                excluded.push(GpuExcluded::NotSelected { node: gpu.node.clone() });
                continue;
            }
            let Some(total) = gpu.vram_total_bytes else {
                excluded.push(GpuExcluded::VramUnknown { node: gpu.node.clone() });
                continue;
            };
            let vram = total.saturating_sub(settings.vram_reserve_bytes);
            if vram == 0 {
                excluded.push(GpuExcluded::TooSmall { node: gpu.node.clone(), vram_bytes: total });
                continue;
            }
            usable.push((gpu, vram));
        }
        let has_headless = usable.iter().any(|(g, _)| g.drives_display == Some(false));
        let mut gpus = Vec::new();
        for (gpu, vram) in usable {
            if gpu.drives_display != Some(false) && (has_headless || !settings.allow_display_gpu) {
                let node = gpu.node.clone();
                excluded.push(if gpu.drives_display.is_none() { GpuExcluded::DisplayUnknown { node } } else { GpuExcluded::DrivesDisplay { node } });
                continue;
            }
            gpus.push(GpuBudget { node: gpu.node.clone(), vendor: Some(gpu.vendor), vram_bytes: vram, vram_reserve_bytes: settings.vram_reserve_bytes });
        }
        Budget { ram_bytes, ram_reserve_bytes: ram_reserve, cpu_milli, gpus, excluded }
    }

    pub fn gpu(&self, node: &str) -> Option<&GpuBudget> {
        self.gpus.iter().find(|g| g.node == node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::GpuNominal;

    fn gpu(node: &str, vram_gib: Option<u64>, display: Option<bool>) -> GpuNominal {
        GpuNominal { node: node.into(), vendor: Vendor::Nvidia, driver: "nvidia".into(), vram_total_bytes: vram_gib.map(|g| g * GIB), drives_display: display }
    }

    #[test]
    fn the_reference_machine_under_the_minimum_profile() {
        // 512 GB, 32 CPUs, a 16 GiB scope with 8 CPUs; A1000 8 GB on the display, A4000 16 GB for compute
        let n = Nominal {
            ram_physical_bytes: Some(512 * GIB),
            ram_cgroup_limit_bytes: Some(16 * GIB),
            cpus_online: Some(32),
            cpu_quota_milli: Some(8000),
            gpus: vec![gpu("renderD128", Some(8), Some(true)), gpu("renderD129", Some(16), Some(false))],
            ..Nominal::default()
        };
        let b = Budget::new(&n, &BudgetSettings::default());
        assert_eq!(b.ram_reserve_bytes, 2 * GIB, "10 % of 16 GiB is below the 2 GiB floor");
        assert_eq!(b.ram_bytes, Some(14 * GIB));
        assert_eq!(b.cpu_milli, Some(7000));
        assert_eq!(
            b.gpus,
            vec![GpuBudget { node: "renderD129".into(), vendor: Some(Vendor::Nvidia), vram_bytes: 16 * GIB - 512 * MIB, vram_reserve_bytes: 512 * MIB }]
        );
        assert_eq!(b.excluded, vec![GpuExcluded::DrivesDisplay { node: "renderD128".into() }]);
    }

    #[test]
    fn a_single_display_gpu_needs_the_users_permission() {
        let n = Nominal { ram_physical_bytes: Some(32 * GIB), gpus: vec![gpu("renderD128", Some(12), Some(true))], ..Nominal::default() };
        assert!(Budget::new(&n, &BudgetSettings::default()).gpus.is_empty());
        let allowed = Budget::new(&n, &BudgetSettings { allow_display_gpu: true, ..BudgetSettings::default() });
        assert_eq!(allowed.gpus.first().map(|g| g.node.as_str()), Some("renderD128"));
        assert_eq!(allowed.ram_reserve_bytes, 32 * GIB / 10);
    }

    #[test]
    fn unknown_vram_and_user_selection_exclude_with_reasons() {
        let n = Nominal {
            gpus: vec![gpu("renderD128", None, None), gpu("renderD129", Some(12), Some(false)), gpu("renderD130", Some(24), Some(false))],
            ..Nominal::default()
        };
        let b = Budget::new(&n, &BudgetSettings { gpus: Some(vec!["renderD128".into(), "renderD129".into()]), ..BudgetSettings::default() });
        assert_eq!(b.gpus.iter().map(|g| g.node.as_str()).collect::<Vec<_>>(), vec!["renderD129"]);
        assert_eq!(b.excluded, vec![GpuExcluded::VramUnknown { node: "renderD128".into() }, GpuExcluded::NotSelected { node: "renderD130".into() }]);
        assert_eq!((b.ram_bytes, b.cpu_milli), (None, None), "unknown RAM and CPUs promise nothing");
    }

    #[test]
    fn unknown_display_use_counts_as_a_display() {
        let n = Nominal { gpus: vec![gpu("renderD128", Some(12), None), gpu("renderD129", Some(12), Some(false))], ..Nominal::default() };
        let b = Budget::new(&n, &BudgetSettings::default());
        assert_eq!(b.gpus.iter().map(|g| g.node.as_str()).collect::<Vec<_>>(), vec!["renderD129"]);
        assert_eq!(b.excluded, vec![GpuExcluded::DisplayUnknown { node: "renderD128".into() }]);
        // alone, it needs the user's permission like a display card
        let alone = Nominal { gpus: vec![gpu("renderD128", Some(12), None)], ..Nominal::default() };
        assert!(Budget::new(&alone, &BudgetSettings::default()).gpus.is_empty());
        assert_eq!(Budget::new(&alone, &BudgetSettings { allow_display_gpu: true, ..BudgetSettings::default() }).gpus.len(), 1);
    }

    #[test]
    fn user_caps_and_hostile_reserves_saturate() {
        let n = Nominal { ram_physical_bytes: Some(64 * GIB), cpus_online: Some(2), gpus: vec![gpu("renderD128", Some(1), Some(false))], ..Nominal::default() };
        let s = BudgetSettings { ram_reserve_bytes: Some(u64::MAX), cpu_reserve_milli: u64::MAX, vram_reserve_bytes: 2 * GIB, ..BudgetSettings::default() };
        let b = Budget::new(&n, &s);
        assert_eq!((b.ram_bytes, b.cpu_milli), (Some(0), Some(0)));
        assert_eq!(b.excluded, vec![GpuExcluded::TooSmall { node: "renderD128".into(), vram_bytes: GIB }]);
        let capped = Budget::new(&n, &BudgetSettings { ram_limit_bytes: Some(8 * GIB), ..BudgetSettings::default() });
        assert_eq!(capped.ram_bytes, Some(8 * GIB));
    }

    #[test]
    fn settings_are_serde_with_defaults() {
        let s: BudgetSettings = serde_json::from_str(r#"{"allowDisplayGpu":true}"#).unwrap();
        assert!(s.allow_display_gpu);
        assert_eq!(s.cpu_reserve_milli, 1000);
    }
}
