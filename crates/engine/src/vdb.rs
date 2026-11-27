//! OpenVDB commands (`vdb.*`), plan CFA4-DNEG-TECHNOLOGY-ADAPTATION-001, N1.
//!
//! | command | does |
//! |---|---|
//! | `vdb.inspect` | grids, value types, voxel size, active voxels, bounding box and metadata of a `.vdb` file, read by the separate `cfa4-vdb-worker` process |
//!
//! | `vdb.import` | adds a `.vdb` file, or a numbered sequence (`name.0001.vdb`), to the Project panel as a volume item (N1-5); no timeline is created |
//! | `vdb.check` | the files of a volume item that are missing on disk (the relink report, N1-7) |
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

fn volume_p(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, VolumeAsset)> {
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

fn spec(id: &'static str, label: &'static str, params: &'static str, run: crate::commands::Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        spec("vdb.inspect", "Inspect VDB File", r#"{"path":"file.vdb"}"#, inspect, false),
        spec("vdb.import", "Import VDB Volume", r#"{"paths":[str],"sequence":bool?,"bin":binId?}"#, import, true),
        spec("vdb.check", "Check VDB Volume", r#"{"item":id?}"#, check, false),
        spec("vdb.relink", "Relink VDB Volume", r#"{"item":id?,"path":str}"#, relink, true),
    ]
}
