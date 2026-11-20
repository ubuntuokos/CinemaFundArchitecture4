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
///
/// Forward compatible (READY-V4.3): an assignment this version cannot read (a control of an
/// unknown protocol or input kind, written by a newer CFA4 or by hand) does not spoil the
/// settings. It is kept, disabled, in [`DeviceSetting::unsupported`], written back unchanged, and
/// reported by `surface.profile.warnings`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawDeviceSetting", into = "RawDeviceSetting")]
pub struct DeviceSetting {
    /// `hid:<vid>:<pid>:<phys>` or `midi:<client>:<port>` (as `surface.devices` names it).
    pub device: String,
    pub enabled: bool,
    /// A profile id from [`PROFILES`] (empty = the driver's default).
    pub profile: String,
    /// What the controls of this device run (one per control).
    pub bindings: Vec<Binding>,
    /// Assignments this version cannot read: never run, saved back as they were.
    pub unsupported: Vec<UnsupportedBinding>,
    /// Pickup distance for a 7-bit control (D4; default 10).
    pub pickup_threshold7: u16,
    /// Pickup distance for a 14-bit control (D4; default 1280).
    pub pickup_threshold14: u16,
    /// `mute` silences the shuttle ring; empty follows the Qt playback rule (N-01).
    pub shuttle_sound: String,
}

impl Default for DeviceSetting {
    fn default() -> Self {
        Self {
            device: String::new(),
            enabled: true,
            profile: String::new(),
            bindings: Vec::new(),
            unsupported: Vec::new(),
            pickup_threshold7: PICKUP_THRESHOLD7,
            pickup_threshold14: PICKUP_THRESHOLD14,
            shuttle_sound: String::new(),
        }
    }
}

impl DeviceSetting {
    /// One line per assignment that is loaded disabled, for `surface.profile.warnings`.
    pub fn warnings(&self) -> Vec<String> {
        self.unsupported.iter().map(|u| format!("assignment of {} is disabled: {}", u.control_kind(), u.reason)).collect()
    }
}

/// An assignment kept as written because this version cannot read it.
#[derive(Clone, Debug, PartialEq)]
pub struct UnsupportedBinding {
    /// The assignment exactly as the preferences hold it.
    pub raw: Value,
    /// Why it could not be read (the parser's message).
    pub reason: String,
}

// `raw` is JSON; equality of JSON values is total for what a preference file holds
impl Eq for UnsupportedBinding {}

impl UnsupportedBinding {
    /// What the assignment's control is, as far as it can be told: `a "tsl" control`, or
    /// `an unreadable control`.
    pub fn control_kind(&self) -> String {
        match self.raw.get("control") {
            Some(Value::Object(m)) if m.len() == 1 => m.keys().next().map_or_else(|| "an unreadable control".into(), |k| format!("a \"{k}\" control")),
            Some(Value::String(k)) => format!("a \"{k}\" control"),
            _ => "an unreadable control".into(),
        }
    }
}

/// The stored form of [`DeviceSetting`]: assignments as plain JSON, so one unreadable assignment
/// cannot fail the whole preferences document.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RawDeviceSetting {
    device: String,
    enabled: bool,
    profile: String,
    bindings: Vec<Value>,
    #[serde(default = "default_threshold7", skip_serializing_if = "is_default_threshold7")]
    pickup_threshold7: u16,
    #[serde(default = "default_threshold14", skip_serializing_if = "is_default_threshold14")]
    pickup_threshold14: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    shuttle_sound: String,
}

fn default_threshold7() -> u16 {
    PICKUP_THRESHOLD7
}

fn default_threshold14() -> u16 {
    PICKUP_THRESHOLD14
}

fn is_default_threshold7(v: &u16) -> bool {
    *v == PICKUP_THRESHOLD7
}

fn is_default_threshold14(v: &u16) -> bool {
    *v == PICKUP_THRESHOLD14
}

impl Default for RawDeviceSetting {
    fn default() -> Self {
        Self {
            device: String::new(),
            enabled: true,
            profile: String::new(),
            bindings: Vec::new(),
            pickup_threshold7: PICKUP_THRESHOLD7,
            pickup_threshold14: PICKUP_THRESHOLD14,
            shuttle_sound: String::new(),
        }
    }
}

impl From<RawDeviceSetting> for DeviceSetting {
    fn from(r: RawDeviceSetting) -> Self {
        let mut bindings = Vec::new();
        let mut unsupported = Vec::new();
        for raw in r.bindings {
            match serde_json::from_value::<Binding>(raw.clone()) {
                Ok(b) => bindings.push(b),
                Err(e) => unsupported.push(UnsupportedBinding { raw, reason: e.to_string() }),
            }
        }
        DeviceSetting {
            device: r.device,
            enabled: r.enabled,
            profile: r.profile,
            bindings,
            unsupported,
            pickup_threshold7: r.pickup_threshold7,
            pickup_threshold14: r.pickup_threshold14,
            shuttle_sound: r.shuttle_sound,
        }
    }
}

impl From<DeviceSetting> for RawDeviceSetting {
    fn from(d: DeviceSetting) -> Self {
        let bindings = d.bindings.iter().map(|b| serde_json::to_value(b).unwrap_or_default()).chain(d.unsupported.into_iter().map(|u| u.raw)).collect();
        RawDeviceSetting {
            device: d.device,
            enabled: d.enabled,
            profile: d.profile,
            bindings,
            pickup_threshold7: d.pickup_threshold7,
            pickup_threshold14: d.pickup_threshold14,
            shuttle_sound: d.shuttle_sound,
        }
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

fn default_learn_first_ms() -> u64 {
    7_000
}

fn default_learn_silence_ms() -> u64 {
    1_500
}

fn learn_first_is_default(v: &u64) -> bool {
    *v == default_learn_first_ms()
}

fn learn_silence_is_default(v: &u64) -> bool {
    *v == default_learn_silence_ms()
}

/// Settings ▸ Control Surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlSurfacePrefs {
    /// Use control surfaces at all.
    pub enabled: bool,
    /// Per-device choices, for devices the user changed something on.
    pub devices: Vec<DeviceSetting>,
    /// How long Learn waits for a first message (F0: 7 s, adjustable; D2).
    #[serde(default = "default_learn_first_ms", skip_serializing_if = "learn_first_is_default")]
    pub learn_first_ms: u64,
    /// Silence after the last Learn message before the type is offered (F0: 1.5 s; D2).
    #[serde(default = "default_learn_silence_ms", skip_serializing_if = "learn_silence_is_default")]
    pub learn_silence_ms: u64,
}

impl Default for ControlSurfacePrefs {
    fn default() -> Self {
        Self { enabled: true, devices: Vec::new(), learn_first_ms: default_learn_first_ms(), learn_silence_ms: default_learn_silence_ms() }
    }
}

/// Pickup distance for a 7-bit control (Ardour `_threshold`, D4).
pub const PICKUP_THRESHOLD7: u16 = 10;

/// Pickup distance for a 14-bit control (D4).
pub const PICKUP_THRESHOLD14: u16 = 1280;

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
                let mut params = json!({"speed": speed});
                if d.shuttle_sound == "mute"
                    && let Some(obj) = params.as_object_mut()
                {
                    obj.insert("sound".into(), json!("mute"));
                }
                vec![Action { command: "playback.shuttleSpeed".into(), params }]
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
            d.unsupported.truncate(MAX_BINDINGS - d.bindings.len());
            d.pickup_threshold7 = d.pickup_threshold7.min(127);
            d.pickup_threshold14 = d.pickup_threshold14.min(16383);
            if d.shuttle_sound != "mute" {
                d.shuttle_sound.clear();
            }
        }
        self.learn_first_ms = self.learn_first_ms.clamp(500, 60_000);
        self.learn_silence_ms = self.learn_silence_ms.clamp(100, 10_000);
        if self.learn_silence_ms >= self.learn_first_ms {
            self.learn_silence_ms = (self.learn_first_ms / 2).max(100);
        }
    }

    /// `surface.profile.warnings`: the devices with assignments loaded disabled, and why.
    pub fn warnings(&self) -> Vec<(String, Vec<String>)> {
        self.devices.iter().map(|d| (d.device.clone(), d.warnings())).filter(|(_, w)| !w.is_empty()).collect()
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

fn profile_warnings(s: &mut Session, _: &Value) -> Result<Value> {
    let devices: Vec<Value> =
        s.prefs.control_surfaces.warnings().into_iter().map(|(device, warnings)| json!({"device": device, "warnings": warnings})).collect();
    Ok(json!({"devices": devices}))
}

fn run_learn(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_learn(s, p)
}

fn run_learn_cancel(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_learn_cancel(s, p)
}

fn run_learn_confirm(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_learn_confirm(s, p)
}

fn run_inject(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_inject(s, p)
}

fn run_clock(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_clock(s, p)
}

fn run_enable(s: &mut Session, p: &Value) -> Result<Value> {
    let result = crate::surface_runtime::cmd_enable(s, p)?;
    let prefs = s.prefs.clone();
    s.set_prefs(prefs).map_err(|e| crate::EngineError::Other(format!("saving preferences: {e}")))?;
    Ok(result)
}

fn run_state(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_state(s, p)
}

fn run_audit(s: &mut Session, p: &Value) -> Result<Value> {
    crate::surface_runtime::cmd_audit(s, p)
}

fn run_reload(s: &mut Session, _p: &Value) -> Result<Value> {
    let mut prefs = s.prefs.control_surfaces.clone();
    let warnings = match s.prefs_path.as_ref().and_then(|path| path.parent()) {
        Some(data) => crate::surface_profile::load_into(&mut prefs, &crate::surface_profile::dir_for(data)),
        None => Vec::new(),
    };
    let devices = serde_json::to_value(&prefs.devices).unwrap_or(Value::Array(Vec::new()));
    Ok(json!({"warnings": warnings, "devices": devices}))
}

pub fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, params, run: fn(&mut Session, &Value) -> Result<Value>| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params,
        enabled: always,
        run,
        journal: false,
    };
    vec![
        spec("surface.learn", "Learn Control", r#"{"target?":{"command":str,"params?":object},"device?":str,"firstMs?":int,"silenceMs?":int}"#, run_learn),
        spec("surface.learnCancel", "Cancel Learn", "{}", run_learn_cancel),
        spec("surface.learnConfirm", "Confirm Learn", r#"{"accept":bool}"#, run_learn_confirm),
        spec("surface.inject", "Inject Control Input", r#"{"device":str,"input":object,"raw?":[int]}"#, run_inject),
        spec("surface.clock", "Control Surface Clock", r#"{"time":int,"playing":bool,"speed":number,"monitor":str}"#, run_clock),
        spec("surface.enable", "Enable Control Surface", r#"{"device":str,"enabled":bool}"#, run_enable),
        spec("surface.state", "Control Surface State", r#"{"device?":str}"#, run_state),
        spec("surface.audit", "Audit Control Surface", r#"{"device?":str}"#, run_audit),
        CommandSpec {
            id: "surface.profile.warnings",
            label: "Control Surface Profile Warnings",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: profile_warnings,
            journal: false,
        },
        CommandSpec {
            id: "controlSurfaces.profiles",
            label: "Control Surface Profiles",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: profiles,
            journal: false,
        },
        CommandSpec {
            id: "controlSurfaces.reloadProfiles",
            label: "Reload Control Surface Profiles",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: run_reload,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_defaults_and_clamp() {
        let mut p = ControlSurfacePrefs {
            enabled: true,
            devices: vec![
                DeviceSetting { device: "midi:a:b".into(), enabled: false, profile: "cfa4.generic-midi".into(), ..DeviceSetting::default() },
                DeviceSetting { device: "midi:a:b".into(), enabled: true, profile: String::new(), ..DeviceSetting::default() },
                DeviceSetting { device: String::new(), enabled: false, profile: String::new(), ..DeviceSetting::default() },
                DeviceSetting { device: "hid:0b33:0030:usb-1.2".into(), enabled: true, profile: "nope".into(), ..DeviceSetting::default() },
            ],
            ..ControlSurfacePrefs::default()
        };
        p.clamp();
        assert_eq!(p.devices.len(), 2);
        assert!(!p.device("midi:a:b").enabled, "the first entry wins");
        assert_eq!(p.device("hid:0b33:0030:usb-1.2").profile, "", "unknown profile goes back to the default");
        let fresh = p.device("midi:new:port");
        assert!(fresh.enabled);
        assert_eq!(fresh.device, "midi:new:port");
    }

    /// READY-V4.3: a profile with a control of an unknown protocol or input kind still loads;
    /// that assignment is disabled, kept as written and reported.
    #[test]
    fn unknown_controls_load_disabled_with_a_warning() {
        let doc = json!({
            "enabled": true,
            "devices": [{
                "device": "midi:k:p",
                "enabled": true,
                "profile": "cfa4.generic-midi",
                "bindings": [
                    {"control": {"midi": {"channel": 0, "kind": "cc", "number": 41}}, "command": "playback.toggle"},
                    {"control": {"tsl": {"screen": 1, "index": 4}}, "command": "live.studio.cut"},
                    {"control": {"midi": {"channel": 0, "kind": "polyAftertouch", "number": 3}}, "command": "markers.markIn"}
                ]
            }]
        });
        let p: ControlSurfacePrefs = serde_json::from_value(doc.clone()).expect("one unknown assignment does not fail the settings");
        let d = p.device("midi:k:p");
        assert_eq!(d.bindings, [bind(cc(41), "playback.toggle")]);
        assert_eq!(d.unsupported.len(), 2);
        assert_eq!(p.action("midi:k:p", &cc(41)).map(|a| a.command), Some("playback.toggle".into()));
        let w = p.warnings();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].0, "midi:k:p");
        assert!(w[0].1[0].starts_with("assignment of a \"tsl\" control is disabled: "), "{:?}", w[0].1);
        assert!(w[0].1[1].starts_with("assignment of a \"midi\" control is disabled: "), "{:?}", w[0].1);
        // written back unchanged, so a newer CFA4 finds its assignments again
        assert_eq!(serde_json::to_value(&p).ok(), Some(doc));
    }

    #[test]
    fn unknown_controls_do_not_reset_the_preferences() {
        let mut v = crate::autosave::Preferences::default().to_value();
        v["controlSurfaces"]["devices"] = json!([{
            "device": "midi:k:p",
            "bindings": [{"control": {"osc2": {"path": "/x"}}, "command": "playback.toggle"}]
        }]);
        v["trim"]["largeTrimOffset"] = json!(7);
        let p = crate::autosave::Preferences::from_value(v);
        assert_eq!(p.trim.large_trim_offset, 7, "the other preferences survive");
        assert_eq!(p.control_surfaces.device("midi:k:p").unsupported.len(), 1);
    }

    #[test]
    fn learn_times_can_be_set_when_the_file_omitted_them() {
        let mut p = crate::autosave::Preferences::default();
        assert!(p.to_value().pointer("/controlSurfaces/learnFirstMs").is_none());
        p.set("controlSurfaces.learnFirstMs", json!(8000)).expect("a default that was not written can still be changed");
        assert_eq!(p.control_surfaces.learn_first_ms, 8000);
        p.set("controlSurfaces.learnSilenceMs", json!(400)).expect("silence");
        assert_eq!(p.control_surfaces.learn_silence_ms, 400);
        assert!(p.set("no.such.group.leaf", json!(1)).is_err());
    }

    #[test]
    fn profile_warnings_command() {
        let mut s = Session::default();
        assert_eq!((profile_warnings)(&mut s, &Value::Null).ok(), Some(json!({"devices": []})));
        s.prefs.control_surfaces.devices.push(DeviceSetting {
            device: "midi:k:p".into(),
            unsupported: vec![UnsupportedBinding { raw: json!({"control": "tsl"}), reason: "unknown variant".into() }],
            ..Default::default()
        });
        let r = (profile_warnings)(&mut s, &Value::Null).ok();
        assert_eq!(r, Some(json!({"devices": [{"device": "midi:k:p", "warnings": ["assignment of a \"tsl\" control is disabled: unknown variant"]}]})));
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
                ..DeviceSetting::default()
            }],
            ..ControlSurfacePrefs::default()
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
        p.devices[0].shuttle_sound = "mute".into();
        let muted = p.motion(dev, &shuttle(2));
        assert_eq!(muted.first().and_then(|a| a.params.get("sound")).and_then(Value::as_str), Some("mute"));
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
            ..ControlSurfacePrefs::default()
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
