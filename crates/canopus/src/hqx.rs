// Ported from FFmpeg n7.1.1 libavcodec/hqx.c, hqxdsp.c and hqxvlc.c (Canopus HQX decoder),
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Canopus HQX frames: 4:2:2 or 4:4:4, optionally with alpha, 12-bit output from 16×16
//! macroblocks of 8×8 DCT blocks (DPCM-coded DC, run/level AC by quantiser class), in 16 slices
//! whose macroblocks are spread over the picture in tiles.

use crate::bits::{Bits, Vlc, ZIGZAG};
use crate::tables::*;
use crate::{Error, Layout, Picture, Planes, Result};

const HEADER_SIZE: usize = 59;

pub(crate) const QUANTS: [[i32; 4]; 16] = [
    [0x1, 0x2, 0x4, 0x8],
    [0x1, 0x3, 0x6, 0xC],
    [0x2, 0x4, 0x8, 0x10],
    [0x3, 0x6, 0xC, 0x18],
    [0x4, 0x8, 0x10, 0x20],
    [0x6, 0xC, 0x18, 0x30],
    [0x8, 0x10, 0x20, 0x40],
    [0xA, 0x14, 0x28, 0x50],
    [0xC, 0x18, 0x30, 0x60],
    [0x10, 0x20, 0x40, 0x80],
    [0x18, 0x30, 0x60, 0xC0],
    [0x20, 0x40, 0x80, 0x100],
    [0x30, 0x60, 0xC0, 0x180],
    [0x40, 0x80, 0x100, 0x200],
    [0x60, 0xC0, 0x180, 0x300],
    [0x80, 0x100, 0x200, 0x400],
];

#[rustfmt::skip]
const QUANT_LUMA: [u8; 64] = [
    16,  16,  16,  19,  19,  19,  42,  44,
    16,  16,  19,  19,  19,  38,  43,  45,
    16,  19,  19,  19,  40,  41,  45,  48,
    19,  19,  19,  40,  41,  42,  46,  49,
    19,  19,  40,  41,  42,  43,  48, 101,
    19,  38,  41,  42,  43,  44,  98, 104,
    42,  43,  45,  46,  48,  98, 109, 116,
    44,  45,  48,  49, 101, 104, 116, 123,
];

#[rustfmt::skip]
const QUANT_CHROMA: [u8; 64] = [
    16,  16,  19,  25,  26,  26,  42,  44,
    16,  19,  25,  25,  26,  38,  43,  91,
    19,  25,  26,  27,  40,  41,  91,  96,
    25,  25,  27,  40,  41,  84,  93, 197,
    26,  26,  40,  41,  84,  86, 191, 203,
    26,  38,  41,  84,  86, 177, 197, 209,
    42,  43,  91,  93, 191, 197, 219, 232,
    44,  91,  96, 197, 203, 209, 232, 246,
];

const SHUFFLE_16: [usize; 16] = [0, 5, 11, 14, 2, 7, 9, 13, 1, 4, 10, 15, 3, 6, 8, 12];

/// An AC lookup: `lut_bits`, `extra_bits` and the expanded `(lev, run, bits)` table.
struct Ac {
    lut_bits: u32,
    extra_bits: u32,
    lut: Vec<(i16, u8, i8)>,
}

impl Ac {
    fn new(lut_bits: u32, extra_bits: u32, rle: &[(u16, i16, u8, i8)]) -> Self {
        let lut = rle.iter().flat_map(|&(n, lev, run, bits)| std::iter::repeat_n((lev, run, bits), usize::from(n))).collect();
        Self { lut_bits, extra_bits, lut }
    }

    /// `hqx_get_ac`: `(run, lev)`.
    fn read(&self, b: &mut Bits) -> (usize, i32) {
        let mut val = b.peek(self.lut_bits) as usize;
        let mut e = self.lut.get(val).copied().unwrap_or((0, 64, 0));
        if e.2 == -1 {
            let extra = (b.peek(self.lut_bits + self.extra_bits) & ((1 << self.extra_bits) - 1)) as usize;
            val = (i32::from(e.0) as usize).wrapping_add(extra);
            e = self.lut.get(val).copied().unwrap_or((0, 64, 0));
        }
        b.skip(u32::from(e.2.max(0) as u8));
        (usize::from(e.1), i32::from(e.0))
    }
}

pub(crate) struct Tables {
    cbp: Vlc,
    dc: [Vlc; 3],
    ac: [Ac; 6],
}

impl Tables {
    pub(crate) fn new() -> Self {
        let cbp_codes: Vec<u16> = HQA_CBP_BITS.iter().map(|&c| u16::from(c)).collect();
        Self {
            cbp: Vlc::new(&cbp_codes, &HQA_CBP_LENS),
            dc: [Vlc::new(&HQX_DC9_BITS, &HQX_DC9_LENS), Vlc::new(&HQX_DC10_BITS, &HQX_DC10_LENS), Vlc::new(&HQX_DC11_BITS, &HQX_DC11_LENS)],
            ac: [
                Ac::new(10, 5, &HQX_AC0_LUT),
                Ac::new(11, 6, &HQX_AC8_LUT),
                Ac::new(11, 6, &HQX_AC16_LUT),
                Ac::new(11, 6, &HQX_AC32_LUT),
                Ac::new(12, 5, &HQX_AC64_LUT),
                Ac::new(11, 6, &HQX_AC128_LUT),
            ],
        }
    }

    #[cfg(test)]
    pub(crate) fn cbp_read(&self, b: &mut Bits) -> Option<usize> {
        self.cbp.read(b)
    }

    #[cfg(test)]
    pub(crate) fn dc_read(&self, dcb: u32, b: &mut Bits) -> Option<usize> {
        self.dc.get(dcb as usize - 9)?.read(b)
    }

    /// `(run, lev)` of the AC table for quantiser `q`.
    #[cfg(test)]
    pub(crate) fn ac_read(&self, q: i32, b: &mut Bits) -> (usize, i32) {
        self.ac.get(ac_class(q)).map_or((64, 0), |a| a.read(b))
    }
}

fn ac_class(q: i32) -> usize {
    match q {
        128.. => 5,
        64.. => 4,
        32.. => 3,
        16.. => 2,
        8.. => 1,
        _ => 0,
    }
}

/// 12-bit planes of the coded (16-aligned) size.
struct Frame {
    planes: Vec<Vec<u16>>,
    widths: Vec<usize>,
    height: usize,
}

impl Frame {
    #[allow(clippy::too_many_arguments)]
    fn put_blocks(&mut self, plane: usize, x: usize, y: usize, ilace: bool, b0: &mut [i16; 64], b1: &mut [i16; 64], quant: &[u8; 64]) {
        let step = if ilace { 2 } else { 1 };
        self.idct_put(plane, x, y, step, b0, quant);
        self.idct_put(plane, x, y + if ilace { 1 } else { 8 }, step, b1, quant);
    }

    fn idct_put(&mut self, plane: usize, x: usize, y: usize, step: usize, block: &mut [i16; 64], quant: &[u8; 64]) {
        idct(block, quant);
        let (Some(p), Some(&w)) = (self.planes.get_mut(plane), self.widths.get(plane)) else { return };
        for r in 0..8 {
            let py = y + r * step;
            if py >= self.height {
                break;
            }
            for c in 0..8 {
                let px = x + c;
                if px >= w {
                    break;
                }
                if let (Some(d), Some(&v)) = (p.get_mut(py * w + px), block.get(r * 8 + c)) {
                    *d = (i32::from(v) + 0x800).clamp(0, 4095) as u16;
                }
            }
        }
    }
}

fn m(a: i32, k: u32) -> u32 {
    (a as u32).wrapping_mul(k)
}

/// `hqx_idct_put`'s transform: dequantised columns, then rows, stored back to 16 bits.
fn idct(blk: &mut [i16; 64], quant: &[u8; 64]) {
    for i in 0..8 {
        let s = |k: usize| i32::from(blk[k * 8 + i]) * i32::from(quant[k * 8 + i]);
        let (s0, s1, s2, s3, s4, s5, s6, s7) = (s(0), s(1), s(2), s(3), s(4), s(5), s(6), s(7));
        let t0 = (m(s3, 19266).wrapping_add(m(s5, 12873)) as i32) >> 15;
        let t1 = (m(s5, 19266).wrapping_sub(m(s3, 12873)) as i32) >> 15;
        let t2 = ((m(s7, 4520).wrapping_add(m(s1, 22725)) as i32) >> 15).wrapping_sub(t0);
        let t3 = ((m(s1, 4520).wrapping_sub(m(s7, 22725)) as i32) >> 15).wrapping_sub(t1);
        let t4 = t0.wrapping_mul(2).wrapping_add(t2);
        let t5 = t1.wrapping_mul(2).wrapping_add(t3);
        let t6 = t2.wrapping_sub(t3);
        let t7 = t3.wrapping_mul(2).wrapping_add(t6);
        let t8 = (m(t6, 11585) as i32) >> 14;
        let t9 = (m(t7, 11585) as i32) >> 14;
        let ta = (m(s2, 8867).wrapping_sub(m(s6, 21407)) as i32) >> 14;
        let tb = (m(s6, 8867).wrapping_add(m(s2, 21407)) as i32) >> 14;
        let tc = (s0 >> 1).wrapping_sub(s4 >> 1);
        let td = (s4 >> 1).wrapping_mul(2).wrapping_add(tc);
        let te = tc.wrapping_sub(ta >> 1);
        let tf = td.wrapping_sub(tb >> 1);
        let t10 = tf.wrapping_sub(t5);
        let t11 = te.wrapping_sub(t8);
        let t12 = te.wrapping_add((ta >> 1).wrapping_mul(2)).wrapping_sub(t9);
        let t13 = tf.wrapping_add((tb >> 1).wrapping_mul(2)).wrapping_sub(t4);
        let out = [
            t13.wrapping_add(t4.wrapping_mul(2)),
            t12.wrapping_add(t9.wrapping_mul(2)),
            t11.wrapping_add(t8.wrapping_mul(2)),
            t10.wrapping_add(t5.wrapping_mul(2)),
            t10,
            t11,
            t12,
            t13,
        ];
        for (k, v) in out.into_iter().enumerate() {
            blk[k * 8 + i] = v as i16;
        }
    }
    for row in blk.chunks_exact_mut(8) {
        let b = |i: usize| i32::from(row[i]);
        let t0 = (b(3) * 19266 + b(5) * 12873) >> 14;
        let t1 = (b(5) * 19266 - b(3) * 12873) >> 14;
        let t2 = ((b(7) * 4520 + b(1) * 22725) >> 14) - t0;
        let t3 = ((b(1) * 4520 - b(7) * 22725) >> 14) - t1;
        let t4 = t0 * 2 + t2;
        let t5 = t1 * 2 + t3;
        let t6 = t2 - t3;
        let t7 = t3 * 2 + t6;
        let t8 = (t6 * 11585) >> 14;
        let t9 = (t7 * 11585) >> 14;
        let ta = (b(2) * 8867 - b(6) * 21407) >> 14;
        let tb = (b(6) * 8867 + b(2) * 21407) >> 14;
        let tc = b(0) - b(4);
        let td = b(4) * 2 + tc;
        let te = tc - ta;
        let tf = td - tb;
        let t10 = tf - t5;
        let t11 = te - t8;
        let t12 = te + ta * 2 - t9;
        let t13 = tf + tb * 2 - t4;
        let out = [t13 + t4 * 2, t12 + t9 * 2, t11 + t8 * 2, t10 + t5 * 2, t10, t11, t12, t13];
        for (d, v) in row.iter_mut().zip(out) {
            *d = ((v + 4) >> 3) as i16;
        }
    }
}

struct Ctx<'t> {
    t: &'t Tables,
    interlaced: bool,
    dcb: u32,
    format: u8,
    blocks: [[i16; 64]; 16],
}

impl Ctx<'_> {
    fn decode_block(&mut self, b: &mut Bits, quants: &[i32; 4], i: usize, last_dc: &mut i32) {
        let t = self.t;
        let Some(block) = self.blocks.get_mut(i) else { return };
        *block = [0; 64];
        let dc = t.dc.get(self.dcb as usize - 9).and_then(|v| v.read(b)).map_or(-1, |v| v as i32);
        *last_dc = last_dc.wrapping_add(dc);
        block[0] = ((((*last_dc as u32) << (12 - self.dcb)) << 20) as i32 >> 20) as i16;
        let q = quants.get(b.get(2) as usize).copied().unwrap_or(0);
        let Some(ac) = t.ac.get(ac_class(q)) else { return };
        let mut pos = 1usize;
        while pos < 64 {
            let (run, lev) = ac.read(b);
            pos += run;
            if pos >= 64 {
                break;
            }
            if let Some(&z) = ZIGZAG.get(pos) {
                block[z] = lev.wrapping_mul(q) as i16;
            }
            pos += 1;
        }
    }

    fn put(&mut self, f: &mut Frame, plane: usize, x: usize, y: usize, flag: bool, i0: usize, i1: usize, quant: &[u8; 64]) {
        let (mut b0, mut b1) = (self.blocks.get(i0).copied().unwrap_or([0; 64]), self.blocks.get(i1).copied().unwrap_or([0; 64]));
        f.put_blocks(plane, x, y, flag, &mut b0, &mut b1, quant);
    }

    fn decode_mb(&mut self, f: &mut Frame, b: &mut Bits, x: usize, y: usize) {
        let alpha = self.format & 2 != 0;
        let full = self.format & 1 != 0;
        let n = match (full, alpha) {
            (false, false) => 8,
            (true, false) | (false, true) => 12,
            (true, true) => 16,
        };
        let mut flag = false;
        if alpha {
            let mut cbp = self.t.cbp.read(b).map_or(-1i32, |v| v as i32);
            for blk in self.blocks.iter_mut().take(n) {
                *blk = [0; 64];
                blk[0] = -0x800;
            }
            if cbp != 0 {
                if self.interlaced {
                    flag = b.bit();
                }
                let quants = QUANTS.get(b.get(4) as usize).copied().unwrap_or([0; 4]);
                cbp |= cbp << 4;
                if full {
                    cbp |= cbp << 8;
                } else {
                    if cbp & 0x3 != 0 {
                        cbp |= 0x500;
                    }
                    if cbp & 0xC != 0 {
                        cbp |= 0xA00;
                    }
                }
                let mut last_dc = 0;
                for i in 0..n {
                    if i % 4 == 0 || (!full && i == 10) {
                        last_dc = 0;
                    }
                    if cbp & (1 << i) != 0 {
                        self.decode_block(b, &quants, i, &mut last_dc);
                    }
                }
            }
        } else {
            if self.interlaced {
                flag = b.bit();
            }
            let quants = QUANTS.get(b.get(4) as usize).copied().unwrap_or([0; 4]);
            let mut last_dc = 0;
            for i in 0..n {
                let reset = if full { i % 4 == 0 } else { i == 0 || i == 4 || i == 6 };
                if reset {
                    last_dc = 0;
                }
                self.decode_block(b, &quants, i, &mut last_dc);
            }
        }
        let (l, c) = (&QUANT_LUMA, &QUANT_CHROMA);
        match (full, alpha) {
            (false, false) => {
                self.put(f, 0, x, y, flag, 0, 2, l);
                self.put(f, 0, x + 8, y, flag, 1, 3, l);
                self.put(f, 2, x >> 1, y, flag, 4, 5, c);
                self.put(f, 1, x >> 1, y, flag, 6, 7, c);
            }
            (false, true) => {
                self.put(f, 3, x, y, flag, 0, 2, l);
                self.put(f, 3, x + 8, y, flag, 1, 3, l);
                self.put(f, 0, x, y, flag, 4, 6, l);
                self.put(f, 0, x + 8, y, flag, 5, 7, l);
                self.put(f, 2, x >> 1, y, flag, 8, 9, c);
                self.put(f, 1, x >> 1, y, flag, 10, 11, c);
            }
            (true, false) => {
                self.put(f, 0, x, y, flag, 0, 2, l);
                self.put(f, 0, x + 8, y, flag, 1, 3, l);
                self.put(f, 2, x, y, flag, 4, 6, c);
                self.put(f, 2, x + 8, y, flag, 5, 7, c);
                self.put(f, 1, x, y, flag, 8, 10, c);
                self.put(f, 1, x + 8, y, flag, 9, 11, c);
            }
            (true, true) => {
                self.put(f, 3, x, y, flag, 0, 2, l);
                self.put(f, 3, x + 8, y, flag, 1, 3, l);
                self.put(f, 0, x, y, flag, 4, 6, l);
                self.put(f, 0, x + 8, y, flag, 5, 7, l);
                self.put(f, 2, x, y, flag, 8, 10, c);
                self.put(f, 2, x + 8, y, flag, 9, 11, c);
                self.put(f, 1, x, y, flag, 12, 14, c);
                self.put(f, 1, x + 8, y, flag, 13, 15, c);
            }
        }
    }

    /// `decode_slice`: the macroblocks of slice `slice_no`, spread over the picture in tiles.
    fn decode_slice(&mut self, f: &mut Frame, b: &mut Bits, slice_no: usize, width: usize, height: usize) {
        let mb_w = width.div_ceil(16);
        let mb_h = height.div_ceil(16);
        let grp_w = mb_w.div_ceil(5);
        let grp_h = mb_h.div_ceil(5);
        let (Some(h_edge), Some(v_edge)) = (mb_w.checked_div(grp_w), mb_h.checked_div(grp_h)) else { return };
        let grp_h_edge = grp_w * h_edge;
        let grp_v_edge = grp_h * v_edge;
        let grp_v_rest = mb_w - grp_h_edge;
        let grp_h_rest = mb_h - grp_v_edge;
        let num_mbs = mb_w * mb_h;
        let num_tiles = num_mbs.div_ceil(480);
        let Some(std_tile_blocks) = num_mbs.checked_div(16 * num_tiles) else { return };
        let mut g_tile = slice_no * num_tiles;
        for tile_no in 0..num_tiles {
            let mut tile_blocks = std_tile_blocks;
            let mut tile_limit = None;
            if g_tile < num_mbs - std_tile_blocks * 16 * num_tiles {
                tile_limit = Some(std_tile_blocks);
                tile_blocks += 1;
            }
            for i in 0..tile_blocks {
                let blk_addr = if tile_limit == Some(i) {
                    g_tile + 16 * num_tiles * i
                } else {
                    tile_no + 16 * num_tiles * i + num_tiles * SHUFFLE_16[(i + slice_no) & 0xF]
                };
                let Some(row) = blk_addr.checked_div(grp_h * mb_w) else { return };
                let loc_row = grp_h * row;
                let loc_addr = blk_addr % (grp_h * mb_w);
                let span = if loc_row >= grp_v_edge { grp_h_rest * grp_w } else { grp_h * grp_w };
                let (Some(col), Some(pos)) = (loc_addr.checked_div(span), loc_addr.checked_rem(span)) else { continue };
                let mb_x = grp_w * col;
                let across = if mb_x >= grp_h_edge { grp_v_rest } else { grp_w };
                let (Some(dx), Some(dy)) = (pos.checked_rem(across), pos.checked_div(across)) else { continue };
                self.decode_mb(f, b, (mb_x + dx) * 16, (loc_row + dy) * 16);
            }
            g_tile += 1;
        }
    }
}

fn be(d: &[u8], at: usize, n: usize) -> u32 {
    (0..n).fold(0u32, |v, i| (v << 8) | u32::from(d.get(at + i).copied().unwrap_or(0)))
}

/// An HQX frame; `src` starts at the `HQ` signature.
pub(crate) fn decode(t: &Tables, src: &[u8], packet_size: usize) -> Result<Picture> {
    if src.len() < HEADER_SIZE {
        return Err(Error::Truncated);
    }
    if src.get(..2) != Some(b"HQ") {
        return Err(Error::Invalid("not an HQX frame"));
    }
    let (b2, b3) = (src.get(2).copied().unwrap_or(0), src.get(3).copied().unwrap_or(0));
    let interlaced = b2 & 0x80 == 0;
    let format = b2 & 7;
    let dcb = u32::from(b3 & 3) + 8;
    let (width, height) = (be(src, 4, 2), be(src, 6, 2));
    let offs: Vec<u32> = (0..17).map(|i| be(src, 8 + i * 3, 3)).collect();
    if dcb == 8 {
        return Err(Error::Invalid("DC precision"));
    }
    crate::check_size(width, height, packet_size)?;
    let layout = match format {
        0 => Layout::Yuv422,
        1 => Layout::Yuv444,
        2 => Layout::Yuva422,
        3 => Layout::Yuva444,
        _ => return Err(Error::Unsupported("HQX format")),
    };
    let (cw, ch) = (width.next_multiple_of(16) as usize, height.next_multiple_of(16) as usize);
    let full = format & 1 != 0;
    let chroma_w = if full { cw } else { cw / 2 };
    let mut widths = vec![cw, chroma_w, chroma_w];
    if format & 2 != 0 {
        widths.push(cw);
    }
    let mut f = Frame { planes: widths.iter().map(|&w| vec![0u16; w * ch]).collect(), widths, height: ch };
    let mut ctx = Ctx { t, interlaced, dcb, format, blocks: [[0; 64]; 16] };
    for slice in 0..16 {
        let (Some(&a), Some(&z)) = (offs.get(slice), offs.get(slice + 1)) else { break };
        if (a as usize) < HEADER_SIZE || a >= z || z as usize > src.len() {
            continue;
        }
        let mut b = Bits::new(src.get(a as usize..z as usize).unwrap_or_default());
        ctx.decode_slice(&mut f, &mut b, slice, width as usize, height as usize);
    }
    let (w, h) = (width as usize, height as usize);
    let pcw = if full { w } else { w.div_ceil(2) };
    let planes = f
        .planes
        .iter()
        .zip(&f.widths)
        .enumerate()
        .map(|(i, (p, &stride))| {
            let pw = if i == 1 || i == 2 { pcw } else { w };
            (0..h).flat_map(|y| p.get(y * stride..y * stride + pw).unwrap_or_default().iter().copied()).collect()
        })
        .collect();
    Ok(Picture { width, height, layout, planes: Planes::U16(planes) })
}
