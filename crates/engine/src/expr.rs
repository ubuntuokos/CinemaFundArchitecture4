//! CFX expression commands (`expr.*`), plan CFA4-AX-INTERPRETER-001, X2.
//!
//! | command | does |
//! |---|---|
//! | `expr.run_preview` | runs a CFX expression over the active voxels of one grid of a volume item (limits: 30 s, 1 GiB of active voxel data, fuel per voxel), writes a new `.vdb` with the original grids plus the written ones, and imports it as a new volume item; with `render: true` it also renders a density preview of the first written float grid through the N2 path (`vdb.render`) |
//!
//! The expression language is `docs/expr-language.md`. A compile error, a run-time error (`R-FUEL`, `R-INT`, ...),
//! a limit, a missing worker or a cancelled run is an ordinary error message that keeps the stable code and the
//! place; nothing is imported then, and the project is unchanged.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use cfa4_expr::{Ty, Value as Cv};

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

fn run_preview(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "expr.run_preview";
    let source = str_p(p, "source").ok_or_else(|| bad(CMD, "`source` is required"))?;
    let domain = str_p(p, "domain").ok_or_else(|| bad(CMD, "`domain` (the grid to run over) is required"))?;
    let (id, v) = crate::vdb::volume_p(s, p, CMD)?;
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
    let mut req = cfa4_vdb::ExprRequest::new(source, domain);
    if let Some(o) = p.get("outputs") {
        let m = o.as_object().ok_or_else(|| bad(CMD, "`outputs` is an object {name: \"float\"|\"vec3\"}"))?;
        for (name, t) in m {
            let ty = match t.as_str() {
                Some("float") => Ty::Float,
                Some("vec3") => Ty::Vec3,
                _ => return Err(bad(CMD, format!("output `{name}`: the type is `float` or `vec3`"))),
            };
            req.outputs.push((name.clone(), ty));
        }
    }
    if let Some(o) = p.get("params") {
        let m = o.as_object().ok_or_else(|| bad(CMD, "`params` is an object {name: value}"))?;
        for (name, val) in m {
            let cv = json_value(val).ok_or_else(|| bad(CMD, format!("parameter `{name}`: a number, a boolean or [x,y,z]")))?;
            req.params.push((name.clone(), cv));
        }
    }
    if let Some(f) = u64_p(p, "fuel") {
        req.fuel = u32::try_from(f).map_err(|_| bad(CMD, "`fuel` is too large"))?.min(cfa4_expr::MAX_FUEL);
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
        None => base.join(format!("expr-{}-f{frame}-{}-{n}.vdb", id.0, std::process::id())),
    };
    if !out.to_string_lossy().to_ascii_lowercase().ends_with(".vdb") {
        return Err(bad(CMD, "`out` must end with .vdb"));
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

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "expr.run_preview",
        label: "Run Expression Preview",
        menu: &[],
        shortcut: None,
        params: r#"{"item":id?,"frame":n?,"source":str,"domain":str,"outputs":{name:"float|vec3"}?,"params":{name:value}?,"fuel":n?,"threads":n?,"timeoutSeconds":n?,"out":path?,"render":bool?,"width":n?,"height":n?,"quality":"preview|standard|high"?}"#,
        enabled: always,
        run: run_preview,
        journal: true,
    }]
}
