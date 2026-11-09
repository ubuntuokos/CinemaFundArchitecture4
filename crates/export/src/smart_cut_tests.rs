//! WP-04: the Smart Cut executor copies the planned H.264 samples of an MP4 unchanged, and the
//! output reads back with the same payloads, timing and codec configuration.

use std::io::Cursor;
use std::process::Command;

use cfa4_isobmff::{AvcConfig, Brand, Mp4File, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::{Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::media_index::{MediaIndex, Packet, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::{CopyGuarantee, SegmentMode};
use cfa4_smart_edit::timestamp::{MediaTime, Rational};
use cfa4_time::Tick;

use crate::smart_cut::{Mp4CopySource, SmartCutError, execute_mp4_copy, verify_mp4_copy};

const FPS: u32 = 25;
const STRICT: Policy = Policy { allow_experimental: false };

/// An MP4 with one H.264 track (64×48, 25 fps, IDR every `keyint` pictures, `bframes` B-frames).
pub(crate) fn source_mp4(frames: usize, keyint: u32, bframes: u8, brand: Brand) -> Vec<u8> {
    use cfa4_h264enc::{Encoder, EncoderConfig, PacketFormat, RateControl, YuvFrame};
    let (w, h) = (64usize, 48usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, FPS, 1);
    cfg.keyint = keyint;
    cfg.bframes = bframes;
    cfg.rate = RateControl::Qp(30);
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.scenecut = false;
    cfg.threads = 1;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..frames {
        let y: Vec<u8> = (0..w * h).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| ((k * 3 + i) % 255) as u8).collect();
        packets.extend(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }, i as i64).unwrap());
    }
    packets.extend(enc.flush());
    let entry = SampleEntry::avc(AvcConfig::parse(&enc.avcc()).unwrap(), w as u16, h as u16);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(brand)).unwrap();
    let mut track = TrackConfig::new(entry, FPS);
    track.media_start = (bframes > 0).then_some(1);
    let t = mux.add_track(track).unwrap();
    for p in &packets {
        mux.write_sample(t, WriteSample { data: &p.data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

/// The WP-02 index of the first track, as an importer builds it.
pub(crate) fn index_of(file: &Mp4File, bytes: &[u8]) -> StreamIndex {
    let track = &file.tracks[0];
    let Some(cfa4_isobmff::CodecConfig::Avc(avc)) = track.codec() else { panic!("not H.264") };
    let ps: Vec<&[u8]> = avc.sps.iter().chain(&avc.pps).map(Vec::as_slice).collect();
    let data: Vec<Vec<u8>> = (0..track.samples.len()).map(|i| file.read_sample(bytes, 0, i).unwrap()).collect();
    let packets = track.samples.iter().zip(&data).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: Some(s.dts), keyframe: s.is_sync });
    index_h264(track.id, Rational::new(1, i64::from(track.timescale)), NalFormat::LengthPrefixed(avc.length_size), &ps, packets).unwrap()
}

pub(crate) fn media(stream: &StreamIndex, size: usize) -> MediaIndex {
    MediaIndex::new("test.mp4", SourceFingerprint { size: size as u64, modified_ns: None, content_id: None }, vec![stream.clone()])
}

pub(crate) fn tick(frames: i64) -> Tick {
    MediaTime::new(frames, Rational::new(1, i64::from(FPS))).to_tick().unwrap()
}

/// Plan `[from, to)` (in frames on the source PTS timeline) with the native registry.
pub(crate) fn plan(stream: &StreamIndex, size: usize, from: i64, to: i64, mode: CutMode, output: Container) -> SmartCutPlan {
    let req = SmartCutRequest {
        stream_id: stream.stream_id,
        in_point: tick(from),
        out_point: tick(to),
        mode,
        guarantee: CopyGuarantee::BitExact,
        source_container: Container::Mp4,
        output_container: output,
        format: Some(StreamFormat { profile: "high".into(), chroma: "4:2:0".into(), bit_depth: 8 }),
    };
    plan_smart_cut(&Registry::native(), &media(stream, size), &req, STRICT).unwrap()
}

pub(crate) struct Fixture {
    pub(crate) bytes: Vec<u8>,
    pub(crate) file: Mp4File,
    pub(crate) index: StreamIndex,
}

pub(crate) fn fixture(frames: usize, keyint: u32, bframes: u8) -> Fixture {
    let bytes = source_mp4(frames, keyint, bframes, Brand::Mp4);
    let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
    let index = index_of(&file, &bytes);
    Fixture { bytes, file, index }
}

impl Fixture {
    pub(crate) fn source(&self) -> Mp4CopySource<'_, [u8]> {
        Mp4CopySource { file: &self.file, bytes: self.bytes.as_slice(), track: 0 }
    }
    /// The first presentation time of the source (its edit list), in frames.
    pub(crate) fn pts0(&self) -> i64 {
        self.file.tracks[0].samples.iter().map(|s| s.pts).min().unwrap()
    }
}

#[test]
fn a_gop_boundary_cut_copies_the_planned_samples_unchanged() {
    let f = fixture(40, 10, 2);
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + 10, p0 + 30, CutMode::GopBoundary, Container::Mp4);
    assert!(p.executable, "the native registry runs MP4 H.264 packet copy: {:?}", p.blockers);
    assert!(p.plan.is_lossless());
    let (out, report) = execute_mp4_copy(&p.plan, &f.index, &f.source(), Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    assert_eq!((report.samples, report.duration, report.timescale), (20, 20, FPS));
    assert_eq!(verify_mp4_copy(&p.plan, &f.index, &f.source(), out.as_slice()).unwrap(), report);

    // reimport: one track, starts on an IDR at presentation time 0, indexes to the same pictures
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    let t = &back.tracks[0];
    assert_eq!(t.samples.len(), 20);
    assert!(t.samples[0].is_sync);
    assert_eq!(t.samples[0].dts, 0);
    let first_shown = t.samples.iter().map(|s| s.pts).min().unwrap();
    assert_eq!(first_shown + t.edit_offset, 0, "the edit list starts the output at the first copied picture");
    let ix = index_of(&back, &out);
    assert_eq!(ix.frames.len(), 20);
    assert!(ix.frames[0].random_access.is_clean());
    let src = &f.index.frames[10..30];
    for (a, b) in ix.frames.iter().zip(src) {
        assert_eq!((a.picture_type, a.size, a.keyframe, a.random_access), (b.picture_type, b.size, b.keyframe, b.random_access));
        assert_eq!(a.presentation_index.unwrap() + 10, b.presentation_index.unwrap());
    }
}

#[test]
fn the_whole_stream_and_a_mov_output_copy_too() {
    let f = fixture(30, 10, 2);
    let p0 = f.pts0();
    for output in [Container::Mp4, Container::Mov] {
        let p = plan(&f.index, f.bytes.len(), p0, p0 + 30, CutMode::GopBoundary, output);
        assert!(p.executable, "{:?}", p.blockers);
        let (out, report) = execute_mp4_copy(&p.plan, &f.index, &f.source(), Cursor::new(Vec::new())).unwrap();
        let out = out.into_inner();
        assert_eq!(report.samples, 30);
        verify_mp4_copy(&p.plan, &f.index, &f.source(), out.as_slice()).unwrap();
        let back = cfa4_isobmff::open(out.as_slice()).unwrap();
        assert_eq!(back.is_quicktime, output == Container::Mov);
    }
}

#[test]
fn a_stream_without_b_frames_needs_no_edit_list() {
    let f = fixture(30, 10, 0);
    let p = plan(&f.index, f.bytes.len(), 10, 20, CutMode::GopBoundary, Container::Mp4);
    let (out, _) = execute_mp4_copy(&p.plan, &f.index, &f.source(), Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    verify_mp4_copy(&p.plan, &f.index, &f.source(), out.as_slice()).unwrap();
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    assert!(back.tracks[0].samples.iter().all(|s| s.pts == s.dts));
    assert_eq!(back.tracks[0].edit_offset, 0);
}

#[test]
fn a_plan_that_re_encodes_is_refused_before_writing() {
    let f = fixture(40, 10, 2);
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + 5, p0 + 25, CutMode::FrameAccurate, Container::Mp4);
    assert!(p.plan.segments.iter().any(|s| s.mode == SegmentMode::BoundaryReencode));
    let mut out = Cursor::new(Vec::new());
    assert_eq!(execute_mp4_copy(&p.plan, &f.index, &f.source(), &mut out).unwrap_err(), SmartCutError::NotLossless);
    assert!(out.into_inner().is_empty(), "nothing is written");
}

#[test]
fn an_index_of_another_file_is_refused() {
    let f = fixture(40, 10, 2);
    let other = fixture(30, 10, 2);
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + 10, p0 + 20, CutMode::GopBoundary, Container::Mp4);
    let err = execute_mp4_copy(&p.plan, &f.index, &other.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(err, SmartCutError::StaleIndex { .. }), "{err:?}");
    // same length, other sizes
    let mut index = f.index.clone();
    index.frames[12].size += 1;
    let err = execute_mp4_copy(&p.plan, &index, &f.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(err, SmartCutError::StaleIndex { .. }), "{err:?}");
    // same access units, other timestamps (a remux at another frame rate) or timebase
    let mut index = f.index.clone();
    index.frames[12].pts = index.frames[12].pts.map(|t| t + 1);
    let err = execute_mp4_copy(&p.plan, &index, &f.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(&err, SmartCutError::StaleIndex { detail } if detail.contains("PTS")), "{err:?}");
    let mut index = f.index.clone();
    index.timebase.den *= 2;
    let err = execute_mp4_copy(&p.plan, &index, &f.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(&err, SmartCutError::StaleIndex { detail } if detail.contains("timebase")), "{err:?}");
    // the boundary re-encoder checks every packet, the copied ones too
    let p = plan(&f.index, f.bytes.len(), p0 + 5, p0 + 20, CutMode::FrameAccurate, Container::Mp4);
    let mut index = f.index.clone();
    index.frames[30].dts = index.frames[30].dts.map(|t| t - 1);
    let encoding = crate::boundary_reencode::BoundaryEncoding { threads: 1, ..Default::default() };
    let err = crate::boundary_reencode::execute_mp4_boundary_reencode(&p.plan, &index, &f.source(), &encoding, Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(&err, SmartCutError::StaleIndex { detail } if detail.contains("DTS")), "{err:?}");
}

#[test]
fn malformed_plans_are_refused() {
    let f = fixture(40, 10, 2);
    let p0 = f.pts0();
    let good = plan(&f.index, f.bytes.len(), p0 + 10, p0 + 30, CutMode::GopBoundary, Container::Mp4).plan;
    let run = |p: &cfa4_smart_edit::plan::ExportExecutionPlan| execute_mp4_copy(p, &f.index, &f.source(), Cursor::new(Vec::new())).map(|_| ()).unwrap_err();

    let mut p = good.clone();
    p.plan_version += 1;
    assert_eq!(run(&p), SmartCutError::PlanVersion { found: good.plan_version + 1 });

    let mut p = good.clone();
    p.output_container = Container::MpegTs;
    assert!(matches!(run(&p), SmartCutError::Unsupported { .. }));

    let mut p = good.clone();
    p.segments[0].stream_id += 1;
    assert!(matches!(run(&p), SmartCutError::WrongStream { .. }));

    let mut p = good.clone();
    p.segments.clear();
    assert_eq!(run(&p), SmartCutError::NotLossless);

    // a timeline plan names a source per segment
    let mut p = good.clone();
    p.segments[0].source = Some(cfa4_smart_edit::plan::SegmentSource { source_id: "other.mp4".into(), container: Container::Mp4, item: 0 });
    assert_eq!(run(&p), SmartCutError::TimelinePlan);

    // two copy segments with a gap in decode order
    let mut p = good.clone();
    let mut second = p.segments[0].clone();
    second.decode.first = p.segments[0].decode.last + 2;
    second.decode.last = second.decode.first;
    p.segments.push(second);
    assert_eq!(run(&p), SmartCutError::NotContiguous { segment: 1 });

    let mut p = good.clone();
    p.segments[0].decode.last = 1000;
    assert!(matches!(run(&p), SmartCutError::StaleIndex { .. }));

    let err = execute_mp4_copy(&good, &f.index, &Mp4CopySource { track: 3, ..f.source() }, Cursor::new(Vec::new())).unwrap_err();
    assert_eq!(err, SmartCutError::NoTrack { track: 3 });
}

#[test]
fn verification_catches_a_changed_payload() {
    let f = fixture(40, 10, 2);
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + 10, p0 + 30, CutMode::GopBoundary, Container::Mp4);
    let (out, _) = execute_mp4_copy(&p.plan, &f.index, &f.source(), Cursor::new(Vec::new())).unwrap();
    let mut out = out.into_inner();
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    let s = back.tracks[0].samples[5];
    out[(s.offset + u64::from(s.size) - 1) as usize] ^= 0x01;
    let err = verify_mp4_copy(&p.plan, &f.index, &f.source(), out.as_slice()).unwrap_err();
    assert!(matches!(&err, SmartCutError::Verification { detail } if detail.contains("payload")), "{err:?}");
    // a cut file is not a copy either
    let err = verify_mp4_copy(&p.plan, &f.index, &f.source(), &out[..out.len() / 2]).unwrap_err();
    assert!(matches!(err, SmartCutError::Verification { .. }), "{err:?}");
}

#[test]
fn errors_are_serde_and_readable() {
    let e = SmartCutError::NotContiguous { segment: 2 };
    let json = serde_json::to_string(&e).unwrap();
    assert_eq!(json, r#"{"reason":"not-contiguous","segment":2}"#);
    assert_eq!(serde_json::from_str::<SmartCutError>(&json).unwrap(), e);
    assert!(e.to_string().contains("segment 2"));
}

/// FFmpeg (oracle only) reads the copy, and its decoded pictures equal the source pictures of the
/// cut range.
#[test]
fn ffmpeg_decodes_the_copy_to_the_source_pictures() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("Smart Cut copy decode") else { return };
    let f = fixture(40, 10, 2);
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + 10, p0 + 30, CutMode::GopBoundary, Container::Mp4);
    let (out, _) = execute_mp4_copy(&p.plan, &f.index, &f.source(), Cursor::new(Vec::new())).unwrap();
    let dir = std::env::temp_dir().join(format!("cfa4-smart-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src_path, out_path) = (dir.join("source.mp4"), dir.join("copy.mp4"));
    std::fs::write(&src_path, &f.bytes).unwrap();
    std::fs::write(&out_path, out.into_inner()).unwrap();
    let hashes = |path: &std::path::Path| -> Vec<String> {
        let o = Command::new(&ffmpeg).args(["-v", "error", "-xerror", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
    };
    let (src, copy) = (hashes(&src_path), hashes(&out_path));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(src.len(), 40);
    assert_eq!(copy, src[10..30]);
}
