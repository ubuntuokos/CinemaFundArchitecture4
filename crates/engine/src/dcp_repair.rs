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

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("dcp.snapshot", "Snapshot DCP Package", r#"{"root":str,"dest":str,"wait":bool=false}"#, snapshot_cmd),
        spec("dcp.manifest.verify", "Verify DCP Package Against Manifest", r#"{"root":str,"manifest":str,"wait":bool=false}"#, verify_cmd),
        spec("dcp.repair.job", "Get DCP Repair Job Result", r#"{"job":int}"#, job_cmd),
        spec("dcp.repair.log", "Read DCP Repair Log", r#"{"path":str}"#, log_cmd),
    ]
}

#[cfg(test)]
mod tests;
