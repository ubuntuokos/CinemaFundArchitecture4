//! Boolean (arithmetic) encoder: the inverse of the decoder of section 9.2. Written from the
//! decoding process: the coder keeps the low end of the interval with 24 bits of precision
//! below the pending output byte and propagates carries into bytes already written.

/// Writes bools with 8-bit probabilities, as [`crate::boolcoder::BoolDecoder`] reads them.
pub struct BoolEncoder {
    out: Vec<u8>,
    /// Low end of the interval; bit 31 is the carry into the bytes already written.
    bottom: u32,
    range: u32,
    /// Shifts left until the next byte is complete.
    bit_count: u32,
}

impl Default for BoolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl BoolEncoder {
    /// A new coder; the first bool is the marker bit init_bool reads (9.2.1), written as 0.
    pub fn new() -> Self {
        let mut e = BoolEncoder { out: Vec::new(), bottom: 0, range: 255, bit_count: 24 };
        e.write_bool(false, 128);
        e
    }

    /// Write `bit` with probability `p / 256` of it being false.
    pub fn write_bool(&mut self, bit: bool, p: u8) {
        let split = 1 + (((self.range - 1) * u32::from(p)) >> 8);
        if bit {
            self.bottom = self.bottom.wrapping_add(split);
            self.range -= split;
        } else {
            self.range = split;
        }
        while self.range < 128 {
            self.range <<= 1;
            if self.bottom & (1 << 31) != 0 {
                self.carry();
            }
            self.bottom <<= 1;
            self.bit_count -= 1;
            if self.bit_count == 0 {
                self.out.push((self.bottom >> 24) as u8);
                self.bottom &= (1 << 24) - 1;
                self.bit_count = 8;
            }
        }
    }

    fn carry(&mut self) {
        for b in self.out.iter_mut().rev() {
            if *b == 0xff {
                *b = 0;
            } else {
                *b += 1;
                return;
            }
        }
    }

    /// `n` bits of `v`, most significant first, each with probability 128 (read_literal).
    pub fn write_literal(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.write_bool((v >> i) & 1 == 1, 128);
        }
    }

    /// The path of leaf `value` through `tree` (9.3.3), each node with `probs[node >> 1]`.
    /// Nothing is written for a value the tree does not hold.
    pub fn write_tree(&mut self, tree: &[i8], probs: &[u8], value: u8) {
        let mut path = [false; 16];
        let mut nodes = [0usize; 16];
        if let Some(len) = tree_path(tree, 0, value, &mut path, &mut nodes, 0) {
            for (bit, node) in path.iter().zip(&nodes).take(len) {
                self.write_bool(*bit, probs.get(node >> 1).copied().unwrap_or(128));
            }
        }
    }

    /// Bytes written so far would decode as what was written once [`Self::finish`] pads them.
    pub fn finish(mut self) -> Vec<u8> {
        // enough zero bits to flush the 24 bits of `low` and to keep the decoder's look-ahead
        // inside the data
        for _ in 0..32 {
            self.write_bool(false, 128);
        }
        // a last byte that looks like a superframe marker would be misread as an index
        if self.out.last().is_some_and(|&b| b & 0xe0 == 0xc0) {
            self.out.push(0);
        }
        self.out
    }
}

/// Depth-first search for `value` below node `n`: the bits and node indices of its path.
fn tree_path(tree: &[i8], n: usize, value: u8, path: &mut [bool; 16], nodes: &mut [usize; 16], depth: usize) -> Option<usize> {
    if depth >= 16 {
        return None;
    }
    for bit in [false, true] {
        let next = *tree.get(n + bit as usize)?;
        path[depth] = bit;
        nodes[depth] = n;
        if next <= 0 {
            if (-next) as u8 == value {
                return Some(depth + 1);
            }
        } else if let Some(len) = tree_path(tree, next as usize, value, path, nodes, depth + 1) {
            return Some(len);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolcoder::BoolDecoder;
    use crate::tables::*;

    #[test]
    fn random_bools_and_trees_round_trip() {
        let mut seed = 0x1234_5678u32;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let items: Vec<(bool, u8)> = (0..20_000)
            .map(|_| {
                let p = (rnd() % 255 + 1) as u8;
                // skewed towards the likely value, as real data is
                (rnd() % 256 >= u32::from(p), p)
            })
            .collect();
        let probs: Vec<u8> = (0..9).map(|_| (rnd() % 255 + 1) as u8).collect();
        let modes: Vec<u8> = (0..500).map(|_| (rnd() % 10) as u8).collect();
        let mut e = BoolEncoder::new();
        for &(b, p) in &items {
            e.write_bool(b, p);
        }
        for &m in &modes {
            e.write_tree(&INTRA_MODE_TREE, &probs, m);
        }
        e.write_literal(0x2a5, 10);
        let data = e.finish();
        let mut d = BoolDecoder::new(&data);
        for (i, &(b, p)) in items.iter().enumerate() {
            assert_eq!(d.read_bool(p), b, "bool {i}");
        }
        for &m in &modes {
            assert_eq!(d.read_tree(&INTRA_MODE_TREE, &probs), m);
        }
        assert_eq!(d.read_literal(10), 0x2a5);
    }

    #[test]
    fn long_runs_of_certain_bits_carry_correctly() {
        for p in [1u8, 2, 128, 254, 255] {
            for bit in [false, true] {
                let mut e = BoolEncoder::new();
                for _ in 0..5000 {
                    e.write_bool(bit, p);
                }
                let data = e.finish();
                let mut d = BoolDecoder::new(&data);
                for i in 0..5000 {
                    assert_eq!(d.read_bool(p), bit, "p {p} bit {bit} at {i}");
                }
            }
        }
    }
}
