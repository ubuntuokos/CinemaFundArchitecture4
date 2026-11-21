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

/// The levels of a clip's audio (16 kHz mono, 10 ms frames), read in pieces so a long clip is
/// never held whole.
fn clip_levels(s: &Session, c: ClipId) -> Result<(Vec<f32>, usize)> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(CMD_DETECT, "no such clip"))?;
    let sr = cfa4_speech::SAMPLE_RATE;
    let m0 = it.source_in.to_units_floor(i64::from(sr));
    let total = it.duration.to_units_floor(i64::from(sr)).max(0) as usize;
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let src = cfa4_render::SourceProvider::source(&provider, it.item).ok_or_else(|| EngineError::Other("the clip's media is offline".into()))?;
    let chunk = CHUNK_SECONDS * sr as usize / FRAME_SAMPLES * FRAME_SAMPLES;
    let mut db = Vec::with_capacity(total / FRAME_SAMPLES + 1);
    let mut done = 0usize;
    while done < total {
        let n = chunk.min(total - done);
        let buf = src
            .audio_stream(it.audio_stream, m0.saturating_add(done as i64), n, sr)
            .map_err(|e| EngineError::Other(format!("reading the clip's audio: {e}")))?;
        let chans = buf.channels.len().max(1) as f32;
        let mut mono = vec![0.0f32; n];
        for ch in &buf.channels {
            for (m, v) in mono.iter_mut().zip(ch) {
                *m += *v / chans;
            }
        }
        db.extend(vad::frame_db(&mono));
        done += n;
    }
    Ok((db, total))
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
    match str_p(p, "detector").unwrap_or("energy") {
        "energy" => {}
        "silero" => return Err(bad(CMD_DETECT, "the speech-model detector (`silero`) needs the AI layer, which is not deployed yet; use `energy`")),
        other => return Err(bad(CMD_DETECT, format!("unknown detector `{other}`: use energy"))),
    }
    let preview = match str_p(p, "mode").unwrap_or("preview") {
        "preview" => true,
        "query" => false,
        other => return Err(bad(CMD_DETECT, format!("unknown mode `{other}`: use preview or query"))),
    };
    let sp = params(p)?;
    let (ids, skipped) = pick_clips(s, p)?;
    let rate = s.sequence_rate();
    let mut analysed = Vec::with_capacity(ids.len());
    for c in &ids {
        let (db, samples) = clip_levels(s, *c)?;
        let regions = vad::silence_regions_from_db(&db, samples, &sp);
        let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let (_, it) = seq.find_item(*c).ok_or_else(|| bad(CMD_DETECT, "no such clip"))?;
        let silent = regions.iter().map(|(a, d)| (it.start + *a, it.start + *a + *d)).collect();
        analysed.push(Analysed { id: *c, span: (it.start, it.start + it.duration), silent });
    }
    let regions = intersect(&analysed, rate);
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

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD_DETECT,
            label: "Detect Silence",
            menu: &[],
            shortcut: None,
            params: r#"{"clips":[id]?,"track":id?,"scope":"selection"|"track"|"sequence"="selection","detector":"energy"="energy","thresholdDb":f?,"minSilence":ticks?,"padding":ticks?,"mode":"preview"|"query"="preview"}"#,
            enabled: has_seq,
            run: detect,
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
