//! Pure-Rust Huffyuv (`HFYU`) decoder, with the sliced packets of Huffyuv MT (`HYMT`).
//!
//! The `HFYU` decoder is clean-room (below). The `HYMT` slice layout is the one place derived from
//! a source reading: it follows FFmpeg n9.0.2 `huffyuvdec.c` (LGPL-2.1-or-later, the owner's
//! general FFmpeg-port approval of 2026-11-14 and the decision of 2026-11-23); see
//! [`Decoder::set_slices`].
//!
//! Clean-room part:
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
//! file ffmpeg writes). Two parts are ported from FFmpeg n9.0.2 `huffyuvdec.c` (same approval as
//! for `HYMT`): the built-in "classic" tables of Huffyuv 1.x files (no tables in the header), and
//! 4:2:0 streams (12 bits per pixel, an ffmpeg encoder extension). FFmpeg's own `FFVH` variant is
//! rejected with [`Error::Unsupported`].
//!
//! ```no_run
//! let extra: Vec<u8> = Vec::new(); // BITMAPINFOHEADER bytes after the first 40
//! let mut dec = cfa4_huffyuv::Decoder::new(720, 576, 16, &extra).unwrap();
//! let frame: Vec<u8> = Vec::new(); // one AVI chunk
//! let pic = dec.decode(&frame).unwrap();
//! println!("{}x{} {:?}", pic.width, pic.height, pic.format);
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod classic;
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
    /// Y, U, V; U and V are `width / 2` x `height / 2` (12 bits per pixel).
    Yuv420,
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
    /// Huffyuv MT (`HYMT`): a packet may carry a slice trailer (see [`Decoder::set_slices`]).
    slices: bool,
}

/// Section "Huffman tables" of the description: run-length coded code lengths. Each byte holds a
/// length in its low 5 bits and a repeat count in its high 3 bits; a count of 0 means the count
/// is the next byte.
pub(crate) fn read_lengths(data: &[u8], pos: &mut usize) -> Result<[u8; 256]> {
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
        // FFmpeg's header versions: 0 no extradata and 1 (low bits of biBitCount set) use the
        // classic tables; 2 has the tables after four bytes of extradata; 3 is FFVH
        let version = if extra.is_empty() {
            0
        } else if bit_count & 7 != 0 && bit_count != 12 {
            1
        } else if extra.get(3) == Some(&0) {
            2
        } else {
            3
        };
        if version == 3 {
            return Err(Error::Unsupported("Huffyuv FFVH extradata".into()));
        }
        let (predictor, decorrelate, bpp, interlace_flag);
        if version == 2 {
            ensure!(extra.len() >= 4, "Huffyuv extradata too short");
            let method = extra.first().copied().unwrap_or(0);
            (predictor, decorrelate) = match method {
                0 | 0xFE => (Predictor::Left, false),
                1 => (Predictor::Gradient, false),
                2 => (Predictor::Median, false),
                0x40 => (Predictor::Left, true),
                0x41 => (Predictor::Gradient, true),
                0x42 => (Predictor::Median, true),
                m => return Err(Error::Unsupported(format!("Huffyuv prediction method {m:#x}"))),
            };
            bpp = extra.get(1).copied().filter(|&b| b != 0).map_or(bit_count & !7, u16::from);
            interlace_flag = extra.get(2).copied().unwrap_or(0);
            if interlace_flag & 0x40 != 0 {
                return Err(Error::Unsupported("Huffyuv adaptive tables".into()));
            }
        } else {
            bpp = bit_count & !7;
            interlace_flag = 0;
            (predictor, decorrelate) = match bit_count & 7 {
                1 => (Predictor::Left, false),
                2 => (Predictor::Left, true),
                3 => (Predictor::Gradient, bit_count >= 24),
                4 => (Predictor::Median, false),
                _ => (Predictor::Left, false),
            };
        }
        let format = match bpp {
            12 => Format::Yuv420,
            16 => Format::Yuv422,
            24 => Format::Rgb,
            32 => Format::Rgba,
            b => return Err(Error::Unsupported(format!("Huffyuv with {b} bits per pixel"))),
        };
        if matches!(format, Format::Yuv422 | Format::Yuv420) {
            ensure!(width.is_multiple_of(2), "YUV frame width {width} is odd");
            ensure!(format != Format::Yuv420 || height.is_multiple_of(2), "4:2:0 frame height {height} is odd");
            // as in FFmpeg, a YUV stream ignores the decorrelation flag
            ensure!(
                format != Format::Yuv422 || predictor != Predictor::Median || width.is_multiple_of(4),
                "YUY2 median frame width {width} is not a multiple of 4"
            );
        }
        // flags: 0x10 interlaced, 0x20 progressive; neither: the 1.x rule (taller than 288)
        let interlaced = match interlace_flag & 0x30 {
            0x10 => true,
            0x20 => false,
            _ => height > 288,
        } && height.is_multiple_of(2);
        let decorrelate = decorrelate && matches!(format, Format::Rgb | Format::Rgba);
        let tables = if version == 2 {
            let mut pos = 4;
            [Table::new(&read_lengths(extra, &mut pos)?)?, Table::new(&read_lengths(extra, &mut pos)?)?, Table::new(&read_lengths(extra, &mut pos)?)?]
        } else {
            classic::tables(matches!(format, Format::Rgb | Format::Rgba))?
        };
        Ok(Decoder { width, height, format, predictor, decorrelate, interlaced, tables, slices: false })
    }

    /// The layout of the pictures this stream decodes to.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Huffyuv MT (`HYMT`) mode. A packet that ends in a 16-byte trailer whose first word is zero is
    /// then sliced: the trailer holds, little-endian, at `len - 12` the slice count, at `len - 8`
    /// the slice height and at `len - 4` the offset of a table of (offset, size) word pairs, one per
    /// slice (a single slice takes the whole packet). Every slice is an independent bit stream of
    /// `slice height` rows with its own raw first pair, and slice 0 is the bottom one. Packets
    /// without the trailer decode as plain Huffyuv. Only YUY2 streams slice (RGB packets with a
    /// trailer are rejected), as in FFmpeg n9.0.2.
    pub fn set_slices(&mut self, on: bool) {
        self.slices = on;
    }

    /// Decode one frame (one AVI chunk).
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture> {
        if self.slices && data.len() > 32 && word(data, data.len() - 16) == Some(0) {
            return self.decode_sliced(data);
        }
        let mut br = BitReader::new(data);
        let planes = match self.format {
            Format::Yuv422 => self.decode_yuv(&mut br)?,
            Format::Yuv420 => self.decode_yuv420(&mut br)?,
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
    fn coded_size(&self, w: u32, rows: usize) -> (usize, usize) {
        let k = if self.interlaced { 2 } else { 1 };
        (w as usize * k, rows / k)
    }

    /// A sliced `HYMT` packet (see [`Decoder::set_slices`]).
    fn decode_sliced(&self, data: &[u8]) -> Result<Picture> {
        ensure!(self.format == Format::Yuv422, "sliced Huffyuv MT packet with RGB");
        let n = data.len();
        let (count, slice_h, info) =
            (word(data, n - 12).unwrap_or(0) as usize, word(data, n - 8).unwrap_or(0) as usize, word(data, n - 4).unwrap_or(0) as usize);
        let height = self.height as usize;
        ensure!(slice_h > 0 && count > 0 && count.checked_mul(slice_h).is_some_and(|r| r <= height), "Huffyuv MT slice geometry");
        ensure!(count.checked_mul(8).and_then(|t| t.checked_add(info)).is_some_and(|e| e <= n - 16), "Huffyuv MT slice table");
        ensure!(!self.interlaced || slice_h.is_multiple_of(2), "interlaced Huffyuv MT slice height {slice_h}");
        let (w, cw) = (self.width as usize, self.width as usize / 2);
        let mut planes = vec![vec![0u8; w * height], vec![0u8; cw * height], vec![0u8; cw * height]];
        for slice in 0..count {
            let (start, size, y0) = if count > 1 {
                let at = info + slice * 8;
                let (start, size) = (word(data, at).unwrap_or(0) as usize, word(data, at + 4).unwrap_or(0) as usize);
                ensure!(size > 0 && start % 4 == 0 && start.checked_add(size).is_some_and(|e| e <= n), "Huffyuv MT slice {slice} bounds");
                (start, size, height - (slice + 1) * slice_h)
            } else {
                (0, n, 0)
            };
            let part = data.get(start..start + size).ok_or_else(|| Error::Invalid("Huffyuv MT slice bounds".into()))?;
            let mut br = BitReader::new(part);
            let rows = self.decode_yuv_rows(&mut br, slice_h)?;
            ensure!(!br.overrun(), "slice data ends early");
            for (dst, (src, pw)) in planes.iter_mut().zip(rows.iter().zip([w, cw, cw])) {
                if let Some(d) = dst.get_mut(y0 * pw..(y0 + slice_h) * pw) {
                    d.copy_from_slice(src);
                }
            }
        }
        Ok(Picture { width: self.width, height: self.height, format: self.format, planes })
    }

    fn decode_yuv(&self, br: &mut BitReader<'_>) -> Result<Vec<Vec<u8>>> {
        self.decode_yuv_rows(br, self.height as usize)
    }

    fn decode_yuv_rows(&self, br: &mut BitReader<'_>, rows: usize) -> Result<Vec<Vec<u8>>> {
        let (w, h) = self.coded_size(self.width, rows);
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

    /// 4:2:0 (12 bits per pixel), as FFmpeg n9.0.2 decodes it. The first row is a row of pairs
    /// (Y U Y V, like YUY2); after it rows of luma only alternate with rows of pairs (a row of
    /// pairs carries the chroma of its own row and, in the left and gradient layouts, of the row
    /// above it too). The median predictor starts differently (see the code). Rows are
    /// predicted from the row above, or from two rows above in interlaced frames; the running
    /// "left" values continue across rows, as in YUY2.
    fn decode_yuv420(&self, br: &mut BitReader<'_>) -> Result<Vec<Vec<u8>>> {
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, ch) = (w / 2, h / 2);
        let mut y = Plane::new(w, h);
        let mut u = Plane::new(cw, ch);
        let mut v = Plane::new(cw, ch);
        let il = usize::from(self.interlaced);
        let fake = 1 + il;
        let [ty, tu, tv] = &self.tables;
        // the first pair is raw, read in the stream's word order as V Y1 U Y0
        let (rv, ry1, ru, ry0) = (br.bits(8) as u8, br.bits(8) as u8, br.bits(8) as u8, br.bits(8) as u8);
        y.set(0, 0, ry0);
        y.set(0, 1, ry1);
        u.set(0, 0, ru);
        v.set(0, 0, rv);
        let (mut lefty, mut leftu, mut leftv) = (ry1, ru, rv);
        // luma-only values: `count` of them; a row of pairs: `count` luma and `count / 2` each of U and V
        let gray = |br: &mut BitReader<'_>, count: usize| -> Result<Vec<u8>> {
            let mut d = Vec::with_capacity(count);
            for _ in 0..count / 2 {
                d.push(ty.decode(br));
                d.push(ty.decode(br));
            }
            if br.overrun() {
                return Err(Error::Invalid("frame data ends early".into()));
            }
            Ok(d)
        };
        let pairs = |br: &mut BitReader<'_>, count: usize| -> Result<[Vec<u8>; 3]> {
            let (mut dy, mut du, mut dv) = (Vec::with_capacity(count), Vec::with_capacity(count / 2), Vec::with_capacity(count / 2));
            for _ in 0..count / 2 {
                dy.push(ty.decode(br));
                du.push(tu.decode(br));
                dy.push(ty.decode(br));
                dv.push(tv.decode(br));
            }
            if br.overrun() {
                return Err(Error::Invalid("frame data ends early".into()));
            }
            Ok([dy, du, dv])
        };
        // the rest of the first row
        let [dy, du, dv] = pairs(br, w - 2)?;
        lefty = y.left(0, 2, &dy, lefty);
        leftu = u.left(0, 1, &du, leftu);
        leftv = v.left(0, 1, &dv, leftv);
        let (mut row, mut cy) = (1usize, 1usize);
        match self.predictor {
            Predictor::Left | Predictor::Gradient => {
                let plane = self.predictor == Predictor::Gradient;
                while row < h {
                    let g = gray(br, w)?;
                    lefty = y.left(row, 0, &g, lefty);
                    if plane && row > il {
                        y.add_above(row, row.checked_sub(fake), w);
                    }
                    row += 1;
                    if row >= h {
                        break;
                    }
                    let [dy, du, dv] = pairs(br, w)?;
                    lefty = y.left(row, 0, &dy, lefty);
                    leftu = u.left(cy, 0, &du, leftu);
                    leftv = v.left(cy, 0, &dv, leftv);
                    if plane && cy > il {
                        y.add_above(row, row.checked_sub(fake), w);
                        u.add_above(cy, cy.checked_sub(fake), cw);
                        v.add_above(cy, cy.checked_sub(fake), cw);
                    }
                    row += 1;
                    cy += 1;
                }
            }
            Predictor::Median => {
                if row < h {
                    // in interlaced frames the second row is left predicted
                    if il == 1 {
                        let [dy, du, dv] = pairs(br, w)?;
                        lefty = y.left(1, 0, &dy, lefty);
                        leftu = u.left(1, 0, &du, leftu);
                        leftv = v.left(1, 0, &dv, leftv);
                        row += 1;
                        cy += 1;
                    }
                }
                if row < h {
                    // the next 4 pixels (2 pairs) are left predicted too, the rest of the row is median predicted
                    let [dy, du, dv] = pairs(br, 4)?;
                    lefty = y.left(fake, 0, &dy, lefty);
                    leftu = u.left(fake, 0, &du, leftu);
                    leftv = v.left(fake, 0, &dv, leftv);
                    let mut lefttopy = y.get(0, 3);
                    let [dy, du, dv] = pairs(br, w - 4)?;
                    y.median(fake, Some(0), 4, &dy, &mut lefty, &mut lefttopy);
                    let (mut lefttopu, mut lefttopv) = (u.get(0, 1), v.get(0, 1));
                    u.median(fake, Some(0), 2, &du, &mut leftu, &mut lefttopu);
                    v.median(fake, Some(0), 2, &dv, &mut leftv, &mut lefttopv);
                    row += 1;
                    cy += 1;
                    while row < h {
                        // luma-only rows until the row count catches up with the chroma rows
                        while 2 * cy > row && row < h {
                            let g = gray(br, w)?;
                            y.median(row, row.checked_sub(fake), 0, &g, &mut lefty, &mut lefttopy);
                            row += 1;
                        }
                        if row >= h {
                            break;
                        }
                        let [dy, du, dv] = pairs(br, w)?;
                        y.median(row, row.checked_sub(fake), 0, &dy, &mut lefty, &mut lefttopy);
                        u.median(cy, cy.checked_sub(fake), 0, &du, &mut leftu, &mut lefttopu);
                        v.median(cy, cy.checked_sub(fake), 0, &dv, &mut leftv, &mut lefttopv);
                        row += 1;
                        cy += 1;
                    }
                }
            }
        }
        Ok(vec![y.d, u.d, v.d])
    }

    fn decode_rgb(&self, br: &mut BitReader<'_>) -> Result<Vec<Vec<u8>>> {
        let (w, h) = self.coded_size(self.width, self.height as usize);
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

/// A plane for the 4:2:0 decoder: rows addressed by (row, column), reads outside give 0 and
/// writes outside are dropped.
struct Plane {
    d: Vec<u8>,
    w: usize,
}

impl Plane {
    fn new(w: usize, h: usize) -> Plane {
        Plane { d: vec![0; w * h], w }
    }

    fn get(&self, row: usize, x: usize) -> u8 {
        row.checked_mul(self.w).and_then(|r| r.checked_add(x)).and_then(|i| self.d.get(i)).copied().unwrap_or(0)
    }

    fn set(&mut self, row: usize, x: usize, val: u8) {
        if let Some(e) = row.checked_mul(self.w).and_then(|r| r.checked_add(x)).and_then(|i| self.d.get_mut(i)) {
            *e = val;
        }
    }

    /// Left prediction of `diff` from column `x0` of `row`, starting from `acc`; the new `acc`.
    fn left(&mut self, row: usize, x0: usize, diff: &[u8], mut acc: u8) -> u8 {
        for (k, &d) in diff.iter().enumerate() {
            acc = acc.wrapping_add(d);
            self.set(row, x0 + k, acc);
        }
        acc
    }

    /// Add the first `n` samples of the row `above` to those of `row`.
    fn add_above(&mut self, row: usize, above: Option<usize>, n: usize) {
        let Some(above) = above else { return };
        for x in 0..n {
            let v = self.get(row, x).wrapping_add(self.get(above, x));
            self.set(row, x, v);
        }
    }

    /// Median prediction of `diff` from column `x0` of `row` (above: `above`, 0 without), with the
    /// running left and left-top values.
    fn median(&mut self, row: usize, above: Option<usize>, x0: usize, diff: &[u8], left: &mut u8, left_top: &mut u8) {
        for (k, &d) in diff.iter().enumerate() {
            let up = above.map_or(0, |a| self.get(a, x0 + k));
            let grad = left.wrapping_add(up).wrapping_sub(*left_top);
            *left = median(*left, up, grad).wrapping_add(d);
            *left_top = up;
            self.set(row, x0 + k, *left);
        }
    }
}

/// The little-endian 32-bit word at `at`.
fn word(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
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
        // no extradata: the classic tables of Huffyuv 1.x
        assert!(Decoder::new(64, 32, 16, &[]).is_ok());
        assert!(Decoder::new(64, 32, 8, &[]).is_err());
        assert!(Decoder::new(64, 32, 16, &[0, 16, 0x20]).is_err());
        assert!(Decoder::new(0, 32, 16, &[0, 16, 0x20, 0]).is_err());
        assert!(Decoder::new(63, 32, 16, &[0, 16, 0x20, 0, 0x28, 0xFF]).is_err());
        for n in 0..80u8 {
            let extra: Vec<u8> = (0..n).map(|i| i.wrapping_mul(53) ^ n).collect();
            let _ = Decoder::new(64, 32, 16, &extra);
            let _ = Decoder::new(64, 32, 24 | u16::from(n & 7), &extra);
        }
    }
}
