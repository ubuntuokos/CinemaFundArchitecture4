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

/// The frame's pixels: RGB over black, alpha opaque, or, with `alpha: true`, the frame's own
/// alpha in the fourth byte (the monitors' Alpha display mode).
fn frame_bytes(img: &cfa4_render::Image, p: &Value) -> Vec<u8> {
    let mut bytes = img.over_black_rgba8();
    if p.get("alpha").and_then(Value::as_bool).unwrap_or(false) {
        for (o, px) in bytes.chunks_exact_mut(4).zip(img.px.chunks_exact(4)) {
            o[3] = (px[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    bytes
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
        let bytes = frame_bytes(&img, p);
        let par = s.project.source_par(item).map_or(1.0, cfa4_project::par_ratio);
        return Ok((
            json!({"w": img.w, "h": img.h, "fullW": w, "fullH": h, "pixelAspect": par, "format": "rgba8", "time": t.0, "item": item.0, "target": "source"}),
            bytes,
        ));
    }
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let (w, h) = (seq.settings.width, seq.settings.height);
    let scale = frame_scale(w, h, p)?;
    let t = time_param(s, p)?;
    let (img, offline) = s.try_render_program_with_offline_at(scale, t).map_err(|e| e.to_string())?;
    let bytes = frame_bytes(&img, p);
    let offline: Vec<Value> = offline.iter().filter_map(|o| serde_json::to_value(o).ok()).collect();
    let par = cfa4_project::par_ratio(seq.settings.par);
    Ok((
        json!({"w": img.w, "h": img.h, "fullW": w, "fullH": h, "pixelAspect": par, "format": "rgba8", "time": t.0, "offline": offline, "target": "program"}),
        bytes,
    ))
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
        let mut meta = json!({"sampleRate": sr, "channels": channels.len(), "frames": frames, "start": start, "format": "f32le", "target": "source"});
        if wants_meter(p) {
            meta["meter"] = meter("source", sr, start, &channels, frames);
        }
        return Ok((meta, bytes));
    }
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let sr = seq.settings.sample_rate.max(1);
    let start = match p.get("start") {
        Some(v) => v.as_i64().ok_or("`start` must be a sample index")?,
        None => time_param(s, p)?.to_units_floor(i64::from(sr)),
    };
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    // through the live mixer state: a fader or knob held in the Audio Track Mixer (`mixer.touch`)
    // is heard while it is held, and each strip's peaks are measured on the way
    let live = &s.previews.live;
    let _ = live.take_meters(); // what earlier mixes left
    let graph = cfa4_render::mixer::mix_graph(&s.project, seq, start, frames, &provider, Some(live));
    let buf = cfa4_render::audio::to_layout(graph, cfa4_audio_dsp::channels::Layout::Stereo, cfa4_audio_dsp::channels::Mixdown::FrontRear);
    let channels = buf.channels.len();
    let bytes = if s.voiceover.recording() {
        // a voice-over's countdown beeps are heard with the programme (not metered, as egui's)
        let mut heard = buf.channels.clone();
        cfa4_engine::voiceover::mix_cues(s, buf.sample_rate, start, &mut heard);
        interleave(&heard, frames)
    } else {
        interleave(&buf.channels, frames)
    };
    let mut meta = json!({"sampleRate": buf.sample_rate, "channels": channels, "frames": frames, "start": start, "format": "f32le", "target": "program"});
    let strips = live.take_meters();
    if wants_meter(p) {
        meta["meter"] = meter("program", buf.sample_rate, start, &buf.channels, frames);
        // each mixer strip's peak (linear, post-fader, one per channel) over this block
        let mut strips: Vec<Value> = strips.into_iter().map(|(id, peaks)| json!({"id": id.0, "peaks": peaks})).collect();
        strips.sort_by_key(|v| v["id"].as_u64());
        meta["meter"]["strips"] = Value::Array(strips);
    }
    Ok((meta, bytes))
}

fn wants_meter(p: &Value) -> bool {
    p.get("meter").and_then(Value::as_bool).unwrap_or(false)
}

/// Meter blocks per second in an `audio.render` reply (`meter: true`).
const METER_BLOCKS_PER_SECOND: u32 = 100;

/// The loudness meter of what the client is playing on one monitor: (target, sample rate,
/// channels, the sample the next block must start at, the meter). One per target, so the Source
/// and the Program playing at once each keep integrating. One server serves one GUI from one
/// thread.
type Loudness = (&'static str, u32, usize, i64, cfa4_audio_dsp::LoudnessMeter);

thread_local! {
    static LOUDNESS: std::cell::RefCell<Vec<Loudness>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What the Audio Meters show for a block of played audio: every 1/100 s, the sample peak of
/// each channel (linear) and the BS.1770 / EBU R128 loudness so far (momentary, short-term,
/// integrated LUFS and true peak dBTP; `null` below the meter's floor). Loudness keeps
/// integrating while the client asks for consecutive blocks of a target (whatever it asks for
/// the other target in between), and starts over on a jump (a seek, or another play).
fn meter(target: &'static str, sr: u32, start: i64, channels: &[Vec<f32>], frames: usize) -> Value {
    let block = (sr / METER_BLOCKS_PER_SECOND).max(1) as usize;
    let nch = channels.len().max(1);
    let finite = |v: f64| if v.is_finite() && v > -70.0 { json!((v * 10.0).round() / 10.0) } else { Value::Null };
    LOUDNESS.with(|cell| {
        let mut meters = cell.borrow_mut();
        let i = match meters.iter().position(|(t, ..)| *t == target) {
            Some(i) => i,
            None => {
                meters.push((target, 0, 0, i64::MIN, cfa4_audio_dsp::LoudnessMeter::new(f64::from(sr), nch)));
                meters.len() - 1
            }
        };
        let Some(entry) = meters.get_mut(i) else { return Value::Null };
        let fresh = !(entry.1 == sr && entry.2 == nch && entry.3 == start);
        if fresh {
            *entry = (target, sr, nch, start, cfa4_audio_dsp::LoudnessMeter::new(f64::from(sr), nch));
        }
        let (_, _, _, next, lm) = entry;
        let mut peaks = Vec::new();
        let mut loudness = Vec::new();
        let mut at = 0;
        while at < frames {
            let n = block.min(frames - at);
            let slices: Vec<&[f32]> = channels.iter().map(|c| c.get(at..(at + n).min(c.len())).unwrap_or(&[])).collect();
            peaks.push(slices.iter().map(|c| json!(c.iter().fold(0.0_f32, |m, x| m.max(x.abs())))).collect::<Vec<_>>());
            let padded: Vec<Vec<f32>> = slices
                .iter()
                .map(|c| {
                    let mut v = c.to_vec();
                    v.resize(n, 0.0);
                    v
                })
                .collect();
            lm.process(&padded.iter().map(Vec::as_slice).collect::<Vec<_>>());
            loudness.push(json!([finite(lm.momentary()), finite(lm.short_term()), finite(lm.integrated()), finite(lm.true_peak_dbtp())]));
            at += n;
        }
        *next = start + frames as i64;
        json!({"block": block, "peaks": peaks, "loudness": loudness, "reset": fresh})
    })
}

/// `time.parse {text, target?, time?}`: a timecode typed into a monitor's timecode field, read the
/// way Premiere reads it (`cfa4_time::parse_timecode`: any of `:;.,` separators, bare digits as
/// right-aligned `HHMMSSFF`, `+`/`-` relative to the monitor's frame: `time` when given, the
/// one the GUI shows, else the engine's playhead) at the Program sequence's or the Source clip's
/// rate. Returns the time (ticks) of that frame.
fn parse_time(s: &Session, p: &Value) -> Result<Value, String> {
    let text = p.get("text").and_then(Value::as_str).ok_or("`text` must be a string")?;
    let (rate, drop_frame, now) = if wants_source(p)? {
        let item = source_item(s, p)?;
        let view = cfa4_engine::clip_ops::source_view(s, item).ok_or("the Source clip is unavailable")?;
        (view.rate, false, s.state.source_playhead)
    } else {
        let seq = s.active_sequence().ok_or("no active sequence")?;
        (seq.settings.frame_rate, seq.settings.drop_frame, s.playhead())
    };
    // the GUI's shown playhead (it runs ahead of the engine's while playing), for "+5" and the like
    let now = match p.get("time") {
        None | Some(Value::Null) => now,
        Some(v) => cfa4_time::Tick(v.as_i64().ok_or("`time` must be integer ticks")?.max(0)),
    };
    let frame = cfa4_time::parse_timecode(text, rate, drop_frame, rate.frame_at(now)).map_err(|e| e.to_string())?;
    Ok(json!({"time": rate.tick_of(frame).0, "frame": frame}))
}

/// `clipMixer.inspect {time?}`: the Audio Clip Mixer's strips, one per audio track: the enabled
/// clip under the playhead with its Volume level (dB) and Panner balance there (keyframes
/// included; a held control's live value while it is held), the track's mute / solo, the Clip
/// Mixer automation mode and the channel layout.
fn clip_mixer_inspect(s: &Session, p: &Value) -> Result<Value, String> {
    use cfa4_time::Tick;
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let now = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let live = &s.previews.live;
    let strips: Vec<Value> = seq
        .audio_tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| {
            let clip = tr.item_at(now).filter(|c| c.enabled);
            let clip = clip.map(|c| {
                let mt = c.source_time_at(now);
                let vol = c.effect("volume").map(|e| e.f64_at("level", mt)).unwrap_or(0.0);
                let pan = c.effect("panner").map(|e| e.f64_at("balance", mt)).unwrap_or(0.0);
                let vol = live.get(tr.id, cfa4_render::audio::CLIP_LANE_VOLUME).map_or(vol, |o| o.value);
                let pan = live.get(tr.id, cfa4_render::audio::CLIP_LANE_PAN).map_or(pan, |o| o.value);
                json!({"id": c.id.0, "name": c.name, "volumeDb": vol, "pan": pan})
            });
            json!({
                "track": tr.id.0,
                "ref": format!("A{}", i + 1),
                "name": tr.name,
                "muted": tr.muted,
                "solo": tr.solo,
                "channels": cfa4_render::mixer::width_of(tr.channels),
                "mode": cfa4_engine::mixer::clip_mode(s, tr.id).label(),
                "clip": clip,
            })
        })
        .collect();
    Ok(json!({"time": now.0, "recording": s.mixrec.active(), "strips": strips}))
}

/// `time.readout {rows: [{source, mode, display}], time?, sourceTime?}`: the Timecode panel's rows
/// (egui's `timecode::row_text`), each `{value, caption}`. `source` is `program` or `source`;
/// `mode` is `current`, `media` (the source timecode of the topmost enabled video clip under the
/// playhead, or the Source clip's own), `duration`, `inOut` or `remaining`; `display` is a
/// `TimeDisplay` (`Timecode`, `Frames`, `Feet16`, `Feet35`, `AudioSamples`, `Seconds`). `time` /
/// `sourceTime` stand in for the playheads while the GUI plays.
fn time_readout(s: &Session, p: &Value) -> Result<Value, String> {
    use cfa4_time::{FrameRate, Tick, TimeDisplay, format_time};
    struct View {
        name: String,
        rate: FrameRate,
        df: bool,
        t: Tick,
        dur: Tick,
        marks: (Option<Tick>, Option<Tick>),
        media: Option<(Tick, FrameRate)>,
        sample_rate: i64,
    }
    let program_t = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let source_t = p.get("sourceTime").and_then(Value::as_i64).map(Tick).unwrap_or(s.state.source_playhead);
    let source_view = || {
        s.state.source_item.and_then(|i| s.project.item(i)).map(|it| {
            let start = it.as_media().and_then(|m| m.info.start_timecode).map(|f| it.frame_rate().tick_of(f)).unwrap_or_default();
            let marks = match &it.kind {
                cfa4_project::ItemKind::Media(m) => (m.mark_in, m.mark_out),
                cfa4_project::ItemKind::Sequence(q) => (q.mark_in, q.mark_out),
                _ => (None, None),
            };
            let sr = it.as_media().and_then(|m| m.info.audio()).map(|a| i64::from(a.sample_rate)).unwrap_or(48000);
            View {
                name: it.name.clone(),
                rate: it.frame_rate(),
                df: false,
                t: source_t,
                dur: it.duration(),
                marks,
                media: Some((start + source_t, it.frame_rate())),
                sample_rate: sr,
            }
        })
    };
    let program_view = || {
        s.active_sequence().map(|q| {
            let t = program_t;
            let media = q.video_tracks.iter().rev().find_map(|tr| tr.item_at(t).filter(|c| c.enabled)).and_then(|c| {
                let it = s.project.item(c.item)?;
                let m = it.as_media()?;
                let rate = m.frame_rate();
                let start = m.info.start_timecode.map(|f| rate.tick_of(f)).unwrap_or_default();
                Some((start + c.source_time_at(t), rate))
            });
            View {
                name: s.state.active_sequence.and_then(|i| s.project.item(i)).map(|i| i.name.clone()).unwrap_or_default(),
                rate: q.settings.frame_rate,
                df: q.settings.drop_frame,
                t,
                dur: q.duration(),
                marks: (q.mark_in, q.mark_out),
                media,
                sample_rate: i64::from(q.settings.sample_rate),
            }
        })
    };
    let rows = p.get("rows").and_then(Value::as_array).ok_or("`rows` must be an array")?;
    let mut out = Vec::new();
    for row in rows {
        let source = row.get("source").and_then(Value::as_str).unwrap_or("program");
        let mode = row.get("mode").and_then(Value::as_str).unwrap_or("current");
        let display: TimeDisplay =
            row.get("display").cloned().map(serde_json::from_value).transpose().map_err(|e| format!("bad `display`: {e}"))?.unwrap_or_default();
        let mode_label = match mode {
            "current" => "Current Time",
            "media" => "Media Time",
            "duration" => "Duration",
            "inOut" => "In/Out Duration",
            "remaining" => "Remaining",
            other => return Err(format!("unknown mode `{other}`")),
        };
        let is_source = source == "source";
        let view = if is_source { source_view() } else { program_view() };
        let Some(v) = view else {
            out.push(json!({"value": "--:--:--:--", "mode": mode_label, "source": if is_source { "Source" } else { "Program" }, "name": null}));
            continue;
        };
        let fmt = |t: Tick, rate: FrameRate, df: bool| format_time(t, rate, df, display, v.sample_rate);
        let (a, b) = (v.marks.0.unwrap_or(Tick::ZERO), v.marks.1.unwrap_or(v.dur));
        let value = match mode {
            "current" => fmt(v.t, v.rate, v.df),
            "media" => v.media.map_or_else(|| "--:--:--:--".to_string(), |(t, r)| fmt(t, r, false)),
            "duration" => fmt(v.dur, v.rate, v.df),
            "inOut" => fmt((b - a).max(Tick::ZERO), v.rate, v.df),
            _ => fmt((b - v.t).max(Tick::ZERO), v.rate, v.df),
        };
        out.push(json!({"value": value, "mode": mode_label, "source": if is_source { "Source" } else { "Program" }, "name": v.name}));
    }
    Ok(json!({"rows": out}))
}

/// Most buckets one `media.peaks` returns.
const MAX_PEAK_BUCKETS: usize = 20_000;
/// Longest media span one `media.peaks` reads (10 minutes at 48 kHz), so a request stays short.
const MAX_PEAK_FRAMES: i64 = 48_000 * 600;

/// `media.peaks {item, start, end, buckets}`: the waveform of a media item's sound between two
/// media times (ticks), as `buckets` (min, max) pairs over all the channels, f32le. The
/// timeline draws its audio clips from these.
fn media_peaks(s: &Session, p: &Value) -> Result<(Value, Vec<u8>), String> {
    let item = source_item(s, p)?;
    let media = media_of(s, item);
    let tick = |k: &str| p.get(k).and_then(Value::as_i64).ok_or_else(|| format!("`{k}` must be a tick count"));
    let (t0, t1) = (tick("start")?, tick("end")?);
    if t1 <= t0 {
        return Err("`end` must be after `start`".into());
    }
    let buckets = p
        .get("buckets")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| (1..=MAX_PEAK_BUCKETS).contains(n))
        .ok_or_else(|| format!("`buckets` must be an integer in 1..={MAX_PEAK_BUCKETS}"))?;
    // a timeline clip plays one of its source's audio streams (`audioStream` in sequence.inspect)
    let stream = match p.get("stream") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < 256).ok_or("`stream` must be an audio stream index")?,
    };
    let sr: u32 = 48_000;
    let f0 = cfa4_time::Tick(t0).to_units_floor(i64::from(sr)).max(0);
    let f1 = cfa4_time::Tick(t1).to_units_floor(i64::from(sr)).max(f0 + 1);
    if f1 - f0 > MAX_PEAK_FRAMES {
        return Err(format!("span too long: at most {} s per request", MAX_PEAK_FRAMES / i64::from(sr)));
    }
    let total = usize::try_from(f1 - f0).map_err(|_| "span too long")?;
    let mut out = vec![0f32; buckets * 2];
    let has_audio = s.project.item(media).is_some_and(|i| i.has_audio());
    if has_audio {
        let provider = s.media.provider(s.project.clone(), s.services.clone());
        let source = cfa4_render::SourceProvider::source(&provider, media).ok_or("media is unavailable (offline?)")?;
        // decode in chunks; each sample lands in bucket floor(i * buckets / total)
        let chunk = 48_000usize;
        let mut done = 0usize;
        while done < total {
            let n = chunk.min(total - done);
            let start = f0 + i64::try_from(done).map_err(|_| "span too long")?;
            let sound = source.audio_stream(stream, start, n, sr).map_err(|e| format!("audio decoding failed: {e}"))?;
            for i in 0..n {
                let b = ((done + i) as u128 * buckets as u128 / total as u128) as usize;
                if let Some(pair) = out.get_mut(b * 2..b * 2 + 2) {
                    // the extremes over every channel: a mono mix would cancel out-of-phase channels
                    for v in sound.channels.iter().filter_map(|c| c.get(i).copied()) {
                        pair[0] = pair[0].min(v);
                        pair[1] = pair[1].max(v);
                    }
                }
            }
            done += n;
        }
    }
    let mut bytes = Vec::with_capacity(out.len() * 4);
    for v in out {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    Ok((json!({"item": media.0, "buckets": buckets, "start": t0, "end": t1, "hasAudio": has_audio, "format": "f32le min,max"}), bytes))
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
        "scopes.image" => match time_param(s, &params).and_then(|t| crate::serve_scopes::image(s, &params, t)) {
            Ok((meta, bytes)) => Reply::with_binary(id, meta, bytes),
            Err(e) => Reply::err(id, e),
        },
        "mediaBrowser.inspect" => match crate::serve_media_browser::inspect(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "mediaBrowser.thumbnail" => match crate::serve_media_browser::thumbnail(s, &params) {
            Ok((meta, bytes)) => Reply::with_binary(id, meta, bytes),
            Err(e) => Reply::err(id, e),
        },
        "text.inspect" => match crate::serve_text::inspect(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "graphics.inspect" => match crate::serve_graphics::inspect(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "lumetri.inspect" => match crate::serve_lumetri::inspect(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "clipMixer.inspect" => match clip_mixer_inspect(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "time.readout" => match time_readout(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "time.parse" => match parse_time(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "media.peaks" => match media_peaks(s, &params) {
            Ok((meta, bytes)) => Reply::with_binary(id, meta, bytes),
            Err(e) => Reply::err(id, e),
        },
        "project.panel" => match crate::serve_project::panel(s, &params) {
            Ok(v) => Reply::ok(id, v),
            Err(e) => Reply::err(id, e),
        },
        "lumetri.presetThumbnail" => match crate::serve_lumetri::preset_thumbnail(&params) {
            Ok((meta, bytes)) => Reply::with_binary(id, meta, bytes),
            Err(e) => Reply::err(id, e),
        },
        "tts.previewAudio" => match crate::serve_tts::preview_audio(s, &params) {
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
    fn frame_bytes_keep_the_alpha_only_when_asked() {
        let mut img = cfa4_render::Image::new(2, 1);
        img.px = vec![0.5, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0];
        let opaque = frame_bytes(&img, &json!({}));
        assert_eq!((opaque[3], opaque[7]), (255, 255));
        let alpha = frame_bytes(&img, &json!({"alpha": true}));
        assert_eq!((alpha[3], alpha[7]), (128, 0));
        assert_eq!(alpha[..3], opaque[..3]);
    }

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
    fn parses_typed_timecodes_and_reports_the_frame_size() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let rate = s.active_sequence().map(|q| q.settings.frame_rate).expect("demo sequence");
        let ask = |s: &mut Session, text: &str| handle(s, json!(1), "time.parse", json!({"text": text})).expect("reply").json;
        let r = ask(&mut s, "00:00:02:00");
        let two_seconds = 2 * rate.timecode_base();
        assert_eq!(r["result"]["frame"], json!(two_seconds), "{r}");
        assert_eq!(r["result"]["time"], json!(rate.tick_of(two_seconds).0), "{r}");
        assert_eq!(ask(&mut s, "200")["result"]["time"], r["result"]["time"], "bare digits are HHMMSSFF");
        s.execute("playhead.set", json!({"time": r["result"]["time"]})).expect("seek");
        let later = ask(&mut s, "+5");
        assert_eq!(later["result"]["frame"].as_i64(), r["result"]["frame"].as_i64().map(|f| f + 5));
        assert_eq!(ask(&mut s, "abc")["ok"], json!(false));
        // relative to the GUI's shown frame (playback runs ahead of the engine's playhead)
        let shown = rate.tick_of(100);
        let rel = handle(&mut s, json!(3), "time.parse", json!({"text": "+5", "time": shown.0})).expect("reply").json;
        assert_eq!(rel["result"]["frame"], json!(105), "{rel}");
        let bad = handle(&mut s, json!(4), "time.parse", json!({"text": "+5", "time": "x"})).expect("reply").json;
        assert_eq!(bad["ok"], json!(false));
        let frame = handle(&mut s, json!(2), "frame.render", json!({"maxWidth": 64})).expect("reply");
        let seq = s.active_sequence().expect("demo sequence");
        assert_eq!(frame.json["result"]["fullW"], json!(seq.settings.width));
        assert_eq!(frame.json["result"]["fullH"], json!(seq.settings.height));
        assert_eq!(frame.json["result"]["pixelAspect"], json!(1.0));
    }

    #[test]
    fn inspects_the_clip_mixer() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let rate = s.active_sequence().map(|q| q.settings.frame_rate).expect("demo sequence");
        let t = rate.tick_of(2 * rate.timecode_base());
        let r = handle(&mut s, json!(1), "clipMixer.inspect", json!({"time": t.0})).expect("reply").json;
        let strips = r["result"]["strips"].as_array().cloned().expect("strips");
        assert_eq!(strips.len(), 3, "{r}");
        assert_eq!(strips[0]["ref"], json!("A1"));
        let clip = &strips[0]["clip"];
        assert!(clip["id"].is_u64(), "a clip under the playhead on A1: {r}");
        assert!(clip["volumeDb"].as_f64().is_some_and(|v| (-96.0..=15.0).contains(&v)), "{r}");
        assert_eq!(strips[0]["mode"], json!("Read"));
    }

    #[test]
    fn reads_out_the_timecode_panel_rows() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let rate = s.active_sequence().map(|q| q.settings.frame_rate).expect("demo sequence");
        let t = rate.tick_of(2 * rate.timecode_base());
        let rows = json!([
            {"source": "program", "mode": "current", "display": "Timecode"},
            {"source": "program", "mode": "current", "display": "Frames"},
            {"source": "program", "mode": "duration"},
            {"source": "program", "mode": "media"},
            {"source": "source", "mode": "current"},
        ]);
        let r = handle(&mut s, json!(1), "time.readout", json!({"rows": rows, "time": t.0})).expect("reply").json;
        let out = r["result"]["rows"].as_array().cloned().expect("rows");
        assert_eq!(out[0]["value"], json!("00:00:02:00"), "{r}");
        assert_eq!(out[0]["mode"], json!("Current Time"));
        assert_eq!(out[1]["value"], json!((2 * rate.timecode_base()).to_string()));
        assert_ne!(out[2]["value"], json!("--:--:--:--"));
        assert_ne!(out[3]["value"], json!("--:--:--:--"), "a clip is under the playhead at 2 s");
        assert_eq!(out[4]["value"], json!("--:--:--:--"), "nothing open in the Source monitor");
        let bad = handle(&mut s, json!(2), "time.readout", json!({"rows": [{"mode": "nope"}]})).expect("reply").json;
        assert_eq!(bad["ok"], json!(false));
    }

    #[test]
    fn audio_render_meters_what_it_plays() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let sr = s.active_sequence().map(|q| q.settings.sample_rate).expect("demo sequence");
        let chunk = (sr / 2) as usize;
        let ask = |s: &mut Session, id: i64, start: i64| {
            let r = handle(s, json!(id), "audio.render", json!({"start": start, "frames": chunk, "meter": true})).expect("reply");
            assert_eq!(r.json["ok"], json!(true), "{}", r.json);
            r.json["result"]["meter"].clone()
        };
        let start = i64::from(sr) * 10;
        let first = ask(&mut s, 1, start);
        assert_eq!(first["reset"], json!(true));
        assert_eq!(first["block"], json!(sr / 100));
        let peaks = first["peaks"].as_array().expect("peaks");
        assert_eq!(peaks.len(), 50, "one block per 1/100 s");
        let channels = peaks[0].as_array().map_or(0, Vec::len);
        assert!(channels >= 2);
        assert!(peaks.iter().flat_map(|b| b.as_array().into_iter().flatten()).all(|v| v.as_f64().is_some_and(|x| (0.0..=4.0).contains(&x))));
        assert!(peaks.iter().flat_map(|b| b.as_array().into_iter().flatten()).any(|v| v.as_f64().unwrap_or(0.0) > 0.001), "the demo plays sound at 10 s");
        // the next consecutive block keeps integrating; momentary loudness is there after 400 ms
        let second = ask(&mut s, 2, start + chunk as i64);
        assert_eq!(second["reset"], json!(false));
        let last = second["loudness"].as_array().and_then(|l| l.last()).cloned().expect("loudness");
        assert!(last[0].as_f64().is_some_and(|m| (-70.0..=0.0).contains(&m)), "momentary {last}");
        assert!(last[2].as_f64().is_some(), "integrated {last}");
        // the other monitor playing in between does not start this one's loudness over
        let tone = vec![vec![0.25_f32; 4800]; 2];
        assert_eq!(meter("source", sr, 0, &tone, 4800)["reset"], json!(true));
        assert_eq!(ask(&mut s, 5, start + 2 * chunk as i64)["reset"], json!(false));
        assert_eq!(meter("source", sr, 4800, &tone, 4800)["reset"], json!(false));
        // a jump starts over
        assert_eq!(ask(&mut s, 3, 0)["reset"], json!(true));
        // each mixer strip's peaks: the audio tracks and the Mix
        let strips = first["strips"].as_array().cloned().expect("strips");
        assert!(strips.len() >= 2, "{strips:?}");
        assert!(strips.iter().any(|v| v["id"].as_u64() == Some(u64::MAX)), "the Mix");
        // without `meter`, no meter
        let plain = handle(&mut s, json!(4), "audio.render", json!({"start": 0, "frames": 480})).expect("reply");
        assert!(plain.json["result"].get("meter").is_none());
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
    fn media_peaks_give_the_waveform_of_a_media_item() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let music = s
            .project
            .items
            .values()
            .find(|i| i.as_media().is_some_and(|m| m.info.video.is_none() && !m.info.audio_streams.is_empty()))
            .map(|i| i.id)
            .expect("demo music");
        let tps = cfa4_time::TICKS_PER_SECOND;
        let r = handle(&mut s, json!(1), "media.peaks", json!({"item": music.0, "start": 0, "end": 2 * tps, "buckets": 100})).expect("reply");
        assert_eq!(r.json["ok"], json!(true), "{}", r.json);
        let bytes = r.binary.expect("peaks");
        assert_eq!(bytes.len(), 100 * 2 * 4);
        let vals: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        assert!(vals.iter().all(|v| v.is_finite() && (-1.5..=1.5).contains(v)));
        assert!(vals.chunks_exact(2).all(|p| p[0] <= p[1]), "min <= max in every bucket");
        let first = handle(&mut s, json!(2), "media.peaks", json!({"item": music.0, "start": 0, "end": 2 * tps, "buckets": 100, "stream": 0})).expect("reply");
        assert_eq!(first.binary.as_deref(), Some(bytes.as_slice()), "stream 0 is the default");
        let bad = handle(&mut s, json!(3), "media.peaks", json!({"item": music.0, "start": 0, "end": 2 * tps, "buckets": 100, "stream": -1})).expect("reply");
        assert_eq!(bad.json["ok"], json!(false));
        assert!(vals.chunks_exact(2).any(|p| p[1] - p[0] > 0.001), "the demo score is not silent");
        for bad in [
            json!({"item": music.0, "start": 10, "end": 5, "buckets": 10}),
            json!({"item": music.0, "start": 0, "end": tps, "buckets": 0}),
            json!({"item": music.0, "start": 0, "end": 3600 * tps, "buckets": 10}),
            json!({"item": 987_654, "start": 0, "end": tps, "buckets": 10}),
        ] {
            let r = handle(&mut s, json!(2), "media.peaks", bad.clone()).expect("reply");
            assert_eq!(r.json["ok"], json!(false), "{bad}");
        }
        // the active sequence reports its targeting for the track headers
        let q = handle(&mut s, json!(3), "engine.execute", json!({"command": "sequence.inspect"})).expect("reply");
        let t = &q.json["result"]["targeting"];
        assert!(t["targeted"].as_array().is_some_and(|a| !a.is_empty()), "{t}");
        assert!(t["videoDest"].is_u64() && t["audioDest"].is_u64(), "{t}");
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
