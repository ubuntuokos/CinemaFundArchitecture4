//! The adapter against a fake engine: a shell script that prints the pinned version and writes a
//! canned `--dump-result` document.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cfa4_dcp_validate::rules::*;
use cfa4_dcp_validate::{DcpValidator, Profile, Status, ValidationControl, ValidationError, ValidationReport, ValidationRequest, Verdict};

use super::*;

const AM: &str = "321f4254-da12-4ba3-99db-f34ccf88f0cf";
const PKL: &str = "e6a93e68-486f-48ca-82dc-1aacc31dbad3";
const CPL: &str = "3809613a-1f51-44be-a79c-9f6aea317387";
const PIC: &str = "f8c4e040-f8de-444a-9956-f4790bf680c1";

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-inspect-test-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("dcp")).unwrap();
    d.canonicalize().unwrap()
}

/// A fake engine in `d`: `--version` prints the pinned version; a run records its arguments and
/// environment in `d`, writes `result` (if any) to the `--dump-result` path and runs `tail`.
fn engine(d: &Path, result: Option<&str>, tail: &str) -> DcpInspect {
    if let Some(r) = result {
        std::fs::write(d.join("result.json"), r).unwrap();
    }
    let copy = if result.is_some() { format!("cat '{}/result.json' > \"$out\"", d.display()) } else { String::new() };
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "dcp_inspect v{ENGINE_VERSION}"; exit 0; fi
printf '%s\n' "$@" > '{d}/args'
printf 'HOME=%s\nTMPDIR=%s\nTERM=%s\nPWD=%s\n' "$HOME" "$TMPDIR" "$TERM" "$PWD" > '{d}/env'
env | cut -d= -f1 | sort > '{d}/envnames'
out=""; prev=""
for a in "$@"; do if [ "$prev" = "--dump-result" ]; then out="$a"; fi; prev="$a"; done
{copy}
{tail}
"#,
        d = d.display()
    );
    let p = d.join("engine");
    std::fs::write(&p, script).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    DcpInspect::new(p)
}

/// A result shaped like the engine's on a CFA4-exported SMPTE package: schema checks pass, the
/// JPEG 2000 check fails, an error no rule covers, a hint.
fn result(root: &Path) -> String {
    let r = root.display();
    format!(
        r#"{{
  "errors": ["CPL {CPL}: Reel 1: MainPicture {PIC}: jpeg2000: QCD guard bits do not match cinema resolution", "CPL {CPL}: Reel 1: Reel duration less than 1 second (0.200 seconds)"],
  "hints": ["CPL {CPL}: Composition MainSound is shorter than the 400 ms integrated-loudness gate"],
  "siginfo": [], "info": ["Hash value: OK: {PIC}: {r}/j2c.mxf"],
  "am_files": ["ASSETMAP.xml"], "pkls": ["{PKL}"], "cpls": ["{CPL}"], "pkls_missing": [], "cpls_missing": [],
  "inspection_run": {{
    "root_path": "{r}",
    "assetmaps": [{{"id": "{AM}", "path": "ASSETMAP.xml", "absolute_path": "{r}/ASSETMAP.xml", "present": true,
      "namespace": "http://www.smpte-ra.org/schemas/429-9/2007/AM", "checks": [{{"kind": "schema", "status": "ok", "message": "AM candidate {AM}", "details": {{}}}}]}}],
    "packing_lists": [{{"id": "{PKL}", "path": "pkl.xml", "present": true, "namespace": null, "checks": [
      {{"kind": "schema", "status": "ok", "message": "PKL {PKL}", "details": {{}}}},
      {{"kind": "signature", "status": "info", "message": "No Signature node found", "details": {{}}}}]}}],
    "compositions": [{{"id": "{CPL}", "path": "cpl.xml", "present": true, "namespace": "http://www.smpte-ra.org/schemas/429-7/2006/CPL", "checks": [
      {{"kind": "schema", "status": "ok", "message": "CPL {CPL}", "details": {{}}}},
      {{"kind": "jpeg2000", "status": "error", "message": "CPL {CPL}: Reel 1: MainPicture {PIC}: 5/5 codestream headers checked", "details": {{}}}},
      {{"kind": "markers", "status": "ok", "message": "Composition markers checked", "details": {{}}}},
      {{"kind": "composition_audio", "status": "skipped", "message": "shorter than the loudness gate", "details": {{}}}},
      {{"kind": "composition", "status": "error", "message": "Composition validation failed: CPL {CPL}", "details": {{}}}}]}}],
    "assets": [{{"id": "{PIC}", "assetmap_path": "j2c.mxf", "absolute_path": "{r}/j2c.mxf", "present": true, "checks": [
      {{"kind": "hash", "status": "ok", "message": "Hash value: OK: {PIC}: {r}/j2c.mxf", "details": {{}}}}]}}],
    "events": []
  }}
}}"#
    )
}

fn run(e: &DcpInspect, root: &Path, profile: &str) -> Result<ValidationReport, ValidationError> {
    e.validate(&ValidationRequest::new(root, Profile::builtin(profile).unwrap()), &ValidationControl::new())
}

fn statuses(r: &ValidationReport, rule: &str) -> Vec<Status> {
    r.findings.iter().filter(|f| f.rule == rule).map(|f| f.status).collect()
}

#[test]
fn version_and_capabilities() {
    let d = dir("version");
    let e = engine(&d, None, "exit 0");
    assert_eq!(e.version().unwrap(), ENGINE_VERSION);
    let c = e.capabilities();
    assert_eq!((c.id.as_str(), c.version.as_str()), (BACKEND_ID, ENGINE_VERSION));
    assert!(c.rules.iter().any(|r| r == REF_ENGINE));
    // every reported rule is in the catalogue
    assert!(c.rules.iter().all(|r| rules::find(r).is_some()), "{:?}", c.rules);
    let missing = DcpInspect::new(d.join("nope"));
    assert_eq!(missing.capabilities().version, "unavailable");
    assert!(matches!(run(&missing, &d.join("dcp"), "quick-scan"), Err(ValidationError::Unavailable(_))));
}

#[test]
fn engine_result_maps_onto_the_rules() {
    let d = dir("map");
    let root = d.join("dcp");
    let e = engine(&d, Some(&result(&root)), "exit 1");
    let r = run(&e, &root, "delivery-preflight").unwrap();
    assert_eq!(r.backend.id, BACKEND_ID);
    assert_eq!(r.backend.version, ENGINE_VERSION);
    assert_eq!(r.standard, Some(cfa4_dcp::Standard::Smpte));
    assert_eq!(r.documents.compositions.len(), 1);
    assert_eq!(r.documents.packing_lists.len(), 1);
    assert!(r.documents.asset_map.is_some());
    assert_eq!(r.assets_checked, 1);
    assert_eq!(statuses(&r, AM_PRESENT), [Status::Pass]);
    assert_eq!(statuses(&r, AM_DOCUMENT), [Status::Pass]);
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Pass]);
    assert_eq!(statuses(&r, CPL_DOCUMENT), [Status::Pass]);
    assert_eq!(statuses(&r, J2K_CODESTREAM), [Status::Fail]);
    assert_eq!(statuses(&r, MARK_MARKERS), [Status::Pass]);
    assert_eq!(statuses(&r, AUD_LOUDNESS), [Status::Skipped]);
    assert_eq!(statuses(&r, SIG_SIGNATURE), [Status::NotApplicable]);
    assert_eq!(statuses(&r, PKL_HASH), [Status::Pass]);
    // a profile rule the engine says nothing about
    assert_eq!(statuses(&r, AM_PATH_SAFE), [Status::Unchecked]);
    // the composition check, both errors and the hint are kept
    let refs = statuses(&r, REF_ENGINE);
    assert_eq!(refs.iter().filter(|s| **s == Status::Fail).count(), 3, "{refs:?}");
    assert_eq!(refs.iter().filter(|s| **s == Status::Warning).count(), 1, "{refs:?}");
    assert_eq!(r.verdict, Verdict::Fail);
    // package-relative paths only
    let root_text = root.to_string_lossy();
    assert!(r.findings.iter().all(|f| !f.message.contains(&*root_text)));
    let hash = r.findings.iter().find(|f| f.rule == PKL_HASH).unwrap();
    assert_eq!(hash.location.file.as_deref(), Some("j2c.mxf"));
    assert!(hash.message.ends_with(": j2c.mxf"), "{}", hash.message);
    // the report reads back
    assert_eq!(ValidationReport::from_json(&r.to_json()).unwrap(), r);
}

#[test]
fn an_unmapped_engine_error_fails_the_package() {
    let d = dir("ref-only");
    let root = d.join("dcp");
    let text = result(&root).replace(r#""status": "error""#, r#""status": "ok""#);
    let r = run(&engine(&d, Some(&text), "exit 1"), &root, "quick-scan").unwrap();
    assert!(r.findings.iter().filter(|f| f.rule != REF_ENGINE).all(|f| f.status != Status::Fail));
    assert_eq!(r.verdict, Verdict::Fail, "the engine's errors fail the package");
    // without errors the profile decides; Quick Scan's path rule is not reported by the engine
    let clean = text.replace(r#""errors": ["#, r#""errors": [], "x": ["#);
    let r = run(&engine(&d, Some(&clean), "exit 0"), &root, "quick-scan").unwrap();
    assert_eq!(r.verdict, Verdict::Incomplete);
    assert_eq!(statuses(&r, AM_PATH_SAFE), [Status::Unchecked]);
}

#[test]
fn profile_selects_the_engine_options() {
    let d = dir("args");
    let root = d.join("dcp");
    let e = engine(&d, Some(&result(&root)), "exit 1");
    let args = || std::fs::read_to_string(d.join("args")).unwrap();
    run(&e, &root, "quick-scan").unwrap();
    let a = args();
    assert!(a.contains("--no-hash\n") && a.contains("--no-audio-analysis\n"), "{a}");
    assert!(a.ends_with(&format!("{}\n", root.display())), "the package is the last argument: {a}");
    run(&e, &root, "full-integrity").unwrap();
    assert!(!args().contains("--no-hash"));
}

#[test]
fn engine_runs_in_a_private_directory() {
    let d = dir("env");
    let root = d.join("dcp");
    run(&engine(&d, Some(&result(&root)), "exit 0"), &root, "quick-scan").unwrap();
    let env = std::fs::read_to_string(d.join("env")).unwrap();
    let get = |k: &str| env.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap().to_string();
    let scratch = PathBuf::from(get("PWD"));
    assert!(scratch.file_name().unwrap().to_string_lossy().starts_with("cfa4-dcp-inspect-"));
    assert_eq!(PathBuf::from(get("HOME")), scratch.join("home"));
    assert_eq!(PathBuf::from(get("TMPDIR")), scratch.join("tmp"));
    assert_eq!(get("TERM"), "dumb");
    assert!(!scratch.exists(), "the private directory is removed");
    // nothing else is inherited
    let names = std::fs::read_to_string(d.join("envnames")).unwrap();
    for n in names.lines() {
        assert!(["PATH", "LANG", "LC_ALL", "HOME", "TMPDIR", "TERM", "COLUMNS", "PWD", "SHLVL", "_", "OLDPWD"].contains(&n), "{n} was inherited");
    }
}

#[test]
fn exit_codes() {
    let d = dir("exit");
    let root = d.join("dcp");
    let err = |tail: &str, result: Option<&str>| run(&engine(&d, result, tail), &root, "quick-scan");
    assert!(
        matches!(err("echo 'Required command not found: asdcp-info' >&2; exit 12", None), Err(ValidationError::Unavailable(m)) if m.contains("asdcp-info"))
    );
    assert!(matches!(err("exit 11", None), Err(ValidationError::Unavailable(_))));
    assert!(matches!(err("exit 21", None), Err(ValidationError::Backend(m)) if m.contains("21")));
    assert!(matches!(err("exit 0", None), Err(ValidationError::Backend(m)) if m.contains("no result")));
    assert!(matches!(err("exit 1", Some("{\"errors\": 3}")), Err(ValidationError::Backend(m)) if m.contains(ENGINE_VERSION)));
    assert!(matches!(err("exit 4", None), Err(ValidationError::NotADirectory(_))));
    let r = err("exit 18", None).unwrap();
    assert_eq!(r.verdict, Verdict::Interrupted);
    assert!(r.findings.iter().all(|f| f.status == Status::Interrupted));
    assert!(matches!(run(&engine(&d, None, "exit 0"), &d.join("missing"), "quick-scan"), Err(ValidationError::NotADirectory(_))));
}

#[test]
fn cancel_stops_the_process_group() {
    let d = dir("cancel");
    let root = d.join("dcp");
    // a child process in the group that would outlive a plain kill of the shell
    let e = engine(&d, None, &format!("sleep 30 & echo $! > '{}/child'; wait", d.display()));
    let control = ValidationControl::new();
    let flag = control.cancel_flag();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let start = Instant::now();
    let r = e.validate(&ValidationRequest::new(&root, Profile::builtin("quick-scan").unwrap()), &control).unwrap();
    t.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(10));
    assert_eq!(r.verdict, Verdict::Interrupted);
    let child: i32 = std::fs::read_to_string(d.join("child")).unwrap().trim().parse().unwrap();
    // the background sleep is gone as well (give the kernel a moment to reap it)
    let gone = (0..40).any(|_| {
        std::thread::sleep(Duration::from_millis(50));
        !alive(child)
    });
    assert!(gone, "child {child} survived the cancel");
}

#[test]
fn timeout_kills_the_engine() {
    let d = dir("timeout");
    let root = d.join("dcp");
    let e = engine(&d, None, "sleep 30").with_timeout(Duration::from_millis(300));
    let start = Instant::now();
    assert!(matches!(run(&e, &root, "quick-scan"), Err(ValidationError::Backend(m)) if m.contains("did not finish")));
    assert!(start.elapsed() < Duration::from_secs(10));
}

/// A process that exists and is not a zombie.
fn alive(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.split(") ").nth(1).is_some_and(|rest| rest.starts_with('Z')))
}
