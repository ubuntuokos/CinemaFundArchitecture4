// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! MSB-first bit reader for EVC NAL unit payloads (`xevd_bsr.c`).
//!
//! EVC has no emulation prevention bytes: NAL units are length-prefixed, so the
//! payload is read as it is. Reading past the end yields zero bits and sets
//! [`Bits::overrun`], which the callers turn into an error.

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    /// Position in bits from the start of `data`.
    pos: usize,
    pub(crate) overrun: bool,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, overrun: false }
    }

    pub(crate) fn bit(&mut self) -> u32 {
        let byte = self.pos >> 3;
        let Some(&b) = self.data.get(byte) else {
            self.overrun = true;
            return 0;
        };
        let v = (b >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        u32::from(v)
    }

    /// Reads `n` (≤ 32) bits.
    pub(crate) fn bits(&mut self, n: u32) -> u32 {
        let mut v: u32 = 0;
        for _ in 0..n.min(32) {
            v = (v << 1) | self.bit();
        }
        v
    }

    pub(crate) fn flag(&mut self) -> bool {
        self.bit() != 0
    }

    /// Unsigned Exp-Golomb code. Codes longer than 32 bits are hostile; they
    /// set `overrun` and return 0.
    pub(crate) fn ue(&mut self) -> u32 {
        let mut zeros = 0u32;
        while self.bit() == 0 {
            if self.overrun || zeros >= 31 {
                self.overrun = true;
                return 0;
            }
            zeros += 1;
        }
        let rest = self.bits(zeros);
        ((1u64 << zeros) - 1 + u64::from(rest)).min(u64::from(u32::MAX)) as u32
    }

    pub(crate) fn se(&mut self) -> i32 {
        let v = i64::from(self.ue());
        let s = if v & 1 == 1 { (v + 1) >> 1 } else { -(v >> 1) };
        s as i32
    }

    pub(crate) fn is_byte_aligned(&self) -> bool {
        self.pos & 7 == 0
    }

    /// Bits left in the payload.
    pub(crate) fn left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb() {
        // 1 | 010 | 011 | 00100 | 0001000 -> 0, 1, 2, 3, 7
        let mut b = Bits::new(&[0b1010_0110, 0b0100_0001, 0b0000_0000]);
        assert_eq!(b.ue(), 0);
        assert_eq!(b.ue(), 1);
        assert_eq!(b.ue(), 2);
        assert_eq!(b.ue(), 3);
        assert_eq!(b.ue(), 7);
        assert!(!b.overrun);
        let mut b = Bits::new(&[0b0100_1100]);
        assert_eq!(b.se(), 1);
        assert_eq!(b.se(), -1);
    }

    #[test]
    fn overrun_is_flagged() {
        let mut b = Bits::new(&[0]);
        assert_eq!(b.ue(), 0);
        assert!(b.overrun);
    }
}
