//! WP-02 gate: the GOP index of ffmpeg-made MPEG-2 and H.264 reference streams (MPEG-TS, open and
//! closed GOPs, B pictures) against ffprobe. ffmpeg/ffprobe are external fixture generators and
//! oracles only (never linked); the tests skip without them (`CFA4_REQUIRE_ORACLES=1` fails).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::media_index::{DependencyStatus, Packet, PictureType, RandomAccessKind, StreamIndex};
use cfa4_smart_edit::mpeg2_index::index_mpeg2;
use cfa4_smart_edit::timestamp::Rational;

const SOURCE: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25", "-t", "4"];

fn fixture(ff: &Path, name: &str, codec_args: &[&str]) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("smart-edit").join(name);
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st =
            Command::new(ff).args(["-y", "-v", "error"]).args(SOURCE).args(codec_args).args(["-an", "-f", "mpegts"]).arg(tmp).stdin(Stdio::null()).status();
        matches!(st, Ok(s) if s.success())
    })
}

fn has_encoder(ff: &Path, name: &str) -> bool {
    Command::new(ff)
        .args(["-hide_banner", "-encoders"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some(name)))
}

/// ffprobe's frames: (pts, picture type) in presentation order.
fn probe_frames(fp: &Path, file: &Path) -> Vec<(i64, char)> {
    let o =
        Command::new(fp).args(["-v", "error", "-select_streams", "v:0", "-show_entries", "frame=pts,pict_type", "-of", "csv=p=0"]).arg(file).output().unwrap();
    assert!(o.status.success());
    let mut v: Vec<(i64, char)> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split(',');
            let pts = it.next()?.trim().parse().ok()?;
            let t = it.next()?.trim().chars().next()?;
            Some((pts, t))
        })
        .collect();
    v.sort();
    v
}

fn index_ts(file: &Path) -> StreamIndex {
    let data = std::fs::read(file).unwrap();
    let ts = cfa4_mpegts::open(&data).unwrap();
    let s = ts.find(cfa4_mpegts::Kind::Video, |_| true).unwrap();
    let stream = &ts.streams[s];
    let units: Vec<Vec<u8>> = (0..stream.units.len()).map(|i| ts.read_unit(&data, s, i).unwrap()).collect();
    let packets = units.iter().zip(&stream.units).map(|(d, u)| Packet { data: d, pts: u.pts, dts: u.dts, keyframe: u.key });
    match stream.codec {
        cfa4_mpegts::Codec::Mpeg2Video => index_mpeg2(0, Rational::MPEG_90K, packets).unwrap(),
        cfa4_mpegts::Codec::H264 => index_h264(0, Rational::MPEG_90K, NalFormat::AnnexB, &[], packets).unwrap(),
        ref c => panic!("unexpected codec {c:?}"),
    }
}

/// Pictures the container flagged as keyframes; every one must start a GOP of the index.
fn keyframes(ix: &StreamIndex) -> usize {
    let starts: Vec<u64> = ix.gops.iter().map(|g| g.start).collect();
    let keys: Vec<u64> = ix.frames.iter().filter(|f| f.keyframe).map(|f| f.decode_index).collect();
    assert_eq!(starts, keys);
    keys.len()
}

/// Picture types and presentation order agree with ffprobe, picture for picture.
fn check_against_ffprobe(fp: &Path, file: &Path, ix: &StreamIndex) {
    let want = probe_frames(fp, file);
    assert_eq!(ix.frames.len(), want.len(), "{}", file.display());
    let mut ours: Vec<(i64, char)> = ix
        .frames
        .iter()
        .map(|f| {
            let t = match f.picture_type {
                PictureType::I | PictureType::D => 'I',
                PictureType::P => 'P',
                PictureType::B => 'B',
                PictureType::Unknown => '?',
            };
            (f.pts.unwrap(), t)
        })
        .collect();
    ours.sort();
    // mpegts unwraps timestamps relative to its own origin: compare offsets from the first picture
    let base_o = ours[0].0;
    let base_w = want[0].0;
    for ((po, to), (pw, tw)) in ours.iter().zip(&want) {
        assert_eq!((po - base_o, *to), (pw - base_w, *tw), "{}", file.display());
    }
    for (k, (pts, _)) in ours.iter().enumerate() {
        let f = ix.frames.iter().find(|f| f.pts == Some(*pts)).unwrap();
        assert_eq!(f.presentation_index, Some(k as u64));
    }
    assert!(ix.timestamp_problems.is_empty(), "{:?}", ix.timestamp_problems);
}

#[test]
fn mpeg2_open_gop_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    let Some(file) = fixture(&ff, "mpeg2_open_gop.ts", &["-c:v", "mpeg2video", "-bf", "2", "-g", "12", "-q:v", "4"]) else {
        panic!("ffmpeg could not make the fixture")
    };
    let ix = index_ts(&file);
    check_against_ffprobe(&fp, &file, &ix);
    // one GOP per keyframe ffmpeg wrote (every 12 pictures over 4 s)
    assert_eq!(ix.gops.len(), keyframes(&ix), "{:?}", ix.gops);
    assert!(ix.gops.len() >= 8);
    assert!(ix.gops.iter().all(|g| matches!(g.kind, RandomAccessKind::OpenGop { broken_link: false } | RandomAccessKind::ClosedGop)));
    // the first GOP's leading pictures have nothing to refer to, later ones refer across the cut
    let crossing = ix.frames.iter().filter(|f| f.dependency_status == DependencyStatus::CrossesAccessPoint).count();
    assert!(crossing > 0);
    for g in &ix.gops[1..] {
        if let RandomAccessKind::OpenGop { .. } = g.kind {
            let lead = &ix.frames[g.start as usize + 1];
            assert_eq!(lead.picture_type, PictureType::B);
            assert_eq!(lead.dependency_status, DependencyStatus::CrossesAccessPoint);
        }
    }
    assert!(ix.frames.iter().all(|f| f.dependency_status != DependencyStatus::Unknown));
}

#[test]
fn mpeg2_closed_gop_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    let Some(file) =
        fixture(&ff, "mpeg2_closed_gop.ts", &["-c:v", "mpeg2video", "-bf", "2", "-g", "12", "-q:v", "4", "-flags", "+cgop", "-sc_threshold", "1000000000"])
    else {
        panic!("ffmpeg could not make the fixture")
    };
    let ix = index_ts(&file);
    check_against_ffprobe(&fp, &file, &ix);
    assert_eq!(ix.gops.len(), keyframes(&ix));
    assert!(ix.gops.len() >= 8);
    assert!(ix.gops.iter().all(|g| g.kind == RandomAccessKind::ClosedGop), "{:?}", ix.gops);
    assert_eq!(ix.clean_access_points(), ix.gops.iter().map(|g| g.start).collect::<Vec<_>>());
    assert!(ix.frames.iter().all(|f| !matches!(f.dependency_status, DependencyStatus::CrossesAccessPoint | DependencyStatus::Undecodable)));
}

#[test]
fn h264_closed_gop_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    if !has_encoder(&ff, "libx264") {
        cfa4_testkit::skip("index_oracle::h264_closed_gop_reference", "ffmpeg has no libx264");
        return;
    }
    let Some(file) =
        fixture(&ff, "h264_closed_gop.ts", &["-c:v", "libx264", "-bf", "2", "-g", "25", "-keyint_min", "25", "-sc_threshold", "0", "-pix_fmt", "yuv420p"])
    else {
        panic!("ffmpeg could not make the fixture")
    };
    let ix = index_ts(&file);
    check_against_ffprobe(&fp, &file, &ix);
    assert_eq!(ix.clean_access_points().len(), 4, "{:?}", ix.gops);
    assert!(ix.gops.iter().all(|g| g.kind == RandomAccessKind::Idr));
    assert!(ix.frames.iter().all(|f| matches!(f.dependency_status, DependencyStatus::Independent | DependencyStatus::WithinGop)));
    assert!(ix.frames.iter().any(|f| f.picture_type == PictureType::B));
}

#[test]
fn h264_open_gop_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    if !has_encoder(&ff, "libx264") {
        cfa4_testkit::skip("index_oracle::h264_open_gop_reference", "ffmpeg has no libx264");
        return;
    }
    let args = ["-c:v", "libx264", "-bf", "2", "-pix_fmt", "yuv420p", "-x264-params", "keyint=25:min-keyint=25:scenecut=0:open-gop=1"];
    let Some(file) = fixture(&ff, "h264_open_gop.ts", &args) else { panic!("ffmpeg could not make the fixture") };
    let ix = index_ts(&file);
    check_against_ffprobe(&fp, &file, &ix);
    // x264 open GOP: one IDR, then I pictures with recovery point SEI and leading B pictures
    assert_eq!(ix.clean_access_points(), vec![0]);
    let rp = ix.frames.iter().filter(|f| matches!(f.random_access, RandomAccessKind::RecoveryPoint { exact_match: true, .. })).count();
    assert_eq!(rp, 3, "{:?}", ix.gops);
    assert!(ix.frames.iter().any(|f| f.dependency_status == DependencyStatus::CrossesAccessPoint));
}
