// SPDX-License-Identifier: BSD-3-Clause-Clear
// Copyright (c) V-Nova International Limited 2014-2026. All rights reserved.
// Ported to Rust for CFA4 from LCEVCdec 4.3.1 (https://github.com/v-novaltd/LCEVCdec, commit
// e295d843). As LCEVCdec requires, this file stays under the BSD-3-Clause-Clear licence and the
// additional licensing information in `crates/lcevc/LICENSE.LCEVCdec`: NO PATENT LICENCES ARE
// GRANTED by it.
//! Byte and bit readers of the LCEVC syntax (LCEVCdec `bytestream.c`, `bitstream.c` and the
//! `HuffmanStream` of `huffman.h`).

use crate::Error;

/// Big-endian byte reader over one unencapsulated LCEVC NAL unit.
pub(crate) struct Bytes<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Error> {
        let v = *self.data.get(self.pos).ok_or(Error::Invalid("truncated block"))?;
        self.pos += 1;
        Ok(v)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Error> {
        let hi = self.u8()?;
        let lo = self.u8()?;
        Ok(u16::from(hi) << 8 | u16::from(lo))
    }

    /// `bytestreamReadMultiByte`: 7 bits per byte, most significant group first, at most 10
    /// bytes.
    pub(crate) fn multibyte(&mut self) -> Result<u64, Error> {
        let mut v: u64 = 0;
        for _ in 0..10 {
            let b = self.u8()?;
            v = (v << 7) | u64::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(Error::Invalid("multibyte value too long"))
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Invalid("size overflow"))?;
        let s = self.data.get(self.pos..end).ok_or(Error::Invalid("truncated block"))?;
        self.pos = end;
        Ok(s)
    }
}

/// MSB-first bit reader. `zero_fill` makes bits past the end read as zeros (the Huffman decode
/// path of LCEVCdec); otherwise reading past the end is an error (table headers, chunk flags).
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    pub(crate) fn consumed_bytes(&self) -> usize {
        self.bit.div_ceil(8)
    }

    fn peek_bit(&self) -> Option<u32> {
        let byte = *self.data.get(self.bit / 8)?;
        Some(u32::from(byte >> (7 - (self.bit % 8))) & 1)
    }

    /// One bit, zero past the end of the data.
    pub(crate) fn bit_or_zero(&mut self) -> u32 {
        let b = self.peek_bit().unwrap_or(0);
        self.bit = self.bit.saturating_add(1);
        b
    }

    pub(crate) fn read(&mut self, n: u32) -> Result<u32, Error> {
        let mut v = 0u32;
        for _ in 0..n {
            let b = self.peek_bit().ok_or(Error::Invalid("truncated bitstream"))?;
            v = (v << 1) | b;
            self.bit += 1;
        }
        Ok(v)
    }
}

/// Removes the NAL framing (`unencapsulate` in LCEVCdec `config_parser.c`): start code, the two
/// header bytes (nal_unit_type 28 non-IDR or 29 IDR), the trailing `0x80` byte and the emulation
/// prevention bytes. Returns the payload and whether the unit is an IDR.
pub(crate) fn unencapsulate(nal: &[u8]) -> Result<(Vec<u8>, bool), Error> {
    let bad = Error::Invalid("not an LCEVC NAL unit");
    if nal.last() != Some(&0x80) {
        return Err(bad);
    }
    let start = match nal {
        [0, 0, 1, ..] => 3,
        [0, 0, 0, 1, ..] => 4,
        _ => return Err(bad),
    };
    let (b0, b1) = match (nal.get(start), nal.get(start + 1)) {
        (Some(&a), Some(&b)) => (a, b),
        _ => return Err(bad),
    };
    if b0 & 0xC1 != 0x41 || b1 != 0xFF {
        return Err(bad);
    }
    let idr = match (b0 & 0x3E) >> 1 {
        28 => false,
        29 => true,
        _ => return Err(bad),
    };
    let body = nal.get(start + 2..nal.len() - 1).unwrap_or(&[]);
    let mut out = Vec::with_capacity(body.len());
    let mut zeros = 0u32;
    for &b in body {
        if zeros == 2 && b == 3 {
            zeros = 0;
            continue;
        }
        if b == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
        out.push(b);
    }
    Ok((out, idr))
}
