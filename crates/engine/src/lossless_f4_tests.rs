//! F4 of CFA4-LOSSLESS-MEDIA-TOOLS-001 (plan 1.1 §10): the mandatory test areas the earlier
//! phases did not cover yet — segments saved and reloaded with the project, the original media
//! left untouched, a write failure in the middle of a batch, a damaged source.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::Session;
use crate::segments_tests::{marker, with_markers};
use crate::smart_cut_tests::{MemoryFiles, frame, session, source_mp4};

#[test]
fn segments_are_saved_with_the_project_and_come_back() {
    let (mut s, item, dir) = with_markers("f4-save", vec![marker(900, 0, 10, "intro"), marker(901, 20, 10, "end")]);
    let before = s.execute("segments.list", json!({"item": item.0})).unwrap()["segments"].clone();
    let path = dir.join("p.fcproj").to_string_lossy().into_owned();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    let mut t = Session::default();
    t.execute("file.open", json!({"path": path})).unwrap();
    assert_eq!(t.execute("segments.list", json!({"item": item.0})).unwrap()["segments"], before);
    // and they still cut and go to the timeline after the reload
    let output = dir.join("{n}.mp4").to_string_lossy().into_owned();
    let r = t.execute("segments.plan", json!({"item": item.0, "output": output})).unwrap();
    assert_eq!(r["summary"]["ranges"], 2, "{r}");
    let r = t.execute("segments.export", json!({"item": item.0, "output": output})).unwrap();
    assert_eq!(r["verified"], true, "{r}");
    for n in 1..=2 {
        let f = dir.join(format!("{n}.mp4"));
        assert!(std::fs::metadata(&f).is_ok_and(|m| m.len() > 0), "{} written after the reload", f.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_original_media_is_never_changed() {
    let (mut s, files) = session();
    let original = source_mp4();
    s.execute("export.start", json!({"path": "clip.mp4", "in": frame(10), "out": frame(30), "output": "a.mp4"})).unwrap();
    s.execute(
        "export.start",
        json!({"path": "clip.mp4", "output": "r{n}.mp4", "ranges": [{"in": frame(0), "out": frame(10)}, {"in": frame(20), "out": frame(40)}]}),
    )
    .unwrap();
    let confirm = json!({"path": "clip.mp4", "in": frame(5), "out": frame(30), "output": "b.mp4", "mode": "frame-accurate", "confirm": true});
    s.execute("export.start", confirm).unwrap();
    assert_eq!(files.0.lock().unwrap()["clip.mp4"], original, "the source bytes are the same");
    // the output may not be the source itself
    assert!(s.execute("export.start", json!({"path": "clip.mp4", "in": frame(10), "out": frame(30), "output": "clip.mp4"})).is_err());
    assert_eq!(files.0.lock().unwrap()["clip.mp4"], original);
}

/// Files that refuse every write after `allowed` of them (a disk that fills up).
#[derive(Default)]
struct FillingDisk {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    allowed: usize,
    writes: Mutex<usize>,
}

impl crate::Services for FillingDisk {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned().ok_or_else(|| std::io::ErrorKind::NotFound.into())
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut n = self.writes.lock().unwrap();
        if *n >= self.allowed {
            return Err(std::io::Error::other("No space left on device"));
        }
        *n += 1;
        self.files.lock().unwrap().insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }
}

#[test]
fn a_full_disk_in_the_middle_of_a_batch_names_what_was_written() {
    let disk = Arc::new(FillingDisk { allowed: 1, ..Default::default() });
    disk.files.lock().unwrap().insert("clip.mp4".into(), source_mp4());
    let mut s = Session::new(disk.clone());
    let e = s
        .execute(
            "export.start",
            json!({"path": "clip.mp4", "output": "d{n}.mp4", "ranges": [{"in": frame(0), "out": frame(10)}, {"in": frame(20), "out": frame(30)}]}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("range 2") && e.contains("No space left") && e.contains("d1.mp4"), "{e}");
    let files = disk.files.lock().unwrap();
    assert!(files.contains_key("d1.mp4") && !files.contains_key("d2.mp4"));
    assert_eq!(files["clip.mp4"], source_mp4(), "the source is untouched");
}

#[test]
fn a_damaged_source_is_refused_and_never_panics() {
    let good = source_mp4();
    let files = Arc::new(MemoryFiles::default());
    {
        let mut f = files.0.lock().unwrap();
        f.insert("cut.mp4".into(), good[..good.len() / 2].to_vec());
        f.insert("empty.mp4".into(), Vec::new());
        let mut flipped = good.clone();
        for b in flipped.iter_mut().skip(200).step_by(97) {
            *b ^= 0x5a;
        }
        f.insert("flipped.mp4".into(), flipped);
    }
    let mut s = Session::new(files.clone());
    for path in ["cut.mp4", "empty.mp4", "flipped.mp4", "missing.mp4"] {
        let _ = s.execute("export.analyze", json!({"path": path}));
        let _ = s.execute("export.plan", json!({"path": path, "in": frame(10), "out": frame(30)}));
        let r = s.execute("export.start", json!({"path": path, "in": frame(10), "out": frame(30), "output": "o.mp4"}));
        // a damaged file either is refused (and nothing is written) or is cut and verified
        match r {
            Ok(v) => assert_eq!(v["verified"], true, "{path}: {v}"),
            Err(e) => assert!(!files.0.lock().unwrap().contains_key("o.mp4"), "{path}: refused ({e}) yet o.mp4 was written"),
        }
        let _ = files.0.lock().unwrap().remove("o.mp4");
    }
}
