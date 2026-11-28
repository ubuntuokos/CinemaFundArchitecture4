//! Learn, pickup, inject and the playback clock (F0 spec §§3, 7 and 10; D2, D4).
//!
//! The session thread owns this state. `surface.inject` takes the same path as a device
//! event. Learn is one global machine. Pickup is the Ardour threshold (10 / 1280) and is
//! not the Learn half-travel (64 / 8192), which stays in [`crate::control_surfaces::Triggers`].

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use web_time::Instant;

use cfa4_surface_model::midi::control_label;
use cfa4_surface_model::{ControlId, DeviceCounters, MidiKind, SurfaceInput};
use serde_json::{Value, json};

use crate::control_surfaces::{Action, ControlSurfacePrefs, Triggers, VALUE_COMMANDS};
use crate::{EngineError, Result, Session};

const CLOCK_VALID: Duration = Duration::from_millis(500);
const PICKUP_IDLE: Duration = Duration::from_millis(500);
const CAPTURE_EMIT: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, PartialEq)]
pub struct LearnTarget {
    pub command: String,
    pub params: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Learning,
    Capturing,
    Pending,
    Conflict,
}

#[derive(Clone, Debug)]
struct Sample {
    device: String,
    input: SurfaceInput,
}

#[derive(Clone, Debug, PartialEq)]
struct Captured {
    device: String,
    control: ControlId,
    mode: String,
    encoding: Option<String>,
}

#[derive(Clone, Debug)]
struct Learn {
    phase: Phase,
    /// Empty: any device.
    device: String,
    target: Option<LearnTarget>,
    first: Duration,
    silence: Duration,
    started: Instant,
    last_msg: Instant,
    last_emit: Instant,
    samples: Vec<Sample>,
    captured: Option<Captured>,
    conflict: Option<String>,
}

impl Learn {
    fn idle(now: Instant) -> Self {
        Self {
            phase: Phase::Idle,
            device: String::new(),
            target: None,
            first: Duration::from_millis(7_000),
            silence: Duration::from_millis(1_500),
            started: now,
            last_msg: now,
            last_emit: now,
            samples: Vec::new(),
            captured: None,
            conflict: None,
        }
    }

    fn active(&self) -> bool {
        matches!(self.phase, Phase::Learning | Phase::Capturing)
    }
}

#[derive(Clone, Debug)]
struct Arm {
    last: u16,
    live: bool,
    at: Instant,
}

#[derive(Clone, Debug)]
struct Clock {
    time: i64,
    playing: bool,
    speed: f64,
    monitor: String,
    at: Instant,
}

#[derive(Clone, Debug)]
struct Fault {
    failures: u32,
    not_before: Option<Instant>,
    disabled: bool,
}

/// Process-wide surface session state (one `serve` thread).
#[derive(Debug)]
pub struct Runtime {
    learn: Learn,
    triggers: Triggers,
    pickup: HashMap<(String, ControlId), Arm>,
    clock: Option<Clock>,
    seq: HashMap<String, u64>,
    counters: HashMap<String, DeviceCounters>,
    /// Runtime fault state, separate from the preference `enabled` flag.
    faults: HashMap<String, Fault>,
    kernel_dropped: u64,
    pub events: Vec<Value>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            learn: Learn::idle(Instant::now()),
            triggers: Triggers::default(),
            pickup: HashMap::new(),
            clock: None,
            seq: HashMap::new(),
            counters: HashMap::new(),
            faults: HashMap::new(),
            kernel_dropped: 0,
            events: Vec::new(),
        }
    }
}

impl Runtime {
    pub fn take_events(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }

    pub fn learn_state(&self) -> &'static str {
        match self.learn.phase {
            Phase::Idle => "idle",
            Phase::Learning => "learning",
            Phase::Capturing => "capturing",
            Phase::Pending => "pending",
            Phase::Conflict => "conflict",
        }
    }

    /// Backoff for a reader that just failed. `Some(disabled)` when the fifth fault disables it.
    pub fn note_fault(&mut self, key: &str, now: Instant) -> Duration {
        let fault = self.faults.entry(key.to_string()).or_insert(Fault { failures: 0, not_before: None, disabled: false });
        if fault.disabled {
            return Duration::from_secs(30);
        }
        fault.failures = fault.failures.saturating_add(1);
        if fault.failures >= 5 {
            fault.disabled = true;
            fault.not_before = None;
            return Duration::from_secs(30);
        }
        let shift = fault.failures.saturating_sub(1).min(6);
        let wait = Duration::from_millis((500u64 << shift).min(30_000));
        fault.not_before = Some(now + wait);
        wait
    }

    pub fn note_ok(&mut self, key: &str) {
        if let Some(fault) = self.faults.get_mut(key) {
            fault.failures = 0;
            fault.not_before = None;
        }
    }

    pub fn disabled(&self, key: &str) -> bool {
        self.faults.get(key).is_some_and(|f| f.disabled)
    }

    /// When a failed reader may try again. `None` means now.
    pub fn wait_until(&self, key: &str) -> Option<Instant> {
        self.faults.get(key).and_then(|f| f.not_before)
    }

    pub fn set_counters(&mut self, counters: HashMap<String, DeviceCounters>, kernel_dropped: u64) {
        for (id, c) in counters {
            self.counters.insert(id, c);
        }
        self.kernel_dropped = kernel_dropped;
    }

    pub fn tick(&mut self, prefs: &ControlSurfacePrefs, now: Instant) -> Vec<Value> {
        let mut out = Vec::new();
        match self.learn.phase {
            Phase::Learning if now.saturating_duration_since(self.learn.started) >= self.learn.first => {
                self.learn = Learn::idle(now);
                out.push(json!({"event": "surfaceLearn", "state": "timeout"}));
            }
            Phase::Capturing if now.saturating_duration_since(self.learn.last_msg) >= self.learn.silence => {
                out.extend(self.finish_capture(prefs, now));
            }
            _ => {}
        }
        out.extend(self.take_events());
        out
    }

    fn finish_capture(&mut self, prefs: &ControlSurfacePrefs, now: Instant) -> Vec<Value> {
        let Some(captured) = classify(&self.learn.samples) else {
            self.learn = Learn::idle(now);
            return vec![json!({"event": "surfaceLearn", "state": "timeout"})];
        };
        let existing = prefs.action(&captured.device, &captured.control).map(|a| a.command);
        let target_cmd = self.learn.target.as_ref().map(|t| t.command.clone());
        let conflict = match (existing, target_cmd) {
            (Some(cmd), Some(target)) if cmd != target => Some(cmd),
            _ => None,
        };
        self.learn.captured = Some(captured.clone());
        self.learn.samples.clear();
        let body = captured_body(&captured, self.learn.target.as_ref());
        if let Some(cmd) = conflict {
            self.learn.phase = Phase::Conflict;
            self.learn.conflict = Some(cmd.clone());
            vec![json!({"event": "surfaceLearn", "state": "conflict", "conflict": cmd, "captured": body})]
        } else {
            self.learn.phase = Phase::Pending;
            self.learn.conflict = None;
            vec![json!({"event": "surfaceLearn", "state": "pending", "captured": body})]
        }
    }

    /// One device input. `seq` is the device sequence from the lane (0 if the caller has none).
    pub fn handle(&mut self, prefs: &ControlSurfacePrefs, device: &str, input: &SurfaceInput, seq: u64, now: Instant) -> Vec<Value> {
        if !prefs.enabled || self.disabled(device) || (device.starts_with("midi:") && self.disabled("alsa")) {
            return Vec::new();
        }
        let device_off = prefs.devices.iter().find(|d| d.device == device).is_some_and(|d| !d.enabled);
        if device_off {
            return Vec::new();
        }
        if let SurfaceInput::Gesture { id, context } = input {
            // The same `surfaceAction` event MIDI and the shuttle use. Not the event fabric.
            return self.dispatch_gesture(prefs, device, id, context, seq).events;
        }
        if matches!(input, SurfaceInput::Jog { .. } | SurfaceInput::Shuttle { .. }) {
            return self.motion(prefs, device, input, seq);
        }
        if self.learn.active() && self.learn_matches(device) && learnable(input) {
            return self.observe(device, input, now);
        }
        if matches!(self.learn.phase, Phase::Pending)
            && let Some(captured) = self.learn.captured.clone()
            && captured.device == device
            && input.control().is_some_and(|c| c == &captured.control)
            && let Some(target) = self.learn.target.clone()
        {
            return self.emit_target(prefs, device, input, &target, seq, now);
        }
        let mut out = Vec::new();
        if self.learn.phase == Phase::Idle
            && let Some(control) = input.control().cloned()
            && prefs.value(device, &control, input).is_some()
        {
            if self.pickup_ok(prefs, device, &control, input, now)
                && let Some(action) = prefs.value(device, &control, input)
            {
                out.push(action_event(device, &control, action, seq));
            }
            return out;
        }
        if let Some(control) = self.triggers.press(device, input)
            && let Some(action) = prefs.action(device, &control)
        {
            out.push(action_event(device, &control, action, seq));
        }
        out
    }

    fn learn_matches(&self, device: &str) -> bool {
        self.learn.device.is_empty() || self.learn.device == device
    }

    fn observe(&mut self, device: &str, input: &SurfaceInput, now: Instant) -> Vec<Value> {
        self.learn.phase = Phase::Capturing;
        self.learn.last_msg = now;
        self.learn.samples.push(Sample { device: device.to_string(), input: input.clone() });
        if self.learn.samples.len() > 64 {
            self.learn.samples.remove(0);
        }
        let count = self.learn.samples.len();
        if count < 10 && now.saturating_duration_since(self.learn.last_emit) < CAPTURE_EMIT && count > 1 {
            return Vec::new();
        }
        self.learn.last_emit = now;
        let hint = classify(&self.learn.samples).map(|c| c.mode);
        vec![json!({"event": "surfaceLearn", "state": "capturing", "count": count, "device": device, "hint": hint})]
    }

    fn emit_target(&mut self, prefs: &ControlSurfacePrefs, device: &str, input: &SurfaceInput, target: &LearnTarget, seq: u64, now: Instant) -> Vec<Value> {
        let Some(control) = input.control().cloned() else { return Vec::new() };
        if VALUE_COMMANDS.contains(&target.command.as_str()) {
            if !self.pickup_ok(prefs, device, &control, input, now) {
                return Vec::new();
            }
            let Some(preview) = value_of(input) else { return Vec::new() };
            let mut params = if target.params.is_object() { target.params.clone() } else { json!({}) };
            if let Some(obj) = params.as_object_mut() {
                obj.insert("value".into(), json!(preview));
            }
            return vec![action_event(device, &control, Action { command: target.command.clone(), params }, seq)];
        }
        if self.triggers.press(device, input).is_some() {
            let params = if target.params.is_null() { json!({}) } else { target.params.clone() };
            return vec![action_event(device, &control, Action { command: target.command.clone(), params }, seq)];
        }
        Vec::new()
    }

    fn motion(&mut self, prefs: &ControlSurfacePrefs, device: &str, input: &SurfaceInput, seq: u64) -> Vec<Value> {
        let Some(control) = input.control().cloned() else { return Vec::new() };
        prefs.motion(device, input).into_iter().map(|a| action_event(device, &control, a, seq)).collect()
    }

    fn pickup_ok(&mut self, prefs: &ControlSurfacePrefs, device: &str, control: &ControlId, input: &SurfaceInput, now: Instant) -> bool {
        let SurfaceInput::Absolute { value, bits, .. } = input else { return true };
        let setting = prefs.device(device);
        let threshold = if *bits >= 14 { setting.pickup_threshold14 } else { setting.pickup_threshold7 };
        let key = (device.to_string(), control.clone());
        let arm = self.pickup.entry(key).or_insert(Arm { last: *value, live: true, at: now });
        if now.saturating_duration_since(arm.at) > PICKUP_IDLE {
            arm.live = false;
        }
        arm.at = now;
        if !arm.live {
            let dist = u32::from(*value).abs_diff(u32::from(arm.last));
            if dist > u32::from(threshold) {
                return false;
            }
            arm.live = true;
        }
        arm.last = *value;
        true
    }

    pub fn next_seq(&mut self, device: &str) -> u64 {
        let seq = self.seq.entry(device.to_string()).or_insert(0);
        *seq = seq.saturating_add(1);
        *seq
    }
}

fn learnable(input: &SurfaceInput) -> bool {
    !matches!(input, SurfaceInput::Raw | SurfaceInput::Jog { .. } | SurfaceInput::Shuttle { .. } | SurfaceInput::Gesture { .. })
}

fn value_of(input: &SurfaceInput) -> Option<f64> {
    match input {
        SurfaceInput::Absolute { value, bits, .. } => {
            let max = (1u32 << u32::from(*bits).clamp(1, 16)) - 1;
            Some(f64::from(*value).min(f64::from(max)) / f64::from(max))
        }
        SurfaceInput::PitchBend { value, .. } => Some(f64::from(*value) / 16383.0),
        _ => None,
    }
}

struct GestureOutcome {
    events: Vec<Value>,
    feedback: Value,
}

impl Runtime {
    /// Map one recognised gesture onto the existing assignment path. A gesture never enters MIDI
    /// Learn and never moves [`Triggers`]. Cancelled and unbound gestures emit nothing.
    fn dispatch_gesture(&self, prefs: &ControlSurfacePrefs, device: &str, id: &str, context: &str, seq: u64) -> GestureOutcome {
        let context_key = crate::control_surfaces::gesture_context_key(context);
        if !prefs.enabled || self.disabled(device) || (device.starts_with("midi:") && self.disabled("alsa")) {
            return GestureOutcome::plain("disabled", context_key, "", "control surfaces are off");
        }
        if prefs.devices.iter().find(|d| d.device == device).is_some_and(|d| !d.enabled) {
            return GestureOutcome::plain("disabled", context_key, "", "the device is off");
        }
        if !crate::control_surfaces::gesture_id_ok(id) {
            return GestureOutcome::plain("rejected", context_key, "", "gesture id is not usable");
        }
        let Some(binding) = prefs.gesture_binding(device, id, context) else {
            return GestureOutcome::plain("unbound", context_key, "", "");
        };
        if !crate::control_surfaces::gesture_command_allowed(&binding.command) {
            return GestureOutcome::plain("refused", context_key, &binding.command, "gesture commands are playback, playhead or window");
        }
        let control = ControlId::Gesture { id: id.to_string() };
        let action = Action { command: binding.command.clone(), params: binding.params.clone() };
        let event = action_event(device, &control, action, seq);
        let feedback = json!({
            "status": "dispatched",
            "command": binding.command,
            "context": context_key,
        });
        GestureOutcome { events: vec![event], feedback }
    }
}

impl GestureOutcome {
    fn plain(status: &str, context: &str, command: &str, reason: &str) -> Self {
        let mut feedback = json!({"status": status, "context": context});
        if let Some(obj) = feedback.as_object_mut() {
            if !command.is_empty() {
                obj.insert("command".into(), json!(command));
            }
            if !reason.is_empty() {
                obj.insert("reason".into(), json!(reason));
            }
        }
        Self { events: Vec::new(), feedback }
    }
}

fn action_event(device: &str, control: &ControlId, action: Action, seq: u64) -> Value {
    let params = if action.params.is_null() { json!({}) } else { action.params };
    json!({"event": "surfaceAction", "command": action.command, "params": params, "device": device, "control": control, "seq": seq})
}

fn captured_body(c: &Captured, target: Option<&LearnTarget>) -> Value {
    let mut body = json!({
        "device": c.device,
        "control": c.control,
        "label": control_label(&c.control),
        "mode": c.mode,
        "encoding": c.encoding,
    });
    if let Some(t) = target
        && let Some(obj) = body.as_object_mut()
    {
        obj.insert("target".into(), json!({"command": t.command, "params": t.params}));
    }
    body
}

fn classify(samples: &[Sample]) -> Option<Captured> {
    let last = samples.last()?;
    if let Some(cc14) = cc14(samples) {
        return Some(Captured { device: last.device.clone(), control: cc14, mode: "cc14".into(), encoding: None });
    }
    if let Some((control, encoding)) = relative(samples) {
        return Some(Captured { device: last.device.clone(), control, mode: "relative".into(), encoding: Some(encoding) });
    }
    match &last.input {
        SurfaceInput::Button { control, .. } => {
            let mode = match control {
                ControlId::Midi { kind: MidiKind::Note, .. } => "note",
                _ => "button",
            };
            Some(Captured { device: last.device.clone(), control: control.clone(), mode: mode.into(), encoding: None })
        }
        SurfaceInput::Absolute { control, .. } => Some(Captured { device: last.device.clone(), control: control.clone(), mode: "cc".into(), encoding: None }),
        // an encoder already assigned: the lane turns its CC into steps, so learning it again
        // keeps its encoding
        SurfaceInput::Relative { control, encoding, .. } => Some(Captured {
            device: last.device.clone(),
            control: control.clone(),
            mode: "relative".into(),
            encoding: serde_json::to_value(encoding).ok().and_then(|v| v.as_str().map(str::to_string)),
        }),
        SurfaceInput::ProgramChange { channel, program } => Some(Captured {
            device: last.device.clone(),
            control: ControlId::Midi { channel: *channel, kind: MidiKind::Cc, number: u16::from(*program) },
            mode: "program".into(),
            encoding: None,
        }),
        SurfaceInput::PitchBend { channel, .. } => Some(Captured {
            device: last.device.clone(),
            control: ControlId::Midi { channel: *channel, kind: MidiKind::Cc, number: 128 },
            mode: "pitchBend".into(),
            encoding: None,
        }),
        other => other.control().cloned().map(|control| Captured { device: last.device.clone(), control, mode: "button".into(), encoding: None }),
    }
}

fn cc_samples(samples: &[Sample]) -> Vec<(u16, u16, String)> {
    samples
        .iter()
        .filter_map(|s| match &s.input {
            SurfaceInput::Absolute { control: ControlId::Midi { kind: MidiKind::Cc, number, .. }, value, .. } => Some((*number, *value, s.device.clone())),
            _ => None,
        })
        .collect()
}

fn cc14(samples: &[Sample]) -> Option<ControlId> {
    let pairs = cc_samples(samples);
    let numbers: HashSet<u16> = pairs.iter().map(|(n, _, _)| *n).collect();
    let low = numbers.iter().copied().find(|n| *n <= 31 && numbers.contains(&(n + 32)))?;
    let channel = samples.iter().find_map(|s| match &s.input {
        SurfaceInput::Absolute { control: ControlId::Midi { channel, .. }, .. } => Some(*channel),
        _ => None,
    })?;
    Some(ControlId::Midi { channel, kind: MidiKind::Cc14, number: low })
}

fn relative(samples: &[Sample]) -> Option<(ControlId, String)> {
    let pairs = cc_samples(samples);
    if pairs.len() < 2 {
        return None;
    }
    let number = pairs[0].0;
    if pairs.iter().any(|(n, _, _)| *n != number) {
        return None;
    }
    let values: Vec<u16> = pairs.iter().map(|(_, v, _)| *v).collect();
    let encoderish = values.iter().all(|v| matches!(*v, 0 | 1 | 63 | 64 | 65 | 127));
    let stepped = values.iter().any(|v| matches!(*v, 1 | 63 | 65 | 127));
    if !encoderish || !stepped {
        return None;
    }
    let channel = samples.iter().find_map(|s| match &s.input {
        SurfaceInput::Absolute { control: ControlId::Midi { channel, .. }, .. } => Some(*channel),
        _ => None,
    })?;
    let encoding = if values.iter().all(|v| matches!(*v, 0 | 1 | 127)) {
        "binary"
    } else if values.iter().all(|v| (63..=65).contains(v)) {
        "offset64"
    } else if values.iter().any(|v| (65..127).contains(v)) {
        "sign-magnitude"
    } else {
        "twos-complement"
    };
    Some((ControlId::Midi { channel, kind: MidiKind::Cc, number }, encoding.into()))
}

fn durations(prefs: &ControlSurfacePrefs, p: &Value) -> (Duration, Duration) {
    let first = p.get("firstMs").and_then(Value::as_u64).unwrap_or(prefs.learn_first_ms).clamp(500, 60_000);
    let silence = p.get("silenceMs").and_then(Value::as_u64).unwrap_or(prefs.learn_silence_ms).clamp(100, 10_000);
    (Duration::from_millis(first), Duration::from_millis(silence.min(first.saturating_sub(1)).max(100)))
}

fn target_of(p: &Value) -> Option<LearnTarget> {
    let t = p.get("target")?;
    let command = t.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if command.is_empty() {
        return None;
    }
    let params = t.get("params").cloned().unwrap_or(Value::Null);
    Some(LearnTarget { command, params })
}

pub fn learn(rt: &mut Runtime, prefs: &ControlSurfacePrefs, p: &Value, now: Instant) -> Result<Value> {
    if rt.learn.phase != Phase::Idle {
        return Err(EngineError::Other("learn_busy: MIDI Learn is already running".into()));
    }
    let (first, silence) = durations(prefs, p);
    rt.learn = Learn {
        phase: Phase::Learning,
        device: p.get("device").and_then(Value::as_str).unwrap_or("").to_string(),
        target: target_of(p),
        first,
        silence,
        started: now,
        last_msg: now,
        last_emit: now,
        samples: Vec::new(),
        captured: None,
        conflict: None,
    };
    Ok(json!({"state": "learning"}))
}

pub fn learn_cancel(rt: &mut Runtime, now: Instant) -> Value {
    rt.learn = Learn::idle(now);
    json!({"state": "cancelled"})
}

pub fn learn_confirm(rt: &mut Runtime, prefs: &ControlSurfacePrefs, accept: bool, now: Instant) -> Result<Value> {
    if !matches!(rt.learn.phase, Phase::Pending | Phase::Conflict) {
        return Err(EngineError::Other("learn_idle: nothing is waiting to be confirmed".into()));
    }
    if !accept {
        rt.learn = Learn::idle(now);
        return Ok(json!({"state": "rejected"}));
    }
    let captured = rt.learn.captured.clone().ok_or_else(|| EngineError::Other("learn_idle: nothing was captured".into()))?;
    let target = rt.learn.target.clone();
    let (command, params) = match target {
        Some(t) => (t.command, t.params),
        None => (String::new(), Value::Null),
    };
    rt.learn = Learn::idle(now);
    let _ = prefs;
    Ok(json!({
        "state": "confirmed",
        "device": captured.device,
        "control": captured.control,
        "label": control_label(&captured.control),
        "mode": captured.mode,
        "encoding": captured.encoding,
        "command": command,
        "params": params,
    }))
}

pub fn inject(rt: &mut Runtime, prefs: &ControlSurfacePrefs, p: &Value, now: Instant) -> Result<Value> {
    let device = p.get("device").and_then(Value::as_str).ok_or_else(|| EngineError::Other("invalid_params: `device` is required".into()))?;
    if device.is_empty() {
        return Err(EngineError::Other("invalid_params: `device` is required".into()));
    }
    let mut input: SurfaceInput =
        serde_json::from_value(p.get("input").cloned().unwrap_or(Value::Null)).map_err(|e| EngineError::Other(format!("invalid_params: input: {e}")))?;
    if let SurfaceInput::Gesture { context, .. } = &mut input
        && context.is_empty()
        && let Some(given) = p.get("context").and_then(Value::as_str)
    {
        *context = given.to_string();
    }
    let seq = rt.next_seq(device);
    if let SurfaceInput::Gesture { id, context } = &input {
        let outcome = rt.dispatch_gesture(prefs, device, id, context, seq);
        rt.events.extend(outcome.events);
        return Ok(json!({"seq": seq, "feedback": outcome.feedback}));
    }
    let events = rt.handle(prefs, device, &input, seq, now);
    rt.events.extend(events);
    Ok(json!({"seq": seq}))
}

pub fn clock(rt: &mut Runtime, p: &Value, now: Instant) -> Result<Value> {
    let time = p.get("time").and_then(Value::as_i64).ok_or_else(|| EngineError::Other("invalid_params: `time` (ticks) is required".into()))?;
    let playing = p.get("playing").and_then(Value::as_bool).ok_or_else(|| EngineError::Other("invalid_params: `playing` is required".into()))?;
    let speed = p.get("speed").and_then(Value::as_f64).ok_or_else(|| EngineError::Other("invalid_params: `speed` is required".into()))?;
    if !speed.is_finite() || speed.abs() > 64.0 {
        return Err(EngineError::Other("invalid_params: `speed` is not a playback speed".into()));
    }
    let monitor = p.get("monitor").and_then(Value::as_str).unwrap_or("program").to_string();
    if !matches!(monitor.as_str(), "program" | "source" | "trim") {
        return Err(EngineError::Other("invalid_params: `monitor` is program, source or trim".into()));
    }
    rt.clock = Some(Clock { time, playing, speed, monitor, at: now });
    Ok(json!({}))
}

pub fn clock_valid(rt: &Runtime, now: Instant) -> bool {
    rt.clock.as_ref().is_some_and(|c| now.saturating_duration_since(c.at) < CLOCK_VALID)
}

pub fn enable(rt: &mut Runtime, prefs: &mut ControlSurfacePrefs, p: &Value) -> Result<Value> {
    let device = p.get("device").and_then(Value::as_str).unwrap_or("").to_string();
    if device.is_empty() {
        return Err(EngineError::Other("invalid_params: `device` is required".into()));
    }
    let enabled = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| EngineError::Other("invalid_params: `enabled` is required".into()))?;
    if enabled {
        rt.faults.remove(&device);
        if device.starts_with("midi:") {
            rt.faults.remove("alsa");
        }
    }
    match prefs.devices.iter_mut().find(|d| d.device == device) {
        Some(d) => d.enabled = enabled,
        None => prefs.devices.push(crate::control_surfaces::DeviceSetting { device: device.clone(), enabled, ..Default::default() }),
    }
    prefs.clamp();
    Ok(json!({"device": device, "enabled": enabled}))
}

pub fn state(rt: &Runtime, prefs: &ControlSurfacePrefs) -> Value {
    let mut devices: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    for d in &prefs.devices {
        seen.insert(d.device.clone());
        devices.push(device_state(rt, prefs, &d.device));
    }
    for id in rt.counters.keys() {
        if seen.insert(id.clone()) {
            devices.push(device_state(rt, prefs, id));
        }
    }
    let now = Instant::now();
    let learn = json!({
        "state": rt.learn_state(),
        "device": rt.learn.device,
        "target": rt.learn.target.as_ref().map(|t| json!({"command": t.command, "params": t.params})),
    });
    let clock =
        rt.clock.as_ref().filter(|_| clock_valid(rt, now)).map(|c| json!({"time": c.time, "playing": c.playing, "speed": c.speed, "monitor": c.monitor}));
    json!({"learn": learn, "devices": devices, "kernelDropped": rt.kernel_dropped, "clockValid": clock.is_some(), "clock": clock})
}

fn device_state(rt: &Runtime, prefs: &ControlSurfacePrefs, id: &str) -> Value {
    let c = rt.counters.get(id).copied().unwrap_or_default();
    let disabled = rt.disabled(id) || (id.starts_with("midi:") && rt.disabled("alsa"));
    let enabled = prefs.device(id).enabled && prefs.enabled;
    let state = if disabled {
        "disabled"
    } else if enabled {
        "online"
    } else {
        "offline"
    };
    json!({
        "device": id,
        "state": state,
        "enabled": enabled && !disabled,
        "received": c.received,
        "dispatched": c.dispatched,
        "coalesced": c.coalesced,
        "stalls": c.stalls,
        "gaps": c.gaps,
        "errors": c.errors,
    })
}

pub fn audit(s: &Session) -> Value {
    let mut rows = Vec::new();
    for d in &s.prefs.control_surfaces.devices {
        let mut seen = HashSet::new();
        for b in &d.bindings {
            if !seen.insert(b.control.clone()) {
                rows.push(json!({"device": d.device, "control": b.control, "problem": "conflict", "detail": "more than one binding"}));
            }
            if !command_known(&b.command) {
                rows.push(json!({"device": d.device, "control": b.control, "problem": "unknown_command", "detail": b.command}));
            }
        }
        for u in &d.unsupported {
            rows.push(
                json!({"device": d.device, "control": u.raw.get("control").cloned().unwrap_or(Value::Null), "problem": "unknown_command", "detail": u.reason}),
            );
        }
    }
    json!(rows)
}

fn command_known(id: &str) -> bool {
    if crate::commands::find(id).is_some() || crate::control_surfaces::VALUE_COMMANDS.contains(&id) {
        return true;
    }
    if matches!(id, "playback.stop" | "playback.shuttleSpeed" | "playback.toggle" | "playhead.stepForward" | "playhead.stepBack") {
        return true;
    }
    for table in [
        crate::shortcut_presets::PREMIERE,
        crate::shortcut_presets::PREMIERE_PANEL,
        crate::shortcut_presets::FINAL_CUT,
        crate::shortcut_presets::AVID,
        crate::shortcut_presets::CFA4_PANEL,
    ] {
        if table.iter().any(|(c, _, _)| *c == id) {
            return true;
        }
    }
    crate::shortcut_presets::QT_ONLY.contains(&id)
}

thread_local! {
    static RUNTIME: std::cell::RefCell<Runtime> = std::cell::RefCell::new(Runtime::default());
}

pub fn with<T>(f: impl FnOnce(&mut Runtime) -> T) -> T {
    RUNTIME.with(|rt| f(&mut rt.borrow_mut()))
}

pub fn cmd_learn(s: &mut Session, p: &Value) -> Result<Value> {
    let prefs = s.prefs.control_surfaces.clone();
    with(|rt| learn(rt, &prefs, p, Instant::now()))
}

pub fn cmd_learn_cancel(_s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(with(|rt| learn_cancel(rt, Instant::now())))
}

pub fn cmd_learn_confirm(s: &mut Session, p: &Value) -> Result<Value> {
    let accept = p.get("accept").and_then(Value::as_bool).ok_or_else(|| EngineError::Other("invalid_params: `accept` is required".into()))?;
    let prefs = s.prefs.control_surfaces.clone();
    with(|rt| learn_confirm(rt, &prefs, accept, Instant::now()))
}

pub fn cmd_inject(s: &mut Session, p: &Value) -> Result<Value> {
    let prefs = s.prefs.control_surfaces.clone();
    let raw = p.get("raw").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok())).collect::<Vec<_>>());
    if raw.as_deref().is_some_and(cfa4_surface_model::midi::is_timing) {
        let device = p.get("device").and_then(Value::as_str).unwrap_or("");
        let seq = with(|rt| rt.next_seq(device));
        return Ok(json!({"seq": seq}));
    }
    with(|rt| inject(rt, &prefs, p, Instant::now()))
}

pub fn cmd_clock(s: &mut Session, p: &Value) -> Result<Value> {
    let _ = s;
    with(|rt| clock(rt, p, Instant::now()))
}

pub fn cmd_enable(s: &mut Session, p: &Value) -> Result<Value> {
    let result = with(|rt| enable(rt, &mut s.prefs.control_surfaces, p))?;
    Ok(result)
}

pub fn cmd_state(s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(with(|rt| state(rt, &s.prefs.control_surfaces)))
}

pub fn cmd_audit(s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(audit(s))
}

pub fn tick_with(prefs: &ControlSurfacePrefs, now: Instant) -> Vec<Value> {
    with(|rt| rt.tick(prefs, now))
}

pub fn take_pending() -> Vec<Value> {
    with(|rt| rt.take_events())
}

/// Test and shutdown helper.
pub fn reset() {
    with(|rt| *rt = Runtime::default());
}

pub fn learning() -> bool {
    with(|rt| rt.learn_state() != "idle")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_surfaces::Binding;
    use cfa4_surface_model::MidiKind;

    fn cc(n: u16, v: u16) -> SurfaceInput {
        SurfaceInput::Absolute { control: ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: n }, value: v, bits: 7 }
    }

    fn note(pressed: bool) -> SurfaceInput {
        SurfaceInput::Button { control: ControlId::Midi { channel: 0, kind: MidiKind::Note, number: 60 }, pressed }
    }

    fn prefs_with(binding: Option<Binding>) -> ControlSurfacePrefs {
        let mut prefs = ControlSurfacePrefs::default();
        if let Some(b) = binding {
            prefs.devices.push(crate::control_surfaces::DeviceSetting { device: "midi:k:p".into(), bindings: vec![b], ..Default::default() });
        }
        prefs
    }

    #[test]
    fn learn_waits_then_confirms_and_a_clock_message_is_not_a_binding() {
        let mut rt = Runtime::default();
        let prefs = prefs_with(None);
        let t0 = Instant::now();
        assert_eq!(
            learn(&mut rt, &prefs, &json!({"device": "midi:k:p", "target": {"command": "edit.undo"}, "firstMs": 7000, "silenceMs": 1500}), t0).ok(),
            Some(json!({"state": "learning"}))
        );
        assert!(learn(&mut rt, &prefs, &json!({}), t0).is_err(), "one Learn at a time");
        // 7 s with no message
        let out = rt.tick(&prefs, t0 + Duration::from_millis(7000));
        assert_eq!(out[0]["state"], "timeout");
        assert_eq!(rt.learn_state(), "idle");

        learn(&mut rt, &prefs, &json!({"device": "midi:k:p", "target": {"command": "edit.undo"}, "firstMs": 7000, "silenceMs": 1500}), t0).ok();
        assert!(rt.handle(&prefs, "midi:k:p", &SurfaceInput::Raw, 1, t0).is_empty(), "raw is not a capture");
        let out = rt.handle(&prefs, "midi:k:p", &note(true), 2, t0 + Duration::from_millis(10));
        assert_eq!(out[0]["state"], "capturing");
        let out = rt.tick(&prefs, t0 + Duration::from_millis(10 + 1500));
        assert_eq!(out[0]["state"], "pending");
        assert_eq!(out[0]["captured"]["mode"], "note");
        let confirmed = learn_confirm(&mut rt, &prefs, true, t0).ok();
        assert_eq!(confirmed.as_ref().and_then(|v| v.get("state")).and_then(Value::as_str), Some("confirmed"));
        assert_eq!(confirmed.as_ref().and_then(|v| v.get("command")).and_then(Value::as_str), Some("edit.undo"));
        assert_eq!(learn_cancel(&mut rt, t0)["state"], "cancelled");
    }

    #[test]
    fn learn_reports_cc_cc14_and_relative_and_a_conflict() {
        let mut rt = Runtime::default();
        let t0 = Instant::now();
        let binding = Binding {
            control: ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 16 },
            command: "markers.markIn".into(),
            params: Value::Null,
            context: String::new(),
        };
        let prefs = prefs_with(Some(binding));
        learn(&mut rt, &prefs, &json!({"target": {"command": "edit.undo"}, "silenceMs": 1500, "firstMs": 7000}), t0).ok();
        let _ = rt.handle(&prefs, "midi:k:p", &cc(16, 10), 1, t0);
        let _ = rt.handle(&prefs, "midi:k:p", &cc(16, 20), 2, t0 + Duration::from_millis(5));
        let out = rt.tick(&prefs, t0 + Duration::from_millis(5 + 1500));
        assert_eq!(out[0]["captured"]["mode"], "cc", "{out:?}");
        assert_eq!(out[0]["state"], "conflict");
        assert_eq!(learn_confirm(&mut rt, &prefs, false, t0).ok().as_ref().and_then(|v| v.get("state")).and_then(Value::as_str), Some("rejected"));

        learn(&mut rt, &prefs, &json!({"target": {"command": "edit.undo"}, "silenceMs": 200, "firstMs": 7000}), t0).ok();
        let _ = rt.handle(&prefs, "d", &cc(1, 1), 1, t0);
        let _ = rt.handle(&prefs, "d", &cc(1, 127), 2, t0);
        let _ = rt.handle(&prefs, "d", &cc(33, 2), 3, t0);
        let out = rt.tick(&prefs, t0 + Duration::from_millis(200));
        assert_eq!(out[0]["captured"]["mode"], "cc14");
        learn_cancel(&mut rt, t0);

        learn(&mut rt, &prefs, &json!({"target": {"command": "edit.undo"}, "silenceMs": 200, "firstMs": 7000}), t0).ok();
        let _ = rt.handle(&prefs, "d", &cc(7, 1), 1, t0);
        let _ = rt.handle(&prefs, "d", &cc(7, 127), 2, t0);
        let out = rt.tick(&prefs, t0 + Duration::from_millis(200));
        assert_eq!(out[0]["captured"]["mode"], "relative");
        assert_eq!(out[0]["captured"]["encoding"], "binary");
        learn_cancel(&mut rt, t0);

        // an encoder already assigned arrives as steps: learning it again keeps its encoding
        learn(&mut rt, &prefs, &json!({"target": {"command": "edit.undo"}, "silenceMs": 200, "firstMs": 7000}), t0).ok();
        let step = SurfaceInput::Relative {
            control: ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 7 },
            delta: 2,
            encoding: cfa4_surface_model::RelEncoding::SignMagnitude,
        };
        let _ = rt.handle(&prefs, "d", &step, 1, t0);
        let out = rt.tick(&prefs, t0 + Duration::from_millis(200));
        assert_eq!(out[0]["captured"]["mode"], "relative");
        assert_eq!(out[0]["captured"]["encoding"], "sign-magnitude");
    }

    #[test]
    fn an_encoder_step_moves_its_value_target_without_pickup() {
        let mut rt = Runtime::default();
        let control = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 16 };
        let binding = Binding {
            control: control.clone(),
            command: "mixer.stripVolume".into(),
            params: json!({"strip": "A1", "encoding": "offset64"}),
            context: String::new(),
        };
        let prefs = prefs_with(Some(binding));
        let t0 = Instant::now();
        let step = |d| SurfaceInput::Relative { control: control.clone(), delta: d, encoding: cfa4_surface_model::RelEncoding::Offset64 };
        for (i, d) in [3, -40, 5].into_iter().enumerate() {
            let out = rt.handle(&prefs, "midi:k:p", &step(d), i as u64 + 1, t0 + Duration::from_secs(i as u64 * 5));
            assert_eq!(out.len(), 1, "every step counts, even after a pause: {out:?}");
            assert_eq!(out[0]["params"]["delta"], json!(f64::from(d) / crate::control_surfaces::ENCODER_STEPS));
        }
    }

    #[test]
    fn pickup_holds_a_jump_and_follows_a_value_inside_the_threshold() {
        let mut rt = Runtime::default();
        let control = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 0 };
        let binding = Binding { control: control.clone(), command: "mixer.stripVolume".into(), params: json!({"strip": "A1"}), context: String::new() };
        let prefs = prefs_with(Some(binding));
        let t0 = Instant::now();
        let first = rt.handle(&prefs, "midi:k:p", &cc(0, 64), 1, t0);
        assert_eq!(first.len(), 1, "the first sample engages");
        // still live: a fast sweep is not a jump
        let sweep = rt.handle(&prefs, "midi:k:p", &cc(0, 70), 2, t0 + Duration::from_millis(10));
        assert_eq!(sweep.len(), 1);
        let jumped = rt.handle(&prefs, "midi:k:p", &cc(0, 127), 3, t0 + Duration::from_millis(600));
        assert!(jumped.is_empty(), "outside the threshold after a rest");
        let back = rt.handle(&prefs, "midi:k:p", &cc(0, 68), 4, t0 + Duration::from_millis(700));
        assert_eq!(back.len(), 1, "within 10 of the last value");
        // 14-bit threshold is 1280, not 10
        let wide = SurfaceInput::Absolute { control: control.clone(), value: 0, bits: 14 };
        let _ = rt.handle(&prefs, "midi:k:p", &wide, 5, t0 + Duration::from_millis(800));
        let far = SurfaceInput::Absolute { control, value: 2000, bits: 14 };
        let held = rt.handle(&prefs, "midi:k:p", &far, 6, t0 + Duration::from_millis(1400));
        assert!(held.is_empty(), "14-bit jump beyond 1280");
    }

    #[test]
    fn inject_matches_a_press_and_five_faults_disable() {
        let mut rt = Runtime::default();
        let binding = Binding {
            control: ControlId::Midi { channel: 0, kind: MidiKind::Note, number: 60 },
            command: "edit.undo".into(),
            params: Value::Null,
            context: String::new(),
        };
        let prefs = prefs_with(Some(binding));
        let t0 = Instant::now();
        let v = inject(
            &mut rt,
            &prefs,
            &json!({"device": "midi:k:p", "input": {"type": "button", "control": {"midi": {"channel": 0, "kind": "note", "number": 60}}, "pressed": true}}),
            t0,
        )
        .ok();
        assert_eq!(v.as_ref().and_then(|x| x.get("seq")).and_then(Value::as_u64), Some(1));
        let ev = rt.take_events();
        assert_eq!(ev[0]["command"], "edit.undo");
        assert_eq!(ev[0]["seq"], 1);
        let now = Instant::now();
        for _ in 0..4 {
            let _ = rt.note_fault("alsa", now);
        }
        assert!(!rt.disabled("alsa"));
        let _ = rt.note_fault("alsa", now);
        assert!(rt.disabled("alsa"));
        let mut prefs = prefs;
        enable(&mut rt, &mut prefs, &json!({"device": "midi:k:p", "enabled": true})).ok();
        assert!(!rt.disabled("alsa"));
    }

    #[test]
    fn half_travel_still_fires_once() {
        let mut t = Triggers::default();
        assert!(t.press("d", &cc(1, 63)).is_none());
        assert!(t.press("d", &cc(1, 64)).is_some());
        let wide_id = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 2 };
        let wide = SurfaceInput::Absolute { control: wide_id, value: 8192, bits: 14 };
        assert!(t.press("d", &wide).is_some(), "14-bit half is 8192");
    }

    #[test]
    fn a_gesture_does_not_enter_midi_learn_and_the_lane_still_dispatches_it() {
        let mut rt = Runtime::default();
        let t0 = Instant::now();
        let prefs = ControlSurfacePrefs {
            devices: vec![crate::control_surfaces::DeviceSetting {
                device: "pointer:seat".into(),
                bindings: vec![Binding {
                    control: ControlId::Gesture { id: "direction.right".into() },
                    command: "playback.toggle".into(),
                    params: Value::Null,
                    context: String::new(),
                }],
                ..Default::default()
            }],
            ..ControlSurfacePrefs::default()
        };
        learn(&mut rt, &prefs, &json!({"target": {"command": "edit.undo"}, "silenceMs": 1500, "firstMs": 7000}), t0).ok();
        let global = SurfaceInput::Gesture { id: "direction.right".into(), context: String::new() };
        let out = rt.handle(&prefs, "pointer:seat", &global, 1, t0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["event"], "surfaceAction");
        assert_eq!(out[0]["command"], "playback.toggle");
        assert_eq!(out[0]["control"]["gesture"]["id"], "direction.right");
        let timeline = SurfaceInput::Gesture { id: "direction.right".into(), context: "timeline".into() };
        assert!(rt.handle(&prefs, "pointer:seat", &timeline, 2, t0).is_empty(), "timeline does not fall back to global");
        assert_eq!(rt.learn_state(), "learning", "a gesture does not start a MIDI capture");
        let note = SurfaceInput::Button { control: ControlId::Midi { channel: 0, kind: MidiKind::Note, number: 60 }, pressed: true };
        let captured = rt.handle(&prefs, "midi:k:p", &note, 3, t0 + Duration::from_millis(10));
        assert_eq!(captured[0]["state"], "capturing");
        assert!(captured.iter().all(|event| event.get("event").and_then(Value::as_str) != Some("surfaceAction")));
    }
}
