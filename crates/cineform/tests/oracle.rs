//! Bit-exactness against ffmpeg's `cfhd` decoder on streams written by ffmpeg's `cfhd` encoder
//! (the only CineForm encoder available as a test oracle). Fixtures are generated into
//! `target/fixtures/cineform/`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_cineform::{EncodedFormat, Flavor, decode_frame, decode_frame_as, probe};
use cfa4_testkit::fixtures::{fixtures_dir, generate};

struct Case {
    pix: &'static str,
    width: u32,
    height: u32,
    quality: u32,
    source: &'static str,
}

fn run(ff: &Path, args: &[&str]) -> bool {
    Command::new(ff).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).status().is_ok_and(|s| s.success())
}

/// (one CineForm sample, ffmpeg's decoded planes as 16-bit little-endian planar data)
fn fixture(ff: &Path, c: &Case) -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = fixtures_dir("cineform");
    let stem = format!("{}_{}x{}_q{}_{}", c.pix, c.width, c.height, c.quality, c.source.split('=').next().unwrap_or("src"));
    let mov = dir.join(format!("{stem}.mov"));
    let src = format!("{}=s={}x{}", c.source, c.width, c.height);
    let q = c.quality.to_string();
    let mov = generate(&mov, |tmp| {
        run(ff, &["-f", "lavfi", "-i", &src, "-frames:v", "1", "-pix_fmt", c.pix, "-c:v", "cfhd", "-quality", &q, tmp.to_str().unwrap_or("")])
    })?;
    let sample: PathBuf = generate(&dir.join(format!("{stem}.cfhd")), |tmp| {
        run(ff, &["-i", mov.to_str().unwrap_or(""), "-c:v", "copy", "-f", "rawvideo", tmp.to_str().unwrap_or("")])
    })?;
    let raw = generate(&dir.join(format!("{stem}.raw")), |tmp| {
        run(ff, &["-i", mov.to_str().unwrap_or(""), "-f", "rawvideo", "-pix_fmt", c.pix, tmp.to_str().unwrap_or("")])
    })?;
    Some((std::fs::read(sample).ok()?, std::fs::read(raw).ok()?))
}

fn le16(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect()
}

fn check(c: &Case) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cineform oracle") else { return };
    let Some((sample, raw)) = fixture(&ff, c) else { panic!("ffmpeg could not make the {} fixture", c.pix) };
    let h = probe(&sample).expect("probe");
    assert_eq!((h.width, h.display_height), (c.width, c.height));
    let f = decode_frame(&sample).expect("decode");
    let reference = le16(&raw);
    // ffmpeg's planar order: Y U V, or G B R (A); ours: Y Cb Cr, or R G B (A)
    let order: &[usize] = match f.format {
        EncodedFormat::Yuv422 => &[0, 1, 2],
        EncodedFormat::Rgb444 => &[1, 2, 0],
        _ => &[1, 2, 0, 3],
    };
    let mut off = 0usize;
    for &i in order {
        let p = &f.planes[i];
        let n = p.width * p.height;
        let want = &reference[off..off + n];
        off += n;
        let bad: Vec<(usize, u16, u16)> = p.data.iter().zip(want).enumerate().filter(|(_, (a, b))| a != b).map(|(k, (&a, &b))| (k, a, b)).take(8).collect();
        let total = p.data.iter().zip(want).filter(|(a, b)| a != b).count();
        assert!(bad.is_empty(), "{} plane {i} ({}x{}): {total} of {n} samples differ, first (index, ours, ffmpeg): {bad:?}", c.pix, p.width, p.height);
    }
    assert_eq!(off, reference.len(), "plane sizes");
}

#[test]
fn yuv422p10_matches_ffmpeg() {
    check(&Case { pix: "yuv422p10le", width: 128, height: 96, quality: 0, source: "testsrc2" });
}

#[test]
fn yuv422p10_low_quality_matches_ffmpeg() {
    check(&Case { pix: "yuv422p10le", width: 320, height: 240, quality: 12, source: "testsrc2" });
}

#[test]
fn yuv422p10_hd_matches_ffmpeg() {
    check(&Case { pix: "yuv422p10le", width: 1920, height: 1080, quality: 4, source: "mandelbrot" });
}

/// SDK rounding keeps one more bit in the prescaled level, so it may differ from ffmpeg's
/// decoder by a code value, never more.
#[test]
fn sdk_flavor_is_within_one_code_of_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cineform oracle") else { return };
    let c = Case { pix: "yuv422p10le", width: 320, height: 240, quality: 12, source: "testsrc2" };
    let Some((sample, raw)) = fixture(&ff, &c) else { panic!("ffmpeg could not make the fixture") };
    let f = decode_frame_as(&sample, Flavor::Sdk).expect("decode");
    let reference = le16(&raw);
    let ours: Vec<u16> = f.planes.iter().flat_map(|p| p.data.iter().copied()).collect();
    assert_eq!(ours.len(), reference.len());
    let max = ours.iter().zip(&reference).map(|(&a, &b)| (i32::from(a) - i32::from(b)).abs()).max().unwrap_or(0);
    assert!(max <= 1, "max difference {max}");
    assert_eq!(probe(&sample).expect("probe").flavor(), Flavor::Ffmpeg);
}

/// A coded height that is a multiple of 8 above the display height (486 → 488): the frame is
/// cropped.
#[test]
fn display_height_crop_matches_ffmpeg() {
    check(&Case { pix: "yuv422p10le", width: 704, height: 486, quality: 0, source: "mandelbrot" });
}

#[test]
fn gbrp12_matches_ffmpeg() {
    check(&Case { pix: "gbrp12le", width: 128, height: 96, quality: 0, source: "testsrc2" });
}

#[test]
fn gbrp12_low_quality_matches_ffmpeg() {
    check(&Case { pix: "gbrp12le", width: 256, height: 144, quality: 10, source: "mandelbrot" });
}

#[test]
fn gbrap12_matches_ffmpeg() {
    check(&Case { pix: "gbrap12le", width: 160, height: 120, quality: 2, source: "testsrc2" });
}

/// Truncated and bit-flipped real streams decode or fail, never panic.
#[test]
fn damaged_streams_never_panic() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cineform oracle") else { return };
    let c = Case { pix: "gbrap12le", width: 160, height: 120, quality: 2, source: "testsrc2" };
    let Some((sample, _)) = fixture(&ff, &c) else { panic!("ffmpeg could not make the fixture") };
    for cut in (0..sample.len()).step_by(97) {
        let _ = decode_frame(&sample[..cut]);
    }
    let mut x = 0x9e37_79b9u32;
    for _ in 0..500 {
        let mut d = sample.clone();
        for _ in 0..3 {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = (x >> 4) as usize % d.len();
            d[i] ^= 1 << (x >> 29);
        }
        let _ = decode_frame(&d);
    }
}

/// Decoding speed of a 1080p frame (printed; `cargo test --release -- --nocapture`).
#[test]
fn hd_decode_speed() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cineform oracle") else { return };
    let c = Case { pix: "yuv422p10le", width: 1920, height: 1080, quality: 4, source: "mandelbrot" };
    let Some((sample, _)) = fixture(&ff, &c) else { panic!("ffmpeg could not make the fixture") };
    let t = std::time::Instant::now();
    let n = 5;
    for _ in 0..n {
        decode_frame(&sample).expect("decode");
    }
    eprintln!("1080p CineForm: {:.1} ms/frame ({} bytes)", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n), sample.len());
}
