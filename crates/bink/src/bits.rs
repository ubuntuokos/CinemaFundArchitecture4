//! A little-endian bit reader (first bit read is the lowest bit of the first byte), like
//! FFmpeg's `BITSTREAM_READER_LE`. Reads past the end return zero bits.

pub struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn count(&self) -> usize {
        self.pos
    }
    pub fn total(&self) -> usize {
        self.data.len() * 8
    }
    pub fn left(&self) -> isize {
        self.total() as isize - self.pos as isize
    }
    /// Peek up to 32 bits.
    pub fn peek(&self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        let byte = self.pos >> 3;
        let mut v = 0u64;
        for i in 0..8 {
            v |= (*self.data.get(byte + i).unwrap_or(&0) as u64) << (8 * i);
        }
        let v = v >> (self.pos & 7);
        (v & ((1u64 << n) - 1)) as u32
    }
    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }
    pub fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }
    pub fn bit(&mut self) -> u32 {
        self.get(1)
    }
    /// Skip to the next 32-bit boundary.
    pub fn align32(&mut self) {
        let r = self.pos & 31;
        if r != 0 {
            self.pos += 32 - r;
        }
    }
}
