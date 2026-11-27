// Ported from FFmpeg n7.1.1 libavcodec/lagarith.c (Copyright (c) 2009 Nathan Caldwell) and
// libavcodec/lagarithrac.c / lagarithrac.h (Copyright (c) 2009 Nathan Caldwell, David Conrad),
// plus the median and left predictors of libavcodec/lossless_videodsp.c, LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **Lagarith** (`LAGS`), Ben Greenwood's lossless VfW video codec: the
//! range-coded RGB24, RGBA, YUY2 and YV12 frames, the uncompressed and zero-run planes and the
//! solid-colour frames. The output is the same as ffmpeg's `lagarith` decoder, byte for byte.
//!
//! [`Decoder::decode`] decodes one frame (one AVI chunk) to a [`Picture`].

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid Lagarith frame: {0}")]
    Invalid(&'static str),
    #[error("unsupported Lagarith frame type {0}")]
    Unsupported(u8),
    #[error("unsupported Lagarith frame size {0}x{1}")]
    Size(u32, u32),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The largest frame decoded, in pixels (8K UHD fits).
pub const MAX_PIXELS: u64 = 1 << 26;

/// The layout of a decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Planes R, G, B.
    Rgb,
    /// Planes R, G, B, A.
    Rgba,
    /// Planes Y, U, V; chroma half width (YUY2).
    Yuv422,
    /// Planes Y, U, V; chroma half width and half height (YV12).
    Yuv420,
}

/// A decoded frame: 8-bit planes, top row first, each `plane_width(i)` bytes wide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub planes: Vec<Vec<u8>>,
}

impl Picture {
    /// The width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (u32, u32) {
        let half = |v: u32| v.div_ceil(2);
        match (self.format, i) {
            (Format::Yuv422, 1 | 2) => (half(self.width), self.height),
            (Format::Yuv420, 1 | 2) => (half(self.width), half(self.height)),
            _ => (self.width, self.height),
        }
    }
}

// frame types
const FRAME_U_RGB24: u8 = 2;
const FRAME_ARITH_YUY2: u8 = 3;
const FRAME_ARITH_RGB24: u8 = 4;
const FRAME_SOLID_GRAY: u8 = 5;
const FRAME_SOLID_COLOR: u8 = 6;
const FRAME_ARITH_RGBA: u8 = 8;
const FRAME_SOLID_RGBA: u8 = 9;
const FRAME_ARITH_YV12: u8 = 10;

/// The prefix code of the probability header: (length, code, value).
pub(crate) const PROB_CODES: [(u8, u8, u8); 53] = {
    const BITS: [u8; 53] = [
        7, 7, 2, 7, 3, 4, 5, 6, 7, 7, 7, 7, 7, 6, 7, 4, 5, 7, 7, 7, 7, 5, 6, 7, 7, 7, 7, 7, 7, 6, 7, 7, 7, 7, 7, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
        7, 7, 7,
    ];
    const CODES: [u8; 53] = [
        0x01, 0x02, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x04, 0x05, 0x08, 0x09, 0x0A, 0x0B, 0x0B, 0x0B, 0x0B, 0x10, 0x11, 0x12, 0x13, 0x13, 0x13, 0x14, 0x15,
        0x20, 0x21, 0x22, 0x23, 0x23, 0x24, 0x25, 0x28, 0x29, 0x2A, 0x2B, 0x2B, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x48, 0x49, 0x4A, 0x4B, 0x50, 0x51, 0x52,
        0x53, 0x54, 0x55,
    ];
    const SYMBOLS: [u8; 53] = [
        20, 12, 0, 12, 1, 2, 4, 7, 7, 28, 4, 25, 17, 10, 17, 3, 6, 2, 23, 15, 15, 5, 9, 10, 31, 1, 22, 14, 14, 8, 9, 30, 6, 27, 19, 11, 19, 0, 21, 13, 13, 8,
        29, 5, 26, 18, 18, 3, 24, 16, 16, 11, 32,
    ];
    let mut t = [(0u8, 0u8, 0u8); 53];
    let mut i = 0;
    while i < 53 {
        t[i] = (BITS[i], CODES[i], SYMBOLS[i]);
        i += 1;
    }
    t
};

fn byte(d: &[u8], i: usize) -> u8 {
    d.get(i).copied().unwrap_or(0)
}

fn rl32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([byte(d, i), byte(d, i + 1), byte(d, i + 2), byte(d, i + 3)])
}

/// `av_log2`: the index of the highest set bit (0 for 0).
fn log2(v: u32) -> u32 {
    if v == 0 { 0 } else { 31 - v.leading_zeros() }
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// An MSB-first bit reader that reads zeros past the end (as FFmpeg's padded, checked reader).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn size(&self) -> usize {
        self.data.len().saturating_mul(8)
    }
    fn peek(&self, n: u32) -> u32 {
        let mut v = 0u32;
        for k in 0..n as usize {
            let p = self.pos + k;
            let bit = (byte(self.data, p / 8) >> (7 - (p % 8))) & 1;
            v = (v << 1) | u32::from(bit);
        }
        v
    }
    fn skip(&mut self, n: u32) {
        self.pos = (self.pos + n as usize).min(self.size() + 8);
    }
    fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n);
        v
    }
}

/// `lag_decode_prob`: one probability (or run) value of the header.
fn read_prob(gb: &mut Bits<'_>) -> Result<u32> {
    let peek = gb.peek(7);
    let found = PROB_CODES.iter().find(|&&(len, code, _)| peek >> (7 - u32::from(len)) == u32::from(code));
    let Some(&(len, _, bits)) = found else {
        return Err(Error::Invalid("bad probability code"));
    };
    gb.skip(u32::from(len));
    if bits > 31 {
        return Err(Error::Invalid("bad probability code"));
    }
    if bits == 0 {
        return Ok(0);
    }
    let val = gb.read(u32::from(bits)) | (1u32 << bits);
    Ok(val - 1)
}

/// The 52-bit mantissa of `1 / denom` (`softfloat_reciprocal`).
fn softfloat_reciprocal(denom: u32) -> u64 {
    let denom = u64::from(denom.max(1));
    let shift = log2((denom - 1) as u32) + 1;
    let mut ret = (1u64 << 52) / denom;
    let mut err = (1u64 << 52) - ret * denom;
    ret <<= shift;
    err <<= shift;
    err += denom / 2;
    ret + err / denom
}

/// `(u32)(x * f)` for the mantissa of `f` (`softfloat_mul`).
fn softfloat_mul(x: u32, mantissa: u64) -> u32 {
    let x = u64::from(x);
    let mut l = x.wrapping_mul(mantissa & 0xffff_ffff);
    let mut h = x.wrapping_mul(mantissa >> 32);
    h = h.wrapping_add(l >> 32);
    l &= 0xffff_ffff;
    l = l.wrapping_add(1u64 << log2((h >> 21) as u32));
    h = h.wrapping_add(l >> 32);
    (h >> 20) as u32
}

/// The range decoder of one plane.
struct Rac<'a> {
    data: &'a [u8],
    pos: isize,
    end: isize,
    low: u32,
    range: u32,
    scale: u32,
    hash_shift: u32,
    overread: u32,
    prob: [u32; 258],
    range_hash: [u8; 1024],
}

const MAX_OVERREAD: u32 = 4;

impl<'a> Rac<'a> {
    /// `lag_read_prob_header` + `ff_lag_rac_init`: the probabilities, then the coder over the
    /// rest of `data` (from the bit reader's next whole byte).
    fn new(data: &'a [u8]) -> Result<Rac<'a>> {
        let mut gb = Bits { data, pos: 0 };
        let mut prob = [0u32; 258];
        prob[257] = u32::MAX;
        let mut cumul: u32 = 0;
        let mut nnz = 0;
        let mut i = 1usize;
        while i < 257 {
            let p = read_prob(&mut gb)?;
            prob[i] = p;
            cumul = cumul.checked_add(p).ok_or(Error::Invalid("probability overflow"))?;
            if p == 0 {
                let run = read_prob(&mut gb)?.min(256 - i as u32);
                for _ in 0..run {
                    i += 1;
                    prob[i] = 0;
                }
            } else {
                nnz += 1;
            }
            i += 1;
        }
        if cumul == 0 {
            return Err(Error::Invalid("all probabilities are zero"));
        }
        if nnz == 1 && gb.peek(32) & 0x00FF_FFFF != 0 {
            return Err(Error::Invalid("single-symbol plane with data"));
        }
        let mut scale = log2(cumul);
        if cumul & (cumul - 1) != 0 {
            let mul = softfloat_reciprocal(cumul);
            let mut scaled: u32 = 0;
            for p in prob.iter_mut().take(129).skip(1) {
                *p = softfloat_mul(*p, mul);
                scaled = scaled.wrapping_add(*p);
            }
            if scaled == 0 {
                return Err(Error::Invalid("scaled probabilities invalid"));
            }
            for p in prob.iter_mut().take(257).skip(129) {
                *p = softfloat_mul(*p, mul);
                scaled = scaled.wrapping_add(*p);
            }
            scale += 1;
            if scale >= 32 {
                return Err(Error::Invalid("probability scale"));
            }
            let target = 1u32 << scale;
            if scaled > target {
                return Err(Error::Invalid("scaled probabilities too large"));
            }
            let mut rest = target - scaled;
            // spread the remainder over the first 128 non-zero symbols, round robin
            let mut k = 1usize;
            while rest > 0 {
                if prob[k] != 0 {
                    prob[k] += 1;
                    rest -= 1;
                }
                k = (k & 0x7f) + 1;
            }
        }
        if scale > 23 {
            return Err(Error::Invalid("probability scale"));
        }
        for k in 1..257 {
            prob[k] = prob[k].wrapping_add(prob[k - 1]);
        }

        // ff_lag_rac_init: from the next whole byte
        gb.skip(((8 - gb.pos % 8) % 8) as u32);
        let left = (gb.size() as isize - gb.pos as isize) >> 3;
        let start = (gb.pos / 8) as isize;
        let hash_shift = scale.max(10) - 10;
        let mut range_hash = [0u8; 1024];
        let mut j = 0usize;
        for (k, h) in range_hash.iter_mut().enumerate() {
            let r = (k as u32) << hash_shift;
            while j < 256 && prob[j + 1] <= r {
                j += 1;
            }
            *h = j as u8;
        }
        Ok(Rac {
            data,
            pos: start,
            end: start + left,
            low: u32::from(byte(data, start.max(0) as usize) >> 1),
            range: 0x80,
            scale,
            hash_shift,
            overread: 0,
            prob,
            range_hash,
        })
    }

    fn refill(&mut self) {
        while self.range <= 0x80_0000 {
            self.low <<= 8;
            self.range <<= 8;
            let p = self.pos.max(0) as usize;
            let w = (u32::from(byte(self.data, p)) << 8) | u32::from(byte(self.data, p + 1));
            self.low |= 0xff & (w >> 1);
            if self.pos < self.end {
                self.pos += 1;
            } else {
                self.overread = self.overread.saturating_add(1);
            }
        }
    }

    /// `lag_get_rac`: one byte.
    fn get(&mut self) -> u8 {
        self.refill();
        let rs = self.range >> self.scale;
        let p = &self.prob;
        let val: usize;
        if self.low < rs.wrapping_mul(p[255]) {
            if self.low < rs.wrapping_mul(p[1]) {
                val = 0;
            } else {
                let div = rs.checked_shl(self.hash_shift).unwrap_or(0);
                let idx = self.low.checked_div(div).unwrap_or(0) as usize;
                let mut v = usize::from(self.range_hash.get(idx).copied().unwrap_or(0));
                while v < 255 && self.low >= rs.wrapping_mul(p[v + 1]) {
                    v += 1;
                }
                val = v;
            }
            self.range = rs.wrapping_mul(p[val + 1].wrapping_sub(p[val]));
        } else {
            val = 255;
            self.range = self.range.wrapping_sub(rs.wrapping_mul(p[255]));
        }
        if self.range == 0 {
            self.range = 0x80;
        }
        self.low = self.low.wrapping_sub(rs.wrapping_mul(p[val]));
        val as u8
    }
}

/// How a plane's residuals turn into pixels.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pred {
    /// RGB planes (`lag_pred_line`, the first pixel of row 1 is top-predicted).
    Rgb,
    /// YV12 planes (`lag_pred_line`, row 1 starts with the true top-left).
    Yv12,
    /// YUY2 planes (`lag_pred_line_yuy2`); `true` for the luma plane.
    Yuy2(bool),
}

/// The decoder of one Lagarith stream.
#[derive(Clone, Debug)]
pub struct Decoder {
    width: u32,
    height: u32,
    bits_per_coded_sample: u16,
    zeros: u32,
    zeros_rem: u32,
}

impl Decoder {
    /// A decoder for `width` x `height` frames; `bits_per_coded_sample` is the
    /// `BITMAPINFOHEADER` bit count (24 or 32: it sets the layout of solid-colour frames).
    pub fn new(width: u32, height: u32, bits_per_coded_sample: u16) -> Result<Decoder> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Size(width, height));
        }
        Ok(Decoder { width, height, bits_per_coded_sample, zeros: 0, zeros_rem: 0 })
    }

    /// Decode one frame.
    pub fn decode(&mut self, buf: &[u8]) -> Result<Picture> {
        let (w, h) = (self.width, self.height);
        let n = w as usize * h as usize;
        let frametype = *buf.first().ok_or(Error::Invalid("empty frame"))?;
        let offset_gu = rl32(buf, 1);
        let offset_bv = rl32(buf, 5);
        let size = buf.len() as u64;
        let rgb24 = self.bits_per_coded_sample == 24;
        let solid = |vals: &[u8]| vals.iter().map(|&v| vec![v; n]).collect::<Vec<_>>();
        let picture = |format, planes| Picture { width: w, height: h, format, planes };
        match frametype {
            FRAME_SOLID_GRAY if rgb24 => Ok(picture(Format::Rgb, solid(&[byte(buf, 1); 3]))),
            FRAME_SOLID_GRAY => Ok(picture(Format::Rgba, solid(&[byte(buf, 1); 4]))),
            FRAME_SOLID_RGBA => Ok(picture(Format::Rgba, solid(&[byte(buf, 3), byte(buf, 2), byte(buf, 1), byte(buf, 4)]))),
            FRAME_SOLID_COLOR if rgb24 => Ok(picture(Format::Rgb, solid(&[byte(buf, 3), byte(buf, 2), byte(buf, 1)]))),
            FRAME_SOLID_COLOR => Ok(picture(Format::Rgba, solid(&[byte(buf, 3), byte(buf, 2), byte(buf, 1), 0xFF]))),
            FRAME_ARITH_RGB24 | FRAME_U_RGB24 | FRAME_ARITH_RGBA => {
                let rgba = frametype == FRAME_ARITH_RGBA;
                let offs: Vec<u32> = if rgba { vec![offset_bv, offset_gu, 13, rl32(buf, 9)] } else { vec![offset_bv, offset_gu, 9] };
                if offs.iter().any(|&o| size <= u64::from(o)) {
                    return Err(Error::Invalid("frame offsets"));
                }
                let mut planes = Vec::with_capacity(offs.len());
                for &o in &offs {
                    let mut p = vec![0u8; n];
                    self.plane(&mut p, w, h, buf.get(o as usize..).unwrap_or_default(), Pred::Rgb)?;
                    // coded bottom-up
                    let rows: Vec<&[u8]> = p.chunks(w as usize).rev().collect();
                    planes.push(rows.concat());
                }
                let g = planes.get(1).cloned().unwrap_or_default();
                for k in [0, 2] {
                    if let Some(p) = planes.get_mut(k) {
                        for (v, &gv) in p.iter_mut().zip(&g) {
                            *v = v.wrapping_add(gv);
                        }
                    }
                }
                // coded planes: R - G, G, B - G (, A)
                let mut it = planes.into_iter();
                let (r, g, b) = (it.next().unwrap_or_default(), it.next().unwrap_or_default(), it.next().unwrap_or_default());
                let mut out = vec![r, g, b];
                out.extend(it);
                Ok(picture(if rgba { Format::Rgba } else { Format::Rgb }, out))
            }
            FRAME_ARITH_YUY2 | FRAME_ARITH_YV12 => {
                if [9, offset_gu, offset_bv].iter().any(|&o| u64::from(o) >= size) {
                    return Err(Error::Invalid("frame offsets"));
                }
                let yv12 = frametype == FRAME_ARITH_YV12;
                let cw = w.div_ceil(2);
                let ch = if yv12 { h.div_ceil(2) } else { h };
                let cn = cw as usize * ch as usize;
                let at = |o: u32| buf.get(o as usize..).unwrap_or_default();
                let (pred_y, pred_c) = if yv12 { (Pred::Yv12, Pred::Yv12) } else { (Pred::Yuy2(true), Pred::Yuy2(cw == w)) };
                let mut y = vec![0u8; n];
                self.plane(&mut y, w, h, at(9), pred_y)?;
                // YUY2 codes U then V; YV12 codes V then U
                let mut first = vec![0u8; cn];
                self.plane(&mut first, cw, ch, at(offset_gu), pred_c)?;
                let mut second = vec![0u8; cn];
                self.plane(&mut second, cw, ch, at(offset_bv), pred_c)?;
                let (u, v) = if yv12 { (second, first) } else { (first, second) };
                Ok(picture(if yv12 { Format::Yuv420 } else { Format::Yuv422 }, vec![y, u, v]))
            }
            t => Err(Error::Unsupported(t)),
        }
    }

    /// `lag_decode_arith_plane`: one plane of `width` x `height` from `src`, in coding order.
    fn plane(&mut self, dst: &mut [u8], width: u32, height: u32, src: &[u8], pred: Pred) -> Result<()> {
        let (wu, hu) = (width as usize, height as usize);
        self.zeros = 0;
        if src.len() < 2 {
            return Err(Error::Invalid("plane too short"));
        }
        let esc = src[0];
        if esc < 4 {
            if src.len() < 5 {
                return Err(Error::Invalid("plane too short"));
            }
            let length = (wu * hu) as u64;
            let offset = if esc != 0 && u64::from(rl32(src, 1)) < length { 5 } else { 1 };
            let mut rac = Rac::new(src.get(offset..).unwrap_or_default())?;
            let esc_count = (esc != 0).then_some(u32::from(esc));
            for row in dst.chunks_mut(wu).take(hu) {
                if rac.overread > MAX_OVERREAD {
                    return Err(Error::Invalid("range coder overread"));
                }
                self.line(&mut rac, row, esc_count);
            }
        } else if esc < 8 {
            let esc = usize::from(esc - 4);
            let src = &src[1..];
            if esc > 0 {
                let mut pos = 0usize;
                for row in dst.chunks_mut(wu).take(hu) {
                    pos += self.zero_run_line(row, src.get(pos..).unwrap_or_default(), esc)?;
                }
            } else {
                if src.len() < wu * hu {
                    return Err(Error::Invalid("uncompressed plane too short"));
                }
                dst.copy_from_slice(&src[..wu * hu]);
            }
        } else if esc == 0xff {
            dst.fill(src[1]);
            return Ok(());
        } else {
            return Err(Error::Invalid("zero-run escape code"));
        }
        for r in 0..hu {
            match pred {
                Pred::Yuy2(luma) => pred_line_yuy2(dst, wu, r, luma),
                _ => pred_line(dst, wu, r, pred == Pred::Yv12),
            }
        }
        Ok(())
    }

    /// `lag_decode_line`: one row of range-coded residuals with the zero-run escape.
    fn line(&mut self, rac: &mut Rac<'_>, dst: &mut [u8], esc_count: Option<u32>) {
        let width = dst.len();
        let mut i = 0usize;
        loop {
            if self.zeros_rem > 0 {
                let count = (self.zeros_rem as usize).min(width - i);
                dst[i..i + count].fill(0);
                i += count;
                self.zeros_rem -= count as u32;
            }
            let mut escaped = false;
            while i < width {
                let v = rac.get();
                dst[i] = v;
                self.zeros = if v != 0 { 0 } else { self.zeros.saturating_add(1) };
                i += 1;
                if Some(self.zeros) == esc_count {
                    let index = rac.get() as i8;
                    self.zeros = 0;
                    self.zeros_rem = u32::from(((i32::from(index) * 2) ^ (i32::from(index) >> 7)) as u8);
                    escaped = true;
                    break;
                }
            }
            if !escaped {
                return;
            }
        }
    }

    /// `lag_decode_zero_run_line`: one row of a zero-run-only plane; returns the bytes read.
    fn zero_run_line(&mut self, dst: &mut [u8], src: &[u8], esc: usize) -> Result<usize> {
        let width = dst.len() as isize;
        let mask1: u8 = if esc < 2 { 0xff } else { 0 };
        let mask2: u8 = if esc < 3 { 0xff } else { 0 };
        let end = width - 2;
        let mut d: isize = 0;
        let mut s: usize = 0;
        let mut i: isize = 0;
        let mut zero_run = false;
        dst.fill(0);
        let src_at = |k: usize| byte(src, k);
        'output_zeros: loop {
            if self.zeros_rem > 0 {
                let count = (self.zeros_rem as isize).min(width - i);
                if end - d < count {
                    return Err(Error::Invalid("too many zeros remaining"));
                }
                // dst is already zero
                self.zeros_rem -= count.max(0) as u32;
                d += count.max(0);
            }
            while d < end {
                i = 0;
                while !zero_run && d + i < end {
                    i += 1;
                    let rem = src.len().saturating_sub(s) as isize;
                    if i + 2 >= rem {
                        return Err(Error::Invalid("zero-run plane too short"));
                    }
                    let k = s + i as usize;
                    zero_run = (src_at(k) | (src_at(k + 1) & mask1) | (src_at(k + 2) & mask2)) == 0;
                }
                let rem = src.len().saturating_sub(s) as isize;
                if zero_run {
                    zero_run = false;
                    i += esc as isize;
                    if i > end - d || i >= rem {
                        return Err(Error::Invalid("zero run past the row"));
                    }
                    let (du, iu) = (d as usize, i as usize);
                    dst[du..du + iu].copy_from_slice(&src[s..s + iu]);
                    d += i;
                    let x = src_at(s + iu) as i8;
                    self.zeros_rem = u32::from(((i32::from(x) * 2) ^ (i32::from(x) >> 7)) as u8);
                    s += iu + 1;
                    continue 'output_zeros;
                }
                let (du, iu) = (d as usize, i as usize);
                if s + iu > src.len() || du + iu > dst.len() {
                    return Err(Error::Invalid("zero-run plane too short"));
                }
                dst[du..du + iu].copy_from_slice(&src[s..s + iu]);
                s += iu;
                d += i;
            }
            return Ok(s);
        }
    }
}

/// `lag_pred_line`: row `r` of a plane `w` wide.
fn pred_line(p: &mut [u8], w: usize, r: usize, yv12: bool) {
    let row = r * w;
    if r == 0 {
        let mut acc = 0u8;
        for v in &mut p[..w] {
            acc = acc.wrapping_add(*v);
            *v = acc;
        }
        return;
    }
    let prev = row - w;
    let left = p[prev + w - 1];
    let top_left = if r == 1 { if yv12 { p[prev] } else { left } } else { p[prev - w + w - 1] };
    let (mut l, mut lt) = (left, top_left);
    for i in 0..w {
        let t = p[prev + i];
        let pred = mid_pred(i32::from(l), i32::from(t), i32::from(l) + i32::from(t) - i32::from(lt));
        l = (pred + i32::from(p[row + i])) as u8;
        lt = t;
        p[row + i] = l;
    }
}

/// `lag_pred_line_yuy2`: row `r` of a YUY2 plane `w` wide.
fn pred_line_yuy2(p: &mut [u8], w: usize, r: usize, luma: bool) {
    let row = r * w;
    if r == 0 {
        let first = p[0];
        if luma {
            p[0] = 0;
        }
        let mut acc = 0u8;
        for v in &mut p[..w] {
            acc = acc.wrapping_add(*v);
            *v = acc;
        }
        if luma {
            p[0] = first;
        }
        return;
    }
    let prev = row - w;
    if r == 1 {
        let head = if luma { 4 } else { 2 };
        let mut l = i32::from(p[prev + w - 1]);
        let mut tl = i32::from(p[prev + head.min(w) - 1]);
        let mut i = 0;
        while i < head.min(w) {
            l += i32::from(p[row + i]);
            p[row + i] = l as u8;
            i += 1;
        }
        while i < w {
            let t = i32::from(p[prev + i]);
            l = mid_pred(l & 0xff, t, (l + t - tl) & 0xff) + i32::from(p[row + i]);
            tl = t;
            p[row + i] = l as u8;
            i += 1;
        }
        return;
    }
    // add_median_pred (the gradient wraps to 8 bits)
    let (mut l, mut lt) = (p[prev + w - 1], p[prev - w + w - 1]);
    for i in 0..w {
        let t = p[prev + i];
        let pred = mid_pred(i32::from(l), i32::from(t), (i32::from(l) + i32::from(t) - i32::from(lt)) & 0xff);
        l = (pred + i32::from(p[row + i])) as u8;
        lt = t;
        p[row + i] = l;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_code_is_prefix_free() {
        for (a, &(la, ca, _)) in PROB_CODES.iter().enumerate() {
            for (b, &(lb, cb, _)) in PROB_CODES.iter().enumerate() {
                if a != b && la <= lb {
                    assert_ne!(u32::from(cb) >> (lb - la), u32::from(ca), "{a} is a prefix of {b}");
                }
            }
        }
    }

    #[test]
    fn softfloat_matches_division() {
        for d in [3u32, 5, 7, 100, 1000, 65_537, 1 << 20 | 1] {
            let m = softfloat_reciprocal(d);
            for x in [0u32, 1, 2, d / 2, d - 1] {
                let q = softfloat_mul(x, m);
                let exact = (u64::from(x) << 20) / u64::from(d);
                // x / d scaled to the next power of two, to within one
                let want = (u64::from(x) << (log2(d - 1) + 1)) / u64::from(d);
                assert!(u64::from(q).abs_diff(want) <= 1, "{x}/{d}: {q} vs {want} ({exact})");
            }
        }
    }

    #[test]
    fn solid_frames() {
        let mut d = Decoder::new(3, 2, 24).unwrap();
        let p = d.decode(&[FRAME_SOLID_COLOR, 10, 20, 30]).unwrap();
        assert_eq!(p.format, Format::Rgb);
        assert_eq!(p.planes, vec![vec![30; 6], vec![20; 6], vec![10; 6]]);
        let mut d = Decoder::new(3, 2, 32).unwrap();
        let p = d.decode(&[FRAME_SOLID_RGBA, 10, 20, 30, 40]).unwrap();
        assert_eq!(p.planes, vec![vec![30; 6], vec![20; 6], vec![10; 6], vec![40; 6]]);
        let p = d.decode(&[FRAME_SOLID_GRAY, 7]).unwrap();
        assert_eq!(p.planes, vec![vec![7; 6]; 4]);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(Decoder::new(0, 4, 24).is_err());
        let mut d = Decoder::new(4, 4, 24).unwrap();
        assert!(d.decode(&[]).is_err());
        assert_eq!(d.decode(&[1]), Err(Error::Unsupported(1)));
        assert!(d.decode(&[FRAME_ARITH_RGB24, 0xff, 0xff, 0, 0, 0xff, 0xff, 0, 0]).is_err());
    }
}
