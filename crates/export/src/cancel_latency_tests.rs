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
//! (DCP, IMF, MXF, OP-Atom: `Out::create_path`) are staged per file the same way (C0 v0.1 spec, D6).

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

/// Cancel a DCP export once `at` frames are done; returns (was cancelled, package folder).
fn dcp_cancelled_at(at: u64, name: &str) -> (bool, std::path::PathBuf) {
    // `tmp` creates a folder of its own for the name and returns a path inside it: that folder is the package
    let dir = std::path::Path::new(&tmp(name)).parent().unwrap().to_path_buf();
    let settings = ExportSettings {
        format: Format::Dcp,
        path: dir.to_string_lossy().to_string(),
        range: Some(cfa4_time::TimeRange::new(cfa4_time::Tick::ZERO, cfa4_time::FrameRate::new(25, 1).tick_of(40))),
        dcp_content_kind: "test".into(),
        dcp_language: "hu-HU".into(),
        ..Default::default()
    };
    let (p, seq, m) = project();
    let prog = Progress::default();
    let was_cancelled = std::thread::scope(|s| {
        let worker = s.spawn(|| export(&p, seq, &settings, &m, &prog));
        while prog.done.load(Ordering::Relaxed) < at && !worker.is_finished() {
            std::hint::spin_loop();
        }
        prog.cancel.store(true, Ordering::Relaxed);
        matches!(worker.join().unwrap(), Err(ExportError::Cancelled))
    });
    (was_cancelled, dir)
}

/// C0 v0.1 spec, D6: the package outputs (DCP, IMF, MXF, OP-Atom) are staged per file too. A cancelled
/// DCP leaves no `.cfa4-export-*` temp file, no asset map (it is written last), and every track file
/// under its final name is a complete MXF.
#[test]
fn a_cancelled_dcp_leaves_no_partial_file_under_a_final_name() {
    let mut cancelled = 0;
    for i in 0..10u64 {
        let (was_cancelled, dir) = dcp_cancelled_at(3 + 3 * (i % 5), &format!("dcp{i}"));
        if !was_cancelled {
            continue;
        }
        cancelled += 1;
        let names: Vec<String> =
            std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
        assert!(names.iter().all(|n| !n.starts_with(".cfa4-export-")), "temp file left in {dir:?}: {names:?}");
        assert!(!names.iter().any(|n| n.starts_with("ASSETMAP")), "a cancelled package has an asset map: {names:?}");
        for n in names.iter().filter(|n| n.ends_with(".mxf")) {
            let bytes = std::fs::read(dir.join(n)).unwrap();
            let mxf = cfa4_mxf::open(&bytes).unwrap_or_else(|e| panic!("{n} is a partial file under its final name: {e:?}"));
            assert!(mxf.warnings.is_empty(), "{n}: {:?}", mxf.warnings);
        }
    }
    assert!(cancelled >= 3, "only {cancelled} of 10 DCP exports were cancelled: the test did not hit a running export");
}

/// The mechanism behind the test above, without timing: a package file that is written but never
/// published (an error, a cancel, a kill) is not there under its final name, and one that is
/// published is complete.
#[test]
fn a_package_file_appears_under_its_final_name_only_when_published() {
    use std::io::Write as _;
    let settings = ExportSettings { format: Format::Dcp, ..Default::default() };
    let dir = std::path::Path::new(&tmp("pkgfile")).parent().unwrap().to_path_buf();
    let path = dir.join("track.mxf").to_string_lossy().to_string();

    let mut out = Out::create_path(&settings, &path).unwrap();
    out.write_all(b"half a track file").unwrap();
    out.flush().unwrap();
    assert!(!std::path::Path::new(&path).exists(), "a written but unpublished package file is visible under its final name");
    drop(out);
    assert!(!std::path::Path::new(&path).exists());
    assert_eq!(temp_files(&path), 0, "a dropped package file left a temp file");

    let mut out = Out::create_path(&settings, &path).unwrap();
    out.write_all(b"a whole track file").unwrap();
    out.publish(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"a whole track file");
    assert_eq!(temp_files(&path), 0);

    // the small documents (CPL, PKL, asset map) go through the same staging
    write_output(&settings, &dir.join("ASSETMAP.xml").to_string_lossy(), b"<AssetMap/>".to_vec()).unwrap();
    assert_eq!(std::fs::read(dir.join("ASSETMAP.xml")).unwrap(), b"<AssetMap/>");
    assert_eq!(temp_files(&path), 0);
}
