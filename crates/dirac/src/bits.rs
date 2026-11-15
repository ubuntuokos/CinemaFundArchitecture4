// Ported from the SMPTE ST 2042-1 pseudocode of the BBC VC-2 conformance software
// (vc2_conformance 2.0.0, Copyright British Broadcasting Corporation, GPL-3.0-only); see
// docs/licensing.md §5.

//! The bit reader of ST 2042-1 Annex A: MSB-first bits, exp-Golomb (interleaved) unsigned and
//! signed integers, and the bounded reads of slice data (A.4.2), which yield 1 bits once the
//! block is used up.

use crate::{Error, Result};

/// The largest exp-Golomb value accepted; anything above is no valid VC-2 stream.
const MAX_UINT: u64 = 1 << 40;

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    /// Bit position from the start of `data`.
    pos: usize,
    /// Bits left in the current bounded block.
    pub bits_left: u64,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bits_left: 0 }
    }

    /// Bytes consumed, counting a partly read byte.
    pub fn bytes_used(&self) -> usize {
        self.pos.div_ceil(8)
    }

    /// Bits not yet read.
    pub fn remaining(&self) -> u64 {
        (self.data.len() as u64).saturating_mul(8).saturating_sub(self.pos as u64)
    }

    pub fn read_bit(&mut self) -> Result<u32> {
        let byte = *self.data.get(self.pos >> 3).ok_or(Error::Truncated)?;
        let bit = (byte >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        Ok(u32::from(bit))
    }

    pub fn byte_align(&mut self) {
        self.pos = self.pos.div_ceil(8).saturating_mul(8);
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_bit()? == 1)
    }

    /// `read_nbits` for `n` ≤ 64.
    pub fn read_nbits(&mut self, n: u32) -> Result<u64> {
        if n > 64 {
            return Err(Error::Invalid("field wider than 64 bits"));
        }
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.read_bit()?);
        }
        Ok(v)
    }

    pub fn read_byte(&mut self) -> Result<u64> {
        self.read_nbits(8)
    }

    /// Skip `n` whole bytes (the reader is byte aligned).
    pub fn skip_bytes(&mut self, n: u64) -> Result<()> {
        let bits = n.checked_mul(8).ok_or(Error::Truncated)?;
        if bits > self.remaining() {
            return Err(Error::Truncated);
        }
        self.pos += bits as usize;
        Ok(())
    }

    pub fn read_uint(&mut self) -> Result<u64> {
        let mut value = 1u64;
        while self.read_bit()? == 0 {
            if value >= MAX_UINT {
                return Err(Error::Invalid("integer out of range"));
            }
            value <<= 1;
            if self.read_bit()? == 1 {
                value += 1;
            }
        }
        Ok(value - 1)
    }

    /// A signed exp-Golomb integer: the magnitude, then a sign bit (1: negative) when non-zero.
    pub fn read_sint(&mut self) -> Result<i64> {
        let v = self.read_uint()? as i64;
        if v != 0 && self.read_bit()? == 1 { Ok(-v) } else { Ok(v) }
    }

    /// A length-prefixed block of a Dirac core syntax picture: the length, byte alignment, then
    /// that many bytes (cut to what is left of the data, as a damaged stream may claim more).
    pub fn block(&mut self) -> Result<&'a [u8]> {
        let len = self.read_uint()?;
        self.byte_align();
        Ok(self.take(len))
    }

    /// Up to `n` bytes from a byte-aligned position (fewer when the data ends first).
    pub fn take(&mut self, n: u64) -> &'a [u8] {
        let start = (self.pos / 8).min(self.data.len());
        let n = usize::try_from(n).unwrap_or(usize::MAX).min(self.data.len() - start);
        self.pos = (start + n) * 8;
        self.data.get(start..start + n).unwrap_or(&[])
    }

    #[inline]
    pub fn read_bitb(&mut self) -> Result<u32> {
        if self.bits_left == 0 {
            return Ok(1);
        }
        self.bits_left -= 1;
        self.read_bit()
    }

    #[inline]
    pub fn read_sintb(&mut self) -> Result<i64> {
        let mut value = 1u64;
        while self.read_bitb()? == 0 {
            if value >= MAX_UINT {
                return Err(Error::Invalid("coefficient out of range"));
            }
            value <<= 1;
            if self.read_bitb()? == 1 {
                value += 1;
            }
        }
        let v = (value - 1) as i64;
        if v != 0 && self.read_bitb()? == 1 { Ok(-v) } else { Ok(v) }
    }

    /// Skip what is left of the bounded block.
    pub fn flush_inputb(&mut self) -> Result<()> {
        if self.bits_left > self.remaining() {
            return Err(Error::Truncated);
        }
        self.pos += self.bits_left as usize;
        self.bits_left = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb() {
        // 1 → 0; 0 0 1 → 1; 0 1 1 → 2; 0 0 0 0 1 → 3
        let mut b = Bits::new(&[0b1001_0110, 0b0001_0000]);
        assert_eq!(b.read_uint().unwrap(), 0);
        assert_eq!(b.read_uint().unwrap(), 1);
        assert_eq!(b.read_uint().unwrap(), 2);
        assert_eq!(b.read_uint().unwrap(), 3);
        assert!(matches!(Bits::new(&[0; 2]).read_uint(), Err(Error::Truncated)));
    }

    #[test]
    fn bounded_reads_yield_ones() {
        let mut b = Bits::new(&[0b0110_0000]);
        b.bits_left = 3;
        // 0 1 1 = 2 then the sign bit reads past the block: 1 → negative
        assert_eq!(b.read_sintb().unwrap(), -2);
        assert_eq!(b.read_sintb().unwrap(), 0);
        assert_eq!(b.bits_left, 0);
    }
}
