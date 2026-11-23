//! OpenVDB commands (`vdb.*`), plan CFA4-DNEG-TECHNOLOGY-ADAPTATION-001, N1.
//!
//! | command | does |
//! |---|---|
//! | `vdb.inspect` | grids, value types, voxel size, active voxels, bounding box and metadata of a `.vdb` file, read by the separate `cfa4-vdb-worker` process |
//!
//! | `vdb.import` | adds a `.vdb` file, or a numbered sequence (`name.0001.vdb`), to the Project panel as a volume item (N1-5); no timeline is created |
//! | `vdb.check` | the files of a volume item that are missing on disk (the relink report, N1-7) |
//! | `vdb.render` | a density preview of a volume item (one frame of a sequence) rendered by the worker to an RGBA PNG and imported as an ordinary still: the compositor and the renderer are unchanged (N2) |
//! | `vdb.relink` | points a volume item at another first file; frames are found again; one undo step |
//!
//! A missing or failing worker is an ordinary error message; the engine never depends on it to cut, play or save.

use serde_json::{Value, json};

use cfa4_project::{ItemId, ItemKind, Label, VolumeAsset};

use crate::commands::{CommandSpec, always, bad, item_p, str_p, u64_p};
use crate::{EngineError, Result, Session};

const EXTENSIONS: &[&str] = &["vdb"];

/// The frames of the numbered `.vdb` sequence that `first` belongs to (empty: not a sequence). A gap is `None`.
fn sequence_frames(s: &Session, first: &str) -> Vec<Option<String>> {
    use cfa4_media::sequence::{Numbered, sequence_frames};
    let Some(n) = Numbered::parse_with(first, EXTENSIONS) else { return Vec::new() };
    let Some(Ok(list)) = s.services.list_dir(n.dir.trim_end_matches(['/', '\\'])) else { return Vec::new() };
    let frames = sequence_frames(&n, &list);
    if frames.len() > 1 { frames } else { Vec::new() }
}

fn name_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

fn exists(s: &Session, path: &str) -> bool {
    s.services.file_size(path).is_ok()
}

pub(crate) fn volume_p(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, VolumeAsset)> {
    let id = match item_p(p, "item") {
        Some(id) => id,
        None => match s.state.project_selection.as_slice() {
            [id] => *id,
            _ => return Err(bad(cmd, "select one volume (or pass `item`)")),
        },
    };
    match s.project.item(id).map(|i| &i.kind) {
        Some(ItemKind::Volume(v)) => Ok((id, v.clone())),
        _ => Err(bad(cmd, "the item is not a volume")),
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "vdb.import";
    let bin = u64_p(p, "bin").map(cfa4_project::BinId);
    let paths: Vec<String> = match p.get("paths").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        None => str_p(p, "path").map(|x| vec![x.to_string()]).unwrap_or_default(),
    };
    if paths.is_empty() {
        return Err(bad(CMD, "need `paths`"));
    }
    let single_file = p.get("sequence").and_then(Value::as_bool) == Some(false);
    let mut items = Vec::new();
    for path in &paths {
        if !path.to_ascii_lowercase().ends_with(".vdb") {
            return Err(bad(CMD, format!("{path}: not a .vdb file")));
        }
        if !exists(s, path) {
            return Err(EngineError::Other(format!("{path}: file not found")));
        }
        let frames = if single_file { Vec::new() } else { sequence_frames(s, path) };
        let missing: Vec<usize> = frames.iter().enumerate().filter(|(_, f)| f.is_none()).map(|(i, _)| i).collect();
        let name = name_of(path);
        let asset = VolumeAsset { path: path.clone(), frames: frames.clone() };
        s.media.allow(path);
        let id = s.edit(&format!("Import {name}"), |pr, st| {
            let id = pr.add_item(&name, Label::Mango, ItemKind::Volume(asset), bin);
            st.project_selection = vec![id];
            Ok(id)
        })?;
        items.push(json!({"item": id.0, "frames": frames.len(), "missing": missing}));
    }
    Ok(json!({"items": items}))
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, v) = volume_p(s, p, "vdb.check")?;
    let missing = v.missing(|f| exists(s, f));
    Ok(json!({"item": id.0, "path": v.path, "frames": v.frames.len(), "missing": missing, "online": missing.is_empty()}))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "vdb.relink";
    let (id, _) = volume_p(s, p, CMD)?;
    let path = str_p(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?.to_string();
    if !path.to_ascii_lowercase().ends_with(".vdb") || !exists(s, &path) {
        return Err(EngineError::Other(format!("{path}: not an existing .vdb file")));
    }
    let frames = sequence_frames(s, &path);
    let asset = VolumeAsset { path: path.clone(), frames };
    s.media.allow(&path);
    let n = asset.frames.len();
    s.edit("Relink volume", |pr, _| {
        if let Some(it) = pr.item_mut(id) {
            it.kind = ItemKind::Volume(asset);
        }
        Ok(())
    })?;
    Ok(json!({"item": id.0, "path": path, "frames": n}))
}

fn inspect(_: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("vdb.inspect", "`path` is required"))?;
    let worker = cfa4_vdb::Worker::discover().map_err(|e| EngineError::Other(e.to_string()))?;
    let info = worker.inspect(std::path::Path::new(path)).map_err(|e| EngineError::Other(e.to_string()))?;
    serde_json::to_value(&info).map(|v| json!({"path": path, "inspection": v})).map_err(|e| EngineError::Other(e.to_string()))
}

/// Where a preview goes when the caller names no file: the temp folder, one file per item, frame, size and quality.
fn default_preview_path(item: ItemId, frame: usize, w: u32, h: u32, q: cfa4_vdb::Quality) -> std::path::PathBuf {
    crate::temp_dir().join("CFA4 Volume Previews").join(format!("volume-{}-f{frame}-{w}x{h}-{}.png", item.0, q.id()))
}

fn render(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "vdb.render";
    let (id, v) = volume_p(s, p, CMD)?;
    let frame = u64_p(p, "frame").unwrap_or(0) as usize;
    let input = if v.is_sequence() {
        match v.frames.get(frame) {
            Some(Some(f)) => f.clone(),
            Some(None) => return Err(EngineError::Other(format!("frame {frame} of {} is missing", v.path))),
            None => return Err(bad(CMD, format!("frame {frame} is outside the sequence of {} frames", v.frames.len()))),
        }
    } else if frame == 0 {
        v.path.clone()
    } else {
        return Err(bad(CMD, "a single file has only frame 0"));
    };
    let dim = |k: &str, default: u32| -> Result<u32> {
        match u64_p(p, k) {
            None => Ok(default),
            Some(n) if (1..=8192).contains(&n) => Ok(n as u32),
            Some(_) => Err(bad(CMD, format!("`{k}` must be 1..8192"))),
        }
    };
    let (w, h) = (dim("width", 640)?, dim("height", 360)?);
    // the level is the user's: it is never lowered automatically (N2-1)
    let quality = match str_p(p, "quality") {
        None => cfa4_vdb::Quality::Standard,
        Some(q) => cfa4_vdb::Quality::from_id(q).ok_or_else(|| bad(CMD, "`quality` must be preview, standard or high"))?,
    };
    let mut req = cfa4_vdb::RenderRequest::new(w, h, quality);
    req.grid = str_p(p, "grid").map(str::to_string);
    if let Some(a) = p.get("azimuth").and_then(Value::as_f64) {
        req.azimuth_deg = a;
    }
    if let Some(e) = p.get("elevation").and_then(Value::as_f64) {
        req.elevation_deg = e;
    }
    let out = match str_p(p, "out") {
        Some(o) => std::path::PathBuf::from(o),
        None => default_preview_path(id, frame, w, h, quality),
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
    }
    let worker = cfa4_vdb::Worker::discover().map_err(|e| EngineError::Other(e.to_string()))?;
    let info = worker.render(std::path::Path::new(&input), &out, &req, None).map_err(|e| EngineError::Other(e.to_string()))?;
    // the picture is ordinary media from here on
    let out_s = out.to_string_lossy().to_string();
    let imported = s.execute("file.import", json!({"paths": [out_s]}))?;
    let new_items = imported.get("items").cloned().unwrap_or(Value::Null);
    Ok(json!({"volume": id.0, "frame": frame, "png": out_s, "items": new_items, "info": serde_json::to_value(&info).unwrap_or(Value::Null)}))
}

fn spec(id: &'static str, label: &'static str, params: &'static str, run: crate::commands::Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        spec("vdb.inspect", "Inspect VDB File", r#"{"path":"file.vdb"}"#, inspect, false),
        spec("vdb.import", "Import VDB Volume", r#"{"paths":[str],"sequence":bool?,"bin":binId?}"#, import, true),
        spec("vdb.check", "Check VDB Volume", r#"{"item":id?}"#, check, false),
        spec(
            "vdb.render",
            "Render VDB Preview",
            r#"{"item":id?,"frame":n?,"width":n?,"height":n?,"quality":"preview|standard|high"?,"grid":str?,"azimuth":deg?,"elevation":deg?,"out":path?}"#,
            render,
            true,
        ),
        spec("vdb.relink", "Relink VDB Volume", r#"{"item":id?,"path":str}"#, relink, true),
    ]
}
