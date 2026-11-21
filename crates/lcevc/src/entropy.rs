// SPDX-License-Identifier: BSD-3-Clause-Clear
// Copyright (c) V-Nova International Limited 2014-2026. All rights reserved.
// Ported to Rust for CFA4 from LCEVCdec 4.3.1 (https://github.com/v-novaltd/LCEVCdec, commit
// e295d843). As LCEVCdec requires, this file stays under the BSD-3-Clause-Clear licence and the
// additional licensing information in `crates/lcevc/LICENSE.LCEVCdec`: NO PATENT LICENCES ARE
// GRANTED by it.
//! Entropy decoding of LCEVC chunks (LCEVCdec `huffman.c`, `entropy.c`): prefix-coded or
//! RLE-only residual symbols and temporal signal runs.

use crate::Error;
use crate::bits::Bits;

/// One canonical prefix-code table.
#[derive(Clone, Debug, Default)]
enum Table {
    /// Table with no symbol: decoding from it is an error.
    #[default]
    Empty,
    /// A single symbol, coded with zero bits.
    Single(u8),
    /// `(bits, code, symbol)`, sorted by length.
    Codes(Vec<(u32, u32, u8)>),
}

/// `bitWidth` for bitstream version 2: `ceil(log2(x + 1))`.
fn bit_width(x: u32) -> u32 {
    32 - x.leading_zeros()
}

impl Table {
    /// `huffmanManualInitializeCommon` followed by the canonical code assignment.
    fn parse(bits: &mut Bits<'_>) -> Result<Self, Error> {
        let min = bits.read(5)?;
        let max = bits.read(5)?;
        if max < min {
            return Err(Error::Invalid("prefix code lengths"));
        }
        if min == 31 && max == 31 {
            return Ok(Table::Empty);
        }
        if min == 0 && max == 0 {
            return Ok(Table::Single(u8::try_from(bits.read(8)?).unwrap_or(0)));
        }
        let length_bits = bit_width(max - min);
        let mut entries: Vec<(u32, u8)> = Vec::new();
        let mut counts = [0u32; 32];
        let mut add = |entries: &mut Vec<(u32, u8)>, symbol: u32, len: u32| -> Result<(), Error> {
            let b = len + min;
            if b > max {
                return Err(Error::Invalid("prefix code length"));
            }
            if b == 0 {
                return Ok(());
            }
            if let Some(c) = counts.get_mut(b.min(max) as usize) {
                *c += 1;
            }
            entries.push((b, u8::try_from(symbol).unwrap_or(0)));
            Ok(())
        };
        if bits.read(1)? != 0 {
            for symbol in 0..256 {
                if bits.read(1)? != 0 {
                    let len = bits.read(length_bits)?;
                    add(&mut entries, symbol, len)?;
                }
            }
        } else {
            let count = bits.read(5)?;
            for _ in 0..count {
                let symbol = bits.read(8)?;
                let len = bits.read(length_bits)?;
                add(&mut entries, symbol, len)?;
            }
        }
        // validateCodeLengths (only lengths up to the 10-bit lookup table are checked)
        let mut code: u32 = 0;
        let mut cur = max;
        for b in (1..=max).rev() {
            if b < cur {
                code >>= cur - b;
                cur = b;
            }
            let count = counts.get(b as usize).copied().unwrap_or(0);
            if b <= 10 && count > 0 {
                let limit = 1u32 << b;
                if code >= limit || count > limit - code {
                    return Err(Error::Invalid("prefix code lengths"));
                }
            }
            code = code.saturating_add(count);
        }
        // Ascending length, then descending symbol; codes are given from the last entry.
        entries.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut codes = vec![(0u32, 0u32, 0u8); entries.len()];
        let mut code: u32 = 0;
        let mut cur = max;
        for (slot, &(b, s)) in codes.iter_mut().zip(entries.iter()).rev() {
            if b < cur {
                code >>= cur - b;
                cur = b;
            }
            *slot = (b, code, s);
            code = code.wrapping_add(1);
        }
        Ok(Table::Codes(codes))
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u8, Error> {
        match self {
            Table::Empty => Err(Error::Invalid("empty prefix code table")),
            Table::Single(s) => Ok(*s),
            Table::Codes(codes) => {
                let max = codes.last().map_or(0, |c| c.0);
                let mut code = 0u32;
                let mut idx = 0usize;
                for len in 1..=max {
                    code = (code << 1) | bits.bit_or_zero();
                    while let Some(&(b, c, s)) = codes.get(idx) {
                        if b > len {
                            break;
                        }
                        if b == len && c == code {
                            return Ok(s);
                        }
                        idx += 1;
                    }
                }
                Err(Error::Invalid("unknown prefix code"))
            }
        }
    }
}

/// Source of 8-bit symbols: raw bytes (RLE-only chunks) or prefix codes.
enum Source<'a> {
    /// The chunk is not entropy coded: no data.
    None,
    Raw {
        data: &'a [u8],
        pos: usize,
    },
    Coded {
        bits: Bits<'a>,
        tables: Vec<Table>,
    },
}

impl Source<'_> {
    fn raw(&mut self) -> Result<u8, Error> {
        match self {
            Source::Raw { data, pos } => {
                let v = *data.get(*pos).ok_or(Error::Invalid("RLE chunk exhausted"))?;
                *pos += 1;
                Ok(v)
            }
            _ => Err(Error::Invalid("RLE chunk exhausted")),
        }
    }

    fn coded(&mut self, table: usize) -> Result<u8, Error> {
        match self {
            Source::Coded { bits, tables } => tables.get(table).ok_or(Error::Invalid("prefix code table"))?.decode(bits),
            Source::Raw { .. } => self.raw(),
            Source::None => Err(Error::Invalid("no data")),
        }
    }
}

/// Result of decoding a layer or temporal symbol run when the chunk carries no data.
pub(crate) const NO_DATA: i32 = -2;

/// Residual layer decoder (`EntropyDecoder` of type `EDTDefault`).
pub(crate) struct Residual<'a> {
    src: Source<'a>,
}

const LSB: usize = 0;
const MSB: usize = 1;
const RL: usize = 2;

impl<'a> Residual<'a> {
    /// `enabled`: the chunk's entropy flag; `data`: its bytes.
    pub(crate) fn new(enabled: bool, rle_only: bool, data: &'a [u8]) -> Result<Self, Error> {
        let src = if !enabled {
            Source::None
        } else if rle_only {
            Source::Raw { data, pos: 0 }
        } else {
            let mut bits = Bits::new(data);
            let tables = vec![Table::parse(&mut bits)?, Table::parse(&mut bits)?, Table::parse(&mut bits)?];
            Source::Coded { bits, tables }
        };
        Ok(Self { src })
    }

    /// One coefficient and the run of zero coefficients after it, or `NO_DATA`.
    pub(crate) fn decode(&mut self, coeff: &mut i16) -> Result<i32, Error> {
        if matches!(self.src, Source::None) {
            *coeff = 0;
            return Ok(NO_DATA);
        }
        let lsb = self.src.coded(LSB)?;
        let mut last = lsb;
        let value = if lsb & 1 != 0 {
            let msb = self.src.coded(MSB)?;
            last = msb;
            let exp = (i32::from(msb & 0x7F) << 8) | i32::from(lsb & 0xFE);
            (exp - 0x4000) >> 1
        } else {
            (i32::from(lsb & 0x7E) - 0x40) >> 1
        };
        *coeff = i16::try_from(value).unwrap_or(0);
        let mut zeros: i32 = 0;
        while last & 0x80 != 0 {
            last = self.src.coded(RL)?;
            if zeros >= 1 << 24 {
                return Err(Error::Invalid("zero run too long"));
            }
            zeros = (zeros << 7) | i32::from(last & 0x7F);
        }
        Ok(zeros)
    }
}

/// Temporal signal decoder (`EntropyDecoder` of type `EDTTemporal`).
pub(crate) struct Temporal<'a> {
    src: Source<'a>,
    started: bool,
    state: u8,
}

impl<'a> Temporal<'a> {
    pub(crate) fn new(enabled: bool, rle_only: bool, data: &'a [u8]) -> Result<Self, Error> {
        let src = if !enabled {
            Source::None
        } else if rle_only {
            Source::Raw { data, pos: 0 }
        } else {
            let mut bits = Bits::new(data);
            let tables = vec![Table::parse(&mut bits)?, Table::parse(&mut bits)?];
            Source::Coded { bits, tables }
        };
        Ok(Self { src, started: false, state: 0 })
    }

    fn symbol(&mut self) -> Result<u8, Error> {
        let s = if !self.started {
            match &mut self.src {
                Source::Coded { bits, .. } => u8::try_from(bits.read(8)?).unwrap_or(0),
                _ => self.src.raw()?,
            }
        } else {
            match self.src {
                Source::Raw { .. } => self.src.raw()?,
                _ => self.src.coded(usize::from(self.state))?,
            }
        };
        if !self.started {
            self.state = s & 1;
            self.started = true;
        } else if s & 0x80 == 0 {
            self.state ^= 1;
        }
        Ok(s)
    }

    /// The next run: `(intra, length)`, or `NO_DATA` with inter.
    pub(crate) fn decode(&mut self) -> Result<(bool, i32), Error> {
        if matches!(self.src, Source::None) {
            return Ok((false, NO_DATA));
        }
        let mut value = self.state;
        if !self.started {
            value = self.symbol()? & 1;
        }
        let mut count: i32 = 0;
        loop {
            let s = self.symbol()?;
            if count > i32::MAX >> 7 {
                return Err(Error::Invalid("temporal run too long"));
            }
            count = (count << 7) | i32::from(s & 0x7F);
            if s & 0x80 == 0 {
                break;
            }
        }
        Ok((value == 1, count))
    }
}
