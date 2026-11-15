//! Settings ▸ Control Surfaces: MIDI controllers and jog/shuttle devices
//! (CFA4-CONTROL-SURFACE-001, `docs/adr/ADR-0012-control-surface.md`).
//!
//! This first step keeps what the user chooses per device (use it or not, which profile) in the
//! preferences (`controlSurfaces.*`). The device list itself comes from the host process
//! (`cfa4-cli serve` ▸ `surface.devices`), which owns the device I/O (L5). Mapping, MIDI Learn and
//! the MIDI Monitor come in the next steps; until then the settings are remembered and shown.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{Result, Session};

/// One device's settings. A list, not a map keyed by device id: ids contain dots and colons, which
/// dotted preference keys cannot carry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceSetting {
    /// `hid:<vid>:<pid>:<phys>` or `midi:<client>:<port>` (as `surface.devices` names it).
    pub device: String,
    pub enabled: bool,
    /// A profile id from [`PROFILES`] (empty = the driver's default).
    pub profile: String,
}

impl Default for DeviceSetting {
    fn default() -> Self {
        Self { device: String::new(), enabled: true, profile: String::new() }
    }
}

/// Settings ▸ Control Surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlSurfacePrefs {
    /// Use control surfaces at all.
    pub enabled: bool,
    /// Per-device choices, for devices the user changed something on.
    pub devices: Vec<DeviceSetting>,
}

impl Default for ControlSurfacePrefs {
    fn default() -> Self {
        Self { enabled: true, devices: Vec::new() }
    }
}

impl ControlSurfacePrefs {
    /// The settings of `device` (the defaults when the user never changed them).
    pub fn device(&self, device: &str) -> DeviceSetting {
        self.devices.iter().find(|d| d.device == device).cloned().unwrap_or_else(|| DeviceSetting { device: device.into(), ..Default::default() })
    }

    /// Drops entries without a device id and duplicates (the first one wins).
    pub fn clamp(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        self.devices.retain(|d| {
            if d.device.is_empty() || seen.contains(&d.device) {
                return false;
            }
            seen.push(d.device.clone());
            true
        });
        for d in &mut self.devices {
            if !d.profile.is_empty() && !PROFILES.iter().any(|p| p.id == d.profile) {
                d.profile.clear();
            }
        }
    }
}

/// A built-in profile.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinProfile {
    pub id: &'static str,
    pub name: &'static str,
    /// The device driver it is for (`hid-shuttle`, `generic-midi`).
    pub driver: &'static str,
    /// The driver's default profile.
    pub default: bool,
}

/// The built-in profiles (their mappings come with the mapping engine, next step).
pub const PROFILES: &[BuiltinProfile] = &[
    BuiltinProfile { id: "cfa4.shuttlepro", name: "CFA4 ShuttlePRO", driver: "hid-shuttle", default: true },
    BuiltinProfile { id: "cfa4.shuttlepro.kdenlive", name: "Kdenlive-style ShuttlePRO", driver: "hid-shuttle", default: false },
    BuiltinProfile { id: "cfa4.generic-midi", name: "Generic MIDI", driver: "generic-midi", default: true },
];

fn profiles(_: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = PROFILES.iter().map(|p| json!({"id": p.id, "name": p.name, "driver": p.driver, "default": p.default, "builtin": true})).collect();
    Ok(json!({"profiles": list}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "controlSurfaces.profiles",
        label: "Control Surface Profiles",
        menu: &[],
        shortcut: None,
        params: "{}",
        enabled: always,
        run: profiles,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_defaults_and_clamp() {
        let mut p = ControlSurfacePrefs {
            enabled: true,
            devices: vec![
                DeviceSetting { device: "midi:a:b".into(), enabled: false, profile: "cfa4.generic-midi".into() },
                DeviceSetting { device: "midi:a:b".into(), enabled: true, profile: String::new() },
                DeviceSetting { device: String::new(), enabled: false, profile: String::new() },
                DeviceSetting { device: "hid:0b33:0030:usb-1.2".into(), enabled: true, profile: "nope".into() },
            ],
        };
        p.clamp();
        assert_eq!(p.devices.len(), 2);
        assert!(!p.device("midi:a:b").enabled, "the first entry wins");
        assert_eq!(p.device("hid:0b33:0030:usb-1.2").profile, "", "unknown profile goes back to the default");
        let fresh = p.device("midi:new:port");
        assert!(fresh.enabled);
        assert_eq!(fresh.device, "midi:new:port");
    }

    #[test]
    fn every_driver_has_one_default_profile() {
        for driver in ["hid-shuttle", "generic-midi"] {
            assert_eq!(PROFILES.iter().filter(|p| p.driver == driver && p.default).count(), 1, "{driver}");
        }
    }
}
