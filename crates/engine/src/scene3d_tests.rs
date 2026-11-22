use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

const TRIANGLE_OBJ: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// An `n`×`n` quad grid as OBJ text (2n² triangles).
fn grid_obj(n: u32) -> String {
    let mut s = String::new();
    for y in 0..=n {
        for x in 0..=n {
            s.push_str(&format!("v {x} {y} 0\n"));
        }
    }
    let w = n + 1;
    for y in 0..n {
        for x in 0..n {
            let a = y * w + x + 1;
            s.push_str(&format!("f {a} {} {}\nf {} {} {}\n", a + 1, a + w, a + 1, a + w + 1, a + w));
        }
    }
    s
}

fn scene3d_key(s: &Session) -> bool {
    serde_json::to_value(&*s.project).unwrap().get("scene3d").is_some()
}

#[test]
fn a_project_without_3d_content_has_no_scene3d_and_import_creates_it() {
    let dir = tmp_dir("scene3d-basic");
    let mut s = Session::default();
    assert!(!scene3d_key(&s), "P-A1/P-A2: no 3D content, no section, no capability");
    assert_eq!(ok(&mut s, "scene3d.list", json!({}))["assets"], json!([]));
    assert!(s.execute("scene3d.remove", json!({"id": 1})).is_err());

    let f = write(&dir, "tri.obj", TRIANGLE_OBJ.as_bytes());
    let a = ok(&mut s, "scene3d.import", json!({"path": f}));
    assert_eq!((a["format"].as_str(), a["route"].as_str(), a["present"].as_bool()), (Some("obj"), Some("A"), Some(true)), "route A by default");
    assert_eq!(a["stats"]["triangles"], 1);
    assert_eq!(a["importer"], "cfa4-model3d 1");
    assert_eq!(a["sha256"].as_str().unwrap().len(), 64);
    assert!(scene3d_key(&s));
    let list = ok(&mut s, "scene3d.list", json!({}));
    assert_eq!(list["capabilities"]["obj"], json!(["reference", "preview"]));
    assert!(s.execute("scene3d.import", json!({"path": f})).is_err(), "the same file is not imported twice");

    // the whole import is one undo step and takes the section away again
    ok(&mut s, "edit.undo", json!({}));
    assert!(!scene3d_key(&s));
    ok(&mut s, "edit.redo", json!({}));
    assert!(scene3d_key(&s));

    let id = a["id"].as_u64().unwrap();
    let r = ok(&mut s, "scene3d.remove", json!({"id": id}));
    assert_eq!(r["scene3d"], false);
    assert!(!scene3d_key(&s), "the last asset takes the section with it");
    assert!(f.exists(), "the file is never touched");
}

#[test]
fn the_route_is_chosen_per_asset_and_switching_loses_nothing() {
    let dir = tmp_dir("scene3d-route");
    let mut s = Session::default();
    // an OBJ with a free-form element lands in the report
    let obj = write(&dir, "a.obj", format!("{TRIANGLE_OBJ}curv 0 1 1 2\n").as_bytes());
    let other = write(&dir, "b.stl", b"solid x\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid x\n");
    let a = ok(&mut s, "scene3d.import", json!({"path": obj, "route": "B", "name": "Chair"}));
    let b = ok(&mut s, "scene3d.import", json!({"path": other}));
    let (ia, ib) = (a["id"].as_u64().unwrap(), b["id"].as_u64().unwrap());
    assert_eq!((a["route"].as_str(), b["route"].as_str(), a["name"].as_str()), (Some("B"), Some("A"), Some("Chair")));
    let report = a["report"].as_array().unwrap().clone();
    assert!(!report.is_empty(), "the unsupported element is reported, not dropped silently");
    assert!(a["unsupported"].as_u64().unwrap() >= 1);

    ok(&mut s, "scene3d.route", json!({"id": ia, "route": "A"}));
    ok(&mut s, "scene3d.route", json!({"id": ia, "route": "B"}));
    let list = ok(&mut s, "scene3d.list", json!({}));
    let first = &list["assets"][0];
    assert_eq!(first["route"], "B");
    assert_eq!(first["report"].as_array().unwrap(), &report, "reference and report survive route changes");
    assert_eq!(list["assets"][1]["route"], "A", "the other asset is not affected");
    assert!(s.execute("scene3d.route", json!({"id": ib, "route": "C"})).is_err());
}

#[test]
fn usd_is_referenced_on_route_a_only() {
    let dir = tmp_dir("scene3d-usd");
    let mut s = Session::default();
    let f = write(&dir, "set.usda", b"#usda 1.0\n");
    assert!(s.execute("scene3d.import", json!({"path": f, "route": "B"})).is_err());
    assert!(!scene3d_key(&s), "a refused import leaves no trace");
    let a = ok(&mut s, "scene3d.import", json!({"path": f}));
    assert_eq!((a["format"].as_str(), a["route"].as_str()), (Some("usd"), Some("A")));
    assert_eq!(a["unsupported"], 1, "no silent loss: the report says USD is not read natively");
    let id = a["id"].as_u64().unwrap();
    assert!(s.execute("scene3d.route", json!({"id": id, "route": "B"})).is_err());
    assert!(s.execute("scene3d.lod", json!({"id": id})).is_err());
}

#[test]
fn hostile_and_unknown_files_are_errors_and_change_nothing() {
    let dir = tmp_dir("scene3d-hostile");
    let mut s = Session::default();
    let bad_glb = write(&dir, "x.glb", b"glTF\x02\x00\x00\x00\xff\xff\xff\xff");
    let bad_json = write(&dir, "y.gltf", b"{\"asset\":{\"version\":\"2.0\"},\"nodes\":[{\"children\":[0]}]}");
    let text = write(&dir, "z.txt", b"hello");
    for f in [&bad_glb, &bad_json, &text, &dir.join("missing.obj")] {
        assert!(s.execute("scene3d.import", json!({"path": f})).is_err(), "{f:?}");
    }
    assert!(!scene3d_key(&s));
    assert!(s.execute("scene3d.import", json!({})).is_err());
    assert!(s.execute("scene3d.import", json!({"path": "a\u{0}b"})).is_err());
}

#[test]
fn a_moved_file_is_relinked_with_a_report_and_a_changed_one_needs_consent() {
    let dir = tmp_dir("scene3d-relink");
    let mut s = Session::default();
    let f = write(&dir, "tri.obj", TRIANGLE_OBJ.as_bytes());
    let id = ok(&mut s, "scene3d.import", json!({"path": f}))["id"].as_u64().unwrap();
    assert_eq!(ok(&mut s, "scene3d.check", json!({}))["problems"], 0);

    let moved = dir.join("moved.obj");
    std::fs::rename(&f, &moved).unwrap();
    let c = ok(&mut s, "scene3d.check", json!({"id": id}));
    assert_eq!(c["assets"][0]["status"], "missing");
    assert!(c["assets"][0]["message"].as_str().unwrap().contains("relink"));
    assert_eq!(ok(&mut s, "scene3d.list", json!({}))["assets"][0]["present"], false, "the project still opens and lists it");

    let r = ok(&mut s, "scene3d.relink", json!({"id": id, "path": moved}));
    assert_eq!(r["changed"], false);
    assert_eq!(ok(&mut s, "scene3d.check", json!({}))["problems"], 0);

    // a different file: refused, nothing changed
    let other = write(&dir, "two.obj", format!("{TRIANGLE_OBJ}v 2 2 2\nf 1 2 4\n").as_bytes());
    let before = s.project.scene3d.clone();
    let e = s.execute("scene3d.relink", json!({"id": id, "path": other})).unwrap_err().to_string();
    assert!(e.contains("differs") && e.contains("allowChanged"), "{e}");
    assert_eq!(s.project.scene3d, before);
    // a different format is refused even with consent
    let stl = write(&dir, "t.stl", b"solid x\nendsolid x\n");
    assert!(s.execute("scene3d.relink", json!({"id": id, "path": stl, "allowChanged": true})).is_err());
    // with consent the file is read again
    let r = ok(&mut s, "scene3d.relink", json!({"id": id, "path": other, "allowChanged": true}));
    assert_eq!((r["changed"].as_bool(), r["asset"]["stats"]["triangles"].as_u64()), (Some(true), Some(2)));
    assert_eq!(ok(&mut s, "scene3d.check", json!({}))["problems"], 0);

    // edit the file behind the project's back
    std::fs::write(&other, TRIANGLE_OBJ).unwrap();
    assert_eq!(ok(&mut s, "scene3d.check", json!({}))["assets"][0]["status"], "changed");
}

#[test]
fn the_section_survives_save_and_load_and_nothing_else_changes() {
    let dir = tmp_dir("scene3d-roundtrip");
    let mut s = Session::default();
    let before = cfa4_format::encode(&s.project, false);
    let f = write(&dir, "tri.obj", TRIANGLE_OBJ.as_bytes());
    ok(&mut s, "scene3d.import", json!({"path": f, "route": "B"}));
    let bytes = cfa4_format::encode(&s.project, false);
    let loaded = cfa4_format::decode(&bytes).unwrap();
    assert_eq!(loaded.project.scene3d, s.project.scene3d);
    assert_eq!(loaded.project.next_id, s.project.next_id);
    // outside `scene3d` the project is what it was
    let mut a: Value = serde_json::from_slice(&bytes).unwrap();
    a["project"].as_object_mut().unwrap().remove("scene3d");
    let mut b: Value = serde_json::from_slice(&before).unwrap();
    for v in [&mut a, &mut b] {
        v["project"].as_object_mut().unwrap().remove("next_id");
    }
    assert_eq!(a["project"], b["project"]);
    // a removed asset leaves the bytes of a project that never had one
    let id = s.project.scene3d.assets[0].id;
    ok(&mut s, "scene3d.remove", json!({"id": id}));
    let after: Value = serde_json::from_slice(&cfa4_format::encode(&s.project, false)).unwrap();
    assert!(after["project"].get("scene3d").is_none());
}

#[test]
fn lod_plans_against_the_frame_and_needs_approval_to_reduce_quality() {
    let dir = tmp_dir("scene3d-lod");
    let mut s = Session::default();
    let f = write(&dir, "grid.obj", grid_obj(60).as_bytes()); // 7 200 triangles
    let id = ok(&mut s, "scene3d.import", json!({"path": f, "route": "B"}))["id"].as_u64().unwrap();

    let fit = ok(&mut s, "scene3d.lod", json!({"id": id, "memoryMb": 512}));
    assert_eq!((fit["needsApproval"].as_bool(), fit["applied"].as_bool(), fit["triangles"].as_u64()), (Some(false), Some(true), Some(7200)));

    let tight = json!({"id": id, "maxTriangles": 1000});
    let plan = ok(&mut s, "scene3d.lod", tight.clone());
    assert_eq!((plan["needsApproval"].as_bool(), plan["applied"].as_bool()), (Some(true), Some(false)), "not applied without approval");
    assert!(plan["reasons"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("triangle frame")));
    let mut approved = tight;
    approved["approve"] = json!(true);
    let done = ok(&mut s, "scene3d.lod", approved);
    assert_eq!(done["applied"], true);
    assert!(done["triangles"].as_u64().unwrap() < 7200);

    // a region cuts the data first
    let region = ok(&mut s, "scene3d.lod", json!({"id": id, "region": {"min": [0, 0, -1], "max": [10, 10, 1]}}));
    assert_eq!(region["triangles"], 200);
    assert!(s.execute("scene3d.lod", json!({"id": id, "region": {"min": [5, 0, 0], "max": [1, 1, 1]}})).is_err());

    // a file that changed under the project is not planned silently
    std::fs::write(&f, TRIANGLE_OBJ).unwrap();
    assert!(s.execute("scene3d.lod", json!({"id": id})).is_err());
    assert!(!s.project.scene3d.is_empty() && s.project.scene3d.assets.len() == 1, "planning stores nothing");
}
