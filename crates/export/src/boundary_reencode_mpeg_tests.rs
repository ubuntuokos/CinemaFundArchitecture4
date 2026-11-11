//! WP-09e: a frame-accurate Smart Cut of MPEG-2 video in program and transport streams copies the
//! whole GOPs and re-encodes only the boundary pictures; the output decodes as one stream, keeps
//! the audio of the cut and is checked against the source.

use std::path::Path;
use std::process::Command;

use cfa4_mpegts::{File, Kind, MuxCodec, MuxFormat, Muxer};
use cfa4_smart_edit::capabilities::{Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::media_index::{MediaIndex, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::{CopyGuarantee, SegmentMode, SegmentReason};
use cfa4_smart_edit::timestamp::{MediaTime, Rational};

use crate::boundary_reencode::BoundaryEncoding;
use crate::boundary_reencode_mpeg::{check_mpeg_boundary_reencode, execute_mpeg_boundary_reencode, mpeg2_stream_format, verify_mpeg_boundary_reencode};
use crate::smart_cut::SmartCutError;
use crate::smart_cut_mpeg::{MpegCopySource, container_of, index_mpeg_video};

const FRAME: i64 = 3600;

/// MPEG-2 video (64×48, 25 fps, two B pictures between anchors; closed GOPs of 10, or open GOPs
/// of 9 whose two leading B pictures refer to the GOP before) and one audio
/// stream: MP2 in a transport stream, LPCM in a program stream. Picture `i` is shown at
/// `pts(i)` frames.
fn source(format: MuxFormat, closed: bool, pts: impl Fn(i64) -> i64) -> Vec<u8> {
    use cfa4_mpeg2v::encode::{Config, Encoder, Input};
    let (w, h) = (64usize, 48usize);
    let mut cfg = Config::new(w as u32, h as u32, 25, 1);
    cfg.gop = if closed { 10 } else { 9 };
    cfg.b_frames = 2;
    cfg.closed_gop = closed;
    cfg.threads = false;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..40 {
        // a moving gradient: the pictures differ, and decode to the same values everywhere
        let y: Vec<u8> = (0..w * h).map(|k| (40 + ((k % w) * 2 + (k / w) + i * 3) % 170) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| (100 + (k % 32 + i) % 50) as u8).collect();
        packets.extend(enc.encode(&Input { y: &y, cb: &c, cr: &c, y_stride: w, c_stride: w / 2 }).unwrap());
    }
    packets.extend(enc.flush().unwrap());
    let audio = if format == MuxFormat::Ps { MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2 } } else { MuxCodec::MpegAudio };
    let mut m = Muxer::new(Vec::new(), format, &[MuxCodec::Mpeg2Video, audio], 20_000_000).unwrap();
    // DTS: the decode position, one frame behind (B pictures reorder by one anchor)
    for p in &packets {
        let (shown, dts) = (pts(p.index as i64) * FRAME, (p.dts) * FRAME);
        let dts = dts.min(shown);
        m.write(0, &p.data, shown, (dts != shown).then_some(dts), p.keyframe).unwrap();
    }
    if format == MuxFormat::Ps {
        for k in 0..160i64 {
            let unit: Vec<u8> = (0..960i64).flat_map(|i| ((((i * 37 + k * 11) % 4000) - 2000) as i16).to_be_bytes()).collect();
            m.write(1, &unit, k * 900, None, true).unwrap();
        }
    } else {
        for k in 0..67i64 {
            let mut f = vec![0xFF, 0xFD, 0xA4, 0x00];
            f.extend((0..572).map(|i| (i as i64 + k * 3) as u8));
            m.write(1, &f, k * 2160, None, true).unwrap();
        }
    }
    m.finish().unwrap()
}

struct Fixture {
    bytes: Vec<u8>,
    file: File,
    video: usize,
    index: StreamIndex,
    format: MuxFormat,
}

impl Fixture {
    fn new(bytes: Vec<u8>, format: MuxFormat) -> Fixture {
        let file = cfa4_mpegts::open(&bytes).unwrap();
        let video = file.find(Kind::Video, |_| true).unwrap();
        let index = index_mpeg_video(&file, &bytes, video).unwrap();
        Fixture { bytes, file, video, index, format }
    }
    fn of(format: MuxFormat) -> Fixture {
        Fixture::new(source(format, true, |i| i), format)
    }
    fn source(&self) -> MpegCopySource<'_, Vec<u8>> {
        MpegCopySource { file: &self.file, bytes: &self.bytes, video: self.video, audio: true }
    }
    fn stream_format(&self) -> StreamFormat {
        let data = self.file.read_unit(&self.bytes, self.video, 0).unwrap();
        let mut dec = cfa4_mpeg2v::Decoder::new();
        let _ = dec.decode(&data, 0);
        mpeg2_stream_format(dec.sequence().unwrap())
    }
    /// Plan presented pictures `[from, to)`.
    fn plan(&self, from: i64, to: i64, mode: CutMode) -> SmartCutPlan {
        let pts0 = self.index.frames.iter().filter_map(|f| f.pts).min().unwrap();
        let tick = |f: i64| MediaTime::new(pts0 + f * FRAME, Rational::new(1, 90_000)).to_tick().unwrap();
        let container = container_of(&self.file);
        let req = SmartCutRequest {
            stream_id: self.index.stream_id,
            in_point: tick(from),
            out_point: tick(to),
            mode,
            guarantee: CopyGuarantee::BitExact,
            source_container: container,
            output_container: container,
            format: Some(self.stream_format()),
        };
        let media =
            MediaIndex::new("src.mpg", SourceFingerprint { size: self.bytes.len() as u64, modified_ns: None, content_id: None }, vec![self.index.clone()]);
        plan_smart_cut(&Registry::native(), &media, &req, Policy { allow_experimental: false }).unwrap()
    }
}

fn encoding() -> BoundaryEncoding {
    BoundaryEncoding { threads: 1, ..BoundaryEncoding::default() }
}

#[test]
fn a_frame_accurate_mpeg2_cut_re_encodes_only_the_boundaries() {
    for format in [MuxFormat::Ts, MuxFormat::Ps] {
        let f = Fixture::of(format);
        let p = f.plan(5, 33, CutMode::FrameAccurate);
        assert!(p.executable, "{format:?}: {:?}", p.blockers);
        let shape: Vec<(SegmentMode, SegmentReason)> = p.plan.segments.iter().map(|s| (s.mode, s.reason)).collect();
        assert_eq!(
            shape,
            [
                (SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary),
                (SegmentMode::StrictCopy, SegmentReason::WholeGops),
                (SegmentMode::BoundaryReencode, SegmentReason::TailBoundary)
            ],
            "{format:?}"
        );
        check_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), format, &encoding()).unwrap();
        let (out, report) = execute_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), format, &encoding(), Vec::new()).unwrap();
        // the I picture 30 is copied too: the tail re-encodes only the two B pictures after it
        assert_eq!((report.samples, report.copied, report.reencoded), (28, 21, 7), "{format:?}");
        assert_eq!(report.duration, 28 * FRAME as u64);
        assert_eq!(report.audio.len(), 1, "{format:?}: {:?}", report.dropped);
        let verified = verify_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), format, &encoding(), &out).unwrap();
        assert!(verified.min_psnr.unwrap() >= 30.0, "{verified:?}");
        assert_eq!((verified.samples, verified.copied, verified.reencoded, &verified.audio), (28, 21, 7, &report.audio));
        // the output opens cleanly and starts with an I picture
        let back = cfa4_mpegts::open(&out).unwrap();
        assert!(back.warnings.is_empty(), "{format:?}: {:?}", back.warnings);
        let v = back.find(Kind::Video, |_| true).unwrap();
        assert!(back.streams[v].units[0].key);
        // a GOP-boundary plan through the same executor is a pure copy
        let gop = f.plan(10, 30, CutMode::GopBoundary);
        let (out, report) = execute_mpeg_boundary_reencode(&gop.plan, &f.index, &f.source(), format, &encoding(), Vec::new()).unwrap();
        assert_eq!((report.copied, report.reencoded), (20, 0));
        let verified = verify_mpeg_boundary_reencode(&gop.plan, &f.index, &f.source(), format, &encoding(), &out).unwrap();
        assert_eq!(verified.min_psnr, None);
    }
}

#[test]
fn a_cut_inside_one_gop_and_open_gops_are_re_encoded_too() {
    // no whole GOP inside: everything re-encoded from the access point before it
    let f = Fixture::of(MuxFormat::Ts);
    let p = f.plan(12, 17, CutMode::FrameAccurate);
    assert!(p.executable, "{:?}", p.blockers);
    let (out, report) = execute_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &encoding(), Vec::new()).unwrap();
    assert_eq!((report.copied, report.reencoded), (0, 5));
    verify_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &encoding(), &out).unwrap();
    // open GOPs: the leading B pictures of a copied GOP need the GOP before it
    let open = Fixture::new(source(MuxFormat::Ts, false, |i| i), MuxFormat::Ts);
    let p = open.plan(5, 33, CutMode::FrameAccurate);
    assert!(p.executable, "{:?}", p.blockers);
    let (out, report) = execute_mpeg_boundary_reencode(&p.plan, &open.index, &open.source(), MuxFormat::Ts, &encoding(), Vec::new()).unwrap();
    assert_eq!(report.samples, 28);
    let verified = verify_mpeg_boundary_reencode(&p.plan, &open.index, &open.source(), MuxFormat::Ts, &encoding(), &out).unwrap();
    assert!(verified.reencoded >= 8 && verified.min_psnr.unwrap() >= 30.0, "{verified:?}");
}

#[test]
fn boundaries_the_encoder_cannot_reproduce_are_refused_before_writing() {
    // picture 7 is shown two frames long: the head 5–9 has a variable frame rate
    let vfr = Fixture::new(source(MuxFormat::Ts, true, |i| if i > 7 { i + 1 } else { i }), MuxFormat::Ts);
    let p = vfr.plan(5, 33, CutMode::FrameAccurate);
    let e = check_mpeg_boundary_reencode(&p.plan, &vfr.index, &vfr.source(), MuxFormat::Ts, &encoding()).unwrap_err();
    assert!(e.to_string().contains("variable frame rate"), "{e}");
    assert!(execute_mpeg_boundary_reencode(&p.plan, &vfr.index, &vfr.source(), MuxFormat::Ts, &encoding(), Vec::new()).is_err());
    // another output container, or a stale index
    let f = Fixture::of(MuxFormat::Ts);
    let p = f.plan(5, 33, CutMode::FrameAccurate);
    assert!(matches!(check_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ps, &encoding()), Err(SmartCutError::Unsupported { .. })));
    let mut stale = f.index.clone();
    stale.frames.pop();
    assert!(matches!(check_mpeg_boundary_reencode(&p.plan, &stale, &f.source(), MuxFormat::Ts, &encoding()), Err(SmartCutError::StaleIndex { .. })));
}

#[test]
fn verification_catches_changed_pictures_and_a_poor_re_encode() {
    let f = Fixture::of(MuxFormat::Ts);
    let p = f.plan(5, 33, CutMode::FrameAccurate);
    let (out, _) = execute_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &encoding(), Vec::new()).unwrap();
    // a copied picture changed: a byte of the first copied unit's slice data
    let back = cfa4_mpegts::open(&out).unwrap();
    let v = back.find(Kind::Video, |_| true).unwrap();
    let unit = back.read_unit(&out, v, 5).unwrap();
    let at = out.windows(16).position(|w| w == &unit[unit.len() - 20..unit.len() - 4]).unwrap();
    let mut bad = out.clone();
    bad[at + 3] ^= 0x10;
    assert!(matches!(verify_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &encoding(), &bad), Err(SmartCutError::Verification { .. })));
    // a coarse re-encode measured against a strict bound
    let coarse = BoundaryEncoding { crf: 51.0, min_psnr: 60.0, threads: 1 };
    let (out, _) = execute_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &coarse, Vec::new()).unwrap();
    let e = verify_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), MuxFormat::Ts, &coarse, &out).unwrap_err();
    assert!(e.to_string().contains("dB"), "{e}");
}

fn framemd5(ffmpeg: &Path, path: &Path) -> Vec<String> {
    let o = Command::new(ffmpeg).args(["-v", "error", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
}

/// FFmpeg (oracle) decodes the spliced stream: 28 pictures, the copied ones equal to the source's.
#[test]
fn ffmpeg_decodes_the_spliced_mpeg2_stream() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("MPEG-2 boundary re-encode") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-mpeg2-splice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, format) in [("ts", MuxFormat::Ts), ("mpg", MuxFormat::Ps)] {
        let f = Fixture::of(format);
        let p = f.plan(5, 33, CutMode::FrameAccurate);
        let (out, _) = execute_mpeg_boundary_reencode(&p.plan, &f.index, &f.source(), format, &encoding(), Vec::new()).unwrap();
        let (src, cut) = (dir.join(format!("src.{name}")), dir.join(format!("cut.{name}")));
        std::fs::write(&src, &f.bytes).unwrap();
        std::fs::write(&cut, &out).unwrap();
        let (all, spliced) = (framemd5(&ffmpeg, &src), framemd5(&ffmpeg, &cut));
        assert_eq!(all.len(), 40, "{name}");
        assert_eq!(spliced.len(), 28, "{name}");
        assert_eq!(spliced[5..26], all[10..31], "{name}: the copied GOPs decode to the source pictures");
    }
    // open GOPs: the leading B pictures were re-encoded, every picture of the cut decodes once
    let open = Fixture::new(source(MuxFormat::Ts, false, |i| i), MuxFormat::Ts);
    let p = open.plan(5, 33, CutMode::FrameAccurate);
    let (out, _) = execute_mpeg_boundary_reencode(&p.plan, &open.index, &open.source(), MuxFormat::Ts, &encoding(), Vec::new()).unwrap();
    let cut = dir.join("open.ts");
    std::fs::write(&cut, &out).unwrap();
    assert_eq!(framemd5(&ffmpeg, &cut).len(), 28);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_registry_allows_mpeg2_boundary_re_encodes_into_program_and_transport_streams() {
    let f = Fixture::of(MuxFormat::Ps);
    let fmt = f.stream_format();
    assert_eq!((fmt.profile.as_str(), fmt.chroma.as_str(), fmt.bit_depth), ("main", "4:2:0", 8));
    for c in [Container::MpegPs, Container::MpegTs] {
        let d = Registry::native().smart_render(cfa4_smart_edit::capabilities::Codec::Mpeg2Video, c, c, &fmt, Policy { allow_experimental: false });
        assert!(d.allowed, "{c:?}: {:?}", d.blockers());
    }
    let mp4 = Registry::native().smart_render(
        cfa4_smart_edit::capabilities::Codec::Mpeg2Video,
        Container::MpegPs,
        Container::Mp4,
        &fmt,
        Policy { allow_experimental: false },
    );
    assert!(!mp4.allowed);
    let _ = f.format;
}
