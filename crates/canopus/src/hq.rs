// Ported from FFmpeg n7.1.1 libavcodec/hq_hqa.c and hq_hqadsp.c (Canopus HQ/HQA decoder),
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Canopus HQ (`UVC` + profile) and HQA (`HQA1`) frames: 8-bit 4:2:2 (HQA with alpha), 16×16
//! macroblocks of 8×8 DCT blocks, AC run/level VLC, per-position quantiser matrices, and
//! slices whose macroblocks follow a profile's permutation table (HQ) or every eighth
//! macroblock (HQA).

use crate::bits::{Bits, Vlc, ZIGZAG};
use crate::tables::*;
use crate::{Error, Layout, Picture, Planes, Result};

pub(crate) struct Tables {
    ac: Vlc,
    cbp: Vlc,
}

impl Tables {
    pub(crate) fn new() -> Self {
        let cbp_codes: Vec<u16> = HQA_CBP_BITS.iter().map(|&c| u16::from(c)).collect();
        Self { ac: Vlc::new(&HQ_AC_CODES, &HQ_AC_BITS), cbp: Vlc::new(&cbp_codes, &HQA_CBP_LENS) }
    }

    #[cfg(test)]
    pub(crate) fn ac_read(&self, b: &mut Bits<'_>) -> Option<usize> {
        self.ac.read(b)
    }

    #[cfg(test)]
    pub(crate) fn cbp_read(&self, b: &mut Bits<'_>) -> Option<usize> {
        self.cbp.read(b)
    }
}

/// 8-bit planes of the coded (16-aligned) size.
struct Frame {
    planes: Vec<Vec<u8>>,
    widths: Vec<usize>,
    height: usize,
}

impl Frame {
    fn put_blocks(&mut self, plane: usize, x: usize, y: usize, ilace: bool, b0: &mut [i16; 64], b1: &mut [i16; 64]) {
        let step = if ilace { 2 } else { 1 };
        self.idct_put(plane, x, y, step, b0);
        self.idct_put(plane, x, y + if ilace { 1 } else { 8 }, step, b1);
    }

    fn idct_put(&mut self, plane: usize, x: usize, y: usize, step: usize, block: &mut [i16; 64]) {
        idct(block);
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
                    *d = v.clamp(0, 255) as u8;
                }
            }
        }
    }
}

const FIX_1_082: i32 = 17734;
const FIX_1_847: i32 = 30274;
const FIX_1_414: i32 = 23170;
const FIX_2_613: i32 = 21407; // divided by two to fit the range

fn mul(a: i32, b: i32) -> i32 {
    ((a as u32).wrapping_mul(b as u32) as i32) >> 16
}

/// `hq_idct_put`'s transform: rows then columns, every pass stored back to 16 bits.
fn idct(blk: &mut [i16; 64]) {
    for row in blk.chunks_exact_mut(8) {
        let b = |i: usize| i32::from(row[i]);
        let tmp0 = b(5).wrapping_sub(b(3));
        let tmp1 = b(5).wrapping_add(b(3));
        let tmp2 = b(1).wrapping_sub(b(7));
        let tmp3 = b(1).wrapping_add(b(7));
        let tmp4 = tmp3.wrapping_sub(tmp1);
        let tmp5 = mul(tmp0.wrapping_add(tmp2), FIX_1_847);
        let tmp6 = mul(tmp2, FIX_1_082).wrapping_sub(tmp5);
        let tmp7 = tmp5.wrapping_sub(mul(tmp0, FIX_2_613).wrapping_mul(2));
        let tmp8 = tmp3.wrapping_add(tmp1);
        let tmp9 = tmp7.wrapping_mul(4).wrapping_sub(tmp8);
        let tmpa = mul(tmp4, FIX_1_414).wrapping_mul(4).wrapping_sub(tmp9);
        let tmpb = tmp6.wrapping_mul(4).wrapping_add(tmpa);
        let tmpc = b(2).wrapping_add(b(6));
        let tmpd = b(2).wrapping_sub(b(6));
        let tmpe = b(0).wrapping_sub(b(4));
        let tmpf = b(0).wrapping_add(b(4));
        let tmp10 = mul(tmpd, FIX_1_414).wrapping_mul(4).wrapping_sub(tmpc);
        let tmp11 = tmpe.wrapping_sub(tmp10);
        let tmp12 = tmpf.wrapping_sub(tmpc);
        let tmp13 = tmpe.wrapping_add(tmp10);
        let tmp14 = tmpf.wrapping_add(tmpc);
        let out = [
            tmp14.wrapping_add(tmp8),
            tmp13.wrapping_add(tmp9),
            tmp11.wrapping_add(tmpa),
            tmp12.wrapping_sub(tmpb),
            tmp12.wrapping_add(tmpb),
            tmp11.wrapping_sub(tmpa),
            tmp13.wrapping_sub(tmp9),
            tmp14.wrapping_sub(tmp8),
        ];
        for (d, v) in row.iter_mut().zip(out) {
            *d = v as i16;
        }
    }
    for i in 0..8 {
        let b = |k: usize| i32::from(blk[k * 8 + i]);
        let tmp0 = b(5).wrapping_sub(b(3));
        let tmp1 = b(5).wrapping_add(b(3));
        let tmp2 = b(1).wrapping_mul(2).wrapping_sub(b(7) >> 2);
        let tmp3 = b(1).wrapping_mul(2).wrapping_add(b(7) >> 2);
        let tmp4 = tmp3.wrapping_sub(tmp1);
        let tmp5 = mul(tmp0.wrapping_add(tmp2), FIX_1_847);
        let tmp6 = mul(tmp2, FIX_1_082).wrapping_sub(tmp5);
        let tmp7 = tmp5.wrapping_sub(mul(tmp0, FIX_2_613).wrapping_mul(2));
        let tmp8 = tmp3.wrapping_add(tmp1) >> 1;
        let tmp9 = tmp7.wrapping_mul(2).wrapping_sub(tmp8);
        let tmpa = mul(tmp4, FIX_1_414).wrapping_mul(2).wrapping_sub(tmp9);
        let tmpb = tmp6.wrapping_mul(2).wrapping_add(tmpa);
        let tmpc = b(2).wrapping_add(b(6) >> 1) >> 1;
        let tmpd = b(2).wrapping_sub(b(6) >> 1);
        let tmpe = (b(0) >> 1).wrapping_sub(b(4) >> 1).wrapping_add(0x2020);
        let tmpf = (b(0) >> 1).wrapping_add(b(4) >> 1).wrapping_add(0x2020);
        let tmp10 = mul(tmpd, FIX_1_414).wrapping_mul(2).wrapping_sub(tmpc);
        let tmp11 = tmpe.wrapping_sub(tmp10);
        let tmp12 = tmpf.wrapping_sub(tmpc);
        let tmp13 = tmpe.wrapping_add(tmp10);
        let tmp14 = tmpf.wrapping_add(tmpc);
        let out = [
            tmp14.wrapping_add(tmp8),
            tmp13.wrapping_add(tmp9),
            tmp11.wrapping_add(tmpa),
            tmp12.wrapping_sub(tmpb),
            tmp12.wrapping_add(tmpb),
            tmp11.wrapping_sub(tmpa),
            tmp13.wrapping_sub(tmp9),
            tmp14.wrapping_sub(tmp8),
        ];
        for (k, v) in out.into_iter().enumerate() {
            blk[k * 8 + i] = (v >> 6) as i16;
        }
    }
}

fn decode_block(t: &Tables, b: &mut Bits<'_>, block: &mut [i16; 64], qsel: usize, chroma: bool, hqa: bool) -> Result<()> {
    *block = [0; 64];
    let quant = |b: &mut Bits<'_>| {
        let m = HQ_QUANTS.get(qsel * 8 + usize::from(chroma) * 4 + b.get(2) as usize).copied().unwrap_or(0);
        let m = usize::from(m) * 64;
        HQ_QMATS.get(m..m + 64)
    };
    let q = if hqa {
        let q = quant(b);
        block[0] = (b.sbits(9) * 64) as i16;
        q
    } else {
        block[0] = (b.sbits(9) * 64) as i16;
        quant(b)
    };
    let q = q.ok_or(Error::Invalid("quantiser"))?;
    let mut pos = 1usize;
    loop {
        let val = t.ac.read(b).ok_or(Error::Invalid("AC code"))?;
        pos += usize::from(HQ_AC_SKIPS.get(val).copied().unwrap_or(64));
        if pos >= 64 {
            break;
        }
        let sym = i32::from(HQ_AC_SYMS.get(val).copied().unwrap_or(0));
        let qv = q.get(pos).copied().unwrap_or(0);
        if let Some(&z) = ZIGZAG.get(pos) {
            block[z] = (((sym as u32).wrapping_mul(qv as u32) as i32) >> 12) as i16;
        }
        pos += 1;
    }
    Ok(())
}

fn decode_mb(t: &Tables, f: &mut Frame, b: &mut Bits<'_>, x: usize, y: usize, blocks: &mut [[i16; 64]; 12]) -> Result<()> {
    let qgroup = b.get(4) as usize;
    let flag = b.bit();
    for (i, blk) in blocks.iter_mut().take(8).enumerate() {
        decode_block(t, b, blk, qgroup, i >= 4, false)?;
    }
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = blocks;
    f.put_blocks(0, x, y, flag, b0, b2);
    f.put_blocks(0, x + 8, y, flag, b1, b3);
    f.put_blocks(2, x >> 1, y, flag, b4, b5);
    f.put_blocks(1, x >> 1, y, flag, b6, b7);
    Ok(())
}

fn be(d: &[u8], at: usize, n: usize) -> u32 {
    (0..n).fold(0u32, |v, i| (v << 8) | u32::from(d.get(at + i).copied().unwrap_or(0)))
}

fn frame(w: usize, h: usize, alpha: bool) -> Frame {
    let mut widths = vec![w, w / 2, w / 2];
    if alpha {
        widths.push(w);
    }
    Frame { planes: widths.iter().map(|&pw| vec![0u8; pw * h]).collect(), widths, height: h }
}

fn finish(f: Frame, width: u32, height: u32, layout: Layout) -> Picture {
    let (w, h) = (width as usize, height as usize);
    let cw = w.div_ceil(2);
    let planes = f
        .planes
        .iter()
        .zip(&f.widths)
        .enumerate()
        .map(|(i, (p, &stride))| {
            let pw = if i == 1 || i == 2 { cw } else { w };
            (0..h).flat_map(|y| p.get(y * stride..y * stride + pw).unwrap_or_default().iter().copied()).collect()
        })
        .collect();
    Picture { width, height, layout, planes: Planes::U8(planes) }
}

/// An HQ frame; `data` starts at the `UVC` tag and `src` is the data after it.
pub(crate) fn decode_hq(t: &Tables, data: &[u8], profile: u8) -> Result<Picture> {
    let src = data.get(4..).unwrap_or_default();
    let data_size = data.len();
    let &(perm_tab, width, height, num_slices, tab_w, tab_h) =
        HQ_PROFILES.get(usize::from(profile)).or(HQ_PROFILES.first()).ok_or(Error::Invalid("profile"))?;
    let (cw, ch) = (width.next_multiple_of(16) as usize, height.next_multiple_of(16) as usize);
    let mut f = frame(cw, ch, false);
    let ns = num_slices as usize;
    let offs: Vec<u32> = (0..=ns).map(|i| be(src, i * 3, 3).wrapping_sub(4)).collect();
    let mut blocks = [[0i16; 64]; 12];
    let mut next_off = 0usize;
    for slice in 0..ns {
        let start_off = next_off;
        next_off = tab_h as usize * (slice + 1) / ns;
        let (Some(&a), Some(&z)) = (offs.get(slice), offs.get(slice + 1)) else { break };
        if (a as usize) < (ns + 1) * 3 || a >= z || z as usize > data_size {
            break;
        }
        let bytes = src.get(a as usize..(z as usize).min(src.len())).unwrap_or_default();
        let mut b = Bits::new(bytes);
        let mut perm = start_off * tab_w as usize * 2;
        for _ in 0..(next_off - start_off) * tab_w as usize {
            let (Some(&px), Some(&py)) = (perm_tab.get(perm), perm_tab.get(perm + 1)) else { return Err(Error::Invalid("permutation")) };
            decode_mb(t, &mut f, &mut b, usize::from(px) * 16, usize::from(py) * 16, &mut blocks)?;
            perm += 2;
        }
    }
    Ok(finish(f, width, height, Layout::Yuv422))
}

fn decode_hqa_mb(t: &Tables, f: &mut Frame, quant: usize, b: &mut Bits<'_>, x: usize, y: usize, blocks: &mut [[i16; 64]; 12]) -> Result<()> {
    if b.left() < 1 {
        return Err(Error::Truncated);
    }
    let mut cbp = t.cbp.read(b).map_or(-1i32, |v| v as i32);
    for blk in blocks.iter_mut() {
        *blk = [0; 64];
        blk[0] = -128 * 64;
    }
    let mut flag = false;
    if cbp != 0 {
        flag = b.bit();
        cbp |= cbp << 4;
        if cbp & 0x3 != 0 {
            cbp |= 0x500;
        }
        if cbp & 0xC != 0 {
            cbp |= 0xA00;
        }
        for (i, blk) in blocks.iter_mut().enumerate() {
            if cbp & (1 << i) != 0 {
                decode_block(t, b, blk, quant, i >= 8, true)?;
            }
        }
    }
    let [b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11] = blocks;
    f.put_blocks(3, x, y, flag, b0, b2);
    f.put_blocks(3, x + 8, y, flag, b1, b3);
    f.put_blocks(0, x, y, flag, b4, b6);
    f.put_blocks(0, x + 8, y, flag, b5, b7);
    f.put_blocks(2, x >> 1, y, flag, b8, b9);
    f.put_blocks(1, x >> 1, y, flag, b10, b11);
    Ok(())
}

/// An HQA frame; `data` starts at the `HQA1` tag.
pub(crate) fn decode_hqa(t: &Tables, data: &[u8]) -> Result<Picture> {
    const SLICES: usize = 8;
    let src = data.get(4..).unwrap_or_default();
    let data_size = data.len();
    if src.len() < 8 + 4 * (SLICES + 1) {
        return Err(Error::Truncated);
    }
    let (width, height) = (be(src, 0, 2), be(src, 2, 2));
    crate::check_size(width, height, data.len())?;
    let quant = usize::from(src.get(4).copied().unwrap_or(0));
    if quant >= 16 {
        return Err(Error::Invalid("quantiser matrix"));
    }
    let (cw, ch) = (width.next_multiple_of(16) as usize, height.next_multiple_of(16) as usize);
    let mut f = frame(cw, ch, true);
    let offs: Vec<u32> = (0..=SLICES).map(|i| be(src, 8 + i * 4, 4).wrapping_sub(4)).collect();
    let mut blocks = [[0i16; 64]; 12];
    for slice in 0..SLICES {
        let (Some(&a), Some(&z)) = (offs.get(slice), offs.get(slice + 1)) else { break };
        if (a as usize) < (SLICES + 1) * 3 || a >= z || z as usize > data_size {
            break;
        }
        let bytes = src.get(a as usize..(z as usize).min(src.len())).unwrap_or_default();
        let mut b = Bits::new(bytes);
        for i in (0..height as usize).step_by(16) {
            let off = (slice * 16 + i * 3) & 0x70;
            for j in (off..width as usize).step_by(128) {
                decode_hqa_mb(t, &mut f, quant, &mut b, j, i, &mut blocks)?;
            }
        }
    }
    Ok(finish(f, width, height, Layout::Yuva422))
}
