// Copyright Rainbaby, from the `truehd` crate 0.7.2 (Apache-2.0, crates/truehd/LICENSE).
// Modified for CFA4: see crates/truehd/README.md for the list of changes.
//! Bitstream I/O utilities for audio parsing.
//!
//! Provides bitstream reading, Huffman decoding, CRC validation,
//! and specialized bit manipulation functions for format parsing.
//!
//! CFA4: rewritten over a plain byte slice instead of the `bitstream-io` crate, with the same
//! interface. Every read is bounds-checked and returns `UnexpectedEof` past the end.

use std::io;

use crate::utils::crc::{Crc8, Crc16, crc8, crc16};

/// Value every huffman table maps its deepest, nine-bit code to.
pub const HUFFMAN_DEEPEST_CODE: i32 = -7;

/// Unsigned integer types [`BitstreamIoReader::get_n`] reads into.
pub trait UnsignedInteger: Sized {
    const BITS: u32;
    fn from_u64(v: u64) -> Self;
}

/// Signed integer types [`BitstreamIoReader::get_s`] reads into.
pub trait SignedInteger: Sized {
    const BITS: u32;
    fn from_i64(v: i64) -> Self;
}

macro_rules! unsigned {
    ($($t:ty),*) => {$(
        impl UnsignedInteger for $t {
            const BITS: u32 = <$t>::BITS;
            #[inline(always)]
            fn from_u64(v: u64) -> Self {
                v as $t
            }
        }
    )*};
}

macro_rules! signed {
    ($($t:ty),*) => {$(
        impl SignedInteger for $t {
            const BITS: u32 = <$t>::BITS;
            #[inline(always)]
            fn from_i64(v: i64) -> Self {
                v as $t
            }
        }
    )*};
}

unsigned!(u8, u16, u32, u64, usize);
signed!(i8, i16, i32, i64);

fn eof(what: &str, n: u64, at: u64) -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, format!("{what}({n}): out of bounds bits at {at}"))
}

/// A big-endian bit reader over a byte slice.
#[derive(Debug, Clone)]
pub struct BitstreamIoReader<'a> {
    buf: &'a [u8],
    /// Position in bits.
    pos: u64,
    /// Length in bits.
    len: u64,
}

pub type BsIoSliceReader<'a> = BitstreamIoReader<'a>;

impl<'a> BitstreamIoReader<'a> {
    pub fn from_slice(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0, len: (buf.len() as u64).saturating_mul(8) }
    }

    /// Reads `n` (at most 64) bits without bounds checks against `len`; the caller checked.
    #[inline(always)]
    fn bits(&mut self, n: u32) -> u64 {
        let mut v = 0u64;
        let mut left = n;
        while left > 0 {
            let byte = self.buf.get((self.pos >> 3) as usize).copied().unwrap_or(0);
            let off = (self.pos & 7) as u32;
            let take = (8 - off).min(left);
            let chunk = (byte >> (8 - off - take)) & ((1u16 << take) - 1) as u8;
            v = (v << take) | u64::from(chunk);
            left -= take;
            self.pos += u64::from(take);
        }
        v
    }

    #[inline(always)]
    pub fn get(&mut self) -> io::Result<bool> {
        if self.pos >= self.len {
            return Err(eof("get", 1, self.pos));
        }
        Ok(self.bits(1) == 1)
    }

    #[inline(always)]
    pub fn get_n<I: UnsignedInteger>(&mut self, n: u32) -> io::Result<I> {
        if n > I::BITS {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "get_n: too many bits for type"));
        }
        if u64::from(n) > self.len - self.pos {
            return Err(eof("get_n", n.into(), self.pos));
        }
        Ok(I::from_u64(self.bits(n)))
    }

    #[inline(always)]
    pub fn get_s<S: SignedInteger>(&mut self, n: u32) -> io::Result<S> {
        if n == 0 || n > S::BITS {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "get_s: invalid bit count for type"));
        }
        if u64::from(n) > self.len - self.pos {
            return Err(eof("get_s", n.into(), self.pos));
        }
        let v = self.bits(n);
        let shift = 64 - n;
        Ok(S::from_i64(((v << shift) as i64) >> shift))
    }

    /// The TrueHD residual huffman codes: a `1` prefix gives the short codes (0..=3, 0..=1 or
    /// 0), a `0` prefix a sign bit and a unary run of up to six zeros, the deepest code taking
    /// one more (ignored) bit.
    #[inline(always)]
    pub fn get_huffman(&mut self, huff_type: usize) -> io::Result<i32> {
        let base = match huff_type {
            1 => 4,
            2 => 2,
            3 => 1,
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "get_huffman: unsupported huff_type")),
        };
        if self.get()? {
            return Ok(match huff_type {
                1 => (i32::from(self.get()?) << 1) | i32::from(self.get()?),
                2 => i32::from(self.get()?),
                _ => 0,
            });
        }
        let positive = self.get()?;
        for k in 0..6 {
            if self.get()? {
                return Ok(if positive { base + k } else { -1 - k });
            }
        }
        self.get()?;
        Ok(if positive { base + 6 } else { HUFFMAN_DEEPEST_CODE })
    }

    #[inline(always)]
    pub fn get_variable_bits_max(&mut self, n: u32, max_num_groups: u32) -> io::Result<u32> {
        let mut value: u32 = 0;
        let mut num_group = 0;
        let mut b_read_more = true;

        while b_read_more && num_group < max_num_groups {
            let read = self.get_n::<u32>(n)?;
            value = value.wrapping_add(read);
            b_read_more = self.get()?;
            if b_read_more {
                value = value.wrapping_add(1).checked_shl(n).unwrap_or(0);
                num_group += 1;
            }
        }

        Ok(value)
    }

    #[inline(always)]
    pub fn seek(&mut self, offset: i64) -> io::Result<u64> {
        let target = i128::from(self.pos) + i128::from(offset);
        if target < 0 || target > i128::from(self.len) {
            return Err(eof("seek", offset.unsigned_abs(), self.pos));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }

    /// The `len >> 3` whole bytes starting at bit `start`, which need not be byte aligned.
    fn byte_run(&self, start: u64, len: u64) -> io::Result<std::borrow::Cow<'a, [u8]>> {
        let bytes = len >> 3;
        if start.checked_add(bytes * 8).is_none_or(|end| end > self.len) {
            return Err(eof("bytes", len, start));
        }
        let from = (start >> 3) as usize;
        let to = from + bytes as usize;
        if start & 7 == 0 {
            return self.buf.get(from..to).map(std::borrow::Cow::Borrowed).ok_or_else(|| eof("bytes", len, start));
        }
        let mut r = self.clone();
        r.pos = start;
        Ok(std::borrow::Cow::Owned((0..bytes).map(|_| r.bits(8) as u8).collect()))
    }

    #[inline(always)]
    // TODO: byte boundary
    pub fn parity_check_for_last_n_bits(&mut self, len: u64) -> io::Result<u8> {
        let start = self.pos.checked_sub(len).ok_or_else(|| eof("parity", len, self.pos))?;
        Ok(self.byte_run(start, len)?.iter().fold(0, |acc, x| acc ^ x))
    }

    #[inline(always)]
    pub fn parity_check_nibble_for_last_n_bits(&mut self, len: u64) -> io::Result<u8> {
        let mut parity = self.parity_check_for_last_n_bits(len)?;

        parity ^= parity >> 4;
        parity &= 0xF;

        Ok(parity)
    }

    #[inline(always)]
    pub fn crc8_check(&mut self, crc: &Crc8, start: u64, len: u64) -> io::Result<u8> {
        if start.checked_add(len).is_none_or(|end| end > self.len) {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "crc8_check: out of bounds bits"));
        }
        let position = self.pos;
        self.pos = start;

        let mut checksum = crc.init;

        let prefix_len = (start & 7).min(len);
        let suffix_len = (len - prefix_len) & 7;
        let middle_len = len - prefix_len - suffix_len;

        if prefix_len != 0 {
            let prefix = self.bits(prefix_len as u32) as u8;
            checksum = crc8(crc.poly, checksum, prefix_len as usize) ^ prefix;
        }

        let middle = self.byte_run(self.pos, middle_len)?;
        checksum = crc.update(checksum, &middle);
        self.pos += middle_len;

        if suffix_len != 0 {
            let suffix = self.bits(suffix_len as u32) as u8;
            checksum = crc8(crc.poly, checksum, suffix_len as usize) ^ suffix;
        }

        self.pos = position;

        Ok(checksum)
    }

    #[inline(always)]
    pub fn crc16_check(&mut self, crc: &Crc16, start: u64, len: u64) -> io::Result<u16> {
        if start.checked_add(len).is_none_or(|end| end > self.len) {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "crc16_check: out of bounds bits"));
        }
        let position = self.pos;
        self.pos = start;

        let mut checksum = crc.init;

        let prefix_len = (start & 7).min(len);
        let suffix_len = (len - prefix_len) & 7;
        let middle_len = len - prefix_len - suffix_len;

        if prefix_len != 0 {
            let prefix = self.bits(prefix_len as u32) as u16;
            checksum = crc16(crc.poly, checksum, prefix_len as usize) ^ prefix;
        }

        let middle = self.byte_run(self.pos, middle_len)?;
        checksum = crc.update(checksum, &middle);
        self.pos += middle_len;

        if suffix_len != 0 {
            let suffix = self.bits(suffix_len as u32) as u16;
            checksum = crc16(crc.poly, checksum, suffix_len as usize) ^ suffix;
        }

        self.pos = position;

        Ok(checksum)
    }

    #[inline(always)]
    pub fn align_16bit(&mut self) -> io::Result<()> {
        let aligned = (self.pos + 15) & !15;
        if aligned > self.len {
            return Err(eof("align_16bit", aligned - self.pos, self.pos));
        }
        self.pos = aligned;
        Ok(())
    }

    #[inline(always)]
    pub fn available(&mut self) -> io::Result<u64> {
        Ok(self.len - self.pos)
    }

    #[inline(always)]
    pub fn skip_n(&mut self, n: u32) -> io::Result<()> {
        if u64::from(n) > self.len - self.pos {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "skip_n: out of bounds bits"));
        }
        self.pos += u64::from(n);
        Ok(())
    }

    #[inline(always)]
    pub fn position(&mut self) -> io::Result<u64> {
        Ok(self.pos)
    }
}

impl Default for BsIoSliceReader<'_> {
    fn default() -> Self {
        Self::from_slice(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bits_signed_values_and_huffman_codes() {
        let mut r = BsIoSliceReader::from_slice(&[0b1011_0011, 0b1111_0000]);
        assert_eq!(r.get_n::<u8>(3).ok(), Some(0b101));
        assert_eq!(r.get_s::<i8>(4).ok(), Some(-7)); // 1001
        assert_eq!(r.get_n::<u16>(9).ok(), Some(0b1_1111_0000));
        assert!(r.get().is_err());

        // tree 1: "11 0" → 2 then "0 1 1" → 4 then "0 0 1" → -1 then "00 000000 1" → -7
        let mut r = BsIoSliceReader::from_slice(&[0b1100_1100, 0b1000_0000, 0b0100_0000]);
        assert_eq!(r.get_huffman(1).ok(), Some(2));
        assert_eq!(r.get_huffman(1).ok(), Some(4));
        assert_eq!(r.get_huffman(1).ok(), Some(-1));
        assert_eq!(r.get_huffman(1).ok(), Some(-7));
        assert_eq!(r.position().ok(), Some(18));
    }

    #[test]
    fn align_skips_to_the_next_16_bit_boundary() {
        let mut r = BsIoSliceReader::from_slice(&[0; 4]);
        r.skip_n(3).ok();
        assert!(r.align_16bit().is_ok());
        assert_eq!(r.position().ok(), Some(16));
        assert!(r.align_16bit().is_ok());
        assert_eq!(r.position().ok(), Some(16));
    }
}
