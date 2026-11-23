// After atracdenc 0.2.3 src/lib/bitstream (Copyright (c) Daniil Cherednik; LGPL-2.1-or-later);
// see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! An MSB-first bit writer.

#[derive(Default, Clone)]
pub(crate) struct BitWriter {
    buf: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Writes the low `n` bits of `val` (n ≤ 32), most significant first.
    pub(crate) fn write(&mut self, val: u32, n: u32) {
        for i in (0..n).rev() {
            let bit = (val >> i) & 1;
            if self.bits.is_multiple_of(8) {
                self.buf.push(0);
            }
            if bit != 0 {
                let last = self.buf.len() - 1;
                self.buf[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    pub(crate) fn len_bits(&self) -> usize {
        self.bits
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msb_first() {
        let mut w = BitWriter::new();
        w.write(0x28, 6);
        w.write(3, 2);
        w.write(1, 1);
        assert_eq!(w.len_bits(), 9);
        assert_eq!(w.into_bytes(), vec![0xA3, 0x80]);
    }
}
