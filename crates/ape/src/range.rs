//! The bit reader and range decoder of a version 3.99+ frame (`UnBitArrayBase.cpp`,
//! `UnBitArray.cpp` of the Monkey's Audio SDK).
//!
//! A frame is read as little-endian 32-bit words whose bits are taken most significant first. As
//! in the reference reader, a read that would reach the last available bit fails; the caller
//! then mutes the frame.

/// The read ran out of data or the range coder hit an impossible state (a damaged frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Damaged;

pub(crate) type Res<T> = Result<T, Damaged>;

const RANGE_OVERFLOW_SHIFT: u32 = 16;
const CODE_BITS: u32 = 32;
const TOP_VALUE: u32 = 1 << (CODE_BITS - 1);
const EXTRA_BITS: u32 = (CODE_BITS - 2) % 8 + 1;
const BOTTOM_VALUE: u32 = TOP_VALUE >> 8;
const OVERFLOW_SIGNAL: u32 = 1;
const OVERFLOW_PIVOT_VALUE: u32 = 32768;
const MODEL_ELEMENTS: u32 = 64;
/// The reference decoder recurses on every overflow signal; a valid stream sends at most one.
const MAX_OVERFLOW_SIGNALS: u32 = 8;

const RANGE_TOTAL_2: [u32; 65] = [
    0, 19578, 36160, 48417, 56323, 60899, 63265, 64435, 64971, 65232, 65351, 65416, 65447, 65466, 65476, 65482, 65485, 65488, 65490, 65491, 65492, 65493,
    65494, 65495, 65496, 65497, 65498, 65499, 65500, 65501, 65502, 65503, 65504, 65505, 65506, 65507, 65508, 65509, 65510, 65511, 65512, 65513, 65514, 65515,
    65516, 65517, 65518, 65519, 65520, 65521, 65522, 65523, 65524, 65525, 65526, 65527, 65528, 65529, 65530, 65531, 65532, 65533, 65534, 65535, 65536,
];
const RANGE_WIDTH_2: [u32; 64] = [
    19578, 16582, 12257, 7906, 4576, 2366, 1170, 536, 261, 119, 65, 31, 19, 10, 6, 3, 3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];
const K_SUM_MIN_BOUNDARY: [u32; 32] = [
    0,
    32,
    64,
    128,
    256,
    512,
    1024,
    2048,
    4096,
    8192,
    16384,
    32768,
    65536,
    131_072,
    262_144,
    524_288,
    1_048_576,
    2_097_152,
    4_194_304,
    8_388_608,
    16_777_216,
    33_554_432,
    67_108_864,
    134_217_728,
    268_435_456,
    536_870_912,
    1_073_741_824,
    2_147_483_648,
    0,
    0,
    0,
    0,
];

/// The adaptive state of one channel's value coder (`UNBIT_ARRAY_STATE`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct State {
    k: u32,
    k_sum: u32,
}

impl State {
    /// `FlushState`.
    pub(crate) fn new() -> Self {
        Self { k: 10, k_sum: (1 << 10) * 16 }
    }
}

/// The overflow symbol for a 16-bit cumulative frequency (`RangeOverflowTable`).
fn overflow_symbol(total: u32) -> u32 {
    // RANGE_TOTAL_2 is increasing: the symbol is the last index whose start is <= total.
    let idx = RANGE_TOTAL_2.partition_point(|&t| t <= total);
    (idx as u32).saturating_sub(1).min(MODEL_ELEMENTS - 1)
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    /// Bits that may be read; a read reaching this index fails.
    bits: u64,
    pos: u64,
    low: u32,
    range: u32,
    buffer: u32,
}

impl<'a> Reader<'a> {
    /// A reader over `data` (which starts on a 4-byte boundary of the file) positioned at bit
    /// `start_bit`.
    pub(crate) fn new(data: &'a [u8], start_bit: u64) -> Self {
        Self { data, bits: data.len() as u64 * 8, pos: start_bit, low: 0, range: 0, buffer: 0 }
    }

    fn byte_at(&self, bit: u64) -> u32 {
        // Bit `bit` lies in word bit/32; within the little-endian word, bits are taken from the
        // most significant byte down.
        let word = (bit / 32) as usize;
        let byte_in_word = 3 - ((bit % 32) / 8) as usize;
        self.data.get(word * 4 + byte_in_word).copied().map_or(0, u32::from)
    }

    /// `DecodeValueXBits` (n ≤ 32).
    pub(crate) fn bits(&mut self, n: u32) -> Res<u32> {
        if self.pos + u64::from(n) >= self.bits {
            return Err(Damaged);
        }
        let mut v: u64 = 0;
        let mut left = n;
        while left > 0 {
            let in_byte = 8 - (self.pos % 8) as u32;
            let take = in_byte.min(left);
            let byte = self.byte_at(self.pos);
            let chunk = (byte >> (in_byte - take)) & ((1 << take) - 1);
            v = (v << take) | u64::from(chunk);
            self.pos += u64::from(take);
            left -= take;
        }
        Ok(v as u32)
    }

    fn byte(&mut self) -> Res<u32> {
        self.bits(8)
    }

    /// `FlushBitArray`: start the range coder at the next byte boundary.
    pub(crate) fn start_range(&mut self) -> Res<()> {
        let rem = self.pos % 8;
        if rem != 0 {
            self.pos += 8 - rem;
        }
        self.byte()?; // a dummy byte the encoder always writes
        self.buffer = self.byte()?;
        self.low = self.buffer >> (8 - EXTRA_BITS);
        self.range = 1 << EXTRA_BITS;
        Ok(())
    }

    fn normalize(&mut self) -> Res<()> {
        while self.range <= BOTTOM_VALUE {
            if self.range == 0 {
                return Err(Damaged);
            }
            self.buffer = (self.buffer << 8) | self.byte()?;
            self.low = (self.low << 8) | ((self.buffer >> 1) & 0xFF);
            self.range <<= 8;
        }
        Ok(())
    }

    fn decode_fast(&mut self, shift: u32) -> Res<u32> {
        self.normalize()?;
        self.range >>= shift;
        self.low.checked_div(self.range).ok_or(Damaged)
    }

    fn decode_fast_update(&mut self, shift: u32) -> Res<u32> {
        self.normalize()?;
        self.range >>= shift;
        let r = self.low.checked_div(self.range).ok_or(Damaged)?;
        self.low %= self.range;
        Ok(r)
    }

    fn decode_divisor(&mut self, divisor: u32) -> Res<u32> {
        self.normalize()?;
        self.range = self.range.checked_div(divisor).ok_or(Damaged)?;
        let r = self.low.checked_div(self.range).ok_or(Damaged)?;
        self.low %= self.range;
        Ok(r)
    }

    fn decode_overflow(&mut self, pivot: &mut u32) -> Res<u32> {
        for _ in 0..=MAX_OVERFLOW_SIGNALS {
            let total = self.decode_fast(RANGE_OVERFLOW_SHIFT)?;
            if total >= 65536 {
                return Err(Damaged);
            }
            let mut overflow = overflow_symbol(total);
            let idx = overflow as usize;
            let (Some(&start), Some(&width)) = (RANGE_TOTAL_2.get(idx), RANGE_WIDTH_2.get(idx)) else {
                return Err(Damaged);
            };
            self.low = self.low.wrapping_sub(self.range.wrapping_mul(start));
            self.range = self.range.wrapping_mul(width);
            if overflow == MODEL_ELEMENTS - 1 {
                overflow = self.decode_fast_update(16)? << 16;
                overflow |= self.decode_fast_update(16)?;
                if overflow == OVERFLOW_SIGNAL {
                    *pivot = OVERFLOW_PIVOT_VALUE;
                    continue;
                }
            }
            return Ok(overflow);
        }
        Err(Damaged)
    }

    /// `DecodeValueRange`: one signed residual.
    pub(crate) fn value(&mut self, state: &mut State) -> Res<i64> {
        let mut pivot = (state.k_sum / 32).max(1);
        let overflow = self.decode_overflow(&mut pivot)?;
        let base = if pivot >= 1 << 16 {
            let pivot_bits = 32 - pivot.leading_zeros();
            let shift = pivot_bits.saturating_sub(16);
            let split = 1u32 << shift;
            let a = self.decode_divisor((pivot / split) + 1)?;
            let b = self.decode_divisor(split)?;
            a.wrapping_mul(split).wrapping_add(b)
        } else {
            self.decode_divisor(pivot)?
        };
        let value = i64::from(base) + i64::from(overflow) * i64::from(pivot);
        state.k_sum = state.k_sum.wrapping_add(((value + 1) / 2) as u32).wrapping_sub(state.k_sum.wrapping_add(16) >> 5);
        let boundary = |k: u32| K_SUM_MIN_BOUNDARY.get(k as usize).copied().unwrap_or(0);
        if state.k_sum < boundary(state.k) {
            state.k = state.k.wrapping_sub(1);
        } else {
            let next = boundary(state.k.wrapping_add(1));
            if next != 0 && state.k_sum >= next {
                state.k = state.k.wrapping_add(1);
            }
        }
        Ok(if value & 1 != 0 { (value >> 1) + 1 } else { -(value >> 1) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_table_matches_the_reference_lookup() {
        // RangeOverflowTable: z >= RANGE_TOTAL[n + 1] advances n.
        let mut n = 0usize;
        for z in 0..65536u32 {
            if z >= RANGE_TOTAL_2[n + 1] {
                n += 1;
            }
            assert_eq!(overflow_symbol(z), n as u32, "z = {z}");
        }
    }

    #[test]
    fn bits_are_read_msb_first_from_little_endian_words() {
        let data = [0x78, 0x56, 0x34, 0x12, 0xF0, 0xDE, 0xBC, 0x9A, 0, 0, 0, 0];
        let mut r = Reader::new(&data, 0);
        assert_eq!(r.bits(8), Ok(0x12));
        assert_eq!(r.bits(4), Ok(0x3));
        assert_eq!(r.bits(24), Ok(0x0045_6789));
        assert_eq!(r.bits(28), Ok(0xABC_DEF0));
        // The last available bit can never be read (as in the reference reader).
        assert_eq!(r.bits(32), Err(Damaged));
    }
}
