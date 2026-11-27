//! Prefix codes given as lists of (length, symbol) in left-to-right tree order, the way
//! FFmpeg's `ff_vlc_init_from_lengths` assigns them: each code is the running sum of
//! `2^-length` of the codes before it.

use crate::bits::Bits;
use crate::{Error, Result};

const FAST_BITS: u32 = 11;

#[derive(Clone)]
pub(crate) struct Vlc {
    /// `(symbol, length)` by the next `FAST_BITS` bits; length 0 means a longer code or none.
    fast: Vec<(i16, u8)>,
    /// `(code, length, symbol)` of the codes longer than `FAST_BITS`.
    slow: Vec<(u32, u8, i16)>,
}

impl Vlc {
    /// A code from `(length, symbol)` pairs in tree order. A list that over-fills the code
    /// space, or has a zero length with other codes, is an error; an incomplete one is allowed
    /// (its missing codes fail to decode).
    pub(crate) fn new(entries: &[(u8, i16)]) -> Result<Self> {
        let mut fast = vec![(0i16, 0u8); 1 << FAST_BITS];
        let mut slow = Vec::new();
        // the running sum of 2^-len, as a fraction of 2^32
        let mut acc: u64 = 0;
        for &(len, sym) in entries {
            if len == 0 || len > 32 {
                // a lone zero-length code (a one-leaf tree) is a code of no bits
                if len == 0 && entries.len() == 1 {
                    return Err(Error::Unsupported("Huffman table with a single zero-length code"));
                }
                return Err(Error::Invalid("Huffman code length"));
            }
            let code = (acc >> (32 - u32::from(len))) as u32;
            acc += 1u64 << (32 - u32::from(len));
            if acc > 1u64 << 32 {
                return Err(Error::Invalid("Huffman code space overflow"));
            }
            if u32::from(len) <= FAST_BITS {
                let shift = FAST_BITS - u32::from(len);
                let base = (code as usize) << shift;
                for k in 0..(1usize << shift) {
                    if let Some(f) = fast.get_mut(base + k) {
                        *f = (sym, len);
                    }
                }
            } else {
                slow.push((code, len, sym));
            }
        }
        Ok(Self { fast, slow })
    }

    /// The next symbol, or `None` when the bits match no code.
    pub(crate) fn read(&self, b: &mut Bits<'_>) -> Option<i16> {
        let (sym, len) = self.fast.get(b.peek(FAST_BITS) as usize).copied().unwrap_or((0, 0));
        if len > 0 {
            b.skip(usize::from(len));
            return Some(sym);
        }
        let v = b.peek(32);
        for &(code, len, sym) in &self.slow {
            if v >> (32 - u32::from(len)) == code {
                b.skip(usize::from(len));
                return Some(sym);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superblock_run_code_matches_the_specification() {
        // lengths of the long-run code (runs 1..=33, then the 12-bit escape)
        let mut lens = vec![1u8, 3, 3, 4, 4, 6, 6, 6, 6];
        lens.extend([8; 8]);
        lens.extend([10; 16]);
        lens.push(6);
        let e: Vec<(u8, i16)> = lens.iter().enumerate().map(|(i, &l)| (l, i as i16 + 1)).collect();
        let v = Vlc::new(&e).unwrap();
        for (bits, run) in [(&[0x00u8][..], 1), (&[0b1000_0000], 2), (&[0b1011_0000], 3), (&[0b1111_1100], 34), (&[0b1111_0000, 0], 10)] {
            let mut b = Bits::new(bits);
            assert_eq!(v.read(&mut b), Some(run), "{bits:?}");
        }
    }

    #[test]
    fn overfull_and_empty_codes() {
        assert!(Vlc::new(&[(1, 0), (1, 1), (1, 2)]).is_err());
        assert!(Vlc::new(&[(0, 0)]).is_err());
        let v = Vlc::new(&[(2, 5)]).unwrap();
        assert_eq!(v.read(&mut Bits::new(&[0xFF])), None);
    }
}
