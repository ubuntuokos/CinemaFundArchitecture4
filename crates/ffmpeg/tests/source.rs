//! The FFmpeg media source and the engine fallback (ADR 0005 step 1), against the FFmpeg this
//! test binary links (`--features ffmpeg`). Fixtures are made by the ffmpeg CLI oracle
//! (`cfa4_testkit::oracle`, the pinned 9.0.2 in CI) into Cargo's temporary directory, and every
//! decoded frame is compared byte for byte with the oracle's own decoding of the same frame. A
//! machine without the oracle, or whose oracle lacks an encoder a fixture needs, skips with a
//! message (`CFA4_REQUIRE_ORACLES=1` turns skips into failures).

#![cfg(feature = "ffmpeg")]

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_frame::{Chroma, PixelData};
use cfa4_media::{FrameRequest, MediaKind, SharedSource};
use cfa4_time::{TICKS_PER_SECOND, Tick};

const FPS: i64 = 25;

fn usable_ffmpeg() -> bool {
    match cfa4_ffmpeg::probe() {
        Ok(s) => match s.refusal() {
            None => true,
            Some(r) => {
                cfa4_testkit::oracle::skip("cfa4-ffmpeg", &r);
                false
            }
        },
        Err(e) => {
            cfa4_testkit::oracle::skip("cfa4-ffmpeg", &e.to_string());
            false
        }
    }
}

/// A fixture made by the oracle, or `None` after a skip. `args` go between the inputs and OUT.
fn fixture(name: &str, size: &str, args: &[&str]) -> Option<PathBuf> {
    if !usable_ffmpeg() {
        return None;
    }
    let ff = cfa4_testkit::oracle::ffmpeg_or_skip("cfa4-ffmpeg fixture")?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cfa4-ffmpeg-source");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(name);
    if path.exists() {
        return Some(path);
    }
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let tmp = dir.join(format!("{name}.{}.{:?}.part.{ext}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
    let source = format!("testsrc2=size={size}:rate={FPS}:duration=2");
    let out = Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &source,
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2:sample_rate=48000",
        ])
        .args(args)
        .arg(&tmp)
        .output()
        .ok()?;
    if !out.status.success() || !tmp.exists() {
        cfa4_testkit::oracle::skip("cfa4-ffmpeg fixture", &format!("{name}: {}", String::from_utf8_lossy(&out.stderr).trim()));
        return None;
    }
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// The oracle's decoding of frame `n` as raw planar bytes in `pix_fmt`.
fn oracle_frame(path: &Path, n: i64, pix_fmt: &str) -> Vec<u8> {
    let ff = cfa4_testkit::oracle::ffmpeg().unwrap_or_else(|| panic!("ffmpeg vanished"));
    let out = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-vf", &format!("select=eq(n\\,{n})"), "-frames:v", "1", "-pix_fmt", pix_fmt, "-f", "rawvideo", "-"])
        .output()
        .unwrap_or_else(|e| panic!("ffmpeg: {e}"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn bytes_of(data: &PixelData) -> Vec<u8> {
    match data {
        PixelData::Yuv8 { planes, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(),
        PixelData::Yuv16 { planes, .. } => planes.iter().flat_map(|p| p.iter().flat_map(|s| s.to_le_bytes())).collect(),
        PixelData::Rgba8(d) => d.to_vec(),
        PixelData::RgbaF32(_) => Vec::new(),
    }
}

fn open(path: &Path) -> SharedSource {
    cfa4_ffmpeg::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn frame_time(n: i64) -> Tick {
    // the middle of frame n, so rounding never lands on a neighbour
    Tick(n * TICKS_PER_SECOND / FPS + TICKS_PER_SECOND / FPS / 2)
}

/// Frames in random order (seeks backwards, forwards and sequential steps) equal the oracle's.
fn check_bit_exact(path: &Path, pix_fmt: &str, label: &str) {
    let src = open(path);
    let v = src.info().video.clone().unwrap_or_else(|| panic!("no video"));
    assert!(v.pixel_format.starts_with(label), "{} reported as {}", path.display(), v.pixel_format);
    for n in [37, 3, 4, 5, 49, 0, 20, 21, 12] {
        let f = src.video_frame(FrameRequest::full(frame_time(n))).unwrap_or_else(|e| panic!("frame {n}: {e}"));
        let ours = bytes_of(&f.data);
        let theirs = oracle_frame(path, n, pix_fmt);
        assert_eq!(ours.len(), theirs.len(), "frame {n}: size");
        assert!(ours == theirs, "{}: frame {n} differs from the oracle's decoding", path.display());
    }
}

#[test]
fn h264_high_422_10_bit_is_native_and_bit_exact() {
    // G3: camera 10-bit 4:2:2 H.264, which the engine's own decoder does not read without hardware
    let Some(p) = fixture("h264_422_10.mp4", "640x360", &["-c:v", "libx264", "-pix_fmt", "yuv422p10le", "-c:a", "aac"]) else { return };
    let src = open(&p);
    let f = src.video_frame(FrameRequest::full(Tick::ZERO)).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(f.data, PixelData::Yuv16 { chroma: Chroma::C422, bits: 10, .. }), "{}", f.format_label());
    check_bit_exact(&p, "yuv422p10le", "YUV 4:2:2 10-bit");
}

#[test]
fn hevc_main_422_10_is_native_and_bit_exact() {
    let Some(p) =
        fixture("hevc_422_10.mov", "640x360", &["-c:v", "libx265", "-pix_fmt", "yuv422p10le", "-x265-params", "log-level=error", "-c:a", "pcm_s16le"])
    else {
        return;
    };
    check_bit_exact(&p, "yuv422p10le", "YUV 4:2:2 10-bit");
}

#[test]
fn dv_pal_avi_mpeg4_and_wmv_open_with_sound() {
    // G10: containers and codecs the engine does not read yet
    let cases: [(&str, &str, &[&str], &str); 4] = [
        ("dv_pal.dv", "720x576", &["-ac", "2", "-c:v", "dvvideo", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"], "yuv420p"),
        ("mpeg4_part2.avi", "640x360", &["-c:v", "mpeg4", "-c:a", "libmp3lame"], "yuv420p"),
        ("wmv2.wmv", "640x360", &["-c:v", "wmv2", "-c:a", "wmav2"], "yuv420p"),
        ("dvcprohd.mov", "960x720", &["-r", "50", "-c:v", "dvvideo", "-pix_fmt", "yuv422p", "-an"], "yuv422p"),
    ];
    for (name, size, args, pix) in cases {
        let Some(p) = fixture(name, size, args) else { continue };
        let src = open(&p);
        let info = src.info();
        assert_eq!(info.kind, MediaKind::Movie, "{name}");
        let secs = info.duration.0 as f64 / TICKS_PER_SECOND as f64;
        assert!((secs - 2.0).abs() < 0.2, "{name}: duration {secs}");
        if info.has_audio() {
            let a = src.audio(24_000, 4_800, 48_000).unwrap_or_else(|e| panic!("{name} audio: {e}"));
            assert_eq!(a.frames(), 4_800);
            let peak = a.channels.iter().flatten().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05, "{name}: the 440 Hz tone is silent ({peak})");
        }
        if name != "dvcprohd.mov" {
            // 50 fps: frame_time() assumes 25
            let f = src.video_frame(FrameRequest::full(frame_time(10))).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(bytes_of(&f.data), oracle_frame(&p, 10, pix), "{name}: frame 10");
        }
    }
}

#[test]
fn audio_is_continuous_across_sequential_reads_and_seeks() {
    let Some(p) = fixture("tone.avi", "320x240", &["-c:v", "mpeg4", "-c:a", "pcm_s16le"]) else { return };
    let src = open(&p);
    let whole = src.audio(0, 48_000, 48_000).unwrap_or_else(|e| panic!("{e}"));
    // the same second in four sequential pieces equals one read
    let mut pieces: Vec<f32> = Vec::new();
    for k in 0..4 {
        let a = src.audio(k * 12_000, 12_000, 48_000).unwrap_or_else(|e| panic!("{e}"));
        pieces.extend_from_slice(&a.channels[0]);
    }
    assert_eq!(pieces.len(), whole.channels[0].len());
    let diff = pieces.iter().zip(&whole.channels[0]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff < 1e-4, "sequential reads differ by {diff}");
    // a seek back lands on the same samples (within a resampler's start-up)
    let again = src.audio(24_000, 1_000, 48_000).unwrap_or_else(|e| panic!("{e}"));
    let diff = again.channels[0].iter().zip(&whole.channels[0][24_000..25_000]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff < 0.05, "seeking back changed the samples by {diff}");
    // another sample rate is resampled by FFmpeg
    let a44 = src.audio(0, 4_410, 44_100).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((a44.sample_rate, a44.frames()), (44_100, 4_410));
}

#[test]
fn damaged_and_hostile_inputs_are_errors_not_crashes() {
    let Some(good) = fixture("damage_src.mp4", "320x240", &["-c:v", "mpeg4", "-c:a", "aac"]) else { return };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cfa4-ffmpeg-damaged");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let bytes = std::fs::read(&good).unwrap_or_else(|e| panic!("{e}"));
    let truncated = dir.join("truncated.mp4");
    std::fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap_or_else(|e| panic!("{e}"));
    let mut noisy = bytes.clone();
    let mut x: u32 = 7;
    for _ in 0..400 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let i = 2_000 + (x as usize % (noisy.len() - 2_000));
        noisy[i] = (x >> 16) as u8;
    }
    let noisy_path = dir.join("noisy.mp4");
    std::fs::write(&noisy_path, &noisy).unwrap_or_else(|e| panic!("{e}"));
    let garbage = dir.join("garbage.avi");
    std::fs::write(&garbage, (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<_>>()).unwrap_or_else(|e| panic!("{e}"));
    let empty = dir.join("empty.mov");
    std::fs::write(&empty, b"").unwrap_or_else(|e| panic!("{e}"));
    for p in [&truncated, &noisy_path, &garbage, &empty] {
        // opening may fail or succeed; every frame request after that is a value, never a panic
        if let Ok(src) = cfa4_ffmpeg::open(p) {
            for n in [0, 30, 10, 49] {
                let _ = src.video_frame(FrameRequest::full(frame_time(n)));
            }
            let _ = src.audio(0, 4_800, 48_000);
        }
    }
    assert!(cfa4_ffmpeg::open(&garbage).is_err());
    assert!(cfa4_ffmpeg::open(&empty).is_err());
    assert!(cfa4_ffmpeg::open(Path::new("/nonexistent/cfa4.mp4")).is_err());
}

#[test]
fn register_makes_ffmpeg_the_first_fallback_and_respects_the_switch() {
    let Some(p) = fixture("fallback.wmv", "320x240", &["-c:v", "wmv2", "-c:a", "wmav2"]) else { return };
    let status = cfa4_ffmpeg::register().unwrap_or_else(|e| panic!("{e}"));
    assert!(status.refusal().is_none());
    assert!(cfa4_media::fallback::fallbacks().iter().any(|f| f.name == "FFmpeg"));
    assert!(cfa4_media::is_importable(&p), ".wmv becomes importable");
    let native = Err(cfa4_media::MediaError::Unsupported("built-in openers do not read WMV".into()));
    let src = cfa4_media::open_with_fallback(&p, native).unwrap_or_else(|e| panic!("{e}"));
    assert!(src.info().container.contains("FFmpeg"), "{}", src.info().container);
    // switched off in Settings ▸ Media Backends: the built-in result comes back unchanged
    cfa4_ffmpeg::set_enabled(false);
    let native = Err(cfa4_media::MediaError::Unsupported("built-in openers do not read WMV".into()));
    assert!(cfa4_media::open_with_fallback(&p, native).is_err());
    cfa4_ffmpeg::set_enabled(true);
    // the report the settings panel shows
    cfa4_ffmpeg::register_status();
    let r = cfa4_media::backend_reports().into_iter().find(|r| r.id == "ffmpeg").unwrap_or_default();
    assert!(r.available && r.compiled_in && r.enabled, "{r:?}");
    assert!(r.license.as_deref().unwrap_or("").contains("GPL version 3"));
}

#[test]
fn the_base_build_has_no_restricted_encoders_or_says_so() {
    let Ok(status) = cfa4_ffmpeg::probe() else { return };
    // the Ubuntu model: whatever this FFmpeg has, the report names both components
    let names: Vec<_> = status.restricted.iter().map(|r| r.encoder.as_str()).collect();
    assert_eq!(names, ["libx264", "libx265"]);
    assert!(!status.nonfree);
}
