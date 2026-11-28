//! Huffyuv streams of random residuals against ffmpeg's decoder: 4:2:0 (12 bits per pixel,
//! Huffyuv 2.x header) and the classic tables of Huffyuv 1.x (no tables in the header). ffmpeg
//! cannot encode either, so the frames are random bits: both tables used here are complete
//! prefix codes (all codes 8 bits long for 4:2:0, the classic tables for 1.x), so every bit
//! string is a valid stream of random residuals, whatever the predictor and the row layout.
//! Skipped when ffmpeg is not installed.

use cfa4_huffyuv::{Decoder, Format};

#[path = "support/hymt.rs"]
mod support;
use support::*;

fn rng(seed: u32) -> impl FnMut() -> u8 {
    let mut s = seed | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s >> 11) as u8
    }
}

/// The tables of 256 codes of 8 bits (run lengths: 255, then 1, of length 8), three times.
fn flat_tables() -> Vec<u8> {
    [0x08u8, 0xFF, 0x28].repeat(3)
}

fn frames(n: usize, bytes: usize, seed: u32) -> Vec<Vec<u8>> {
    let mut r = rng(seed);
    (0..n).map(|_| (0..bytes).map(|_| r()).collect()).collect()
}

fn ffmpeg_raw(avi_bytes: &[u8], name: &str, pix_fmt: &str) -> Vec<u8> {
    let file = dir().join(format!("{name}.avi"));
    std::fs::write(&file, avi_bytes).expect("write");
    let raw = dir().join(format!("{name}.raw"));
    ffmpeg(&["-i", &file.to_string_lossy(), "-f", "rawvideo", "-pix_fmt", pix_fmt, &raw.to_string_lossy()]);
    std::fs::read(&raw).expect("raw")
}

/// Our pictures in ffmpeg's raw layout (planar Y U V, planar G B R, interleaved B G R A).
fn to_raw(p: &cfa4_huffyuv::Picture) -> Vec<u8> {
    match p.format {
        Format::Yuv422 | Format::Yuv420 => p.planes.concat(),
        Format::Rgb => [&p.planes[1], &p.planes[2], &p.planes[0]].iter().flat_map(|pl| pl.iter().copied()).collect(),
        Format::Rgba => (0..p.planes[0].len()).flat_map(|i| [p.planes[2][i], p.planes[1][i], p.planes[0][i], p.planes[3][i]]).collect(),
    }
}

fn check(name: &str, w: u32, h: u32, bit_count: u16, extra: &[u8], pix_fmt: &str, data: &[Vec<u8>]) {
    let want = ffmpeg_raw(&avi_bits(w, h, bit_count, b"HFYU", extra, data), name, pix_fmt);
    let mut dec = Decoder::new(w, h, bit_count, extra).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut ours = Vec::new();
    for (i, f) in data.iter().enumerate() {
        ours.extend(to_raw(&dec.decode(f).unwrap_or_else(|e| panic!("{name} frame {i}: {e}"))));
    }
    assert_eq!(ours.len(), want.len(), "{name}: size");
    if let Some(i) = ours.iter().zip(&want).position(|(a, b)| a != b) {
        panic!("{name}: first difference at byte {i} of {}", want.len());
    }
}

#[test]
fn yuv420_matches_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let sizes = [(8u32, 2u32), (8, 4), (10, 6), (32, 16), (36, 20), (64, 48), (64, 34)];
    for (pi, pred) in ["left", "plane", "median"].iter().enumerate() {
        for (flags, il) in [(0x20u8, "prog"), (0x10, "il")] {
            for &(w, h) in &sizes {
                let mut extra = vec![pi as u8, 12, flags, 0];
                extra.extend(flat_tables());
                let name = format!("y420_{pred}_{il}_{w}x{h}");
                check(&name, w, h, 12, &extra, "yuv420p", &frames(2, (w * h * 3) as usize + 64, w * 31 + h));
            }
        }
    }
    // no flag: the 1.x rule (taller than 288 rows is interlaced)
    for (pi, h) in [(0u8, 300u32), (1, 300), (2, 300), (2, 288)] {
        let mut extra = vec![pi, 12, 0, 0];
        extra.extend(flat_tables());
        check(&format!("y420_rule_{pi}_{h}"), 16, h, 12, &extra, "yuv420p", &frames(1, (16 * h * 3) as usize + 64, h));
    }
}

#[test]
fn classic_tables_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    // biBitCount low bits: 0 old (left), 1 left, 2 left with decorrelation, 3 gradient, 4 median
    for low in 0u16..=4 {
        for (bpp, pix_fmt) in [(16u16, "yuv422p"), (24, "gbrp")] {
            // ffmpeg has no median predictor for RGB
            if bpp == 24 && low == 4 {
                continue;
            }
            for &(w, h) in &[(64u32, 16u32), (32, 5), (68, 33)] {
                let name = format!("classic_{bpp}_{low}_{w}x{h}");
                check(&name, w, h, bpp | low, &[], pix_fmt, &frames(2, (w * h) as usize * 6 + 64, u32::from(bpp | low) * 7 + w));
            }
        }
    }
    // interlaced by the 1.x rule
    check("classic_il", 64, 300, 16 | 3, &[], "yuv422p", &frames(1, 64 * 300 * 6 + 64, 5));
    check("classic_rgba", 40, 24, 32 | 1, &[], "bgra", &frames(2, 40 * 24 * 8 + 64, 6));
}

#[test]
fn damaged_headers_and_data_never_panic() {
    let mut r = rng(77);
    for n in 0..70u8 {
        let extra: Vec<u8> = (0..n).map(|i| i.wrapping_mul(53) ^ n).collect();
        for bits in [8u16, 12, 13, 16, 17, 19, 20, 24, 25, 32, 33] {
            if let Ok(mut d) = Decoder::new(16, 8, bits, &extra) {
                for len in [0usize, 3, 20, 200] {
                    let data: Vec<u8> = (0..len).map(|_| r()).collect();
                    let _ = d.decode(&data);
                }
            }
        }
    }
    for pred in 0..3u8 {
        let mut extra = vec![pred, 12, 0x20, 0];
        extra.extend(flat_tables());
        if let Ok(mut d) = Decoder::new(16, 8, 12, &extra) {
            for len in [0usize, 3, 5, 40, 150, 400] {
                let data: Vec<u8> = (0..len).map(|_| r()).collect();
                let _ = d.decode(&data);
            }
        }
    }
}
