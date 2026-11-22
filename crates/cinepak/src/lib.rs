//! Cinepak (`cvid`) video decoder, a Rust re-expression of FFmpeg n7.1.1 `libavcodec/cinepak.c`
//! (LGPL-2.1-or-later, Copyright (C) 2003 The FFmpeg project, Ewald Snel; Cinepak colorspace
//! support (c) 2013 Rl, Aetey Global Technologies AB). See `README.md` for the licence notes.
//!
//! A frame is a list of strips; each strip carries optional V4 and V1 codebook updates and one
//! vector chunk that paints 4x4 blocks from the codebooks. The picture persists between frames
//! (skipped blocks keep their pixels) and so do the codebooks of every strip.
//!
//! Safe Rust only. Damaged data is never a panic: like ffmpeg, a frame that fails part-way keeps
//! what was decoded so far and is flagged [`Picture::damaged`].

use thiserror::Error;

/// Cinepak decoding errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("Cinepak: invalid picture size {0}x{1}")]
    Size(u32, u32),
    #[error("Cinepak: frame too short or damaged")]
    Invalid,
    #[error("Cinepak: unsupported frame (encoded size 0)")]
    Unsupported,
}

/// A codebook entry: four pixels (2x2, row-major) of RGB.
type Entry = [u8; 12];

const MAX_STRIPS: usize = 32;

#[derive(Clone)]
struct Strip {
    x1: u16,
    y1: u16,
    x2: u16,
    y2: u16,
    v4: Vec<Entry>,
    v1: Vec<Entry>,
}

impl Strip {
    fn new() -> Self {
        Self { x1: 0, y1: 0, x2: 0, y2: 0, v4: vec![[0; 12]; 256], v1: vec![[0; 12]; 256] }
    }
}

/// The pixel format of a decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Packed RGB, 3 bytes per pixel.
    Rgb24,
    /// 8-bit palette indices (the stream's `BITMAPINFOHEADER` bit count is 8), see [`Picture::palette`].
    Pal8,
}

/// A decoded picture.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    /// `Rgb24`: `width * height * 3` bytes; `Pal8`: `width * height` indices.
    pub data: Vec<u8>,
    /// The palette as R, G, B, A (A is 255) for 256 entries; `Pal8` only.
    pub palette: Vec<u8>,
    /// Whether a strip with the key id (0x10) was seen.
    pub key: bool,
    /// Whether decoding stopped early on invalid data (the picture is partly updated).
    pub damaged: bool,
}

impl Picture {
    /// The picture as RGBA, palette applied.
    pub fn to_rgba(&self) -> Vec<u8> {
        let n = (self.width as usize).saturating_mul(self.height as usize);
        let mut out = Vec::with_capacity(n.saturating_mul(4));
        match self.format {
            Format::Rgb24 => {
                for px in self.data.chunks_exact(3).take(n) {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            Format::Pal8 => {
                for &i in self.data.iter().take(n) {
                    let o = usize::from(i) * 4;
                    out.extend_from_slice(self.palette.get(o..o + 4).unwrap_or(&[0, 0, 0, 255]));
                }
            }
        }
        out
    }
}

/// A Cinepak decoder for one stream.
pub struct Decoder {
    width: u32,
    height: u32,
    /// Width and height rounded up to whole 4x4 blocks (the coded area).
    cw: usize,
    ch: usize,
    palette_video: bool,
    strips: Vec<Strip>,
    /// The picture, `cw + 4` pixels per row (blocks at odd x offsets overhang), `ch` rows.
    frame: Vec<u8>,
    /// `None` until the first frame decides (Sega FILM streams have extra bytes after the header).
    sega_skip: Option<usize>,
    key: bool,
    /// R, G, B, A for 256 entries.
    pal: Vec<u8>,
}

fn rb16(d: &[u8], o: usize) -> u32 {
    u32::from(d.get(o).copied().unwrap_or(0)) << 8 | u32::from(d.get(o + 1).copied().unwrap_or(0))
}

fn rb24(d: &[u8], o: usize) -> u32 {
    rb16(d, o) << 8 | u32::from(d.get(o + 2).copied().unwrap_or(0))
}

fn rb32(d: &[u8], o: usize) -> u32 {
    rb16(d, o) << 16 | rb16(d, o + 2)
}

fn clip(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

impl Decoder {
    /// A decoder for `width` x `height` pictures. `bit_count` is the `BITMAPINFOHEADER` bit count
    /// (8: palette video, anything else RGB24).
    pub fn new(width: u32, height: u32, bit_count: u16) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(Error::Size(width, height));
        }
        let (cw, ch) = ((width as usize + 3) & !3, (height as usize + 3) & !3);
        let palette_video = bit_count == 8;
        let bpp = if palette_video { 1 } else { 3 };
        Ok(Self {
            width,
            height,
            cw,
            ch,
            palette_video,
            strips: vec![Strip::new(); MAX_STRIPS],
            frame: vec![0; (cw + 4) * ch * bpp],
            key: false,
            sega_skip: None,
            pal: [0u8, 0, 0, 255].repeat(256),
        })
    }

    /// Set the palette of 8-bit streams from `BITMAPINFOHEADER` colour entries (B, G, R, unused,
    /// up to 256 of them).
    pub fn set_palette_bgr0(&mut self, quads: &[u8]) {
        for (i, q) in quads.chunks_exact(4).take(256).enumerate() {
            self.pal[i * 4..i * 4 + 4].copy_from_slice(&[q[2], q[1], q[0], 255]);
        }
    }

    /// Decode one frame.
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture, Error> {
        if data.len() < 10 {
            return Err(Error::Invalid);
        }
        let num_strips = rb16(data, 8) as usize;
        if num_strips != 0 {
            self.predecode_check(data)?;
        }
        self.key = false;
        let damaged = num_strips != 0 && self.decode_strips(data).is_err();
        Ok(self.picture(damaged))
    }

    /// Reset to the state of a new stream.
    pub fn reset(&mut self) {
        self.strips = vec![Strip::new(); MAX_STRIPS];
        self.frame.fill(0);
        self.sega_skip = None;
    }

    fn predecode_check(&mut self, d: &[u8]) -> Result<(), Error> {
        let num_strips = rb16(d, 8) as usize;
        let encoded = rb24(d, 1) as usize;
        // ffmpeg's default discard_damaged_percentage is 95
        if (d.len() as u64) < encoded as u64 * 5 / 100 {
            return Err(Error::Invalid);
        }
        let skip = match self.sega_skip {
            Some(s) => s,
            None => {
                if encoded == 0 {
                    return Err(Error::Unsupported);
                }
                // a container size that is not the encoded size (nor a multiple) means Sega FILM
                let s = if encoded != d.len() && !d.len().is_multiple_of(encoded) {
                    if d.len() >= 16 && d[10..16] == [0xFE, 0, 0, 6, 0, 0] { 6 } else { 2 }
                } else {
                    0
                };
                self.sega_skip = Some(s);
                s
            }
        };
        if d.len() < 10 + skip + num_strips * 12 {
            return Err(Error::Invalid);
        }
        let first = 10 + skip;
        let strip_size = rb24(d, first + 1) as usize;
        if strip_size < 12 || strip_size > encoded {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    fn decode_strips(&mut self, d: &[u8]) -> Result<(), Error> {
        let flags = d[0];
        let n = (rb16(d, 8) as usize).min(MAX_STRIPS);
        let mut pos = 10 + self.sega_skip.unwrap_or(0);
        let mut y0 = 0u32;
        for i in 0..n {
            let h = d.get(pos..pos + 12).ok_or(Error::Invalid)?;
            let (id, y1_raw) = (h[0], rb16(h, 4));
            // a zero y1 means "relative to the previous strip"; y2 wraps like ffmpeg's uint16
            let (y1, y2) = if y1_raw == 0 { (y0 as u16, (y0 + rb16(h, 8)) as u16) } else { (y1_raw as u16, rb16(h, 8) as u16) };
            let strip = &mut self.strips[i];
            (strip.y1, strip.y2, strip.x1, strip.x2) = (y1, y2, rb16(h, 6) as u16, rb16(h, 10) as u16);
            let size = (rb24(h, 1) as usize).checked_sub(12).ok_or(Error::Invalid)?;
            self.key |= id == 0x10;
            pos += 12;
            let size = size.min(d.len() - pos);
            if i > 0 && flags & 1 == 0 {
                let (v4, v1) = (self.strips[i - 1].v4.clone(), self.strips[i - 1].v1.clone());
                self.strips[i].v4 = v4;
                self.strips[i].v1 = v1;
            }
            self.decode_strip(i, &d[pos..pos + size])?;
            pos += size;
            y0 = u32::from(self.strips[i].y2);
        }
        Ok(())
    }

    fn decode_strip(&mut self, i: usize, d: &[u8]) -> Result<(), Error> {
        let s = &self.strips[i];
        if usize::from(s.x2) > self.cw || usize::from(s.y2) > self.ch || s.x1 >= s.x2 || s.y1 >= s.y2 {
            return Err(Error::Invalid);
        }
        let mut pos = 0;
        while pos + 4 <= d.len() {
            let id = d[pos];
            let size = (rb24(d, pos + 1) as usize).checked_sub(4).ok_or(Error::Invalid)?;
            pos += 4;
            let size = size.min(d.len() - pos);
            let chunk = &d[pos..pos + size];
            match id {
                0x20 | 0x21 | 0x24 | 0x25 => decode_codebook(&mut self.strips[i].v4, id, chunk),
                0x22 | 0x23 | 0x26 | 0x27 => decode_codebook(&mut self.strips[i].v1, id, chunk),
                0x30..=0x32 => return self.decode_vectors(i, id, chunk),
                _ => {}
            }
            pos += size;
        }
        Err(Error::Invalid)
    }

    fn decode_vectors(&mut self, si: usize, id: u8, d: &[u8]) -> Result<(), Error> {
        let strip = &self.strips[si];
        let (x1, x2, y1, y2) = (usize::from(strip.x1), usize::from(strip.x2), usize::from(strip.y1), usize::from(strip.y2));
        let bpp = if self.palette_video { 1 } else { 3 };
        let (stride, height) = ((self.cw + 4) * bpp, self.height as usize);
        let (mut flag, mut mask, mut pos) = (0u32, 0u32, 0usize);
        for y in (y1..y2).step_by(4) {
            // rows: a height that is not a multiple of 4 aliases the missing rows onto row 0 (ffmpeg's
            // order of writes, bottom up, ends with the right data)
            let mut rows = [y; 4];
            if height > y + 1 {
                rows[1] = y + 1;
                if height > y + 2 {
                    rows[2] = y + 2;
                    if height > y + 3 {
                        rows[3] = y + 3;
                    }
                }
            }
            let mut x = x1;
            while x < x2 {
                if id & 1 != 0 {
                    mask >>= 1;
                    if mask == 0 {
                        if pos + 4 > d.len() {
                            return Err(Error::Invalid);
                        }
                        flag = rb32(d, pos);
                        pos += 4;
                        mask = 0x8000_0000;
                    }
                }
                if id & 1 == 0 || flag & mask != 0 {
                    if id & 2 == 0 {
                        mask >>= 1;
                        if mask == 0 {
                            if pos + 4 > d.len() {
                                return Err(Error::Invalid);
                            }
                            flag = rb32(d, pos);
                            pos += 4;
                            mask = 0x8000_0000;
                        }
                    }
                    let o = x * bpp;
                    if id & 2 != 0 || !flag & mask != 0 {
                        let p = *self.strips[si].v1.get(usize::from(*d.get(pos).ok_or(Error::Invalid)?)).ok_or(Error::Invalid)?;
                        pos += 1;
                        if self.palette_video {
                            for (r, (a, b)) in [(3, (6, 9)), (2, (6, 9)), (1, (0, 3)), (0, (0, 3))] {
                                let l = &mut self.frame[rows[r] * stride + o..][..4];
                                l.copy_from_slice(&[p[a], p[a], p[b], p[b]]);
                            }
                        } else {
                            for (r, (a, b)) in [(3, (6, 9)), (2, (6, 9)), (1, (0, 3)), (0, (0, 3))] {
                                let l = &mut self.frame[rows[r] * stride + o..][..12];
                                for k in 0..2 {
                                    l[k * 3..k * 3 + 3].copy_from_slice(&p[a..a + 3]);
                                    l[6 + k * 3..6 + k * 3 + 3].copy_from_slice(&p[b..b + 3]);
                                }
                            }
                        }
                    } else if flag & mask != 0 {
                        if pos + 4 > d.len() {
                            return Err(Error::Invalid);
                        }
                        let cb = [d[pos], d[pos + 1], d[pos + 2], d[pos + 3]].map(|i| self.strips[si].v4[usize::from(i)]);
                        pos += 4;
                        if self.palette_video {
                            // (row, left codebook, right codebook, offset of the codebook row)
                            for (r, l, rr, off) in [(3, 2, 3, 6), (2, 2, 3, 0), (1, 0, 1, 6), (0, 0, 1, 0)] {
                                let line = &mut self.frame[rows[r] * stride + o..][..4];
                                line.copy_from_slice(&[cb[l][off], cb[l][off + 3], cb[rr][off], cb[rr][off + 3]]);
                            }
                        } else {
                            for (r, l, rr, off) in [(3, 2, 3, 6), (2, 2, 3, 0), (1, 0, 1, 6), (0, 0, 1, 0)] {
                                let line = &mut self.frame[rows[r] * stride + o..][..12];
                                line[..6].copy_from_slice(&cb[l][off..off + 6]);
                                line[6..].copy_from_slice(&cb[rr][off..off + 6]);
                            }
                        }
                    }
                }
                x += 4;
            }
        }
        Ok(())
    }

    fn picture(&self, damaged: bool) -> Picture {
        let (w, h) = (self.width as usize, self.height as usize);
        let bpp = if self.palette_video { 1 } else { 3 };
        let mut data = Vec::with_capacity(w * h * bpp);
        for row in self.frame.chunks_exact((self.cw + 4) * bpp).take(h) {
            data.extend_from_slice(&row[..w * bpp]);
        }
        Picture {
            width: self.width,
            height: self.height,
            format: if self.palette_video { Format::Pal8 } else { Format::Rgb24 },
            data,
            palette: if self.palette_video { self.pal.clone() } else { Vec::new() },
            key: self.key,
            damaged,
        }
    }
}

fn decode_codebook(cb: &mut [Entry], id: u8, d: &[u8]) {
    let n = if id & 4 != 0 { 4 } else { 6 };
    let (mut flag, mut mask, mut pos) = (0u32, 0u32, 0usize);
    for entry in cb.iter_mut().take(256) {
        if id & 1 != 0 {
            mask >>= 1;
            if mask == 0 {
                if pos + 4 > d.len() {
                    break;
                }
                flag = rb32(d, pos);
                pos += 4;
                mask = 0x8000_0000;
            }
        }
        if id & 1 == 0 || flag & mask != 0 {
            if pos + n > d.len() {
                break;
            }
            for k in 0..4 {
                entry[k * 3..k * 3 + 3].fill(d[pos + k]);
            }
            if n == 6 {
                let (u, v) = (i32::from(d[pos + 4] as i8), i32::from(d[pos + 5] as i8));
                for k in 0..4 {
                    let (r, g, b) = (i32::from(entry[k * 3]), i32::from(entry[k * 3 + 1]), i32::from(entry[k * 3 + 2]));
                    entry[k * 3] = clip(r + v * 2);
                    entry[k * 3 + 1] = clip(g - u / 2 - v);
                    entry[k * 3 + 2] = clip(b + u * 2);
                }
            }
            pos += n;
        }
    }
}
