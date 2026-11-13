//! Segments of a source (CFA4-LOSSLESS-MEDIA-TOOLS-001 1.1, K2): the range markers of a media
//! item, kept in the project (`.fcproj`) like every other marker, cut losslessly in one command.
//!
//! - A **segment** is a marker with a duration on a media item (Source monitor ▸ Add Marker with
//!   `durationFrames`, or Scene Edit Detection's clip markers). Its name, comment and colour are the
//!   marker's; there is no second store.
//! - `segments.list {"item"?}` lists them in time order (default item: the Source monitor's clip).
//! - `segments.plan` / `segments.export {"item"?, "segments":[marker id]? , "output": pattern, …}`
//!   cut the chosen segments (default: all, in time order; an explicit list sets the order) through
//!   `export.plan` / `export.start` with `ranges` ([`crate::smart_cut_ranges`]): one verified file
//!   per segment, `{n}` and `{name}` in the output pattern, nothing written unless every segment
//!   can run, one `"confirm"` for a batch that re-encodes.
//! - `segments.toSubclips` (K3) makes one subclip per chosen segment, in one undo step, named after
//!   the marker, so the segment keeps its link to the source file and its media times.
//!   `segments.toTimeline` also places those subclips back to back on the active sequence
//!   (`timeline.place` with `items`; `time`, `track`, `insert` pass through).

use std::collections::BTreeSet;

use cfa4_project::{ItemId, ItemKind, Marker};
use cfa4_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::clip_ops::source_view;
use crate::commands::{CommandSpec, always, bad, bool_p, item_p};
use crate::{EngineError, Result, Session};

/// The media item named by `item`, or the Source monitor's clip.
fn item_of(s: &Session, p: &Value, cmd: &str) -> Result<ItemId> {
    match p.get("item") {
        None | Some(Value::Null) => s.state.source_item.ok_or_else(|| bad(cmd, "`item` is required when no Source clip is open")),
        Some(_) => item_p(p, "item").ok_or_else(|| bad(cmd, "`item` must be a project item id")),
    }
}

/// The segments (markers with a duration) of an item, in time order.
fn segments_of(s: &Session, item: ItemId, cmd: &str) -> Result<Vec<Marker>> {
    let view = source_view(s, item).ok_or_else(|| bad(cmd, format!("no media item {}", item.0)))?;
    let mut list: Vec<Marker> = view.markers.into_iter().filter(|m| m.duration.0 > 0).collect();
    list.sort_by_key(|m| (m.start, m.id.0));
    Ok(list)
}

fn segment_json(m: &Marker) -> Value {
    json!({
        "marker": m.id.0,
        "in": m.start.0,
        "out": m.start.0.saturating_add(m.duration.0),
        "duration": m.duration.0,
        "name": m.name,
        "comment": m.comment,
        "color": m.color,
        "kind": m.kind,
    })
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "segments.list";
    let item = item_of(s, p, cmd)?;
    let segments = segments_of(s, item, cmd)?;
    Ok(json!({"item": item.0, "segments": segments.iter().map(segment_json).collect::<Vec<_>>()}))
}

/// The segments chosen by `segments` (default: all, in time order; a list sets the order).
fn chosen(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, Vec<Marker>)> {
    let item = item_of(s, p, cmd)?;
    let all = segments_of(s, item, cmd)?;
    let chosen: Vec<Marker> = match p.get("segments") {
        None | Some(Value::Null) => all,
        Some(Value::Array(ids)) => {
            if ids.is_empty() {
                return Err(bad(cmd, "`segments` is empty"));
            }
            if ids.len() > crate::smart_cut_ranges::MAX_RANGES {
                return Err(bad(cmd, format!("at most {} segments per command", crate::smart_cut_ranges::MAX_RANGES)));
            }
            let mut seen = BTreeSet::new();
            let mut out = Vec::with_capacity(ids.len());
            for v in ids {
                let id = v.as_u64().ok_or_else(|| bad(cmd, "`segments` must list marker ids"))?;
                if !seen.insert(id) {
                    return Err(bad(cmd, format!("segment {id} is listed twice")));
                }
                out.push(all.iter().find(|m| m.id.0 == id).cloned().ok_or_else(|| bad(cmd, format!("item {} has no segment (range marker) {id}", item.0)))?);
            }
            out
        }
        Some(_) => return Err(bad(cmd, "`segments` must be a list of marker ids")),
    };
    if chosen.is_empty() {
        return Err(bad(cmd, format!("item {} has no segments: add range markers in the Source monitor first", item.0)));
    }
    Ok((item, chosen))
}

/// The `export.*` parameters of the chosen segments: the command's own, with `item` and `ranges`.
fn ranges_of(s: &Session, p: &Value, cmd: &str) -> Result<Value> {
    let (item, chosen) = chosen(s, p, cmd)?;
    let ranges: Vec<Value> = chosen
        .iter()
        .map(|m| json!({"in": m.start.0, "out": m.start.0.saturating_add(m.duration.0), "name": if m.name.is_empty() { Value::Null } else { json!(m.name) }}))
        .collect();
    let mut q = p.as_object().cloned().ok_or_else(|| bad(cmd, "the parameters must be an object"))?;
    q.remove("segments");
    q.remove("path");
    q.remove("scope");
    q.insert("item".into(), json!(item.0));
    q.insert("ranges".into(), Value::Array(ranges));
    Ok(Value::Object(q))
}

/// The export result with each range's marker id beside it.
fn with_markers(s: &Session, p: &Value, cmd: &str, mut r: Value) -> Result<Value> {
    let (item, chosen) = chosen(s, p, cmd)?;
    if let Some(list) = r.get_mut("ranges").and_then(Value::as_array_mut) {
        for (entry, m) in list.iter_mut().zip(&chosen) {
            entry["marker"] = json!(m.id.0);
        }
    }
    r["item"] = json!(item.0);
    Ok(r)
}

fn plan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "segments.plan";
    let q = ranges_of(s, p, cmd)?;
    let r = s.execute("export.plan", q)?;
    with_markers(s, p, cmd, r)
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "segments.export";
    let q = ranges_of(s, p, cmd)?;
    let r = s.execute("export.start", q)?;
    with_markers(s, p, cmd, r)
}

/// One subclip per chosen segment, in one undo step; their ids in the segments' order.
fn make_subclips(s: &mut Session, p: &Value, cmd: &str) -> Result<Vec<(u64, ItemId)>> {
    let (item, chosen) = chosen(s, p, cmd)?;
    let view = source_view(s, item).ok_or_else(|| bad(cmd, format!("no media item {}", item.0)))?;
    let parent = view.media;
    let (base, label) = s.project.item(parent).map(|i| (i.name.clone(), i.label)).ok_or_else(|| EngineError::Other(format!("no media item {}", parent.0)))?;
    let restrict = bool_p(p, "restrictTrims").unwrap_or(false);
    let bin = s.project.root.parent_of(parent).filter(|b| *b != s.project.root.id);
    // the subclips stay inside the source (hostile marker times are clamped, empty ones refused)
    let (lo, hi) = (view.start.max(Tick::ZERO), view.end);
    let mut planned = Vec::with_capacity(chosen.len());
    for (n, m) in chosen.iter().enumerate() {
        let a = m.start.clamp(lo, hi);
        let b = Tick(m.start.0.saturating_add(m.duration.0)).clamp(lo, hi);
        if b <= a {
            return Err(bad(cmd, format!("segment {} lies outside the source", m.id.0)));
        }
        let name = if m.name.trim().is_empty() { format!("{base} {}", n + 1) } else { m.name.clone() };
        planned.push((m.id.0, name, TimeRange::from_bounds(a, b)));
    }
    s.edit("Segments to Subclips", |pr, st| {
        let mut made = Vec::with_capacity(planned.len());
        for (marker, name, range) in &planned {
            if pr.next_id == u64::MAX {
                return Err(bad(cmd, "project ids are exhausted"));
            }
            let id = pr.add_item(name, label, ItemKind::Subclip { parent, range: *range, restrict_trims: restrict }, bin);
            made.push((*marker, id));
        }
        st.project_selection = made.iter().map(|(_, id)| *id).collect();
        Ok(made)
    })
}

fn to_subclips(s: &mut Session, p: &Value) -> Result<Value> {
    let made = make_subclips(s, p, "segments.toSubclips")?;
    Ok(json!({"subclips": made.iter().map(|(m, id)| json!({"marker": m, "item": id.0})).collect::<Vec<_>>()}))
}

fn to_timeline(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "segments.toTimeline";
    if s.active_sequence().is_none() {
        return Err(bad(cmd, "open a sequence first"));
    }
    let made = make_subclips(s, p, cmd)?;
    let mut q = json!({"items": made.iter().map(|(_, id)| id.0).collect::<Vec<_>>()});
    for k in ["time", "frame", "seconds", "track", "audioTrack", "insert", "video", "audio"] {
        if let Some(v) = p.get(k) {
            q[k] = v.clone();
        }
    }
    let placed = s.execute("timeline.place", q)?;
    Ok(json!({"subclips": made.iter().map(|(m, id)| json!({"marker": m, "item": id.0})).collect::<Vec<_>>(), "placed": placed}))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "segments.list",
            label: "List Segments",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?} (default: the Source clip) → {item, segments:[{marker, in, out, duration, name, comment, color, kind}]} (range markers of the media item, time order)"#,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "segments.plan",
            label: "Plan Segment Export",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"segments":[marker id]? (default: all, time order),"output":"pattern with {n} and {name}"?,"mode":…,"guarantee":…,"crf":…,"audio":…} → export.plan with ranges, each with its marker"#,
            enabled: always,
            run: plan,
            journal: false,
        },
        CommandSpec {
            id: "segments.export",
            label: "Export Segments…",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"segments":[marker id]?,"output":"pattern with {n} and {name}","mode":…,"guarantee":…,"confirm":bool (once for the batch),"crf":…,"audio":…} → export.start with ranges: one verified file per segment, nothing written unless every segment can run"#,
            enabled: always,
            run: export,
            journal: true,
        },
        CommandSpec {
            id: "segments.toSubclips",
            label: "Segments to Subclips",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"segments":[marker id]?,"restrictTrims":bool=false} → {subclips:[{marker, item}]} (one undo step, named after the markers)"#,
            enabled: always,
            run: to_subclips,
            journal: true,
        },
        CommandSpec {
            id: "segments.toTimeline",
            label: "Segments to Timeline",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"segments":[marker id]?,"time":ticks?,"track":…?,"insert":bool?,"video":bool?,"audio":bool?} → {subclips:[{marker, item}], placed} (the subclips back to back on the active sequence, as timeline.place with items)"#,
            enabled: always,
            run: to_timeline,
            journal: true,
        },
    ]
}
