//! DCP repair safety (DCP-24, plan DCP-F21): `dcp.snapshot` makes a byte-identical copy of a package
//! outside it with a hash manifest and starts the repair log, `dcp.manifest.verify` proves that a package is
//! (still) the one a manifest names, `dcp.repair.job` reads the result of either job. The package is only
//! read; a copy inside the package or over an existing folder is refused (`cfa4_dcp_validate::snapshot`).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use cfa4_dcp_validate::snapshot::{self, LogEntry, Manifest};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

/// Results kept in the session (the oldest is dropped first).
const MAX_RESULTS: usize = 16;

type Outcome = Arc<Mutex<Option<std::result::Result<Value, String>>>>;

/// Session state: the outcome of each job started here.
#[derive(Default)]
pub struct DcpRepairSession {
    results: Vec<(u64, Outcome)>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts `work` as a job; `work` gets the job's progress and returns the answer. With `wait` the job runs
/// now and the answer comes back at once.
fn start(
    s: &mut Session,
    cmd: &str,
    label: String,
    wait: bool,
    work: impl FnOnce(&Arc<cfa4_export::Progress>) -> std::result::Result<Value, String> + Send + 'static,
) -> Result<Value> {
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label, progress: Default::default(), result: Default::default() };
    job.progress.total.store(1, Ordering::Relaxed);
    let outcome: Outcome = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), outcome.clone());
    let run = move || {
        let t0 = web_time::Instant::now();
        let r = work(&prog);
        if let Err(e) = &r {
            *lock(&prog.error) = Some(e.clone());
        }
        prog.done.store(prog.total.load(Ordering::Relaxed), Ordering::Relaxed);
        *lock(&res) = Some(match &r {
            Ok(_) => Ok(cfa4_export::Report {
                path: String::new(),
                frames: 0,
                seconds: t0.elapsed().as_secs_f64(),
                bytes: 0,
                render_fps: 0.0,
                extra_files: Vec::new(),
            }),
            Err(e) => Err(e.clone()),
        });
        *lock(&out) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    let list = &mut s.dcp_repair.results;
    list.push((id, outcome.clone()));
    let drop = list.len().saturating_sub(MAX_RESULTS);
    list.drain(..drop);
    if wait || cfg!(target_arch = "wasm32") {
        run();
        return answer(&outcome, cmd, id);
    }
    std::thread::Builder::new().name("cfa4-dcp-repair".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(json!({"job": id}))
}

fn answer(outcome: &Outcome, cmd: &str, job: u64) -> Result<Value> {
    match lock(outcome).clone() {
        Some(Ok(v)) => {
            let mut v = v;
            v["job"] = json!(job);
            Ok(v)
        }
        Some(Err(e)) => Err(bad(cmd, e)),
        None => Ok(json!({"job": job, "finished": false})),
    }
}

/// `dcp.snapshot {root, dest, wait}`: copy the package at `root` to the new folder `dest` (outside it),
/// with `manifest.json` and a repair log that already holds the `snapshot` entry.
fn snapshot_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.snapshot";
    let root = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root` (the DCP folder)"))?.to_string();
    let dest = str_p(p, "dest").ok_or_else(|| bad(cmd, "need `dest` (a new folder outside the package)"))?.to_string();
    let wait = bool_p(p, "wait").unwrap_or(false);
    let before = crate::dcp_validation::latest_report_json(s, &root).map(|j| snapshot::report_id(&j));
    let name = Path::new(&root).file_name().map_or_else(|| root.clone(), |n| n.to_string_lossy().into_owned());
    let (r, d) = (root.clone(), dest.clone());
    start(s, cmd, format!("DCP snapshot {name}"), wait, move |prog| {
        let report = prog.clone();
        let progress = move |done: u64, total: u64, file: &str| {
            report.total.store(total.max(1), Ordering::Relaxed);
            report.done.store(done.min(total.max(1)), Ordering::Relaxed);
            *lock(&report.status) = if file.is_empty() { "DCP snapshot done".into() } else { format!("Copying {file}") };
        };
        let manifest = snapshot::snapshot(Path::new(&r), Path::new(&d), &prog.cancel, &progress).map_err(|e| e.to_string())?;
        let mut entry = LogEntry::new("snapshot", &r, &manifest);
        entry.report_before = before;
        let log = PathBuf::from(&d).join(snapshot::LOG_FILE);
        snapshot::append_log(&log, &entry).map_err(|e| e.to_string())?;
        Ok(json!({
            "dest": d,
            "manifest": PathBuf::from(&d).join(snapshot::MANIFEST_FILE).to_string_lossy(),
            "log": log.to_string_lossy(),
            "manifestId": manifest.id(),
            "files": manifest.files.len(),
            "bytes": manifest.total_bytes(),
            "skipped": manifest.skipped,
        }))
    })
}

/// The manifest at `path`: a `manifest.json` file, or the snapshot folder that holds one.
fn read_manifest(path: &str) -> std::result::Result<Manifest, String> {
    let p = Path::new(path);
    let file = if p.is_dir() { p.join(snapshot::MANIFEST_FILE) } else { p.to_path_buf() };
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    Manifest::from_json(&text).map_err(|e| e.to_string())
}

/// `dcp.manifest.verify {root, manifest, wait}`: is the package the one the manifest names?
fn verify_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.manifest.verify";
    let root = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root` (the DCP folder)"))?.to_string();
    let path = str_p(p, "manifest").ok_or_else(|| bad(cmd, "need `manifest` (manifest.json or the snapshot folder)"))?.to_string();
    let wait = bool_p(p, "wait").unwrap_or(false);
    let manifest = read_manifest(&path).map_err(|e| bad(cmd, e))?;
    let name = Path::new(&root).file_name().map_or_else(|| root.clone(), |n| n.to_string_lossy().into_owned());
    start(s, cmd, format!("DCP verify {name}"), wait, move |prog| {
        prog.status.lock().map(|mut g| *g = "Comparing with the manifest".into()).ok();
        let v = snapshot::verify(Path::new(&root), &manifest, &prog.cancel).map_err(|e| e.to_string())?;
        Ok(json!({"root": root, "manifestId": manifest.id(), "verification": v}))
    })
}

/// `dcp.repair.job {job}`: the answer of a snapshot or verify job (`finished: false` while it runs).
fn job_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.repair.job";
    let job = p.get("job").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "need `job`"))?;
    let outcome = s
        .dcp_repair
        .results
        .iter()
        .find(|(id, _)| *id == job)
        .map(|(_, o)| o.clone())
        .ok_or_else(|| bad(cmd, format!("no snapshot or verify job {job} in this session")))?;
    answer(&outcome, cmd, job)
}

/// `dcp.repair.log {path}`: the entries of a repair log (a `repair-log.jsonl` file or the snapshot folder).
fn log_cmd(_: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.repair.log";
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path` (repair-log.jsonl or the snapshot folder)"))?;
    let pb = Path::new(path);
    let file = if pb.is_dir() { pb.join(snapshot::LOG_FILE) } else { pb.to_path_buf() };
    let entries = snapshot::read_log(&file).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(json!({"schema": snapshot::LOG_SCHEMA, "path": file.to_string_lossy(), "entries": entries}))
}

/// Name of the next re-export of the package called `name`: `<base>_r2`, `_r3`, … where `base` is `name`
/// without a `_rN` suffix. Never a name that exists in `dir` or is already queued.
fn next_name(s: &Session, dir: &Path, name: &str) -> String {
    let base = match name.rsplit_once("_r") {
        Some((b, n)) if !b.is_empty() && !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => b,
        _ => name,
    };
    (2u32..)
        .map(|n| format!("{base}_r{n}"))
        .find(|c| {
            let p = dir.join(c);
            std::fs::symlink_metadata(&p).is_err() && !s.export_queue.items.iter().any(|i| Path::new(&i.settings.path) == p)
        })
        .unwrap_or_default()
}

/// `dcp.repair.reexport {root, path?, dest?, …export params}`: the package at `root` has bad content, so
/// export the current project again as a new package `<name>_rN` (in the folder `path`, default: next to
/// `root`). First an exact copy of `root` is made (`dest`, default `<root>.cfa4repair-<UTC time>`), as
/// `dcp.snapshot` does; the package and its report are never changed. The repair log in the copy gets a
/// `reexport` entry that names the new package. Other parameters (`sequence`, `preset`, `settings`, `range`,
/// `dcpValidate`, `start`, `wait`) go to `export.queue.add`; the format is always `dcp`.
fn reexport_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.repair.reexport";
    let root = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root` (the DCP folder)"))?.to_string();
    let root_path = Path::new(root.trim_end_matches(['/', '\\']));
    let name = root_path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| bad(cmd, "`root` has no folder name"))?;
    if s.state.active_sequence.is_none() && p.get("sequence").is_none() {
        return Err(bad(cmd, "no project is open: open the project the package was exported from, then re-export"));
    }
    if !root_path.is_dir() {
        return Err(bad(cmd, format!("{root}: not a folder")));
    }
    let parent = root_path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let out_dir = str_p(p, "path").map_or_else(|| parent.to_path_buf(), PathBuf::from);
    let new_path = out_dir.join(next_name(s, &out_dir, &name));
    let stamp = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()).format("%Y%m%dT%H%M%SZ");
    let dest = str_p(p, "dest").map_or_else(|| parent.join(format!("{name}.cfa4repair-{stamp}")), PathBuf::from);

    // 1. the exact copy, with its manifest and log (the package is only read)
    let before = crate::dcp_validation::latest_report_json(s, &root).map(|j| snapshot::report_id(&j));
    let never = std::sync::atomic::AtomicBool::new(false);
    let manifest = snapshot::snapshot(root_path, &dest, &never, &|_, _, _| {}).map_err(|e| bad(cmd, e.to_string()))?;
    let log = dest.join(snapshot::LOG_FILE);
    let mut first = LogEntry::new("snapshot", &root, &manifest);
    first.report_before = before.clone();
    snapshot::append_log(&log, &first).map_err(|e| bad(cmd, e.to_string()))?;

    // 2. the new export of the current project, always a DCP into a new place
    let mut q = p.clone();
    let Some(obj) = q.as_object_mut() else { return Err(bad(cmd, "parameters must be an object")) };
    for k in ["root", "dest"] {
        obj.remove(k);
    }
    obj.insert("format".into(), json!("dcp"));
    obj.insert("path".into(), json!(new_path.to_string_lossy()));
    obj.entry("dcpValidate").or_insert(json!(true));
    let mut entry = LogEntry::new("reexport", &root, &manifest);
    entry.report_before = before;
    match crate::export_tools::queue_add(s, &q) {
        Ok(added) => {
            entry.result = Some(new_path.to_string_lossy().into_owned());
            snapshot::append_log(&log, &entry).map_err(|e| bad(cmd, e.to_string()))?;
            Ok(json!({
                "snapshot": dest.to_string_lossy(),
                "manifest": dest.join(snapshot::MANIFEST_FILE).to_string_lossy(),
                "log": log.to_string_lossy(),
                "manifestId": manifest.id(),
                "newPackage": new_path.to_string_lossy(),
                "newReport": format!("{}{}", new_path.to_string_lossy(), crate::dcp_export::REPORT_SUFFIX),
                "queued": added,
            }))
        }
        Err(e) => {
            // the copy stays; the log says that nothing new was made
            entry.result = Some(format!("not queued: {e}"));
            let _ = snapshot::append_log(&log, &entry);
            Err(bad(cmd, format!("{e} (the exact copy of the package is kept in {})", dest.display())))
        }
    }
}

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("dcp.snapshot", "Snapshot DCP Package", r#"{"root":str,"dest":str,"wait":bool=false}"#, snapshot_cmd),
        spec("dcp.manifest.verify", "Verify DCP Package Against Manifest", r#"{"root":str,"manifest":str,"wait":bool=false}"#, verify_cmd),
        spec(
            "dcp.repair.reexport",
            "Re-export DCP Under a New Name",
            r#"{"root":str,"path":str?,"dest":str?,"sequence":id?,"preset":str?,"settings":object?,"range":range?,"dcpValidate":str|bool?,"start":bool?,"wait":bool?}"#,
            reexport_cmd,
        ),
        spec("dcp.repair.job", "Get DCP Repair Job Result", r#"{"job":int}"#, job_cmd),
        spec("dcp.repair.log", "Read DCP Repair Log", r#"{"path":str}"#, log_cmd),
    ]
}

#[cfg(test)]
mod tests;
