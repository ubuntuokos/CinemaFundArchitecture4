//! Markers ▸ Add Beat Markers: sequence markers on the beats of a music clip, so cuts can snap to
//! the music (marketing and promo edits are cut on the beat).
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `markers.addBeats` | Markers ▸ Add Beat Markers | marks every beat (or every 2nd / 4th / 8th) of the selected audio clip |
//! | `markers.clearBeats` | Markers ▸ Clear Beat Markers | removes the markers `markers.addBeats` made |
//!
//! The beats come from the same deterministic tracker as Clip ▸ Remix
//! ([`cfa4_audio_dsp::remix::analyze`], cached per media range), moved from the analysis' joint
//! position onto the attack ([`cfa4_audio_dsp::remix::beat_lead`]). Speed-changed clips are
//! mapped through their speed, remixed clips through their pieces; reversed clips are refused.
//! Beat markers carry a comment starting with [`BEAT_COMMENT`]; adding again replaces the beat
//! markers inside the clip's span (one undo step), other markers are never touched.

use cfa4_audio_dsp::remix as dsp;
use cfa4_project::{ClipId, Label, Marker, MarkerId, MarkerKind};
use cfa4_render::remix::Remix;
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, marker_color_p, u64_p};
use crate::{EngineError, Result, Session};

/// The comment every beat marker starts with (how they are found again).
pub const BEAT_COMMENT: &str = "Beat marker";

fn is_beat_marker(m: &Marker) -> bool {
    m.comment.starts_with(BEAT_COMMENT)
}

fn can_add(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    let c = crate::remix::selected_audio(s).ok_or("select a music clip on an audio track")?;
    crate::remix::check_clip(s, c).map_err(|e| e.replace("Remix works on", "Beat markers need").replace(" to remix", ""))
}

fn has_beat_markers(s: &Session) -> std::result::Result<(), String> {
    let seq = s.active_sequence().ok_or("no sequence is open")?;
    if seq.markers.iter().any(is_beat_marker) { Ok(()) } else { Err("the sequence has no beat markers".into()) }
}

/// Source time → sequence time pieces of a clip: (sequence start, source start, source length,
/// speed). A remixed clip has one piece per remix piece.
fn pieces(it: &cfa4_project::TrackItem) -> Vec<(Tick, Tick, Tick, f64)> {
    match Remix::of(it) {
        Some(r) => {
            let mut at = it.start;
            r.pieces
                .iter()
                .map(|p| {
                    let x = (at, p.src, p.len, 1.0);
                    at += p.len;
                    x
                })
                .collect()
        }
        None => {
            let speed = it.speed.abs().max(1e-6);
            vec![(it.start, it.source_in, Tick((it.duration.0 as f64 * speed).round() as i64), speed)]
        }
    }
}

fn add_beats(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "markers.addBeats";
    let c = match u64_p(p, "clip") {
        Some(c) => ClipId(c),
        None => crate::remix::selected_audio(s).ok_or_else(|| bad(CMD, "select a music clip or pass `clip`"))?,
    };
    crate::remix::check_clip(s, c).map_err(|e| bad(CMD, e))?;
    let every = u64_p(p, "every").unwrap_or(1);
    if ![1, 2, 4, 8].contains(&every) {
        return Err(bad(CMD, "`every` must be 1, 2, 4 or 8"));
    }
    let color = marker_color_p(p, CMD)?.unwrap_or(Label::Cerulean);
    let replace = bool_p(p, "replace").unwrap_or(true);
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let sr = seq.settings.sample_rate.max(1) as i64;
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(CMD, "no such clip"))?;
    if it.reverse || it.speed < 0.0 {
        return Err(bad(CMD, "the clip plays reversed; beat markers need it to play forward"));
    }
    let it = it.clone();
    let span = (it.start, it.start + it.duration);
    let original = Remix::of(&it).map(|r| r.original).unwrap_or(Tick((it.duration.0 as f64 * it.speed.abs().max(1e-6)).round() as i64));
    let a = crate::remix::analysis(s, it.item, it.source_in, original).map_err(|e| match e {
        EngineError::Other(m) => EngineError::Other(m.replace("cannot remix", "cannot find beats")),
        e => e,
    })?;
    let m0 = it.source_in.to_units_floor(sr);
    let lead = dsp::beat_lead(a.sample_rate);
    let mut times = Vec::new();
    for (k, &b) in a.beats.iter().enumerate() {
        if !(k as u64).is_multiple_of(every) {
            continue;
        }
        let src = Tick::from_units(m0.saturating_add(b).saturating_add(lead), sr);
        for &(at, s0, len, speed) in &pieces(&it) {
            if src >= s0 && src < s0 + len {
                let t = at + Tick(((src - s0).0 as f64 / speed).round() as i64);
                if t >= span.0 && t < span.1 {
                    times.push((k, t));
                }
            }
        }
    }
    times.sort_by_key(|(_, t)| *t);
    times.dedup_by_key(|(_, t)| *t);
    let comment = format!("{BEAT_COMMENT} · {:.1} BPM", a.bpm);
    let n = times.len();
    let ids = s.edit_sequence("Add Beat Markers", |q, ctx, _| {
        if replace {
            q.markers.retain(|m| !(is_beat_marker(m) && m.start >= span.0 && m.start < span.1));
        }
        let mut ids = Vec::with_capacity(times.len());
        for (k, t) in &times {
            let id = MarkerId(ctx.alloc());
            let name = if every == 4 { format!("Bar {}", k / 4 + 1) } else { format!("Beat {}", k + 1) };
            q.markers.push(Marker { id, start: *t, duration: Tick::default(), name, comment: comment.clone(), kind: MarkerKind::Comment, color });
            ids.push(id.0);
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(ids)
    })?;
    Ok(json!({"clip": c.0, "markers": ids, "count": n, "bpm": a.bpm, "every": every}))
}

fn clear_beats(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.edit_sequence("Clear Beat Markers", |q, _, _| {
        let before = q.markers.len();
        q.markers.retain(|m| !is_beat_marker(m));
        Ok(before - q.markers.len())
    })?;
    Ok(json!({"removed": n}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "markers.addBeats",
            label: "Add Beat Markers",
            menu: &["Markers"],
            shortcut: None,
            params: r#"{"clip":id?,"every":1|2|4|8=1,"color":label?,"replace":bool=true}"#,
            enabled: can_add,
            run: add_beats,
            journal: true,
        },
        CommandSpec {
            id: "markers.clearBeats",
            label: "Clear Beat Markers",
            menu: &["Markers"],
            shortcut: None,
            params: "{}",
            enabled: has_beat_markers,
            run: clear_beats,
            journal: true,
        },
    ]
}
