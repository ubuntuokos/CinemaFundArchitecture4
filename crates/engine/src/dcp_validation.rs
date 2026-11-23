//! DCP validation commands (`dcp.validate*`, DCP-12): check a DCP package folder with one of the
//! validation profiles of `cfa4-dcp-validate` in a background job, keep the latest report of each
//! package in the session, and save it as JSON or Markdown where the user says.
//!
//! The package is only read (plan §9): the report never lands inside the DCP folder, it stays in
//! the session until `dcp.validate.save` writes it to a path of the caller's choosing. The
//! validation is the native backend; the Qt6 panel and the CLI drive it through these commands.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, ValidationControl, ValidationReport, ValidationRequest, profile};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

/// Most reports kept in the session (the oldest is dropped first).
const MAX_REPORTS: usize = 16;

type Outcome = Arc<Mutex<Option<std::result::Result<ValidationReport, String>>>>;

/// A validation whose result is still to be filed.
pub struct PendingValidation {
    pub job: u64,
    pub root: String,
    /// The queued DCP export this check follows (DCP-13 postflight), if any.
    pub queue_item: Option<u64>,
    outcome: Outcome,
}

/// Session state of the DCP validation commands.
#[derive(Default)]
pub struct DcpValidationSession {
    pub pending: Vec<PendingValidation>,
    /// Latest report of each package root.
    reports: Vec<(String, ValidationReport)>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A built-in profile id, or a profile file (`.json`, `cfa4-dcp-profile/1`) read from disk.
pub(crate) fn load_profile(cmd: &str, arg: Option<&str>) -> Result<Profile> {
    let arg = arg.unwrap_or("quick-scan");
    if arg.ends_with(".json") {
        return Profile::load(std::path::Path::new(arg)).map_err(|e| bad(cmd, format!("{arg}: {e}")));
    }
    Profile::builtin(arg).ok_or_else(|| bad(cmd, format!("no profile `{arg}` (built-in: {}; or a .json profile file)", profile::BUILTIN.join(", "))))
}

fn profile_json(p: &Profile) -> Value {
    let required = p.rules.iter().filter(|r| r.required).count();
    json!({
        "id": p.id,
        "version": p.version,
        "title": p.title,
        "standard": p.standard,
        "rules": p.rules.iter().map(|r| json!({"rule": r.rule, "required": r.required})).collect::<Vec<_>>(),
        "required": required,
        "optional": p.rules.len() - required,
    })
}

/// `dcp.validate.profiles`: the built-in profiles with their rules.
fn profiles(_: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = profile::BUILTIN.iter().filter_map(|id| Profile::builtin(id)).map(|p| profile_json(&p)).collect();
    Ok(json!({"profiles": list}))
}

fn word(v: cfa4_dcp_validate::Verdict) -> &'static str {
    use cfa4_dcp_validate::Verdict;
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Incomplete => "INCOMPLETE",
        Verdict::Interrupted => "INTERRUPTED",
    }
}

fn is_cancelled(s: &Session, job: u64) -> bool {
    s.jobs.iter().find(|j| j.id == job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed))
}

/// `dcp.validate`: check the package in `root` with `profile`; returns the job id (`wait`: the
/// report too).
fn validate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.validate";
    let root = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root` (the DCP folder)"))?.to_string();
    let wait = bool_p(p, "wait").unwrap_or(false);
    let profile = load_profile(cmd, str_p(p, "profile"))?;
    start(s, cmd, root, profile, wait, None)
}

/// Start the validation job of `root`; `queue_item` is the queued export it follows.
pub(crate) fn start(s: &mut Session, cmd: &str, root: String, profile: Profile, wait: bool, queue_item: Option<u64>) -> Result<Value> {
    if !std::path::Path::new(&root).is_dir() {
        return Err(bad(cmd, format!("{root}: not a folder")));
    }
    if let Some(other) = s.dcp_validation.pending.iter().find(|v| v.root == root && !is_cancelled(s, v.job)) {
        return Err(bad(cmd, format!("{root} is already being validated (job {}); wait for it or cancel it with jobs.cancel", other.job)));
    }
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let name = std::path::Path::new(&root).file_name().map_or_else(|| root.clone(), |n| n.to_string_lossy().into_owned());
    let job = crate::Job { id, label: format!("DCP validation {name}"), progress: Default::default(), result: Default::default() };
    job.progress.total.store(1, Ordering::Relaxed);
    *lock(&job.progress.status) = format!("Validating {name} ({})", profile.id);
    let outcome: Outcome = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), outcome.clone());
    let request = ValidationRequest::new(root.clone(), profile);
    let run = move || {
        let t0 = web_time::Instant::now();
        let control = ValidationControl::new();
        let flag = control.cancel_flag();
        let reporter = prog.clone();
        let control = control.with_progress(move |pr| {
            if reporter.cancel.load(Ordering::Relaxed) {
                flag.store(true, Ordering::Relaxed);
            }
            reporter.total.store(pr.total.max(1), Ordering::Relaxed);
            reporter.done.store(pr.done.min(pr.total.max(1)), Ordering::Relaxed);
            *lock(&reporter.status) = format!("{} {}/{}", pr.stage, pr.done, pr.total);
        });
        if prog.cancel.load(Ordering::Relaxed) {
            control.cancel();
        }
        let r = NativeValidator.validate(&request, &control).map_err(|e| e.to_string());
        *lock(&prog.status) = match &r {
            Ok(rep) => format!("DCP validation {}", word(rep.verdict)),
            Err(e) => e.clone(),
        };
        if let Err(e) = &r {
            *lock(&prog.error) = Some(e.clone());
        }
        prog.done.store(prog.total.load(Ordering::Relaxed), Ordering::Relaxed);
        let report = |rep: &ValidationReport| cfa4_export::Report {
            path: rep.root.clone(),
            frames: 0,
            seconds: t0.elapsed().as_secs_f64(),
            bytes: 0,
            render_fps: 0.0,
            extra_files: Vec::new(),
        };
        *lock(&res) = Some(r.as_ref().map(report).map_err(|e| e.clone()));
        *lock(&out) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.dcp_validation.pending.push(PendingValidation { job: id, root: root.clone(), queue_item, outcome });
    if let Some(q) = queue_item {
        crate::dcp_export::running(s, q);
    }
    if wait || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
    } else {
        std::thread::Builder::new().name("cfa4-dcp-validate".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    let mut out = json!({"job": id, "root": root});
    if wait && let Some((_, rep)) = s.dcp_validation.reports.iter().find(|(r, _)| *r == root) {
        out["report"] = serde_json::to_value(rep).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    Ok(out)
}

/// File the results of finished validations.
pub fn poll(s: &mut Session) {
    let mut i = 0;
    while i < s.dcp_validation.pending.len() {
        let finished = s.jobs.iter().find(|j| j.id == s.dcp_validation.pending[i].job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        if !finished {
            i += 1;
            continue;
        }
        let pv = s.dcp_validation.pending.remove(i);
        let cancelled = is_cancelled(s, pv.job);
        match lock(&pv.outcome).take() {
            Some(Ok(report)) => {
                if let Some(q) = pv.queue_item {
                    crate::dcp_export::settled(s, q, Ok(&report));
                }
                let name = std::path::Path::new(&pv.root).file_name().map_or_else(|| pv.root.clone(), |n| n.to_string_lossy().into_owned());
                let message = format!("DCP {name}: {}", word(report.verdict));
                let failed = report.verdict == cfa4_dcp_validate::Verdict::Fail;
                let reports = &mut s.dcp_validation.reports;
                reports.retain(|(r, _)| *r != pv.root);
                reports.push((pv.root.clone(), report));
                let drop = reports.len().saturating_sub(MAX_REPORTS);
                reports.drain(..drop);
                s.events.push(crate::Event::Toast { message, error: failed });
            }
            Some(Err(e)) => {
                if let Some(q) = pv.queue_item {
                    crate::dcp_export::settled(s, q, Err(if cancelled { "cancelled" } else { &e }));
                }
                if !cancelled {
                    s.error_toast("dcp.validate", format!("DCP validation: {e}"));
                }
            }
            None => {
                if let Some(q) = pv.queue_item {
                    crate::dcp_export::settled(s, q, Err(if cancelled { "cancelled" } else { "the validation job stopped" }));
                }
            }
        }
    }
}

fn find<'a>(s: &'a Session, cmd: &str, root: &str) -> Result<&'a ValidationReport> {
    s.dcp_validation
        .reports
        .iter()
        .find(|(r, _)| r == root)
        .map(|(_, rep)| rep)
        .ok_or_else(|| bad(cmd, format!("no report for {root}; run dcp.validate first")))
}

/// `dcp.validate.report`: the latest report of a package (JSON, plus its Markdown rendering).
fn report_get(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.validate.report";
    let root = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root`"))?;
    let rep = find(s, cmd, root)?;
    Ok(json!({"root": root, "verdict": rep.verdict, "report": rep, "markdown": rep.to_markdown()}))
}

/// `dcp.validate.save`: write the latest report of a package to `path` (`format` `json`, `md` or `html`).
/// Refuses a path inside the package: the DCP is only read.
fn report_save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.validate.save";
    let root = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root`"))?;
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path`"))?;
    let rep = find(s, cmd, root)?;
    let text = match str_p(p, "format").unwrap_or(if path.ends_with(".md") {
        "md"
    } else if path.ends_with(".html") || path.ends_with(".htm") {
        "html"
    } else {
        "json"
    }) {
        "json" => rep.to_json(),
        "md" | "markdown" => rep.to_markdown(),
        "html" => rep.to_html(),
        other => return Err(bad(cmd, format!("`format` is `json`, `md` or `html`, not `{other}`"))),
    };
    let (target, package) = (std::path::absolute(path), std::path::absolute(root));
    if let (Ok(t), Ok(r)) = (&target, &package)
        && t.starts_with(r)
    {
        return Err(bad(cmd, format!("{path} is inside the package {root}; the DCP is only read, save the report elsewhere")));
    }
    s.services.write_file(path, text.as_bytes()).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    Ok(json!({"path": path, "bytes": text.len()}))
}

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("dcp.validate", "Validate DCP Package", r#"{"root":str,"profile":str?,"wait":bool=false}"#, validate),
        spec("dcp.validate.profiles", "List DCP Validation Profiles", "{}", profiles),
        spec("dcp.validate.report", "Get DCP Validation Report", r#"{"root":str}"#, report_get),
        spec("dcp.validate.save", "Save DCP Validation Report", r#"{"root":str,"path":str,"format":str?}"#, report_save),
    ]
}

#[cfg(test)]
mod tests;
