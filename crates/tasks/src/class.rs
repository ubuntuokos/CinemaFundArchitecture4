//! The machine's class against the owner's floors (decisions 2026-11-07, ADR-0008 §3, §9).
//!
//! - RAM: below 16 GB local AI work is not supported; 16 GB is the minimum ("runs slower, never
//!   crashes"); 32 GB is the recommended size the free features' 2K performance promise is made
//!   for. `run-minspec.sh` has one profile for each.
//! - AI floor: at least one GPU with 12 GB VRAM that does not drive a display, on top of the RAM
//!   minimum. There is no GPU-less AI path. Media work (decode, render) has no floor and always
//!   keeps its CPU fallback (VISION 2.4).
//!
//! The thresholds sit a little below the marketing sizes: `MemTotal` of a 16 GB machine reads
//! about 15.6 GiB (firmware and kernel keep a part), and VRAM totals are exact powers of two.

use serde::{Deserialize, Serialize};

use crate::resources::Nominal;

const GIB: u64 = 1 << 30;
/// A "16 GB" machine: `MemTotal` or a `MemoryMax=16G` limit.
pub const RAM_MINIMUM_BYTES: u64 = 15 * GIB;
/// A "32 GB" machine.
pub const RAM_RECOMMENDED_BYTES: u64 = 30 * GIB;
/// A "12 GB" card (12 288 MiB; some boards report a few MiB less).
pub const VRAM_AI_FLOOR_BYTES: u64 = 11 * GIB + GIB / 2;

/// Where the effective RAM puts the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RamProfile {
    /// Below 16 GB: CFA4 runs, its AI features are not supported.
    BelowMinimum,
    /// 16 GB up to 32 GB: every supported feature runs, possibly slower.
    Minimum,
    /// 32 GB or more: the performance promise holds.
    Recommended,
}

/// Why the AI floor is or is not met.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "state")]
pub enum AiFloor {
    /// Met; `gpus` are the render nodes that qualify (12 GB or more, no display).
    Met { gpus: Vec<String> },
    /// Not met, with every reason.
    NotMet { reasons: Vec<FloorReason> },
    /// Cannot be decided yet: RAM or some GPU's VRAM is unknown (NVIDIA before NVML reports).
    Unknown { missing: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum FloorReason {
    RamBelowMinimum {
        effective_bytes: u64,
    },
    NoGpu,
    /// Every GPU with enough VRAM drives a display, or none has enough.
    NoQualifyingGpu {
        largest_vram_bytes: Option<u64>,
    },
}

/// The RAM profile of the effective RAM (cgroup limit or physical, the smaller), `None` when
/// unknown.
pub fn ram_profile(nominal: &Nominal) -> Option<RamProfile> {
    let ram = nominal.ram_effective_bytes()?;
    Some(if ram >= RAM_RECOMMENDED_BYTES {
        RamProfile::Recommended
    } else if ram >= RAM_MINIMUM_BYTES {
        RamProfile::Minimum
    } else {
        RamProfile::BelowMinimum
    })
}

/// Whether the machine meets the AI floor. Unknown numbers give [`AiFloor::Unknown`] unless the
/// known ones already decide it: unknown is never read as enough.
pub fn ai_floor(nominal: &Nominal) -> AiFloor {
    let mut reasons = Vec::new();
    let mut missing = Vec::new();
    match nominal.ram_effective_bytes() {
        Some(ram) if ram < RAM_MINIMUM_BYTES => reasons.push(FloorReason::RamBelowMinimum { effective_bytes: ram }),
        Some(_) => {}
        None => missing.push("ram".to_owned()),
    }
    let qualifying: Vec<String> = nominal.gpus.iter().filter(|g| qualifies(g)).map(|g| g.node.clone()).collect();
    let undecided: Vec<String> = nominal.gpus.iter().filter(|g| g.vram_total_bytes.is_none()).map(|g| format!("vram:{}", g.node)).collect();
    if nominal.gpus.is_empty() {
        reasons.push(FloorReason::NoGpu);
    } else if qualifying.is_empty() {
        if undecided.is_empty() {
            let largest = nominal.gpus.iter().filter_map(|g| g.vram_total_bytes).max();
            reasons.push(FloorReason::NoQualifyingGpu { largest_vram_bytes: largest });
        } else {
            missing.extend(undecided);
        }
    }
    if !reasons.is_empty() {
        AiFloor::NotMet { reasons }
    } else if !missing.is_empty() {
        AiFloor::Unknown { missing }
    } else {
        AiFloor::Met { gpus: qualifying }
    }
}

/// 12 GB or more and not known to drive a display. A big card whose display use the driver does
/// not publish (NVIDIA without modeset) counts: excluding it would turn a missing sysfs entry
/// into "no AI". The resource manager (ACS-2) still asks the user before it uses a GPU that turns
/// out to drive the desktop.
fn qualifies(g: &crate::resources::GpuNominal) -> bool {
    g.vram_total_bytes.is_some_and(|v| v >= VRAM_AI_FLOOR_BYTES) && g.drives_display != Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::GpuNominal;
    use cfa4_route::Vendor;

    fn gpu(node: &str, vram_gib: Option<u64>, display: Option<bool>) -> GpuNominal {
        GpuNominal { node: node.into(), vendor: Vendor::Nvidia, driver: "nvidia".into(), vram_total_bytes: vram_gib.map(|g| g * GIB), drives_display: display }
    }

    fn machine(ram_gib: Option<u64>, limit_gib: Option<u64>, gpus: Vec<GpuNominal>) -> Nominal {
        Nominal { ram_physical_bytes: ram_gib.map(|g| g * GIB), ram_cgroup_limit_bytes: limit_gib.map(|g| g * GIB), gpus, ..Nominal::default() }
    }

    #[test]
    fn ram_profiles_follow_the_owner_decision() {
        // a 16 GB machine's MemTotal: about 15.6 GiB
        let sixteen = Nominal { ram_physical_bytes: Some(16_337_000 * 1024), ..Nominal::default() };
        assert_eq!(ram_profile(&sixteen), Some(RamProfile::Minimum));
        assert_eq!(ram_profile(&machine(Some(8), None, vec![])), Some(RamProfile::BelowMinimum));
        assert_eq!(ram_profile(&machine(Some(31), None, vec![])), Some(RamProfile::Recommended));
        assert_eq!(ram_profile(&machine(Some(512), Some(16), vec![])), Some(RamProfile::Minimum), "run-minspec --profile minimum");
        assert_eq!(ram_profile(&machine(Some(512), Some(32), vec![])), Some(RamProfile::Recommended));
        assert_eq!(ram_profile(&machine(None, None, vec![])), None);
    }

    #[test]
    fn the_reference_machine_meets_the_floor_with_the_compute_card_only() {
        // A4000 16 GB for compute, A1000 8 GB drives the display
        let n = machine(Some(512), Some(16), vec![gpu("renderD128", Some(8), Some(true)), gpu("renderD129", Some(16), Some(false))]);
        assert_eq!(ai_floor(&n), AiFloor::Met { gpus: vec!["renderD129".into()] });
    }

    #[test]
    fn below_the_floor_every_reason_is_listed() {
        let n = machine(Some(8), None, vec![gpu("renderD128", Some(8), Some(false))]);
        assert_eq!(
            ai_floor(&n),
            AiFloor::NotMet {
                reasons: vec![FloorReason::RamBelowMinimum { effective_bytes: 8 * GIB }, FloorReason::NoQualifyingGpu { largest_vram_bytes: Some(8 * GIB) }]
            }
        );
        assert_eq!(ai_floor(&machine(Some(64), None, vec![])), AiFloor::NotMet { reasons: vec![FloorReason::NoGpu] });
        let display_only = machine(Some(64), None, vec![gpu("renderD128", Some(24), Some(true))]);
        assert!(matches!(ai_floor(&display_only), AiFloor::NotMet { .. }), "the only big card drives the desktop");
    }

    #[test]
    fn unknown_is_never_enough() {
        let nvidia_before_nvml = machine(Some(64), None, vec![gpu("renderD128", None, None)]);
        assert_eq!(ai_floor(&nvidia_before_nvml), AiFloor::Unknown { missing: vec!["vram:renderD128".into()] });
        let no_ram = machine(None, None, vec![gpu("renderD128", Some(12), Some(false))]);
        assert_eq!(ai_floor(&no_ram), AiFloor::Unknown { missing: vec!["ram".into()] });
        let modeset_off = machine(Some(64), None, vec![gpu("renderD128", Some(12), None)]);
        assert_eq!(ai_floor(&modeset_off), AiFloor::Met { gpus: vec!["renderD128".into()] });
        let small_ram_decides = machine(Some(4), None, vec![gpu("renderD128", None, None)]);
        assert!(matches!(ai_floor(&small_ram_decides), AiFloor::NotMet { .. }), "known facts decide even with unknowns left");
    }
}
