//! CABAC arithmetic encoder (9.3.5, informative in the standard) and a bit-cost estimator with the
//! same interface, so syntax writers can either produce the slice data or measure its cost.

use crate::cabac::{Contexts, NEXT_STATE, RANGE_TAB_LPS};
use std::sync::OnceLock;

/// Sink for CABAC bins. `ctx` gives access to the context states, which both implementations
/// update identically.
pub trait Bins {
    fn decision(&mut self, ctx: usize, bin: u32);
    fn bypass(&mut self, bin: u32);
    /// `n` (<= 32) bypass bins, most significant first.
    fn bypass_bits(&mut self, v: u32, n: u32);
    fn terminate(&mut self, bin: u32);
}

/// Renormalisation shift after an LPS, indexed by rangeLPS >> 3.
static RENORM: [u8; 32] = [6, 5, 4, 4, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];

/// Byte-oriented CABAC encoder with carry propagation through buffered 0xff bytes.
pub struct Writer {
    pub ctx: Contexts,
    low: u32,
    range: u32,
    bits_left: i32,
    buffered: u32,
    num_buffered: u32,
    out: Vec<u8>,
}

impl Writer {
    pub fn new(ctx: Contexts) -> Self {
        Writer { ctx, low: 0, range: 510, bits_left: 23, buffered: 0xff, num_buffered: 0, out: Vec::new() }
    }

    #[inline]
    fn test_and_write_out(&mut self) {
        if self.bits_left < 12 {
            self.write_out();
        }
    }

    fn write_out(&mut self) {
        let lead = self.low >> (24 - self.bits_left);
        self.bits_left += 8;
        self.low &= 0xffff_ffffu32 >> self.bits_left;
        if lead == 0xff {
            self.num_buffered += 1;
        } else if self.num_buffered > 0 {
            let carry = lead >> 8;
            let byte = self.buffered + carry;
            self.buffered = lead & 0xff;
            self.out.push(byte as u8);
            let fill = ((0xff + carry) & 0xff) as u8;
            while self.num_buffered > 1 {
                self.out.push(fill);
                self.num_buffered -= 1;
            }
        } else {
            self.num_buffered = 1;
            self.buffered = lead;
        }
    }

    /// Flush the engine after end_of_slice_segment_flag (or end_of_subset_one_bit) and append the
    /// stop bit and alignment: returns the slice segment data bytes.
    pub fn finish(mut self) -> Vec<u8> {
        if (self.low >> (32 - self.bits_left)) != 0 {
            self.out.push((self.buffered + 1) as u8);
            while self.num_buffered > 1 {
                self.out.push(0x00);
                self.num_buffered -= 1;
            }
            self.low -= 1 << (32 - self.bits_left);
        } else {
            if self.num_buffered > 0 {
                self.out.push(self.buffered as u8);
            }
            while self.num_buffered > 1 {
                self.out.push(0xff);
                self.num_buffered -= 1;
            }
        }
        // the remaining 24 - bits_left bits of low, then rbsp_slice_segment_trailing_bits
        let n = (24 - self.bits_left) as u32;
        let mut acc = ((self.low >> 8) as u64) & ((1u64 << n) - 1);
        let mut nbits = n;
        acc = (acc << 1) | 1;
        nbits += 1;
        let pad = (8 - nbits % 8) % 8;
        acc <<= pad;
        nbits += pad;
        while nbits > 0 {
            nbits -= 8;
            self.out.push((acc >> nbits) as u8);
        }
        self.out
    }
}

impl Bins for Writer {
    #[inline]
    fn decision(&mut self, ctx: usize, bin: u32) {
        let Some(&s) = self.ctx.get(ctx) else { return };
        let state = (s >> 1) as usize;
        let mps = (s & 1) as u32;
        let lps = RANGE_TAB_LPS[state][((self.range >> 6) & 3) as usize] as u32;
        self.range -= lps;
        if bin != mps {
            let nb = RENORM[(lps >> 3) as usize] as i32;
            self.low = (self.low + self.range) << nb;
            self.range = lps << nb;
            self.bits_left -= nb;
            self.ctx[ctx] = NEXT_STATE[s as usize][1];
        } else {
            self.ctx[ctx] = NEXT_STATE[s as usize][0];
            if self.range >= 256 {
                return;
            }
            self.low <<= 1;
            self.range <<= 1;
            self.bits_left -= 1;
        }
        self.test_and_write_out();
    }

    #[inline]
    fn bypass(&mut self, bin: u32) {
        self.low <<= 1;
        if bin != 0 {
            self.low += self.range;
        }
        self.bits_left -= 1;
        self.test_and_write_out();
    }

    fn bypass_bits(&mut self, v: u32, n: u32) {
        let mut n = n.min(32);
        while n > 8 {
            n -= 8;
            let part = (v >> n) & 0xff;
            self.low = (self.low << 8) + self.range * part;
            self.bits_left -= 8;
            self.test_and_write_out();
        }
        if n > 0 {
            let part = v & ((1u32 << n) - 1);
            self.low = (self.low << n) + self.range * part;
            self.bits_left -= n as i32;
            self.test_and_write_out();
        }
    }

    fn terminate(&mut self, bin: u32) {
        self.range -= 2;
        if bin != 0 {
            self.low += self.range;
            self.low <<= 7;
            self.range = 2 << 7;
            self.bits_left -= 7;
        } else if self.range >= 256 {
            return;
        } else {
            self.low <<= 1;
            self.range <<= 1;
            self.bits_left -= 1;
        }
        self.test_and_write_out();
    }
}

/// Fractional bits are counted in units of 1 / 2^15 bit.
pub const BIT: u64 = 1 << 15;

/// Cost in 1/32768 bits of coding an MPS (`[s][0]`) or an LPS (`[s][1]`) in state `s >> 1`.
fn entropy_bits() -> &'static [[u32; 2]; 64] {
    static T: OnceLock<[[u32; 2]; 64]> = OnceLock::new();
    T.get_or_init(|| {
        // pLPS(0) = 0.5, pLPS(63) = 0.01875 with a geometric progression in between (9.3.4.3.1)
        let alpha = (0.01875f64 / 0.5).powf(1.0 / 63.0);
        let mut t = [[0u32; 2]; 64];
        for (s, e) in t.iter_mut().enumerate() {
            let p = 0.5 * alpha.powi(s as i32);
            e[0] = (-(1.0 - p).log2() * BIT as f64).round() as u32;
            e[1] = (-p.log2() * BIT as f64).round() as u32;
        }
        t
    })
}

/// Bit-cost estimator: tracks context states like the encoder and sums the entropy of each bin.
#[derive(Clone)]
pub struct Estimator {
    pub ctx: Contexts,
    pub bits: u64,
    table: &'static [[u32; 2]; 64],
}

impl Estimator {
    pub fn new(ctx: Contexts) -> Self {
        Estimator { ctx, bits: 0, table: entropy_bits() }
    }
}

impl Bins for Estimator {
    #[inline]
    fn decision(&mut self, ctx: usize, bin: u32) {
        let Some(&s) = self.ctx.get(ctx) else { return };
        let lps = (bin != (s & 1) as u32) as usize;
        self.bits += self.table[(s >> 1) as usize][lps] as u64;
        self.ctx[ctx] = NEXT_STATE[s as usize][lps];
    }
    #[inline]
    fn bypass(&mut self, _bin: u32) {
        self.bits += BIT;
    }
    #[inline]
    fn bypass_bits(&mut self, _v: u32, n: u32) {
        self.bits += BIT * n as u64;
    }
    fn terminate(&mut self, _bin: u32) {
        self.bits += 7 * BIT / 1000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cabac::{Cabac, init_contexts};

    /// Encode pseudo-random bins of every kind and decode them back with the decoder's engine.
    #[test]
    fn round_trip() {
        let mut ctx: Contexts = [0; crate::spec_tables::NUM_CTX.next_power_of_two()];
        init_contexts(&mut ctx, 30, 1);
        let mut w = Writer::new(ctx);
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            seed >> 16
        };
        let mut ops = Vec::new();
        for i in 0..20000 {
            let r = rnd();
            let kind = r % 7;
            match kind {
                0..=3 => {
                    let c = (r as usize / 7) % 40;
                    // skewed bins so contexts adapt
                    let bin = ((rnd() % 10) < (c as u32 % 9 + 1)) as u32;
                    w.decision(c, bin);
                    ops.push((0, c as u32, bin));
                }
                4 => {
                    let b = rnd() & 1;
                    w.bypass(b);
                    ops.push((1, 0, b));
                }
                5 => {
                    let n = rnd() % 17;
                    let v = rnd() & ((1u32 << n) - 1);
                    w.bypass_bits(v, n);
                    ops.push((2, n, v));
                }
                _ => {
                    let b = (i % 997 == 0) as u32;
                    if b == 0 {
                        w.terminate(0);
                        ops.push((3, 0, 0));
                    }
                }
            }
        }
        w.terminate(1);
        let data = w.finish();
        let mut d = Cabac::new(&data, 0, 30, 1).expect("engine");
        for (k, &(kind, a, b)) in ops.iter().enumerate() {
            let got = match kind {
                0 => d.decode_decision(a as usize),
                1 => d.decode_bypass(),
                2 => d.decode_bypass_bits(a),
                _ => d.decode_terminate(),
            };
            assert_eq!(got, b, "op {k} kind {kind}");
        }
        assert_eq!(d.decode_terminate(), 1);
        // the stop bit follows the arithmetic code at the decoder's position
        let pos = d.bit_pos();
        assert!((data.len() * 8).saturating_sub(pos) < 8, "{pos} of {}", data.len() * 8);
    }

    #[test]
    fn estimator_tracks_states() {
        let mut ctx: Contexts = [0; crate::spec_tables::NUM_CTX.next_power_of_two()];
        init_contexts(&mut ctx, 30, 0);
        let mut w = Writer::new(ctx);
        let mut e = Estimator::new(ctx);
        for i in 0..1000 {
            let bin = (i % 5 == 0) as u32;
            w.decision(3, bin);
            e.decision(3, bin);
        }
        assert_eq!(w.ctx, e.ctx);
        // 1000 skewed bins cost well under 1000 bits
        assert!(e.bits < 800 * BIT && e.bits > 500 * BIT, "{}", e.bits / BIT);
    }
}
