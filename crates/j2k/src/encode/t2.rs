//! Tier-2 encoding (T.800 Annex B): packet header bits, tag trees and packets of a single quality
//! layer, the mirror of [`crate::t2`] and the decoder's packet reader.

use crate::decode::floor_log2;

/// Packet header bits (B.10.1): after a 0xFF byte the next byte carries 7 bits, its MSB a stuffed
/// zero.
pub(crate) struct BitWriter {
    pub out: Vec<u8>,
    buf: u8,
    n: u32,
    cap: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        BitWriter { out: Vec::new(), buf: 0, n: 0, cap: 8 }
    }

    pub fn bit(&mut self, b: u32) {
        self.buf = (self.buf << 1) | (b & 1) as u8;
        self.n += 1;
        if self.n == self.cap {
            self.emit();
        }
    }

    pub fn bits(&mut self, v: u32, n: u32) {
        for k in (0..n.min(32)).rev() {
            self.bit(v >> k);
        }
    }

    fn emit(&mut self) {
        self.out.push(self.buf);
        self.cap = if self.buf == 0xFF { 7 } else { 8 };
        self.buf = 0;
        self.n = 0;
    }

    /// Pads the last byte with zeros; a final 0xFF is followed by a zero byte, which the decoder
    /// skips (B.10.1).
    pub fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.buf <<= self.cap - self.n;
            self.emit();
        }
        if self.out.last() == Some(&0xFF) {
            self.out.push(0);
        }
        self.out
    }

    /// Number of coding passes (Table B.4), 1-164.
    pub fn passes(&mut self, n: u32) {
        match n {
            0..=1 => self.bit(0),
            2 => self.bits(0b10, 2),
            3..=5 => self.bits(0b1100 | (n - 3), 4),
            6..=36 => self.bits((0b1111 << 5) | (n - 6), 9),
            _ => self.bits((0b1_1111_1111 << 7) | (n.min(164) - 37), 16),
        }
    }
}

/// A tag tree encoder (B.10.2) over a grid of code-blocks.
pub(crate) struct TagTree {
    /// (width, height, offset) of each level, leaves first.
    levels: Vec<(usize, usize, usize)>,
    value: Vec<u32>,
    low: Vec<u32>,
    known: Vec<bool>,
}

impl TagTree {
    /// A tree over the `w` × `h` leaves `leaves` (raster order); a node holds its children's
    /// minimum.
    pub fn new(w: usize, h: usize, leaves: &[u32]) -> Self {
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
        let mut value = vec![u32::MAX; off];
        for (v, &l) in value.iter_mut().zip(leaves.iter().take(w * h)) {
            *v = l;
        }
        for k in 1..levels.len() {
            let (cw, ch, coff) = levels[k - 1];
            let (pw, _, poff) = levels[k];
            for y in 0..ch {
                for x in 0..cw {
                    let c = value[coff + y * cw + x];
                    let p = &mut value[poff + (y / 2) * pw + x / 2];
                    *p = (*p).min(c);
                }
            }
        }
        TagTree { levels, low: vec![0; off], known: vec![false; off], value }
    }

    /// Codes whether the value of leaf (x, y) is below `threshold`, as the decoder reads it.
    pub fn encode(&mut self, w: &mut BitWriter, x: usize, y: usize, threshold: u32) {
        let mut low = 0;
        for (l, &(lw, _, off)) in self.levels.iter().enumerate().rev() {
            let i = off + (y >> l) * lw + (x >> l);
            let (Some(node_low), Some(&value)) = (self.low.get(i).copied(), self.value.get(i)) else { return };
            if low > node_low {
                self.low[i] = low;
            } else {
                low = node_low;
            }
            while low < threshold {
                if low >= value {
                    if !self.known[i] {
                        w.bit(1);
                        self.known[i] = true;
                    }
                    break;
                }
                w.bit(0);
                low += 1;
            }
            self.low[i] = low;
        }
    }
}

/// One code-block's contribution to the packet of its precinct (single quality layer).
#[derive(Clone, Copy)]
pub(crate) struct Contribution<'a> {
    /// Missing most significant bit-planes (Mb - coded planes).
    pub zero: u32,
    pub passes: u32,
    pub data: &'a [u8],
}

/// The packet of a precinct in layer 0: per band, its code-block grid width and its blocks'
/// contributions (raster order, `None` for a block not included). The header, followed by the
/// body with `body`, and the body's length.
pub(crate) fn packet(bands: &[(usize, Vec<Option<Contribution>>)], body: bool) -> (Vec<u8>, usize) {
    let mut w = BitWriter::new();
    let any = bands.iter().any(|(_, b)| b.iter().any(Option::is_some));
    if !any {
        w.bit(0);
        return (w.finish(), 0);
    }
    w.bit(1);
    let mut data = Vec::new();
    let mut len_sum = 0;
    for (cw, blocks) in bands {
        let cw = (*cw).max(1);
        let ch = blocks.len().div_ceil(cw);
        let incl: Vec<u32> = blocks.iter().map(|b| u32::from(b.is_none())).collect();
        let zeros: Vec<u32> = blocks.iter().map(|b| b.map_or(0, |b| b.zero)).collect();
        let mut incl = TagTree::new(cw, ch, &incl);
        let mut zero = TagTree::new(cw, ch, &zeros);
        for (k, b) in blocks.iter().enumerate() {
            let (x, y) = (k % cw, k / cw);
            incl.encode(&mut w, x, y, 1);
            let Some(b) = b else { continue };
            for t in 1..=b.zero + 1 {
                zero.encode(&mut w, x, y, t);
            }
            w.passes(b.passes);
            // Lblock (B.10.7.1): 3, raised until the length fits
            let len = b.data.len() as u32;
            let need = 32 - len.leading_zeros();
            let extra = floor_log2(b.passes);
            let mut lblock = 3;
            while lblock + extra < need {
                w.bit(1);
                lblock += 1;
            }
            w.bit(0);
            w.bits(len, lblock + extra);
            len_sum += b.data.len();
            if body {
                data.extend_from_slice(b.data);
            }
        }
    }
    let mut out = w.finish();
    out.extend_from_slice(&data);
    (out, len_sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::t2::{Bits, TagTree as Decoder};

    #[test]
    fn pass_counts_round_trip() {
        let mut w = BitWriter::new();
        let counts: Vec<u32> = (1..=164).collect();
        for &n in &counts {
            w.passes(n);
        }
        let d = w.finish();
        let mut b = Bits::new(&d, 0);
        for &n in &counts {
            assert_eq!(b.passes().unwrap(), n);
        }
    }

    #[test]
    fn stuffing_follows_ff() {
        let mut w = BitWriter::new();
        w.bits(0xFF, 8);
        w.bits(0x7F, 7);
        w.bits(0b1, 1);
        let d = w.finish();
        assert_eq!(d, [0xFF, 0x7F, 0x80]);
        let mut w = BitWriter::new();
        w.bits(0xFF, 8);
        assert_eq!(w.finish(), [0xFF, 0x00]);
    }

    #[test]
    fn tag_trees_decode_to_their_values() {
        let mut seed = 7u32;
        for (cw, ch) in [(1, 1), (3, 2), (5, 7), (16, 1), (8, 8)] {
            let vals: Vec<u32> = (0..cw * ch)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) % 9
                })
                .collect();
            let mut enc = TagTree::new(cw, ch, &vals);
            let mut w = BitWriter::new();
            // as the decoder asks: each leaf with growing thresholds until it is below
            for (k, &v) in vals.iter().enumerate() {
                for t in 1..=v + 1 {
                    enc.encode(&mut w, k % cw, k / cw, t);
                }
            }
            let d = w.finish();
            let mut b = Bits::new(&d, 0);
            let mut dec = Decoder::new(cw, ch);
            for (k, &v) in vals.iter().enumerate() {
                let mut t = 1;
                while !dec.decode(&mut b, k % cw, k / cw, t).unwrap() {
                    t += 1;
                }
                assert_eq!(t - 1, v, "{cw}x{ch} leaf {k}");
            }
        }
    }
}
