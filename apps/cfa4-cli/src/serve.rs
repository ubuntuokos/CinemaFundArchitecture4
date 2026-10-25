//! `cfa4-cli serve`: the headless engine server behind the CFA4 Qt 6 GUI.
//!
//! The Qt application starts this process as its child and talks to it over stdin/stdout. No
//! socket is opened and no egui window is needed: the Rust engine (project, timeline, render,
//! codecs, export) runs here, the Qt process only draws. Protocol (one request per line):
//!
//! ```text
//! → {"id": 1, "method": "engine.execute", "params": {"command": "timeline.razor", "params": {"seconds": 3}}}
//! ← {"id": 1, "ok": true, "result": {...}}
//! ← {"id": 2, "ok": true, "result": {"w": 960, "h": 540, "format": "rgba8"}, "binary": 2073600}<2073600 raw bytes>
//! ← {"event": "projectChanged", "revision": 7}
//! ```
//!
//! A reply whose JSON object has `binary: N` is followed by exactly `N` raw bytes (a frame as
//! RGBA8, or audio as interleaved little-endian `f32`). Lines with an `event` key and no `id` are
//! engine notifications. Every request gets exactly one reply, errors included; a malformed line
//! gets an error reply with `id: null` and the server keeps serving (the parent is trusted, it is the
//! process that started us). Stdin EOF or `app.quit` ends the session cleanly (auto-save worker
//! stopped). Documented in `docs/qt-gui.md`.

use cfa4_engine::{Event, Session};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::sync::mpsc;
use std::time::Duration;

/// Protocol version, bumped on incompatible changes. The Qt client checks it in `hello`.
pub const PROTOCOL: u32 = 1;
/// Longest accepted request line (as the desktop control channel).
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Largest frame edge `frame.render` produces.
const MAX_EDGE: u32 = 8192;
/// Most audio sample frames one `audio.render` returns (10 s at 192 kHz).
const MAX_AUDIO_FRAMES: usize = 1_920_000;

/// One line from stdin: a request, or a line that could not be read as one.
enum Incoming {
    Line(Vec<u8>),
    TooLong,
    Eof,
}

/// A reply ready to write: the JSON envelope and an optional binary tail.
#[derive(Debug)]
pub struct Reply {
    pub json: Value,
    pub binary: Option<Vec<u8>>,
}

impl Reply {
    fn ok(id: Value, result: Value) -> Self {
        Self { json: json!({"id": id, "ok": true, "result": result}), binary: None }
    }

    fn err(id: Value, error: impl std::fmt::Display) -> Self {
        Self { json: json!({"id": id, "ok": false, "error": error.to_string()}), binary: None }
    }

    fn with_binary(id: Value, result: Value, bytes: Vec<u8>) -> Self {
        Self { json: json!({"id": id, "ok": true, "result": result, "binary": bytes.len()}), binary: Some(bytes) }
    }
}

/// Reads stdin on its own thread so the main loop can keep pumping export jobs and the auto-save
/// worker between requests.
fn spawn_reader() -> mpsc::Receiver<Incoming> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut h = stdin.lock();
        loop {
            let mut buf = Vec::new();
            let limit = u64::try_from(MAX_LINE).unwrap_or(u64::MAX).saturating_add(1);
            let n: usize = (&mut h).take(limit).read_until(b'\n', &mut buf).unwrap_or_default();
            if n == 0 {
                let _ = tx.send(Incoming::Eof);
                return;
            }
            if buf.len() > MAX_LINE && buf.last() != Some(&b'\n') {
                // drop the rest of the over-long line, then report it once
                let mut sink = Vec::new();
                loop {
                    sink.clear();
                    match (&mut h).take(64 * 1024).read_until(b'\n', &mut sink) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if sink.last() == Some(&b'\n') => break,
                        Ok(_) => {}
                    }
                }
                if tx.send(Incoming::TooLong).is_err() {
                    return;
                }
                continue;
            }
            if tx.send(Incoming::Line(buf)).is_err() {
                return;
            }
        }
    });
    rx
}

fn write_reply(out: &mut dyn Write, r: &Reply) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(&r.json).map_err(std::io::Error::other)?;
    line.push(b'\n');
    out.write_all(&line)?;
    if let Some(b) = &r.binary {
        out.write_all(b)?;
    }
    out.flush()
}

/// An engine event as a notification line, or `None` for events the GUI needs no line for.
pub fn event_json(ev: &Event) -> Value {
    match ev {
        Event::ProjectChanged { revision } => json!({"event": "projectChanged", "revision": revision}),
        Event::Toast { message, error } => json!({"event": "toast", "message": message, "error": error}),
        Event::OpenSequence(i) => json!({"event": "openSequence", "item": i.0}),
        Event::OpenSource(i) => json!({"event": "openSource", "item": i.0}),
        Event::RevealInProject(i) => json!({"event": "revealInProject", "item": i.0}),
    }
}

/// Parses one request line. `Err` carries the reply for a line that is not a request.
pub fn parse_request(line: &[u8]) -> Result<(Value, String, Value), Reply> {
    let v: Value = serde_json::from_slice(line).map_err(|e| Reply::err(Value::Null, format!("invalid JSON request: {e}")))?;
    let Value::Object(mut o) = v else {
        return Err(Reply::err(Value::Null, "a request must be a JSON object"));
    };
    let id = o.remove("id").unwrap_or(Value::Null);
    let Some(Value::String(method)) = o.remove("method") else {
        return Err(Reply::err(id, "a request needs a string `method`"));
    };
    let params = match o.remove("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p,
        Some(_) => return Err(Reply::err(id, "`params` must be a JSON object")),
    };
    Ok((id, method, params))
}

/// The time a request names: `time` (ticks), `seconds` or `frame`; else the playhead.
fn time_param(s: &Session, p: &Value) -> Result<cfa4_time::Tick, String> {
    if let Some(t) = p.get("time") {
        return t.as_i64().map(cfa4_time::Tick).ok_or_else(|| "`time` must be an integer tick count".to_string());
    }
    if let Some(sec) = p.get("seconds") {
        let sec = sec.as_f64().filter(|v| v.is_finite()).ok_or("`seconds` must be a finite number")?;
        return Ok(cfa4_time::Tick::from_seconds_f64(sec));
    }
    if let Some(f) = p.get("frame") {
        let f = f.as_i64().ok_or("`frame` must be an integer")?;
        return Ok(s.sequence_rate().tick_of(f));
    }
    Ok(s.playhead())
}

/// Scale for a frame no larger than `maxWidth` × `maxHeight` (or `scale` itself), within (0, 1].
pub fn frame_scale(seq_w: u32, seq_h: u32, p: &Value) -> Result<f32, String> {
    let mut scale = match p.get("scale") {
        None => 1.0f64,
        Some(v) => v.as_f64().filter(|x| x.is_finite() && *x > 0.0).ok_or("`scale` must be a positive number")?,
    };
    let edge = |k: &str| -> Result<Option<u32>, String> {
        match p.get(k) {
            None => Ok(None),
            Some(v) => v
                .as_u64()
                .and_then(|x| u32::try_from(x).ok())
                .filter(|x| (16..=MAX_EDGE).contains(x))
                .map(Some)
                .ok_or_else(|| format!("`{k}` must be an integer in 16..={MAX_EDGE}")),
        }
    };
    if let Some(mw) = edge("maxWidth")? {
        scale = scale.min(f64::from(mw) / f64::from(seq_w.max(1)));
    }
    if let Some(mh) = edge("maxHeight")? {
        scale = scale.min(f64::from(mh) / f64::from(seq_h.max(1)));
    }
    let longest = f64::from(seq_w.max(seq_h).max(1));
    scale = scale.min(f64::from(MAX_EDGE) / longest).clamp(1.0 / longest, 1.0);
    Ok(scale as f32)
}

/// Is this a request for the Source monitor (`target: "source"`) rather than the Program?
fn wants_source(p: &Value) -> Result<bool, String> {
    match p.get("target") {
        None => Ok(false),
        Some(Value::String(t)) if t == "program" => Ok(false),
        Some(Value::String(t)) if t == "source" => Ok(true),
        Some(_) => Err("`target` must be \"program\" or \"source\"".into()),
    }
}

/// The source item a Source request names (`item`), else the one open in the Source monitor.
fn source_item(s: &Session, p: &Value) -> Result<cfa4_project::ItemId, String> {
    let id = match p.get("item") {
        Some(v) => cfa4_project::ItemId(v.as_u64().ok_or("`item` must be an item id")?),
        None => s.state.source_item.ok_or("no clip is open in the Source monitor")?,
    };
    if s.project.item(id).is_none() {
        return Err(format!("no such item {}", id.0));
    }
    Ok(id)
}

/// The media item behind a source item (a subclip's parent), for its size and its sound.
fn media_of(s: &Session, mut id: cfa4_project::ItemId) -> cfa4_project::ItemId {
    // bounded: subclips of subclips are not expected, a cycle must not hang the server
    for _ in 0..8 {
        match s.project.item(id).map(|i| &i.kind) {
            Some(cfa4_project::ItemKind::Subclip { parent, .. }) => id = *parent,
            _ => break,
        }
    }
    id
}

/// Native picture size of a source item (media, subclip or sequence).
fn item_size(s: &Session, id: cfa4_project::ItemId) -> (u32, u32) {
    let id = media_of(s, id);
    let Some(item) = s.project.item(id) else { return (1920, 1080) };
    if let Some(q) = item.as_sequence() {
        return (q.settings.width, q.settings.height);
    }
    item.as_media().and_then(|m| m.info.video.as_ref()).map(|v| (v.width, v.height)).unwrap_or((1920, 1080))
}

fn render_frame(s: &Session, p: &Value) -> Result<(Value, Vec<u8>), String> {
    if wants_source(p)? {
        let item = source_item(s, p)?;
        let (w, h) = item_size(s, item);
        let scale = frame_scale(w, h, p)?;
        let t = match p.get("time").or(p.get("seconds")).or(p.get("frame")) {
            Some(_) => time_param(s, p)?,
            None => s.state.source_playhead,
        };
        let provider = s.media.provider(s.project.clone(), s.services.clone());
        let img = cfa4_render::render_item(&s.project, item, t, scale, &provider).ok_or("the source picture is unavailable (offline media?)")?;
        let bytes = img.over_black_rgba8();
        return Ok((json!({"w": img.w, "h": img.h, "format": "rgba8", "time": t.0, "item": item.0, "target": "source"}), bytes));
    }
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let (w, h) = (seq.settings.width, seq.settings.height);
    let scale = frame_scale(w, h, p)?;
    let t = time_param(s, p)?;
    let (img, offline) = s.try_render_program_with_offline_at(scale, t).map_err(|e| e.to_string())?;
    let bytes = img.over_black_rgba8();
    let offline: Vec<Value> = offline.iter().filter_map(|o| serde_json::to_value(o).ok()).collect();
    Ok((json!({"w": img.w, "h": img.h, "format": "rgba8", "time": t.0, "offline": offline, "target": "program"}), bytes))
}

fn audio_frames(p: &Value) -> Result<usize, String> {
    p.get("frames")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| (1..=MAX_AUDIO_FRAMES).contains(n))
        .ok_or_else(|| format!("`frames` must be an integer in 1..={MAX_AUDIO_FRAMES}"))
}

/// Interleaved little-endian f32 samples of `channels`, `frames` long (missing samples are silence).
fn interleave(channels: &[Vec<f32>], frames: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(frames.saturating_mul(channels.len()).saturating_mul(4));
    for i in 0..frames {
        for ch in channels {
            let v = ch.get(i).copied().unwrap_or(0.0);
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

fn render_audio(s: &Session, p: &Value) -> Result<(Value, Vec<u8>), String> {
    let frames = audio_frames(p)?;
    if wants_source(p)? {
        let item = source_item(s, p)?;
        let media = media_of(s, item);
        let sr = p
            .get("sampleRate")
            .map(|v| v.as_u64().and_then(|x| u32::try_from(x).ok()).filter(|x| (8_000..=192_000).contains(x)).ok_or("`sampleRate` must be 8000..=192000"))
            .transpose()?
            .unwrap_or(48_000);
        let start = match p.get("start") {
            Some(v) => v.as_i64().ok_or("`start` must be a sample index")?,
            None => match p.get("time").or(p.get("seconds")).or(p.get("frame")) {
                Some(_) => time_param(s, p)?.to_units_floor(i64::from(sr)),
                None => s.state.source_playhead.to_units_floor(i64::from(sr)),
            },
        };
        let has_audio = s.project.item(media).is_some_and(|i| i.has_audio());
        let channels: Vec<Vec<f32>> = if has_audio && start >= 0 {
            let provider = s.media.provider(s.project.clone(), s.services.clone());
            let source = cfa4_render::SourceProvider::source(&provider, media).ok_or("source audio is unavailable (offline media?)")?;
            let sound = source.audio(start, frames, sr).map_err(|e| format!("source audio decoding failed: {e}"))?;
            let sound = cfa4_render::audio::to_layout(sound, cfa4_audio_dsp::channels::Layout::Stereo, cfa4_audio_dsp::channels::Mixdown::default());
            sound.channels
        } else {
            vec![vec![0.0; frames]; 2]
        };
        let bytes = interleave(&channels, frames);
        return Ok((json!({"sampleRate": sr, "channels": channels.len(), "frames": frames, "start": start, "format": "f32le", "target": "source"}), bytes));
    }
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let sr = seq.settings.sample_rate.max(1);
    let start = match p.get("start") {
        Some(v) => v.as_i64().ok_or("`start` must be a sample index")?,
        None => time_param(s, p)?.to_units_floor(i64::from(sr)),
    };
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let buf = cfa4_render::audio::mix_sequence(&s.project, seq, start, frames, &provider);
    let channels = buf.channels.len();
    let bytes = interleave(&buf.channels, frames);
    Ok((json!({"sampleRate": buf.sample_rate, "channels": channels, "frames": frames, "start": start, "format": "f32le", "target": "program"}), bytes))
}

/// Handles one request. `Ok(None)` asks the loop to end the session.
pub fn handle(s: &mut Session, id: Value, method: &str, params: Value) -> Option<Reply> {
    let reply = match method {
        "hello" => Reply::ok(
            id,
            json!({
                "protocol": PROTOCOL,
                "engine": format!("CFA4 CLI {}", env!("CARGO_PKG_VERSION")),
                "pid": std::process::id(),
                "autosave": s.persistence.is_some(),
                // unsaved changes of sessions that died: the GUI offers Recover / Not Now / Discard
                "recovery": serde_json::to_value(s.recovery_candidates()).unwrap_or_default(),
            }),
        ),
        "engine.execute" => {
            let Some(cmd) = params.get("command").and_then(Value::as_str).map(str::to_string) else {
                return Some(Reply::err(id, "engine.execute needs a string `command`"));
            };
            let p = params.get("params").cloned().unwrap_or_else(|| json!({}));
            match s.execute(&cmd, p) {
                Ok(v) => Reply::ok(id, v),
                Err(e) => Reply::err(id, e),
            }
        }
        "engine.commands" => match s.execute("command.list", json!({})) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "frame.render" => match render_frame(s, &params) {
            Ok((meta, bytes)) => Reply::with_binary(id, meta, bytes),
            Err(e) => Reply::err(id, e),
        },
        "audio.render" => match render_audio(s, &params) {
            Ok((meta, bytes)) => Reply::with_binary(id, meta, bytes),
            Err(e) => Reply::err(id, e),
        },
        "session.state" => Reply::ok(
            id,
            json!({
                "dirty": s.is_dirty(),
                "revision": s.revision,
                "path": s.path,
                "canUndo": s.history.can_undo(),
                "canRedo": s.history.can_redo(),
                "playhead": s.playhead().0,
            }),
        ),
        "app.quit" => return None,
        other => Reply::err(id, format!("unknown method `{other}`")),
    };
    Some(reply)
}

/// Serves until stdin closes or `app.quit` arrives.
pub fn run(mut s: Session) {
    let rx = spawn_reader();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut jobs = false;
    loop {
        let wait = if jobs { Duration::from_millis(5) } else { Duration::from_millis(50) };
        let incoming = match rx.recv_timeout(wait) {
            Ok(i) => Some(i),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Incoming::Eof),
        };
        let mut quit = false;
        let reply = match incoming {
            None => None,
            Some(Incoming::Eof) => {
                quit = true;
                None
            }
            Some(Incoming::TooLong) => Some(Reply::err(Value::Null, format!("request line longer than {MAX_LINE} bytes"))),
            Some(Incoming::Line(line)) if line.iter().all(u8::is_ascii_whitespace) => None,
            Some(Incoming::Line(line)) => match parse_request(&line) {
                Err(r) => Some(r),
                Ok((id, method, params)) => {
                    // a panic in one command must not end the session: the GUI keeps its project
                    let rid = id.clone();
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&mut s, id, &method, params))) {
                        Ok(Some(r)) => Some(r),
                        Ok(None) => {
                            quit = true;
                            Some(Reply::ok(rid, json!({"quit": true})))
                        }
                        Err(_) => Some(Reply::err(rid, format!("internal error while running `{method}` (the session continues)"))),
                    }
                }
            },
        };
        if let Some(r) = reply
            && write_reply(&mut out, &r).is_err()
        {
            break;
        }
        // upkeep, as the desktop frontend does every frame
        let upkeep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.poll_persistence();
            s.pump_jobs(Duration::from_millis(8))
        }));
        jobs = upkeep.unwrap_or(false);
        let mut broken = false;
        for ev in s.drain_events() {
            if write_reply(&mut out, &Reply { json: event_json(&ev), binary: None }).is_err() {
                broken = true;
                break;
            }
        }
        if quit || broken {
            break;
        }
    }
    s.shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests_and_rejects_non_requests() {
        let (id, m, p) = parse_request(br#"{"id":3,"method":"hello"}"#).expect("request");
        assert_eq!((id, m.as_str(), p), (json!(3), "hello", json!({})));
        for bad in [&b"not json"[..], b"[1,2]", br#"{"id":1}"#, br#"{"method":5}"#, br#"{"method":"x","params":[1]}"#] {
            let r = parse_request(bad).expect_err("error reply");
            assert_eq!(r.json["ok"], json!(false), "{}", String::from_utf8_lossy(bad));
        }
        // the id is kept on a request with a bad method, so the client can match the error
        let r = parse_request(br#"{"id":"a","method":1}"#).expect_err("error reply");
        assert_eq!(r.json["id"], json!("a"));
    }

    #[test]
    fn frame_scale_is_bounded() {
        let s = |p: Value| frame_scale(1920, 1080, &p);
        assert_eq!(s(json!({})), Ok(1.0));
        assert_eq!(s(json!({"maxWidth": 960})), Ok(0.5));
        assert_eq!(s(json!({"maxWidth": 960, "maxHeight": 270})), Ok(0.25));
        assert_eq!(s(json!({"scale": 4.0})), Ok(1.0));
        assert!(s(json!({"scale": -1})).is_err());
        assert!(s(json!({"scale": "x"})).is_err());
        assert!(s(json!({"maxWidth": 0})).is_err());
        assert!(s(json!({"maxWidth": 99999})).is_err());
        // a degenerate sequence size still yields a finite, positive scale
        let z = frame_scale(0, 0, &json!({})).expect("scale");
        assert!(z.is_finite() && z > 0.0);
    }

    #[test]
    fn executes_commands_renders_and_quits() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let hello = handle(&mut s, json!(1), "hello", json!({})).expect("reply");
        assert_eq!(hello.json["result"]["protocol"], json!(PROTOCOL));
        let list = handle(&mut s, json!(2), "engine.commands", json!({})).expect("reply");
        assert!(list.json["result"].as_array().is_some_and(|a| a.len() > 50));
        let bad = handle(&mut s, json!(3), "engine.execute", json!({"command": "no.such.command"})).expect("reply");
        assert_eq!(bad.json["ok"], json!(false));
        let missing = handle(&mut s, json!(4), "engine.execute", json!({})).expect("reply");
        assert_eq!(missing.json["ok"], json!(false));
        let frame = handle(&mut s, json!(5), "frame.render", json!({"maxWidth": 64, "seconds": 1.0})).expect("reply");
        assert_eq!(frame.json["ok"], json!(true), "{}", frame.json);
        let (w, h) = (frame.json["result"]["w"].as_u64().unwrap_or(0), frame.json["result"]["h"].as_u64().unwrap_or(0));
        assert!(w > 0 && w <= 64 && h > 0);
        assert_eq!(frame.binary.as_ref().map(Vec::len), Some((w * h * 4) as usize));
        assert_eq!(frame.json["binary"], json!(w * h * 4));
        let audio = handle(&mut s, json!(6), "audio.render", json!({"start": 0, "frames": 480})).expect("reply");
        assert_eq!(audio.json["ok"], json!(true), "{}", audio.json);
        let ch = audio.json["result"]["channels"].as_u64().unwrap_or(0) as usize;
        assert_eq!(audio.binary.as_ref().map(Vec::len), Some(480 * ch * 4));
        let too_many = handle(&mut s, json!(7), "audio.render", json!({"frames": 0})).expect("reply");
        assert_eq!(too_many.json["ok"], json!(false));
        let unknown = handle(&mut s, json!(8), "nope", json!({})).expect("reply");
        assert_eq!(unknown.json["ok"], json!(false));
        assert!(handle(&mut s, json!(9), "app.quit", json!({})).is_none());
    }

    #[test]
    fn renders_the_source_monitor_picture_and_sound() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        // nothing open in the Source monitor yet: a clear error, not a panic
        let none = handle(&mut s, json!(1), "frame.render", json!({"target": "source"})).expect("reply");
        assert_eq!(none.json["ok"], json!(false));
        let bad = handle(&mut s, json!(2), "frame.render", json!({"target": "elsewhere"})).expect("reply");
        assert_eq!(bad.json["ok"], json!(false));
        let ghost = handle(&mut s, json!(3), "frame.render", json!({"target": "source", "item": 999_999})).expect("reply");
        assert_eq!(ghost.json["ok"], json!(false));
        let movie = s
            .project
            .items
            .values()
            .find(|i| i.as_media().is_some_and(|m| m.info.video.is_some() && !m.info.audio_streams.is_empty()))
            .map(|i| i.id)
            .expect("a demo movie with sound");
        s.execute("source.open", json!({"item": movie.0})).expect("open source");
        let frame = handle(&mut s, json!(4), "frame.render", json!({"target": "source", "maxWidth": 80, "seconds": 1.0})).expect("reply");
        assert_eq!(frame.json["ok"], json!(true), "{}", frame.json);
        assert_eq!(frame.json["result"]["item"], json!(movie.0));
        let (w, h) = (frame.json["result"]["w"].as_u64().unwrap_or(0), frame.json["result"]["h"].as_u64().unwrap_or(0));
        // media decode by whole-number decimation: at least the asked size, well below full size
        assert!((80..160).contains(&w) && h > 0, "{w}x{h}");
        assert_eq!(frame.binary.as_ref().map(Vec::len), Some((w * h * 4) as usize));
        let audio = handle(&mut s, json!(5), "audio.render", json!({"target": "source", "start": 0, "frames": 4800})).expect("reply");
        assert_eq!(audio.json["ok"], json!(true), "{}", audio.json);
        assert_eq!(audio.json["result"]["channels"], json!(2));
        assert_eq!(audio.binary.as_ref().map(Vec::len), Some(4800 * 2 * 4));
        let rate = handle(&mut s, json!(6), "audio.render", json!({"target": "source", "frames": 10, "sampleRate": 5})).expect("reply");
        assert_eq!(rate.json["ok"], json!(false));
    }

    #[test]
    fn replies_are_framed_with_their_binary_tail() {
        let mut buf = Vec::new();
        write_reply(&mut buf, &Reply::with_binary(json!(1), json!({"w": 1}), vec![1, 2, 3, 4])).expect("write");
        let nl = buf.iter().position(|b| *b == b'\n').expect("newline");
        let head: Value = serde_json::from_slice(&buf[..nl]).expect("json");
        assert_eq!(head["binary"], json!(4));
        assert_eq!(&buf[nl + 1..], &[1, 2, 3, 4]);
    }

    #[test]
    fn events_become_notification_lines() {
        let v = event_json(&Event::Toast { message: "hi".into(), error: true });
        assert_eq!(v, json!({"event": "toast", "message": "hi", "error": true}));
        assert!(v.get("id").is_none());
    }
}
