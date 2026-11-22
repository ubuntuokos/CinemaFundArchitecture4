//! `vdb.inspect`: parameters, the "no worker" message, and (when a built worker is found through
//! `CFA4_VDB_WORKER`) the real answer on a synthetic file.

use serde_json::json;

use super::*;

#[test]
fn inspect_requires_a_path() {
    let mut s = Session::default();
    assert!(matches!(s.execute("vdb.inspect", json!({})), Err(EngineError::BadParams { .. })));
}

#[test]
fn inspect_without_or_with_worker() {
    let mut s = Session::default();
    match cfa4_vdb::Worker::discover() {
        Err(_) => {
            let e = s.execute("vdb.inspect", json!({"path": "/x.vdb"})).unwrap_err();
            assert!(e.to_string().contains("not available"), "{e}");
        }
        Ok(w) => {
            let f = std::env::temp_dir().join(format!("cfa4-engine-vdb-{}.vdb", std::process::id()));
            w.write_selftest(&f).unwrap();
            let r = s.execute("vdb.inspect", json!({"path": f.to_str().unwrap()})).unwrap();
            assert_eq!(r["inspection"]["grids"].as_array().unwrap().len(), 2);
            // a worker error is an error, not a panic
            assert!(s.execute("vdb.inspect", json!({"path": "/nonexistent.vdb"})).is_err());
            let _ = std::fs::remove_file(f);
        }
    }
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-engine-vdb-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Placeholder files: import reads only names, never the grids (those go through the worker).
fn touch(d: &std::path::Path, names: &[&str]) {
    for n in names {
        std::fs::write(d.join(n), b"placeholder").unwrap();
    }
}

fn path(d: &std::path::Path, n: &str) -> String {
    d.join(n).to_string_lossy().to_string()
}

#[test]
fn a_numbered_sequence_is_one_volume_item_and_makes_no_timeline() {
    let d = dir("seq");
    touch(&d, &["smoke.0001.vdb", "smoke.0002.vdb", "smoke.0004.vdb"]);
    let mut s = Session::default();
    let seqs_before = s.project.items.values().filter(|i| matches!(i.kind, cfa4_project::ItemKind::Sequence(_))).count();
    let r = s.execute("vdb.import", json!({"paths": [path(&d, "smoke.0001.vdb")]})).unwrap();
    assert_eq!(r["items"][0]["frames"], 4);
    assert_eq!(r["items"][0]["missing"], json!([2]), "frame 0003 is a gap");
    let seqs_after = s.project.items.values().filter(|i| matches!(i.kind, cfa4_project::ItemKind::Sequence(_))).count();
    assert_eq!(seqs_before, seqs_after, "no new timeline");
    let id = r["items"][0]["item"].as_u64().unwrap();
    let it = s.project.item(cfa4_project::ItemId(id)).unwrap();
    assert_eq!(it.type_label(), "Volume Sequence (OpenVDB)");
    assert!(!it.has_video() && !it.has_audio());
    // one undo step removes it
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.item(cfa4_project::ItemId(id)).is_none());
}

#[test]
fn a_single_file_and_forced_single_file_are_plain_volumes() {
    let d = dir("single");
    touch(&d, &["a.vdb", "b.0001.vdb", "b.0002.vdb"]);
    let mut s = Session::default();
    let r = s.execute("vdb.import", json!({"paths": [path(&d, "a.vdb")]})).unwrap();
    assert_eq!(r["items"][0]["frames"], 0);
    let r = s.execute("vdb.import", json!({"paths": [path(&d, "b.0001.vdb")], "sequence": false})).unwrap();
    assert_eq!(r["items"][0]["frames"], 0);
    assert!(s.execute("vdb.import", json!({"paths": [path(&d, "nope.vdb")]})).is_err());
    assert!(s.execute("vdb.import", json!({"paths": [path(&d, "a.vdb").replace(".vdb", ".exr")]})).is_err());
}

#[test]
fn project_round_trip_and_relink_report() {
    let d = dir("project");
    touch(&d, &["fx.0001.vdb", "fx.0002.vdb"]);
    let mut s = Session::default();
    let r = s.execute("vdb.import", json!({"paths": [path(&d, "fx.0001.vdb")]})).unwrap();
    let id = r["items"][0]["item"].as_u64().unwrap();
    let proj = path(&d, "p.fcproj");
    s.execute("file.saveAs", json!({"path": proj})).unwrap();
    let mut t = Session::default();
    t.execute("file.open", json!({"path": proj})).unwrap();
    assert_eq!(*t.project, *s.project, "the volume reference survives save and open");
    assert_eq!(t.execute("vdb.check", json!({"item": id})).unwrap()["online"], true);
    // a frame goes missing: reported by name, nothing panics
    std::fs::remove_file(d.join("fx.0002.vdb")).unwrap();
    let c = t.execute("vdb.check", json!({"item": id})).unwrap();
    assert_eq!(c["online"], false);
    assert!(c["missing"][0].as_str().unwrap().ends_with("fx.0002.vdb"));
    // relink to another place; one undo step
    let d2 = dir("project-moved");
    touch(&d2, &["fx.0001.vdb", "fx.0002.vdb"]);
    t.execute("vdb.relink", json!({"item": id, "path": path(&d2, "fx.0001.vdb")})).unwrap();
    assert_eq!(t.execute("vdb.check", json!({"item": id})).unwrap()["online"], true);
    t.execute("edit.undo", json!({})).unwrap();
    assert_eq!(t.execute("vdb.check", json!({"item": id})).unwrap()["online"], false);
}

#[test]
fn a_volume_preview_is_a_plain_still_the_timeline_takes() {
    // needs a built worker; returns early without one
    let Ok(w) = cfa4_vdb::Worker::discover() else { return };
    let d = dir("render");
    let vdb = d.join("t.vdb");
    w.write_selftest(&vdb).unwrap();
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "v", "audio": 1, "video": 1})).unwrap();
    let r = s.execute("vdb.import", json!({"paths": [vdb.to_string_lossy()]})).unwrap();
    let vol = r["items"][0]["item"].as_u64().unwrap();
    // a bad quality or size is refused before any worker runs
    assert!(s.execute("vdb.render", json!({"item": vol, "quality": "ultra"})).is_err());
    assert!(s.execute("vdb.render", json!({"item": vol, "width": 0})).is_err());
    assert!(s.execute("vdb.render", json!({"item": vol, "frame": 3})).is_err());
    let out = d.join("p.png");
    let r = s.execute("vdb.render", json!({"item": vol, "width": 160, "height": 90, "quality": "preview", "out": out.to_string_lossy()})).unwrap();
    let still = r["items"][0].as_u64().expect("the picture is imported as an item");
    let it = s.project.item(cfa4_project::ItemId(still)).unwrap();
    assert_eq!(it.type_label(), "Still Image");
    assert_eq!(s.project.source_size(cfa4_project::ItemId(still)), Some((160, 90)));
    // placed as an ordinary clip: no new render path
    s.execute("timeline.place", json!({"item": still, "videoTrack": "V1", "seconds": 0.0})).unwrap();
    assert_eq!(r["info"]["grid"], "density");
}
