//! `surface.devices`: the control surfaces of this machine for Preferences ▸ Control Surfaces
//! (CFA4-CONTROL-SURFACE-001, `docs/adr/ADR-0012-control-surface.md`).
//!
//! The device list comes from `cfa4-surface` (L5: ALSA sequencer MIDI ports, hidraw jog/shuttle
//! devices); each device carries the user's choices from `controlSurfaces.devices` (enabled,
//! profile, falling back to the driver's default profile). A machine without an ALSA sequencer or
//! hidraw gives a shorter list and a note in `problems`, never an error.
//!
//! `surface.monitor {enabled, device?, filter?: {clock?, sense?}}`: the MIDI Monitor. While it is
//! on, every MIDI message from the controllers comes as `{"event": "surfaceMonitor", "messages":
//! [{device, at, raw, decoded}], "dropped": n}`, at most 20 times a second, in batches of at most
//! [`BATCH`] messages; what does not fit is counted in `dropped`. MIDI clock and active sensing are
//! left out unless the filter asks for them. Turning it off (or `surface.monitor` failing to find
//! a sequencer) stops reading.
//!
//! `surface.learn {device?}` / `surface.learnCancel`: MIDI Learn (the next press is reported as a
//! `surfaceLearn` event). Assignments (`controlSurfaces.devices[].bindings`) are read whenever a
//! used device has one: a press of an assigned control is `{"event": "surfaceAction", command,
//! params, device, control}`, which the GUI runs as it runs a menu item.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use cfa4_engine::Session;
use cfa4_engine::control_surfaces::{ControlSurfacePrefs, PROFILES, Triggers};
use cfa4_surface::midi_in::{MidiMessage, MidiReader};
use cfa4_surface_model::midi::{self, Chatter};
use serde_json::{Value, json};

/// Most messages in one `surfaceMonitor` event.
pub const BATCH: usize = 200;

/// Shortest time between two `surfaceMonitor` events (20 Hz).
const EVERY: Duration = Duration::from_millis(50);

/// What the monitor shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only this device (empty = all).
    pub device: String,
    pub clock: bool,
    pub sense: bool,
}

impl Filter {
    fn keeps(&self, m: &MidiMessage) -> bool {
        if !self.device.is_empty() && m.device.0 != self.device {
            return false;
        }
        match midi::is_chatter(&m.raw) {
            Some(Chatter::Clock) => self.clock,
            Some(Chatter::Sense) => self.sense,
            None => true,
        }
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
                json!({"device": m.device.0, "at": m.at.0, "raw": midi::hex(&m.raw), "decoded": decoded})
            })
            .collect();
        Some(json!({"event": "surfaceMonitor", "messages": messages, "dropped": std::mem::take(&mut self.dropped)}))
    }
}

/// Where the messages read go: the MIDI Monitor, MIDI Learn and the assignments.
#[derive(Debug, Default)]
pub struct Router {
    /// The MIDI Monitor, while it is on.
    pub monitor: Option<Batcher>,
    /// MIDI Learn waits for a press on this device ("" = any device).
    pub learn: Option<String>,
    triggers: Triggers,
}

impl Router {
    /// Whether anything needs the controllers read.
    pub fn wants_reader(&self, prefs: &ControlSurfacePrefs) -> bool {
        self.monitor.is_some() || self.learn.is_some() || prefs.maps_anything()
    }

    /// The events `messages` give at once (`surfaceLearn` when Learn caught a press, else
    /// `surfaceAction` for an assigned control); the monitor collects them for its next batch.
    pub fn route(&mut self, prefs: &ControlSurfacePrefs, messages: Vec<MidiMessage>, lost: u64) -> Vec<Value> {
        let mut out = Vec::new();
        for m in &messages {
            let device = m.device.0.as_str();
            let Some(control) = self.triggers.press(device, &midi::input(&m.raw)) else { continue };
            if self.learn.as_deref().is_some_and(|d| d.is_empty() || d == device) {
                self.learn = None;
                out.push(json!({"event": "surfaceLearn", "state": "captured", "device": device, "control": control, "label": midi::control_label(&control)}));
                continue; // a press caught by Learn runs nothing
            }
            if let Some(a) = prefs.action(device, &control) {
                let params = if a.params.is_null() { json!({}) } else { a.params };
                out.push(json!({"event": "surfaceAction", "command": a.command, "params": params, "device": device, "control": control}));
            }
        }
        if let Some(b) = self.monitor.as_mut() {
            b.push(messages, lost);
        }
        out
    }
}

/// How long to wait before trying the sequencer again for the assignments after it failed.
const RETRY: Duration = Duration::from_secs(10);

#[derive(Default)]
struct State {
    reader: Option<MidiReader>,
    seen_lost: u64,
    router: Router,
    /// No reading for the assignments before this (the sequencer failed).
    retry_at: Option<Instant>,
}

impl State {
    /// Starts reading if it does not run yet (`surface.monitor`, `surface.learn` ask for it).
    fn ensure_reader(&mut self) -> Result<(), String> {
        if self.reader.is_none() {
            self.reader = Some(MidiReader::start()?);
            self.seen_lost = 0;
        }
        Ok(())
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// `surface.monitor`: turns the MIDI Monitor on or off.
pub fn monitor(p: &Value) -> Result<Value, String> {
    let Some(enabled) = p.get("enabled").and_then(Value::as_bool) else { return Err("`enabled` (true or false) is required".into()) };
    let filter = Filter {
        device: p.get("device").and_then(Value::as_str).unwrap_or_default().to_string(),
        clock: p.pointer("/filter/clock").and_then(Value::as_bool).unwrap_or(false),
        sense: p.pointer("/filter/sense").and_then(Value::as_bool).unwrap_or(false),
    };
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        if !enabled {
            st.router.monitor = None; // upkeep stops the reader when nothing else needs it
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

/// `surface.learn {device?}`: the next press (on `device`, or on any controller) is reported as
/// `{"event": "surfaceLearn", "state": "captured", device, control, label}` instead of running
/// its assignment. The GUI puts it into its draft.
pub fn learn(p: &Value) -> Result<Value, String> {
    let device = p.get("device").and_then(Value::as_str).unwrap_or_default().to_string();
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        st.ensure_reader()?;
        st.router.learn = Some(device);
        Ok(json!({"state": "learning"}))
    })
}

/// `surface.learnCancel`: stops waiting for a press.
pub fn learn_cancel() -> Value {
    STATE.with(|st| st.borrow_mut().router.learn = None);
    json!({"state": "cancelled"})
}

/// The events to send now (called by the serve loop after every request and timeout): presses
/// that run commands, Learn results and the monitor's batches. Reads the controllers only while
/// something needs it.
pub fn upkeep(s: &Session) -> Vec<Value> {
    let prefs = &s.prefs.control_surfaces;
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        let now = Instant::now();
        if !st.router.wants_reader(prefs) {
            st.reader = None; // dropping the reader stops its thread
            return Vec::new();
        }
        if st.reader.is_none() && st.retry_at.is_none_or(|t| now >= t) {
            // only the assignments need it here (Monitor and Learn started it themselves)
            if st.ensure_reader().is_err() {
                st.retry_at = Some(now + RETRY);
            }
        }
        let Some(reader) = st.reader.as_ref() else { return Vec::new() };
        let lost = reader.dropped();
        let messages = reader.drain(cfa4_surface::midi_in::QUEUE);
        let running = reader.running();
        let new_lost = lost.saturating_sub(st.seen_lost);
        st.seen_lost = lost;
        let mut out = st.router.route(prefs, messages, new_lost);
        if let Some(e) = st.router.monitor.as_mut().and_then(|b| b.flush(now)) {
            out.push(e);
        }
        if !running {
            st.reader = None;
            st.retry_at = Some(now + RETRY);
            if st.router.monitor.take().is_some() || st.router.learn.take().is_some() {
                out.push(json!({"event": "surfaceError", "code": "alsa_unavailable", "error": "MIDI: the sequencer stopped; the MIDI Monitor and MIDI Learn are off"}));
            }
        }
        out
    })
}

/// Stops reading (the session ends).
pub fn shutdown() {
    STATE.with(|st| *st.borrow_mut() = State::default());
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
            let mut v = serde_json::to_value(d).unwrap_or_else(|_| json!({}));
            if let Some(o) = v.as_object_mut() {
                o.insert("enabled".into(), json!(set.enabled));
                o.insert("profile".into(), json!(profile));
            }
            v
        })
        .collect();
    json!({"enabled": prefs.enabled, "devices": devices, "problems": found.problems})
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_engine::control_surfaces::DeviceSetting;
    use cfa4_surface::{DeviceInfo, Discovery};
    use cfa4_surface_model::{DeviceId, DeviceState, SurfaceProtocol};

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
        MidiMessage { device: DeviceId(device.into()), at: cfa4_surface_model::MonoNs(at), raw: raw.to_vec(), len: raw.len() }
    }

    #[test]
    fn monitor_batches_filters_and_limits_its_rate() {
        let mut b = Batcher::default();
        let t0 = Instant::now();
        assert_eq!(b.flush(t0), None, "nothing to say");
        b.push(vec![msg("midi:a:b", 5, &[0xb0, 16, 64]), msg("midi:a:b", 6, &[0xf8]), msg("midi:a:b", 7, &[0xfe])], 0);
        let e = b.flush(t0).unwrap_or_default();
        assert_eq!(e["event"], "surfaceMonitor");
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(1), "clock and active sensing are left out");
        assert_eq!(e["messages"][0], json!({"device": "midi:a:b", "at": 5, "raw": "B0 10 40", "decoded": "Ch 1 CC 16 = 64"}));
        assert_eq!(e["dropped"], 0);
        b.push(vec![msg("midi:a:b", 8, &[0x90, 60, 1])], 0);
        assert_eq!(b.flush(t0 + Duration::from_millis(10)), None, "at most 20 events a second");
        let e = b.flush(t0 + Duration::from_millis(60)).unwrap_or_default();
        assert_eq!(e["messages"][0]["decoded"], "Ch 1 Note On 60 vel 1");
        // a full batch counts the rest as dropped, with what the reader lost
        b.push((0..BATCH as u64 + 5).map(|i| msg("midi:a:b", i, &[0xb0, 1, 2])).collect(), 3);
        let e = b.flush(t0 + Duration::from_millis(200)).unwrap_or_default();
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(BATCH));
        assert_eq!(e["dropped"], 8);
        // the filter: one device, clock on
        b.filter = Filter { device: "midi:x:y".into(), clock: true, sense: false };
        b.push(vec![msg("midi:a:b", 1, &[0xb0, 1, 2]), msg("midi:x:y", 2, &[0xf8]), msg("midi:x:y", 3, &[0xfe])], 0);
        let e = b.flush(t0 + Duration::from_millis(300)).unwrap_or_default();
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(e["messages"][0]["decoded"], "Clock");
        // a long SysEx says it was cut
        b.filter = Filter::default();
        b.push(vec![MidiMessage { device: DeviceId("midi:a:b".into()), at: cfa4_surface_model::MonoNs(9), raw: vec![0xf0, 1, 2], len: 300 }], 0);
        let e = b.flush(t0 + Duration::from_millis(400)).unwrap_or_default();
        assert_eq!(e["messages"][0]["decoded"], "SysEx (3 bytes) (first 3 shown)");
    }

    #[test]
    fn presses_run_assignments_and_learn_catches_one() {
        use cfa4_engine::control_surfaces::{Binding, DeviceSetting};
        use cfa4_surface_model::{ControlId, MidiKind};
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
        let press = |at| msg("midi:k:p", at, &[0xb0, 41, 127]);
        let release = |at| msg("midi:k:p", at, &[0xb0, 41, 0]);
        let out = r.route(&prefs, vec![press(1), press(2), release(3)], 0);
        assert_eq!(
            out,
            vec![
                json!({"event": "surfaceAction", "command": "playback.toggle", "params": {}, "device": "midi:k:p", "control": {"midi": {"channel": 0, "kind": "cc", "number": 41}}})
            ]
        );
        assert!(r.route(&prefs, vec![msg("midi:k:p", 4, &[0xb0, 42, 127])], 0).is_empty(), "no assignment");
        // Learn on another device ignores this one; on this device it takes the press instead of running it
        r.learn = Some("midi:x:y".into());
        assert_eq!(r.route(&prefs, vec![press(5), release(6)], 0).len(), 1);
        r.learn = Some(String::new());
        let out = r.route(&prefs, vec![msg("midi:k:p", 7, &[0x99, 36, 100]), press(8)], 0);
        assert_eq!(
            out[0],
            json!({"event": "surfaceLearn", "state": "captured", "device": "midi:k:p", "control": {"midi": {"channel": 9, "kind": "note", "number": 36}}, "label": "Ch 10 Note 36"})
        );
        assert_eq!(out[1]["event"], "surfaceAction", "Learn takes one press only");
        assert_eq!(r.learn, None);
        // the monitor sees every message
        r.monitor = Some(Batcher::default());
        let _ = r.route(&prefs, vec![release(9), msg("midi:k:p", 10, &[0xfe])], 0);
        let e = r.monitor.as_mut().and_then(|b| b.flush(Instant::now())).unwrap_or_default();
        assert_eq!(e["messages"].as_array().map(Vec::len), Some(1));
    }

    /// Without a sequencer (CI) Learn is an error that says why; cancelling always works.
    #[test]
    fn learn_without_a_sequencer() {
        match learn(&json!({})) {
            Ok(v) => assert_eq!(v["state"], "learning"),
            Err(e) => assert!(e.contains("ALSA sequencer"), "{e}"),
        }
        assert_eq!(learn_cancel()["state"], "cancelled");
        assert!(upkeep(&Session::default()).is_empty());
    }

    /// Without a sequencer (CI) turning the monitor on is an error that says why; off always works.
    #[test]
    fn monitor_on_and_off() {
        assert!(monitor(&json!({})).is_err());
        match monitor(&json!({"enabled": true})) {
            Ok(v) => assert_eq!(v["enabled"], true),
            Err(e) => assert!(e.contains("ALSA sequencer"), "{e}"),
        }
        let _ = upkeep(&Session::default());
        assert_eq!(monitor(&json!({"enabled": false})).unwrap_or_default()["enabled"], false);
        assert!(upkeep(&Session::default()).is_empty());
    }

    /// In CI there is neither a sequencer nor hidraw: the reply is still a list.
    #[test]
    fn real_discovery_is_a_list() {
        let s = Session::default();
        let v = devices(&s);
        assert!(v["devices"].is_array());
        assert!(v["problems"].is_array());
    }
}
