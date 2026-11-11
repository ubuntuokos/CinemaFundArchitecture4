//! WP-09d: the edge cases the WP-09 validation report left partial. A source in another
//! timescale (case 9), a variable-frame-rate source (case 10) and an HDR source (case 15) are cut
//! losslessly and verified; what the boundary re-encoder cannot honour is refused before writing.

use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use cfa4_isobmff::{
    AvcConfig, Brand, CodecConfig, ColorInfo, HevcConfig, MasteringDisplay, Mp4File, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use cfa4_smart_edit::capabilities::{Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::h264_index::NalFormat;
use cfa4_smart_edit::hevc_index::index_hevc;
use cfa4_smart_edit::media_index::{Packet, StreamIndex};
use cfa4_smart_edit::plan::CopyGuarantee;
use cfa4_smart_edit::timestamp::{MediaTime, Rational};

use crate::boundary_reencode::{BoundaryEncoding, check_mp4_boundary_reencode, execute_mp4_boundary_reencode, verify_mp4_boundary_reencode};
use crate::smart_cut::{Mp4CopySource, execute_mp4_copy, verify_mp4_copy};
use crate::smart_cut_tests::{index_of, media};

const STRICT: Policy = Policy { allow_experimental: false };

/// 40 pictures of H.264 (64×48, an IDR every 10, no B-frames) in `timescale`, picture `i` lasting
/// `duration(i)` ticks.
fn h264_mp4(timescale: u32, duration: impl Fn(usize) -> u32) -> Vec<u8> {
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
    let t = mux.add_track(TrackConfig::new(entry, timescale)).unwrap();
    for (i, p) in packets.iter().enumerate() {
        mux.write_sample(t, WriteSample { data: &p.data, duration: duration(i), composition_offset: 0, is_sync: p.keyframe }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

struct Src {
    bytes: Vec<u8>,
    file: Mp4File,
    index: StreamIndex,
}

impl Src {
    fn h264(bytes: Vec<u8>) -> Src {
        let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
        let index = index_of(&file, &bytes);
        Src { bytes, file, index }
    }
    fn source(&self) -> Mp4CopySource<'_, [u8]> {
        Mp4CopySource { file: &self.file, bytes: self.bytes.as_slice(), track: 0 }
    }
    /// The PTS (source timescale) of picture `n` in presentation order; `n == len` is the end.
    fn pts(&self, n: usize) -> i64 {
        let mut s: Vec<(i64, u32)> = self.file.tracks[0].samples.iter().map(|s| (s.pts, s.duration)).collect();
        s.sort_unstable();
        match s.get(n) {
            Some(&(p, _)) => p,
            None => s.last().map_or(0, |&(p, d)| p + i64::from(d)),
        }
    }
    /// Plan pictures `[from, to)` in `mode`.
    fn plan(&self, from: usize, to: usize, mode: CutMode, format: Option<StreamFormat>) -> SmartCutPlan {
        let tb = Rational::new(1, i64::from(self.file.tracks[0].timescale));
        let tick = |p: i64| MediaTime::new(p, tb).to_tick().unwrap();
        let req = SmartCutRequest {
            stream_id: self.index.stream_id,
            in_point: tick(self.pts(from)),
            out_point: tick(self.pts(to)),
            mode,
            guarantee: CopyGuarantee::BitExact,
            source_container: Container::Mp4,
            output_container: Container::Mp4,
            format,
        };
        plan_smart_cut(&Registry::native(), &media(&self.index, self.bytes.len()), &req, STRICT).unwrap()
    }
    /// Copy and verify a lossless plan of `[from, to)`.
    fn copy(&self, from: usize, to: usize) -> Vec<u8> {
        let p = self.plan(from, to, CutMode::GopBoundary, None);
        assert!(p.executable, "{:?}", p.blockers);
        let (out, report) = execute_mp4_copy(&p.plan, &self.index, &self.source(), Cursor::new(Vec::new())).unwrap();
        let out = out.into_inner();
        assert_eq!(verify_mp4_copy(&p.plan, &self.index, &self.source(), out.as_slice()).unwrap(), report);
        out
    }
}

fn framemd5(ffmpeg: &Path, path: &Path) -> Vec<String> {
    let o = Command::new(ffmpeg).args(["-v", "error", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
}

/// Case 9: one stream in three timescales (25, 90 000 and 12 800 ticks per second) is cut to the
/// same samples, each copy keeping its own timescale and durations.
#[test]
fn one_stream_in_other_timescales_is_cut_to_the_same_samples() {
    let sources = [(25u32, 1u32), (90_000, 3600), (12_800, 512)].map(|(ts, d)| (ts, d, Src::h264(h264_mp4(ts, |_| d))));
    let mut payloads = Vec::new();
    for (ts, d, src) in &sources {
        let out = src.copy(10, 30);
        let back = cfa4_isobmff::open(out.as_slice()).unwrap();
        let t = &back.tracks[0];
        assert_eq!(t.timescale, *ts);
        assert_eq!(t.samples.len(), 20);
        assert!(t.samples.iter().all(|s| s.duration == *d), "{ts}");
        payloads.push((0..20).map(|k| back.read_sample(out.as_slice(), 0, k).unwrap()).collect::<Vec<_>>());
    }
    assert!(payloads.windows(2).all(|w| w[0] == w[1]));
}

/// The durations of the VFR source: 40 ms pictures with a 120 ms one every 7th and an 80 ms one in
/// the first GOP's tail (milliseconds).
fn vfr(i: usize) -> u32 {
    if i % 7 == 6 {
        120
    } else if i == 7 {
        80
    } else {
        40
    }
}

/// Case 10: a variable-frame-rate source is copied with every picture's own duration; a frame-
/// accurate cut whose re-encoded pictures do not share one duration is refused before writing.
#[test]
fn a_variable_frame_rate_source_keeps_its_durations() {
    let src = Src::h264(h264_mp4(1000, vfr));
    let out = src.copy(10, 30);
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    let durations: Vec<u32> = back.tracks[0].samples.iter().map(|s| s.duration).collect();
    assert_eq!(durations, (10..30).map(vfr).collect::<Vec<_>>());
    // the presentation times follow the durations from the first copied picture
    let mut at = 0i64;
    for (k, s) in back.tracks[0].samples.iter().enumerate() {
        assert_eq!(s.pts - back.tracks[0].samples[0].pts, at, "picture {k}");
        at += i64::from(s.duration);
    }
    // head 5–9 holds 40, 120 (6) and 80 (7) ms pictures: the encoder signals one rate
    let fmt = Some(StreamFormat { profile: "high".into(), chroma: "4:2:0".into(), bit_depth: 8 });
    let p = src.plan(5, 30, CutMode::FrameAccurate, fmt.clone());
    assert!(p.executable, "{:?}", p.blockers);
    let e = check_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &BoundaryEncoding::default()).unwrap_err();
    assert!(e.to_string().contains("variable frame rate"), "{e}");
    assert!(execute_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &BoundaryEncoding::default(), Cursor::new(Vec::new())).is_err());
    // pictures 21–26 all last 40 ms: re-encoded (no whole GOP inside), with their durations
    let p = src.plan(21, 27, CutMode::FrameAccurate, fmt);
    let encoding = BoundaryEncoding { threads: 1, ..BoundaryEncoding::default() };
    check_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &encoding).unwrap();
    let (out, report) = execute_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &encoding, Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    let verified = verify_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &encoding, out.as_slice()).unwrap();
    assert_eq!((verified.reencoded, verified.copied), (report.reencoded, report.copied));
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    assert_eq!(back.tracks[0].samples.iter().map(|s| s.duration).collect::<Vec<_>>(), (21..27).map(vfr).collect::<Vec<_>>());
}

/// FFmpeg (oracle) decodes the VFR and the 90 kHz copies to the source pictures.
#[test]
fn ffmpeg_decodes_the_timescale_and_vfr_copies() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("Smart Cut timescale / VFR copies") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-smart-cut-edge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in [("vfr", Src::h264(h264_mp4(1000, vfr))), ("90k", Src::h264(h264_mp4(90_000, |_| 3600)))] {
        let (s, o) = (dir.join(format!("{name}-src.mp4")), dir.join(format!("{name}-cut.mp4")));
        std::fs::write(&s, &src.bytes).unwrap();
        std::fs::write(&o, src.copy(10, 30)).unwrap();
        let (all, cut) = (framemd5(&ffmpeg, &s), framemd5(&ffmpeg, &o));
        assert_eq!(all.len(), 40, "{name}");
        assert_eq!(cut, all[10..30], "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// HEVC Main 10 BT.2020 / PQ with ST 2086 and content light SEI in every IDR, and the same metadata
/// in the sample entry (`colr`, `mdcv`, `clli`).
fn hdr_hevc_mp4() -> (Vec<u8>, MasteringDisplay) {
    use cfa4_hevc::encode::{ColorConfig, Encoder, EncoderConfig, PacketFormat, YuvFrame};
    let md = MasteringDisplay::bt2020(1000.0, 0.005);
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = 10;
    cfg.bit_depth = 10;
    cfg.format = PacketFormat::LengthPrefixed;
    cfg.color = ColorConfig { primaries: 9, transfer: 16, matrix: 9, full_range: false };
    cfg.mastering_display = Some(md.to_bytes().try_into().unwrap());
    cfg.content_light = Some((1000, 400));
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40usize {
        let y: Vec<u16> = (0..64 * 48).map(|k| (64 + (k * 5 + i * 11) % 876) as u16).collect();
        let c: Vec<u16> = (0..32 * 24).map(|k| (64 + (k * 3 + i) % 896) as u16).collect();
        packets.extend(enc.encode(&YuvFrame::U16 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i as i64).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    let mut entry = SampleEntry::hevc(HevcConfig::parse(&enc.hvcc()).unwrap(), 64, 48);
    if let Some(v) = entry.video.as_mut() {
        v.color = Some(ColorInfo::Nclx { primaries: 9, transfer: 16, matrix: 9, full_range: false });
        v.mastering_display = Some(md);
        v.content_light = Some((1000, 400));
    }
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(entry, 25)).unwrap();
    for p in &packets {
        mux.write_sample(t, WriteSample { data: &p.data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe }).unwrap();
    }
    (mux.finish().unwrap().into_inner(), md)
}

fn hevc_src(bytes: Vec<u8>) -> Src {
    let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
    let track = &file.tracks[0];
    let Some(CodecConfig::Hevc(hvcc)) = track.codec() else { panic!("not HEVC") };
    let ps: Vec<&[u8]> = hvcc.arrays.iter().flat_map(|a| a.nalus.iter().map(Vec::as_slice)).collect();
    let data: Vec<Vec<u8>> = (0..track.samples.len()).map(|i| file.read_sample(bytes.as_slice(), 0, i).unwrap()).collect();
    let packets = track.samples.iter().zip(&data).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: Some(s.dts), keyframe: s.is_sync });
    let index = index_hevc(track.id, Rational::new(1, i64::from(track.timescale)), NalFormat::LengthPrefixed(hvcc.length_size), &ps, packets).unwrap();
    Src { bytes, file, index }
}

/// The prefix SEI NAL units (type 39) of a length-prefixed HEVC sample.
fn prefix_sei(sample: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= sample.len() {
        let len = u32::from_be_bytes(sample[at..at + 4].try_into().unwrap()) as usize;
        let nal = &sample[at + 4..(at + 4 + len).min(sample.len())];
        if nal.first().is_some_and(|b| (b >> 1) & 0x3f == 39) {
            out.push(nal.to_vec());
        }
        at += 4 + len;
    }
    out
}

/// Case 15: an HDR source keeps its colour description and static metadata. A copy keeps the
/// sample entry and the SEI byte for byte; a re-encoded boundary carries the same SEI in its IDR.
#[test]
fn hdr_metadata_survives_copies_and_re_encoded_boundaries() {
    let (bytes, md) = hdr_hevc_mp4();
    let src = hevc_src(bytes);
    let source_sei = prefix_sei(&src.file.read_sample(src.bytes.as_slice(), 0, 0).unwrap());
    assert!(!source_sei.is_empty(), "the source IDR carries its HDR SEI");
    let check_entry = |out: &[u8]| {
        let back = cfa4_isobmff::open(out).unwrap();
        let v = back.tracks[0].entries[0].video.as_ref().unwrap();
        assert_eq!(v.color, Some(ColorInfo::Nclx { primaries: 9, transfer: 16, matrix: 9, full_range: false }));
        assert_eq!((v.mastering_display, v.content_light), (Some(md), Some((1000, 400))));
        back
    };
    // a GOP copy
    let out = src.copy(10, 30);
    let back = check_entry(&out);
    assert_eq!(prefix_sei(&back.read_sample(out.as_slice(), 0, 0).unwrap()), source_sei);
    // a frame-accurate cut: the re-encoded head (Main 10) starts with an IDR carrying the same SEI
    let fmt = match src.file.tracks[0].codec() {
        Some(CodecConfig::Hevc(hvcc)) => crate::boundary_reencode::hevc_stream_format(hvcc),
        _ => None,
    };
    assert_eq!(fmt.as_ref().map(|f| f.bit_depth), Some(10));
    let p = src.plan(5, 30, CutMode::FrameAccurate, fmt);
    assert!(p.executable, "{:?}", p.blockers);
    let encoding = BoundaryEncoding { threads: 1, ..BoundaryEncoding::default() };
    let (out, report) = execute_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &encoding, Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    let verified = verify_mp4_boundary_reencode(&p.plan, &src.index, &src.source(), &encoding, out.as_slice()).unwrap();
    assert_eq!((verified.reencoded, report.reencoded), (5, 5));
    let back = check_entry(&out);
    assert!(back.tracks[0].samples[0].is_sync);
    let sei = prefix_sei(&back.read_sample(out.as_slice(), 0, 0).unwrap());
    assert_eq!(sei, source_sei, "the re-encoded IDR carries the source's HDR SEI");
}

/// FFprobe (oracle) reads the copy's colour description as BT.2020 / PQ.
#[test]
fn ffprobe_reads_the_hdr_copy_as_bt2020_pq() {
    let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("Smart Cut HDR copy") else { return };
    let src = hevc_src(hdr_hevc_mp4().0);
    let dir = std::env::temp_dir().join(format!("cfa4-smart-cut-hdr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cut.mp4");
    std::fs::write(&path, src.copy(10, 30)).unwrap();
    let o =
        Command::new(&ffprobe).args(["-v", "error", "-show_streams", "-show_frames", "-read_intervals", "%+#1", "-of", "json"]).arg(&path).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let s = &v["streams"][0];
    assert_eq!(
        (s["color_primaries"].as_str(), s["color_transfer"].as_str(), s["pix_fmt"].as_str()),
        (Some("bt2020"), Some("smpte2084"), Some("yuv420p10le")),
        "{s}"
    );
    let side = v["frames"][0]["side_data_list"].to_string();
    assert!(side.contains("Mastering display") && side.contains("Content light"), "{side}");
}
