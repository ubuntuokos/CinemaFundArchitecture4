// Ported from the WavPack library (read_words.c, entropy_utils.c, the bitstream macros of
// wavpack_local.h), Copyright (c) 1998 - 2025 David Bryant, Conifer Software; BSD-3-Clause
// (see `LICENSE.WavPack`). CFA4 changes: rewritten in safe Rust, GPL-3.0-or-later.

//! The WavPack bitstream and its entropy coder: a Rice variant with three adaptive medians,
//! zero-run escapes, and (in hybrid mode) a bitrate-controlled error limit.

/// "End of stream or damaged": the reference decoder's `WORD_EOF`.
pub const WORD_EOF: i32 = i32::MIN;

const SLS: u32 = 8;
const SLO: u32 = 1 << (SLS - 1);
const LIMIT_ONES: u32 = 16;
const DIV0: u32 = 128;
const DIV1: u32 = 64;
const DIV2: u32 = 32;

/// LSB-first bit reader over one metadata sub-block. Past the end it reads zero bits and
/// remembers that it did (the reference wraps around and flags an error; either way the block's
/// CRC no longer matches and the block is muted).
#[derive(Clone, Default)]
pub struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    pub open: bool,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, open: true }
    }

    #[inline]
    pub fn bit(&mut self) -> u32 {
        let b = self.data.get(self.pos >> 3).map_or(0, |b| u32::from(*b >> (self.pos & 7)) & 1);
        self.pos = self.pos.saturating_add(1);
        b
    }

    /// `n` (≤ 32) bits, first bit in the least significant place.
    #[inline]
    pub fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n.min(32) {
            v |= self.bit() << i;
        }
        v
    }

    /// The reference `read_code`: a value in 0..=maxcode in the fewest bits (one more bit for
    /// the upper codes when the range is not a power of two).
    #[inline]
    fn code(&mut self, maxcode: u32) -> u32 {
        if maxcode < 2 {
            return if maxcode != 0 { self.bit() } else { 0 };
        }
        let bitcount = 32 - maxcode.leading_zeros();
        let extras = ((1u64 << bitcount) - u64::from(maxcode) - 1) as u32;
        let code = self.bits(bitcount - 1);
        if code >= extras { (code << 1).wrapping_sub(extras).wrapping_add(self.bit()) } else { code }
    }

    /// A count of 1 bits (up to 33) ended by a 0; then that many - 1 bits under an implied top bit.
    /// `None` for a run of 33 ones (the reference treats it as end of data).
    fn escape(&mut self) -> Option<u32> {
        let mut cbits = 0;
        while cbits < 33 && self.bit() != 0 {
            cbits += 1;
        }
        if cbits == 33 {
            return None;
        }
        if cbits < 2 {
            return Some(cbits);
        }
        let mut v = 0u32;
        let mut mask = 1u32;
        for _ in 1..cbits {
            if self.bit() != 0 {
                v |= mask;
            }
            mask = mask.wrapping_shl(1);
        }
        Some(v | mask)
    }

    /// Unary count of ones (with the `LIMIT_ONES` escape); `None` on a run that is too long.
    fn ones(&mut self) -> Option<u32> {
        let mut n = 0;
        while n < LIMIT_ONES + 1 && self.bit() != 0 {
            n += 1;
        }
        if n == LIMIT_ONES + 1 {
            return None;
        }
        if n == LIMIT_ONES {
            return Some(self.escape()?.wrapping_add(LIMIT_ONES));
        }
        Some(n)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Entropy {
    pub median: [u32; 3],
    pub slow_level: u32,
    pub error_limit: u32,
}

impl Entropy {
    #[inline]
    fn med(&self, i: usize) -> u32 {
        (self.median[i] >> 4) + 1
    }
    #[inline]
    fn inc(&mut self, i: usize) {
        let div = [DIV0, DIV1, DIV2][i];
        let m = self.median[i];
        self.median[i] = m.wrapping_add((m.wrapping_add(div) / div).wrapping_mul(5));
    }
    #[inline]
    fn dec(&mut self, i: usize) {
        let div = [DIV0, DIV1, DIV2][i];
        let m = self.median[i];
        self.median[i] = m.wrapping_sub((m.wrapping_add(div - 2) / div).wrapping_mul(2));
    }

    /// The bounds of the value range a run of `ones` selects, updating the medians.
    #[inline]
    fn range(&mut self, ones: u32) -> (u32, u32) {
        if ones == 0 {
            let high = self.med(0).wrapping_sub(1);
            self.dec(0);
            return (0, high);
        }
        let mut low = self.med(0);
        self.inc(0);
        if ones == 1 {
            let high = low.wrapping_add(self.med(1)).wrapping_sub(1);
            self.dec(1);
            return (low, high);
        }
        low = low.wrapping_add(self.med(1));
        self.inc(1);
        if ones == 2 {
            let high = low.wrapping_add(self.med(2)).wrapping_sub(1);
            self.dec(2);
            return (low, high);
        }
        low = low.wrapping_add((ones - 2).wrapping_mul(self.med(2)));
        let high = low.wrapping_add(self.med(2)).wrapping_sub(1);
        self.inc(2);
        (low, high)
    }
}

/// The reference `words_data`: entropy state shared by the channels of a block.
#[derive(Clone, Default)]
pub struct Words {
    pub bitrate_delta: [u32; 2],
    pub bitrate_acc: [u32; 2],
    pub holding_one: u32,
    pub zeros_acc: u32,
    pub holding_zero: bool,
    pub c: [Entropy; 2],
}

/// Block flags the entropy decoder looks at.
pub struct Mode {
    pub mono: bool,
    pub hybrid: bool,
    pub hybrid_bitrate: bool,
    pub hybrid_balance: bool,
}

impl Words {
    fn zero_run(&mut self, bs: &mut Bits) -> Option<bool> {
        if self.zeros_acc != 0 {
            self.zeros_acc -= 1;
            return Some(self.zeros_acc != 0);
        }
        self.zeros_acc = bs.escape()?;
        if self.zeros_acc != 0 {
            self.c[0].median = [0; 3];
            self.c[1].median = [0; 3];
            return Some(true);
        }
        Some(false)
    }

    /// The reference `get_words_lossless`: `out` holds `out.len()` words (interleaved when
    /// stereo). Returns how many were read before the stream ended or broke.
    pub fn lossless(&mut self, bs: &mut Bits, mono: bool, out: &mut [i32]) -> usize {
        let n = out.len();
        let mut i = 0;
        while i < n {
            let mut ch = if mono { 0 } else { i & 1 };
            if self.holding_zero {
                self.holding_zero = false;
                let c = &mut self.c[ch];
                let low = bs.code(c.med(0).wrapping_sub(1));
                c.dec(0);
                if let Some(o) = out.get_mut(i) {
                    *o = if bs.bit() != 0 { !(low as i32) } else { low as i32 };
                }
                i += 1;
                if i == n {
                    break;
                }
                ch = if mono { 0 } else { i & 1 };
            }
            if self.c[0].median[0] < 2 && self.holding_one == 0 && self.c[1].median[0] < 2 {
                match self.zero_run(bs) {
                    None => break,
                    Some(true) => {
                        if let Some(o) = out.get_mut(i) {
                            *o = 0;
                        }
                        i += 1;
                        continue;
                    }
                    Some(false) => {}
                }
            }
            let Some(ones) = bs.ones() else { break };
            let held = self.holding_one;
            self.holding_one = ones & 1;
            self.holding_zero = ones & 1 == 0;
            let ones = (ones >> 1).wrapping_add(held);
            let c = &mut self.c[ch];
            let (low, high) = c.range(ones);
            let v = low.wrapping_add(bs.code(high.wrapping_sub(low)));
            if let Some(o) = out.get_mut(i) {
                *o = if bs.bit() != 0 { !(v as i32) } else { v as i32 };
            }
            i += 1;
        }
        i
    }

    /// The reference `get_word` (hybrid streams, no correction file): one word of channel `ch`.
    pub fn word(&mut self, bs: &mut Bits, ch: usize, mode: &Mode) -> i32 {
        if !bs.open {
            return WORD_EOF;
        }
        let ch = ch.min(1);
        if self.c[0].median[0] & !1 == 0 && !self.holding_zero && self.holding_one == 0 && self.c[1].median[0] & !1 == 0 {
            match self.zero_run(bs) {
                None => return WORD_EOF,
                Some(true) => {
                    let c = &mut self.c[ch];
                    c.slow_level = c.slow_level.wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
                    return 0;
                }
                Some(false) => {}
            }
        }
        let ones = if self.holding_zero {
            self.holding_zero = false;
            0
        } else {
            let Some(ones) = bs.ones() else { return WORD_EOF };
            let ones = if self.holding_one != 0 {
                self.holding_one = ones & 1;
                (ones >> 1) + 1
            } else {
                self.holding_one = ones & 1;
                ones >> 1
            };
            self.holding_zero = self.holding_one == 0;
            ones
        };
        if mode.hybrid && ch == 0 {
            self.update_error_limit(mode);
        }
        let c = &mut self.c[ch];
        let (low, high) = c.range(ones);
        let mut low = low & 0x7fff_ffff;
        let mut high = high & 0x7fff_ffff;
        if low > high {
            high = low;
        }
        let mut mid = (high + low + 1) >> 1;
        if c.error_limit == 0 {
            mid = bs.code(high - low).wrapping_add(low);
        } else {
            while high - low > c.error_limit {
                if bs.bit() != 0 {
                    low = mid;
                    mid = (high + low + 1) >> 1;
                } else {
                    high = mid.wrapping_sub(1);
                    mid = (high.wrapping_add(low).wrapping_add(1)) >> 1;
                }
                if high < low {
                    break;
                }
            }
        }
        let sign = bs.bit() != 0;
        if mode.hybrid_bitrate {
            c.slow_level = c.slow_level.wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
            c.slow_level = c.slow_level.wrapping_add(log2(mid) as u32);
        }
        if sign { !(mid as i32) } else { mid as i32 }
    }

    /// The reference `update_error_limit`.
    fn update_error_limit(&mut self, mode: &Mode) {
        self.bitrate_acc[0] = self.bitrate_acc[0].wrapping_add(self.bitrate_delta[0]);
        let mut bitrate_0 = (self.bitrate_acc[0] >> 16) as i32;
        let limit = |slow: u32, rate: i32| {
            let slow_log = (slow.wrapping_add(SLO) >> SLS) as i32;
            if slow_log - rate > -0x100 { exp2s(slow_log - rate + 0x100) as u32 } else { 0 }
        };
        if mode.mono {
            self.c[0].error_limit = if mode.hybrid_bitrate { limit(self.c[0].slow_level, bitrate_0) } else { exp2s(bitrate_0) as u32 };
            return;
        }
        self.bitrate_acc[1] = self.bitrate_acc[1].wrapping_add(self.bitrate_delta[1]);
        let mut bitrate_1 = (self.bitrate_acc[1] >> 16) as i32;
        if mode.hybrid_bitrate {
            let slow_log_0 = (self.c[0].slow_level.wrapping_add(SLO) >> SLS) as i32;
            let slow_log_1 = (self.c[1].slow_level.wrapping_add(SLO) >> SLS) as i32;
            if mode.hybrid_balance {
                let balance = (slow_log_1 - slow_log_0 + bitrate_1 + 1) >> 1;
                if balance > bitrate_0 {
                    bitrate_1 = bitrate_0 * 2;
                    bitrate_0 = 0;
                } else if -balance > bitrate_0 {
                    bitrate_0 *= 2;
                    bitrate_1 = 0;
                } else {
                    bitrate_1 = bitrate_0 + balance;
                    bitrate_0 -= balance;
                }
            }
            self.c[0].error_limit = limit(self.c[0].slow_level, bitrate_0);
            self.c[1].error_limit = limit(self.c[1].slow_level, bitrate_1);
        } else {
            self.c[0].error_limit = exp2s(bitrate_0) as u32;
            self.c[1].error_limit = exp2s(bitrate_1) as u32;
        }
    }
}

const LOG2_TABLE: [u8; 256] = [
    0x00, 0x01, 0x03, 0x04, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x10, 0x11, 0x12, 0x14, 0x15, 0x16, 0x18, 0x19, 0x1a, 0x1c, 0x1d, 0x1e, 0x20, 0x21, 0x22,
    0x24, 0x25, 0x26, 0x28, 0x29, 0x2a, 0x2c, 0x2d, 0x2e, 0x2f, 0x31, 0x32, 0x33, 0x34, 0x36, 0x37, 0x38, 0x39, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x41, 0x42, 0x43,
    0x44, 0x45, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x61,
    0x62, 0x63, 0x64, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d,
    0x7e, 0x7f, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
    0x98, 0x99, 0x9a, 0x9b, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
    0xb0, 0xb1, 0xb2, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcb, 0xcc, 0xcd, 0xce, 0xcf, 0xd0, 0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd8, 0xd9, 0xda, 0xdb,
    0xdc, 0xdc, 0xdd, 0xde, 0xdf, 0xe0, 0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe4, 0xe5, 0xe6, 0xe7, 0xe7, 0xe8, 0xe9, 0xea, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xee, 0xef,
    0xf0, 0xf1, 0xf1, 0xf2, 0xf3, 0xf4, 0xf4, 0xf5, 0xf6, 0xf7, 0xf7, 0xf8, 0xf9, 0xf9, 0xfa, 0xfb, 0xfc, 0xfc, 0xfd, 0xfe, 0xff, 0xff,
];

const EXP2_TABLE: [u8; 256] = [
    0x00, 0x01, 0x01, 0x02, 0x03, 0x03, 0x04, 0x05, 0x06, 0x06, 0x07, 0x08, 0x08, 0x09, 0x0a, 0x0b, 0x0b, 0x0c, 0x0d, 0x0e, 0x0e, 0x0f, 0x10, 0x10, 0x11, 0x12,
    0x13, 0x13, 0x14, 0x15, 0x16, 0x16, 0x17, 0x18, 0x19, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1d, 0x1e, 0x1f, 0x20, 0x20, 0x21, 0x22, 0x23, 0x24, 0x24, 0x25, 0x26,
    0x27, 0x28, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3a, 0x3b,
    0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x51, 0x52,
    0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b,
    0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85,
    0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9f, 0xa0, 0xa1, 0xa2,
    0xa3, 0xa4, 0xa5, 0xa6, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xaf, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbc, 0xbd, 0xbe, 0xbf, 0xc0,
    0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc8, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf, 0xd0, 0xd2, 0xd3, 0xd4, 0xd6, 0xd7, 0xd8, 0xd9, 0xdb, 0xdc, 0xdd, 0xde, 0xe0, 0xe1,
    0xe2, 0xe4, 0xe5, 0xe6, 0xe8, 0xe9, 0xea, 0xec, 0xed, 0xee, 0xf0, 0xf1, 0xf2, 0xf4, 0xf5, 0xf6, 0xf8, 0xf9, 0xfa, 0xfc, 0xfd, 0xff,
];

#[inline]
fn nbits(v: u32) -> u32 {
    32 - v.leading_zeros()
}

/// The reference `wp_log2`: an 8.8 fixed-point log2 that is linear around zero.
pub fn log2(avalue: u32) -> i32 {
    let a = avalue.wrapping_add(avalue >> 9);
    let dbits = nbits(a);
    let frac = if a < 256 { (a << (9 - dbits)) & 0xff } else { (a >> (dbits - 9)) & 0xff };
    ((dbits << 8) + u32::from(LOG2_TABLE.get(frac as usize).copied().unwrap_or(0))) as i32
}

/// The reference `wp_exp2s`: the inverse of [`log2`], signed.
pub fn exp2s(log: i32) -> i32 {
    if log < 0 {
        // ~((uint32_t) wp_exp2s (-log) - 1), i.e. the negation, for any input
        return !(exp2s(log.wrapping_neg().max(0)) as u32).wrapping_sub(1) as i32;
    }
    let value = u32::from(EXP2_TABLE.get((log & 0xff) as usize).copied().unwrap_or(0)) | 0x100;
    let shift = log >> 8;
    if shift <= 9 { (value >> (9 - shift)) as i32 } else { value.wrapping_shl(((shift - 9) & 0x1f) as u32) as i32 }
}

/// The reference `restore_weight`: an 8-bit stored weight back to ±1024.
pub fn restore_weight(w: i8) -> i32 {
    let mut r = i32::from(w) * 8;
    if r > 0 {
        r += (r + 64) >> 7;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_and_exp_round_trip() {
        assert_eq!((exp2s(0), exp2s(256)), (0, 1));
        assert_eq!(exp2s(-256), -1);
        assert_eq!(log2(0), 0);
        for v in [1u32, 2, 3, 100, 115, 1000, 65535, 1 << 20, 1 << 30] {
            let back = exp2s(log2(v)) as i64;
            assert!((back - i64::from(v)).abs() * 225 <= i64::from(v) + 225, "{v} -> {back}");
        }
        // hostile logs do not panic
        for l in [i32::MIN, i32::MIN + 1, -8192, 8447, 65535, i32::MAX] {
            let _ = exp2s(l);
        }
        assert_eq!(restore_weight(127), 1024);
        assert_eq!(restore_weight(-128), -1024);
    }

    #[test]
    fn codes_read_lsb_first() {
        let mut b = Bits::new(&[0b1010_0101, 0xff]);
        assert_eq!(b.bits(4), 0b0101);
        assert_eq!(b.code(3), 1);
        // past the end: zeros
        let mut e = Bits::new(&[]);
        assert_eq!(e.bits(32), 0);
        assert_eq!(e.ones(), Some(0));
    }
}
