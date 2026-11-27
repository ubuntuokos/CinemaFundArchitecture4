//! Button state, recording, timeout and cancellation for pointer gestures (MG1).
//!
//! The recogniser in `cfa4-surface-model` is called from here. This module does not read a device,
//! does not open `/dev/input`, and does not call [`crate::Session::execute`]. A finished gesture
//! is a [`SurfaceInput::Gesture`] placed on the existing `surface.inject` path. A binding emits
//! one `surfaceAction` there (MG2). [`Triggers::press`] is unchanged, and nothing is published to
//! the event fabric.
//!
//! Learn lives here, not in the MIDI Learn machine. While it is on, no gesture is injected.
//!
//! [`Triggers::press`]: crate::control_surfaces::Triggers::press
//! [`SurfaceInput::Gesture`]: cfa4_surface_model::SurfaceInput::Gesture

use std::collections::HashMap;

use cfa4_surface_model::gesture::{self, GestureError, MAX_SAMPLES, PointerSample, Recognize, Template, Thresholds};
use cfa4_surface_model::{ControlId, MonoNs, SurfaceEvent, SurfaceInput, SurfaceProtocol};
use serde_json::{Value, json};
use web_time::Instant;

use crate::control_surfaces::ControlSurfacePrefs;
use crate::surface_runtime::Runtime as SurfaceRuntime;

/// Qt `MouseButton` values (`qnamespace.h`). On this path they are already the logical buttons.
pub const QT_LEFT: u32 = 0x1;
pub const QT_RIGHT: u32 = 0x2;
pub const QT_MIDDLE: u32 = 0x4;

/// Linux `input-event-codes.h`. Used only to name a code the caller already has. Nothing is read.
pub const EV_KEY: u16 = 0x01;
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

/// The only path MG1 records. Evdev reading is a later phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputPath {
    Qt,
}

/// Which physical code [`logical_role`] interprets. `Evdev` does not open a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RolePath {
    Qt,
    Evdev,
}

/// The physical code kept beside the logical role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysicalCode {
    Qt(u32),
    Evdev { ev_type: u16, code: u16 },
}

/// Logical button. The registry is a map, not a fixed array; learned roles are numbered as they
/// are learned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogicalRole {
    Primary,
    Secondary,
    Middle,
    Learned(u32),
}

impl LogicalRole {
    /// Bit position in [`PointerSample::buttons`]. Primary is 0.
    pub const fn index(self) -> u32 {
        match self {
            LogicalRole::Primary => 0,
            LogicalRole::Secondary => 1,
            LogicalRole::Middle => 2,
            LogicalRole::Learned(n) => n.saturating_add(3),
        }
    }

    /// The sample bit, when the role still fits in the `u32` bitset. Roles past bit 31 stay in
    /// the registry and in combinations; they simply have no bit.
    pub const fn bit(self) -> Option<u32> {
        let index = self.index();
        if index < 32 { Some(1u32 << index) } else { None }
    }
}

/// Map a physical code to a logical role.
///
/// On the Qt path the code is already logical (`LeftButton` is primary). `system_swaps` is
/// ignored there, so the compositor's swap is not applied a second time. On an evdev code,
/// `system_swaps` selects the role once. An unknown code is `None` (not an error).
pub fn logical_role(path: RolePath, code: &PhysicalCode, system_swaps: bool) -> Option<LogicalRole> {
    match (path, code) {
        (RolePath::Qt, PhysicalCode::Qt(button)) => match *button {
            QT_LEFT => Some(LogicalRole::Primary),
            QT_RIGHT => Some(LogicalRole::Secondary),
            QT_MIDDLE => Some(LogicalRole::Middle),
            _ => None,
        },
        (RolePath::Evdev, PhysicalCode::Evdev { ev_type, code }) if *ev_type == EV_KEY => match *code {
            BTN_LEFT => Some(if system_swaps { LogicalRole::Secondary } else { LogicalRole::Primary }),
            BTN_RIGHT => Some(if system_swaps { LogicalRole::Primary } else { LogicalRole::Secondary }),
            BTN_MIDDLE => Some(LogicalRole::Middle),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonView {
    pub code: u32,
    pub control: ControlId,
    pub role: Option<LogicalRole>,
    pub fresh: bool,
    pub pressed: bool,
}

/// Buttons that are down, as sorted role indexes plus the current modifier bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combination {
    pub roles: Vec<u32>,
    pub modifiers: u8,
}

impl Combination {
    /// Stable text form. The same set of roles is the same key whatever the press order was.
    pub fn key(&self) -> String {
        let mut key = String::new();
        for (i, role) in self.roles.iter().enumerate() {
            if i > 0 {
                key.push(',');
            }
            key.push_str(&role.to_string());
        }
        key.push('+');
        key.push_str(&self.modifiers.to_string());
        key
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Idle,
    Tracking,
    Ended,
}

#[derive(Debug)]
struct ButtonRec {
    physical: PhysicalCode,
    role: Option<LogicalRole>,
    pressed: bool,
    pressed_at_ns: Option<u64>,
    hold: bool,
}

/// Runtime state for one gesture session. Time is the `t_ns` the caller passes in.
#[derive(Debug)]
pub struct GestureRuntime {
    thresholds: Thresholds,
    enabled: bool,
    learning: bool,
    buttons: HashMap<(InputPath, ControlId), ButtonRec>,
    next_learned: u32,
    learned: Vec<ButtonView>,
    samples: Vec<PointerSample>,
    stroke_open: bool,
    stroke_started_ns: u64,
    modifiers: u8,
    last_outcome: Option<Recognize>,
    injected: u64,
    /// How many `surfaceAction` events this session has handed to the surface runtime.
    /// This module never calls [`crate::Session::execute`].
    commands: u64,
}

impl GestureRuntime {
    pub fn new(thresholds: Thresholds) -> Self {
        Self {
            thresholds,
            enabled: false,
            learning: false,
            buttons: HashMap::new(),
            next_learned: 0,
            learned: Vec::new(),
            samples: Vec::new(),
            stroke_open: false,
            stroke_started_ns: 0,
            modifiers: 0,
            last_outcome: None,
            injected: 0,
            commands: 0,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled && self.stroke_open {
            self.abort();
        }
    }

    /// Replace the thresholds. Buttons and an open stroke stay. The caller has already rejected
    /// a zero or inverted range.
    pub fn set_thresholds(&mut self, thresholds: Thresholds) {
        self.thresholds = thresholds;
    }

    pub fn thresholds(&self) -> Thresholds {
        self.thresholds
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Always 0 in MG1: no command is dispatched from here.
    pub fn commands_dispatched(&self) -> u64 {
        self.commands
    }

    pub fn injected(&self) -> u64 {
        self.injected
    }

    pub fn last_outcome(&self) -> Option<&Recognize> {
        self.last_outcome.as_ref()
    }

    pub fn learn_active(&self) -> bool {
        self.learning
    }

    pub fn learned(&self) -> &[ButtonView] {
        &self.learned
    }

    /// Start button Learn. An open stroke is dropped and does not become a gesture.
    pub fn learn_begin(&mut self) {
        self.learning = true;
        self.learned.clear();
        if self.stroke_open {
            self.abort();
        }
    }

    pub fn learn_end(&mut self) {
        self.learning = false;
    }

    /// Record a Qt button. An unknown code is stored and does not fail. During Learn no stroke
    /// starts, so nothing is injected.
    pub fn press(&mut self, code: u32, t_ns: u64) -> ButtonView {
        let control = qt_control(code);
        let key = (InputPath::Qt, control);
        let fresh = !self.buttons.contains_key(&key);
        if fresh {
            let role = logical_role(RolePath::Qt, &PhysicalCode::Qt(code), false);
            self.buttons.insert(key.clone(), ButtonRec { physical: PhysicalCode::Qt(code), role, pressed: false, pressed_at_ns: None, hold: false });
        }
        let needs_role = self.learning && self.buttons.get(&key).is_some_and(|rec| rec.role.is_none());
        if needs_role
            && let Some(role) = self.alloc_learned()
            && let Some(rec) = self.buttons.get_mut(&key)
        {
            rec.role = Some(role);
        }
        if let Some(rec) = self.buttons.get_mut(&key)
            && !rec.pressed
        {
            rec.pressed = true;
            rec.pressed_at_ns = Some(t_ns);
            rec.hold = false;
        }
        self.mark_holds(t_ns);
        let view = self.button_view(code, fresh);
        if self.learning {
            self.learned.push(view.clone());
            return view;
        }
        if self.enabled && view.role.is_some() && !self.stroke_open {
            self.open_stroke(t_ns);
        }
        view
    }

    /// Release a Qt button. The last role-button of a stroke runs recognition. Unknown codes,
    /// Learn, cancellation and timeout produce no [`SurfaceInput`].
    pub fn release(&mut self, code: u32, t_ns: u64, templates: &[Template]) -> std::result::Result<Option<SurfaceInput>, GestureError> {
        self.mark_holds(t_ns);
        let key = (InputPath::Qt, qt_control(code));
        if let Some(rec) = self.buttons.get_mut(&key) {
            rec.pressed = false;
            rec.hold = false;
            rec.pressed_at_ns = None;
        }
        if self.learning || !self.stroke_open || self.any_role_down() {
            return Ok(None);
        }
        self.finish(t_ns, templates)
    }

    /// Add a pointer sample. A stroke that runs past `max_ms`, or past [`MAX_SAMPLES`], ends as
    /// [`Recognize::Cancelled`] and is not recognised.
    pub fn move_to(&mut self, x: i32, y: i32, t_ns: u64, modifiers: u8) -> Motion {
        self.modifiers = modifiers;
        self.mark_holds(t_ns);
        if !self.stroke_open {
            return Motion::Idle;
        }
        if self.timed_out(t_ns) || self.samples.len() >= MAX_SAMPLES {
            self.abort();
            return Motion::Ended;
        }
        self.samples.push(PointerSample { t_ns, x_px: x, y_px: y, buttons: self.button_bits(), modifiers });
        Motion::Tracking
    }

    /// Drop the open stroke. A cancelled stroke does not become an input.
    pub fn cancel(&mut self) -> bool {
        if !self.stroke_open {
            return false;
        }
        self.abort();
        true
    }

    /// Synthetic release of every pressed button (disconnect or focus loss). The open stroke is
    /// dropped and is not recognised.
    pub fn release_stuck(&mut self) -> usize {
        let mut released = 0usize;
        for rec in self.buttons.values_mut() {
            if rec.pressed {
                rec.pressed = false;
                rec.hold = false;
                rec.pressed_at_ns = None;
                released += 1;
            }
        }
        if self.stroke_open {
            self.abort();
        }
        released
    }

    pub fn combination(&self) -> Combination {
        let mut roles = Vec::new();
        for rec in self.buttons.values() {
            if rec.pressed
                && let Some(role) = rec.role
            {
                roles.push(role.index());
            }
        }
        roles.sort_unstable();
        roles.dedup();
        Combination { roles, modifiers: self.modifiers }
    }

    /// Known buttons, sorted by Qt code. Order does not follow the map's hash order.
    pub fn buttons(&self) -> Vec<ButtonView> {
        let mut views = Vec::new();
        for ((path, control), rec) in &self.buttons {
            if *path != InputPath::Qt {
                continue;
            }
            let PhysicalCode::Qt(code) = rec.physical else { continue };
            views.push(ButtonView { code, control: control.clone(), role: rec.role, fresh: false, pressed: rec.pressed });
        }
        views.sort_by_key(|view| view.code);
        views
    }

    pub fn is_pressed(&self, code: u32) -> bool {
        self.buttons.get(&(InputPath::Qt, qt_control(code))).is_some_and(|rec| rec.pressed)
    }

    pub fn hold_count(&self) -> usize {
        self.buttons.values().filter(|rec| rec.hold).count()
    }

    /// JSON body `surface.inject` accepts for this gesture. The device is [`gesture::POINTER_SEAT`].
    pub fn inject_body(input: &SurfaceInput) -> Value {
        json!({ "device": gesture::pointer_seat().to_string(), "input": input })
    }

    /// Put `input` on the existing `surface.inject` path, in the global context. Learn refuses
    /// the call. `now` is only what `surface.inject` already requires; gesture timing uses the
    /// sample timestamps.
    pub fn deliver(&mut self, surfaces: &mut SurfaceRuntime, prefs: &ControlSurfacePrefs, input: &SurfaceInput, now: Instant) -> crate::Result<Value> {
        self.deliver_in(surfaces, prefs, input, "global", now)
    }

    /// [`deliver`] in one context. A narrower context does not fall back to global.
    pub fn deliver_in(
        &mut self,
        surfaces: &mut SurfaceRuntime,
        prefs: &ControlSurfacePrefs,
        input: &SurfaceInput,
        context: &str,
        now: Instant,
    ) -> crate::Result<Value> {
        if self.learning {
            return Ok(json!({ "injected": false }));
        }
        let input = with_context(input, context);
        self.injected = self.injected.saturating_add(1);
        let value = crate::surface_runtime::inject(surfaces, prefs, &Self::inject_body(&input), now)?;
        if value.pointer("/feedback/status").and_then(Value::as_str) == Some("dispatched") {
            self.commands = self.commands.saturating_add(1);
        }
        Ok(value)
    }

    fn finish(&mut self, t_ns: u64, templates: &[Template]) -> std::result::Result<Option<SurfaceInput>, GestureError> {
        if self.timed_out(t_ns) {
            self.abort();
            return Ok(None);
        }
        let samples = std::mem::take(&mut self.samples);
        self.stroke_open = false;
        let outcome = gesture::recognize(&samples, templates, &self.thresholds)?;
        let input = input_of(&outcome);
        self.last_outcome = Some(outcome);
        Ok(input)
    }

    fn open_stroke(&mut self, t_ns: u64) {
        self.stroke_open = true;
        self.stroke_started_ns = t_ns;
        self.samples.clear();
        self.last_outcome = None;
    }

    fn abort(&mut self) {
        self.samples.clear();
        self.stroke_open = false;
        self.last_outcome = Some(Recognize::Cancelled);
    }

    fn timed_out(&self, t_ns: u64) -> bool {
        t_ns.saturating_sub(self.stroke_started_ns) > self.thresholds.max_ms.saturating_mul(1_000_000)
    }

    fn mark_holds(&mut self, t_ns: u64) {
        let hold_ns = self.thresholds.hold_ms.saturating_mul(1_000_000);
        for rec in self.buttons.values_mut() {
            if rec.pressed && rec.pressed_at_ns.is_some_and(|at| t_ns.saturating_sub(at) >= hold_ns) {
                rec.hold = true;
            }
        }
    }

    fn any_role_down(&self) -> bool {
        self.buttons.values().any(|rec| rec.pressed && rec.role.is_some())
    }

    fn button_bits(&self) -> u32 {
        let mut bits = 0u32;
        for rec in self.buttons.values() {
            if rec.pressed
                && let Some(bit) = rec.role.and_then(LogicalRole::bit)
            {
                bits |= bit;
            }
        }
        bits
    }

    fn alloc_learned(&mut self) -> Option<LogicalRole> {
        if self.next_learned == u32::MAX {
            return None;
        }
        let n = self.next_learned;
        self.next_learned = self.next_learned.saturating_add(1);
        Some(LogicalRole::Learned(n))
    }

    fn button_view(&self, code: u32, fresh: bool) -> ButtonView {
        let control = qt_control(code);
        let rec = self.buttons.get(&(InputPath::Qt, control.clone()));
        ButtonView { code, control, role: rec.map(|rec| rec.role).unwrap_or(None), fresh, pressed: rec.is_some_and(|rec| rec.pressed) }
    }
}

fn input_of(outcome: &Recognize) -> Option<SurfaceInput> {
    let id = match outcome {
        Recognize::Direction(direction) => direction.as_id().to_string(),
        Recognize::Match { id, .. } => id.clone(),
        Recognize::None | Recognize::Cancelled => return None,
    };
    Some(SurfaceInput::Gesture { id, context: String::new() })
}

fn with_context(input: &SurfaceInput, context: &str) -> SurfaceInput {
    match input {
        SurfaceInput::Gesture { id, context: existing } if existing.is_empty() => SurfaceInput::Gesture { id: id.clone(), context: context.to_string() },
        other => other.clone(),
    }
}

/// Templates stored on the pointer profile. Ergonomic templates are mirrored only when the
/// profile asks for it. Spatial templates keep their direction.
pub fn templates_from_profile(prefs: &ControlSurfacePrefs) -> Vec<Template> {
    let device = prefs.device(gesture::POINTER_SEAT);
    device
        .gestures
        .iter()
        .filter(|template| !template.points.is_empty() && template.points.len() <= gesture::MAX_TEMPLATE_POINTS)
        .map(|template| {
            let built = Template { id: template.id.clone(), points: template.points.clone(), spatial: template.spatial };
            if device.mirror_ergonomic { gesture::mirror_template(&built) } else { built }
        })
        .collect()
}

/// The pointer as a [`SurfaceEvent`]. No device is opened. The recognised gesture is the input.
pub struct PointerAdapter;

impl PointerAdapter {
    pub fn input(id: &str, context: &str) -> SurfaceInput {
        SurfaceInput::Gesture { id: id.to_string(), context: context.to_string() }
    }

    pub fn event(id: &str, context: &str, seq: u64, at: MonoNs) -> SurfaceEvent {
        SurfaceEvent { device: gesture::pointer_seat(), at, protocol: SurfaceProtocol::Pointer, input: Self::input(id, context), seq, raw: Vec::new() }
    }

    /// Recognise nothing here: `id` is already a gesture. Dispatch goes through `surface.inject`.
    pub fn submit(
        runtime: &mut GestureRuntime,
        surfaces: &mut SurfaceRuntime,
        prefs: &ControlSurfacePrefs,
        id: &str,
        context: &str,
        now: Instant,
    ) -> crate::Result<Value> {
        runtime.deliver_in(surfaces, prefs, &Self::input(id, context), context, now)
    }
}

fn qt_control(code: u32) -> ControlId {
    ControlId::Hid { name: format!("qt-button-{code}"), usage_page: 0, usage: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_surface_model::gesture::{self, Direction, Recognize, Template};

    fn thresholds() -> Thresholds {
        Thresholds { min_length_px: 40, max_length_px: 2_000, slop_px: 8, min_ms: 20, max_ms: 1_500, angle_milli_deg: 20_000, hold_ms: 300 }
    }

    fn runtime() -> GestureRuntime {
        GestureRuntime::new(thresholds())
    }

    fn hook() -> Template {
        Template { id: "hook".into(), points: vec![(0, 0), (0, 100), (100, 100)], spatial: false }
    }

    fn no_action(events: &[Value]) -> bool {
        events.iter().all(|event| event.get("event").and_then(Value::as_str) != Some("surfaceAction"))
    }

    /// AC-07: cancel and timeout produce no input and no surfaceAction.
    #[test]
    fn ac07_cancel_and_timeout_do_not_dispatch() {
        let mut rt = runtime();
        rt.set_enabled(true);
        let surfaces = SurfaceRuntime::default();
        let prefs = ControlSurfacePrefs::default();
        let now = Instant::now();

        rt.press(QT_RIGHT, 1_000_000_000);
        assert_eq!(rt.move_to(10, 0, 1_030_000_000, 0), Motion::Tracking);
        assert_eq!(rt.move_to(40, 0, 1_060_000_000, 0), Motion::Tracking);
        assert!(rt.cancel());
        assert_eq!(rt.last_outcome(), Some(&Recognize::Cancelled));
        assert!(rt.release(QT_RIGHT, 1_090_000_000, &[]).unwrap().is_none());
        assert_eq!(rt.commands_dispatched(), 0);
        assert_eq!(rt.injected(), 0);

        rt.press(QT_RIGHT, 2_000_000_000);
        assert_eq!(rt.move_to(20, 0, 2_040_000_000, 0), Motion::Tracking);
        assert_eq!(rt.move_to(120, 0, 2_000_000_000 + 2_000_000_000, 0), Motion::Ended);
        assert_eq!(rt.last_outcome(), Some(&Recognize::Cancelled));
        assert!(rt.release(QT_RIGHT, 4_100_000_000, &[]).unwrap().is_none());
        assert_eq!(rt.injected(), 0);
        assert_eq!(rt.commands_dispatched(), 0);
        assert!(surfaces.events.is_empty());
        assert!(no_action(&surfaces.events));
        let _ = (prefs, now);
    }

    /// AC-08: the registry grows with the codes it is given. There is no fixed button array.
    #[test]
    fn ac08_button_count_is_not_fixed() {
        let mut rt = runtime();
        let mut codes: Vec<u32> = (0..48).map(|i| 0x20 + i).collect();
        codes.extend([QT_LEFT, QT_RIGHT, QT_MIDDLE, 0x0800_0000, u32::MAX, 0xA5A5_A5A5]);
        for (i, code) in codes.iter().copied().enumerate() {
            let view = rt.press(code, i as u64);
            assert!(view.pressed, "{code}");
            assert!(rt.is_pressed(code));
        }
        assert_eq!(rt.buttons().len(), codes.len());
        let again = rt.buttons().len();
        rt.press(codes[0], 10_000);
        assert_eq!(rt.buttons().len(), again, "a second press does not grow a slot");
        for n in [3usize, 5, 8, 12] {
            assert!(rt.buttons().len() >= n);
        }
        assert!(rt.buttons().iter().any(|button| button.code == u32::MAX));
        assert!(rt.buttons().iter().any(|button| button.code == 0xA5A5_A5A5 && button.role.is_none()));
        assert_eq!(logical_role(RolePath::Evdev, &PhysicalCode::Evdev { ev_type: EV_KEY, code: gesture::KEY_MAX }, false), None);
        assert_eq!(rt.commands_dispatched(), 0);
        assert_eq!(rt.injected(), 0);

        rt.learn_begin();
        for i in 0..30 {
            rt.press(0x200 + i, 20_000 + u64::from(i));
        }
        rt.learn_end();
        let wide = rt.buttons().iter().find(|button| button.code == 0x200 + 29).cloned();
        let wide = wide.expect("30th learned button");
        assert_eq!(wide.role.map(LogicalRole::index), Some(32), "a role past the u32 bitset is still stored");
        assert!(wide.role.is_some_and(|role| role.bit().is_none()));
    }

    /// AC-09: Learn records buttons and runs no command.
    #[test]
    fn ac09_learn_dispatches_nothing() {
        let mut rt = runtime();
        rt.set_enabled(true);
        let mut surfaces = SurfaceRuntime::default();
        let prefs = ControlSurfacePrefs::default();
        rt.learn_begin();
        assert!(rt.learn_active());
        let left = rt.press(QT_LEFT, 0);
        let extra = rt.press(0x40, 1_000);
        assert_eq!(left.role, Some(LogicalRole::Primary));
        assert_eq!(extra.role, Some(LogicalRole::Learned(0)));
        assert_eq!(rt.move_to(10, 10, 30_000_000, 0), Motion::Idle);
        assert!(rt.release(QT_LEFT, 40_000_000, &[hook()]).unwrap().is_none());
        assert!(rt.release(0x40, 50_000_000, &[hook()]).unwrap().is_none());
        let refused =
            rt.deliver(&mut surfaces, &prefs, &SurfaceInput::Gesture { id: "direction.right".into(), context: String::new() }, Instant::now()).unwrap();
        assert_eq!(refused["injected"], false);
        rt.learn_end();
        assert_eq!(rt.learned().len(), 2);
        assert_eq!(rt.injected(), 0);
        assert_eq!(rt.commands_dispatched(), 0);
        assert!(surfaces.events.is_empty());
        assert!(no_action(&surfaces.events));
    }

    /// AC-10: two buttons and three or more resolve to one key, independent of press order.
    #[test]
    fn ac10_combinations_are_deterministic() {
        let mut rt = runtime();
        rt.press(QT_LEFT, 0);
        rt.press(QT_RIGHT, 1);
        assert_eq!(rt.combination().key(), "0,1+0");
        rt.release_stuck();
        rt.press(QT_RIGHT, 2);
        rt.press(QT_LEFT, 3);
        assert_eq!(rt.combination().key(), "0,1+0");
        rt.press(QT_MIDDLE, 4);
        assert_eq!(rt.combination().key(), "0,1,2+0");
        rt.learn_begin();
        rt.press(0x40, 5);
        rt.learn_end();
        assert_eq!(rt.combination().key(), "0,1,2,3+0");
        rt.release_stuck();
        for code in [QT_MIDDLE, 0x40u32, QT_LEFT, QT_RIGHT] {
            rt.press(code, 10);
        }
        assert_eq!(rt.combination().key(), "0,1,2,3+0");
        rt.move_to(0, 0, 11, gesture::MOD_SHIFT);
        assert_eq!(rt.combination().key(), "0,1,2,3+2");
        rt.move_to(0, 0, 12, 0);
        assert_eq!(rt.combination().key(), "0,1,2,3+0");
        assert_eq!(rt.commands_dispatched(), 0);
        assert_eq!(rt.injected(), 0);
    }

    /// AC-12: an unknown code is stored, does not fail, and does not start a command.
    #[test]
    fn ac12_unknown_button_is_stored_and_does_not_dispatch() {
        let mut rt = runtime();
        rt.set_enabled(true);
        let code = 0x00AB_CDEF;
        let view = rt.press(code, 0);
        assert!(view.fresh);
        assert!(view.role.is_none());
        assert!(view.pressed);
        assert!(rt.buttons().iter().any(|button| button.code == code));
        assert!(rt.combination().roles.is_empty());
        assert_eq!(rt.move_to(30, 0, 40_000_000, 0), Motion::Idle);
        assert!(rt.release(code, 80_000_000, &[]).unwrap().is_none());
        assert!(rt.release(0x1111_2222, 90_000_000, &[]).unwrap().is_none(), "a code never seen is not an error");
        assert_eq!(rt.injected(), 0);
        assert_eq!(rt.commands_dispatched(), 0);
    }

    /// AC-14: disconnect and focus loss synthesize a release for every stuck button.
    #[test]
    fn ac14_stuck_buttons_are_released_without_a_command() {
        let mut rt = runtime();
        rt.set_enabled(true);
        for (cause, t0) in [("disconnect", 0u64), ("focus", 1_000_000_000u64)] {
            rt.press(QT_LEFT, t0);
            rt.press(QT_RIGHT, t0 + 1);
            rt.press(0x55, t0 + 2);
            assert_eq!(rt.move_to(80, 0, t0 + 50_000_000, 0), Motion::Tracking);
            let released = rt.release_stuck();
            assert_eq!(released, 3, "{cause}");
            assert!(!rt.is_pressed(QT_LEFT) && !rt.is_pressed(QT_RIGHT) && !rt.is_pressed(0x55));
            assert_eq!(rt.last_outcome(), Some(&Recognize::Cancelled));
            assert_eq!(rt.injected(), 0, "{cause}");
            assert_eq!(rt.commands_dispatched(), 0, "{cause}");
        }
    }

    /// AC-18: the four system-swap × path cases give one logical role, with no second swap and no
    /// missed swap. A mirrored ergonomic stroke keeps the gesture id; a spatial direction does not flip.
    #[test]
    fn ac18_logical_roles_and_mirrored_ids_match() {
        let mut double = 0;
        let mut missed = 0;
        let mut seen = 0;
        for system_swaps in [false, true] {
            for path in [RolePath::Qt, RolePath::Evdev] {
                for physical_left in [true, false] {
                    let delivered = delivered(path, system_swaps, physical_left);
                    let actual = logical_role(path, &delivered, system_swaps);
                    let expected = expected_role(system_swaps, physical_left);
                    if actual != Some(expected) && path == RolePath::Qt && system_swaps {
                        double += 1;
                    }
                    if actual != Some(expected) && path == RolePath::Evdev && system_swaps {
                        missed += 1;
                    }
                    assert_eq!(actual, Some(expected), "{path:?} swaps={system_swaps} left={physical_left}");
                    seen += 1;
                }
            }
        }
        assert_eq!(seen, 8);
        assert_eq!(double, 0);
        assert_eq!(missed, 0);
        assert_eq!(logical_role(RolePath::Qt, &PhysicalCode::Qt(QT_LEFT), true), Some(LogicalRole::Primary));
        assert_eq!(logical_role(RolePath::Qt, &PhysicalCode::Qt(QT_MIDDLE), true), Some(LogicalRole::Middle));
        assert_eq!(logical_role(RolePath::Evdev, &PhysicalCode::Evdev { ev_type: EV_KEY, code: BTN_MIDDLE }, true), Some(LogicalRole::Middle));
        assert_eq!(logical_role(RolePath::Evdev, &PhysicalCode::Evdev { ev_type: EV_KEY, code: BTN_LEFT }, true), Some(LogicalRole::Secondary));

        let mut right_hand = runtime();
        let mut left_hand = runtime();
        right_hand.set_enabled(true);
        left_hand.set_enabled(true);
        let original = hook();
        let mirrored = gesture::mirror_template(&original);
        assert_eq!(gesture::mirror_template(&Template { id: "time".into(), points: vec![(0, 0), (100, 0)], spatial: true }).points, vec![(0, 0), (100, 0)]);
        let right_line = [(0, 0), (30, 0), (60, 0), (90, 0), (120, 0)];
        let ids_right = ids_of(&mut right_hand, &right_line, &original.points, &original);
        let ids_left = ids_of(&mut left_hand, &right_line, &mirrored.points, &mirrored);
        assert_eq!(ids_right, ids_left);
        assert_eq!(ids_right.first().map(String::as_str), Some(Direction::Right.as_id()));
        assert_eq!(ids_right.get(1).map(String::as_str), Some("hook"));

        let mut surfaces_right = SurfaceRuntime::default();
        let mut surfaces_left = SurfaceRuntime::default();
        let prefs = ControlSurfacePrefs::default();
        let now = Instant::now();
        for id in &ids_right {
            right_hand.deliver(&mut surfaces_right, &prefs, &SurfaceInput::Gesture { id: id.clone(), context: String::new() }, now).unwrap();
        }
        for id in &ids_left {
            left_hand.deliver(&mut surfaces_left, &prefs, &SurfaceInput::Gesture { id: id.clone(), context: String::new() }, now).unwrap();
        }
        assert!(no_action(&surfaces_right.events));
        assert!(no_action(&surfaces_left.events));
        assert_eq!(surfaces_right.events.iter().filter(|event| event.get("event").and_then(Value::as_str) == Some("surfaceAction")).count(), 0);
        assert_eq!(surfaces_left.events.iter().filter(|event| event.get("event").and_then(Value::as_str) == Some("surfaceAction")).count(), 0);
        assert_eq!(right_hand.commands_dispatched(), 0);
        assert_eq!(left_hand.commands_dispatched(), 0);
    }

    #[test]
    fn a_recognised_gesture_is_injected_and_is_not_a_surface_action() {
        let mut rt = runtime();
        rt.set_enabled(true);
        let mut surfaces = SurfaceRuntime::default();
        let prefs = ControlSurfacePrefs::default();
        let input = stroke(&mut rt, &[(0, 0), (40, 0), (80, 0), (120, 0)], &[hook()]).expect("direction");
        assert_eq!(input, SurfaceInput::Gesture { id: Direction::Right.as_id().into(), context: String::new() });
        let body = GestureRuntime::inject_body(&input);
        assert_eq!(body["device"], "pointer:seat");
        assert_eq!(body["input"]["type"], "gesture");
        assert_eq!(body["input"]["id"], "direction.right");
        let first = rt.deliver(&mut surfaces, &prefs, &input, Instant::now()).unwrap();
        let second = rt.deliver(&mut surfaces, &prefs, &input, Instant::now()).unwrap();
        assert_eq!(first["seq"], 1);
        assert_eq!(second["seq"], 2);
        assert!(no_action(&surfaces.events));
        assert_eq!(rt.injected(), 2);
        assert_eq!(rt.commands_dispatched(), 0);
        assert_eq!(first["feedback"]["status"], "unbound");
    }

    /// AC-02: a bound gesture is one surfaceAction on the common path, and nowhere else.
    #[test]
    fn ac02_a_bound_gesture_is_a_surface_action() {
        let mut rt = runtime();
        rt.set_enabled(true);
        let mut surfaces = SurfaceRuntime::default();
        let hook = crate::control_surfaces::StoredGesture { id: "hook".into(), points: vec![(0, 0), (0, 40), (40, 40)], spatial: false };
        let mut prefs = ControlSurfacePrefs {
            devices: vec![crate::control_surfaces::DeviceSetting {
                device: gesture::POINTER_SEAT.into(),
                bindings: vec![
                    crate::control_surfaces::Binding {
                        control: cfa4_surface_model::ControlId::Gesture { id: "direction.right".into() },
                        command: "playback.toggle".into(),
                        params: serde_json::json!({}),
                        context: String::new(),
                    },
                    crate::control_surfaces::Binding {
                        control: cfa4_surface_model::ControlId::Gesture { id: "direction.right".into() },
                        command: "playhead.start".into(),
                        params: serde_json::Value::Null,
                        context: "timeline".into(),
                    },
                    crate::control_surfaces::Binding {
                        control: cfa4_surface_model::ControlId::Gesture { id: "direction.left".into() },
                        command: "edit.undo".into(),
                        params: serde_json::Value::Null,
                        context: String::new(),
                    },
                ],
                gestures: vec![hook.clone()],
                mirror_ergonomic: true,
                ..crate::control_surfaces::DeviceSetting::default()
            }],
            ..ControlSurfacePrefs::default()
        };
        let templates = templates_from_profile(&prefs);
        assert_eq!(templates.len(), 1);
        assert_ne!(templates[0].points, hook.points, "an ergonomic template is mirrored");
        assert!(
            templates_from_profile(&{
                prefs.devices[0].mirror_ergonomic = false;
                prefs.clone()
            })[0]
                .points
                == hook.points
        );

        let event = PointerAdapter::event("direction.right", "global", 1, cfa4_surface_model::MonoNs(0));
        assert_eq!(event.protocol, cfa4_surface_model::SurfaceProtocol::Pointer);
        assert_eq!(event.device.0, "pointer:seat");
        assert!(event.raw.is_empty());
        assert!(matches!(event.input, SurfaceInput::Gesture { .. }));

        let now = Instant::now();
        let global = PointerAdapter::submit(&mut rt, &mut surfaces, &prefs, "direction.right", "global", now).unwrap();
        assert_eq!(global["feedback"]["status"], "dispatched");
        assert_eq!(global["feedback"]["command"], "playback.toggle");
        assert_eq!(surfaces.events.len(), 1);
        assert_eq!(surfaces.events[0]["event"], "surfaceAction");
        assert_eq!(surfaces.events[0]["command"], "playback.toggle");
        assert_eq!(surfaces.events[0]["device"], "pointer:seat");
        assert_eq!(surfaces.events[0]["control"]["gesture"]["id"], "direction.right");
        assert_eq!(rt.commands_dispatched(), 1);

        let narrower =
            rt.deliver_in(&mut surfaces, &prefs, &SurfaceInput::Gesture { id: "direction.up".into(), context: String::new() }, "timeline", now).unwrap();
        assert_eq!(narrower["feedback"]["status"], "unbound");
        assert_eq!(surfaces.events.len(), 1, "a timeline stroke does not use the global binding");

        let timeline = PointerAdapter::submit(&mut rt, &mut surfaces, &prefs, "direction.right", "timeline", now).unwrap();
        assert_eq!(timeline["feedback"]["command"], "playhead.start");
        assert_eq!(surfaces.events[1]["command"], "playhead.start");

        let refused = PointerAdapter::submit(&mut rt, &mut surfaces, &prefs, "direction.left", "global", now).unwrap();
        assert_eq!(refused["feedback"]["status"], "refused");
        assert_eq!(surfaces.events.len(), 2, "a command outside playback, playhead and window does not run");
        assert!(surfaces.events.iter().all(|event| event["event"] == "surfaceAction"));
        assert_eq!(rt.commands_dispatched(), 2);

        prefs.enabled = false;
        let off = PointerAdapter::submit(&mut rt, &mut surfaces, &prefs, "direction.right", "global", now).unwrap();
        assert_eq!(off["feedback"]["status"], "disabled");
        assert_eq!(surfaces.events.len(), 2);
    }

    #[test]
    fn a_hold_inside_slop_sets_the_flag_and_does_not_dispatch() {
        let mut rt = runtime();
        rt.set_enabled(true);
        rt.press(QT_RIGHT, 0);
        assert_eq!(rt.hold_count(), 0);
        assert_eq!(rt.move_to(2, 1, 400_000_000, 0), Motion::Tracking);
        assert_eq!(rt.hold_count(), 1);
        assert!(rt.release(QT_RIGHT, 420_000_000, &[]).unwrap().is_none());
        assert_eq!(rt.injected(), 0);
        assert_eq!(rt.commands_dispatched(), 0);
    }

    fn delivered(path: RolePath, system_swaps: bool, physical_left: bool) -> PhysicalCode {
        match path {
            RolePath::Qt => {
                let code = if physical_left == !system_swaps { QT_LEFT } else { QT_RIGHT };
                PhysicalCode::Qt(code)
            }
            RolePath::Evdev => PhysicalCode::Evdev { ev_type: EV_KEY, code: if physical_left { BTN_LEFT } else { BTN_RIGHT } },
        }
    }

    fn expected_role(system_swaps: bool, physical_left: bool) -> LogicalRole {
        if physical_left ^ system_swaps { LogicalRole::Primary } else { LogicalRole::Secondary }
    }

    fn ids_of(rt: &mut GestureRuntime, line: &[(i32, i32)], shape: &[(i32, i32)], template: &Template) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(SurfaceInput::Gesture { id, .. }) = stroke(rt, line, std::slice::from_ref(template)) {
            ids.push(id);
        }
        if let Some(SurfaceInput::Gesture { id, .. }) = stroke(rt, shape, std::slice::from_ref(template)) {
            ids.push(id);
        }
        ids
    }

    fn stroke(rt: &mut GestureRuntime, points: &[(i32, i32)], templates: &[Template]) -> Option<SurfaceInput> {
        let t0 = 8_000_000_000u64;
        rt.press(QT_RIGHT, t0);
        for (i, (x, y)) in points.iter().enumerate() {
            rt.move_to(*x, *y, t0 + 40_000_000 * (i as u64 + 1), 0);
        }
        rt.release(QT_RIGHT, t0 + 40_000_000 * (points.len() as u64 + 2), templates).unwrap()
    }
}
