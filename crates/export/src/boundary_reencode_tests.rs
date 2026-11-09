//! WP-06: a frame-accurate Smart Cut copies the whole GOPs and re-encodes only the boundary
//! pictures; the output is one valid H.264 stream with the planned timeline.

use std::io::Cursor;
use std::process::Command;

use cfa4_smart_edit::capabilities::Container;
use cfa4_smart_edit::copy_planner::CutMode;
use cfa4_smart_edit::plan::{SegmentMode, SegmentReason, SegmentSource};

use crate::boundary_reencode::{BoundaryEncoding, BoundaryReport, execute_mp4_boundary_reencode, verify_mp4_boundary_reencode};
use crate::smart_cut::SmartCutError;
use crate::smart_cut_tests::{Fixture, fixture, plan};

fn encoding() -> BoundaryEncoding {
    BoundaryEncoding { threads: 1, ..BoundaryEncoding::default() }
}

/// Plan `[from, to)` frames (relative to the first picture), run it and verify it.
fn run(f: &Fixture, from: i64, to: i64, mode: CutMode) -> (Vec<u8>, BoundaryReport, cfa4_smart_edit::plan::ExportExecutionPlan) {
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + from, p0 + to, mode, Container::Mp4).plan;
    let (out, report) = execute_mp4_boundary_reencode(&p, &f.index, &f.source(), &encoding(), Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    let verified = verify_mp4_boundary_reencode(&p, &f.index, &f.source(), &encoding(), out.as_slice()).unwrap();
    assert_eq!((verified.samples, verified.copied, verified.reencoded), (report.samples, report.copied, report.reencoded));
    (out, verified, p)
}

#[test]
fn a_frame_accurate_cut_re_encodes_only_the_boundaries() {
    use SegmentMode::{BoundaryReencode, StrictCopy};
    use SegmentReason::{HeadBoundary, TailBoundary, WholeGops};
    // without B-frames the copy may end anywhere; with them the tail after the IDR at 30 is
    // re-encoded, and the copied IDR is followed directly by the re-encoded one
    let cases = [
        (0u8, vec![(BoundaryReencode, HeadBoundary), (StrictCopy, WholeGops)], (23u64, 5u64), vec![0usize, 5, 15, 25]),
        (2, vec![(BoundaryReencode, HeadBoundary), (StrictCopy, WholeGops), (BoundaryReencode, TailBoundary)], (21, 7), vec![0, 5, 15, 25, 26]),
    ];
    for (bframes, modes, (copied, reencoded), sync) in cases {
        let f = fixture(40, 10, bframes);
        let (out, report, p) = run(&f, 5, 33, CutMode::FrameAccurate);
        assert_eq!(p.segments.iter().map(|s| (s.mode, s.reason)).collect::<Vec<_>>(), modes, "bframes {bframes}");
        assert_eq!((report.samples, report.copied, report.reencoded), (28, copied, reencoded), "bframes {bframes}");
        assert_eq!((report.duration, report.timescale), (28, 25));
        assert!(report.min_psnr.unwrap() >= 30.0, "{report:?}");
        // one track, one sample entry with both parameter-set pairs, an IDR at every splice
        let back = cfa4_isobmff::open(out.as_slice()).unwrap();
        let track = &back.tracks[0];
        let Some(cfa4_isobmff::CodecConfig::Avc(avc)) = track.codec() else { panic!("not H.264") };
        assert_eq!((avc.sps.len(), avc.pps.len()), (2, 2));
        let found: Vec<usize> = track.samples.iter().enumerate().filter(|(_, s)| s.is_sync).map(|(i, _)| i).collect();
        assert_eq!(found, sync, "bframes {bframes}");
    }
}

#[test]
fn a_range_shorter_than_a_gop_is_re_encoded_whole() {
    let f = fixture(40, 10, 2);
    let (_, report, p) = run(&f, 12, 17, CutMode::FrameAccurate);
    assert_eq!(p.segments.len(), 1);
    assert_eq!(p.segments[0].reason, SegmentReason::NoCopyableRange);
    assert_eq!((report.samples, report.copied, report.reencoded), (5, 0, 5));
}

#[test]
fn a_gop_boundary_plan_runs_as_a_pure_copy() {
    let f = fixture(40, 10, 2);
    let (_, report, _) = run(&f, 10, 30, CutMode::GopBoundary);
    assert_eq!((report.samples, report.copied, report.reencoded, report.min_psnr), (20, 20, 0, None));
}

#[test]
fn rendered_and_timeline_segments_are_refused_before_writing() {
    let f = fixture(40, 10, 0);
    let p0 = f.pts0();
    let base = plan(&f.index, f.bytes.len(), p0 + 5, p0 + 33, CutMode::FrameAccurate, Container::Mp4).plan;
    let refuse = |p: &cfa4_smart_edit::plan::ExportExecutionPlan| {
        let mut out = Cursor::new(Vec::new());
        let e = execute_mp4_boundary_reencode(p, &f.index, &f.source(), &encoding(), &mut out).unwrap_err();
        assert!(out.get_ref().is_empty(), "{e}: nothing is written");
        e
    };
    let mut render = base.clone();
    render.segments[0].mode = SegmentMode::Render;
    assert_eq!(refuse(&render), SmartCutError::RenderSegment { segment: 0 });
    let mut timeline = base.clone();
    timeline.segments[1].source = Some(SegmentSource { source_id: "a".into(), container: Container::Mp4, item: 0 });
    assert_eq!(refuse(&timeline), SmartCutError::TimelinePlan);
    let mut gap = base.clone();
    gap.segments.swap(0, 1);
    assert_eq!(refuse(&gap), SmartCutError::NotContiguous { segment: 1 });
    let mut mkv = base.clone();
    mkv.output_container = Container::Matroska;
    assert!(matches!(refuse(&mkv), SmartCutError::Unsupported { .. }));
}

#[test]
fn verification_catches_a_poor_or_damaged_re_encode() {
    let f = fixture(40, 10, 0);
    let p0 = f.pts0();
    let p = plan(&f.index, f.bytes.len(), p0 + 5, p0 + 33, CutMode::FrameAccurate, Container::Mp4).plan;
    let (out, _) = execute_mp4_boundary_reencode(&p, &f.index, &f.source(), &encoding(), Cursor::new(Vec::new())).unwrap();
    let out = out.into_inner();
    let strict = BoundaryEncoding { min_psnr: 99.5, ..encoding() };
    let e = verify_mp4_boundary_reencode(&p, &f.index, &f.source(), &strict, out.as_slice()).unwrap_err();
    assert!(matches!(&e, SmartCutError::Verification { detail } if detail.contains("dB")), "{e}");
    // the first copied sample (output sample 5) changed
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    let s = back.tracks[0].samples[5];
    let mut damaged = out.clone();
    let at = (s.offset + u64::from(s.size) - 1) as usize;
    damaged[at] ^= 0x55;
    let e = verify_mp4_boundary_reencode(&p, &f.index, &f.source(), &encoding(), damaged.as_slice()).unwrap_err();
    assert!(matches!(e, SmartCutError::Verification { .. }), "{e}");
}

/// FFmpeg as the oracle (SCE-001 §18): the output decodes without errors to 28 pictures, and the
/// copied pictures are bit-identical to the source pictures.
#[test]
fn ffmpeg_decodes_the_spliced_stream() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("Smart Render boundary decode") else { return };
    for bframes in [0, 2] {
        let f = fixture(40, 10, bframes);
        let (out, _, _) = run(&f, 5, 33, CutMode::FrameAccurate);
        let dir = std::env::temp_dir().join(format!("cfa4-boundary-{}-{bframes}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src_path, out_path) = (dir.join("source.mp4"), dir.join("cut.mp4"));
        std::fs::write(&src_path, &f.bytes).unwrap();
        std::fs::write(&out_path, &out).unwrap();
        let hashes = |path: &std::path::Path| -> Vec<String> {
            let o = Command::new(&ffmpeg).args(["-v", "error", "-xerror", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
            assert!(o.status.success() && o.stderr.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
        };
        let (src, cut) = (hashes(&src_path), hashes(&out_path));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(src.len(), 40);
        assert_eq!(cut.len(), 28, "bframes {bframes}");
        assert_eq!(cut[5..25], src[10..30], "bframes {bframes}: the copied pictures are the source pictures");
    }
}

/// The fixture remuxed with each sample changed by `edit(decode index, payload) -> (payload,
/// duration)`.
fn remux(f: &Fixture, edit: impl Fn(usize, Vec<u8>) -> (Vec<u8>, u32)) -> Fixture {
    remux_with(f, None, edit)
}

fn remux_with(f: &Fixture, matrix: Option<[i32; 9]>, edit: impl Fn(usize, Vec<u8>) -> (Vec<u8>, u32)) -> Fixture {
    use cfa4_isobmff::{Brand, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
    let track = &f.file.tracks[0];
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let mut cfg = TrackConfig::new(track.entries[0].clone(), track.timescale);
    cfg.matrix = matrix;
    let t = mux.add_track(cfg).unwrap();
    for (d, s) in track.samples.iter().enumerate() {
        let (data, duration) = edit(d, f.file.read_sample(f.bytes.as_slice(), 0, d).unwrap());
        mux.write_sample(t, WriteSample { data: &data, duration, composition_offset: (s.pts - s.dts) as i32, is_sync: s.is_sync }).unwrap();
    }
    let bytes = mux.finish().unwrap().into_inner();
    let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
    let index = crate::smart_cut_tests::index_of(&file, &bytes);
    Fixture { bytes, file, index }
}

/// What the executor would refuse is known before decoding (`check_mp4_boundary_reencode`), and
/// the executor refuses it before writing: captions in SEI user data, a variable frame rate.
#[test]
fn sources_a_boundary_re_encode_would_damage_are_refused_up_front() {
    use crate::boundary_reencode::check_mp4_boundary_reencode;
    let f = fixture(40, 10, 0);
    let p0 = f.pts0();
    let cut = |f: &Fixture| plan(&f.index, f.bytes.len(), p0 + 5, p0 + 33, CutMode::FrameAccurate, Container::Mp4).plan;
    assert_eq!(check_mp4_boundary_reencode(&cut(&f), &f.index, &f.source(), &encoding()), Ok(()));
    let refused = |f: &Fixture| {
        let p = cut(f);
        let e = check_mp4_boundary_reencode(&p, &f.index, &f.source(), &encoding()).unwrap_err();
        let mut out = Cursor::new(Vec::new());
        assert_eq!(execute_mp4_boundary_reencode(&p, &f.index, &f.source(), &encoding(), &mut out).unwrap_err(), e);
        assert!(out.get_ref().is_empty());
        e
    };
    // a CEA-708 style user_data_registered SEI (ITU-T T.35, country 0xB5) on picture 3
    let captions = remux(&f, |d, data| {
        let mut out = data;
        if d == 3 {
            let sei = [0x06, 0x04, 0x03, 0xB5, 0x00, 0x31, 0x80];
            let mut with = (sei.len() as u32).to_be_bytes().to_vec();
            with.extend_from_slice(&sei);
            with.extend_from_slice(&out);
            out = with;
        }
        (out, 1)
    });
    assert!(matches!(refused(&captions), SmartCutError::Incompatible { detail } if detail.contains("SEI")));
    // application metadata in an unregistered SEI (an unknown UUID with printable text)
    let metadata = remux(&f, |d, data| {
        let mut out = data;
        if d == 2 {
            let mut sei = vec![0x06, 0x05, 20];
            sei.extend_from_slice(&[0x11; 16]);
            sei.extend_from_slice(b"note");
            sei.push(0x80);
            let mut with = (sei.len() as u32).to_be_bytes().to_vec();
            with.extend_from_slice(&sei);
            with.extend_from_slice(&out);
            out = with;
        }
        (out, 1)
    });
    assert!(matches!(refused(&metadata), SmartCutError::Incompatible { detail } if detail.contains("unregistered")));
    // an alternative transfer characteristics SEI (type 147) the encoder cannot reproduce
    let transfer = remux(&f, |d, data| {
        let mut out = data;
        if d == 0 {
            let sei = [0x06, 0x93, 0x01, 18, 0x80];
            let mut with = (sei.len() as u32).to_be_bytes().to_vec();
            with.extend_from_slice(&sei);
            with.extend_from_slice(&out);
            out = with;
        }
        (out, 1)
    });
    assert!(matches!(refused(&transfer), SmartCutError::Incompatible { detail } if detail.contains("147")));
    // a parameter set repeated in-band, here inside a copied GOP
    let inband = remux(&f, |d, data| {
        let mut out = data;
        if d == 20 {
            let Some(cfa4_isobmff::CodecConfig::Avc(avc)) = f.file.tracks[0].codec() else { unreachable!() };
            let mut with = (avc.pps[0].len() as u32).to_be_bytes().to_vec();
            with.extend_from_slice(&avc.pps[0]);
            with.extend_from_slice(&out);
            out = with;
        }
        (out, 1)
    });
    assert!(matches!(refused(&inband), SmartCutError::Incompatible { detail } if detail.contains("in-band")));
    // picture 7 (inside the re-encoded head) is shown twice as long
    let vfr = remux(&f, |d, data| (data, if d == 7 { 2 } else { 1 }));
    assert!(matches!(refused(&vfr), SmartCutError::Incompatible { detail } if detail.contains("frame rate")));
    // a variable rate inside the copied GOPs is no obstacle
    let copied_vfr = remux(&f, |d, data| (data, if d == 20 { 2 } else { 1 }));
    assert_eq!(check_mp4_boundary_reencode(&cut(&copied_vfr), &copied_vfr.index, &copied_vfr.source(), &encoding()), Ok(()));
}

/// A rotated source (portrait phone footage) keeps its display matrix, copied or re-encoded.
#[test]
fn the_display_matrix_is_kept() {
    let rotated = [0, 0x10000, 0, -0x10000, 0, 0, 0, 0, 0x4000_0000];
    let f = remux_with(&fixture(40, 10, 0), Some(rotated), |_, data| (data, 1));
    assert_eq!(f.file.tracks[0].matrix, rotated);
    let (out, _, _) = run(&f, 5, 33, CutMode::FrameAccurate);
    assert_eq!(cfa4_isobmff::open(out.as_slice()).unwrap().tracks[0].matrix, rotated);
    let p = plan(&f.index, f.bytes.len(), f.pts0() + 10, f.pts0() + 30, CutMode::GopBoundary, Container::Mp4).plan;
    let (copy, _) = crate::smart_cut::execute_mp4_copy(&p, &f.index, &f.source(), Cursor::new(Vec::new())).unwrap();
    assert_eq!(cfa4_isobmff::open(copy.into_inner().as_slice()).unwrap().tracks[0].matrix, rotated);
}

#[test]
fn rbsp_bytes_match_the_unescaped_rbsp() {
    let cases: [&[u8]; 5] = [&[0, 0, 3, 1], &[0, 0, 3, 0, 0, 3], &[0, 0, 0, 3], &[1, 0, 0, 3, 3, 0, 0], &[0x80]];
    for nal in cases {
        let streamed: Vec<u8> = crate::boundary_reencode::rbsp_bytes(nal).collect();
        assert_eq!(streamed, cfa4_bitstream::unescape_rbsp(nal), "{nal:?}");
    }
}
