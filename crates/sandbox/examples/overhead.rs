//! CFA4-SECURITY-001 B0 measurement kit: what confinement costs.
//!
//! `cargo run --release -p cfa4-sandbox --example overhead -- [files] [rounds]`
//!
//! For each case (unconfined, then confined with a profile) a fresh child process is started; it
//! applies the profile (timed), then runs a system-call-heavy loop over `files` small files (open,
//! read, close, stat, list the directory) `rounds` times, and reports the medians. Landlock checks
//! paths on open and seccomp runs its filter on every system call, so this loop shows their cost
//! at its worst; decoding and rendering make far fewer calls per second. Run it on the reference
//! machine through `scripts/dev/security-overhead.sh`, which pins the cores and records `lscpu`
//! (docs/development/reference-machine.md). Output: one JSON document on stdout.
//!
//! The D2 target (playback and export at most 5% slower through `cfa4-media-host`) is measured in
//! phase B4, when that process exists; this is the baseline of the kernel mechanisms alone.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use cfa4_sandbox::{Context, Mode, ProcessKind, Profile, apply};
use serde_json::{Value, json};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// The bounds of the command line's `files` and `rounds` (also for the child's arguments).
const FILES: (usize, usize) = (1, 100_000);
const ROUNDS: (usize, usize) = (1, 1000);

fn bounded(arg: Option<&String>, default: usize, (lo, hi): (usize, usize)) -> usize {
    arg.and_then(|s| s.parse().ok()).unwrap_or(default).clamp(lo, hi)
}

/// One round over the files; returns nanoseconds per file. Every operation must succeed: a refused
/// read (a confinement that denies the files) would otherwise time less work.
fn round(dir: &Path, files: usize) -> Result<f64, String> {
    let t = Instant::now();
    let mut bytes = 0usize;
    for i in 0..files {
        let p = dir.join(format!("f{i:05}.bin"));
        bytes += std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?.len();
        std::fs::metadata(&p).map_err(|e| format!("stat {}: {e}", p.display()))?;
    }
    let rd = std::fs::read_dir(dir).map_err(|e| format!("list {}: {e}", dir.display()))?;
    for entry in rd {
        entry.map_err(|e| format!("list {}: {e}", dir.display()))?;
        bytes += 1;
    }
    std::hint::black_box(bytes);
    Ok(t.elapsed().as_nanos() as f64 / files.max(1) as f64)
}

fn child(case: &str, dir: &Path, files: usize, rounds: usize) -> Result<Value, String> {
    let profile = match case {
        "none" => None,
        "pluginHost" => Some(Profile::for_process(ProcessKind::PluginHost, &Context { plugin_dirs: vec![dir.to_path_buf()], ..Context::default() })),
        "engine" => {
            Some(Profile::for_process(ProcessKind::Engine, &Context { media: vec![dir.to_path_buf()], loopback_ports: vec![9876], ..Context::default() }))
        }
        other => return Err(format!("unknown case {other}")),
    };
    // warm the page cache and the dentry cache before confining
    round(dir, files)?;
    let t = Instant::now();
    let enforcement = match &profile {
        Some(p) => Some(apply(p, Mode::Strict).map_err(|e| e.to_string())?),
        None => None,
    };
    let apply_us = t.elapsed().as_nanos() as f64 / 1e3;
    let per_file = (0..rounds).map(|_| round(dir, files)).collect::<Result<Vec<f64>, String>>()?;
    Ok(json!({"case": case, "applyUs": apply_us, "nsPerFile": median(per_file), "enforcement": enforcement}))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--child") {
        let (Some(case), Some(dir), Some(files), Some(rounds)) = (args.get(2), args.get(3), args.get(4), args.get(5)) else {
            eprintln!("--child CASE DIR FILES ROUNDS");
            std::process::exit(2);
        };
        let (files, rounds) = (bounded(Some(files), 1000, FILES), bounded(Some(rounds), 20, ROUNDS));
        match child(case, Path::new(dir), files, rounds) {
            Ok(v) => println!("{v}"),
            Err(e) => {
                println!("{}", json!({"case": case, "error": e}));
                std::process::exit(1);
            }
        }
        return;
    }
    let files = bounded(args.get(1), 2000, FILES);
    let rounds = bounded(args.get(2), 20, ROUNDS);
    if let Err(e) = run(files, rounds) {
        eprintln!("overhead: {e}");
        std::process::exit(1);
    }
}

fn run(files: usize, rounds: usize) -> Result<(), String> {
    // `create_dir` (not `_all`) fails on an existing path: a directory or link planted under the
    // predictable name is never written into
    let dir: PathBuf = std::env::temp_dir().join(format!("cfa4-sandbox-overhead-{}-{}", std::process::id(), nonce()));
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let result = measure(&dir, files, rounds);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// A per-run suffix for the temporary directory's name.
fn nonce() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn measure(dir: &Path, files: usize, rounds: usize) -> Result<(), String> {
    for i in 0..files {
        std::fs::write(dir.join(format!("f{i:05}.bin")), [i as u8; 4096]).map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    // interleave the cases so drift (thermal, other load) hits them alike
    for _ in 0..5 {
        for case in ["none", "pluginHost", "engine"] {
            let out = Command::new(&exe)
                .args(["--child", case, &dir.to_string_lossy(), &files.to_string(), &rounds.to_string()])
                .output()
                .map_err(|e| e.to_string())?;
            let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("{case}: {e}: {}", String::from_utf8_lossy(&out.stderr)))?;
            // a failed run (its exit status or an `error`) is reported, never counted
            if !out.status.success() || !v["error"].is_null() {
                return Err(format!("{case}: run failed ({}): {}", out.status, v["error"]));
            }
            cases.push(v);
        }
    }
    let pick = |case: &str, key: &str| -> Option<f64> {
        let v: Vec<f64> = cases.iter().filter(|v| v["case"] == case).filter_map(|v| v[key].as_f64()).collect();
        (!v.is_empty()).then(|| median(v))
    };
    let base = pick("none", "nsPerFile");
    let summary: Vec<Value> = ["none", "pluginHost", "engine"]
        .iter()
        .map(|c| {
            let ns = pick(c, "nsPerFile");
            let overhead = match (ns, base) {
                (Some(ns), Some(base)) if base > 0.0 => Some((ns / base - 1.0) * 100.0),
                _ => None,
            };
            json!({"case": c, "applyUs": pick(c, "applyUs"), "nsPerFile": ns, "overheadPct": overhead})
        })
        .collect();
    let report = json!({
        "files": files,
        "rounds": rounds,
        "support": cfa4_sandbox::probe(),
        "summary": summary,
        "runs": cases,
    });
    println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    Ok(())
}
