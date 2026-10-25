//! The device inventory from Linux sysfs.
//!
//! GPUs: every `/sys/class/drm/renderD*` node (render nodes only, so a GPU with a display
//! connector is not counted once per connector). NPUs: every `/sys/class/accel/accel*` node,
//! the kernel's compute-accelerator subsystem (Intel `intel_vpu`, AMD `amdxdna`…). For each, the
//! PCI ids are read from `device/vendor` and `device/device`, and the driver from the name of
//! the `device/driver` link.
//!
//! Every read is bounded (a few bytes per file, [`MAX_NODES`] nodes per class) and every failure
//! is a note in [`Inventory::notes`], never an error or a panic: a machine whose sysfs we cannot
//! read simply has no accelerators, and the CPU backends run.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::inventory::{Device, DeviceKind, Inventory};
use crate::vendor::{Vendor, parse_hex_id};

/// At most this many nodes per class are read.
pub const MAX_NODES: usize = 64;
/// At most this many bytes are read from one sysfs attribute.
const MAX_ATTR_BYTES: u64 = 64;

/// The inventory of this machine (`/sys`). Empty outside Linux.
pub fn probe() -> Inventory {
    if cfg!(target_os = "linux") {
        let inventory = probe_root(Path::new("/"));
        for device in &inventory.devices {
            log::info!(
                "accelerator: {:?} {} ({:04x}:{:04x}) driver \"{}\" at {}",
                device.kind,
                device.vendor.name(),
                device.pci_vendor,
                device.pci_device,
                device.driver,
                device.node
            );
        }
        for note in &inventory.notes {
            log::info!("accelerator probe: {note}");
        }
        inventory
    } else {
        Inventory::default()
    }
}

/// The inventory of the sysfs tree under `root` (`root/sys/class/…`). Tests point it at a
/// synthetic tree.
pub fn probe_root(root: &Path) -> Inventory {
    let mut inventory = Inventory::default();
    let class = root.join("sys").join("class");
    scan(&class.join("drm"), "renderD", DeviceKind::Gpu, &mut inventory);
    scan(&class.join("accel"), "accel", DeviceKind::Npu, &mut inventory);
    inventory
}

fn scan(dir: &Path, prefix: &str, kind: DeviceKind, inventory: &mut Inventory) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // no such class on this kernel: nothing of this kind, nothing to note
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            inventory.notes.push(format!("{}: {err}", dir.display()));
            return;
        }
    };
    let mut nodes: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // `accel` must not match a hypothetical `accel_foo` class entry: digits only after it
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if nodes.len() >= MAX_NODES {
            inventory.notes.push(format!("{}: more than {MAX_NODES} {prefix} nodes, the rest are left out", dir.display()));
            break;
        }
        nodes.push(name);
    }
    // read_dir order is arbitrary; renderD128 before renderD129, accel2 before accel10
    nodes.sort_by_key(|n| (n.len(), n.clone()));
    for node in nodes {
        let device_dir = dir.join(&node).join("device");
        let pci_vendor = read_id(&device_dir.join("vendor"));
        let pci_device = read_id(&device_dir.join("device"));
        let driver =
            fs::read_link(device_dir.join("driver")).ok().and_then(|link| link.file_name().and_then(|n| n.to_str()).map(str::to_owned)).unwrap_or_default();
        if pci_vendor.is_none() && driver.is_empty() {
            inventory.notes.push(format!("{node}: no PCI vendor and no driver, left out"));
            continue;
        }
        let pci_vendor = pci_vendor.unwrap_or(0);
        inventory.devices.push(Device { kind, vendor: vendor_of(pci_vendor, &driver), pci_vendor, pci_device: pci_device.unwrap_or(0), driver, node });
    }
}

/// The vendor from the PCI id, or from the driver for devices without one (platform devices).
fn vendor_of(pci_vendor: u32, driver: &str) -> Vendor {
    match Vendor::from_pci(pci_vendor) {
        Vendor::Other => match driver {
            "nvidia" | "nouveau" | "nvidia-drm" => Vendor::Nvidia,
            "amdgpu" | "radeon" | "amdxdna" => Vendor::Amd,
            "i915" | "xe" | "intel_vpu" => Vendor::Intel,
            _ => Vendor::Other,
        },
        known => known,
    }
}

fn read_id(path: &Path) -> Option<u32> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_ATTR_BYTES).read_to_string(&mut text).ok()?;
    parse_hex_id(&text)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    /// A synthetic sysfs tree in a fresh temporary directory, removed on drop.
    struct FakeSys {
        root: PathBuf,
    }

    impl FakeSys {
        fn new(name: &str) -> std::io::Result<Self> {
            let root = std::env::temp_dir().join(format!("cfa4-accel-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root)?;
            Ok(FakeSys { root })
        }

        /// `class/node/device/{vendor,device}` and a `driver` link named `driver`.
        fn node(&self, class: &str, node: &str, vendor: Option<&str>, device: Option<&str>, driver: Option<&str>) -> std::io::Result<()> {
            let dir = self.root.join("sys/class").join(class).join(node).join("device");
            fs::create_dir_all(&dir)?;
            if let Some(v) = vendor {
                fs::write(dir.join("vendor"), v)?;
            }
            if let Some(d) = device {
                fs::write(dir.join("device"), d)?;
            }
            if let Some(drv) = driver {
                let target = self.root.join("sys/bus/pci/drivers").join(drv);
                fs::create_dir_all(&target)?;
                symlink(&target, dir.join("driver"))?;
            }
            Ok(())
        }
    }

    impl Drop for FakeSys {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_laptop_with_intel_graphics_an_nvidia_gpu_and_an_intel_npu() {
        let sys = FakeSys::new("laptop").unwrap();
        sys.node("drm", "card0", Some("0x8086\n"), Some("0x7d55\n"), Some("i915")).unwrap();
        sys.node("drm", "card0-eDP-1", None, None, None).unwrap();
        sys.node("drm", "renderD129", Some("0x10de\n"), Some("0x28a0\n"), Some("nvidia")).unwrap();
        sys.node("drm", "renderD128", Some("0x8086\n"), Some("0x7d55\n"), Some("i915")).unwrap();
        sys.node("accel", "accel0", Some("0x8086\n"), Some("0x7d1d\n"), Some("intel_vpu")).unwrap();
        let inv = probe_root(&sys.root);
        let summary: Vec<(DeviceKind, Vendor, &str, &str)> = inv.devices.iter().map(|d| (d.kind, d.vendor, d.driver.as_str(), d.node.as_str())).collect();
        assert_eq!(
            summary,
            vec![
                (DeviceKind::Gpu, Vendor::Intel, "i915", "renderD128"),
                (DeviceKind::Gpu, Vendor::Nvidia, "nvidia", "renderD129"),
                (DeviceKind::Npu, Vendor::Intel, "intel_vpu", "accel0"),
            ]
        );
        assert_eq!(inv.devices.get(1).map(|d| d.pci_device), Some(0x28a0));
        assert!(inv.notes.is_empty(), "{:?}", inv.notes);
    }

    #[test]
    fn a_machine_without_sysfs_classes_has_no_accelerators() {
        let sys = FakeSys::new("empty").unwrap();
        let inv = probe_root(&sys.root);
        assert!(inv.devices.is_empty() && inv.notes.is_empty());
        let missing = probe_root(&sys.root.join("does-not-exist"));
        assert_eq!(missing, Inventory::default());
    }

    #[test]
    fn hostile_entries_are_notes_or_skipped_never_panics() {
        let sys = FakeSys::new("hostile").unwrap();
        // an id file far longer than any id: only the first bytes are read, and they do not parse
        sys.node("drm", "renderD128", Some(&"f".repeat(1 << 20)), Some("garbage"), Some("amdgpu")).unwrap();
        // nothing readable at all
        sys.node("drm", "renderD129", None, None, None).unwrap();
        // names that only look like nodes
        sys.node("drm", "renderD", Some("0x1002"), None, Some("amdgpu")).unwrap();
        sys.node("drm", "renderDx1", Some("0x1002"), None, Some("amdgpu")).unwrap();
        sys.node("accel", "accel_ctl", Some("0x1022"), None, Some("amdxdna")).unwrap();
        // a platform NPU without a PCI id: the vendor comes from its driver
        sys.node("accel", "accel10", None, None, Some("amdxdna")).unwrap();
        sys.node("accel", "accel2", Some("0x1022"), Some("0x1502"), Some("amdxdna")).unwrap();
        let inv = probe_root(&sys.root);
        let nodes: Vec<&str> = inv.devices.iter().map(|d| d.node.as_str()).collect();
        assert_eq!(nodes, vec!["renderD128", "accel2", "accel10"]);
        let first = inv.devices.first().unwrap();
        assert_eq!((first.vendor, first.pci_vendor, first.pci_device), (Vendor::Amd, 0, 0), "vendor from the driver");
        assert_eq!(inv.devices.get(2).map(|d| d.vendor), Some(Vendor::Amd));
        assert_eq!(inv.notes.len(), 1, "{:?}", inv.notes);
    }

    #[test]
    fn too_many_nodes_are_capped_with_a_note() {
        let sys = FakeSys::new("many").unwrap();
        for i in 0..(MAX_NODES + 5) {
            sys.node("drm", &format!("renderD{}", 128 + i), Some("0x1002"), Some("0x744c"), Some("amdgpu")).unwrap();
        }
        let inv = probe_root(&sys.root);
        assert_eq!(inv.devices.len(), MAX_NODES);
        assert!(inv.notes.iter().any(|n| n.contains("more than")));
    }

    #[test]
    fn the_real_machine_probes_without_panicking() {
        let inv = probe();
        for device in &inv.devices {
            assert!(!device.node.is_empty());
        }
    }
}
