//! VC-2 streams written here, with random coefficients, decoded by us and by ffmpeg's decoder
//! (an external oracle): every sample must be equal. ffmpeg's encoder writes high-quality
//! pictures with four of the seven wavelets only; these streams also cover low-delay pictures
//! (DC prediction, slice sizes from a fraction), the Deslauriers-Dubuc (13,7), Fidelity and
//! Daubechies (9,7) wavelets, custom quantisation matrices, slice prefix bytes and size scalers.
//! Asymmetric transforms (ffmpeg doesn't decode them) are checked on a picture with a known
//! result.
//!
//! Where ffmpeg departs from ST 2042-1 the BBC reference decoder (`vc2-reference`) settles it:
//! Daubechies (9,7) rounding (allowed as drift here), and edge extension in subbands only one
//! or two samples high, which these streams avoid (every subband is at least 3 samples high). The oracle tests skip when ffmpeg is not installed.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use cfa4_dirac::Decoder;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bit: u32,
}

impl Writer {
    fn bit(&mut self, b: bool) {
        if self.bit == 0 {
            self.bytes.push(0);
        }
        if b {
            *self.bytes.last_mut().unwrap() |= 0x80 >> self.bit;
        }
        self.bit = (self.bit + 1) % 8;
    }
    fn bits(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.bit(v >> i & 1 == 1);
        }
    }
    fn uint(&mut self, v: u64) {
        let x = v + 1;
        let n = 63 - x.leading_zeros();
        for i in (0..n).rev() {
            self.bit(false);
            self.bit(x >> i & 1 == 1);
        }
        self.bit(true);
    }
    fn sint(&mut self, v: i64) {
        self.uint(v.unsigned_abs());
        if v != 0 {
            self.bit(v < 0);
        }
    }
    fn align(&mut self) {
        self.bit = 0;
    }
    fn len_bits(&self) -> usize {
        if self.bit == 0 { self.bytes.len() * 8 } else { (self.bytes.len() - 1) * 8 + self.bit as usize }
    }
    /// Append the bits of `o`.
    fn append(&mut self, o: &Writer) {
        for i in 0..o.len_bits() {
            self.bit(o.bytes[i / 8] >> (7 - i % 8) & 1 == 1);
        }
    }
}

struct Params {
    w: u32,
    h: u32,
    /// Colour difference format index (0 4:4:4, 1 4:2:2, 2 4:2:0).
    cdf: u64,
    /// Signal range preset (2: 8-bit video, 3: 10-bit video).
    range: u64,
    major: u64,
    ld: bool,
    wavelet: u64,
    wavelet_ho: u64,
    depth: u32,
    depth_ho: u32,
    slices: (u64, u64),
    custom_qm: Option<Vec<u64>>,
    /// High quality: prefix bytes and size scaler.
    hq: (u64, u64),
    /// Low delay: bytes per slice denominator (the numerator is chosen to fit).
    ld_den: u64,
    max_coeff: i64,
    max_qindex: u32,
    /// Every coefficient: `Some(v)` writes `v` in the DC band and zero elsewhere.
    dc_only: Option<i64>,
}

fn parse_info(code: u8, body: &[u8], next: bool, previous: usize) -> Vec<u8> {
    let mut v = b"BBCD".to_vec();
    v.push(code);
    let n = if next { (13 + body.len()) as u32 } else { 0 };
    v.extend_from_slice(&n.to_be_bytes());
    v.extend_from_slice(&(previous as u32).to_be_bytes());
    v.extend_from_slice(body);
    v
}

/// Subband sizes in coding order for a `w` x `h` component.
fn bands(p: &Params, w: u32, h: u32) -> Vec<(u64, u64)> {
    let all = p.depth + p.depth_ho;
    let pw = u64::from(w).div_ceil(1 << all) << all;
    let ph = u64::from(h).div_ceil(1 << p.depth) << p.depth;
    let mut out = vec![(pw >> all, ph >> p.depth)];
    for level in 1..=all {
        let bw = pw >> (all - level + 1);
        if level <= p.depth_ho {
            out.push((bw, ph >> p.depth));
        } else {
            out.extend([(bw, ph >> (all - level + 1)); 3]);
        }
    }
    out
}

/// A sequence header, one picture and an end of sequence.
fn stream(p: &Params, rng: &mut Rng) -> Vec<u8> {
    let mut s = Writer::default();
    s.uint(p.major);
    s.uint(0);
    s.uint(if p.ld { 0 } else { 3 });
    s.uint(0);
    s.uint(0); // base video format: custom
    s.bit(true);
    s.uint(u64::from(p.w));
    s.uint(u64::from(p.h));
    s.bit(true);
    s.uint(p.cdf);
    s.bit(false); // scan format
    s.bit(false); // frame rate
    s.bit(false); // pixel aspect ratio
    s.bit(true); // clean area: the whole frame
    s.uint(u64::from(p.w));
    s.uint(u64::from(p.h));
    s.uint(0);
    s.uint(0);
    s.bit(true);
    s.uint(p.range);
    s.bit(false); // colour spec
    s.uint(0); // frames
    s.align();

    let (cw, ch) = match p.cdf {
        0 => (p.w, p.h),
        1 => (p.w / 2, p.h),
        _ => (p.w / 2, p.h / 2),
    };
    let (yb, cb) = (bands(p, p.w, p.h), bands(p, cw, ch));
    let nbands = yb.len();
    let (sx_n, sy_n) = p.slices;
    let coeff = |rng: &mut Rng, band: usize| -> i64 {
        match p.dc_only {
            Some(v) => {
                if band == 0 {
                    v
                } else {
                    0
                }
            }
            None => {
                if rng.below(3) == 0 {
                    0
                } else {
                    i64::from(rng.below(2 * p.max_coeff as u32 + 1)) - p.max_coeff
                }
            }
        }
    };
    let region = |b: (u64, u64), sx: u64, sy: u64| (b.0 * sx / sx_n, b.0 * (sx + 1) / sx_n, b.1 * sy / sy_n, b.1 * (sy + 1) / sy_n);
    // Each slice: qindex, luma bits, colour difference bits (C1 and C2 separately for HQ).
    let mut slices = Vec::new();
    for sy in 0..sy_n {
        for sx in 0..sx_n {
            let q = u64::from(rng.below(p.max_qindex + 1));
            let mut y = Writer::default();
            for (i, &b) in yb.iter().enumerate() {
                let (x1, x2, y1, y2) = region(b, sx, sy);
                for _ in 0..(x2 - x1) * (y2 - y1) {
                    y.sint(coeff(rng, i));
                }
            }
            let mut comps = [Writer::default(), Writer::default()];
            if p.ld {
                for (i, &b) in cb.iter().enumerate() {
                    let (x1, x2, y1, y2) = region(b, sx, sy);
                    for _ in 0..(x2 - x1) * (y2 - y1) {
                        comps[0].sint(coeff(rng, i));
                        comps[0].sint(coeff(rng, i));
                    }
                }
            } else {
                for c in &mut comps {
                    for (i, &b) in cb.iter().enumerate() {
                        let (x1, x2, y1, y2) = region(b, sx, sy);
                        for _ in 0..(x2 - x1) * (y2 - y1) {
                            c.sint(coeff(rng, i));
                        }
                    }
                }
            }
            slices.push((q, y, comps));
        }
    }

    let mut pic = Writer::default();
    pic.bits(7, 32); // picture number
    pic.uint(p.wavelet);
    pic.uint(u64::from(p.depth));
    if p.major >= 3 {
        pic.bit(p.wavelet_ho != p.wavelet);
        if p.wavelet_ho != p.wavelet {
            pic.uint(p.wavelet_ho);
        }
        pic.bit(p.depth_ho > 0);
        if p.depth_ho > 0 {
            pic.uint(u64::from(p.depth_ho));
        }
    }
    pic.uint(sx_n);
    pic.uint(sy_n);
    // low delay: the largest slice decides the bytes per slice
    let need = slices.iter().map(|(_, y, c)| 7 + 64 + y.len_bits() + c[0].len_bits()).max().unwrap() as u64;
    let (num, den) = (need.div_ceil(8) * p.ld_den + p.ld_den / 2, p.ld_den);
    if p.ld {
        pic.uint(num);
        pic.uint(den);
    } else {
        pic.uint(p.hq.0);
        pic.uint(p.hq.1);
    }
    match &p.custom_qm {
        Some(m) => {
            assert_eq!(m.len(), nbands);
            pic.bit(true);
            for &v in m {
                pic.uint(v);
            }
        }
        None => pic.bit(false),
    }
    pic.align();
    for (n, (q, y, comps)) in slices.iter().enumerate() {
        let n = n as u64;
        if p.ld {
            let bytes = (n + 1) * num / den - n * num / den;
            let start = pic.len_bits();
            pic.bits(*q, 7);
            let length_bits = 64 - (bytes * 8 - 8).leading_zeros();
            pic.bits(y.len_bits() as u64, length_bits);
            pic.append(y);
            pic.append(&comps[0]);
            let used = pic.len_bits() - start;
            assert!(used as u64 <= bytes * 8);
            for _ in used as u64..bytes * 8 {
                pic.bit(rng.below(2) == 1);
            }
        } else {
            for _ in 0..p.hq.0 {
                pic.bits(u64::from(rng.below(256)), 8);
            }
            pic.bits(*q, 8);
            for w in [y, &comps[0], &comps[1]] {
                let len = (w.len_bits() as u64).div_ceil(8).div_ceil(p.hq.1);
                assert!(len < 256, "slice component too long");
                pic.bits(len, 8);
                pic.append(w);
                pic.align();
                for _ in w.len_bits().div_ceil(8) as u64..len * p.hq.1 {
                    pic.bits(u64::from(rng.below(256)), 8);
                }
            }
        }
    }
    let header = parse_info(0x00, &s.bytes, true, 0);
    let picture = parse_info(if p.ld { 0xC8 } else { 0xE8 }, &pic.bytes, true, header.len());
    let end = parse_info(0x10, &[], false, picture.len());
    [header, picture, end].concat()
}

fn base(ld: bool) -> Params {
    Params {
        w: 64,
        h: 48,
        cdf: 2,
        range: 2,
        major: if ld { 1 } else { 2 },
        ld,
        wavelet: 0,
        wavelet_ho: 0,
        depth: 2,
        depth_ho: 0,
        slices: (4, 3),
        custom_qm: None,
        hq: (0, 1),
        ld_den: 1,
        max_coeff: 6,
        max_qindex: 12,
        dc_only: None,
    }
}

/// The largest difference allowed against ffmpeg's Daubechies (9,7) synthesis.
const DAUB97_DRIFT: u8 = 2;

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn ffmpeg_decode(name: &str, data: &[u8], pix_fmt: &str) -> Option<Vec<u8>> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/dirac");
    std::fs::create_dir_all(&d).unwrap();
    let (src, raw) = (d.join(format!("syn_{name}.vc2")), d.join(format!("syn_{name}.yuv")));
    std::fs::write(&src, data).unwrap();
    let ok = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "dirac", "-i"])
        .arg(&src)
        .args(["-f", "rawvideo", "-pix_fmt", pix_fmt])
        .arg(&raw)
        .status()
        .is_ok_and(|s| s.success());
    if ok { std::fs::read(&raw).ok() } else { None }
}

#[test]
fn synthetic_streams_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("SKIPPED (VC-2 synthetic oracle): ffmpeg is not installed");
        return;
    }
    let mut cases: Vec<(String, Params)> = Vec::new();
    for wavelet in 0..=6 {
        for ld in [true, false] {
            let mut p = base(ld);
            p.wavelet = wavelet;
            p.wavelet_ho = wavelet;
            p.depth = 1 + (wavelet as u32 % 3);
            cases.push((format!("{}_w{wavelet}", if ld { "ld" } else { "hq" }), p));
        }
    }
    let mut p = base(true);
    (p.ld_den, p.slices, p.cdf) = (3, (5, 4), 1);
    cases.push(("ld_fraction_422".into(), p));
    let mut p = base(true);
    (p.cdf, p.depth, p.custom_qm) = (0, 3, Some(vec![1, 2, 2, 0, 3, 3, 1, 4, 4, 2]));
    cases.push(("ld_qm_444".into(), p));
    let mut p = base(true);
    (p.w, p.h, p.slices, p.depth) = (64, 48, (1, 1), 3);
    cases.push(("ld_one_slice".into(), p));
    let mut p = base(false);
    (p.hq, p.custom_qm, p.depth) = ((3, 2), Some(vec![0, 1, 1, 0, 2, 2, 1]), 2);
    cases.push(("hq_prefix_scaler_qm".into(), p));
    let mut p = base(false);
    (p.range, p.cdf, p.max_coeff, p.max_qindex) = (3, 1, 40, 24);
    cases.push(("hq_10bit_422".into(), p));
    let mut p = base(false);
    (p.w, p.h, p.slices, p.depth) = (72, 40, (3, 5), 3);
    cases.push(("hq_odd_slices".into(), p));

    let mut rng = Rng(0x2042_0001);
    let mut failed = Vec::new();
    for (name, p) in &cases {
        let data = stream(p, &mut rng);
        let pix_fmt = match (p.cdf, p.range) {
            (0, 3) => "yuv444p10le",
            (1, 3) => "yuv422p10le",
            (_, 3) => "yuv420p10le",
            (0, _) => "yuv444p",
            (1, _) => "yuv422p",
            _ => "yuv420p",
        };
        let Some(want) = ffmpeg_decode(name, &data, pix_fmt) else {
            failed.push(format!("{name}: ffmpeg could not decode it"));
            continue;
        };
        let pics = Decoder::new().decode(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(pics.len(), 1, "{name}");
        let got: Vec<u8> = if p.range == 3 {
            pics[0].planes.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()
        } else {
            pics[0].planes.iter().flatten().map(|&v| v as u8).collect()
        };
        if got.len() != want.len() {
            failed.push(format!("{name}: {} bytes, ffmpeg {}", got.len(), want.len()));
        } else if p.wavelet == 6 {
            // ffmpeg ends each Daubechies (9,7) level with a plain right shift (it drops the
            // rounding of §15.4.2, which the BBC reference decoder keeps): allow that drift.
            let worst = got.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            if worst > DAUB97_DRIFT {
                failed.push(format!("{name}: differs from ffmpeg by up to {worst}"));
            }
        } else if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            failed.push(format!("{name}: byte {i} differs (ours {}, ffmpeg {})", got[i], want[i]));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// Asymmetric transforms (`dwt_depth_ho`, major version 3): a picture whose coefficients are a
/// constant DC band synthesises, with the Haar filter without shift, to that constant everywhere.
#[test]
fn asymmetric_transform_keeps_a_flat_picture_flat() {
    for (wavelet_ho, depth, depth_ho) in [(3, 1, 2), (3, 2, 1), (3, 0, 3)] {
        let mut p = base(false);
        (p.major, p.wavelet, p.wavelet_ho, p.depth, p.depth_ho, p.dc_only, p.max_qindex) = (3, 3, wavelet_ho, depth, depth_ho, Some(-20), 0);
        let data = stream(&p, &mut Rng(1));
        let pics = Decoder::new().decode(&data).unwrap_or_else(|e| panic!("ho {depth_ho}: {e}"));
        assert_eq!(pics.len(), 1);
        for plane in &pics[0].planes {
            assert!(plane.iter().all(|&v| v == 108), "depth {depth} / ho {depth_ho}: {:?}", &plane[..8]);
        }
    }
}
