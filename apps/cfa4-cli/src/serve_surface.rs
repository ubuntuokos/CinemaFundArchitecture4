//! Control surfaces on `cfa4-cli serve` (CFA4-CONTROL-SURFACE-001, ADR 0012).
//!
//! L5 (`cfa4-surface`) reads MIDI and hidraw. This module empties those lanes on the session
//! thread and asks the engine (`surface_runtime`) to map them. `surface.inject` uses that same
//! mapping. The MIDI Monitor is display only: clock, active sensing and MTC are left out unless
//! the filter asks (F0 spec §5).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use cfa4_engine::Session;
use cfa4_engine::control_surfaces::{ControlSurfacePrefs, PROFILES};
use cfa4_engine::surface_runtime;
use cfa4_surface::hid_in::HidReader;
use cfa4_surface::hid_watch::DevWatch;
use cfa4_surface::lane::Queued;
use cfa4_surface::midi_in::{MidiMessage, MidiReader};
use cfa4_surface_model::midi::{self, Chatter};
use serde_json::{Value, json};

/// Most messages in one `surfaceMonitor` event.
pub const BATCH: usize = 200;

/// Shortest time between two `surfaceMonitor` events (20 Hz).
const EVERY: Duration = Duration::from_millis(50);

/// How often jog/shuttle devices are reconciled when inotify is quiet (D5: the scan is the net).
const HID_SCAN: Duration = Duration::from_secs(2);

/// A failure the Qt client can show. `code` is the protocol contract's code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceError {
    pub message: String,
    pub code: &'static str,
}

impl SurfaceError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { message: message.into(), code }
    }
}

fn engine_err(e: cfa4_engine::EngineError) -> SurfaceError {
    let message = e.to_string();
    let code = if message.starts_with("learn_busy") {
        "learn_busy"
    } else if message.starts_with("learn_idle") {
        "learn_idle"
    } else if message.starts_with("invalid_params") {
        "invalid_params"
    } else if message.contains("ALSA") {
        "alsa_unavailable"
    } else {
        "internal"
    };
    SurfaceError { message, code }
}

/// What the monitor shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only this device (empty = all).
    pub device: String,
    pub clock: bool,
    pub sense: bool,
    /// MIDI timecode (quarter-frame and full-frame). Off in the default view.
    pub mtc: bool,
}

impl Filter {
    fn keeps(&self, m: &MidiMessage) -> bool {
        if !self.device.is_empty() && m.device.0 != self.device {
            return false;
        }
        if midi::is_timing(&m.raw) {
            return match midi::is_chatter(&m.raw) {
                Some(Chatter::Clock) => self.clock,
                Some(Chatter::Sense) => self.sense,
                None => self.mtc,
            };
        }
        true
    }
}

/// Collects messages into rate-limited `surfaceMonitor` events.
#[derive(Debug, Default)]
pub struct Batcher {
    pub filter: Filter,
    pending: Vec<MidiMessage>,
    dropped: u64,
    last: Option<Instant>,
}

impl Batcher {
    /// Takes the messages read since the last call; `lost` is what the reader could not queue.
    pub fn push(&mut self, messages: Vec<MidiMessage>, lost: u64) {
        self.dropped += lost;
        for m in messages.into_iter().filter(|m| self.filter.keeps(m)) {
            if self.pending.len() < BATCH {
                self.pending.push(m);
            } else {
                self.dropped += 1;
            }
        }
    }

    /// The event to send now, if there is something to say and the last one is old enough.
    pub fn flush(&mut self, now: Instant) -> Option<Value> {
        if self.pending.is_empty() && self.dropped == 0 {
            return None;
        }
        if self.last.is_some_and(|t| now.duration_since(t) < EVERY) {
            return None;
        }
        self.last = Some(now);
        let messages: Vec<Value> = self
            .pending
            .drain(..)
            .map(|m| {
                let decoded = if m.len > m.raw.len() { format!("{} (first {} shown)", midi::describe(&m.raw), m.raw.len()) } else { midi::describe(&m.raw) };
                json!({"device": m.device.0, "at": m.at.0, "raw": midi::hex(&m.raw), "decoded": decoded, "seq": m.seq})
            })
            .collect();
        Some(json!({"event": "surfaceMonitor", "messages": messages, "dropped": std::mem::take(&mut self.dropped)}))
    }
}

/// Where the messages read go.
#[derive(Debug, Default)]
pub struct Router {
    /// The MIDI Monitor, while it is on.
    pub monitor: Option<Batcher>,
}

impl Router {
    pub fn wants_reader(&self, prefs: &ControlSurfacePrefs) -> bool {
        self.monitor.is_some() || surface_runtime::learning() || prefs.maps_anything()
    }

    /// Map one pass of lane inputs, coalescing continuous values, and keep the monitor's copy.
    pub fn route(&mut self, prefs: &ControlSurfacePrefs, messages: Vec<MidiMessage>, inputs: Vec<Queued>, lost: u64, now: Instant) -> Vec<Value> {
        let mut out = Vec::new();
        surface_runtime::with(|rt| {
            for q in &inputs {
                out.extend(rt.handle(prefs, &q.device.0, &q.input, q.seq, now));
            }
            out.extend(rt.tick(prefs, now));
            out.extend(rt.take_events());
        });
        if let Some(b) = self.monitor.as_mut() {
            b.push(messages, lost);
        }
        coalesce_values(out)
    }
}

/// One value action per control per pass (the last position wins).
fn coalesce_values(events: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for ev in events {
        let key = value_key(&ev);
        if let Some(key) = key {
            if let Some(i) = at.get(&key).copied() {
                out[i] = ev;
                continue;
            }
            at.insert(key, out.len());
        }
        out.push(ev);
    }
    out
}

fn value_key(ev: &Value) -> Option<String> {
    if ev.get("event").and_then(Value::as_str) != Some("surfaceAction") {
        return None;
    }
    ev.pointer("/params/value")?;
    Some(format!("{}|{}", ev.get("device").unwrap_or(&Value::Null), ev.get("control").unwrap_or(&Value::Null)))
}

#[derive(Default)]
struct State {
    reader: Option<MidiReader>,
    seen_lost: u64,
    router: Router,
    hid: HashMap<String, HidReader>,
    hid_scanned: Option<Instant>,
    watch: Option<DevWatch>,
    /// Present at the last scan, so a dead reader is an unplug rather than a fault.
    hid_present: Vec<String>,
    last_error: Option<(String, Instant)>,
    disabled_reported: bool,
}

impl State {
    fn keep_hid(&mut self, prefs: &ControlSurfacePrefs, now: Instant) -> Vec<Queued> {
        let mut leftover = Vec::new();
        let present = self.hid_present.clone();
        let dead: Vec<String> = self.hid.iter().filter(|(_, r)| !r.running()).map(|(id, _)| id.clone()).collect();
        for id in dead {
            if let Some(reader) = self.hid.remove(&id) {
                leftover.extend(reader.drain(cfa4_surface::lane::TRIGGER_CAPACITY));
            }
            if present.iter().any(|p| p == &id) {
                surface_runtime::with(|rt| {
                    let _ = rt.note_fault(&id, now);
                });
            }
        }
        self.hid.retain(|id, r| r.running() && prefs.enabled && prefs.device(id).enabled && !surface_runtime::with(|rt| rt.disabled(id)));
        if self.watch.is_none() {
            self.watch = DevWatch::open(Path::new("/dev")).ok();
        }
        let woke = self.watch.as_ref().is_some_and(|w| w.changed());
        let due = self.hid_scanned.is_none_or(|t| now.saturating_duration_since(t) >= HID_SCAN);
        if !prefs.enabled || (!woke && !due) {
            return leftover;
        }
        self.hid_scanned = Some(now);
        let Ok(found) = cfa4_surface::hid::scan(Path::new("/sys/class/hidraw"), Path::new("/dev")) else { return leftover };
        self.hid_present = found.iter().map(|d| d.device.0.clone()).collect();
        for d in found {
            if d.error.is_some() || self.hid.contains_key(&d.device.0) || !prefs.device(&d.device.0).enabled {
                continue;
            }
            if surface_runtime::with(|rt| rt.disabled(&d.device.0)) {
                continue;
            }
            let buttons = d.vid.zip(d.pid).and_then(|(v, p)| cfa4_surface::hid::known(v, p)).map_or(15, |k| k.3);
            if let Ok(r) = HidReader::start(d.device.clone(), Path::new(&d.port), buttons) {
                self.hid.insert(d.device.0.clone(), r);
                surface_runtime::with(|rt| rt.note_ok(&d.device.0));
            }
        }
        leftover
    }

    fn ensure_reader(&mut self) -> Result<(), SurfaceError> {
        if self.reader.is_none() {
            self.reader = Some(MidiReader::start().map_err(|e| SurfaceError::new("alsa_unavailable", e))?);
            self.seen_lost = 0;
        }
        Ok(())
    }

    fn note_error(&mut self, code: &str, message: &str, now: Instant, out: &mut Vec<Value>) {
        if self.last_error.as_ref().is_some_and(|(c, t)| c == code && now.saturating_duration_since(*t) < Duration::from_secs(1)) {
            return;
        }
        self.last_error = Some((code.to_string(), now));
        out.push(json!({"event": "surfaceError", "code": code, "error": message}));
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// `surface.monitor`: turns the MIDI Monitor on or off.
pub fn monitor(p: &Value) -> Result<Value, SurfaceError> {
    let Some(enabled) = p.get("enabled").and_then(Value::as_bool) else {
        return Err(SurfaceError::new("invalid_params", "`enabled` (true or false) is required"));
    };
    let filter = Filter {
        device: p.get("device").and_then(Value::as_str).unwrap_or_default().to_string(),
        clock: p.pointer("/filter/clock").and_then(Value::as_bool).unwrap_or(false),
        sense: p.pointer("/filter/sense").and_then(Value::as_bool).unwrap_or(false),
        mtc: p.pointer("/filter/mtc").and_then(Value::as_bool).unwrap_or(false),
    };
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        if !enabled {
            st.router.monitor = None;
            return Ok(json!({"enabled": false}));
        }
        st.ensure_reader()?;
        match st.router.monitor.as_mut() {
            Some(b) => b.filter = filter,
            None => st.router.monitor = Some(Batcher { filter, ..Batcher::default() }),
        }
        Ok(json!({"enabled": true}))
    })
}

/// `surface.learn`. The full F0 machine (D2): the result is `learning` until silence, then
/// `pending` or `conflict`, and only [`learn_confirm`] keeps the assignment.
pub fn learn(s: &Session, p: &Value) -> Result<Value, SurfaceError> {
    let device = p.get("device").and_then(Value::as_str).unwrap_or_default();
    if !device.starts_with("hid:") {
        STATE.with(|st| st.borrow_mut().ensure_reader())?;
    }
    let prefs = s.prefs.control_surfaces.clone();
    surface_runtime::with(|rt| surface_runtime::learn(rt, &prefs, p, Instant::now())).map_err(engine_err)
}

/// `surface.learnCancel`.
pub fn learn_cancel() -> Value {
    surface_runtime::with(|rt| surface_runtime::learn_cancel(rt, Instant::now()))
}

/// `surface.learnConfirm {accept}`.
pub fn learn_confirm(s: &Session, p: &Value) -> Result<Value, SurfaceError> {
    let accept = p.get("accept").and_then(Value::as_bool).ok_or_else(|| SurfaceError::new("invalid_params", "`accept` is required"))?;
    let prefs = s.prefs.control_surfaces.clone();
    surface_runtime::with(|rt| surface_runtime::learn_confirm(rt, &prefs, accept, Instant::now())).map_err(engine_err)
}

/// `surface.inject`: the same mapping as a device event.
pub fn inject(s: &Session, p: &Value) -> Result<Value, SurfaceError> {
    let prefs = s.prefs.control_surfaces.clone();
    surface_runtime::with(|rt| surface_runtime::inject(rt, &prefs, p, Instant::now())).map_err(engine_err)
}

/// `surface.clock`.
pub fn clock(p: &Value) -> Result<Value, SurfaceError> {
    surface_runtime::with(|rt| surface_runtime::clock(rt, p, Instant::now())).map_err(engine_err)
}

/// `surface.enable`: clears a fault-disabled reader and stores the preference.
pub fn enable(s: &mut Session, p: &Value) -> Result<Value, SurfaceError> {
    let result = surface_runtime::with(|rt| surface_runtime::enable(rt, &mut s.prefs.control_surfaces, p)).map_err(engine_err)?;
    let prefs = s.prefs.clone();
    s.set_prefs(prefs).map_err(|e| SurfaceError::new("internal", format!("saving preferences: {e}")))?;
    Ok(result)
}

/// `surface.state`.
pub fn state(s: &Session) -> Value {
    publish_counters();
    surface_runtime::with(|rt| surface_runtime::state(rt, &s.prefs.control_surfaces))
}

/// `surface.audit`.
pub fn audit(s: &Session) -> Value {
    surface_runtime::audit(s)
}

fn publish_counters() {
    STATE.with(|st| {
        let st = st.borrow();
        let mut counters = HashMap::new();
        let mut kernel = 0;
        if let Some(reader) = &st.reader {
            counters.extend(reader.counters());
            kernel = reader.dropped();
        }
        for reader in st.hid.values() {
            // HID counters are inside the lane; dropped() is the stall total, not per device.
            let _ = reader.dropped();
        }
        drop(st);
        surface_runtime::with(|rt| rt.set_counters(counters, kernel));
    });
}

/// The events to send now.
pub fn upkeep(s: &Session) -> Vec<Value> {
    let prefs = &s.prefs.control_surfaces;
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        let now = Instant::now();
        let mut out = midi_upkeep(&mut st, prefs, now);
        let leftover = st.keep_hid(prefs, now);
        let inputs: Vec<Queued> = st.hid.values().flat_map(|r| r.drain(cfa4_surface::lane::TRIGGER_CAPACITY)).chain(leftover).collect();
        out.extend(map_inputs(&mut st.router, prefs, inputs, now));
        out.extend(surface_runtime::take_pending());
        out
    })
}

fn map_inputs(router: &mut Router, prefs: &ControlSurfacePrefs, inputs: Vec<Queued>, now: Instant) -> Vec<Value> {
    let _ = router;
    let mut out = Vec::new();
    surface_runtime::with(|rt| {
        for q in &inputs {
            out.extend(rt.handle(prefs, &q.device.0, &q.input, q.seq, now));
        }
        out.extend(rt.tick(prefs, now));
    });
    coalesce_values(out)
}

fn midi_upkeep(st: &mut State, prefs: &ControlSurfacePrefs, now: Instant) -> Vec<Value> {
    let mut out = Vec::new();
    let wanted = st.router.wants_reader(prefs);
    if surface_runtime::with(|rt| rt.disabled("alsa")) {
        st.reader = None;
        if wanted && !st.disabled_reported {
            st.disabled_reported = true;
            st.note_error("overload", "MIDI: stopped after repeated sequencer failures; turn the device back on to try again", now, &mut out);
        }
        out.extend(surface_runtime::tick_with(prefs, now));
        return out;
    }
    st.disabled_reported = false;
    if !wanted {
        st.reader = None;
        out.extend(surface_runtime::tick_with(prefs, now));
        return out;
    }
    if st.reader.is_none() {
        let wait = surface_runtime::with(|rt| rt.wait_until("alsa"));
        if wait.is_some_and(|t| now < t) {
            out.extend(surface_runtime::tick_with(prefs, now));
            return out;
        }
        if let Err(e) = st.ensure_reader() {
            let disabled = surface_runtime::with(|rt| {
                let _ = rt.note_fault("alsa", now);
                rt.disabled("alsa")
            });
            st.note_error(e.code, &e.message, now, &mut out);
            if disabled {
                st.note_error("overload", "MIDI: stopped after repeated sequencer failures; turn the device back on to try again", now, &mut out);
            }
            out.extend(surface_runtime::tick_with(prefs, now));
            return out;
        }
        surface_runtime::with(|rt| rt.note_ok("alsa"));
    }
    let snapshot = st.reader.as_ref().map(|reader| {
        reader.set_relative(prefs.relative_controls());
        let lost = reader.dropped();
        let inputs = reader.inputs();
        let messages = reader.drain(4_096);
        let running = reader.running();
        let counters = reader.counters();
        let dropped = reader.dropped();
        (lost, inputs, messages, running, counters, dropped)
    });
    let Some((lost, inputs, messages, running, counters, dropped)) = snapshot else {
        out.extend(surface_runtime::tick_with(prefs, now));
        return out;
    };
    let new_lost = lost.saturating_sub(st.seen_lost);
    st.seen_lost = lost;
    surface_runtime::with(|rt| rt.set_counters(counters, dropped));
    out.extend(st.router.route(prefs, messages, inputs, new_lost, now));
    if let Some(e) = st.router.monitor.as_mut().and_then(|b| b.flush(now)) {
        out.push(e);
    }
    if !running {
        st.reader = None;
        let disabled = surface_runtime::with(|rt| {
            let _ = rt.note_fault("alsa", now);
            rt.disabled("alsa")
        });
        if st.router.monitor.take().is_some() || surface_runtime::learning() {
            st.note_error("alsa_unavailable", "MIDI: the sequencer stopped; the MIDI Monitor and MIDI Learn are off", now, &mut out);
        }
        if disabled {
            surface_runtime::with(|rt| surface_runtime::learn_cancel(rt, now));
            st.note_error("overload", "MIDI: stopped after repeated sequencer failures; turn the device back on to try again", now, &mut out);
        }
    }
    out
}

/// Stops reading (the session ends).
pub fn shutdown() {
    STATE.with(|st| *st.borrow_mut() = State::default());
    surface_runtime::reset();
}

/// The reply of `surface.devices`.
pub fn devices(s: &Session) -> Value {
    with_settings(s, cfa4_surface::discover())
}

fn with_settings(s: &Session, found: cfa4_surface::Discovery) -> Value {
    let prefs = &s.prefs.control_surfaces;
    let devices: Vec<Value> = found
        .devices
        .iter()
        .map(|d| {
            let set = prefs.device(&d.device.0);
            let profile = if set.profile.is_empty() {
                PROFILES.iter().find(|p| p.driver == d.driver && p.default).map(|p| p.id).unwrap_or_default().to_string()
            } else {
                set.profile.clone()
            };
            let disabled = surface_runtime::with(|rt| rt.disabled(&d.device.0) || (d.device.0.starts_with("midi:") && rt.disabled("alsa")));
            let mut v = serde_json::to_value(d).unwrap_or_else(|_| json!({}));
            if let Some(o) = v.as_object_mut() {
                o.insert("enabled".into(), json!(set.enabled && !disabled));
                o.insert("profile".into(), json!(profile));
                if disabled {
                    o.insert("state".into(), json!("disabled"));
                }
            }
            v
        })
        .collect();
    json!({"enabled": prefs.enabled, "devices": devices, "problems": found.problems})
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_engine::control_surfaces::{Binding, DeviceSetting};
    use cfa4_surface::{DeviceInfo, Discovery};
    use cfa4_surface_model::{ControlId, DeviceId, DeviceState, MidiKind, SurfaceInput, SurfaceProtocol};

    fn dev(id: &str, driver: &str) -> DeviceInfo {
        DeviceInfo {
            device: DeviceId(id.into()),
            name: id.into(),
            protocol: if driver == "hid-shuttle" { SurfaceProtocol::Hid } else { SurfaceProtocol::Midi },
            driver: driver.into(),
            state: DeviceState::Online,
            port: "24:0".into(),
            vid: None,
            pid: None,
            error: None,
        }
    }

    fn fresh() {
        surface_runtime::reset();
    }

    #[test]
    fn devices_carry_the_users_choices() {
        let mut s = Session::default();
        s.prefs.control_surfaces.devices.push(DeviceSetting { device: "midi:a:b".into(), enabled: false, ..Default::default() });
        let found = Discovery { devices: vec![dev("midi:a:b", "generic-midi"), dev("hid:0b33:0030:usb-1", "hid-shuttle")], problems: vec!["MIDI: x".into()] };
        let v = with_settings(&s, found);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["devices"][0]["enabled"], false);
        assert_eq!(v["devices"][0]["profile"], "cfa4.generic-midi");
        assert_eq!(v["devices"][0]["protocol"], "midi");
        assert_eq!(v["devices"][1]["enabled"], true);
        assert_eq!(v["devices"][1]["profile"], "cfa4.shuttlepro");
        assert_eq!(v["devices"][1]["state"], "online");
        assert_eq!(v["problems"][0], "MIDI: x");
    }

    fn msg(device: &str, at: u64, raw: &[u8]) -> MidiMessage {
        MidiMessage { device: DeviceId(device.into()), at: cfa4_surface_model::MonoNs(at), raw: raw.to_vec(), len: raw.len(), seq: at }
    }

    #[test]
    fn monitor_batches_filters_and_limits_its_rate() {
        let mut b = Batcher::default();
        let t0 = Instant::now();
        assert_eq!(b.flush(t0), None, "nothing to say");
        b.push(vec![msg("midi:a:b", 5, &[0xb0, 16, 64]), msg("midi:a:b", 6, &[0xf8]), msg("midi:a:b", 7, &[0xfe]), msg("midi:a:b", 8, &[0xf1, 0x20])], 0);
        let e = b.flush(t0).unwrap_or_default();
        assert_eq!(e["event"], "surfaceMonitor");
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(1), "clock, active sensing and MTC are left out");
        assert_eq!(e["messages"][0]["decoded"], "Ch 1 CC 16 = 64");
        assert_eq!(e["dropped"], 0);
        b.filter.mtc = true;
        b.push(vec![msg("midi:a:b", 9, &[0xf1, 0x00]), msg("midi:a:b", 10, &[0xf0, 0x7f, 0x00, 0x01, 0x01, 0xf7])], 0);
        let e = b.flush(t0 + Duration::from_millis(60)).unwrap_or_default();
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(2), "MTC is shown when the filter asks");
        b.filter = Filter::default();
        b.push(vec![msg("midi:a:b", 8, &[0x90, 60, 1])], 0);
        assert_eq!(b.flush(t0 + Duration::from_millis(70)), None, "at most 20 events a second");
        let e = b.flush(t0 + Duration::from_millis(130)).unwrap_or_default();
        assert_eq!(e["messages"][0]["decoded"], "Ch 1 Note On 60 vel 1");
        b.push((0..BATCH as u64 + 5).map(|i| msg("midi:a:b", i, &[0xb0, 1, 2])).collect(), 3);
        let e = b.flush(t0 + Duration::from_millis(200)).unwrap_or_default();
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(BATCH));
        assert_eq!(e["dropped"], 8);
        b.filter = Filter { device: "midi:x:y".into(), clock: true, sense: false, mtc: false };
        b.push(vec![msg("midi:a:b", 1, &[0xb0, 1, 2]), msg("midi:x:y", 2, &[0xf8]), msg("midi:x:y", 3, &[0xfe])], 0);
        let e = b.flush(t0 + Duration::from_millis(300)).unwrap_or_default();
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(e["messages"][0]["decoded"], "Clock");
        b.filter = Filter::default();
        b.push(vec![MidiMessage { device: DeviceId("midi:a:b".into()), at: cfa4_surface_model::MonoNs(9), raw: vec![0xf0, 1, 2], len: 300, seq: 1 }], 0);
        let e = b.flush(t0 + Duration::from_millis(400)).unwrap_or_default();
        assert_eq!(e["messages"][0]["decoded"], "SysEx (3 bytes) (first 3 shown)");
    }

    fn queued(device: &str, seq: u64, input: SurfaceInput) -> Queued {
        Queued { device: DeviceId(device.into()), seq, at: cfa4_surface_model::MonoNs(seq), input, raw: vec![] }
    }

    #[test]
    fn presses_run_assignments_and_learn_waits_for_confirm() {
        fresh();
        let cc41 = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 41 };
        let mut prefs = ControlSurfacePrefs::default();
        let mut r = Router::default();
        assert!(!r.wants_reader(&prefs));
        prefs.devices.push(DeviceSetting {
            device: "midi:k:p".into(),
            bindings: vec![Binding { control: cc41.clone(), command: "playback.toggle".into(), params: Value::Null }],
            ..Default::default()
        });
        assert!(r.wants_reader(&prefs), "an assignment keeps the controllers read");
        let t0 = Instant::now();
        let press = SurfaceInput::Absolute { control: cc41.clone(), value: 127, bits: 7 };
        let out = r.route(&prefs, vec![], vec![queued("midi:k:p", 1, press.clone()), queued("midi:k:p", 2, press.clone())], 0, t0);
        assert_eq!(out.len(), 1, "held is not a second press");
        assert_eq!(out[0]["command"], "playback.toggle");
        assert_eq!(out[0]["seq"], 1);
        // Learn does not finish on the first message
        surface_runtime::with(|rt| {
            surface_runtime::learn(rt, &prefs, &json!({"device": "midi:k:p", "target": {"command": "edit.undo"}, "silenceMs": 1500, "firstMs": 7000}), t0)
        })
        .ok();
        let note = SurfaceInput::Button { control: ControlId::Midi { channel: 9, kind: MidiKind::Note, number: 36 }, pressed: true };
        let out = r.route(&prefs, vec![], vec![queued("midi:k:p", 3, note)], 0, t0 + Duration::from_millis(10));
        assert_eq!(out[0]["state"], "capturing");
        assert!(out.iter().all(|e| e["event"] != "surfaceAction"), "Learn does not run the old assignment");
        let out = r.route(&prefs, vec![], vec![], 0, t0 + Duration::from_millis(10 + 1500));
        assert_eq!(out[0]["state"], "pending");
        assert_eq!(out[0]["captured"]["mode"], "note");
        let confirmed = surface_runtime::with(|rt| surface_runtime::learn_confirm(rt, &prefs, true, t0)).unwrap_or_default();
        assert_eq!(confirmed["command"], "edit.undo");
        let release = SurfaceInput::Absolute { control: cc41, value: 0, bits: 7 };
        let _ = r.route(&prefs, vec![], vec![queued("midi:k:p", 4, release)], 0, t0);
        let out = r.route(&prefs, vec![], vec![queued("midi:k:p", 5, press)], 0, t0);
        assert_eq!(out.first().and_then(|e| e.get("event")).and_then(Value::as_str), Some("surfaceAction"));
    }

    #[test]
    fn jog_shuttle_and_their_buttons() {
        fresh();
        let dev = "hid:0b33:0030:usb-1";
        let hid = |seq, input| queued(dev, seq, input);
        let button = |pressed| SurfaceInput::Button { control: ControlId::hid_named("button3"), pressed };
        let prefs = ControlSurfacePrefs::default();
        let mut r = Router::default();
        let t0 = Instant::now();
        let out = r.route(
            &prefs,
            vec![],
            vec![
                hid(1, SurfaceInput::Shuttle { control: ControlId::hid_named("shuttle"), position: 4, range: 7 }),
                hid(2, SurfaceInput::Jog { control: ControlId::hid_named("jog"), delta: 2 }),
                hid(3, button(true)),
            ],
            0,
            t0,
        );
        assert_eq!(out[0]["command"], "playback.shuttleSpeed");
        assert_eq!(out[0]["params"]["speed"], 1.5);
        assert_eq!(out[1]["command"], "playhead.stepForward");
        assert_eq!(out[2]["command"], "playhead.stepForward");
        assert_eq!(out.len(), 3, "a button without an assignment runs nothing");
        surface_runtime::with(|rt| {
            surface_runtime::learn(rt, &prefs, &json!({"device": dev, "target": {"command": "markers.markIn"}, "silenceMs": 1500, "firstMs": 7000}), t0)
        })
        .ok();
        let out = r.route(
            &prefs,
            vec![],
            vec![hid(4, SurfaceInput::Jog { control: ControlId::hid_named("jog"), delta: -1 }), hid(5, button(false)), hid(6, button(true))],
            0,
            t0,
        );
        assert_eq!(out[0]["command"], "playhead.stepBack");
        assert!(out.iter().any(|e| e["state"] == "capturing"));
    }

    #[test]
    fn fader_values_are_coalesced_and_pickup_blocks_a_jump() {
        fresh();
        let fader = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 0 };
        let mut prefs = ControlSurfacePrefs::default();
        prefs.devices.push(DeviceSetting {
            device: "midi:k:p".into(),
            bindings: vec![Binding { control: fader.clone(), command: "mixer.stripPan".into(), params: json!({"strip": "Mix"}) }],
            ..Default::default()
        });
        let mut r = Router::default();
        let t0 = Instant::now();
        let moves = [10u16, 40, 90, 127]
            .iter()
            .enumerate()
            .map(|(i, v)| queued("midi:k:p", i as u64 + 1, SurfaceInput::Absolute { control: fader.clone(), value: *v, bits: 7 }))
            .collect();
        let out = r.route(&prefs, vec![], moves, 0, t0);
        assert_eq!(out.len(), 1, "one event per control per pass");
        assert_eq!(out[0]["params"]["value"], 1.0);
        let later = t0 + Duration::from_millis(600);
        let jumped = r.route(&prefs, vec![], vec![queued("midi:k:p", 9, SurfaceInput::Absolute { control: fader.clone(), value: 0, bits: 7 })], 0, later);
        assert!(jumped.iter().all(|e| e["event"] != "surfaceAction"), "pickup: a jump after a rest does not write, {jumped:?}");
        let near = r.route(
            &prefs,
            vec![],
            vec![queued("midi:k:p", 10, SurfaceInput::Absolute { control: fader, value: 120, bits: 7 })],
            0,
            later + Duration::from_millis(10),
        );
        assert_eq!(near[0]["event"], "surfaceAction", "within threshold 10 of 127");
    }

    #[test]
    fn inject_runs_the_same_path_and_mtc_is_not_learned() {
        fresh();
        let mut s = Session::default();
        s.prefs.control_surfaces.devices.push(DeviceSetting {
            device: "midi:k:p".into(),
            bindings: vec![Binding {
                control: ControlId::Midi { channel: 0, kind: MidiKind::Note, number: 60 },
                command: "edit.undo".into(),
                params: Value::Null,
            }],
            ..Default::default()
        });
        let seq = inject(
            &s,
            &json!({"device": "midi:k:p", "input": {"type": "button", "control": {"midi": {"channel": 0, "kind": "note", "number": 60}}, "pressed": true}}),
        )
        .unwrap_or_default();
        assert_eq!(seq["seq"], 1);
        let ev = surface_runtime::take_pending();
        assert_eq!(ev[0]["command"], "edit.undo");
        let t0 = Instant::now();
        surface_runtime::with(|rt| {
            surface_runtime::learn(rt, &s.prefs.control_surfaces, &json!({"target": {"command": "edit.redo"}, "silenceMs": 1500, "firstMs": 7000}), t0)
        })
        .ok();
        let mtc = inject(&s, &json!({"device": "midi:k:p", "input": {"type": "raw"}, "raw": [241, 0]})).unwrap_or_default();
        assert!(mtc["seq"].as_u64().is_some());
        let out = surface_runtime::tick_with(&s.prefs.control_surfaces, t0 + Duration::from_millis(7000));
        assert_eq!(out[0]["state"], "timeout", "MTC did not start a capture");
    }

    /// Without a sequencer (CI) Learn is an error that says why; cancelling always works.
    #[test]
    fn learn_without_a_sequencer() {
        fresh();
        let s = Session::default();
        match learn(&s, &json!({})) {
            Ok(v) => assert_eq!(v["state"], "learning"),
            Err(e) => assert_eq!(e.code, "alsa_unavailable"),
        }
        assert_eq!(learn_cancel()["state"], "cancelled");
        assert_eq!(learn(&s, &json!({"device": "hid:0b33:0030:usb-1"})).unwrap_or_default()["state"], "learning", "HID needs no sequencer");
        assert_eq!(learn_cancel()["state"], "cancelled");
        assert!(upkeep(&Session::default()).iter().all(|e| e["event"] != "surfaceAction"));
    }

    /// Without a sequencer (CI) turning the monitor on is an error that says why; off always works.
    #[test]
    fn monitor_on_and_off() {
        assert!(monitor(&json!({})).is_err());
        match monitor(&json!({"enabled": true})) {
            Ok(v) => assert_eq!(v["enabled"], true),
            Err(e) => {
                assert_eq!(e.code, "alsa_unavailable");
                assert!(e.message.contains("ALSA sequencer"), "{}", e.message);
            }
        }
        let _ = upkeep(&Session::default());
        assert_eq!(monitor(&json!({"enabled": false})).unwrap_or_default()["enabled"], false);
    }

    /// In CI there is neither a sequencer nor hidraw: the reply is still a list.
    #[test]
    fn real_discovery_is_a_list() {
        let s = Session::default();
        let v = devices(&s);
        assert!(v["devices"].is_array());
        assert!(v["problems"].is_array());
    }

    #[test]
    fn five_reader_faults_disable_until_enable() {
        fresh();
        let now = Instant::now();
        surface_runtime::with(|rt| {
            for _ in 0..4 {
                let _ = rt.note_fault("alsa", now);
            }
            assert!(!rt.disabled("alsa"));
            let _ = rt.note_fault("alsa", now);
            assert!(rt.disabled("alsa"));
        });
        let mut s = Session::default();
        let v = enable(&mut s, &json!({"device": "midi:k:p", "enabled": true})).unwrap_or_default();
        assert_eq!(v["enabled"], true);
        assert!(!surface_runtime::with(|rt| rt.disabled("alsa")));
    }
}
