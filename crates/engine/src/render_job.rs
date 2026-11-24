//! The render job API v0 on the control channel (C0 v0.1 spec, section 3): the plan's job states
//! and events as a *view* of the jobs the session already runs. There is no second queue and no
//! second state machine: a [`Job`] keeps its progress and result where it always did, and these
//! commands only read them through the contract types of `cfa4-tasks` ([`RenderState`],
//! [`RenderEvent`]) so that the CLI and the Qt GUI see the same shapes.
//!
//! | Command | Does |
//! |---|---|
//! | `render.status` `{"job":id}` | state and the latest event of one job |
//! | `render.list` | the same for every job |
//! | `render.cancel` `{"job":id}` | asks the job to stop (cooperative, same flag as `jobs.cancel`) |
//!
//! A render is submitted with the existing export commands (`file.exportMedia` and the others),
//! which answer the job id; `render.submit` is not a second way in.

use std::sync::atomic::Ordering;

use cfa4_export::ExportError;
use cfa4_tasks::{RenderEvent, RenderEventKind, RenderState};
use serde_json::{Value, json};

use crate::Job;
use crate::commands::{CommandSpec, always, bad, u64_p};

/// The state a client shows for `job`. A job exists from the moment it starts, so there is no
/// Queued here (the Export Queue lists what has not started).
pub fn state_of(job: &Job) -> RenderState {
    let cancel_asked = job.progress.cancel.load(Ordering::Relaxed);
    match &*job.result.lock().unwrap_or_else(|e| e.into_inner()) {
        None if cancel_asked => RenderState::Cancelling,
        None => RenderState::Running,
        Some(Ok(_)) => RenderState::Completed,
        Some(Err(e)) if cancel_asked && *e == ExportError::Cancelled.to_string() => RenderState::Cancelled,
        Some(Err(_)) => RenderState::Failed,
    }
}

/// The latest report of `job`: a progress event while it runs or when it completes, a warning
/// carrying the error when it failed (a cancel is not a failure and carries no warning).
pub fn event_of(job: &Job) -> RenderEvent {
    let id = job.id.to_string();
    let done = job.progress.done.load(Ordering::Relaxed);
    let total = job.progress.total.load(Ordering::Relaxed);
    let eta_ms = job.progress.eta().map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let status = job.progress.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let failed = state_of(job) == RenderState::Failed;
    if failed {
        let error = job.progress.error.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
        return RenderEvent { job: id, kind: RenderEventKind::Warning, progress: None, message: error, fallback: None };
    }
    RenderEvent::progress(&id, done, total, eta_ms, &status)
}

fn status_json(job: &Job) -> Value {
    let event = event_of(job);
    json!({
        "job": job.id,
        "label": job.label,
        "state": state_of(job),
        "event": event,
    })
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "render.status",
            label: "Render Job Status",
            menu: &[],
            shortcut: None,
            params: r#"{"job":id}"#,
            enabled: always,
            run: |s, p| {
                let id = u64_p(p, "job").ok_or_else(|| bad("render.status", "need `job`"))?;
                let j = s.jobs.iter().find(|j| j.id == id).ok_or_else(|| bad("render.status", "no such job"))?;
                Ok(status_json(j))
            },
            journal: false,
        },
        CommandSpec {
            id: "render.list",
            label: "List Render Jobs",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(Value::Array(s.jobs.iter().map(status_json).collect())),
            journal: false,
        },
        CommandSpec {
            id: "render.cancel",
            label: "Cancel Render Job",
            menu: &[],
            shortcut: None,
            params: r#"{"job":id}"#,
            enabled: always,
            run: |s, p| {
                let id = u64_p(p, "job").ok_or_else(|| bad("render.cancel", "need `job`"))?;
                let j = s.jobs.iter().find(|j| j.id == id).ok_or_else(|| bad("render.cancel", "no such job"))?;
                j.progress.cancel.store(true, Ordering::Relaxed);
                Ok(json!({"job": id, "state": state_of(j)}))
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn job(result: Option<std::result::Result<cfa4_export::Report, String>>, cancel: bool) -> Job {
        let progress: Arc<cfa4_export::Progress> = Arc::default();
        progress.cancel.store(cancel, Ordering::Relaxed);
        Job { id: 7, label: "x".into(), progress, result: Arc::new(Mutex::new(result)) }
    }

    #[test]
    fn job_states_follow_the_plans_names() {
        assert_eq!(state_of(&job(None, false)), RenderState::Running);
        assert_eq!(state_of(&job(None, true)), RenderState::Cancelling);
        assert_eq!(state_of(&job(Some(Err(ExportError::Cancelled.to_string())), true)), RenderState::Cancelled);
        assert_eq!(state_of(&job(Some(Err("I/O: disk full".into())), false)), RenderState::Failed);
        // a failure that arrives after a cancel was asked is still a failure
        assert_eq!(state_of(&job(Some(Err("I/O: disk full".into())), true)), RenderState::Failed);
        assert_eq!(
            state_of(&job(
                Some(Ok(cfa4_export::Report { path: String::new(), frames: 0, seconds: 0.0, bytes: 0, render_fps: 0.0, extra_files: Vec::new() })),
                false
            )),
            RenderState::Completed
        );
    }

    #[test]
    fn events_are_consistent_and_a_failure_is_a_warning() {
        let running = job(None, false);
        running.progress.total.store(10, Ordering::Relaxed);
        running.progress.done.store(4, Ordering::Relaxed);
        let e = event_of(&running);
        assert!(e.is_consistent());
        assert_eq!((e.kind, e.progress.map(|p| (p.done, p.total))), (RenderEventKind::Progress, Some((4, 10))));

        let failed = job(Some(Err("I/O: disk full".into())), false);
        *failed.progress.error.lock().unwrap() = Some("I/O: disk full".into());
        let e = event_of(&failed);
        assert!(e.is_consistent());
        assert_eq!((e.kind, e.message.as_str()), (RenderEventKind::Warning, "I/O: disk full"));

        let cancelled = job(Some(Err(ExportError::Cancelled.to_string())), true);
        assert_eq!(event_of(&cancelled).kind, RenderEventKind::Progress, "a cancel is not a warning");
    }
}

/// Through the control channel: a real export is started, followed with `render.status`, listed, and
/// a second one is cancelled; the states are the plan's names and every event is consistent.
#[cfg(test)]
mod command_tests {
    use serde_json::json;

    use crate::Session;

    fn state(s: &mut Session, job: u64) -> serde_json::Value {
        s.execute("render.status", json!({"job": job})).unwrap()
    }

    #[test]
    fn an_export_is_followed_through_render_status_to_completed() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let dir = std::env::temp_dir().join(format!("cfa4-render-job-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = s
            .execute(
                "file.exportMedia",
                json!({"path": dir.join("a.wav").to_string_lossy(), "format": "wav", "range": "custom", "startSeconds": 0, "endSeconds": 0.25}),
            )
            .unwrap();
        let job = r["job"].as_u64().unwrap();
        let mut last = json!(null);
        for _ in 0..1200 {
            last = state(&mut s, job);
            if last["state"] == "completed" {
                break;
            }
            assert!(matches!(last["state"].as_str(), Some("running")), "{last}");
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(last["state"], "completed", "{last}");
        assert_eq!(last["event"]["kind"], "progress", "{last}");
        let listed = s.execute("render.list", json!({})).unwrap();
        assert!(listed.as_array().unwrap().iter().any(|j| j["job"] == job && j["state"] == "completed"), "{listed}");
        assert!(s.execute("render.status", json!({"job": 99_999_u64})).is_err());
        assert!(s.execute("render.status", json!({})).is_err());
        assert!(s.execute("render.cancel", json!({"job": 99_999_u64})).is_err());
    }

    /// C0 exit condition "the renderer through the contract is bit-identical to the golden set, PTS diff 0":
    /// the golden here is the waited export (the existing synchronous path); the same export run as a job and
    /// followed through `render.status` must produce the same bytes, so the job view changes nothing in the
    /// render. Video (MJPEG in MOV) and audio (WAV) over the demo project; synthetic, own material.
    #[test]
    fn a_job_export_is_byte_identical_to_the_waited_export() {
        let dir = std::env::temp_dir().join(format!("cfa4-render-job-golden-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (format, ext) in [("mjpeg", "mov"), ("wav", "wav")] {
            let mut s = Session::default();
            s.execute("file.openDemoProject", json!({})).unwrap();
            let golden = dir.join(format!("golden.{ext}"));
            let viajob = dir.join(format!("job.{ext}"));
            let p = |path: &std::path::Path| json!({"path": path.to_string_lossy(), "format": format, "range": "custom", "startSeconds": 0, "endSeconds": 0.5});
            let mut waited = p(&golden);
            waited["wait"] = json!(true);
            s.execute("file.exportMedia", waited).unwrap();
            let job = s.execute("file.exportMedia", p(&viajob)).unwrap()["job"].as_u64().unwrap();
            let mut last = json!(null);
            for _ in 0..2400 {
                last = state(&mut s, job);
                if last["state"] != "running" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            assert_eq!(last["state"], "completed", "{format}: {last}");
            let (a, b) = (std::fs::read(&golden).unwrap(), std::fs::read(&viajob).unwrap());
            assert!(!a.is_empty(), "{format}: empty golden");
            assert_eq!(a, b, "{format}: the job export differs from the waited export ({} vs {} bytes)", a.len(), b.len());
        }
    }
}
