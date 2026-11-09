//! `export.analyze` / `export.plan` / `export.start` (Smart Cut of a source file, WP-04): a GOP-
//! boundary cut is copied, verified and written through the host; anything that would re-encode
//! or is invalid is refused before a file is written.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use cfa4_isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
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

/// 40 pictures of H.264 at 25 fps, an IDR every 10, no B-frames (PTS = picture number).
fn source_mp4() -> Vec<u8> {
    use cfa4_h264enc::{Encoder, EncoderConfig, PacketFormat, RateControl, YuvFrame};
    let (w, h) = (64usize, 48usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.keyint = 10;
    cfg.bframes = 0;
    cfg.rate = RateControl::Qp(30);
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.scenecut = false;
    cfg.threads = 1;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40 {
        let y: Vec<u8> = (0..w * h).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| ((k * 3 + i) % 255) as u8).collect();
        packets.extend(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }, i as i64).unwrap());
    }
    packets.extend(enc.flush());
    let entry = SampleEntry::avc(AvcConfig::parse(&enc.avcc()).unwrap(), w as u16, h as u16);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(entry, 25)).unwrap();
    for p in &packets {
        mux.write_sample(t, WriteSample { data: &p.data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

fn session() -> (Session, Arc<MemoryFiles>) {
    let files = Arc::new(MemoryFiles::default());
    files.0.lock().unwrap().insert("clip.mp4".into(), source_mp4());
    (Session::new(files.clone()), files)
}

fn frame(n: i64) -> Value {
    json!({"frame": n})
}

#[test]
fn analyze_reports_the_access_points_and_the_registry_decision() {
    let (mut s, _) = session();
    let r = s.execute("export.analyze", json!({"path": "clip.mp4"})).unwrap();
    assert_eq!(r["codec"], "h264");
    assert_eq!(r["frames"], 40);
    assert_eq!(r["frameRate"], json!({"num": 25, "den": 1}));
    let points: Vec<u64> = r["accessPoints"].as_array().unwrap().iter().map(|p| p["presentation"].as_u64().unwrap()).collect();
    assert_eq!(points, [0, 10, 20, 30]);
    assert!(r["accessPoints"].as_array().unwrap().iter().all(|p| p["clean"] == true));
    assert_eq!(r["smartCut"]["allowed"], true, "{}", r["smartCut"]);
}

#[test]
fn plan_names_the_safe_points_or_the_boundaries() {
    let (mut s, _) = session();
    let r = s.execute("export.plan", json!({"path": "clip.mp4", "in": frame(10), "out": frame(30)})).unwrap();
    assert_eq!(r["summary"], json!({"lossless": true, "requiresConfirmation": false, "copiedFrames": 20, "reencodedFrames": 0}));
    assert_eq!(r["plan"]["executable"], true);
    // mid-GOP in GOP-boundary mode: refused with the safe points
    let e = s.execute("export.plan", json!({"path": "clip.mp4", "in": frame(5), "out": frame(30)})).unwrap_err().to_string();
    assert!(e.contains("safe"), "{e}");
    // frame-accurate: the head is re-encoded and needs a confirmation
    let r = s.execute("export.plan", json!({"path": "clip.mp4", "in": frame(5), "out": frame(30), "mode": "frame-accurate"})).unwrap();
    assert_eq!(r["summary"]["requiresConfirmation"], true);
    assert_eq!(r["summary"]["reencodedFrames"], 5);
}

#[test]
fn start_copies_verifies_and_writes_the_cut() {
    let (mut s, files) = session();
    let source = files.0.lock().unwrap()["clip.mp4"].clone();
    let r = s.execute("export.start", json!({"path": "clip.mp4", "in": frame(10), "out": frame(30), "output": "cut.mp4"})).unwrap();
    assert_eq!(r["verified"], true);
    assert_eq!(r["report"]["samples"], 20);
    let files = files.0.lock().unwrap();
    assert_eq!(files["clip.mp4"], source, "the source is untouched");
    let out = cfa4_isobmff::open(files["cut.mp4"].as_slice()).unwrap();
    assert_eq!(out.tracks[0].samples.len(), 20);
    assert!(out.tracks[0].samples[0].is_sync);
    assert_eq!(r["bytes"], files["cut.mp4"].len());
}

#[test]
fn start_refuses_before_writing_anything() {
    let (mut s, files) = session();
    let run = |s: &mut Session, p: Value| s.execute("export.start", p).unwrap_err().to_string();
    let base = json!({"path": "clip.mp4", "in": frame(5), "out": frame(30), "output": "cut.mp4"});
    // a re-encoding plan: no boundary re-encoder yet, confirmed or not
    for confirm in [false, true] {
        let mut p = base.clone();
        p["mode"] = json!("frame-accurate");
        p["confirm"] = json!(confirm);
        assert!(run(&mut s, p).contains("WP-06"));
    }
    let ok = json!({"path": "clip.mp4", "in": frame(10), "out": frame(30)});
    assert!(run(&mut s, ok.clone()).contains("output"));
    let mut same = ok.clone();
    same["output"] = json!("clip.mp4");
    assert!(run(&mut s, same).contains("source"));
    let mut mkv = ok.clone();
    mkv["output"] = json!("cut.mkv");
    assert!(run(&mut s, mkv).contains(".mp4"));
    let mut bad_mode = ok.clone();
    bad_mode["output"] = json!("cut.mp4");
    bad_mode["mode"] = json!("fast");
    assert!(run(&mut s, bad_mode).contains("mode"));
    let mut timeline = ok.clone();
    timeline["output"] = json!("cut.mp4");
    timeline["scope"] = json!("timeline");
    assert!(run(&mut s, timeline).contains("WP-06"));
    assert!(run(&mut s, json!({"path": "missing.mp4", "in": 0, "out": 1, "output": "x.mp4"})).contains("missing.mp4"));
    assert_eq!(files.0.lock().unwrap().keys().collect::<Vec<_>>(), ["clip.mp4"], "nothing was written");
}
