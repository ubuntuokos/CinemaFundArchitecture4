//! Planning: the ordered fallback chain of backends for one workload on one machine.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Backend, Maturity, Unit, Workload, entries};
use crate::inventory::Inventory;
use crate::vendor::Vendor;

/// Where the user wants a workload to run (Settings ▸ Acceleration, one choice per workload).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preference {
    /// Every available backend in catalogue order, the CPU last.
    #[default]
    Auto,
    /// GPU backends, then the CPU (never the NPU).
    Gpu,
    /// NPU backends, then the CPU (never the GPU).
    Npu,
    /// The CPU only.
    Cpu,
}

impl Preference {
    const fn admits(self, unit: Unit) -> bool {
        matches!((self, unit), (_, Unit::Cpu) | (Preference::Auto, _) | (Preference::Gpu, Unit::Gpu) | (Preference::Npu, Unit::Npu))
    }
}

/// The acceleration settings. Serde, so the UI, the control channel and MCP read and set them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    /// Per workload; a workload not listed is [`Preference::Auto`].
    pub preferences: BTreeMap<Workload, Preference>,
    /// Whether backends that run on one vendor's hardware only (NVDEC, NVENC) may be used. Off
    /// gives a strictly vendor-neutral build of the chain, for testing that every workload still
    /// runs without them.
    pub allow_vendor_locked: bool,
    /// Whether [`Maturity::Planned`] backends are listed too (reports and roadmaps; never for
    /// running work).
    pub include_planned: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { preferences: BTreeMap::new(), allow_vendor_locked: true, include_planned: false }
    }
}

impl Settings {
    pub fn preference(&self, workload: Workload) -> Preference {
        self.preferences.get(&workload).copied().unwrap_or_default()
    }
}

/// Why a backend is not in a chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub enum Skip {
    /// Listed in the catalogue as planned, not yet written.
    NotImplemented,
    /// The workload's [`Preference`] leaves this kind of processor out.
    ExcludedByPreference,
    /// Vendor-locked backends are switched off ([`Settings::allow_vendor_locked`]).
    VendorLockedOff { vendor: Vendor },
    /// This machine has no device the backend can run on.
    NoDevice { needs: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub backend: Backend,
    #[serde(flatten)]
    pub why: Skip,
}

/// The plan for one workload: the backends to try, in order, and why the others are left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub workload: Workload,
    pub preference: Preference,
    /// Never empty; always ends with [`Backend::Cpu`].
    pub chain: Vec<Backend>,
    pub skipped: Vec<Skipped>,
}

impl Plan {
    /// The first backend of the chain: what runs when nothing declines.
    pub fn first(&self) -> Backend {
        self.chain.first().copied().unwrap_or(Backend::Cpu)
    }

    /// Whether the first backend is an accelerator (not the CPU).
    pub fn is_accelerated(&self) -> bool {
        self.first() != Backend::Cpu
    }
}

/// What device a backend needs on this machine, `Err` naming it when there is none.
fn device_for(inventory: &Inventory, backend: Backend) -> Result<(), &'static str> {
    let found = match backend {
        Backend::Wgpu | Backend::GpuCompute | Backend::VulkanVideo => inventory.has_gpu(),
        Backend::VaApiDecode | Backend::VaApiEncode => inventory.gpus().any(|d| d.has_vaapi_driver()),
        Backend::Nvdec | Backend::Nvenc => inventory.gpus().any(|d| d.is_nvidia_proprietary()),
        Backend::Npu => inventory.has_npu(),
        Backend::Cpu => true,
    };
    if found {
        return Ok(());
    }
    Err(match backend {
        Backend::VaApiDecode | Backend::VaApiEncode => "a GPU on amdgpu, radeon, i915, xe or nouveau",
        Backend::Nvdec | Backend::Nvenc => "an NVIDIA GPU on the proprietary driver",
        Backend::Npu => "an NPU (/dev/accel)",
        _ => "a GPU render node (/dev/dri/renderD*)",
    })
}

/// The chain of backends for `workload` on the machine described by `inventory`.
pub fn plan(inventory: &Inventory, workload: Workload, settings: &Settings) -> Plan {
    let preference = settings.preference(workload);
    let mut chain = Vec::new();
    let mut skipped = Vec::new();
    for e in entries(workload) {
        let backend = e.backend;
        let why = if e.maturity == Maturity::Planned && !settings.include_planned {
            Some(Skip::NotImplemented)
        } else if !preference.admits(backend.unit()) {
            Some(Skip::ExcludedByPreference)
        } else if let (Some(vendor), false) = (backend.vendor_lock(), settings.allow_vendor_locked) {
            Some(Skip::VendorLockedOff { vendor })
        } else {
            device_for(inventory, backend).err().map(|needs| Skip::NoDevice { needs })
        };
        match why {
            Some(why) => skipped.push(Skipped { backend, why }),
            None => chain.push(backend),
        }
    }
    // the catalogue ends every workload with the CPU (tested); this keeps the promise regardless
    if chain.last() != Some(&Backend::Cpu) {
        chain.retain(|b| *b != Backend::Cpu);
        chain.push(Backend::Cpu);
    }
    Plan { workload, preference, chain, skipped }
}

/// [`plan`] for every workload, for `perf.stats` and Settings ▸ Acceleration.
pub fn plan_all(inventory: &Inventory, settings: &Settings) -> Vec<Plan> {
    Workload::ALL.iter().map(|w| plan(inventory, *w, settings)).collect()
}

/// How vendor-neutral one workload's acceleration is, from the catalogue alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub workload: Workload,
    /// Implemented accelerators (GPU / NPU) that run on any vendor's hardware.
    pub open: Vec<Backend>,
    /// Implemented accelerators that run on one vendor's hardware only.
    pub locked: Vec<Backend>,
    /// Planned vendor-neutral accelerators: the work that closes the gap.
    pub planned_open: Vec<Backend>,
}

impl Coverage {
    /// Whether every vendor's hardware can accelerate this workload today.
    pub fn is_vendor_neutral(&self) -> bool {
        !self.open.is_empty()
    }
}

/// [`Coverage`] of every workload.
pub fn coverage() -> Vec<Coverage> {
    Workload::ALL
        .iter()
        .map(|&workload| {
            let accelerators = || entries(workload).filter(|e| e.backend.unit() != Unit::Cpu);
            let open = |m: Maturity| accelerators().filter(move |e| e.maturity == m && e.backend.vendor_lock().is_none()).map(|e| e.backend).collect();
            Coverage {
                workload,
                open: open(Maturity::Implemented),
                locked: accelerators().filter(|e| e.maturity == Maturity::Implemented && e.backend.vendor_lock().is_some()).map(|e| e.backend).collect(),
                planned_open: open(Maturity::Planned),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{Device, DeviceKind};
    use Backend::*;

    fn device(kind: DeviceKind, pci_vendor: u32, driver: &str, node: &str) -> Device {
        Device { kind, vendor: Vendor::from_pci(pci_vendor), pci_vendor, pci_device: 1, driver: driver.into(), node: node.into() }
    }

    fn machine(devices: Vec<Device>) -> Inventory {
        Inventory { devices, notes: Vec::new() }
    }

    fn amd() -> Inventory {
        machine(vec![device(DeviceKind::Gpu, 0x1002, "amdgpu", "renderD128")])
    }

    fn intel_with_npu() -> Inventory {
        machine(vec![device(DeviceKind::Gpu, 0x8086, "xe", "renderD128"), device(DeviceKind::Npu, 0x8086, "intel_vpu", "accel0")])
    }

    fn nvidia() -> Inventory {
        machine(vec![device(DeviceKind::Gpu, 0x10de, "nvidia", "renderD128")])
    }

    fn nouveau() -> Inventory {
        machine(vec![device(DeviceKind::Gpu, 0x10de, "nouveau", "renderD128")])
    }

    fn hybrid_laptop() -> Inventory {
        machine(vec![device(DeviceKind::Gpu, 0x8086, "i915", "renderD128"), device(DeviceKind::Gpu, 0x10de, "nvidia", "renderD129")])
    }

    fn amd_with_npu() -> Inventory {
        machine(vec![device(DeviceKind::Gpu, 0x1002, "amdgpu", "renderD128"), device(DeviceKind::Npu, 0x1022, "amdxdna", "accel0")])
    }

    fn virtual_machine() -> Inventory {
        Inventory::default()
    }

    fn machines() -> Vec<(&'static str, Inventory)> {
        vec![
            ("amd", amd()),
            ("intel+npu", intel_with_npu()),
            ("nvidia", nvidia()),
            ("nouveau", nouveau()),
            ("hybrid", hybrid_laptop()),
            ("amd+npu", amd_with_npu()),
            ("vm", virtual_machine()),
        ]
    }

    fn chain(inv: &Inventory, w: Workload, s: &Settings) -> Vec<Backend> {
        plan(inv, w, s).chain
    }

    fn with(f: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = Settings::default();
        f(&mut s);
        s
    }

    #[test]
    fn amd_and_intel_get_the_same_acceleration_as_nvidia_where_it_exists() {
        let s = Settings::default();
        for inv in [amd(), intel_with_npu(), nouveau()] {
            assert_eq!(chain(&inv, Workload::Composite, &s), vec![Wgpu, Cpu]);
            assert_eq!(chain(&inv, Workload::VideoDecode, &s), vec![VaApiDecode, Cpu]);
        }
        assert_eq!(chain(&nvidia(), Workload::Composite, &s), vec![Wgpu, Cpu]);
        assert_eq!(chain(&nvidia(), Workload::VideoDecode, &s), vec![Nvdec, Cpu]);
        // both GPUs of a hybrid laptop: NVDEC first (as `cfa4_platform::register`), VA-API next
        assert_eq!(chain(&hybrid_laptop(), Workload::VideoDecode, &s), vec![Nvdec, VaApiDecode, Cpu]);
    }

    #[test]
    fn the_open_gaps_show_up_in_the_plans() {
        let s = Settings::default();
        // hardware encoding is NVIDIA-only today: AMD and Intel encode on the CPU
        assert_eq!(chain(&amd(), Workload::VideoEncode, &s), vec![Cpu]);
        assert_eq!(chain(&nvidia(), Workload::VideoEncode, &s), vec![Nvenc, Cpu]);
        let skipped = plan(&amd(), Workload::VideoEncode, &s).skipped;
        assert!(skipped.contains(&Skipped { backend: Nvenc, why: Skip::NoDevice { needs: "an NVIDIA GPU on the proprietary driver" } }));
        assert!(skipped.contains(&Skipped { backend: VaApiEncode, why: Skip::NotImplemented }));
        // local AI is CPU-only on every machine
        for (_, inv) in machines() {
            assert_eq!(chain(&inv, Workload::SpeechToText, &s), vec![Cpu]);
            assert_eq!(chain(&inv, Workload::TextToSpeech, &s), vec![Cpu]);
        }
    }

    #[test]
    fn without_vendor_locked_backends_nvidia_still_runs_everything() {
        let s = with(|s| s.allow_vendor_locked = false);
        let inv = nvidia();
        assert_eq!(chain(&inv, Workload::Composite, &s), vec![Wgpu, Cpu]);
        assert_eq!(chain(&inv, Workload::VideoDecode, &s), vec![Cpu]);
        assert_eq!(chain(&inv, Workload::VideoEncode, &s), vec![Cpu]);
        assert!(plan(&inv, Workload::VideoDecode, &s).skipped.contains(&Skipped { backend: Nvdec, why: Skip::VendorLockedOff { vendor: Vendor::Nvidia } }));
        // the hybrid laptop decodes on its Intel GPU instead
        assert_eq!(chain(&hybrid_laptop(), Workload::VideoDecode, &s), vec![VaApiDecode, Cpu]);
    }

    #[test]
    fn preferences_choose_the_processor() {
        let planned = |p: Preference| {
            with(|s| {
                s.include_planned = true;
                s.preferences.insert(Workload::SpeechToText, p);
            })
        };
        let inv = intel_with_npu();
        assert_eq!(chain(&inv, Workload::SpeechToText, &planned(Preference::Auto)), vec![GpuCompute, Npu, Cpu]);
        assert_eq!(chain(&inv, Workload::SpeechToText, &planned(Preference::Gpu)), vec![GpuCompute, Cpu]);
        assert_eq!(chain(&inv, Workload::SpeechToText, &planned(Preference::Npu)), vec![Npu, Cpu]);
        assert_eq!(chain(&inv, Workload::SpeechToText, &planned(Preference::Cpu)), vec![Cpu]);
        // an NPU preference on a machine without one is the CPU, with the reason
        let p = plan(&amd(), Workload::SpeechToText, &planned(Preference::Npu));
        assert_eq!(p.chain, vec![Cpu]);
        assert!(p.skipped.contains(&Skipped { backend: Npu, why: Skip::NoDevice { needs: "an NPU (/dev/accel)" } }));
        // other workloads keep Auto
        assert_eq!(chain(&amd(), Workload::Composite, &planned(Preference::Npu)), vec![Wgpu, Cpu]);
    }

    #[test]
    fn planned_backends_show_the_vendor_neutral_roadmap() {
        let s = with(|s| s.include_planned = true);
        assert_eq!(chain(&amd(), Workload::VideoEncode, &s), vec![VaApiEncode, VulkanVideo, Cpu]);
        assert_eq!(chain(&nvidia(), Workload::VideoEncode, &s), vec![Nvenc, VulkanVideo, Cpu]);
        assert_eq!(chain(&virtual_machine(), Workload::VideoEncode, &s), vec![Cpu]);
    }

    #[test]
    fn every_plan_ends_on_the_cpu_and_respects_the_settings() {
        let preferences = [Preference::Auto, Preference::Gpu, Preference::Npu, Preference::Cpu];
        for (name, inv) in machines() {
            for p in preferences {
                for allow_vendor_locked in [true, false] {
                    for include_planned in [true, false] {
                        let s = Settings { preferences: Workload::ALL.iter().map(|w| (*w, p)).collect(), allow_vendor_locked, include_planned };
                        for plan in plan_all(&inv, &s) {
                            let ctx = format!("{name} {p:?} locked={allow_vendor_locked} planned={include_planned} {:?}", plan.workload);
                            assert_eq!(plan.chain.last(), Some(&Cpu), "{ctx}");
                            let mut unique = plan.chain.clone();
                            unique.sort();
                            unique.dedup();
                            assert_eq!(unique.len(), plan.chain.len(), "{ctx}");
                            for b in &plan.chain {
                                assert!(p.admits(b.unit()), "{ctx}: {b:?}");
                                assert!(allow_vendor_locked || b.vendor_lock().is_none(), "{ctx}: {b:?}");
                                assert!(!plan.skipped.iter().any(|s| s.backend == *b), "{ctx}: {b:?} both chosen and skipped");
                            }
                            let listed = entries(plan.workload).count();
                            assert_eq!(plan.chain.len() + plan.skipped.len(), listed, "{ctx}");
                            if p == Preference::Cpu {
                                assert_eq!(plan.chain, vec![Cpu], "{ctx}");
                            }
                        }
                    }
                }
            }
        }
        assert!(plan_all(&virtual_machine(), &Settings::default()).iter().all(|p| !p.is_accelerated()));
    }

    /// Update this test with the catalogue: it is the vendor-neutrality status of the workspace.
    #[test]
    fn coverage_matches_the_workspace() {
        let by_workload: BTreeMap<Workload, Coverage> = coverage().into_iter().map(|c| (c.workload, c)).collect();
        let neutral: Vec<Workload> = by_workload.values().filter(|c| c.is_vendor_neutral()).map(|c| c.workload).collect();
        assert_eq!(neutral, vec![Workload::Composite, Workload::VideoDecode]);
        let encode = by_workload.get(&Workload::VideoEncode).unwrap();
        assert_eq!((encode.open.clone(), encode.locked.clone(), encode.planned_open.clone()), (vec![], vec![Nvenc], vec![VaApiEncode, VulkanVideo]));
        let speech = by_workload.get(&Workload::SpeechToText).unwrap();
        assert_eq!((speech.open.clone(), speech.planned_open.clone()), (vec![], vec![GpuCompute, Npu]));
    }

    #[test]
    fn settings_round_trip_and_hostile_json_is_an_error() {
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
        let s = with(|s| {
            s.allow_vendor_locked = false;
            s.preferences.insert(Workload::VideoEncode, Preference::Gpu);
        });
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"preferences":{"videoEncode":"gpu"},"allowVendorLocked":false,"includePlanned":false}"#);
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        for bad in [
            r#"{"preferences":{"mining":"gpu"}}"#,
            r#"{"preferences":{"composite":"tpu"}}"#,
            r#"{"preferences":[]}"#,
            r#"{"allowVendorLocked":"yes"}"#,
            r#"{"unknown":1}"#,
            "",
        ] {
            assert!(serde_json::from_str::<Settings>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn plans_serialize_for_perf_stats() {
        let json = serde_json::to_value(plan(&amd(), Workload::VideoEncode, &Settings::default())).unwrap();
        assert_eq!(json["workload"], "videoEncode");
        assert_eq!(json["chain"], serde_json::json!(["cpu"]));
        assert_eq!(json["skipped"][0], serde_json::json!({"backend": "nvenc", "reason": "noDevice", "needs": "an NVIDIA GPU on the proprietary driver"}));
        assert_eq!(json["skipped"][1], serde_json::json!({"backend": "vaApiEncode", "reason": "notImplemented"}));
    }
}
