//! MSB-first bit reader with the behaviour of FFmpeg's checked `GetBitContext`: reads past the
//! end give zero bits, and the exp-Golomb readers accept the code lengths `golomb.h` accepts.

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
    /// Size in bits.
    len: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, len: data.len().saturating_mul(8) }
    }

    /// The next 64 bits (zeros past the end).
    fn peek64(&self) -> u64 {
        let byte = self.pos / 8;
        let mut v = 0u128;
        for k in 0..9 {
            v = (v << 8) | u128::from(self.data.get(byte + k).copied().unwrap_or(0));
        }
        (v >> (8 - self.pos % 8)) as u64
    }

    pub fn peek(&self, n: u32) -> u32 {
        if n == 0 { 0 } else { (self.peek64() >> (64 - n.min(32))) as u32 }
    }

    pub fn skip(&mut self, n: usize) {
        // FFmpeg's checked reader stops 8 bits past the end
        self.pos = self.pos.saturating_add(n).min(self.len.saturating_add(8));
    }

    pub fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n as usize);
        v
    }

    pub fn bit(&mut self) -> bool {
        self.get(1) != 0
    }

    pub fn count(&self) -> usize {
        self.pos
    }

    pub fn left(&self) -> isize {
        self.len as isize - self.pos as isize
    }

    /// `get_ue_golomb`: codes of up to 25 bits (values up to 8190); longer ones are invalid.
    pub fn ue(&mut self) -> Option<u32> {
        let buf = self.peek(32);
        let lz = buf.leading_zeros();
        if lz > 12 {
            self.skip((2 * lz as usize + 1).min(32));
            return None;
        }
        let n = 2 * lz + 1;
        let v = (buf >> (32 - n)) - 1;
        self.skip(n as usize);
        Some(v)
    }

    /// `get_ue_golomb_31`: a code of up to 9 bits (longer codes are reported as invalid).
    pub fn ue31(&mut self) -> Option<u32> {
        if self.peek(32).leading_zeros() > 4 {
            self.skip(9);
            return None;
        }
        self.ue()
    }

    /// `get_se_golomb`.
    pub fn se(&mut self) -> Option<i32> {
        let buf = self.peek64();
        let lz = buf.leading_zeros();
        if lz >= 32 {
            self.skip(32);
            return None;
        }
        let n = 2 * lz + 1;
        let k = buf >> (64 - n); // codeNum + 1
        self.skip(n as usize);
        let v = if k & 1 == 1 { -((k >> 1) as i64) } else { (k >> 1) as i64 };
        Some(v as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golomb_codes() {
        // 1 | 010 | 011 | 00100 | 0001000
        let data = [0b1010_0110, 0b0100_0001, 0b0000_0000];
        let mut b = Bits::new(&data);
        assert_eq!(b.ue(), Some(0));
        assert_eq!(b.ue(), Some(1));
        assert_eq!(b.se(), Some(-1));
        assert_eq!(b.ue(), Some(3));
        assert_eq!(b.se(), Some(4));
        assert_eq!(b.count(), 19);
    }

    #[test]
    fn past_the_end_reads_zeros() {
        let mut b = Bits::new(&[0xff]);
        assert_eq!(b.get(4), 0xf);
        assert_eq!(b.get(8), 0xf0);
        assert!(b.left() < 0);
        assert_eq!(b.ue(), None);
        assert_eq!(b.se(), None);
    }
}
