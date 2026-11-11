//! `export.plan` / `export.start` with `"scope": "timeline"` (Smart Render of a sequence, WP-06):
//! unchanged H.264 clips are copied, their boundaries re-encoded, and gaps or changed ranges
//! rendered by the export pipeline into one verified MP4.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cfa4_isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_project::{ItemId, ParamValue, TrackKind};
use cfa4_time::{FrameRate, Tick, TimeRange};
use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::{session_with, tmp_dir};

const RATE: FrameRate = FrameRate { num: 25, den: 1 };

/// 40 pictures of H.264 (64×48, 25 fps, an IDR every 10, `bframes` B-frames) with pictures that
/// depend on `seed`.
fn source_mp4(seed: usize, bframes: u8) -> Vec<u8> {
    source_mp4_in(seed, bframes, 25)
}

/// [`source_mp4`] in `timescale` ticks per second (a multiple of 25).
fn source_mp4_in(seed: usize, bframes: u8, timescale: u32) -> Vec<u8> {
    use cfa4_h264enc::{Encoder, EncoderConfig, PacketFormat, RateControl, YuvFrame};
    let (w, h) = (64usize, 48usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.keyint = 10;
    cfg.bframes = bframes;
    cfg.rate = RateControl::Qp(30);
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.scenecut = false;
    cfg.threads = 1;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40 {
        let y: Vec<u8> = (0..w * h).map(|k| ((k + i * 7 + seed * 13) % 200 + 30) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| ((k * 3 + i) % 60 + 100) as u8).collect();
        packets.extend(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }, i as i64).unwrap());
    }
    packets.extend(enc.flush());
    let entry = SampleEntry::avc(AvcConfig::parse(&enc.avcc()).unwrap(), w as u16, h as u16);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let step = timescale / 25;
    let mut track = TrackConfig::new(entry, timescale);
    track.media_start = (bframes > 0).then_some(i64::from(step));
    let t = mux.add_track(track).unwrap();
    for p in &packets {
        let offset = (p.pts - p.dts) as i32 * step as i32;
        mux.write_sample(t, WriteSample { data: &p.data, duration: step, composition_offset: offset, is_sync: p.keyframe }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

struct Fixture {
    dir: PathBuf,
    s: Session,
    items: Vec<ItemId>,
    seq: ItemId,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn f(n: i64) -> Tick {
    RATE.tick_of(n)
}

/// Sources a.mp4 and b.mp4 imported, with the sequence's V1 replaced by `layout`:
/// (item index, sequence frame, source frame, frames).
fn fixture(name: &str, bframes: u8, layout: &[(usize, i64, i64, i64)]) -> Fixture {
    let dir = tmp_dir(name);
    let (a, b) = (dir.join("a.mp4"), dir.join("b.mp4"));
    std::fs::write(&a, source_mp4(0, bframes)).unwrap();
    std::fs::write(&b, source_mp4(1, bframes)).unwrap();
    let (mut s, items, seq) = session_with(&[a.as_path(), b.as_path()]);
    let mut p = (*s.project).clone();
    let mut v1 = Vec::new();
    for &(i, at, from, len) in layout {
        let mut it = p.make_track_item(items[i], TrackKind::Video, f(at), TimeRange::new(f(from), f(len)), RATE).unwrap();
        for e in &mut it.effects {
            cfa4_project::resolve_auto_points(e, (64, 48), (64, 48));
        }
        v1.push(it);
    }
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items = v1;
    q.audio_tracks[0].items.clear();
    s.project = Arc::new(p);
    Fixture { dir, s, items, seq }
}

impl Fixture {
    fn out(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }
    fn plan(&mut self) -> Value {
        self.s.execute("export.plan", json!({"scope": "timeline", "output": self.out("plan.mp4")})).unwrap()
    }
    fn start(&mut self, name: &str, confirm: bool) -> crate::Result<Value> {
        let output = self.out(name);
        self.s.execute("export.start", json!({"scope": "timeline", "output": output, "confirm": confirm}))
    }
}

/// (mode, reason) of every plan segment.
fn shape(plan: &Value) -> Vec<(String, String)> {
    plan["plan"]["plan"]["segments"].as_array().unwrap().iter().map(|s| (s["mode"].as_str().unwrap().into(), s["reason"].as_str().unwrap().into())).collect()
}

#[test]
fn unchanged_clips_are_copied_without_confirmation() {
    // a.mp4 whole, then b.mp4 frames 10–30
    let mut fx = fixture("smart-render-copy", 0, &[(0, 0, 0, 40), (1, 40, 10, 20)]);
    let plan = fx.plan();
    assert_eq!(plan["summary"]["lossless"], true, "{plan}");
    assert_eq!(plan["summary"]["copiedFrames"], 60);
    assert_eq!(plan["plan"]["executable"], true, "{plan}");
    assert_eq!(plan["timelineRender"]["ready"], true, "{plan}");
    let r = fx.start("copy.mp4", false).unwrap();
    assert_eq!(r["verified"], true);
    assert_eq!((r["report"]["copied"].as_u64(), r["report"]["rendered"].as_u64()), (Some(60), Some(0)), "{r}");
    let written = std::fs::read(fx.out("copy.mp4")).unwrap();
    let file = cfa4_isobmff::open(written.as_slice()).unwrap();
    assert_eq!(file.tracks[0].samples.len(), 60);
}

#[test]
fn trims_gaps_and_effects_need_confirmation_and_are_rendered() {
    // a.mp4 5–20 (a head boundary), a 5-frame gap, b.mp4 10–30 with its opacity at 50 %
    let mut fx = fixture("smart-render-mixed", 0, &[(0, 0, 5, 15), (1, 20, 10, 20)]);
    let mut p = (*fx.s.project).clone();
    let q = p.sequence_mut(fx.seq).unwrap();
    let op = q.video_tracks[0].items[1].effects.iter_mut().find(|e| e.effect == "opacity").unwrap();
    op.params.get_mut("opacity").unwrap().value = ParamValue::Float(50.0);
    fx.s.project = Arc::new(p);
    let plan = fx.plan();
    let segments = shape(&plan);
    assert!(segments.contains(&("boundary-reencode".into(), "head-boundary".into())), "{segments:?}");
    assert!(segments.contains(&("render".into(), "gap".into())), "{segments:?}");
    assert!(segments.contains(&("render".into(), "transformed".into())), "{segments:?}");
    let changed = plan["plan"]["plan"]["segments"].as_array().unwrap().iter().find(|s| s["reason"] == "transformed").unwrap();
    assert_eq!(changed["render_causes"], json!(["composite"]));
    assert_eq!(plan["summary"]["requiresConfirmation"], true);
    assert_eq!(plan["summary"]["renderedFrames"], 25, "{plan}");
    // nothing is written without "confirm"
    let e = fx.start("mixed.mp4", false).unwrap_err();
    assert!(e.to_string().contains("confirm"), "{e}");
    assert!(!Path::new(&fx.out("mixed.mp4")).exists());
    let r = fx.start("mixed.mp4", true).unwrap();
    assert_eq!(r["report"]["samples"], 40, "{r}");
    assert_eq!(r["report"]["rendered"], 25, "{r}");
    assert!(r["report"]["copied"].as_u64().unwrap() >= 10, "{r}");
}

#[test]
fn bad_parameters_are_refused() {
    let mut fx = fixture("smart-render-bad", 0, &[(0, 0, 0, 40)]);
    let e = fx.s.execute("export.plan", json!({"scope": "everything"})).unwrap_err();
    assert!(e.to_string().contains("timeline"), "{e}");
    let e = fx.s.execute("export.start", json!({"scope": "timeline"})).unwrap_err();
    assert!(e.to_string().contains("output"), "{e}");
    let e = fx.s.execute("export.plan", json!({"scope": "timeline", "output": "x.mkv"})).unwrap_err();
    assert!(e.to_string().contains(".mp4"), "{e}");
    let source = fx.dir.join("a.mp4").to_string_lossy().into_owned();
    let e = fx.s.execute("export.start", json!({"scope": "timeline", "output": source})).unwrap_err();
    assert!(e.to_string().contains("source"), "{e}");
    let _ = &fx.items;
}

/// FFmpeg as the oracle: the copied pictures of the export are the source pictures, at the right
/// sequence frames, with and without B-frames.
#[test]
fn ffmpeg_decodes_the_sequence_export() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("Smart Render sequence decode") else { return };
    for bframes in [0, 2] {
        // a.mp4 0–20, b.mp4 10–30, a 5-frame gap, a.mp4 5–17
        let mut fx = fixture("smart-render-ffmpeg", bframes, &[(0, 0, 0, 20), (1, 20, 10, 20), (0, 45, 5, 12)]);
        let r = fx.start("timeline.mp4", true).unwrap();
        assert_eq!(r["report"]["samples"], 57, "{r}");
        let hashes = |path: &str| -> Vec<String> {
            let o = Command::new(&ffmpeg).args(["-v", "error", "-xerror", "-i", path, "-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
            assert!(o.status.success() && o.stderr.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
        };
        let (ha, hb, ht) = (hashes(&fx.out("a.mp4")), hashes(&fx.out("b.mp4")), hashes(&fx.out("timeline.mp4")));
        assert_eq!(ht.len(), 57, "bframes {bframes}");
        assert_eq!(ht[0..20], ha[0..20], "bframes {bframes}: a.mp4 copied");
        assert_eq!(ht[20..40], hb[10..30], "bframes {bframes}: b.mp4 copied");
        assert_eq!(ht[50..57], ha[10..17], "bframes {bframes}: a.mp4 copied after its boundary");
        assert!(ht[40..45].iter().all(|h| *h == ht[40]), "bframes {bframes}: the gap is one picture");
    }
}

/// HEVC clips are cut by the source-scope commands (WP-07); the timeline executor copies H.264
/// only, so on a sequence they are rendered, with the `source` cause.
#[test]
fn hevc_clips_on_a_sequence_are_rendered() {
    use cfa4_hevc::encode::{Encoder, EncoderConfig, PacketFormat, YuvFrame};
    let dir = tmp_dir("smart-render-hevc");
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = 10;
    cfg.format = PacketFormat::LengthPrefixed;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..20usize {
        let y: Vec<u8> = (0..64 * 48).map(|k| ((k + i * 7) % 200 + 30) as u8).collect();
        let c = vec![128u8; 32 * 24];
        packets.extend(enc.encode(&YuvFrame::U8 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i as i64).unwrap());
    }
    let entry = SampleEntry::hevc(cfa4_isobmff::HevcConfig::parse(&enc.hvcc()).unwrap(), 64, 48);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(entry, 25)).unwrap();
    for p in &packets {
        mux.write_sample(t, WriteSample { data: &p.data, duration: 1, composition_offset: 0, is_sync: p.keyframe }).unwrap();
    }
    let path = dir.join("h.mp4");
    std::fs::write(&path, mux.finish().unwrap().into_inner()).unwrap();
    let (mut s, items, seq) = session_with(&[path.as_path()]);
    let mut p = (*s.project).clone();
    let mut it = p.make_track_item(items[0], TrackKind::Video, f(0), TimeRange::new(f(0), f(20)), RATE).unwrap();
    for e in &mut it.effects {
        cfa4_project::resolve_auto_points(e, (64, 48), (64, 48));
    }
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items = vec![it];
    q.audio_tracks[0].items.clear();
    s.project = Arc::new(p);
    let out = dir.join("plan.mp4").to_string_lossy().into_owned();
    let plan = s.execute("export.plan", json!({"scope": "timeline", "output": out})).unwrap();
    let segments = plan["plan"]["plan"]["segments"].as_array().unwrap();
    assert!(!segments.is_empty() && segments.iter().all(|s| s["mode"] == "render" && s["render_causes"] == json!(["source"])), "{plan}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The timeline's GOP overlay (WP-08): each clip's access points in sequence time, and whether it
/// starts on one.
#[test]
fn gop_marks_place_each_clips_access_points_on_the_sequence() {
    // a: source 10–29 at 0; b: source 5–24 at 20 (an IDR every 10 source frames)
    let mut fx = fixture("gop-marks", 0, &[(0, 0, 10, 20), (1, 20, 5, 20)]);
    let r = fx.s.execute("export.gopMarks", json!({})).unwrap();
    assert_eq!(r["sequence"], fx.seq.0);
    let clips = r["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 2, "{r}");
    let times = |c: &Value| c["points"].as_array().unwrap().iter().map(|p| p["time"].as_i64().unwrap()).collect::<Vec<_>>();
    assert_eq!(times(&clips[0]), [f(0).0, f(10).0]);
    assert_eq!(times(&clips[1]), [f(25).0, f(35).0]);
    assert_eq!((clips[0]["in"].as_bool(), clips[1]["in"].as_bool()), (Some(true), Some(false)));
    assert_eq!(clips[0]["codec"], "h264");
    assert!(clips.iter().all(|c| c["points"].as_array().unwrap().iter().all(|p| p["clean"] == true)));
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// [`source_mp4`] with a 48 kHz mono PCM tone as long as the video.
fn source_with_tone(seed: usize, bframes: u8) -> Vec<u8> {
    use cfa4_isobmff::PcmConfig;
    let video = source_mp4(seed, bframes);
    let v = cfa4_isobmff::open(video.as_slice()).unwrap();
    let vt = &v.tracks[0];
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let mut cfg = TrackConfig::new(vt.entries[0].clone(), vt.timescale);
    cfg.edits = vt.edits.clone();
    let t = mux.add_track(cfg).unwrap();
    for (i, s) in vt.samples.iter().enumerate() {
        let data = v.read_sample(video.as_slice(), 0, i).unwrap();
        mux.write_sample(t, WriteSample { data: &data, duration: s.duration, composition_offset: (s.pts - s.dts) as i32, is_sync: s.is_sync }).unwrap();
    }
    let pcm = PcmConfig { bits: 16, float: false, big_endian: false, signed: true, channels: 1, sample_rate: 48_000.0 };
    let p = mux.add_track(TrackConfig::new(SampleEntry::pcm(pcm), 48_000)).unwrap();
    let tone: Vec<u8> = (0..48_000 * 8 / 5).flat_map(|i: u32| (((i as f32 * 0.0577).sin() * 12_000.0) as i16).to_le_bytes()).collect();
    mux.write_sample(p, WriteSample { data: &tone, duration: 0, composition_offset: 0, is_sync: true }).unwrap();
    mux.finish().unwrap().into_inner()
}

#[test]
fn the_sequence_audio_is_mixed_encoded_and_presented_with_the_video() {
    // SCE-001 §9 (WP-09b): the timeline's audio is a mix, so it is encoded as AAC; its priming is
    // hidden and it lasts exactly as long as the copied video
    let dir = tmp_dir("smart-render-audio");
    let a = dir.join("a.mp4");
    std::fs::write(&a, source_with_tone(0, 2)).unwrap();
    let (mut s, items, seq) = session_with(&[a.as_path()]);
    let mut p = (*s.project).clone();
    let v = p.make_track_item(items[0], TrackKind::Video, f(0), TimeRange::new(f(0), f(40)), RATE).unwrap();
    let au = p.make_track_item(items[0], TrackKind::Audio, f(0), TimeRange::new(f(0), f(40)), RATE).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items = vec![v];
    q.audio_tracks[0].items = vec![au];
    s.project = Arc::new(p);
    let out = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let r = s.execute("export.start", json!({"scope": "timeline", "output": out("av.mp4")})).unwrap();
    assert_eq!(r["summary"]["lossless"], true, "{r}");
    let verified = r["audio"]["verified"].as_array().unwrap();
    assert_eq!(verified.len(), 1, "{r}");
    assert_eq!(verified[0]["codec"], "aac");
    assert!(verified[0]["syncErrorNs"].as_i64().unwrap().abs() <= 10_417, "{r}");
    let file = cfa4_isobmff::open(std::fs::read(out("av.mp4")).unwrap().as_slice()).unwrap();
    assert_eq!(file.tracks.len(), 2);
    let audio = &file.tracks[1];
    // presented: the edit list hides the priming and ends with the video (1.6 s)
    let presented: u64 = audio.edits.iter().filter(|e| e.media_time >= 0).map(|e| e.segment_duration).sum();
    assert_eq!(presented * 48_000 / u64::from(file.timescale), 76_800, "{:?} @ {}", audio.edits, file.timescale);
    if let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("Smart Render audio decode") {
        let o = Command::new(&ffmpeg).args(["-v", "error", "-i", &out("av.mp4"), "-map", "0:a", "-f", "s16le", "-ac", "1", "-"]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let samples: Vec<i16> = o.stdout.chunks(2).map(|x| i16::from_le_bytes([x[0], x[1]])).collect();
        assert!((76_800..76_800 + 1024).contains(&samples.len()), "{}", samples.len());
        // the tone is there from the first picture (no priming silence in front of it)
        let loud = |r: std::ops::Range<usize>| samples[r].iter().map(|x| i64::from(*x).abs()).max().unwrap();
        assert!(loud(0..480) > 3000, "the first 10 ms are the tone, not priming silence");
        assert!(loud(38_400..48_000) > 3000);
    }
    // "audio": false leaves the video alone
    let r = s.execute("export.start", json!({"scope": "timeline", "output": out("mute.mp4"), "audio": false})).unwrap();
    assert_eq!(r["audio"]["verified"], json!([]));
    let file = cfa4_isobmff::open(std::fs::read(out("mute.mp4")).unwrap().as_slice()).unwrap();
    assert_eq!(file.tracks.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WP-09d, case 9: clips whose sources count time in other timescales (25 and 12 800 ticks per
/// second) at the same frame rate are still copied, into one verified stream.
#[test]
fn clips_in_other_timescales_at_one_frame_rate_are_copied() {
    let dir = tmp_dir("smart-render-timescales");
    let (a, b) = (dir.join("a.mp4"), dir.join("b.mp4"));
    std::fs::write(&a, source_mp4_in(0, 2, 25)).unwrap();
    std::fs::write(&b, source_mp4_in(1, 2, 12_800)).unwrap();
    let (mut s, items, seq) = session_with(&[a.as_path(), b.as_path()]);
    let mut p = (*s.project).clone();
    let mut v1 = Vec::new();
    for (i, at, from, len) in [(0usize, 0i64, 0i64, 40i64), (1, 40, 10, 20)] {
        let mut it = p.make_track_item(items[i], TrackKind::Video, f(at), TimeRange::new(f(from), f(len)), RATE).unwrap();
        for e in &mut it.effects {
            cfa4_project::resolve_auto_points(e, (64, 48), (64, 48));
        }
        v1.push(it);
    }
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items = v1;
    q.audio_tracks[0].items.clear();
    s.project = Arc::new(p);
    let out = dir.join("mixed.mp4").to_string_lossy().into_owned();
    let plan = s.execute("export.plan", json!({"scope": "timeline", "output": out})).unwrap();
    assert_eq!(plan["summary"]["lossless"], true, "{plan}");
    assert_eq!(plan["summary"]["copiedFrames"], 60, "{plan}");
    let r = s.execute("export.start", json!({"scope": "timeline", "output": out})).unwrap();
    assert_eq!((r["report"]["copied"].as_u64(), r["report"]["rendered"].as_u64()), (Some(60), Some(0)), "{r}");
    let written = std::fs::read(&out).unwrap();
    let file = cfa4_isobmff::open(written.as_slice()).unwrap();
    assert_eq!(file.tracks[0].samples.len(), 60);
    let _ = std::fs::remove_dir_all(&dir);
}

/// WP-09d, case 19: an export runs on the project as it is when it starts. A plan made before an
/// edit is never reused: the edit shows in the next plan and run, and an edit that needs
/// rendering asks for the confirmation again.
#[test]
fn an_edit_after_the_plan_is_what_the_export_writes() {
    let mut fx = fixture("smart-render-edited", 0, &[(0, 0, 0, 40), (1, 40, 10, 20)]);
    let plan = fx.plan();
    assert_eq!(plan["summary"]["requiresConfirmation"], false, "{plan}");
    // the second clip's opacity changes after the plan was shown
    let mut p = (*fx.s.project).clone();
    let q = p.sequence_mut(fx.seq).unwrap();
    let op = q.video_tracks[0].items[1].effects.iter_mut().find(|e| e.effect == "opacity").unwrap();
    op.params.get_mut("opacity").unwrap().value = ParamValue::Float(50.0);
    fx.s.project = Arc::new(p);
    let e = fx.start("edited.mp4", false).unwrap_err();
    assert!(e.to_string().contains("confirm"), "{e}");
    assert!(!Path::new(&fx.out("edited.mp4")).exists(), "nothing is written from the stale plan");
    let r = fx.start("edited.mp4", true).unwrap();
    assert_eq!((r["report"]["copied"].as_u64(), r["report"]["rendered"].as_u64()), (Some(40), Some(20)), "{r}");
    // the export does not change the project it read
    let after = fx.s.project.sequence(fx.seq).unwrap().video_tracks[0].items.len();
    assert_eq!(after, 2);
}
