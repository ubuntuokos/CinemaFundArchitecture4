// Bit reading after FFmpeg n7.1.1 libavcodec/get_bits.h (the checked reader) and the canonical
// code assignment of ff_vlc_init_from_lengths, LGPL-2.1-or-later; see docs/licensing.md §5. CFA4
// changes GPL-3.0-or-later.

//! An MSB-first bit reader with FFmpeg's checked-reader semantics and canonical prefix codes.

/// Bits past the end read as zero; the position stops 8 bits past the end, as FFmpeg's
/// `size_in_bits_plus8` clamp does, so `count` behaves the same on overreads.
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    limit: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8], size_bits: usize) -> Self {
        Self { data, pos: 0, limit: size_bits.saturating_add(8) }
    }

    fn bit_at(&self, p: usize) -> u32 {
        self.data.get(p / 8).map_or(0, |b| u32::from(b >> (7 - (p % 8)) & 1))
    }

    pub(crate) fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n as usize {
            v = (v << 1) | self.bit_at(self.pos + i);
        }
        self.pos = (self.pos + n as usize).min(self.limit);
        v
    }

    /// The next `n` bits without consuming them.
    pub(crate) fn show(&self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n as usize {
            v = (v << 1) | self.bit_at(self.pos + i);
        }
        v
    }

    pub(crate) fn skip(&mut self, n: u32) {
        self.pos = (self.pos + n as usize).min(self.limit);
    }

    pub(crate) fn get1(&mut self) -> bool {
        self.get(1) != 0
    }

    pub(crate) fn count(&self) -> usize {
        self.pos
    }

    pub(crate) fn left(&self) -> isize {
        (self.limit as isize - 8) - self.pos as isize
    }
}

/// A prefix code given as code counts per length (1..=16) and the symbols in code order
/// (`ff_vlc_init_from_lengths` with lengths in increasing order: a canonical code).
#[derive(Clone)]
pub(crate) struct Huff {
    counts: [u16; 16],
    syms: Vec<i32>,
}

impl Huff {
    pub(crate) fn new(counts: &[u8], syms: impl Iterator<Item = i32>) -> Self {
        let mut c = [0u16; 16];
        for (d, s) in c.iter_mut().zip(counts) {
            *d = u16::from(*s);
        }
        let total: usize = c.iter().map(|&x| usize::from(x)).sum();
        Self { counts: c, syms: syms.take(total).collect() }
    }

    /// The symbol, or `None` for a code that is not in the table.
    pub(crate) fn read(&self, b: &mut Bits<'_>) -> Option<i32> {
        let mut code = 0u32;
        let mut first = 0u32;
        let mut idx = 0usize;
        for &count in &self.counts {
            code |= b.get(1);
            let count = u32::from(count);
            if code.wrapping_sub(first) < count {
                return self.syms.get(idx + (code - first) as usize).copied();
            }
            idx += count as usize;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_codes() {
        // lengths 1, 2, 3, 3 -> 0, 10, 110, 111
        let h = Huff::new(&[1, 1, 2], [7, 8, 9, 10].into_iter());
        let data = [0b0101_1011, 0b1000_0000];
        let mut b = Bits::new(&data, 16);
        assert_eq!(h.read(&mut b), Some(7));
        assert_eq!(h.read(&mut b), Some(8));
        assert_eq!(h.read(&mut b), Some(9));
        assert_eq!(h.read(&mut b), Some(10));
        assert_eq!(b.count(), 9);
    }

    #[test]
    fn overread_stops_past_the_end() {
        let mut b = Bits::new(&[0xFF], 8);
        assert_eq!(b.get(8), 0xFF);
        assert_eq!(b.get(16), 0);
        assert_eq!(b.count(), 16);
        assert_eq!(b.left(), -8);
    }
}
