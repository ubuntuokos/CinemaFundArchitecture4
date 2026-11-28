//! `dcp.package.info` on a package written by CFA4's own DCP export, and on things that are no DCP.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;

fn scratch(name: &str) -> PathBuf {
    let d = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("dcp-package-{name}-{}", std::process::id()));
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

fn files(root: &std::path::Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut v: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| {
            let m = e.metadata().unwrap();
            (e.file_name().to_string_lossy().into_owned(), m.len(), m.modified().unwrap())
        })
        .collect();
    v.sort();
    v
}

#[test]
fn exported_package_shows_its_structure_and_stays_unchanged() {
    let dir = scratch("ok");
    let (mut s, path) = exported(&dir);
    let before = files(&path);
    let r = s.execute("dcp.package.info", json!({"root": path.to_string_lossy()})).unwrap();
    let r = r.get("result").cloned().unwrap_or(r);
    assert_eq!(r["schema"], "cfa4-dcp-package/1", "{r}");
    assert_eq!(r["standard"], "Smpte");
    assert_eq!(r["assetMap"]["file"], "ASSETMAP.xml");
    let list = r["files"].as_array().unwrap();
    assert!(list.len() >= 4 && list.iter().all(|f| f["state"] == "file" && f["size"].as_u64().unwrap() > 0), "{r}");
    assert_eq!(r["packingLists"].as_array().unwrap().len(), 1);
    assert!(r["packingLists"][0]["assets"].as_array().unwrap().iter().all(|a| a["inAssetMap"] == true));
    let cpls = r["compositions"].as_array().unwrap();
    assert_eq!(cpls.len(), 1, "{r}");
    let reel = &cpls[0]["document"]["reels"][0];
    assert!(reel["assets"].as_array().unwrap().iter().any(|a| a["track"] == true && a["file"].is_string()), "{reel}");
    assert!(r["security"]["encryptedAssets"].as_array().unwrap().is_empty());
    assert_eq!(before, files(&path), "reading the structure must not change the package");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_lost_track_file_is_marked_missing() {
    let dir = scratch("missing");
    let (mut s, path) = exported(&dir);
    let victim = std::fs::read_dir(&path).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("pcm_")).unwrap().path();
    std::fs::remove_file(&victim).unwrap();
    let r = s.execute("dcp.package.info", json!({"root": path.to_string_lossy()})).unwrap();
    let r = r.get("result").cloned().unwrap_or(r);
    let missing: Vec<&Value> = r["files"].as_array().unwrap().iter().filter(|f| f["state"] == "missing").collect();
    assert_eq!(missing.len(), 1, "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn things_that_are_no_dcp_are_refused_with_a_message() {
    let dir = scratch("bad");
    let mut s = Session::default();
    let e = s.execute("dcp.package.info", json!({"root": dir.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("not a DCP folder"), "{e}");
    assert!(s.execute("dcp.package.info", json!({"root": dir.join("nope").to_string_lossy()})).is_err());
    assert!(s.execute("dcp.package.info", json!({})).is_err());
    std::fs::write(dir.join("ASSETMAP.xml"), "<not xml").unwrap();
    assert!(s.execute("dcp.package.info", json!({"root": dir.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
