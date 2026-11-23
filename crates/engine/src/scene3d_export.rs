//! `scene3d.export` (CFA4-OPENUSD-3D-EXPORT-001, phase U3a): writes a project's 3D asset as glTF
//! (`.glb` or `.gltf` + `.bin`), OBJ (+ MTL) or STL, with a loss report shown first.
//!
//! | param | |
//! |---|---|
//! | `id` | the asset |
//! | `dir` | the output folder; it must exist, nothing outside it is written |
//! | `format` | `glb` (default), `gltf`, `obj`, `stl` (binary) or `stl-ascii` |
//! | `name` | file name without extension (default: the asset's name) |
//! | `approve` | write although the loss report is not empty (the report is returned either way) |
//! | `overwrite` | replace files that already exist (default: refuse and list them) |
//! | `copyTextures` | put the referenced texture images next to the output (default true) |
//! | `embedTextures` | `.glb` only: put the images into the file |
//! | `memoryMb` | memory budget of the read and of the export |
//!
//! Without `approve`, a lossy export is only *planned*: the response carries the loss report and
//! `written: false`; a lossless one is written at once. Writing is atomic per file (a temporary file in
//! the same folder, then a rename or an exclusive link), the source file and its dependencies are never
//! replaced, symbolic links are never followed, and a failure or a cancel leaves no half-written file.
//! The project is not changed (no undo step): the command only reads it.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cfa4_model3d as m3d;
use cfa4_project::scene3d::{Asset3d, Format3d};
use serde_json::{Value, json};

use super::{MAX_FILE_BYTES, memory_p, sha256_file};
use crate::commands::{CommandSpec, bad, bool_p, str_p, u64_p};
use crate::{Result, Session};

const CMD: &str = "scene3d.export";
/// Largest image the exporter reads into memory to embed it.
const MAX_EMBED_BYTES: u64 = 256 << 20;

/// What one output file is made of.
#[derive(Debug)]
pub(super) enum Content {
    Bytes(Vec<u8>),
    Copy(PathBuf),
}

#[derive(Debug)]
pub(super) struct Planned {
    pub dest: PathBuf,
    pub content: Content,
}

fn io_err(path: &Path, e: &std::io::Error) -> String {
    format!("{}: {e}", path.display())
}

/// Reads the asset's scene again (the same read as the import), after checking the file is unchanged.
fn load_scene(a: &Asset3d, memory: usize) -> std::result::Result<m3d::Scene, String> {
    let path = PathBuf::from(&a.path);
    let hash = sha256_file(&path).map_err(|e| format!("{}: {e}; relink the asset (scene3d.relink)", a.path))?;
    if hash != a.sha256 {
        return Err("the file changed since it was imported; relink it with allowChanged first".into());
    }
    let data = std::fs::read(&path).map_err(|e| io_err(&path, &e))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut resolver = move |uri: &str| {
        let p = dir.join(uri);
        let meta = std::fs::metadata(&p).ok()?;
        (meta.is_file() && meta.len() <= MAX_FILE_BYTES).then(|| std::fs::read(&p).ok()).flatten()
    };
    let format = match a.format {
        Format3d::Gltf => m3d::Format::Gltf,
        Format3d::Obj => m3d::Format::Obj,
        Format3d::Stl => m3d::Format::Stl,
        Format3d::Usd => return Err("USD is not read natively".into()),
    };
    let limits = m3d::Limits { max_memory_bytes: memory, ..m3d::Limits::default() };
    m3d::read_as(format, &data, &mut resolver, &limits).map_err(|e| format!("{}: {e}", a.path))
}

/// The output folder: it exists, is a directory, and is spelled canonically.
fn out_dir(p: &Value) -> Result<PathBuf> {
    let s = str_p(p, "dir").map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad(CMD, "give the output folder (`dir`)"))?;
    if s.contains('\0') || s.len() > cfa4_project::scene3d::MAX_PATH_BYTES {
        return Err(bad(CMD, "the folder is too long or not a path"));
    }
    let dir = std::fs::canonicalize(s).map_err(|e| bad(CMD, format!("{s}: {e} (the output folder must exist)")))?;
    if !dir.is_dir() {
        return Err(bad(CMD, format!("{}: not a folder", dir.display())));
    }
    Ok(dir)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Plans where every file goes, and refuses what must never be written: a symbolic link, a folder, the
/// source asset or one of its dependencies, two outputs with the same name, and (without `overwrite`) a
/// file that already exists. Texture copies whose destination already holds the same bytes are skipped.
pub(super) fn plan_outputs(
    dir: &Path,
    files: &[m3d::ExportFile],
    copies: &[(String, PathBuf)],
    protected: &[PathBuf],
    overwrite: bool,
) -> std::result::Result<Vec<Planned>, String> {
    let mut out: Vec<Planned> = Vec::new();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    let mut existing: Vec<String> = Vec::new();
    let mut check = |rel: &str, content: Content, copy_of: Option<&Path>| -> std::result::Result<(), String> {
        if !m3d::safe_relative_uri(rel) {
            return Err(format!("{rel}: not a safe relative file name"));
        }
        // every folder on the way must be a real folder, never a link
        let mut at = dir.to_path_buf();
        let parts: Vec<&str> = rel.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(&at, &e)),
                Ok(m) if m.file_type().is_symlink() => return Err(format!("{}: is a symbolic link; it is never followed", at.display())),
                Ok(m) if i + 1 < parts.len() && !m.is_dir() => return Err(format!("{}: is a file, not a folder", at.display())),
                Ok(m) if i + 1 == parts.len() && m.is_dir() => return Err(format!("{}: is a folder", at.display())),
                Ok(_) => {}
            }
        }
        let dest = at;
        if !taken.insert(dest.clone()) {
            return Err(format!("{rel}: two outputs have the same name"));
        }
        if dest.exists() {
            if protected.iter().any(|p| same_file(p, &dest)) {
                return Err(format!("{}: is the source asset or one of its files; it is never overwritten", dest.display()));
            }
            if let Some(src) = copy_of
                && sha256_file(src).ok().is_some_and(|a| sha256_file(&dest).ok() == Some(a))
            {
                return Ok(()); // the same image is already there
            }
            if !overwrite {
                existing.push(rel.to_string());
                return Ok(());
            }
        }
        out.push(Planned { dest, content });
        Ok(())
    };
    for f in files {
        check(&f.name, Content::Bytes(f.data.clone()), None)?;
    }
    for (rel, src) in copies {
        check(rel, Content::Copy(src.clone()), Some(src))?;
    }
    if !existing.is_empty() {
        return Err(format!("these files already exist: {}; pass `overwrite` to replace them, or choose another `name` or folder", existing.join(", ")));
    }
    Ok(out)
}

fn stage_name(dest: &Path, n: usize) -> PathBuf {
    let name = dest.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{name}.cfa4-{}-{n}.tmp", std::process::id()))
}

fn stage(p: &Planned, tmp: &Path) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
    match &p.content {
        Content::Bytes(b) => f.write_all(b)?,
        Content::Copy(src) => {
            let mut r = std::fs::File::open(src)?;
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = r.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                f.write_all(buf.get(..n).unwrap_or(&[]))?;
            }
        }
    }
    f.sync_all()
}

fn publish(tmp: &Path, dest: &Path, overwrite: bool) -> std::io::Result<()> {
    if overwrite {
        return std::fs::rename(tmp, dest);
    }
    match std::fs::hard_link(tmp, dest) {
        Ok(()) => std::fs::remove_file(tmp),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        Err(_) if !dest.exists() => std::fs::rename(tmp, dest), // a filesystem without hard links
        Err(e) => Err(e),
    }
}

/// Writes the planned files: all are staged in temporary files first, then published one by one. A
/// failure, or `cancelled()` turning true, removes every temporary file and every file this call newly
/// created; nothing half-written stays. (With `overwrite`, a file that was already replaced cannot be
/// brought back; the error says so.)
pub(super) fn commit(planned: &[Planned], overwrite: bool, cancelled: &dyn Fn() -> bool) -> std::result::Result<Vec<PathBuf>, String> {
    let mut staged: Vec<PathBuf> = Vec::new();
    let remove_all = |v: &[PathBuf]| {
        for p in v {
            let _ = std::fs::remove_file(p);
        }
    };
    for (n, p) in planned.iter().enumerate() {
        if cancelled() {
            remove_all(&staged);
            return Err("cancelled; nothing was written".into());
        }
        if let Some(parent) = p.dest.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            remove_all(&staged);
            return Err(io_err(parent, &e));
        }
        let tmp = stage_name(&p.dest, n);
        staged.push(tmp.clone());
        if let Err(e) = stage(p, &tmp) {
            remove_all(&staged);
            return Err(io_err(&p.dest, &e));
        }
    }
    let mut created: Vec<PathBuf> = Vec::new();
    let mut replaced = 0usize;
    for (p, tmp) in planned.iter().zip(&staged) {
        let existed = p.dest.exists();
        let stop = cancelled().then(|| "cancelled".to_string());
        let res = match stop {
            Some(s) => Err(s),
            None => publish(tmp, &p.dest, overwrite).map_err(|e| io_err(&p.dest, &e)),
        };
        match res {
            Ok(()) if existed => replaced += 1,
            Ok(()) => created.push(p.dest.clone()),
            Err(e) => {
                remove_all(&staged);
                remove_all(&created);
                let note = if replaced > 0 { format!("; {replaced} existing file(s) had already been replaced") } else { String::new() };
                return Err(format!("{e}; the files written so far were removed{note}"));
            }
        }
    }
    remove_all(&staged); // staged files already renamed are gone; hard-linked ones were removed
    Ok(planned.iter().map(|p| p.dest.clone()).collect())
}

fn report_json(r: &m3d::Report) -> Value {
    Value::Array(
        r.items
            .iter()
            .map(|i| {
                json!({
                    "severity": match i.severity { m3d::Severity::Unsupported => "unsupported", m3d::Severity::Approximated => "approximated" },
                    "path": i.path,
                    "message": i.message,
                })
            })
            .collect(),
    )
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let id = u64_p(p, "id").ok_or_else(|| bad(CMD, "name the asset (`id`)"))?;
    let Some(a) = s.project.scene3d.asset(id).cloned() else { return Err(bad(CMD, format!("there is no 3D asset {id}"))) };
    if a.format == Format3d::Usd {
        return Err(bad(
            CMD,
            "USD export (U3b) needs the USD worker (U2), which is not enabled in this build (no native USD, G-USD-1); export a glTF, OBJ or STL asset",
        ));
    }
    let target = match p.get("format") {
        None | Some(Value::Null) => m3d::Target::Glb,
        Some(v) => v.as_str().and_then(m3d::Target::from_name).ok_or_else(|| bad(CMD, "`format` is glb, gltf, obj, stl or stl-ascii"))?,
    };
    let dir = out_dir(p)?;
    let memory = memory_p(CMD, p, "memoryMb")?;
    let approve = bool_p(p, "approve").unwrap_or(false);
    let overwrite = bool_p(p, "overwrite").unwrap_or(false);
    let copy_textures = bool_p(p, "copyTextures").unwrap_or(true);
    let embed = bool_p(p, "embedTextures").unwrap_or(false);
    let stem = match p.get("name") {
        None | Some(Value::Null) => {
            let base = std::path::Path::new(&a.name).file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| a.name.clone());
            m3d::sanitize_stem(&base)
        }
        Some(Value::String(n)) if !n.trim().is_empty() => m3d::sanitize_stem(n),
        Some(_) => return Err(bad(CMD, "`name` is a non-empty string")),
    };

    let scene = load_scene(&a, memory).map_err(|e| bad(CMD, e))?;
    let source_dir = Path::new(&a.path).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut opts = m3d::ExportOptions::new(target, &stem);
    opts.generator = format!("CFA4 {}", env!("CARGO_PKG_VERSION"));
    opts.embed_textures = embed;
    opts.max_memory_bytes = memory;
    let mut resolver = |uri: &str| -> Option<Vec<u8>> {
        let p = source_dir.join(uri);
        let meta = std::fs::metadata(&p).ok()?;
        (meta.is_file() && meta.len() <= MAX_EMBED_BYTES).then(|| std::fs::read(&p).ok()).flatten()
    };
    let exp = m3d::export(&scene, &opts, &mut resolver).map_err(|e| bad(CMD, e.to_string()))?;

    // textures: copy the ones that are there, name the others
    let mut copies: Vec<(String, PathBuf)> = Vec::new();
    let mut tex_rows: Vec<Value> = Vec::new();
    for uri in &exp.textures {
        let src = source_dir.join(uri);
        let there = std::fs::metadata(&src).is_ok_and(|m| m.is_file());
        let status = match (copy_textures, there) {
            (false, _) => "referenced",
            (true, false) => "missing",
            (true, true) => {
                copies.push((uri.clone(), src));
                "copy"
            }
        };
        tex_rows.push(json!({"uri": uri, "status": status}));
    }
    let mut protected: Vec<PathBuf> = vec![PathBuf::from(&a.path)];
    protected.extend(a.dependencies.iter().map(|d| source_dir.join(&d.uri)));
    let planned = plan_outputs(&dir, &exp.files, &copies, &protected, overwrite).map_err(|e| bad(CMD, e))?;

    let lossless = exp.report.is_clean();
    let mut out = json!({
        "id": id,
        "format": target.name(),
        "dir": dir.to_string_lossy(),
        "files": exp.files.iter().map(|f| json!({"name": f.name, "bytes": f.data.len()})).collect::<Vec<_>>(),
        "textures": tex_rows,
        "report": report_json(&exp.report),
        "dropped": exp.report.dropped,
        "lossless": lossless,
        "needsApproval": !lossless,
        "written": false,
    });
    if !lossless && !approve {
        out["message"] = json!("the target format cannot carry everything (see `report`); nothing was written; repeat with `approve` to write it anyway");
        return Ok(out);
    }
    let written = commit(&planned, overwrite, &|| false).map_err(|e| bad(CMD, e))?;
    out["written"] = json!(true);
    out["paths"] = json!(written.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>());
    Ok(out)
}

pub(super) fn spec() -> CommandSpec {
    CommandSpec {
        id: CMD,
        label: "Export 3D Asset",
        menu: &[],
        shortcut: None,
        params: r#"{"id":asset,"dir":folder,"format":"glb"|"gltf"|"obj"|"stl"|"stl-ascii"?,"name":string?,"approve":bool?,"overwrite":bool?,"copyTextures":bool?,"embedTextures":bool?,"memoryMb":n?} → {files, textures, report, lossless, needsApproval, written, paths?}"#,
        enabled: super::has_assets,
        run: export,
        journal: false,
    }
}

#[cfg(test)]
#[path = "scene3d_export_tests.rs"]
mod tests;
