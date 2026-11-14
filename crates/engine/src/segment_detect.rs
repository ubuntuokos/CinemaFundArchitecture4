//! Segment proposals from an analysis of a source (CFA4-LOSSLESS-MEDIA-TOOLS-001 1.1, K4 and K5):
//! scene changes, black pictures and silence of a media item, found without touching the
//! project; the user corrects the proposal and accepts it as range markers (the segments of
//! [`crate::segments`]).
//!
//! - `segments.detectScenes {"item"?, "in"?, "out"?, "sensitivity"=50, "minShotFrames"=6}`: one
//!   proposed segment per shot, with the detector of Scene Edit Detection ([`cfa4_render::scene`]).
//! - `segments.detectBlack {…, "pixelThreshold"=0.10, "pictureThreshold"=0.98, "minSeconds"=0.5}`:
//!   one per run of black pictures (a luma sample at most `pixelThreshold` of the black–white range
//!   is black; a picture is black when `pictureThreshold` of its samples are, as `blackdetect`).
//! - `segments.detectSilence {…, "thresholdDb"=-60, "minSeconds"=2}`: one per silent stretch
//!   (10 ms blocks whose loudest sample, over every channel, stays below `thresholdDb` dBFS).
//!
//! Every analysis covers the item's source span (a subclip's range), or `in`..`out` (media time)
//! inside it, and runs as a background job (`jobs.list` shows it, `jobs.cancel` stops it;
//! `"wait": true` runs it in the call). `segments.proposal {"job"}` returns the proposal when the
//! job is done. `segments.add {"item"?, "segments": [{"in", "out", "name"?, "comment"?, "color"?}]}`
//! writes the (corrected) proposal as range markers in one undo step; nothing else changes the
//! project, and the source file is never touched.

use std::sync::{Arc, Mutex};

use cfa4_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use cfa4_render::scene::{SceneDetector, SceneOptions};
use cfa4_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::clip_ops::source_view;
use crate::commands::{CommandSpec, always, bad, tick_or_time_p};
use crate::{EngineError, Result, Session};

/// Longest side of the pictures analysed (speed; the measures don't need detail).
const ANALYSIS_WIDTH: f64 = 256.0;
/// Silence is measured in blocks of this many seconds (1/100 s).
const BLOCKS_PER_SECOND: u32 = 100;
/// Finished proposals kept for `segments.proposal` (the oldest are dropped).
const KEEP: usize = 16;
/// More proposed segments than this are cut off (and the proposal says so).
pub(crate) const MAX_SEGMENTS: usize = crate::smart_cut_ranges::MAX_RANGES;

/// What an analysis looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanKind {
    Scenes,
    Black,
    Silence,
}

impl ScanKind {
    pub fn name(self) -> &'static str {
        match self {
            ScanKind::Scenes => "scenes",
            ScanKind::Black => "black",
            ScanKind::Silence => "silence",
        }
    }
    fn label(self) -> &'static str {
        match self {
            ScanKind::Scenes => "Scene",
            ScanKind::Black => "Black",
            ScanKind::Silence => "Silence",
        }
    }
    fn color(self) -> Label {
        match self {
            ScanKind::Scenes => Label::Green,
            ScanKind::Black => Label::Violet,
            ScanKind::Silence => Label::Cerulean,
        }
    }
}

/// The settings of one analysis.
#[derive(Clone, Copy, Debug)]
enum Settings {
    Scenes(SceneOptions),
    Black { pixel: f64, picture: f64, min: Tick },
    Silence { threshold_db: f64, min: Tick },
}

/// The proposal of a finished analysis: media time ranges, in time order.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub segments: Vec<(Tick, Tick)>,
    /// More segments were found than [`MAX_SEGMENTS`].
    pub truncated: bool,
}

/// One analysis (running or finished) and its proposal.
pub struct Scan {
    pub job: u64,
    pub item: ItemId,
    pub kind: ScanKind,
    pub span: (Tick, Tick),
    pub found: Arc<Mutex<Option<Found>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs of `true` at least `min` long: (first index, past-the-end index).
fn runs(flags: &[bool], min: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, f) in flags.iter().chain(std::iter::once(&false)).enumerate() {
        match (*f, start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => {
                if i - a >= min.max(1) {
                    out.push((a, i));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// The silent stretches, in samples, at least `min` samples long. Block `i` starts at
/// `s0 + i * block` and ends at `ends[i]`, so a short last block counts only the samples it has.
pub(crate) fn silent_spans(flags: &[bool], ends: &[i64], s0: i64, block: i64, min: i64) -> Vec<(i64, i64)> {
    runs(flags, 1)
        .into_iter()
        .filter_map(|(a, b)| {
            let start = s0.saturating_add((a as i64).saturating_mul(block));
            let end = *ends.get(b.checked_sub(1)?)?;
            (end.saturating_sub(start) >= min.max(1)).then_some((start, end))
        })
        .collect()
}

/// The share of black luma samples of an RGBA picture (Rec. 709 luma of the display values).
fn black_share(rgba: &[u8], pixel: f64) -> f64 {
    let limit = pixel.clamp(0.0, 1.0) * 255.0;
    let mut n = 0u64;
    let mut black = 0u64;
    for p in rgba.chunks_exact(4) {
        let y = 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]);
        n += 1;
        if y <= limit {
            black += 1;
        }
    }
    if n == 0 { 0.0 } else { black as f64 / n as f64 }
}

/// Frames `first..last` (frame indices at `rate`) of the analysed span.
fn frame_span(rate: FrameRate, (a, b): (Tick, Tick)) -> (i64, i64) {
    let first = rate.frame_at(a);
    let first = if rate.tick_of(first) < a { first.saturating_add(1) } else { first };
    let last = rate.frame_at(Tick(b.0.saturating_sub(1))).saturating_add(1);
    (first, last.max(first))
}

/// `a / b` rounded up, for `a >= 0`, `b > 0` (saturating).
fn ceil_div(a: i64, b: i64) -> i64 {
    let b = b.max(1);
    a.max(0) / b + i64::from(a.max(0) % b != 0)
}

/// Cap the proposal at [`MAX_SEGMENTS`].
fn capped(mut segments: Vec<(Tick, Tick)>) -> Found {
    let truncated = segments.len() > MAX_SEGMENTS;
    segments.truncate(MAX_SEGMENTS);
    Found { segments, truncated }
}

struct Work {
    src: cfa4_media::SharedSource,
    rate: FrameRate,
    span: (Tick, Tick),
    scale: f32,
    settings: Settings,
}

/// The analysis itself (on the job's thread). `tick(done, total)` reports progress and returns
/// false when the job was stopped.
fn analyse(w: &Work, tick: &mut dyn FnMut(u64, u64) -> bool) -> std::result::Result<Found, String> {
    match w.settings {
        Settings::Scenes(opt) => {
            let (first, last) = frame_span(w.rate, w.span);
            let total = (last - first) as u64;
            let mut det = SceneDetector::new();
            for (k, f) in (first..last).enumerate() {
                if !tick(k as u64, total) {
                    return Err("stopped".into());
                }
                let pic = w
                    .src
                    .video_frame(cfa4_media::FrameRequest { time: w.rate.tick_of(f), scale: w.scale })
                    .map_err(|e| format!("can't decode frame {f}: {e}"))?;
                det.push_rgba(&pic.to_rgba8(), pic.width as usize, pic.height as usize);
            }
            let mut bounds: Vec<i64> = vec![first];
            bounds.extend(det.cuts(&opt).into_iter().map(|i| first.saturating_add(i as i64)));
            bounds.push(last);
            bounds.dedup();
            let at = |f: i64| w.rate.tick_of(f).clamp(w.span.0, w.span.1);
            Ok(capped(bounds.windows(2).map(|b| (at(b[0]), at(b[1]))).filter(|(a, b)| b > a).collect()))
        }
        Settings::Black { pixel, picture, min } => {
            let (first, last) = frame_span(w.rate, w.span);
            let total = (last - first) as u64;
            let mut flags = Vec::new();
            for (k, f) in (first..last).enumerate() {
                if !tick(k as u64, total) {
                    return Err("stopped".into());
                }
                let pic = w
                    .src
                    .video_frame(cfa4_media::FrameRequest { time: w.rate.tick_of(f), scale: w.scale })
                    .map_err(|e| format!("can't decode frame {f}: {e}"))?;
                flags.push(black_share(&pic.to_rgba8(), pixel) >= picture);
            }
            let fd = w.rate.frame_duration().0.max(1);
            let min_frames = usize::try_from(ceil_div(min.0, fd)).unwrap_or(usize::MAX).max(1);
            let at = |i: usize| w.rate.tick_of(first.saturating_add(i as i64)).clamp(w.span.0, w.span.1);
            Ok(capped(runs(&flags, min_frames).into_iter().map(|(a, b)| (at(a), at(b))).filter(|(a, b)| b > a).collect()))
        }
        Settings::Silence { threshold_db, min } => {
            let info = w.src.info().audio().cloned().ok_or("the source has no audio")?;
            let sr = info.sample_rate;
            if sr == 0 {
                return Err("the source's audio has no sample rate".into());
            }
            let rate = i64::from(sr);
            let (s0, s1) = (w.span.0.to_units_floor(rate), w.span.1.to_units_floor(rate));
            let block = (sr / BLOCKS_PER_SECOND).max(1) as i64;
            let limit = 10f64.powf(threshold_db / 20.0) as f32;
            let total = u64::try_from(s1.saturating_sub(s0)).unwrap_or(0);
            let chunk = block.saturating_mul(i64::from(BLOCKS_PER_SECOND));
            let mut flags = Vec::new();
            // the sample where each block ends (the last one read may be short)
            let mut ends = Vec::new();
            let mut at = s0;
            while at < s1 {
                if !tick(u64::try_from(at - s0).unwrap_or(0), total) {
                    return Err("stopped".into());
                }
                let n = chunk.min(s1 - at);
                let buf = w.src.audio(at, n as usize, sr).map_err(|e| format!("can't decode the audio at sample {at}: {e}"))?;
                let len = buf.frames().min(n as usize);
                let mut k = 0;
                while k < len {
                    let end = (k + block as usize).min(len);
                    let peak = buf.channels.iter().flat_map(|c| c.get(k..end).unwrap_or(&[]).iter()).fold(0f32, |m, v| m.max(v.abs()));
                    flags.push(peak < limit);
                    ends.push(at + end as i64);
                    k = end;
                }
                if len < n as usize {
                    // a short read: the rest is not audio (no silence finding is made up)
                    break;
                }
                at += n;
            }
            let time = |x: i64| Tick::from_units(x, rate).clamp(w.span.0, w.span.1);
            let found = silent_spans(&flags, &ends, s0, block, min.to_units_floor(rate));
            Ok(capped(found.into_iter().map(|(a, b)| (time(a), time(b))).filter(|(a, b)| b > a).collect()))
        }
    }
}

fn settings_of(kind: ScanKind, p: &Value, cmd: &str) -> Result<Settings> {
    let num = |k: &str, lo: f64, hi: f64, default: f64| -> Result<f64> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => {
                v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)).ok_or_else(|| bad(cmd, format!("`{k}` must be a number from {lo} to {hi}")))
            }
        }
    };
    let seconds = |default: f64| -> Result<Tick> { Ok(Tick::from_seconds_f64(num("minSeconds", 0.0, 86_400.0, default)?)) };
    Ok(match kind {
        ScanKind::Scenes => Settings::Scenes(SceneOptions {
            sensitivity: num("sensitivity", 0.0, 100.0, 50.0)? as f32,
            min_shot_frames: num("minShotFrames", 1.0, 10_000.0, 6.0)? as usize,
        }),
        ScanKind::Black => {
            Settings::Black { pixel: num("pixelThreshold", 0.0, 1.0, 0.10)?, picture: num("pictureThreshold", 0.0, 1.0, 0.98)?, min: seconds(0.5)? }
        }
        ScanKind::Silence => Settings::Silence { threshold_db: num("thresholdDb", -120.0, 0.0, -60.0)?, min: seconds(2.0)? },
    })
}

/// The media item named by `item`, or the Source monitor's clip.
fn item_of(s: &Session, p: &Value, cmd: &str) -> Result<ItemId> {
    match p.get("item") {
        None | Some(Value::Null) => s.state.source_item.ok_or_else(|| bad(cmd, "`item` is required when no Source clip is open")),
        Some(_) => crate::commands::item_p(p, "item").ok_or_else(|| bad(cmd, "`item` must be a project item id")),
    }
}

/// The media span an item covers: a subclip's own range (also when its trims are not restricted
/// and the Source monitor shows the whole parent media), otherwise the item's.
fn bounds(view: &crate::clip_ops::SourceView) -> (Tick, Tick) {
    match view.subclip {
        Some((r, _)) => (r.start.max(Tick::ZERO), r.end()),
        None => (view.start.max(Tick::ZERO), view.end),
    }
}

/// The source span to analyse: the item's (a subclip's range), narrowed by `in` / `out`.
fn span_of(s: &Session, p: &Value, item: ItemId, cmd: &str) -> Result<(crate::clip_ops::SourceView, (Tick, Tick))> {
    let view = source_view(s, item).ok_or_else(|| bad(cmd, format!("no media item {}", item.0)))?;
    let (lo, hi) = bounds(&view);
    let a = tick_or_time_p(p, "in", view.rate, false, cmd)?.unwrap_or(lo).clamp(lo, hi);
    let b = tick_or_time_p(p, "out", view.rate, false, cmd)?.unwrap_or(hi).clamp(lo, hi);
    if b <= a {
        return Err(bad(cmd, "the span to analyse is empty (`out` must come after `in`, inside the source)"));
    }
    Ok((view, (a, b)))
}

fn start(s: &mut Session, p: &Value, kind: ScanKind, cmd: &str) -> Result<Value> {
    let settings = settings_of(kind, p, cmd)?;
    let item = item_of(s, p, cmd)?;
    let (view, span) = span_of(s, p, item, cmd)?;
    let src = s.source(view.media).ok_or_else(|| bad(cmd, format!("item {} has no media to analyse (offline?)", item.0)))?;
    let info = src.info().clone();
    let scale = match (kind, &info.video) {
        (ScanKind::Silence, _) => 1.0,
        (_, Some(v)) => (ANALYSIS_WIDTH / f64::from(v.width.max(1))).min(1.0) as f32,
        (_, None) => return Err(bad(cmd, format!("item {} has no video to analyse", item.0))),
    };
    if kind == ScanKind::Silence && !info.has_audio() {
        return Err(bad(cmd, format!("item {} has no audio to analyse", item.0)));
    }
    let rate = if view.rate.frame_duration() > Tick::ZERO { view.rate } else { info.frame_rate() };
    let work = Work { src, rate, span, scale, settings };
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let name = s.project.item(item).map(|i| i.name.clone()).unwrap_or_default();
    let label = format!(
        "Find {} ({name})",
        match kind {
            ScanKind::Scenes => "Scenes",
            ScanKind::Black => "Black",
            ScanKind::Silence => "Silence",
        }
    );
    let job = crate::Job { id, label, progress: Default::default(), result: Default::default() };
    let found: Arc<Mutex<Option<Found>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), found.clone());
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let mut done_units = 0;
        let mut tick = |done: u64, total: u64| {
            prog.total.store(total, Ordering::Relaxed);
            prog.done.store(done, Ordering::Relaxed);
            done_units = done;
            !prog.cancel.load(Ordering::Relaxed)
        };
        let r = analyse(&work, &mut tick);
        let secs = t0.elapsed().as_secs_f64();
        *lock(&prog.status) = match &r {
            Ok(f) => format!("Found {} segment(s) ({secs:.1}s)", f.segments.len()),
            Err(e) if e == "stopped" => "Stopped".into(),
            Err(e) => e.clone(),
        };
        let report = r.map(|f| {
            *lock(&out) = Some(f);
            cfa4_export::Report { path: String::new(), frames: done_units, seconds: secs, bytes: 0, render_fps: 0.0, extra_files: Vec::new() }
        });
        *lock(&res) = Some(report);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.segment_scans.push(Scan { job: id, item, kind, span, found });
    // keep the last KEEP analyses, but never one still running (its result would be lost)
    let running = |s: &Session, job: u64| s.jobs.iter().any(|j| j.id == job && !j.progress.finished.load(std::sync::atomic::Ordering::Relaxed));
    let mut over = s.segment_scans.len().saturating_sub(KEEP);
    let mut i = 0;
    while over > 0 && i < s.segment_scans.len() {
        if running(s, s.segment_scans[i].job) {
            i += 1;
        } else {
            s.segment_scans.remove(i);
            over -= 1;
        }
    }
    if crate::commands::bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        return proposal_of(s, id, cmd);
    }
    if let Err(e) = std::thread::Builder::new().name("cfa4-segment-scan".into()).spawn(run) {
        // no thread will ever finish this analysis: leave no running job behind
        s.jobs.retain(|j| j.id != id);
        s.segment_scans.retain(|x| x.job != id);
        return Err(EngineError::Other(e.to_string()));
    }
    Ok(json!({"job": id, "item": item.0, "kind": kind.name(), "state": "running"}))
}

/// The state of an analysis and, when it is done, its proposal.
fn proposal_of(s: &Session, job: u64, cmd: &str) -> Result<Value> {
    use std::sync::atomic::Ordering;
    let scan = s.segment_scans.iter().find(|x| x.job == job).ok_or_else(|| bad(cmd, format!("no segment analysis {job} (only the last {KEEP} are kept)")))?;
    let j = s.jobs.iter().find(|j| j.id == job);
    let mut out = json!({"job": job, "item": scan.item.0, "kind": scan.kind.name(), "in": scan.span.0.0, "out": scan.span.1.0});
    if let Some(j) = j {
        out["progress"] = json!({"done": j.progress.done.load(Ordering::Relaxed), "total": j.progress.total.load(Ordering::Relaxed)});
    }
    let finished = j.is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
    if !finished {
        out["state"] = json!("running");
        return Ok(out);
    }
    if let Some(f) = lock(&scan.found).as_ref() {
        let segments: Vec<Value> = f
            .segments
            .iter()
            .enumerate()
            .map(|(n, (a, b))| json!({"in": a.0, "out": b.0, "duration": b.0 - a.0, "name": format!("{} {:02}", scan.kind.label(), n + 1), "color": scan.kind.color().name()}))
            .collect();
        out["state"] = json!("done");
        out["segments"] = Value::Array(segments);
        out["truncated"] = json!(f.truncated);
        return Ok(out);
    }
    let error = j.and_then(|j| lock(&j.result).clone()).and_then(|r| r.err()).unwrap_or_else(|| "the analysis gave no result".into());
    out["state"] = json!(if error == "stopped" { "stopped" } else { "failed" });
    out["error"] = json!(error);
    Ok(out)
}

fn scenes(s: &mut Session, p: &Value) -> Result<Value> {
    start(s, p, ScanKind::Scenes, "segments.detectScenes")
}
fn black(s: &mut Session, p: &Value) -> Result<Value> {
    start(s, p, ScanKind::Black, "segments.detectBlack")
}
fn silence(s: &mut Session, p: &Value) -> Result<Value> {
    start(s, p, ScanKind::Silence, "segments.detectSilence")
}

fn proposal(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "segments.proposal";
    let job = p.get("job").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`job` must be the id an analysis returned"))?;
    proposal_of(s, job, cmd)
}

/// `segments.add`: the (corrected) proposal as range markers of the item's media, one undo step.
fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "segments.add";
    let item = item_of(s, p, cmd)?;
    let view = source_view(s, item).ok_or_else(|| bad(cmd, format!("no media item {}", item.0)))?;
    let list = p
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(cmd, "`segments` must be a list of {\"in\", \"out\", \"name\"?, \"comment\"?, \"color\"?}"))?;
    if list.is_empty() {
        return Err(bad(cmd, "`segments` is empty"));
    }
    if list.len() > MAX_SEGMENTS {
        return Err(bad(cmd, format!("at most {MAX_SEGMENTS} segments per command")));
    }
    let (lo, hi) = bounds(&view);
    let mut planned = Vec::with_capacity(list.len());
    for (i, v) in list.iter().enumerate() {
        let n = i + 1;
        if !v.is_object() {
            return Err(bad(cmd, format!("segment {n}: must be an object")));
        }
        let a = tick_or_time_p(v, "in", view.rate, false, cmd)?.ok_or_else(|| bad(cmd, format!("segment {n}: `in` is required")))?;
        let b = tick_or_time_p(v, "out", view.rate, false, cmd)?.ok_or_else(|| bad(cmd, format!("segment {n}: `out` is required")))?;
        let (a, b) = (a.clamp(lo, hi), b.clamp(lo, hi));
        if b <= a {
            return Err(bad(cmd, format!("segment {n}: `out` must come after `in`, inside the source")));
        }
        let text = |k: &str| -> Result<String> {
            match v.get(k) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(t)) => Ok(t.chars().take(1000).collect()),
                Some(_) => Err(bad(cmd, format!("segment {n}: `{k}` must be a string"))),
            }
        };
        let color = crate::commands::marker_color_p(v, cmd)?.unwrap_or(Label::Green);
        planned.push((a, b, text("name")?, text("comment")?, color));
    }
    let media = view.media;
    let ids = s.edit("Add Segments", |pr, _| {
        if pr.next_id.checked_add(planned.len() as u64).is_none() {
            return Err(bad(cmd, "project ids are exhausted"));
        }
        let ids: Vec<u64> = planned.iter().map(|_| pr.alloc_id()).collect();
        let m = pr.item_mut(media).and_then(|i| i.as_media_mut()).ok_or_else(|| bad(cmd, format!("no media item {}", media.0)))?;
        let mut added = Vec::new();
        for ((a, b, name, comment, color), id) in planned.iter().zip(ids) {
            // the same segment twice is kept once
            if m.markers.iter().any(|x| x.start == *a && x.duration == *b - *a) {
                continue;
            }
            m.markers.push(Marker {
                id: MarkerId(id),
                start: *a,
                duration: *b - *a,
                name: name.clone(),
                comment: comment.clone(),
                kind: MarkerKind::Comment,
                color: *color,
            });
            added.push(id);
        }
        m.markers.sort_by_key(|x| (x.start, x.id.0));
        Ok(added)
    })?;
    Ok(json!({"item": item.0, "markers": ids, "skipped": planned.len() - ids.len()}))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "segments.detectScenes",
            label: "Find Scenes…",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id? (default: the Source clip),"in":media time?,"out":media time?,"sensitivity":0..100=50,"minShotFrames":1..10000=6,"wait":bool=false} → {job, state, …}: one proposed segment per shot (segments.proposal)"#,
            enabled: always,
            run: scenes,
            journal: false,
        },
        CommandSpec {
            id: "segments.detectBlack",
            label: "Find Black…",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"in":media time?,"out":media time?,"pixelThreshold":0..1=0.10,"pictureThreshold":0..1=0.98,"minSeconds":n=0.5,"wait":bool=false} → {job, state, …}: one proposed segment per run of black pictures"#,
            enabled: always,
            run: black,
            journal: false,
        },
        CommandSpec {
            id: "segments.detectSilence",
            label: "Find Silence…",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"in":media time?,"out":media time?,"thresholdDb":-120..0=-60,"minSeconds":n=2,"wait":bool=false} → {job, state, …}: one proposed segment per silent stretch (10 ms blocks below the threshold on every channel)"#,
            enabled: always,
            run: silence,
            journal: false,
        },
        CommandSpec {
            id: "segments.proposal",
            label: "Segment Proposal",
            menu: &[],
            shortcut: None,
            params: r#"{"job":id} → {job, item, kind, in, out, state:"running"|"done"|"stopped"|"failed", progress?, segments?:[{in, out, duration, name, color}], truncated?, error?}"#,
            enabled: always,
            run: proposal,
            journal: false,
        },
        CommandSpec {
            id: "segments.add",
            label: "Add Segments",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"segments":[{"in":media time,"out":media time,"name":str?,"comment":str?,"color":label?}]} → {item, markers:[id], skipped} (range markers of the item's media, one undo step; a segment already marked is skipped)"#,
            enabled: always,
            run: add,
            journal: true,
        },
    ]
}
