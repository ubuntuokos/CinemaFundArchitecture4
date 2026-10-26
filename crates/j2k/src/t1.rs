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

const SIG: u8 = 1;
const VISIT: u8 = 2;
const REFINED: u8 = 4;
const NEG: u8 = 8;

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
fn zc(h: u32, v: u32, d: u32, orient: Orient) -> usize {
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

struct State {
    w: usize,
    h: usize,
    stride: usize,
    /// Flags with a one-sample border.
    flags: Vec<u8>,
    mag: Vec<u32>,
    vsc: bool,
    orient: Orient,
}

impl State {
    fn at(&self, x: usize, y: usize) -> usize {
        (y + 1) * self.stride + x + 1
    }

    fn sig(&self, i: usize) -> u32 {
        u32::from(self.flags.get(i).copied().unwrap_or(0) & SIG)
    }

    /// Whether the row below is outside the stripe for context formation (vertically causal
    /// mode, D.7).
    fn causal(&self, y: usize) -> bool {
        self.vsc && y % 4 == 3
    }

    fn neighbours(&self, x: usize, y: usize) -> (u32, u32, u32) {
        let i = self.at(x, y);
        let (s, up, dn) = (self.stride, i - self.stride, i + self.stride);
        let below = !self.causal(y);
        let h = self.sig(i - 1) + self.sig(i + 1);
        let v = self.sig(up) + if below { self.sig(dn) } else { 0 };
        let d = self.sig(up - 1) + self.sig(up + 1) + if below { self.sig(dn - 1) + self.sig(dn + 1) } else { 0 };
        let _ = s;
        (h, v, d)
    }

    fn zc(&self, x: usize, y: usize) -> usize {
        let (h, v, d) = self.neighbours(x, y);
        zc(h, v, d, self.orient)
    }

    /// Sign coding context and XOR bit (Tables D.2, D.3).
    fn sc(&self, x: usize, y: usize) -> (usize, u8) {
        let i = self.at(x, y);
        let contrib = |j: usize| -> i32 {
            match self.flags.get(j).copied().unwrap_or(0) {
                f if f & SIG == 0 => 0,
                f if f & NEG != 0 => -1,
                _ => 1,
            }
        };
        let h = (contrib(i - 1) + contrib(i + 1)).clamp(-1, 1);
        let below = if self.causal(y) { 0 } else { contrib(i + self.stride) };
        let v = (contrib(i - self.stride) + below).clamp(-1, 1);
        match (h, v) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, -1) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, -1) => (10, 1),
            (-1, 1) => (11, 1),
            (-1, 0) => (12, 1),
            _ => (13, 1),
        }
    }

    fn become_significant(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], x: usize, y: usize, plane: u32) {
        let (c, xor) = self.sc(x, y);
        let sign = mq.decode(&mut cx[c]) ^ xor;
        let i = self.at(x, y);
        if let Some(f) = self.flags.get_mut(i) {
            *f |= SIG | if sign == 1 { NEG } else { 0 };
        }
        if let Some(m) = self.mag.get_mut(y * self.w + x) {
            *m = 3 << plane;
        }
    }

    fn flag(&self, x: usize, y: usize) -> u8 {
        self.flags.get(self.at(x, y)).copied().unwrap_or(0)
    }

    fn set(&mut self, x: usize, y: usize, bits: u8) {
        let i = self.at(x, y);
        if let Some(f) = self.flags.get_mut(i) {
            *f |= bits;
        }
    }

    /// Significance propagation pass (D.3.1).
    fn spp(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], plane: u32) {
        for y0 in (0..self.h).step_by(4) {
            for x in 0..self.w {
                for y in y0..(y0 + 4).min(self.h) {
                    if self.flag(x, y) & SIG != 0 {
                        continue;
                    }
                    let c = self.zc(x, y);
                    if c == 0 {
                        continue;
                    }
                    if mq.decode(&mut cx[c]) == 1 {
                        self.become_significant(mq, cx, x, y, plane);
                    }
                    self.set(x, y, VISIT);
                }
            }
        }
    }

    /// Magnitude refinement pass (D.3.3).
    fn mrp(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], plane: u32) {
        for y0 in (0..self.h).step_by(4) {
            for x in 0..self.w {
                for y in y0..(y0 + 4).min(self.h) {
                    let f = self.flag(x, y);
                    if f & (SIG | VISIT) != SIG {
                        continue;
                    }
                    let c = if f & REFINED != 0 {
                        16
                    } else {
                        let (h, v, d) = self.neighbours(x, y);
                        if h + v + d > 0 { 15 } else { 14 }
                    };
                    let bit = mq.decode(&mut cx[c]);
                    if let Some(m) = self.mag.get_mut(y * self.w + x) {
                        *m = if bit == 1 { m.wrapping_add(1 << plane) } else { m.wrapping_sub(1 << plane) };
                    }
                    self.set(x, y, REFINED);
                }
            }
        }
    }

    /// Cleanup pass (D.3.4), with run-length coding of all-insignificant stripe columns.
    fn cup(&mut self, mq: &mut Mq, cx: &mut [Ctx; CONTEXTS], plane: u32) {
        for y0 in (0..self.h).step_by(4) {
            for x in 0..self.w {
                let mut y = y0;
                let end = (y0 + 4).min(self.h);
                if y0 + 4 <= self.h && (y0..end).all(|yy| self.flag(x, yy) & (SIG | VISIT) == 0 && self.zc(x, yy) == 0) {
                    if mq.decode(&mut cx[CTX_RL]) == 0 {
                        continue;
                    }
                    let r = (usize::from(mq.decode(&mut cx[CTX_UNIFORM])) << 1) | usize::from(mq.decode(&mut cx[CTX_UNIFORM]));
                    y = y0 + r;
                    self.become_significant(mq, cx, x, y, plane);
                    y += 1;
                }
                while y < end {
                    if self.flag(x, y) & (SIG | VISIT) == 0 {
                        let c = self.zc(x, y);
                        if mq.decode(&mut cx[c]) == 1 {
                            self.become_significant(mq, cx, x, y, plane);
                        }
                    }
                    y += 1;
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
    let mut st = State { w: b.w, h: b.h, stride, flags: vec![0; stride * (b.h + 2)], mag: vec![0; n], vsc: b.style & STYLE_VSC != 0, orient: b.orient };
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
    Ok(st
        .mag
        .iter()
        .enumerate()
        .map(|(i, &m)| {
            let f = st.flags.get(st.at(i % b.w, i / b.w)).copied().unwrap_or(0);
            let m = m.min(i32::MAX as u32) as i32;
            if f & NEG != 0 { -m } else { m }
        })
        .collect())
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
