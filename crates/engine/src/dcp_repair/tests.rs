//! `dcp.snapshot`, `dcp.manifest.verify`, `dcp.repair.job` and `dcp.repair.log` on a package written by
//! CFA4's own DCP export.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;

fn scratch(name: &str) -> PathBuf {
    let d = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("dcp-repair-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn exported(dir: &std::path::Path) -> (Session, PathBuf) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let path = dir.join("Pkg");
    s.execute(
        "export.queue.add",
        json!({"format": "dcp", "path": path.to_string_lossy(), "range": "custom", "startSeconds": 1.0, "endSeconds": 1.25,
               "dcpValidate": false, "start": true, "wait": true}),
    )
    .unwrap();
    (s, path)
}

fn out(v: Value) -> Value {
    v.get("result").cloned().unwrap_or(v)
}

#[test]
fn snapshot_copies_verifies_and_logs_without_touching_the_package() {
    let dir = scratch("ok");
    let (mut s, pkg) = exported(&dir);
    let root = pkg.to_string_lossy().into_owned();
    // a report to name in the log
    s.execute("dcp.validate", json!({"root": root, "profile": "quick-scan", "wait": true})).unwrap();
    let dest = dir.join("backup");
    let r = out(s.execute("dcp.snapshot", json!({"root": root, "dest": dest.to_string_lossy(), "wait": true})).unwrap());
    assert!(r["files"].as_u64().unwrap() >= 4 && r["bytes"].as_u64().unwrap() > 0, "{r}");
    assert!(dest.join("manifest.json").is_file() && dest.join("package/ASSETMAP.xml").is_file());

    // the package is the one the manifest names
    let v = out(s.execute("dcp.manifest.verify", json!({"root": root, "manifest": dest.to_string_lossy(), "wait": true})).unwrap());
    assert_eq!(v["verification"]["identical"], true, "{v}");
    assert_eq!(v["manifestId"], r["manifestId"]);
    // the copy is a package of its own: it verifies against the same manifest
    let c = out(s
        .execute(
            "dcp.manifest.verify",
            json!({"root": dest.join("package").to_string_lossy(), "manifest": dest.join("manifest.json").to_string_lossy(), "wait": true}),
        )
        .unwrap());
    assert_eq!(c["verification"]["identical"], true, "{c}");

    // the log holds the snapshot entry, tied to the manifest and to the report of that moment
    let log = out(s.execute("dcp.repair.log", json!({"path": dest.to_string_lossy()})).unwrap());
    let e = &log["entries"][0];
    assert_eq!((log["entries"].as_array().unwrap().len(), e["action"].as_str()), (1, Some("snapshot")));
    assert_eq!(e["manifest"], r["manifestId"]);
    assert!(e["report_before"].as_str().is_some_and(|h| h.len() == 64), "{e}");

    // a changed package no longer verifies
    let victim = std::fs::read_dir(&pkg).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("pcm_")).unwrap().path();
    std::fs::remove_file(&victim).unwrap();
    let v = out(s.execute("dcp.manifest.verify", json!({"root": root, "manifest": dest.to_string_lossy(), "wait": true})).unwrap());
    assert_eq!(v["verification"]["identical"], false);
    assert_eq!(v["verification"]["differences"][0]["state"], "missing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_refuses_the_package_itself_and_existing_folders() {
    let dir = scratch("refuse");
    let (mut s, pkg) = exported(&dir);
    let root = pkg.to_string_lossy().into_owned();
    let inside = s.execute("dcp.snapshot", json!({"root": root, "dest": pkg.join("copy").to_string_lossy(), "wait": true})).unwrap_err().to_string();
    assert!(inside.contains("inside the package"), "{inside}");
    assert!(!pkg.join("copy").exists());
    let exists = s.execute("dcp.snapshot", json!({"root": root, "dest": dir.to_string_lossy(), "wait": true})).unwrap_err().to_string();
    assert!(exists.contains("already exists"), "{exists}");
    assert!(s.execute("dcp.snapshot", json!({"root": root})).is_err());
    assert!(s.execute("dcp.manifest.verify", json!({"root": root, "manifest": dir.join("nope").to_string_lossy()})).is_err());
    assert!(s.execute("dcp.repair.job", json!({"job": 99999})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_background_snapshot_is_read_back_with_its_job_id() {
    let dir = scratch("job");
    let (mut s, pkg) = exported(&dir);
    let dest = dir.join("backup");
    let r = out(s.execute("dcp.snapshot", json!({"root": pkg.to_string_lossy(), "dest": dest.to_string_lossy()})).unwrap());
    let job = r["job"].as_u64().unwrap();
    let mut done = Value::Null;
    for _ in 0..200 {
        done = out(s.execute("dcp.repair.job", json!({"job": job})).unwrap());
        if done["finished"] != json!(false) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(done["manifestId"].is_string(), "{done}");
    let _ = std::fs::remove_dir_all(&dir);
}
