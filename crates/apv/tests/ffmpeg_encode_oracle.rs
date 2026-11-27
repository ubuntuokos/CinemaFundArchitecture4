//! FFmpeg's own APV encoder (`apv_vulkan`) as a second, independent APV source
//! (CFA4-FFMPEG-9.1-001 F2b). The other oracle direction (`oracle.rs`) checks that FFmpeg reads
//! what our encoder writes; this one checks that our decoder reads what FFmpeg writes, bit-exact
//! with FFmpeg's own decode of the same file.
//!
//! `apv_vulkan` needs a Vulkan device. CI uses Mesa lavapipe (software Vulkan);
//! `CFA4_APV_VULKAN_DEVICE` picks another device (index or name, FFmpeg `-init_hw_device` syntax).
//! Without the pinned FFmpeg (`CFA4_FFMPEG_MIN_VERSION` in `packaging/ffmpeg-version.env`),
//! the encoder or a Vulkan device the test skips (a failure under
//! `CFA4_REQUIRE_ORACLES=1`).

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_apv::{ChromaFormat, Frame, Profile, decode_frame, probe};

const CONTEXT: &str = "apv ffmpeg-encode oracle";

fn vulkan_device() -> String {
    std::env::var("CFA4_APV_VULKAN_DEVICE").ok().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| "0".into())
}

/// `CFA4_FFMPEG_MIN_VERSION` from `packaging/ffmpeg-version.env`, the single version source.
fn min_ffmpeg_version() -> String {
    let env =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/ffmpeg-version.env")).expect("read packaging/ffmpeg-version.env");
    env.lines()
        .find_map(|l| l.trim().strip_prefix("CFA4_FFMPEG_MIN_VERSION="))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("CFA4_FFMPEG_MIN_VERSION in packaging/ffmpeg-version.env")
}

/// The pinned minimum FFmpeg or newer with `apv_vulkan` and a working Vulkan device, else `None` (skip).
fn ffmpeg() -> Option<PathBuf> {
    let ff = cfa4_testkit::ffmpeg_with_format_or_skip(CONTEXT, "apv", &min_ffmpeg_version())?;
    if !cfa4_testkit::encoders_or_skip(CONTEXT, &ff, &["apv_vulkan"]) {
        return None;
    }
    let out = Command::new(&ff)
        .args(["-hide_banner", "-v", "error", "-init_hw_device"])
        .arg(format!("vulkan=vk:{}", vulkan_device()))
        .args(["-f", "lavfi", "-i", "nullsrc=s=64x64:d=0.04", "-frames:v", "1", "-f", "null", "-"])
        .output()
        .expect("run ffmpeg");
    if !out.status.success() {
        let reason = format!("no usable Vulkan device {} for apv_vulkan: {}", vulkan_device(), String::from_utf8_lossy(&out.stderr).trim());
        cfa4_testkit::skip(CONTEXT, &reason);
        return None;
    }
    Some(ff)
}

fn pix_fmt(profile: Profile) -> &'static str {
    match profile {
        Profile::P422_10 => "yuv422p10le",
        Profile::P422_12 => "yuv422p12le",
        Profile::P444_10 => "yuv444p10le",
        Profile::P444_12 => "yuv444p12le",
        Profile::P4444_10 => "yuva444p10le",
        Profile::P4444_12 => "yuva444p12le",
        Profile::P400_10 => "gray10le",
    }
}

/// Smooth gradients plus a checker pattern, so the DCT, quantizer and entropy coder all work.
fn source_frame(profile: Profile, w: u32, h: u32) -> Frame {
    let chroma = profile.default_chroma();
    let depth = profile.default_bit_depth();
    let mut f = Frame::new(w, h, chroma, depth, chroma == ChromaFormat::Yuv4444);
    let lo = 16u32 << (depth - 8);
    let span = (235u32 << (depth - 8)) - lo;
    for y in 0..h {
        for x in 0..w {
            let checker = if ((x / 4) ^ (y / 6)) & 1 == 0 { 32 << (depth - 10) } else { 0 };
            f.y[(y * w + x) as usize] = (lo + ((x * 5 + y * 9) % span + checker) % span) as u16;
        }
    }
    let (cw, ch) = (f.chroma_width(), f.chroma_height());
    for y in 0..ch {
        for x in 0..cw {
            f.cb[(y * cw + x) as usize] = (lo + (x * 7 + y * 3 + 90) % span) as u16;
            f.cr[(y * cw + x) as usize] = (lo + (x * 2 + y * 11 + 250) % span) as u16;
        }
    }
    if let Some(a) = f.alpha.as_mut() {
        for (i, s) in a.iter_mut().enumerate() {
            *s = (lo + (i as u32 * 13) % span) as u16;
        }
    }
    f
}

/// Planar samples in FFmpeg's plane order (Y, U, V, A).
fn planes(f: &Frame) -> Vec<u16> {
    let mut v = Vec::new();
    v.extend_from_slice(&f.y);
    v.extend_from_slice(&f.cb);
    v.extend_from_slice(&f.cr);
    if let Some(a) = &f.alpha {
        v.extend_from_slice(a);
    }
    v
}

fn ffmpeg_encode(ff: &Path, src: &Frame, profile: Profile, raw: &Path, apv: &Path) {
    let bytes: Vec<u8> = planes(src).iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(raw, bytes).expect("write raw source");
    let out = Command::new(ff)
        .args(["-hide_banner", "-v", "error", "-xerror", "-y", "-init_hw_device"])
        .arg(format!("vulkan=vk:{}", vulkan_device()))
        .args(["-filter_hw_device", "vk", "-f", "rawvideo", "-pix_fmt", pix_fmt(profile), "-s"])
        .arg(format!("{}x{}", src.width, src.height))
        .arg("-i")
        .arg(raw)
        .args(["-vf", &format!("format={},hwupload", pix_fmt(profile)), "-c:v", "apv_vulkan", "-qp", "12", "-frames:v", "1", "-f", "apv"])
        .arg(apv)
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "{profile:?}: apv_vulkan encode failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn ffmpeg_decode(ff: &Path, apv: &Path, profile: Profile) -> Vec<u16> {
    let out = Command::new(ff)
        .args(["-hide_banner", "-v", "error", "-xerror", "-f", "apv", "-i"])
        .arg(apv)
        .args(["-f", "rawvideo", "-pix_fmt", pix_fmt(profile), "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "{profile:?}: ffmpeg APV decode failed: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

fn psnr(a: &[u16], b: &[u16], peak: f64) -> f64 {
    let mse = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum::<f64>() / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (peak * peak / mse).log10() }
}

#[test]
fn ffmpeg_apv_vulkan_streams_decode_bit_exact() {
    let Some(ff) = ffmpeg() else { return };
    let dir = cfa4_testkit::fixtures_dir("apv");

    let cases = Profile::ALL.into_iter().map(|p| (p, 320u32, 192u32)).chain([(Profile::P422_10, 640, 360), (Profile::P444_10, 334, 202)]);
    for (profile, w, h) in cases {
        let name = format!("ffenc_{}_{w}x{h}", profile.short_name());
        let raw = cfa4_testkit::temp_path(&dir.join(format!("{name}.raw")));
        let apv = dir.join(format!("{name}.apv"));
        let src = source_frame(profile, w, h);
        ffmpeg_encode(&ff, &src, profile, &raw, &apv);
        let _ = std::fs::remove_file(&raw);
        let data = std::fs::read(&apv).expect("read .apv");

        let hdr = probe(&data).expect("probe FFmpeg-written APV");
        assert_eq!((hdr.width, hdr.height), (w, h), "{profile:?} {w}x{h}: frame size");
        assert_eq!(hdr.profile_idc, profile.idc(), "{profile:?} {w}x{h}: profile");
        assert_eq!(hdr.bit_depth, profile.default_bit_depth(), "{profile:?} {w}x{h}: bit depth");

        let ours = decode_frame(&data).expect("our decoder on FFmpeg-written APV");
        let theirs = ffmpeg_decode(&ff, &apv, profile);
        let ours = planes(&ours);
        assert_eq!(ours.len(), theirs.len(), "{profile:?} {w}x{h}: sample count");
        let max_diff = ours.iter().zip(&theirs).map(|(&a, &b)| a.abs_diff(b)).max().unwrap_or(0);
        assert_eq!(max_diff, 0, "{profile:?} {w}x{h}: our decoder must match FFmpeg bit-exact on apv_vulkan output");

        // Both decoders agreeing is only meaningful if the stream carries the source.
        let peak = ((1u32 << profile.default_bit_depth()) - 1) as f64;
        let y = psnr(&ours[..src.y.len()], &src.y, peak);
        assert!(y > 40.0, "{profile:?} {w}x{h}: luma PSNR {y:.2} dB at qp 12 is too low");
    }
}
