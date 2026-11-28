//! The headphone monitor of `audio.render {monitor}` (`docs/plans/CFA4-BINAURAL-MONITOR-001.md`
//! §3.3, work package B-03): the sound a monitor plays, heard through headphones. The Program's
//! Mix and the Source clip's channels are rendered by `cfa4_hrtf::ChannelMonitor` as virtual
//! loudspeakers in their own layout instead of being folded to stereo.
//!
//! An HRTF set is made on a thread of its own the first time a rate (or set) is asked for; until
//! it is ready the monitor plays the usual stereo fold and says so (`monitor.pending`), so the
//! first block never waits. A set that cannot be made is reported (`monitor.error`) and the stereo
//! fold plays; nothing here panics.

use std::sync::{Arc, Mutex};

use cfa4_hrtf::{ChannelMonitor, HrtfSet, MonitorConfig, ObjectMonitor};
use serde_json::{Value, json};

/// The HRTF set a request names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetId {
    /// The built-in SADIE II KU100 set.
    Builtin,
    /// A `.maudhrtf` file.
    File(String),
}

impl SetId {
    fn name(&self) -> &str {
        match self {
            SetId::Builtin => "builtin",
            SetId::File(p) => p,
        }
    }
}

/// `audio.render`'s `monitor` when it asks for headphones.
#[derive(Clone, Debug, PartialEq)]
pub struct Binaural {
    pub set: SetId,
    pub config: MonitorConfig,
}

/// Reads `monitor: {mode: "speakers" | "binaural", hrtf?, room?, lfe_db?}`; `None` for the
/// loudspeaker monitor (the default).
pub fn parse(p: &Value) -> Result<Option<Binaural>, String> {
    let Some(m) = p.get("monitor").filter(|v| !v.is_null()) else { return Ok(None) };
    let mode = m.get("mode").and_then(Value::as_str).ok_or("`monitor.mode` must be \"speakers\" or \"binaural\"")?;
    match mode {
        "speakers" => return Ok(None),
        "binaural" => {}
        _ => return Err("`monitor.mode` must be \"speakers\" or \"binaural\"".into()),
    }
    let set = match m.get("hrtf") {
        None | Some(Value::Null) => SetId::Builtin,
        Some(v) => match v.as_str() {
            Some("builtin") => SetId::Builtin,
            Some(path) if path.to_ascii_lowercase().ends_with(".maudhrtf") => SetId::File(path.to_owned()),
            _ => return Err("`monitor.hrtf` must be \"builtin\" or a .maudhrtf file".into()),
        },
    };
    let number = |key: &str, default: f32, lo: f32, hi: f32| -> Result<f32, String> {
        match m.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_f64()
                .map(|x| x as f32)
                .filter(|x| x.is_finite() && (lo..=hi).contains(x))
                .ok_or_else(|| format!("`monitor.{key}` must be a number in {lo}..={hi}")),
        }
    };
    let defaults = MonitorConfig::default();
    let config = MonitorConfig { room: number("room", defaults.room, 0.0, 1.0)?, lfe_db: number("lfe_db", defaults.lfe_db, -30.0, 0.0)? };
    Ok(Some(Binaural { set, config }))
}

/// A set being made, made, or refused.
#[derive(Clone)]
enum Load {
    Pending,
    Ready(Arc<HrtfSet>),
    Failed(String),
}

/// Every set asked for: (set, rate, state). A few entries at most (one per rate and set).
static SETS: Mutex<Vec<(SetId, u32, Load)>> = Mutex::new(Vec::new());

fn make(set: &SetId, rate: u32) -> Result<HrtfSet, String> {
    match set {
        SetId::Builtin => HrtfSet::builtin(rate).map_err(|e| e.to_string()),
        SetId::File(path) => {
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            HrtfSet::from_maudhrtf(&bytes, Some(rate)).map_err(|e| format!("{path}: {e}"))
        }
    }
}

/// The set for `rate`, starting to make it when nobody has.
fn set_at(set: &SetId, rate: u32) -> Load {
    let Ok(mut sets) = SETS.lock() else { return Load::Failed("the HRTF sets are unavailable".into()) };
    if let Some((.., load)) = sets.iter().find(|(s, r, _)| s == set && *r == rate) {
        return load.clone();
    }
    sets.push((set.clone(), rate, Load::Pending));
    let id = set.clone();
    let spawned = std::thread::Builder::new().name("hrtf-load".into()).spawn(move || {
        let made = match std::panic::catch_unwind(|| make(&id, rate)) {
            Ok(Ok(s)) => Load::Ready(Arc::new(s)),
            Ok(Err(e)) => Load::Failed(e),
            Err(_) => Load::Failed("the HRTF set could not be made".into()),
        };
        if let Ok(mut sets) = SETS.lock()
            && let Some(entry) = sets.iter_mut().find(|(s, r, _)| *s == id && *r == rate)
        {
            entry.2 = made;
        }
    });
    if spawned.is_err() {
        // no thread: make it here, once
        let made = match make(set, rate) {
            Ok(s) => Load::Ready(Arc::new(s)),
            Err(e) => Load::Failed(e),
        };
        if let Some(entry) = sets.iter_mut().find(|(s, r, _)| s == set && *r == rate) {
            entry.2 = made.clone();
        }
        return made;
    }
    Load::Pending
}

/// Waits (up to `seconds`) until the set for `rate` is made or refused; for tests and the CLI.
#[cfg(test)]
pub fn wait_for(set: &SetId, rate: u32, seconds: u64) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    loop {
        if !matches!(set_at(set, rate), Load::Pending) {
            return true;
        }
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// One monitor's renderer: (target, set, config, renderer).
type Monitor = (&'static str, SetId, MonitorConfig, ChannelMonitor);

thread_local! {
    /// The Source's and the Program's renderer, so each keeps its history across blocks.
    static MONITORS: std::cell::RefCell<Vec<Monitor>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The two ears of `channels` (one mix of the mixer's layouts, at `rate`, starting at sample
/// `start` of `target`'s clock) and the reply's `monitor` object; no ears when the set is not
/// ready or the layout cannot be played (the caller then plays its stereo fold).
pub fn render(target: &'static str, rate: u32, start: i64, channels: &[Vec<f32>], frames: usize, b: &Binaural) -> (Option<[Vec<f32>; 2]>, Value) {
    let speakers = |extra: Value| {
        let mut v = json!({"mode": "speakers", "hrtf": b.set.name()});
        if let (Some(v), Value::Object(extra)) = (v.as_object_mut(), extra) {
            v.extend(extra);
        }
        v
    };
    let set = match set_at(&b.set, rate) {
        Load::Ready(set) => set,
        Load::Pending => return (None, speakers(json!({"pending": "binaural"}))),
        Load::Failed(e) => return (None, speakers(json!({"error": e}))),
    };
    let inputs: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    let mut ears = [vec![0.0_f32; frames], vec![0.0_f32; frames]];
    let done = MONITORS.with(|cell| -> Result<(), String> {
        let mut monitors = cell.borrow_mut();
        let i = match monitors.iter().position(|(t, s, c, m)| *t == target && *s == b.set && *c == b.config && Arc::ptr_eq(m.hrtf(), &set)) {
            Some(i) => i,
            None => {
                monitors.retain(|(t, ..)| *t != target);
                monitors.push((target, b.set.clone(), b.config, ChannelMonitor::new(set.clone(), b.config).map_err(|e| e.to_string())?));
                monitors.len() - 1
            }
        };
        let (.., m) = monitors.get_mut(i).ok_or("monitor lost")?;
        let [l, r] = &mut ears;
        m.process(start, &inputs, l, r).map_err(|e| e.to_string())
    });
    match done {
        Ok(()) => (
            Some(ears),
            json!({"mode": "binaural", "hrtf": b.set.name(), "set": set.name(), "room": b.config.room, "lfe_db": b.config.lfe_db, "layout": channels.len()}),
        ),
        Err(e) => (None, speakers(json!({"error": e}))),
    }
}

/// The Source clip's object renderer: (media id, set, config, renderer).
type Objects = (u64, SetId, MonitorConfig, ObjectMonitor);

thread_local! {
    static OBJECTS: std::cell::RefCell<Option<Objects>> = const { std::cell::RefCell::new(None) };
}

/// The two ears of the Source clip's object audio (Dolby TrueHD Atmos) from sample `start` at
/// `rate`, and the reply's `monitor` object; `None` when the clip has no objects, when they are
/// not at `rate`, when the set is not ready or when this block's objects cannot be decoded (the
/// clip's channels then play through [`render`]).
pub fn render_objects(media: u64, rate: u32, start: i64, frames: usize, source: &dyn cfa4_media::MediaSource, b: &Binaural) -> Option<([Vec<f32>; 2], Value)> {
    let Load::Ready(set) = set_at(&b.set, rate) else { return None };
    let block = match source.object_audio(start, frames)? {
        Ok(block) => block,
        Err(e) => {
            log::warn!("headphone monitor: the objects do not decode here ({e}); their channels play");
            return None;
        }
    };
    if block.sample_rate != rate {
        return None;
    }
    let start = u64::try_from(start).ok()?;
    OBJECTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let fits = slot.as_ref().is_some_and(|(m, s, c, r)| *m == media && *s == b.set && *c == b.config && Arc::ptr_eq(r.hrtf(), &set));
        if !fits {
            *slot = Some((media, b.set.clone(), b.config, ObjectMonitor::new(set.clone(), b.config).ok()?));
        }
        let (.., monitor) = slot.as_mut()?;
        let inputs: Vec<&[f32]> = block.objects.iter().map(Vec::as_slice).collect();
        let mut ears = [vec![0.0_f32; frames], vec![0.0_f32; frames]];
        let [l, r] = &mut ears;
        monitor.process(start, &inputs, &block.current, &block.events, l, r);
        let meta = json!({
            "mode": "binaural", "hrtf": b.set.name(), "set": set.name(), "room": b.config.room, "lfe_db": b.config.lfe_db,
            "objects": block.objects.len(), "format": "Dolby TrueHD Atmos",
        });
        Some((ears, meta))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monitor_parameter_is_checked() {
        assert_eq!(parse(&json!({})), Ok(None));
        assert_eq!(parse(&json!({"monitor": {"mode": "speakers"}})), Ok(None));
        let b = parse(&json!({"monitor": {"mode": "binaural"}})).expect("binaural").expect("some");
        assert_eq!(b.set, SetId::Builtin);
        assert_eq!(b.config, MonitorConfig::default());
        let b = parse(&json!({"monitor": {"mode": "binaural", "room": 0, "lfe_db": -20, "hrtf": "/x/kemar.maudhrtf"}})).expect("ok").expect("some");
        assert_eq!(b.set, SetId::File("/x/kemar.maudhrtf".into()));
        assert_eq!(b.config, MonitorConfig { room: 0.0, lfe_db: -20.0 });
        for bad in [
            json!({"monitor": {"mode": "loud"}}),
            json!({"monitor": {}}),
            json!({"monitor": {"mode": "binaural", "room": 2}}),
            json!({"monitor": {"mode": "binaural", "lfe_db": "x"}}),
            json!({"monitor": {"mode": "binaural", "hrtf": "a.sofa"}}),
        ] {
            assert!(parse(&bad).is_err(), "{bad} is refused");
        }
    }

    #[test]
    fn a_missing_set_file_is_reported_not_fatal() {
        let b = Binaural { set: SetId::File("/nonexistent/none.maudhrtf".into()), config: MonitorConfig::default() };
        assert!(wait_for(&b.set, 48_000, 10));
        let (ears, meta) = render("program", 48_000, 0, &[vec![0.0; 16], vec![0.0; 16]], 16, &b);
        assert!(ears.is_none());
        assert_eq!(meta["mode"], "speakers");
        assert!(meta["error"].as_str().is_some_and(|e| e.contains("none.maudhrtf")), "{meta}");
    }
}
