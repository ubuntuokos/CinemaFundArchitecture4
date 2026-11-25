//! An MSB-first bit reader with FFmpeg's semantics: reads past the end give zero bits, and
//! [`Bits::left`] says how many real bits remain (the checks FFmpeg's loops make).

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Real bits not yet read (never negative).
    pub(crate) fn left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    fn bit(&self, p: usize) -> u32 {
        self.data.get(p / 8).map_or(0, |b| u32::from(b >> (7 - (p % 8)) & 1))
    }

    /// `n` (at most 32) bits.
    pub(crate) fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n as usize {
            v = (v << 1) | self.bit(self.pos + i);
        }
        self.pos += n as usize;
        v
    }

    pub(crate) fn get1(&mut self) -> bool {
        self.get(1) != 0
    }

    /// The next `n` bits (at most 32) without consuming them.
    pub(crate) fn peek(&self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n as usize {
            v = (v << 1) | self.bit(self.pos + i);
        }
        v
    }

    pub(crate) fn skip(&mut self, n: usize) {
        self.pos += n;
    }
}
