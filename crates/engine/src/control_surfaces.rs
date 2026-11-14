//! Settings ▸ Control Surfaces: MIDI controllers and jog/shuttle devices
//! (CFA4-CONTROL-SURFACE-001, `docs/adr/ADR-0012-control-surface.md`).
//!
//! The preferences (`controlSurfaces.*`) keep what the user chooses per device: use it or not,
//! which profile, and its assignments ([`Binding`]: a control runs a command, set up by hand or
//! with MIDI Learn). The device list itself and the reading come from the host process
//! (`cfa4-cli serve` ▸ `surface.devices`, `surface.monitor`, `surface.learn`), which owns the
//! device I/O (L5) and uses [`Triggers`] and [`ControlSurfacePrefs::action`] here to turn a
//! press into the command to run.

use std::collections::HashSet;

use cfa4_surface_model::{ControlId, SurfaceInput};
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
    /// What the controls of this device run (one per control).
    pub bindings: Vec<Binding>,
}

impl Default for DeviceSetting {
    fn default() -> Self {
        Self { device: String::new(), enabled: true, profile: String::new(), bindings: Vec::new() }
    }
}

/// Most assignments one device keeps.
pub const MAX_BINDINGS: usize = 512;

/// One assignment: pressing `control` runs `command` (a command id of the engine or of the GUI,
/// such as `playback.toggle`) with `params`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub control: ControlId,
    pub command: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

// `params` is JSON; equality of JSON values is total for what a preference file holds
impl Eq for Binding {}

/// What a press asks the GUI to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub command: String,
    pub params: Value,
}

/// Turns inputs into presses: a button press, or a controller crossing its middle upwards (a
/// MIDI button that sends CC 127 / 0, or a fader pushed past half way). Releases and the rest of
/// a continuous movement are not presses, so one press runs a command once.
#[derive(Clone, Debug, Default)]
pub struct Triggers {
    down: HashSet<(String, ControlId)>,
}

impl Triggers {
    /// The control `input` presses on `device`, if it is a press.
    pub fn press(&mut self, device: &str, input: &SurfaceInput) -> Option<ControlId> {
        let (control, high) = match input {
            SurfaceInput::Button { control, pressed } => (control, *pressed),
            SurfaceInput::Absolute { control, value, bits } => {
                let half = 1u32 << (u32::from(*bits).clamp(1, 16) - 1);
                (control, u32::from(*value) >= half)
            }
            _ => return None,
        };
        let key = (device.to_string(), control.clone());
        if high {
            self.down.insert(key).then(|| control.clone())
        } else {
            self.down.remove(&key);
            None
        }
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

    /// What pressing `control` on `device` runs: `None` when control surfaces are off, the device
    /// is not used, or the control has no assignment.
    pub fn action(&self, device: &str, control: &ControlId) -> Option<Action> {
        if !self.enabled {
            return None;
        }
        let d = self.devices.iter().find(|d| d.device == device)?;
        if !d.enabled {
            return None;
        }
        d.bindings.iter().find(|b| &b.control == control).map(|b| Action { command: b.command.clone(), params: b.params.clone() })
    }

    /// Whether any used device has an assignment (the host then keeps reading the devices).
    pub fn maps_anything(&self) -> bool {
        self.enabled && self.devices.iter().any(|d| d.enabled && !d.bindings.is_empty())
    }

    /// Drops entries without a device id and duplicates (the first one wins); per device,
    /// assignments without a command and later assignments of the same control.
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
            let mut controls: Vec<ControlId> = Vec::new();
            d.bindings.retain(|b| {
                if b.command.trim().is_empty() || controls.contains(&b.control) {
                    return false;
                }
                controls.push(b.control.clone());
                true
            });
            d.bindings.truncate(MAX_BINDINGS);
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
                DeviceSetting { device: "midi:a:b".into(), enabled: false, profile: "cfa4.generic-midi".into(), bindings: vec![] },
                DeviceSetting { device: "midi:a:b".into(), enabled: true, profile: String::new(), bindings: vec![] },
                DeviceSetting { device: String::new(), enabled: false, profile: String::new(), bindings: vec![] },
                DeviceSetting { device: "hid:0b33:0030:usb-1.2".into(), enabled: true, profile: "nope".into(), bindings: vec![] },
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

    fn cc(number: u16) -> ControlId {
        ControlId::Midi { channel: 0, kind: cfa4_surface_model::MidiKind::Cc, number }
    }

    fn bind(control: ControlId, command: &str) -> Binding {
        Binding { control, command: command.into(), params: Value::Null }
    }

    #[test]
    fn presses_fire_once() {
        let mut t = Triggers::default();
        let abs = |v| SurfaceInput::Absolute { control: cc(41), value: v, bits: 7 };
        assert_eq!(t.press("d", &abs(127)), Some(cc(41)), "a MIDI button sends 127");
        assert_eq!(t.press("d", &abs(127)), None, "held: no second press");
        assert_eq!(t.press("other", &abs(127)), Some(cc(41)), "per device");
        assert_eq!(t.press("d", &abs(0)), None, "release");
        assert_eq!(t.press("d", &abs(70)), Some(cc(41)), "a fader past half way");
        assert_eq!(t.press("d", &abs(90)), None, "moving on is not a new press");
        assert_eq!(t.press("d", &abs(63)), None);
        assert_eq!(t.press("d", &abs(64)), Some(cc(41)));
        let note = ControlId::Midi { channel: 1, kind: cfa4_surface_model::MidiKind::Note, number: 36 };
        let button = |pressed| SurfaceInput::Button { control: note.clone(), pressed };
        assert_eq!(t.press("d", &button(true)), Some(note.clone()));
        assert_eq!(t.press("d", &button(false)), None);
        assert_eq!(t.press("d", &button(true)), Some(note.clone()));
        assert_eq!(t.press("d", &SurfaceInput::PitchBend { channel: 0, value: 16000 }), None);
        let wide = SurfaceInput::Absolute { control: cc(7), value: 9000, bits: 14 };
        assert_eq!(t.press("d", &wide), Some(cc(7)), "14-bit: half is 8192");
    }

    #[test]
    fn actions_follow_the_settings() {
        let mut p = ControlSurfacePrefs {
            enabled: true,
            devices: vec![DeviceSetting {
                device: "midi:k:p".into(),
                enabled: true,
                profile: String::new(),
                bindings: vec![bind(cc(41), "playback.toggle"), bind(cc(41), "edit.undo"), bind(cc(42), " "), bind(cc(43), "edit.undo")],
            }],
        };
        p.clamp();
        assert_eq!(p.devices[0].bindings, vec![bind(cc(41), "playback.toggle"), bind(cc(43), "edit.undo")], "first per control, no empty command");
        assert!(p.maps_anything());
        assert_eq!(p.action("midi:k:p", &cc(41)).map(|a| a.command), Some("playback.toggle".into()));
        assert_eq!(p.action("midi:k:p", &cc(44)), None);
        assert_eq!(p.action("midi:other", &cc(41)), None);
        p.devices[0].enabled = false;
        assert_eq!(p.action("midi:k:p", &cc(41)), None, "a device not in use runs nothing");
        assert!(!p.maps_anything());
        p.devices[0].enabled = true;
        p.enabled = false;
        assert_eq!(p.action("midi:k:p", &cc(41)), None, "control surfaces off");
        assert!(!p.maps_anything());
    }

    #[test]
    fn bindings_are_json_the_gui_can_write() {
        let v = serde_json::json!({"device": "midi:k:p", "bindings": [{"control": {"midi": {"channel": 0, "kind": "cc", "number": 41}}, "command": "playback.toggle"}]});
        let d: DeviceSetting = serde_json::from_value(v).unwrap_or_default();
        assert!(d.enabled, "missing fields take their defaults");
        assert_eq!(d.bindings, vec![bind(cc(41), "playback.toggle")]);
        let back = serde_json::to_value(&d).unwrap_or_default();
        assert_eq!(back["bindings"][0]["control"]["midi"]["number"], 41);
        assert!(back["bindings"][0].get("params").is_none());
    }

    #[test]
    fn every_driver_has_one_default_profile() {
        for driver in ["hid-shuttle", "generic-midi"] {
            assert_eq!(PROFILES.iter().filter(|p| p.driver == driver && p.default).count(), 1, "{driver}");
        }
    }
}
