//! File ▸ Export ▸ Check Before Export…: finds what usually goes wrong in a finished promo before
//! it is exported, and marks each problem on the timeline.
//!
//! | kind | what is found |
//! |---|---|
//! | `flashFrame` | a picture clip (or graphic) only one or two frames long |
//! | `blackGap` | time inside the edit where no enabled video track shows anything |
//! | `offline` | a clip whose media is offline |
//! | `captionFast` | a caption with more characters per second than `maxCps` (20) |
//! | `captionShort` | a caption on screen for less than 0.7 s |
//! | `offFrame` | a graphic layer that reaches outside the frame |
//! | `safeZone` | a text layer under a platform's buttons (TikTok / Reels / Shorts, see [`crate::social::SafeZone`]) |
//!
//! Everything is computed from the timeline (no rendering, no models), so it is instant and
//! deterministic. Graphics are measured on their canvas (the sequence frame) at the middle of the
//! clip, before the clip's Motion. With `markers` (default on) each problem gets a red sequence
//! marker whose comment starts with [`CHECK_COMMENT`]; running the check again replaces them, and
//! `markers.clearChecks` removes them.

use cfa4_project::graphic::LayerContent;
use cfa4_project::{ItemKind, Label, Marker, MarkerId, MarkerKind, TrackKind};
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, f64_p, has_seq, str_p, u64_p};
use crate::social::{SafeZone, strictest_safe_rect};
use crate::{EngineError, Result, Session};

/// The comment every check marker starts with.
pub const CHECK_COMMENT: &str = "Delivery check";

const MIN_CAPTION_S: f64 = 0.7;

#[derive(Clone, Debug)]
struct Issue {
    kind: &'static str,
    title: String,
    start: Tick,
    duration: Tick,
    clip: Option<u64>,
}

fn is_check_marker(m: &Marker) -> bool {
    m.comment.starts_with(CHECK_COMMENT)
}

fn has_checks(s: &Session) -> std::result::Result<(), String> {
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.markers.iter().any(is_check_marker) { Ok(()) } else { Err("the sequence has no check markers".into()) }
}

/// Merge `[start, end)` ranges; returns the sorted union.
fn union(mut r: Vec<(Tick, Tick)>) -> Vec<(Tick, Tick)> {
    r.sort();
    let mut out: Vec<(Tick, Tick)> = Vec::new();
    for (a, b) in r {
        match out.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn inside(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] >= b[0] - 0.5 && a[1] >= b[1] - 0.5 && a[2] <= b[2] + 0.5 && a[3] <= b[3] + 0.5
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "sequence.checkDelivery";
    let seq_id = crate::export_tools::sequence_param(s, p, CMD)?;
    let q = s.project.sequence(seq_id).ok_or(EngineError::NoSequence)?.clone();
    let rate = q.settings.frame_rate;
    let frame = rate.frame_duration().max(Tick(1));
    let (w, h) = (q.settings.width.max(1) as f64, q.settings.height.max(1) as f64);
    let max_cps = f64_p(p, "maxCps").unwrap_or(20.0);
    if !max_cps.is_finite() || max_cps <= 0.0 {
        return Err(bad(CMD, "maxCps must be positive"));
    }
    let flash_frames = u64_p(p, "flashFrames").unwrap_or(2).min(1000) as i64;
    // which safe zone: one platform, all of them (the strictest), or none; vertical frames
    // default to all
    let zone = match str_p(p, "zone").unwrap_or(if h > w { "all" } else { "none" }) {
        "none" | "off" => None,
        "all" => Some(("all platforms", strictest_safe_rect(w, h))),
        z => {
            let z = SafeZone::from_id(z).ok_or_else(|| bad(CMD, format!("unknown zone `{z}`; use tiktok, reels, shorts, all or none")))?;
            Some((z.label(), z.safe_rect(w, h)))
        }
    };
    let mut issues = Vec::new();
    // the edit's extent: from 0 to the end of the last clip on any track
    let end = q.all_tracks().map(|t| t.end()).max().unwrap_or(Tick::ZERO);
    let mut shown = Vec::new();
    for t in q.video_tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        for it in &t.items {
            if t.enabled && it.enabled {
                shown.push((it.start, it.end()));
            }
            let pi = s.project.item(it.item);
            if it.duration <= Tick(frame.0.saturating_mul(flash_frames)) {
                let n = rate.frame_at(it.duration).max(1);
                issues.push(Issue {
                    kind: "flashFrame",
                    title: format!("Flash frame ({n} frame{})", if n == 1 { "" } else { "s" }),
                    start: it.start,
                    duration: it.duration,
                    clip: Some(it.id.0),
                });
            }
            if pi.is_some_and(|x| matches!(x.kind, ItemKind::Media(_) | ItemKind::Subclip { .. })) && crate::project_tools::is_offline(s, it.item) {
                issues.push(Issue { kind: "offline", title: "Offline media".into(), start: it.start, duration: it.duration, clip: Some(it.id.0) });
            }
            let Some(ItemKind::Graphic { width, height, .. }) = pi.map(|x| &x.kind) else { continue };
            let canvas = (*width, *height);
            let mid = it.source_in + Tick(it.duration.0 / 2);
            let frame_rect = [0.0, 0.0, canvas.0 as f64, canvas.1 as f64];
            let mut off = false;
            let mut unsafe_text = false;
            for (_, spec) in cfa4_render::graphic_clip::item_layer_specs(it, mid, canvas) {
                let b = cfa4_render::graphic_clip::layer_bounds(&spec);
                if !b.iter().all(|v| v.is_finite()) || b[2] <= b[0] || b[3] <= b[1] {
                    continue;
                }
                off |= !inside(b, frame_rect);
                if let (Some((_, safe)), LayerContent::Text(_)) = (zone, &spec.content)
                    && overlaps(b, frame_rect)
                    && !inside(b, safe)
                {
                    unsafe_text = true;
                }
            }
            if off {
                issues.push(Issue { kind: "offFrame", title: "Graphic leaves the frame".into(), start: it.start, duration: it.duration, clip: Some(it.id.0) });
            }
            if unsafe_text && let Some((name, _)) = zone {
                issues.push(Issue {
                    kind: "safeZone",
                    title: format!("Text under {name} buttons"),
                    start: it.start,
                    duration: it.duration,
                    clip: Some(it.id.0),
                });
            }
        }
    }
    let mut at = Tick::ZERO;
    for (a, b) in union(shown) {
        if a > at && a - at >= frame {
            issues.push(Issue { kind: "blackGap", title: format!("Black gap ({:.2} s)", (a - at).seconds()), start: at, duration: a - at, clip: None });
        }
        at = at.max(b);
    }
    if end > at && end - at >= frame {
        issues.push(Issue { kind: "blackGap", title: format!("Black gap ({:.2} s)", (end - at).seconds()), start: at, duration: end - at, clip: None });
    }
    for ct in &q.caption_tracks {
        for c in &ct.captions {
            let secs = c.duration.seconds();
            let chars = c.text.chars().filter(|ch| !ch.is_whitespace()).count() as f64;
            if secs < MIN_CAPTION_S {
                issues.push(Issue {
                    kind: "captionShort",
                    title: format!("Caption on screen {secs:.2} s"),
                    start: c.start,
                    duration: c.duration,
                    clip: Some(c.id.0),
                });
            } else if chars / secs > max_cps {
                issues.push(Issue {
                    kind: "captionFast",
                    title: format!("Caption too fast ({:.0} chars/s)", chars / secs),
                    start: c.start,
                    duration: c.duration,
                    clip: Some(c.id.0),
                });
            }
        }
    }
    issues.sort_by_key(|i| (i.start, i.kind));
    let markers = bool_p(p, "markers").unwrap_or(true);
    let active = s.state.active_sequence == Some(seq_id);
    if markers && active {
        let list = issues.clone();
        s.edit_sequence("Check Before Export", |q, ctx, _| {
            q.markers.retain(|m| !is_check_marker(m));
            for i in &list {
                q.markers.push(Marker {
                    id: MarkerId(ctx.alloc()),
                    start: i.start,
                    duration: i.duration,
                    name: i.title.clone(),
                    comment: format!("{CHECK_COMMENT}: {}", i.kind),
                    kind: MarkerKind::Comment,
                    color: Label::Rose,
                });
            }
            q.markers.sort_by_key(|m| m.start);
            Ok(())
        })?;
    }
    let mut counts = serde_json::Map::new();
    for i in &issues {
        let n = counts.get(i.kind).and_then(Value::as_u64).unwrap_or(0);
        counts.insert(i.kind.into(), json!(n + 1));
    }
    Ok(json!({
        "ok": issues.is_empty(),
        "count": issues.len(),
        "counts": counts,
        "zone": zone.map(|(n, _)| n),
        "issues": issues.iter().map(|i| json!({
            "kind": i.kind, "title": i.title, "time": i.start.0, "seconds": i.start.seconds(),
            "duration": i.duration.0, "clip": i.clip,
        })).collect::<Vec<_>>(),
        "markers": markers && active,
    }))
}

fn clear(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.edit_sequence("Clear Check Markers", |q, _, _| {
        let before = q.markers.len();
        q.markers.retain(|m| !is_check_marker(m));
        Ok(before - q.markers.len())
    })?;
    Ok(json!({"removed": n}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "sequence.checkDelivery",
            label: "Check Before Export…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r#"{"sequence":id?,"zone":"tiktok|reels|shorts|all|none"?,"maxCps":f64=20,"flashFrames":n=2,"markers":bool=true}"#,
            enabled: has_seq,
            run: check,
            journal: true,
        },
        CommandSpec {
            id: "markers.clearChecks",
            label: "Clear Check Markers",
            menu: &["Markers"],
            shortcut: None,
            params: "{}",
            enabled: has_checks,
            run: clear,
            journal: true,
        },
    ]
}
