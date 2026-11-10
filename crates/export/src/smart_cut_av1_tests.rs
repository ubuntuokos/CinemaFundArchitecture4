//! WP-07: the Smart Cut executor copies AV1 (`av1C`) temporal units of an MP4 unchanged, on the
//! shown key frames of the AV1 index; FFmpeg decodes the cut to the source pictures.

use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};

use cfa4_isobmff::{Av1Config, Brand, CodecConfig, Mp4File, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::av1_index::index_av1;
use cfa4_smart_edit::capabilities::{Codec, Container, Policy, Registry};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::media_index::{MediaIndex, Packet, RandomAccessKind, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::{CopyGuarantee, ExportExecutionPlan};
use cfa4_smart_edit::timestamp::{MediaTime, Rational};

use crate::smart_cut::{Mp4CopySource, SmartCutError, execute_mp4_copy, verify_mp4_copy};

/// An OBU of `kind` with a size field.
fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(kind << 3) | 0x02, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A sequence header OBU (profile 0, not reduced).
fn sequence_header() -> Vec<u8> {
    obu(1, &[0x00, 0x01, 0x80])
}

/// A temporal unit (no temporal delimiter, as in MP4): an optional sequence header, a frame of `frame_type` (0 key, 1
/// inter) that is shown, and `tag` to tell the units apart.
fn unit(seq: bool, frame_type: u8, tag: u8) -> Vec<u8> {
    let mut out = Vec::new();
    if seq {
        out.extend(sequence_header());
    }
    out.extend(obu(6, &[(frame_type << 5) | 0x10, tag, 0x55]));
    out
}

/// 40 synthetic temporal units at 25 fps, a key frame every 10; the sequence header is in `av1C`
/// (`in_band` false) or only in the first unit (`in_band` true).
fn synthetic_av1_mp4(in_band: bool) -> Vec<u8> {
    let cfg = Av1Config {
        chroma_subsampling_x: true,
        chroma_subsampling_y: true,
        config_obus: if in_band { Vec::new() } else { sequence_header() },
        ..Default::default()
    };
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(SampleEntry::av1(cfg, 64, 48), 25)).unwrap();
    for i in 0..40usize {
        let key = i % 10 == 0;
        let data = unit(in_band && i == 0, if key { 0 } else { 1 }, i as u8);
        mux.write_sample(t, WriteSample { data: &data, duration: 1, composition_offset: 0, is_sync: key }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

struct Source {
    bytes: Vec<u8>,
    file: Mp4File,
    index: StreamIndex,
    output: Container,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Source {
        let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
        let track = &file.tracks[0];
        let Some(CodecConfig::Av1(av1c)) = track.codec() else { panic!("not AV1") };
        let data: Vec<Vec<u8>> = (0..track.samples.len()).map(|i| file.read_sample(bytes.as_slice(), 0, i).unwrap()).collect();
        let packets = track.samples.iter().zip(&data).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: Some(s.dts), keyframe: s.is_sync });
        let index = index_av1(track.id, Rational::new(1, i64::from(track.timescale)), &av1c.config_obus, packets).unwrap();
        Source { bytes, file, index, output: Container::Mp4 }
    }

    fn source(&self) -> Mp4CopySource<'_, [u8]> {
        Mp4CopySource { file: &self.file, bytes: self.bytes.as_slice(), track: 0 }
    }

    /// The GOP-boundary plan of `[from, to)` pictures after the first one.
    fn plan(&self, from: i64, to: i64) -> ExportExecutionPlan {
        let t = &self.file.tracks[0];
        let pts0 = t.samples.iter().map(|s| s.pts).min().unwrap();
        let step = i64::from(t.samples[0].duration);
        let tb = Rational::new(1, i64::from(t.timescale));
        let tick = |f: i64| MediaTime::new(pts0 + f * step, tb).to_tick().unwrap();
        let req = SmartCutRequest {
            stream_id: self.index.stream_id,
            in_point: tick(from),
            out_point: tick(to),
            mode: CutMode::GopBoundary,
            guarantee: CopyGuarantee::BitExact,
            source_container: Container::Mp4,
            output_container: self.output,
            format: None,
        };
        let media =
            MediaIndex::new("src.mp4", SourceFingerprint { size: self.bytes.len() as u64, modified_ns: None, content_id: None }, vec![self.index.clone()]);
        let p = plan_smart_cut(&Registry::native(), &media, &req, Policy { allow_experimental: false }).unwrap();
        assert!(p.executable, "{:?}", p.blockers);
        p.plan
    }

    /// Run and verify the plan.
    fn cut(&self, plan: &ExportExecutionPlan) -> Vec<u8> {
        let (out, report) = execute_mp4_copy(plan, &self.index, &self.source(), Cursor::new(Vec::new())).unwrap();
        let out = out.into_inner();
        assert_eq!(verify_mp4_copy(plan, &self.index, &self.source(), out.as_slice()).unwrap(), report);
        out
    }
}

#[test]
fn av1_temporal_units_are_copied_unchanged() {
    let mut src = Source::new(synthetic_av1_mp4(false));
    assert_eq!(src.index.clean_access_points().len(), 4);
    for output in [Container::Mp4, Container::Mov] {
        src.output = output;
        let plan = src.plan(10, 30);
        assert!(plan.is_lossless());
        assert_eq!(plan.codec, Codec::Av1);
        assert_eq!(plan.copied_frames(), 20);
        let out = src.cut(&plan);
        let back = cfa4_isobmff::open(out.as_slice()).unwrap();
        assert_eq!(back.is_quicktime, output == Container::Mov);
        let t = &back.tracks[0];
        assert_eq!(t.codec(), src.file.tracks[0].codec(), "the av1C sample entry is kept");
        assert_eq!(t.samples.len(), 20);
        for k in 0..20 {
            assert_eq!(back.read_sample(out.as_slice(), 0, k).unwrap(), src.file.read_sample(src.bytes.as_slice(), 0, k + 10).unwrap());
        }
    }
}

#[test]
fn a_stale_av1_index_is_refused() {
    let src = Source::new(synthetic_av1_mp4(false));
    let plan = src.plan(10, 30);
    // the index says the unit at 20 is intra-only, the bitstream shows a key frame
    let mut stale = Source::new(src.bytes.clone());
    stale.index.frames[20].random_access = RandomAccessKind::IntraOnly;
    let e = execute_mp4_copy(&plan, &stale.index, &stale.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(&e, SmartCutError::StaleIndex { detail } if detail.contains("packet 20")), "{e}");
    // an HEVC plan over an AV1 source
    let mut other = plan.clone();
    other.codec = Codec::Hevc;
    assert!(matches!(execute_mp4_copy(&other, &src.index, &src.source(), Cursor::new(Vec::new())), Err(SmartCutError::Unsupported { .. })));
}

/// The sequence header only in the first unit: a later key frame is a clean access point of the
/// stream, but a copy starting there would have no sequence header, so the executor refuses it.
#[test]
fn a_copy_without_a_sequence_header_is_refused() {
    let src = Source::new(synthetic_av1_mp4(true));
    assert_eq!(src.index.clean_access_points().len(), 4);
    let whole = src.plan(0, 20);
    src.cut(&whole);
    let plan = src.plan(10, 30);
    let e = execute_mp4_copy(&plan, &src.index, &src.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(&e, SmartCutError::StaleIndex { detail } if detail.contains("packet 10")), "{e}");
}

/// An AV1 MP4 from ffmpeg (`args` select the encoder), 100 pictures at 25 fps, a key frame every 25.
fn ffmpeg_av1_mp4(ffmpeg: &Path, path: &Path, args: &[&str]) -> Vec<u8> {
    let st = Command::new(ffmpeg)
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=128x96:rate=25", "-t", "4"])
        .args(args)
        .args(["-pix_fmt", "yuv420p", "-an"])
        .arg(path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::read(path).unwrap()
}

fn framemd5(ffmpeg: &Path, path: &Path) -> Vec<String> {
    let o = Command::new(ffmpeg).args(["-v", "error", "-xerror", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
}

/// FFmpeg as the oracle: libaom (alt-ref frames, show-existing units) and SVT-AV1 streams cut on
/// a shown key frame decode to the source pictures, from MP4 and MOV.
#[test]
fn ffmpeg_decodes_av1_cuts_to_the_source_pictures() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("AV1 Smart Cut decode") else { return };
    let encoders: [(&str, &[&str]); 2] = [
        ("libaom-av1", &["-c:v", "libaom-av1", "-cpu-used", "8", "-g", "25", "-keyint_min", "25", "-lag-in-frames", "16"]),
        ("libsvtav1", &["-c:v", "libsvtav1", "-preset", "12", "-g", "25", "-svtav1-params", "scd=0:irefresh-type=2"]),
    ];
    let dir = std::env::temp_dir().join(format!("cfa4-av1-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut ran = 0;
    for (name, args) in encoders {
        // one AV1 encoder is enough; the test is skipped only when ffmpeg has none
        if !cfa4_testkit::ffmpeg_has_encoder(&ffmpeg, name) {
            continue;
        }
        ran += 1;
        let path = dir.join(format!("{name}.mp4"));
        let mut src = Source::new(ffmpeg_av1_mp4(&ffmpeg, &path, args));
        let clean = src.index.clean_access_points();
        assert!(clean.contains(&25) && clean.contains(&50), "{name}: key frames at {clean:?}");
        let plan = src.plan(25, 75);
        assert_eq!(plan.segments.first().unwrap().decode.first, 25);
        let n = plan.copied_frames() as usize;
        let cut = dir.join(format!("{name}_cut.mp4"));
        std::fs::write(&cut, src.cut(&plan)).unwrap();
        let (all, part) = (framemd5(&ffmpeg, &path), framemd5(&ffmpeg, &cut));
        assert_eq!(part.len(), 50, "{name}: shown pictures of the cut");
        assert!(n >= 50, "{name}: {n} temporal units copied");
        assert_eq!(part[..], all[25..75], "{name}: the copied pictures are the source pictures");
        src.output = Container::Mov;
        let mov = dir.join(format!("{name}_cut.mov"));
        std::fs::write(&mov, src.cut(&src.plan(25, 75))).unwrap();
        assert_eq!(framemd5(&ffmpeg, &mov), part, "{name}: the MOV copy decodes like the MP4 copy");
    }
    if ran == 0 {
        cfa4_testkit::skip("smart_cut_av1_tests", "ffmpeg has no AV1 encoder");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
