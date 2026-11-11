//! Clean-room pure-Rust Huffyuv (`HFYU`) decoder.
//!
//! Huffyuv has no formal specification. The format facts come from Roberto Togni's public
//! description of the bitstream (multimedia.cx `huffyuv.txt`, also the MultimediaWiki `HuffYUV`
//! page); the details it leaves open (the order codes are assigned from the stored code lengths,
//! the 32-bit word order of the bit stream, the raw first pixel, the edges of the gradient and
//! median predictors, the interlace flag) were established by black-box analysis of files that
//! ffmpeg writes, which is used only as an external fixture generator and test oracle. No
//! decoder source code was read.
//!
//! Supported: YUY2 (4:2:2, 16 bits per pixel), RGB24 and RGBA (32 bits per pixel) with the
//! left, gradient and median predictors, RGB with or without green decorrelation, progressive
//! and interlaced frames, Huffman tables stored in the stream header (Huffyuv 2.x and every
//! file ffmpeg writes). Huffyuv 1.x files that rely on the codec's built-in "classic" tables are
//! rejected with [`Error::Unsupported`]; so is FFmpeg's own `FFVH` variant.
//!
//! ```no_run
//! let extra: Vec<u8> = Vec::new(); // BITMAPINFOHEADER bytes after the first 40
//! let mut dec = cfa4_huffyuv::Decoder::new(720, 576, 16, &extra).unwrap();
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
    /// Y, U, V; U and V are `width / 2` x `height`.
    Yuv422,
    /// R, G, B, all full size.
    Rgb,
    /// R, G, B, A, all full size.
    Rgba,
}

/// A decoded frame: 8-bit planes, top row first, stride = plane width.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub planes: Vec<Vec<u8>>,
}

/// The prediction method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Predictor {
    Left,
    Gradient,
    Median,
}

/// A Huffyuv stream: its header decoded once, then one [`Decoder::decode`] per frame (every
/// frame is a key frame).
#[derive(Debug)]
pub struct Decoder {
    width: u32,
    height: u32,
    format: Format,
    predictor: Predictor,
    /// RGB: green is coded as is, blue and red as differences from it.
    decorrelate: bool,
    interlaced: bool,
    tables: [Table; 3],
}

/// Section "Huffman tables" of the description: run-length coded code lengths. Each byte holds a
/// length in its low 5 bits and a repeat count in its high 3 bits; a count of 0 means the count
/// is the next byte.
fn read_lengths(data: &[u8], pos: &mut usize) -> Result<[u8; 256]> {
    let mut lens = [0u8; 256];
    let mut n = 0usize;
    while n < 256 {
        let b = *data.get(*pos).ok_or_else(|| Error::Invalid("Huffman table truncated".into()))?;
        *pos += 1;
        let len = b & 31;
        let mut count = usize::from(b >> 5);
        if count == 0 {
            count = usize::from(*data.get(*pos).ok_or_else(|| Error::Invalid("Huffman table truncated".into()))?);
            *pos += 1;
        }
        ensure!(count <= 256 - n, "Huffman table run past 256 entries");
        if let Some(run) = lens.get_mut(n..n + count) {
            run.fill(len);
        }
        n += count;
    }
    Ok(lens)
}

impl Decoder {
    /// `bit_count` is the `BITMAPINFOHEADER` `biBitCount`, `extra` the header's bytes after the
    /// standard 40 (the Huffyuv extradata).
    pub fn new(width: u32, height: u32, bit_count: u16, extra: &[u8]) -> Result<Decoder> {
        ensure!(width > 0 && height > 0 && width <= MAX_SIDE && height <= MAX_SIDE, "frame size {width}x{height}");
        let (method, bpp, interlace_flag, tables_at) = if bit_count & 7 == 0 {
            // 2.x: method, bits per pixel, flags, reserved, then the tables
            let method = *extra.first().ok_or_else(|| Error::Unsupported("Huffyuv 1.x classic tables".into()))?;
            let bpp = extra.get(1).copied().filter(|&b| b != 0).map_or(bit_count, u16::from);
            (method, bpp, extra.get(2).copied().unwrap_or(0), 4)
        } else {
            // the low bits of biBitCount select the predictor; any tables follow the header
            let method = match bit_count & 7 {
                1 | 2 => 0,
                3 => 1,
                4 => 2,
                m => return Err(Error::Unsupported(format!("Huffyuv predictor code {m}"))),
            } | if bit_count & 7 == 2 || bit_count & 7 == 3 { 0x40 } else { 0 };
            (method, bit_count & !7, 0, 0)
        };
        ensure!(extra.len() > tables_at, "Huffyuv 1.x classic tables");
        let (predictor, decorrelate) = match method {
            0 | 0xFE => (Predictor::Left, false),
            1 => (Predictor::Gradient, false),
            2 => (Predictor::Median, false),
            0x40 => (Predictor::Left, true),
            0x41 => (Predictor::Gradient, true),
            0x42 => (Predictor::Median, true),
            m => return Err(Error::Unsupported(format!("Huffyuv prediction method {m:#x}"))),
        };
        let format = match bpp {
            16 => Format::Yuv422,
            24 => Format::Rgb,
            32 => Format::Rgba,
            b => return Err(Error::Unsupported(format!("Huffyuv with {b} bits per pixel"))),
        };
        if format == Format::Yuv422 {
            ensure!(width.is_multiple_of(2), "YUY2 frame width {width} is odd");
            ensure!(!decorrelate, "decorrelation with YUY2");
        }
        // flags: 0x10 interlaced, 0x20 progressive; neither: the 1.x rule (taller than 288)
        let interlaced = match interlace_flag & 0x30 {
            0x10 => true,
            0x20 => false,
            _ => height > 288,
        } && height.is_multiple_of(2);
        let mut pos = tables_at;
        let tables = [Table::new(&read_lengths(extra, &mut pos)?)?, Table::new(&read_lengths(extra, &mut pos)?)?, Table::new(&read_lengths(extra, &mut pos)?)?];
        Ok(Decoder { width, height, format, predictor, decorrelate, interlaced, tables })
    }

    /// The layout of the pictures this stream decodes to.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Decode one frame (one AVI chunk).
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture> {
        let mut br = BitReader::new(data);
        let planes = match self.format {
            Format::Yuv422 => self.decode_yuv(&mut br)?,
            Format::Rgb | Format::Rgba => self.decode_rgb(&mut br)?,
        };
        ensure!(!br.overrun(), "frame data ends early");
        Ok(Picture { width: self.width, height: self.height, format: self.format, planes })
    }

    /// The predicted value of sample `i` of a plane `w` wide (the picture as the coder sees it:
    /// an interlaced frame is twice as wide and half as tall, its two fields side by side).
    /// `first_left`: on the second row, samples before it use the left predictor (median).
    #[inline]
    fn predict(&self, p: &[u8], i: usize, w: usize, first_left: usize) -> u8 {
        let left = i.checked_sub(1).and_then(|k| p.get(k)).copied().unwrap_or(0);
        if i < w || self.predictor == Predictor::Left {
            return left;
        }
        let up = p.get(i - w).copied().unwrap_or(0);
        let up_left = i.checked_sub(w + 1).and_then(|k| p.get(k)).copied().unwrap_or(0);
        let grad = left.wrapping_add(up).wrapping_sub(up_left);
        match self.predictor {
            Predictor::Median if i >= w + first_left => median(left, up, grad),
            Predictor::Median | Predictor::Left => left,
            Predictor::Gradient => grad,
        }
    }

    /// The coded picture size: (width, height) with the fields side by side when interlaced.
    fn coded_size(&self, w: u32) -> (usize, usize) {
        let k = if self.interlaced { 2 } else { 1 };
        (w as usize * k, self.height as usize / k)
    }

    fn decode_yuv(&self, br: &mut BitReader) -> Result<Vec<Vec<u8>>> {
        let (w, h) = self.coded_size(self.width);
        let cw = w / 2;
        let mut y = vec![0u8; w * h];
        let mut u = vec![0u8; cw * h];
        let mut v = vec![0u8; cw * h];
        // the first pair is raw, read in the stream's word order as V Y1 U Y0
        let (rv, ry1, ru, ry0) = (br.bits(8) as u8, br.bits(8) as u8, br.bits(8) as u8, br.bits(8) as u8);
        if let Some(s) = y.get_mut(..2) {
            s.copy_from_slice(&[ry0, ry1]);
        }
        for (p, val) in [(&mut u, ru), (&mut v, rv)] {
            if let Some(s) = p.first_mut() {
                *s = val;
            }
        }
        let [ty, tu, tv] = &self.tables;
        for k in 1..cw * h {
            let (dy0, du, dy1, dv) = (ty.decode(br), tu.decode(br), ty.decode(br), tv.decode(br));
            if br.overrun() {
                return Err(Error::Invalid("frame data ends early".into()));
            }
            let i = 2 * k;
            let p = self.predict(&y, i, w, 4).wrapping_add(dy0);
            if let Some(s) = y.get_mut(i) {
                *s = p;
            }
            let p = self.predict(&y, i + 1, w, 4).wrapping_add(dy1);
            if let Some(s) = y.get_mut(i + 1) {
                *s = p;
            }
            let p = self.predict(&u, k, cw, 2).wrapping_add(du);
            if let Some(s) = u.get_mut(k) {
                *s = p;
            }
            let p = self.predict(&v, k, cw, 2).wrapping_add(dv);
            if let Some(s) = v.get_mut(k) {
                *s = p;
            }
        }
        // the coded picture's rows are the frame's rows (fields side by side = two frame rows)
        Ok(vec![y, u, v])
    }

    fn decode_rgb(&self, br: &mut BitReader) -> Result<Vec<Vec<u8>>> {
        let (w, h) = self.coded_size(self.width);
        let n = w * h;
        let alpha = self.format == Format::Rgba;
        // planes in coding order: B, G, R, A; the coded picture is bottom row first
        let mut planes = vec![vec![0u8; n]; if alpha { 4 } else { 3 }];
        let first: [u8; 4] = [br.bits(8) as u8, br.bits(8) as u8, br.bits(8) as u8, br.bits(8) as u8];
        // raw first pixel in the stream's word order: R G B X (RGB24) or A R G B (RGBA)
        let (r0, g0, b0, a0) = if alpha { (first[1], first[2], first[3], first[0]) } else { (first[0], first[1], first[2], 255) };
        for (p, v) in planes.iter_mut().zip([b0, g0, r0, a0]) {
            if let Some(s) = p.first_mut() {
                *s = v;
            }
        }
        let [t0, t1, t2] = &self.tables;
        for i in 1..n {
            let res = if self.decorrelate {
                // tables B-G, G, R-G; coded g, b-g, r-g (a with the R-G table)
                let g = t1.decode(br);
                let b = t0.decode(br).wrapping_add(g);
                let r = t2.decode(br).wrapping_add(g);
                let a = if alpha { t2.decode(br) } else { 0 };
                [b, g, r, a]
            } else {
                // tables B, G, R; coded b, g, r (a with the R table)
                let b = t0.decode(br);
                let g = t1.decode(br);
                let r = t2.decode(br);
                let a = if alpha { t2.decode(br) } else { 0 };
                [b, g, r, a]
            };
            if br.overrun() {
                return Err(Error::Invalid("frame data ends early".into()));
            }
            for (p, d) in planes.iter_mut().zip(res) {
                let v = self.predict(p, i, w, 0).wrapping_add(d);
                if let Some(s) = p.get_mut(i) {
                    *s = v;
                }
            }
        }
        // to R, G, B (, A), top row first
        let (fw, fh) = (self.width as usize, self.height as usize);
        let flip = |p: &[u8]| -> Vec<u8> { (0..fh).rev().flat_map(|r| p.get(r * fw..(r + 1) * fw).unwrap_or_default().iter().copied()).collect() };
        let mut out: Vec<Vec<u8>> = [2usize, 1, 0].iter().filter_map(|&k| planes.get(k)).map(|p| flip(p)).collect();
        if let Some(a) = planes.get(3) {
            out.push(flip(a));
        }
        Ok(out)
    }
}

#[inline]
fn median(a: u8, b: u8, c: u8) -> u8 {
    a.max(b).min(a.min(b).max(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_three() {
        for (a, b, c, m) in [(1, 2, 3, 2), (3, 2, 1, 2), (2, 3, 1, 2), (5, 5, 1, 5), (0, 255, 128, 128)] {
            assert_eq!(median(a, b, c), m);
        }
    }

    #[test]
    fn table_runs() {
        // two of length 8, 252 more of length 8 (count in the next byte), two of length 10
        let data = [0x48u8, 0x08, 0xFC, 0x4A];
        let mut pos = 0;
        let lens = read_lengths(&data, &mut pos).unwrap();
        assert_eq!((lens[0], lens[253], lens[254], lens[255], pos), (8, 8, 10, 10, 4));
    }

    #[test]
    fn hostile_headers_are_errors() {
        assert!(Decoder::new(64, 32, 16, &[]).is_err());
        assert!(Decoder::new(0, 32, 16, &[0, 16, 0x20, 0]).is_err());
        assert!(Decoder::new(63, 32, 16, &[0, 16, 0x20, 0, 0x28, 0xFF]).is_err());
        for n in 0..80u8 {
            let extra: Vec<u8> = (0..n).map(|i| i.wrapping_mul(53) ^ n).collect();
            let _ = Decoder::new(64, 32, 16, &extra);
            let _ = Decoder::new(64, 32, 24 | u16::from(n & 7), &extra);
        }
    }
}
