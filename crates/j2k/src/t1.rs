//! EBCOT tier-1 (T.800 Annex D): the bit-plane coding passes of one code-block.
//!
//! Magnitudes are kept with one extra fractional bit, at the reconstruction midpoint: a
//! coefficient that becomes significant in bit-plane p holds 3·2^p (1.5·2^p), each refinement
//! moves it by ±2^q. Fully decoded, `mag >> 1` is the exact magnitude; truncated, `mag / 2` is
//! the midpoint reconstruction (r = 1/2, E.1.1.2).

use crate::mq::{CONTEXTS, CTX_RL, CTX_UNIFORM, Ctx, Mq, initial_contexts};
use crate::{Error, Result};

/// Code-block style bits (A.6.1, Table A.19).
pub(crate) const STYLE_BYPASS: u8 = 1;
pub(crate) const STYLE_RESET: u8 = 2;
pub(crate) const STYLE_TERMALL: u8 = 4;
pub(crate) const STYLE_VSC: u8 = 8;
pub(crate) const STYLE_SEGSYM: u8 = 32;

/// Most bit-planes a code-block may have (the fractional bit must fit 32 bits).
pub(crate) const MAX_PLANES: u32 = 30;

// Per-sample flags (u16, with a one-sample border): the sample's own state, then the
// significance of its eight neighbours and the signs of the four it shares an edge with, kept up
// to date as samples become significant so the contexts are table lookups.
const SIG: u16 = 1;
const VISIT: u16 = 2;
const REFINED: u16 = 4;
const NEG: u16 = 8;
const SIG_N: u16 = 1 << 4;
const SIG_S: u16 = 1 << 5;
const SIG_W: u16 = 1 << 6;
const SIG_E: u16 = 1 << 7;
const SIG_NW: u16 = 1 << 8;
const SIG_NE: u16 = 1 << 9;
const SIG_SW: u16 = 1 << 10;
const SIG_SE: u16 = 1 << 11;
const NEG_N: u16 = 1 << 12;
const NEG_S: u16 = 1 << 13;
const NEG_W: u16 = 1 << 14;
const NEG_E: u16 = 1 << 15;
/// Any significant neighbour.
const NEIGHBOURS: u16 = 0xFF0;
/// The neighbours below, left out of the contexts of a stripe's last row in vertically causal
/// mode (D.7).
const BELOW: u16 = SIG_S | SIG_SW | SIG_SE | NEG_S;

/// Subband orientation: LL 0, HL 1, LH 2, HH 3.
pub(crate) type Orient = u8;

/// A code-block to decode.
pub(crate) struct Block<'a> {
    pub w: usize,
    pub h: usize,
    pub orient: Orient,
    pub style: u8,
    /// Bit-planes of the subband (Mb, E.1.1.1).
    pub mb: u32,
    /// Missing most significant bit-planes (from the packet headers).
    pub zero: u32,
    /// Codeword segments: their bytes and how many coding passes each holds.
    pub segments: &'a [(Vec<u8>, u32)],
}

/// Zero coding context (Table D.1) from the significant neighbours: horizontal, vertical and
/// diagonal counts.
const fn zc(h: u32, v: u32, d: u32, orient: Orient) -> usize {
    let (h, v) = if orient == 1 { (v, h) } else { (h, v) };
    if orient == 3 {
        let hv = h + v;
        return match (d, hv) {
            (3.., _) => 8,
            (2, 1..) => 7,
            (2, 0) => 6,
            (1, 2..) => 5,
            (1, 1) => 4,
            (1, 0) => 3,
            (0, 2..) => 2,
            (0, 1) => 1,
            _ => 0,
        };
    }
    match (h, v, d) {
        (2.., _, _) => 8,
        (1, 1.., _) => 7,
        (1, 0, 1..) => 6,
        (1, 0, 0) => 5,
        (0, 2.., _) => 4,
        (0, 1, _) => 3,
        (0, 0, 2..) => 2,
        (0, 0, 1) => 1,
        _ => 0,
    }
}

const fn bit(f: u16, m: u16) -> u32 {
    (f & m != 0) as u32
}

/// A neighbour's contribution to the sign context: 0 insignificant, else ±1 by its sign.
const fn contrib(n: usize, sig: usize, neg: usize) -> i32 {
    if n & (1 << sig) == 0 {
        0
    } else if n & (1 << neg) != 0 {
        -1
    } else {
        1
    }
}

const fn clamp1(v: i32) -> i32 {
    if v > 1 {
        1
    } else if v < -1 {
        -1
    } else {
        v
    }
}

/// [`zc`] for every orientation and neighbour significance pattern (flag bits 4-11).
const ZC_LUT: [[u8; 256]; 4] = {
    let mut t = [[0u8; 256]; 4];
    let mut o = 0;
    while o < 4 {
        let mut n = 0;
        while n < 256 {
            let f = (n as u16) << 4;
            let h = bit(f, SIG_W) + bit(f, SIG_E);
            let v = bit(f, SIG_N) + bit(f, SIG_S);
            let d = bit(f, SIG_NW) + bit(f, SIG_NE) + bit(f, SIG_SW) + bit(f, SIG_SE);
            t[o][n] = zc(h, v, d, o as u8) as u8;
            n += 1;
        }
        o += 1;
    }
    t
};

/// Sign coding context and XOR bit (Tables D.2, D.3) for every pattern of the edge neighbours'
/// significance (index bits 0-3: N, S, W, E) and signs (bits 4-7).
const SC_LUT: [(u8, u8); 256] = {
    let mut t = [(0u8, 0u8); 256];
    let mut n = 0;
    while n < 256 {
        let h = clamp1(contrib(n, 2, 6) + contrib(n, 3, 7));
        let v = clamp1(contrib(n, 0, 4) + contrib(n, 1, 5));
        t[n] = match (h, v) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, -1) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, -1) => (10, 1),
            (-1, 1) => (11, 1),
            (-1, 0) => (12, 1),
            _ => (13, 1),
        };
        n += 1;
    }
    t
};

/// Stripe-column summary bits: some sample of the column is significant or has a significant
/// neighbour (it may be coded in a significance pass), and some sample is significant.
const COL_ACTIVE: u8 = 1;
const COL_SIG: u8 = 2;

struct State {
    w: usize,
    h: usize,
    stride: usize,
    /// Flags and magnitudes with a one-sample border: sample (x, y) is at (y + 1) · stride + x + 1.
    flags: Vec<u16>,
    mag: Vec<u32>,
    /// Per stripe and column (with a one-column border, `stride` per stripe): [`COL_ACTIVE`],
    /// [`COL_SIG`]. A column with neither has nothing to code but cleanup run-lengths, so the
    /// passes skip it with one load.
    cols: Vec<u8>,
    /// Mask for the flags of a stripe's last row: drops the neighbours below in vertically causal
    /// mode (D.7).
    last_row: u16,
    zc: &'static [u8; 256],
}

impl State {
    fn at(&self, x: usize, y: usize) -> usize {
        (y + 1) * self.stride + x + 1
    }

    /// The flags of the sample at `i`, row `k` of its stripe.
    #[inline]
    fn flags(&self, i: usize, k: usize) -> u16 {
        let f = self.flags.get(i).copied().unwrap_or(0);
        if k == 3 { f & self.last_row } else { f }
    }

    #[inline]
    fn zc(&self, f: u16) -> usize {
        usize::from(self.zc[usize::from((f >> 4) as u8)])
    }

    #[inline]
    fn or(&mut self, i: usize, bits: u16) {
        if let Some(f) = self.flags.get_mut(i) {
            *f |= bits;
        }
    }

    #[inline]
    fn col(&self, stripe: usize, x: usize) -> u8 {
        self.cols.get(stripe * self.stride + x + 1).copied().unwrap_or(0)
    }

    /// Marks columns x - 1 ..= x + 1 of `stripe` active.
    #[inline]
    fn activate(&mut self, stripe: usize, x: usize) {
        let c = stripe * self.stride + x;
        if let Some(cols) = self.cols.get_mut(c..c + 3) {
            cols.iter_mut().for_each(|v| *v |= COL_ACTIVE);
        }
    }

    /// Decodes the sign of the sample at `i` (column `x`, row `k` of stripe `stripe`), which has
    /// become significant in bit-plane `plane`, and tells its neighbours.
    #[inline]
    fn become_significant(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], i: usize, x: usize, stripe: usize, k: usize, plane: u32) {
        let f = self.flags(i, k);
        let (c, xor) = SC_LUT[usize::from((f >> 4) & 0xF) | usize::from((f >> 12) << 4)];
        let neg = mq.decode(&mut cx[usize::from(c)]) ^ xor == 1;
        let s = self.stride;
        self.or(i, SIG | if neg { NEG } else { 0 });
        self.or(i - s, SIG_S | if neg { NEG_S } else { 0 });
        self.or(i + s, SIG_N | if neg { NEG_N } else { 0 });
        self.or(i - 1, SIG_E | if neg { NEG_E } else { 0 });
        self.or(i + 1, SIG_W | if neg { NEG_W } else { 0 });
        self.or(i - s - 1, SIG_SE);
        self.or(i - s + 1, SIG_SW);
        self.or(i + s - 1, SIG_NE);
        self.or(i + s + 1, SIG_NW);
        if let Some(m) = self.mag.get_mut(i) {
            *m = 3 << plane;
        }
        self.activate(stripe, x);
        if let Some(v) = self.cols.get_mut(stripe * s + x + 1) {
            *v |= COL_SIG;
        }
        // the rows next to this one in the stripes above and below
        if k == 0 && stripe > 0 {
            self.activate(stripe - 1, x);
        }
        if k == 3 {
            self.activate(stripe + 1, x);
        }
    }

    /// Significance propagation pass (D.3.1).
    fn spp(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], plane: u32) {
        let s = self.stride;
        for (stripe, y0) in (0..self.h).step_by(4).enumerate() {
            let rows = (self.h - y0).min(4);
            let base = self.at(0, y0);
            for x in 0..self.w {
                if self.col(stripe, x) & COL_ACTIVE == 0 {
                    continue;
                }
                for k in 0..rows {
                    let i = base + x + k * s;
                    let f = self.flags(i, k);
                    if f & SIG != 0 || f & NEIGHBOURS == 0 {
                        continue;
                    }
                    if mq.decode(&mut cx[self.zc(f)]) == 1 {
                        self.become_significant(mq, cx, i, x, stripe, k, plane);
                    }
                    self.or(i, VISIT);
                }
            }
        }
    }

    /// Magnitude refinement pass (D.3.3).
    fn mrp(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], plane: u32) {
        let s = self.stride;
        for (stripe, y0) in (0..self.h).step_by(4).enumerate() {
            let rows = (self.h - y0).min(4);
            let base = self.at(0, y0);
            for x in 0..self.w {
                if self.col(stripe, x) & COL_SIG == 0 {
                    continue;
                }
                for k in 0..rows {
                    let i = base + x + k * s;
                    let f = self.flags(i, k);
                    if f & (SIG | VISIT) != SIG {
                        continue;
                    }
                    let c = if f & REFINED != 0 {
                        16
                    } else if f & NEIGHBOURS != 0 {
                        15
                    } else {
                        14
                    };
                    let bit = mq.decode(&mut cx[c]);
                    if let Some(m) = self.mag.get_mut(i) {
                        *m = if bit == 1 { m.wrapping_add(1 << plane) } else { m.wrapping_sub(1 << plane) };
                    }
                    self.or(i, REFINED);
                }
            }
        }
    }

    /// Cleanup pass (D.3.4), with run-length coding of all-insignificant stripe columns.
    fn cup(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], plane: u32) {
        let s = self.stride;
        for (stripe, y0) in (0..self.h).step_by(4).enumerate() {
            let rows = (self.h - y0).min(4);
            let base = self.at(0, y0);
            for x in 0..self.w {
                let col = base + x;
                let mut k = 0;
                // an inactive column has no significant samples or neighbours (and so no
                // samples visited by the significance pass)
                let quiet =
                    rows == 4 && (self.col(stripe, x) & COL_ACTIVE == 0 || (0..4).all(|k| self.flags(col + k * s, k) & (SIG | VISIT | NEIGHBOURS) == 0));
                if quiet {
                    if mq.decode(&mut cx[CTX_RL]) == 0 {
                        continue;
                    }
                    k = (usize::from(mq.decode(&mut cx[CTX_UNIFORM])) << 1) | usize::from(mq.decode(&mut cx[CTX_UNIFORM]));
                    self.become_significant(mq, cx, col + k * s, x, stripe, k, plane);
                    k += 1;
                }
                while k < rows {
                    let i = col + k * s;
                    let f = self.flags(i, k);
                    if f & (SIG | VISIT) == 0 && mq.decode(&mut cx[self.zc(f)]) == 1 {
                        self.become_significant(mq, cx, i, x, stripe, k, plane);
                    }
                    k += 1;
                }
            }
        }
        for f in &mut self.flags {
            *f &= !VISIT;
        }
    }
}

/// Decodes a code-block into signed coefficients with one fractional bit (see the module
/// documentation), `w · h` of them in raster order.
pub(crate) fn decode(b: &Block) -> Result<Vec<i32>> {
    let n = b.w.checked_mul(b.h).ok_or(Error::TooLarge)?;
    if b.style & STYLE_BYPASS != 0 {
        return Err(Error::Unsupported("selective arithmetic coding bypass"));
    }
    if b.mb <= b.zero || b.segments.is_empty() || n == 0 {
        return Ok(vec![0; n]);
    }
    let planes = b.mb - b.zero;
    if planes > MAX_PLANES {
        return Err(Error::Unsupported("code-block with more than 30 bit-planes"));
    }
    let stride = b.w + 2;
    let zc = ZC_LUT.get(usize::from(b.orient)).ok_or(Error::Invalid("subband orientation"))?;
    let padded = stride * (b.h + 2);
    let last_row = if b.style & STYLE_VSC != 0 { !BELOW } else { !0 };
    let cols = vec![0; stride * (b.h.div_ceil(4) + 1)];
    let mut st = State { w: b.w, h: b.h, stride, flags: vec![0; padded], mag: vec![0; padded], cols, last_row, zc };
    let mut cx = initial_contexts();
    let mut pass = 0u32;
    let total: u32 = b.segments.iter().map(|s| s.1).sum();
    let total = total.min(3 * planes - 2);
    'segments: for (data, count) in b.segments {
        let mut mq = Mq::new(data);
        for _ in 0..*count {
            if pass >= total {
                break 'segments;
            }
            // pass 0 is the cleanup of the first plane, then SPP, MRP, CUP per plane
            let kind = (pass + 2) % 3;
            let Some(plane) = (planes - 1).checked_sub(pass.div_ceil(3)) else { break 'segments };
            match kind {
                0 => st.spp(&mut mq, &mut cx, plane),
                1 => st.mrp(&mut mq, &mut cx, plane),
                _ => {
                    st.cup(&mut mq, &mut cx, plane);
                    if b.style & STYLE_SEGSYM != 0 {
                        for _ in 0..4 {
                            mq.decode(&mut cx[CTX_UNIFORM]);
                        }
                    }
                }
            }
            if b.style & STYLE_RESET != 0 {
                cx = initial_contexts();
            }
            pass += 1;
        }
    }
    let mut out = Vec::with_capacity(n);
    for y in 0..b.h {
        let i = st.at(0, y);
        let (Some(m), Some(f)) = (st.mag.get(i..i + b.w), st.flags.get(i..i + b.w)) else { break };
        out.extend(m.iter().zip(f).map(|(&m, &f)| {
            let m = m.min(i32::MAX as u32) as i32;
            if f & NEG != 0 { -m } else { m }
        }));
    }
    out.resize(n, 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_coding_contexts_follow_table_d1() {
        assert_eq!(zc(2, 0, 0, 0), 8);
        assert_eq!(zc(0, 2, 0, 0), 4);
        assert_eq!(zc(0, 2, 0, 1), 8); // HL swaps H and V
        assert_eq!(zc(0, 0, 1, 0), 1);
        assert_eq!(zc(1, 0, 3, 3), 8);
        assert_eq!(zc(1, 1, 1, 3), 5);
        assert_eq!(zc(0, 0, 0, 3), 0);
    }

    #[test]
    fn an_empty_block_decodes_to_zeros() {
        let b = Block { w: 4, h: 3, orient: 0, style: 0, mb: 8, zero: 8, segments: &[] };
        assert_eq!(decode(&b).ok(), Some(vec![0; 12]));
    }
}
