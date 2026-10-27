//! Symbol (arithmetic) decoder, spec 8.2.
//!
//! The state follows the specification literally: `SymbolValue` / `SymbolRange` are 15/16-bit
//! quantities renormalised by reading `bits` new bits at a time, and `SymbolMaxBits` counts the
//! bits still available (going negative while zero padding is consumed).

const EC_PROB_SHIFT: u32 = 6;
const EC_MIN_PROB: u32 = 4;

pub(crate) struct SymbolDecoder<'a> {
    data: &'a [u8],
    /// Next byte of `data` to load into the window.
    next: usize,
    /// SymbolValue followed by `k` look-ahead bits: the bitstream with every bit inverted
    /// (the spec's renormalisation XORs the new bits with ones), zero bits past the end of
    /// the data reading as ones. SymbolValue = win >> k.
    win: u64,
    k: u32,
    range: u32,
    pub disable_update: bool,
}

impl<'a> SymbolDecoder<'a> {
    /// `init_symbol( sz )` over `data` (the tile's bytes).
    pub fn new(data: &'a [u8], disable_update: bool) -> Self {
        let mut d = SymbolDecoder { data, next: 0, win: 0, k: 0, range: 1 << 15, disable_update };
        // SymbolValue = ((1 << 15) - 1) ^ (first 15 bits): 15 inverted bits.
        let mut filled = 0;
        while filled < 15 + 32 {
            d.win = (d.win << 8) | d.inverted_byte() as u64;
            filled += 8;
        }
        d.k = filled - 15;
        d
    }

    #[inline(always)]
    fn inverted_byte(&mut self) -> u8 {
        let b = self.data.get(self.next).copied().unwrap_or(0);
        self.next += 1;
        !b
    }

    /// `read_symbol( cdf )`: `cdf` has N + 1 entries (the last is the adaptation counter).
    #[inline]
    pub fn read_symbol(&mut self, cdf: &mut [u16]) -> usize {
        let n = cdf.len() - 1;
        let symbol = self.decode(cdf, n);
        if !self.disable_update {
            update_cdf(cdf, n, symbol);
        }
        symbol
    }

    #[inline(always)]
    fn decode(&mut self, cdf: &[u16], n: usize) -> usize {
        let value = (self.win >> self.k) as u32;
        let mut cur = self.range;
        let mut symbol = 0usize;
        let mut prev;
        loop {
            prev = cur;
            let f = (1u32 << 15) - cdf[symbol] as u32;
            cur = ((self.range >> 8) * (f >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT);
            cur += EC_MIN_PROB * (n - symbol - 1) as u32;
            if value >= cur {
                break;
            }
            symbol += 1;
        }
        self.range = prev - cur;
        self.win -= (cur as u64) << self.k;
        self.renormalize();
        symbol
    }

    /// Renormalisation (8.2.6): SymbolValue takes `bits` more (inverted) bits, which are
    /// already in the window.
    #[inline(always)]
    fn renormalize(&mut self) {
        let bits = 15 - (31 - self.range.leading_zeros());
        self.range <<= bits;
        self.k -= bits;
        if self.k < 16 {
            // SymbolValue < 2^16, so 16 + k <= 64 bits stay in the window.
            while self.k <= 40 {
                self.win = (self.win << 8) | self.inverted_byte() as u64;
                self.k += 8;
            }
        }
    }

    /// `read_bool()`
    #[inline]
    pub fn read_bool(&mut self) -> u32 {
        let cdf = [1u16 << 14, 1 << 15, 0];
        self.decode(&cdf, 2) as u32
    }

    /// `read_literal( n )` (L(n))
    pub fn read_literal(&mut self, n: u32) -> u32 {
        let mut x = 0;
        for _ in 0..n {
            x = (x << 1) | self.read_bool();
        }
        x
    }

    /// NS(n)
    pub fn read_ns(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        let w = 32 - n.leading_zeros();
        let m = (1u32 << w) - n;
        let v = self.read_literal(w - 1);
        if v < m {
            return v;
        }
        let extra = self.read_literal(1);
        (v << 1) - m + extra
    }
}

/// A source of symbols for the tile syntax: the arithmetic decoder, or the encoder, which codes
/// the value its policy chose (`v`) and returns it, so the same syntax code drives both.
pub(crate) trait SymbolIo {
    /// The syntax code asks the encoder's policy for each value only when this is set.
    const ENCODE: bool;
    /// `read_symbol( cdf )`; the encoder codes `v` (clamped to the alphabet).
    fn symbol(&mut self, cdf: &mut [u16], v: u32) -> usize;
    /// `read_bool()`
    fn bool_bit(&mut self, v: u32) -> u32;
    fn disable_update(&self) -> bool;
    fn set_disable_update(&mut self, on: bool);

    /// `read_literal( n )` (L(n)), most significant bit first.
    fn literal(&mut self, n: u32, v: u32) -> u32 {
        let mut x = 0;
        for i in (0..n.min(32)).rev() {
            x = (x << 1) | self.bool_bit((v >> i) & 1);
        }
        x
    }

    /// NS(n)
    fn ns(&mut self, n: u32, v: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        let w = 32 - n.leading_zeros();
        let m = (1u32 << w) - n;
        let v = v.min(n - 1);
        let (hi, extra) = if v < m { (v, 0) } else { ((v + m) >> 1, (v + m) & 1) };
        let r = self.literal(w - 1, hi);
        if r < m {
            return r;
        }
        let e = self.literal(1, extra);
        (r << 1) - m + e
    }
}

impl SymbolIo for SymbolDecoder<'_> {
    const ENCODE: bool = false;
    #[inline(always)]
    fn symbol(&mut self, cdf: &mut [u16], _v: u32) -> usize {
        self.read_symbol(cdf)
    }
    #[inline(always)]
    fn bool_bit(&mut self, _v: u32) -> u32 {
        self.read_bool()
    }
    fn disable_update(&self) -> bool {
        self.disable_update
    }
    fn set_disable_update(&mut self, on: bool) {
        self.disable_update = on;
    }
    #[inline]
    fn literal(&mut self, n: u32, _v: u32) -> u32 {
        self.read_literal(n)
    }
    fn ns(&mut self, n: u32, _v: u32) -> u32 {
        self.read_ns(n)
    }
}

/// The arithmetic encoder matching [`SymbolDecoder`]: the interval arithmetic of 8.2.6 run
/// forwards, with the bytes kept with room for carries until [`SymbolEncoder::finish`].
pub(crate) struct SymbolEncoder {
    low: u64,
    rng: u32,
    /// Bits in `low` not yet output, minus 24 (it starts at -9: 15 bits of precision plus the 24
    /// carry bits).
    cnt: i32,
    /// Output bytes before carry propagation (each may carry into the previous one).
    pre: Vec<u16>,
    pub disable_update: bool,
}

impl SymbolEncoder {
    pub fn new(disable_update: bool) -> Self {
        SymbolEncoder { low: 0, rng: 0x8000, cnt: -9, pre: Vec::new(), disable_update }
    }

    /// Codes symbol `s` of the `n`-symbol alphabet of `cdf` (increasing, the spec's tables).
    #[inline]
    fn encode(&mut self, cdf: &[u16], n: usize, s: usize) {
        let r = self.rng;
        let bound = |k: usize| -> u32 {
            let f = (1u32 << 15) - cdf.get(k).copied().unwrap_or(1 << 15) as u32;
            (((r >> 8) * (f >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (n - k - 1) as u32
        };
        let v = bound(s);
        if s > 0 {
            let u = bound(s - 1);
            self.low += (r - u) as u64;
            self.rng = u - v;
        } else {
            self.rng = r - v;
        }
        self.normalize();
    }

    #[inline]
    fn normalize(&mut self) {
        let rng = self.rng.max(1);
        let d = 15 - (31 - rng.leading_zeros()) as i32;
        let mut c = self.cnt;
        let mut s = c + d;
        if s >= 0 {
            c += 16;
            let mut m = (1u64 << c) - 1;
            if s >= 8 {
                self.pre.push((self.low >> c) as u16);
                self.low &= m;
                c -= 8;
                m >>= 8;
            }
            self.pre.push((self.low >> c) as u16);
            s = c + d - 24;
            self.low &= m;
        }
        self.low <<= d;
        self.rng = rng << d;
        self.cnt = s;
    }

    /// Bytes output so far (an estimate of the coded size).
    pub fn bytes(&self) -> usize {
        self.pre.len()
    }

    /// Terminates the coded data: the fewest bits that decode every coded symbol whatever
    /// follows, then carries propagated into the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let mut c = self.cnt;
        let mut s = c + 10;
        let m = 0x3FFFu64;
        let mut e = ((self.low + m) & !m) | (m + 1);
        if s > 0 {
            let mut n = (1u64 << (c + 16)) - 1;
            while s > 0 {
                self.pre.push((e >> (c + 16)) as u16);
                e &= n;
                s -= 8;
                c -= 8;
                n >>= 8;
            }
        }
        let mut out = vec![0u8; self.pre.len()];
        let mut carry = 0u32;
        for (o, &p) in out.iter_mut().zip(&self.pre).rev() {
            carry += p as u32;
            *o = carry as u8;
            carry >>= 8;
        }
        out
    }
}

impl SymbolIo for SymbolEncoder {
    const ENCODE: bool = true;
    fn symbol(&mut self, cdf: &mut [u16], v: u32) -> usize {
        let n = cdf.len().saturating_sub(1);
        if n < 2 {
            return 0;
        }
        let s = (v as usize).min(n - 1);
        self.encode(cdf, n, s);
        if !self.disable_update {
            update_cdf(cdf, n, s);
        }
        s
    }
    fn bool_bit(&mut self, v: u32) -> u32 {
        let cdf = [1u16 << 14, 1 << 15, 0];
        let b = (v != 0) as usize;
        self.encode(&cdf, 2, b);
        b as u32
    }
    fn disable_update(&self) -> bool {
        self.disable_update
    }
    fn set_disable_update(&mut self, on: bool) {
        self.disable_update = on;
    }
}

/// Cost of coding symbol `s` with `cdf`, in 1/256 bits (for the encoder's decisions).
pub(crate) fn symbol_cost(cdf: &[u16], s: usize) -> u32 {
    let hi = cdf.get(s).copied().unwrap_or(1 << 15) as u32;
    let lo = if s == 0 { 0 } else { cdf.get(s - 1).copied().unwrap_or(0) as u32 };
    let p = hi.saturating_sub(lo).max(4);
    // -log2(p / 32768) * 256 = (15 - log2 p) * 256
    let l = 31 - p.leading_zeros();
    let frac = ((p << (15 - l.min(15))) >> 7) & 0xFF; // 8 fractional bits of the mantissa
    let log2p = (l << 8) + LOG2_FRAC[frac as usize] as u32;
    (15 << 8) - log2p.min(15 << 8)
}

/// log2(1 + i / 256) * 256
static LOG2_FRAC: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        // Binary logarithm digit by digit: square the Q30 mantissa, a digit is 1 when it reaches 2.
        let mut y: u64 = (256 + i as u64) << 22;
        let mut r = 0u16;
        let mut k = 0;
        while k < 8 {
            y = (y * y) >> 30;
            r <<= 1;
            if y >= 2 << 30 {
                y >>= 1;
                r |= 1;
            }
            k += 1;
        }
        t[i] = r;
        i += 1;
    }
    t
};

/// CDF adaptation (8.2.6).
#[inline(always)]
pub(crate) fn update_cdf(cdf: &mut [u16], n: usize, symbol: usize) {
    let count = cdf[n];
    let rate = 3 + (count > 15) as u32 + (count > 31) as u32 + (31 - (n as u32).leading_zeros()).min(2);
    // Entries below `symbol` move towards 0, the others towards 1 << 15 (the specification's
    // comparison with tmp, resolved per entry: c >= 0 and c <= 1 << 15), without branching on
    // the decoded symbol.
    for (i, e) in cdf[..n - 1].iter_mut().enumerate() {
        let c = *e as u32;
        *e = if i < symbol { c - (c >> rate) } else { c + (((1 << 15) - c) >> rate) } as u16;
    }
    cdf[n] += (count < 32) as u16;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u64) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 16) as u32
    }

    /// A random increasing CDF of `n` symbols (plus the counter), some very skewed.
    fn random_cdf(seed: &mut u64, n: usize) -> Vec<u16> {
        let mut w: Vec<u32> = (0..n).map(|_| 1 + rng(seed) % if rng(seed).is_multiple_of(4) { 3 } else { 1000 }).collect();
        if rng(seed).is_multiple_of(3) {
            w[rng(seed) as usize % n] += 100_000;
        }
        let total: u32 = w.iter().sum();
        let mut acc = 0u64;
        let mut cdf = Vec::new();
        for (i, x) in w.iter().enumerate() {
            acc += *x as u64;
            let v = if i + 1 == n { 32768 } else { ((acc * 32768) / total as u64).clamp(1, 32767) as u16 as u64 };
            cdf.push(v as u16);
        }
        for i in 1..n {
            cdf[i] = cdf[i].max(cdf[i - 1]);
        }
        cdf.push(0);
        cdf
    }

    #[test]
    fn encoder_round_trips_through_the_decoder() {
        for round in 0..200u64 {
            let mut seed = 0x9E37_79B9_7F4A_7C15 ^ round;
            let count = 1 + rng(&mut seed) as usize % 3000;
            let disable = round % 7 == 3;
            let mut cdfs: Vec<Vec<u16>> = (0..8).map(|i| random_cdf(&mut seed, 2 + (i * 3 + round as usize) % 15)).collect();
            let start = cdfs.clone();
            let mut ops = Vec::new();
            let mut enc = SymbolEncoder::new(disable);
            for _ in 0..count {
                match rng(&mut seed) % 5 {
                    0 => {
                        let n = 1 + rng(&mut seed) % 20;
                        let v = rng(&mut seed) & ((1u32 << n) - 1);
                        enc.literal(n, v);
                        ops.push((0usize, n, v));
                    }
                    1 => {
                        let n = 1 + rng(&mut seed) % 300;
                        let v = rng(&mut seed) % n;
                        enc.ns(n, v);
                        ops.push((1, n, v));
                    }
                    _ => {
                        let k = rng(&mut seed) as usize % cdfs.len();
                        let n = cdfs[k].len() as u32 - 1;
                        // favour likely symbols like real data
                        let v = if rng(&mut seed).is_multiple_of(2) { rng(&mut seed) % n } else { 0 };
                        enc.symbol(&mut cdfs[k], v);
                        ops.push((2 + k, n, v));
                    }
                }
            }
            let data = enc.finish();
            let mut cdfs = start;
            let mut dec = SymbolDecoder::new(&data, disable);
            for (i, &(op, n, v)) in ops.iter().enumerate() {
                let got = match op {
                    0 => dec.read_literal(n),
                    1 => dec.read_ns(n),
                    k => dec.read_symbol(&mut cdfs[k - 2]) as u32,
                };
                assert_eq!(got, v, "round {round} op {i} of {}", ops.len());
            }
        }
    }

    #[test]
    fn costs_follow_probabilities() {
        let cdf = [16384u16, 32768, 0];
        assert_eq!(symbol_cost(&cdf, 0), 256);
        assert_eq!(symbol_cost(&cdf, 1), 256);
        let cdf = [8192u16, 32768, 0];
        assert_eq!(symbol_cost(&cdf, 0), 512);
        let c = symbol_cost(&cdf, 1);
        assert!((100..=110).contains(&c), "{c}"); // log2(4/3) = 0.415 bits
    }
}
