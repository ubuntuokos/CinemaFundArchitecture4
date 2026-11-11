//! The range coder (RFC 9043 section 3.8.1).

use crate::tables::{CONTEXT_SIZE, DEFAULT_STATE_TRANSITION};

/// `one_state` / `zero_state` (section 3.8.1.4).
#[derive(Clone, Debug)]
pub(crate) struct StateTables {
    pub one: [u8; 256],
    pub zero: [u8; 256],
}

impl StateTables {
    /// The tables for `state_transition_delta` (all zero: the default table, Figure 24).
    /// `None` when a delta takes a state outside 0..=255.
    pub fn new(delta: &[i32; 256]) -> Option<StateTables> {
        let mut one = [0u8; 256];
        for (i, o) in one.iter_mut().enumerate() {
            let v = i32::from(DEFAULT_STATE_TRANSITION[i]) + delta[i];
            *o = u8::try_from(v).ok()?;
        }
        // Figure 23: zero_state_i = 256 - one_state_(256-i), kept in 8 bits (a one_state of 0 at
        // the top of the default table gives 0); state 0 has no 1-side partner
        let mut zero = [0u8; 256];
        for (i, z) in zero.iter_mut().enumerate().skip(1) {
            *z = (256 - u32::from(one[256 - i])) as u8;
        }
        Some(StateTables { one, zero })
    }

    pub fn default_tables() -> StateTables {
        // the default table is in range by construction
        StateTables::new(&[0; 256]).unwrap_or(StateTables { one: DEFAULT_STATE_TRANSITION, zero: [0; 256] })
    }
}

/// A range decoder over `buf` in Closed mode: bytes past the end read as 0.
pub(crate) struct RangeDecoder<'a> {
    buf: &'a [u8],
    /// Bytes consumed so far (j_i).
    pos: usize,
    low: u32,
    range: u32,
    end: bool,
}

impl<'a> RangeDecoder<'a> {
    /// Figure 18.
    pub fn new(buf: &'a [u8]) -> RangeDecoder<'a> {
        let b = |i: usize| u32::from(buf.get(i).copied().unwrap_or(0));
        let mut low = (b(0) << 8) | b(1);
        let range = 0xFF00;
        let mut end = buf.len() <= 2;
        if low >= range {
            low = range;
            end = true;
        }
        RangeDecoder { buf, pos: 2, low, range, end }
    }

    /// Bytes read from the buffer so far.
    pub fn bytes_read(&self) -> usize {
        self.pos
    }

    /// Figure 19.
    fn refill(&mut self) {
        if self.range < 256 {
            self.range <<= 8;
            self.low <<= 8;
            if !self.end {
                self.low += u32::from(self.buf.get(self.pos).copied().unwrap_or(0));
                self.pos += 1;
                if self.pos >= self.buf.len() {
                    self.end = true;
                }
            }
        }
    }

    /// Figure 20: one binary value with its adaptive `state`.
    #[inline]
    pub fn get_rac(&mut self, t: &StateTables, state: &mut u8) -> bool {
        let rangeoff = (self.range * u32::from(*state)) >> 8;
        self.range -= rangeoff;
        if self.low < self.range {
            *state = t.zero[usize::from(*state)];
            self.refill();
            false
        } else {
            self.low -= self.range;
            *state = t.one[usize::from(*state)];
            self.range = rangeoff;
            self.refill();
            true
        }
    }

    /// Figure 21: a scalar with the 32 states of its context.
    pub fn get_symbol(&mut self, t: &StateTables, state: &mut [u8; CONTEXT_SIZE], signed: bool) -> i32 {
        if self.get_rac(t, &mut state[0]) {
            return 0;
        }
        let mut e = 0usize;
        while self.get_rac(t, &mut state[1 + e.min(9)]) {
            e += 1;
            // a 32-bit value has at most 31 exponent bits; anything longer is corrupt
            if e > 31 {
                return 0;
            }
        }
        let mut a: i64 = 1;
        for i in (0..e).rev() {
            a = a * 2 + i64::from(self.get_rac(t, &mut state[22 + i.min(9)]));
        }
        let a = i32::try_from(a).unwrap_or(i32::MAX);
        if signed && self.get_rac(t, &mut state[11 + e.min(10)]) { -a } else { a }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_state_mirrors_one_state() {
        let t = StateTables::default_tables();
        for i in 1..256 {
            assert_eq!(u32::from(t.zero[i]), (256 - u32::from(t.one[256 - i])) & 255);
        }
    }

    #[test]
    fn empty_buffer_never_panics() {
        let t = StateTables::default_tables();
        let mut rc = RangeDecoder::new(&[]);
        let mut st = [128u8; CONTEXT_SIZE];
        for _ in 0..1000 {
            let _ = rc.get_symbol(&t, &mut st, true);
        }
    }
}
