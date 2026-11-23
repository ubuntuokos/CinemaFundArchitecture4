//! The project's 3D assets (CFA4-OPENUSD-3D-001 phase U1, `scene3d`).
//!
//! There is no global 3D switch. Importing a 3D file creates the project's `scene3d` section
//! (`Project::scene3d`) and switches on the capabilities of that content type; removing the last asset
//! takes the section away again, so a project without 3D content is exactly what it was (P-A1, P-A2).
//! The project keeps a *reference* (file path + SHA-256), the importer's unsupported report and the
//! chosen route, never the geometry. Every change is one undoable project edit.
//!
//! | command | params | what |
//! |---|---|---|
//! | `scene3d.list` | – | the assets, the capabilities switched on and whether each file is still there |
//! | `scene3d.import` | `{path, route?, name?, memoryMb?}` | reads the file (glTF/GLB, OBJ, STL; USD is referenced on route A only), keeps path + SHA-256 + report |
//! | `scene3d.route` | `{id, route}` | `"A"` (external EXR replacement, default) or `"B"` (native import); reference and report stay |
//! | `scene3d.relink` | `{id, path, allowChanged?, memoryMb?}` | points the asset at another file; a different SHA-256 is refused unless `allowChanged` (then the file is read again) |
//! | `scene3d.check` | `{id?}` | per asset: `ok`, `missing` (relink it) or `changed` (the file's SHA-256 differs from the import) |
//! | `scene3d.lod` | `{id, memoryMb, maxTriangles?, region?, approve?, memoryMbRead?}` | the level-of-detail plan against the frame, with reasons; `approve` applies it (nothing is stored) |
//! | `scene3d.remove` | `{id}` | removes the reference (the file is not touched) |
//! | `scene3d.export` | `{id, dir, format?, name?, approve?, overwrite?, copyTextures?, embedTextures?, memoryMb?}` | U3a: writes the asset as glTF/GLB, OBJ or STL into `dir`, loss report first (see `scene3d_export.rs`) |

use std::io::Read;
use std::path::{Path, PathBuf};

use cfa4_model3d as m3d;
use cfa4_project::scene3d::{self as model, Asset3d, AssetStats, Dependency, Format3d, ReportEntry, ReportSeverity, Route};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::commands::{CommandSpec, always, bad, str_p, u64_p};
use crate::{Result, Session};

/// The importer named in the project; bump when a reader changes what it keeps.
pub const IMPORTER: &str = "cfa4-model3d 1";
/// Largest 3D file (and external resource) read.
const MAX_FILE_BYTES: u64 = 1 << 30;
/// Default memory budget of one read, in MiB.
const DEFAULT_MEMORY_MB: u64 = 1024;

fn route_json(r: Route) -> &'static str {
    match r {
        Route::A => "A",
        Route::B => "B",
    }
}

fn asset_json(a: &Asset3d, present: bool) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or_else(|_| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("present".into(), json!(present));
        o.insert("unsupported".into(), json!(a.report.iter().filter(|r| r.severity == ReportSeverity::Unsupported).count()));
    }
    v
}

fn route_p(cmd: &str, p: &Value) -> Result<Option<Route>> {
    match p.get("route") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("a") => Ok(Some(Route::A)),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("b") => Ok(Some(Route::B)),
        Some(_) => Err(bad(cmd, "`route` is \"A\" (external EXR replacement) or \"B\" (native import)")),
    }
}

fn memory_p(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    let mb = match p.get(key) {
        None | Some(Value::Null) => DEFAULT_MEMORY_MB,
        Some(v) => v.as_u64().filter(|n| (1..=1 << 20).contains(n)).ok_or_else(|| bad(cmd, format!("`{key}` is a whole number of MiB, 1 to {}", 1 << 20)))?,
    };
    usize::try_from(mb << 20).map_err(|_| bad(cmd, format!("`{key}` is too large")))
}

fn path_p(cmd: &str, p: &Value) -> Result<PathBuf> {
    let s = str_p(p, "path").map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "give the 3D file (`path`)"))?;
    if s.len() > model::MAX_PATH_BYTES || s.contains('\0') {
        return Err(bad(cmd, "the path is too long or not a path"));
    }
    Ok(std::fs::canonicalize(s).unwrap_or_else(|_| PathBuf::from(s)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file, streamed (the file is not held in memory for the hash alone).
fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok(hex(&h.finalize()))
}

/// What reading a 3D file produced.
struct Imported {
    format: Format3d,
    sha256: String,
    importer: Option<String>,
    stats: Option<AssetStats>,
    report: Vec<ReportEntry>,
    dependencies: Vec<Dependency>,
    dropped: u64,
}

fn sha256_bytes(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn convert_report(r: &m3d::Report) -> (Vec<ReportEntry>, u64) {
    let items = r
        .items
        .iter()
        .map(|i| ReportEntry {
            severity: match i.severity {
                m3d::Severity::Unsupported => ReportSeverity::Unsupported,
                m3d::Severity::Approximated => ReportSeverity::Approximated,
            },
            path: i.path.clone(),
            message: i.message.clone(),
        })
        .collect();
    (items, r.dropped as u64)
}

/// Reads `path`: the format by extension, the hash of the file, and (except for USD) the scene through
/// `cfa4-model3d` with sibling files resolved next to it. Any failure is an error message, never a panic.
fn read_file(path: &Path, memory: usize) -> std::result::Result<Imported, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let format = Format3d::from_extension(ext).ok_or_else(|| format!("{}: not a glTF, GLB, OBJ, STL or USD file (by its extension)", path.display()))?;
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{}: not a file", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{}: larger than {} MiB", path.display(), MAX_FILE_BYTES >> 20));
    }
    let sha256 = sha256_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if format == Format3d::Usd {
        let report = vec![ReportEntry {
            severity: ReportSeverity::Unsupported,
            path: "usd".into(),
            message: "USD is not read natively in this build; the file is referenced and handed to the external backend (route A)".into(),
        }];
        return Ok(Imported { format, sha256, importer: None, stats: None, report, dependencies: Vec::new(), dropped: 0 });
    }
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut deps: Vec<Dependency> = Vec::new();
    let mut resolver = |uri: &str| -> Option<Vec<u8>> {
        // the reader has already refused absolute paths, `..` and schemes
        let p = dir.join(uri);
        let meta = std::fs::metadata(&p).ok()?;
        let bytes = (meta.is_file() && meta.len() <= MAX_FILE_BYTES).then(|| std::fs::read(&p).ok()).flatten()?;
        if !deps.iter().any(|d| d.uri == uri) {
            deps.push(Dependency { uri: uri.to_string(), sha256: sha256_bytes(&bytes) });
        }
        Some(bytes)
    };
    let mformat = match format {
        Format3d::Gltf => m3d::Format::Gltf,
        Format3d::Obj => m3d::Format::Obj,
        Format3d::Stl => m3d::Format::Stl,
        Format3d::Usd => return Err("USD is not read natively".into()),
    };
    let limits = m3d::Limits { max_memory_bytes: memory, ..m3d::Limits::default() };
    let scene = m3d::read_as(mformat, &data, &mut resolver, &limits).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut extra_report = Vec::new();
    // texture images are references the reader does not open: hash the ones that are there
    for (mi, tex) in scene.materials.iter().enumerate().flat_map(|(i, m)| m.textures.iter().map(move |t| (i, t))) {
        let Some(uri) = tex.uri.as_deref().filter(|u| m3d::safe_relative_uri(u)) else { continue };
        if deps.iter().any(|d| d.uri == uri) {
            continue;
        }
        let p = dir.join(uri);
        match std::fs::metadata(&p) {
            Ok(m) if m.is_file() && m.len() <= MAX_FILE_BYTES => match sha256_file(&p) {
                Ok(h) => deps.push(Dependency { uri: uri.to_string(), sha256: h }),
                Err(e) => extra_report.push((mi, tex.slot.clone(), format!("{uri}: {e}"))),
            },
            _ => extra_report.push((mi, tex.slot.clone(), format!("{uri}: not found next to the file"))),
        }
    }
    let stats = AssetStats {
        nodes: scene.nodes.len() as u64,
        meshes: scene.meshes.len() as u64,
        vertices: scene.vertex_count() as u64,
        triangles: scene.triangle_count() as u64,
        cameras: scene.cameras.len() as u64,
        materials: scene.materials.len() as u64,
        animations: scene.animations.len() as u64,
    };
    let (mut report, dropped) = convert_report(&scene.report);
    for (mi, slot, msg) in extra_report {
        if report.len() < model::MAX_REPORT_ITEMS {
            report.push(ReportEntry {
                severity: ReportSeverity::Approximated,
                path: format!("materials[{mi}].{slot}"),
                message: format!("texture image: {msg}"),
            });
        }
    }
    deps.truncate(model::MAX_DEPENDENCIES);
    Ok(Imported { format, sha256, importer: Some(IMPORTER.into()), stats: Some(stats), report, dependencies: deps, dropped })
}

fn name_of(cmd: &str, p: &Value, path: &Path) -> Result<String> {
    match p.get("name") {
        None | Some(Value::Null) => {
            Ok(path.file_name().map_or_else(|| "3D asset".into(), |f| f.to_string_lossy().chars().take(model::MAX_NAME_CHARS).collect()))
        }
        Some(Value::String(s)) if !s.trim().is_empty() && s.chars().count() <= model::MAX_NAME_CHARS => Ok(s.trim().to_string()),
        Some(_) => Err(bad(cmd, format!("`name` is a non-empty string of at most {} characters", model::MAX_NAME_CHARS))),
    }
}

fn asset_id_p(s: &Session, cmd: &str, p: &Value) -> Result<u64> {
    let id = u64_p(p, "id").ok_or_else(|| bad(cmd, "name the asset (`id`)"))?;
    s.project.scene3d.asset(id).map(|a| a.id).ok_or_else(|| bad(cmd, format!("there is no 3D asset {id}")))
}

fn present(a: &Asset3d) -> bool {
    std::fs::metadata(&a.path).is_ok_and(|m| m.is_file())
}

// ------------------------------------------------------------------------------------- commands

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let sc = &s.project.scene3d;
    let caps: serde_json::Map<String, Value> = sc.capabilities.iter().map(|(f, c)| (f.name().to_string(), json!(c))).collect();
    Ok(json!({"assets": sc.assets.iter().map(|a| asset_json(a, present(a))).collect::<Vec<_>>(), "capabilities": caps}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.import";
    if s.project.scene3d.assets.len() >= model::MAX_ASSETS {
        return Err(bad(CMD, format!("at most {} 3D assets", model::MAX_ASSETS)));
    }
    let path = path_p(CMD, p)?;
    let route = route_p(CMD, p)?.unwrap_or_default();
    let name = name_of(CMD, p, &path)?;
    let memory = memory_p(CMD, p, "memoryMb")?;
    let got = read_file(&path, memory).map_err(|e| bad(CMD, e))?;
    if route == Route::B && !Route::native_supported(got.format) {
        return Err(bad(CMD, "USD has no native import in this build (route B); use route A (the default)"));
    }
    let path_s = path.to_string_lossy().into_owned();
    if let Some(dup) = s.project.scene3d.assets.iter().find(|a| a.path == path_s && a.sha256 == got.sha256) {
        return Err(bad(CMD, format!("this file is already imported as asset {}", dup.id)));
    }
    let id = s.edit("Import 3D Asset", |pr, _| {
        let id = pr.alloc_id();
        pr.scene3d.assets.push(Asset3d {
            id,
            name,
            path: path_s,
            sha256: got.sha256,
            format: got.format,
            route,
            importer: got.importer,
            stats: got.stats,
            report: got.report,
            dependencies: got.dependencies,
            dropped: got.dropped,
        });
        pr.scene3d.refresh_capabilities();
        Ok(id)
    })?;
    Ok(s.project.scene3d.asset(id).map(|a| asset_json(a, true)).unwrap_or(Value::Null))
}

fn set_route(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.route";
    let id = asset_id_p(s, CMD, p)?;
    let route = route_p(CMD, p)?.ok_or_else(|| bad(CMD, "give the `route`: \"A\" or \"B\""))?;
    let format = s.project.scene3d.asset(id).map(|a| a.format).unwrap_or(Format3d::Usd);
    if route == Route::B && !Route::native_supported(format) {
        return Err(bad(CMD, "USD has no native import in this build (route B); it stays on route A"));
    }
    if s.project.scene3d.asset(id).is_some_and(|a| a.route != route) {
        s.edit("Change 3D Route", |pr, _| {
            if let Some(a) = pr.scene3d.asset_mut(id) {
                a.route = route;
            }
            Ok(())
        })?;
    }
    Ok(json!({"id": id, "route": route_json(route)}))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.relink";
    let id = asset_id_p(s, CMD, p)?;
    let path = path_p(CMD, p)?;
    let allow = p.get("allowChanged").and_then(Value::as_bool).unwrap_or(false);
    let memory = memory_p(CMD, p, "memoryMb")?;
    let Some(old) = s.project.scene3d.asset(id).cloned() else { return Err(bad(CMD, format!("there is no 3D asset {id}"))) };
    let got = read_file(&path, memory).map_err(|e| bad(CMD, e))?;
    if got.format != old.format {
        return Err(bad(CMD, format!("the new file is {}, the asset is {}", got.format.name(), old.format.name())));
    }
    let same = got.sha256 == old.sha256;
    if !same && !allow {
        return Err(bad(
            CMD,
            format!("the file differs from the one imported (SHA-256 {} instead of {}); pass `allowChanged` to read it again", got.sha256, old.sha256),
        ));
    }
    let path_s = path.to_string_lossy().into_owned();
    s.edit("Relink 3D Asset", |pr, _| {
        if let Some(a) = pr.scene3d.asset_mut(id) {
            a.path = path_s;
            if !same {
                a.sha256 = got.sha256;
                a.importer = got.importer;
                a.stats = got.stats;
                a.report = got.report;
                a.dependencies = got.dependencies;
                a.dropped = got.dropped;
            }
        }
        Ok(())
    })?;
    Ok(json!({"asset": s.project.scene3d.asset(id).map(|a| asset_json(a, true)), "changed": !same}))
}

fn check_one(a: &Asset3d) -> Value {
    let (status, message) = match std::fs::metadata(&a.path) {
        Err(e) => ("missing", format!("{}: {e}; relink it (scene3d.relink)", a.path)),
        Ok(m) if !m.is_file() => ("missing", format!("{}: not a file; relink it (scene3d.relink)", a.path)),
        Ok(_) => match sha256_file(Path::new(&a.path)) {
            Ok(h) if h == a.sha256 => dependency_status(a),
            Ok(h) => ("changed", format!("SHA-256 is {h}, imported as {}; relink with allowChanged to read it again", a.sha256)),
            Err(e) => ("missing", format!("{}: {e}", a.path)),
        },
    };
    json!({"id": a.id, "name": a.name, "status": status, "message": message})
}

/// The main file is as imported: are its buffers, materials and textures too?
fn dependency_status(a: &Asset3d) -> (&'static str, String) {
    let dir = Path::new(&a.path).parent().map(Path::to_path_buf).unwrap_or_default();
    for d in &a.dependencies {
        match sha256_file(&dir.join(&d.uri)) {
            Ok(h) if h == d.sha256 => {}
            Ok(h) => return ("changed", format!("{}: SHA-256 is {h}, imported as {}; relink with allowChanged to read it again", d.uri, d.sha256)),
            Err(e) => return ("missing", format!("{}: {e}; put it back next to the file", d.uri)),
        }
    }
    ("ok", String::new())
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.check";
    let sc = &s.project.scene3d;
    let rows: Vec<Value> = match u64_p(p, "id") {
        Some(id) => vec![check_one(sc.asset(id).ok_or_else(|| bad(CMD, format!("there is no 3D asset {id}")))?)],
        None => sc.assets.iter().map(check_one).collect(),
    };
    let problems = rows.iter().filter(|r| r["status"] != "ok").count();
    Ok(json!({"assets": rows, "problems": problems}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = asset_id_p(s, "scene3d.remove", p)?;
    s.edit("Remove 3D Asset", |pr, _| {
        pr.scene3d.assets.retain(|a| a.id != id);
        pr.scene3d.refresh_capabilities();
        Ok(())
    })?;
    Ok(json!({"removed": id, "scene3d": !s.project.scene3d.is_empty()}))
}

fn bounds_p(cmd: &str, v: &Value) -> Result<m3d::lod::Bounds> {
    let triple = |k: &str| -> Result<[f32; 3]> {
        let a = v.get(k).and_then(Value::as_array).filter(|a| a.len() == 3).ok_or_else(|| bad(cmd, format!("`region.{k}` is [x, y, z]")))?;
        let mut out = [0f32; 3];
        for (i, x) in a.iter().enumerate() {
            out[i] = x.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(cmd, format!("`region.{k}` holds finite numbers")))? as f32;
        }
        Ok(out)
    };
    let b = m3d::lod::Bounds { min: triple("min")?, max: triple("max")? };
    if b.is_valid() { Ok(b) } else { Err(bad(cmd, "`region.min` must not exceed `region.max`")) }
}

fn lod(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "scene3d.lod";
    let id = asset_id_p(s, CMD, p)?;
    let Some(a) = s.project.scene3d.asset(id).cloned() else { return Err(bad(CMD, format!("there is no 3D asset {id}"))) };
    if !Route::native_supported(a.format) {
        return Err(bad(CMD, "USD has no native import in this build; level of detail needs a natively read asset"));
    }
    let frame_bytes = memory_p(CMD, p, "memoryMb")?;
    let max_triangles = match p.get("maxTriangles") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(CMD, "`maxTriangles` is a whole number"))?),
    };
    let approve = p.get("approve").and_then(Value::as_bool).unwrap_or(false);
    let read_mem = memory_p(CMD, p, "memoryMbRead")?;
    let path = PathBuf::from(&a.path);
    let got_hash = sha256_file(&path).map_err(|e| bad(CMD, format!("{}: {e}; relink the asset (scene3d.relink)", a.path)))?;
    if got_hash != a.sha256 {
        return Err(bad(CMD, "the file changed since it was imported; relink it with allowChanged first"));
    }
    // the same read as the import, kept this time
    let data = std::fs::read(&path).map_err(|e| bad(CMD, format!("{}: {e}", a.path)))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut resolver = move |uri: &str| std::fs::read(dir.join(uri)).ok();
    let mformat = match a.format {
        Format3d::Gltf => m3d::Format::Gltf,
        Format3d::Obj => m3d::Format::Obj,
        _ => m3d::Format::Stl,
    };
    let limits = m3d::Limits { max_memory_bytes: read_mem, ..m3d::Limits::default() };
    let mut scene = m3d::read_as(mformat, &data, &mut resolver, &limits).map_err(|e| bad(CMD, format!("{}: {e}", a.path)))?;
    if let Some(r) = p.get("region").filter(|v| !v.is_null()) {
        scene = m3d::lod::extract_region(&scene, bounds_p(CMD, r)?);
    }
    let plan = m3d::lod::plan(&scene, m3d::lod::Frame { memory_bytes: frame_bytes, max_triangles });
    let levels: Vec<Value> = plan.levels.iter().map(|l| json!({"level": format!("{:?}", l.level), "triangles": l.triangles, "bytes": l.bytes})).collect();
    let mut out = json!({
        "id": id,
        "levels": levels,
        "chosen": plan.chosen,
        "fits": plan.fits,
        "changesResult": plan.changes_result,
        "needsApproval": plan.needs_approval,
        "reasons": plan.reasons,
        "applied": false,
    });
    if (approve || !plan.needs_approval)
        && let Some(done) = m3d::lod::apply(&scene, &plan, approve)
    {
        out["applied"] = json!(true);
        out["triangles"] = json!(done.triangle_count());
        out["vertices"] = json!(done.vertex_count());
    }
    Ok(out)
}

fn has_assets(s: &Session) -> std::result::Result<(), String> {
    if s.project.scene3d.assets.is_empty() { Err("the project has no 3D assets".into()) } else { Ok(()) }
}

#[path = "scene3d_export.rs"]
mod export;

pub(crate) fn commands() -> Vec<CommandSpec> {
    let c = |id, label, params, enabled, run| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal: true };
    let q = |id, label, params, enabled, run| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal: false };
    vec![
        q("scene3d.list", "List 3D Assets", "{} → {assets:[{id, name, path, sha256, format, route, stats, report, present}], capabilities}", always, list),
        c("scene3d.import", "Import 3D Asset", r#"{"path":file,"route":"A"|"B"?,"name":string?,"memoryMb":n?}"#, always, import),
        c("scene3d.route", "Change 3D Route", r#"{"id":asset,"route":"A"|"B"}"#, has_assets, set_route),
        c("scene3d.relink", "Relink 3D Asset", r#"{"id":asset,"path":file,"allowChanged":bool?,"memoryMb":n?}"#, has_assets, relink),
        q("scene3d.check", "Check 3D Assets", r#"{"id":asset?} → {assets:[{id, name, status: ok|missing|changed, message}], problems}"#, has_assets, check),
        q(
            "scene3d.lod",
            "Plan 3D Level of Detail",
            r#"{"id":asset,"memoryMb":n?,"maxTriangles":n?,"region":{"min":[x,y,z],"max":[x,y,z]}?,"approve":bool?,"memoryMbRead":n?}"#,
            has_assets,
            lod,
        ),
        c("scene3d.remove", "Remove 3D Asset", r#"{"id":asset}"#, has_assets, remove),
        export::spec(),
    ]
}

#[cfg(test)]
#[path = "scene3d_tests.rs"]
mod tests;
