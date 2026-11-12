// Ported from FFmpeg n7.1.1 (libavcodec/get_bits.h and vlc.c semantics), LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! An MSB-first bit reader (bits past the end read as 0, as FFmpeg's zeroed padding) and a
//! single-level VLC lookup table built from `(code, length)` pairs whose symbol is their index.

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The next `n` (≤ 32) bits without consuming them.
    pub(crate) fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let byte = self.pos / 8;
        let mut w = 0u64;
        for i in 0..8 {
            w = (w << 8) | u64::from(self.data.get(byte.saturating_add(i)).copied().unwrap_or(0));
        }
        let w = w << (self.pos % 8);
        (w >> (64 - n.min(32))) as u32
    }

    pub(crate) fn skip(&mut self, n: u32) {
        // capped a little past the end, as FFmpeg's checked reader
        let cap = self.data.len().saturating_mul(8).saturating_add(8);
        self.pos = self.pos.saturating_add(n as usize).min(cap);
    }

    pub(crate) fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n);
        v
    }

    pub(crate) fn bit(&mut self) -> bool {
        self.get(1) != 0
    }

    /// `n` bits as a two's complement number.
    pub(crate) fn sbits(&mut self, n: u32) -> i32 {
        let v = self.get(n);
        let s = 32 - n;
        ((v << s) as i32) >> s
    }

    /// Bits left in the buffer (negative once read past its end).
    pub(crate) fn left(&self) -> i64 {
        (self.data.len() as i64).saturating_mul(8).saturating_sub(self.pos as i64)
    }
}

/// A complete lookup over the longest code: `(symbol, length)`, length 0 for no code.
pub(crate) struct Vlc {
    bits: u32,
    table: Vec<(u16, u8)>,
}

impl Vlc {
    /// Codes `codes[i]` of `lens[i]` bits (0 = unused) for symbol `i`.
    pub(crate) fn new(codes: &[u16], lens: &[u8]) -> Self {
        let bits = u32::from(lens.iter().copied().max().unwrap_or(0)).min(16);
        let mut table = vec![(0u16, 0u8); 1usize << bits];
        for (sym, (&code, &len)) in codes.iter().zip(lens).enumerate() {
            let len32 = u32::from(len);
            if len == 0 || len32 > bits {
                continue;
            }
            let shift = bits - len32;
            let start = (usize::from(code) & ((1usize << len32) - 1)) << shift;
            for e in table.iter_mut().skip(start).take(1usize << shift) {
                *e = (sym as u16, len);
            }
        }
        Self { bits, table }
    }

    /// The next symbol, or `None` (consuming nothing) when the bits are no code.
    pub(crate) fn read(&self, b: &mut Bits) -> Option<usize> {
        let &(sym, len) = self.table.get(b.peek(self.bits) as usize)?;
        if len == 0 {
            return None;
        }
        b.skip(u32::from(len));
        Some(usize::from(sym))
    }
}

/// `ff_zigzag_direct`.
pub(crate) const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_msb_first_and_zeros_past_the_end() {
        let mut b = Bits::new(&[0b1010_0000, 0xFF]);
        assert_eq!(b.get(3), 0b101);
        assert_eq!(b.sbits(5), 0);
        assert_eq!(b.sbits(4), -1);
        assert_eq!(b.get(8), 0xF0);
        assert_eq!(b.left(), -4);
        b.skip(1000);
        assert_eq!(b.get(32), 0);
    }

    #[test]
    fn vlc_lookup() {
        // 0 -> "1", 1 -> "01", 2 -> "00"
        let v = Vlc::new(&[1, 1, 0], &[1, 2, 2]);
        let mut b = Bits::new(&[0b1010_0100]);
        let got: Vec<_> = (0..4).filter_map(|_| v.read(&mut b)).collect();
        assert_eq!(got, [0, 1, 2, 0]);
        let partial = Vlc::new(&[1], &[2]);
        assert_eq!(partial.read(&mut Bits::new(&[0])), None);
    }
}
