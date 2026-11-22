//! N1-8: the worker runs confined (Landlock + seccomp through `cfa4-vdb-launch`): it reads only the granted
//! file, writes only the granted output, has no network and starts no program. Cases skip (and say so) on a
//! kernel without Landlock or seccomp.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cfa4_vdb::{Error, Worker};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_cfa4-vdb-launch");

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-vdb-conf-{}-{name}", std::process::id()));
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
echo "{{\"grids\":[],\"fileMetaCount\":0,\"probe\":\"$r $s $n $x\"}}"
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
    let granted = d.join("granted.vdb");
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

#[test]
fn real_worker_runs_confined() {
    let Ok(w) = Worker::discover() else { return };
    let w = w.with_launcher(LAUNCHER);
    let d = dir("real");
    let (a, b) = (d.join("a.vdb"), d.join("b.vdb"));
    match w.write_selftest(&a) {
        Err(e) if unsupported(&e) => {
            eprintln!("SKIP: {e}");
            return;
        }
        r => r.unwrap(),
    }
    let ia = w.inspect(&a).unwrap();
    assert_eq!(ia.grids.len(), 2);
    w.copy(&a, &b).unwrap();
    assert_eq!(ia, w.inspect(&b).unwrap());
    assert!(w.inspect(&d.join("missing.vdb")).is_err());
}
