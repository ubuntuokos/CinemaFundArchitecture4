//! Device probe from what Linux shows in `/sys` (and `/proc` for NVIDIA's proprietary driver).
//!
//! - GPUs: every `/sys/class/drm/renderD<n>` node (render nodes only, so a GPU with display
//!   connectors is not counted once per connector).
//! - NPUs: every `/sys/class/accel/accel<n>` node, the kernel's compute-accelerator subsystem
//!   (Intel `intel_vpu`, AMD `amdxdna`).
//! - NVIDIA GPUs on the proprietary driver without a render node (no `nvidia-drm` loaded; CUDA,
//!   NVDEC and NVENC still work there): `/proc/driver/nvidia/gpus/<pci address>`, read only when
//!   no render node is on that driver.
//!
//! For each node the PCI vendor and device ids come from `device/vendor` and `device/device`, the
//! driver from the name of the `device/driver` link; a device without a PCI id (a platform NPU)
//! takes its vendor from its driver.
//!
//! It only reads small text files and symlinks: no driver is loaded and no device node is opened.
//! Whether a backend can use a device (a driver library present, a profile supported) is the
//! backend's own question ([`crate::Backend::accepts`]). Every read is bounded ([`MAX_ATTR_BYTES`]
//! per attribute, [`MAX_DEVICES`] nodes per class) and every failure is a note in the inventory,
//! never an error or a panic: a machine whose `/sys` cannot be read simply has the CPU. The root
//! directory is a parameter so tests use a fake tree; production passes `/`.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::{Device, DeviceId, DeviceKind, Inventory, MAX_DEVICES, Vendor, parse_pci_id};

/// Largest `/sys` attribute read (bytes); an id is a few bytes, anything longer does not parse.
pub const MAX_ATTR_BYTES: u64 = 64;
/// Longest directory entry name considered.
const MAX_NAME_LEN: usize = 32;

fn read_attr(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_ATTR_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

fn read_id(path: &Path) -> Option<u16> {
    read_attr(path).as_deref().and_then(parse_pci_id)
}

fn driver_name(device_dir: &Path) -> Option<String> {
    let target = fs::read_link(device_dir.join("driver")).ok()?;
    let name = target.file_name()?.to_str()?;
    (!name.is_empty() && name.len() <= 64).then(|| name.to_string())
}

/// The nodes of `dir` named `prefix` followed by digits only, in numeric order (`renderD128`
/// before `renderD129`, `accel2` before `accel10`), at most [`MAX_DEVICES`] (with a note past
/// that). A missing directory is no nodes and no note: the kernel has no such class.
fn nodes(dir: &Path, prefix: &str, inv: &mut Inventory) -> Vec<String> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            inv.note(format!("{}: {err}", dir.display()));
            return Vec::new();
        }
    };
    let mut names = Vec::new();
    for entry in rd.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() || name.len() > MAX_NAME_LEN || !rest.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if names.len() >= MAX_DEVICES {
            inv.note(format!("{}: more than {MAX_DEVICES} {prefix} nodes, the rest are left out", dir.display()));
            break;
        }
        names.push(name);
    }
    names.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
    names
}

fn device_name(kind: DeviceKind, vendor: Vendor, driver: Option<&str>, node: &str) -> String {
    let kind = match kind {
        DeviceKind::Gpu => "GPU",
        DeviceKind::Npu => "NPU",
        DeviceKind::Cpu => "CPU",
    };
    match driver {
        Some(d) => format!("{} {kind} ({d}, {node})", vendor.name()),
        None => format!("{} {kind} ({node})", vendor.name()),
    }
}

fn scan(inv: &mut Inventory, dir: &Path, prefix: &str, kind: DeviceKind, id_prefix: &str) {
    for node in nodes(dir, prefix, inv) {
        let device_dir = dir.join(&node).join("device");
        let pci_vendor = read_id(&device_dir.join("vendor"));
        let pci_device = read_id(&device_dir.join("device"));
        let driver = driver_name(&device_dir);
        if pci_vendor.is_none() && driver.is_none() {
            inv.note(format!("{node}: no PCI vendor and no driver, left out"));
            continue;
        }
        let vendor = match pci_vendor.map(Vendor::from_pci) {
            Some(v) if v != Vendor::Other => v,
            _ => driver.as_deref().map_or(Vendor::Other, Vendor::from_driver),
        };
        let Ok(id) = DeviceId::new(format!("{id_prefix}:{}", node.to_ascii_lowercase())) else {
            continue;
        };
        let name = device_name(kind, vendor, driver.as_deref(), &node);
        inv.add(Device { id, kind, vendor, name, driver, pci_device, free_memory_mb: None });
    }
}

/// NVIDIA GPUs the proprietary driver lists in `/proc/driver/nvidia/gpus` (one directory per PCI
/// address), when no render node is on that driver.
fn scan_nvidia_proc(inv: &mut Inventory, root: &Path) {
    if inv.gpus().any(Device::is_nvidia_proprietary) {
        return;
    }
    let Ok(rd) = fs::read_dir(root.join("proc/driver/nvidia/gpus")) else {
        return;
    };
    let mut addrs: Vec<String> = rd
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.is_empty() && n.len() <= MAX_NAME_LEN && n.bytes().all(|b| b.is_ascii_hexdigit() || matches!(b, b':' | b'.')))
        .take(MAX_DEVICES)
        .collect();
    addrs.sort();
    for addr in addrs {
        let Ok(id) = DeviceId::new(format!("gpu:nvidia:{}", addr.to_ascii_lowercase())) else {
            continue;
        };
        let name = device_name(DeviceKind::Gpu, Vendor::Nvidia, Some("nvidia"), &addr);
        inv.add(Device { id, kind: DeviceKind::Gpu, vendor: Vendor::Nvidia, name, driver: Some("nvidia".into()), pci_device: None, free_memory_mb: None });
    }
}

/// The devices under `root` (`/` on a real system). The CPU is always in the result; a missing or
/// unreadable `/sys` gives just the CPU.
pub fn probe(root: &Path) -> Inventory {
    let mut inv = Inventory::default();
    let class = root.join("sys/class");
    scan(&mut inv, &class.join("drm"), "renderD", DeviceKind::Gpu, "gpu");
    scan(&mut inv, &class.join("accel"), "accel", DeviceKind::Npu, "npu");
    scan_nvidia_proc(&mut inv, root);
    inv
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    /// A synthetic `/sys` tree in a fresh temporary directory, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let dir = std::env::temp_dir().join(format!("cfa4-route-probe-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }

        /// `class/node/device/{vendor,device}` and a `driver` link named after the driver.
        fn node(&self, class: &str, node: &str, vendor: Option<&str>, device: Option<&str>, driver: Option<&str>) {
            let dev = self.0.join("sys/class").join(class).join(node).join("device");
            fs::create_dir_all(&dev).unwrap();
            if let Some(v) = vendor {
                fs::write(dev.join("vendor"), v).unwrap();
            }
            if let Some(d) = device {
                fs::write(dev.join("device"), d).unwrap();
            }
            if let Some(d) = driver {
                let target = self.0.join("sys/bus/pci/drivers").join(d);
                fs::create_dir_all(&target).unwrap();
                symlink(&target, dev.join("driver")).unwrap();
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn summary(inv: &Inventory) -> Vec<(&str, DeviceKind, Vendor, Option<&str>)> {
        inv.devices().iter().map(|d| (d.id.as_str(), d.kind, d.vendor, d.driver.as_deref())).collect()
    }

    #[test]
    fn no_sys_gives_the_cpu() {
        let inv = probe(Path::new("/nonexistent/cfa4"));
        assert_eq!(inv.devices().len(), 1);
        assert_eq!(inv.devices()[0].kind, DeviceKind::Cpu);
        assert!(inv.notes().is_empty());
        let t = Tree::new("empty");
        assert_eq!(probe(&t.0), Inventory::default());
    }

    #[test]
    fn a_laptop_with_intel_graphics_an_nvidia_gpu_and_npus() {
        let t = Tree::new("laptop");
        t.node("drm", "card0", Some("0x8086\n"), Some("0x7d55\n"), Some("i915")); // not a render node
        t.node("drm", "card0-eDP-1", None, None, None);
        t.node("drm", "renderD129", Some("0x10de\n"), Some("0x28a0\n"), Some("nvidia"));
        t.node("drm", "renderD128", Some("0x8086\n"), Some("0x7d55\n"), Some("i915"));
        t.node("accel", "accel0", None, None, Some("intel_vpu"));
        t.node("accel", "accel1", Some("0x1022\n"), Some("0x1502\n"), Some("amdxdna"));
        let inv = probe(&t.0);
        assert_eq!(
            summary(&inv),
            vec![
                ("cpu", DeviceKind::Cpu, Vendor::Other, None),
                ("gpu:renderd128", DeviceKind::Gpu, Vendor::Intel, Some("i915")),
                ("gpu:renderd129", DeviceKind::Gpu, Vendor::Nvidia, Some("nvidia")),
                ("npu:accel0", DeviceKind::Npu, Vendor::Intel, Some("intel_vpu")),
                ("npu:accel1", DeviceKind::Npu, Vendor::Amd, Some("amdxdna")),
            ]
        );
        assert_eq!(inv.devices()[2].name, "NVIDIA GPU (nvidia, renderD129)");
        assert_eq!(inv.devices()[2].pci_device, Some(0x28a0));
        assert!(inv.devices()[2].is_nvidia_proprietary() && inv.devices()[1].has_vaapi_driver());
        assert!(inv.notes().is_empty(), "{:?}", inv.notes());
    }

    #[test]
    fn nvidia_without_a_render_node_comes_from_proc() {
        let t = Tree::new("nvproc");
        t.node("drm", "renderD128", Some("0x1002"), None, Some("amdgpu"));
        fs::create_dir_all(t.0.join("proc/driver/nvidia/gpus/0000:01:00.0")).unwrap();
        fs::create_dir_all(t.0.join("proc/driver/nvidia/gpus/not a gpu")).unwrap();
        let inv = probe(&t.0);
        let nv: Vec<_> = inv.gpus().filter(|d| d.is_nvidia_proprietary()).map(|d| d.id.as_str()).collect();
        assert_eq!(nv, vec!["gpu:nvidia:0000:01:00.0"]);

        // with a render node on the proprietary driver, /proc adds nothing
        t.node("drm", "renderD129", Some("0x10de"), None, Some("nvidia"));
        let inv = probe(&t.0);
        assert_eq!(inv.gpus().filter(|d| d.vendor == Vendor::Nvidia).count(), 1);
    }

    #[test]
    fn hostile_sys_contents_are_notes_or_skipped_never_panics() {
        let t = Tree::new("hostile");
        // an id file far longer than any id: only the first bytes are read, and they do not parse;
        // the vendor then comes from the driver
        t.node("drm", "renderD128", Some(&"f".repeat(1 << 20)), Some("garbage"), Some("amdgpu"));
        // nothing readable at all
        t.node("drm", "renderD129", None, None, None);
        t.node("drm", "renderD130", Some("not hex"), None, None);
        // a vendor attribute that is a directory
        fs::create_dir_all(t.0.join("sys/class/drm/renderD131/device/vendor")).unwrap();
        // names that only look like nodes
        t.node("drm", "renderD", Some("0x1002"), None, Some("amdgpu"));
        t.node("drm", "renderDx1", Some("0x1002"), None, Some("amdgpu"));
        t.node("accel", "accel_ctl", Some("0x1022"), None, Some("amdxdna"));
        // a platform NPU without a PCI id: the vendor comes from its driver
        t.node("accel", "accel10", None, None, Some("amdxdna"));
        t.node("accel", "accel2", Some("0x1022"), Some("0x1502"), Some("amdxdna"));
        let inv = probe(&t.0);
        let ids: Vec<&str> = inv.devices().iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["cpu", "gpu:renderd128", "npu:accel2", "npu:accel10"]);
        let first = &inv.devices()[1];
        assert_eq!((first.vendor, first.pci_device), (Vendor::Amd, None), "vendor from the driver");
        assert_eq!(inv.devices()[3].vendor, Vendor::Amd);
        assert_eq!(inv.notes().len(), 3, "{:?}", inv.notes());
    }

    #[test]
    fn too_many_nodes_are_capped_with_a_note() {
        let t = Tree::new("many");
        for i in 0..(MAX_DEVICES + 5) {
            t.node("drm", &format!("renderD{}", 128 + i), Some("0x1002"), Some("0x744c"), Some("amdgpu"));
        }
        for i in 0..200 {
            fs::create_dir_all(t.0.join(format!("sys/class/accel/accel{i}/device"))).unwrap();
        }
        let inv = probe(&t.0);
        assert_eq!(inv.devices().len(), MAX_DEVICES, "the CPU plus 63 GPUs");
        assert!(inv.notes().iter().any(|n| n.contains("more than")));
        assert!(inv.notes().len() <= crate::MAX_NOTES);
    }

    #[test]
    fn the_real_machine_probes_without_panicking() {
        let inv = probe(Path::new("/"));
        assert_eq!(inv.devices()[0].kind, DeviceKind::Cpu);
        assert!(inv.devices().len() <= MAX_DEVICES);
    }
}
