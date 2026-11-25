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

/// C0 v0.1 spec exit condition, "20 cancel/SIGKILL cycles leave 0 partial files": the real kill. The
/// test binary starts itself as a child that exports DCP packages in a loop (`child_dcp_export_loop`,
/// active only with `CFA4_SIGKILL_ROOT` set), the parent SIGKILLs it after a different delay each
/// cycle and inspects what is left. A killed process cannot clean up, so `.cfa4-export-*` temp files
/// may remain (they are swept by `atomic::sweep_orphans`), but nothing under a final name may be
/// partial: every `.mxf` must open without warnings, and an asset map exists only next to complete tracks.
/// Limit of the meter: the track writers hand the file over in few large writes, so a kill rarely lands inside
/// one; with staging switched off the same 20 cycles also found no partial file. The test therefore measures
/// the crash behaviour (orphans, sweep, no asset map without tracks); the guarantee itself is shown by
/// `a_package_file_appears_under_its_final_name_only_when_published`.
#[test]
fn child_dcp_export_loop() {
    let Ok(root) = std::env::var("CFA4_SIGKILL_ROOT") else { return };
    let root = std::path::PathBuf::from(root);
    for n in 0.. {
        let dir = root.join(format!("pkg{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = ExportSettings {
            format: Format::Dcp,
            path: dir.to_string_lossy().to_string(),
            range: Some(cfa4_time::TimeRange::new(cfa4_time::Tick::ZERO, cfa4_time::FrameRate::new(25, 1).tick_of(40))),
            dcp_content_kind: "test".into(),
            dcp_language: "hu-HU".into(),
            ..Default::default()
        };
        let (p, seq, m) = project();
        let _ = export(&p, seq, &settings, &m, &Progress::default());
    }
}

#[test]
fn sigkill_cycles_leave_no_partial_file_under_a_final_name() {
    if std::env::var("CFA4_SIGKILL_ROOT").is_ok() {
        return; // this is the child process
    }
    let root = std::env::temp_dir().join(format!("fc-export-sigkill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let exe = std::env::current_exe().unwrap();
    let (mut orphans, mut finals, mut packages_done, mut mid_write) = (0usize, 0usize, 0usize, 0usize);
    let spawn = |dir: &std::path::Path| {
        std::process::Command::new(&exe)
            .args(["--exact", "cancel_latency_tests::child_dcp_export_loop", "--nocapture", "--test-threads=1"])
            .env("CFA4_SIGKILL_ROOT", dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    // The kill is timed from the moment the child starts its first package, in fractions of the
    // time one package takes on this machine: a fixed wall-clock delay from the spawn found no
    // write at all on a loaded CI runner, where starting the child and its first package alone
    // took longer than the longest delay.
    let wait_for = |path: &std::path::Path| {
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < Duration::from_secs(300), "the child never reached {}", path.display());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let package = {
        let dir = root.join("calibration");
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = spawn(&dir);
        wait_for(&dir.join("pkg0"));
        let start = Instant::now();
        wait_for(&dir.join("pkg1"));
        let package = start.elapsed();
        child.kill().unwrap();
        child.wait().unwrap();
        package
    };
    const CYCLES: u64 = 20;
    for cycle in 0..CYCLES {
        let dir = root.join(format!("cycle{cycle}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = spawn(&dir);
        wait_for(&dir.join("pkg0"));
        // 6 % .. 100 % of a package (0.3 s .. 4.8 s of a 4.8 s one), so the kill lands before,
        // during and after the track files
        let permille = 60 + (cycle * 49) % 940;
        std::thread::sleep(package.mul_f64(permille as f64 / 1000.0));
        child.kill().unwrap(); // SIGKILL on unix
        child.wait().unwrap();

        for pkg in std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().is_dir()) {
            let names: Vec<String> = std::fs::read_dir(pkg.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            let temps = names.iter().filter(|n| n.starts_with(".cfa4-export-")).count();
            orphans += temps;
            mid_write += usize::from(temps > 0);
            let mxfs: Vec<&String> = names.iter().filter(|n| n.ends_with(".mxf")).collect();
            for n in &mxfs {
                let bytes = std::fs::read(pkg.path().join(n)).unwrap();
                let mxf = cfa4_mxf::open(&bytes).unwrap_or_else(|e| panic!("cycle {cycle}: {n} is partial under its final name: {e:?}"));
                assert!(mxf.warnings.is_empty(), "cycle {cycle}: {n}: {:?}", mxf.warnings);
                finals += 1;
            }
            if names.iter().any(|n| n.starts_with("ASSETMAP")) {
                packages_done += 1;
                assert!(names.iter().any(|n| n.starts_with("PKL") || n.contains("pkl")), "cycle {cycle}: asset map without a PKL: {names:?}");
                assert!(!mxfs.is_empty(), "cycle {cycle}: asset map without track files: {names:?}");
            }
            // the sweep removes what the kill left behind, and only that
            let swept = atomic::sweep_orphans(&pkg.path(), Duration::ZERO).unwrap();
            assert_eq!(swept.len(), temps, "cycle {cycle}: sweep removed {} of {temps} orphans", swept.len());
        }
    }
    println!(
        "C0 SIGKILL: {CYCLES} cycles; {finals} track files under final names, all complete; {packages_done} complete packages; {orphans} orphan temp files in {mid_write} packages (swept)"
    );
    assert!(finals + orphans > 0, "no cycle reached a file write: the kill came too early to measure anything");
    let _ = std::fs::remove_dir_all(&root);
}
