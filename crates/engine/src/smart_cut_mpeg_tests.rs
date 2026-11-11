//! WP-09c/e: `export.analyze`, `export.plan` and `export.start` with MPEG-2 program and transport
//! stream sources (lossless GOP copies with their audio, and frame-accurate cuts that re-encode the
//! boundary pictures).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use cfa4_mpegts::{Kind, MuxCodec, MuxFormat, Muxer};
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

const FRAME: i64 = 3600;

/// 40 pictures of MPEG-2 video (64×48, 25 fps, closed GOPs of 10, IBBP) and one audio stream over
/// the same span: 16-bit stereo LPCM in a program stream, MPEG-1 layer II in a transport stream.
fn source(format: MuxFormat) -> Vec<u8> {
    use cfa4_mpeg2v::encode::{Config, Encoder, Input};
    let (w, h) = (64usize, 48usize);
    let mut cfg = Config::new(w as u32, h as u32, 25, 1);
    cfg.gop = 10;
    cfg.b_frames = 2;
    cfg.threads = false;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40 {
        let y: Vec<u8> = (0..w * h).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| ((k * 3 + i) % 255) as u8).collect();
        packets.extend(enc.encode(&Input { y: &y, cb: &c, cr: &c, y_stride: w, c_stride: w / 2 }).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    let audio = if format == MuxFormat::Ps { MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2 } } else { MuxCodec::MpegAudio };
    let mut m = Muxer::new(Vec::new(), format, &[MuxCodec::Mpeg2Video, audio], 20_000_000).unwrap();
    for p in &packets {
        let (pts, dts) = (p.index as i64 * FRAME, p.dts * FRAME);
        m.write(0, &p.data, pts, (dts != pts).then_some(dts), p.keyframe).unwrap();
    }
    if format == MuxFormat::Ps {
        // 480 samples (900 ticks) per unit
        for k in 0..160i64 {
            let unit: Vec<u8> = (0..960i64).flat_map(|i| ((((i * 37 + k * 11) % 4000) - 2000) as i16).to_be_bytes()).collect();
            m.write(1, &unit, k * 900, None, true).unwrap();
        }
    } else {
        // MPEG-1 layer II, 48 kHz 192 kb/s: 1152 samples (2160 ticks), 576 bytes
        for k in 0..67i64 {
            let mut f = vec![0xFF, 0xFD, 0xA4, 0x00];
            f.extend((0..572).map(|i| (i as i64 + k * 3) as u8));
            m.write(1, &f, k * 2160, None, true).unwrap();
        }
    }
    m.finish().unwrap()
}

fn session() -> (Session, Arc<MemoryFiles>) {
    let files = Arc::new(MemoryFiles::default());
    files.0.lock().unwrap().insert("clip.mpg".into(), source(MuxFormat::Ps));
    files.0.lock().unwrap().insert("clip.ts".into(), source(MuxFormat::Ts));
    (Session::new(files.clone()), files)
}

fn frame(n: i64) -> Value {
    json!({"frame": n})
}

#[test]
fn mpeg_sources_are_analyzed_with_their_gops() {
    let (mut s, _) = session();
    for (path, container) in [("clip.mpg", "mpeg-ps"), ("clip.ts", "mpeg-ts")] {
        let r = s.execute("export.analyze", json!({"path": path})).unwrap();
        assert_eq!(r["codec"], "mpeg2-video", "{r}");
        assert_eq!(serde_json::to_value(r["container"].clone()).unwrap(), json!(container), "{r}");
        assert_eq!(r["frames"], 40);
        assert_eq!(r["frameRate"], json!({"num": 90000, "den": 3600}));
        assert_eq!(r["gops"], 4);
        let media: Vec<i64> = r["accessPoints"].as_array().unwrap().iter().map(|p| p["mediaTime"].as_i64().unwrap()).collect();
        assert_eq!(media, [0, 10, 20, 30].map(|n| cfa4_time::FrameRate { num: 25, den: 1 }.tick_of(n).0), "{r}");
        assert_eq!(r["smartCut"]["allowed"], true, "{}", r["smartCut"]);
    }
}

#[test]
fn a_gop_cut_of_an_mpeg_source_is_copied_with_its_audio_and_verified() {
    let (mut s, files) = session();
    for (path, out) in [("clip.mpg", "cut.mpg"), ("clip.ts", "cut.ts")] {
        let p = json!({"path": path, "in": frame(10), "out": frame(30), "output": out});
        let r = s.execute("export.plan", p.clone()).unwrap();
        assert_eq!(r["summary"], json!({"lossless": true, "requiresConfirmation": false, "copiedFrames": 20, "reencodedFrames": 0}), "{r}");
        assert_eq!(r["audio"]["tracks"].as_array().unwrap().len(), 1, "{r}");
        let r = s.execute("export.start", p).unwrap();
        assert_eq!(r["verified"], true);
        assert_eq!(r["report"]["samples"], 20);
        assert_eq!(r["audio"]["tracks"].as_array().unwrap().len(), 1, "{r}");
        let bytes = files.0.lock().unwrap()[out].clone();
        assert_eq!(r["bytes"], bytes.len());
        let f = cfa4_mpegts::open(&bytes).unwrap();
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert_eq!(f.streams.iter().filter(|s| s.kind() == Kind::Video).map(|s| s.units.len()).sum::<usize>(), 20);
        assert_eq!(f.streams.iter().filter(|s| s.kind() == Kind::Audio).count(), 1);
    }
    // the video alone, and an M2TS output of the transport stream
    let r = s.execute("export.start", json!({"path": "clip.ts", "in": frame(0), "out": frame(20), "output": "v.m2ts", "audio": false})).unwrap();
    assert_eq!(r["audio"]["tracks"], json!([]));
    let bytes = files.0.lock().unwrap()["v.m2ts"].clone();
    let f = cfa4_mpegts::open(&bytes).unwrap();
    assert!(matches!(f.format, cfa4_mpegts::Format::Ts { packet_size: 192, .. }));
    assert!(f.streams.iter().all(|s| s.kind() != Kind::Audio));
}

#[test]
fn mpeg_cuts_the_executor_cannot_run_are_refused_before_writing() {
    let (mut s, files) = session();
    let run = |s: &mut Session, p: Value| s.execute("export.start", p).unwrap_err().to_string();
    // frame accurate: nothing is re-encoded without "confirm"
    let p = json!({"path": "clip.ts", "in": frame(5), "out": frame(30), "mode": "frame-accurate", "output": "x.ts"});
    assert!(run(&mut s, p).contains("\"confirm\": true"));
    // an MP4 or Matroska output: MPEG-2 is copied into program and transport streams only
    assert!(run(&mut s, json!({"path": "clip.ts", "in": frame(10), "out": frame(30), "output": "x.mp4"})).contains(".ts"));
    assert!(run(&mut s, json!({"path": "clip.ts", "in": frame(10), "out": frame(30), "output": "x.mkv"})).contains(".ts"));
    // mid-GOP in GOP-boundary mode
    assert!(run(&mut s, json!({"path": "clip.mpg", "in": frame(5), "out": frame(30), "output": "x.mpg"})).contains("safe"));
    assert_eq!(files.0.lock().unwrap().len(), 2, "nothing was written");
    // a transport stream into a program stream: the same elementary stream, checked the same way
    let r = s.execute("export.start", json!({"path": "clip.ts", "in": frame(10), "out": frame(30), "output": "x.mpg"})).unwrap();
    assert_eq!((r["verified"].as_bool(), r["report"]["samples"].as_u64()), (Some(true), Some(20)), "{r}");
    let f = cfa4_mpegts::open(&files.0.lock().unwrap()["x.mpg"]).unwrap();
    assert!(matches!(f.format, cfa4_mpegts::Format::Ps { .. }));
}

/// The Source monitor's MPEG clip: media time 0 is the first presented picture.
#[test]
fn a_project_mpeg_item_is_cut_on_its_marked_range() {
    let dir = crate::media_test_util::tmp_dir("smart-cut-mpeg-item");
    let path = dir.join("clip.mpg");
    std::fs::write(&path, source(MuxFormat::Ps)).unwrap();
    let (mut s, items, _) = crate::media_test_util::session_with(&[path.as_path()]);
    let item = items[0];
    let rate = cfa4_time::FrameRate { num: 25, den: 1 };
    let r = s.execute("export.analyze", json!({"item": item.0})).unwrap();
    let media: Vec<i64> = r["accessPoints"].as_array().unwrap().iter().map(|p| p["mediaTime"].as_i64().unwrap()).collect();
    assert_eq!(media, [0, 10, 20, 30].map(|n| rate.tick_of(n).0), "{r}");
    let mut p = (*s.project).clone();
    if let Some(cfa4_project::ItemKind::Media(m)) = p.item_mut(item).map(|i| &mut i.kind) {
        (m.mark_in, m.mark_out) = (Some(rate.tick_of(20)), Some(rate.tick_of(39)));
    }
    s.project = Arc::new(p);
    let out = dir.join("cut.mpg").to_string_lossy().into_owned();
    let r = s.execute("export.start", json!({"item": item.0, "output": out})).unwrap();
    assert_eq!((r["verified"].as_bool(), r["report"]["samples"].as_u64()), (Some(true), Some(20)), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Frame accurate: the whole GOPs are copied, the boundary pictures re-encoded with CFA4's MPEG-2
/// encoder, the result verified (decoded, PSNR measured) and the audio of the cut copied along.
#[test]
fn a_frame_accurate_mpeg_cut_re_encodes_its_boundaries_once_confirmed() {
    let (mut s, files) = session();
    for (path, output) in [("clip.ts", "fa.ts"), ("clip.mpg", "fa.mpg")] {
        let p = json!({"path": path, "in": frame(5), "out": frame(33), "mode": "frame-accurate", "output": output, "confirm": true});
        let r = s.execute("export.plan", p.clone()).unwrap();
        assert_eq!(r["boundaryReencode"], json!({"ready": true, "problem": null}), "{r}");
        assert_eq!(r["summary"]["requiresConfirmation"], true, "{r}");
        assert_eq!(r["audio"]["tracks"].as_array().unwrap().len(), 1, "{r}");
        let r = s.execute("export.start", p).unwrap();
        assert_eq!(r["verified"], true, "{r}");
        let report = &r["report"];
        assert_eq!(report["samples"], 28, "{r}");
        assert!(report["reencoded"].as_u64().unwrap() > 0 && report["copied"].as_u64().unwrap() >= 20, "{r}");
        assert!(report["minPsnr"].as_f64().unwrap() >= 30.0, "{r}");
        assert_eq!(r["audio"]["tracks"].as_array().unwrap().len(), 1, "{r}");
        // the output is a stream Smart Cut reads again: 28 pictures, from an access point
        let back = s.execute("export.analyze", json!({"path": output})).unwrap();
        assert_eq!((back["codec"].as_str(), back["frames"].as_u64()), (Some("mpeg2-video"), Some(28)), "{back}");
        assert!(files.0.lock().unwrap().contains_key(output));
    }
    // `crf` sets the quantiser of the re-encoded pictures: a coarser one measures lower
    let fine = s.execute("export.plan", json!({"path": "clip.ts", "in": frame(5), "out": frame(33), "mode": "frame-accurate", "output": "c.ts"})).unwrap();
    assert_eq!(fine["boundaryReencode"]["ready"], true);
    let psnr = |s: &mut Session, crf: f64| {
        let p = json!({"path": "clip.ts", "in": frame(5), "out": frame(33), "mode": "frame-accurate", "output": "c.ts", "confirm": true, "crf": crf});
        s.execute("export.start", p).unwrap()["report"]["minPsnr"].as_f64().unwrap()
    };
    assert!(psnr(&mut s, 4.0) > psnr(&mut s, 51.0));
}
