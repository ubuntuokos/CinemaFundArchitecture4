//! WP-07: the Smart Cut executor copies HEVC (`hvcC`) samples of an MP4 unchanged, on the clean
//! access points of the HEVC index; FFmpeg decodes the cut to the source pictures.

use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};

use cfa4_isobmff::{Brand, CodecConfig, HevcConfig, Mp4File, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::{Container, Policy, Registry};
use cfa4_smart_edit::copy_planner::{CutMode, PlanRefusal, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::h264_index::NalFormat;
use cfa4_smart_edit::hevc_index::index_hevc;
use cfa4_smart_edit::media_index::{MediaIndex, Packet, RandomAccessKind, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::{CopyGuarantee, ExportExecutionPlan};
use cfa4_smart_edit::timestamp::{MediaTime, Rational};

use crate::smart_cut::{Mp4CopySource, SmartCutError, execute_mp4_copy, verify_mp4_copy};

/// 40 pictures of HEVC from the CFA4 encoder (64×48, 25 fps, an IDR every 10, P pictures).
pub(crate) fn cfa4_hevc_mp4() -> Vec<u8> {
    cfa4_hevc_mp4_with(false)
}

/// [`cfa4_hevc_mp4`]; with `in_band`, every IDR sample also carries the VPS / SPS / PPS (as in
/// `hev1` streams).
pub(crate) fn cfa4_hevc_mp4_with(in_band: bool) -> Vec<u8> {
    use cfa4_hevc::encode::{Encoder, EncoderConfig, PacketFormat, YuvFrame};
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = 10;
    cfg.format = PacketFormat::LengthPrefixed;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40usize {
        let y: Vec<u8> = (0..64 * 48).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..32 * 24).map(|k| ((k * 3 + i) % 255) as u8).collect();
        packets.extend(enc.encode(&YuvFrame::U8 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i as i64).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    let entry = SampleEntry::hevc(HevcConfig::parse(&enc.hvcc()).unwrap(), 64, 48);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = mux.add_track(TrackConfig::new(entry, 25)).unwrap();
    let mut prefix = Vec::new();
    for ps in enc.parameter_sets() {
        prefix.extend((ps.len() as u32).to_be_bytes());
        prefix.extend(ps);
    }
    for p in &packets {
        let data = if in_band && p.keyframe { [prefix.as_slice(), &p.data].concat() } else { p.data.clone() };
        mux.write_sample(t, WriteSample { data: &data, duration: 1, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe }).unwrap();
    }
    mux.finish().unwrap().into_inner()
}

pub(crate) struct Source {
    pub(crate) bytes: Vec<u8>,
    pub(crate) file: Mp4File,
    pub(crate) index: StreamIndex,
    pub(crate) output: Container,
    pub(crate) mode: CutMode,
}

impl Source {
    pub(crate) fn new(bytes: Vec<u8>) -> Source {
        let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
        let track = &file.tracks[0];
        let Some(CodecConfig::Hevc(hvcc)) = track.codec() else { panic!("not HEVC") };
        let ps: Vec<&[u8]> = hvcc.arrays.iter().flat_map(|a| a.nalus.iter().map(Vec::as_slice)).collect();
        let data: Vec<Vec<u8>> = (0..track.samples.len()).map(|i| file.read_sample(bytes.as_slice(), 0, i).unwrap()).collect();
        let packets = track.samples.iter().zip(&data).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: Some(s.dts), keyframe: s.is_sync });
        let index = index_hevc(track.id, Rational::new(1, i64::from(track.timescale)), NalFormat::LengthPrefixed(hvcc.length_size), &ps, packets).unwrap();
        Source { bytes, file, index, output: Container::Mp4, mode: CutMode::GopBoundary }
    }

    pub(crate) fn source(&self) -> Mp4CopySource<'_, [u8]> {
        Mp4CopySource { file: &self.file, bytes: self.bytes.as_slice(), track: 0 }
    }

    /// The plan (in `self.mode`) of `[from, to)` pictures after the first one.
    pub(crate) fn plan(&self, from: i64, to: i64) -> ExportExecutionPlan {
        let p = self.try_plan(from, to).unwrap();
        assert!(p.executable, "{:?}", p.blockers);
        p.plan
    }

    pub(crate) fn try_plan(&self, from: i64, to: i64) -> Result<SmartCutPlan, PlanRefusal> {
        let t = &self.file.tracks[0];
        let pts0 = t.samples.iter().map(|s| s.pts).min().unwrap();
        let step = i64::from(t.samples[0].duration);
        let tb = Rational::new(1, i64::from(t.timescale));
        let tick = |f: i64| MediaTime::new(pts0 + f * step, tb).to_tick().unwrap();
        let req = SmartCutRequest {
            stream_id: self.index.stream_id,
            in_point: tick(from),
            out_point: tick(to),
            mode: self.mode,
            guarantee: CopyGuarantee::BitExact,
            source_container: Container::Mp4,
            output_container: self.output,
            format: match t.codec() {
                Some(CodecConfig::Hevc(hvcc)) => crate::boundary_reencode::hevc_stream_format(hvcc),
                _ => None,
            },
        };
        let media =
            MediaIndex::new("src.mp4", SourceFingerprint { size: self.bytes.len() as u64, modified_ns: None, content_id: None }, vec![self.index.clone()]);
        plan_smart_cut(&Registry::native(), &media, &req, Policy { allow_experimental: false })
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
fn hevc_gops_are_copied_unchanged() {
    let mut src = Source::new(cfa4_hevc_mp4());
    for output in [Container::Mp4, Container::Mov] {
        src.output = output;
        let plan = src.plan(10, 30);
        assert!(plan.is_lossless());
        assert_eq!(plan.copied_frames(), 20);
        let out = src.cut(&plan);
        let back = cfa4_isobmff::open(out.as_slice()).unwrap();
        assert_eq!(back.is_quicktime, output == Container::Mov);
        let t = &back.tracks[0];
        assert!(matches!(t.codec(), Some(CodecConfig::Hevc(_))), "the hvcC sample entry is kept");
        assert_eq!(t.samples.len(), 20);
        for k in 0..20 {
            assert_eq!(back.read_sample(out.as_slice(), 0, k).unwrap(), src.file.read_sample(src.bytes.as_slice(), 0, k + 10).unwrap());
        }
    }
}

#[test]
fn a_stale_hevc_index_is_refused() {
    let src = Source::new(cfa4_hevc_mp4());
    let plan = src.plan(10, 30);
    // the index says the picture at 10 is a CRA, the bitstream holds an IDR
    let mut stale = Source::new(src.bytes.clone());
    stale.index.frames[10].random_access = RandomAccessKind::Cra { rasl: false };
    let e = execute_mp4_copy(&plan, &stale.index, &stale.source(), Cursor::new(Vec::new())).unwrap_err();
    assert!(matches!(&e, SmartCutError::StaleIndex { detail } if detail.contains("IDR")), "{e}");
    // an H.264 plan over an HEVC source
    let mut other = plan.clone();
    other.codec = cfa4_smart_edit::capabilities::Codec::H264;
    assert!(matches!(execute_mp4_copy(&other, &src.index, &src.source(), Cursor::new(Vec::new())), Err(SmartCutError::Unsupported { .. })));
}

/// An x265 MP4 (`params` for `-x265-params`), 100 pictures at 25 fps with B pictures.
pub(crate) fn x265_mp4(ffmpeg: &Path, dir: &Path, name: &str, params: &str) -> Vec<u8> {
    let path = dir.join(name);
    let st = Command::new(ffmpeg)
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=128x96:rate=25", "-t", "4", "-c:v", "libx265", "-pix_fmt", "yuv420p", "-tag:v", "hvc1"])
        .args(["-x265-params", params, "-an"])
        .arg(&path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::read(&path).unwrap()
}

pub(crate) fn framemd5(ffmpeg: &Path, path: &Path) -> Vec<String> {
    let o = Command::new(ffmpeg).args(["-v", "error", "-xerror", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
}

/// FFmpeg as the oracle: x265 closed GOPs with B pictures cut on an IDR decode to the source
/// pictures; in an open GOP stream the CRA pictures with RASL leading pictures are not used as
/// cut-in points.
#[test]
fn ffmpeg_decodes_hevc_cuts_to_the_source_pictures() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("HEVC Smart Cut decode") else { return };
    if !cfa4_testkit::encoders_or_skip("smart_cut_hevc_tests", &ffmpeg, &["libx265"]) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("cfa4-hevc-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // closed GOPs: an IDR every 25 pictures
    let closed = Source::new(x265_mp4(&ffmpeg, &dir, "closed.mp4", "keyint=25:min-keyint=25:scenecut=0:open-gop=0:bframes=3:log-level=error"));
    let plan = closed.plan(25, 75);
    assert_eq!(plan.copied_frames(), 50);
    std::fs::write(dir.join("closed_cut.mp4"), closed.cut(&plan)).unwrap();
    let (src, cut) = (framemd5(&ffmpeg, &dir.join("closed.mp4")), framemd5(&ffmpeg, &dir.join("closed_cut.mp4")));
    assert_eq!(cut.len(), 50);
    assert_eq!(cut[..], src[25..75], "the copied pictures are the source pictures");
    let mut mov = closed;
    mov.output = Container::Mov;
    std::fs::write(dir.join("closed_cut.mov"), mov.cut(&mov.plan(25, 75))).unwrap();
    assert_eq!(framemd5(&ffmpeg, &dir.join("closed_cut.mov")), cut, "the MOV copy decodes like the MP4 copy");
    // open GOPs: a cut-in at a CRA with RASL pictures moves to a clean access point
    let open = Source::new(x265_mp4(&ffmpeg, &dir, "open.mp4", "keyint=25:min-keyint=25:scenecut=0:open-gop=1:bframes=3:log-level=error"));
    let clean = open.index.clean_access_points();
    // a copy stops before the RASL pictures of the next CRA: cut out at the safe point offered
    let Err(PlanRefusal::NotAtSafePoints { safe, .. }) = open.try_plan(0, 50) else { panic!("50 is not a safe out point of an open GOP") };
    let out = safe.out_before.expect("a safe out point before 50").presentation;
    let plan = open.plan(0, out as i64 + 1);
    let first = plan.segments.first().unwrap().decode.first;
    assert!(clean.contains(&first), "copy starts at a clean access point: {first} not in {clean:?}");
    std::fs::write(dir.join("open_cut.mp4"), open.cut(&plan)).unwrap();
    let (src, cut) = (framemd5(&ffmpeg, &dir.join("open.mp4")), framemd5(&ffmpeg, &dir.join("open_cut.mp4")));
    let n = plan.copied_frames() as usize;
    assert_eq!(cut.len(), n);
    assert_eq!(cut[..], src[..n]);
    let _ = std::fs::remove_dir_all(&dir);
}
