//! The encoder's output against libdav1d: every stream it writes must decode, with libdav1d and
//! with our decoder, to the same pictures, and close to the source.

mod common;
use cfa4_av1::encode::{Encoder, EncoderConfig, Preset, RateControl, YuvFrame};
use common::*;

/// Raw planar 4:2:0 frames (8-bit samples, or 16-bit little-endian for 10-bit) from a lavfi
/// source.
fn source(ff: &std::path::Path, name: &str, lavfi: &str, frames: u32, ten_bit: bool) -> Vec<u8> {
    let path = fixture_dir().join(format!("{name}.yuv"));
    let pix = if ten_bit { "yuv420p10le" } else { "yuv420p" };
    let n = frames.to_string();
    run(ff, &["-f", "lavfi", "-i", lavfi, "-frames:v", &n, "-pix_fmt", pix, "-f", "rawvideo", path.to_str().expect("path")]);
    std::fs::read(&path).expect("read source")
}

fn ivf(w: u32, h: u32, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"DKIF");
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"AV01");
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.extend_from_slice(&25u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(&(i as u64).to_le_bytes());
        out.extend_from_slice(f);
    }
    out
}

struct Case {
    name: &'static str,
    lavfi: &'static str,
    w: u32,
    h: u32,
    frames: u32,
    ten_bit: bool,
    preset: Preset,
    rate: RateControl,
    tiles: u32,
}

fn check(ff: &std::path::Path, c: &Case) {
    let raw = source(ff, c.name, c.lavfi, c.frames, c.ten_bit);
    let (w, h) = (c.w as usize, c.h as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let per = w * h + 2 * cw * ch;
    let mut cfg = EncoderConfig::new(c.w, c.h, 25, 1);
    cfg.bit_depth = if c.ten_bit { 10 } else { 8 };
    cfg.preset = c.preset;
    cfg.rate = c.rate;
    cfg.tiles = c.tiles;
    cfg.threads = 2;
    cfg.keyint = 4;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut packets = Vec::new();
    for i in 0..c.frames as usize {
        let pk = if c.ten_bit {
            let s: Vec<u16> = raw[i * per * 2..(i + 1) * per * 2].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let f = YuvFrame::U16 { y: &s[..w * h], u: &s[w * h..w * h + cw * ch], v: &s[w * h + cw * ch..], y_stride: w, uv_stride: cw };
            enc.encode(&f, i as i64)
        } else {
            let s = &raw[i * per..(i + 1) * per];
            let f = YuvFrame::U8 { y: &s[..w * h], u: &s[w * h..w * h + cw * ch], v: &s[w * h + cw * ch..], y_stride: w, uv_stride: cw };
            enc.encode(&f, i as i64)
        };
        packets.extend(pk.expect("encode").into_iter().map(|p| p.data));
    }
    let path = fixture_dir().join(format!("{}.ivf", c.name));
    std::fs::write(&path, ivf(c.w, c.h, &packets)).expect("write ivf");
    // libdav1d and our decoder agree ...
    check_file(ff, c.name, &path);
    // ... and the pictures are close to the source
    let pics = decode_all(&path).expect("decode");
    let max = if c.ten_bit { 1023.0f64 } else { 255.0 };
    for (i, p) in pics.iter().enumerate() {
        let src: Vec<f64> = if c.ten_bit {
            raw[i * per * 2..i * per * 2 + w * h * 2].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]]) as f64).collect()
        } else {
            raw[i * per..i * per + w * h].iter().map(|&b| b as f64).collect()
        };
        let mse = p.planes[0].iter().zip(&src).map(|(&a, &b)| (a as f64 - b).powi(2)).sum::<f64>() / src.len() as f64;
        let psnr = 10.0 * (max * max / mse.max(1e-9)).log10();
        assert!(psnr > 30.0, "{}: frame {i} psnr {psnr:.2}", c.name);
    }
}

#[test]
fn encoder_streams_decode_identically_with_libdav1d() {
    let Some(ff) = ffmpeg_dav1d() else { return };
    let cases = [
        Case {
            name: "enc_testsrc_speed",
            lavfi: "testsrc2=s=352x288:r=25",
            w: 352,
            h: 288,
            frames: 6,
            ten_bit: false,
            preset: Preset::Speed,
            rate: RateControl::Crf(30.0),
            tiles: 0,
        },
        Case {
            name: "enc_mandel_balanced_tiles",
            lavfi: "mandelbrot=s=320x240:r=25",
            w: 320,
            h: 240,
            frames: 6,
            ten_bit: false,
            preset: Preset::Balanced,
            rate: RateControl::QIndex(100),
            tiles: 4,
        },
        Case {
            name: "enc_noise_odd_quality",
            lavfi: "testsrc2=s=204x118:r=25,scale=203:117,noise=alls=12:allf=t",
            w: 203,
            h: 117,
            frames: 5,
            ten_bit: false,
            preset: Preset::Quality,
            rate: RateControl::Crf(24.0),
            tiles: 0,
        },
        Case {
            name: "enc_grad_10bit",
            lavfi: "gradients=s=256x144:r=25,noise=alls=6:allf=t",
            w: 256,
            h: 144,
            frames: 5,
            ten_bit: true,
            preset: Preset::Balanced,
            rate: RateControl::Crf(28.0),
            tiles: 2,
        },
    ];
    for c in &cases {
        check(&ff, c);
    }
}
