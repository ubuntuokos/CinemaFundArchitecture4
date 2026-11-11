//! WP-09c: Smart Cut of MPEG-2 video in program and transport streams copies the planned pictures
//! and the audio frames of the same span unchanged, and the output reads back with the same
//! payloads and the same audio offset against the first picture.

use std::process::Command;

use cfa4_mpegts::{Codec as TsCodec, File, Kind, MuxCodec, MuxFormat, Muxer};
use cfa4_smart_edit::capabilities::{Container, Policy, Registry};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::media_index::{MediaIndex, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::CopyGuarantee;
use cfa4_smart_edit::timestamp::{MediaTime, Rational};

use crate::smart_cut::SmartCutError;
use crate::smart_cut_mpeg::{MpegCopySource, container_of, execute_mpeg_copy, index_mpeg_video, plan_mpeg_audio, verify_mpeg_copy};

const STRICT: Policy = Policy { allow_experimental: false };
/// One picture at 25 fps, 90 kHz.
const FRAME: i64 = 3600;
/// The source's first presentation time (a stream rarely starts at zero).
const START: i64 = 10 * FRAME;

/// ADTS AAC-LC frame, 48 kHz stereo (1024 samples, 1920 ticks), 100 bytes.
fn adts_frame(k: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0xF1, 0x4C, 0x80, 0x0C, 0x9F, 0xFC];
    v.extend((0..93).map(|i| (i + k * 7) as u8));
    v
}

/// MPEG-1 layer II frame, 48 kHz 192 kb/s (1152 samples, 2160 ticks), 576 bytes.
fn mp2_frame(k: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFD, 0xA4, 0x00];
    v.extend((0..572).map(|i| (i + k * 3) as u8));
    v
}

/// AC-3 frame, 48 kHz 64 kb/s stereo (1536 samples, 2880 ticks), 256 bytes.
fn ac3_frame(k: usize) -> Vec<u8> {
    let mut v = vec![0x0B, 0x77, 0, 0, 0x08, 0x40, 0x40];
    v.extend((7..256).map(|i| (i + k * 5) as u8));
    v
}

/// 16-bit stereo 48 kHz LPCM, 480 samples (900 ticks) per unit, big-endian.
fn lpcm_unit(k: usize) -> Vec<u8> {
    (0..480 * 2).flat_map(|i| (((i * 37 + k * 11) % 4000) as i16 - 2000).to_be_bytes()).collect()
}

/// MPEG-2 video pictures (64×48, 25 fps, closed GOPs of 10, IBBP) in decode order, with their PTS
/// and DTS (90 kHz, from [`START`]).
fn pictures(frames: usize) -> Vec<(Vec<u8>, i64, i64, bool)> {
    use cfa4_mpeg2v::encode::{Config, Encoder, Input};
    let (w, h) = (64usize, 48usize);
    let mut cfg = Config::new(w as u32, h as u32, 25, 1);
    cfg.gop = 10;
    cfg.b_frames = 2;
    cfg.threads = false;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..frames {
        let y: Vec<u8> = (0..w * h).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| ((k * 3 + i) % 255) as u8).collect();
        packets.extend(enc.encode(&Input { y: &y, cb: &c, cr: &c, y_stride: w, c_stride: w / 2 }).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    packets.into_iter().map(|p| (p.data, START + p.index as i64 * FRAME, START + p.dts * FRAME, p.keyframe)).collect()
}

/// A source stream: MPEG-2 video and `audio` (the muxer codec, its units and their PTS).
fn source(format: MuxFormat, frames: usize, audio: &[(MuxCodec, Vec<(Vec<u8>, i64)>)]) -> Vec<u8> {
    let mut codecs = vec![MuxCodec::Mpeg2Video];
    codecs.extend(audio.iter().map(|a| a.0));
    let mut m = Muxer::new(Vec::new(), format, &codecs, 20_000_000).unwrap();
    for (data, pts, dts, key) in pictures(frames) {
        m.write(0, &data, pts, (dts != pts).then_some(dts), key).unwrap();
    }
    for (k, (_, units)) in audio.iter().enumerate() {
        for (data, pts) in units {
            m.write(k + 1, data, *pts, None, true).unwrap();
        }
    }
    m.finish().unwrap()
}

/// `count` units of `frame`, one every `step` ticks from `from`.
fn units(frame: fn(usize) -> Vec<u8>, count: usize, from: i64, step: i64) -> Vec<(Vec<u8>, i64)> {
    (0..count).map(|k| (frame(k), from + k as i64 * step)).collect()
}

struct Fixture {
    bytes: Vec<u8>,
    file: File,
    video: usize,
    index: StreamIndex,
}

impl Fixture {
    fn new(bytes: Vec<u8>) -> Fixture {
        let file = cfa4_mpegts::open(&bytes).unwrap();
        assert!(file.warnings.is_empty(), "{:?}", file.warnings);
        let video = file.find(Kind::Video, |c| *c == TsCodec::Mpeg2Video).unwrap();
        let index = index_mpeg_video(&file, &bytes, video).unwrap();
        Fixture { bytes, file, video, index }
    }
    fn source(&self) -> MpegCopySource<'_, Vec<u8>> {
        MpegCopySource { file: &self.file, bytes: &self.bytes, video: self.video, audio: true }
    }
    /// Plan pictures `[from, to)` (display order) into the source's own container.
    fn plan(&self, from: i64, to: i64, mode: CutMode) -> SmartCutPlan {
        let container = container_of(&self.file);
        // the muxer delays the whole source: picture 0 is at the indexed first PTS
        let pts0 = self.index.frames.iter().filter_map(|f| f.pts).min().unwrap();
        let tick = |f: i64| MediaTime::new(pts0 + f * FRAME, Rational::new(1, 90_000)).to_tick().unwrap();
        let req = SmartCutRequest {
            stream_id: self.index.stream_id,
            in_point: tick(from),
            out_point: tick(to),
            mode,
            guarantee: CopyGuarantee::BitExact,
            source_container: container,
            output_container: container,
            format: None,
        };
        let media =
            MediaIndex::new("test.mpg", SourceFingerprint { size: self.bytes.len() as u64, modified_ns: None, content_id: None }, vec![self.index.clone()]);
        plan_smart_cut(&Registry::native(), &media, &req, STRICT).unwrap()
    }
}

fn format_of(file: &File) -> MuxFormat {
    if container_of(file) == Container::MpegPs { MuxFormat::Ps } else { MuxFormat::Ts }
}

/// The transport stream of the tests: video, AAC and MP2 audio over the whole span.
fn transport() -> Fixture {
    let audio = [(MuxCodec::AacAdts, units(adts_frame, 80, START, 1920)), (MuxCodec::MpegAudio, units(mp2_frame, 70, START + 500, 2160))];
    Fixture::new(source(MuxFormat::Ts, 40, &audio))
}

/// The program stream of the tests: video, LPCM and AC-3 audio.
fn program() -> Fixture {
    let audio = [
        (MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2, bits: 16 }, units(lpcm_unit, 160, START, 900)),
        (MuxCodec::Ac3, units(ac3_frame, 50, START + 1000, 2880)),
    ];
    Fixture::new(source(MuxFormat::Ps, 40, &audio))
}

#[test]
fn the_registry_allows_mpeg2_copies_into_program_and_transport_streams() {
    for f in [transport(), program()] {
        let p = f.plan(10, 30, CutMode::GopBoundary);
        assert!(p.executable, "{:?}", p.blockers);
        assert_eq!(p.plan.source_container, container_of(&f.file));
    }
}

#[test]
fn a_gop_boundary_cut_copies_the_pictures_and_the_audio_of_the_span() {
    for f in [transport(), program()] {
        let format = format_of(&f.file);
        let p = f.plan(10, 30, CutMode::GopBoundary);
        let (out, report) = execute_mpeg_copy(&p.plan, &f.index, &f.source(), format, Vec::new()).unwrap();
        assert_eq!(report.samples, 20, "{format:?}");
        assert_eq!(report.duration, 20 * FRAME as u64);
        assert_eq!(report.audio.len(), 2, "{format:?}: {:?}", report.dropped);
        assert!(report.dropped.is_empty());
        for a in &report.audio {
            // whole frames inside the cut: the first one starts at or after the first picture
            assert!(a.units > 0 && (0..3000).contains(&a.offset), "{format:?}: {a:?}");
        }
        let verified = verify_mpeg_copy(&p.plan, &f.index, &f.source(), format, &out).unwrap();
        assert_eq!(verified, report, "{format:?}");
        let copy = cfa4_mpegts::open(&out).unwrap();
        assert!(copy.warnings.is_empty(), "{format:?}: {:?}", copy.warnings);
        assert_eq!(container_of(&copy), container_of(&f.file));
        assert_eq!(copy.streams.iter().filter(|s| s.kind() == Kind::Audio).count(), 2);
    }
}

#[test]
fn the_preview_names_the_copied_audio_and_video_only_copies_leave_it_out() {
    let f = program();
    let p = f.plan(10, 30, CutMode::GopBoundary);
    let (tracks, dropped) = plan_mpeg_audio(&p.plan, &f.index, &f.source(), MuxFormat::Ps).unwrap();
    let (_, report) = execute_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ps, Vec::new()).unwrap();
    assert!(dropped.is_empty());
    assert_eq!(
        tracks.iter().map(|t| (t.stream, t.codec.clone(), t.units)).collect::<Vec<_>>(),
        report.audio.iter().map(|a| (a.stream, a.codec.clone(), a.units)).collect::<Vec<_>>()
    );
    let video_only = MpegCopySource { audio: false, ..f.source() };
    assert_eq!(plan_mpeg_audio(&p.plan, &f.index, &video_only, MuxFormat::Ps).unwrap(), (Vec::new(), Vec::new()));
    let (out, report) = execute_mpeg_copy(&p.plan, &f.index, &video_only, MuxFormat::Ps, Vec::new()).unwrap();
    assert!(report.audio.is_empty() && report.samples == 20);
    assert_eq!(verify_mpeg_copy(&p.plan, &f.index, &video_only, MuxFormat::Ps, &out).unwrap(), report);
    let copy = cfa4_mpegts::open(&out).unwrap();
    assert_eq!(copy.streams.iter().filter(|s| s.kind() == Kind::Audio).count(), 0);
}

#[test]
fn the_audio_keeps_its_offset_against_the_first_picture() {
    // MP2 starts 500 ticks after the first picture of the source: the first MP2 frame of the cut
    // keeps the source's distance to the first copied picture
    let f = transport();
    let p = f.plan(10, 30, CutMode::GopBoundary);
    let (out, report) = execute_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ts, Vec::new()).unwrap();
    let mp2 = report.audio.iter().find(|a| a.codec == TsCodec::MpegAudio.name()).unwrap();
    // the cut starts at START + 10 frames = START + 36000; MP2 frames at START + 500 + k·2160
    let first = (0..).map(|k| 500 + k * 2160).find(|&t| t >= 36_000).unwrap();
    assert_eq!(mp2.offset, first - 36_000);
    let copy = cfa4_mpegts::open(&out).unwrap();
    let video = copy.find(Kind::Video, |_| true).unwrap();
    let audio = copy.find(Kind::Audio, |c| *c == TsCodec::MpegAudio).unwrap();
    let v0 = copy.streams[video].units.iter().filter_map(|u| u.pts).min().unwrap();
    assert_eq!(copy.streams[audio].units[0].pts, Some(v0 + mp2.offset));
}

#[test]
fn plans_the_executor_cannot_honour_are_refused_before_writing() {
    let f = transport();
    // a cut inside a GOP needs boundary re-encodes, which MPEG-2 does not have here
    let inside = f.plan(12, 30, CutMode::FrameAccurate);
    assert!(matches!(execute_mpeg_copy(&inside.plan, &f.index, &f.source(), MuxFormat::Ts, Vec::new()), Err(SmartCutError::NotLossless)));
    let p = f.plan(10, 30, CutMode::GopBoundary);
    // another output container
    assert!(matches!(execute_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ps, Vec::new()), Err(SmartCutError::Unsupported { .. })));
    // an index of another stream
    let mut other = f.index.clone();
    other.frames.pop();
    assert!(matches!(execute_mpeg_copy(&p.plan, &other, &f.source(), MuxFormat::Ts, Vec::new()), Err(SmartCutError::StaleIndex { .. })));
    // an index of the program stream, whose pictures are the same but whose source changed
    let ps = program();
    assert!(execute_mpeg_copy(&p.plan, &f.index, &ps.source(), MuxFormat::Ts, Vec::new()).is_err());
}

#[test]
fn verification_catches_changed_pictures_audio_and_timing() {
    let f = transport();
    let p = f.plan(10, 30, CutMode::GopBoundary);
    let (out, _) = execute_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ts, Vec::new()).unwrap();
    // another span written in place of the planned one
    let other = f.plan(20, 40, CutMode::GopBoundary);
    let (wrong, _) = execute_mpeg_copy(&other.plan, &f.index, &f.source(), MuxFormat::Ts, Vec::new()).unwrap();
    assert!(matches!(verify_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &wrong), Err(SmartCutError::Verification { .. })));
    // a changed audio byte inside the first MP2 frame of the output
    let copy = cfa4_mpegts::open(&out).unwrap();
    let audio = copy.find(Kind::Audio, |c| *c == TsCodec::MpegAudio).unwrap();
    let frame = copy.read_unit(&out, audio, 0).unwrap();
    let at = (8..frame.len() - 16).step_by(16).find_map(|o| out.windows(16).position(|w| w == &frame[o..o + 16])).unwrap();
    let mut bad = out.clone();
    bad[at + 3] ^= 0x55;
    assert!(matches!(verify_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &bad), Err(SmartCutError::Verification { .. })));
    // the same pictures and audio frames, but the source's MP2 sits 90 ticks later: the copy is
    // out of sync with that source
    let late = Fixture::new(source(
        MuxFormat::Ts,
        40,
        &[(MuxCodec::AacAdts, units(adts_frame, 80, START, 1920)), (MuxCodec::MpegAudio, units(mp2_frame, 70, START + 590, 2160))],
    ));
    assert_eq!(late.index.frames, f.index.frames);
    let e = verify_mpeg_copy(&p.plan, &late.index, &late.source(), MuxFormat::Ts, &out).unwrap_err();
    assert!(e.to_string().contains("sync"), "{e}");
    // not a program stream
    assert!(verify_mpeg_copy(&p.plan, &f.index, &f.source(), MuxFormat::Ps, &out).is_err());
}

#[test]
fn ffmpeg_decodes_the_copy_to_the_source_pictures() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("MPEG-2 Smart Cut copy decode") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-smart-cut-mpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, f) in [("ts", transport()), ("mpg", program())] {
        let p = f.plan(10, 30, CutMode::GopBoundary);
        let (out, _) = execute_mpeg_copy(&p.plan, &f.index, &f.source(), format_of(&f.file), Vec::new()).unwrap();
        let (src_path, out_path) = (dir.join(format!("source.{name}")), dir.join(format!("copy.{name}")));
        std::fs::write(&src_path, &f.bytes).unwrap();
        std::fs::write(&out_path, &out).unwrap();
        let hashes = |path: &std::path::Path| -> Vec<String> {
            let o = Command::new(&ffmpeg).args(["-v", "error", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
        };
        let (src, copy) = (hashes(&src_path), hashes(&out_path));
        assert_eq!(src.len(), 40, "{name}");
        assert_eq!(copy, src[10..30], "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
