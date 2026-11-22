//! Adaptive execution planner (ACS-3): of a task's validated execution paths, which one runs on
//! this machine now, which needs the user's approval, and how long it will take.
//!
//! The paths come from the worker manifest (SDK manifest version 5, `paths`); the client
//! (`cfa4-ai-jobs`, render, proxies) resolves the numbers for the job at hand (the VRAM for the
//! job's resolution key, the work units) and hands this module plain [`ExecutionPath`]s, so the
//! crate stays free of AI and manifest details.
//!
//! The owner's rules (ADR-0008 §3, 2026-11-07):
//!
//! - **Available** means at least one validated path fits the budget ([`crate::Budget`]); a path
//!   that can never fit is skipped, one that fits but not now waits.
//! - **Two-part OOM ladder:** the reference path first, then the paths whose
//!   [`Tolerance::automatic`] is `true` (the result stays within the tolerance), in the
//!   manifest's order. A path that changes the result (`automatic: false`) is only **offered**,
//!   the user approves it: no silent quality loss.
//! - **Unknown is never enough:** a path whose RAM or VRAM need is not stated cannot be planned.
//!
//! Every decision carries its reasons, and the estimate is a range with its confidence, never a
//! single promised number.

use cfa4_route::Vendor;
use serde::{Deserialize, Serialize};

use crate::class::{RamProfile, ram_profile};
use crate::manager::{Class, Denied, GpuRequest, Manager, Request, WaitReason};
use crate::monitor::{Level, Monitor, Signal};
use crate::resources::Snapshot;

/// The runtime a path runs on (manifest `provider`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Cuda,
    Tensorrt,
    Migraphx,
    Rocm,
    Openvino,
    Cpu,
}

impl Provider {
    /// The GPU vendor the runtime needs; `None` for the CPU.
    pub fn vendor(self) -> Option<Vendor> {
        match self {
            Provider::Cuda | Provider::Tensorrt => Some(Vendor::Nvidia),
            Provider::Migraphx | Provider::Rocm => Some(Vendor::Amd),
            Provider::Openvino => Some(Vendor::Intel),
            Provider::Cpu => None,
        }
    }
}

/// How a path's output may differ from the reference path's (manifest `tolerance.metric`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Exact,
    MaxAbs,
    PsnrDb,
    BoundaryMs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tolerance {
    pub metric: Metric,
    pub value: f64,
    /// `true`: the result stays within the tolerance, the host may switch on its own.
    pub automatic: bool,
}

/// One validated path, with its numbers resolved for the job at hand.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPath {
    pub id: String,
    /// The path every other one is compared with; exactly one per task.
    pub reference: bool,
    pub provider: Provider,
    /// Measured peak RAM; `None`: not stated, the path cannot be planned.
    pub ram_bytes: Option<u64>,
    /// Measured peak VRAM at the job's resolution; required for every provider but the CPU.
    pub vram_bytes: Option<u64>,
    /// CPU it keeps busy, in thousandths of a CPU.
    pub cpu_milli: u64,
    /// Measured work units (frames, seconds of audio) per second at the job's resolution.
    pub units_per_second: Option<f64>,
    /// `None` only on the reference path.
    pub tolerance: Option<Tolerance>,
    /// The `run-minspec.sh` profile it was measured under.
    pub validated_profile: Option<RamProfile>,
}

/// What is to be run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub class: Class,
    /// Frames, seconds of audio…, in the unit of [`ExecutionPath::units_per_second`]; `None`:
    /// unknown, no estimate.
    pub work_units: Option<u64>,
}

/// How one path stands on this machine now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "state")]
pub enum Fit {
    /// Fits now; `gpu_node` is the GPU the manager would choose.
    Ready { gpu_node: Option<String> },
    /// Fits the budget, not now.
    Wait { reasons: Vec<WaitReason> },
    /// Never fits this machine as configured.
    NeverFits { denied: Denied },
    /// The path does not state what it needs (`ram`, `vram`).
    NotStated { missing: Vec<String> },
}

/// One path's assessment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub path: String,
    /// The reference path, or one the host may switch to on its own.
    pub automatic: bool,
    pub fit: Fit,
    pub estimate: Option<Estimate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// Measured under another `run-minspec.sh` profile than this machine's, or unknown.
    Low,
    /// Measured under this machine's profile.
    Medium,
}

/// How long a path takes, as a range: never a single promised number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub min_s: u64,
    pub max_s: u64,
    pub confidence: Confidence,
}

/// The planner's answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "decision")]
pub enum Decision {
    /// Run `path` now (reserve it with [`Manager::try_reserve`] and [`request`]);
    /// `switched` when it is not the reference path (an automatic, result-preserving switch the
    /// user is told about).
    Run { path: String, gpu_node: Option<String>, switched: bool },
    /// The first automatic path that can ever fit has to wait; `approval_now` names a path that
    /// changes the result but could run now, if the user prefers it to waiting.
    Wait { path: String, reasons: Vec<WaitReason>, approval_now: Option<String> },
    /// No automatic path can ever fit; `path` (which changes the result) can, if the user
    /// approves it. `ready` when it fits now.
    NeedsApproval { path: String, tolerance: Option<Tolerance>, ready: bool },
    /// "Currently not executable": no path fits this machine; every reason is in the
    /// assessments.
    Unavailable,
    /// The path list breaks the manifest rules (no paths, not exactly one reference, a
    /// non-reference path without tolerance, two paths with the same id).
    InvalidPaths { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub decision: Decision,
    /// Every path in the order they are tried.
    pub assessments: Vec<Assessment>,
}

impl Plan {
    /// Whether the task is available on this machine (now or after waiting or approval).
    pub fn available(&self) -> bool {
        matches!(self.decision, Decision::Run { .. } | Decision::Wait { .. } | Decision::NeedsApproval { .. })
    }
}

/// The resource request of a path, `None` when its needs are not stated.
pub fn request(job: &Job, path: &ExecutionPath) -> Option<Request> {
    let ram_bytes = path.ram_bytes?;
    let gpu = match path.provider.vendor() {
        Some(vendor) => Some(GpuRequest { vram_bytes: path.vram_bytes?, node: None, vendor: Some(vendor) }),
        None => None,
    };
    Some(Request { class: job.class, ram_bytes, cpu_milli: path.cpu_milli, gpu })
}

/// Plan `job` over `paths` against the manager's budget and reservations and the current load.
pub fn plan(job: &Job, paths: &[ExecutionPath], manager: &Manager, snap: &Snapshot, monitor: &Monitor) -> Plan {
    if let Err(reason) = validate(paths) {
        return Plan { decision: Decision::InvalidPaths { reason }, assessments: Vec::new() };
    }
    // reference, then the automatic paths, then the ones needing approval; manifest order within
    let automatic = |p: &ExecutionPath| p.reference || p.tolerance.as_ref().is_some_and(|t| t.automatic);
    let mut ordered: Vec<&ExecutionPath> = paths.iter().filter(|p| p.reference).collect();
    ordered.extend(paths.iter().filter(|p| !p.reference && automatic(p)));
    ordered.extend(paths.iter().filter(|p| !automatic(p)));

    let profile = ram_profile(&snap.nominal);
    let slowed = [Signal::CpuPressure, Signal::IoPressure].iter().any(|s| monitor.level(s) == Level::Sustained);
    let assessments: Vec<Assessment> = ordered
        .iter()
        .map(|p| Assessment {
            path: p.id.clone(),
            automatic: automatic(p),
            fit: fit(job, p, manager, snap, monitor),
            estimate: estimate(job, p, profile, slowed),
        })
        .collect();

    let auto = || assessments.iter().filter(|a| a.automatic);
    let manual = || assessments.iter().filter(|a| !a.automatic);
    let ready_manual = manual().find(|a| matches!(a.fit, Fit::Ready { .. })).map(|a| a.path.clone());
    let reference = assessments.first().map(|a| a.path.clone());

    let decision = if let Some((a, gpu_node)) = auto().find_map(|a| match &a.fit {
        Fit::Ready { gpu_node } => Some((a, gpu_node.clone())),
        _ => None,
    }) {
        Decision::Run { path: a.path.clone(), gpu_node, switched: Some(&a.path) != reference.as_ref() }
    } else if let Some((a, reasons)) = auto().find_map(|a| match &a.fit {
        Fit::Wait { reasons } => Some((a, reasons.clone())),
        _ => None,
    }) {
        Decision::Wait { path: a.path.clone(), reasons, approval_now: ready_manual }
    } else if let Some(a) = manual().find(|a| matches!(a.fit, Fit::Ready { .. } | Fit::Wait { .. })) {
        let tolerance = paths.iter().find(|p| p.id == a.path).and_then(|p| p.tolerance.clone());
        Decision::NeedsApproval { path: a.path.clone(), tolerance, ready: matches!(a.fit, Fit::Ready { .. }) }
    } else {
        Decision::Unavailable
    };
    Plan { decision, assessments }
}

fn validate(paths: &[ExecutionPath]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("no paths".into());
    }
    let references = paths.iter().filter(|p| p.reference).count();
    if references != 1 {
        return Err(format!("{references} reference paths, exactly one expected"));
    }
    if let Some(p) = paths.iter().find(|p| !p.reference && p.tolerance.is_none()) {
        return Err(format!("path {} has no tolerance", p.id));
    }
    if let Some(p) = paths.iter().find(|p| p.tolerance.as_ref().is_some_and(|t| !t.value.is_finite() || t.value < 0.0)) {
        return Err(format!("path {} has an invalid tolerance value", p.id));
    }
    for (i, p) in paths.iter().enumerate() {
        if paths.iter().skip(i.saturating_add(1)).any(|q| q.id == p.id) {
            return Err(format!("path id {} is used twice", p.id));
        }
    }
    Ok(())
}

fn fit(job: &Job, path: &ExecutionPath, manager: &Manager, snap: &Snapshot, monitor: &Monitor) -> Fit {
    let Some(request) = request(job, path) else {
        let mut missing = Vec::new();
        if path.ram_bytes.is_none() {
            missing.push("ram".to_owned());
        }
        if path.provider.vendor().is_some() && path.vram_bytes.is_none() {
            missing.push("vram".to_owned());
        }
        return Fit::NotStated { missing };
    };
    match manager.check(&request, snap, monitor) {
        Ok(gpu_node) => Fit::Ready { gpu_node },
        Err(Denied::Wait { reasons }) => Fit::Wait { reasons },
        // only `try_reserve` hands out ids; a check never runs out of them
        Err(denied) => Fit::NeverFits { denied },
    }
}

/// The time range of a path: `work_units / units_per_second`, widened by how far the
/// measurement is from this machine (another profile: low confidence, wider) and doubled at the
/// top while CPU or I/O pressure is sustained.
fn estimate(job: &Job, path: &ExecutionPath, profile: Option<RamProfile>, slowed: bool) -> Option<Estimate> {
    let units = job.work_units? as f64;
    let rate = path.units_per_second.filter(|r| r.is_finite() && *r > 0.0)?;
    let base = units / rate;
    let (confidence, low, high) = match (profile, path.validated_profile) {
        (Some(a), Some(b)) if a == b => (Confidence::Medium, 0.8, 1.5),
        _ => (Confidence::Low, 0.5, 3.0),
    };
    let high = if slowed { high * 2.0 } else { high };
    Some(Estimate { min_s: seconds(base * low)?, max_s: seconds(base * high)?, confidence })
}

/// Whole seconds, rounded up; `None` for a value no `u64` holds (a hostile rate or unit count).
fn seconds(s: f64) -> Option<u64> {
    const MAX: f64 = 1.0e15; // about 31 million years: anything above is a hostile number
    (s.is_finite() && (0.0..=MAX).contains(&s)).then(|| s.ceil() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{Budget, BudgetSettings};
    use crate::manager::Resource;
    use crate::monitor::MonitorConfig;
    use crate::resources::{Current, GpuCurrent, GpuNominal, Nominal};
    use crate::system::Pressure;

    const GIB: u64 = 1 << 30;

    /// The reference machine in the minimum profile: 16 GiB, 8 CPUs, a 12 GiB NVIDIA card for
    /// compute (and an 8 GiB display card); `used_gib` VRAM in use on the compute card.
    fn machine(used_gib: u64) -> Snapshot {
        let gpu = |node: &str, vram: u64, display: bool| GpuNominal {
            node: node.into(),
            vendor: Vendor::Nvidia,
            driver: "nvidia".into(),
            vram_total_bytes: Some(vram * GIB),
            drives_display: Some(display),
        };
        Snapshot {
            nominal: Nominal {
                ram_physical_bytes: Some(16 * GIB),
                cpus_online: Some(8),
                gpus: vec![gpu("renderD128", 8, true), gpu("renderD129", 12, false)],
                ..Nominal::default()
            },
            current: Current {
                ram_available_bytes: Some(13 * GIB),
                gpus: vec![
                    GpuCurrent { node: "renderD128".into(), vram_used_bytes: Some(GIB), busy_percent: None },
                    GpuCurrent { node: "renderD129".into(), vram_used_bytes: Some(used_gib * GIB), busy_percent: None },
                ],
                ..Current::default()
            },
            notes: vec![],
        }
    }

    fn path(id: &str, provider: Provider, vram_gib: Option<u64>, automatic: Option<bool>) -> ExecutionPath {
        ExecutionPath {
            id: id.into(),
            reference: automatic.is_none(),
            provider,
            ram_bytes: Some(4 * GIB),
            vram_bytes: vram_gib.map(|v| v * GIB),
            cpu_milli: 2000,
            units_per_second: Some(10.0),
            tolerance: automatic.map(|automatic| Tolerance { metric: Metric::PsnrDb, value: 40.0, automatic }),
            validated_profile: Some(RamProfile::Minimum),
        }
    }

    fn job() -> Job {
        Job { class: Class::Background, work_units: Some(1000) }
    }

    fn setup(used_gib: u64) -> (Snapshot, Manager) {
        let snap = machine(used_gib);
        let m = Manager::new(Budget::new(&snap.nominal, &BudgetSettings::default()));
        (snap, m)
    }

    #[test]
    fn the_reference_path_runs_when_it_fits() {
        let (snap, m) = setup(0);
        let paths = [path("fp16", Provider::Cuda, Some(10), None), path("tiled", Provider::Cuda, Some(6), Some(true))];
        let p = plan(&job(), &paths, &m, &snap, &Monitor::default());
        assert_eq!(p.decision, Decision::Run { path: "fp16".into(), gpu_node: Some("renderD129".into()), switched: false });
        // 1000 frames at 10 fps, measured under this machine's profile
        assert_eq!(p.assessments.first().and_then(|a| a.estimate), Some(Estimate { min_s: 80, max_s: 150, confidence: Confidence::Medium }));
    }

    #[test]
    fn a_reference_too_big_for_the_card_switches_only_to_an_automatic_path() {
        let (snap, m) = setup(0);
        // 14 GiB never fits the 11.5 GiB budget of a 12 GiB card
        let paths = [
            path("fp32", Provider::Cuda, Some(14), None),
            path("int8", Provider::Cuda, Some(5), Some(false)),
            path("tiled", Provider::Cuda, Some(8), Some(true)),
        ];
        let p = plan(&job(), &paths, &m, &snap, &Monitor::default());
        assert_eq!(p.decision, Decision::Run { path: "tiled".into(), gpu_node: Some("renderD129".into()), switched: true });
        assert_eq!(p.assessments.iter().map(|a| a.path.as_str()).collect::<Vec<_>>(), vec!["fp32", "tiled", "int8"], "reference, automatic, approval");
        assert!(matches!(p.assessments.first().map(|a| &a.fit), Some(Fit::NeverFits { .. })));
    }

    #[test]
    fn a_result_changing_path_is_only_offered() {
        let (snap, m) = setup(0);
        let paths = [path("fp32", Provider::Cuda, Some(14), None), path("int8", Provider::Cuda, Some(5), Some(false))];
        let p = plan(&job(), &paths, &m, &snap, &Monitor::default());
        assert!(matches!(&p.decision, Decision::NeedsApproval { path, ready: true, tolerance: Some(_) } if path == "int8"), "{:?}", p.decision);
        assert!(p.available());
    }

    #[test]
    fn waiting_beats_a_silent_quality_loss_but_the_user_may_choose() {
        // someone else holds 9 GiB of the card: the reference fits the budget, not now
        let (snap, m) = setup(9);
        let paths = [path("fp16", Provider::Cuda, Some(10), None), path("int8", Provider::Cuda, Some(2), Some(false))];
        let p = plan(&job(), &paths, &m, &snap, &Monitor::default());
        assert!(matches!(&p.decision, Decision::Wait { path, approval_now: Some(now), .. } if path == "fp16" && now == "int8"), "{:?}", p.decision);
    }

    #[test]
    fn no_fitting_path_is_unavailable_with_reasons() {
        let (snap, m) = setup(0);
        let paths =
            [path("rocm", Provider::Rocm, Some(8), None), path("big", Provider::Cuda, Some(20), Some(true)), path("cpu", Provider::Cpu, None, Some(false))];
        let mut paths = paths.to_vec();
        if let Some(cpu) = paths.get_mut(2) {
            cpu.ram_bytes = Some(64 * GIB);
        }
        let p = plan(&job(), &paths, &m, &snap, &Monitor::default());
        assert_eq!(p.decision, Decision::Unavailable);
        assert!(!p.available());
        // no AMD card: the ROCm path has no GPU at all
        assert!(matches!(p.assessments.first().map(|a| &a.fit), Some(Fit::NeverFits { denied: Denied::NeverFits { resource: Resource::Gpu { .. }, .. } })));
        assert!(p.assessments.iter().all(|a| matches!(a.fit, Fit::NeverFits { .. })));
    }

    #[test]
    fn unstated_needs_are_never_enough() {
        let (snap, m) = setup(0);
        let mut reference = path("fp16", Provider::Cuda, None, None);
        reference.ram_bytes = None;
        let p = plan(&job(), &[reference], &m, &snap, &Monitor::default());
        assert_eq!(p.decision, Decision::Unavailable);
        assert_eq!(p.assessments.first().map(|a| &a.fit), Some(&Fit::NotStated { missing: vec!["ram".into(), "vram".into()] }));
    }

    #[test]
    fn broken_path_lists_are_refused() {
        let (snap, m) = setup(0);
        let mon = Monitor::default();
        let invalid = |paths: &[ExecutionPath]| matches!(plan(&job(), paths, &m, &snap, &mon).decision, Decision::InvalidPaths { .. });
        assert!(invalid(&[]));
        assert!(invalid(&[path("a", Provider::Cuda, Some(1), Some(true))]), "no reference");
        assert!(invalid(&[path("a", Provider::Cuda, Some(1), None), path("b", Provider::Cuda, Some(1), None)]), "two references");
        assert!(invalid(&[path("a", Provider::Cuda, Some(1), None), path("a", Provider::Cuda, Some(1), Some(true))]), "same id");
        let mut no_tolerance = path("b", Provider::Cuda, Some(1), Some(true));
        no_tolerance.tolerance = None;
        assert!(invalid(&[path("a", Provider::Cuda, Some(1), None), no_tolerance]));
        let mut nan = path("b", Provider::Cuda, Some(1), Some(true));
        nan.tolerance = Some(Tolerance { metric: Metric::MaxAbs, value: f64::NAN, automatic: true });
        assert!(invalid(&[path("a", Provider::Cuda, Some(1), None), nan]));
    }

    #[test]
    fn estimates_are_ranges_and_hostile_numbers_give_none() {
        let p = path("a", Provider::Cuda, Some(1), None);
        let other_profile = estimate(&job(), &p, Some(RamProfile::Recommended), false);
        assert_eq!(other_profile, Some(Estimate { min_s: 50, max_s: 300, confidence: Confidence::Low }));
        let slowed = estimate(&job(), &p, Some(RamProfile::Minimum), true);
        assert_eq!(slowed.map(|e| e.max_s), Some(300), "sustained CPU pressure doubles the top");
        assert_eq!(estimate(&Job { work_units: None, ..job() }, &p, None, false), None);
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300] {
            let p = ExecutionPath { units_per_second: Some(rate), ..p.clone() };
            assert_eq!(estimate(&Job { work_units: Some(u64::MAX), ..job() }, &p, None, false), None, "{rate}");
        }
    }

    #[test]
    fn sustained_cpu_pressure_widens_the_estimate_in_the_plan() {
        let (mut snap, m) = setup(0);
        let mut mon = Monitor::new(MonitorConfig { sustain_samples: 1, ..MonitorConfig::default() });
        snap.current.cpu_pressure = Some(Pressure { some_avg10_bp: 9000, full_avg10_bp: 0 });
        mon.observe(&snap);
        mon.observe(&snap);
        let p = plan(&job(), &[path("fp16", Provider::Cuda, Some(10), None)], &m, &snap, &mon);
        assert_eq!(p.assessments.first().and_then(|a| a.estimate).map(|e| e.max_s), Some(300));
    }

    #[test]
    fn the_plan_is_serde() {
        let (snap, m) = setup(0);
        let p = plan(&job(), &[path("fp16", Provider::Cuda, Some(10), None)], &m, &snap, &Monitor::default());
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""decision":{"decision":"run","path":"fp16","gpuNode":"renderD129","switched":false}"#), "{json}");
        let back: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        let provider: Provider = serde_json::from_str(r#""tensorrt""#).unwrap();
        assert_eq!(provider.vendor(), Some(Vendor::Nvidia));
        let metric: Metric = serde_json::from_str(r#""psnr_db""#).unwrap();
        assert_eq!(metric, Metric::PsnrDb);
    }
}
