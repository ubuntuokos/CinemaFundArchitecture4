//! R2-03 of `CFA4-SMART-CUT-RENDER-R2`: Matroska sources are demuxed, indexed and planned for a
//! Smart Cut the same way as MP4 sources.
//!
//! FFmpeg (the oracle) encodes a clip in each codec into Matroska and copies it into MP4. Three
//! checks follow:
//! - Demux oracle: the Matroska demuxer gives the frames ffprobe gives, with the same order,
//!   presentation times and key flags.
//! - Index: the index of the Matroska track is the index of the same stream in MP4, picture for
//!   picture.
//! - Plan: a GOP-boundary cut plans the same segments from both. Only the Matroska output (R2-04)
//!   still blocks a Matroska → Matroska copy.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_isobmff::{CodecConfig, Mp4File};
use cfa4_smart_edit::av1_index::index_av1;
use cfa4_smart_edit::capabilities::{Codec, Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::copy_planner::{CutMode, PlanRefusal, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::hevc_index::index_hevc;
use cfa4_smart_edit::media_index::{MediaIndex, Packet, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::mpeg2_index::index_mpeg2;
use cfa4_smart_edit::plan::CopyGuarantee;
use cfa4_smart_edit::timestamp::{MediaTime, Rational};
use cfa4_testkit::oracle::{ffmpeg_has_encoder, ffmpeg_or_skip, ffprobe_or_skip, skip};

use crate::mkv_source::{MkvSourceError, container, index_mkv_video};

const STRICT: Policy = Policy { allow_experimental: false };
const KEYINT: u64 = 12;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/smart-cut-mkv").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 48-picture, 25 fps clip encoded by FFmpeg into `name/source.mkv` (with `extra` muxer
/// options) and copied into `name/source.mp4`.
fn sources(name: &str, encoder: &str, args: &[&str], ext: &str) -> Option<(PathBuf, PathBuf)> {
    let context = format!("Smart Cut Matroska source {name}");
    let ff = ffmpeg_or_skip(&context)?;
    if !ffmpeg_has_encoder(&ff, encoder) {
        skip(&context, &format!("ffmpeg has no {encoder} encoder"));
        return None;
    }
    let d = dir(name);
    let (mkv, mp4) = (d.join(format!("source.{ext}")), d.join("source.mp4"));
    let o = Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=128x96:rate=25", "-frames:v", "48", "-c:v", encoder])
        .args(args)
        .arg(&mkv)
        .output()
        .unwrap();
    assert!(o.status.success(), "{context}: {}", String::from_utf8_lossy(&o.stderr));
    let o = Command::new(&ff).args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&mkv).args(["-c", "copy"]).arg(&mp4).output().unwrap();
    assert!(o.status.success(), "{context}: {}", String::from_utf8_lossy(&o.stderr));
    Some((mkv, mp4))
}

/// ffprobe's packets of the video stream, in file order: (PTS in milliseconds, key flag).
fn probe(fp: &Path, file: &Path) -> Vec<(i64, bool)> {
    let o = Command::new(fp).args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts,flags", "-of", "csv=p=0"]).arg(file).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (pts, flags) = l.split_once(',').unwrap();
            (pts.trim().parse().unwrap(), flags.starts_with('K'))
        })
        .collect()
}

/// The WP-02 index of the video track of an MP4, as the MP4 Smart Cut builds it.
pub(crate) fn index_mp4(file: &Mp4File, bytes: &[u8]) -> StreamIndex {
    let i = file.tracks.iter().position(|t| t.kind == cfa4_isobmff::TrackKind::Video).unwrap();
    let t = &file.tracks[i];
    let data: Vec<Vec<u8>> = (0..t.samples.len()).map(|k| file.read_sample(bytes, i, k).unwrap()).collect();
    let packets = t.samples.iter().zip(&data).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: Some(s.dts), keyframe: s.is_sync });
    let tb = Rational::new(1, i64::from(t.timescale));
    match t.codec() {
        Some(CodecConfig::Avc(avc)) => {
            let ps: Vec<&[u8]> = avc.sps.iter().chain(&avc.pps).map(Vec::as_slice).collect();
            index_h264(t.id, tb, NalFormat::LengthPrefixed(avc.length_size), &ps, packets)
        }
        Some(CodecConfig::Hevc(h)) => {
            let ps: Vec<&[u8]> = h.arrays.iter().flat_map(|a| a.nalus.iter().map(Vec::as_slice)).collect();
            index_hevc(t.id, tb, NalFormat::LengthPrefixed(h.length_size), &ps, packets)
        }
        Some(CodecConfig::Av1(a)) => index_av1(t.id, tb, &a.config_obus, packets),
        _ => index_mpeg2(t.id, tb, packets),
    }
    .unwrap()
}

/// Plan a GOP-boundary cut of the pictures `[from, to)` (presentation positions) of `stream` out
/// of `source` into `output`, moved to the nearest safe points when they are not safe (open GOPs).
fn plan(stream: &StreamIndex, source: Container, output: Container, from: u64, to: u64) -> SmartCutPlan {
    let pts = |p: u64| {
        let f = stream.frames.iter().find(|f| f.presentation_index == Some(p)).unwrap();
        MediaTime::new(f.pts.unwrap(), stream.timebase).to_tick().unwrap()
    };
    let media = MediaIndex::new("source", SourceFingerprint { size: 1, modified_ns: None, content_id: None }, vec![stream.clone()]);
    let mut req = SmartCutRequest {
        stream_id: stream.stream_id,
        in_point: pts(from),
        out_point: pts(to),
        mode: CutMode::GopBoundary,
        guarantee: CopyGuarantee::BitExact,
        source_container: source,
        output_container: output,
        format: Some(StreamFormat { profile: "main".into(), chroma: "4:2:0".into(), bit_depth: 8 }),
    };
    match plan_smart_cut(&Registry::native(), &media, &req, STRICT) {
        Err(PlanRefusal::NotAtSafePoints { safe, .. }) => {
            let (i, o) = (safe.in_after.or(safe.in_before).unwrap(), safe.out_after.or(safe.out_before).unwrap());
            req.in_point = i.time.to_tick().unwrap();
            req.out_point = o.time.to_tick().unwrap();
            plan_smart_cut(&Registry::native(), &media, &req, STRICT).unwrap()
        }
        p => p.unwrap(),
    }
}

fn check(name: &str, encoder: &str, args: &[&str], codec: Codec, ext: &str) {
    let Some((mkv_path, mp4_path)) = sources(name, encoder, args, ext) else { return };
    let mkv_bytes = std::fs::read(&mkv_path).unwrap();
    let mkv = cfa4_matroska::open(mkv_bytes.as_slice()).unwrap();
    let want = if ext == "webm" { Container::WebM } else { Container::Matroska };
    assert_eq!(container(&mkv), want, "{name}");
    let video = index_mkv_video(&mkv, mkv_bytes.as_slice()).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(video.codec, codec, "{name}");
    let track = &mkv.tracks[video.track];

    // demux oracle: the frames ffprobe reads, in file order
    let Some(fp) = ffprobe_or_skip(name) else { return };
    let probed = probe(&fp, &mkv_path);
    assert_eq!(track.timebase, (1, 1000), "{name}: FFmpeg writes millisecond timestamps");
    let ours: Vec<(i64, bool)> = track.samples.iter().map(|s| (s.pts, s.keyframe)).collect();
    assert_eq!(ours, probed, "{name}: frames, PTS and key flags against ffprobe");

    // the index of the Matroska track is the index of the same stream in MP4
    let mp4_bytes = std::fs::read(&mp4_path).unwrap();
    let mp4 = cfa4_isobmff::open(mp4_bytes.as_slice()).unwrap();
    let reference = index_mp4(&mp4, &mp4_bytes);
    let ix = &video.index;
    assert_eq!(ix.frames.len(), 48, "{name}");
    let shape = |s: &StreamIndex| s.frames.iter().map(|f| (f.presentation_index, f.picture_type, f.reference, f.random_access, f.keyframe)).collect::<Vec<_>>();
    assert_eq!(shape(ix), shape(&reference), "{name}: picture for picture as from MP4");
    let gops = |s: &StreamIndex| s.gops.iter().map(|g| (g.start, g.count, g.kind)).collect::<Vec<_>>();
    assert_eq!(gops(ix), gops(&reference), "{name}: the same GOPs");
    assert!(ix.config_changes.is_empty() && ix.timestamp_problems.is_empty(), "{name}: {:?} {:?}", ix.config_changes, ix.timestamp_problems);
    let duration = |s: &StreamIndex| s.duration().unwrap().to_tick().unwrap().0;
    assert_eq!(duration(ix), duration(&reference), "{name}: duration");

    // a GOP-boundary cut plans the same segments from both
    let (from, to) = (KEYINT, 3 * KEYINT);
    let p = plan(ix, want, want, from, to);
    let r = plan(&reference, Container::Mp4, Container::Mp4, from, to);
    let spans = |p: &SmartCutPlan| p.plan.segments.iter().map(|s| (s.mode, s.presentation, s.decode)).collect::<Vec<_>>();
    assert_eq!(spans(&p), spans(&r), "{name}: the same segments");
    assert_eq!(p.plan.expected_duration.to_tick(), r.plan.expected_duration.to_tick(), "{name}");
    // MP4 takes no MPEG-2 video (no sample entry), so only the other codecs copy MP4 → MP4
    assert!(r.executable || codec == Codec::Mpeg2Video, "{name}: {:?}", r.blockers);
    // R2-04: a Matroska → Matroska copy is supported end to end
    assert!(p.executable, "{name}: {:?}", p.blockers);
    if codec != Codec::Mpeg2Video {
        // the payload framing is the same in MP4: a bit-exact copy into MP4 plans executable
        let x = plan(ix, want, Container::Mp4, from, to);
        assert!(x.executable, "{name}: {:?}", x.blockers);
    }
}

#[test]
fn h264_from_matroska_indexes_and_plans_as_from_mp4() {
    let args = ["-bf", "2", "-pix_fmt", "yuv420p", "-x264-params", "keyint=12:min-keyint=12:scenecut=0:log-level=error"];
    check("h264", "libx264", &args, Codec::H264, "mkv");
}

#[test]
fn h264_open_gop_from_matroska_indexes_as_from_mp4() {
    let args = ["-bf", "2", "-pix_fmt", "yuv420p", "-x264-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=1:log-level=error"];
    check("h264-open-gop", "libx264", &args, Codec::H264, "mkv");
}

#[test]
fn hevc_from_matroska_indexes_and_plans_as_from_mp4() {
    let args = ["-pix_fmt", "yuv420p", "-x265-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=0:bframes=3:log-level=error"];
    check("hevc", "libx265", &args, Codec::Hevc, "mkv");
}

#[test]
fn hevc_cra_open_gop_from_matroska_indexes_as_from_mp4() {
    let args = ["-pix_fmt", "yuv420p", "-x265-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=1:bframes=3:log-level=error"];
    check("hevc-cra", "libx265", &args, Codec::Hevc, "mkv");
}

#[test]
fn av1_from_matroska_and_webm_indexes_and_plans_as_from_mp4() {
    let args = ["-cpu-used", "8", "-g", "12", "-keyint_min", "12", "-lag-in-frames", "16", "-pix_fmt", "yuv420p"];
    check("av1", "libaom-av1", &args, Codec::Av1, "mkv");
    check("av1-webm", "libaom-av1", &args, Codec::Av1, "webm");
}

#[test]
fn mpeg2_from_matroska_indexes_and_plans_as_from_mp4() {
    let args = ["-bf", "2", "-g", "12", "-q:v", "4", "-flags", "+cgop", "-sc_threshold", "1000000000"];
    check("mpeg2", "mpeg2video", &args, Codec::Mpeg2Video, "mkv");
}

#[test]
fn a_track_smart_cut_cannot_copy_is_refused_with_a_reason() {
    let Some((mkv_path, _)) = sources("vp9", "libvpx-vp9", &["-g", "12", "-deadline", "realtime", "-cpu-used", "8"], "mkv") else { return };
    let bytes = std::fs::read(&mkv_path).unwrap();
    let mkv = cfa4_matroska::open(bytes.as_slice()).unwrap();
    let err = index_mkv_video(&mkv, bytes.as_slice()).unwrap_err();
    assert!(matches!(&err, MkvSourceError::Unsupported { codec } if codec == "V_VP9"), "{err:?}");
    assert!(err.to_string().contains("H.264, HEVC, AV1 and MPEG-2"));
    // a file opened without its frame tables is refused, not indexed empty
    let bare = cfa4_matroska::open_with(bytes.as_slice(), &cfa4_matroska::OpenOptions { index: false, verify_crc: false }).unwrap();
    assert_eq!(index_mkv_video(&bare, bytes.as_slice()).unwrap_err(), MkvSourceError::NotIndexed);
}

/// Physical test (R2-07 sample set, `docs/development/deferred-physical-tests.md`): every `.mkv`
/// in `CFA4_SMART_CUT_SAMPLES` indexes, and its key frames are clean access points. Skips when the
/// variable is unset; the samples are never committed.
#[test]
fn physical_matroska_samples_index() {
    let Some(dir) = std::env::var_os("CFA4_SMART_CUT_SAMPLES") else {
        // a physical test: not an oracle test, so CFA4_REQUIRE_ORACLES does not make it fail
        eprintln!("SKIPPED (Smart Cut Matroska samples): CFA4_SMART_CUT_SAMPLES is not set");
        return;
    };
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "mkv") {
            continue;
        }
        let file = std::fs::File::open(&path).unwrap();
        let mkv = cfa4_matroska::open(&file).unwrap();
        let video = index_mkv_video(&mkv, &file).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let ix = &video.index;
        let keys = ix.frames.iter().filter(|f| f.keyframe).count();
        let clean = ix.frames.iter().filter(|f| f.keyframe && f.random_access.is_clean()).count();
        eprintln!("{}: {:?}, {} pictures, {} GOPs, {keys} key frames, {clean} clean", path.display(), video.codec, ix.frames.len(), ix.gops.len());
        assert!(clean > 0 && ix.config_changes.is_empty(), "{}", path.display());
        seen += 1;
    }
    assert!(seen > 0, "no .mkv in CFA4_SMART_CUT_SAMPLES");
}
