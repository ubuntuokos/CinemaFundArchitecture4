//! DCP export integration: the preflight before a DCP is queued and the validation of the
//! finished package.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn scratch(name: &str) -> PathBuf {
    let d = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("dcp-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ids(r: &Value) -> Vec<String> {
    r["findings"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn preflight_names_what_blocks_and_what_only_warns() {
    let dir = scratch("preflight");
    let mut s = demo();
    let path = dir.join("Fresh").to_string_lossy().to_string();
    let r = s.execute("dcp.preflight", json!({"path": path})).unwrap();
    assert_ne!(r["verdict"], "blocked", "{r}");
    assert!(ids(&r).contains(&"DCP.PRE.006".to_string()), "no language is a warning: {r}");
    assert!(ids(&r).contains(&"DCP.PRE.009".to_string()), "the unsigned package is stated: {r}");

    // a folder that already holds a package is never written into
    // (an existing folder is the parent: the package is written to <folder>/<sequence name>)
    let busy = dir.join("Busy");
    std::fs::create_dir_all(&busy).unwrap();
    let pkg = PathBuf::from(s.execute("dcp.preflight", json!({"path": busy.to_string_lossy()})).unwrap()["path"].as_str().unwrap());
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("ASSETMAP.xml"), "x").unwrap();
    let busy = &busy;
    let r = s.execute("dcp.preflight", json!({"path": busy.to_string_lossy()})).unwrap();
    assert_eq!(r["verdict"], "blocked", "{r}");
    assert!(ids(&r).contains(&"DCP.PRE.002".to_string()), "{r}");
    let e = s.execute("export.queue.add", json!({"format": "dcp", "path": busy.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("DCP preflight") && e.contains("not empty"), "{e}");
    assert_eq!(s.execute("export.queue.list", json!({})).unwrap()["items"], json!([]), "nothing was queued");
    assert_eq!(std::fs::read_to_string(pkg.join("ASSETMAP.xml")).unwrap(), "x");

    // another format: stated, not an error
    let r = s.execute("dcp.preflight", json!({"format": "h264", "path": dir.join("a.mp4").to_string_lossy()})).unwrap();
    assert_eq!(ids(&r), vec!["DCP.PRE.000"], "{r}");
    assert_eq!(r["verdict"], "ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dcp_validate_option_is_checked() {
    let dir = scratch("option");
    let mut s = demo();
    let path = dir.join("Opt").to_string_lossy().to_string();
    for bad in [json!("no-such-profile"), json!(3)] {
        assert!(s.execute("export.queue.add", json!({"format": "dcp", "path": path, "dcpValidate": bad})).is_err());
    }
    // off, and the default for a DCP
    let r = s.execute("export.queue.add", json!({"format": "dcp", "path": path, "dcpValidate": false})).unwrap();
    assert!(r["items"][0]["dcpValidation"]["profile"].is_null(), "{r}");
    let r = s.execute("export.queue.add", json!({"format": "dcp", "path": dir.join("Opt2").to_string_lossy()})).unwrap();
    assert_eq!(r["items"][1]["dcpValidation"]["profile"], "delivery-preflight", "{r}");
    assert_eq!(r["items"][1]["dcpValidation"]["status"], "notChecked");
    assert_eq!(r["dcpPreflight"].as_array().unwrap().len(), 1);
    // a file export never gets one
    let r = s.execute("export.queue.add", json!({"format": "h264", "path": dir.join("a.mp4").to_string_lossy(), "dcpValidate": "quick-scan"})).unwrap();
    assert!(r["items"][2]["dcpValidation"]["profile"].is_null(), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The package is exported, validated in the background and the result lands on the queue item;
/// the report is written next to the package, never inside it.
#[test]
fn exported_package_is_validated_after_the_export() {
    let dir = scratch("postflight");
    let mut s = demo();
    let path = dir.join("Pkg");
    let r = s
        .execute(
            "export.queue.add",
            json!({"format": "dcp", "path": path.to_string_lossy(), "range": "custom", "startSeconds": 1.0, "endSeconds": 1.25,
                   "dcpValidate": "quick-scan", "start": true, "wait": true}),
        )
        .unwrap();
    let id = r["added"][0].as_u64().unwrap();
    for _ in 0..200 {
        s.poll_persistence();
        let l = s.execute("export.queue.list", json!({})).unwrap();
        if !matches!(l["items"][0]["dcpValidation"]["status"].as_str(), Some("running" | "notChecked")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let l = s.execute("export.queue.list", json!({})).unwrap();
    let item = &l["items"][0];
    assert_eq!(item["id"], id);
    assert_eq!(item["status"], "done", "{item}");
    assert_eq!(item["dcpValidation"]["status"], "passed", "{item}");
    let report = PathBuf::from(item["dcpValidation"]["report"].as_str().unwrap());
    assert_eq!(report, dir.join("Pkg.cfa4dcp.json"));
    assert!(report.is_file() && !path.join("Pkg.cfa4dcp.json").exists());
    let text = std::fs::read_to_string(&report).unwrap();
    assert!(text.contains("cfa4-dcp-report/1"));

    // a package that lost a track file fails a new validation
    let victim = std::fs::read_dir(&path).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("pcm_")).unwrap().path();
    std::fs::remove_file(&victim).unwrap();
    let v = s.execute("dcp.validate", json!({"root": path.to_string_lossy(), "profile": "quick-scan", "wait": true})).unwrap();
    assert_eq!(v["report"]["verdict"], "FAIL", "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}
