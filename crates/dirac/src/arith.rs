// Ported from FFmpeg n7.1.1 libavcodec/dirac_arith.c and dirac_arith.h (Copyright (C) 2007 Marco
// Gerards, 2009 David Conrad), LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes
// GPL-3.0-or-later.

//! The binary arithmetic decoder of Dirac core syntax (Dirac specification §B): 16-bit range,
//! adaptive probabilities, and the context sets of coefficients, block data and quantiser deltas.
//! Bytes past the end of a block read as 1 bits.

/// Coefficient and quantiser contexts (the context indices of FFmpeg's `dirac_arith_contexts`).
pub(crate) const ZPZN_F1: usize = 0;
pub(crate) const ZP_F2: usize = 4;
pub(crate) const ZP_F5: usize = 7;
pub(crate) const NPZN_F1: usize = 2;
pub(crate) const COEFF_DATA: usize = 14;
pub(crate) const SIGN_ZERO: usize = 16;
pub(crate) const ZERO_BLOCK: usize = 18;
pub(crate) const DELTA_Q_F: usize = 19;
pub(crate) const DELTA_Q_DATA: usize = 20;
const COUNT: usize = 22;

/// Block data contexts: the decoder is restarted for each kind, so they share indices.
pub(crate) const SB_F1: usize = ZP_F5;
pub(crate) const SB_DATA: usize = 0;
pub(crate) const PMODE_REF1: usize = 0;
pub(crate) const PMODE_REF2: usize = 1;
pub(crate) const GLOBAL_BLOCK: usize = 2;
pub(crate) const MV_F1: usize = ZP_F2;
pub(crate) const MV_DATA: usize = 0;
pub(crate) const DC_F1: usize = ZP_F5;
pub(crate) const DC_DATA: usize = 0;

/// The follow context after each (`ZP_F6`, `NP_F6` and `DELTA_Q_F` repeat).
const NEXT: [usize; COUNT] = [4, 4, 9, 9, 5, 6, 7, 8, 8, 10, 11, 12, 13, 13, 0, 0, 0, 0, 0, 19, 0, 0];

/// Probability updates (Table B.1 of the specification, scaled to 16 bits).
const PROB: [u16; 256] = [
    0, 2, 5, 8, 11, 15, 20, 24, 29, 35, 41, 47, 53, 60, 67, 74, 82, 89, 97, 106, 114, 123, 132, 141, 150, 160, 170, 180, 190, 201, 211, 222, 233, 244, 256,
    267, 279, 291, 303, 315, 327, 340, 353, 366, 379, 392, 405, 419, 433, 447, 461, 475, 489, 504, 518, 533, 548, 563, 578, 593, 609, 624, 640, 656, 672, 688,
    705, 721, 738, 754, 771, 788, 805, 822, 840, 857, 875, 892, 910, 928, 946, 964, 983, 1001, 1020, 1038, 1057, 1076, 1095, 1114, 1133, 1153, 1172, 1192,
    1211, 1231, 1251, 1271, 1291, 1311, 1332, 1352, 1373, 1393, 1414, 1435, 1456, 1477, 1498, 1520, 1541, 1562, 1584, 1606, 1628, 1649, 1671, 1694, 1716, 1738,
    1760, 1783, 1806, 1828, 1851, 1874, 1897, 1920, 1935, 1942, 1949, 1955, 1961, 1968, 1974, 1980, 1985, 1991, 1996, 2001, 2006, 2011, 2016, 2021, 2025, 2029,
    2033, 2037, 2040, 2044, 2047, 2050, 2053, 2056, 2058, 2061, 2063, 2065, 2066, 2068, 2069, 2070, 2071, 2072, 2072, 2072, 2072, 2072, 2072, 2071, 2070, 2069,
    2068, 2066, 2065, 2063, 2060, 2058, 2055, 2052, 2049, 2045, 2042, 2038, 2033, 2029, 2024, 2019, 2013, 2008, 2002, 1996, 1989, 1982, 1975, 1968, 1960, 1952,
    1943, 1934, 1925, 1916, 1906, 1896, 1885, 1874, 1863, 1851, 1839, 1827, 1814, 1800, 1786, 1772, 1757, 1742, 1727, 1710, 1694, 1676, 1659, 1640, 1622, 1602,
    1582, 1561, 1540, 1518, 1495, 1471, 1447, 1422, 1396, 1369, 1341, 1312, 1282, 1251, 1219, 1186, 1151, 1114, 1077, 1037, 995, 952, 906, 857, 805, 750, 690,
    625, 553, 471, 376, 255,
];

/// The probability of a 0 after decoding `bit` in a context whose probability was `p`.
#[inline]
pub(crate) fn update(p: u16, bit: bool) -> u16 {
    let i = usize::from(p >> 8);
    if bit { p.wrapping_sub(PROB.get(i).copied().unwrap_or(0)) } else { p.wrapping_add(PROB.get(255 - i).copied().unwrap_or(0)) }
}

pub(crate) struct Arith<'a> {
    data: &'a [u8],
    pos: usize,
    low: u32,
    range: u32,
    counter: i32,
    contexts: [u16; COUNT],
    overread: u32,
}

impl<'a> Arith<'a> {
    /// A decoder over `data` (one block of arithmetic coded data).
    pub fn new(data: &'a [u8]) -> Self {
        let mut low = 0u32;
        let mut pos = 0;
        for _ in 0..4 {
            low = (low << 8) | u32::from(data.get(pos).copied().unwrap_or(0xFF));
            pos = (pos + 1).min(data.len());
        }
        Self { data, pos, low, range: 0xFFFF, counter: -16, contexts: [0x8000; COUNT], overread: 0 }
    }

    /// More than four overreads: the block was damaged (its decode is garbage but bounded).
    pub fn damaged(&self) -> bool {
        self.overread > 4
    }

    fn refill(&mut self) {
        if self.counter >= 0 {
            let mut new = 0u32;
            for k in 0..2 {
                let byte = match self.data.get(self.pos + k) {
                    Some(&b) => u32::from(b),
                    None => 0xFF,
                };
                new = (new << 8) | byte;
            }
            if self.pos + 2 > self.data.len() {
                self.overread += 1;
                self.pos = self.data.len();
            } else {
                self.pos += 2;
            }
            self.low = self.low.wrapping_add(new.wrapping_shl(self.counter as u32 & 31));
            self.counter -= 16;
        }
    }

    pub fn bit(&mut self, ctx: usize) -> bool {
        let Some(&p) = self.contexts.get(ctx) else { return false };
        let rtp = (self.range * u32::from(p)) >> 16;
        let bit = (self.low >> 16) >= rtp;
        if bit {
            self.low = self.low.wrapping_sub(rtp << 16);
            self.range -= rtp;
        } else {
            self.range = rtp;
        }
        if let Some(c) = self.contexts.get_mut(ctx) {
            *c = update(p, bit);
        }
        // renormalise; a damaged stream may drive the range to zero, which stops here
        while self.range <= 0x4000 && self.range > 0 {
            self.low <<= 1;
            self.range <<= 1;
            self.counter += 1;
        }
        self.refill();
        bit
    }

    /// An unsigned exp-Golomb style integer: follow bits in `follow` contexts, data bits in `data`.
    pub fn uint(&mut self, follow: usize, data: usize) -> u32 {
        let mut v = 1u32;
        let mut follow = follow;
        while !self.bit(follow) {
            if v >= 0x4000_0000 {
                self.overread = u32::MAX;
                return 0;
            }
            v = (v << 1) | u32::from(self.bit(data));
            follow = NEXT.get(follow).copied().unwrap_or(follow);
        }
        v - 1
    }

    /// A signed integer: the magnitude, then a sign bit in context `data + 1`.
    pub fn int(&mut self, follow: usize, data: usize) -> i32 {
        let v = self.uint(follow, data) as i32;
        if v != 0 && self.bit(data + 1) { -v } else { v }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The matching encoder (exact interval arithmetic with carries), for tests.
    pub(crate) struct Encoder {
        /// The code value's bits, MSB first.
        bits: Vec<u8>,
        /// Bits of `bits` fixed so far (the interval base is at this scale).
        scale: usize,
        range: u32,
        contexts: [u16; COUNT],
    }

    impl Encoder {
        pub fn new() -> Self {
            Self { bits: vec![0; 16], scale: 0, range: 0xFFFF, contexts: [0x8000; COUNT] }
        }

        /// Add `v` (16 bits) to the base at the current scale.
        fn add(&mut self, v: u32) {
            let end = self.scale + 16;
            if self.bits.len() < end {
                self.bits.resize(end, 0);
            }
            let mut carry = 0u32;
            for k in (0..end).rev() {
                let add = if k >= self.scale { (v >> (15 - (k - self.scale))) & 1 } else { 0 };
                let s = u32::from(self.bits[k]) + add + carry;
                self.bits[k] = (s & 1) as u8;
                carry = s >> 1;
                if k < self.scale && carry == 0 {
                    break;
                }
            }
        }

        pub fn bit(&mut self, ctx: usize, bit: bool) {
            let p = self.contexts[ctx];
            let rtp = (self.range * u32::from(p)) >> 16;
            if bit {
                self.add(rtp);
                self.range -= rtp;
            } else {
                self.range = rtp;
            }
            self.contexts[ctx] = update(p, bit);
            while self.range <= 0x4000 {
                self.range <<= 1;
                self.scale += 1;
            }
        }

        pub fn uint(&mut self, follow: usize, data: usize, v: u32) {
            let v = v + 1;
            let top = 31 - v.leading_zeros();
            let mut follow = follow;
            for k in (0..top).rev() {
                self.bit(follow, false);
                self.bit(data, (v >> k) & 1 == 1);
                follow = NEXT[follow];
            }
            self.bit(follow, true);
        }

        pub fn int(&mut self, follow: usize, data: usize, v: i32) {
            self.uint(follow, data, v.unsigned_abs());
            if v != 0 {
                self.bit(data + 1, v < 0);
            }
        }

        /// The coded bytes: the base plus `range - 1` (the decoder reads 1 bits past the end).
        pub fn finish(mut self) -> Vec<u8> {
            let r = self.range - 1;
            self.add(r);
            let mut out = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut b = 0u8;
                for (k, &bit) in chunk.iter().enumerate() {
                    b |= bit << (7 - k);
                }
                if chunk.len() < 8 {
                    b |= 0xFF >> chunk.len();
                }
                out.push(b);
            }
            out
        }
    }

    #[test]
    fn round_trip() {
        let mut e = Encoder::new();
        let values: Vec<i32> = (0..2000).map(|i: i32| ((i * 7919) % 61) - 30).map(|v| if v.abs() < 20 { 0 } else { v }).collect();
        for &v in &values {
            e.int(ZPZN_F1, COEFF_DATA, v);
            e.bit(ZERO_BLOCK, v > 0);
        }
        let bytes = e.finish();
        let mut d = Arith::new(&bytes);
        for &v in &values {
            assert_eq!(d.int(ZPZN_F1, COEFF_DATA), v);
            assert_eq!(d.bit(ZERO_BLOCK), v > 0);
        }
        assert!(!d.damaged());
    }
}
