//! AC-08 meter (CFA4-RENDER-INTEGRATION-PLAN-001, F0V/C0): an export killed with SIGKILL at any
//! moment never leaves a truncated file under the final name. The writer is a child process (this
//! test binary run again) using `cfa4_export::atomic::AtomicOutput`; the parent kills it at random
//! and at fixed moments and inspects the destination.
//!
//! What is measured: files with the *final name* are absent or complete, in every cycle. What is
//! reported, not hidden: the staged `.cfa4-export-*` temp file of a killed writer stays in the
//! destination directory (no destructor runs on SIGKILL); a sweep of those is a separate item.
//! The mutation check writes straight to the final name and must be caught.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use cfa4_export::atomic::{AtomicOutput, Overwrite};

const TOTAL: usize = 6 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 7)) as u8
}

/// The child: write `TOTAL` bytes to `<dir>/out.bin` either atomically or (mutation) directly.
/// Half way it creates `<dir>/half` and, in `hold` mode, sleeps so the parent can kill it there.
fn child(dir: &Path, mode: &str) {
    let final_path = dir.join("out.bin");
    let hold = mode.ends_with("-hold");
    let buf: Vec<u8> = (0..CHUNK).map(pattern).collect();
    let mut written = 0;
    let step = |w: &mut dyn Write, written: &mut usize| {
        while *written < TOTAL {
            let n = CHUNK.min(TOTAL - *written);
            w.write_all(&buf[..n]).ok();
            *written += n;
            if *written >= TOTAL / 2 && !dir.join("half").exists() {
                w.flush().ok();
                fs::write(dir.join("half"), b"1").ok();
                if hold {
                    sleep(Duration::from_secs(30));
                }
            }
        }
    };
    if mode.starts_with("direct") {
        let mut f = fs::File::create(&final_path).expect("create");
        step(&mut f, &mut written);
    } else {
        let mut out = AtomicOutput::create(&final_path, Overwrite::Refuse).expect("stage");
        step(&mut out, &mut written);
        out.commit().expect("commit");
    }
}

#[test]
fn sigkill_child_entry() {
    // Run as a child only when asked; as a plain test it does nothing.
    if let (Ok(dir), Ok(mode)) = (std::env::var("CFA4_SIGKILL_DIR"), std::env::var("CFA4_SIGKILL_MODE")) {
        child(Path::new(&dir), &mode);
    }
}

fn spawn(dir: &Path, mode: &str) -> std::process::Child {
    let exe = std::env::current_exe().expect("exe");
    Command::new(exe)
        .args(["--exact", "sigkill_child_entry", "--nocapture", "--test-threads=1"])
        .env("CFA4_SIGKILL_DIR", dir)
        .env("CFA4_SIGKILL_MODE", mode)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child")
}

/// Whether the final-name file is absent or exactly the full pattern.
fn final_state(dir: &Path) -> Result<bool, String> {
    let p = dir.join("out.bin");
    match fs::read(&p) {
        Err(_) => Ok(false),
        Ok(b) if b.len() == TOTAL && b.iter().enumerate().all(|(i, v)| *v == pattern(i % CHUNK)) => Ok(true),
        Ok(b) => Err(format!("truncated or wrong final file: {} of {TOTAL} bytes", b.len())),
    }
}

fn temps(dir: &Path) -> usize {
    fs::read_dir(dir).map(|d| d.filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with(".cfa4-export-")).count()).unwrap_or(0)
}

fn wait_for(path: PathBuf) {
    let t = Instant::now();
    while !path.exists() {
        assert!(t.elapsed() < Duration::from_secs(20), "child never reached the half way mark");
        sleep(Duration::from_millis(2));
    }
}

#[test]
fn killed_at_the_half_way_mark_leaves_no_final_file() {
    let d = tempfile::tempdir().expect("tmp");
    let mut c = spawn(d.path(), "atomic-hold");
    wait_for(d.path().join("half"));
    c.kill().expect("SIGKILL");
    c.wait().expect("wait");
    assert_eq!(final_state(d.path()), Ok(false), "the final name must not exist");
    eprintln!("AC-08 fixed kill: final absent; orphan staged temp files: {}", temps(d.path()));
}

#[test]
fn random_time_kills_never_leave_a_truncated_final_file() {
    const CYCLES: usize = 40;
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let (mut absent, mut complete, mut orphans) = (0, 0, 0);
    for cycle in 0..CYCLES {
        let d = tempfile::tempdir().expect("tmp");
        let mut c = spawn(d.path(), "atomic");
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        sleep(Duration::from_millis(5 + (seed >> 33) % 120));
        c.kill().ok();
        c.wait().ok();
        match final_state(d.path()) {
            Ok(true) => complete += 1,
            Ok(false) => absent += 1,
            Err(e) => panic!("cycle {cycle}: {e}"),
        }
        orphans += temps(d.path());
    }
    eprintln!("AC-08 random kills: {CYCLES} cycles, final absent {absent}, complete {complete}, truncated 0; orphan staged temp files {orphans}");
    assert_eq!(absent + complete, CYCLES);
}

/// The meter can fail: a writer that goes straight to the final name is caught.
#[test]
fn the_meter_catches_a_non_atomic_writer() {
    let d = tempfile::tempdir().expect("tmp");
    let mut c = spawn(d.path(), "direct-hold");
    wait_for(d.path().join("half"));
    c.kill().expect("SIGKILL");
    c.wait().expect("wait");
    assert!(final_state(d.path()).is_err(), "a direct writer killed half way must leave a truncated final file");
}
