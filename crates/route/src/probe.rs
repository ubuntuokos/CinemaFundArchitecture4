//! Device probe from what Linux shows in `/sys`: GPUs through their DRM render nodes, NPUs through
//! the accel subsystem (Intel `intel_vpu`, AMD `amdxdna`).
//!
//! It only reads small text files and symlinks: no driver is loaded and no device node is opened.
//! Whether a backend can use a device (a driver library present, a profile supported) is the
//! backend's own question ([`crate::Backend::accepts`]). The root directory is a parameter so tests
//! can use a fake tree; production passes `/`.

use std::fs;
use std::path::Path;

use crate::{Device, DeviceId, DeviceKind, Inventory, MAX_DEVICES, Vendor};

/// Largest `/sys` attribute file read.
const MAX_ATTR_BYTES: u64 = 4096;

fn read_attr(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_ATTR_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    Some(text.trim().to_string())
}

fn pci_vendor(device_dir: &Path) -> Vendor {
    read_attr(&device_dir.join("vendor")).and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok()).map_or(Vendor::Other, Vendor::from_pci)
}

fn driver_name(device_dir: &Path) -> Option<String> {
    let target = fs::read_link(device_dir.join("driver")).ok()?;
    let name = target.file_name()?.to_str()?;
    (!name.is_empty() && name.len() <= 64).then(|| name.to_string())
}

/// Entries of `dir` whose names start with `prefix`, sorted, at most [`MAX_DEVICES`].
fn entries(dir: &Path, prefix: &str) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with(prefix) && n.len() <= 32)
        .take(MAX_DEVICES * 4)
        .collect();
    names.sort();
    names.truncate(MAX_DEVICES);
    names
}

fn device_name(kind: DeviceKind, vendor: Vendor, driver: Option<&str>, node: &str) -> String {
    let vendor = match vendor {
        Vendor::Nvidia => "NVIDIA",
        Vendor::Amd => "AMD",
        Vendor::Intel => "Intel",
        Vendor::Other => "Other",
    };
    let kind = match kind {
        DeviceKind::Gpu => "GPU",
        DeviceKind::Npu => "NPU",
        DeviceKind::Cpu => "CPU",
    };
    match driver {
        Some(d) => format!("{vendor} {kind} ({d}, {node})"),
        None => format!("{vendor} {kind} ({node})"),
    }
}

/// The devices under `root` (`/` on a real system). The CPU is always in the result; a missing or
/// unreadable `/sys` gives just the CPU.
pub fn probe(root: &Path) -> Inventory {
    let mut inv = Inventory::default();

    let drm = root.join("sys/class/drm");
    for node in entries(&drm, "renderD") {
        let dir = drm.join(&node).join("device");
        let vendor = pci_vendor(&dir);
        let driver = driver_name(&dir);
        let Ok(id) = DeviceId::new(format!("gpu:{}", node.to_ascii_lowercase())) else {
            continue;
        };
        let name = device_name(DeviceKind::Gpu, vendor, driver.as_deref(), &node);
        inv.add(Device { id, kind: DeviceKind::Gpu, vendor, name, free_memory_mb: None });
    }

    let accel = root.join("sys/class/accel");
    for node in entries(&accel, "accel") {
        let dir = accel.join(&node).join("device");
        let driver = driver_name(&dir);
        let mut vendor = pci_vendor(&dir);
        if vendor == Vendor::Other {
            vendor = match driver.as_deref() {
                Some("intel_vpu") => Vendor::Intel,
                Some("amdxdna") => Vendor::Amd,
                _ => Vendor::Other,
            };
        }
        let Ok(id) = DeviceId::new(format!("npu:{}", node.to_ascii_lowercase())) else {
            continue;
        };
        let name = device_name(DeviceKind::Npu, vendor, driver.as_deref(), &node);
        inv.add(Device { id, kind: DeviceKind::Npu, vendor, name, free_memory_mb: None });
    }
    inv
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let dir = std::env::temp_dir().join(format!("cfa4-route-probe-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }
        fn node(&self, class: &str, node: &str, vendor: Option<&str>, driver: Option<&str>) {
            let dev = self.0.join("sys/class").join(class).join(node).join("device");
            fs::create_dir_all(&dev).unwrap();
            if let Some(v) = vendor {
                fs::write(dev.join("vendor"), format!("{v}\n")).unwrap();
            }
            if let Some(d) = driver {
                let target = self.0.join("sys/bus/pci/drivers").join(d);
                fs::create_dir_all(&target).unwrap();
                std::os::unix::fs::symlink(&target, dev.join("driver")).unwrap();
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_sys_gives_the_cpu() {
        let inv = probe(Path::new("/nonexistent/cfa4"));
        assert_eq!(inv.devices().len(), 1);
        assert_eq!(inv.devices()[0].kind, DeviceKind::Cpu);
    }

    #[test]
    fn gpus_and_npus_are_found_with_their_vendor() {
        let t = Tree::new("mixed");
        t.node("drm", "renderD128", Some("0x10de"), Some("nvidia"));
        t.node("drm", "renderD129", Some("0x8086"), Some("i915"));
        t.node("drm", "card0", Some("0x10de"), None); // not a render node
        t.node("accel", "accel0", None, Some("intel_vpu"));
        t.node("accel", "accel1", Some("0x1022"), Some("amdxdna"));
        let inv = probe(&t.0);
        let got: Vec<_> = inv.devices().iter().map(|d| (d.id.as_str(), d.kind, d.vendor)).collect();
        assert_eq!(
            got,
            vec![
                ("cpu", DeviceKind::Cpu, Vendor::Other),
                ("gpu:renderd128", DeviceKind::Gpu, Vendor::Nvidia),
                ("gpu:renderd129", DeviceKind::Gpu, Vendor::Intel),
                ("npu:accel0", DeviceKind::Npu, Vendor::Intel),
                ("npu:accel1", DeviceKind::Npu, Vendor::Amd),
            ]
        );
        assert_eq!(inv.devices()[1].name, "NVIDIA GPU (nvidia, renderD128)");
    }

    #[test]
    fn hostile_sys_contents_do_not_break_the_probe() {
        let t = Tree::new("hostile");
        t.node("drm", "renderD128", Some("not hex"), None);
        t.node("drm", "renderD129", Some(""), None);
        let big = t.0.join("sys/class/drm/renderD130/device");
        fs::create_dir_all(&big).unwrap();
        fs::write(big.join("vendor"), "0".repeat(100_000)).unwrap();
        // A vendor attribute that is a directory.
        fs::create_dir_all(t.0.join("sys/class/drm/renderD131/device/vendor")).unwrap();
        for i in 0..200 {
            fs::create_dir_all(t.0.join(format!("sys/class/accel/accel{i}/device"))).unwrap();
        }
        let inv = probe(&t.0);
        assert!(inv.devices().len() <= MAX_DEVICES);
        assert!(inv.devices().iter().filter(|d| d.kind == DeviceKind::Gpu).all(|d| d.vendor == Vendor::Other));
    }
}
