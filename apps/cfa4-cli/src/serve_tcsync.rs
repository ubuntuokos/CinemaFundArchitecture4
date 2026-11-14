//! External timecode sync in `cfa4-cli serve` (plan TC-5): the [`SyncManager`] with the hardware
//! devices, driven by the `timecodeSync.*` preferences.
//!
//! - `timecodeSync.transport` `{"playing": bool, "time"|"seconds"|"frame": …, "speed": 1.0}`: the
//!   GUI reports its player whenever it starts, stops, seeks or changes speed. LTC and MTC output
//!   follow it; chase compares it with the external clock.
//! - Events `{"event": "timecodeSync.locate", "time": ticks, "seconds": s, "play": bool}` and
//!   `{"event": "timecodeSync.stop"}`: what the player should do to follow the external clock.
//! - `timecodeSync.status` (an engine command) shows the chase state and device errors.

use std::cell::RefCell;

use cfa4_engine::Session;
use cfa4_tcsync::chase::ChaseAction;
use cfa4_tcsync_io::{Backend, ExternalRate, Hardware, Source, SyncManager, SyncSettings};
use cfa4_time::Tick;
use serde_json::{Value, json};

struct State<B: Backend> {
    manager: Option<SyncManager<B>>,
    /// The player's last report, given to a manager opened later.
    transport: (bool, Tick, f64),
}

thread_local! {
    static SYNC: RefCell<State<Hardware>> = const { RefCell::new(State { manager: None, transport: (false, Tick(0), 1.0) }) };
}

/// The sync settings from the preferences and the active sequence.
pub fn settings(s: &Session) -> Result<SyncSettings, String> {
    let p = &s.prefs.timecode_sync;
    let (seq_rate, seq_df, start) = s.active_sequence().map(|q| (q.settings.frame_rate, q.settings.drop_frame, q.start_timecode)).unwrap_or_default();
    let (rate, df) = p.external_rate(seq_rate, seq_df);
    let rate = ExternalRate::new(rate, df);
    // the sequence's first frame shows its start timecode: external time `start` is position 0
    let start = seq_rate.sane().tick_of(start);
    let offset = Tick(p.offset_ticks(rate.rate, rate.drop_frame)?.0.saturating_sub(start.0));
    let chase = if p.chase {
        let mut c = p.chase_config(seq_rate, seq_df)?;
        c.offset = offset;
        Some(c)
    } else {
        None
    };
    let channel = |c: u32| u16::try_from(c.min(63)).unwrap_or(0);
    Ok(SyncSettings {
        chase,
        source: if p.source == "mtc" { Source::Mtc } else { Source::Ltc },
        rate,
        offset,
        audio_host: s.prefs.audio_hardware.device_class.clone(),
        ltc_input: p.ltc_input.clone(),
        ltc_input_channel: channel(p.ltc_input_channel),
        generate_ltc: p.generate_ltc,
        ltc_output: p.ltc_output.clone(),
        ltc_output_channel: channel(p.ltc_output_channel),
        ltc_level_dbfs: p.ltc_level_dbfs.clamp(-60.0, 0.0) as f32,
        generate_mtc: p.generate_mtc,
        mtc_input_port: p.mtc_input_port.clone(),
        mtc_output_port: p.mtc_output_port.clone(),
        output_latency: Tick::from_units(i64::from(p.output_latency_ms.min(1000)), 1000),
    })
}

/// `timecodeSync.transport`.
pub fn transport(s: &Session, p: &Value) -> Result<Value, String> {
    let playing = p.get("playing").and_then(Value::as_bool).ok_or("`playing` must be true or false")?;
    let position = crate::serve::time_param(s, p)?;
    let speed = match p.get("speed") {
        None => 1.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or("`speed` must be a finite number")?,
    };
    let active = SYNC.with_borrow_mut(|st| {
        st.transport = (playing, position, speed);
        st.manager.as_mut().map(|m| m.set_transport(playing, position, speed)).is_some()
    });
    Ok(json!({ "active": active }))
}

/// `timecodeSync.devices`: the names the LTC and MTC settings take (cpal devices of the
/// Settings ▸ Audio Hardware host, ALSA sequencer ports).
pub fn devices(s: &Session) -> Value {
    let (inputs, outputs) = cfa4_tcsync_io::cpal_dev::device_names(&s.prefs.audio_hardware.device_class);
    let (midi_inputs, midi_outputs) = cfa4_tcsync_io::alsa_seq::port_names();
    json!({ "ltcInputs": inputs, "ltcOutputs": outputs, "mtcInputs": midi_inputs, "mtcOutputs": midi_outputs })
}

/// Applies the preferences, reads the inputs and returns the events for the GUI. Returns whether
/// sync is on (the serve loop then checks more often).
pub fn upkeep(s: &mut Session) -> (Vec<Value>, bool) {
    SYNC.with_borrow_mut(|st| upkeep_with(st, s, |_| Hardware))
}

fn upkeep_with<B: Backend>(st: &mut State<B>, s: &mut Session, backend: impl FnOnce(&Session) -> B) -> (Vec<Value>, bool) {
    let wanted = settings(s);
    let active = wanted.as_ref().is_ok_and(SyncSettings::active);
    if !active {
        if let Some(mut m) = st.manager.take() {
            m.shutdown();
        }
        let rt = &mut s.timecode_sync;
        rt.chase = None;
        rt.ltc_out_running = false;
        rt.mtc_out_running = false;
        rt.warning = None;
        rt.error = wanted.err();
        return (Vec::new(), false);
    }
    let Ok(wanted) = wanted else {
        return (Vec::new(), false);
    };
    let m = st.manager.get_or_insert_with(|| {
        let mut m = SyncManager::new(backend(s));
        let (playing, position, speed) = st.transport;
        m.set_transport(playing, position, speed);
        m
    });
    m.apply(wanted);
    let events = m
        .poll()
        .into_iter()
        .map(|a| match a {
            ChaseAction::Locate { to, play } => json!({"event": "timecodeSync.locate", "time": to.0, "seconds": to.seconds(), "play": play}),
            ChaseAction::Stop => json!({"event": "timecodeSync.stop"}),
        })
        .collect();
    let status = m.status();
    let rt = &mut s.timecode_sync;
    rt.chase = status.chase;
    rt.ltc_out_running = status.ltc_out_running;
    rt.mtc_out_running = status.mtc_out_running;
    rt.error = status.error;
    rt.warning = status.warning;
    (events, true)
}

/// Closes the devices (end of the session).
pub fn shutdown() {
    SYNC.with_borrow_mut(|st| {
        if let Some(mut m) = st.manager.take() {
            m.shutdown();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Sender;

    use cfa4_tcsync::chase::Observation;
    use cfa4_tcsync_io::manager::{Device, LtcInputConfig, LtcOutputConfig, MtcConfig};
    use cfa4_tcsync_io::{Input, SharedTransport};
    use cfa4_time::FrameRate;

    use super::*;

    /// Inputs that replay frames from a running external clock; outputs that do nothing.
    #[derive(Default)]
    struct Fake {
        tx: std::rc::Rc<RefCell<Option<Sender<Input>>>>,
    }

    struct Quiet;

    impl Device for Quiet {
        fn error(&self) -> Option<String> {
            None
        }
    }

    impl Backend for Fake {
        fn ltc_input(&mut self, _: &LtcInputConfig, tx: Sender<Input>) -> Result<Box<dyn Device>, String> {
            *self.tx.borrow_mut() = Some(tx);
            Ok(Box::new(Quiet))
        }
        fn ltc_output(&mut self, _: &LtcOutputConfig, _: SharedTransport) -> Result<Box<dyn Device>, String> {
            Ok(Box::new(Quiet))
        }
        fn mtc(&mut self, _: &MtcConfig, _: Sender<Input>, _: SharedTransport) -> Result<Box<dyn Device>, String> {
            Err("no ALSA sequencer".into())
        }
    }

    #[test]
    fn settings_follow_the_preferences_and_the_sequence_start() {
        let mut s = Session::default();
        s.prefs.timecode_sync.chase = true;
        s.prefs.timecode_sync.rate = "25".into();
        s.prefs.timecode_sync.offset = "-00:00:01:00".into();
        s.prefs.timecode_sync.ltc_input_channel = 1000;
        let start = s.active_sequence().map(|q| q.start_timecode).unwrap_or(0);
        let seq_rate = s.sequence_rate();
        let w = settings(&s).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(w.rate.rate, FrameRate::FPS_25);
        assert_eq!(w.offset, Tick(-Tick::from_units(1, 1).0 - seq_rate.tick_of(start).0));
        assert_eq!(w.chase.map(|c| c.offset), Some(w.offset));
        assert_eq!(w.ltc_input_channel, 63);
        assert!(w.active());
    }

    #[test]
    fn upkeep_turns_frames_into_locate_events_and_reports_errors() {
        let mut s = Session::default();
        let fake = Fake::default();
        let tx = fake.tx.clone();
        let mut st = State { manager: None, transport: (false, Tick(0), 1.0) };
        let mut backend = Some(fake);
        let mut take = |_: &Session| backend.take().unwrap_or_default();

        // everything off: no manager, no events
        assert_eq!(upkeep_with(&mut st, &mut s, &mut take), (Vec::new(), false));
        assert!(st.manager.is_none());

        s.prefs.timecode_sync.chase = true;
        s.prefs.timecode_sync.rate = "25".into();
        s.prefs.timecode_sync.generate_mtc = true;
        let (ev, on) = upkeep_with(&mut st, &mut s, &mut take);
        assert!(on && ev.is_empty());
        assert_eq!(s.timecode_sync.error.as_deref(), Some("no ALSA sequencer"));

        let sender = tx.borrow().clone().expect("LTC input opened");
        let clock = st.manager.as_ref().map(|m| m.clock()).expect("manager");
        let frame = FrameRate::FPS_25.frame_duration().0;
        let start = s.active_sequence().map(|q| q.start_timecode).unwrap_or(0);
        let ext0 = Tick(s.sequence_rate().tick_of(start).0 + Tick::from_units(20, 1).0);
        let t0 = Tick(clock.now().0 - 3 * frame);
        for k in 0..3 {
            let _ = sender.send(Input::Frame(Observation { at: Tick(t0.0 + k * frame), position: Tick(ext0.0 + k * frame) }));
        }
        let (ev, _) = upkeep_with(&mut st, &mut s, &mut take);
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert_eq!(ev[0]["event"], "timecodeSync.locate");
        assert_eq!(ev[0]["play"], true);
        let to = ev[0]["seconds"].as_f64().unwrap_or(0.0);
        assert!((20.0..20.5).contains(&to), "{to}");
        assert!(s.timecode_sync.chase.is_some());

        // turning everything off closes the devices and clears the status
        s.prefs.timecode_sync.chase = false;
        s.prefs.timecode_sync.generate_mtc = false;
        assert_eq!(upkeep_with(&mut st, &mut s, &mut take), (Vec::new(), false));
        assert!(st.manager.is_none() && s.timecode_sync.chase.is_none() && s.timecode_sync.error.is_none());
    }

    #[test]
    fn transport_requests_are_checked() {
        let s = Session::default();
        assert!(transport(&s, &json!({})).is_err());
        assert!(transport(&s, &json!({"playing": true, "speed": "fast"})).is_err());
        assert_eq!(transport(&s, &json!({"playing": true, "seconds": 2.0})), Ok(json!({"active": false})));
        assert_eq!(SYNC.with_borrow(|st| st.transport), (true, Tick::from_units(2, 1), 1.0));
        // no sound card or sequencer in CI: empty lists, never a failure
        let d = devices(&s);
        assert!(d["ltcInputs"].is_array() && d["mtcOutputs"].is_array());
    }
}
