//! Qt6 gesture editor (CFA4-CONTROL-SURFACE-GESTURE-001, MG3).
//!
//! The recogniser stays in [`crate::gesture_runtime`]. These commands feed it samples, keep the
//! pointer profile the MG2 path already stores, and put a finished gesture on `surface.inject`.
//! A binding there is one `surfaceAction`. Nothing here opens `/dev/input`, and nothing here
//! changes MIDI or HID lookup or [`crate::control_surfaces::Triggers`].
//!
//! HUD, thresholds, handedness, input access and combinations are not fields of the version-2
//! profile (`additionalProperties: false`). They live in `gesture-ui.json` beside the profiles.
//! A stored combination or hold dispatches one `surfaceAction` from here. MIDI and HID lookup
//! and [`crate::control_surfaces::Triggers`] stay on the press edge.
//!
//! Timeline and Media Browser are contexts (plan §8.1), not a second profile schema. The
//! Timeline context has a built-in spatial map. A narrower context that the user configured
//! does not fall back to a wider one.

use std::collections::HashMap;
use std::path::PathBuf;

use cfa4_surface_model::gesture::{self, Direction, MAX_TEMPLATE_POINTS, Recognize, Thresholds};
use cfa4_surface_model::{ControlId, SurfaceInput};
use serde_json::{Value, json};
use web_time::Instant;

use crate::commands::{CommandSpec, always};
use crate::control_surfaces::{Binding, ControlSurfacePrefs, DeviceSetting, StoredGesture, gesture_command_allowed, gesture_context_key, gesture_id_ok};
use crate::gesture_runtime::{ButtonView, GestureRuntime, LogicalRole, Motion};
use crate::{EngineError, Result, Session};

const SIDECAR: &str = "gesture-ui.json";
const MAX_COMBOS: usize = 64;
const MAX_NAME: usize = 64;
const MAX_BUTTON_NAMES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Off,
    ButtonLearn,
    GestureLearn,
    ButtonTest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hud {
    show: bool,
    trail: bool,
    direction: bool,
    preview: bool,
    hint: bool,
    execution: bool,
    error: bool,
    cancel: bool,
    reduced: bool,
    contrast: bool,
}

impl Default for Hud {
    fn default() -> Self {
        Self {
            show: true,
            trail: true,
            direction: true,
            preview: true,
            hint: true,
            execution: true,
            error: true,
            cancel: true,
            reduced: false,
            contrast: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    Chord,
    Hold,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Combo {
    roles: Vec<u32>,
    modifiers: u8,
    command: String,
    context: String,
    trigger: Trigger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Suppressed {
    id: String,
    context: String,
}

#[derive(Debug)]
struct UiState {
    runtime: GestureRuntime,
    mode: Mode,
    armed: bool,
    hand: String,
    context: String,
    hud: Hud,
    evdev_requested: bool,
    evdev_device: String,
    names: HashMap<u32, String>,
    combos: Vec<Combo>,
    suppressed: Vec<Suppressed>,
    active_context: String,
    blocked: bool,
    latched: bool,
    fired_chords: Vec<String>,
    hold_sent: Vec<u32>,
    learn_points: Vec<(i32, i32)>,
    learn_down: bool,
    trail: Vec<(i32, i32)>,
    clock: u64,
    status: String,
    loaded: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            runtime: GestureRuntime::new(default_thresholds()),
            mode: Mode::Off,
            armed: false,
            hand: "follow".into(),
            context: "global".into(),
            hud: Hud::default(),
            evdev_requested: false,
            evdev_device: String::new(),
            names: HashMap::new(),
            combos: Vec::new(),
            suppressed: Vec::new(),
            active_context: "global".into(),
            blocked: false,
            latched: false,
            fired_chords: Vec::new(),
            hold_sent: Vec::new(),
            learn_points: Vec::new(),
            learn_down: false,
            trail: Vec::new(),
            clock: 1_000_000_000,
            status: "idle".into(),
            loaded: false,
        }
    }
}

fn default_thresholds() -> Thresholds {
    Thresholds { min_length_px: 40, max_length_px: 2_000, slop_px: 8, min_ms: 20, max_ms: 1_500, angle_milli_deg: 20_000, hold_ms: 300 }
}

thread_local! {
    static UI: std::cell::RefCell<UiState> = std::cell::RefCell::new(UiState::default());
}

fn with_ui<T>(f: impl FnOnce(&mut UiState) -> T) -> T {
    UI.with(|ui| f(&mut ui.borrow_mut()))
}

/// Test helper. Drops the editor state on this thread.
pub fn reset() {
    with_ui(|ui| *ui = UiState::default());
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
        spec("surface.gesture.inspect", "Inspect Mouse Gestures", "{}", inspect),
        spec(
            "surface.gesture.configure",
            "Configure Mouse Gestures",
            r#"{"enabled?":bool,"armed?":bool,"hand?":str,"mirror?":bool,"context?":str,"thresholds?":object,"hud?":object,"evdevRequested?":bool,"evdevDevice?":str,"buttonCode?":int,"buttonName?":str}"#,
            configure,
        ),
        spec(
            "surface.gesture.sample",
            "Mouse Gesture Sample",
            r#"{"op":"press"|"move"|"release"|"cancel"|"focus"|"tick","code?":int,"x?":int,"y?":int,"tNs?":int,"modifiers?":int,"from?":"editor"|"window","context?":str,"widget?":str,"conflict?":bool}"#,
            sample,
        ),
        spec(
            "surface.gesture.learn",
            "Mouse Gesture Learn",
            r#"{"mode":"button"|"gesture"|"test"|"off","phase":"begin"|"end"|"save","id?":str,"spatial?":bool}"#,
            learn,
        ),
        spec("surface.gesture.template", "Mouse Gesture Template", r#"{"op":"set"|"remove","id":str,"points?":[[int,int]],"spatial?":bool}"#, template),
        spec("surface.gesture.map", "Map Mouse Gesture", r#"{"op":"set"|"remove","id":str,"command?":str,"context?":str,"params?":object}"#, map_gesture),
        spec(
            "surface.gesture.combo",
            "Mouse Gesture Combination",
            r#"{"op":"set"|"remove","roles":[int],"modifiers?":int,"command?":str,"context?":str,"trigger?":"chord"|"hold"}"#,
            combo,
        ),
        spec("surface.gesture.profile", "Mouse Gesture Profile", r#"{"op":"export"|"import"|"save"|"load"|"delete","document?":object}"#, profile),
        spec("surface.gesture.restore", "Restore Mouse Gestures", r#"{"state":object}"#, restore),
    ]
}

fn inspect(s: &mut Session, _p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|ui| {
        ui.runtime.set_enabled(pointer_enabled(&prefs));
        view(ui, &prefs)
    }))
}

fn configure(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let mut ui = with_ui(|ui| ui.clone_config());
    if let Some(enabled) = p.get("enabled").and_then(Value::as_bool) {
        ui.enabled = enabled;
    }
    if let Some(armed) = p.get("armed").and_then(Value::as_bool) {
        ui.armed = armed;
    }
    if let Some(hand) = p.get("hand").and_then(Value::as_str) {
        if !matches!(hand, "follow" | "right" | "left") {
            return Err(bad("hand is follow, right or left"));
        }
        ui.hand = hand.to_string();
    }
    if let Some(context) = p.get("context").and_then(Value::as_str) {
        ui.context = take_context(context)?;
    }
    if let Some(thresholds) = p.get("thresholds") {
        ui.thresholds = read_thresholds(thresholds)?;
    }
    if let Some(hud) = p.get("hud") {
        ui.hud = read_hud(&ui.hud, hud)?;
    }
    if let Some(flag) = p.get("evdevRequested").and_then(Value::as_bool) {
        ui.evdev_requested = flag;
    }
    if let Some(device) = p.get("evdevDevice").and_then(Value::as_str) {
        if device.chars().any(|c| c.is_control()) || device.len() > 128 {
            return Err(bad("evdev device name is not usable"));
        }
        ui.evdev_device = device.to_string();
    }
    if p.get("buttonCode").is_some() || p.get("buttonName").is_some() {
        let code = u32_field(p, "buttonCode")?;
        let name = p.get("buttonName").and_then(Value::as_str).unwrap_or("").trim();
        if name.chars().any(|c| c.is_control()) || name.chars().count() > MAX_NAME {
            return Err(bad("button name is not usable"));
        }
        if name.is_empty() {
            ui.names.remove(&code);
        } else {
            if ui.names.len() >= MAX_BUTTON_NAMES && !ui.names.contains_key(&code) {
                return Err(bad("too many button names"));
            }
            ui.names.insert(code, name.to_string());
        }
    }
    let mirror = p.get("mirror").and_then(Value::as_bool);
    apply_config(s, &ui, mirror)?;
    persist(s)?;
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|live| view(live, &prefs)))
}

fn sample(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let op = p.get("op").and_then(Value::as_str).unwrap_or("");
    let from = p.get("from").and_then(Value::as_str).unwrap_or("editor");
    if from != "editor" && from != "window" {
        return Err(bad("from is editor or window"));
    }
    let code = p.get("code").map(|_| u32_field(p, "code")).transpose()?;
    let x = i32_field(p, "x").unwrap_or(0);
    let y = i32_field(p, "y").unwrap_or(0);
    let modifiers = u8_field(p, "modifiers")?;
    let given = p.get("tNs").map(|_| u64_field(p, "tNs")).transpose()?;
    let context = p.get("context").and_then(Value::as_str).map(str::to_string);
    if let Some(context) = context.as_deref() {
        let _ = take_context(context)?;
    }
    let widget = p.get("widget").and_then(Value::as_str).unwrap_or("");
    if widget.chars().any(|c| c.is_control()) || widget.len() > 128 {
        return Err(bad("widget is not usable"));
    }
    let conflict = p.get("conflict").and_then(Value::as_bool).unwrap_or(false);
    let prefs = s.prefs.control_surfaces.clone();
    let reply = with_ui(|ui| {
        ui.runtime.set_enabled(pointer_enabled(&prefs));
        if from == "window" && !ui.armed {
            ui.status = "idle".into();
            return Ok(view(ui, &prefs));
        }
        if let Some(context) = context.as_deref() {
            ui.context = take_context(context)?;
        }
        let t_ns = match given {
            Some(t) => {
                ui.clock = ui.clock.max(t);
                t
            }
            None => {
                ui.clock = ui.clock.saturating_add(20_000_000);
                ui.clock
            }
        };
        let sample = PointerSampleIn { op, code, x, y, modifiers, t_ns, conflict, widget, explicit: context.as_deref() };
        feed(ui, &prefs, &sample)
    })?;
    Ok(reply)
}

fn learn(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("");
    let phase = p.get("phase").and_then(Value::as_str).unwrap_or("");
    let id = p.get("id").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let spatial = p.get("spatial").and_then(Value::as_bool).unwrap_or(false);
    let saved = with_ui(|ui| begin_learn(ui, mode, phase, &id, spatial))?;
    if let Some(template) = saved {
        store_template(s, template)?;
        persist(s)?;
    }
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|ui| view(ui, &prefs)))
}

fn template(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let op = p.get("op").and_then(Value::as_str).unwrap_or("");
    let id = required_id(p)?;
    match op {
        "remove" => {
            pointer_mut(&mut s.prefs.control_surfaces).gestures.retain(|template| template.id != id);
        }
        "set" => {
            let points = read_points(p.get("points"))?;
            let spatial = p.get("spatial").and_then(Value::as_bool).ok_or_else(|| bad("spatial is required"))?;
            store_template(s, StoredGesture { id, points, spatial })?;
        }
        _ => return Err(bad("op is set or remove")),
    }
    s.prefs.control_surfaces.clamp();
    persist(s)?;
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|ui| view(ui, &prefs)))
}

fn map_gesture(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let op = p.get("op").and_then(Value::as_str).unwrap_or("");
    let id = required_id(p)?;
    let context = take_context(p.get("context").and_then(Value::as_str).unwrap_or("global"))?;
    let device = pointer_mut(&mut s.prefs.control_surfaces);
    match op {
        "remove" => {
            let had_user = device.bindings.iter().any(|binding| gesture_key_is(binding, &id, &context));
            device.bindings.retain(|binding| !gesture_key_is(binding, &id, &context));
            if !had_user && builtin_command(&context, &id).is_some() {
                with_ui(|ui| suppress(ui, &id, &context));
            }
        }
        "set" => {
            let command = p.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if !gesture_command_allowed(&command) {
                return Err(bad("command is not playback, playhead or window"));
            }
            let params = p.get("params").cloned().unwrap_or(Value::Null);
            device.bindings.retain(|binding| !gesture_key_is(binding, &id, &context));
            device.bindings.push(Binding { control: ControlId::Gesture { id: id.clone() }, command, params, context: context.clone() });
            with_ui(|ui| ui.suppressed.retain(|row| row.id != id || row.context != context));
        }
        _ => return Err(bad("op is set or remove")),
    }
    s.prefs.control_surfaces.clamp();
    persist(s)?;
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|ui| view(ui, &prefs)))
}

fn combo(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let op = p.get("op").and_then(Value::as_str).unwrap_or("");
    let trigger = read_trigger(p.get("trigger").and_then(Value::as_str))?;
    let roles = match trigger {
        Trigger::Hold => read_hold_roles(p.get("roles"))?,
        Trigger::Chord => read_roles(p.get("roles"))?,
    };
    let modifiers = u8_field(p, "modifiers")?;
    let context = take_context(p.get("context").and_then(Value::as_str).unwrap_or("global"))?;
    with_ui(|ui| match op {
        "remove" => {
            ui.combos.retain(|combo| !same_combo(combo, &roles, modifiers, &context, trigger));
            Ok(())
        }
        "set" => {
            let command = p.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if !gesture_command_allowed(&command) {
                return Err(bad("command is not playback, playhead or window"));
            }
            if ui.combos.len() >= MAX_COMBOS && !ui.combos.iter().any(|combo| same_combo(combo, &roles, modifiers, &context, trigger)) {
                return Err(bad("too many combinations"));
            }
            ui.combos.retain(|combo| !same_combo(combo, &roles, modifiers, &context, trigger));
            ui.combos.push(Combo { roles, modifiers, command, context, trigger });
            Ok(())
        }
        _ => Err(bad("op is set or remove")),
    })?;
    persist(s)?;
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|ui| view(ui, &prefs)))
}

fn profile(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let op = p.get("op").and_then(Value::as_str).unwrap_or("");
    match op {
        "export" => {
            let prefs = s.prefs.control_surfaces.clone();
            let device = prefs.device(gesture::POINTER_SEAT);
            let document = crate::surface_profile::document(&device);
            let ui = with_ui(|ui| sidecar_of(ui));
            Ok(json!({"document": {"format": "cfa4-gesture-bundle", "version": 1, "profile": document, "ui": ui}}))
        }
        "import" => {
            let document = p.get("document").ok_or_else(|| bad("document is required"))?;
            import_document(s, document)?;
            persist(s)?;
            let prefs = s.prefs.control_surfaces.clone();
            Ok(with_ui(|ui| view(ui, &prefs)))
        }
        "save" => {
            persist(s)?;
            let prefs = s.prefs.control_surfaces.clone();
            Ok(with_ui(|ui| view(ui, &prefs)))
        }
        "load" => {
            with_ui(|ui| ui.loaded = false);
            ensure_loaded(s);
            let prefs = s.prefs.control_surfaces.clone();
            Ok(with_ui(|ui| view(ui, &prefs)))
        }
        "delete" => {
            let device = pointer_mut(&mut s.prefs.control_surfaces);
            device.gestures.clear();
            device.bindings.retain(|binding| !matches!(binding.control, ControlId::Gesture { .. }));
            device.mirror_ergonomic = false;
            with_ui(|ui| {
                ui.combos.clear();
                ui.suppressed.clear();
                ui.names.clear();
                ui.fired_chords.clear();
                ui.hold_sent.clear();
                ui.latched = false;
                ui.blocked = false;
                ui.mode = Mode::Off;
                ui.status = "idle".into();
            });
            persist(s)?;
            let prefs = s.prefs.control_surfaces.clone();
            Ok(with_ui(|ui| view(ui, &prefs)))
        }
        _ => Err(bad("op is export, import, save, load or delete")),
    }
}

fn restore(s: &mut Session, p: &Value) -> Result<Value> {
    ensure_loaded(s);
    let state = p.get("state").ok_or_else(|| bad("state is required"))?;
    apply_state(s, state)?;
    persist(s)?;
    let prefs = s.prefs.control_surfaces.clone();
    Ok(with_ui(|ui| view(ui, &prefs)))
}

#[derive(Clone, Debug)]
struct ConfigPatch {
    enabled: bool,
    armed: bool,
    hand: String,
    context: String,
    thresholds: Thresholds,
    hud: Hud,
    evdev_requested: bool,
    evdev_device: String,
    names: HashMap<u32, String>,
}

impl UiState {
    fn clone_config(&self) -> ConfigPatch {
        ConfigPatch {
            enabled: self.runtime.enabled(),
            armed: self.armed,
            hand: self.hand.clone(),
            context: self.context.clone(),
            thresholds: self.runtime.thresholds(),
            hud: self.hud.clone(),
            evdev_requested: self.evdev_requested,
            evdev_device: self.evdev_device.clone(),
            names: self.names.clone(),
        }
    }
}

fn apply_config(s: &mut Session, patch: &ConfigPatch, mirror: Option<bool>) -> Result<()> {
    let device = pointer_mut(&mut s.prefs.control_surfaces);
    device.enabled = patch.enabled;
    if let Some(mirror) = mirror {
        device.mirror_ergonomic = mirror;
    }
    s.prefs.control_surfaces.clamp();
    with_ui(|ui| {
        ui.runtime.set_enabled(patch.enabled);
        ui.runtime.set_thresholds(patch.thresholds);
        ui.armed = patch.armed;
        ui.hand = patch.hand.clone();
        ui.context = patch.context.clone();
        ui.hud = patch.hud.clone();
        ui.evdev_requested = patch.evdev_requested;
        ui.evdev_device = patch.evdev_device.clone();
        ui.names = patch.names.clone();
        ui.status = "idle".into();
    });
    Ok(())
}

fn begin_learn(ui: &mut UiState, mode: &str, phase: &str, id: &str, spatial: bool) -> Result<Option<StoredGesture>> {
    match (mode, phase) {
        ("off", "end") | ("off", "begin") => {
            ui.runtime.learn_end();
            ui.mode = Mode::Off;
            ui.learn_down = false;
            ui.learn_points.clear();
            ui.status = "idle".into();
            Ok(None)
        }
        ("button", "begin") => {
            ui.runtime.learn_begin();
            ui.mode = Mode::ButtonLearn;
            ui.status = "learning".into();
            Ok(None)
        }
        ("button", "end") => {
            ui.runtime.learn_end();
            ui.mode = Mode::Off;
            ui.status = "idle".into();
            Ok(None)
        }
        ("gesture", "begin") => {
            ui.runtime.learn_end();
            ui.mode = Mode::GestureLearn;
            ui.learn_points.clear();
            ui.learn_down = false;
            ui.status = "learning".into();
            Ok(None)
        }
        ("gesture", "end") => {
            ui.mode = Mode::Off;
            ui.learn_down = false;
            ui.status = "idle".into();
            Ok(None)
        }
        ("gesture", "save") => {
            if !gesture_id_ok(id) {
                return Err(bad("gesture id is not usable"));
            }
            if ui.learn_points.is_empty() {
                return Err(bad("draw a gesture first"));
            }
            let points = fit_points(&ui.learn_points);
            ui.mode = Mode::Off;
            ui.learn_down = false;
            ui.learn_points.clear();
            ui.status = "idle".into();
            Ok(Some(StoredGesture { id: id.to_string(), points, spatial }))
        }
        ("test", "begin") => {
            ui.runtime.learn_end();
            ui.mode = Mode::ButtonTest;
            ui.status = "test".into();
            Ok(None)
        }
        ("test", "end") => {
            ui.mode = Mode::Off;
            ui.status = "idle".into();
            Ok(None)
        }
        _ => Err(bad("learn mode or phase is not usable")),
    }
}

struct PointerSampleIn<'a> {
    op: &'a str,
    code: Option<u32>,
    x: i32,
    y: i32,
    modifiers: u8,
    t_ns: u64,
    conflict: bool,
    widget: &'a str,
    explicit: Option<&'a str>,
}

fn feed(ui: &mut UiState, prefs: &ControlSurfacePrefs, sample: &PointerSampleIn<'_>) -> Result<Value> {
    let capturing = !matches!(ui.mode, Mode::Off);
    begin_sample(ui, prefs, sample.conflict, sample.widget, sample.explicit);
    match sample.op {
        "press" => {
            let code = sample.code.ok_or_else(|| bad("code is required"))?;
            ui.runtime.set_modifiers(sample.modifiers);
            let view = ui.runtime.press(code, sample.t_ns);
            if ui.mode == Mode::GestureLearn {
                ui.learn_down = true;
                ui.learn_points.clear();
                ui.learn_points.push((sample.x, sample.y));
            }
            ui.trail.clear();
            ui.trail.push((sample.x, sample.y));
            ui.status = status_of(ui, Some(&view));
            let delivered = dispatch_buttons(ui, prefs, capturing)?;
            finish_flags(ui);
            Ok(view_with_button(ui, prefs, &view, Some((delivered, ui.runtime.hold_count()))))
        }
        "move" => {
            let motion = ui.runtime.move_to(sample.x, sample.y, sample.t_ns, sample.modifiers);
            if ui.mode == Mode::GestureLearn && ui.learn_down && ui.learn_points.last().is_none_or(|last| *last != (sample.x, sample.y)) {
                ui.learn_points.push((sample.x, sample.y));
                ui.learn_points.truncate(4_096);
            }
            if ui.trail.last().is_none_or(|last| *last != (sample.x, sample.y)) {
                ui.trail.push((sample.x, sample.y));
                ui.trail.truncate(64);
            }
            if motion == Motion::Ended {
                ui.status = "cancelled".into();
            }
            let delivered = dispatch_buttons(ui, prefs, capturing)?;
            let button = held_view(ui, sample.code);
            finish_flags(ui);
            Ok(view_with_button(ui, prefs, &button, Some((delivered, ui.runtime.hold_count()))))
        }
        "release" => {
            let code = sample.code.ok_or_else(|| bad("code is required"))?;
            let hold = ui.runtime.hold_count();
            let blocked = ui.blocked;
            let latched = ui.latched;
            let templates = crate::gesture_runtime::templates_from_profile(prefs);
            let input = ui.runtime.release(code, sample.t_ns, &templates).map_err(|e| bad(&e.to_string()))?;
            if ui.mode == Mode::GestureLearn {
                ui.learn_down = false;
            }
            let delivered = if blocked || latched { None } else { deliver_if_live(ui, prefs, input, capturing)? };
            finish_flags(ui);
            ui.status = status_of(ui, None);
            Ok(view_with_button(ui, prefs, &held_view(ui, Some(code)), Some((delivered, hold))))
        }
        "cancel" => {
            let cancelled = ui.runtime.cancel();
            ui.learn_down = false;
            ui.status = if cancelled { "cancelled".into() } else { ui.status.clone() };
            Ok(view_with_button(ui, prefs, &held_view(ui, sample.code), None))
        }
        "focus" => {
            let released = ui.runtime.release_stuck();
            ui.learn_down = false;
            ui.blocked = false;
            ui.latched = false;
            ui.fired_chords.clear();
            ui.hold_sent.clear();
            ui.status = "idle".into();
            let mut body = view(ui, prefs);
            if let Some(obj) = body.as_object_mut() {
                obj.insert("released".into(), json!(released));
            }
            Ok(body)
        }
        "tick" => {
            let _ = ui.runtime.move_to(sample.x, sample.y, sample.t_ns, sample.modifiers);
            let delivered = dispatch_buttons(ui, prefs, capturing)?;
            let button = held_view(ui, sample.code);
            finish_flags(ui);
            Ok(view_with_button(ui, prefs, &button, Some((delivered, ui.runtime.hold_count()))))
        }
        _ => Err(bad("op is press, move, release, cancel, focus or tick")),
    }
}

fn deliver_if_live(ui: &mut UiState, prefs: &ControlSurfacePrefs, input: Option<SurfaceInput>, capturing: bool) -> Result<Option<Value>> {
    if capturing || ui.blocked || ui.latched || !ui.runtime.enabled() || !prefs.enabled {
        return Ok(None);
    }
    let Some(input) = input else { return Ok(None) };
    let SurfaceInput::Gesture { id, .. } = &input else { return Ok(None) };
    let context = ui.active_context.clone();
    let id = id.clone();
    let value = crate::surface_runtime::with(|surfaces| ui.runtime.deliver_in(surfaces, prefs, &input, &context, Instant::now()))?;
    let status = value.pointer("/feedback/status").and_then(Value::as_str).unwrap_or("");
    if status != "unbound" {
        return Ok(Some(value));
    }
    if ui.suppressed.iter().any(|row| row.id == id && row.context == context) {
        return Ok(Some(value));
    }
    let Some(command) = builtin_command(&context, &id) else { return Ok(Some(value)) };
    if !gestures_live(prefs) {
        return Ok(Some(value));
    }
    let control = ControlId::Gesture { id };
    let emitted = crate::surface_runtime::with(|surfaces| surfaces.emit_gesture_action(gesture::POINTER_SEAT, control, command, Value::Null, &context));
    ui.runtime.note_dispatched();
    Ok(Some(emitted))
}

fn held_view(ui: &UiState, code: Option<u32>) -> ButtonView {
    let code = code.unwrap_or(0);
    ui.runtime.buttons().into_iter().find(|button| button.code == code).unwrap_or(ButtonView {
        code,
        control: ControlId::Hid { name: format!("qt-button-{code}"), usage_page: 0, usage: 0 },
        role: None,
        fresh: false,
        pressed: ui.runtime.is_pressed(code),
    })
}

fn view_with_button(ui: &UiState, prefs: &ControlSurfacePrefs, button: &ButtonView, extra: Option<(Option<Value>, usize)>) -> Value {
    let mut body = view(ui, prefs);
    if let Some(obj) = body.as_object_mut() {
        obj.insert("button".into(), button_json(ui, button));
        if let Some((delivered, hold)) = extra {
            obj.insert("hold".into(), json!(hold > 0));
            if let Some(delivered) = delivered {
                obj.insert("injected".into(), delivered);
            }
        }
    }
    body
}

fn status_of(ui: &UiState, button: Option<&ButtonView>) -> String {
    if ui.mode == Mode::ButtonTest {
        if button.is_some_and(|b| b.pressed) { "press".into() } else { "release".into() }
    } else if ui.mode == Mode::ButtonLearn || ui.mode == Mode::GestureLearn {
        "learning".into()
    } else if matches!(ui.runtime.last_outcome(), Some(Recognize::Cancelled)) {
        "cancelled".into()
    } else {
        "idle".into()
    }
}

fn view(ui: &UiState, prefs: &ControlSurfacePrefs) -> Value {
    let device = prefs.device(gesture::POINTER_SEAT);
    let templates: Vec<Value> =
        device.gestures.iter().map(|template| json!({"id": template.id, "spatial": template.spatial, "points": template.points})).collect();
    let mappings: Vec<Value> = device
        .bindings
        .iter()
        .filter_map(|binding| match &binding.control {
            ControlId::Gesture { id } => {
                Some(json!({"id": id, "command": binding.command, "context": gesture_context_key(&binding.context), "params": binding.params}))
            }
            _ => None,
        })
        .collect();
    let buttons: Vec<Value> = ui.runtime.buttons().iter().map(|button| button_json(ui, button)).collect();
    let learned: Vec<Value> = ui.runtime.learned().iter().map(|button| button_json(ui, button)).collect();
    let combinations: Vec<Value> = ui.combos.iter().map(combo_json).collect();
    let thresholds = ui.runtime.thresholds();
    let outcome = outcome_json(ui.runtime.last_outcome());
    json!({
        "enabled": pointer_enabled(prefs),
        "surfacesEnabled": prefs.enabled,
        "armed": ui.armed,
        "hand": ui.hand,
        "mirror": device.mirror_ergonomic,
        "context": ui.context,
        "activeContext": ui.active_context,
        "profiles": profiles_json(ui, prefs),
        "path": "qt",
        "evdevRequested": ui.evdev_requested,
        "evdevDevice": ui.evdev_device,
        "evdev": "not-opened",
        "mode": mode_name(ui.mode),
        "status": ui.status,
        "thresholds": {
            "minLengthPx": thresholds.min_length_px,
            "maxLengthPx": thresholds.max_length_px,
            "slopPx": thresholds.slop_px,
            "minMs": thresholds.min_ms,
            "maxMs": thresholds.max_ms,
            "angleMilliDeg": thresholds.angle_milli_deg,
            "holdMs": thresholds.hold_ms,
        },
        "hud": hud_json(&ui.hud),
        "hudFrame": hud_frame(ui, &device),
        "templates": templates,
        "mappings": mappings,
        "buttons": buttons,
        "learned": learned,
        "combinations": combinations,
        "outcome": outcome,
        "warning": if ui.evdev_requested { "evdev is stored and is not opened; the Qt path stays" } else { "" },
        "state": state_blob(ui, &device, pointer_enabled(prefs)),
    })
}

fn hud_frame(ui: &UiState, device: &DeviceSetting) -> Value {
    let outcome = ui.runtime.last_outcome();
    let hint = outcome.and_then(|item| {
        let id = match item {
            Recognize::Direction(direction) => direction.as_id(),
            Recognize::Match { id, .. } => id.as_str(),
            Recognize::None | Recognize::Cancelled => return None,
        };
        command_for(ui, device, id, &ui.active_context)
    });
    json!({
        "show": ui.hud.show,
        "trail": if ui.hud.trail && !ui.hud.reduced { ui.trail.clone() } else { Vec::<(i32, i32)>::new() },
        "direction": outcome.and_then(direction_name).unwrap_or_default(),
        "preview": outcome_json(outcome),
        "hint": hint.unwrap_or_default(),
        "cancelled": matches!(outcome, Some(Recognize::Cancelled)),
        "error": "",
        "reduced": ui.hud.reduced,
        "contrast": ui.hud.contrast,
        "hand": ui.hand,
    })
}

fn state_blob(ui: &UiState, device: &DeviceSetting, enabled: bool) -> Value {
    json!({
        "ui": sidecar_of(ui),
        "gestures": device.gestures,
        "bindings": device.bindings.iter().filter(|binding| matches!(binding.control, ControlId::Gesture { .. })).cloned().collect::<Vec<_>>(),
        "mirror": device.mirror_ergonomic,
        "enabled": enabled,
    })
}

fn sidecar_of(ui: &UiState) -> Value {
    let names: Vec<Value> = ui.names.iter().map(|(code, name)| json!({"code": code, "name": name})).collect();
    json!({
        "format": "cfa4-gesture-ui",
        "version": 1,
        "thresholds": {
            "minLengthPx": ui.runtime.thresholds().min_length_px,
            "maxLengthPx": ui.runtime.thresholds().max_length_px,
            "slopPx": ui.runtime.thresholds().slop_px,
            "minMs": ui.runtime.thresholds().min_ms,
            "maxMs": ui.runtime.thresholds().max_ms,
            "angleMilliDeg": ui.runtime.thresholds().angle_milli_deg,
            "holdMs": ui.runtime.thresholds().hold_ms,
        },
        "hud": hud_json(&ui.hud),
        "armed": ui.armed,
        "hand": ui.hand,
        "context": ui.context,
        "evdevRequested": ui.evdev_requested,
        "evdevDevice": ui.evdev_device,
        "buttonNames": names,
        "combinations": ui.combos.iter().map(combo_json).collect::<Vec<_>>(),
        "suppressed": ui.suppressed.iter().map(|row| json!({"id": row.id, "context": row.context})).collect::<Vec<_>>(),
    })
}

fn apply_state(s: &mut Session, state: &Value) -> Result<()> {
    let ui_doc = state.get("ui").cloned().unwrap_or_else(|| state.clone());
    apply_sidecar(&ui_doc)?;
    let device = pointer_mut(&mut s.prefs.control_surfaces);
    if let Some(enabled) = state.get("enabled").and_then(Value::as_bool) {
        device.enabled = enabled;
    }
    if let Some(mirror) = state.get("mirror").and_then(Value::as_bool) {
        device.mirror_ergonomic = mirror;
    }
    if let Some(gestures) = state.get("gestures").and_then(Value::as_array) {
        device.gestures = gestures.iter().filter_map(stored_of).collect();
    }
    if let Some(bindings) = state.get("bindings").and_then(Value::as_array) {
        device.bindings.retain(|binding| !matches!(binding.control, ControlId::Gesture { .. }));
        for binding in bindings {
            if let Some(binding) = binding_of(binding) {
                device.bindings.push(binding);
            }
        }
    }
    s.prefs.control_surfaces.clamp();
    let enabled = s.prefs.control_surfaces.device(gesture::POINTER_SEAT).enabled;
    with_ui(|ui| ui.runtime.set_enabled(enabled));
    Ok(())
}

fn import_document(s: &mut Session, document: &Value) -> Result<()> {
    let format = document.get("format").and_then(Value::as_str).unwrap_or("");
    match format {
        "cfa4-gesture-bundle" => {
            let profile = document.get("profile").ok_or_else(|| bad("profile is missing"))?;
            import_profile(s, profile)?;
            if let Some(ui) = document.get("ui") {
                apply_sidecar(ui)?;
            }
            Ok(())
        }
        "cfa4-surface-profile" => import_profile(s, document),
        "cfa4-gesture-ui" => apply_sidecar(document),
        _ => Err(bad("document format is not a gesture profile")),
    }
}

fn import_profile(s: &mut Session, document: &Value) -> Result<()> {
    let device_id = crate::surface_profile::validate(document).map_err(|e| bad(&format!("{e:?}")))?;
    if device_id != gesture::POINTER_SEAT && !device_id.starts_with("pointer:") {
        return Err(bad("the file is not a pointer profile"));
    }
    let gestures: Vec<StoredGesture> =
        document.get("gestures").and_then(Value::as_array).map(|rows| rows.iter().filter_map(stored_of).collect()).unwrap_or_default();
    let bindings: Vec<Binding> =
        document.get("controls").and_then(Value::as_array).map(|rows| rows.iter().filter_map(control_binding).collect()).unwrap_or_default();
    let mirror = document.get("mirrorErgonomic").and_then(Value::as_bool).unwrap_or(false);
    let device = pointer_mut(&mut s.prefs.control_surfaces);
    device.gestures = gestures;
    device.mirror_ergonomic = mirror;
    device.bindings.retain(|binding| !matches!(binding.control, ControlId::Gesture { .. }));
    device.bindings.extend(bindings);
    s.prefs.control_surfaces.clamp();
    Ok(())
}

fn ensure_loaded(s: &Session) {
    let bytes = profile_dir(s).and_then(|dir| std::fs::read(dir.join(SIDECAR)).ok());
    with_ui(|ui| {
        if ui.loaded {
            return;
        }
        ui.loaded = true;
        if let Some(bytes) = &bytes
            && let Ok(value) = serde_json::from_slice::<Value>(bytes)
        {
            let _ = apply_sidecar_to(ui, &value);
        }
    });
}

fn pointer_enabled(prefs: &ControlSurfacePrefs) -> bool {
    prefs.devices.iter().any(|device| device.device == gesture::POINTER_SEAT && device.enabled)
}

fn apply_sidecar(document: &Value) -> Result<()> {
    with_ui(|ui| apply_sidecar_to(ui, document))
}

fn apply_sidecar_to(ui: &mut UiState, document: &Value) -> Result<()> {
    let thresholds = document.get("thresholds").map(read_thresholds).transpose()?.unwrap_or_else(default_thresholds);
    let hand = document.get("hand").and_then(Value::as_str).unwrap_or("follow");
    if !matches!(hand, "follow" | "right" | "left") {
        return Err(bad("hand is follow, right or left"));
    }
    let hand = hand.to_string();
    let context = take_context(document.get("context").and_then(Value::as_str).unwrap_or("global"))?;
    let mut names = HashMap::new();
    for row in document.get("buttonNames").and_then(Value::as_array).into_iter().flatten() {
        let Some(code) = row.get("code").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()) else { continue };
        let Some(name) = row.get("name").and_then(Value::as_str) else { continue };
        if name.chars().any(|c| c.is_control()) || name.chars().count() > MAX_NAME || names.len() >= MAX_BUTTON_NAMES {
            continue;
        }
        names.insert(code, name.to_string());
    }
    let mut combos = Vec::new();
    for row in document.get("combinations").and_then(Value::as_array).into_iter().flatten() {
        if combos.len() >= MAX_COMBOS {
            break;
        }
        let Some(combo) = combo_of(row) else { continue };
        combos.push(combo);
    }
    let mut suppressed = Vec::new();
    for row in document.get("suppressed").and_then(Value::as_array).into_iter().flatten() {
        if suppressed.len() >= MAX_COMBOS {
            break;
        }
        let Some(id) = row.get("id").and_then(Value::as_str) else { continue };
        if !gesture_id_ok(id) {
            continue;
        }
        let Ok(context) = take_context(row.get("context").and_then(Value::as_str).unwrap_or("global")) else { continue };
        suppressed.push(Suppressed { id: id.to_string(), context });
    }
    let armed = document.get("armed").and_then(Value::as_bool).unwrap_or(false);
    let evdev_requested = document.get("evdevRequested").and_then(Value::as_bool).unwrap_or(false);
    let evdev_device = document.get("evdevDevice").and_then(Value::as_str).unwrap_or("").to_string();
    let hud = document.get("hud").map(|hud| read_hud(&ui.hud, hud)).transpose()?.unwrap_or_else(|| ui.hud.clone());
    ui.runtime.set_thresholds(thresholds);
    ui.hud = hud;
    ui.hand = hand;
    ui.context = context;
    ui.names = names;
    ui.combos = combos;
    ui.suppressed = suppressed;
    ui.armed = armed;
    ui.evdev_requested = evdev_requested;
    ui.evdev_device = evdev_device;
    Ok(())
}

fn store_template(s: &mut Session, template: StoredGesture) -> Result<()> {
    if !gesture_id_ok(&template.id) || template.points.is_empty() || template.points.len() > MAX_TEMPLATE_POINTS {
        return Err(bad("gesture template is not usable"));
    }
    let device = pointer_mut(&mut s.prefs.control_surfaces);
    device.gestures.retain(|known| known.id != template.id);
    device.gestures.push(template);
    s.prefs.control_surfaces.clamp();
    Ok(())
}

fn persist(s: &Session) -> Result<()> {
    let Some(dir) = profile_dir(s) else { return Ok(()) };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(EngineError::Other(format!("gesture profiles: cannot create {}: {e}", dir.display())));
    }
    let prefs = &s.prefs.control_surfaces;
    let warnings = crate::surface_profile::save_all(prefs, &dir);
    if warnings.iter().any(|warning| warning.contains("pointer")) {
        return Err(EngineError::Other(warnings.join("; ")));
    }
    let body = with_ui(|ui| sidecar_of(ui));
    let bytes = serde_json::to_vec_pretty(&body).map_err(|e| EngineError::Other(e.to_string()))?;
    cfa4_format::atomic_write(&dir.join(SIDECAR), &bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(())
}

fn profile_dir(s: &Session) -> Option<PathBuf> {
    s.prefs_path.as_ref().and_then(|path| path.parent()).map(crate::surface_profile::dir_for)
}

fn pointer_mut(prefs: &mut ControlSurfacePrefs) -> &mut DeviceSetting {
    let id = gesture::POINTER_SEAT;
    if !prefs.devices.iter().any(|device| device.device == id) {
        prefs.devices.push(DeviceSetting { device: id.to_string(), enabled: false, ..DeviceSetting::default() });
    }
    let index = prefs.devices.iter().position(|device| device.device == id).unwrap_or(0);
    if index < prefs.devices.len() {
        return &mut prefs.devices[index];
    }
    prefs.devices.push(DeviceSetting { device: id.to_string(), enabled: false, ..DeviceSetting::default() });
    let last = prefs.devices.len().saturating_sub(1);
    &mut prefs.devices[last]
}

fn gesture_key_is(binding: &Binding, id: &str, context: &str) -> bool {
    matches!(&binding.control, ControlId::Gesture { id: bound } if bound == id && gesture_context_key(&binding.context) == context)
}

fn button_json(ui: &UiState, button: &ButtonView) -> Value {
    json!({
        "code": button.code,
        "role": role_name(button.role),
        "roleIndex": button.role.map(LogicalRole::index),
        "pressed": button.pressed,
        "fresh": button.fresh,
        "name": ui.names.get(&button.code).cloned().unwrap_or_default(),
        "path": "qt",
        "label": button_label(ui.hand.as_str(), button.role, ui.names.get(&button.code).map(String::as_str)),
    })
}

fn combo_json(combo: &Combo) -> Value {
    let key = combo.roles.iter().map(|role| role.to_string()).collect::<Vec<_>>().join(",");
    json!({
        "roles": combo.roles,
        "modifiers": combo.modifiers,
        "command": combo.command,
        "context": combo.context,
        "trigger": trigger_name(combo.trigger),
        "key": format!("{key}+{}", combo.modifiers),
    })
}

fn combo_of(row: &Value) -> Option<Combo> {
    let trigger = read_trigger(row.get("trigger").and_then(Value::as_str)).ok()?;
    let roles = match trigger {
        Trigger::Hold => read_hold_roles(row.get("roles")).ok()?,
        Trigger::Chord => read_roles(row.get("roles")).ok()?,
    };
    let modifiers = u8::try_from(row.get("modifiers").and_then(Value::as_u64).unwrap_or(0)).ok()?;
    let command = row.get("command").and_then(Value::as_str)?.trim().to_string();
    if !gesture_command_allowed(&command) {
        return None;
    }
    let context = take_context(row.get("context").and_then(Value::as_str).unwrap_or("global")).ok()?;
    Some(Combo { roles, modifiers, command, context, trigger })
}

fn stored_of(row: &Value) -> Option<StoredGesture> {
    let id = row.get("id").and_then(Value::as_str)?.to_string();
    if !gesture_id_ok(&id) {
        return None;
    }
    let spatial = row.get("spatial").and_then(Value::as_bool).unwrap_or(false);
    let points = read_points(row.get("points")).ok()?;
    Some(StoredGesture { id, points, spatial })
}

fn binding_of(row: &Value) -> Option<Binding> {
    control_binding(row)
}

fn control_binding(row: &Value) -> Option<Binding> {
    let control = if let Some(id) = row.get("id").and_then(Value::as_str) {
        ControlId::Gesture { id: id.to_string() }
    } else {
        serde_json::from_value(row.get("control")?.clone()).ok()?
    };
    let ControlId::Gesture { id } = &control else { return None };
    if !gesture_id_ok(id) {
        return None;
    }
    let command = row.get("command").and_then(Value::as_str)?.trim().to_string();
    if !gesture_command_allowed(&command) {
        return None;
    }
    let params = row.get("params").cloned().unwrap_or(Value::Null);
    let context = take_context(row.get("context").and_then(Value::as_str).unwrap_or("global")).ok()?;
    Some(Binding { control, command, params, context })
}

/// Spatial directions on the Timeline. Time runs left to right, so these ids are not mirrored.
const TIMELINE_PROFILE: &[(&str, &str)] = &[
    ("direction.right", "playhead.nextEdit"),
    ("direction.left", "playhead.prevEdit"),
    ("direction.up-right", "playhead.end"),
    ("direction.down-left", "playhead.start"),
    ("direction.up", "playback.forward"),
    ("direction.down", "playback.reverse"),
    ("direction.up-left", "playback.toggle"),
    ("direction.down-right", "playback.stop"),
];

/// Media Browser examples that MG0 decision 6 does not allow on a gesture (`mediaBrowser.*`).
const MEDIA_BROWSER_DEFERRED: &[&str] = &["mediaBrowser.navigate", "mediaBrowser.select", "mediaBrowser.favorite"];

fn take_context(context: &str) -> Result<String> {
    let canonical = canonical_context(context);
    if canonical != "global" && !gesture_id_ok(&canonical) {
        return Err(bad("context is not usable"));
    }
    Ok(canonical)
}

fn canonical_context(context: &str) -> String {
    let key = gesture_context_key(context.trim());
    match key.to_ascii_lowercase().as_str() {
        "timeline" => "Timeline".into(),
        "mediabrowser" | "media-browser" | "media browser" => "MediaBrowser".into(),
        _ => key.to_string(),
    }
}

fn builtin_command<'a>(context: &str, id: &str) -> Option<&'a str> {
    if context != "Timeline" {
        return None;
    }
    TIMELINE_PROFILE.iter().find(|(gesture, _)| *gesture == id).map(|(_, command)| *command)
}

fn gestures_live(prefs: &ControlSurfacePrefs) -> bool {
    prefs.enabled && prefs.devices.iter().any(|device| device.device == gesture::POINTER_SEAT && device.enabled)
}

fn same_combo(combo: &Combo, roles: &[u32], modifiers: u8, context: &str, trigger: Trigger) -> bool {
    combo.roles == roles && combo.modifiers == modifiers && combo.context == context && combo.trigger == trigger
}

fn read_trigger(value: Option<&str>) -> Result<Trigger> {
    match value.unwrap_or("chord") {
        "chord" => Ok(Trigger::Chord),
        "hold" => Ok(Trigger::Hold),
        _ => Err(bad("trigger is chord or hold")),
    }
}

fn trigger_name(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::Chord => "chord",
        Trigger::Hold => "hold",
    }
}

fn suppress(ui: &mut UiState, id: &str, context: &str) {
    if ui.suppressed.iter().any(|row| row.id == id && row.context == context) {
        return;
    }
    if ui.suppressed.len() >= MAX_COMBOS {
        return;
    }
    ui.suppressed.push(Suppressed { id: id.to_string(), context: context.to_string() });
}

fn context_configured(ui: &UiState, prefs: &ControlSurfacePrefs, context: &str) -> bool {
    if context.is_empty() {
        return false;
    }
    if ui.combos.iter().any(|combo| combo.context == context) || ui.suppressed.iter().any(|row| row.context == context) {
        return true;
    }
    let device = prefs.device(gesture::POINTER_SEAT);
    device.bindings.iter().any(|binding| matches!(&binding.control, ControlId::Gesture { .. } if gesture_context_key(&binding.context) == context))
}

fn module_of(widget: &str) -> Option<&'static str> {
    let widget = widget.trim();
    if widget == "panel.Timeline" || widget.starts_with("timeline.") || widget.starts_with("panel.menu.Timeline") {
        return Some("Timeline");
    }
    if widget == "panel.MediaBrowser" || widget.starts_with("mediaBrowser.") || widget.starts_with("panel.menu.MediaBrowser") {
        return Some("MediaBrowser");
    }
    None
}

fn resolve_context(ui: &UiState, prefs: &ControlSurfacePrefs, widget: &str, explicit: Option<&str>) -> String {
    if let Some(explicit) = explicit
        && let Ok(context) = take_context(explicit)
    {
        return context;
    }
    let widget = widget.trim();
    if context_configured(ui, prefs, widget)
        && let Ok(context) = take_context(widget)
    {
        return context;
    }
    if let Some(module) = module_of(widget) {
        return module.to_string();
    }
    if widget.is_empty() { ui.context.clone() } else { "global".into() }
}

fn begin_sample(ui: &mut UiState, prefs: &ControlSurfacePrefs, conflict: bool, widget: &str, explicit: Option<&str>) {
    if conflict {
        ui.blocked = true;
    }
    ui.active_context = resolve_context(ui, prefs, widget, explicit);
}

fn finish_flags(ui: &mut UiState) {
    if !ui.runtime.buttons().iter().any(|button| button.pressed) {
        ui.blocked = false;
        ui.latched = false;
        ui.fired_chords.clear();
        ui.hold_sent.clear();
        return;
    }
    let down = ui.runtime.combination().roles;
    ui.hold_sent.retain(|role| down.contains(role));
}

fn command_for(ui: &UiState, device: &DeviceSetting, id: &str, context: &str) -> Option<String> {
    if ui.suppressed.iter().any(|row| row.id == id && row.context == context) {
        return None;
    }
    if let Some(binding) = device.bindings.iter().find(|binding| gesture_key_is(binding, id, context)) {
        return Some(binding.command.clone());
    }
    builtin_command(context, id).map(str::to_string)
}

fn profiles_json(ui: &UiState, prefs: &ControlSurfacePrefs) -> Vec<Value> {
    let device = prefs.device(gesture::POINTER_SEAT);
    let timeline: Vec<Value> = TIMELINE_PROFILE
        .iter()
        .map(|(id, builtin)| {
            let source = if ui.suppressed.iter().any(|row| row.id == *id && row.context == "Timeline") {
                "off"
            } else if device.bindings.iter().any(|binding| gesture_key_is(binding, id, "Timeline")) {
                "user"
            } else {
                "builtin"
            };
            json!({"id": id, "command": command_for(ui, &device, id, "Timeline").unwrap_or_default(), "context": "Timeline", "source": source, "builtinCommand": builtin})
        })
        .collect();
    vec![
        json!({"context": "Timeline", "panel": "Timeline", "builtin": true, "mappings": timeline}),
        json!({"context": "MediaBrowser", "panel": "Media Browser", "builtin": true, "mappings": [], "deferred": MEDIA_BROWSER_DEFERRED}),
    ]
}

fn dispatch_buttons(ui: &mut UiState, prefs: &ControlSurfacePrefs, capturing: bool) -> Result<Option<Value>> {
    if capturing || ui.blocked || !ui.runtime.enabled() || !prefs.enabled || ui.runtime.moved_past_slop() {
        return Ok(None);
    }
    let live = ui.runtime.combination();
    let chord = if live.roles.len() >= 2 { find_combo(ui, &live.roles, live.modifiers, Trigger::Chord) } else { None };
    if let Some(combo) = chord {
        let key = chord_key(&combo);
        if !ui.fired_chords.iter().any(|fired| fired == &key) {
            ui.fired_chords.push(key);
            ui.latched = true;
            return Ok(Some(emit_combo(ui, combo)));
        }
    }
    let held = ui.runtime.held_roles();
    if held.len() == 1 && live.roles == held {
        let role = held[0];
        let hold = if ui.hold_sent.contains(&role) { None } else { find_combo(ui, &held, live.modifiers, Trigger::Hold) };
        if let Some(combo) = hold {
            ui.hold_sent.push(role);
            ui.latched = true;
            return Ok(Some(emit_combo(ui, combo)));
        }
    }
    Ok(None)
}

fn find_combo(ui: &UiState, roles: &[u32], modifiers: u8, trigger: Trigger) -> Option<Combo> {
    let context = ui.active_context.as_str();
    ui.combos.iter().find(|combo| combo.trigger == trigger && combo.roles == roles && combo.modifiers == modifiers && combo.context == context).cloned()
}

fn chord_key(combo: &Combo) -> String {
    let roles = combo.roles.iter().map(|role| role.to_string()).collect::<Vec<_>>().join(".");
    format!("{}:{}:{roles}:{}", combo.context, trigger_name(combo.trigger), combo.modifiers)
}

fn emit_combo(ui: &mut UiState, combo: Combo) -> Value {
    let control = ControlId::Gesture { id: combo_control_id(&combo) };
    let context = combo.context.clone();
    let command = combo.command.clone();
    let value = crate::surface_runtime::with(|surfaces| surfaces.emit_gesture_action(gesture::POINTER_SEAT, control, &command, Value::Null, &context));
    ui.runtime.note_dispatched();
    value
}

fn combo_control_id(combo: &Combo) -> String {
    let roles = combo.roles.iter().map(|role| role.to_string()).collect::<Vec<_>>().join(".");
    format!("{}.{roles}.m{}", trigger_name(combo.trigger), combo.modifiers)
}

fn read_points(value: Option<&Value>) -> Result<Vec<(i32, i32)>> {
    let Some(rows) = value.and_then(Value::as_array) else { return Err(bad("points are required")) };
    if rows.is_empty() || rows.len() > MAX_TEMPLATE_POINTS {
        return Err(bad("points are empty or too long"));
    }
    let mut points = Vec::new();
    for row in rows {
        let pair = row.as_array().ok_or_else(|| bad("a point is not a pair"))?;
        if pair.len() != 2 {
            return Err(bad("a point is not a pair"));
        }
        let x = pair.first().and_then(Value::as_i64).ok_or_else(|| bad("a point is not an integer"))?;
        let y = pair.get(1).and_then(Value::as_i64).ok_or_else(|| bad("a point is not an integer"))?;
        points.push((i32::try_from(x).map_err(|_| bad("a point is out of range"))?, i32::try_from(y).map_err(|_| bad("a point is out of range"))?));
    }
    Ok(points)
}

fn read_hold_roles(value: Option<&Value>) -> Result<Vec<u32>> {
    let Some(rows) = value.and_then(Value::as_array) else { return Err(bad("roles are required")) };
    if rows.is_empty() || rows.len() > 8 {
        return Err(bad("a hold needs one role"));
    }
    let mut roles = Vec::new();
    for row in rows {
        let role = row.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad("a role is not a number"))?;
        roles.push(role);
    }
    roles.sort_unstable();
    roles.dedup();
    if roles.len() != 1 {
        return Err(bad("a hold needs one role"));
    }
    Ok(roles)
}

fn read_roles(value: Option<&Value>) -> Result<Vec<u32>> {
    let Some(rows) = value.and_then(Value::as_array) else { return Err(bad("roles are required")) };
    if rows.len() < 2 || rows.len() > 8 {
        return Err(bad("a combination needs 2 to 8 roles"));
    }
    let mut roles = Vec::new();
    for row in rows {
        let role = row.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad("a role is not a number"))?;
        roles.push(role);
    }
    roles.sort_unstable();
    roles.dedup();
    if roles.len() < 2 {
        return Err(bad("a combination needs 2 roles"));
    }
    Ok(roles)
}

fn read_thresholds(value: &Value) -> Result<Thresholds> {
    let min_length = u32_of(value, "minLengthPx", 40)?;
    let max_length = u32_of(value, "maxLengthPx", 2_000)?;
    let slop = u32_of(value, "slopPx", 8)?;
    let min_ms = u64_of(value, "minMs", 20)?;
    let max_ms = u64_of(value, "maxMs", 1_500)?;
    let angle = u32_of(value, "angleMilliDeg", 20_000)?;
    let hold = u64_of(value, "holdMs", 300)?;
    if min_length == 0
        || max_length < min_length
        || max_length > 100_000
        || slop > 10_000
        || min_ms == 0
        || max_ms <= min_ms
        || max_ms > 60_000
        || angle > 180_000
        || hold == 0
        || hold > 60_000
    {
        return Err(bad("thresholds are out of range"));
    }
    Ok(Thresholds { min_length_px: min_length, max_length_px: max_length, slop_px: slop, min_ms, max_ms, angle_milli_deg: angle, hold_ms: hold })
}

fn read_hud(current: &Hud, value: &Value) -> Result<Hud> {
    if !value.is_object() {
        return Err(bad("hud is not an object"));
    }
    let flag = |key, default| value.get(key).and_then(Value::as_bool).unwrap_or(default);
    Ok(Hud {
        show: flag("show", current.show),
        trail: flag("trail", current.trail),
        direction: flag("direction", current.direction),
        preview: flag("preview", current.preview),
        hint: flag("hint", current.hint),
        execution: flag("execution", current.execution),
        error: flag("error", current.error),
        cancel: flag("cancel", current.cancel),
        reduced: flag("reduced", current.reduced),
        contrast: flag("contrast", current.contrast),
    })
}

fn hud_json(hud: &Hud) -> Value {
    json!({"show": hud.show, "trail": hud.trail, "direction": hud.direction, "preview": hud.preview, "hint": hud.hint, "execution": hud.execution, "error": hud.error, "cancel": hud.cancel, "reduced": hud.reduced, "contrast": hud.contrast})
}

fn fit_points(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if points.len() <= MAX_TEMPLATE_POINTS {
        return points.to_vec();
    }
    let last = points.len().saturating_sub(1);
    (0..MAX_TEMPLATE_POINTS).filter_map(|i| points.get(i.saturating_mul(last) / (MAX_TEMPLATE_POINTS - 1)).copied()).collect()
}

fn outcome_json(outcome: Option<&Recognize>) -> Value {
    match outcome {
        None => Value::Null,
        Some(Recognize::None) => json!({"kind": "none"}),
        Some(Recognize::Cancelled) => json!({"kind": "cancelled"}),
        Some(Recognize::Direction(direction)) => json!({"kind": "direction", "id": direction.as_id()}),
        Some(Recognize::Match { id, confidence_milli }) => json!({"kind": "match", "id": id, "confidence": confidence_milli}),
    }
}

fn direction_name(outcome: &Recognize) -> Option<String> {
    match outcome {
        Recognize::Direction(direction) => Some(
            match direction {
                Direction::Right => "right",
                Direction::Left => "left",
                Direction::Up => "up",
                Direction::Down => "down",
                Direction::UpLeft => "up-left",
                Direction::UpRight => "up-right",
                Direction::DownLeft => "down-left",
                Direction::DownRight => "down-right",
            }
            .to_string(),
        ),
        _ => None,
    }
}

fn role_name(role: Option<LogicalRole>) -> String {
    match role {
        None => String::new(),
        Some(LogicalRole::Primary) => "primary".into(),
        Some(LogicalRole::Secondary) => "secondary".into(),
        Some(LogicalRole::Middle) => "middle".into(),
        Some(LogicalRole::Learned(n)) => format!("learned-{n}"),
    }
}

fn button_label(hand: &str, role: Option<LogicalRole>, name: Option<&str>) -> String {
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    let left_handed = hand == "left";
    match role {
        Some(LogicalRole::Primary) => {
            if left_handed {
                "right button".into()
            } else {
                "left button".into()
            }
        }
        Some(LogicalRole::Secondary) => {
            if left_handed {
                "left button".into()
            } else {
                "right button".into()
            }
        }
        Some(LogicalRole::Middle) => "middle button".into(),
        Some(LogicalRole::Learned(n)) => format!("learned button {n}"),
        None => "unknown button".into(),
    }
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Off => "off",
        Mode::ButtonLearn => "button",
        Mode::GestureLearn => "gesture",
        Mode::ButtonTest => "test",
    }
}

fn required_id(p: &Value) -> Result<String> {
    let id = p.get("id").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !gesture_id_ok(&id) {
        return Err(bad("gesture id is not usable"));
    }
    Ok(id)
}

fn u32_of(value: &Value, key: &str, default: u32) -> Result<u32> {
    match value.get(key).and_then(Value::as_u64) {
        None => Ok(default),
        Some(n) => u32::try_from(n).map_err(|_| bad("a threshold is out of range")),
    }
}

fn u64_of(value: &Value, key: &str, default: u64) -> Result<u64> {
    Ok(value.get(key).and_then(Value::as_u64).unwrap_or(default))
}

fn u32_field(p: &Value, key: &str) -> Result<u32> {
    let n = p.get(key).and_then(Value::as_u64).ok_or_else(|| bad(&format!("{key} is required")))?;
    u32::try_from(n).map_err(|_| bad(&format!("{key} is out of range")))
}

fn u64_field(p: &Value, key: &str) -> Result<u64> {
    p.get(key).and_then(Value::as_u64).ok_or_else(|| bad(&format!("{key} is required")))
}

fn i32_field(p: &Value, key: &str) -> Option<i32> {
    i32::try_from(p.get(key)?.as_i64()?).ok()
}

fn u8_field(p: &Value, key: &str) -> Result<u8> {
    match p.get(key).and_then(Value::as_u64) {
        None => Ok(0),
        Some(n) => u8::try_from(n).map_err(|_| bad(&format!("{key} is out of range"))),
    }
}

fn bad(msg: &str) -> EngineError {
    EngineError::BadParams { cmd: "surface.gesture".into(), msg: msg.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture_runtime::{QT_LEFT, QT_MIDDLE, QT_RIGHT};
    use cfa4_surface_model::MidiKind;

    fn session() -> Session {
        crate::surface_runtime::reset();
        reset();
        Session::default()
    }

    fn no_action(events: &[Value]) -> bool {
        events.iter().all(|event| event.get("event").and_then(Value::as_str) != Some("surfaceAction"))
    }

    fn stroke(s: &mut Session, code: u32) -> Value {
        s.execute("surface.gesture.sample", json!({"op": "press", "code": code, "x": 0, "y": 0, "tNs": 8_000_000_000u64, "from": "editor"})).unwrap();
        for (i, x) in [40, 80, 120].into_iter().enumerate() {
            s.execute("surface.gesture.sample", json!({"op": "move", "x": x, "y": 0, "tNs": 8_000_000_000u64 + 40_000_000 * (i as u64 + 1), "from": "editor"}))
                .unwrap();
        }
        s.execute("surface.gesture.sample", json!({"op": "release", "code": code, "x": 120, "y": 0, "tNs": 8_200_000_000u64, "from": "editor"})).unwrap()
    }

    #[test]
    fn inspect_starts_on_the_qt_path_and_does_not_open_evdev() {
        let mut s = session();
        let view = s.execute("surface.gesture.inspect", json!({})).unwrap();
        assert_eq!(view["path"], "qt");
        assert_eq!(view["evdev"], "not-opened");
        assert_eq!(view["evdevRequested"], false);
        assert_eq!(view["enabled"], false);
        assert_eq!(view["armed"], false);
        assert!(view["buttons"].as_array().unwrap().is_empty());
        assert!(crate::surface_runtime::with(|rt| no_action(&rt.events)));
    }

    #[test]
    fn a_mapped_stroke_is_one_surface_action_and_midi_is_unchanged() {
        let mut s = session();
        let midi = "midi:nano:1";
        let control = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 41 };
        s.prefs.control_surfaces.devices.push(DeviceSetting {
            device: midi.into(),
            bindings: vec![Binding { control: control.clone(), command: "edit.undo".into(), params: Value::Null, context: String::new() }],
            ..DeviceSetting::default()
        });
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.right", "command": "playback.toggle", "context": "global"})).unwrap();
        let reply = stroke(&mut s, QT_RIGHT);
        assert_eq!(reply["outcome"]["id"], "direction.right");
        assert_eq!(reply["injected"]["feedback"]["status"], "dispatched");
        let events = crate::surface_runtime::with(|rt| rt.take_events());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "surfaceAction");
        assert_eq!(events[0]["command"], "playback.toggle");
        assert_eq!(events[0]["device"], "pointer:seat");
        assert_eq!(s.prefs.control_surfaces.action(midi, &control).map(|a| a.command), Some("edit.undo".into()));
        let midi_in = json!({"device": midi, "input": {"type": "button", "control": {"midi": {"channel": 0, "kind": "cc", "number": 41}}, "pressed": true}});
        crate::surface_runtime::with(|rt| crate::surface_runtime::inject(rt, &s.prefs.control_surfaces, &midi_in, Instant::now())).unwrap();
        let midi_events = crate::surface_runtime::with(|rt| rt.take_events());
        assert_eq!(midi_events[0]["command"], "edit.undo");
        assert!(s.prefs.control_surfaces.devices.iter().any(|d| d.device == midi && d.bindings.len() == 1));
    }

    #[test]
    fn learn_and_test_record_buttons_and_dispatch_nothing() {
        let mut s = session();
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        s.execute("surface.gesture.learn", json!({"mode": "button", "phase": "begin"})).unwrap();
        let learned = s.execute("surface.gesture.sample", json!({"op": "press", "code": 64, "tNs": 1_000, "from": "editor"})).unwrap();
        assert_eq!(learned["button"]["role"], "learned-0");
        assert_eq!(learned["button"]["path"], "qt");
        assert!(learned.get("injected").is_none());
        s.execute("surface.gesture.sample", json!({"op": "release", "code": 64, "tNs": 2_000, "from": "editor"})).unwrap();
        s.execute("surface.gesture.learn", json!({"mode": "button", "phase": "end"})).unwrap();
        s.execute("surface.gesture.configure", json!({"buttonCode": 64, "buttonName": "thumb"})).unwrap();
        s.execute("surface.gesture.learn", json!({"mode": "test", "phase": "begin"})).unwrap();
        let press = s.execute("surface.gesture.sample", json!({"op": "press", "code": 64, "tNs": 3_000, "from": "editor"})).unwrap();
        assert_eq!(press["status"], "press");
        assert_eq!(press["button"]["name"], "thumb");
        let held = s.execute("surface.gesture.sample", json!({"op": "tick", "code": 64, "tNs": 3_000 + 400_000_000, "from": "editor"})).unwrap();
        assert_eq!(held["button"]["pressed"], true);
        let release = s.execute("surface.gesture.sample", json!({"op": "release", "code": 64, "tNs": 3_500_000_000u64, "from": "editor"})).unwrap();
        assert_eq!(release["status"], "release");
        assert!(release.get("injected").is_none());
        let buttons = release["buttons"].as_array().unwrap();
        assert!(!buttons.is_empty());
        assert!(buttons.iter().any(|button| button["code"] == 64));
        assert!(crate::surface_runtime::with(|rt| no_action(&rt.events)));
    }

    #[test]
    fn twelve_buttons_stay_a_list_and_a_combination_dispatches_once() {
        let mut s = session();
        let midi = "midi:nano:1";
        let control = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 41 };
        s.prefs.control_surfaces.devices.push(DeviceSetting {
            device: midi.into(),
            bindings: vec![Binding { control: control.clone(), command: "edit.undo".into(), params: Value::Null, context: String::new() }],
            ..DeviceSetting::default()
        });
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        for code in 0..12u32 {
            s.execute("surface.gesture.sample", json!({"op": "press", "code": 32 + code, "tNs": code, "from": "editor"})).unwrap();
        }
        let view = s.execute("surface.gesture.inspect", json!({})).unwrap();
        assert!(view["buttons"].as_array().unwrap().len() >= 12);
        s.execute("surface.gesture.combo", json!({"op": "set", "roles": [0, 1], "modifiers": 0, "command": "playback.stop", "context": "global"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_LEFT, "tNs": 10_000, "from": "editor"})).unwrap();
        let press = s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_RIGHT, "tNs": 11_000, "from": "editor"})).unwrap();
        assert!(press["combinations"].as_array().unwrap().iter().any(|combo| combo["command"] == "playback.stop" && combo["trigger"] == "chord"));
        assert_eq!(press["injected"]["feedback"]["command"], "playback.stop");
        let again = s.execute("surface.gesture.sample", json!({"op": "tick", "code": QT_RIGHT, "tNs": 12_000, "from": "editor"})).unwrap();
        assert!(again.get("injected").is_none());
        let release = s.execute("surface.gesture.sample", json!({"op": "release", "code": QT_LEFT, "tNs": 13_000, "from": "editor"})).unwrap();
        assert!(release.get("injected").is_none());
        let events = crate::surface_runtime::with(|rt| rt.take_events());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "surfaceAction");
        assert_eq!(events[0]["command"], "playback.stop");
        let midi_in = json!({"device": midi, "input": {"type": "button", "control": {"midi": {"channel": 0, "kind": "cc", "number": 41}}, "pressed": true}});
        crate::surface_runtime::with(|rt| crate::surface_runtime::inject(rt, &s.prefs.control_surfaces, &midi_in, Instant::now())).unwrap();
        let midi_events = crate::surface_runtime::with(|rt| rt.take_events());
        assert_eq!(midi_events[0]["command"], "edit.undo");
    }

    #[test]
    fn cancel_timeout_and_a_window_sample_while_disarmed_do_not_dispatch() {
        let mut s = session();
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.right", "command": "playback.toggle"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_RIGHT, "x": 0, "y": 0, "tNs": 1_000_000_000u64, "from": "editor"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "move", "x": 80, "y": 0, "tNs": 1_040_000_000u64, "from": "editor"})).unwrap();
        let cancelled = s.execute("surface.gesture.sample", json!({"op": "cancel", "from": "editor"})).unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        s.execute("surface.gesture.sample", json!({"op": "release", "code": QT_RIGHT, "tNs": 1_080_000_000u64, "from": "editor"})).unwrap();
        let ignored = s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_RIGHT, "from": "window", "tNs": 2_000_000_000u64})).unwrap();
        assert_eq!(ignored["status"], "idle");
        assert!(crate::surface_runtime::with(|rt| no_action(&rt.events)));
    }

    #[test]
    fn thresholds_hud_and_evdev_are_configurable_and_evdev_stays_closed() {
        let mut s = session();
        let view = s
            .execute(
                "surface.gesture.configure",
                json!({
                    "thresholds": {"minLengthPx": 400, "maxLengthPx": 1500, "slopPx": 4, "minMs": 30, "maxMs": 900, "angleMilliDeg": 10000, "holdMs": 200},
                    "hud": {"show": false, "reduced": true, "contrast": true},
                    "evdevRequested": true,
                    "evdevDevice": "event5",
                    "hand": "left",
                    "mirror": true,
                }),
            )
            .unwrap();
        assert_eq!(view["thresholds"]["minLengthPx"], 400);
        assert_eq!(view["hud"]["show"], false);
        assert_eq!(view["hud"]["reduced"], true);
        assert_eq!(view["path"], "qt");
        assert_eq!(view["evdev"], "not-opened");
        assert_eq!(view["evdevRequested"], true);
        assert!(view["warning"].as_str().unwrap().contains("not opened"));
        assert_eq!(view["hand"], "left");
        assert_eq!(view["mirror"], true);
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        let short = stroke(&mut s, QT_RIGHT);
        assert_ne!(short["outcome"]["id"], "direction.right");
    }

    #[test]
    fn gesture_learn_saves_a_template_and_import_export_round_trips() {
        let mut s = session();
        s.execute("surface.gesture.learn", json!({"mode": "gesture", "phase": "begin"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_RIGHT, "x": 0, "y": 0, "tNs": 1, "from": "editor"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "move", "x": 0, "y": 40, "tNs": 2, "from": "editor"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "move", "x": 40, "y": 40, "tNs": 3, "from": "editor"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "release", "code": QT_RIGHT, "tNs": 4, "from": "editor"})).unwrap();
        let saved = s.execute("surface.gesture.learn", json!({"mode": "gesture", "phase": "save", "id": "hook", "spatial": false})).unwrap();
        assert!(saved["templates"].as_array().unwrap().iter().any(|template| template["id"] == "hook"));
        assert!(crate::surface_runtime::with(|rt| no_action(&rt.events)));
        let exported = s.execute("surface.gesture.profile", json!({"op": "export"})).unwrap();
        s.execute("surface.gesture.profile", json!({"op": "delete"})).unwrap();
        let empty = s.execute("surface.gesture.inspect", json!({})).unwrap();
        assert!(empty["templates"].as_array().unwrap().is_empty());
        let imported = s.execute("surface.gesture.profile", json!({"op": "import", "document": exported["document"].clone()})).unwrap();
        assert!(imported["templates"].as_array().unwrap().iter().any(|template| template["id"] == "hook"));
    }

    #[test]
    fn a_midi_profile_is_refused_and_hostile_parameters_do_not_panic() {
        let mut s = session();
        let midi = json!({"format": "cfa4-surface-profile", "version": 1, "id": "u.nano", "name": "Nano", "driver": "generic-midi", "match": {"midiPortName": "midi:nano:1"}, "controls": []});
        assert!(s.execute("surface.gesture.profile", json!({"op": "import", "document": midi})).is_err());
        assert!(s.execute("surface.gesture.configure", json!({"thresholds": {"minLengthPx": 0}})).is_err());
        assert!(s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.right", "command": "edit.undo"})).is_err());
        assert!(s.execute("surface.gesture.sample", json!({"op": "nope"})).is_err());
        assert!(s.execute("surface.gesture.combo", json!({"op": "set", "roles": [1], "command": "playback.stop"})).is_err());
        let view = s.execute("surface.gesture.inspect", json!({})).unwrap();
        assert_eq!(view["path"], "qt");
        assert!(s.prefs.control_surfaces.devices.iter().all(|d| d.device != "midi:nano:1" || d.bindings.is_empty()));
    }

    #[test]
    fn restore_puts_the_snapshot_back() {
        let mut s = session();
        let before = s.execute("surface.gesture.inspect", json!({})).unwrap();
        s.execute("surface.gesture.configure", json!({"enabled": true, "hand": "left"})).unwrap();
        s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.up", "command": "playhead.start"})).unwrap();
        let back = s.execute("surface.gesture.restore", json!({"state": before["state"].clone()})).unwrap();
        assert_eq!(back["enabled"], false);
        assert_eq!(back["hand"], "follow");
        assert!(back["mappings"].as_array().unwrap().is_empty());
    }

    fn line(s: &mut Session, code: u32, dx: i32, widget: &str, conflict: bool, from: &str, t0: u64) -> Value {
        s.execute(
            "surface.gesture.sample",
            json!({"op": "press", "code": code, "x": 0, "y": 0, "tNs": t0, "from": from, "widget": widget, "conflict": conflict}),
        )
        .unwrap();
        for step in 1..=3 {
            let x = dx / 3 * step;
            s.execute(
                "surface.gesture.sample",
                json!({"op": "move", "x": x, "y": 0, "tNs": t0 + 40_000_000 * step as u64, "from": from, "widget": widget, "conflict": conflict}),
            )
            .unwrap();
        }
        s.execute(
            "surface.gesture.sample",
            json!({"op": "release", "code": code, "x": dx, "y": 0, "tNs": t0 + 200_000_000, "from": from, "widget": widget, "conflict": conflict}),
        )
        .unwrap()
    }

    fn action_commands(events: &[Value]) -> Vec<&str> {
        events
            .iter()
            .filter(|event| event.get("event").and_then(Value::as_str) == Some("surfaceAction"))
            .filter_map(|event| event.get("command").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn timeline_profile_follows_the_widget_and_does_not_fall_back() {
        let mut s = session();
        s.execute("surface.gesture.configure", json!({"enabled": true, "mirror": true})).unwrap();
        s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.right", "command": "playback.toggle", "context": "global"})).unwrap();
        let view = s.execute("surface.gesture.inspect", json!({})).unwrap();
        assert_eq!(view["profiles"][0]["context"], "Timeline");
        assert_eq!(view["profiles"][1]["context"], "MediaBrowser");
        assert_eq!(view["profiles"][1]["panel"], "Media Browser");
        assert!(view["profiles"][1]["deferred"].as_array().unwrap().iter().any(|row| row == "mediaBrowser.navigate"));
        assert!(view["profiles"][0]["mappings"].as_array().unwrap().iter().any(|row| row["id"] == "direction.right" && row["source"] == "builtin"));
        let timeline = line(&mut s, QT_RIGHT, 120, "timeline.canvas", false, "editor", 9_000_000_000);
        assert_eq!(timeline["activeContext"], "Timeline");
        assert_eq!(timeline["injected"]["feedback"]["command"], "playhead.nextEdit");
        let conflict = line(&mut s, QT_RIGHT, 120, "timeline.canvas", true, "editor", 10_000_000_000);
        assert!(conflict.get("injected").is_none());
        let click = s
            .execute(
                "surface.gesture.sample",
                json!({"op": "press", "code": QT_RIGHT, "x": 0, "y": 0, "tNs": 11_000_000_000u64, "widget": "timeline.canvas", "from": "editor"}),
            )
            .unwrap();
        let click = s
            .execute(
                "surface.gesture.sample",
                json!({"op": "release", "code": QT_RIGHT, "x": 0, "y": 0, "tNs": 11_050_000_000u64, "widget": "timeline.canvas", "from": "editor"}),
            )
            .unwrap_or(click);
        assert!(click.get("injected").is_none() || click["injected"]["feedback"]["status"] != "dispatched");
        s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.right", "command": "playback.stop", "context": "timeline.canvas"})).unwrap();
        let widget = line(&mut s, QT_RIGHT, 120, "timeline.canvas", false, "editor", 12_000_000_000);
        assert_eq!(widget["activeContext"], "timeline.canvas");
        assert_eq!(widget["injected"]["feedback"]["command"], "playback.stop");
        let tabs = line(&mut s, QT_RIGHT, 120, "timeline.tabs", false, "editor", 13_000_000_000);
        assert_eq!(tabs["activeContext"], "Timeline");
        assert_eq!(tabs["injected"]["feedback"]["command"], "playhead.nextEdit");
        s.execute("surface.gesture.map", json!({"op": "remove", "id": "direction.right", "context": "Timeline"})).unwrap();
        let hidden = line(&mut s, QT_RIGHT, 120, "timeline.tabs", false, "editor", 14_000_000_000);
        assert!(hidden.get("injected").is_none() || hidden["injected"]["feedback"]["status"] != "dispatched");
        let elsewhere = line(&mut s, QT_RIGHT, 120, "program.monitor", false, "editor", 15_000_000_000);
        assert_eq!(elsewhere["activeContext"], "global");
        assert_eq!(elsewhere["injected"]["feedback"]["command"], "playback.toggle");
        let events = crate::surface_runtime::with(|rt| rt.take_events());
        let commands = action_commands(&events);
        assert!(commands.contains(&"playhead.nextEdit"));
        assert!(!commands.iter().any(|command| command.starts_with("mediaBrowser")));
    }

    #[test]
    fn media_browser_context_uses_only_its_own_mapping() {
        let mut s = session();
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        s.execute("surface.gesture.map", json!({"op": "set", "id": "direction.left", "command": "playhead.stepBack", "context": "Media Browser"})).unwrap();
        let browser = line(&mut s, QT_RIGHT, -120, "mediaBrowser.list", false, "editor", 9_000_000_000);
        assert_eq!(browser["activeContext"], "MediaBrowser");
        assert_eq!(browser["injected"]["feedback"]["command"], "playhead.stepBack");
        let timeline = line(&mut s, QT_RIGHT, -120, "timeline.canvas", false, "editor", 10_000_000_000);
        assert_eq!(timeline["activeContext"], "Timeline");
        assert_eq!(timeline["injected"]["feedback"]["command"], "playhead.prevEdit");
        let armed = s.execute("surface.gesture.configure", json!({"armed": true})).unwrap();
        assert_eq!(armed["armed"], true);
        let window = line(&mut s, QT_RIGHT, -120, "mediaBrowser.tree", false, "window", 11_000_000_000);
        assert_eq!(window["injected"]["feedback"]["command"], "playhead.stepBack");
    }

    #[test]
    fn hold_and_a_three_button_chord_dispatch_once_each() {
        let mut s = session();
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        s.execute("surface.gesture.combo", json!({"op": "set", "trigger": "hold", "roles": [0], "command": "playback.stop", "context": "global"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_LEFT, "x": 4, "y": 4, "tNs": 2_000_000_000u64, "from": "editor"})).unwrap();
        let held =
            s.execute("surface.gesture.sample", json!({"op": "tick", "code": QT_LEFT, "x": 4, "y": 4, "tNs": 2_400_000_000u64, "from": "editor"})).unwrap();
        assert_eq!(held["injected"]["feedback"]["command"], "playback.stop");
        let again =
            s.execute("surface.gesture.sample", json!({"op": "tick", "code": QT_LEFT, "x": 4, "y": 4, "tNs": 2_500_000_000u64, "from": "editor"})).unwrap();
        assert!(again.get("injected").is_none());
        s.execute("surface.gesture.sample", json!({"op": "release", "code": QT_LEFT, "x": 4, "y": 4, "tNs": 2_600_000_000u64, "from": "editor"})).unwrap();
        s.execute("surface.gesture.combo", json!({"op": "set", "roles": [0, 1], "command": "playback.toggle", "context": "global"})).unwrap();
        s.execute("surface.gesture.combo", json!({"op": "set", "roles": [0, 1, 2], "command": "playhead.end", "context": "global"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_LEFT, "tNs": 3_000_000_000u64, "from": "editor"})).unwrap();
        let two = s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_RIGHT, "tNs": 3_010_000_000u64, "from": "editor"})).unwrap();
        assert_eq!(two["injected"]["feedback"]["command"], "playback.toggle");
        let three = s.execute("surface.gesture.sample", json!({"op": "press", "code": QT_MIDDLE, "tNs": 3_020_000_000u64, "from": "editor"})).unwrap();
        assert_eq!(three["injected"]["feedback"]["command"], "playhead.end");
        s.execute("surface.gesture.sample", json!({"op": "release", "code": QT_MIDDLE, "tNs": 3_030_000_000u64, "from": "editor"})).unwrap();
        let repeat = s.execute("surface.gesture.sample", json!({"op": "tick", "tNs": 3_040_000_000u64, "from": "editor"})).unwrap();
        assert!(repeat.get("injected").is_none() || repeat["injected"]["feedback"]["command"] != "playhead.end");
        let events = crate::surface_runtime::with(|rt| rt.take_events());
        let commands = action_commands(&events);
        assert_eq!(commands.iter().filter(|command| **command == "playback.stop").count(), 1);
        assert_eq!(commands.iter().filter(|command| **command == "playhead.end").count(), 1);
    }

    #[test]
    fn a_learned_side_button_runs_the_timeline_profile_and_a_chord() {
        let mut s = session();
        s.execute("surface.gesture.configure", json!({"enabled": true})).unwrap();
        s.execute("surface.gesture.learn", json!({"mode": "button", "phase": "begin"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": 8, "tNs": 1_000, "from": "editor"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": 16, "tNs": 2_000, "from": "editor"})).unwrap();
        s.execute("surface.gesture.learn", json!({"mode": "off", "phase": "end"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "release", "code": 8, "tNs": 3_000, "from": "editor"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "release", "code": 16, "tNs": 4_000, "from": "editor"})).unwrap();
        let drawn = line(&mut s, 8, 120, "timeline.canvas", false, "editor", 9_000_000_000);
        assert_eq!(drawn["injected"]["feedback"]["command"], "playhead.nextEdit");
        s.execute("surface.gesture.combo", json!({"op": "set", "roles": [3, 4], "command": "playback.stop", "context": "Timeline"})).unwrap();
        s.execute("surface.gesture.sample", json!({"op": "press", "code": 8, "tNs": 10_000_000_000u64, "widget": "timeline.canvas", "from": "editor"}))
            .unwrap();
        let chord = s
            .execute("surface.gesture.sample", json!({"op": "press", "code": 16, "tNs": 10_010_000_000u64, "widget": "timeline.canvas", "from": "editor"}))
            .unwrap();
        assert_eq!(chord["activeContext"], "Timeline");
        assert_eq!(chord["injected"]["feedback"]["command"], "playback.stop");
        s.execute("surface.gesture.sample", json!({"op": "release", "code": 16, "tNs": 10_020_000_000u64, "widget": "timeline.canvas", "from": "editor"}))
            .unwrap();
        s.execute("surface.gesture.sample", json!({"op": "release", "code": 8, "tNs": 10_030_000_000u64, "widget": "timeline.canvas", "from": "editor"}))
            .unwrap();
        let blocked = s
            .execute(
                "surface.gesture.sample",
                json!({"op": "press", "code": 8, "tNs": 11_000_000_000u64, "widget": "timeline.canvas", "conflict": true, "from": "editor"}),
            )
            .unwrap();
        let blocked = s
            .execute(
                "surface.gesture.sample",
                json!({"op": "press", "code": 16, "tNs": 11_010_000_000u64, "widget": "timeline.canvas", "conflict": true, "from": "editor"}),
            )
            .unwrap_or(blocked);
        assert!(blocked.get("injected").is_none());
    }
}
