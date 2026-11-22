//! Nominal and current resources, kept apart (ACS-1 acceptance criterion).
//!
//! **Nominal** is what the machine has and what this process may use at most: physical RAM,
//! online CPUs, the cgroup limits it runs under (`systemd-run -p MemoryMax=… -p CPUQuota=…`, as
//! `scripts/dev/run-minspec.sh` sets them), every GPU's VRAM. It changes rarely (a GPU hot-unplug,
//! a changed cgroup limit), and [`crate::monitor`] reports when it does.
//!
//! **Current** is what is free or busy now: available memory, swap in use, Linux pressure
//! stall information (PSI) for memory, CPU and I/O, VRAM in use by every process and the GPU
//! engine load. It changes all the time; the monitor tells a short spike from a lasting limit.
//!
//! A planner reads both and never mixes them up: "this machine has 32 GB" (nominal) does not mean
//! "32 GB are free" (current). Every number is `Option`: `None` is unknown, never zero.

use std::path::Path;

use cfa4_route::{Inventory, Vendor};
use serde::{Deserialize, Serialize};

use crate::gpumem::{GpuMemory, GpuMemoryReport};
use crate::system::{self, Pressure};

/// One GPU, nominal part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuNominal {
    /// Render node (`renderD128`): the stable key between nominal and current.
    pub node: String,
    pub vendor: Vendor,
    pub driver: String,
    pub vram_total_bytes: Option<u64>,
    /// A GPU that drives a display is left out of AI and other heavy work by default (ADR-0008).
    pub drives_display: Option<bool>,
}

/// One GPU, current part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuCurrent {
    pub node: String,
    pub vram_used_bytes: Option<u64>,
    pub busy_percent: Option<u8>,
}

/// What the machine has and what this process may use at most.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nominal {
    /// `MemTotal`: physical RAM the kernel manages (a little below the module size, because of
    /// firmware and kernel reservations).
    pub ram_physical_bytes: Option<u64>,
    /// The tightest cgroup `memory.max` above this process; `None` when unlimited.
    pub ram_cgroup_limit_bytes: Option<u64>,
    /// Some cgroup `memory.max` above this process exists but cannot be read: the real limit is
    /// unknown (and may be tighter than [`Nominal::ram_cgroup_limit_bytes`]).
    #[serde(default)]
    pub ram_cgroup_limit_unknown: bool,
    pub swap_total_bytes: Option<u64>,
    /// Online logical CPUs.
    pub cpus_online: Option<u32>,
    /// CPUs this process may run on (affinity, narrowed by the cgroup `cpuset`).
    #[serde(default)]
    pub cpus_allowed: Option<u32>,
    /// The tightest cgroup `cpu.max` quota, in thousandths of a CPU; `None` when unlimited.
    pub cpu_quota_milli: Option<u64>,
    pub gpus: Vec<GpuNominal>,
}

impl Nominal {
    /// RAM this process may use: the physical RAM, or the cgroup limit when that is smaller.
    pub fn ram_effective_bytes(&self) -> Option<u64> {
        match (self.ram_physical_bytes, self.ram_cgroup_limit_bytes) {
            (Some(p), Some(l)) => Some(p.min(l)),
            (p, l) => p.or(l),
        }
    }

    /// CPUs this process may use, in thousandths: the smallest of the online CPUs, the CPUs it
    /// may run on (affinity, cpuset) and the `cpu.max` quota.
    pub fn cpus_effective_milli(&self) -> Option<u64> {
        let online = self.cpus_online.map(|c| u64::from(c).saturating_mul(1000));
        let allowed = self.cpus_allowed.map(|c| u64::from(c).saturating_mul(1000));
        [online, allowed, self.cpu_quota_milli].into_iter().flatten().min()
    }

    pub fn gpu(&self, node: &str) -> Option<&GpuNominal> {
        self.gpus.iter().find(|g| g.node == node)
    }
}

/// What is free or busy now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Current {
    /// `MemAvailable`: what the kernel can hand out without swapping, machine-wide.
    pub ram_available_bytes: Option<u64>,
    /// `memory.current` of this process's cgroup.
    pub ram_cgroup_used_bytes: Option<u64>,
    /// Memory still free under the cgroup limits: per limited cgroup (this one and its
    /// ancestors) its limit minus its own usage, which includes sibling cgroups; the smallest.
    /// `None` when unlimited, or when a limited cgroup's usage is unreadable.
    pub ram_cgroup_free_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
    pub memory_pressure: Option<Pressure>,
    pub cpu_pressure: Option<Pressure>,
    pub io_pressure: Option<Pressure>,
    pub gpus: Vec<GpuCurrent>,
}

impl Current {
    pub fn gpu(&self, node: &str) -> Option<&GpuCurrent> {
        self.gpus.iter().find(|g| g.node == node)
    }
}

/// Nominal and current resources at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub nominal: Nominal,
    pub current: Current,
    /// Unreadable sources and other remarks, for the log and `perf.stats`.
    pub notes: Vec<String>,
}

impl Snapshot {
    /// RAM this process can still take now: machine-wide `MemAvailable`, or what is left under
    /// the cgroup limits when that is less. `None` (unknown, never "plenty") when `MemAvailable`
    /// is unreadable, or when a cgroup limit exists but its remaining room cannot be read.
    pub fn ram_free_effective_bytes(&self) -> Option<u64> {
        let available = self.current.ram_available_bytes?;
        if self.nominal.ram_cgroup_limit_unknown {
            return None;
        }
        match (self.nominal.ram_cgroup_limit_bytes, self.current.ram_cgroup_free_bytes) {
            (None, _) => Some(available),
            (Some(_), Some(free)) => Some(available.min(free)),
            (Some(_), None) => None,
        }
    }

    /// VRAM free now on `node`, when both numbers are known.
    pub fn vram_free_bytes(&self, node: &str) -> Option<u64> {
        let total = self.nominal.gpu(node)?.vram_total_bytes?;
        let used = self.current.gpu(node)?.vram_used_bytes?;
        Some(total.saturating_sub(used))
    }

    /// Fill in VRAM numbers sysfs left unknown (NVIDIA, Intel) from an outside reading.
    pub fn apply_gpu_report(&mut self, report: &GpuMemoryReport) {
        if let Some(g) = self.nominal.gpus.iter_mut().find(|g| g.node == report.node)
            && g.vram_total_bytes.is_none()
        {
            g.vram_total_bytes = report.vram_total_bytes;
        }
        if let Some(g) = self.current.gpus.iter_mut().find(|g| g.node == report.node)
            && g.vram_used_bytes.is_none()
        {
            g.vram_used_bytes = report.vram_used_bytes;
        }
    }
}

/// Read nominal and current resources of this machine and process. On anything but Linux the
/// snapshot is empty (all unknown).
pub fn sample() -> Snapshot {
    if cfg!(target_os = "linux") { sample_root(Path::new("/")) } else { Snapshot::default() }
}

/// The same under `root` (tests point it at a synthetic tree). Nominal is read again every time,
/// so the monitor sees a GPU disappear or a cgroup limit change.
pub fn sample_root(root: &Path) -> Snapshot {
    let inventory = cfa4_route::probe::probe(root);
    let nodes = gpu_nodes(root, &inventory);
    let (gpu_memory, gpu_notes) = crate::gpumem::gpu_memory_root(root, &nodes);
    sample_from(root, &inventory, &nodes, &gpu_memory, gpu_notes)
}

/// The render node of each GPU in the inventory, in inventory order. A route device is named
/// `gpu:<node>` (the case the router writes may differ from the kernel's); the node is the
/// directory of `sys/class/drm` that matches it ignoring case, else the name as written (an
/// NVIDIA GPU without a render node has no sysfs entry and stays unknown until NVML reports).
fn gpu_nodes(root: &Path, inventory: &Inventory) -> Vec<String> {
    let names: Vec<String> = std::fs::read_dir(root.join("sys").join("class").join("drm"))
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).take(512).collect())
        .unwrap_or_default();
    inventory
        .gpus()
        .map(|d| {
            let id = d.id.to_string();
            let node = id.strip_prefix("gpu:").unwrap_or(&id);
            names.iter().find(|n| n.eq_ignore_ascii_case(node)).cloned().unwrap_or_else(|| node.to_owned())
        })
        .collect()
}

fn sample_from(root: &Path, inventory: &Inventory, nodes: &[String], gpu_memory: &[GpuMemory], gpu_notes: Vec<String>) -> Snapshot {
    let mut notes = inventory.notes().to_vec();
    notes.extend(gpu_notes);
    let mem = system::meminfo(root);
    if mem.total.is_none() {
        notes.push("proc/meminfo: no MemTotal".into());
    }
    let cgroup = system::own_cgroup(root);
    let ((ram_limit, ram_limit_unknown), cpu_quota, cgroup_used, cgroup_free) = match &cgroup {
        Some(dir) => (
            system::cgroup_memory_max(root, dir),
            system::cgroup_cpu_quota_milli(root, dir),
            system::cgroup_memory_current(dir),
            system::cgroup_memory_free(root, dir),
        ),
        None => ((None, false), None, None, None),
    };
    let mut gpus_nominal = Vec::new();
    let mut gpus_current = Vec::new();
    for (device, node) in inventory.gpus().zip(nodes) {
        let mem = gpu_memory.iter().find(|m| &m.node == node);
        gpus_nominal.push(GpuNominal {
            node: node.clone(),
            vendor: device.vendor,
            driver: device.driver.clone().unwrap_or_default(),
            vram_total_bytes: mem.and_then(|m| m.vram_total_bytes),
            drives_display: mem.and_then(|m| m.drives_display),
        });
        gpus_current.push(GpuCurrent {
            node: node.clone(),
            vram_used_bytes: mem.and_then(|m| m.vram_used_bytes),
            busy_percent: mem.and_then(|m| m.busy_percent),
        });
    }
    let swap_used = match (mem.swap_total, mem.swap_free) {
        (Some(t), Some(f)) => Some(t.saturating_sub(f)),
        _ => None,
    };
    Snapshot {
        nominal: Nominal {
            ram_physical_bytes: mem.total,
            ram_cgroup_limit_bytes: ram_limit,
            ram_cgroup_limit_unknown: ram_limit_unknown,
            swap_total_bytes: mem.swap_total,
            cpus_online: system::online_cpus(root),
            cpus_allowed: system::allowed_cpus(root),
            cpu_quota_milli: cpu_quota,
            gpus: gpus_nominal,
        },
        current: Current {
            ram_available_bytes: mem.available,
            ram_cgroup_used_bytes: cgroup_used,
            ram_cgroup_free_bytes: cgroup_free,
            swap_used_bytes: swap_used,
            memory_pressure: system::pressure(root, "memory"),
            cpu_pressure: system::pressure(root, "cpu"),
            io_pressure: system::pressure(root, "io"),
            gpus: gpus_current,
        },
        notes,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    const GIB: u64 = 1 << 30;

    struct FakeRoot {
        root: PathBuf,
    }

    impl FakeRoot {
        fn new(name: &str) -> std::io::Result<Self> {
            let root = std::env::temp_dir().join(format!("cfa4-tasks-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root)?;
            Ok(FakeRoot { root })
        }

        fn write(&self, rel: &str, text: &str) -> std::io::Result<()> {
            let path = self.root.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, text)
        }

        /// The reference machine under `run-minspec.sh --profile minimum`: 512 GB, 32 CPUs, an
        /// AMD card with 12 GiB, in a scope limited to 16 GiB and 8 CPUs.
        fn minspec(&self) -> std::io::Result<()> {
            self.write("proc/meminfo", "MemTotal: 527958016 kB\nMemAvailable: 500000000 kB\nSwapTotal: 8388608 kB\nSwapFree: 8388608 kB\n")?;
            self.write("sys/devices/system/cpu/online", "0-31\n")?;
            self.write("proc/self/cgroup", "0::/system.slice/run-minspec.scope\n")?;
            self.write("sys/fs/cgroup/system.slice/run-minspec.scope/memory.max", "17179869184\n")?;
            self.write("sys/fs/cgroup/system.slice/run-minspec.scope/memory.current", "4294967296\n")?;
            self.write("sys/fs/cgroup/system.slice/run-minspec.scope/cpu.max", "800000 100000\n")?;
            self.write("sys/fs/cgroup/system.slice/memory.max", "max\n")?;
            self.write("proc/pressure/memory", "some avg10=1.50 avg60=0 avg300=0 total=0\nfull avg10=0.25 avg60=0 avg300=0 total=0\n")?;
            self.write("proc/pressure/cpu", "some avg10=30.00 avg60=0 avg300=0 total=0\n")?;
            let pci = self.root.join("sys/devices/pci0000:00/0000:03:00.0");
            fs::create_dir_all(&pci)?;
            fs::write(pci.join("vendor"), "0x1002\n")?;
            fs::write(pci.join("device"), "0x73df\n")?;
            fs::write(pci.join("mem_info_vram_total"), format!("{}\n", 12 * GIB))?;
            fs::write(pci.join("mem_info_vram_used"), format!("{}\n", GIB))?;
            let drv = self.root.join("sys/bus/pci/drivers/amdgpu");
            fs::create_dir_all(&drv)?;
            symlink(&drv, pci.join("driver"))?;
            for node in ["renderD128", "card0"] {
                let dir = self.root.join("sys/class/drm").join(node);
                fs::create_dir_all(&dir)?;
                symlink(&pci, dir.join("device"))?;
            }
            Ok(())
        }
    }

    impl Drop for FakeRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn nominal_and_current_are_read_apart_and_the_cgroup_wins() {
        let fake = FakeRoot::new("minspec").unwrap();
        fake.minspec().unwrap();
        let snap = sample_root(&fake.root);
        let n = &snap.nominal;
        assert_eq!(n.ram_physical_bytes, Some(527_958_016 * 1024));
        assert_eq!(n.ram_cgroup_limit_bytes, Some(16 * GIB));
        assert_eq!(n.ram_effective_bytes(), Some(16 * GIB), "the scope's limit, not the machine's 512 GB");
        assert_eq!((n.cpus_online, n.cpu_quota_milli, n.cpus_effective_milli()), (Some(32), Some(8000), Some(8000)));
        let gpu = n.gpus.first().unwrap();
        assert_eq!((gpu.node.as_str(), gpu.vendor, gpu.vram_total_bytes), ("renderD128", Vendor::Amd, Some(12 * GIB)));
        let c = &snap.current;
        assert_eq!(c.ram_available_bytes, Some(500_000_000 * 1024));
        assert_eq!(snap.ram_free_effective_bytes(), Some(12 * GIB), "16 GiB limit minus 4 GiB used, not MemAvailable");
        assert_eq!(c.swap_used_bytes, Some(0));
        assert_eq!(c.memory_pressure, Some(Pressure { some_avg10_bp: 150, full_avg10_bp: 25 }));
        assert_eq!(c.cpu_pressure.map(|p| p.some_avg10_bp), Some(3000));
        assert_eq!(c.io_pressure, None, "missing file: unknown");
        assert_eq!(snap.vram_free_bytes("renderD128"), Some(11 * GIB));
    }

    #[test]
    fn an_empty_tree_is_all_unknown_never_zero() {
        let fake = FakeRoot::new("empty").unwrap();
        let snap = sample_root(&fake.root);
        assert_eq!(snap.nominal, Nominal::default());
        assert_eq!(snap.current, Current::default());
        assert_eq!(snap.nominal.ram_effective_bytes(), None);
        assert_eq!(snap.ram_free_effective_bytes(), None);
        assert!(snap.notes.iter().any(|n| n.contains("MemTotal")));
    }

    #[test]
    fn hostile_cgroup_paths_and_values_never_panic() {
        let fake = FakeRoot::new("hostile").unwrap();
        fake.write("proc/self/cgroup", "0::/../../etc\n").unwrap();
        fake.write("proc/meminfo", "MemTotal: 1 kB\n").unwrap();
        let snap = sample_root(&fake.root);
        assert_eq!(snap.nominal.ram_cgroup_limit_bytes, None, "a path leaving the cgroup tree is refused");
        fake.write("proc/self/cgroup", "0::/a\n").unwrap();
        fake.write("sys/fs/cgroup/a/memory.max", "-1\n").unwrap();
        fake.write("sys/fs/cgroup/a/memory.current", &"9".repeat(100_000)).unwrap();
        fake.write("sys/fs/cgroup/a/cpu.max", "1 0\n").unwrap();
        let snap = sample_root(&fake.root);
        assert_eq!((snap.nominal.ram_cgroup_limit_bytes, snap.nominal.cpu_quota_milli, snap.current.ram_cgroup_used_bytes), (None, None, None));
        assert!(snap.nominal.ram_cgroup_limit_unknown, "a malformed memory.max is an unknown limit, not none");
        assert_eq!(snap.ram_free_effective_bytes(), None);
    }

    #[test]
    fn an_unreadable_limit_is_unknown_and_cpusets_narrow_the_cpus() {
        let fake = FakeRoot::new("cpuset").unwrap();
        fake.minspec().unwrap();
        // the leaf is limited and readable, but the parent's limit is garbage: it may be tighter
        fake.write("sys/fs/cgroup/system.slice/memory.max", "garbage\n").unwrap();
        fake.write("sys/fs/cgroup/system.slice/memory.current", "0\n").unwrap();
        fake.write("proc/self/status", "Name:\tcfa4\nCpus_allowed:\tf\nCpus_allowed_list:\t0-3\n").unwrap();
        let snap = sample_root(&fake.root);
        assert_eq!(snap.nominal.ram_cgroup_limit_bytes, Some(16 * GIB));
        assert!(snap.nominal.ram_cgroup_limit_unknown);
        assert_eq!((snap.current.ram_cgroup_free_bytes, snap.ram_free_effective_bytes()), (None, None));
        assert_eq!(snap.nominal.cpus_allowed, Some(4));
        assert_eq!(snap.nominal.cpus_effective_milli(), Some(4000), "4 allowed CPUs beat the 8-CPU quota");
        // "max" is a confirmed absence of a limit
        fake.write("sys/fs/cgroup/system.slice/memory.max", "max\n").unwrap();
        let snap = sample_root(&fake.root);
        assert!(!snap.nominal.ram_cgroup_limit_unknown);
        assert!(snap.ram_free_effective_bytes().is_some());
    }

    #[test]
    fn an_outside_report_fills_nvidia_vram() {
        let mut snap = Snapshot {
            nominal: Nominal {
                gpus: vec![GpuNominal {
                    node: "renderD129".into(),
                    vendor: Vendor::Nvidia,
                    driver: "nvidia".into(),
                    vram_total_bytes: None,
                    drives_display: None,
                }],
                ..Nominal::default()
            },
            current: Current { gpus: vec![GpuCurrent { node: "renderD129".into(), vram_used_bytes: None, busy_percent: None }], ..Current::default() },
            notes: vec![],
        };
        assert_eq!(snap.vram_free_bytes("renderD129"), None);
        snap.apply_gpu_report(&GpuMemoryReport { node: "renderD129".into(), vram_total_bytes: Some(12 * GIB), vram_used_bytes: Some(3 * GIB) });
        assert_eq!(snap.vram_free_bytes("renderD129"), Some(9 * GIB));
        assert_eq!(snap.vram_free_bytes("renderD200"), None);
    }

    #[test]
    fn the_snapshot_is_serde_for_the_control_channel() {
        let fake = FakeRoot::new("serde").unwrap();
        fake.minspec().unwrap();
        let snap = sample_root(&fake.root);
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"ramCgroupLimitBytes\""));
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn the_real_machine_samples_without_panicking() {
        let snap = sample();
        if let (Some(p), Some(e)) = (snap.nominal.ram_physical_bytes, snap.nominal.ram_effective_bytes()) {
            assert!(e <= p);
        }
    }
}
