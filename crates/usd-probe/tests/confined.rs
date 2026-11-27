//! SB-G6 (3): the worker runs confined (Landlock + seccomp through `cfa4-usd-probe-launch`): it reads only the granted
//! file, writes only the granted output, has no network and starts no program. Cases skip (and say so) on a
//! kernel without Landlock or seccomp.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cfa4_usd_probe::{Error, Worker};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_cfa4-usd-probe-launch");

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-usd-probe-conf-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn unsupported(e: &Error) -> bool {
    matches!(e, Error::Reported(m) if m.contains("cannot confine"))
}

/// A fake worker (shell) that reports what it could do.
fn probe_worker(d: &Path, secret: &Path) -> Worker {
    let p = d.join("probe.sh");
    let body = format!(
        r#"#!/bin/bash
r=ok; read -r _ < "$2" 2>/dev/null || r=granted_read_failed
s=blocked; read -r _ < '{}' 2>/dev/null && s=LEAKED
n=blocked; (exec 3<>/dev/tcp/127.0.0.1/9) 2>/dev/null && n=NETWORK
x=blocked; /usr/bin/id >/dev/null 2>&1 && x=EXEC
echo "{{\"probe\":\"$r $s $n $x\"}}"
"#,
        secret.display()
    );
    fs::write(&p, body).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    Worker::at(p).with_launcher(LAUNCHER).with_timeout(Duration::from_secs(20))
}

#[test]
fn confined_worker_cannot_read_elsewhere_use_network_or_start_programs() {
    let d = dir("probe");
    let granted = d.join("granted.usda");
    let secret = d.join("secret.txt");
    fs::write(&granted, b"x\n").unwrap();
    fs::write(&secret, b"s\n").unwrap();
    let w = probe_worker(&d, &secret);
    let probe = d.join("probe.sh");
    let out = std::process::Command::new(LAUNCHER)
        .args([
            "--read".as_ref(),
            granted.as_os_str(),
            "--exec".as_ref(),
            "/bin/bash".as_ref(),
            "--".as_ref(),
            probe.as_os_str(),
            "inspect".as_ref(),
            granted.as_os_str(),
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    if text.contains("cannot confine") {
        eprintln!("SKIP: {text}");
        return;
    }
    assert!(text.contains(r#""probe":"ok blocked blocked blocked""#), "{text}");
    drop(w);
}

#[test]
fn missing_launcher_input_is_an_error_not_a_panic() {
    let out = std::process::Command::new(LAUNCHER).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("error"));
}

/// Test files are written by an unconfined probe: USD writes through a temporary file in the output's folder,
/// which the single-file output grant does not allow (a confined exporter is U3's design, not this probe's).
fn writer() -> Option<Worker> {
    Worker::discover().ok()
}

#[test]
fn real_probe_runs_confined() {
    let Some(plain) = writer() else { return };
    let w = plain.clone().with_launcher(LAUNCHER);
    let d = dir("real");
    let files = [d.join("a.usda"), d.join("a.usdc"), d.join("a.usdz")];
    for f in &files {
        plain.write_selftest(f).unwrap();
    }
    match w.inspect(&files[0]) {
        Err(e) if unsupported(&e) => {
            eprintln!("SKIP: {e}");
            return;
        }
        r => assert_eq!(r.unwrap().prim_count, 3),
    }
    for f in &files {
        let i = w.inspect(f).unwrap();
        assert_eq!(i.prim_count, 3);
        assert_eq!(i.root_prims, ["World"]);
    }
    assert!(w.inspect(&d.join("missing.usda")).is_err());
}

/// A scene whose reference points outside the opened file's folder is not readable: the stage opens, the
/// referenced prim is missing (plan §9 point 4, U2-A1 "unresolvable"). The unconfined probe, for contrast,
/// does follow the reference.
#[test]
fn real_probe_cannot_follow_a_reference_out_of_the_folder() {
    let Some(plain) = writer() else { return };
    let w = plain.clone().with_launcher(LAUNCHER);
    let d = dir("outside");
    let (inner, outer) = (d.join("inner"), d.join("outer"));
    fs::create_dir_all(&inner).unwrap();
    fs::create_dir_all(&outer).unwrap();
    let secret = outer.join("secret.usda");
    plain.write_selftest(&secret).unwrap();
    let scene = inner.join("scene.usda");
    fs::write(&scene, format!("#usda 1.0\ndef Xform \"Own\"\n{{\n}}\ndef Xform \"Stolen\" (\n    references = @{}@</World>\n)\n{{\n}}\n", secret.display()))
        .unwrap();
    // Without confinement the reference resolves: Own, Stolen, Stolen/Cam, Stolen/Lights.
    assert_eq!(plain.inspect(&scene).unwrap().prim_count, 4);
    match w.inspect(&scene) {
        Err(e) if unsupported(&e) => eprintln!("SKIP: {e}"),
        // Confined: nothing from the secret file is composed under `Stolen`.
        r => assert_eq!(r.unwrap().prim_count, 2),
    }
}

/// SB-G6 (1) and (3) together: the hostile-file loop of `src/tests.rs`, with every inspect confined.
/// `CFA4_USD_FUZZ_ITERS_CONFINED` sets the count (default 30).
#[test]
fn real_probe_survives_hostile_files_confined() {
    let Some(plain) = writer() else { return };
    let w = plain.clone().with_launcher(LAUNCHER);
    let d = dir("hostile");
    let iters: u32 = std::env::var("CFA4_USD_FUZZ_ITERS_CONFINED").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    let mut seed = 0x1234_5678_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    let mut seeds = Vec::new();
    for name in ["s.usda", "s.usdc", "s.usdz"] {
        let f = d.join(name);
        plain.write_selftest(&f).unwrap();
        seeds.push((name, fs::read(&f).unwrap()));
    }
    for i in 0..iters {
        let (name, good) = &seeds[(i as usize) % seeds.len()];
        let mut bytes = good.clone();
        match i % 3 {
            0 => bytes.truncate((next() as usize) % bytes.len()),
            1 => {
                for _ in 0..8 {
                    let at = (next() as usize) % bytes.len();
                    bytes[at] ^= (next() >> 24) as u8 | 1;
                }
            }
            _ => {
                let at = (next() as usize) % bytes.len();
                let end = (at + 8).min(bytes.len());
                bytes[at..end].fill(0xff);
            }
        }
        let f = d.join(format!("m-{name}"));
        fs::write(&f, &bytes).unwrap();
        match w.inspect(&f) {
            Err(e) if unsupported(&e) => {
                eprintln!("SKIP: {e}");
                return;
            }
            _ => {}
        }
    }
}
