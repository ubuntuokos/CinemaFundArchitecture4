//! WP-07: a frame-accurate Smart Cut of HEVC copies the whole GOPs and re-encodes only the
//! boundary pictures with the CFA4 HEVC encoder; the output is one valid HEVC stream.

use std::io::Cursor;

use cfa4_isobmff::CodecConfig;
use cfa4_smart_edit::copy_planner::CutMode;
use cfa4_smart_edit::media_index::RandomAccessKind;
use cfa4_smart_edit::plan::{ExportExecutionPlan, SegmentMode, SegmentReason};

use crate::boundary_reencode::{BoundaryEncoding, BoundaryReport, check_mp4_boundary_reencode, execute_mp4_boundary_reencode, verify_mp4_boundary_reencode};
use crate::smart_cut::SmartCutError;
use crate::smart_cut_hevc_tests::{Source, cfa4_hevc_mp4, framemd5, has_libx265, x265_mp4};

fn encoding() -> BoundaryEncoding {
    BoundaryEncoding { threads: 1, ..BoundaryEncoding::default() }
}

fn frame_accurate(bytes: Vec<u8>) -> Source {
    let mut s = Source::new(bytes);
    s.mode = CutMode::FrameAccurate;
    s
}

/// Run and verify `plan`.
fn run(src: &Source, plan: &ExportExecutionPlan) -> (Vec<u8>, BoundaryReport) {
    let (out, report) = execute_mp4_boundary_reencode(plan, &src.index, &src.source(), &encoding(), Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    let verified = verify_mp4_boundary_reencode(plan, &src.index, &src.source(), &encoding(), out.as_slice()).unwrap();
    assert_eq!((verified.samples, verified.copied, verified.reencoded), (report.samples, report.copied, report.reencoded));
    (out, verified)
}

#[test]
fn a_frame_accurate_hevc_cut_re_encodes_only_the_head() {
    let src = frame_accurate(cfa4_hevc_mp4());
    let plan = src.plan(5, 33);
    let shape: Vec<(SegmentMode, SegmentReason)> = plan.segments.iter().map(|s| (s.mode, s.reason)).collect();
    assert_eq!(shape, [(SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary), (SegmentMode::StrictCopy, SegmentReason::WholeGops)]);
    let (out, report) = run(&src, &plan);
    assert_eq!((report.samples, report.copied, report.reencoded), (28, 23, 5));
    assert!(report.min_psnr.unwrap() >= 30.0, "{report:?}");
    // one sample entry with both parameter-set triples; an IDR at every splice
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    let track = &back.tracks[0];
    let Some(CodecConfig::Hevc(hvcc)) = track.codec() else { panic!("not HEVC") };
    assert!(hvcc.arrays.iter().all(|a| a.nalus.len() == 2), "{:?}", hvcc.arrays.iter().map(|a| (a.nal_type, a.nalus.len())).collect::<Vec<_>>());
    let sync: Vec<usize> = track.samples.iter().enumerate().filter(|(_, s)| s.is_sync).map(|(i, _)| i).collect();
    assert_eq!(sync, [0, 5, 15, 25]);
    // a pure GOP-boundary plan still runs as a copy
    let mut gop = Source::new(src.bytes.clone());
    gop.mode = CutMode::GopBoundary;
    let (_, report) = run(&gop, &gop.plan(10, 30));
    assert_eq!((report.copied, report.reencoded, report.min_psnr), (20, 0, None));
}

#[test]
fn hevc_boundaries_are_checked_before_writing() {
    let src = frame_accurate(cfa4_hevc_mp4());
    let plan = src.plan(5, 33);
    // a stale index: the copied IDR at 10 is not a clean access point any more
    let mut stale = frame_accurate(src.bytes.clone());
    stale.index.frames[10].random_access = RandomAccessKind::Cra { rasl: true };
    let mut out = Cursor::new(Vec::new());
    let e = execute_mp4_boundary_reencode(&plan, &stale.index, &stale.source(), &encoding(), &mut out).unwrap_err();
    assert!(matches!(e, SmartCutError::StaleIndex { .. } | SmartCutError::Incompatible { .. }), "{e}");
    assert!(out.get_ref().is_empty());
    // a poor re-encode fails verification
    let (out, _) = execute_mp4_boundary_reencode(&plan, &src.index, &src.source(), &encoding(), Cursor::new(Vec::new())).unwrap();
    let strict = BoundaryEncoding { min_psnr: 99.5, ..encoding() };
    let e = verify_mp4_boundary_reencode(&plan, &src.index, &src.source(), &strict, out.into_inner().as_slice()).unwrap_err();
    assert!(matches!(&e, SmartCutError::Verification { detail } if detail.contains("dB")), "{e}");
    // an HEVC plan over an H.264 index is not run
    let mut h264 = plan.clone();
    h264.codec = cfa4_smart_edit::capabilities::Codec::H264;
    assert!(matches!(check_mp4_boundary_reencode(&h264, &src.index, &src.source(), &encoding()), Err(SmartCutError::Unsupported { .. })));
}

/// FFmpeg as the oracle: x265 streams with B pictures, closed and open GOPs, cut frame-accurately;
/// the output decodes without errors and the copied pictures are the source pictures. In the open
/// GOP stream the copy after a boundary may start at a clean CRA picture (after an
/// end-of-sequence NAL unit).
#[test]
fn ffmpeg_decodes_spliced_hevc() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("HEVC boundary re-encode decode") else { return };
    if !has_libx265(&ffmpeg) {
        cfa4_testkit::skip("boundary_reencode_hevc_tests", "ffmpeg has no libx265");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cfa4-hevc-splice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, open_gop) in [("closed", 0), ("open", 1)] {
        let params = format!("keyint=25:min-keyint=25:scenecut=0:open-gop={open_gop}:bframes=3:log-level=error");
        let src = frame_accurate(x265_mp4(&ffmpeg, &dir, &format!("{name}.mp4"), &params));
        let n = src.index.frames.len() as i64;
        let mut copied_from_cra = false;
        for (from, to) in [(10, 60), (40, n)] {
            let plan = src.plan(from, to);
            copied_from_cra |= plan
                .segments
                .iter()
                .any(|s| s.mode == SegmentMode::StrictCopy && matches!(src.index.frames[s.decode.first as usize].random_access, RandomAccessKind::Cra { .. }));
            assert!(plan.reencoded_frames() > 0, "{name} {from}..{to}: {:?}", plan.segments);
            let (out, report) = run(&src, &plan);
            assert_eq!(report.samples, (to - from) as u64, "{name} {from}..{to}");
            let cut = dir.join(format!("{name}_{from}_{to}.mp4"));
            std::fs::write(&cut, out).unwrap();
            let (a, b) = (framemd5(&ffmpeg, &dir.join(format!("{name}.mp4"))), framemd5(&ffmpeg, &cut));
            assert_eq!(b.len(), (to - from) as usize, "{name} {from}..{to}: every picture decodes");
            // the copied pictures are the source pictures
            for s in plan.segments.iter().filter(|s| s.mode == SegmentMode::StrictCopy) {
                for p in s.presentation.first..=s.presentation.last {
                    let p = p as usize;
                    assert_eq!(b[p - from as usize], a[p], "{name} {from}..{to}: copied picture {p}");
                }
            }
        }
        assert_eq!(copied_from_cra, open_gop == 1, "{name}: a copy starts at a clean CRA picture after a boundary");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Parameter sets inside the samples could collide with the encoder's ids: refused.
#[test]
fn in_band_parameter_sets_are_refused() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("HEVC in-band parameter sets") else { return };
    if !has_libx265(&ffmpeg) {
        cfa4_testkit::skip("boundary_reencode_hevc_tests", "ffmpeg has no libx265");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cfa4-hevc-inband-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = frame_accurate(x265_mp4(&ffmpeg, &dir, "inband.mp4", "keyint=25:min-keyint=25:scenecut=0:bframes=0:repeat-headers=1:log-level=error"));
    let plan = src.plan(10, 60);
    let e = check_mp4_boundary_reencode(&plan, &src.index, &src.source(), &encoding()).unwrap_err();
    assert!(matches!(&e, SmartCutError::Incompatible { detail } if detail.contains("in-band")), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
