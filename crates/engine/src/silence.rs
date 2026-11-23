//! Remove Silence (F6 / V1-A, plan CFA4-V1-SILENCE-REMOVAL-001): find the silent stretches of
//! audio clips without a transcript, show them as editable markers, and delete only what the user
//! left selected.
//!
//! | command | what it does |
//! |---|---|
//! Both deleting-capable entry points have no engine menu entry: the Qt6 dialog (Tools ▸ Audio
//! Tools ▸ Remove Silence…) shows the regions first, so a menu click can never delete blindly.
//!
//! | `audio.detectSpeech` | measures the audio clips and returns the silent regions (`mode: "query"`) or also puts them on the sequence as range markers (`mode: "preview"`, the default). **Never deletes.** |
//! | `silence.mark` | with `background: true` the analysis runs as a cancellable job (`jobs.list`, `jobs.cancel`); this puts its finished result on the sequence as markers |
//! | `silence.regions` | lists the silence markers (id, start, duration, kept or not, clip) |
//! | `silence.clear` | removes the silence markers (nothing else) |
//! | `silence.keep` | marks silence markers as kept (the removal skips them) or not |
//! | `clip.removeSilence` | deletes the silence markers' ranges (`mode: "cut"`, ripple by default, or lift with `ripple: false`) in one undo step; `mode: "mark"` deletes nothing. Refuses to run without silence markers, so it never deletes blindly. |
//!
//! A region is a range marker whose comment starts with [`SILENCE_COMMENT`]; the kept flag is
//! [`KEEP_FLAG`] in that comment, so both live in the project file without a new field, and the
//! ordinary marker commands move and resize a region. Analysing the same clips again replaces
//! the silence markers inside their spans (one undo step), other markers are never touched.
//!
//! With several clips a region is the time where **every** analysed clip that is present is
//! silent (the intersection), so speech on another track is never cut. The energy detector
//! ([`cfa4_speech::vad`]) is not an AI feature (plan D1); the speech-model detector (`silero`)
//! comes with the AI layer and is refused until then. Speed-changed, reversed and remixed clips
//! are refused (their audio is not the source read at 1×).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use cfa4_edit::transcript::{self as tx, merge_ranges};
use cfa4_project::{ClipId, Label, Marker, MarkerId, MarkerKind, TrackKind};
use cfa4_speech::vad::{self, FRAME_SAMPLES, SilenceParams};
use cfa4_time::{TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// The comment every silence region marker starts with (how they are found again).
pub const SILENCE_COMMENT: &str = "Silence region";
/// Appended to the comment of a region the removal must skip.
pub const KEEP_FLAG: &str = " [silence:keep]";

const CMD_DETECT: &str = "audio.detectSpeech";
const CMD_MARK: &str = "silence.mark";
const CMD_KEEP: &str = "silence.keep";
const CMD_REMOVE: &str = "clip.removeSilence";
/// Audio is read in pieces of this many seconds (a whole number of analysis frames).
const CHUNK_SECONDS: usize = 30;

fn is_silence(m: &Marker) -> bool {
    m.comment.starts_with(SILENCE_COMMENT)
}

fn is_kept(m: &Marker) -> bool {
    m.comment.contains(KEEP_FLAG.trim_start())
}

fn has_silence_markers(s: &Session) -> std::result::Result<(), String> {
    let seq = s.active_sequence().ok_or("no sequence is open")?;
    if seq.markers.iter().any(is_silence) { Ok(()) } else { Err("the sequence has no silence regions; analyse first".into()) }
}

/// An analysed clip: where it sits on the sequence and its silent ranges there.
struct Analysed {
    id: ClipId,
    span: (Tick, Tick),
    silent: Vec<(Tick, Tick)>,
}

/// Why a clip's audio cannot be analysed (`None`: it can).
fn unsupported(s: &Session, c: ClipId) -> Option<String> {
    let seq = s.active_sequence()?;
    let Some((t, it)) = seq.find_item(c) else { return Some("no such clip".into()) };
    if seq.track(t).is_none_or(|t| t.kind != TrackKind::Audio) {
        return Some("not on an audio track".into());
    }
    match s.project.item(it.item) {
        Some(item) if matches!(item.kind, cfa4_project::ItemKind::Media(_)) && item.has_audio() => {}
        _ => return Some("it has no audio media".into()),
    }
    if it.reverse || it.speed < 0.0 {
        return Some("it plays reversed".into());
    }
    if (it.speed - 1.0).abs() > 1e-9 || cfa4_render::remix::is_remixed(it) {
        return Some("its speed is changed or it is remixed".into());
    }
    None
}

/// The clips to analyse: the explicit `clips`, else by `scope`. The second value counts the
/// clips of a scope that were left out because they cannot be analysed.
fn pick_clips(s: &Session, p: &Value) -> Result<(Vec<ClipId>, usize)> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if let Some(list) = p.get("clips") {
        let list = list.as_array().ok_or_else(|| bad(CMD_DETECT, "`clips` must be a list of clip ids"))?;
        let mut out = Vec::new();
        for v in list {
            let id = ClipId(v.as_u64().ok_or_else(|| bad(CMD_DETECT, "`clips` must be a list of clip ids"))?);
            if let Some(why) = unsupported(s, id) {
                return Err(bad(CMD_DETECT, format!("clip {}: {why}", id.0)));
            }
            if !out.contains(&id) {
                out.push(id);
            }
        }
        return Ok((out, 0));
    }
    let candidates: Vec<ClipId> = match str_p(p, "scope").unwrap_or("selection") {
        "selection" => s
            .state
            .selection
            .iter()
            .copied()
            .filter(|c| seq.find_item(*c).and_then(|(t, _)| seq.track(t)).is_some_and(|t| t.kind == TrackKind::Audio))
            .collect(),
        "track" => {
            // `track`, or else every audio track that holds a selected clip
            let tids: Vec<u64> = match p.get("track") {
                None | Some(Value::Null) => {
                    seq.audio_tracks.iter().filter(|t| t.items.iter().any(|i| s.state.selection.contains(&i.id))).map(|t| t.id.0).collect()
                }
                Some(v) => vec![v.as_u64().ok_or_else(|| bad(CMD_DETECT, "`track` must be a track id"))?],
            };
            if tids.is_empty() {
                return Err(bad(CMD_DETECT, "`scope: track` needs `track`, or a selected audio clip"));
            }
            let mut out = Vec::new();
            for tid in tids {
                let t = seq.audio_tracks.iter().find(|t| t.id.0 == tid).ok_or_else(|| bad(CMD_DETECT, "no such audio track"))?;
                out.extend(t.items.iter().map(|i| i.id));
            }
            out
        }
        "sequence" => seq.audio_tracks.iter().flat_map(|t| t.items.iter().map(|i| i.id)).collect(),
        other => return Err(bad(CMD_DETECT, format!("unknown scope `{other}`: use selection, track or sequence"))),
    };
    let total = candidates.len();
    let ok: Vec<ClipId> = candidates.into_iter().filter(|c| unsupported(s, *c).is_none()).collect();
    if ok.is_empty() {
        return Err(bad(
            CMD_DETECT,
            if total == 0 {
                "select audio clips (or pass `clips`, `track` or `scope`)"
            } else {
                "none of the clips can be analysed (they need audio and 1× forward speed)"
            },
        ));
    }
    let skipped = total - ok.len();
    Ok((ok, skipped))
}

/// A tick parameter: absent is `None`; negative or non-integer values are refused.
fn ticks_p(p: &Value, k: &str) -> Result<Option<Tick>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_i64() {
            Some(n) if n >= 0 => Ok(Some(Tick(n))),
            _ => Err(bad(CMD_DETECT, format!("`{k}` must be a whole number of ticks, zero or more"))),
        },
    }
}

fn params(p: &Value) -> Result<SilenceParams> {
    let mut sp = SilenceParams::default();
    match p.get("thresholdDb") {
        None | Some(Value::Null) => {}
        Some(v) => match v.as_f64() {
            Some(db) if db.is_finite() && (-120.0..=0.0).contains(&db) => sp.threshold_db = Some(db as f32),
            _ => return Err(bad(CMD_DETECT, "`thresholdDb` must be between -120 and 0")),
        },
    }
    if let Some(t) = ticks_p(p, "minSilence")? {
        sp.min_silence = t;
    }
    if let Some(t) = ticks_p(p, "padding")? {
        sp.padding = t;
    }
    Ok(sp)
}

/// What decides which frames are speech.
#[derive(Clone)]
enum Detector {
    /// Level against a threshold (no model).
    Energy,
    /// The Silero VAD model (feature `silero`).
    #[cfg(feature = "silero")]
    Silero(std::sync::Arc<cfa4_speech::vad::silero::SileroVad>),
}

/// The Silero detector from the model store. Never falls back to another detector: a missing,
/// modified or unsupported model is an error that tells the user what to do.
fn silero_detector() -> Result<Detector> {
    #[cfg(feature = "silero")]
    {
        let dir = crate::transcript::models_dir().ok_or_else(|| EngineError::Other("no data directory for speech models".into()))?;
        match cfa4_speech::vad::silero::SileroVad::load(&dir) {
            Ok(v) => Ok(Detector::Silero(std::sync::Arc::new(v))),
            Err(cfa4_speech::SpeechError::NotInstalled(_)) => Err(bad(
                CMD_DETECT,
                "the Silero VAD model is not downloaded: see `speech.vadModel`, accept its licence and run `speech.downloadVadModel`, or use detector `energy`",
            )),
            Err(e) => Err(EngineError::Other(e.to_string())),
        }
    }
    #[cfg(not(feature = "silero"))]
    {
        Err(bad(CMD_DETECT, "this build has no speech-model detector (built without the `silero` feature); use `energy`"))
    }
}

/// What the analysis of one clip needs, so it can run on a thread that has no session.
#[derive(Clone)]
struct ClipRead {
    id: ClipId,
    item: cfa4_project::ItemId,
    stream: usize,
    /// First sample (16 kHz) of the clip's audio in its source.
    m0: i64,
    /// Samples (16 kHz) the clip plays.
    total: usize,
    start: Tick,
    duration: Tick,
}

fn read_info(s: &Session, c: ClipId) -> Result<ClipRead> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(CMD_DETECT, "no such clip"))?;
    let sr = i64::from(cfa4_speech::SAMPLE_RATE);
    Ok(ClipRead {
        id: c,
        item: it.item,
        stream: it.audio_stream,
        m0: it.source_in.to_units_floor(sr),
        total: it.duration.to_units_floor(sr).max(0) as usize,
        start: it.start,
        duration: it.duration,
    })
}

/// Measure one clip: its audio (16 kHz mono, 10 ms frames) is read in pieces, so a long clip is
/// never held whole, and `cancel` is looked at between pieces. `done` counts the samples read.
fn analyse(
    provider: &dyn cfa4_render::SourceProvider,
    r: &ClipRead,
    sp: &SilenceParams,
    detector: &Detector,
    cancel: &AtomicBool,
    done: &AtomicU64,
) -> Result<Analysed> {
    let sr = cfa4_speech::SAMPLE_RATE;
    let src = provider.source(r.item).ok_or_else(|| EngineError::Other("the clip's media is offline".into()))?;
    let chunk = CHUNK_SECONDS * sr as usize / FRAME_SAMPLES * FRAME_SAMPLES;
    let mut db = Vec::with_capacity(r.total / FRAME_SAMPLES + 1);
    #[cfg(feature = "silero")]
    let mut stream = match detector {
        Detector::Silero(v) => Some((v.stream(), Vec::<f32>::new())),
        Detector::Energy => None,
    };
    #[cfg(not(feature = "silero"))]
    let _ = detector;
    let mut at = 0usize;
    while at < r.total {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Other("cancelled".into()));
        }
        let n = chunk.min(r.total - at);
        let buf =
            src.audio_stream(r.stream, r.m0.saturating_add(at as i64), n, sr).map_err(|e| EngineError::Other(format!("reading the clip's audio: {e}")))?;
        let chans = buf.channels.len().max(1) as f32;
        let mut mono = vec![0.0f32; n];
        for ch in &buf.channels {
            for (m, v) in mono.iter_mut().zip(ch) {
                *m += *v / chans;
            }
        }
        #[cfg(feature = "silero")]
        if let Some((st, probs)) = stream.as_mut() {
            probs.extend(st.push(&mono).map_err(|e| EngineError::Other(e.to_string()))?);
        } else {
            db.extend(vad::frame_db(&mono));
        }
        #[cfg(not(feature = "silero"))]
        db.extend(vad::frame_db(&mono));
        at += n;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    #[cfg(feature = "silero")]
    let regions = match stream.as_mut() {
        Some((st, probs)) => {
            probs.extend(st.finish().map_err(|e| EngineError::Other(e.to_string()))?);
            vad::silence_regions_from_voiced(&cfa4_speech::vad::silero::voiced(probs), cfa4_speech::vad::silero::WINDOW, r.total, sp)
        }
        None => vad::silence_regions_from_db(&db, r.total, sp),
    };
    #[cfg(not(feature = "silero"))]
    let regions = vad::silence_regions_from_db(&db, r.total, sp);
    let silent = regions.iter().map(|(a, d)| (r.start + *a, r.start + *a + *d)).collect();
    Ok(Analysed { id: r.id, span: (r.start, r.start + r.duration), silent })
}

/// The time where every analysed clip that is present is silent, snapped inwards to the
/// sequence's frame grid (so a region never reaches into speech).
fn intersect(clips: &[Analysed], rate: cfa4_time::FrameRate) -> Vec<(Tick, Tick)> {
    let mut points: Vec<Tick> = clips.iter().flat_map(|c| [c.span.0, c.span.1].into_iter().chain(c.silent.iter().flat_map(|r| [r.0, r.1]))).collect();
    points.sort();
    points.dedup();
    let mut out: Vec<(Tick, Tick)> = Vec::new();
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        let covering = clips.iter().filter(|c| c.span.0 <= p && q <= c.span.1).count();
        let silent = clips.iter().filter(|c| c.span.0 <= p && q <= c.span.1 && c.silent.iter().any(|r| r.0 <= p && q <= r.1)).count();
        if covering > 0 && covering == silent {
            match out.last_mut() {
                Some(l) if l.1 == p => l.1 = q,
                _ => out.push((p, q)),
            }
        }
    }
    let frame = rate.tick_of(1).0.max(1);
    out.into_iter()
        .filter_map(|(a, b)| {
            let mut a2 = rate.snap(a);
            if a2 < a {
                a2 = rate.snap(Tick(a.0.saturating_add(frame)));
            }
            let b2 = rate.snap(b);
            (b2 > a2).then_some((a2, b2))
        })
        .collect()
}

fn detect(s: &mut Session, p: &Value) -> Result<Value> {
    let detector = match str_p(p, "detector").unwrap_or("energy") {
        "energy" => Detector::Energy,
        "silero" => silero_detector()?,
        other => return Err(bad(CMD_DETECT, format!("unknown detector `{other}`: use energy or silero"))),
    };
    let preview = match str_p(p, "mode").unwrap_or("preview") {
        "preview" => true,
        "query" => false,
        other => return Err(bad(CMD_DETECT, format!("unknown mode `{other}`: use preview or query"))),
    };
    let sp = params(p)?;
    let (ids, skipped) = pick_clips(s, p)?;
    let reads = ids.iter().map(|c| read_info(s, *c)).collect::<Result<Vec<_>>>()?;
    if bool_p(p, "background").unwrap_or(false) {
        return start_job(s, reads, sp, detector, skipped);
    }
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let (never, done) = (AtomicBool::new(false), AtomicU64::new(0));
    let analysed = reads.iter().map(|r| analyse(&provider, r, &sp, &detector, &never, &done)).collect::<Result<Vec<_>>>()?;
    finish(s, &analysed, preview, skipped)
}

/// The regions of the analysed clips, and with `preview` the markers for them.
fn finish(s: &mut Session, analysed: &[Analysed], preview: bool, skipped: usize) -> Result<Value> {
    let regions = intersect(analysed, s.sequence_rate());
    let total: i64 = regions.iter().map(|(a, b)| b.0 - a.0).sum();
    let clip = if analysed.len() == 1 { analysed.first().map(|a| a.id.0) } else { None };
    if !preview {
        let list: Vec<Value> = regions.iter().map(|(a, b)| json!({"start": a.0, "duration": b.0 - a.0, "keep": false})).collect();
        return Ok(
            json!({"regions": list, "clips": analysed.len(), "skipped": skipped, "ticks": total, "seconds": total as f64 / TICKS_PER_SECOND as f64, "clip": clip}),
        );
    }
    let spans: Vec<(Tick, Tick)> = analysed.iter().map(|a| a.span).collect();
    let made = s.edit_sequence("Detect Silence", |q, ctx, _| {
        q.markers.retain(|m| !(is_silence(m) && spans.iter().any(|sp| m.start >= sp.0 && m.start < sp.1)));
        let mut made = Vec::with_capacity(regions.len());
        for (k, (a, b)) in regions.iter().enumerate() {
            let id = MarkerId(ctx.alloc());
            q.markers.push(Marker {
                id,
                start: *a,
                duration: *b - *a,
                name: format!("Silence {}", k + 1),
                comment: SILENCE_COMMENT.into(),
                kind: MarkerKind::Comment,
                color: Label::Rose,
            });
            made.push(json!({"id": id.0, "start": a.0, "duration": b.0 - a.0, "keep": false}));
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(made)
    })?;
    Ok(json!({"regions": made, "clips": analysed.len(), "skipped": skipped, "ticks": total, "seconds": total as f64 / TICKS_PER_SECOND as f64, "clip": clip}))
}

/// Finished background analyses, by job id, until `silence.mark` takes them.
fn mailbox() -> &'static Mutex<HashMap<u64, Vec<Analysed>>> {
    static M: OnceLock<Mutex<HashMap<u64, Vec<Analysed>>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// `audio.detectSpeech` with `background: true`: the same analysis as a cancellable job
/// (`jobs.list` shows its progress, `jobs.cancel` stops it between pieces of audio). The session
/// is not touched; `silence.mark {job}` then puts the result on the sequence.
fn start_job(s: &mut Session, reads: Vec<ClipRead>, sp: SilenceParams, detector: Detector, skipped: usize) -> Result<Value> {
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let n = reads.len();
    let job = crate::Job { id, label: format!("Detect Silence ({n} clips)"), progress: Default::default(), result: Default::default() };
    let total: usize = reads.iter().map(|r| r.total).sum();
    job.progress.total.store(total.max(1) as u64, Ordering::Relaxed);
    let (prog, res) = (job.progress.clone(), job.result.clone());
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let work = move || {
        let t0 = web_time::Instant::now();
        let outcome: std::result::Result<Vec<Analysed>, String> =
            reads.iter().map(|r| analyse(&provider, r, &sp, &detector, &prog.cancel, &prog.done).map_err(|e| e.to_string())).collect();
        let r = outcome.map(|a| {
            let found = a.iter().map(|c| c.silent.len() as u64).sum();
            mailbox().lock().unwrap_or_else(PoisonError::into_inner).insert(id, a);
            *prog.status.lock().unwrap_or_else(PoisonError::into_inner) = format!("{n} clips analysed");
            cfa4_export::Report {
                path: format!("silence-analysis:{id}"),
                frames: found,
                seconds: t0.elapsed().as_secs_f64(),
                bytes: 0,
                render_fps: 0.0,
                extra_files: Vec::new(),
            }
        });
        if let Err(e) = &r {
            *prog.status.lock().unwrap_or_else(PoisonError::into_inner) = e.clone();
            *prog.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.clone());
        }
        prog.finished.store(true, Ordering::Relaxed);
        *res.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
    };
    let work = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), work);
    s.jobs.push(job);
    std::thread::Builder::new().name("cfa4-silence".into()).spawn(work).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(json!({"job": id, "clips": n, "skipped": skipped}))
}

/// `silence.mark {job}`: the result of a finished background analysis as silence markers (one
/// undo step), exactly what `audio.detectSpeech` does in `preview` mode.
fn mark(s: &mut Session, p: &Value) -> Result<Value> {
    let id = p.get("job").and_then(Value::as_u64).ok_or_else(|| bad(CMD_MARK, "`job` must be the id `audio.detectSpeech` returned"))?;
    let state = s.jobs.iter().find(|j| j.id == id).map(|j| j.result.lock().unwrap_or_else(PoisonError::into_inner).clone());
    match state {
        None => return Err(bad(CMD_MARK, "no such job")),
        Some(None) => return Err(bad(CMD_MARK, "the analysis is still running")),
        Some(Some(Err(e))) => return Err(bad(CMD_MARK, e)),
        Some(Some(Ok(_))) => {}
    }
    let analysed = mailbox().lock().unwrap_or_else(PoisonError::into_inner).remove(&id).ok_or_else(|| bad(CMD_MARK, "the result was already taken"))?;
    finish(s, &analysed, true, 0)
}

fn regions(s: &mut Session, _: &Value) -> Result<Value> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let clip_at = |t: Tick| seq.audio_tracks.iter().flat_map(|tr| tr.items.iter()).find(|i| i.start <= t && t < i.start + i.duration).map(|i| i.id.0);
    let list: Vec<Value> = seq
        .markers
        .iter()
        .filter(|m| is_silence(m))
        .map(|m| json!({"id": m.id.0, "start": m.start.0, "duration": m.duration.0, "keep": is_kept(m), "clip": clip_at(m.start)}))
        .collect();
    let total: i64 = seq.markers.iter().filter(|m| is_silence(m) && !is_kept(m)).map(|m| m.duration.0.max(0)).sum();
    Ok(json!({"regions": list, "ticks": total, "seconds": total as f64 / TICKS_PER_SECOND as f64}))
}

fn keep(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<u64> = p
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(CMD_KEEP, "`ids` must be a list of silence region ids"))?
        .iter()
        .map(|v| v.as_u64().ok_or_else(|| bad(CMD_KEEP, "`ids` must be a list of silence region ids")))
        .collect::<Result<_>>()?;
    let flag = bool_p(p, "keep").ok_or_else(|| bad(CMD_KEEP, "`keep` must be true or false"))?;
    {
        let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
        for id in &ids {
            if !seq.markers.iter().any(|m| m.id.0 == *id && is_silence(m)) {
                return Err(bad(CMD_KEEP, format!("{id} is not a silence region")));
            }
        }
    }
    let n = s.edit_sequence(if flag { "Keep Silence Regions" } else { "Unkeep Silence Regions" }, |q, _, _| {
        let mut n = 0;
        for m in q.markers.iter_mut().filter(|m| ids.contains(&m.id.0) && is_silence(m)) {
            let has = is_kept(m);
            if flag && !has {
                m.comment.push_str(KEEP_FLAG);
                n += 1;
            } else if !flag && has {
                m.comment = m.comment.replace(KEEP_FLAG, "");
                n += 1;
            }
        }
        Ok(n)
    })?;
    Ok(json!({"changed": n}))
}

fn clear(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.edit_sequence("Clear Silence Markers", |q, _, _| {
        let before = q.markers.len();
        q.markers.retain(|m| !is_silence(m));
        Ok(before - q.markers.len())
    })?;
    Ok(json!({"removed": n}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let ripple = bool_p(p, "ripple").unwrap_or(true);
    let keep_markers = bool_p(p, "keepMarkers").unwrap_or(false);
    let cut = match str_p(p, "mode").unwrap_or("cut") {
        "cut" => true,
        "mark" => false,
        other => return Err(bad(CMD_REMOVE, format!("unknown mode `{other}`: use cut or mark"))),
    };
    let wanted: Option<Vec<u64>> = match p.get("regions") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t == "markers" => None,
        Some(Value::Array(a)) => {
            Some(a.iter().map(|v| v.as_u64().ok_or_else(|| bad(CMD_REMOVE, "`regions` must be \"markers\" or a list of ids"))).collect::<Result<_>>()?)
        }
        Some(_) => return Err(bad(CMD_REMOVE, "`regions` must be \"markers\" or a list of ids")),
    };
    let (ids, ranges): (Vec<u64>, Vec<TimeRange>) = {
        let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
        if !seq.markers.iter().any(is_silence) {
            return Err(bad(CMD_REMOVE, "the sequence has no silence regions; analyse first"));
        }
        if let Some(w) = &wanted {
            for id in w {
                if !seq.markers.iter().any(|m| m.id.0 == *id && is_silence(m)) {
                    return Err(bad(CMD_REMOVE, format!("{id} is not a silence region")));
                }
            }
        }
        seq.markers
            .iter()
            .filter(|m| is_silence(m) && !is_kept(m) && m.duration > Tick::ZERO && wanted.as_ref().is_none_or(|w| w.contains(&m.id.0)))
            .map(|m| (m.id.0, TimeRange::new(m.start, m.duration)))
            .unzip()
    };
    if ranges.is_empty() {
        return Err(bad(CMD_REMOVE, "no silence region is selected for removal (all are kept or empty)"));
    }
    if !cut {
        return Ok(json!({"removed": 0, "marked": ranges.len(), "ticks": 0}));
    }
    let n = ranges.len();
    let total = s.edit_sequence("Remove Silence", |q, ctx, _| {
        let merged = merge_ranges(ranges.clone());
        let total = if ripple {
            tx::ripple_delete_ranges(q, ranges, ctx)
        } else {
            let tracks: Vec<_> = q.all_tracks().filter(|t| !t.locked).map(|t| t.id).collect();
            let mut total = Tick::ZERO;
            for r in merged.iter().rev() {
                cfa4_edit::lift(q, &tracks, *r, ctx);
                total += r.duration;
            }
            total
        };
        if !keep_markers {
            q.markers.retain(|m| !ids.contains(&m.id.0));
        }
        if ripple {
            // the closed gaps pull the remaining silence markers up with the audio
            for m in q.markers.iter_mut().filter(|m| is_silence(m)) {
                let back: i64 = merged.iter().map(|r| r.duration.0.min((m.start.0 - r.start.0).max(0))).sum();
                m.start = Tick(m.start.0 - back);
            }
            q.markers.sort_by_key(|m| m.start);
        }
        Ok(total)
    })?;
    Ok(json!({"removed": n, "ticks": total.0, "seconds": total.0 as f64 / TICKS_PER_SECOND as f64, "ripple": ripple}))
}

/// `speech.vadModel`: the Silero VAD model's record (source, licence, size) and whether it is in
/// the model store. Hosts show this, ask for the one-time licence acceptance, then run
/// `speech.downloadVadModel`. Nothing is downloaded by any other command.
fn vad_model(_: &mut Session, _: &Value) -> Result<Value> {
    let m = &cfa4_speech::vad::silero::MODEL;
    let dir = crate::transcript::models_dir();
    Ok(json!({
        "id": m.id, "name": m.name, "description": m.description, "author": m.author,
        "license": m.license, "licenseUrl": m.license_url, "source": m.source, "size": m.size(),
        "dir": dir.as_ref().map(|d| cfa4_speech::models::model_dir(d, m).to_string_lossy().to_string()),
        "installed": dir.as_ref().is_some_and(|d| cfa4_speech::vad::silero::installed(d)),
        "detectorAvailable": cfg!(feature = "silero"),
        "downloadAvailable": cfg!(feature = "speech-download"),
    }))
}

/// `speech.downloadVadModel {licenseAccepted: true}`: download the Silero VAD model into the store
/// (confined `cfa4-fetch`, pinned URL and SHA-256). Refused unless the caller says the user has
/// seen the licence and accepted it; the host keeps the one-time acceptance.
fn download_vad_model(_: &mut Session, p: &Value) -> Result<Value> {
    if bool_p(p, "licenseAccepted") != Some(true) {
        return Err(bad(
            "speech.downloadVadModel",
            "the model's licence must be shown and accepted first: pass `licenseAccepted: true` after the user accepted it (see `speech.vadModel`)",
        ));
    }
    let m = &cfa4_speech::vad::silero::MODEL;
    let dir = crate::transcript::models_dir().ok_or_else(|| EngineError::Other("no data directory for speech models".into()))?;
    #[cfg(feature = "speech-download")]
    {
        let files = m.files.iter().map(|f| (f.name.to_string(), f.url.to_string(), f.sha256.to_string(), f.size));
        crate::security::download(&cfa4_speech::models::model_dir(&dir, m), files, &mut |_, _, _| true)
            .map_err(|e| EngineError::Other(format!("downloading the Silero VAD model: {e}")))?;
        Ok(json!({"model": m.id, "dir": cfa4_speech::models::model_dir(&dir, m).to_string_lossy()}))
    }
    #[cfg(not(feature = "speech-download"))]
    {
        let _ = (m, dir);
        Err(EngineError::Other("model downloads are not available in this build (built without the `speech-download` feature)".into()))
    }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "speech.vadModel",
            label: "Silero VAD Model Info",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: vad_model,
            journal: false,
        },
        CommandSpec {
            id: "speech.downloadVadModel",
            label: "Download Silero VAD Model",
            menu: &[],
            shortcut: None,
            params: r#"{"licenseAccepted":bool}"#,
            enabled: |_| {
                if cfg!(feature = "speech-download") {
                    Ok(())
                } else {
                    Err("model downloads are not available in this build (built without the `speech-download` feature)".into())
                }
            },
            run: download_vad_model,
            journal: false,
        },
        CommandSpec {
            id: CMD_DETECT,
            label: "Detect Silence",
            menu: &[],
            shortcut: None,
            params: r#"{"clips":[id]?,"track":id?,"scope":"selection"|"track"|"sequence"="selection","detector":"energy"|"silero"="energy","thresholdDb":f?,"minSilence":ticks?,"padding":ticks?,"mode":"preview"|"query"="preview","background":bool=false}"#,
            enabled: has_seq,
            run: detect,
            journal: true,
        },
        CommandSpec {
            id: CMD_MARK,
            label: "Mark Detected Silence",
            menu: &[],
            shortcut: None,
            params: r#"{"job":id}"#,
            enabled: has_seq,
            run: mark,
            journal: true,
        },
        CommandSpec {
            id: "silence.regions",
            label: "List Silence Regions",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_seq,
            run: regions,
            journal: false,
        },
        CommandSpec {
            id: CMD_KEEP,
            label: "Keep Silence Regions",
            menu: &[],
            shortcut: None,
            params: r#"{"ids":[id],"keep":bool}"#,
            enabled: has_silence_markers,
            run: keep,
            journal: true,
        },
        CommandSpec {
            id: "silence.clear",
            label: "Clear Silence Markers",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_silence_markers,
            run: clear,
            journal: true,
        },
        CommandSpec {
            id: CMD_REMOVE,
            label: "Remove Silence",
            menu: &[],
            // no one-key binding: the command deletes media from the sequence
            shortcut: None,
            params: r#"{"regions":"markers"|[id]="markers","ripple":bool=true,"mode":"cut"|"mark"="cut","keepMarkers":bool=false}"#,
            enabled: has_silence_markers,
            run: remove,
            journal: true,
        },
    ]
}
