//! Active control-surface profiles on disk (F0 spec §8, decision D3).
//!
//! Settings OK writes the draft into preferences and, in the same step, a schema-checked file
//! `active-<device>.json` under `$XDG_DATA_HOME/cfa4/Control Surfaces/` via
//! [`cfa4_format::atomic_write`]. On load the file wins when it is valid. A file that does not
//! parse keeps the preferences and the built-in profile, with a warning. A higher `version` is
//! left untouched.

use std::fs;
use std::path::{Path, PathBuf};

use cfa4_surface_model::{ControlId, DeviceId};
use serde_json::{Value, json};

use crate::control_surfaces::{
    Binding, ControlSurfacePrefs, DeviceSetting, PROFILES, StoredGesture, VALUE_COMMANDS, gesture_command_allowed, gesture_context_key, gesture_id_ok,
};

const FORMAT: &str = "cfa4-surface-profile";
const VERSION: i64 = 1;
/// Pointer-gesture profiles. A version-2 file for any other driver stays read-only.
const POINTER_VERSION: i64 = 2;

/// Where active profiles live, next to the preferences file.
pub fn dir_for(data_dir: &Path) -> PathBuf {
    data_dir.join("Control Surfaces")
}

pub(crate) fn active_path(dir: &Path, device: &str) -> PathBuf {
    dir.join(format!("active-{}.json", DeviceId(device.to_string()).file_stem()))
}

fn profile_id(device: &str) -> String {
    let mut id: String =
        DeviceId(device.to_string()).file_stem().to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).collect();
    id.truncate(60);
    let id = format!("u.{id}");
    if id.len() > 64 {
        id[..64].to_string()
    } else if id.chars().nth(2).is_some_and(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        id
    } else {
        "u.device".into()
    }
}

fn driver_of(device: &str, profile: &str) -> &'static str {
    PROFILES.iter().find(|p| p.id == profile).map(|p| p.driver).unwrap_or(if device.starts_with("hid:") { "hid-shuttle" } else { "generic-midi" })
}

fn command_ok(command: &str) -> bool {
    let mut parts = command.split('.');
    let Some(first) = parts.next() else { return false };
    let Some(second) = parts.next() else { return false };
    !first.is_empty()
        && first.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && first.chars().all(|c| c.is_ascii_alphanumeric())
        && !second.is_empty()
        && parts.chain(std::iter::once(second)).all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn mode_of(command: &str) -> &'static str {
    if VALUE_COMMANDS.contains(&command) { "absolute" } else { "trigger" }
}

fn is_pointer_device(device: &str) -> bool {
    device == cfa4_surface_model::gesture::POINTER_SEAT || device.starts_with("pointer:")
}

fn binding_json(b: &Binding) -> Option<Value> {
    if matches!(b.control, ControlId::Gesture { .. }) || !command_ok(&b.command) {
        return None;
    }
    let mut v = json!({
        "control": b.control,
        "mode": mode_of(&b.command),
        "command": b.command,
    });
    if b.params.is_object()
        && let Some(obj) = v.as_object_mut()
    {
        obj.insert("params".into(), b.params.clone());
    }
    Some(v)
}

fn pointer_binding_json(b: &Binding) -> Option<Value> {
    let ControlId::Gesture { id } = &b.control else { return None };
    if !gesture_id_ok(id) || !gesture_command_allowed(&b.command) || !command_ok(&b.command) {
        return None;
    }
    let mut v = json!({
        "control": b.control,
        "mode": "trigger",
        "command": b.command,
    });
    if let Some(obj) = v.as_object_mut() {
        if b.params.is_object() {
            obj.insert("params".into(), b.params.clone());
        }
        if gesture_context_key(&b.context) != "global" {
            obj.insert("context".into(), json!(b.context.trim()));
        }
    }
    Some(v)
}

fn pointer_template_json(template: &StoredGesture) -> Option<Value> {
    if !gesture_id_ok(&template.id) || template.points.is_empty() || template.points.len() > cfa4_surface_model::gesture::MAX_TEMPLATE_POINTS {
        return None;
    }
    Some(json!({"id": template.id, "spatial": template.spatial, "points": template.points}))
}

fn pointer_document(device: &DeviceSetting) -> Value {
    let controls: Vec<Value> = device.bindings.iter().filter_map(pointer_binding_json).collect();
    let gestures: Vec<Value> = device.gestures.iter().filter_map(pointer_template_json).collect();
    let mut doc = json!({
        "format": FORMAT,
        "version": POINTER_VERSION,
        "id": profile_id(&device.device),
        "name": "Pointer gestures",
        "driver": "pointer",
        "match": {"midiPortName": device.device},
        "modified": true,
        "basedOn": if device.profile.is_empty() { Value::Null } else { json!(device.profile) },
        "pickup": {"enabled": true, "threshold7": device.pickup_threshold7, "threshold14": device.pickup_threshold14},
        "shuttle": {"sound": if device.shuttle_sound == "mute" { "mute" } else { "follow-playback" }},
        "controls": controls,
        "gestures": gestures,
    });
    if device.mirror_ergonomic
        && let Some(obj) = doc.as_object_mut()
    {
        obj.insert("mirrorErgonomic".into(), json!(true));
    }
    doc
}

/// The document Settings OK writes for one device. `None` when nothing valid can be written.
pub fn document(device: &DeviceSetting) -> Value {
    if is_pointer_device(&device.device) {
        return pointer_document(device);
    }
    let controls: Vec<Value> = device.bindings.iter().filter_map(binding_json).collect();
    json!({
        "format": FORMAT,
        "version": VERSION,
        "id": profile_id(&device.device),
        "name": "Control surface",
        "driver": driver_of(&device.device, &device.profile),
        "match": {"midiPortName": device.device},
        "modified": true,
        "basedOn": if device.profile.is_empty() { Value::Null } else { json!(device.profile) },
        "pickup": {"enabled": true, "threshold7": device.pickup_threshold7, "threshold14": device.pickup_threshold14},
        "shuttle": {"sound": if device.shuttle_sound == "mute" { "mute" } else { "follow-playback" }},
        "controls": controls,
    })
}

/// Why a document cannot be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileFault {
    /// `version` is newer than this build. The file must not be overwritten.
    Readonly(i64),
    /// The file does not match the version-1 schema.
    Invalid(String),
}

/// Check a document. `Ok(device id)` when this build may apply it.
pub fn validate(v: &Value) -> Result<String, ProfileFault> {
    let obj = v.as_object().ok_or_else(|| ProfileFault::Invalid("profile is not an object".into()))?;
    if obj.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ProfileFault::Invalid("format is not cfa4-surface-profile".into()));
    }
    let version = obj.get("version").and_then(Value::as_i64).ok_or_else(|| ProfileFault::Invalid("version is missing".into()))?;
    if version > POINTER_VERSION {
        return Err(ProfileFault::Readonly(version));
    }
    if version == POINTER_VERSION {
        return validate_pointer(obj);
    }
    if version > VERSION {
        return Err(ProfileFault::Readonly(version));
    }
    if version < 1 {
        return Err(ProfileFault::Invalid("version is below 1".into()));
    }
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
    if !id_ok(id) {
        return Err(ProfileFault::Invalid("id is not a profile id".into()));
    }
    let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() || name.len() > 128 {
        return Err(ProfileFault::Invalid("name is missing".into()));
    }
    let driver = obj.get("driver").and_then(Value::as_str).unwrap_or("");
    if !matches!(driver, "hid-shuttle" | "generic-midi" | "osc") {
        return Err(ProfileFault::Invalid("driver is unknown".into()));
    }
    let Some(match_) = obj.get("match").and_then(Value::as_object) else {
        return Err(ProfileFault::Invalid("match is missing".into()));
    };
    if match_.is_empty() {
        return Err(ProfileFault::Invalid("match is empty".into()));
    }
    let device = match_.get("midiPortName").and_then(Value::as_str).unwrap_or("").to_string();
    if device.is_empty() {
        return Err(ProfileFault::Invalid("match.midiPortName does not name the device".into()));
    }
    let Some(controls) = obj.get("controls").and_then(Value::as_array) else {
        return Err(ProfileFault::Invalid("controls is missing".into()));
    };
    if controls.len() > crate::control_surfaces::MAX_BINDINGS {
        return Err(ProfileFault::Invalid("controls is too long".into()));
    }
    for (i, c) in controls.iter().enumerate() {
        check_control(c).map_err(|e| ProfileFault::Invalid(format!("controls[{i}]: {e}")))?;
    }
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "format"
                | "version"
                | "id"
                | "name"
                | "builtin"
                | "modified"
                | "basedOn"
                | "driver"
                | "match"
                | "device"
                | "banks"
                | "pickup"
                | "feedback"
                | "shuttle"
                | "contexts"
                | "controls"
        ) {
            return Err(ProfileFault::Invalid(format!("unknown field {key}")));
        }
    }
    Ok(device)
}

fn id_ok(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else { return false };
    first.is_ascii_alphanumeric() && id.len() <= 64 && chars.all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn check_control(v: &Value) -> Result<(), String> {
    let obj = v.as_object().ok_or_else(|| "not an object".to_string())?;
    let mode = obj.get("mode").and_then(Value::as_str).unwrap_or("");
    if !matches!(
        mode,
        "trigger" | "toggle" | "momentary" | "absolute" | "relative" | "encoder-left" | "encoder-right" | "encoder-2" | "encoder-binary" | "jog" | "shuttle"
    ) {
        return Err("mode is unknown".into());
    }
    let command = obj.get("command").and_then(Value::as_str).unwrap_or("");
    if !command_ok(command) {
        return Err("command is not a command id".into());
    }
    let control = obj.get("control").ok_or_else(|| "control is missing".to_string())?;
    let keys = control.as_object().map(|m| m.len()).unwrap_or(0);
    if keys != 1 {
        return Err("control must have one protocol".into());
    }
    // the same parser as preferences: an unknown kind fails the control, not the whole file
    let parsed = serde_json::from_value::<ControlId>(control.clone()).map_err(|e| e.to_string())?;
    if matches!(parsed, ControlId::Gesture { .. }) {
        return Err("a gesture control needs profile version 2".into());
    }
    Ok(())
}

fn validate_pointer(obj: &serde_json::Map<String, Value>) -> Result<String, ProfileFault> {
    let driver = obj.get("driver").and_then(Value::as_str).unwrap_or("");
    if driver != "pointer" {
        return Err(ProfileFault::Readonly(POINTER_VERSION));
    }
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
    if !id_ok(id) {
        return Err(ProfileFault::Invalid("id is not a profile id".into()));
    }
    let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() || name.len() > 128 {
        return Err(ProfileFault::Invalid("name is missing".into()));
    }
    let Some(match_) = obj.get("match").and_then(Value::as_object) else {
        return Err(ProfileFault::Invalid("match is missing".into()));
    };
    if match_.is_empty() {
        return Err(ProfileFault::Invalid("match is empty".into()));
    }
    let device = match_.get("midiPortName").and_then(Value::as_str).unwrap_or("").to_string();
    if !is_pointer_device(&device) {
        return Err(ProfileFault::Invalid("match.midiPortName does not name the pointer".into()));
    }
    let Some(controls) = obj.get("controls").and_then(Value::as_array) else {
        return Err(ProfileFault::Invalid("controls is missing".into()));
    };
    if controls.len() > crate::control_surfaces::MAX_BINDINGS {
        return Err(ProfileFault::Invalid("controls is too long".into()));
    }
    for (i, c) in controls.iter().enumerate() {
        check_gesture_control(c).map_err(|e| ProfileFault::Invalid(format!("controls[{i}]: {e}")))?;
    }
    let gestures = obj.get("gestures").and_then(Value::as_array);
    let gestures = gestures.ok_or_else(|| ProfileFault::Invalid("gestures is missing".into()))?;
    if gestures.len() > crate::control_surfaces::MAX_GESTURE_TEMPLATES {
        return Err(ProfileFault::Invalid("gestures is too long".into()));
    }
    let mut seen = Vec::new();
    for (i, template) in gestures.iter().enumerate() {
        let id = check_template(template).map_err(|e| ProfileFault::Invalid(format!("gestures[{i}]: {e}")))?;
        if seen.iter().any(|known: &String| known == &id) {
            return Err(ProfileFault::Invalid(format!("gestures[{i}]: duplicate id")));
        }
        seen.push(id);
    }
    if let Some(mirror) = obj.get("mirrorErgonomic")
        && !mirror.is_boolean()
    {
        return Err(ProfileFault::Invalid("mirrorErgonomic is not a boolean".into()));
    }
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "format"
                | "version"
                | "id"
                | "name"
                | "builtin"
                | "modified"
                | "basedOn"
                | "driver"
                | "match"
                | "device"
                | "banks"
                | "pickup"
                | "feedback"
                | "shuttle"
                | "contexts"
                | "controls"
                | "gestures"
                | "mirrorErgonomic"
        ) {
            return Err(ProfileFault::Invalid(format!("unknown field {key}")));
        }
    }
    Ok(device)
}

fn check_gesture_control(v: &Value) -> Result<(), String> {
    let obj = v.as_object().ok_or_else(|| "not an object".to_string())?;
    let mode = obj.get("mode").and_then(Value::as_str).unwrap_or("");
    if mode != "trigger" {
        return Err("gesture mode is trigger".into());
    }
    let command = obj.get("command").and_then(Value::as_str).unwrap_or("");
    if !command_ok(command) || !gesture_command_allowed(command) {
        return Err("command is not playback, playhead or window".into());
    }
    let control = obj.get("control").ok_or_else(|| "control is missing".to_string())?;
    let parsed = serde_json::from_value::<ControlId>(control.clone()).map_err(|e| e.to_string())?;
    let ControlId::Gesture { id } = parsed else {
        return Err("a pointer profile assigns gestures".into());
    };
    if !gesture_id_ok(&id) {
        return Err("gesture id is not usable".into());
    }
    if let Some(context) = obj.get("context") {
        let text = context.as_str().ok_or_else(|| "context is not text".to_string())?;
        if !gesture_id_ok(text) && gesture_context_key(text) != "global" {
            return Err("context is not usable".into());
        }
    }
    Ok(())
}

fn check_template(v: &Value) -> Result<String, String> {
    let obj = v.as_object().ok_or_else(|| "not an object".to_string())?;
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
    if !gesture_id_ok(id) {
        return Err("id is not usable".into());
    }
    if !obj.get("spatial").and_then(Value::as_bool).is_some() {
        return Err("spatial is missing".into());
    }
    let points = obj.get("points").and_then(Value::as_array).ok_or_else(|| "points is missing".to_string())?;
    if points.is_empty() || points.len() > cfa4_surface_model::gesture::MAX_TEMPLATE_POINTS {
        return Err("points is empty or too long".into());
    }
    for point in points {
        let pair = point.as_array().ok_or_else(|| "a point is not a pair".to_string())?;
        if pair.len() != 2 {
            return Err("a point is not a pair".into());
        }
        for coord in pair {
            let Some(n) = coord.as_i64() else { return Err("a point is not an integer".into()) };
            if i32::try_from(n).is_err() {
                return Err("a point is out of range".into());
            }
        }
    }
    Ok(id.to_string())
}

fn apply_file(d: &mut DeviceSetting, value: &Value) {
    if let Some(pickup) = value.get("pickup").and_then(Value::as_object) {
        if let Some(t) = pickup.get("threshold7").and_then(Value::as_u64) {
            d.pickup_threshold7 = u16::try_from(t).unwrap_or(crate::control_surfaces::PICKUP_THRESHOLD7).min(127);
        }
        if let Some(t) = pickup.get("threshold14").and_then(Value::as_u64) {
            d.pickup_threshold14 = u16::try_from(t).unwrap_or(crate::control_surfaces::PICKUP_THRESHOLD14).min(16383);
        }
    }
    let sound = value.get("shuttle").and_then(|s| s.get("sound")).and_then(Value::as_str).unwrap_or("");
    d.shuttle_sound = if sound == "mute" { "mute".into() } else { String::new() };
}

fn gestures_of(v: &Value) -> Vec<StoredGesture> {
    v.get("gestures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|template| {
            let id = template.get("id").and_then(Value::as_str)?.to_string();
            let spatial = template.get("spatial").and_then(Value::as_bool).unwrap_or(false);
            let points = template.get("points").and_then(Value::as_array)?;
            let mut parsed = Vec::new();
            for point in points {
                let pair = point.as_array()?;
                let x = pair.first().and_then(Value::as_i64)?;
                let y = pair.get(1).and_then(Value::as_i64)?;
                parsed.push((i32::try_from(x).ok()?, i32::try_from(y).ok()?));
            }
            Some(StoredGesture { id, points: parsed, spatial })
        })
        .collect()
}

fn bindings_of(v: &Value) -> Vec<Binding> {
    v.get("controls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let control = serde_json::from_value::<ControlId>(c.get("control")?.clone()).ok()?;
            let command = c.get("command").and_then(Value::as_str)?.to_string();
            let params = c.get("params").cloned().unwrap_or(Value::Null);
            let context = c.get("context").and_then(Value::as_str).unwrap_or("").to_string();
            Some(Binding { control, command, params, context })
        })
        .collect()
}

/// Read every `active-*.json`. A valid file replaces that device's bindings in `prefs`.
/// Returns warnings (corrupt files, read-only versions). Does not delete or rewrite files.
pub fn load_into(prefs: &mut ControlSurfacePrefs, dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return warnings,
        Err(e) => {
            warnings.push(format!("control surface profiles: cannot read {}: {e}", dir.display()));
            return warnings;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("active-") || !name.ends_with(".json") {
            continue;
        }
        let path = entry.path();
        let text = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                warnings.push(format!("{}: {e}; keeping preferences", path.display()));
                continue;
            }
        };
        let value: Value = match serde_json::from_slice(&text) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!("{}: {e}; keeping preferences and the built-in profile", path.display()));
                continue;
            }
        };
        match validate(&value) {
            Ok(device) => {
                let bindings = bindings_of(&value);
                let profile = value.get("basedOn").and_then(Value::as_str).unwrap_or("").to_string();
                if !prefs.devices.iter().any(|d| d.device == device) {
                    prefs.devices.push(DeviceSetting { device: device.clone(), ..DeviceSetting::default() });
                }
                if let Some(d) = prefs.devices.iter_mut().find(|d| d.device == device) {
                    d.bindings = bindings;
                    d.gestures = gestures_of(&value);
                    d.mirror_ergonomic = value.get("mirrorErgonomic").and_then(Value::as_bool).unwrap_or(false);
                    if !profile.is_empty() {
                        d.profile = profile;
                    }
                    apply_file(d, &value);
                }
            }
            Err(ProfileFault::Readonly(v)) => {
                warnings.push(format!("{}: version {v} is read-only; not applied and not overwritten", path.display()));
            }
            Err(ProfileFault::Invalid(e)) => {
                warnings.push(format!("{}: {e}; keeping preferences and the built-in profile", path.display()));
            }
        }
    }
    prefs.clamp();
    warnings
}

/// Write one active file per device. A file whose `version` is newer is left as it is.
pub fn save_all(prefs: &ControlSurfacePrefs, dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(e) = fs::create_dir_all(dir) {
        warnings.push(format!("control surface profiles: cannot create {}: {e}", dir.display()));
        return warnings;
    }
    for device in &prefs.devices {
        if device.device.is_empty() {
            continue;
        }
        let path = active_path(dir, &device.device);
        if let Ok(bytes) = fs::read(&path)
            && let Ok(existing) = serde_json::from_slice::<Value>(&bytes)
            && let Err(ProfileFault::Readonly(v)) = validate(&existing)
        {
            warnings.push(format!("{}: version {v} is read-only; left unchanged", path.display()));
            continue;
        }
        let doc = document(device);
        if let Err(e) = validate(&doc) {
            warnings.push(format!("{}: not written ({e:?})", path.display()));
            continue;
        }
        let body = match serde_json::to_vec_pretty(&doc) {
            Ok(b) => b,
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        if let Err(e) = cfa4_format::atomic_write(&path, &body) {
            warnings.push(format!("{}: {e}", path.display()));
        }
    }
    warnings
}

/// Hostile bytes and broken objects do not panic.
pub fn load_bytes(bytes: &[u8]) -> Result<String, ProfileFault> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| ProfileFault::Invalid(e.to_string()))?;
    validate(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_surface_model::MidiKind;
    use serde_json::json;

    fn cc() -> ControlId {
        ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 41 }
    }

    #[test]
    fn a_valid_file_wins_and_a_broken_file_keeps_preferences() {
        let dir = std::env::temp_dir().join(format!("cfa4-surface-profile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let mut prefs = ControlSurfacePrefs {
            enabled: true,
            devices: vec![DeviceSetting {
                device: "midi:k:p".into(),
                bindings: vec![Binding { control: cc(), command: "edit.undo".into(), params: Value::Null, context: String::new() }],
                ..DeviceSetting::default()
            }],
            ..ControlSurfacePrefs::default()
        };
        assert!(save_all(&prefs, &dir).is_empty(), "the document we write validates");
        prefs.devices[0].bindings[0].command = "edit.redo".into();
        let warnings = load_into(&mut prefs, &dir);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(prefs.devices[0].bindings[0].command, "edit.undo", "the file is primary");

        let path = active_path(&dir, "midi:k:p");
        fs::write(&path, b"{").expect("corrupt");
        prefs.devices[0].bindings[0].command = "edit.redo".into();
        let warnings = load_into(&mut prefs, &dir);
        assert!(warnings.iter().any(|w| w.contains("keeping preferences")), "{warnings:?}");
        assert_eq!(prefs.devices[0].bindings[0].command, "edit.redo");

        let newer = json!({"format": FORMAT, "version": 9, "id": "u.x", "name": "n", "driver": "generic-midi", "match": {"any": true}, "controls": []});
        fs::write(&path, serde_json::to_vec(&newer).unwrap_or_default()).expect("version");
        let before = fs::read(&path).unwrap_or_default();
        let warnings = save_all(&prefs, &dir);
        assert!(warnings.iter().any(|w| w.contains("read-only")), "{warnings:?}");
        assert_eq!(fs::read(&path).unwrap_or_default(), before, "a newer version is not overwritten");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pickup_and_shuttle_sound_round_trip() {
        let dir = std::env::temp_dir().join(format!("cfa4-surface-profile-pickup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let mut prefs = ControlSurfacePrefs {
            devices: vec![DeviceSetting {
                device: "midi:k:p".into(),
                pickup_threshold7: 12,
                pickup_threshold14: 200,
                shuttle_sound: "mute".into(),
                bindings: vec![Binding { control: cc(), command: "edit.undo".into(), params: Value::Null, context: String::new() }],
                ..DeviceSetting::default()
            }],
            ..ControlSurfacePrefs::default()
        };
        assert!(save_all(&prefs, &dir).is_empty(), "the document we write validates");
        prefs.devices[0].pickup_threshold7 = 1;
        prefs.devices[0].shuttle_sound.clear();
        let warnings = load_into(&mut prefs, &dir);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(prefs.devices[0].pickup_threshold7, 12);
        assert_eq!(prefs.devices[0].pickup_threshold14, 200);
        assert_eq!(prefs.devices[0].shuttle_sound, "mute");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_profiles_do_not_panic() {
        for bytes in [b"".as_slice(), b"{", b"null", b"[]", b"{\"version\":1}", &[0xff, 0xfe], b"{\"format\":\"cfa4-surface-profile\",\"version\":99}"] {
            let _ = load_bytes(bytes);
        }
        let huge = json!({"format": FORMAT, "version": 1, "id": "u.x", "name": "n", "driver": "generic-midi", "match": {"midiPortName": "midi:k:p"}, "controls": vec![json!({"control": {"midi": {"channel": 0, "kind": "cc", "number": 1}}, "mode": "trigger", "command": "edit.undo"}); 600]});
        assert!(validate(&huge).is_err());
    }

    #[test]
    fn a_pointer_profile_is_version_2_and_a_midi_file_stays_version_1() {
        let dir = std::env::temp_dir().join(format!("cfa4-surface-profile-pointer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let gesture = Binding {
            control: ControlId::Gesture { id: "direction.right".into() },
            command: "playback.toggle".into(),
            params: Value::Null,
            context: "timeline".into(),
        };
        let template = StoredGesture { id: "hook".into(), points: vec![(0, 0), (0, 100), (100, 100)], spatial: false };
        let mut prefs = ControlSurfacePrefs {
            devices: vec![
                DeviceSetting {
                    device: "midi:k:p".into(),
                    bindings: vec![Binding { control: cc(), command: "edit.undo".into(), params: Value::Null, context: String::new() }],
                    ..DeviceSetting::default()
                },
                DeviceSetting {
                    device: "pointer:seat".into(),
                    bindings: vec![gesture],
                    gestures: vec![template.clone()],
                    mirror_ergonomic: true,
                    ..DeviceSetting::default()
                },
            ],
            ..ControlSurfacePrefs::default()
        };
        assert!(save_all(&prefs, &dir).is_empty(), "both documents validate");
        let midi = serde_json::from_slice::<Value>(&fs::read(active_path(&dir, "midi:k:p")).unwrap_or_default()).unwrap_or_default();
        assert_eq!(midi["version"], 1);
        assert_eq!(midi["driver"], "generic-midi");
        assert!(midi.get("gestures").is_none(), "a MIDI profile has no gesture templates");
        assert!(midi["controls"][0].get("context").is_none());
        let pointer = serde_json::from_slice::<Value>(&fs::read(active_path(&dir, "pointer:seat")).unwrap_or_default()).unwrap_or_default();
        assert_eq!(pointer["version"], 2);
        assert_eq!(pointer["driver"], "pointer");
        assert_eq!(pointer["mirrorErgonomic"], true);
        assert_eq!(pointer["gestures"][0]["id"], "hook");
        assert_eq!(pointer["controls"][0]["context"], "timeline");

        prefs.devices.clear();
        let warnings = load_into(&mut prefs, &dir);
        assert!(warnings.is_empty(), "{warnings:?}");
        let loaded = prefs.devices.iter().find(|d| d.device == "pointer:seat").expect("pointer");
        assert_eq!(loaded.gestures, vec![template]);
        assert!(loaded.mirror_ergonomic);
        assert_eq!(loaded.bindings[0].command, "playback.toggle");
        assert_eq!(loaded.bindings[0].context, "timeline");
        assert_eq!(prefs.devices.iter().find(|d| d.device == "midi:k:p").map(|d| d.bindings[0].command.as_str()), Some("edit.undo"));

        let foreign =
            json!({"format": FORMAT, "version": 2, "id": "u.x", "name": "n", "driver": "generic-midi", "match": {"midiPortName": "midi:k:p"}, "controls": []});
        let path = active_path(&dir, "midi:k:p");
        fs::write(&path, serde_json::to_vec(&foreign).unwrap_or_default()).expect("foreign v2");
        let before = fs::read(&path).unwrap_or_default();
        let warnings = save_all(&prefs, &dir);
        assert!(warnings.iter().any(|w| w.contains("read-only")), "{warnings:?}");
        assert_eq!(fs::read(&path).unwrap_or_default(), before, "a version-2 MIDI profile is not overwritten");
        let _ = fs::remove_dir_all(&dir);
    }
}
