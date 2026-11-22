// Ported from FFmpeg n7.1.1 libavcodec/mjpegdec.c (the baseline sequential path and the THP
// variant), LGPL-2.1-or-later, Copyright (c) 2000, 2001 Fabrice Bellard, (c) 2003 Alex Beregszaszi,
// (c) 2003-2004 Michael Niedermayer.

//! The JPEG decoder of `thp` video: baseline Huffman JPEG, 8 bits, one scan.
//!
//! THP differs from plain JPEG in two ways ffmpeg's `thp` decoder spells out: the scan data is
//! not byte-stuffed (it runs to the end of the packet as it is), and a restart interval has no
//! `RSTn` markers: the bit stream is byte-aligned and the DC predictors reset after every
//! interval.

use crate::idct;
use crate::{Error, Result};

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Most pixels one picture may have.
pub const MAX_PIXELS: u64 = 1 << 26;

/// How the chroma planes are sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    /// One plane.
    Gray,
    C420,
    C422,
    C440,
    C444,
}

/// A decoded picture: full range YUV (JFIF), planes without padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub chroma: Chroma,
    /// Y, Cb, Cr (the last two empty for [`Chroma::Gray`]).
    pub planes: [Vec<u8>; 3],
}

struct Huff {
    /// `maxcode[l]` is the largest code of length `l` (or -1), `valptr[l]` the index of the
    /// first value of that length, `mincode[l]` its first code.
    maxcode: [i32; 18],
    mincode: [i32; 17],
    valptr: [usize; 17],
    vals: Vec<u8>,
}

impl Huff {
    fn new(bits: &[u8; 17], vals: &[u8]) -> Result<Self> {
        let mut h = Huff { maxcode: [-1; 18], mincode: [0; 17], valptr: [0; 17], vals: vals.to_vec() };
        let mut code = 0i32;
        let mut k = 0usize;
        for l in 1..=16 {
            h.valptr[l] = k;
            h.mincode[l] = code;
            code += i32::from(bits[l]);
            k += usize::from(bits[l]);
            if bits[l] > 0 {
                h.maxcode[l] = code - 1;
            }
            if code > (1 << l) {
                return Err(Error::Invalid("Huffman table over-subscribed"));
            }
            code <<= 1;
        }
        Ok(h)
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> i64 {
        self.data.len() as i64 * 8 - self.pos as i64
    }
    /// The next `n` (at most 25) bits, zeros past the end.
    fn show(&self, n: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut v = 0u32;
        for i in 0..4 {
            v = (v << 8) | u32::from(self.data.get(byte + i).copied().unwrap_or(0));
        }
        (v << (self.pos & 7)) >> (32 - n)
    }
    fn skip(&mut self, n: u32) {
        self.pos += n as usize;
    }
    fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.show(n);
        self.skip(n);
        v
    }
    fn align(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }
    fn huff(&mut self, h: &Huff) -> Option<u8> {
        let mut code = 0i32;
        for l in 1..=16usize {
            code = (code << 1) | self.show(1) as i32;
            self.skip(1);
            if h.maxcode[l] >= 0 && code <= h.maxcode[l] && code >= h.mincode[l] {
                return h.vals.get(h.valptr[l] + (code - h.mincode[l]) as usize).copied();
            }
        }
        None
    }
}

/// `get_xbits`: the `n`-bit amplitude of a Huffman code.
fn xbits(b: &mut Bits<'_>, n: u32) -> i32 {
    let v = b.get(n) as i32;
    if v >> (n - 1) == 0 { v - ((1 << n) - 1) } else { v }
}

#[derive(Default, Clone, Copy)]
struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
}

fn be16(d: &[u8], p: usize) -> Option<usize> {
    Some(usize::from(*d.get(p)?) << 8 | usize::from(*d.get(p + 1)?))
}

/// Decode one THP video frame (a JPEG from `SOI` on).
pub fn decode(data: &[u8]) -> Result<Picture> {
    let mut quant = [[0u16; 64]; 4];
    let mut dc: [Option<Huff>; 4] = [None, None, None, None];
    let mut ac: [Option<Huff>; 4] = [None, None, None, None];
    let mut comps: Vec<Comp> = Vec::new();
    let (mut width, mut height) = (0usize, 0usize);
    let mut restart = 0usize;
    let mut got_sof = false;
    let mut pos = 0usize;
    loop {
        // the next marker
        while pos + 1 < data.len() && !(data[pos] == 0xFF && data[pos + 1] != 0 && data[pos + 1] != 0xFF) {
            pos += 1;
        }
        if pos + 1 >= data.len() {
            return Err(Error::Invalid("no scan"));
        }
        let marker = data[pos + 1];
        pos += 2;
        match marker {
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 => return Err(Error::Invalid("end of image before the scan")),
            _ => {}
        }
        let len = be16(data, pos).ok_or(Error::Truncated)?;
        if len < 2 || pos + len > data.len() {
            return Err(Error::Truncated);
        }
        let seg = &data[pos + 2..pos + len];
        match marker {
            0xDB => {
                let mut s = seg;
                while s.len() >= 65 {
                    let (pr, idx) = (s[0] >> 4, usize::from(s[0] & 15));
                    if pr > 1 || idx >= 4 {
                        return Err(Error::Invalid("quantisation table"));
                    }
                    let step = 1 + 64 * (1 + usize::from(pr));
                    if s.len() < step {
                        break;
                    }
                    for i in 0..64 {
                        quant[idx][i] = if pr == 0 { u16::from(s[1 + i]) } else { u16::from_be_bytes([s[1 + 2 * i], s[2 + 2 * i]]) };
                    }
                    s = &s[step..];
                }
            }
            0xC4 => {
                let mut s = seg;
                while !s.is_empty() {
                    if s.len() < 17 {
                        return Err(Error::Invalid("Huffman table"));
                    }
                    let (class, idx) = (s[0] >> 4, usize::from(s[0] & 15));
                    if class >= 2 || idx >= 4 {
                        return Err(Error::Invalid("Huffman table"));
                    }
                    let mut bits = [0u8; 17];
                    bits[1..].copy_from_slice(&s[1..17]);
                    let n: usize = bits.iter().map(|&b| usize::from(b)).sum();
                    if n > 256 || s.len() < 17 + n {
                        return Err(Error::Invalid("Huffman table"));
                    }
                    let h = Huff::new(&bits, &s[17..17 + n])?;
                    if class == 0 {
                        dc[idx] = Some(h)
                    } else {
                        ac[idx] = Some(h)
                    }
                    s = &s[17 + n..];
                }
            }
            0xDD => restart = be16(seg, 0).ok_or(Error::Truncated)?,
            0xC0 | 0xC1 => {
                if seg.len() < 6 || seg[0] != 8 {
                    return Err(Error::Invalid("only 8-bit baseline JPEG"));
                }
                height = be16(seg, 1).unwrap_or(0);
                width = be16(seg, 3).unwrap_or(0);
                let n = usize::from(seg[5]);
                if width == 0 || height == 0 || (width * height) as u64 > MAX_PIXELS {
                    return Err(Error::Invalid("picture size"));
                }
                if !(n == 1 || n == 3) || seg.len() < 6 + 3 * n {
                    return Err(Error::Invalid("component count"));
                }
                comps = (0..n)
                    .map(|i| {
                        let b = &seg[6 + 3 * i..];
                        Comp { id: b[0], h: usize::from(b[1] >> 4), v: usize::from(b[1] & 15), tq: usize::from(b[2]) }
                    })
                    .collect();
                if comps.iter().any(|c| c.h == 0 || c.h > 4 || c.v == 0 || c.v > 4 || c.tq >= 4) {
                    return Err(Error::Invalid("component sampling"));
                }
                got_sof = true;
            }
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return Err(Error::Invalid("not baseline JPEG")),
            0xDA => {
                if !got_sof {
                    return Err(Error::Invalid("scan before SOF"));
                }
                let n = usize::from(*seg.first().ok_or(Error::Truncated)?);
                if n != comps.len() || len != 6 + 2 * n || seg.len() < 1 + 2 * n {
                    return Err(Error::Invalid("scan components"));
                }
                let mut sel = Vec::new();
                for i in 0..n {
                    let id = seg[1 + 2 * i];
                    let idx = comps.iter().position(|c| c.id == id).ok_or(Error::Invalid("scan component"))?;
                    let (td, ta) = (usize::from(seg[2 + 2 * i] >> 4), usize::from(seg[2 + 2 * i] & 15));
                    if td >= 4 || ta >= 4 || dc[td].is_none() || ac[ta].is_none() {
                        return Err(Error::Invalid("scan tables"));
                    }
                    sel.push((idx, td, ta));
                }
                return scan(&data[pos + len..], width, height, &comps, &sel, &quant, &dc, &ac, restart);
            }
            _ => {}
        }
        pos += len;
    }
}

#[allow(clippy::too_many_arguments)]
fn scan(
    data: &[u8],
    width: usize,
    height: usize,
    comps: &[Comp],
    sel: &[(usize, usize, usize)],
    quant: &[[u16; 64]; 4],
    dc: &[Option<Huff>; 4],
    ac: &[Option<Huff>; 4],
    restart: usize,
) -> Result<Picture> {
    let n = comps.len();
    let (hmax, vmax) = if n == 1 { (1, 1) } else { (comps.iter().map(|c| c.h).max().unwrap_or(1), comps.iter().map(|c| c.v).max().unwrap_or(1)) };
    let sampling = |c: &Comp| if n == 1 { (1, 1) } else { (c.h, c.v) };
    let chroma = if n == 1 {
        Chroma::Gray
    } else {
        let (a, b, c) = (sampling(&comps[0]), sampling(&comps[1]), sampling(&comps[2]));
        match (a, b, c) {
            ((1, 1), (1, 1), (1, 1)) => Chroma::C444,
            ((2, 2), (1, 1), (1, 1)) => Chroma::C420,
            ((2, 1), (1, 1), (1, 1)) => Chroma::C422,
            ((1, 2), (1, 1), (1, 1)) => Chroma::C440,
            _ => return Err(Error::Invalid("unsupported chroma sampling")),
        }
    };
    let (hs, vs) = match chroma {
        Chroma::Gray | Chroma::C444 => (0, 0),
        Chroma::C420 => (1, 1),
        Chroma::C422 => (1, 0),
        Chroma::C440 => (0, 1),
    };
    let mb_w = width.div_ceil(hmax * 8);
    let mb_h = height.div_ceil(vmax * 8);
    // padded planes
    let mut strides = Vec::new();
    let mut planes: Vec<Vec<u8>> = Vec::new();
    for c in comps {
        let (h, v) = sampling(c);
        let (sw, sh) = (mb_w * h * 8, mb_h * v * 8);
        strides.push(sw);
        planes.push(vec![0u8; sw * sh]);
    }
    let vis = |ci: usize| if ci == 0 || n == 1 { (width, height) } else { ((width + (1 << hs) - 1) >> hs, (height + (1 << vs) - 1) >> vs) };
    let mut b = Bits { data, pos: 0 };
    let mut last_dc = [1024i32; 3];
    let mut count = 0usize;
    let mut block = [0i16; 64];
    for mb_y in 0..mb_h {
        for mb_x in 0..mb_w {
            if restart != 0 && count == 0 {
                count = restart;
            }
            if b.left() < 0 {
                return Err(Error::Invalid("scan data ends early"));
            }
            for (si, &(ci, td, ta)) in sel.iter().enumerate() {
                let (h, v) = sampling(&comps[ci]);
                let q = &quant[comps[ci].tq];
                let (dct, act) = (dc[td].as_ref().expect("checked"), ac[ta].as_ref().expect("checked"));
                for blk in 0..h * v {
                    let (x, y) = (blk % h, blk / h);
                    block.fill(0);
                    // DC
                    let code = b.huff(dct).ok_or(Error::Invalid("bad DC code"))?;
                    if code > 16 {
                        return Err(Error::Invalid("bad DC code"));
                    }
                    let diff = if code == 0 { 0 } else { xbits(&mut b, u32::from(code)) };
                    let val = (diff as u32).wrapping_mul(u32::from(q[0])).wrapping_add(last_dc[si] as u32) as i32;
                    last_dc[si] = val;
                    block[0] = val.clamp(-32768, 32767) as i16;
                    // AC
                    let mut i = 0usize;
                    while i < 63 {
                        let code = b.huff(act).ok_or(Error::Invalid("bad AC code"))?;
                        let (run, size) = (usize::from(code >> 4), u32::from(code & 15));
                        if size == 0 {
                            if run == 15 {
                                i += 16;
                                continue;
                            }
                            break;
                        }
                        i += run + 1;
                        if i > 63 {
                            return Err(Error::Invalid("AC coefficient index"));
                        }
                        let level = xbits(&mut b, size);
                        block[usize::from(ZIGZAG[i])] = level.wrapping_mul(i32::from(q[i])) as i16;
                    }
                    let (bx, by) = (h * mb_x + x, v * mb_y + y);
                    let (vw, vh) = vis(ci);
                    if 8 * bx < vw && 8 * by < vh {
                        idct::put(&mut block, &mut planes[ci], by * 8 * strides[ci] + bx * 8, strides[ci]);
                    }
                }
            }
            if restart != 0 {
                count -= 1;
                if count == 0 {
                    b.align();
                    last_dc = [1024; 3];
                }
                // `handle_rstn`: ffmpeg also looks for an RSTn marker behind the interval
                if count == 0 {
                    let i = 8 + ((8 - (b.pos & 7)) & 7) as u32;
                    let s = b.show(i);
                    if s == (1 << i) - 1 || s == 0xFF {
                        let pos = b.pos;
                        b.align();
                        while b.left() >= 8 && b.show(8) == 0xFF {
                            b.skip(8);
                        }
                        if b.left() >= 8 && (b.get(8) & 0xF8) == 0xD0 {
                            last_dc = [1024; 3];
                        } else {
                            b.pos = pos;
                        }
                    }
                }
            }
        }
    }
    // crop the padded planes
    let mut out: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for ci in 0..n {
        let (vw, vh) = vis(ci);
        let mut p = Vec::with_capacity(vw * vh);
        for y in 0..vh {
            p.extend_from_slice(&planes[ci][y * strides[ci]..y * strides[ci] + vw]);
        }
        out[ci] = p;
    }
    Ok(Picture { width: width as u32, height: height as u32, chroma, planes: out })
}
