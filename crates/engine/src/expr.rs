//! CFX expression commands (`expr.*`), plan CFA4-AX-INTERPRETER-001, X2 and X3.
//!
//! | command | does |
//! |---|---|
//! | `expr.check` | compiles a source against the attributes the caller names and answers `ok` with what it reads, writes and declares, or `ok: false` with the diagnostic (code, line, column); runs nothing |
//! | `expr.save` | stores an expression (source, hash, grid, outputs, parameter values, fuel) in the project, one undo step; the user who saves it approves its hash on this machine |
//! | `expr.list`, `expr.get` | the saved expressions, each with `approved` (this machine) |
//! | `expr.delete` | removes a saved expression, one undo step |
//! | `expr.approve`, `expr.revoke` | records / withdraws this machine's approval of a saved expression's hash |
//! | `expr.bake` | runs a saved expression over a volume (same limits as the preview) and imports the result next to the source file; refuses unless this machine approved the hash |
//! | `expr.export`, `expr.import` | preset files (`cfa4.expr-presets`); imported expressions are **not** approved |
//! | `expr.run_preview` | runs a CFX expression over the active voxels of one grid of a volume item (limits: 30 s, 1 GiB of active voxel data, fuel per voxel), writes a new `.vdb` with the original grids plus the written ones, and imports it as a new volume item; with `render: true` it also renders a density preview of the first written float grid through the N2 path (`vdb.render`) |
//!
//! **Approval (plan §5.1, N3-7′).** Opening a project never compiles or runs an expression, and the project file
//! never records an approval. Approvals live per machine (`<data dir>/expression-approvals.json`, hashes only) and
//! are keyed by the SHA-256 of the source text; text that did not come from this machine's user (a project or preset
//! from elsewhere) is `approved: false` until `expr.approve`, and `expr.bake` refuses it (`E-APPROVAL`). Editing the
//! text changes the hash, so an edit must be saved (and so approved) again. `expr.run_preview` runs text the caller
//! gives it, as a user action, and is unchanged.
//!
//! The expression language is `docs/expr-language.md`. A compile error, a run-time error (`R-FUEL`, `R-INT`, ...),
//! a limit, a missing worker or a cancelled run is an ordinary error message that keeps the stable code and the
//! place; nothing is imported then, and the project is unchanged.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use cfa4_expr::{Context, Schema, Ty, Value as Cv};
use cfa4_project::expression::{ExprError, ExprId, Expression, source_hash};

use crate::commands::{CommandSpec, always, bad, str_p, u64_p};
use crate::{EngineError, Result, Session};

static RUN: AtomicU64 = AtomicU64::new(0);

fn json_value(v: &Value) -> Option<Cv> {
    match v {
        Value::Bool(b) => Some(Cv::Bool(*b)),
        Value::Number(n) => n.as_i64().and_then(|i| i32::try_from(i).ok()).map(Cv::Int).or_else(|| n.as_f64().map(|f| Cv::Float(f as f32))),
        Value::Array(a) if a.len() == 3 => {
            let f: Vec<f32> = a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
            (f.len() == 3).then(|| Cv::Vec3([f[0], f[1], f[2]]))
        }
        _ => None,
    }
}

/// `<dir of the source>/<stem>.<tag>.vdb`, numbered (`-2`, `-3`, …) while the name is taken.
fn baked_path(input: &std::path::Path, tag: &str) -> std::path::PathBuf {
    let dir = input.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let stem = input.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "volume".into());
    let first = dir.join(format!("{stem}.{tag}.vdb"));
    if !first.exists() {
        return first;
    }
    (2..10_000u32).map(|k| dir.join(format!("{stem}.{tag}-{k}.vdb"))).find(|c| !c.exists()).unwrap_or(first)
}

fn run_preview(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, "expr.run_preview", None)
}

/// The shared run of `expr.run_preview` and `expr.bake`. A bake (`bake_tag` is the part of the file name that names
/// the expression) writes next to the source file and never over an existing file; the preview uses the temp folder.
fn run(s: &mut Session, p: &Value, cmd: &str, bake_tag: Option<&str>) -> Result<Value> {
    let source = str_p(p, "source").ok_or_else(|| bad(cmd, "`source` is required"))?;
    let domain = str_p(p, "domain").ok_or_else(|| bad(cmd, "`domain` (the grid to run over) is required"))?;
    let (id, v) = crate::vdb::volume_p(s, p, cmd)?;
    let frame = u64_p(p, "frame").unwrap_or(0) as usize;
    let input = if v.is_sequence() {
        match v.frames.get(frame) {
            Some(Some(f)) => f.clone(),
            Some(None) => return Err(EngineError::Other(format!("frame {frame} of {} is missing", v.path))),
            None => return Err(bad(cmd, format!("frame {frame} is outside the sequence of {} frames", v.frames.len()))),
        }
    } else if frame == 0 {
        v.path.clone()
    } else {
        return Err(bad(cmd, "a single file has only frame 0"));
    };
    let mut req = cfa4_vdb::ExprRequest::new(source, domain);
    if let Some(o) = p.get("outputs") {
        let m = o.as_object().ok_or_else(|| bad(cmd, "`outputs` is an object {name: \"float\"|\"vec3\"}"))?;
        for (name, t) in m {
            let ty = match t.as_str() {
                Some("float") => Ty::Float,
                Some("vec3") => Ty::Vec3,
                _ => return Err(bad(cmd, format!("output `{name}`: the type is `float` or `vec3`"))),
            };
            req.outputs.push((name.clone(), ty));
        }
    }
    if let Some(o) = p.get("params") {
        let m = o.as_object().ok_or_else(|| bad(cmd, "`params` is an object {name: value}"))?;
        for (name, val) in m {
            let cv = json_value(val).ok_or_else(|| bad(cmd, format!("parameter `{name}`: a number, a boolean or [x,y,z]")))?;
            req.params.push((name.clone(), cv));
        }
    }
    if let Some(f) = u64_p(p, "fuel") {
        req.fuel = u32::try_from(f).map_err(|_| bad(cmd, "`fuel` is too large"))?.min(cfa4_expr::MAX_FUEL);
    }
    if let Some(t) = u64_p(p, "threads") {
        req.threads = usize::try_from(t).unwrap_or(usize::MAX).min(256);
    }
    // the preview limits are the owner's (Q-X2): a caller may only lower them
    if let Some(t) = p.get("timeoutSeconds").and_then(Value::as_f64).filter(|t| *t > 0.0) {
        req.timeout = req.timeout.min(std::time::Duration::from_secs_f64(t));
    }
    let n = RUN.fetch_add(1, Ordering::Relaxed);
    let base = crate::temp_dir().join("CFA4 Expression Previews");
    let out = match str_p(p, "out") {
        Some(o) => std::path::PathBuf::from(o),
        None => match bake_tag {
            Some(tag) => baked_path(std::path::Path::new(&input), tag),
            None => base.join(format!("expr-{}-f{frame}-{}-{n}.vdb", id.0, std::process::id())),
        },
    };
    if bake_tag.is_some() && out.exists() && p.get("overwrite").and_then(Value::as_bool) != Some(true) {
        return Err(bad(cmd, format!("{} exists; pass `overwrite: true` or another `out`", out.display())));
    }
    if !out.to_string_lossy().to_ascii_lowercase().ends_with(".vdb") {
        return Err(bad(cmd, "`out` must end with .vdb"));
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
    }
    let work = base.join(format!("work-{}-{n}", std::process::id()));
    let worker = cfa4_vdb::Worker::discover().map_err(|e| EngineError::Other(e.to_string()))?;
    let rep = worker.apply_expression(std::path::Path::new(&input), &out, &work, &req, None).map_err(|e| EngineError::Other(e.to_string()))?;
    let out_s = out.to_string_lossy().to_string();
    let imported = s.execute("vdb.import", json!({"paths": [out_s], "sequence": false}))?;
    let new_item = imported["items"][0]["item"].clone();
    let mut result = json!({
        "source": id.0, "frame": frame, "vdb": out_s, "item": new_item,
        "report": {
            "voxels": rep.voxels, "leaves": rep.leaves, "fuelUsed": rep.fuel_used, "wroteNan": rep.wrote_nan,
            "threads": rep.threads, "written": rep.written, "warnings": rep.warnings, "elapsedMs": rep.elapsed_ms,
        },
    });
    if p.get("render").and_then(Value::as_bool) == Some(true) {
        let mut rp = json!({"item": new_item, "quality": str_p(p, "quality").unwrap_or("preview")});
        for k in ["width", "height", "azimuth", "elevation"] {
            if let Some(v) = p.get(k) {
                rp[k] = v.clone();
            }
        }
        if let Some(g) = req.outputs.iter().find(|(_, t)| *t == Ty::Float).map(|(n, _)| n.clone()).or_else(|| rep.written.first().cloned()) {
            rp["grid"] = json!(g);
        }
        result["render"] = s.execute("vdb.render", rp)?;
    }
    Ok(result)
}

// ---- X3: saved expressions, per-machine approval, bake -------------------------------------------------------

const APPROVALS_FILE: &str = "expression-approvals.json";
const APPROVALS_FORMAT: &str = "cfa4.expr-approvals";
const PRESET_FORMAT: &str = "cfa4.expr-presets";
/// The largest preset or approvals file read (a file from elsewhere is hostile).
const MAX_FILE_BYTES: u64 = 4 << 20;
const MAX_PRESETS: usize = 256;
const MAX_APPROVALS: usize = 100_000;

/// The expressions this machine's user has approved, by source hash. Kept outside the project (plan §5.1, N3-7′):
/// in `<data dir>/expression-approvals.json` once the data directory is known, in memory before.
#[derive(Debug, Default)]
pub struct Approvals {
    hashes: std::collections::BTreeSet<String>,
    dir: Option<std::path::PathBuf>,
}

fn is_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Approvals {
    /// Persist in `data_dir` and load what is there. A damaged or oversized file approves nothing.
    pub fn set_dir(&mut self, data_dir: &std::path::Path) {
        self.dir = Some(data_dir.to_path_buf());
        let path = data_dir.join(APPROVALS_FILE);
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
            log::warn!("{}: too large, ignored", path.display());
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else { return };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            log::warn!("{}: not valid JSON, ignored", path.display());
            return;
        };
        if v["format"] != APPROVALS_FORMAT {
            return;
        }
        if let Some(a) = v["hashes"].as_array() {
            self.hashes.extend(a.iter().filter_map(Value::as_str).filter(|h| is_hash(h)).take(MAX_APPROVALS).map(str::to_string));
        }
    }
    #[must_use]
    pub fn is_approved(&self, hash: &str) -> bool {
        self.hashes.contains(hash)
    }
    fn persist(&self) -> std::result::Result<(), String> {
        let Some(d) = &self.dir else { return Ok(()) };
        let _ = std::fs::create_dir_all(d);
        let doc = json!({"format": APPROVALS_FORMAT, "version": 1, "hashes": self.hashes.iter().collect::<Vec<_>>()});
        let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
        cfa4_format::atomic_write(&d.join(APPROVALS_FILE), &bytes).map_err(|e| e.to_string())
    }
    fn approve(&mut self, hash: &str) -> Result<()> {
        if self.hashes.len() >= MAX_APPROVALS && !self.hashes.contains(hash) {
            return Err(EngineError::Other("too many approved expressions".into()));
        }
        self.hashes.insert(hash.to_string());
        self.persist().map_err(|e| EngineError::Other(format!("approval is kept for this session only: {e}")))
    }
    fn revoke(&mut self, hash: &str) -> Result<()> {
        self.hashes.remove(hash);
        self.persist().map_err(|e| EngineError::Other(format!("could not write {APPROVALS_FILE}: {e}")))
    }
}

fn ty_of(name: &str) -> Option<Ty> {
    match name {
        "float" => Some(Ty::Float),
        "vec3" => Some(Ty::Vec3),
        _ => None,
    }
}

fn ty_name(t: Ty) -> &'static str {
    match t {
        Ty::Float => "float",
        Ty::Int => "int",
        Ty::Bool => "bool",
        Ty::Vec3 => "vec3",
    }
}

fn cv_json(v: Cv) -> Value {
    match v {
        Cv::Float(f) => json!(f),
        Cv::Int(i) => json!(i),
        Cv::Bool(b) => json!(b),
        Cv::Vec3(a) => json!(a),
    }
}

fn diag_json(d: &cfa4_expr::Diagnostic) -> Value {
    json!({"code": d.code, "line": d.line, "col": d.col, "len": d.len, "messageHu": d.message_hu, "messageEn": d.message_en})
}

/// `{name: "float"|"vec3"}` as pairs, in name order.
fn types_p(p: &Value, key: &str, cmd: &str) -> Result<Vec<(String, String)>> {
    let Some(o) = p.get(key).filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let m = o.as_object().ok_or_else(|| bad(cmd, format!("`{key}` is an object {{name: \"float\"|\"vec3\"}}")))?;
    let mut v = Vec::new();
    for (name, t) in m {
        match t.as_str() {
            Some(t @ ("float" | "vec3")) => v.push((name.clone(), t.to_string())),
            _ => return Err(bad(cmd, format!("`{key}.{name}`: the type is `float` or `vec3`"))),
        }
    }
    Ok(v)
}

fn params_p(p: &Value, cmd: &str) -> Result<std::collections::BTreeMap<String, Value>> {
    let Some(o) = p.get("params").filter(|v| !v.is_null()) else { return Ok(Default::default()) };
    let m = o.as_object().ok_or_else(|| bad(cmd, "`params` is an object {name: value}"))?;
    Ok(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn expr_err(cmd: &str, e: ExprError) -> EngineError {
    bad(cmd, e.to_string())
}

fn schema_of(attrs: &[(String, String)], context: Context) -> Schema {
    Schema { context, attrs: attrs.iter().filter_map(|(n, t)| ty_of(t).map(|t| (n.clone(), t))).collect() }
}

fn context_p(p: &Value, cmd: &str) -> Result<Context> {
    match str_p(p, "context") {
        None | Some("volume") => Ok(Context::Volume),
        Some("points") => Ok(Context::Points),
        Some(_) => Err(bad(cmd, "`context` is `volume` or `points`")),
    }
}

/// The attributes a check or a save-time check sees: `attrs` plus `outputs` (an output that is also an attribute keeps
/// the `attrs` type).
fn attrs_p(p: &Value, cmd: &str) -> Result<Vec<(String, String)>> {
    let mut attrs = types_p(p, "attrs", cmd)?;
    for (n, t) in types_p(p, "outputs", cmd)? {
        if attrs.iter().all(|(a, _)| *a != n) {
            attrs.push((n, t));
        }
    }
    Ok(attrs)
}

fn check(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "expr.check";
    let source = str_p(p, "source").ok_or_else(|| bad(CMD, "`source` is required"))?;
    let ctx = context_p(p, CMD)?;
    let attrs = attrs_p(p, CMD)?;
    let schema = schema_of(&attrs, ctx);
    Ok(match cfa4_expr::compile(source, &schema) {
        Ok(prog) => {
            let names = |ix: &[usize]| ix.iter().filter_map(|i| schema.attrs.get(*i).map(|(n, _)| n.clone())).collect::<Vec<_>>();
            json!({
                "ok": true, "hash": source_hash(source), "instructions": prog.len(),
                "reads": names(prog.reads()), "writes": names(prog.writes()),
                "params": prog.params().iter().map(|d| json!({"name": d.name, "type": ty_name(d.ty), "default": cv_json(d.default)})).collect::<Vec<_>>(),
                "warnings": prog.warnings().iter().map(diag_json).collect::<Vec<_>>(),
            })
        }
        Err(d) => json!({"ok": false, "hash": source_hash(source), "error": diag_json(&d)}),
    })
}

fn expression_json(e: &Expression, approved: bool, with_source: bool) -> Value {
    let mut v = json!({
        "id": e.id.0, "name": e.name, "hash": e.hash, "domain": e.domain,
        "outputs": e.outputs.iter().map(|(n, t)| (n.clone(), json!(t))).collect::<serde_json::Map<_, _>>(),
        "params": e.params, "fuel": e.fuel, "sourceBytes": e.source.len(), "approved": approved,
    });
    if with_source {
        v["source"] = json!(e.source);
    }
    v
}

fn expr_id_p(p: &Value, cmd: &str) -> Result<ExprId> {
    u64_p(p, "id").map(ExprId).ok_or_else(|| bad(cmd, "`id` (a saved expression) is required"))
}

fn saved(s: &Session, id: ExprId, cmd: &str) -> Result<Expression> {
    s.project.expressions.get(&id).cloned().ok_or_else(|| bad(cmd, format!("no saved expression {}", id.0)))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "expr.save";
    let source = str_p(p, "source").ok_or_else(|| bad(CMD, "`source` is required"))?.to_string();
    let existing = u64_p(p, "id").map(ExprId);
    if let Some(id) = existing
        && !s.project.expressions.contains_key(&id)
    {
        return Err(bad(CMD, format!("no saved expression {}", id.0)));
    }
    if existing.is_none() && s.project.expressions.len() >= cfa4_project::expression::MAX_EXPRESSIONS {
        return Err(bad(CMD, "too many saved expressions"));
    }
    let old = existing.and_then(|id| s.project.expressions.get(&id));
    let name = str_p(p, "name").map(str::to_string).or_else(|| old.map(|e| e.name.clone())).ok_or_else(|| bad(CMD, "`name` is required"))?;
    let domain = str_p(p, "domain")
        .map(str::to_string)
        .or_else(|| old.map(|e| e.domain.clone()))
        .ok_or_else(|| bad(CMD, "`domain` (the grid to run over) is required"))?;
    let outputs = if p.get("outputs").is_some() { types_p(p, "outputs", CMD)? } else { old.map(|e| e.outputs.clone()).unwrap_or_default() };
    let params = if p.get("params").is_some() { params_p(p, CMD)? } else { old.map(|e| e.params.clone()).unwrap_or_default() };
    let fuel = match p.get("fuel") {
        None => old.and_then(|e| e.fuel),
        Some(Value::Null) => None,
        Some(_) => Some(u64_p(p, "fuel").and_then(|f| u32::try_from(f).ok()).ok_or_else(|| bad(CMD, "`fuel` is a positive number"))?),
    };
    // with `attrs` the text is compiled before it is stored; a diagnostic keeps its code and place
    let mut checked = false;
    if p.get("attrs").is_some() {
        let schema = schema_of(&attrs_p(p, CMD)?, context_p(p, CMD)?);
        cfa4_expr::compile(&source, &schema).map_err(|d| EngineError::Other(d.to_string()))?;
        checked = true;
    }
    let mut project_ids = (*s.project).clone();
    let id = existing.unwrap_or_else(|| ExprId(project_ids.alloc_id()));
    let e = Expression::new(id, name, source, domain, outputs, params, fuel).map_err(|e| expr_err(CMD, e))?;
    // the user saving the text on this machine approves it; a failing write only costs the approval's persistence
    let approve_note = s.expr_approvals.approve(&e.hash).err().map(|e| e.to_string());
    let label = format!("Save Expression {}", e.name);
    let out = expression_json(&e, true, false);
    s.edit(&label, |pr, _| {
        pr.next_id = pr.next_id.max(project_ids.next_id);
        pr.expressions.insert(e.id, e.clone());
        Ok(())
    })?;
    let mut r = json!({"expression": out, "checked": checked});
    if let Some(n) = approve_note {
        r["warning"] = json!(n);
    }
    Ok(r)
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let v: Vec<Value> = s.project.expressions.values().map(|e| expression_json(e, s.expr_approvals.is_approved(&e.hash), false)).collect();
    Ok(json!({"expressions": v}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let e = saved(s, expr_id_p(p, "expr.get")?, "expr.get")?;
    Ok(json!({"expression": expression_json(&e, s.expr_approvals.is_approved(&e.hash), true)}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let id = expr_id_p(p, "expr.delete")?;
    let e = saved(s, id, "expr.delete")?;
    s.edit(&format!("Delete Expression {}", e.name), |pr, _| {
        pr.expressions.remove(&id);
        Ok(())
    })?;
    Ok(json!({"deleted": id.0}))
}

fn approve(s: &mut Session, p: &Value) -> Result<Value> {
    let e = saved(s, expr_id_p(p, "expr.approve")?, "expr.approve")?;
    s.expr_approvals.approve(&e.hash)?;
    Ok(json!({"id": e.id.0, "hash": e.hash, "approved": true}))
}

fn revoke(s: &mut Session, p: &Value) -> Result<Value> {
    let e = saved(s, expr_id_p(p, "expr.revoke")?, "expr.revoke")?;
    s.expr_approvals.revoke(&e.hash)?;
    Ok(json!({"id": e.id.0, "hash": e.hash, "approved": false}))
}

fn slug(name: &str) -> String {
    let t: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let t = t.trim_matches('-');
    let t = if t.is_empty() { "expr" } else { t };
    t.chars().take(24).collect()
}

fn bake(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "expr.bake";
    let id = expr_id_p(p, CMD)?;
    let e = saved(s, id, CMD)?;
    // the text must be the one this machine approved; nothing is compiled or started before this check
    if !s.expr_approvals.is_approved(&e.hash) {
        return Err(EngineError::Other(format!(
            "E-APPROVAL: the expression `{}` ({}) is not approved on this machine; read it with expr.get, then expr.approve",
            e.name,
            &e.hash[..12]
        )));
    }
    let mut q = json!({"source": e.source, "domain": e.domain});
    if !e.outputs.is_empty() {
        q["outputs"] = Value::Object(e.outputs.iter().map(|(n, t)| (n.clone(), json!(t))).collect());
    }
    let mut params: serde_json::Map<String, Value> = e.params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some(o) = p.get("params").filter(|v| !v.is_null()) {
        let m = o.as_object().ok_or_else(|| bad(CMD, "`params` is an object {name: value}"))?;
        params.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    q["params"] = Value::Object(params);
    match p.get("fuel").filter(|v| !v.is_null()) {
        Some(f) => q["fuel"] = f.clone(),
        None => {
            if let Some(f) = e.fuel {
                q["fuel"] = json!(f);
            }
        }
    }
    for k in ["item", "frame", "threads", "timeoutSeconds", "out", "overwrite", "render", "width", "height", "quality", "azimuth", "elevation"] {
        if let Some(v) = p.get(k) {
            q[k] = v.clone();
        }
    }
    let tag = format!("{}-{}", slug(&e.name), &e.hash[..8]);
    let mut r = run(s, &q, CMD, Some(&tag))?;
    r["expression"] = json!({"id": e.id.0, "name": e.name, "hash": e.hash});
    Ok(r)
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "expr.export";
    let path = str_p(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let ids: Vec<ExprId> = match p.get("ids").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).map(ExprId).collect(),
        None => s.project.expressions.keys().copied().collect(),
    };
    let mut items = Vec::new();
    for id in &ids {
        let e = saved(s, *id, CMD)?;
        items.push(json!({
            "name": e.name, "source": e.source, "domain": e.domain,
            "outputs": e.outputs.iter().map(|(n, t)| (n.clone(), json!(t))).collect::<serde_json::Map<_, _>>(),
            "params": e.params, "fuel": e.fuel,
        }));
    }
    let doc = json!({"format": PRESET_FORMAT, "version": 1, "expressions": items});
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| EngineError::Other(e.to_string()))?;
    cfa4_format::atomic_write(std::path::Path::new(path), &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "count": ids.len()}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "expr.import";
    let path = str_p(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let len = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.len();
    if len > MAX_FILE_BYTES {
        return Err(bad(CMD, format!("{path}: larger than {MAX_FILE_BYTES} bytes")));
    }
    let bytes = std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| bad(CMD, format!("{path}: not valid JSON: {e}")))?;
    if doc["format"] != PRESET_FORMAT || doc["version"] != 1 {
        return Err(bad(CMD, format!("{path}: not a `{PRESET_FORMAT}` version 1 file")));
    }
    let list = doc["expressions"].as_array().ok_or_else(|| bad(CMD, "`expressions` is missing"))?;
    if list.len() > MAX_PRESETS || s.project.expressions.len() + list.len() > cfa4_project::expression::MAX_EXPRESSIONS {
        return Err(bad(CMD, "too many expressions"));
    }
    let mut pr = (*s.project).clone();
    let mut made = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let name = str_p(item, "name").ok_or_else(|| bad(CMD, format!("expression {i}: `name` is required")))?;
        let source = str_p(item, "source").ok_or_else(|| bad(CMD, format!("expression {i}: `source` is required")))?;
        let domain = str_p(item, "domain").ok_or_else(|| bad(CMD, format!("expression {i}: `domain` is required")))?;
        let outputs = types_p(item, "outputs", CMD)?;
        let params = params_p(item, CMD)?;
        let fuel = match item.get("fuel") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| bad(CMD, format!("expression {i}: `fuel` is a positive number")))?),
        };
        let e = Expression::new(ExprId(pr.alloc_id()), name, source, domain, outputs, params, fuel).map_err(|e| expr_err(CMD, e))?;
        made.push(e);
    }
    let count = made.len();
    let shown: Vec<Value> = made.iter().map(|e| expression_json(e, s.expr_approvals.is_approved(&e.hash), false)).collect();
    let next = pr.next_id;
    s.edit("Import Expressions", |p2, _| {
        p2.next_id = p2.next_id.max(next);
        for e in made {
            p2.expressions.insert(e.id, e);
        }
        Ok(())
    })?;
    // nothing is approved by importing: a hash this machine has approved before shows as approved, any other does not
    Ok(json!({"count": count, "expressions": shown}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "expr.check",
            label: "Check Expression",
            menu: &[],
            shortcut: None,
            params: r#"{"source":str,"context":"volume|points"?,"attrs":{name:"float|vec3"}?,"outputs":{name:"float|vec3"}?}"#,
            enabled: always,
            run: check,
            journal: false,
        },
        CommandSpec {
            id: "expr.save",
            label: "Save Expression",
            menu: &[],
            shortcut: None,
            params: r#"{"id":n?,"name":str,"source":str,"domain":str,"outputs":{name:"float|vec3"}?,"params":{name:value}?,"fuel":n?,"attrs":{name:"float|vec3"}?}"#,
            enabled: always,
            run: save,
            journal: true,
        },
        CommandSpec { id: "expr.list", label: "List Expressions", menu: &[], shortcut: None, params: "{}", enabled: always, run: list, journal: false },
        CommandSpec { id: "expr.get", label: "Get Expression", menu: &[], shortcut: None, params: r#"{"id":n}"#, enabled: always, run: get, journal: false },
        CommandSpec {
            id: "expr.delete",
            label: "Delete Expression",
            menu: &[],
            shortcut: None,
            params: r#"{"id":n}"#,
            enabled: always,
            run: delete,
            journal: true,
        },
        CommandSpec {
            id: "expr.approve",
            label: "Approve Expression",
            menu: &[],
            shortcut: None,
            params: r#"{"id":n}"#,
            enabled: always,
            run: approve,
            journal: true,
        },
        CommandSpec {
            id: "expr.revoke",
            label: "Revoke Expression Approval",
            menu: &[],
            shortcut: None,
            params: r#"{"id":n}"#,
            enabled: always,
            run: revoke,
            journal: true,
        },
        CommandSpec {
            id: "expr.bake",
            label: "Bake Expression",
            menu: &[],
            shortcut: None,
            params: r#"{"id":n,"item":id?,"frame":n?,"params":{name:value}?,"fuel":n?,"threads":n?,"timeoutSeconds":n?,"out":path?,"overwrite":bool?,"render":bool?,"width":n?,"height":n?,"quality":"preview|standard|high"?}"#,
            enabled: always,
            run: bake,
            journal: true,
        },
        CommandSpec {
            id: "expr.export",
            label: "Export Expression Presets",
            menu: &[],
            shortcut: None,
            params: r#"{"path":path,"ids":[n]?}"#,
            enabled: always,
            run: export,
            journal: false,
        },
        CommandSpec {
            id: "expr.import",
            label: "Import Expression Presets",
            menu: &[],
            shortcut: None,
            params: r#"{"path":path}"#,
            enabled: always,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: "expr.run_preview",
            label: "Run Expression Preview",
            menu: &[],
            shortcut: None,
            params: r#"{"item":id?,"frame":n?,"source":str,"domain":str,"outputs":{name:"float|vec3"}?,"params":{name:value}?,"fuel":n?,"threads":n?,"timeoutSeconds":n?,"out":path?,"render":bool?,"width":n?,"height":n?,"quality":"preview|standard|high"?}"#,
            enabled: always,
            run: run_preview,
            journal: true,
        },
    ]
}
