//! HID jog/shuttle controllers through `/sys/class/hidraw` (HID evaluation: hidraw first).
//!
//! Each `hidrawN` directory has a `device/uevent` file with `HID_ID=<bus>:<vendor>:<product>`,
//! `HID_NAME` and `HID_PHYS`. Only the devices CFA4 has a driver for are listed. Whether the node
//! in `/dev` can be read is checked by opening it: without the `uaccess` udev rule it cannot, and
//! the device is listed with the reason, so the user knows what to fix. When the hidraw node
//! cannot be opened but the device's `/dev/input/eventN` node can, that node is the port and the
//! reader decodes evdev records instead (decision N-02).

use crate::DeviceInfo;
use cfa4_surface_model::{DeviceId, DeviceState, SurfaceProtocol};
use std::fs;
use std::path::{Path, PathBuf};

/// Contour Design's USB vendor id.
pub const CONTOUR_VID: u16 = 0x0b33;

/// The supported devices: (vendor, product, name, buttons).
pub const KNOWN: &[(u16, u16, &str, u8)] =
    &[(CONTOUR_VID, 0x0010, "Contour ShuttlePRO", 13), (CONTOUR_VID, 0x0020, "Contour ShuttleXpress", 5), (CONTOUR_VID, 0x0030, "Contour ShuttlePRO v2", 15)];

/// The fields of a hidraw `uevent` file CFA4 uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uevent {
    pub bus: u16,
    pub vid: u16,
    pub pid: u16,
    pub name: String,
    pub phys: String,
}

/// Reads `HID_ID=0003:00000B33:00000030`, `HID_NAME=…` and `HID_PHYS=…`; `None` without a
/// valid `HID_ID`.
pub fn parse_uevent(text: &str) -> Option<Uevent> {
    let mut u = Uevent::default();
    let mut have_id = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "HID_ID" => {
                let mut it = v.trim().split(':');
                let (Some(b), Some(vd), Some(pd)) = (it.next(), it.next(), it.next()) else { return None };
                u.bus = u16::try_from(u32::from_str_radix(b, 16).ok()?).ok()?;
                u.vid = u16::try_from(u32::from_str_radix(vd, 16).ok()?).ok()?;
                u.pid = u16::try_from(u32::from_str_radix(pd, 16).ok()?).ok()?;
                have_id = true;
            }
            "HID_NAME" => u.name = v.trim().to_string(),
            "HID_PHYS" => u.phys = v.trim().to_string(),
            _ => {}
        }
    }
    have_id.then_some(u)
}

/// The supported device a `uevent` describes.
pub fn known(vid: u16, pid: u16) -> Option<&'static (u16, u16, &'static str, u8)> {
    KNOWN.iter().find(|k| k.0 == vid && k.1 == pid)
}

/// Lists the supported HID devices under `sys_hidraw` (normally `/sys/class/hidraw`), with
/// their nodes in `dev` (normally `/dev`). Sorted by node name.
pub fn scan(sys_hidraw: &Path, dev: &Path) -> Result<Vec<DeviceInfo>, String> {
    let entries = fs::read_dir(sys_hidraw).map_err(|e| format!("HID: cannot read {} ({e}); jog/shuttle controllers are not listed", sys_hidraw.display()))?;
    let mut names: Vec<String> =
        entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("hidraw")).collect();
    names.sort_by_key(|n| (n.len(), n.clone()));
    let mut out = Vec::new();
    for n in names {
        let Ok(text) = fs::read_to_string(sys_hidraw.join(&n).join("device").join("uevent")) else { continue };
        let Some(u) = parse_uevent(&text) else { continue };
        let Some(&(_, _, model, _)) = known(u.vid, u.pid) else { continue };
        let mut node = dev.join(&n);
        let mut error = match fs::File::open(&node) {
            Ok(_) => None,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Some("no permission to read the device: install CFA4's udev rule (uaccess) and plug it in again".to_string())
            }
            Err(e) => Some(format!("cannot open {}: {e}", node.display())),
        };
        // the evdev node of the same device is the fallback (decision N-02)
        if error.is_some()
            && let Some(event) = evdev_node(&sys_hidraw.join(&n), dev)
            && fs::File::open(&event).is_ok()
        {
            node = event;
            error = None;
        }
        let phys = if u.phys.is_empty() { n.clone() } else { u.phys.clone() };
        out.push(DeviceInfo {
            device: DeviceId::hid(u.vid, u.pid, &phys),
            name: if u.name.is_empty() { model.to_string() } else { u.name.clone() },
            protocol: SurfaceProtocol::Hid,
            driver: "hid-shuttle".into(),
            state: if error.is_none() { DeviceState::Online } else { DeviceState::Error },
            port: node.to_string_lossy().into_owned(),
            vid: Some(u.vid),
            pid: Some(u.pid),
            error,
        });
    }
    Ok(out)
}

/// The `/dev/input/eventN` node of the device behind a hidraw directory, from
/// `<hidrawN>/device/input/input*/event*`. The first one in name order when there are several.
pub fn evdev_node(hidraw_dir: &Path, dev: &Path) -> Option<PathBuf> {
    let inputs = fs::read_dir(hidraw_dir.join("device").join("input")).ok()?;
    let mut events: Vec<String> = inputs
        .filter_map(|e| e.ok())
        .filter_map(|input| fs::read_dir(input.path()).ok())
        .flat_map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.strip_prefix("event").is_some_and(|k| !k.is_empty() && k.bytes().all(|c| c.is_ascii_digit())))
        .collect();
    events.sort_by_key(|n| (n.len(), n.clone()));
    events.first().map(|e| dev.join("input").join(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tmp {
        let d = std::env::temp_dir().join(format!("cfa4-surface-hid-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        Tmp(d)
    }

    fn add(sys: &Path, dev: &Path, node: &str, uevent: &str) {
        let dir = sys.join(node).join("device");
        assert!(fs::create_dir_all(&dir).is_ok());
        assert!(fs::write(dir.join("uevent"), uevent).is_ok());
        assert!(fs::create_dir_all(dev).is_ok());
        assert!(fs::write(dev.join(node), b"").is_ok());
    }

    #[test]
    fn uevent_fields() {
        let u = parse_uevent("DRIVER=hid-generic\nHID_ID=0003:00000B33:00000030\nHID_NAME=Contour Design ShuttlePRO v2\nHID_PHYS=usb-0000:00:14.0-3/input0\n");
        assert_eq!(u, Some(Uevent { bus: 3, vid: 0x0b33, pid: 0x0030, name: "Contour Design ShuttlePRO v2".into(), phys: "usb-0000:00:14.0-3/input0".into() }));
        assert_eq!(parse_uevent("HID_NAME=x\n"), None);
        assert_eq!(parse_uevent("HID_ID=0003:zz:00000030\n"), None);
        assert_eq!(parse_uevent("HID_ID=0003:FFFFFFFF:00000030\n"), None, "out of range is rejected, not truncated");
        assert_eq!(parse_uevent("HID_ID=0003\n"), None);
    }

    #[test]
    fn scan_lists_only_supported_devices() {
        let t = tree("scan");
        let (sys, dev) = (t.0.join("sys"), t.0.join("dev"));
        add(&sys, &dev, "hidraw10", "HID_ID=0003:00000B33:00000020\nHID_NAME=Contour ShuttleXpress\nHID_PHYS=usb-1/input0\n");
        add(&sys, &dev, "hidraw2", "HID_ID=0003:00000B33:00000030\nHID_PHYS=usb-2/input0\n");
        add(&sys, &dev, "hidraw3", "HID_ID=0003:0000046D:0000C52B\nHID_NAME=Logitech receiver\n");
        add(&sys, &dev, "hidraw4", "garbage");
        let found = scan(&sys, &dev).unwrap_or_default();
        let names: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Contour ShuttlePRO v2", "Contour ShuttleXpress"], "numeric order, unknown devices left out");
        assert_eq!(found[0].device.0, "hid:0b33:0030:usb-2/input0");
        assert_eq!(found[0].driver, "hid-shuttle");
        assert_eq!(found[0].state, DeviceState::Online);
        assert!(found[0].port.ends_with("hidraw2"));
    }

    #[test]
    fn a_missing_node_is_listed_with_its_reason() {
        let t = tree("missing");
        let (sys, dev) = (t.0.join("sys"), t.0.join("dev"));
        add(&sys, &dev, "hidraw0", "HID_ID=0003:00000B33:00000010\n");
        assert!(fs::remove_file(dev.join("hidraw0")).is_ok());
        let found = scan(&sys, &dev).unwrap_or_default();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].state, DeviceState::Error);
        assert!(found[0].error.as_deref().unwrap_or_default().contains("cannot open"));
        assert_eq!(found[0].name, "Contour ShuttlePRO");
    }

    fn add_event(sys: &Path, dev: &Path, node: &str, event: &str) {
        let dir = sys.join(node).join("device").join("input").join("input7");
        assert!(fs::create_dir_all(dir.join(event)).is_ok());
        assert!(fs::create_dir_all(dir.join("mouse2")).is_ok());
        assert!(fs::create_dir_all(dev.join("input")).is_ok());
        assert!(fs::write(dev.join("input").join(event), b"").is_ok());
    }

    #[test]
    fn an_unreadable_hidraw_node_falls_back_to_evdev() {
        let t = tree("evdev");
        let (sys, dev) = (t.0.join("sys"), t.0.join("dev"));
        add(&sys, &dev, "hidraw0", "HID_ID=0003:00000B33:00000030\n");
        add_event(&sys, &dev, "hidraw0", "event12");
        assert!(fs::remove_file(dev.join("hidraw0")).is_ok());
        let found = scan(&sys, &dev).unwrap_or_default();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].state, DeviceState::Online);
        assert_eq!(found[0].error, None);
        assert!(found[0].port.ends_with("input/event12"), "{}", found[0].port);
        assert!(crate::hid_in::is_evdev(Path::new(&found[0].port)));
    }

    #[test]
    fn a_readable_hidraw_node_wins_over_evdev() {
        let t = tree("hidraw-first");
        let (sys, dev) = (t.0.join("sys"), t.0.join("dev"));
        add(&sys, &dev, "hidraw0", "HID_ID=0003:00000B33:00000030\n");
        add_event(&sys, &dev, "hidraw0", "event3");
        let found = scan(&sys, &dev).unwrap_or_default();
        assert!(found[0].port.ends_with("hidraw0"));
    }

    #[test]
    fn the_evdev_node_is_found_in_sysfs() {
        let t = tree("evdev-node");
        let (sys, dev) = (t.0.join("sys"), t.0.join("dev"));
        add(&sys, &dev, "hidraw1", "HID_ID=0003:00000B33:00000030\n");
        assert_eq!(evdev_node(&sys.join("hidraw1"), &dev), None, "no input directory");
        add_event(&sys, &dev, "hidraw1", "event10");
        add_event(&sys, &dev, "hidraw1", "event9");
        assert_eq!(evdev_node(&sys.join("hidraw1"), &dev), Some(dev.join("input").join("event9")));
    }

    #[test]
    fn no_sysfs_is_an_error_not_a_panic() {
        assert!(scan(Path::new("/nonexistent/cfa4"), Path::new("/dev")).is_err());
    }
}
