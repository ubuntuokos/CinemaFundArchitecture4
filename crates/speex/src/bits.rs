//! The Speex bit reader (`libspeex/bits.c`): bits are read MSB first; a read past the end sets
//! the overflow flag, after which every read returns 0.

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    nb_bits: i64,
    pos: i64,
    overflow: bool,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Bits { data, nb_bits: (data.len() as i64).saturating_mul(8), pos: 0, overflow: false }
    }

    fn bit(&self, at: i64) -> u32 {
        let byte = usize::try_from(at >> 3).ok().and_then(|i| self.data.get(i)).copied().unwrap_or(0);
        u32::from((byte >> (7 - (at & 7))) & 1)
    }

    /// `speex_bits_unpack_unsigned`.
    pub(crate) fn unpack(&mut self, n: u32) -> u32 {
        if self.pos + i64::from(n) > self.nb_bits {
            self.overflow = true;
        }
        if self.overflow {
            return 0;
        }
        let mut d = 0u32;
        for _ in 0..n {
            d = (d << 1) | self.bit(self.pos);
            self.pos += 1;
        }
        d
    }

    /// `speex_bits_peek` (one bit, not consumed).
    pub(crate) fn peek1(&mut self) -> u32 {
        if self.pos + 1 > self.nb_bits {
            self.overflow = true;
        }
        if self.overflow {
            return 0;
        }
        self.bit(self.pos)
    }

    /// `speex_bits_advance` (may go backwards, as the reference does for a zero skip).
    pub(crate) fn advance(&mut self, n: i64) {
        let to = self.pos + n;
        if to > self.nb_bits || self.overflow || to < 0 {
            self.overflow = true;
            return;
        }
        self.pos = to;
    }

    /// `speex_bits_remaining`: -1 after an overflow.
    pub(crate) fn remaining(&self) -> i64 {
        if self.overflow { -1 } else { self.nb_bits - self.pos }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_msb_first_and_flags_overflow() {
        let mut b = Bits::new(&[0b1010_0000, 0xFF]);
        assert_eq!(b.unpack(3), 0b101);
        assert_eq!(b.peek1(), 0);
        assert_eq!(b.unpack(5), 0);
        assert_eq!(b.remaining(), 8);
        assert_eq!(b.unpack(9), 0);
        assert_eq!(b.remaining(), -1);
        assert_eq!(b.unpack(1), 0);
    }
}
