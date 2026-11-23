//! The AMV video decoder: FFmpeg's `sp5x_decode_frame` for AMV wraps the packet in a fixed JPEG
//! header (two quantisation tables of quality 60, the standard Huffman tables, a 4:2:0 baseline
//! frame) and hands it to the MJPEG decoder with the picture flipped upside down. The packet is
//! the entropy-coded scan between its first two and last two bytes.

use crate::Error;
use crate::tables::{DHT, QUANT};

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The most pixels along a side.
const MAX_SIDE: u32 = 16384;

/// One decoded picture, 4:2:0, full range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Y (`width` x `height`), Cb and Cr (`ceil(width / 2)` x `ceil(height / 2)`).
    pub planes: [Vec<u8>; 3],
}

/// A canonical Huffman table (JPEG annex C).
#[derive(Clone)]
struct Huff {
    /// First code of each length, the number of codes up to and excluding that length, and the
    /// largest code of each length (-1: none).
    mincode: [i32; 17],
    maxcode: [i32; 18],
    valptr: [usize; 17],
    vals: Vec<u8>,
}

impl Huff {
    fn new(counts: &[u8], vals: &[u8]) -> Self {
        let mut h = Huff { mincode: [0; 17], maxcode: [-1; 18], valptr: [0; 17], vals: vals.to_vec() };
        let (mut code, mut k) = (0i32, 0usize);
        for len in 1..=16 {
            let n = usize::from(counts.get(len - 1).copied().unwrap_or(0));
            h.valptr[len] = k;
            h.mincode[len] = code;
            if n > 0 {
                code += n as i32;
                k += n;
                h.maxcode[len] = code - 1;
            }
            code <<= 1;
        }
        h
    }
}

/// The scan's bits, with the padding zeros past its end counted.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> u32 {
        let b = self.d.get(self.pos >> 3).map_or(0, |&x| u32::from(x >> (7 - (self.pos & 7)) & 1));
        self.pos += 1;
        b
    }
    fn get(&mut self, n: u32) -> u32 {
        (0..n).fold(0, |a, _| a << 1 | self.bit())
    }
    fn overread(&self) -> bool {
        self.pos > self.d.len() * 8
    }
    fn symbol(&mut self, h: &Huff) -> Option<u8> {
        let mut code = 0i32;
        for len in 1..=16usize {
            code = code << 1 | self.bit() as i32;
            if h.maxcode[len] >= 0 && code <= h.maxcode[len] && code >= h.mincode[len] {
                return h.vals.get(h.valptr[len] + (code - h.mincode[len]) as usize).copied();
            }
        }
        None
    }
}

/// FFmpeg's `ff_mjpeg_unescape_sos`: `ff00` becomes `ff`, restart markers stay, any other marker
/// ends the data.
fn unescape(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len());
    let mut i = 0;
    while i < d.len() {
        let mut x = d[i];
        i += 1;
        out.push(x);
        if x == 0xff {
            while i < d.len() && x == 0xff {
                x = d[i];
                i += 1;
            }
            if (0xd0..=0xd7).contains(&x) {
                out.push(x);
            } else if x != 0 {
                break;
            }
        }
    }
    out
}

/// FFmpeg's `simple_idct` (8-bit, C reference) of `block` into `dst` at `stride`.
fn idct_put(block: &mut [i16; 64], dst: &mut [u8], stride: usize) {
    const W1: u32 = 22725;
    const W2: u32 = 21407;
    const W3: u32 = 19266;
    const W4: u32 = 16383;
    const W5: u32 = 12873;
    const W6: u32 = 8867;
    const W7: u32 = 4520;
    for r in 0..8 {
        let row = &mut block[r * 8..r * 8 + 8];
        if row[1..].iter().all(|&v| v == 0) {
            let t = (i32::from(row[0]) << 3) as i16;
            row.fill(t);
            continue;
        }
        let v = |i: usize| i32::from(row[i]) as u32;
        let mut a0 = W4.wrapping_mul(v(0)).wrapping_add(1 << 10);
        let mut a1 = a0;
        let mut a2 = a0;
        let mut a3 = a0;
        a0 = a0.wrapping_add(W2.wrapping_mul(v(2)));
        a1 = a1.wrapping_add(W6.wrapping_mul(v(2)));
        a2 = a2.wrapping_sub(W6.wrapping_mul(v(2)));
        a3 = a3.wrapping_sub(W2.wrapping_mul(v(2)));
        let mut b0 = W1.wrapping_mul(v(1)).wrapping_add(W3.wrapping_mul(v(3)));
        let mut b1 = W3.wrapping_mul(v(1)).wrapping_sub(W7.wrapping_mul(v(3)));
        let mut b2 = W5.wrapping_mul(v(1)).wrapping_sub(W1.wrapping_mul(v(3)));
        let mut b3 = W7.wrapping_mul(v(1)).wrapping_sub(W5.wrapping_mul(v(3)));
        if row[4..].iter().any(|&x| x != 0) {
            a0 = a0.wrapping_add(W4.wrapping_mul(v(4))).wrapping_add(W6.wrapping_mul(v(6)));
            a1 = a1.wrapping_sub(W4.wrapping_mul(v(4))).wrapping_sub(W2.wrapping_mul(v(6)));
            a2 = a2.wrapping_sub(W4.wrapping_mul(v(4))).wrapping_add(W2.wrapping_mul(v(6)));
            a3 = a3.wrapping_add(W4.wrapping_mul(v(4))).wrapping_sub(W6.wrapping_mul(v(6)));
            b0 = b0.wrapping_add(W5.wrapping_mul(v(5))).wrapping_add(W7.wrapping_mul(v(7)));
            b1 = b1.wrapping_sub(W1.wrapping_mul(v(5))).wrapping_sub(W5.wrapping_mul(v(7)));
            b2 = b2.wrapping_add(W7.wrapping_mul(v(5))).wrapping_add(W3.wrapping_mul(v(7)));
            b3 = b3.wrapping_add(W3.wrapping_mul(v(5))).wrapping_sub(W1.wrapping_mul(v(7)));
        }
        let s = |x: u32| ((x as i32) >> 11) as i16;
        row[0] = s(a0.wrapping_add(b0));
        row[7] = s(a0.wrapping_sub(b0));
        row[1] = s(a1.wrapping_add(b1));
        row[6] = s(a1.wrapping_sub(b1));
        row[2] = s(a2.wrapping_add(b2));
        row[5] = s(a2.wrapping_sub(b2));
        row[3] = s(a3.wrapping_add(b3));
        row[4] = s(a3.wrapping_sub(b3));
    }
    for c in 0..8 {
        let col = |i: usize| i32::from(block[8 * i + c]) as u32;
        let mut a0 = W4.wrapping_mul(col(0).wrapping_add(32));
        let mut a1 = a0;
        let mut a2 = a0;
        let mut a3 = a0;
        a0 = a0.wrapping_add(W2.wrapping_mul(col(2)));
        a1 = a1.wrapping_add(W6.wrapping_mul(col(2)));
        a2 = a2.wrapping_sub(W6.wrapping_mul(col(2)));
        a3 = a3.wrapping_sub(W2.wrapping_mul(col(2)));
        let mut b0 = W1.wrapping_mul(col(1)).wrapping_add(W3.wrapping_mul(col(3)));
        let mut b1 = W3.wrapping_mul(col(1)).wrapping_sub(W7.wrapping_mul(col(3)));
        let mut b2 = W5.wrapping_mul(col(1)).wrapping_sub(W1.wrapping_mul(col(3)));
        let mut b3 = W7.wrapping_mul(col(1)).wrapping_sub(W5.wrapping_mul(col(3)));
        a0 = a0.wrapping_add(W4.wrapping_mul(col(4)));
        a1 = a1.wrapping_sub(W4.wrapping_mul(col(4)));
        a2 = a2.wrapping_sub(W4.wrapping_mul(col(4)));
        a3 = a3.wrapping_add(W4.wrapping_mul(col(4)));
        b0 = b0.wrapping_add(W5.wrapping_mul(col(5)));
        b1 = b1.wrapping_sub(W1.wrapping_mul(col(5)));
        b2 = b2.wrapping_add(W7.wrapping_mul(col(5)));
        b3 = b3.wrapping_add(W3.wrapping_mul(col(5)));
        a0 = a0.wrapping_add(W6.wrapping_mul(col(6)));
        a1 = a1.wrapping_sub(W2.wrapping_mul(col(6)));
        a2 = a2.wrapping_add(W2.wrapping_mul(col(6)));
        a3 = a3.wrapping_sub(W6.wrapping_mul(col(6)));
        b0 = b0.wrapping_add(W7.wrapping_mul(col(7)));
        b1 = b1.wrapping_sub(W5.wrapping_mul(col(7)));
        b2 = b2.wrapping_add(W3.wrapping_mul(col(7)));
        b3 = b3.wrapping_sub(W1.wrapping_mul(col(7)));
        let px = |x: u32| ((x as i32) >> 20).clamp(0, 255) as u8;
        let outs = [
            a0.wrapping_add(b0),
            a1.wrapping_add(b1),
            a2.wrapping_add(b2),
            a3.wrapping_add(b3),
            a3.wrapping_sub(b3),
            a2.wrapping_sub(b2),
            a1.wrapping_sub(b1),
            a0.wrapping_sub(b0),
        ];
        for (i, o) in outs.into_iter().enumerate() {
            if let Some(p) = dst.get_mut(i * stride + c) {
                *p = px(o);
            }
        }
    }
}

/// An AMV frame decoder for pictures of one size.
#[derive(Clone)]
pub struct Decoder {
    width: u32,
    height: u32,
    dc: [Huff; 2],
    ac: [Huff; 2],
}

impl Decoder {
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::Invalid(format!("picture size {width}x{height}")));
        }
        // the fixed DHT segment: four tables, each a class/id byte, 16 counts and the symbols
        let mut p = 0;
        let mut tables: Vec<Huff> = Vec::new();
        while p + 17 <= DHT.len() {
            let counts = &DHT[p + 1..p + 17];
            let n: usize = counts.iter().map(|&c| usize::from(c)).sum();
            let vals = DHT.get(p + 17..p + 17 + n).ok_or_else(|| Error::Invalid("table".into()))?;
            tables.push(Huff::new(counts, vals));
            p += 17 + n;
        }
        let mut it = tables.into_iter();
        let (Some(dc0), Some(dc1), Some(ac0), Some(ac1)) = (it.next(), it.next(), it.next(), it.next()) else {
            return Err(Error::Invalid("tables".into()));
        };
        Ok(Self { width, height, dc: [dc0, dc1], ac: [ac0, ac1] })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Decodes one video chunk.
    pub fn decode(&self, packet: &[u8]) -> Result<Picture, Error> {
        let scan = packet.get(2..packet.len().saturating_sub(2)).unwrap_or(&[]);
        let data = unescape(scan);
        let (w, h) = (self.width as usize, self.height as usize);
        let (mcux, mcuy) = (w.div_ceil(16), h.div_ceil(16));
        let (ys, cs) = (mcux * 16, mcux * 8);
        let mut y = vec![0u8; ys * mcuy * 16];
        let mut cb = vec![0u8; cs * mcuy * 8];
        let mut cr = vec![0u8; cs * mcuy * 8];
        let mut br = Bits { d: &data, pos: 0 };
        // the DC predictors start at the level shift (4 << 8), already in the dequantised scale
        let mut last = [1024i32; 3];
        for my in 0..mcuy {
            for mx in 0..mcux {
                for blk in 0..6usize {
                    let comp = if blk < 4 { 0 } else { blk - 3 };
                    let t = usize::from(comp != 0);
                    let mut block = [0i16; 64];
                    self.block(&mut br, &mut block, &mut last[comp], t)?;
                    match blk {
                        0..=3 => {
                            let off = (my * 16 + (blk >> 1) * 8) * ys + mx * 16 + (blk & 1) * 8;
                            idct_put(&mut block, y.get_mut(off..).unwrap_or(&mut []), ys);
                        }
                        _ => {
                            let off = my * 8 * cs + mx * 8;
                            let plane = if blk == 4 { &mut cb } else { &mut cr };
                            idct_put(&mut block, plane.get_mut(off..).unwrap_or(&mut []), cs);
                        }
                    }
                }
                if br.overread() {
                    return Err(Error::Invalid("scan data ends early".into()));
                }
            }
        }
        // the picture is stored upside down
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let flip = |src: &[u8], stride: usize, cw: usize, ch: usize| -> Vec<u8> {
            (0..ch).flat_map(|r| src[(ch - 1 - r) * stride..(ch - 1 - r) * stride + cw].iter().copied()).collect()
        };
        Ok(Picture { width: self.width, height: self.height, planes: [flip(&y, ys, w, h), flip(&cb, cs, cw, ch), flip(&cr, cs, cw, ch)] })
    }

    /// FFmpeg's `decode_block`: one 8x8 block, dequantised.
    fn block(&self, br: &mut Bits<'_>, block: &mut [i16; 64], last: &mut i32, t: usize) -> Result<(), Error> {
        let q = &QUANT[t];
        let code = br.symbol(&self.dc[t]).ok_or_else(|| Error::Invalid("bad DC code".into()))?;
        if code > 16 {
            return Err(Error::Invalid("bad DC code".into()));
        }
        let diff = if code == 0 {
            0
        } else {
            let v = br.get(u32::from(code)) as i32;
            if v < 1 << (code - 1) { v - ((1 << code) - 1) } else { v }
        };
        let v = (diff as u32).wrapping_mul(u32::from(q[0])).wrapping_add(*last as u32) as i32;
        *last = v;
        block[0] = v.clamp(-32768, 32767) as i16;
        let mut i = 0usize;
        loop {
            let rs = br.symbol(&self.ac[t]).ok_or_else(|| Error::Invalid("bad AC code".into()))?;
            let size = u32::from(rs & 15);
            // end of block jumps past the end, as FFmpeg's table does
            i += if rs == 0 { 256 } else { usize::from(rs >> 4) + 1 };
            if size != 0 {
                let v = br.get(size) as i32;
                let level = if v < 1 << (size - 1) { v - ((1 << size) - 1) } else { v };
                if i > 63 {
                    return Err(Error::Invalid("coefficient index past the block".into()));
                }
                block[usize::from(ZIGZAG[i])] = level.wrapping_mul(i32::from(q[i])) as i16;
            }
            if i >= 63 {
                break;
            }
        }
        Ok(())
    }
}
