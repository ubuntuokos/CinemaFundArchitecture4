//! GPU memory and display use from Linux sysfs (ACS-1, hardware discovery).
//!
//! For every GPU of a route inventory (`cfa4_route::Inventory`) this reads what the kernel driver publishes without any
//! vendor library:
//!
//! - `device/mem_info_vram_total` and `device/mem_info_vram_used` (bytes): the card's VRAM and
//!   what is in use now. Published by `amdgpu`; the other drivers leave the fields empty, see
//!   below.
//! - `device/gpu_busy_percent` (0–100): the engine load, `amdgpu` only.
//! - whether the GPU drives a display: a `cardN-<connector>` entry of the same PCI device whose
//!   `status` is `connected`.
//!
//! NVIDIA's proprietary driver and Intel `xe` / `i915` publish no VRAM counters in sysfs. Their
//! fields stay `None` ("unknown", never zero), and a reading from outside (NVML through
//! `cfa4-platform`, or a worker's report) fills them in later via [`GpuMemory::apply_report`].
//! Unknown is not the same as "none": a planner must not treat a missing number as free memory.
//!
//! Reads are bounded like [`crate::probe`] and every failure is a note, never a panic.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// At most this many bytes are read from one sysfs attribute.
const MAX_ATTR_BYTES: u64 = 64;
/// At most this many `/sys/class/drm` entries are looked at for connectors.
const MAX_DRM_ENTRIES: usize = 512;

/// The memory and display state of one GPU. `None` means unknown, never zero.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemory {
    /// The render node (`renderD128`), the key that ties it to the route device `gpu:<node>`.
    pub node: String,
    /// The card's VRAM in bytes (nominal: it does not change while the card is present).
    pub vram_total_bytes: Option<u64>,
    /// VRAM in use now, by every process on the machine, in bytes (current).
    pub vram_used_bytes: Option<u64>,
    /// Engine load now, 0–100 (current).
    pub busy_percent: Option<u8>,
    /// Whether a connected display hangs on this GPU. `None` when the driver publishes no
    /// connectors for it (NVIDIA without `nvidia-drm.modeset=1`).
    pub drives_display: Option<bool>,
}

/// A reading from outside sysfs (NVML via `cfa4-platform`, a worker's report).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemoryReport {
    pub node: String,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_bytes: Option<u64>,
}

/// A PCI address (`0000:03:00.0`; NVML writes the domain with eight digits, `00000000:03:00.0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PciAddress {
    pub domain: u32,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

fn hex(t: &str, max: usize) -> Option<&str> {
    (!t.is_empty() && t.len() <= max && t.bytes().all(|b| b.is_ascii_hexdigit())).then_some(t)
}

impl PciAddress {
    /// `domain:bus:device.function` in hex, any case; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split(':');
        let (domain, bus, rest) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let (device, function) = rest.split_once('.')?;
        let address = PciAddress {
            domain: u32::from_str_radix(hex(domain, 8)?, 16).ok()?,
            bus: u8::from_str_radix(hex(bus, 2)?, 16).ok()?,
            device: u8::from_str_radix(hex(device, 2)?, 16).ok()?,
            function: u8::from_str_radix(hex(function, 1)?, 16).ok()?,
        };
        // a PCI device number is 5 bits, a function 3 bits
        (address.device < 32 && address.function < 8).then_some(address)
    }
}

/// A VRAM reading keyed by PCI address (what NVML reports), before it is tied to a render node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PciMemoryReading {
    pub pci: PciAddress,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_bytes: Option<u64>,
}

/// The PCI address behind a render node (`/sys/class/drm/<node>/device`).
pub fn node_pci_address_root(root: &Path, node: &str) -> Option<PciAddress> {
    if node.is_empty() || node.contains('/') || node.contains("..") {
        return None;
    }
    let target = fs::canonicalize(root.join("sys/class/drm").join(node).join("device")).ok()?;
    PciAddress::parse(target.file_name()?.to_str()?)
}

/// Tie PCI-keyed readings to the render `nodes`: one report per GPU whose PCI
/// address has a reading. Readings for unknown addresses are left out.
pub fn reports_by_pci_root(root: &Path, nodes: &[String], readings: &[PciMemoryReading]) -> Vec<GpuMemoryReport> {
    nodes
        .iter()
        .filter_map(|node| {
            let pci = node_pci_address_root(root, node)?;
            let r = readings.iter().find(|r| r.pci == pci)?;
            Some(GpuMemoryReport { node: node.clone(), vram_total_bytes: r.vram_total_bytes, vram_used_bytes: r.vram_used_bytes })
        })
        .collect()
}

/// [`reports_by_pci_root`] on this machine; empty outside Linux.
pub fn reports_by_pci(nodes: &[String], readings: &[PciMemoryReading]) -> Vec<GpuMemoryReport> {
    if cfg!(target_os = "linux") { reports_by_pci_root(Path::new("/"), nodes, readings) } else { Vec::new() }
}

impl GpuMemory {
    /// VRAM free now, when both numbers are known (saturating: a used value above the total,
    /// from a racing or hostile reading, gives zero, not an underflow).
    pub fn vram_free_bytes(&self) -> Option<u64> {
        Some(self.vram_total_bytes?.saturating_sub(self.vram_used_bytes?))
    }

    /// Fill in what sysfs left unknown from an outside reading of the same node. A number sysfs
    /// did publish is kept: the kernel's own counter wins over a second-hand one.
    pub fn apply_report(&mut self, report: &GpuMemoryReport) {
        if report.node != self.node {
            return;
        }
        if self.vram_total_bytes.is_none() {
            self.vram_total_bytes = report.vram_total_bytes;
        }
        if self.vram_used_bytes.is_none() {
            self.vram_used_bytes = report.vram_used_bytes;
        }
    }
}

/// Memory and display state of every render node of `nodes`, read from `/sys`. Empty outside Linux.
pub fn gpu_memory(nodes: &[String]) -> (Vec<GpuMemory>, Vec<String>) {
    if cfg!(target_os = "linux") { gpu_memory_root(Path::new("/"), nodes) } else { (Vec::new(), Vec::new()) }
}

/// The same under `root` (`root/sys/class/drm/…`). Tests point it at a synthetic tree.
pub fn gpu_memory_root(root: &Path, nodes: &[String]) -> (Vec<GpuMemory>, Vec<String>) {
    let drm = root.join("sys").join("class").join("drm");
    let mut notes = Vec::new();
    let displays = connectors(&drm, &mut notes);
    let mut out = Vec::new();
    for node in nodes {
        let device_dir = drm.join(node).join("device");
        let pci = fs::canonicalize(&device_dir).ok();
        let drives_display = pci.as_ref().and_then(|pci| displays.iter().find(|(p, _)| p == pci).map(|(_, connected)| *connected));
        out.push(GpuMemory {
            node: node.clone(),
            vram_total_bytes: read_u64(&device_dir.join("mem_info_vram_total")),
            vram_used_bytes: read_u64(&device_dir.join("mem_info_vram_used")),
            busy_percent: read_u64(&device_dir.join("gpu_busy_percent")).and_then(|v| u8::try_from(v).ok()).filter(|v| *v <= 100),
            drives_display,
        });
    }
    (out, notes)
}

/// Per PCI device (canonical path): whether any of its connectors is connected. Only devices
/// with at least one connector entry appear.
fn connectors(drm: &Path, notes: &mut Vec<String>) -> Vec<(PathBuf, bool)> {
    let entries = match fs::read_dir(drm) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            notes.push(format!("{}: {err}", drm.display()));
            return Vec::new();
        }
    };
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    for (seen, entry) in entries.flatten().enumerate() {
        if seen >= MAX_DRM_ENTRIES {
            notes.push(format!("{}: more than {MAX_DRM_ENTRIES} entries, the rest are left out", drm.display()));
            break;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // `card0-DP-1`: the card is the part before the first '-', and it must be `card<digits>`
        let Some((card, _connector)) = name.split_once('-') else {
            continue;
        };
        let Some(digits) = card.strip_prefix("card") else {
            continue;
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(pci) = fs::canonicalize(drm.join(card).join("device")) else {
            continue;
        };
        let connected = read_text(&drm.join(&name).join("status")).is_some_and(|s| s.trim() == "connected");
        match out.iter_mut().find(|(p, _)| *p == pci) {
            Some((_, any)) => *any |= connected,
            None => out.push((pci, connected)),
        }
    }
    out
}

fn read_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_ATTR_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

fn read_u64(path: &Path) -> Option<u64> {
    read_text(path)?.trim().parse().ok()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct FakeSys {
        root: PathBuf,
    }

    impl FakeSys {
        fn new(name: &str) -> std::io::Result<Self> {
            let root = std::env::temp_dir().join(format!("cfa4-tasks-gpumem-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root)?;
            Ok(FakeSys { root })
        }

        fn pci(&self, slot: &str) -> std::io::Result<PathBuf> {
            let dir = self.root.join("sys/devices/pci0000:00").join(slot);
            fs::create_dir_all(&dir)?;
            Ok(dir)
        }

        /// `renderD*` and `card*` nodes whose `device` links point at the same PCI directory.
        fn gpu(&self, slot: &str, render: &str, card: &str, attrs: &[(&str, &str)]) -> std::io::Result<()> {
            let pci = self.pci(slot)?;
            for (k, v) in attrs {
                fs::write(pci.join(k), v)?;
            }
            for node in [render, card] {
                let dir = self.root.join("sys/class/drm").join(node);
                fs::create_dir_all(&dir)?;
                symlink(&pci, dir.join("device"))?;
            }
            Ok(())
        }

        fn connector(&self, name: &str, status: &str) -> std::io::Result<()> {
            let dir = self.root.join("sys/class/drm").join(name);
            fs::create_dir_all(&dir)?;
            fs::write(dir.join("status"), status)
        }
    }

    impl Drop for FakeSys {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn pci_addresses_parse_strictly() {
        let a = PciAddress { domain: 0, bus: 3, device: 0, function: 0 };
        assert_eq!(PciAddress::parse("0000:03:00.0"), Some(a));
        assert_eq!(PciAddress::parse("00000000:03:00.0\n"), Some(a), "NVML's eight-digit domain");
        assert_eq!(PciAddress::parse("0000:0A:1f.7"), Some(PciAddress { domain: 0, bus: 10, device: 31, function: 7 }));
        for bad in ["", "0000:03:00", "0000:03:20.0", "0000:03:00.8", "000000000:03:00.0", "0000:003:00.0", "x:03:00.0", "0000:03:00.0:1", "+0:03:00.0"] {
            assert_eq!(PciAddress::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn nvml_readings_are_tied_to_render_nodes_by_pci_address() {
        let sys = FakeSys::new("nvml").unwrap();
        sys.gpu("0000:01:00.0", "renderD128", "card0", &[]).unwrap();
        sys.gpu("0000:02:00.0", "renderD129", "card1", &[]).unwrap();
        let inv: Vec<String> = ["renderD128", "renderD129"].iter().map(|n: &&str| (*n).to_string()).collect();
        let pci = |bus| PciAddress { domain: 0, bus, device: 0, function: 0 };
        let readings = [
            PciMemoryReading { pci: pci(2), vram_total_bytes: Some(16 << 30), vram_used_bytes: Some(1 << 30) },
            PciMemoryReading { pci: pci(9), vram_total_bytes: Some(1), vram_used_bytes: Some(1) },
        ];
        let reports = reports_by_pci_root(&sys.root, &inv, &readings);
        assert_eq!(reports, vec![GpuMemoryReport { node: "renderD129".into(), vram_total_bytes: Some(16 << 30), vram_used_bytes: Some(1 << 30) }]);
        assert_eq!(node_pci_address_root(&sys.root, "../../etc"), None);
    }

    #[test]
    fn amd_publishes_vram_and_load_nvidia_stays_unknown() {
        let sys = FakeSys::new("mixed").unwrap();
        sys.gpu(
            "0000:03:00.0",
            "renderD128",
            "card0",
            &[("mem_info_vram_total", "12884901888\n"), ("mem_info_vram_used", "1073741824\n"), ("gpu_busy_percent", "37\n")],
        )
        .unwrap();
        sys.gpu("0000:01:00.0", "renderD129", "card1", &[]).unwrap();
        sys.connector("card0-DP-1", "disconnected\n").unwrap();
        sys.connector("card0-HDMI-A-1", "connected\n").unwrap();
        let inv: Vec<String> = ["renderD128", "renderD129"].iter().map(|n: &&str| (*n).to_string()).collect();
        let (mem, notes) = gpu_memory_root(&sys.root, &inv);
        assert!(notes.is_empty(), "{notes:?}");
        let amd = mem.first().unwrap();
        assert_eq!((amd.vram_total_bytes, amd.vram_used_bytes, amd.busy_percent), (Some(12 << 30), Some(1 << 30), Some(37)));
        assert_eq!(amd.vram_free_bytes(), Some(11 << 30));
        assert_eq!(amd.drives_display, Some(true));
        let nv = mem.get(1).unwrap();
        assert_eq!((nv.vram_total_bytes, nv.vram_used_bytes, nv.drives_display), (None, None, None), "unknown, not zero");
        assert_eq!(nv.vram_free_bytes(), None);
    }

    #[test]
    fn an_outside_report_fills_only_what_sysfs_left_unknown() {
        let mut nv = GpuMemory { node: "renderD129".into(), ..GpuMemory::default() };
        nv.apply_report(&GpuMemoryReport { node: "renderD128".into(), vram_total_bytes: Some(1), vram_used_bytes: Some(1) });
        assert_eq!(nv.vram_total_bytes, None, "another node's report is ignored");
        nv.apply_report(&GpuMemoryReport { node: "renderD129".into(), vram_total_bytes: Some(12 << 30), vram_used_bytes: Some(2 << 30) });
        assert_eq!(nv.vram_free_bytes(), Some(10 << 30));
        let mut amd = GpuMemory { node: "renderD128".into(), vram_total_bytes: Some(16 << 30), ..GpuMemory::default() };
        amd.apply_report(&GpuMemoryReport { node: "renderD128".into(), vram_total_bytes: Some(1), vram_used_bytes: Some(3) });
        assert_eq!((amd.vram_total_bytes, amd.vram_used_bytes), (Some(16 << 30), Some(3)), "the kernel's number wins");
    }

    #[test]
    fn hostile_values_are_unknown_never_panics() {
        let sys = FakeSys::new("hostile").unwrap();
        sys.gpu(
            "0000:03:00.0",
            "renderD128",
            "card0",
            &[("mem_info_vram_total", &"9".repeat(1 << 16)), ("mem_info_vram_used", "-5"), ("gpu_busy_percent", "250")],
        )
        .unwrap();
        sys.connector("cardX-DP-1", "connected").unwrap();
        sys.connector("card7-DP-1", "connected").unwrap();
        let inv: Vec<String> = ["renderD128", "renderD130"].iter().map(|n: &&str| (*n).to_string()).collect();
        let (mem, _) = gpu_memory_root(&sys.root, &inv);
        let first = mem.first().unwrap();
        assert_eq!((first.vram_total_bytes, first.vram_used_bytes, first.busy_percent, first.drives_display), (None, None, None, None));
        let used_over_total = GpuMemory { vram_total_bytes: Some(1), vram_used_bytes: Some(9), ..GpuMemory::default() };
        assert_eq!(used_over_total.vram_free_bytes(), Some(0));
        assert_eq!(mem.len(), 2, "a node without sysfs entries is still listed, all unknown");
    }
}
