//! Intra CineForm encoder: three-level 2/6 spatial wavelet, run-length coded highpass bands
//! (codebook 17 with cubic companding) and a 16-bit lowpass band.
//!
//! The forward transform is the exact inverse of the decoder's filters ([`crate::wavelet`]):
//! every lowpass value is the sum of a pair and every highpass value is chosen so that the
//! decoder's synthesis gives the pair back, so without quantization the transform is lossless
//! (apart from the encoder prescale of the middle levels, as in the SDK). The sample layout
//! (tag order, level and band headers, channel size index) follows what the decoders expect
//! from SDK and ffmpeg encoders.

use cfa4_bitstream::BitWriter;

use crate::tables::{CODESET17, CODESET17_BAND_END};
use crate::tags::*;
use crate::wavelet::lowpass_pair;
use crate::{EncodedFormat, Error, Result};

/// Wavelet levels of an intra sample.
const LEVELS: usize = 3;
/// Largest accepted frame side (as the decoder).
const MAX_SIDE: u32 = 16384;
/// Largest companded magnitude of codebook 17 (`255 + 255³·768 >> 24`).
const MAX_COMPANDED: i32 = 1014;

/// Quality levels, best first; they set the quantizers of the finest wavelet level (the SDK's
/// encoded quality names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Quality {
    FilmScan3,
    FilmScan2,
    #[default]
    FilmScan1,
    High,
    Medium,
    Low,
}

impl Quality {
    pub const ALL: [Quality; 6] = [Quality::FilmScan3, Quality::FilmScan2, Quality::FilmScan1, Quality::High, Quality::Medium, Quality::Low];

    /// Base quantizer of the finest level's horizontal and vertical bands at 10 bits.
    fn fine(self) -> u16 {
        match self {
            Quality::FilmScan3 => 2,
            Quality::FilmScan2 => 4,
            Quality::FilmScan1 => 6,
            Quality::High => 10,
            Quality::Medium => 16,
            Quality::Low => 32,
        }
    }
}

/// What to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Width in pixels: a multiple of 16 for 4:2:2, of 8 for 4:4:4.
    pub width: u32,
    /// Height in pixels (coded as the next multiple of 8, signalled as the display height).
    pub height: u32,
    /// [`EncodedFormat::Yuv422`] (10-bit), [`EncodedFormat::Rgb444`] or
    /// [`EncodedFormat::Rgba4444`] (12-bit).
    pub format: EncodedFormat,
    pub quality: Quality,
}

impl EncoderConfig {
    /// Bits per input sample: 10 for 4:2:2, 12 for RGB(A).
    pub fn bit_depth(&self) -> u8 {
        match self.format {
            EncodedFormat::Yuv422 => 10,
            _ => 12,
        }
    }

    /// Size of input plane `c` (Y Cb Cr or R G B A order): chroma is half width for 4:2:2.
    pub fn plane_size(&self, c: usize) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if self.format == EncodedFormat::Yuv422 && c > 0 { (w / 2, h) } else { (w, h) }
    }

    /// Check that the frame can be coded.
    pub fn validate(&self) -> Result<()> {
        let align = match self.format {
            EncodedFormat::Yuv422 => 16,
            EncodedFormat::Rgb444 | EncodedFormat::Rgba4444 => 8,
            _ => return Err(Error::Unsupported("encoding this format")),
        };
        if self.width == 0 || self.height == 0 || self.width > MAX_SIDE || self.height > MAX_SIDE {
            return Err(Error::Invalid("frame size"));
        }
        if !self.width.is_multiple_of(align) {
            return Err(Error::Unsupported("width not a multiple of 16 (4:2:2) or 8 (4:4:4)"));
        }
        Ok(())
    }
}

/// Writes tags and raw words.
struct Out(Vec<u8>);

impl Out {
    fn tag(&mut self, tag: u16, value: u16) {
        self.0.extend_from_slice(&tag.to_be_bytes());
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn optional(&mut self, tag: u16, value: u16) {
        self.0.extend_from_slice(&tag.wrapping_neg().to_be_bytes());
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn word(&mut self, w: u32) {
        self.0.extend_from_slice(&w.to_be_bytes());
    }
}

// tags only an encoder writes (values from the SDK's `codec.h`)
const NUM_FRAMES: u16 = 11;
const NUM_SUBBANDS: u16 = 14;
const NUM_SPATIAL: u16 = 15;
const FIRST_WAVELET: u16 = 16;
const SAMPLE_END: u16 = 18;
const LOWPASS_SUBBAND: u16 = 25;
const NUM_LEVELS: u16 = 26;
const WAVELET_TYPE: u16 = 37;
const WAVELET_NUMBER: u16 = 38;
const WAVELET_LEVEL: u16 = 39;
const NUM_BANDS: u16 = 40;
const HIGHPASS_WIDTH: u16 = 41;
const HIGHPASS_HEIGHT: u16 = 42;
const LOWPASS_BORDER: u16 = 43;
const HIGHPASS_BORDER: u16 = 44;
const LOWPASS_SCALE: u16 = 45;
const LOWPASS_DIVISOR: u16 = 46;
const BAND_NUMBER: u16 = 48;
const BAND_SCALE: u16 = 54;
const BAND_TRAILER: u16 = 56;
const INTERLACED_FLAGS: u16 = 69;
const ENCODER_QUALITY: u16 = 68;
const SAMPLE_TYPE_IFRAME: u16 = 9;
const SAMPLE_TYPE_CHANNEL: u16 = 3;
const MARKER_LOWPASS_HEADER: u16 = 0x1a4a;
const MARKER_LOWPASS_TRAILER: u16 = 0x1b4b;
const MARKER_HIGHPASS_HEADER: u16 = 0x0d0d;
const MARKER_HIGHPASS_TRAILER: u16 = 0x0c0c;
const MARKER_BAND_HEADER: u16 = 0x0e0e;
/// Wavelet types: spatial, and the frame-level type the finest level is labelled with.
const WAVELET_SPATIAL: u16 = 3;
const WAVELET_FINEST: u16 = 5;

/// Encode one frame. `planes` are Y, Cb, Cr (chroma at half width) or R, G, B (, A), each
/// `plane_size(c)` row-major samples of [`EncoderConfig::bit_depth`] bits.
pub fn encode_frame(cfg: &EncoderConfig, planes: &[&[u16]]) -> Result<Vec<u8>> {
    cfg.validate()?;
    let n = cfg.format.channels();
    if planes.len() < n {
        return Err(Error::Invalid("too few planes"));
    }
    for (c, p) in planes.iter().take(n).enumerate() {
        let (w, h) = cfg.plane_size(c);
        if p.len() < w * h {
            return Err(Error::Invalid("plane too small"));
        }
    }
    let depth = cfg.bit_depth();
    let prescale = prescale_table(depth);
    // coded channel order: Y Cr Cb / G R B A
    let order: &[usize] = match cfg.format {
        EncodedFormat::Yuv422 => &[0, 2, 1],
        EncodedFormat::Rgb444 => &[1, 0, 2],
        _ => &[1, 0, 2, 3],
    };
    let coded_height = cfg.height.next_multiple_of(8) as usize;
    let one = |&c: &usize| -> Vec<u8> {
        let (w, h) = cfg.plane_size(c);
        let src = planes.get(c).copied().unwrap_or(&[]);
        encode_channel(src, w, h, coded_height, depth, &prescale, cfg.quality)
    };
    #[cfg(feature = "threads")]
    let channels: Vec<Vec<u8>> = {
        use rayon::prelude::*;
        order.par_iter().map(one).collect()
    };
    #[cfg(not(feature = "threads"))]
    let channels: Vec<Vec<u8>> = order.iter().map(one).collect();

    let mut out = Out(Vec::with_capacity(channels.iter().map(Vec::len).sum::<usize>() + 256));
    out.tag(SAMPLE, SAMPLE_TYPE_IFRAME);
    out.tag(INDEX, n as u16);
    for ch in &channels {
        // a channel's size leaves out its closing level trailer
        out.word(ch.len().saturating_sub(4) as u32);
    }
    out.tag(TRANSFORM_TYPE, TRANSFORM_TYPE_SPATIAL);
    out.tag(NUM_FRAMES, 1);
    out.tag(NUM_CHANNELS, n as u16);
    out.tag(
        ENCODED_FORMAT,
        match cfg.format {
            EncodedFormat::Yuv422 => 1,
            EncodedFormat::Rgb444 => 3,
            _ => 4,
        },
    );
    out.tag(NUM_WAVELETS, LEVELS as u16);
    out.tag(NUM_SUBBANDS, (3 * LEVELS + 1) as u16);
    out.tag(NUM_SPATIAL, (LEVELS - 1) as u16);
    out.tag(FIRST_WAVELET, WAVELET_SPATIAL);
    out.tag(FRAME_WIDTH, cfg.width as u16);
    out.tag(FRAME_HEIGHT, coded_height as u16);
    out.optional(FRAME_DISPLAY_HEIGHT, cfg.height as u16);
    out.optional(INTERLACED_FLAGS, 0);
    out.tag(PRECISION, u16::from(depth));
    out.tag(PRESCALE_TABLE, prescale.iter().enumerate().fold(0u16, |v, (i, &p)| v | (u16::from(p) << (14 - 2 * i))));
    out.tag(ENCODER_QUALITY, 1);
    for (c, ch) in channels.iter().enumerate() {
        if c > 0 {
            out.tag(SAMPLE, SAMPLE_TYPE_CHANNEL);
            out.tag(CHANNEL, c as u16);
        }
        out.0.extend_from_slice(ch);
    }
    out.tag(SAMPLE_END, 0);
    Ok(out.0)
}

/// Encoder prescale per wavelet (finest first), the decoder's defaults.
fn prescale_table(depth: u8) -> [u8; LEVELS] {
    if depth >= 12 { [0, 2, 2] } else { [0, 2, 0] }
}

/// One level of the forward transform: `src` (`w × h`, both even) into the four bands of
/// `w/2 × h/2` (lowpass, horizontal highpass, vertical highpass, diagonal).
fn forward_level(src: &[i32], w: usize, h: usize) -> [Vec<i32>; 4] {
    let (hw, hh) = (w / 2, h / 2);
    // horizontal: rows into lowpass and highpass halves
    let mut lo = vec![0i32; hw * h];
    let mut hi = vec![0i32; hw * h];
    for ((row, l), hrow) in src.chunks_exact(w).zip(lo.chunks_exact_mut(hw)).zip(hi.chunks_exact_mut(hw)) {
        forward_1d(row, l, hrow);
    }
    // vertical, a row at a time: the lowpass and highpass of each half
    let (b0, b2) = forward_vertical(&lo, hw, hh);
    let (b1, b3) = forward_vertical(&hi, hw, hh);
    [b0, b1, b2, b3]
}

/// Vertical forward filter of a `w × 2n` image into `w × n` lowpass and highpass images.
fn forward_vertical(src: &[i32], w: usize, n: usize) -> (Vec<i32>, Vec<i32>) {
    let mut low = vec![0i32; w * n];
    let mut high = vec![0i32; w * n];
    if w == 0 {
        return (low, high);
    }
    for (i, l) in low.chunks_exact_mut(w).enumerate() {
        let (a, b) = (src.get(2 * i * w..(2 * i + 1) * w).unwrap_or(&[]), src.get((2 * i + 1) * w..(2 * i + 2) * w).unwrap_or(&[]));
        for ((d, &x0), &x1) in l.iter_mut().zip(a).zip(b) {
            *d = x0 + x1;
        }
    }
    let row = |b: &[i32], r: usize| -> Vec<i32> { b.get(r * w..(r + 1) * w).map(<[i32]>::to_vec).unwrap_or_default() };
    for (i, hrow) in high.chunks_exact_mut(w).enumerate() {
        let even = src.get(2 * i * w..(2 * i + 1) * w).unwrap_or(&[]);
        if i == 0 || i + 1 >= n {
            for (x, (d, &x0)) in hrow.iter_mut().zip(even).enumerate() {
                let (e, _) = lowpass_pair(n, i, |k| low.get(k * w + x).copied().unwrap_or(0));
                *d = 2 * x0 - e;
            }
            continue;
        }
        let (p, c, q) = (row(&low, i - 1), row(&low, i), row(&low, i + 1));
        for ((((d, &x0), &p), &c), &q) in hrow.iter_mut().zip(even).zip(&p).zip(&c).zip(&q) {
            *d = 2 * x0 - (((p - q + 4) >> 3) + c);
        }
    }
    (low, high)
}

/// Forward 2/6 filter of `2n` samples into `n = low.len()` lowpass and highpass values:
/// lowpass `x0 + x1`, highpass `2·x0 − e` where `e` is the decoder's even prediction from the
/// lowpass values, so that synthesis gives `x0` and `x1` back exactly.
fn forward_1d(src: &[i32], low: &mut [i32], high: &mut [i32]) {
    let n = low.len().min(high.len());
    for (l, pair) in low.iter_mut().zip(src.chunks_exact(2)) {
        if let &[x0, x1] = pair {
            *l = x0 + x1;
        }
    }
    let low: &[i32] = low;
    let x = |k: usize| src.get(k).copied().unwrap_or(0);
    let edge = |i: usize| 2 * x(2 * i) - lowpass_pair(n, i, |k| low.get(k).copied().unwrap_or(0)).0;
    if n < 3 {
        for (i, h) in high.iter_mut().take(n).enumerate() {
            *h = edge(i);
        }
        return;
    }
    if let Some(h) = high.first_mut() {
        *h = edge(0);
    }
    for ((h, w), pair) in high.iter_mut().skip(1).zip(low.windows(3)).zip(src.chunks_exact(2).skip(1)) {
        if let (&[p, c, q], Some(&x0)) = (w, pair.first()) {
            *h = 2 * x0 - (((p - q + 4) >> 3) + c);
        }
    }
    if let Some(h) = high.get_mut(n - 1) {
        *h = edge(n - 1);
    }
}

/// Encode one channel: the lowpass band and the nine highpass bands, finest level last.
fn encode_channel(src: &[u16], w: usize, h: usize, coded_h: usize, depth: u8, prescale: &[u8; LEVELS], quality: Quality) -> Vec<u8> {
    // the coded plane repeats the last row down to a multiple of 8
    let mut plane = Vec::with_capacity(w * coded_h);
    let max = (1i32 << depth) - 1;
    for y in 0..coded_h {
        let row = src.get(y.min(h.saturating_sub(1)) * w..).unwrap_or(&[]);
        plane.extend((0..w).map(|x| i32::from(row.get(x).copied().unwrap_or(0)).min(max)));
    }
    let mut levels: Vec<[Vec<i32>; 4]> = Vec::with_capacity(LEVELS);
    let (mut cw, mut ch) = (w, coded_h);
    let mut input = plane;
    for (i, &p) in prescale.iter().enumerate() {
        if i > 0 && p == 2 {
            for v in &mut input {
                *v = (*v + 2) >> 2;
            }
        }
        let mut bands = forward_level(&input, cw, ch);
        input = std::mem::take(&mut bands[0]);
        levels.push(bands);
        cw /= 2;
        ch /= 2;
    }
    let scale = if depth >= 12 { 4 } else { 1 };
    let fine = quality.fine() * scale;
    // quantizers per level (finest first) and band (horizontal, vertical, diagonal)
    let quants: [[u16; 3]; LEVELS] = [[fine, fine, fine + fine / 2], [4 * scale, 4 * scale, 2 * scale], [16, 16, 8]];
    let quants = if depth >= 12 { [quants[0], [16, 16, 8], [16, 16, 8]] } else { quants };

    let mut out = Out(Vec::new());
    // lowpass band (the coarsest level's band 0)
    out.tag(MARKER, MARKER_LOWPASS_HEADER);
    out.tag(LOWPASS_SUBBAND, 0);
    out.tag(NUM_LEVELS, LEVELS as u16);
    out.tag(LOWPASS_WIDTH, cw as u16);
    out.tag(LOWPASS_HEIGHT, ch as u16);
    out.tag(PIXEL_OFFSET, 0);
    out.tag(QUANTIZATION, 1);
    out.tag(PIXEL_DEPTH, 16);
    out.tag(MARKER, COEFFICIENT_START_CODE);
    for v in &input {
        out.0.extend_from_slice(&((*v).clamp(0, i32::from(i16::MAX)) as u16).to_be_bytes());
    }
    while !out.0.len().is_multiple_of(4) {
        out.0.push(0);
    }
    out.tag(MARKER, MARKER_LOWPASS_TRAILER);
    for k in (0..LEVELS).rev() {
        let (bw, bh) = (w >> (k + 1), coded_h >> (k + 1));
        out.tag(MARKER, MARKER_HIGHPASS_HEADER);
        out.tag(WAVELET_TYPE, if k == 0 { WAVELET_FINEST } else { WAVELET_SPATIAL });
        out.tag(WAVELET_NUMBER, k as u16 + 1);
        out.tag(WAVELET_LEVEL, k as u16 + 1);
        out.tag(NUM_BANDS, 4);
        out.tag(HIGHPASS_WIDTH, bw as u16);
        out.tag(HIGHPASS_HEIGHT, bh as u16);
        out.tag(LOWPASS_BORDER, 0);
        out.tag(HIGHPASS_BORDER, 0);
        out.tag(LOWPASS_SCALE, 1);
        out.tag(LOWPASS_DIVISOR, 1);
        let level = levels.get(k);
        for b in 1..=3usize {
            let coeffs = level.and_then(|l| l.get(b)).map(Vec::as_slice).unwrap_or(&[]);
            let base = quants.get(k).and_then(|q| q.get(b - 1)).copied().unwrap_or(16);
            // a coarser quantizer where the largest value would not fit the codebook
            let peak = coeffs.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
            let quant = base.max(peak.div_ceil(MAX_COMPANDED as u32).min(u32::from(u16::MAX)) as u16).max(1);
            out.tag(MARKER, MARKER_BAND_HEADER);
            out.tag(BAND_NUMBER, b as u16);
            out.tag(BAND_CODING_FLAGS, 1);
            out.tag(BAND_WIDTH, bw as u16);
            out.tag(BAND_HEIGHT, bh as u16);
            out.tag(BAND_SUBBAND, ((LEVELS - 1 - k) * 3 + b) as u16);
            out.tag(BAND_ENCODING, BAND_ENCODING_RUNLENGTHS);
            out.tag(BAND_QUANTIZATION, quant);
            out.tag(BAND_SCALE, 1);
            out.tag(BAND_HEADER, 0);
            out.0.extend_from_slice(&encode_band(coeffs, bw, bh, quant));
            out.tag(BAND_TRAILER, 0);
        }
        out.tag(MARKER, MARKER_HIGHPASS_TRAILER);
    }
    out.0
}

/// Codeword of each magnitude 0..=255 and the zero runs, longest first (codebook 17).
struct Codes {
    value: [(u8, u32); 256],
    runs: Vec<(usize, u8, u32)>,
    /// Best magnitude index of each companded value `0..=MAX_COMPANDED`.
    index: Vec<u8>,
}

fn codes() -> &'static Codes {
    static CODES: std::sync::OnceLock<Codes> = std::sync::OnceLock::new();
    CODES.get_or_init(|| {
        let mut value = [(0u8, 0u32); 256];
        let mut runs = Vec::new();
        for e in CODESET17.iter() {
            if e.count == 1 {
                if let Some(v) = value.get_mut(usize::from(e.value)) {
                    *v = (e.len, e.code);
                }
            } else if e.value == 0 {
                runs.push((usize::from(e.count), e.len, e.code));
            }
        }
        runs.sort_unstable_by_key(|r| std::cmp::Reverse(r.0));
        let curve: Vec<i32> = (0..256).map(|m| m + ((i64::from(m) * i64::from(m) * i64::from(m) * 768) >> 24) as i32).collect();
        let index = (0..=MAX_COMPANDED)
            .map(|t| {
                let k = curve.partition_point(|&c| c < t);
                // the nearer of the two neighbours
                let above = k.min(255);
                let below = k.saturating_sub(1);
                let pick =
                    if (curve.get(above).copied().unwrap_or(i32::MAX) - t).abs() < (t - curve.get(below).copied().unwrap_or(0)).abs() { above } else { below };
                pick as u8
            })
            .collect();
        Codes { value, runs, index }
    })
}

/// Run-length code one band (`w × h`, rows padded to a multiple of 8) with quantizer `quant`,
/// end codeword included, padded to 32 bits.
fn encode_band(coeffs: &[i32], w: usize, h: usize, quant: u16) -> Vec<u8> {
    let c = codes();
    let q = i32::from(quant.max(1));
    let stride = w.next_multiple_of(8);
    let mut bw = BitWriter::new();
    let mut zeros = 0usize;
    let flush = |bw: &mut BitWriter, zeros: &mut usize| {
        for &(count, len, code) in &c.runs {
            while *zeros >= count {
                bw.write_bits(code, u32::from(len));
                *zeros -= count;
            }
        }
        while *zeros > 0 {
            let (len, code) = c.value.first().copied().unwrap_or_default();
            bw.write_bits(code, u32::from(len));
            *zeros -= 1;
        }
    };
    for y in 0..h {
        for x in 0..stride {
            let v = if x < w { coeffs.get(y * w + x).copied().unwrap_or(0) } else { 0 };
            let t = ((v.abs() + q / 2) / q).min(MAX_COMPANDED);
            let m = c.index.get(t as usize).copied().unwrap_or(255);
            if m == 0 {
                zeros += 1;
                continue;
            }
            flush(&mut bw, &mut zeros);
            let (len, code) = c.value.get(usize::from(m)).copied().unwrap_or_default();
            bw.write_bits(code, u32::from(len));
            bw.write_bit(v < 0);
        }
    }
    flush(&mut bw, &mut zeros);
    bw.write_bits(CODESET17_BAND_END.1, u32::from(CODESET17_BAND_END.0));
    while !bw.bit_len().is_multiple_of(32) {
        bw.write_bit(false);
    }
    bw.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Flavor, decode_frame_as};

    /// A moving-free test picture: gradients, edges and texture, `bits` deep.
    fn picture(w: usize, h: usize, bits: u8, seed: u32) -> Vec<u16> {
        let max = (1u32 << bits) - 1;
        let mut s = seed;
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let mut v = (x * max as usize / w.max(1) + y * 7) as u32 % (max + 1);
                if (w / 4..w / 2).contains(&x) && (h / 4..h / 2).contains(&y) {
                    v = if (x / 3 + y / 3) % 2 == 0 { max } else { 0 };
                }
                (v + (s >> 28)).min(max) as u16
            })
            .collect()
    }

    fn psnr(a: &[u16], b: &[u16], bits: u8) -> f64 {
        let max = f64::from((1u32 << bits) - 1);
        let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
        let mse = se / a.len().max(1) as f64;
        if mse == 0.0 { 99.0 } else { 10.0 * (max * max / mse).log10() }
    }

    fn round_trip(format: EncodedFormat, w: u32, h: u32, quality: Quality) -> (Vec<f64>, usize) {
        let cfg = EncoderConfig { width: w, height: h, format, quality };
        let planes: Vec<Vec<u16>> = (0..format.channels())
            .map(|c| {
                let (pw, ph) = cfg.plane_size(c);
                picture(pw, ph, cfg.bit_depth(), c as u32)
            })
            .collect();
        let refs: Vec<&[u16]> = planes.iter().map(Vec::as_slice).collect();
        let data = encode_frame(&cfg, &refs).expect("encode");
        let f = decode_frame_as(&data, Flavor::Ffmpeg).expect("decode");
        assert_eq!((f.width, f.height, f.format), (w, h, format));
        let q = f.planes.iter().zip(&planes).map(|(p, src)| psnr(&p.data, src, cfg.bit_depth())).collect();
        (q, data.len())
    }

    #[test]
    fn frames_round_trip_through_the_decoder() {
        for format in [EncodedFormat::Yuv422, EncodedFormat::Rgb444, EncodedFormat::Rgba4444] {
            let mut last = f64::MAX;
            for quality in Quality::ALL {
                let (q, size) = round_trip(format, 96, 64, quality);
                let worst = q.iter().copied().fold(f64::MAX, f64::min);
                eprintln!("{format:?} {quality:?}: {q:.1?} dB, {size} bytes");
                assert!(worst > 30.0, "{format:?} {quality:?}: {q:?}");
                assert!(worst <= last + 0.5, "{format:?}: quality does not fall with the setting");
                last = worst;
            }
        }
    }

    #[test]
    fn odd_heights_and_tiny_frames() {
        for (w, h) in [(16, 1), (16, 7), (32, 486 / 9), (48, 9)] {
            let (q, _) = round_trip(EncodedFormat::Yuv422, w, h, Quality::FilmScan1);
            assert!(q.iter().all(|&v| v > 30.0), "{w}x{h}: {q:?}");
        }
        let (q, _) = round_trip(EncodedFormat::Rgb444, 8, 8, Quality::High);
        assert!(q.iter().all(|&v| v > 30.0), "{q:?}");
    }

    #[test]
    fn bad_configurations_are_errors() {
        let cfg = |width, height, format| EncoderConfig { width, height, format, quality: Quality::High };
        let p = vec![0u16; 64 * 64];
        assert!(encode_frame(&cfg(24, 16, EncodedFormat::Yuv422), &[&p, &p, &p]).is_err());
        assert!(encode_frame(&cfg(0, 16, EncodedFormat::Rgb444), &[&p, &p, &p]).is_err());
        assert!(encode_frame(&cfg(16, 16, EncodedFormat::Bayer), &[&p, &p, &p, &p]).is_err());
        assert!(encode_frame(&cfg(16, 16, EncodedFormat::Rgb444), &[&p, &p]).is_err());
        assert!(encode_frame(&cfg(64, 64, EncodedFormat::Rgb444), &[&p, &p, &p[..10]]).is_err());
        // out-of-range samples are clamped, not a panic
        let big = vec![u16::MAX; 16 * 16];
        assert!(encode_frame(&cfg(16, 16, EncodedFormat::Rgb444), &[&big, &big, &big]).is_ok());
    }

    #[test]
    fn the_transform_alone_is_lossless_at_the_finest_level() {
        let x: Vec<i32> = (0..12).map(|i| (i * 37 % 11) * 50).collect();
        let (mut l, mut h) = (vec![0; 6], vec![0; 6]);
        forward_1d(&x, &mut l, &mut h);
        let l16: Vec<i16> = l.iter().map(|&v| v as i16).collect();
        let h16: Vec<i16> = h.iter().map(|&v| v as i16).collect();
        let mut back = vec![0i32; 12];
        crate::wavelet::tests_support::horizontal_halve(&l16, &h16, &mut back);
        assert_eq!(back, x);
    }
}
