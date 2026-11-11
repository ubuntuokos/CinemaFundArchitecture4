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
    source_mp4_of(40)
}

/// `n` pictures of the clip of [`source_mp4`].
fn source_mp4_of(n: usize) -> Vec<u8> {
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
    for i in 0..n {
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
    assert_eq!(r["boundaryReencode"], json!({"ready": true, "problem": null}));
    assert!(s.execute("export.plan", json!({"path": "clip.mp4", "in": frame(5), "out": frame(30), "crf": 60})).unwrap_err().to_string().contains("crf"));
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
    // a re-encoding plan runs only when confirmed
    let mut p = base.clone();
    p["mode"] = json!("frame-accurate");
    assert!(run(&mut s, p.clone()).contains("confirm"));
    p["confirm"] = json!(false);
    assert!(run(&mut s, p).contains("confirm"));
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
    // the timeline scope exports a sequence (smart_render_tests); this session has none
    assert!(run(&mut s, timeline).to_lowercase().contains("sequence"));
    assert!(run(&mut s, json!({"path": "missing.mp4", "in": 0, "out": 1, "output": "x.mp4"})).contains("missing.mp4"));
    assert_eq!(files.0.lock().unwrap().keys().collect::<Vec<_>>(), ["clip.mp4"], "nothing was written");
}

#[test]
fn start_re_encodes_the_boundaries_of_a_confirmed_frame_accurate_cut() {
    let (mut s, files) = session();
    let source = files.0.lock().unwrap()["clip.mp4"].clone();
    let p = json!({"path": "clip.mp4", "in": frame(5), "out": frame(30), "output": "cut.mp4", "mode": "frame-accurate", "confirm": true});
    let r = s.execute("export.start", p).unwrap();
    assert_eq!(r["verified"], true);
    let counts = (r["report"]["samples"].as_u64(), r["report"]["copied"].as_u64(), r["report"]["reencoded"].as_u64());
    assert_eq!(counts, (Some(25), Some(20), Some(5)), "{}", r["report"]);
    assert!(r["report"]["minPsnr"].as_f64().is_some_and(|db| db >= 30.0), "{}", r["report"]);
    let files = files.0.lock().unwrap();
    assert_eq!(files["clip.mp4"], source, "the source is untouched");
    let out = cfa4_isobmff::open(files["cut.mp4"].as_slice()).unwrap();
    let sync: Vec<usize> = out.tracks[0].samples.iter().enumerate().filter(|(_, x)| x.is_sync).map(|(i, _)| i).collect();
    assert_eq!(sync, [0, 5, 15]);
}

/// 40 pictures of HEVC from the CFA4 encoder (64×48, 25 fps, an IDR every 10, P pictures).
fn hevc_mp4() -> Vec<u8> {
    use cfa4_hevc::encode::{Encoder, EncoderConfig, PacketFormat, RateControl, YuvFrame};
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = 10;
    cfg.rate = RateControl::Qp(30);
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.slices = 1;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40usize {
        let y: Vec<u8> = (0..64 * 48).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..32 * 24).map(|k| ((k * 3 + i) % 255) as u8).collect();
        packets.extend(enc.encode(&YuvFrame::U8 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i as i64).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    let entry = SampleEntry::hevc(cfa4_isobmff::HevcConfig::parse(&enc.hvcc()).unwrap(), 64, 48);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(entry, 25)).unwrap();
    for p in &packets {
        mux.write_sample(t, WriteSample { data: &p.data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

/// WP-07: the same commands cut HEVC sources: a copy on IDR pictures, and a confirmed
/// frame-accurate cut whose head is re-encoded with the CFA4 HEVC encoder.
#[test]
fn hevc_sources_are_analyzed_copied_and_re_encoded_at_the_boundaries() {
    let (mut s, files) = session();
    files.0.lock().unwrap().insert("hevc.mp4".into(), hevc_mp4());
    let r = s.execute("export.analyze", json!({"path": "hevc.mp4"})).unwrap();
    assert_eq!(r["codec"], "hevc");
    assert_eq!(r["smartCut"]["allowed"], true, "{}", r["smartCut"]);
    let points: Vec<u64> = r["accessPoints"].as_array().unwrap().iter().map(|p| p["presentation"].as_u64().unwrap()).collect();
    assert_eq!(points, [0, 10, 20, 30]);
    let r = s.execute("export.start", json!({"path": "hevc.mp4", "in": frame(10), "out": frame(30), "output": "copy.mp4"})).unwrap();
    assert_eq!((r["verified"].as_bool(), r["report"]["samples"].as_u64()), (Some(true), Some(20)), "{r}");
    let r = s.execute("export.plan", json!({"path": "hevc.mp4", "in": frame(5), "out": frame(30), "mode": "frame-accurate"})).unwrap();
    assert_eq!(r["boundaryReencode"], json!({"ready": true, "problem": null}), "{r}");
    let p = json!({"path": "hevc.mp4", "in": frame(5), "out": frame(30), "output": "cut.mp4", "mode": "frame-accurate", "confirm": true});
    let r = s.execute("export.start", p).unwrap();
    let counts = (r["report"]["samples"].as_u64(), r["report"]["copied"].as_u64(), r["report"]["reencoded"].as_u64());
    assert_eq!(counts, (Some(25), Some(20), Some(5)), "{}", r["report"]);
    let files = files.0.lock().unwrap();
    let out = cfa4_isobmff::open(files["cut.mp4"].as_slice()).unwrap();
    assert!(matches!(out.tracks[0].codec(), Some(cfa4_isobmff::CodecConfig::Hevc(_))));
}

/// 40 synthetic AV1 temporal units (64×48, 25 fps, a shown key frame every 10, the sequence
/// header in `av1C`): enough for the index and the copy, not for a decoder.
fn av1_mp4() -> Vec<u8> {
    let obu = |kind: u8, payload: &[u8]| {
        let mut out = vec![(kind << 3) | 0x02, payload.len() as u8];
        out.extend_from_slice(payload);
        out
    };
    let cfg =
        cfa4_isobmff::Av1Config { chroma_subsampling_x: true, chroma_subsampling_y: true, config_obus: obu(1, &[0x00, 0x01, 0x80]), ..Default::default() };
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(SampleEntry::av1(cfg, 64, 48), 25)).unwrap();
    for i in 0..40u8 {
        let key = i % 10 == 0;
        let data = obu(6, &[(u8::from(!key) << 5) | 0x10, i, 0x55]);
        mux.write_sample(t, WriteSample { data: &data, duration: 1, composition_offset: 0, is_sync: key }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

#[test]
fn av1_sources_are_analyzed_and_copied_but_not_re_encoded() {
    let (mut s, files) = session();
    files.0.lock().unwrap().insert("av1.mp4".into(), av1_mp4());
    let r = s.execute("export.analyze", json!({"path": "av1.mp4"})).unwrap();
    assert_eq!(r["codec"], "av1");
    assert_eq!(r["smartCut"]["allowed"], true, "{}", r["smartCut"]);
    let points: Vec<u64> = r["accessPoints"].as_array().unwrap().iter().map(|p| p["presentation"].as_u64().unwrap()).collect();
    assert_eq!(points, [0, 10, 20, 30]);
    assert!(r["accessPoints"].as_array().unwrap().iter().all(|p| p["kind"] == "key-frame" && p["clean"] == true), "{}", r["accessPoints"]);
    for output in ["copy.mp4", "copy.mov"] {
        let r = s.execute("export.start", json!({"path": "av1.mp4", "in": frame(10), "out": frame(30), "output": output})).unwrap();
        assert_eq!((r["verified"].as_bool(), r["report"]["samples"].as_u64()), (Some(true), Some(20)), "{r}");
    }
    {
        let files = files.0.lock().unwrap();
        let out = cfa4_isobmff::open(files["copy.mov"].as_slice()).unwrap();
        assert!(out.is_quicktime);
        assert!(matches!(out.tracks[0].codec(), Some(cfa4_isobmff::CodecConfig::Av1(_))));
    }
    // CFA4 has no AV1 encoder: a frame-accurate cut off the key frames is not offered
    let r = s.execute("export.plan", json!({"path": "av1.mp4", "in": frame(5), "out": frame(30), "mode": "frame-accurate"}));
    let text = match r {
        Ok(v) => v.to_string(),
        Err(e) => e.to_string(),
    };
    assert!(text.contains("AV1"), "{text}");
    assert!(!files.0.lock().unwrap().contains_key("cut.mp4"));
}

/// The Source monitor's clip as the source: a project media item, its marked range by default and
/// media times (WP-08, the Source monitor's Smart Cut).
#[test]
fn a_project_media_item_is_cut_on_its_marked_range() {
    let dir = crate::media_test_util::tmp_dir("smart-cut-item");
    let path = dir.join("clip.mp4");
    std::fs::write(&path, source_mp4()).unwrap();
    let (mut s, items, seq) = crate::media_test_util::session_with(&[path.as_path()]);
    let item = items[0];
    let rate = cfa4_time::FrameRate { num: 25, den: 1 };
    let r = s.execute("export.analyze", json!({"item": item.0})).unwrap();
    assert_eq!(r["item"], item.0);
    let media: Vec<i64> = r["accessPoints"].as_array().unwrap().iter().map(|p| p["mediaTime"].as_i64().unwrap()).collect();
    assert_eq!(media, [0, 10, 20, 30].map(|n| rate.tick_of(n).0));
    // In at 10, Out (inclusive) at 29: the marked range is copied
    let mut p = (*s.project).clone();
    if let Some(cfa4_project::ItemKind::Media(m)) = p.item_mut(item).map(|i| &mut i.kind) {
        (m.mark_in, m.mark_out) = (Some(rate.tick_of(10)), Some(rate.tick_of(29)));
    }
    s.project = Arc::new(p);
    let out = dir.join("cut.mp4").to_string_lossy().into_owned();
    let r = s.execute("export.plan", json!({"item": item.0})).unwrap();
    assert_eq!(r["summary"]["copiedFrames"], 20, "{r}");
    let r = s.execute("export.start", json!({"item": item.0, "output": out})).unwrap();
    assert_eq!((r["verified"].as_bool(), r["report"]["samples"].as_u64()), (Some(true), Some(20)), "{r}");
    // explicit media times win over the marks
    let r = s.execute("export.plan", json!({"item": item.0, "in": frame(0), "out": frame(10)})).unwrap();
    assert_eq!(r["summary"]["copiedFrames"], 10, "{r}");
    // a sequence is no Smart Cut source
    let e = s.execute("export.analyze", json!({"item": seq.0})).unwrap_err();
    assert!(e.to_string().contains("media file"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// [`source_mp4`] with an AAC track (stand-in packets of 1024 samples at 48 kHz, 2112 samples of
/// priming hidden by its edit list) and a 16-bit mono PCM track, both as long as the video.
fn av_mp4() -> Vec<u8> {
    use cfa4_isobmff::PcmConfig;
    let video = source_mp4();
    let v = cfa4_isobmff::open(video.as_slice()).unwrap();
    let vt = &v.tracks[0];
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(vt.entries[0].clone(), vt.timescale)).unwrap();
    for (i, s) in vt.samples.iter().enumerate() {
        let data = v.read_sample(video.as_slice(), 0, i).unwrap();
        mux.write_sample(t, WriteSample { data: &data, duration: s.duration, composition_offset: (s.pts - s.dts) as i32, is_sync: s.is_sync }).unwrap();
    }
    let mut aac = TrackConfig::new(SampleEntry::aac(vec![0x11, 0x90], 2, 48_000), 48_000);
    aac.media_start = Some(2112);
    let a = mux.add_track(aac).unwrap();
    for i in 0..(48_000 * 8 / 5 + 2112) / 1024 + 1 {
        let data: Vec<u8> = (0..200).map(|k| ((i * 31 + k) % 256) as u8).collect();
        mux.write_sample(a, WriteSample { data: &data, duration: 1024, composition_offset: 0, is_sync: true }).unwrap();
    }
    let pcm = PcmConfig { bits: 16, float: false, big_endian: false, signed: true, channels: 1, sample_rate: 48_000.0 };
    let p = mux.add_track(TrackConfig::new(SampleEntry::pcm(pcm), 48_000)).unwrap();
    let samples: Vec<u8> = (0..48_000 * 8 / 5).flat_map(|i: u32| ((i % 30_000) as i16).to_le_bytes()).collect();
    mux.write_sample(p, WriteSample { data: &samples, duration: 0, composition_offset: 0, is_sync: true }).unwrap();
    mux.finish().unwrap().into_inner()
}

#[test]
fn the_audio_tracks_are_copied_with_the_cut_and_in_sync() {
    // SCE-001 §9 (WP-09b): AAC and PCM are copied next to the video, verified, and named in the plan
    let (mut s, files) = session();
    files.0.lock().unwrap().insert("av.mp4".into(), av_mp4());
    let r = s.execute("export.plan", json!({"path": "av.mp4", "in": frame(10), "out": frame(30)})).unwrap();
    assert_eq!(r["audio"]["tracks"], json!([{"trackId": 2, "codec": "aac"}, {"trackId": 3, "codec": "pcm"}]), "{}", r["audio"]);
    assert_eq!(r["audio"]["dropped"], json!([]));
    for (mode, extra) in [("gop-boundary", json!({"in": frame(10)})), ("frame-accurate", json!({"in": frame(5), "confirm": true}))] {
        let mut p = json!({"path": "av.mp4", "out": frame(30), "mode": mode, "output": format!("{mode}.mp4")});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let r = s.execute("export.start", p).unwrap();
        let verified = r["audio"]["verified"].as_array().unwrap();
        assert_eq!(verified.len(), 2, "{mode}: {}", r["audio"]);
        assert!(verified.iter().all(|t| t["syncErrorNs"].as_i64().unwrap().abs() <= 10_417), "{mode}: {}", r["audio"]);
        let out = cfa4_isobmff::open(files.0.lock().unwrap()[&format!("{mode}.mp4")].as_slice()).unwrap();
        assert_eq!(out.tracks.len(), 3, "{mode}");
        assert_eq!(out.tracks[2].samples.iter().map(|x| u64::from(x.duration)).sum::<u64>(), if mode == "gop-boundary" { 38_400 } else { 48_000 }, "{mode}");
    }
    // "audio": false writes the video alone
    let r = s.execute("export.start", json!({"path": "av.mp4", "in": frame(10), "out": frame(30), "output": "mute.mp4", "audio": false})).unwrap();
    assert_eq!(r["audio"]["tracks"], json!([]));
    assert_eq!(cfa4_isobmff::open(files.0.lock().unwrap()["mute.mp4"].as_slice()).unwrap().tracks.len(), 1);
}

#[test]
fn repeated_exports_of_one_source_are_identical_and_follow_a_changed_source() {
    // SCE-001 §13 edge cases 21–23 (WP-09): the same request writes the same bytes, and a source
    // replaced on disk is analyzed again instead of being cut from a stale index
    let (mut s, files) = session();
    let requests = [
        ("gop-boundary", json!({"path": "clip.mp4", "in": frame(10), "out": frame(30)})),
        ("frame-accurate", json!({"path": "clip.mp4", "in": frame(5), "out": frame(30), "mode": "frame-accurate", "confirm": true})),
    ];
    for (mode, mut p) in requests {
        let mut outputs = Vec::new();
        for n in 0..2 {
            p["output"] = json!(format!("{mode}-{n}.mp4"));
            let r = s.execute("export.start", p.clone()).unwrap();
            assert_eq!(r["verified"], true, "{mode}");
            outputs.push(files.0.lock().unwrap()[&format!("{mode}-{n}.mp4")].clone());
        }
        assert_eq!(outputs[0], outputs[1], "{mode}: a repeated export writes the same bytes");
    }
    // the source shrinks to its first 20 pictures: the next cut sees only those
    files.0.lock().unwrap().insert("clip.mp4".into(), source_mp4_of(20));
    let a = s.execute("export.analyze", json!({"path": "clip.mp4"})).unwrap();
    assert_eq!(a["frames"], 20);
    let err = s.execute("export.start", json!({"path": "clip.mp4", "in": frame(10), "out": frame(30), "output": "late.mp4"})).unwrap_err().to_string();
    assert!(!files.0.lock().unwrap().contains_key("late.mp4"), "nothing is written for a range past the new end: {err}");
}
