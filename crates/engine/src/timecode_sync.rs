//! External timecode sync: settings, chase configuration and commands (plan TC-4,
//! `docs/plans/TIMECODE-SYNC-LTC-MTC-2026-11-11.md`).
//!
//! The settings live in Settings ▸ Timecode Sync (`timecodeSync.*` preference keys). The devices
//! (LTC audio in/out, MTC MIDI ports) run in the host process, which reports the chase state back
//! through [`SyncRuntime`]; `timecodeSync.status` shows it to the UI, the CLI and agents.

use cfa4_tcsync::chase::{ChaseConfig, ChaseState, ChaseStatus};
use cfa4_time::{FrameRate, Tick, fields_to_frames, format_timecode_frames};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{Result, Session};

/// Settings ▸ Timecode Sync.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimecodeSyncPrefs {
    /// Follow an external timecode with playback.
    pub chase: bool,
    /// Chase source: `ltc` (audio input) or `mtc` (MIDI input).
    pub source: String,
    /// Rate of the external timecode: `sequence` or one of the LTC/MTC rates.
    pub rate: String,
    /// Added to the external timecode to get the sequence time, `[-]HH:MM:SS:FF`.
    pub offset: String,
    /// Agreeing frames before playback locks.
    pub lock_frames: u32,
    /// Frames to keep playing after the signal stops.
    pub freewheel_frames: u32,
    /// LTC input device (empty = Audio Hardware ▸ Default Input) and channel (0-based).
    pub ltc_input: String,
    pub ltc_input_channel: u32,
    /// Send LTC during playback.
    pub generate_ltc: bool,
    /// LTC output device (empty = the default output) and channel (0-based).
    pub ltc_output: String,
    pub ltc_output_channel: u32,
    /// LTC output peak level.
    pub ltc_level_dbfs: f64,
    /// Send MIDI Timecode during playback.
    pub generate_mtc: bool,
    /// MIDI ports to connect to (empty = only CFA4's own port, which other programs connect to).
    pub mtc_input_port: String,
    pub mtc_output_port: String,
    /// Send timecode this much ahead of the picture, for receivers that react late.
    pub output_latency_ms: u32,
}

impl Default for TimecodeSyncPrefs {
    fn default() -> Self {
        Self {
            chase: false,
            source: "ltc".into(),
            rate: "sequence".into(),
            offset: "00:00:00:00".into(),
            lock_frames: 3,
            freewheel_frames: 10,
            ltc_input: String::new(),
            ltc_input_channel: 0,
            generate_ltc: false,
            ltc_output: String::new(),
            ltc_output_channel: 0,
            ltc_level_dbfs: -18.0,
            generate_mtc: false,
            mtc_input_port: String::new(),
            mtc_output_port: String::new(),
            output_latency_ms: 0,
        }
    }
}

/// `timecodeSync.source` choices.
pub const SOURCES: &[(&str, &str)] = &[("ltc", "LTC (audio input)"), ("mtc", "MIDI Timecode (MIDI input)")];

/// `timecodeSync.rate` choices.
pub const RATES: &[(&str, &str)] = &[
    ("sequence", "Same as the sequence"),
    ("24", "24 fps"),
    ("25", "25 fps"),
    ("29.97df", "29.97 fps drop-frame"),
    ("29.97", "29.97 fps non-drop"),
    ("30", "30 fps"),
];

/// Signed SMPTE fields of an offset: `[-|+]HH:MM:SS:FF` (`;` also accepted before the frames).
fn parse_offset(s: &str) -> std::result::Result<(bool, [i64; 4]), String> {
    let t = s.trim();
    let (neg, body) = match t.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let parts: Vec<&str> = body.split([':', ';']).collect();
    let err = || format!("offset `{s}` must look like 00:00:00:00 or -01:00:00:00");
    let [h, m, sec, f] = parts.as_slice() else {
        return Err(err());
    };
    let mut out = [0i64; 4];
    for (slot, p) in out.iter_mut().zip([h, m, sec, f]) {
        if p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err());
        }
        *slot = p.parse().map_err(|_| err())?;
    }
    if out[0] > 23 || out[1] > 59 || out[2] > 59 || out[3] > 59 {
        return Err(err());
    }
    Ok((neg, out))
}

/// Check `timecodeSync.offset` (used by the settings schema).
pub fn validate_offset(v: &Value) -> std::result::Result<(), String> {
    let s = v.as_str().ok_or("the offset must be a timecode string like 00:00:00:00")?;
    parse_offset(s).map(|_| ())
}

impl TimecodeSyncPrefs {
    /// The external timecode's rate and drop-frame flag (`sequence` = the sequence's own).
    pub fn external_rate(&self, sequence: FrameRate, sequence_drop_frame: bool) -> (FrameRate, bool) {
        match self.rate.as_str() {
            "24" => (FrameRate::FPS_24, false),
            "25" => (FrameRate::FPS_25, false),
            "29.97df" => (FrameRate::FPS_29_97, true),
            "29.97" => (FrameRate::FPS_29_97, false),
            "30" => (FrameRate::FPS_30, false),
            _ => {
                let r = sequence.sane();
                (r, sequence_drop_frame && r.supports_drop_frame())
            }
        }
    }

    /// The offset in ticks, its frames counted at `rate`.
    pub fn offset_ticks(&self, rate: FrameRate, drop_frame: bool) -> std::result::Result<Tick, String> {
        let (neg, [h, m, s, f]) = parse_offset(&self.offset)?;
        let frames = fields_to_frames(h, m, s, f, rate, drop_frame);
        let t = rate.tick_of(frames);
        Ok(if neg { Tick(t.0.saturating_neg()) } else { t })
    }

    /// The chase settings for a sequence at `sequence` fps.
    pub fn chase_config(&self, sequence: FrameRate, sequence_drop_frame: bool) -> std::result::Result<ChaseConfig, String> {
        let (rate, df) = self.external_rate(sequence, sequence_drop_frame);
        let mut cfg = ChaseConfig::new(rate);
        cfg.offset = self.offset_ticks(rate, df)?;
        cfg.lock_frames = self.lock_frames.clamp(1, 30);
        cfg.freewheel = Tick(rate.frame_duration().0.saturating_mul(i64::from(self.freewheel_frames.clamp(2, 300))));
        Ok(cfg)
    }
}

/// What the device layer reports while sync runs (not saved).
#[derive(Clone, Debug, Default)]
pub struct SyncRuntime {
    /// Chase state; `None` while chase is off or its input is not open.
    pub chase: Option<ChaseStatus>,
    /// Whether the LTC / MTC outputs are running.
    pub ltc_out_running: bool,
    pub mtc_out_running: bool,
    /// The last device error (an input that could not be opened, a port that vanished).
    pub error: Option<String>,
    /// A note for the user, e.g. that MTC has no code for the sequence rate.
    pub warning: Option<String>,
}

fn state_name(s: ChaseState) -> &'static str {
    match s {
        ChaseState::NoSignal => "noSignal",
        ChaseState::Locating => "locating",
        ChaseState::Locked => "locked",
        ChaseState::Freewheel => "freewheel",
        ChaseState::Scrubbing => "scrubbing",
    }
}

fn status(s: &mut Session, _: &Value) -> Result<Value> {
    let p = &s.prefs.timecode_sync;
    let (seq_rate, seq_df) = s.active_sequence().map(|q| (q.settings.frame_rate, q.settings.drop_frame)).unwrap_or_default();
    let (rate, df) = p.external_rate(seq_rate, seq_df);
    let rt = &s.timecode_sync;
    let tc = |t: Tick| format_timecode_frames(rate.frame_at(t), rate, df);
    let chase = rt.chase.map(|c| {
        json!({
            "state": state_name(c.state),
            "timecode": c.position.map(tc),
            "speed": c.speed,
            "driftFrames": c.drift.map(|d| rate.frame_at(d)),
        })
    });
    Ok(json!({
        "chase": p.chase,
        "source": p.source,
        "generateLtc": p.generate_ltc,
        "generateMtc": p.generate_mtc,
        "rate": rate.label(),
        "dropFrame": df,
        "offsetValid": p.offset_ticks(rate, df).is_ok(),
        "chaseStatus": chase,
        "ltcOutRunning": rt.ltc_out_running,
        "mtcOutRunning": rt.mtc_out_running,
        "error": rt.error,
        "warning": rt.warning,
    }))
}

fn toggle(s: &mut Session, key: &str) -> Result<Value> {
    let mut next = s.prefs.clone();
    let on = next.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    next.set(key, json!(!on)).map_err(|e| crate::commands::bad(key, e))?;
    s.set_prefs(next).map_err(|e| crate::EngineError::Other(format!("saving preferences: {e}")))?;
    Ok(json!({ "on": !on }))
}

fn toggle_chase(s: &mut Session, _: &Value) -> Result<Value> {
    toggle(s, "timecodeSync.chase")
}

fn toggle_generate_ltc(s: &mut Session, _: &Value) -> Result<Value> {
    toggle(s, "timecodeSync.generateLtc")
}

fn toggle_generate_mtc(s: &mut Session, _: &Value) -> Result<Value> {
    toggle(s, "timecodeSync.generateMtc")
}

pub fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, run, journal| CommandSpec { id, label, menu: &[], shortcut: None, params: "{}", enabled: always, run, journal };
    vec![
        spec("timecodeSync.status", "Timecode Sync Status", status as fn(&mut Session, &Value) -> Result<Value>, false),
        spec("timecodeSync.toggleChase", "Chase External Timecode", toggle_chase, true),
        spec("timecodeSync.toggleGenerateLtc", "Send LTC", toggle_generate_ltc, true),
        spec("timecodeSync.toggleGenerateMtc", "Send MIDI Timecode", toggle_generate_mtc, true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_parse_and_convert() {
        let mut p = TimecodeSyncPrefs { offset: "-01:00:00:00".into(), ..Default::default() };
        assert_eq!(p.offset_ticks(FrameRate::FPS_25, false), Ok(Tick(-Tick::from_units(3600, 1).0)));
        p.offset = "+00:00:01;05".into();
        assert_eq!(p.offset_ticks(FrameRate::FPS_25, false), Ok(FrameRate::FPS_25.tick_of(30)));
        for bad in ["", "1:2:3", "aa:00:00:00", "24:00:00:00", "00:60:00:00", "000:00:00:00", "--00:00:00:00"] {
            p.offset = bad.into();
            assert!(p.offset_ticks(FrameRate::FPS_25, false).is_err(), "{bad}");
        }
        assert!(validate_offset(&json!(5)).is_err());
    }

    #[test]
    fn external_rate_and_chase_config() {
        let mut p = TimecodeSyncPrefs::default();
        assert_eq!(p.external_rate(FrameRate::FPS_29_97, true), (FrameRate::FPS_29_97, true));
        assert_eq!(p.external_rate(FrameRate { num: 0, den: 0 }, true), (FrameRate::default(), false));
        p.rate = "25".into();
        p.lock_frames = 0;
        p.freewheel_frames = 1_000_000;
        let c = p.chase_config(FrameRate::FPS_24, false).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.rate, FrameRate::FPS_25);
        assert_eq!(c.lock_frames, 1);
        assert_eq!(c.freewheel, FrameRate::FPS_25.tick_of(300));
    }
}

#[cfg(test)]
mod session_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn prefs_toggles_and_status() {
        let mut s = Session::default();
        let st = s.execute("timecodeSync.status", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(st["chase"], false);
        assert_eq!(st["chaseStatus"], serde_json::Value::Null);
        assert_eq!(s.execute("timecodeSync.toggleChase", json!({})).ok(), Some(json!({"on": true})));
        assert!(s.prefs.timecode_sync.chase);
        assert_eq!(s.execute("timecodeSync.toggleGenerateLtc", json!({})).ok(), Some(json!({"on": true})));
        assert_eq!(s.execute("timecodeSync.toggleGenerateMtc", json!({})).ok(), Some(json!({"on": true})));
        assert_eq!(s.execute("timecodeSync.toggleGenerateMtc", json!({})).ok(), Some(json!({"on": false})));

        // Hostile values: bad offsets and choices are refused, ranges clamp.
        assert!(s.execute("prefs.set", json!({"key": "timecodeSync.offset", "value": "99:99"})).is_err());
        assert!(s.execute("prefs.set", json!({"key": "timecodeSync.offset", "value": 7})).is_err());
        assert!(s.execute("prefs.set", json!({"key": "timecodeSync.source", "value": "smoke"})).is_err());
        assert!(s.execute("prefs.set", json!({"key": "timecodeSync.offset", "value": "-01:00:00:00"})).is_ok());
        assert!(s.execute("prefs.set", json!({"key": "timecodeSync.ltcLevelDbfs", "value": -500.0})).is_ok());
        assert_eq!(s.prefs.timecode_sync.ltc_level_dbfs, -60.0);
        assert!(s.execute("prefs.set", json!({"key": "timecodeSync.lockFrames", "value": 1000})).is_ok());
        assert_eq!(s.prefs.timecode_sync.lock_frames, 30);

        s.timecode_sync.chase = Some(cfa4_tcsync::chase::ChaseStatus {
            state: cfa4_tcsync::chase::ChaseState::Locked,
            position: Some(cfa4_time::FrameRate::default().tick_of(24)),
            speed: 1.0,
            drift: None,
        });
        let st = s.execute("timecodeSync.status", json!({})).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(st["chaseStatus"]["state"], "locked");
        assert_eq!(st["offsetValid"], true);
    }

    #[test]
    fn a_damaged_offset_in_the_file_is_repaired() {
        let mut v = crate::autosave::Preferences::default().to_value();
        v["timecodeSync"]["offset"] = json!("garbage");
        v["timecodeSync"]["rate"] = json!("17");
        let p = crate::autosave::Preferences::from_value(v);
        assert_eq!(p.timecode_sync.offset, "00:00:00:00");
        assert_eq!(p.timecode_sync.rate, "sequence");
    }
}
