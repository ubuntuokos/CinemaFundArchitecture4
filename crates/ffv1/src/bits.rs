//! MSB-first bit reader for Golomb Rice slices (RFC 9043 section 3.8.2); reads past the end as 0.

pub(crate) struct BitReader<'a> {
    buf: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> BitReader<'a> {
        BitReader { buf, bit: 0 }
    }

    #[inline]
    pub fn bit(&mut self) -> u32 {
        let b = self.buf.get(self.bit >> 3).map_or(0, |&b| (b >> (7 - (self.bit & 7))) & 1);
        self.bit += 1;
        u32::from(b)
    }

    /// `get_bits(n)` for n <= 32.
    pub fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n.min(32) {
            v = (v << 1) | self.bit();
        }
        v
    }

    /// Whether more than 64 bits past the end were read (a corrupt stream).
    pub fn overrun(&self) -> bool {
        self.bit > self.buf.len() * 8 + 64
    }
}
