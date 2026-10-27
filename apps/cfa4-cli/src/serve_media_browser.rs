//! The Qt GUI's Media Browser, as egui's (`crates/ui-egui/src/panels/media_browser.rs`):
//! `mediaBrowser.inspect` lists a directory with the list view's column texts (probing a few
//! files per request, as egui probes a few per frame), the browser's state and preferences;
//! `mediaBrowser.thumbnail` renders a file's thumbnail (a frame `fraction` into it, for hover
//! scrub) whether or not it is in the project. Navigation, selection, Favorites, settings and
//! import stay the engine's `mediaBrowser.*` commands.

use std::sync::Arc;

use cfa4_engine::Session;
use cfa4_engine::media_browser as mb;
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, ProjectItem};
use cfa4_time::Tick;
use serde_json::{Value, json};

/// Files probed per `mediaBrowser.inspect` at most (the GUI asks again while some are left).
const PROBES_PER_CALL: usize = 8;
/// Columns that need the file's media properties.
const PROBED: [&str; 6] = ["Frame Rate", "Media Duration", "Video Info", "Audio Info", "Video Codec", "Audio Codec"];
const PREVIEW_ID: ItemId = ItemId(1);

/// `mediaBrowser.inspect {path?, probe?, refresh?}`; `refresh` forgets the probed properties.
pub fn inspect(s: &mut Session, p: &Value) -> Result<Value, String> {
    if p.get("refresh").and_then(Value::as_bool) == Some(true) {
        s.browser.probes.clear();
    }
    let dir = p.get("path").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| mb::current_dir(s));
    let prefs = s.prefs.media_browser.clone();
    let budget = p.get("probe").and_then(Value::as_u64).map_or(PROBES_PER_CALL, |n| usize::try_from(n).unwrap_or(0).min(64));
    let needs_probe = prefs.columns.iter().any(|c| PROBED.contains(&c.as_str()));
    let mut out = json!({
        "dir": dir,
        "canBack": !s.browser.back.is_empty(),
        "canForward": !s.browser.forward.is_empty(),
        "canUp": mb::parent(&dir).is_some(),
        "selection": s.browser.selection,
        "favorite": prefs.favorites.contains(&dir),
        "fileTypes": prefs.file_types,
        "view": prefs.view,
        "columns": prefs.columns,
        "importAsImageSequence": prefs.import_as_image_sequence,
        "hoverScrub": prefs.hover_scrub,
        "thumbnailSize": prefs.thumbnail_size,
        "recent": prefs.recent,
        "createFolderForImportedProjects": s.prefs.media.create_folder_for_imported_projects,
        "allowDuplicateMedia": s.prefs.media.allow_duplicate_media,
        "ingest": s.project.settings.ingest.enabled,
        "fileTypeChoices": mb::FILE_TYPES.iter().map(|(k, l)| json!([k, l])).collect::<Vec<_>>(),
        "extensions": mb::extensions(),
        "allColumns": mb::ALL_COLUMNS,
    });
    let entries = match mb::list(&*s.services, &dir, &prefs.file_types) {
        Ok(e) => e,
        Err(e) => {
            out["error"] = json!(e.to_string());
            out["entries"] = json!([]);
            return Ok(out);
        }
    };
    let mut left = budget;
    let mut pending = 0usize;
    let rows: Vec<Value> = entries
        .iter()
        .map(|e| {
            let probed = s.browser.probes.contains_key(&e.path);
            let info = if needs_probe && !e.is_dir && (probed || left > 0) {
                if !probed {
                    left -= 1;
                }
                mb::probe(s, &e.path)
            } else {
                if needs_probe && !e.is_dir {
                    pending += 1;
                }
                None
            };
            json!({
                "name": e.name,
                "path": e.path,
                "isDir": e.is_dir,
                "kind": e.kind,
                "numbered": e.numbered,
                "inProject": mb::item_for_path(s, &e.path).is_some(),
                "cells": prefs.columns.iter().map(|c| mb::column_text(e, c, info.as_ref())).collect::<Vec<_>>(),
            })
        })
        .collect();
    out["entries"] = Value::Array(rows);
    out["pending"] = json!(pending);
    Ok(out)
}

/// A one-item project holding `path` for its thumbnail (egui's preview items).
fn preview_project(path: &str, info: cfa4_media::MediaInfo) -> Project {
    let mut p = Project::default();
    p.items.insert(
        PREVIEW_ID,
        ProjectItem {
            id: PREVIEW_ID,
            name: mb::base_name(path),
            label: Label::Iris,
            kind: ItemKind::Media(MediaClip {
                media: MediaRef::File { path: path.to_string() },
                info,
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            metadata: Default::default(),
            created: 0,
            split: Default::default(),
        },
    );
    p
}

/// `mediaBrowser.thumbnail {path, fraction?, width?}` → RGBA8 of the frame `fraction` (0…1,
/// default 0.3) into the file, `width` pixels wide (default 160).
pub fn thumbnail(s: &mut Session, p: &Value) -> Result<(Value, Vec<u8>), String> {
    let path = p.get("path").and_then(Value::as_str).ok_or("need `path`")?.to_string();
    let frac = p.get("fraction").and_then(Value::as_f64).filter(|f| f.is_finite()).unwrap_or(0.3).clamp(0.0, 1.0);
    let width = p.get("width").and_then(Value::as_u64).unwrap_or(160).clamp(32, 640) as f32;
    let (project, id) = match mb::item_for_path(s, &path) {
        Some(i) => (s.project.clone(), i),
        None => {
            let info = mb::probe(s, &path).ok_or("not a readable media file")?;
            (Arc::new(preview_project(&path, info)), PREVIEW_ID)
        }
    };
    let item = project.item(id).ok_or("no such item")?;
    let src_w = item.as_media().and_then(|m| m.info.video.as_ref()).map(|v| v.width).filter(|w| *w > 0).ok_or("no picture")?;
    let (dur, rate) = (item.duration(), item.frame_rate());
    let t = rate.tick_of(rate.frame_at(Tick((dur.0 as f64 * frac) as i64)));
    let provider = s.media.provider(project.clone(), s.services.clone());
    let img = cfa4_render::render_item(&project, id, t, width / src_w as f32, &provider).ok_or("the picture is unavailable")?;
    Ok((json!({"w": img.w, "h": img.h, "format": "rgba8", "path": path, "time": t.0}), img.over_black_rgba8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_directory_and_renders_thumbnails() {
        let dir = cfa4_testkit::fixtures_dir("cli/media_browser");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Shots")).expect("dir");
        let png = dir.join("still.png");
        let px: Vec<u8> = (0..64 * 36).flat_map(|i| [(i % 64 * 4) as u8, 90, 200, 255]).collect();
        cfa4_testkit::golden::write_png(&png, &cfa4_testkit::golden::Rgba8 { w: 64, h: 36, px }).expect("png");
        std::fs::write(dir.join("notes.txt"), "not media").expect("txt");
        let mut s = Session::default();
        let d = dir.to_string_lossy().to_string();
        s.execute("mediaBrowser.navigate", json!({"path": d})).expect("navigate");
        s.execute("mediaBrowser.settings", json!({"columns": ["Name", "Media Type", "Video Info"]})).expect("columns");
        let r = inspect(&mut s, &json!({})).expect("inspect");
        assert_eq!(r["dir"], json!(d));
        assert_eq!(r["columns"], json!(["Name", "Media Type", "Video Info"]));
        let entries = r["entries"].as_array().cloned().expect("entries");
        let names: Vec<&str> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
        assert_eq!(names, vec!["Shots", "still.png"], "folders first, unsupported files hidden: {r}");
        assert_eq!(entries[0]["isDir"], json!(true));
        assert_eq!(entries[1]["cells"], json!(["still.png", "Image", "64 x 36"]), "{r}");
        assert_eq!(r["pending"], json!(0));
        assert_eq!(r["fileTypeChoices"][0], json!(["all", "All Supported Files"]));
        let none = inspect(&mut s, &json!({"path": dir.join("missing").to_string_lossy()})).expect("inspect");
        assert!(none["error"].is_string(), "{none}");
        let (meta, bytes) = thumbnail(&mut s, &json!({"path": png.to_string_lossy(), "width": 32})).expect("thumbnail");
        assert_eq!((meta["w"].as_u64(), meta["h"].as_u64()), (Some(32), Some(18)), "{meta}");
        assert_eq!(bytes.len(), 32 * 18 * 4);
        assert!(bytes.chunks(4).any(|c| c[2] > 150), "the still's blue is drawn");
        assert!(thumbnail(&mut s, &json!({"path": dir.join("notes.txt").to_string_lossy()})).is_err());
    }
}
