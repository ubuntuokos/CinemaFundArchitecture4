//! Background jobs through the CFA4 task queue (ACS-6): proxies, transcodes, the project manager,
//! scene detection, mask tracking, preview renders and narration ask [`crate::task_host`] for
//! their resources before their thread starts.
//!
//! A job is listed (`jobs.list`) from the moment it is asked for. When the machine has no room for
//! it, it waits with its reason in its status line and starts from [`pump`] (called with the
//! other per-frame work, [`crate::Session::poll_persistence`]) once it fits; cancelling it while
//! it waits ends it at once. When its thread finishes, its reservation is released. Blocking
//! (`wait`) and single-threaded (wasm32) runs do not come here: the caller runs them at once.

use std::sync::atomic::Ordering;

use cfa4_tasks::manager::Class;

use crate::task_host::{Admission, Outcome};
use crate::{EngineError, Result, Session};

type Run = Box<dyn FnOnce() + Send + 'static>;

/// A job waiting for its resources.
struct Waiting {
    job: u64,
    task: String,
    name: String,
    run: Run,
}

/// The jobs this module started or holds back.
#[derive(Default)]
pub struct Background {
    waiting: Vec<Waiting>,
    /// (job, task) of the jobs whose thread runs.
    running: Vec<(u64, String)>,
}

impl Background {
    /// Jobs still waiting for resources.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

/// What a job asks of the machine.
#[derive(Clone, Copy, Debug)]
pub struct Need {
    pub class: Class,
    pub ram_bytes: u64,
    pub cpu_milli: u64,
}

impl Need {
    pub const fn background(ram_mib: u64, cpus: u64) -> Need {
        Need { class: Class::Background, ram_bytes: ram_mib.saturating_mul(1 << 20), cpu_milli: cpus.saturating_mul(1000) }
    }
}

fn set_status(s: &Session, job: u64, text: String) {
    if let Some(j) = s.jobs.iter().find(|j| j.id == job) {
        *j.progress.status.lock().unwrap_or_else(|e| e.into_inner()) = text;
    }
}

fn fail(s: &Session, job: u64, message: String) {
    if let Some(j) = s.jobs.iter().find(|j| j.id == job) {
        *j.progress.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(message.clone());
        j.progress.finished.store(true, Ordering::Relaxed);
        *j.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err(message));
    }
}

fn spawn(s: &mut Session, job: u64, task: String, name: &str, run: Run) -> Result<()> {
    match std::thread::Builder::new().name(name.into()).spawn(run) {
        Ok(_) => {
            s.background.running.push((job, task));
            Ok(())
        }
        Err(e) => {
            s.tasks.end(&task, Outcome::Failed);
            Err(EngineError::Other(e.to_string()))
        }
    }
}

/// Start job `job` (already in `s.jobs`) on a thread named `name` once the task queue gives it
/// `need`; until then it waits. `run` is the guarded job body.
pub fn start(s: &mut Session, job: u64, need: Need, name: &str, run: impl FnOnce() + Send + 'static) -> Result<()> {
    let task = s.tasks.new_id("job");
    let label = s.jobs.iter().find(|j| j.id == job).map_or_else(|| name.to_owned(), |j| j.label.clone());
    s.tasks.set_label(&task, &label);
    match s.tasks.admit(&task, |h| h.fit(need.class, need.ram_bytes, need.cpu_milli)) {
        Admission::Start => spawn(s, job, task, name, Box::new(run)),
        Admission::Wait(reasons) => {
            set_status(s, job, format!("Waiting: {}", reasons.join("; ")));
            s.background.waiting.push(Waiting { job, task, name: name.into(), run: Box::new(run) });
            Ok(())
        }
        Admission::Refused(reason) => {
            s.tasks.end(&task, Outcome::Failed);
            fail(s, job, format!("no resources for this job: {reason}"));
            Ok(())
        }
    }
}

/// Release the resources of finished jobs and start the waiting ones that fit now.
pub fn pump(s: &mut Session) {
    let running = std::mem::take(&mut s.background.running);
    for (job, task) in running {
        let result = s.jobs.iter().find(|j| j.id == job).map(|j| j.result.lock().unwrap_or_else(|e| e.into_inner()).clone());
        let outcome = match result {
            Some(None) => {
                s.background.running.push((job, task));
                continue;
            }
            Some(Some(Ok(_))) => Outcome::Succeeded,
            Some(Some(Err(e))) if e == "cancelled" => Outcome::Cancelled,
            Some(Some(Err(_))) | None => Outcome::Failed,
        };
        s.tasks.end(&task, outcome);
    }
    let waiting = std::mem::take(&mut s.background.waiting);
    for w in waiting {
        let cancelled = s.jobs.iter().find(|j| j.id == w.job).is_none_or(|j| j.progress.cancel.load(Ordering::Relaxed));
        if cancelled {
            s.tasks.end(&w.task, Outcome::Cancelled);
            fail(s, w.job, "cancelled".into());
            continue;
        }
        match s.tasks.admit(&w.task, |h| h.fit(Class::Background, 0, 0)) {
            Admission::Start => {
                set_status(s, w.job, String::new());
                if let Err(e) = spawn(s, w.job, w.task, &w.name, w.run) {
                    fail(s, w.job, e.to_string());
                }
            }
            Admission::Wait(reasons) => {
                set_status(s, w.job, format!("Waiting: {}", reasons.join("; ")));
                s.background.waiting.push(w);
            }
            Admission::Refused(reason) => {
                s.tasks.end(&w.task, Outcome::Failed);
                fail(s, w.job, format!("no resources for this job: {reason}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    use cfa4_tasks::{Current, Nominal, Snapshot};

    use super::*;
    use crate::task_host::TaskHost;

    const GIB: u64 = 1 << 30;

    fn session(free: Arc<AtomicU64>) -> Session {
        Session {
            tasks: TaskHost::new(Arc::new(move || Snapshot {
                nominal: Nominal { ram_physical_bytes: Some(16 * GIB), cpus_online: Some(8), ..Nominal::default() },
                current: Current { ram_available_bytes: Some(free.load(Ordering::Relaxed)), ..Current::default() },
                notes: vec![],
            })),
            ..Session::default()
        }
    }

    fn job(s: &mut Session, id: u64) -> crate::Job {
        let job = crate::Job { id, label: format!("job {id}"), progress: Default::default(), result: Default::default() };
        s.jobs.push(job.clone());
        job
    }

    fn body(job: &crate::Job, ran: &Arc<AtomicBool>) -> impl FnOnce() + Send + 'static {
        let (res, ran) = (job.result.clone(), ran.clone());
        move || {
            ran.store(true, Ordering::Relaxed);
            *res.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err("done in test".into()));
        }
    }

    fn settle(s: &mut Session, job: &crate::Job) {
        let t0 = web_time::Instant::now();
        while job.result.lock().unwrap().is_none() || !s.background.running.is_empty() {
            assert!(t0.elapsed() < std::time::Duration::from_secs(10));
            pump(s);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_job_waits_for_memory_then_runs_and_releases_it() {
        let free = Arc::new(AtomicU64::new(GIB));
        let mut s = session(free.clone());
        let j = job(&mut s, 1);
        let ran = Arc::new(AtomicBool::new(false));
        start(&mut s, 1, Need::background(512, 1), "test-job", body(&j, &ran)).unwrap();
        assert_eq!(s.background.waiting(), 1);
        assert!(j.progress.status.lock().unwrap().starts_with("Waiting: "), "{}", j.progress.status.lock().unwrap());
        pump(&mut s);
        assert!(!ran.load(Ordering::Relaxed), "still no room");
        free.store(12 * GIB, Ordering::Relaxed);
        s.tasks.resample();
        settle(&mut s, &j);
        assert!(ran.load(Ordering::Relaxed));
        assert_eq!((s.background.waiting(), s.tasks.held()), (0, 0));
    }

    #[test]
    fn cancelling_a_waiting_job_ends_it_without_running_it() {
        let mut s = session(Arc::new(AtomicU64::new(GIB)));
        let j = job(&mut s, 1);
        let ran = Arc::new(AtomicBool::new(false));
        start(&mut s, 1, Need::background(512, 1), "test-job", body(&j, &ran)).unwrap();
        j.progress.cancel.store(true, Ordering::Relaxed);
        pump(&mut s);
        assert!(matches!(j.result.lock().unwrap().as_ref(), Some(Err(e)) if e == "cancelled"));
        assert!(!ran.load(Ordering::Relaxed));
        assert_eq!(s.tasks.queue().tasks()[0].state, cfa4_tasks::State::Cancelled);
    }

    #[test]
    fn a_job_that_fits_starts_at_once() {
        let mut s = session(Arc::new(AtomicU64::new(12 * GIB)));
        let j = job(&mut s, 1);
        let ran = Arc::new(AtomicBool::new(false));
        start(&mut s, 1, Need::background(512, 1), "test-job", body(&j, &ran)).unwrap();
        assert_eq!(s.background.waiting(), 0);
        settle(&mut s, &j);
        assert!(ran.load(Ordering::Relaxed));
        assert_eq!(s.tasks.queue().tasks()[0].state, cfa4_tasks::State::Failed, "the body reported an error");
    }
}
