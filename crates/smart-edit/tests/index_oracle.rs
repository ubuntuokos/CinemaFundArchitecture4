//! WP-02 / WP-07 gate: the GOP index of ffmpeg-made MPEG-2, H.264 and HEVC reference streams (MPEG-TS, open and
//! closed GOPs, B pictures) and AV1 streams (MP4, hidden alt-ref frames) against ffprobe. ffmpeg/ffprobe are external fixture generators and
//! oracles only (never linked); the tests skip without them (`CFA4_REQUIRE_ORACLES=1` fails).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_smart_edit::av1_index::index_av1;
use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::hevc_index::index_hevc;
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
        cfa4_mpegts::Codec::Hevc => index_hevc(0, Rational::MPEG_90K, NalFormat::AnnexB, &[], packets).unwrap(),
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
    if !cfa4_testkit::encoders_or_skip("index_oracle::h264_closed_gop_reference", &ff, &["libx264"]) {
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
    if !cfa4_testkit::encoders_or_skip("index_oracle::h264_open_gop_reference", &ff, &["libx264"]) {
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

#[test]
fn hevc_closed_gop_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    if !cfa4_testkit::encoders_or_skip("index_oracle::hevc_closed_gop_reference", &ff, &["libx265"]) {
        return;
    }
    let args = ["-c:v", "libx265", "-pix_fmt", "yuv420p", "-x265-params", "keyint=25:min-keyint=25:scenecut=0:open-gop=0:bframes=3:log-level=error"];
    let Some(file) = fixture(&ff, "hevc_closed_gop.ts", &args) else { panic!("ffmpeg could not make the fixture") };
    let ix = index_ts(&file);
    check_against_ffprobe(&fp, &file, &ix);
    assert_eq!(ix.clean_access_points().len(), 4, "{:?}", ix.gops);
    assert!(ix.gops.iter().all(|g| g.kind.is_clean()), "{:?}", ix.gops);
    assert!(ix.frames.iter().all(|f| matches!(f.dependency_status, DependencyStatus::Independent | DependencyStatus::WithinGop)));
    assert!(ix.frames.iter().any(|f| f.picture_type == PictureType::B));
}

#[test]
fn hevc_open_gop_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    if !cfa4_testkit::encoders_or_skip("index_oracle::hevc_open_gop_reference", &ff, &["libx265"]) {
        return;
    }
    let args = ["-c:v", "libx265", "-pix_fmt", "yuv420p", "-x265-params", "keyint=25:min-keyint=25:scenecut=0:open-gop=1:bframes=3:log-level=error"];
    let Some(file) = fixture(&ff, "hevc_open_gop.ts", &args) else { panic!("ffmpeg could not make the fixture") };
    let ix = index_ts(&file);
    check_against_ffprobe(&fp, &file, &ix);
    // x265 open GOP: an IDR first, then a CRA every 25 pictures; a CRA whose RASL leading pictures
    // need the GOP before is no clean start (the last one, flushed without leading pictures, is)
    assert_eq!(ix.gops.len(), 4, "{:?}", ix.gops);
    assert_eq!(ix.gops[0].kind, RandomAccessKind::Idr);
    assert!(ix.gops[1..].iter().all(|g| matches!(g.kind, RandomAccessKind::Cra { .. })), "{:?}", ix.gops);
    let rasl: Vec<&cfa4_smart_edit::media_index::GopInfo> = ix.gops.iter().filter(|g| g.kind == RandomAccessKind::Cra { rasl: true }).collect();
    assert!(rasl.len() >= 2, "{:?}", ix.gops);
    for g in &rasl {
        let gop = &ix.frames[g.start as usize..(g.start + g.count) as usize];
        assert!(gop.iter().any(|f| f.dependency_status == DependencyStatus::CrossesAccessPoint), "{g:?}");
    }
    let clean: Vec<u64> = ix.gops.iter().filter(|g| g.kind.is_clean()).map(|g| g.start).collect();
    assert_eq!(ix.clean_access_points(), clean);
    assert_eq!(clean.len(), ix.gops.len() - rasl.len());
}

/// An AV1 MP4 made by ffmpeg with `codec_args`, and its index.
fn av1_mp4(ff: &Path, name: &str, codec_args: &[&str]) -> Option<(PathBuf, StreamIndex)> {
    let out = cfa4_testkit::fixtures_dir("smart-edit").join(name);
    let file = cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff).args(["-y", "-v", "error"]).args(SOURCE).args(codec_args).args(["-an", "-f", "mp4"]).arg(tmp).stdin(Stdio::null()).status();
        matches!(st, Ok(s) if s.success())
    })?;
    let data = std::fs::read(&file).unwrap();
    let mp4 = cfa4_isobmff::open(data.as_slice()).unwrap();
    let t = &mp4.tracks[0];
    let Some(cfa4_isobmff::CodecConfig::Av1(av1c)) = t.codec() else { panic!("not AV1") };
    let units: Vec<Vec<u8>> = (0..t.samples.len()).map(|i| mp4.read_sample(data.as_slice(), 0, i).unwrap()).collect();
    let packets = t.samples.iter().zip(&units).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: Some(s.dts), keyframe: s.is_sync });
    let ix = index_av1(t.id, Rational::new(1, i64::from(t.timescale)), &av1c.config_obus, packets).unwrap();
    Some((file, ix))
}

/// ffprobe's shown frames: (pts, key frame), in presentation order, and what ffprobe said on stderr
/// (decode errors). Every row must parse: a frame without a timestamp fails here, it is not dropped.
fn probe_keys(fp: &Path, file: &Path) -> (Vec<(i64, bool)>, String) {
    let o = Command::new(fp)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "frame=pts,key_frame", "-of", "compact=p=0"])
        .arg(file)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(o.status.success(), "ffprobe failed on {}: {stderr}", file.display());
    let mut rows: Vec<(i64, bool)> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let field = |name: &str| l.split('|').find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=')?.trim().parse::<i64>().ok());
            let (Some(pts), Some(key)) = (field("pts"), field("key_frame")) else { panic!("unexpected ffprobe row {l:?} for {}", file.display()) };
            (pts, key == 1)
        })
        .collect();
    rows.sort();
    (rows, stderr)
}

fn check_av1(fp: &Path, file: &Path, ix: &StreamIndex) {
    let (shown, stderr) = probe_keys(fp, file);
    assert_eq!(ix.frames.len(), shown.len(), "one temporal unit per shown frame: {} (ffprobe stderr: {stderr:?})", file.display());
    // positions in presentation order, so the two timelines need not share an origin
    let keys: Vec<usize> = shown.iter().enumerate().filter(|(_, (_, k))| *k).map(|(i, _)| i).collect();
    let ours: Vec<usize> = ix
        .frames
        .iter()
        .filter(|f| f.random_access == RandomAccessKind::KeyFrame)
        .filter_map(|f| f.presentation_index.and_then(|p| usize::try_from(p).ok()))
        .collect();
    assert_eq!(ours, keys, "{}", file.display());
    assert!(!ours.is_empty() && ours[0] == 0);
    assert!(ix.frames.iter().all(|f| matches!(f.dependency_status, DependencyStatus::Independent | DependencyStatus::WithinGop)), "{:?}", ix.frames);
    assert!(ix.timestamp_problems.is_empty(), "{:?}", ix.timestamp_problems);
    // every clean start is a sync sample of the container
    assert!(ix.frames.iter().filter(|f| f.random_access == RandomAccessKind::KeyFrame).all(|f| f.keyframe));
}

#[test]
fn av1_libaom_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    if !has_encoder(&ff, "libaom-av1") {
        cfa4_testkit::skip("index_oracle::av1_libaom_reference", "ffmpeg has no libaom-av1");
        return;
    }
    // alt-ref (hidden) frames, a key frame every 25 pictures
    let args = ["-c:v", "libaom-av1", "-cpu-used", "8", "-g", "25", "-keyint_min", "25", "-lag-in-frames", "16", "-pix_fmt", "yuv420p"];
    let Some((file, ix)) = av1_mp4(&ff, "av1_libaom.mp4", &args) else { panic!("ffmpeg could not make the fixture") };
    check_av1(&fp, &file, &ix);
    assert_eq!(ix.clean_access_points().len(), 4, "{:?}", ix.gops);
    // alt-ref frames are decoded hidden and shown later: some units show an existing frame
    assert!(ix.frames.iter().any(|f| !f.reference), "no show_existing_frame unit: {:?}", ix.frames);
}

#[test]
fn av1_svt_reference() {
    let (ff, fp) = cfa4_testkit::require_oracles!();
    if !has_encoder(&ff, "libsvtav1") {
        cfa4_testkit::skip("index_oracle::av1_svt_reference", "ffmpeg has no libsvtav1");
        return;
    }
    // `lp=1`: one encoder thread. Multi-threaded SVT-AV1 (2.3.0 in CI) does not always write the same
    // stream; one CI run got 100 temporal units of which libdav1d showed 99. ffprobe decodes with one
    // thread already, so the encoder is the varying side; a single-threaded encode is reproducible.
    let args = ["-c:v", "libsvtav1", "-preset", "12", "-g", "25", "-svtav1-params", "scd=0:irefresh-type=2:lp=1", "-pix_fmt", "yuv420p"];
    let Some((file, ix)) = av1_mp4(&ff, "av1_svt_lp1.mp4", &args) else { panic!("ffmpeg could not make the fixture") };
    check_av1(&fp, &file, &ix);
    assert!(ix.clean_access_points().len() >= 4, "{:?}", ix.gops);
}
