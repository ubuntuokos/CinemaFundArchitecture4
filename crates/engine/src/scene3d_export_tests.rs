use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Content, Planned, commit, plan_outputs};
use crate::Session;
use crate::media_test_util::tmp_dir;
use cfa4_model3d as m3d;

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("{cmd} should have failed"))
}

fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, bytes).unwrap();
    p
}

/// A textured OBJ with a material library; returns the OBJ path.
fn obj_asset(dir: &Path) -> PathBuf {
    put(dir, "m.mtl", b"newmtl red\nKd 1 0 0\nmap_Kd tex/red.png\n");
    put(dir, "tex/red.png", b"png-bytes");
    put(dir, "tri.obj", b"mtllib m.mtl\no tri\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n")
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    fn walk(base: &Path, at: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(at).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    walk(dir, dir, &mut v);
    v.sort();
    v
}

#[test]
fn a_lossy_export_is_planned_first_and_written_after_approval_with_its_textures() {
    let src = tmp_dir("export-src");
    let out = tmp_dir("export-out");
    let mut s = Session::default();
    let a = ok(&mut s, "scene3d.import", json!({"path": obj_asset(&src)}));
    let id = a["id"].as_u64().unwrap();
    let before = serde_json::to_string(&*s.project).unwrap();

    let plan = ok(&mut s, "scene3d.export", json!({"id": id, "dir": out}));
    assert_eq!((plan["written"].clone(), plan["needsApproval"].clone(), plan["lossless"].clone()), (json!(false), json!(true), json!(false)));
    assert!(plan["report"].as_array().unwrap().iter().any(|r| r["message"].as_str().unwrap().contains("no unit")), "{}", plan["report"]);
    assert!(files_in(&out).is_empty(), "no report approved, no file written");
    assert_eq!(plan["files"][0]["name"], "tri.glb");
    assert_eq!(plan["textures"][0], json!({"uri": "tex/red.png", "status": "copy"}));

    let done = ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "approve": true}));
    assert_eq!(done["written"], true);
    assert_eq!(files_in(&out), vec!["tex/red.png".to_string(), "tri.glb".to_string()]);
    assert_eq!(std::fs::read(out.join("tex/red.png")).unwrap(), b"png-bytes");
    let glb = std::fs::read(out.join("tri.glb")).unwrap();
    let back = m3d::read(&glb, Some("tri.glb"), &mut m3d::NoResources, &m3d::Limits::default()).unwrap();
    assert_eq!((back.triangle_count(), back.materials[0].name.as_deref()), (1, Some("red")));
    assert_eq!(back.materials[0].textures[0].uri.as_deref(), Some("tex/red.png"));
    assert!(back.asset.generator.as_deref().is_some_and(|g| g.starts_with("CFA4 ")));
    assert!(files_in(&src).iter().all(|f| !f.ends_with(".tmp")));
    assert_eq!(serde_json::to_string(&*s.project).unwrap(), before, "exporting does not change the project (no undo step)");
}

#[test]
fn a_lossless_export_needs_no_approval_and_is_byte_identical_everywhere() {
    let src = tmp_dir("export-lossless-src");
    let out1 = tmp_dir("export-lossless-1");
    let out2 = tmp_dir("export-lossless-2");
    let mut s = Session::default();
    let id = ok(&mut s, "scene3d.import", json!({"path": obj_asset(&src)}))["id"].as_u64().unwrap();
    ok(&mut s, "scene3d.export", json!({"id": id, "dir": out1, "approve": true, "copyTextures": false}));
    // the glb we wrote is a glTF asset: glTF to glTF loses nothing
    let glb = out1.join("tri.glb");
    let id2 = ok(&mut s, "scene3d.import", json!({"path": glb}))["id"].as_u64().unwrap();
    let r = ok(&mut s, "scene3d.export", json!({"id": id2, "dir": out2, "name": "tri", "copyTextures": false}));
    assert_eq!((r["lossless"].clone(), r["written"].clone(), r["report"].clone()), (json!(true), json!(true), json!([])), "{r}");
    assert_eq!(std::fs::read(out2.join("tri.glb")).unwrap(), std::fs::read(&glb).unwrap(), "same bytes after a second round");
    assert_eq!(r["textures"][0]["status"], "referenced");
    assert!(!out2.join("tex").exists());

    // a missing image is named, not copied
    std::fs::remove_file(out1.join("tex/red.png")).ok();
    let out3 = tmp_dir("export-lossless-3");
    let m = ok(&mut s, "scene3d.export", json!({"id": id2, "dir": out3}));
    assert_eq!(m["textures"][0]["status"], "missing");
    assert_eq!(m["written"], true);
}

#[test]
fn existing_files_the_source_links_and_odd_names_are_never_overwritten() {
    let src = tmp_dir("export-safe-src");
    let out = tmp_dir("export-safe-out");
    let mut s = Session::default();
    let id = ok(&mut s, "scene3d.import", json!({"path": obj_asset(&src)}))["id"].as_u64().unwrap();
    ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "approve": true}));

    // a second export to the same place asks first
    let e = err(&mut s, "scene3d.export", json!({"id": id, "dir": out, "approve": true}));
    assert!(e.contains("already exist") && e.contains("tri.glb") && e.contains("overwrite"), "{e}");
    let before = std::fs::read(out.join("tri.glb")).unwrap();
    ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "approve": true, "overwrite": true}));
    assert_eq!(std::fs::read(out.join("tri.glb")).unwrap(), before);

    // the source itself is never replaced, even with overwrite
    let original = std::fs::read(src.join("tri.obj")).unwrap();
    let e = err(&mut s, "scene3d.export", json!({"id": id, "dir": src, "format": "obj", "approve": true, "overwrite": true}));
    assert!(e.contains("source"), "{e}");
    assert_eq!(std::fs::read(src.join("tri.obj")).unwrap(), original);
    // nor the files the asset depends on (the texture is copied onto itself only when identical: skipped)
    let e = err(&mut s, "scene3d.export", json!({"id": id, "dir": src, "format": "obj", "name": "m", "approve": true, "overwrite": true}));
    assert!(e.contains("source"), "{e}"); // m.obj is fine, m.mtl is the asset's own material library
    assert_eq!(std::fs::read(src.join("m.mtl")).unwrap(), b"newmtl red\nKd 1 0 0\nmap_Kd tex/red.png\n");

    // a name cannot climb out of the folder
    let r = ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "name": "../../evil", "approve": true, "copyTextures": false}));
    let name = format!("{}.glb", m3d::sanitize_stem("../../evil"));
    assert_eq!(r["files"][0]["name"], name.as_str());
    assert!(out.join(&name).exists() && !out.parent().unwrap().join("evil.glb").exists());
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_never_followed() {
    let src = tmp_dir("export-link-src");
    let out = tmp_dir("export-link-out");
    let elsewhere = tmp_dir("export-link-else");
    let mut s = Session::default();
    let id = ok(&mut s, "scene3d.import", json!({"path": obj_asset(&src)}))["id"].as_u64().unwrap();
    let target = put(&elsewhere, "precious.txt", b"keep me");
    std::os::unix::fs::symlink(&target, out.join("tri.glb")).unwrap();
    let e = err(&mut s, "scene3d.export", json!({"id": id, "dir": out, "approve": true, "overwrite": true}));
    assert!(e.contains("symbolic link"), "{e}");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
    // a folder that is a link, on the way of a texture copy
    std::fs::remove_file(out.join("tri.glb")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.join("tex")).unwrap();
    let e = err(&mut s, "scene3d.export", json!({"id": id, "dir": out, "approve": true}));
    assert!(e.contains("symbolic link"), "{e}");
    assert!(!elsewhere.join("red.png").exists());
    assert!(files_in(&out).is_empty() || files_in(&out) == vec!["tex/precious.txt".to_string()]);
}

#[test]
fn bad_parameters_usd_and_changed_sources_are_errors() {
    let src = tmp_dir("export-bad-src");
    let out = tmp_dir("export-bad-out");
    let mut s = Session::default();
    assert!(s.execute("scene3d.export", json!({"id": 1, "dir": out})).is_err(), "no 3D content, no export");
    let obj = obj_asset(&src);
    let id = ok(&mut s, "scene3d.import", json!({"path": obj}))["id"].as_u64().unwrap();
    assert!(err(&mut s, "scene3d.export", json!({"id": id})).contains("`dir`"));
    assert!(err(&mut s, "scene3d.export", json!({"id": id, "dir": out.join("nope")})).contains("must exist"));
    assert!(err(&mut s, "scene3d.export", json!({"id": id, "dir": obj})).contains("not a folder"));
    assert!(err(&mut s, "scene3d.export", json!({"id": id, "dir": out, "format": "usd"})).contains("`format`"));
    assert!(err(&mut s, "scene3d.export", json!({"id": id, "dir": out, "name": 3})).contains("`name`"));
    assert!(err(&mut s, "scene3d.export", json!({"id": 99, "dir": out})).contains("no 3D asset"));
    assert!(err(&mut s, "scene3d.export", json!({"id": id, "dir": out, "memoryMb": 0})).contains("memoryMb"));
    // a memory budget too small for the scene is a limit error, nothing written
    assert!(s.execute("scene3d.export", json!({"id": id, "dir": out, "memoryMb": 1, "approve": true})).is_ok(), "one MiB is plenty for a triangle");
    // USD is referenced, not read: its export waits for U3b
    let usd = put(&src, "scene.usda", b"#usda 1.0\n");
    let uid = ok(&mut s, "scene3d.import", json!({"path": usd}))["id"].as_u64().unwrap();
    let e = err(&mut s, "scene3d.export", json!({"id": uid, "dir": out}));
    assert!(e.contains("U3b") && e.contains("U2"), "{e}");
    // the file changed under the project
    std::fs::write(&obj, b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    assert!(err(&mut s, "scene3d.export", json!({"id": id, "dir": out})).contains("changed"));
}

#[test]
fn obj_and_stl_exports_write_their_companion_files() {
    let src = tmp_dir("export-obj-src");
    let out = tmp_dir("export-obj-out");
    let mut s = Session::default();
    let id = ok(&mut s, "scene3d.import", json!({"path": obj_asset(&src)}))["id"].as_u64().unwrap();
    let r = ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "format": "obj", "name": "copy", "approve": true}));
    assert_eq!(r["written"], true);
    assert_eq!(files_in(&out), vec!["copy.mtl".to_string(), "copy.obj".to_string(), "tex/red.png".to_string()]);
    let r = ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "format": "stl-ascii", "approve": true}));
    assert!(r["report"].as_array().unwrap().iter().any(|i| i["message"].as_str().unwrap().contains("no materials")));
    assert!(std::fs::read_to_string(out.join("tri.stl")).unwrap().starts_with("solid tri"));
    let r = ok(&mut s, "scene3d.export", json!({"id": id, "dir": out, "format": "stl", "name": "bin", "approve": true}));
    assert_eq!(r["files"][0]["bytes"], 84 + 50);
}

// ---------------------------------------------------------------------- the writer on its own

fn planned(dir: &Path, names: &[&str]) -> Vec<Planned> {
    names.iter().map(|n| Planned { dest: dir.join(n), content: Content::Bytes(n.as_bytes().to_vec()) }).collect()
}

fn leftovers(dir: &Path) -> Vec<String> {
    files_in(dir)
}

#[test]
fn a_cancel_or_a_failure_leaves_no_half_written_file() {
    let dir = tmp_dir("export-commit");
    // cancel before the second file is staged
    let calls = std::cell::Cell::new(0);
    let cancel = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    let e = commit(&planned(&dir, &["a.glb", "b.bin"]), false, &cancel).unwrap_err();
    assert!(e.contains("cancelled"), "{e}");
    assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    // cancel while publishing: the first file is removed again
    let calls = std::cell::Cell::new(0);
    let cancel = || {
        calls.set(calls.get() + 1);
        calls.get() > 3
    };
    let e = commit(&planned(&dir, &["a.glb", "b.bin"]), false, &cancel).unwrap_err();
    assert!(e.contains("removed"), "{e}");
    assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    // a texture that cannot be read (here: a folder) fails the whole export
    let bad_src = tmp_dir("export-commit-src");
    let jobs = vec![
        Planned { dest: dir.join("ok.glb"), content: Content::Bytes(b"x".to_vec()) },
        Planned { dest: dir.join("tex/t.png"), content: Content::Copy(bad_src) },
    ];
    assert!(commit(&jobs, false, &|| false).is_err());
    assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    // a destination that became a folder in the meantime
    std::fs::create_dir(dir.join("in-the-way")).unwrap();
    let e = commit(&planned(&dir, &["fine.glb", "in-the-way"]), true, &|| false).unwrap_err();
    assert!(e.contains("in-the-way"), "{e}");
    assert!(!dir.join("fine.glb").exists() && leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    // and the good case
    let done = commit(&planned(&dir, &["a.glb", "b.bin"]), false, &|| false).unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(leftovers(&dir), vec!["a.glb".to_string(), "b.bin".to_string()]);
}

#[test]
fn planning_refuses_unsafe_names_and_duplicates() {
    let dir = tmp_dir("export-plan");
    let file = |n: &str| m3d::ExportFile { name: n.into(), data: vec![1] };
    assert!(plan_outputs(&dir, &[file("../x.glb")], &[], &[], false).unwrap_err().contains("safe"));
    assert!(plan_outputs(&dir, &[file("/etc/x")], &[], &[], false).is_err());
    assert!(plan_outputs(&dir, &[file("a.glb"), file("a.glb")], &[], &[], false).unwrap_err().contains("same name"));
    let src = put(&dir, "src/img.png", b"i");
    let p = plan_outputs(&dir, &[file("a.glb")], &[("img.png".into(), src.clone())], &[], false).unwrap();
    assert_eq!(p.len(), 2);
    // an identical image is already there: skipped, not a conflict
    std::fs::copy(&src, dir.join("img.png")).unwrap();
    let p = plan_outputs(&dir, &[file("a.glb")], &[("img.png".into(), src.clone())], &[], false).unwrap();
    assert_eq!(p.len(), 1);
    // a different image of the same name is
    std::fs::write(dir.join("img.png"), b"other").unwrap();
    assert!(plan_outputs(&dir, &[file("a.glb")], &[("img.png".into(), src)], &[], false).unwrap_err().contains("img.png"));
}
