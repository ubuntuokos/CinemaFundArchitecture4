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

#[test]
fn reexport_keeps_the_old_package_and_writes_a_new_validated_one() {
    let dir = scratch("reexport");
    let (mut s, pkg) = exported(&dir);
    let root = pkg.to_string_lossy().into_owned();
    s.execute("dcp.validate", json!({"root": root, "profile": "quick-scan", "wait": true})).unwrap();
    let range = json!({"range": "custom", "startSeconds": 1.0, "endSeconds": 1.25});
    let mut params = json!({"root": root, "start": true, "wait": true});
    params.as_object_mut().unwrap().extend(range.as_object().unwrap().clone());
    let r = out(s.execute("dcp.repair.reexport", params.clone()).unwrap());

    // a new name next to the old package, never the same place
    let new = dir.join("Pkg_r2");
    assert_eq!(r["newPackage"], json!(new.to_string_lossy()), "{r}");
    assert!(new.join("ASSETMAP.xml").is_file());
    // the old package is byte-identical to the copy made first (per its manifest)
    let snap = PathBuf::from(r["snapshot"].as_str().unwrap());
    assert!(snap.starts_with(&dir) && snap.join("package/ASSETMAP.xml").is_file());
    let v = out(s.execute("dcp.manifest.verify", json!({"root": root, "manifest": snap.to_string_lossy(), "wait": true})).unwrap());
    assert_eq!(v["verification"]["identical"], true, "{v}");
    // the new package was validated, its report is next to it
    let q = out(s.execute("export.queue.list", json!({})).unwrap());
    let item = q["items"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(item["dcpValidation"]["status"], "passed", "{item}");
    assert!(PathBuf::from(r["newReport"].as_str().unwrap()).is_file());
    // the log: the snapshot, then the re-export that names the new package
    let log = out(s.execute("dcp.repair.log", json!({"path": snap.to_string_lossy()})).unwrap());
    let acts: Vec<_> = log["entries"].as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap().to_string()).collect();
    assert_eq!(acts, ["snapshot", "reexport"], "{log}");
    assert_eq!(log["entries"][1]["result"], json!(new.to_string_lossy()));
    assert_eq!(log["entries"][1]["manifest"], r["manifestId"]);
    assert!(log["entries"][1]["report_before"].is_string());

    // a second run goes to _r3, a re-export of the re-export too
    let again = out(s.execute("dcp.repair.reexport", params.clone()).unwrap());
    assert_eq!(again["newPackage"], json!(dir.join("Pkg_r3").to_string_lossy()));
    let mut from_r2 = params;
    from_r2["root"] = json!(new.to_string_lossy());
    let third = out(s.execute("dcp.repair.reexport", from_r2).unwrap());
    assert_eq!(third["newPackage"], json!(dir.join("Pkg_r4").to_string_lossy()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reexport_without_a_project_or_package_says_why_and_writes_nothing() {
    let dir = scratch("reexport-refuse");
    let (mut s, pkg) = exported(&dir);
    let root = pkg.to_string_lossy().into_owned();
    let mut empty = Session::default();
    let e = empty.execute("dcp.repair.reexport", json!({"root": root})).unwrap_err().to_string();
    assert!(e.contains("no project is open"), "{e}");
    let e = s.execute("dcp.repair.reexport", json!({"root": dir.join("nope").to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("not a folder"), "{e}");
    assert!(s.execute("dcp.repair.reexport", json!({})).is_err());
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    assert_eq!(entries.len(), 1, "only the original package: {entries:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
