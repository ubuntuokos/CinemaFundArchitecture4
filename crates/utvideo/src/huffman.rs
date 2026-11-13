//! The bit reader and the Huffman tables.

use crate::error::{Error, Result};

/// Bits looked up at once.
const FAST_BITS: u32 = 11;

/// The stream is a sequence of 32-bit little-endian words, each read from its most significant
/// bit down.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Next byte (stream order, after the word swap) to load into `cache`.
    next: usize,
    /// Stream length in bytes, rounded up to whole words.
    len: usize,
    cache: u64,
    /// Valid bits at the top of `cache`.
    avail: u32,
    /// Bits consumed past the end of the data, or an invalid code.
    bad: bool,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, next: 0, len: data.len().div_ceil(4).saturating_mul(4), cache: 0, avail: 0, bad: false }
    }

    #[inline]
    fn byte(&self, p: usize) -> u8 {
        self.data.get((p & !3) + 3 - (p & 3)).copied().unwrap_or(0)
    }

    #[inline]
    fn refill(&mut self) {
        while self.avail <= 56 {
            let b = if self.next < self.len { self.byte(self.next) } else { 0 };
            self.next = self.next.saturating_add(1);
            self.cache |= u64::from(b) << (56 - self.avail);
            self.avail += 8;
        }
    }

    /// The next `n` (1..=32) bits without consuming them.
    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        if self.avail < n {
            self.refill();
        }
        (self.cache >> (64 - n)) as u32
    }

    #[inline]
    fn skip(&mut self, n: u32) {
        self.cache = self.cache.checked_shl(n).unwrap_or(0);
        self.avail = self.avail.saturating_sub(n);
    }

    /// Read `n` (1..=32) bits.
    #[cfg(test)]
    pub fn bits(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n);
        v
    }

    /// Whether more bits were read than the frame holds, or a code was invalid.
    pub fn overrun(&self) -> bool {
        // bits loaded past the end that were then consumed
        let loaded = self.next.saturating_mul(8);
        let consumed = loaded.saturating_sub(self.avail as usize);
        self.bad || consumed > self.len.saturating_mul(8)
    }
}

/// A Huffman table built from 256 code lengths (0: symbol absent).
#[derive(Clone, Debug)]
pub(crate) struct Table {
    /// Indexed by the next `FAST_BITS` bits: (symbol, length), length 0 when the code is longer.
    fast: Vec<(u8, u8)>,
    /// Per length: first code and the symbols of that length in code order.
    first: [u32; 33],
    syms: Vec<Vec<u8>>,
}

impl Table {
    /// Codes are assigned from the longest length down (the longest code is all zeros, the
    /// shortest all ones); within a length in decreasing symbol order, each code one more than
    /// the last; moving to the next shorter length halves the running code. Lengths above 32
    /// (255 in practice) mark absent symbols.
    pub fn new(lens: &[u8; 256]) -> Result<Table> {
        let mut first = [0u32; 33];
        let mut syms: Vec<Vec<u8>> = vec![Vec::new(); 33];
        let mut code: u64 = 0;
        for len in (1..=32u32).rev() {
            first[len as usize] = code as u32;
            for (s, &l) in lens.iter().enumerate().rev() {
                if u32::from(l) == len {
                    if let Some(v) = syms.get_mut(len as usize) {
                        v.push(s as u8);
                    }
                    code += 1;
                }
            }
            if code > 1u64 << len {
                return Err(Error::Invalid("Huffman code lengths oversubscribed".into()));
            }
            code >>= 1;
        }
        let mut fast = vec![(0u8, 0u8); 1 << FAST_BITS];
        for len in 1..=FAST_BITS {
            for (k, &s) in syms.get(len as usize).map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
                let c = first[len as usize] + k as u32;
                let shift = FAST_BITS - len;
                let start = (c << shift) as usize;
                for e in fast.iter_mut().skip(start).take(1 << shift) {
                    *e = (s, len as u8);
                }
            }
        }
        Ok(Table { fast, first, syms })
    }

    /// Decode one symbol; an invalid code marks the reader bad and gives 0.
    #[inline]
    pub fn decode(&self, br: &mut BitReader<'_>) -> u8 {
        let (s, l) = self.fast.get(br.peek(FAST_BITS) as usize).copied().unwrap_or((0, 0));
        if l != 0 {
            br.skip(u32::from(l));
            return s;
        }
        let bits = br.peek(32);
        for len in FAST_BITS + 1..=32 {
            let c = (u64::from(bits) >> (32 - len)) as u32;
            let Some(k) = c.checked_sub(self.first[len as usize]) else { continue };
            if let Some(&s) = self.syms.get(len as usize).and_then(|v| v.get(k as usize)) {
                br.skip(len);
                return s;
            }
        }
        br.bad = true;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_read_most_significant_bit_first() {
        let mut br = BitReader::new(&[0x78, 0x56, 0x34, 0x12, 0xAA]);
        assert_eq!(br.bits(8), 0x12);
        assert_eq!(br.bits(16), 0x3456);
        assert_eq!(br.bits(8), 0x78);
        assert_eq!(br.bits(24), 0);
        assert_eq!(br.bits(8), 0xAA);
        assert!(!br.overrun());
        br.bits(1);
        assert!(br.overrun());
    }

    #[test]
    fn codes_from_lengths() {
        // lengths 1, 2, 3, 3 for symbols 0..3: longest first, higher symbols first:
        // 3 = 000, 2 = 001, 1 = 01, 0 = 1
        let mut lens = [255u8; 256];
        lens[..4].copy_from_slice(&[1, 2, 3, 3]);
        let t = Table::new(&lens).unwrap();
        // bits: 1 01 001 000 -> 0, 1, 2, 3
        let mut br = BitReader::new(&[0, 0, 0x00, 0xA4]);
        assert_eq!([t.decode(&mut br), t.decode(&mut br), t.decode(&mut br), t.decode(&mut br)], [0, 1, 2, 3]);
        assert!(!br.overrun());
    }

    #[test]
    fn oversubscribed_lengths_are_errors() {
        assert!(Table::new(&[1u8; 256]).is_err());
    }

    #[test]
    fn thirty_two_bit_codes_decode() {
        // symbol k has length k + 1 (1..=32) for k < 32, symbol 32 shares length 32
        let mut lens = [255u8; 256];
        for (k, l) in lens.iter_mut().enumerate().take(32) {
            *l = k as u8 + 1;
        }
        lens[32] = 32;
        let t = Table::new(&lens).unwrap();
        // the all-zero 32-bit code is the higher symbol of the longest length
        let mut br = BitReader::new(&[0, 0, 0, 0]);
        assert_eq!(t.decode(&mut br), 32);
        assert!(!br.overrun());
    }
}
