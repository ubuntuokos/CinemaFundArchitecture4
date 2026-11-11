//! Clean-room pure-Rust Ut Video decoder.
//!
//! Ut Video has no formal specification. The format facts come from the public MultimediaWiki
//! `Ut_Video` page (extradata fields, frame layout, slices, predictor field, the "longest code is
//! all zeros" code construction); the details it leaves open (the bit order, the order of
//! symbols of equal code length, the edges of the median predictor, the RGB plane order and
//! offset) were established by black-box analysis of files that ffmpeg writes, which is used only
//! as an external fixture generator and test oracle. No decoder source code was read.
//!
//! Supported: the 8-bit `ULY0` / `ULH0` (4:2:0), `ULY2` / `ULH2` (4:2:2), `ULY4` / `ULH4`
//! (4:4:4), `ULRG` (RGB) and `ULRA` (RGBA) formats with no, left, gradient or median prediction
//! and any number of slices, progressive. Interlaced streams, the 10-bit `UQ..` and the `UM..`
//! formats are rejected with [`Error::Unsupported`].
//!
//! ```no_run
//! let extra: Vec<u8> = Vec::new(); // BITMAPINFOHEADER bytes after the first 40
//! let mut dec = cfa4_utvideo::Decoder::new(*b"ULY0", 1920, 1080, &extra).unwrap();
//! let frame: Vec<u8> = Vec::new(); // one AVI chunk
//! let pic = dec.decode(&frame).unwrap();
//! println!("{}x{} {:?}", pic.width, pic.height, pic.format);
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod error;
mod huffman;

pub use error::{Error, Result};

use error::ensure;
use huffman::{BitReader, Table};

/// Largest accepted frame side (samples), so allocations stay bounded.
const MAX_SIDE: u32 = 16384;

/// How the planes of a [`Picture`] are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Y, U, V; chroma planes are `width >> log2_h` x `height >> log2_v`.
    Yuv { log2_h: u32, log2_v: u32 },
    /// R, G, B, all full size.
    Rgb,
    /// R, G, B, A, all full size.
    Rgba,
}

/// The YCbCr matrix a FourCC names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    /// `ULY.`
    Bt601,
    /// `ULH.`
    Bt709,
}

/// A decoded frame: 8-bit planes, top row first, stride = plane width.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    /// For YCbCr formats.
    pub matrix: Matrix,
    pub planes: Vec<Vec<u8>>,
}

/// A Ut Video stream: its header decoded once, then one [`Decoder::decode`] per frame (every
/// frame is a key frame).
#[derive(Debug)]
pub struct Decoder {
    width: u32,
    height: u32,
    format: Format,
    matrix: Matrix,
    slices: usize,
}

impl Decoder {
    /// `fourcc` is the `BITMAPINFOHEADER` compression, `extra` the header's bytes after the
    /// standard 40 (the Ut Video extradata).
    pub fn new(fourcc: [u8; 4], width: u32, height: u32, extra: &[u8]) -> Result<Decoder> {
        ensure!(width > 0 && height > 0 && width <= MAX_SIDE && height <= MAX_SIDE, "frame size {width}x{height}");
        let (format, matrix) = match &fourcc {
            b"ULY0" => (Format::Yuv { log2_h: 1, log2_v: 1 }, Matrix::Bt601),
            b"ULH0" => (Format::Yuv { log2_h: 1, log2_v: 1 }, Matrix::Bt709),
            b"ULY2" => (Format::Yuv { log2_h: 1, log2_v: 0 }, Matrix::Bt601),
            b"ULH2" => (Format::Yuv { log2_h: 1, log2_v: 0 }, Matrix::Bt709),
            b"ULY4" => (Format::Yuv { log2_h: 0, log2_v: 0 }, Matrix::Bt601),
            b"ULH4" => (Format::Yuv { log2_h: 0, log2_v: 0 }, Matrix::Bt709),
            b"ULRG" => (Format::Rgb, Matrix::Bt601),
            b"ULRA" => (Format::Rgba, Matrix::Bt601),
            f => return Err(Error::Unsupported(format!("Ut Video format {}", String::from_utf8_lossy(f)))),
        };
        if let Format::Yuv { log2_h, log2_v } = format {
            ensure!(width.is_multiple_of(1 << log2_h) && height.is_multiple_of(1 << log2_v), "frame size {width}x{height} for chroma subsampling");
        }
        // 32-bit little-endian words: encoder version, source format, frame info size, flags
        let word = |i: usize| extra.get(4 * i..4 * i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let info_size = word(2).ok_or_else(|| Error::Invalid(format!("Ut Video extradata of {} bytes", extra.len())))?;
        let flags = word(3).ok_or_else(|| Error::Invalid(format!("Ut Video extradata of {} bytes", extra.len())))?;
        ensure!(info_size == 4, "frame information size {info_size}");
        if flags & 1 == 0 {
            return Err(Error::Unsupported("Ut Video without Huffman compression".into()));
        }
        if flags & 0x800 != 0 {
            return Err(Error::Unsupported("interlaced Ut Video".into()));
        }
        let slices = (flags >> 24) as usize + 1;
        Ok(Decoder { width, height, format, matrix, slices })
    }

    /// The layout of the pictures this stream decodes to.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The (width, height, slice row step) of every coded plane, in coding order (Y U V, or
    /// G B R A). Slice boundaries fall on whole chroma rows: in luma rows, multiples of the
    /// vertical subsampling.
    fn plane_sizes(&self) -> Vec<(usize, usize, usize)> {
        let (w, h) = (self.width as usize, self.height as usize);
        match self.format {
            Format::Yuv { log2_h, log2_v } => {
                let c = (w >> log2_h, h >> log2_v, 1);
                vec![(w, h, 1 << log2_v), c, c]
            }
            Format::Rgb => vec![(w, h, 1); 3],
            Format::Rgba => vec![(w, h, 1); 4],
        }
    }

    /// Decode one frame (one AVI chunk).
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture> {
        ensure!(data.len() >= 4, "frame of {} bytes", data.len());
        let info = data.get(data.len() - 4..).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let predictor = (info >> 8) & 3;
        let body = data.get(..data.len() - 4).unwrap_or_default();
        let mut pos = 0usize;
        let mut planes = Vec::new();
        for (pw, ph, step) in self.plane_sizes() {
            let lens: [u8; 256] = body.get(pos..pos + 256).and_then(|b| b.try_into().ok()).ok_or_else(|| Error::Invalid("plane header truncated".into()))?;
            pos += 256;
            let offsets_len = 4 * self.slices;
            let offsets = body.get(pos..pos + offsets_len).ok_or_else(|| Error::Invalid("slice offsets truncated".into()))?;
            pos += offsets_len;
            let ends: Vec<usize> = offsets.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).collect();
            let total = ends.last().copied().unwrap_or(0);
            let slice_data = body.get(pos..pos.saturating_add(total)).ok_or_else(|| Error::Invalid("slice data truncated".into()))?;
            pos += total;
            let mut plane = vec![0u8; pw * ph];
            decode_plane(&mut plane, pw, ph, step, &lens, &ends, slice_data, predictor)?;
            planes.push(plane);
        }
        let planes = match self.format {
            Format::Yuv { .. } => planes,
            Format::Rgb | Format::Rgba => {
                // G, B - G + 0x80, R - G + 0x80 (, A) to R, G, B (, A)
                let mut it = planes.into_iter();
                let (g, mut b, mut r) = (it.next().unwrap_or_default(), it.next().unwrap_or_default(), it.next().unwrap_or_default());
                for ((bv, rv), &gv) in b.iter_mut().zip(r.iter_mut()).zip(&g) {
                    *bv = bv.wrapping_add(gv).wrapping_sub(0x80);
                    *rv = rv.wrapping_add(gv).wrapping_sub(0x80);
                }
                let mut out = vec![r, g, b];
                out.extend(it);
                out
            }
        };
        Ok(Picture { width: self.width, height: self.height, format: self.format, matrix: self.matrix, planes })
    }
}

#[inline]
fn median(a: u8, b: u8, c: u8) -> u8 {
    a.max(b).min(a.min(b).max(c))
}

/// One plane: `ends` are the slices' end offsets in `data`; slice `k` holds rows
/// `ph * k / n .. ph * (k + 1) / n`, counted in units of `step` rows.
#[allow(clippy::too_many_arguments)]
fn decode_plane(plane: &mut [u8], pw: usize, ph: usize, step: usize, lens: &[u8; 256], ends: &[usize], data: &[u8], predictor: u32) -> Result<()> {
    // a single symbol of length 0 fills the plane without any bits
    let single = lens.iter().position(|&l| l == 0);
    let table = if single.is_none() { Some(Table::new(lens)?) } else { None };
    let n = ends.len();
    let mut start = 0usize;
    for (k, &end) in ends.iter().enumerate() {
        ensure!(end >= start && end <= data.len(), "slice {k} offsets");
        let units = ph / step;
        let (y0, y1) = (units * k / n * step, if k + 1 == n { ph } else { units * (k + 1) / n * step });
        let rows = plane.get_mut(y0 * pw..y1 * pw).ok_or_else(|| Error::Invalid("slice rows".into()))?;
        match (single, &table) {
            (Some(s), _) => rows.fill(s as u8),
            (None, Some(t)) => {
                let mut br = BitReader::new(data.get(start..end).unwrap_or_default());
                for v in rows.iter_mut() {
                    *v = t.decode(&mut br);
                }
                ensure!(!br.overrun(), "slice {k} data ends early");
            }
            (None, None) => {}
        }
        restore(rows, pw, predictor);
        start = end;
    }
    Ok(())
}

/// Undo the prediction of one slice in place (`s` holds the residuals, row by row).
fn restore(s: &mut [u8], w: usize, predictor: u32) {
    match predictor {
        // left: one running sum over the whole slice, starting from 0x80
        1 => {
            let mut prev = 0x80u8;
            for v in s.iter_mut() {
                prev = prev.wrapping_add(*v);
                *v = prev;
            }
        }
        // gradient: the first row left; then left + up - up-left, continuous over row ends
        2 | 3 => {
            let mut prev = 0x80u8;
            for v in s.iter_mut().take(w) {
                prev = prev.wrapping_add(*v);
                *v = prev;
            }
            for i in w..s.len() {
                let up = s.get(i - w).copied().unwrap_or(0);
                let left = s.get(i - 1).copied().unwrap_or(0);
                let p = if i == w {
                    // the first sample of the second row: the one above
                    up
                } else {
                    let up_left = s.get(i - w - 1).copied().unwrap_or(0);
                    let grad = left.wrapping_add(up).wrapping_sub(up_left);
                    if predictor == 3 { median(left, up, grad) } else { grad }
                };
                if let Some(v) = s.get_mut(i) {
                    *v = v.wrapping_add(p);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(slices: u32, flags: u32) -> Vec<u8> {
        [0x0100_00F0u32, 0, 4, flags | ((slices - 1) << 24)].iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn median_of_three() {
        for (a, b, c, m) in [(1, 2, 3, 2), (3, 2, 1, 2), (2, 3, 1, 2), (5, 5, 1, 5), (0, 255, 128, 128)] {
            assert_eq!(median(a, b, c), m);
        }
    }

    #[test]
    fn single_symbol_planes_need_no_bits() {
        // 4x2 RGB, every plane one symbol: G = 10, B - G = 0x80 + 5, R - G = 0x80 - 3
        let mut d = Vec::new();
        for s in [10u8, 0x85, 0x7D] {
            let mut lens = [255u8; 256];
            lens[usize::from(s)] = 0;
            d.extend_from_slice(&lens);
            d.extend_from_slice(&0u32.to_le_bytes());
        }
        d.extend_from_slice(&0u32.to_le_bytes()); // no prediction
        let mut dec = Decoder::new(*b"ULRG", 4, 2, &extra(1, 1)).unwrap();
        let p = dec.decode(&d).unwrap();
        assert_eq!((p.planes[0][0], p.planes[1][7], p.planes[2][3]), (7, 10, 15));
    }

    #[test]
    fn hostile_input_is_an_error() {
        assert!(Decoder::new(*b"ULY0", 64, 32, &[]).is_err());
        assert!(Decoder::new(*b"ULY0", 63, 32, &extra(1, 1)).is_err());
        assert!(matches!(Decoder::new(*b"ULY0", 64, 32, &extra(1, 0x801)), Err(Error::Unsupported(_))));
        assert!(matches!(Decoder::new(*b"UQY2", 64, 32, &extra(1, 1)), Err(Error::Unsupported(_))));
        let mut dec = Decoder::new(*b"ULY4", 8, 8, &extra(256, 1)).unwrap();
        for n in 0..1200usize {
            let d: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(29) ^ (n as u8)).collect();
            let _ = dec.decode(&d);
        }
    }
}
