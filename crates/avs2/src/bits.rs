// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/bitstream.cc`, `vlc.h`), Copyright (C) 2018~ VCL, NELVT, Peking University
// (Falei Luo et al.), GPL-2.0-or-later.
//! Start codes, the removal of the two stuffing bits after `00 00 02` (pseudo start codes) and
//! the fixed / Exp-Golomb bit reader of the headers.

use crate::Error;

/// Start code values (the byte after `00 00 01`).
pub(crate) const SC_SEQUENCE_HEADER: u8 = 0xB0;
pub(crate) const SC_USER_DATA: u8 = 0xB2;
pub(crate) const SC_INTRA_PICTURE: u8 = 0xB3;
pub(crate) const SC_EXTENSION: u8 = 0xB5;
pub(crate) const SC_INTER_PICTURE: u8 = 0xB6;
/// Slice start codes are `0x00..=SC_SLICE_CODE_MAX`.
pub(crate) const SC_SLICE_CODE_MAX: u8 = 0x8F;

/// `bs_dispose_pseudo_code`: removes the two stuffing bits that follow every `00 00 02` in the
/// data of picture and slice units (not in sequence headers, user data or extensions).
pub(crate) fn dispose_pseudo_code(src: &[u8]) -> Vec<u8> {
    const BITMASK: [u8; 8] = [0x00, 0x00, 0xc0, 0x00, 0xf0, 0x00, 0xfc, 0x00];
    let mask = |i: u32| BITMASK.get(i as usize).copied().unwrap_or(0);
    let mut dst = Vec::with_capacity(src.len());
    let mut found_start_code = false;
    let mut leading_zeros = 0;
    let mut last_bit_count: u32 = 0;
    let mut dispose = false;
    let mut last_byte: u8 = 0;
    for &curr_byte in src {
        let mut curr_bit_count = 8;
        match curr_byte {
            0 => {
                if found_start_code {
                    dispose = true;
                    found_start_code = false;
                }
                leading_zeros += 1;
            }
            1 => {
                if leading_zeros >= 2 {
                    found_start_code = true;
                    if last_bit_count != 0 {
                        last_bit_count = 0;
                        dst.push(0);
                    }
                }
                leading_zeros = 0;
            }
            2 => {
                if dispose && leading_zeros == 2 {
                    curr_bit_count = 6;
                }
                leading_zeros = 0;
            }
            _ => {
                if found_start_code {
                    dispose = !matches!(curr_byte, SC_SEQUENCE_HEADER | SC_USER_DATA | SC_EXTENSION);
                    found_start_code = false;
                }
                leading_zeros = 0;
            }
        }
        if curr_bit_count == 8 {
            if last_bit_count == 0 {
                dst.push(curr_byte);
            } else {
                dst.push((last_byte & mask(last_bit_count)) | ((curr_byte & mask(8 - last_bit_count)) >> last_bit_count));
                last_byte = (curr_byte << (8 - last_bit_count)) & mask(last_bit_count);
            }
        } else if last_bit_count == 0 {
            last_byte = curr_byte;
            last_bit_count = curr_bit_count;
        } else {
            dst.push((last_byte & mask(last_bit_count)) | ((curr_byte & mask(8 - last_bit_count)) >> last_bit_count));
            last_byte = (curr_byte << (8 - last_bit_count)) & mask(last_bit_count - 2);
            last_bit_count -= 2;
        }
    }
    if last_bit_count != 0 && last_byte != 0 {
        dst.push(last_byte);
    }
    dst
}

/// MSB-first reader of fixed-length and Exp-Golomb codes.
#[derive(Clone)]
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn align(&mut self) {
        self.pos = self.pos.div_ceil(8).saturating_mul(8);
    }

    /// Byte offset of the reading position, rounded up.
    pub(crate) fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }

    fn bit(&mut self) -> Result<u32, Error> {
        let b = self.data.get(self.pos >> 3).ok_or(Error::Truncated)?;
        let v = (b >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        Ok(u32::from(v))
    }

    /// `u(n)`, `n` up to 32.
    pub(crate) fn u(&mut self, n: u32) -> Result<u32, Error> {
        let mut v = 0u32;
        for _ in 0..n.min(32) {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }

    pub(crate) fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.bit()? != 0)
    }

    /// `ue(v)`.
    pub(crate) fn ue(&mut self) -> Result<u32, Error> {
        let mut zeros = 0u32;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(Error::Invalid("Exp-Golomb code too long"));
            }
        }
        let info = self.u(zeros)?;
        Ok(((1u64 << zeros) + u64::from(info) - 1).min(u64::from(u32::MAX)) as u32)
    }

    /// `se(v)`.
    pub(crate) fn se(&mut self) -> Result<i32, Error> {
        let n = i64::from(self.ue()?);
        let v = (n + 1) >> 1;
        Ok((if n & 1 == 0 { -v } else { v }) as i32)
    }
}
