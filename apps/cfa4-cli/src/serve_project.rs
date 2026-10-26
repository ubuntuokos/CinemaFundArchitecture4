//! `project.panel`: what the Qt GUI's Project panel shows of one bin, as egui's
//! `crates/ui-egui/src/panels/project.rs` and `project_views.rs` read it from the session: the
//! breadcrumb, the item count and selection, and the rows of the List view (sub-bins twirled open,
//! the Metadata Display columns' cells) or the cards of the Icon view (Sort Icons order), each
//! with what a row or card draws (type icon, label colour, duration, In/Out, poster frame, badges).
//! Every change goes through engine commands (`project.select`, `project.sort`,
//! `project.columns.*`, `project.view.set`, `project.viewPreset.*`, `clip.rename`,
//! `project.renameBin`, `project.moveToBin`, `project.delete`, …).

use std::collections::HashSet;

use cfa4_engine::Session;
use cfa4_engine::project_panel::{self as pp, ViewMode};
use cfa4_project::{Bin, BinEntry, BinId, ItemId, ItemKind, MediaRef, ProjectItem};
use serde_json::{Value, json};

/// Rows at most in one reply (a project of many thousand clips still answers quickly).
const MAX_ROWS: usize = 20_000;
/// How deep twirled-open bins are listed (bins nest without limit in a project file).
const MAX_DEPTH: usize = 64;

/// The icon a row or card draws for an item (egui's `item_icon`).
fn kind_icon(it: &ProjectItem) -> &'static str {
    match &it.kind {
        ItemKind::Media(m) => match m.info.kind {
            cfa4_media::MediaKind::AudioOnly => "audio",
            cfa4_media::MediaKind::Still | cfa4_media::MediaKind::ImageSequence => "image",
            _ => "film",
        },
        ItemKind::Sequence(_) => "sequence",
        ItemKind::Subclip { .. } => "film",
        ItemKind::AdjustmentLayer { .. } | ItemKind::Graphic { .. } => "adjust",
    }
}

/// Offline text (egui's `media_badge`): None when the media is there.
fn offline(s: &Session, it: &ProjectItem) -> (Option<&'static str>, bool) {
    let target = match &it.kind {
        ItemKind::Subclip { parent, .. } => s.project.item(*parent),
        _ => Some(it),
    };
    let Some((id, m)) = target.and_then(|t| t.as_media().map(|m| (t.id, m))) else { return (None, false) };
    let text = if m.offline {
        Some("Offline")
    } else if s.offline.missing.contains(&id) {
        Some("Media missing")
    } else {
        s.media.offline_status(id).map(|st| if st.reason == cfa4_render::offline::OfflineReason::Unreadable { "Unreadable" } else { "Media missing" })
    };
    (text, m.proxy.is_some())
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn item_row(s: &Session, it: &ProjectItem, depth: usize, columns: &[pp::Column]) -> Value {
    let p = &s.project;
    let cells: serde_json::Map<String, Value> = columns.iter().map(|c| (c.name.clone(), json!(pp::cell(p, it, &c.name).0))).collect();
    let marks = match &it.kind {
        ItemKind::Media(m) => (m.mark_in, m.mark_out),
        ItemKind::Sequence(q) => (q.mark_in, q.mark_out),
        _ => (None, None),
    };
    let (offline, proxy) = offline(s, it);
    let duration = it.duration();
    let poster = cfa4_engine::keyboard::poster_frame(it).unwrap_or(cfa4_time::Tick(duration.0 / 10 * 3));
    let matte = matches!(&it.kind, ItemKind::Media(m) if matches!(m.media, MediaRef::Generator(cfa4_media::Generator::ColorMatte { .. })));
    let file = p.resolve_media(it.id).is_some_and(|(_, m, _)| matches!(m.media, MediaRef::File { .. }));
    json!({
        "item": it.id.0,
        "name": it.name,
        "depth": depth,
        "icon": kind_icon(it),
        "type": it.type_label(),
        "sequence": matches!(it.kind, ItemKind::Sequence(_)),
        "subclip": matches!(it.kind, ItemKind::Subclip { .. }),
        "media": matches!(it.kind, ItemKind::Media(_)),
        "file": file,
        "matte": matte,
        "label": it.label.name(),
        "labelColor": hex(s.prefs.labels.rgb(it.label)),
        "cells": cells,
        "duration": duration.0,
        "frame": it.frame_rate().frame_duration().0,
        "poster": poster.0,
        "in": marks.0.map(|t| t.0),
        "out": marks.1.map(|t| t.0),
        "hasVideo": it.has_video(),
        "hasAudio": it.has_audio(),
        "offline": offline,
        "proxy": proxy,
    })
}

fn bin_row(b: &Bin, depth: usize, open: bool) -> Value {
    let mut n = Vec::new();
    b.all_items(&mut n);
    json!({"bin": b.id.0, "name": b.name, "depth": depth, "count": n.len(), "open": open})
}

fn matches(s: &Session, it: &ProjectItem, filter: &str) -> bool {
    pp::listed(&s.project, it) && (filter.is_empty() || it.name.to_lowercase().contains(filter))
}

/// The List view's rows: sub-bins first (twirled open when in `expanded`, all while searching),
/// then the items, in the sort order.
#[allow(clippy::too_many_arguments)]
fn list_rows(s: &Session, bin: &Bin, depth: usize, expanded: &HashSet<u64>, filter: &str, columns: &[pp::Column], out: &mut Vec<Value>) {
    if depth > MAX_DEPTH {
        return;
    }
    let p = &s.project;
    let (bins, items) = pp::bin_children(p, bin, &s.prefs.project_panel.view.sort);
    for bid in bins {
        let Some(sub) = p.root.find_bin(bid) else { continue };
        if out.len() >= MAX_ROWS {
            return;
        }
        let open = expanded.contains(&bid.0) || !filter.is_empty();
        out.push(bin_row(sub, depth, open));
        if open {
            list_rows(s, sub, depth + 1, expanded, filter, columns, out);
        }
    }
    for id in items {
        if out.len() >= MAX_ROWS {
            return;
        }
        if let Some(it) = p.item(id).filter(|it| matches(s, it, filter)) {
            out.push(item_row(s, it, depth, columns));
        }
    }
}

/// The Icon view's cards: the bin's sub-bins, then its items in the Sort Icons order (the bin's
/// own order for User Order). A search shows the matches from sub-bins too, without bins.
fn icon_rows(s: &Session, bin: &Bin, filter: &str, columns: &[pp::Column], out: &mut Vec<Value>) {
    let p = &s.project;
    let mut ids = Vec::new();
    if filter.is_empty() {
        for e in &bin.children {
            match e {
                BinEntry::Bin(b) => out.push(bin_row(b, 0, false)),
                BinEntry::Item(i) => ids.push(*i),
            }
        }
    } else {
        bin.all_items(&mut ids);
    }
    ids.retain(|i| p.item(*i).is_some_and(|it| matches(s, it, filter)));
    pp::sort_items(p, &mut ids, &s.prefs.project_panel.view.icon_sort);
    for id in ids.into_iter().take(MAX_ROWS) {
        if let Some(it) = p.item(id) {
            out.push(item_row(s, it, 0, columns));
        }
    }
}

/// The names of the bins from the root down to `target` (the root itself not included).
fn bin_path(root: &Bin, target: BinId) -> Vec<Value> {
    fn walk(b: &Bin, target: BinId, path: &mut Vec<Value>, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        for c in &b.children {
            if let BinEntry::Bin(sub) = c {
                path.push(json!({"bin": sub.id.0, "name": sub.name}));
                if sub.id == target || walk(sub, target, path, depth + 1) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    let mut out = Vec::new();
    if root.id != target {
        walk(root, target, &mut out, 0);
    }
    out
}

/// The bin that holds bin `id` (None at the root).
fn parent_bin(b: &Bin, id: BinId, depth: usize) -> Option<BinId> {
    if depth > MAX_DEPTH {
        return None;
    }
    for c in &b.children {
        if let BinEntry::Bin(sub) = c {
            if sub.id == id {
                return Some(b.id);
            }
            if let Some(p) = parent_bin(sub, id, depth + 1) {
                return Some(p);
            }
        }
    }
    None
}

/// `project.panel {bin?, mode?, expanded?, filter?}`: one Project panel view of `bin` (left out or
/// gone: the project's root) in `mode` (`list` | `icon`; left out: the panel's View setting).
/// `expanded`: the bins twirled open in the List view. `filter`: the search text (names containing
/// it, from every sub-bin).
pub fn panel(s: &Session, p: &Value) -> Result<Value, String> {
    let pr = &s.project;
    let bin = match p.get("bin") {
        None | Some(Value::Null) => pr.root.id,
        Some(v) => BinId(v.as_u64().ok_or("`bin` must be a bin id")?),
    };
    let bin = pr.root.find_bin(bin).map_or(pr.root.id, |b| b.id);
    let shown = pr.root.find_bin(bin).ok_or("the project has no root bin")?;
    let mode = match p.get("mode").and_then(Value::as_str) {
        None => s.prefs.project_panel.view.mode,
        Some(m) => ViewMode::from_name(m).ok_or_else(|| format!("unknown view `{m}` (list, icon)"))?,
    };
    let expanded: HashSet<u64> = p.get("expanded").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let filter = p.get("filter").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    let columns = s.prefs.project_panel.view.columns.clone();
    let mut rows = Vec::new();
    match mode {
        ViewMode::Icon => icon_rows(s, shown, &filter, &columns, &mut rows),
        _ => list_rows(s, shown, 0, &expanded, &filter, &columns, &mut rows),
    }
    let mut all = Vec::new();
    shown.all_items(&mut all);
    let total = all.iter().filter(|i| pr.item(**i).is_some_and(|it| pp::listed(pr, it))).count();
    let selection: Vec<u64> = s.state.project_selection.iter().map(|i: &ItemId| i.0).collect();
    // the Preview Area: the first selected item, its info and how often the sequences use it
    let preview = s.state.project_selection.first().and_then(|id| pr.item(*id)).map(|it| {
        let info: Vec<pp::Column> =
            ["Video Info", "Media Duration", "Frame Rate", "Audio Info"].iter().map(|c| pp::Column { name: (*c).to_string(), width: 0.0 }).collect();
        let mut row = item_row(s, it, 0, &info);
        let (video, audio) = pp::usage(pr, it.id);
        if let Some(o) = row.as_object_mut() {
            o.insert("videoUsage".into(), json!(video));
            o.insert("audioUsage".into(), json!(audio));
        }
        row
    });
    let g = &s.prefs.general;
    Ok(json!({
        "bin": bin.0,
        "root": pr.root.id.0,
        "parent": parent_bin(&pr.root, bin, 0).map(|b| b.0),
        "project": pr.name,
        "path": bin_path(&pr.root, bin),
        "mode": mode,
        "columns": columns,
        "sort": s.prefs.project_panel.view.sort,
        "iconSort": s.prefs.project_panel.view.icon_sort,
        "total": total,
        "selection": selection,
        "rows": rows,
        "truncated": rows.len() >= MAX_ROWS,
        "preview": preview,
        // how a double-click opens a bin (Settings ▸ General ▸ Bins): plain, with Ctrl, with Alt
        "open": {"plain": g.bins_double_click, "ctrl": g.bins_cmd_double_click, "alt": g.bins_opt_double_click},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_rows_and_cards_of_a_bin() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let r = s.execute("file.newBin", json!({"name": "Shots"})).expect("bin");
        let bin = r["bin"].as_u64().expect("bin id");
        let mut all = Vec::new();
        s.project.root.all_items(&mut all);
        let first = all.into_iter().find(|i| s.project.item(*i).is_some_and(|it| pp::listed(&s.project, it))).expect("an item");
        s.execute("project.moveToBin", json!({"items": [first.0], "bin": bin})).expect("move");
        let list = panel(&s, &json!({"mode": "list"})).expect("list");
        let rows = list["rows"].as_array().cloned().expect("rows");
        let at = rows.iter().position(|r| r["bin"] == json!(bin)).expect("the new bin");
        assert!(rows.iter().take_while(|r| r.get("bin").is_some()).count() > at, "bins first: {list}");
        assert_eq!(rows[at]["open"], json!(false));
        assert!(rows.iter().all(|r| r["item"] != json!(first.0)), "a closed bin hides its items");
        let open = panel(&s, &json!({"mode": "list", "expanded": [bin]})).expect("list");
        let rows = open["rows"].as_array().cloned().expect("rows");
        let item = &rows[at + 1];
        assert_eq!(item["item"], json!(first.0), "twirled open: {open}");
        assert_eq!(item["depth"], json!(1));
        assert!(item["cells"].get("Name").is_some(), "the shown columns' cells: {item}");
        assert!(item["labelColor"].as_str().is_some_and(|c| c.starts_with('#') && c.len() == 7));
        let inside = panel(&s, &json!({"bin": bin, "mode": "icon"})).expect("icon");
        assert_eq!(inside["parent"], json!(s.project.root.id.0));
        assert_eq!(inside["path"][0]["name"], json!("Shots"));
        assert_eq!(inside["total"], json!(1));
        let name = s.project.item(first).map(|i| i.name.clone()).expect("name");
        let found = panel(&s, &json!({"mode": "icon", "filter": name.to_uppercase()})).expect("search");
        assert!(found["rows"].as_array().is_some_and(|r| r.iter().any(|x| x["item"] == json!(first.0))), "a search finds items in sub-bins");
        // hostile input: a bin that is gone shows the root, a wrong type is refused
        assert_eq!(panel(&s, &json!({"bin": 999_999})).expect("root")["bin"], json!(s.project.root.id.0));
        assert!(panel(&s, &json!({"bin": "x"})).is_err());
        s.execute("project.select", json!({"items": [first.0]})).expect("select");
        let shown = panel(&s, &json!({})).expect("preview");
        assert_eq!(shown["preview"]["item"], json!(first.0), "{shown}");
        assert!(shown["preview"]["videoUsage"].is_u64());
        assert_eq!(shown["open"]["plain"], json!("openInPlace"));
        assert!(panel(&s, &json!({"mode": "nope"})).is_err());
    }
}
