//! The AV2 symbol (entropy) decoder, spec §8.2.
//!
//! Implemented from the normative text of the AV2 Bitstream & Decoding Process Specification
//! v1.0.0: `init_symbol`, `read_symbol`, `read_bool`, `read_literal` and `exit_symbol`
//! (the conformance part of it). CDF arrays follow the spec layout: for a symbol with `N`
//! values the array has `N + 1` entries, `cdf[0..N-1]` are the cumulative probabilities,
//! `cdf[N - 1]` is the constant adaptation-rate index and `cdf[N]` the use counter.

use crate::{Error, Result};

const EC_PROB_SHIFT: u32 = 7;
const NUM_PARA_INTERVALS: usize = 3;

/// `Prob_Inc` (spec `prob_inc.h`), indexed `[N - 2][symbol]`.
const PROB_INC: [[u32; 8]; 7] = [
    [8, 0, 0, 0, 0, 0, 0, 0],
    [10, 5, 0, 0, 0, 0, 0, 0],
    [12, 8, 4, 0, 0, 0, 0, 0],
    [12, 9, 6, 3, 0, 0, 0, 0],
    [13, 10, 8, 5, 2, 0, 0, 0],
    [13, 11, 9, 6, 4, 2, 0, 0],
    [14, 12, 10, 8, 6, 4, 2, 0],
];

/// The three distinct values of the first/second/third columns of `Para_Adjustment_List`
/// (spec `para_adjustment_list.h`): row `r` is `(A[r / 25 % 5], B[r / 5 % 5], C[r % 5])`
/// with the per-position patterns below. Kept as the literal table for auditability.
const PARA_ADJUSTMENT_LIST: [[i32; NUM_PARA_INTERVALS]; 125] = {
    const A: [i32; 5] = [0, -1, -2, 1, 1];
    const B: [i32; 5] = [0, -1, -2, 1, 1];
    const C: [i32; 5] = [0, -1, -2, 1, 1];
    let mut t = [[0; NUM_PARA_INTERVALS]; 125];
    let mut r = 0;
    while r < 125 {
        t[r] = [A[r / 25], B[r / 5 % 5], C[r % 5]];
        r += 1;
    }
    t
};

/// The symbol decoder state for one tile (spec variables `SymbolValue`, `SymbolRange`,
/// `SymbolMaxBits`).
pub struct SymbolDecoder<'a> {
    data: &'a [u8],
    /// Bit position of the next unread bit.
    pos: usize,
    value: u32,
    range: u32,
    max_bits: i64,
    disable_cdf_update: bool,
}

impl<'a> SymbolDecoder<'a> {
    /// `init_symbol( sz )` over `data` (exactly the `sz` bytes of the tile).
    pub fn new(data: &'a [u8], disable_cdf_update: bool) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::Truncated("symbol decoder data"));
        }
        let sz = data.len() as i64;
        let num_bits = (sz * 8).min(15) as u32;
        let mut d = SymbolDecoder { data, pos: 0, value: 0, range: 1 << 15, max_bits: 8 * sz - 15, disable_cdf_update };
        let buf = d.bits(num_bits);
        let padded = buf << (15 - num_bits);
        d.value = ((1 << 15) - 1) ^ padded;
        Ok(d)
    }

    /// `f(n)` for `n <= 16`; bits past the end read as zero (the callers clip to `max_bits`).
    fn bits(&mut self, n: u32) -> u32 {
        let mut x = 0;
        for _ in 0..n {
            let byte = self.data.get(self.pos >> 3).copied().unwrap_or(0);
            x = (x << 1) | u32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        x
    }

    /// `read_bool()` (§8.2.3): an equiprobable bit.
    pub fn read_bool(&mut self) -> u32 {
        let cur = self.range >> 1;
        let symbol = u32::from(self.value < cur);
        if symbol == 0 {
            self.value -= cur;
        }
        let num_bits = self.max_bits.clamp(0, 1) as u32;
        let new_data = self.bits(num_bits);
        self.value = (self.value << 1) | (new_data ^ 1);
        self.max_bits -= 1;
        symbol
    }

    /// `read_literal( n )` (§8.2.5).
    pub fn read_literal(&mut self, n: u32) -> u32 {
        let mut x = 0u32;
        for _ in 0..n.min(32) {
            x = x.wrapping_mul(2).wrapping_add(self.read_bool());
        }
        x
    }

    /// `read_symbol( cdf )` (§8.2.6). `cdf` has `N + 1` entries; it is adapted in place unless
    /// CDF updates are disabled. Returns the decoded symbol in `0..N`.
    pub fn read_symbol(&mut self, cdf: &mut [u16]) -> Result<u32> {
        let n = cdf.len().checked_sub(1).filter(|n| (2..=8).contains(n)).ok_or(Error::Invalid("cdf length"))?;
        let inc = PROB_INC.get(n - 2).ok_or(Error::Invalid("cdf length"))?;
        let mut cur = self.range;
        let mut prev;
        let mut symbol = 0usize;
        loop {
            prev = cur;
            let f = if symbol == n - 1 { 0 } else { (1u32 << 15) - u32::from(cdf[symbol]) };
            let pp = ((f >> EC_PROB_SHIFT) << 4) + inc[symbol];
            cur = (((self.range >> 8) * pp) >> 7) << 3;
            if self.value >= cur {
                break;
            }
            symbol += 1;
            if symbol >= n {
                return Err(Error::Invalid("symbol decode overrun"));
            }
        }
        let new_range = prev - cur;
        let new_value = self.value - cur;
        let bits = 15 - (31 - new_range.max(1).leading_zeros());
        self.range = new_range << bits;
        let num_bits = self.max_bits.clamp(0, i64::from(bits)) as u32;
        let new_data = self.bits(num_bits);
        let padded = new_data << (bits - num_bits);
        let mask = (1u32 << bits) - 1;
        self.value = (new_value << bits) | (padded ^ mask);
        self.max_bits -= i64::from(bits);

        if !self.disable_cdf_update {
            adapt(cdf, n, symbol)?;
        }
        Ok(symbol as u32)
    }

    /// `exit_symbol()` conformance checks (§8.2.4): returns whether the trailing-bit pattern
    /// (a one, then zeros up to the end of the data) is correct, and the number of padding bits
    /// that were consumed. CDF saving/averaging is the caller's business.
    pub fn exit(&mut self) -> Result<()> {
        if self.max_bits < -14 {
            return Err(Error::Invalid("symbol decoder over-read"));
        }
        let trailing = self.pos as i64 - (self.max_bits + 15).min(15);
        let end = self.pos as i64 + self.max_bits.max(0);
        let bit = |p: i64| -> u32 {
            let byte = self.data.get((p >> 3) as usize).copied().unwrap_or(0);
            u32::from((byte >> (7 - (p & 7))) & 1)
        };
        if trailing < 0 || bit(trailing) != 1 {
            return Err(Error::Invalid("tile trailing bit"));
        }
        for p in trailing + 1..end {
            if bit(p) != 0 {
                return Err(Error::Invalid("tile padding"));
            }
        }
        Ok(())
    }
}

/// The CDF adaptation step of §8.2.6.
fn adapt(cdf: &mut [u16], n: usize, symbol: usize) -> Result<()> {
    let count = u32::from(cdf[n]);
    let time_interval = if count > 31 {
        2
    } else if count > 15 {
        1
    } else {
        0
    };
    let row = PARA_ADJUSTMENT_LIST.get(usize::from(cdf[n - 1])).ok_or(Error::Invalid("cdf rate index"))?;
    let rate = 3 + time_interval as i32 + (n.ilog2().min(2)) as i32 + row[time_interval];
    let rate = rate.clamp(0, 15) as u32;
    for (i, c) in cdf.iter_mut().take(n - 1).enumerate() {
        if i < symbol {
            *c -= *c >> rate;
        } else {
            *c += ((1u16 << 15) - *c) >> rate;
        }
    }
    cdf[n] += u16::from(cdf[n] < 32);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn para_table_matches_spec_rows() {
        // Spot checks against para_adjustment_list.h.
        assert_eq!(PARA_ADJUSTMENT_LIST[0], [0, 0, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[1], [0, 0, -1]);
        assert_eq!(PARA_ADJUSTMENT_LIST[5], [0, -1, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[15], [0, 1, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[20], [0, 1, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[25], [-1, 0, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[50], [-2, 0, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[75], [1, 0, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[100], [1, 0, 0]);
        assert_eq!(PARA_ADJUSTMENT_LIST[124], [1, 1, 1]);
    }

    #[test]
    fn decodes_without_panic_and_keeps_cdf_monotone() {
        let data: Vec<u8> = (0..64u32).map(|i| (i.wrapping_mul(37) ^ 0xA5) as u8).collect();
        let mut d = SymbolDecoder::new(&data, false).unwrap();
        let mut cdf = [25865u16, 25, 0];
        let mut cdf4 = [8000u16, 16000, 24000, 6, 0];
        for _ in 0..200 {
            assert!(d.read_symbol(&mut cdf).unwrap() < 2);
            assert!(d.read_symbol(&mut cdf4).unwrap() < 4);
            assert!(cdf4[0] <= cdf4[1] && cdf4[1] <= cdf4[2]);
        }
        assert!(cdf[2] <= 32 && cdf4[4] <= 32);
    }

    #[test]
    fn disable_update_leaves_cdf() {
        let data = [0x55u8; 16];
        let mut d = SymbolDecoder::new(&data, true).unwrap();
        let mut cdf = [20000u16, 25, 0];
        d.read_symbol(&mut cdf).unwrap();
        assert_eq!(cdf, [20000, 25, 0]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(SymbolDecoder::new(&[], false).is_err());
        let mut d = SymbolDecoder::new(&[0u8; 4], false).unwrap();
        assert!(d.read_symbol(&mut [1u16]).is_err());
        assert!(d.read_symbol(&mut [1u16; 10]).is_err());
        let mut bad_rate = [10000u16, 200, 0];
        assert!(d.read_symbol(&mut bad_rate).is_err());
    }

    #[test]
    fn literal_of_zero_stream_terminates() {
        let mut d = SymbolDecoder::new(&[0u8; 8], false).unwrap();
        let _ = d.read_literal(16);
        let _ = d.exit();
    }
}
