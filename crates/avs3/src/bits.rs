// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/bitstream.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Pseudo start code removal and the fixed / Exp-Golomb bit reader of the headers.

use crate::Error;

/// `dec_bs_demulate`: removes the two stuffing bits that follow every `00 00 02` of a unit. The
/// result keeps the bytes before the first such pattern as they are.
pub(crate) fn demulate(src: &[u8]) -> Vec<u8> {
    let Some(first) = src.windows(3).position(|w| w == [0, 0, 2]) else {
        return src.to_vec();
    };
    let mut out = Vec::with_capacity(src.len());
    out.extend_from_slice(&src[..first]);
    // `bits` collects output bits MSB first; `pending` is the partly filled output byte
    let mut prev: u32 = 0;
    let mut acc: u32 = 0;
    let mut nacc: u32 = 0;
    let mut push_bits = |out: &mut Vec<u8>, v: u32, n: u32| {
        acc = (acc << n) | (v & ((1 << n) - 1));
        nacc += n;
        while nacc >= 8 {
            out.push((acc >> (nacc - 8)) as u8);
            nacc -= 8;
            acc &= (1 << nacc) - 1;
        }
    };
    for &val in &src[first..] {
        prev = ((prev << 8) | u32::from(val)) & 0x00ff_ffff;
        if prev == 2 {
            push_bits(&mut out, u32::from(val) >> 2, 6);
        } else {
            push_bits(&mut out, u32::from(val), 8);
        }
    }
    if nacc > 0 {
        out.push((acc << (8 - nacc)) as u8);
    }
    out
}

/// A bit reader over a de-emulated unit.
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }

    pub(crate) fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    pub(crate) fn remaining_bits(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    pub(crate) fn read(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if self.remaining_bits() < n as usize {
            self.pos = self.data.len() * 8;
            return Err(Error::Truncated);
        }
        let mut v: u64 = 0;
        for _ in 0..n {
            let byte = self.data[self.pos >> 3];
            v = (v << 1) | u64::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        Ok(v as u32)
    }

    pub(crate) fn read1(&mut self) -> Result<u32, Error> {
        self.read(1)
    }

    pub(crate) fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.read(1)? != 0)
    }

    /// A value with a range check (`dec_bs_read(bs, n, min, max)`).
    pub(crate) fn read_range(&mut self, n: u32, min: u32, max: u32) -> Result<u32, Error> {
        let v = self.read(n)?;
        if v < min || v > max {
            return Err(Error::Invalid("syntax element out of range"));
        }
        Ok(v)
    }

    pub(crate) fn ue(&mut self) -> Result<u32, Error> {
        let mut zeros = 0u32;
        while self.read1()? == 0 {
            zeros += 1;
            if zeros > 32 {
                return Err(Error::Invalid("Exp-Golomb code too long"));
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        let rest = u64::from(self.read(zeros)?);
        Ok((((1u64 << zeros) | rest) - 1) as u32)
    }

    pub(crate) fn ue_range(&mut self, min: u32, max: u32) -> Result<u32, Error> {
        let v = self.ue()?;
        if v < min || v > max {
            return Err(Error::Invalid("syntax element out of range"));
        }
        Ok(v)
    }

    pub(crate) fn se(&mut self) -> Result<i32, Error> {
        let v = self.ue()?;
        Ok(if v & 1 != 0 { v.div_ceil(2) as i32 } else { -((v >> 1) as i32) })
    }

    pub(crate) fn se_range(&mut self, min: i32, max: i32) -> Result<i32, Error> {
        let v = self.se()?;
        if v < min || v > max {
            return Err(Error::Invalid("syntax element out of range"));
        }
        Ok(v)
    }

    /// The next `n` (at most 32) bits without consuming them; missing bits read as 0.
    pub(crate) fn peek(&self, n: u32) -> u32 {
        let mut v: u64 = 0;
        for i in 0..n as usize {
            let p = self.pos + i;
            let bit = self.data.get(p >> 3).map_or(0, |b| (b >> (7 - (p & 7))) & 1);
            v = (v << 1) | u64::from(bit);
        }
        v as u32
    }

    pub(crate) fn skip_bytes(&mut self, n: usize) {
        self.pos = (self.pos + n * 8).min(self.data.len() * 8);
    }

    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.data.len() * 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demulate_drops_two_bits_after_00_00_02() {
        // 00 00 02 FF -> 00 00 | 000000 + FF top 2 bits (11) = 00 00 03 | FF<<2 = FC
        assert_eq!(demulate(&[0, 0, 2, 0xFF]), vec![0, 0, 0x03, 0xFC]);
        assert_eq!(demulate(&[1, 2, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn exp_golomb() {
        let mut b = Bits::new(&[0b1010_0110, 0b0100_0000]);
        assert_eq!(b.ue().unwrap(), 0);
        assert_eq!(b.ue().unwrap(), 1);
        assert_eq!(b.ue().unwrap(), 2);
        assert_eq!(b.se().unwrap(), 2);
    }
}
