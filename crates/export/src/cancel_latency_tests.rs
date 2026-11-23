//! AC-09 (render plan, phase C0): how long a running export takes to stop after it is asked to.
//!
//! The meter: 20 exports, each cancelled at a different frame count. Latency is the time from the
//! store of `Progress::cancel` to the return of `export`. The threshold of the plan (Küszöb@F0 for
//! AC-09) is PENDING until the reference machine has measured it, so this test only asserts a
//! sanity bound and the properties that must always hold: a cancelled export returns
//! `Cancelled` and an uncancelled one still completes.
//!
//! The main output of `export` goes through `AtomicOutput` (`Out::Atomic`): a cancelled export
//! leaves neither a file under the final name nor a `.cfa4-export-*` temp file. Package outputs
//! (DCP, IMF, MXF, OP-Atom: `Out::create_path`) still write in place; that is a separate step.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::*;
use crate::tests::project;

/// Not the AC-09 threshold: only a bound that no healthy run reaches, so a hang fails the test.
const SANITY_BOUND: Duration = Duration::from_secs(5);

/// A directory of its own per export, so the temp-file check never sees another test's export.
fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("fc-export-cancel-{}", std::process::id())).join(name.replace('.', "-"));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().to_string()
}

/// `.cfa4-export-*` temp files in the directory of `path`.
fn temp_files(path: &str) -> usize {
    let dir = std::path::Path::new(path).parent().unwrap();
    std::fs::read_dir(dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(".cfa4-export-")).count()
}

/// Run one export, cancel it once `at` frames are done, and return (latency, result was Cancelled).
fn cancelled_at(at: u64, name: &str) -> (Duration, bool, String) {
    let settings = ExportSettings { format: Format::Mjpeg, path: tmp(name), ..Default::default() };
    let (p, seq, m) = project();
    let prog = Progress::default();
    let (latency, was_cancelled) = std::thread::scope(|s| {
        let worker = s.spawn(|| {
            let r = export(&p, seq, &settings, &m, &prog);
            (Instant::now(), r)
        });
        while prog.done.load(Ordering::Relaxed) < at && !worker.is_finished() {
            std::hint::spin_loop();
        }
        let asked = Instant::now();
        prog.cancel.store(true, Ordering::Relaxed);
        let (returned, r) = worker.join().unwrap();
        (returned.saturating_duration_since(asked), matches!(r, Err(ExportError::Cancelled)))
    });
    (latency, was_cancelled, settings.path)
}

#[test]
fn cancel_latency_is_measured() {
    let mut latencies = Vec::new();
    let (mut cancelled, mut left) = (0, 0);
    for i in 0..20u64 {
        // cancel early: the 48-frame test project is exported in a fraction of a second
        let at = 1 + i % 10;
        let (latency, was_cancelled, path) = cancelled_at(at, &format!("c{i}.mov"));
        assert!(latency < SANITY_BOUND, "cancel at frame {at} took {latency:?}");
        if was_cancelled {
            cancelled += 1;
            left += usize::from(std::path::Path::new(&path).exists());
            assert_eq!(temp_files(&path), 0, "a cancelled export left a temp file next to {path}");
        }
        latencies.push(latency);
    }
    latencies.sort();
    let (max, median, p95) = (latencies[19], latencies[10], latencies[18]);
    // run with --nocapture to read the measurement
    println!("AC-09 cancel latency over 20 runs ({cancelled} cancelled, {left} left a file under the final name): median {median:?}, p95 {p95:?}, max {max:?}");
    assert!(cancelled >= 10, "only {cancelled} of 20 exports were cancelled: the meter did not hit a running export");
}

#[test]
fn an_uncancelled_export_still_completes() {
    let settings = ExportSettings { format: Format::Mjpeg, path: tmp("whole.mov"), ..Default::default() };
    let (p, seq, m) = project();
    let prog = Progress::default();
    export(&p, seq, &settings, &m, &prog).unwrap();
    assert!(!prog.cancel.load(Ordering::Relaxed));
    assert!(std::path::Path::new(&settings.path).exists());
}

#[test]
fn a_cancelled_export_leaves_no_file_under_its_final_name() {
    for i in 0..20u64 {
        let (_, was_cancelled, path) = cancelled_at(1 + i % 10, &format!("n{i}.mov"));
        if was_cancelled {
            assert!(!std::path::Path::new(&path).exists(), "a cancelled export left {path}");
            assert_eq!(temp_files(&path), 0, "a cancelled export left a temp file next to {path}");
        }
    }
}
