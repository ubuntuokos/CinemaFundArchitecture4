// Ported from FFmpeg n7.1.1 libavcodec/ra144.c and ra144dec.c (Copyright (c) 2003 The FFmpeg
// project, Nick Kurshev, 2008 Vitor Sessak), with ff_celp_lp_synthesis_filter from celp_filters.c,
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! RealAudio 1.0 (14.4 kbit/s, `lpcJ`): a fixed-point CELP codec, 20-byte frames of 160 samples
//! at 8 kHz mono. The arithmetic is the C code's, wrapping included, so the output is FFmpeg's
//! bit for bit.

use crate::bits::Bits;
use crate::tables144::*;
use crate::{Error, Result};

const NBLOCKS: usize = 4;
const BLOCKSIZE: usize = 40;
const BUFFERSIZE: usize = 146;
/// Bytes per frame.
pub const FRAME_SIZE: usize = 20;
/// Samples per frame.
pub const FRAME_SAMPLES: usize = NBLOCKS * BLOCKSIZE;
const LPC_ORDER: usize = 10;

fn refl_cb(i: usize) -> &'static [i16] {
    match i {
        0 => &LPC_REFL_CB1,
        1 => &LPC_REFL_CB2,
        2 => &LPC_REFL_CB3,
        3 => &LPC_REFL_CB4,
        4 => &LPC_REFL_CB5,
        5 => &LPC_REFL_CB6,
        6 => &LPC_REFL_CB7,
        7 => &LPC_REFL_CB8,
        8 => &LPC_REFL_CB9,
        _ => &LPC_REFL_CB10,
    }
}

/// `ff_sqrt`: the integer square root (rounded down).
fn isqrt(a: u32) -> u32 {
    let mut r = (f64::from(a)).sqrt() as u32;
    while u64::from(r) * u64::from(r) > u64::from(a) {
        r -= 1;
    }
    while u64::from(r + 1) * u64::from(r + 1) <= u64::from(a) {
        r += 1;
    }
    r
}

/// `ff_t_sqrt`: sqrt(x << 24), the binary decoder's way.
fn t_sqrt(mut x: u32) -> i32 {
    let mut s = 2u32;
    while x > 0xFFF {
        s += 1;
        x >>= 2;
    }
    isqrt(x << 20).wrapping_shl(s) as i32
}

/// `ff_rms`.
fn rms(data: &[i32; LPC_ORDER]) -> u32 {
    let mut res: u32 = 0x10000;
    let mut b = LPC_ORDER as u32;
    for &d in data {
        res = ((0x100_0000i32.wrapping_sub(d.wrapping_mul(d)) >> 12) as u32).wrapping_mul(res) >> 12;
        if res == 0 {
            return 0;
        }
        while res <= 0x3FFF {
            b += 1;
            res <<= 2;
        }
    }
    t_sqrt(res).checked_shr(b).unwrap_or(0) as u32
}

/// `ff_eval_refl`: true when the coefficients are unstable.
fn eval_refl(refl: &mut [i32; LPC_ORDER], coefs: &[i16; LPC_ORDER]) -> bool {
    let mut bp1 = [0i32; LPC_ORDER];
    let mut bp2 = [0i32; LPC_ORDER];
    for (d, &c) in bp2.iter_mut().zip(coefs) {
        *d = i32::from(c);
    }
    refl[LPC_ORDER - 1] = bp2[LPC_ORDER - 1];
    if (bp2[LPC_ORDER - 1] as u32).wrapping_add(0x1000) > 0x1FFF {
        return true;
    }
    for i in (0..LPC_ORDER - 1).rev() {
        let mut b = 0x1000i32.wrapping_sub(bp2[i + 1].wrapping_mul(bp2[i + 1]) >> 12);
        if b == 0 {
            b = -2;
        }
        b = 0x100_0000i32.wrapping_div(b);
        for j in 0..=i {
            let t = (refl[i + 1] as u32).wrapping_mul(bp2[i - j] as u32) as i32 >> 12;
            bp1[j] = (bp2[j].wrapping_sub(t) as u32).wrapping_mul(b as u32) as i32 >> 12;
        }
        if (bp1[i] as u32).wrapping_add(0x1000) > 0x1FFF {
            return true;
        }
        refl[i] = bp1[i];
        std::mem::swap(&mut bp1, &mut bp2);
    }
    false
}

/// `ff_eval_coefs`.
fn eval_coefs(coefs: &mut [i32; LPC_ORDER], refl: &[i32; LPC_ORDER]) {
    let mut b1 = [0i32; LPC_ORDER];
    let mut b2 = *coefs;
    for i in 0..LPC_ORDER {
        b1[i] = refl[i].wrapping_mul(16);
        for j in 0..i {
            b1[j] = ((refl[i] as u32).wrapping_mul(b2[i - j - 1] as u32) as i32 >> 12).wrapping_add(b2[j]);
        }
        std::mem::swap(&mut b1, &mut b2);
    }
    // ten swaps: the result is back in the caller's array
    for (c, v) in coefs.iter_mut().zip(b2) {
        *c = v >> 4;
    }
}

fn rescale_rms(rms: u32, energy: u32) -> u32 {
    rms.wrapping_mul(energy) >> 10
}

/// A RealAudio 1.0 decoder.
#[derive(Clone)]
pub struct Decoder {
    old_energy: u32,
    lpc_tables: [[i32; LPC_ORDER]; 2],
    /// Which of `lpc_tables` is `lpc_coef[0]` (the current frame's).
    cur: usize,
    lpc_refl_rms: [u32; 2],
    curr_sblock: [i16; 50],
    adapt_cb: [i16; BUFFERSIZE + 2],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self { old_energy: 0, lpc_tables: [[0; LPC_ORDER]; 2], cur: 0, lpc_refl_rms: [0; 2], curr_sblock: [0; 50], adapt_cb: [0; BUFFERSIZE + 2] }
    }

    fn coef(&self, k: usize) -> &[i32; LPC_ORDER] {
        &self.lpc_tables[(self.cur + k) & 1]
    }

    /// `ff_interp`.
    fn interp(&self, out: &mut [i16; LPC_ORDER], a: i32, copyold: usize, energy: u32) -> u32 {
        let b = NBLOCKS as i32 - a;
        let (c0, c1) = (self.coef(0), self.coef(1));
        for i in 0..LPC_ORDER {
            out[i] = ((a as u32).wrapping_mul(c0[i] as u32).wrapping_add((b as u32).wrapping_mul(c1[i] as u32)) >> 2) as i16;
        }
        let mut work = [0i32; LPC_ORDER];
        if eval_refl(&mut work, out) {
            for (o, &c) in out.iter_mut().zip(self.coef(copyold)) {
                *o = c as i16;
            }
            rescale_rms(self.lpc_refl_rms[copyold], energy)
        } else {
            rescale_rms(rms(&work), energy)
        }
    }

    /// `ff_subblock_synthesis`.
    fn subblock(&mut self, lpc: &[i16; LPC_ORDER], cba_idx: u32, cb1: usize, cb2: usize, gval: i32, gain: usize) {
        let mut buffer_a = [0i16; BLOCKSIZE];
        let mut m = [0i32; 3];
        let cba = cba_idx as usize;
        if cba != 0 {
            let off = cba + BLOCKSIZE / 2 - 1;
            // ff_copy_and_dup
            let src = BUFFERSIZE - off;
            let n = off.min(BLOCKSIZE);
            for i in 0..n {
                buffer_a[i] = self.adapt_cb.get(src + i).copied().unwrap_or(0);
            }
            if off < BLOCKSIZE {
                for i in 0..BLOCKSIZE - off {
                    buffer_a[off + i] = self.adapt_cb.get(src + i).copied().unwrap_or(0);
                }
            }
            // ff_irms
            let sum = buffer_a.iter().fold(0u32, |s, &v| s.wrapping_add((i32::from(v) * i32::from(v)) as u32));
            let irms = if sum == 0 {
                0
            } else {
                let d = t_sqrt(sum) >> 8;
                if d == 0 { 0 } else { 0x2000_0000i32.wrapping_div(d) }
            };
            m[0] = ((irms as u32).wrapping_mul(gval as u32) >> 12) as i32;
        }
        m[1] = i32::from(CB1_BASE.get(cb1).copied().unwrap_or(0)).wrapping_mul(gval) >> 8;
        m[2] = i32::from(CB2_BASE.get(cb2).copied().unwrap_or(0)).wrapping_mul(gval) >> 8;
        self.adapt_cb.copy_within(BLOCKSIZE..BUFFERSIZE, 0);
        // add_wav
        let mut v = [0i32; 3];
        let first = usize::from(cba == 0);
        let exp = u32::from(GAIN_EXP_TAB.get(gain).copied().unwrap_or(0));
        for i in first..3 {
            let g = GAIN_VAL_TAB.get(gain * 3 + i).copied().unwrap_or(0);
            v[i] = ((i32::from(g) as u32).wrapping_mul(m[i] as u32).checked_shr(exp).unwrap_or(0)) as i32;
        }
        let s2 = CB1_VECTS.get(cb1 * BLOCKSIZE..(cb1 + 1) * BLOCKSIZE).unwrap_or(&[]);
        let s3 = CB2_VECTS.get(cb2 * BLOCKSIZE..(cb2 + 1) * BLOCKSIZE).unwrap_or(&[]);
        let base = BUFFERSIZE - BLOCKSIZE;
        for i in 0..BLOCKSIZE {
            let a2 = i32::from(s2.get(i).copied().unwrap_or(0)).wrapping_mul(v[1]);
            let a3 = i32::from(s3.get(i).copied().unwrap_or(0)).wrapping_mul(v[2]);
            let val = if v[0] != 0 {
                ((i32::from(buffer_a[i]) as u32).wrapping_mul(v[0] as u32).wrapping_add(a2 as u32).wrapping_add(a3 as u32) as i32) >> 12
            } else {
                a2.wrapping_add(a3) >> 12
            };
            self.adapt_cb[base + i] = val as i16;
        }
        self.curr_sblock.copy_within(BLOCKSIZE..BLOCKSIZE + LPC_ORDER, 0);
        // ff_celp_lp_synthesis_filter(out = curr_sblock + 10, ..., stop_on_overflow 1, shift 0, rounder 0xfff)
        for n in 0..BLOCKSIZE {
            let mut sum: i32 = 0xFFF;
            for i in 1..=LPC_ORDER {
                let o = i32::from(self.curr_sblock[LPC_ORDER + n - i]);
                sum = sum.wrapping_sub(i32::from(lpc[i - 1]).wrapping_mul(o));
            }
            let sum1 = (sum >> 12).wrapping_add(i32::from(self.adapt_cb[base + n]));
            let clipped = sum1.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
            if clipped != sum1 {
                self.curr_sblock = [0; 50];
                return;
            }
            self.curr_sblock[LPC_ORDER + n] = clipped as i16;
        }
    }

    /// `ra144_decode_frame`: one 20-byte frame to 160 samples.
    pub fn decode(&mut self, buf: &[u8]) -> Result<[i16; FRAME_SAMPLES]> {
        const SIZES: [u32; LPC_ORDER] = [6, 5, 5, 4, 4, 3, 3, 3, 3, 2];
        let frame = buf.get(..FRAME_SIZE).ok_or(Error::Invalid("RealAudio 1.0 frame too small"))?;
        let mut gb = Bits::new(frame, FRAME_SIZE * 8);
        let mut lpc_refl = [0i32; LPC_ORDER];
        for (i, r) in lpc_refl.iter_mut().enumerate() {
            *r = i32::from(refl_cb(i).get(gb.get(SIZES[i]) as usize).copied().unwrap_or(0));
        }
        let cur = self.cur;
        eval_coefs(&mut self.lpc_tables[cur], &lpc_refl);
        self.lpc_refl_rms[0] = rms(&lpc_refl);
        let energy = ENERGY_TAB.get(gb.get(5) as usize).copied().unwrap_or(0) as u32;
        let mut block_coefs = [[0i16; LPC_ORDER]; NBLOCKS];
        let mut refl_rms = [0u32; NBLOCKS];
        refl_rms[0] = self.interp(&mut block_coefs[0], 1, 1, self.old_energy);
        refl_rms[1] = self.interp(&mut block_coefs[1], 2, usize::from(energy <= self.old_energy), (t_sqrt(energy.wrapping_mul(self.old_energy)) >> 12) as u32);
        refl_rms[2] = self.interp(&mut block_coefs[2], 3, 0, energy);
        refl_rms[3] = rescale_rms(self.lpc_refl_rms[0], energy);
        for (o, &c) in block_coefs[3].iter_mut().zip(self.coef(0)) {
            *o = c as i16;
        }
        let mut out = [0i16; FRAME_SAMPLES];
        for i in 0..NBLOCKS {
            let cba = gb.get(7);
            let gain = gb.get(8) as usize;
            let cb1 = gb.get(7) as usize;
            let cb2 = gb.get(7) as usize;
            let coefs = block_coefs[i];
            self.subblock(&coefs, cba, cb1, cb2, refl_rms[i] as i32, gain);
            for j in 0..BLOCKSIZE {
                out[i * BLOCKSIZE + j] = (i32::from(self.curr_sblock[j + LPC_ORDER]) * 4).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            }
        }
        self.old_energy = energy;
        self.lpc_refl_rms[1] = self.lpc_refl_rms[0];
        self.cur ^= 1;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_square_root() {
        for a in [0u32, 1, 2, 3, 4, 15, 16, 255, 256, 4095, 1 << 20, (1 << 31) + 12345, u32::MAX] {
            let r = u64::from(isqrt(a));
            assert!(r * r <= u64::from(a) && (r + 1) * (r + 1) > u64::from(a), "{a}");
        }
    }
}
