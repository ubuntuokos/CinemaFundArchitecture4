//! AS-02 bundle import (#1363): a bundle built here from a component file CFA4 exports and a
//! hand-made version file (no essence, two tracks of clips with network locators).

use std::path::PathBuf;

use serde_json::json;

use crate::Session;
use cfa4_mxf::testing::*;
use cfa4_time::Tick;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn scratch(name: &str) -> PathBuf {
    let d = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("engine-as02-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("media")).unwrap();
    d
}

const OP1B: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x02, 0x09, 0x00];

/// A version file: one picture clip and one sound clip, `frames` long at 25 fps, from the
/// components `umid(2)` and `umid(3)`, located at `locator`.
fn version(frames: i64, locator: &str) -> Vec<u8> {
    let mut md = Vec::new();
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&[]));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B08, &uuid(3)).p(0x3B09, &OP1B);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut cs = LocalSet::new(2);
    cs.p(0x1901, &batch(&[&uuid(3), &uuid(4), &uuid(5)]));
    klv(&mut md, set_key(0x18), &cs.0);
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4402, &utf16("Version A")).p(0x4403, &batch(&[&uuid(10), &uuid(11)]));
    klv(&mut md, set_key(0x36), &mp.0);
    for (k, (def, number, pkg)) in [(PICTURE_DEF, 0x1501_0500u32, umid(2)), (SOUND_DEF, 0x1601_0300, umid(3))].into_iter().enumerate() {
        let k8 = k as u8;
        klv(&mut md, set_key(0x3B), &track(10 + k8, 1 + k as u32, number, 20 + k8));
        klv(&mut md, set_key(0x0F), &sequence(20 + k8, def, frames, 30 + k8));
        klv(&mut md, set_key(0x11), &source_clip(30 + k8, def, frames, pkg, 1));
    }
    for (k, umid_n) in [2u8, 3].into_iter().enumerate() {
        let k8 = k as u8;
        let mut fp = LocalSet::new(4 + k8);
        fp.p(0x4401, &umid(umid_n)).p(0x4403, &batch(&[&uuid(13 + k8)])).p(0x4701, &uuid(40 + k8));
        klv(&mut md, set_key(0x37), &fp.0);
        klv(&mut md, set_key(0x3B), &track(13 + k8, 1, 0, 23 + k8));
        klv(&mut md, set_key(0x0F), &sequence(23 + k8, PICTURE_DEF, frames, 33 + k8));
        klv(&mut md, set_key(0x11), &source_clip(33 + k8, PICTURE_DEF, frames, [0; 32], 0));
        let loc = uuid(50 + k8);
        let mut d = LocalSet::new(40 + k8);
        d.p(0x3001, &[0, 0, 0, 25, 0, 0, 0, 1]).p(0x2F01, &batch(&[&loc]));
        klv(&mut md, set_key(0x25), &d.0);
        let mut l = LocalSet::new(loc[15]);
        l.p(0x4001, &utf16(locator));
        klv(&mut md, set_key(0x32), &l.0);
    }
    let mut f = partition_with(2, 0x04, 0, 0, md.len() as u64, &[]);
    f.extend_from_slice(&md);
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(4, 0x04, at, 0, 0, &[]));
    f
}

fn manifest(with_components: bool) -> String {
    let id = |n: u8| cfa4_mxf::as02::file_id_of_umid(&umid(n));
    let file = |id: String, role: &str, path: &str| {
        format!("<mft:File><mft:FileID>{id}</mft:FileID><mft:Role>{role}</mft:Role><mft:Size>1</mft:Size><mft:Path>{path}</mft:Path></mft:File>")
    };
    let mut files = file(id(1), "version", "bundle.mxf");
    if with_components {
        files += &file(id(2), "essencecomponent", "media/comp.mxf");
        files += &file(id(3), "essencecomponent", "media/comp.mxf");
    }
    format!(
        "<mft:Manifest xmlns:mft=\"{}\"><mft:BundleName>Test bundle</mft:BundleName><mft:BundleID>urn:uuid:11111111-2222-4333-8444-555555555555</mft:BundleID>\
         <mft:Creator>test</mft:Creator><mft:CreationDate>2026-11-20T00:00:00Z</mft:CreationDate><mft:FileList>{files}</mft:FileList></mft:Manifest>",
        cfa4_mxf::as02::MANIFEST_NS
    )
}

fn component(dir: &std::path::Path) {
    let mut s = demo();
    let out = dir.join("media").join("comp.mxf");
    s.execute(
        "file.exportMedia",
        json!({"format": "mxf-op1a", "path": out.to_string_lossy(), "range": "custom", "startSeconds": 1.0, "endSeconds": 1.5, "wait": true}),
    )
    .unwrap();
}

#[test]
fn a_bundle_imports_as_a_sequence_per_version() {
    let dir = scratch("ok");
    component(&dir);
    std::fs::write(dir.join("bundle.mxf"), version(5, "media/comp.mxf")).unwrap();
    std::fs::write(dir.join("manifest.xml"), manifest(true)).unwrap();
    let mut s = demo();
    let before = s.project.items.len();
    let r = s.execute("file.import", json!({"paths": [dir.to_string_lossy()]})).unwrap();
    let doc = &r["documents"][0];
    assert_eq!(doc["format"], "AS-02", "{r}");
    assert_eq!(doc["versions"][0]["placed"], 2, "{r}");
    assert_eq!(doc["versions"][0]["pattern"], "OP1b", "{r}");
    assert!(doc["warnings"].as_array().unwrap().is_empty(), "{r}");
    let q = s.active_sequence().unwrap();
    let v = &q.video_tracks[0].items;
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].start, Tick::ZERO);
    assert!(q.audio_tracks.iter().any(|t| !t.items.is_empty()));
    assert!(s.project.items.len() > before);
    // one undo step
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.items.len(), before);
    // the manifest itself imports the same bundle
    let r = s.execute("file.import", json!({"paths": [dir.join("manifest.xml").to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["versions"][0]["placed"], 2, "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A component the manifest lacks is found by the locator; a locator out of the bundle is refused.
#[test]
fn locators_stay_inside_the_bundle() {
    let dir = scratch("loc");
    component(&dir);
    std::fs::write(dir.join("manifest.xml"), manifest(false)).unwrap();
    std::fs::write(dir.join("bundle.mxf"), version(5, "media/comp.mxf")).unwrap();
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [dir.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["versions"][0]["placed"], 2, "the locator finds the file: {r}");
    std::fs::write(dir.join("bundle.mxf"), version(5, "../secret.mxf")).unwrap();
    let e = s.execute("file.import", json!({"paths": [dir.to_string_lossy()]})).unwrap_err().to_string();
    assert!(e.contains("../secret.mxf") && e.contains("unsafe path"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn not_a_bundle_is_left_to_the_other_importers() {
    let dir = scratch("none");
    let mut s = demo();
    assert!(crate::as02_import::import_package(&mut s, &dir.to_string_lossy(), None).is_none());
    std::fs::write(dir.join("manifest.xml"), "<Manifest/>").unwrap();
    assert!(crate::as02_import::import_package(&mut s, &dir.to_string_lossy(), None).is_none());
    std::fs::write(dir.join("manifest.xml"), manifest(false)).unwrap();
    let e = crate::as02_import::import_package(&mut s, &dir.to_string_lossy(), None).unwrap().unwrap_err().to_string();
    assert!(e.contains("nothing could be imported"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1477: a bundle exported with `mxfAs02` comes back through `file.import` as one sequence with
/// a picture track and the audio channel tracks, as long as the export.
#[test]
fn an_exported_bundle_imports_again() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Bundle", "fps": 25.0, "width": 320, "height": 180, "video": 1, "audio": 1})).unwrap();
    let dir = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("engine-as02-roundtrip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.to_string_lossy().to_string();
    let p = json!({"format": "mxf-op1a", "path": out, "mxfAs02": true, "range": "custom", "startSeconds": 0.0, "endSeconds": 1.0, "wait": true});
    s.execute("file.exportMedia", p).unwrap();
    assert!(dir.join("manifest.xml").exists() && dir.join("media").is_dir());
    let r = s.execute("file.import", json!({"paths": [out]})).unwrap();
    let d = &r["documents"][0];
    assert_eq!(d["format"], "AS-02", "{r}");
    assert_eq!(d["versions"].as_array().map(Vec::len), Some(1), "{r}");
    assert_eq!(d["versions"][0]["placed"], 3, "picture + two audio channels: {r}");
    assert!(d["warnings"].as_array().is_none_or(Vec::is_empty), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1536: `mxfAs02Versions` writes more sequences into the same bundle; import gives one sequence
/// per version again.
#[test]
fn a_bundle_with_two_versions_imports_as_two_sequences() {
    let mut s = Session::default();
    let new = |s: &mut Session, name: &str| {
        s.execute("file.newSequence", json!({"name": name, "fps": 25.0, "width": 320, "height": 180, "video": 1, "audio": 1})).unwrap();
        s.state.active_sequence.unwrap()
    };
    let a = new(&mut s, "Version A");
    let b = new(&mut s, "Version B");
    let dir = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("engine-as02-versions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.to_string_lossy().to_string();
    let p = json!({"format": "mxf-op1a", "path": out, "sequence": a.0, "mxfAs02": true, "mxfAs02Versions": [b.0], "wait": true});
    s.execute("file.exportMedia", p).unwrap();
    let r = s.execute("file.import", json!({"paths": [out]})).unwrap();
    let d = &r["documents"][0];
    assert_eq!(d["versions"].as_array().map(Vec::len), Some(2), "{r}");
    // an unknown sequence is refused before anything is written
    let bad = json!({"format": "mxf-op1a", "path": out, "mxfAs02": true, "mxfAs02Versions": [999_999], "wait": true});
    assert!(s.execute("file.exportMedia", bad).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
