// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! Start codes, the removal of the two stuffing bits after `00 00 02`, and the fixed / Exp-Golomb
//! bit reader of the headers.

use crate::Error;

/// `(code, offset of its 00 00 01)` for every start code in `data`.
pub(crate) fn start_codes(data: &[u8]) -> impl Iterator<Item = (u8, usize)> + '_ {
    data.windows(4).enumerate().filter_map(|(i, w)| (w[0] == 0 && w[1] == 0 && w[2] == 1).then_some((w[3], i)))
}

/// `dec_bs_demulate`: removes the two stuffing bits that follow every `00 00 02`. `src` starts at
/// the byte after the `00 00 01` prefix. The result is as long as `src` (uavs3d keeps the length and
/// leaves garbage after the data); the tail is zero here.
pub(crate) fn demulate(src: &[u8]) -> Vec<u8> {
    let len = src.len();
    let first = (0..len.saturating_sub(3)).find(|&i| src[i] == 0 && src.get(i + 1) == Some(&0) && src.get(i + 2) == Some(&2));
    let Some(first) = first else {
        return src.to_vec();
    };
    let at = |i: usize| src.get(i).copied().unwrap_or(0);
    let mut out = src.get(..first).unwrap_or_default().to_vec();
    let mut s = first;
    let mut prev: u32 = 0;
    let mut bit_pos: u32 = 0;
    for _ in 0..len - first {
        let val = at(s);
        s += 1;
        prev = ((prev << 8) | u32::from(val)) & 0xff_ffff;
        if prev != 2 {
            out.push(((u32::from(val) << bit_pos) | (u32::from(at(s)) >> (8 - bit_pos))) as u8);
        } else {
            bit_pos += 2;
            if bit_pos == 8 {
                let v = at(s);
                s += 1;
                out.push(v);
                prev = ((prev << 8) | u32::from(v)) & 0xff_ffff;
                bit_pos = 0;
            } else {
                out.push((u32::from(at(s)) >> (8 - bit_pos)) as u8);
            }
        }
    }
    out
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

    /// `u(n)` that must lie in `min..=max`.
    pub(crate) fn ur(&mut self, n: u32, min: u32, max: u32) -> Result<u32, Error> {
        let v = self.u(n)?;
        if v < min || v > max {
            return Err(Error::Invalid("syntax element out of range"));
        }
        Ok(v)
    }

    pub(crate) fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.bit()? != 0)
    }

    /// A flag that must be `want` (a marker bit).
    pub(crate) fn marker(&mut self) -> Result<(), Error> {
        if self.bit()? != 1 {
            return Err(Error::Invalid("marker bit"));
        }
        Ok(())
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

    /// `ue(v)` that must lie in `min..=max`.
    pub(crate) fn uer(&mut self, min: u32, max: u32) -> Result<u32, Error> {
        let v = self.ue()?;
        if v < min || v > max {
            return Err(Error::Invalid("syntax element out of range"));
        }
        Ok(v)
    }

    /// `se(v)` that must lie in `min..=max`.
    pub(crate) fn ser(&mut self, min: i32, max: i32) -> Result<i32, Error> {
        let n = i64::from(self.ue()?);
        let v = (n + 1) >> 1;
        let v = (if n & 1 == 0 { -v } else { v }) as i32;
        if v < min || v > max {
            return Err(Error::Invalid("syntax element out of range"));
        }
        Ok(v)
    }
}
