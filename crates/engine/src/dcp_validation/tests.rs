//! DCP validation commands: profiles, a validation run on a (broken) package, the report kept in
//! the session and its saving outside the package.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::Session;

#[derive(Default)]
struct MemoryFiles(Mutex<BTreeMap<String, Vec<u8>>>);

impl crate::Services for MemoryFiles {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.0.lock().unwrap().get(path).cloned().ok_or_else(|| std::io::ErrorKind::NotFound.into())
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.0.lock().unwrap().insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }
}

fn session() -> (Session, Arc<MemoryFiles>) {
    let files = Arc::new(MemoryFiles::default());
    (Session::new(files.clone()), files)
}

fn result(v: Value) -> Value {
    v.get("result").cloned().unwrap_or(v)
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cfa4-dcp-validation-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn profiles_lists_the_builtin_profiles_with_their_rules() {
    let (mut s, _) = session();
    let r = result(s.execute("dcp.validate.profiles", json!({})).unwrap());
    let list = r["profiles"].as_array().unwrap();
    assert_eq!(list.len(), cfa4_dcp_validate::profile::BUILTIN.len());
    let quick = list.iter().find(|p| p["id"] == "quick-scan").unwrap();
    assert_eq!(quick["title"], "Quick Scan");
    assert!(quick["required"].as_u64().unwrap() > 0 && quick["rules"][0]["rule"].as_str().unwrap().starts_with("DCP."));
}

#[test]
fn broken_package_fails_and_its_report_is_saved_outside_the_package() {
    let (mut s, files) = session();
    let dir = temp("empty");
    let root = dir.to_string_lossy().into_owned();
    let r = result(s.execute("dcp.validate", json!({"root": root, "profile": "quick-scan", "wait": true})).unwrap());
    assert_eq!(r["report"]["verdict"], "FAIL", "{r}");
    assert_eq!(r["report"]["schema"], "cfa4-dcp-report/1");
    assert!(r["report"]["findings"].as_array().unwrap().iter().any(|f| f["rule"] == "DCP.AM.001" && f["status"] == "FAIL"));

    let got = result(s.execute("dcp.validate.report", json!({"root": root})).unwrap());
    assert_eq!(got["verdict"], "FAIL");
    assert!(got["markdown"].as_str().unwrap().contains("DCP.AM.001"));

    let out = result(s.execute("dcp.validate.save", json!({"root": root, "path": "/reports/a.md"})).unwrap());
    assert_eq!(out["path"], "/reports/a.md");
    let md = String::from_utf8(files.0.lock().unwrap()["/reports/a.md"].clone()).unwrap();
    assert!(md.contains("DCP.AM.001"));
    s.execute("dcp.validate.save", json!({"root": root, "path": "/reports/a.json", "format": "json"})).unwrap();
    let json_text = String::from_utf8(files.0.lock().unwrap()["/reports/a.json"].clone()).unwrap();
    assert!(json_text.contains("cfa4-dcp-report/1"));

    // the DCP is only read: a report is never written into it
    let inside = dir.join("report.json").to_string_lossy().into_owned();
    let err = s.execute("dcp.validate.save", json!({"root": root, "path": inside})).unwrap_err().to_string();
    assert!(err.contains("inside the package"), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refusals_name_the_problem() {
    let (mut s, _) = session();
    let err = |s: &mut Session, p: Value| s.execute("dcp.validate", p).unwrap_err().to_string();
    assert!(err(&mut s, json!({})).contains("need `root`"));
    assert!(err(&mut s, json!({"root": "/no/such/folder/cfa4"})).contains("not a folder"));
    let dir = temp("refuse");
    let root = dir.to_string_lossy().into_owned();
    let e = err(&mut s, json!({"root": root, "profile": "nope"}));
    assert!(e.contains("no profile `nope`") && e.contains("quick-scan"), "{e}");
    let e = s.execute("dcp.validate.report", json!({"root": root})).unwrap_err().to_string();
    assert!(e.contains("run dcp.validate first"), "{e}");
    let e = s.execute("dcp.validate.save", json!({"root": root, "path": "/r/x", "format": "pdf"})).unwrap_err().to_string();
    assert!(e.contains("no report") || e.contains("`json` or `md`"), "{e}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn background_validation_is_filed_by_poll() {
    let (mut s, _) = session();
    let dir = temp("bg");
    let root = dir.to_string_lossy().into_owned();
    let started = result(s.execute("dcp.validate", json!({"root": root})).unwrap());
    assert!(started["job"].is_u64() && started.get("report").is_none());
    for _ in 0..200 {
        s.poll_persistence();
        if s.dcp_validation.pending.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(s.dcp_validation.pending.is_empty(), "the job did not finish");
    let got = result(s.execute("dcp.validate.report", json!({"root": root})).unwrap());
    assert_eq!(got["verdict"], "FAIL");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn repair_plan_sorts_the_findings_and_changes_nothing() {
    let (mut s, _) = session();
    let dir = temp("repair");
    let root = dir.to_string_lossy().into_owned();
    assert!(s.execute("dcp.repair.plan", json!({"root": root})).is_err(), "needs a report first");
    s.execute("dcp.validate", json!({"root": root, "profile": "quick-scan", "wait": true})).unwrap();
    let r = result(s.execute("dcp.repair.plan", json!({"root": root})).unwrap());
    assert_eq!(r["plan"]["schema"], "cfa4-dcp-repair/1");
    assert_eq!(r["plan"]["modifiesPackage"], false);
    let first = &r["plan"]["proposals"][0];
    assert_eq!(first["status"], "FAIL", "{r}");
    assert_eq!(first["class"], "reexport");
    assert!(r["counts"]["reexport"].as_u64().unwrap() >= 1 && r["counts"]["unsupported"].is_u64(), "{r}");
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "the package folder is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
