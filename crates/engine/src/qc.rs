//! Media QC commands (`qc.*`, docs/media-qc.md): check an exported file with a QC profile in a
//! background job, keep the report and its contact sheet next to the export, and save the human
//! review into the report.
//!
//! Storage (plan decision 3): `<export>.cfa4qc.json` and `<export>.cfa4qc.png` next to the
//! export, plus `<project>.cfa4qc/index.json` next to a saved project. QC never writes the project
//! itself. A queued export keeps its QC state apart from its render state
//! ([`crate::export_tools::QueueItem::qc`]).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use cfa4_qc::report::{ComputeMode, Links, RenderOutcome, RenderStatus, RunInfo};
use cfa4_qc::{Expectation, HumanReview, Options, QcProfile, QcReport, Verdict};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p, u64_p};
use crate::export_tools::QueueStatus;
use crate::{EngineError, Result, Session};

pub const REPORT_SUFFIX: &str = ".cfa4qc.json";
pub const SHEET_SUFFIX: &str = ".cfa4qc.png";
/// Largest report read back (a report with every event kept is far below this).
const MAX_REPORT_BYTES: u64 = 32 * 1024 * 1024;
/// Most entries kept in a project's QC index (oldest dropped first).
const MAX_INDEX: usize = 10_000;

/// The QC state of a queued export, kept apart from its render state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QcStatus {
    #[default]
    NotChecked,
    Running,
    Passed,
    Warned,
    Failed,
    Incomplete,
}

impl From<Verdict> for QcStatus {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Pass => QcStatus::Passed,
            Verdict::Warn => QcStatus::Warned,
            Verdict::Fail => QcStatus::Failed,
            Verdict::Incomplete => QcStatus::Incomplete,
        }
    }
}

/// QC of a queued export: what was asked and where it stands.
#[derive(Debug, Clone, Default)]
pub struct ItemQc {
    pub status: QcStatus,
    /// Profile to check with when the export finishes (`None`: no automatic QC).
    pub profile: Option<String>,
    pub report: Option<String>,
    pub error: Option<String>,
}

/// What a finished QC job produced.
#[derive(Debug, Clone)]
struct Outcome {
    verdict: Verdict,
    report_path: String,
    sheet_path: Option<String>,
    profile: String,
    profile_version: u32,
    finished_unix_ms: u64,
}

/// A QC job whose result is still to be applied (queue state, project index).
pub struct PendingQc {
    pub job: u64,
    pub media: String,
    pub queue_item: Option<u64>,
    /// The QC index of the project the check was started in: opening another project meanwhile
    /// must not file the result there.
    index: Option<String>,
    outcome: Arc<Mutex<Option<std::result::Result<Outcome, String>>>>,
}

/// Session state of the QC commands.
#[derive(Default)]
pub struct QcSession {
    pub pending: Vec<PendingQc>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Where the report of `media` is kept.
pub fn report_path(media: &str) -> String {
    format!("{media}{REPORT_SUFFIX}")
}

fn sheet_path(media: &str) -> String {
    format!("{media}{SHEET_SUFFIX}")
}

/// The media file a path names: a report or contact-sheet path names its export.
fn media_of(path: &str) -> &str {
    path.strip_suffix(REPORT_SUFFIX).or_else(|| path.strip_suffix(SHEET_SUFFIX)).unwrap_or(path)
}

/// The project's QC index (`<project>.cfa4qc/index.json`), when the project is saved.
fn index_path(s: &Session) -> Option<String> {
    let p = s.path.as_deref()?;
    let stem = p.strip_suffix(".fcproj").unwrap_or(p);
    Some(format!("{stem}.cfa4qc/index.json"))
}

/// A built-in profile id, or a profile file (`.toml`) read through the host.
fn load_profile(s: &Session, cmd: &str, arg: Option<&str>) -> Result<QcProfile> {
    let arg = arg.unwrap_or("delivery-web");
    if arg.ends_with(".toml") {
        let bytes = s.services.read_file(arg).map_err(|e| bad(cmd, format!("{arg}: {e}")))?;
        let text = String::from_utf8(bytes).map_err(|_| bad(cmd, format!("{arg}: not UTF-8 text")))?;
        return QcProfile::from_toml(&text).map_err(|e| bad(cmd, format!("{arg}: {e}")));
    }
    QcProfile::builtin(arg).map_err(|e| {
        let ids: Vec<&str> = cfa4_qc::profile::BUILTIN.iter().map(|(id, _)| *id).collect();
        bad(cmd, format!("{e} (built-in: {}; or a .toml file)", ids.join(", ")))
    })
}

fn open(s: &Session, path: &str) -> std::io::Result<cfa4_media::SharedReader> {
    if let Some(r) = s.services.reader(path) {
        return r;
    }
    let bytes: Arc<[u8]> = s.services.read_file(path)?.into();
    Ok(Arc::new(cfa4_media::reader::MemReader(bytes)))
}

/// The render plan of a queued export: what its settings and its finished job say the file holds.
fn expectation_of(s: &Session, item: &crate::export_tools::QueueItem) -> Expectation {
    let st = &item.settings;
    let Some(seq) = item.project.sequence(item.sequence) else { return Expectation::default() };
    let q = &seq.settings;
    let r = st.resolve(q.width, q.height, q.frame_rate, q.sample_rate);
    let video = st.has_video();
    let frames = item.job.and_then(|id| s.jobs.iter().find(|j| j.id == id)).and_then(|j| lock(&j.result).clone()).and_then(|r| r.ok()).map(|r| r.frames);
    let mut e = Expectation { video: Some(video), ..Expectation::default() };
    if video && !st.format.is_image_sequence() {
        e.frames = frames;
        e.frame_rate = Some(cfa4_qc::expect::Rate { num: r.rate.num, den: r.rate.den });
        e.width = Some(r.width);
        e.height = Some(r.height);
    }
    if st.has_audio() {
        e.audio_tracks = Some(1);
        e.audio_channels = Some(r.channels);
        e.sample_rate = Some(r.sample_rate);
    } else if video {
        e.audio_tracks = Some(0);
    }
    e
}

/// Fields of `over` replace those of `base` (an explicit `expect` refines a derived plan).
fn merge_expect(base: Expectation, over: &Value, cmd: &str) -> Result<Expectation> {
    let mut v = serde_json::to_value(&base).map_err(|e| EngineError::Other(e.to_string()))?;
    if let (Some(dst), Some(src)) = (v.as_object_mut(), over.as_object()) {
        for (k, x) in src {
            dst.insert(k.clone(), x.clone());
        }
    }
    Expectation::from_json(&v.to_string()).map_err(|e| bad(cmd, e.to_string()))
}

/// Start a QC job for `media`; returns the job id.
fn spawn(
    s: &mut Session,
    media: String,
    profile: QcProfile,
    expect: Expectation,
    links: Links,
    render: Option<RenderOutcome>,
    queue_item: Option<u64>,
    wait: bool,
) -> Result<u64> {
    let reader = open(s, &media).map_err(|e| EngineError::Other(format!("{media}: {e}")))?;
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let name = std::path::Path::new(&media).file_name().map_or_else(|| media.clone(), |n| n.to_string_lossy().into_owned());
    let job = crate::Job { id, label: format!("QC {name}"), progress: Default::default(), result: Default::default() };
    job.progress.total.store(1, Ordering::Relaxed);
    *lock(&job.progress.status) = format!("Checking {name} ({})", profile.id);
    let outcome: Arc<Mutex<Option<std::result::Result<Outcome, String>>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), outcome.clone());
    let services = s.services.clone();
    let tool_path = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let media_path = media.clone();
    let run = move || {
        let started = now_ms();
        let t0 = web_time::Instant::now();
        let opts = Options {
            decode: true,
            links,
            render,
            input: None,
            run: RunInfo { started_unix_ms: started, tool_path, compute: ComputeMode::Cpu, ..RunInfo::default() },
        };
        let r = cfa4_qc::check_with_outputs(&media_path, reader, &profile, &expect, opts, &prog.cancel).map_err(|e| e.to_string()).and_then(|mut c| {
            let sheet = match c.contact_sheet.take() {
                Some(png) => {
                    let p = sheet_path(&media_path);
                    services.write_file(&p, &png).map_err(|e| format!("{p}: {e}"))?;
                    if let Some(r) = c.report.checks.iter_mut().find(|r| r.id == "video.contactSheet") {
                        r.detail.insert("file".into(), json!(p));
                    }
                    Some(p)
                }
                None => None,
            };
            c.report.run.duration_ms = t0.elapsed().as_millis() as u64;
            let report_path = report_path(&media_path);
            let text = c.report.to_json().map_err(|e| e.to_string())?;
            services.write_file(&report_path, text.as_bytes()).map_err(|e| format!("{report_path}: {e}"))?;
            Ok(Outcome {
                verdict: c.report.summary.verdict,
                report_path,
                sheet_path: sheet,
                profile: profile.id.clone(),
                profile_version: profile.version,
                finished_unix_ms: now_ms(),
            })
        });
        *lock(&prog.status) = match &r {
            Ok(o) => format!("QC {}", verdict_word(o.verdict)),
            Err(e) => e.clone(),
        };
        if let Err(e) = &r {
            *lock(&prog.error) = Some(e.clone());
        }
        prog.done.store(1, Ordering::Relaxed);
        let report = |o: &Outcome| cfa4_export::Report {
            path: o.report_path.clone(),
            frames: 0,
            seconds: t0.elapsed().as_secs_f64(),
            bytes: 0,
            render_fps: 0.0,
            extra_files: o.sheet_path.iter().cloned().collect(),
        };
        *lock(&res) = Some(r.as_ref().map(report).map_err(|e| e.clone()));
        *lock(&out) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    let index = index_path(s);
    s.qc.pending.push(PendingQc { job: id, media, queue_item, index, outcome });
    if let Some(i) = queue_item.and_then(|q| s.export_queue.items.iter_mut().find(|i| i.id == q)) {
        i.qc.status = QcStatus::Running;
        i.qc.error = None;
    }
    if wait || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
    } else {
        std::thread::Builder::new().name("cfa4-qc".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    Ok(id)
}

/// Apply finished QC jobs: the queue item's QC state, the project index, a message.
pub fn poll(s: &mut Session) {
    let mut i = 0;
    while i < s.qc.pending.len() {
        let finished = s.jobs.iter().find(|j| j.id == s.qc.pending[i].job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        if !finished {
            i += 1;
            continue;
        }
        let pj = s.qc.pending.remove(i);
        let outcome = lock(&pj.outcome).take();
        let cancelled = s.jobs.iter().find(|j| j.id == pj.job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        let item = pj.queue_item.and_then(|q| s.export_queue.items.iter().position(|i| i.id == q));
        match outcome {
            Some(Ok(o)) => {
                if let Some(it) = item.and_then(|k| s.export_queue.items.get_mut(k)) {
                    it.qc.status = o.verdict.into();
                    it.qc.report = Some(o.report_path.clone());
                }
                if let Err(e) = update_index(s, pj.index.as_deref(), &pj.media, &o) {
                    s.error_toast("qc.run", format!("QC index: {e}"));
                }
                let name = std::path::Path::new(&pj.media).file_name().map_or_else(|| pj.media.clone(), |n| n.to_string_lossy().into_owned());
                let message = format!("QC {name}: {}", verdict_word(o.verdict));
                s.events.push(crate::Event::Toast { message, error: o.verdict == Verdict::Fail });
            }
            Some(Err(e)) => {
                if let Some(it) = item.and_then(|k| s.export_queue.items.get_mut(k)) {
                    it.qc.status = QcStatus::Incomplete;
                    it.qc.error = Some(e.clone());
                }
                if !cancelled {
                    s.error_toast("qc.run", format!("QC: {e}"));
                }
            }
            None => {
                // the job ended without an outcome (a crash the guard caught, or cancelled early)
                if let Some(it) = item.and_then(|k| s.export_queue.items.get_mut(k)) {
                    it.qc.status = QcStatus::Incomplete;
                    it.qc.error = Some(if cancelled { "cancelled".into() } else { "the QC job stopped".into() });
                }
            }
        }
    }
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Warn => "WARN",
        Verdict::Fail => "FAIL",
        Verdict::Incomplete => "INCOMPLETE",
    }
}

fn read_index(s: &Session, path: &str) -> Vec<Value> {
    let Ok(bytes) = s.services.read_file(path) else { return Vec::new() };
    serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| v.get("entries").and_then(Value::as_array).cloned()).unwrap_or_default()
}

/// Record a report in the project's QC index (one entry per export, the latest wins).
fn update_index(s: &Session, index: Option<&str>, media: &str, o: &Outcome) -> std::result::Result<(), String> {
    let Some(path) = index.map(str::to_owned) else { return Ok(()) };
    let mut entries = read_index(s, &path);
    entries.retain(|e| e.get("media").and_then(Value::as_str) != Some(media));
    entries.push(json!({
        "media": media,
        "report": o.report_path,
        "contactSheet": o.sheet_path,
        "verdict": o.verdict,
        "profile": o.profile,
        "profileVersion": o.profile_version,
        "checkedUnixMs": o.finished_unix_ms,
    }));
    let drop = entries.len().saturating_sub(MAX_INDEX);
    entries.drain(..drop);
    let text = serde_json::to_vec_pretty(&json!({"schema": "cfa4-qc-index/1", "entries": entries})).map_err(|e| e.to_string())?;
    if let Some(dir) = std::path::Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty() && !s.services.export_in_memory()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    s.services.write_file(&path, &text).map_err(|e| format!("{path}: {e}"))
}

/// Read and verify the report of `media`.
fn load_report(s: &Session, cmd: &str, media: &str) -> Result<(String, QcReport)> {
    let path = report_path(media);
    let size = s.services.file_size(&path).map_err(|e| bad(cmd, format!("no QC report for {media} ({path}: {e}); run qc.run first")))?;
    if size > MAX_REPORT_BYTES {
        return Err(bad(cmd, format!("{path}: larger than {MAX_REPORT_BYTES} bytes")));
    }
    let bytes = s.services.read_file(&path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| bad(cmd, format!("{path}: not UTF-8 text")))?;
    let report = QcReport::from_json(&text).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    Ok((path, report))
}

fn summary_json(path: &str, r: &QcReport) -> Value {
    json!({
        "report": path,
        "verdict": r.summary.verdict,
        "qualityChecked": r.quality_checked(),
        "summary": r.summary,
    })
}

/// `qc.run`: check a file (or a queued export) with a profile.
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "qc.run";
    let wait = bool_p(p, "wait").unwrap_or(false);
    let profile = load_profile(s, cmd, str_p(p, "profile"))?;
    let (media, base, links, render, item_id) = match u64_p(p, "queueItem") {
        Some(qid) => {
            let it = s.export_queue.items.iter().find(|i| i.id == qid).ok_or_else(|| bad(cmd, format!("no queued export {qid} (see export.queue.list)")))?;
            let render = match it.status {
                QueueStatus::Done => RenderOutcome { status: RenderStatus::Done, error: None },
                QueueStatus::Failed => RenderOutcome { status: RenderStatus::Failed, error: it.error.clone() },
                QueueStatus::Cancelled => RenderOutcome { status: RenderStatus::Cancelled, error: None },
                QueueStatus::Ready | QueueStatus::Encoding => return Err(bad(cmd, format!("queued export {qid} has not finished rendering"))),
            };
            let links = Links {
                project: s.path.clone(),
                sequence: Some(it.sequence_name.clone()),
                export_job: it.job,
                preset: (!it.preset.is_empty()).then(|| it.preset.clone()),
            };
            (it.settings.path.clone(), expectation_of(s, it), links, Some(render), Some(qid))
        }
        None => {
            let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path` (a media file) or `queueItem`"))?;
            (media_of(path).to_string(), Expectation::default(), Links { project: s.path.clone(), ..Links::default() }, None, None)
        }
    };
    let expect = match p.get("expect").filter(|v| !v.is_null()) {
        Some(v) if v.is_object() => merge_expect(base, v, cmd)?,
        Some(_) => return Err(bad(cmd, "`expect` must be an object (the render plan)")),
        None => base,
    };
    if render.as_ref().is_some_and(|r| r.status != RenderStatus::Done) && s.services.file_size(&media).is_err() {
        return Err(bad(cmd, format!("{media}: the render did not finish and left no file")));
    }
    // Two checks of one file would write the same report; whichever finished last would win the
    // report while the queue and the index showed the other. A cancelled check is on its way out.
    if let Some(other) = s.qc.pending.iter().find(|pq| pq.media == media && !job_cancelled(s, pq.job)) {
        return Err(bad(cmd, format!("{media} is already being checked (job {}); wait for it or cancel it with jobs.cancel", other.job)));
    }
    let job = spawn(s, media.clone(), profile, expect, links, render, item_id, wait)?;
    let mut out = json!({"job": job, "path": media, "report": report_path(&media)});
    if wait {
        match lock_job_error(s, job) {
            Some(e) => return Err(EngineError::Other(e)),
            None => {
                let (path, r) = load_report(s, cmd, &media)?;
                out = summary_json(&path, &r);
                out["job"] = json!(job);
                out["path"] = json!(media);
            }
        }
    } else {
        out["poll"] = json!("jobs.list");
        out["cancel"] = json!("jobs.cancel");
    }
    Ok(out)
}

fn job_cancelled(s: &Session, job: u64) -> bool {
    s.jobs.iter().find(|j| j.id == job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed))
}

/// A queued export is rendered again (`export.queue.retry`): its earlier check no longer describes
/// the file. The result is cleared (the profile stays, so the new render is checked), and a check
/// still running on the old file is cancelled and no longer reports to the item.
pub(crate) fn reset_for_retry(s: &mut Session, item_id: u64) {
    let jobs: Vec<u64> =
        s.qc.pending
            .iter_mut()
            .filter(|pq| pq.queue_item == Some(item_id))
            .map(|pq| {
                pq.queue_item = None;
                pq.job
            })
            .collect();
    for j in s.jobs.iter().filter(|j| jobs.contains(&j.id)) {
        j.progress.cancel.store(true, Ordering::Relaxed);
    }
    if let Some(it) = s.export_queue.items.iter_mut().find(|i| i.id == item_id) {
        it.qc = ItemQc { profile: it.qc.profile.take(), ..ItemQc::default() };
    }
}

fn lock_job_error(s: &Session, job: u64) -> Option<String> {
    s.jobs.iter().find(|j| j.id == job).and_then(|j| lock(&j.result).clone()).and_then(|r| r.err())
}

/// `qc.report.get`: the report of a file, verified.
fn report_get(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "qc.report.get";
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path` (the media file or its report)"))?;
    let media = media_of(path);
    let (rpath, r) = load_report(s, cmd, media)?;
    let sheet = sheet_path(media);
    let mut out = summary_json(&rpath, &r);
    out["path"] = json!(media);
    out["contactSheet"] = if s.services.file_size(&sheet).is_ok() { json!(sheet) } else { Value::Null };
    out["questions"] = json!(cfa4_qc::report::QUESTIONS.iter().map(|(id, q)| json!({"id": id, "question": q})).collect::<Vec<_>>());
    out["full"] = serde_json::to_value(&r).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(out)
}

/// `qc.review.save`: store the person's review in the report and recompute the verdict.
fn review_save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "qc.review.save";
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path` (the media file or its report)"))?;
    let media = media_of(path).to_string();
    let review: HumanReview = serde_json::from_value(p.get("review").cloned().unwrap_or(Value::Null)).map_err(|e| bad(cmd, format!("`review`: {e}")))?;
    let (rpath, r) = load_report(s, cmd, &media)?;
    let r = r.with_review(review).map_err(|e| bad(cmd, e.to_string()))?;
    let text = r.to_json().map_err(|e| EngineError::Other(e.to_string()))?;
    s.services.write_file(&rpath, text.as_bytes()).map_err(|e| EngineError::Other(format!("{rpath}: {e}")))?;
    for it in s.export_queue.items.iter_mut().filter(|i| i.settings.path == media) {
        it.qc.status = r.summary.verdict.into();
        it.qc.report = Some(rpath.clone());
    }
    let o = Outcome {
        verdict: r.summary.verdict,
        report_path: rpath.clone(),
        sheet_path: Some(sheet_path(&media)).filter(|x| s.services.file_size(x).is_ok()),
        profile: r.profile.id.clone(),
        profile_version: r.profile.version,
        finished_unix_ms: now_ms(),
    };
    if let Err(e) = update_index(s, index_path(s).as_deref(), &media, &o) {
        s.error_toast(cmd, format!("QC index: {e}"));
    }
    Ok(summary_json(&rpath, &r))
}

/// `qc.profiles.list`: the built-in profiles and their checks.
fn profiles_list(_: &mut Session, _: &Value) -> Result<Value> {
    let mut out = Vec::new();
    for (id, _) in cfa4_qc::profile::BUILTIN {
        let Ok(pr) = QcProfile::builtin(id) else { continue };
        let checks: Vec<Value> = pr
            .checks
            .iter()
            .map(|(cid, spec)| {
                let info = cfa4_qc::catalog::check(cid);
                json!({"id": cid, "requirement": spec.requirement, "pass": spec.pass, "accept": spec.accept,
                    "unit": info.map(|i| i.unit), "title": info.map(|i| i.title)})
            })
            .collect();
        out.push(
            json!({"id": pr.id, "version": pr.version, "title": pr.title, "description": pr.description, "humanReview": pr.human_review, "checks": checks}),
        );
    }
    Ok(json!({"profiles": out}))
}

/// `qc.index`: the reports recorded for the saved project.
fn index(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(path) = index_path(s) else { return Ok(json!({"index": null, "entries": []})) };
    let entries = read_index(s, &path);
    Ok(json!({"index": path, "entries": entries}))
}

/// Start the automatic QC of queued exports that finished rendering (`export.queue.add` with
/// `qc`). Called by the queue after it settles an item.
pub(crate) fn after_export(s: &mut Session, item_id: u64, wait: bool) {
    let Some(it) = s.export_queue.items.iter().find(|i| i.id == item_id) else { return };
    let Some(profile) = it.qc.profile.clone() else { return };
    if it.status != QueueStatus::Done || it.qc.status != QcStatus::NotChecked {
        return;
    }
    let params = json!({"queueItem": item_id, "profile": profile, "wait": wait});
    if let Err(e) = run(s, &params) {
        if let Some(it) = s.export_queue.items.iter_mut().find(|i| i.id == item_id) {
            it.qc.status = QcStatus::Incomplete;
            it.qc.error = Some(e.to_string());
        }
        s.error_toast("qc.run", format!("QC: {e}"));
    }
}

/// The QC part of a queue item's JSON.
pub(crate) fn item_json(q: &ItemQc) -> Value {
    json!({"status": q.status, "profile": q.profile, "report": q.report, "error": q.error})
}

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("qc.run", "Check Media Quality", r#"{"path":str?,"queueItem":id?,"profile":str?,"expect":object?,"wait":bool=false}"#, run, false),
        spec("qc.report.get", "Get QC Report", r#"{"path":str}"#, report_get, false),
        spec("qc.review.save", "Save QC Review", r#"{"path":str,"review":HumanReview}"#, review_save, false),
        spec("qc.profiles.list", "List QC Profiles", "{}", profiles_list, false),
        spec("qc.index", "List Project QC Reports", "{}", index, false),
    ]
}
