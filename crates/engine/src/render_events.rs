//! C0: turns an export's [`cfa4_export::Progress`] into the render job's [`RenderEvent`]s.
//!
//! The adapter lives here because `cfa4-engine` is the lowest crate that sees both `cfa4-export`
//! and `cfa4-tasks` (the layering keeps them apart). It only reads the shared counters.

use std::sync::atomic::Ordering;

use cfa4_export::Progress;
use cfa4_tasks::RenderEvent;

/// The current state of `progress` as a progress event for job `job`. The ETA is read from the
/// pace window of [`Progress::eta`], so ask at a steady rhythm (its readings feed the estimate).
pub fn progress_event(job: &str, progress: &Progress) -> RenderEvent {
    let done = progress.done.load(Ordering::Relaxed);
    let total = progress.total.load(Ordering::Relaxed);
    let eta_ms = progress.eta().map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64);
    let message = progress.status.lock().map(|s| s.clone()).unwrap_or_default();
    RenderEvent::progress(job, done.min(total.max(done)), total, eta_ms, &message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_counters_and_the_status_text() {
        let p = Progress::default();
        p.total.store(48, Ordering::Relaxed);
        p.done.store(12, Ordering::Relaxed);
        *p.status.lock().unwrap() = "encoding".into();
        let e = progress_event("job-1", &p);
        assert_eq!(e.job, "job-1");
        assert!(e.is_consistent());
        let pr = e.progress.unwrap();
        assert_eq!((pr.done, pr.total), (12, 48));
        assert_eq!(e.message, "encoding");
        assert!((pr.fraction() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn no_eta_before_there_is_enough_to_estimate_from() {
        let p = Progress::default();
        p.total.store(10, Ordering::Relaxed);
        assert_eq!(progress_event("j", &p).progress.unwrap().eta_ms, None);
    }

    #[test]
    fn the_event_serialises_with_the_wire_names() {
        let p = Progress::default();
        p.total.store(2, Ordering::Relaxed);
        let json = serde_json::to_string(&progress_event("j", &p)).unwrap();
        assert!(json.contains("\"job\":\"j\"") && json.contains("\"progress\""), "{json}");
    }
}
