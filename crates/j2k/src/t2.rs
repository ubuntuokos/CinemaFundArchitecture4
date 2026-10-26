//! Tier-2 (T.800 Annex B): packet headers, tag trees and the code-block contributions of each
//! packet.

use crate::{Error, Result};

/// Packet header bits (B.10.1): after a 0xFF byte the next byte carries 7 bits, its MSB being a
/// stuffed zero.
pub(crate) struct Bits<'a> {
    d: &'a [u8],
    pub pos: usize,
    buf: u8,
    ct: u32,
    prev: u8,
}

impl<'a> Bits<'a> {
    pub fn new(d: &'a [u8], pos: usize) -> Self {
        Bits { d, pos, buf: 0, ct: 0, prev: 0 }
    }

    pub fn bit(&mut self) -> Result<u32> {
        if self.ct == 0 {
            let b = *self.d.get(self.pos).ok_or(Error::Truncated)?;
            self.pos += 1;
            self.ct = if self.prev == 0xFF { 7 } else { 8 };
            self.buf = b;
            self.prev = b;
        }
        self.ct -= 1;
        Ok(u32::from(self.buf >> self.ct) & 1)
    }

    pub fn bits(&mut self, n: u32) -> Result<u32> {
        if n > 32 {
            return Err(Error::Invalid("packet header field wider than 32 bits"));
        }
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }

    /// End of the header: the rest of the byte, and the byte after a final 0xFF.
    pub fn align(&mut self) {
        self.ct = 0;
        if self.prev == 0xFF {
            self.pos += 1;
            self.prev = 0;
        }
    }

    /// Number of coding passes (Table B.4).
    pub fn passes(&mut self) -> Result<u32> {
        if self.bit()? == 0 {
            return Ok(1);
        }
        if self.bit()? == 0 {
            return Ok(2);
        }
        let v = self.bits(2)?;
        if v < 3 {
            return Ok(3 + v);
        }
        let v = self.bits(5)?;
        if v < 31 {
            return Ok(6 + v);
        }
        Ok(37 + self.bits(7)?)
    }
}

/// A tag tree (B.10.2) over a grid of code-blocks.
pub(crate) struct TagTree {
    /// (width, height, offset) of each level, leaves first.
    levels: Vec<(usize, usize, usize)>,
    value: Vec<u32>,
    low: Vec<u32>,
}

impl TagTree {
    pub fn new(w: usize, h: usize) -> Self {
        let mut levels = Vec::new();
        let (mut lw, mut lh, mut off) = (w.max(1), h.max(1), 0);
        loop {
            levels.push((lw, lh, off));
            off += lw * lh;
            if lw == 1 && lh == 1 {
                break;
            }
            lw = lw.div_ceil(2);
            lh = lh.div_ceil(2);
        }
        TagTree { levels, value: vec![u32::MAX; off], low: vec![0; off] }
    }

    /// Whether the value of leaf (x, y) is below `threshold`, reading the bits that decide it.
    pub fn decode(&mut self, bits: &mut Bits, x: usize, y: usize, threshold: u32) -> Result<bool> {
        let mut low = 0;
        let mut leaf = 0;
        for (l, &(lw, _, off)) in self.levels.iter().enumerate().rev() {
            let i = off + (y >> l) * lw + (x >> l);
            let (Some(node_low), Some(&node_value)) = (self.low.get(i).copied(), self.value.get(i)) else {
                return Err(Error::Invalid("tag tree index"));
            };
            if low > node_low {
                self.low[i] = low;
            } else {
                low = node_low;
            }
            let mut value = node_value;
            while low < threshold && low < value {
                if bits.bit()? == 1 {
                    value = low;
                } else {
                    low += 1;
                }
            }
            self.value[i] = value;
            self.low[i] = low;
            leaf = i;
        }
        Ok(self.value.get(leaf).is_some_and(|&v| v < threshold))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuffed_bits_after_ff_are_skipped() {
        // 0xFF then 0x7F: 8 ones, then 7 bits of 0x7F (MSB stuffed)
        let d = [0xFF, 0x7F, 0x80];
        let mut b = Bits::new(&d, 0);
        assert_eq!(b.bits(15).ok(), Some(0x7FFF));
        assert_eq!(b.bit().ok(), Some(1));
    }

    #[test]
    fn pass_counts_follow_table_b4() {
        // 0 → 1; 10 → 2; 1100 → 3; 1111 00000 → 6; 1111 11110 → 36
        let d = [0b0101_1001, 0b1110_0000, 0b1111_1111, 0b0000_0000];
        let mut b = Bits::new(&d, 0);
        assert_eq!(b.passes().ok(), Some(1));
        assert_eq!(b.passes().ok(), Some(2));
        assert_eq!(b.passes().ok(), Some(3));
        assert_eq!(b.passes().ok(), Some(6));
        assert_eq!(b.passes().ok(), Some(36));
    }

    #[test]
    fn tag_tree_decodes_the_example_values() {
        // a 1x1 tree holding 2: bits 0 0 1
        let d = [0b0010_0000];
        let mut b = Bits::new(&d, 0);
        let mut t = TagTree::new(1, 1);
        assert_eq!(t.decode(&mut b, 0, 0, 1).ok(), Some(false));
        assert_eq!(t.decode(&mut b, 0, 0, 3).ok(), Some(true));
        assert_eq!(b.pos, 1);
    }
}
