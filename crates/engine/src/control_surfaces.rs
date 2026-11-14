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

/// Shuttle speeds of the built-in `CFA4 ShuttlePRO` profile for positions 1–7 (Ardour-like;
/// F0 spec §4). Backwards is the same, negative.
pub const SHUTTLE_ARDOUR: [f64; 7] = [0.5, 0.75, 1.0, 1.5, 2.0, 5.0, 8.0];

/// Shuttle speeds of the `Kdenlive-style ShuttlePRO` profile (its 16× and 60× are capped at the
/// player's 8×).
pub const SHUTTLE_KDENLIVE: [f64; 7] = [1.0, 2.0, 4.0, 5.0, 8.0, 8.0, 8.0];

/// GUI targets that follow a control's value instead of running on a press: an Audio Track
/// Mixer strip's volume or pan (`params.strip`: `A1`, `A2`, … or `Mix`). The action carries the
/// control's position as `value` in 0..=1.
pub const VALUE_COMMANDS: &[&str] = &["mixer.stripVolume", "mixer.stripPan"];

/// Most frame steps one jog report runs (a fast spin is still bounded).
pub const MAX_JOG_STEPS: u32 = 32;

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

    /// What moving `control` sets, when it is assigned to a [`VALUE_COMMANDS`] target: the
    /// target's params plus `value` (the position in 0..=1). `None` for a press-type assignment,
    /// an input without a position, or a device not in use.
    pub fn value(&self, device: &str, control: &ControlId, input: &SurfaceInput) -> Option<Action> {
        let SurfaceInput::Absolute { value, bits, .. } = input else { return None };
        if !self.enabled {
            return None;
        }
        let d = self.devices.iter().find(|d| d.device == device && d.enabled)?;
        let b = d.bindings.iter().find(|b| &b.control == control && VALUE_COMMANDS.contains(&b.command.as_str()))?;
        let max = (1u32 << u32::from(*bits).clamp(1, 16)) - 1;
        let v = f64::from(*value).min(f64::from(max)) / f64::from(max);
        let mut params = if b.params.is_object() { b.params.clone() } else { json!({}) };
        if let Some(o) = params.as_object_mut() {
            o.insert("value".into(), json!(v));
        }
        Some(Action { command: b.command.clone(), params })
    }

    /// What a jog wheel or shuttle ring movement runs, by the device's profile: the shuttle
    /// plays at the profile's speed for its position (0 = stop), the jog steps one frame per
    /// click (at most [`MAX_JOG_STEPS`] per report). Empty when the device is not used.
    pub fn motion(&self, device: &str, input: &SurfaceInput) -> Vec<Action> {
        let d = self.device(device);
        if !self.enabled || !d.enabled {
            return Vec::new();
        }
        match input {
            SurfaceInput::Shuttle { position, .. } => {
                if *position == 0 {
                    return vec![Action { command: "playback.stop".into(), params: Value::Null }];
                }
                let table = if d.profile == "cfa4.shuttlepro.kdenlive" { &SHUTTLE_KDENLIVE } else { &SHUTTLE_ARDOUR };
                let i = usize::from(position.unsigned_abs().clamp(1, 7)) - 1;
                let speed = if *position < 0 { -table[i] } else { table[i] };
                vec![Action { command: "playback.shuttleSpeed".into(), params: json!({"speed": speed}) }]
            }
            SurfaceInput::Jog { delta, .. } => {
                let command = if *delta > 0 { "playhead.stepForward" } else { "playhead.stepBack" };
                (0..delta.unsigned_abs().min(MAX_JOG_STEPS)).map(|_| Action { command: command.into(), params: Value::Null }).collect()
            }
            _ => Vec::new(),
        }
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
    fn jog_and_shuttle_follow_the_profile() {
        use cfa4_surface_model::ControlId;
        let dev = "hid:0b33:0030:usb-1";
        let shuttle = |position| SurfaceInput::Shuttle { control: ControlId::hid_named("shuttle"), position, range: 7 };
        let jog = |delta| SurfaceInput::Jog { control: ControlId::hid_named("jog"), delta };
        let mut p = ControlSurfacePrefs::default();
        let speed = |p: &ControlSurfacePrefs, pos| p.motion(dev, &shuttle(pos)).first().map(|a| (a.command.clone(), a.params["speed"].as_f64()));
        assert_eq!(speed(&p, 1), Some(("playback.shuttleSpeed".into(), Some(0.5))), "a device without settings uses the default profile");
        assert_eq!(speed(&p, 7), Some(("playback.shuttleSpeed".into(), Some(8.0))));
        assert_eq!(speed(&p, -3), Some(("playback.shuttleSpeed".into(), Some(-1.0))));
        assert_eq!(speed(&p, 0), Some(("playback.stop".into(), None)));
        p.devices.push(DeviceSetting { device: dev.into(), profile: "cfa4.shuttlepro.kdenlive".into(), ..Default::default() });
        assert_eq!(speed(&p, 2), Some(("playback.shuttleSpeed".into(), Some(2.0))));
        let steps = p.motion(dev, &jog(-3));
        assert_eq!(steps.len(), 3);
        assert!(steps.iter().all(|a| a.command == "playhead.stepBack"));
        assert_eq!(p.motion(dev, &jog(200)).len(), MAX_JOG_STEPS as usize);
        assert!(p.motion(dev, &SurfaceInput::Raw).is_empty());
        p.devices[0].enabled = false;
        assert!(p.motion(dev, &jog(1)).is_empty(), "a device not in use");
        p.devices[0].enabled = true;
        p.enabled = false;
        assert!(p.motion(dev, &shuttle(3)).is_empty(), "control surfaces off");
    }

    #[test]
    fn faders_follow_their_value_target() {
        let dev = "midi:k:p";
        let fader = |v| SurfaceInput::Absolute { control: cc(0), value: v, bits: 7 };
        let mut p = ControlSurfacePrefs {
            enabled: true,
            devices: vec![DeviceSetting {
                device: dev.into(),
                bindings: vec![
                    Binding { control: cc(0), command: "mixer.stripVolume".into(), params: json!({"strip": "A1"}) },
                    bind(cc(41), "playback.toggle"),
                ],
                ..Default::default()
            }],
        };
        let a = p.value(dev, &cc(0), &fader(127));
        assert_eq!(a, Some(Action { command: "mixer.stripVolume".into(), params: json!({"strip": "A1", "value": 1.0}) }));
        assert_eq!(p.value(dev, &cc(0), &fader(0)).map(|a| a.params["value"].clone()), Some(json!(0.0)));
        let wide = SurfaceInput::Absolute { control: cc(0), value: 8192, bits: 14 };
        let half = p.value(dev, &cc(0), &wide).and_then(|a| a.params["value"].as_f64()).unwrap_or_default();
        assert!((half - 0.5).abs() < 0.001, "{half}");
        assert_eq!(p.value(dev, &cc(41), &fader(127)), None, "a press-type assignment");
        assert_eq!(p.value(dev, &cc(0), &SurfaceInput::Button { control: cc(0), pressed: true }), None);
        p.enabled = false;
        assert_eq!(p.value(dev, &cc(0), &fader(1)), None);
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
