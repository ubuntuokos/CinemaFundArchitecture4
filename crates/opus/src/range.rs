//! Range decoder (RFC 6716 §4.1).
//!
//! The decoder reads range-coded symbols from the front of the buffer and raw bits from the back.
//! Reading past either end yields zeros (never panics); callers detect corruption through
//! [`RangeDecoder::tell`] exceeding the budget or through out-of-range values.

const SYM_BITS: u32 = 8;
const CODE_BITS: u32 = 32;
const SYM_MAX: u32 = (1 << SYM_BITS) - 1;
const CODE_SHIFT: u32 = CODE_BITS - SYM_BITS - 1;
const CODE_TOP: u32 = 1 << (CODE_BITS - 1);
const CODE_BOT: u32 = CODE_TOP >> SYM_BITS;
const CODE_EXTRA: u32 = (CODE_BITS - 2) % SYM_BITS + 1;
/// Bit resolution of [`RangeDecoder::tell_frac`] (1/8 bit).
pub const BITRES: u32 = 3;

/// Number of bits needed to represent `x` (0 for 0).
#[inline]
pub fn ilog(x: u32) -> i32 {
    (32 - x.leading_zeros()) as i32
}

#[derive(Clone, Debug)]
pub struct RangeDecoder<'a> {
    buf: &'a [u8],
    /// Bytes available to the decoder (may be reduced, e.g. for redundancy).
    storage: usize,
    offs: usize,
    end_offs: usize,
    end_window: u32,
    nend_bits: i32,
    nbits_total: i32,
    rng: u32,
    val: u32,
    rem: u32,
    /// Set when an out-of-range value was decoded.
    pub error: bool,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        let mut d = RangeDecoder {
            buf,
            storage: buf.len(),
            offs: 0,
            end_offs: 0,
            end_window: 0,
            nend_bits: 0,
            nbits_total: (CODE_BITS + 1 - ((CODE_BITS - CODE_EXTRA) / SYM_BITS) * SYM_BITS) as i32,
            rng: 1 << CODE_EXTRA,
            val: 0,
            rem: 0,
            error: false,
        };
        d.rem = d.read_byte();
        d.val = d.rng - 1 - (d.rem >> (SYM_BITS - CODE_EXTRA));
        d.normalize();
        d
    }

    /// Size of the buffer in bytes as seen by the decoder.
    pub fn storage(&self) -> usize {
        self.storage
    }

    /// Shrinks the buffer seen by the decoder (raw bits are then read from the new end).
    pub fn shrink_storage(&mut self, by: usize) {
        self.storage = self.storage.saturating_sub(by);
    }

    pub fn rng(&self) -> u32 {
        self.rng
    }

    #[inline]
    fn read_byte(&mut self) -> u32 {
        if self.offs < self.storage {
            let b = self.buf[self.offs];
            self.offs += 1;
            b as u32
        } else {
            0
        }
    }

    #[inline]
    fn read_byte_from_end(&mut self) -> u32 {
        if self.end_offs < self.storage {
            self.end_offs += 1;
            self.buf[self.storage - self.end_offs] as u32
        } else {
            0
        }
    }

    #[inline]
    fn normalize(&mut self) {
        while self.rng <= CODE_BOT {
            self.nbits_total += SYM_BITS as i32;
            self.rng <<= SYM_BITS;
            let sym = self.rem;
            self.rem = self.read_byte();
            let sym = ((sym << SYM_BITS) | self.rem) >> (SYM_BITS - CODE_EXTRA);
            self.val = ((self.val << SYM_BITS).wrapping_add(SYM_MAX & !sym)) & (CODE_TOP - 1);
        }
    }

    /// Returns the cumulative frequency of the next symbol for total `ft` (call [`Self::update`] next).
    #[inline]
    pub fn decode(&mut self, ft: u32) -> u32 {
        let ext = self.rng / ft;
        let s = self.val / ext;
        // stash ext in rem-free form: recomputed by update()
        ft - (s + 1).min(ft)
    }

    /// Like [`Self::decode`] with `ft = 1 << bits`.
    #[inline]
    pub fn decode_bin(&mut self, bits: u32) -> u32 {
        let ext = self.rng >> bits;
        let s = self.val / ext;
        (1 << bits) - (s + 1).min(1 << bits)
    }

    /// Consumes the symbol with cumulative range `[fl, fh)` of total `ft`.
    #[inline]
    pub fn update(&mut self, fl: u32, fh: u32, ft: u32) {
        let ext = self.rng / ft;
        let s = ext * (ft - fh);
        self.val = self.val.wrapping_sub(s);
        self.rng = if fl > 0 { ext * (fh - fl) } else { self.rng - s };
        self.normalize();
    }

    /// Like [`Self::update`] for a symbol decoded with [`Self::decode_bin`].
    #[inline]
    pub fn update_bin(&mut self, fl: u32, fh: u32, bits: u32) {
        let ext = self.rng >> bits;
        let ft = 1u32 << bits;
        let s = ext * (ft - fh);
        self.val = self.val.wrapping_sub(s);
        self.rng = if fl > 0 { ext * (fh - fl) } else { self.rng - s };
        self.normalize();
    }

    /// Decodes a binary symbol whose probability of being 1 is `1/2^logp`.
    #[inline]
    pub fn bit_logp(&mut self, logp: u32) -> bool {
        let r = self.rng;
        let d = self.val;
        let s = r >> logp;
        let ret = d < s;
        if !ret {
            self.val = d - s;
        }
        self.rng = if ret { s } else { r - s };
        self.normalize();
        ret
    }

    /// Decodes a symbol with an "inverse CDF" table (`icdf[i] = ft - cdf[i+1]`, `ft = 1 << ftb`).
    #[inline]
    pub fn icdf(&mut self, icdf: &[u8], ftb: u32) -> usize {
        let mut s = self.rng;
        let d = self.val;
        let r = s >> ftb;
        let mut ret = 0usize;
        let mut t;
        loop {
            t = s;
            let v = icdf.get(ret).copied().unwrap_or(0) as u32;
            s = r * v;
            if d >= s || ret + 1 >= icdf.len() {
                break;
            }
            ret += 1;
        }
        // If the table is exhausted without d >= s, force the last symbol (only on corrupt tables).
        if d < s {
            s = 0;
        }
        self.val = d - s;
        self.rng = t - s;
        self.normalize();
        ret
    }

    /// Decodes a uniformly distributed integer in `[0, ft)` (`ft >= 2`).
    pub fn uint(&mut self, ft: u32) -> u32 {
        debug_assert!(ft > 1);
        let ft1 = ft - 1;
        let ftb = ilog(ft1);
        if ftb > 8 {
            let ftb = (ftb - 8) as u32;
            let top = (ft1 >> ftb) + 1;
            let s = self.decode(top);
            self.update(s, s + 1, top);
            let t = (s << ftb) | self.bits(ftb);
            if t <= ft1 {
                return t;
            }
            self.error = true;
            ft1
        } else {
            let s = self.decode(ft);
            self.update(s, s + 1, ft);
            s
        }
    }

    /// Reads `bits` raw bits (0..=25) from the end of the buffer.
    pub fn bits(&mut self, bits: u32) -> u32 {
        if bits == 0 {
            return 0;
        }
        let mut window = self.end_window;
        let mut available = self.nend_bits;
        if (available as u32) < bits {
            loop {
                window |= self.read_byte_from_end() << available;
                available += SYM_BITS as i32;
                if available > (CODE_BITS - SYM_BITS) as i32 {
                    break;
                }
            }
        }
        let ret = window & ((1u32 << bits) - 1);
        window >>= bits;
        available -= bits as i32;
        self.end_window = window;
        self.nend_bits = available;
        self.nbits_total += bits as i32;
        ret
    }

    /// Advances the bit counter so that [`Self::tell`] reports `total` (CELT silence frames).
    pub fn skip_to_end(&mut self, total: i32) {
        let t = self.tell();
        self.nbits_total += total - t;
    }

    /// Number of bits consumed so far (rounded up).
    #[inline]
    pub fn tell(&self) -> i32 {
        self.nbits_total - ilog(self.rng)
    }

    /// Number of 1/8 bits consumed so far (rounded up).
    pub fn tell_frac(&self) -> u32 {
        let nbits = (self.nbits_total as u32) << BITRES;
        let mut l = ilog(self.rng) as u32;
        let mut r = self.rng >> (l - 16);
        for _ in 0..BITRES {
            r = (r * r) >> 15;
            let b = r >> 16;
            l = (l << 1) | b;
            r >>= b;
        }
        nbits.wrapping_sub(l)
    }
}

/// Range encoder (RFC 6716 §5.1): symbols from the front of the buffer, raw bits from the back.
/// Running out of room sets [`RangeEncoder::error`] (never panics); the caller then codes the
/// frame again with less.
#[derive(Clone, Debug)]
pub struct RangeEncoder {
    out: Vec<u8>,
    end: Vec<u8>,
    end_window: u32,
    nend_bits: i32,
    nbits_total: i32,
    rng: u32,
    low: u32,
    rem: i32,
    ext: u32,
    size: usize,
    /// Set when the coded data no longer fits in `size` bytes.
    pub error: bool,
}

impl RangeEncoder {
    /// An encoder for a buffer of `size` bytes.
    pub fn new(size: usize) -> Self {
        RangeEncoder {
            out: Vec::with_capacity(size),
            end: Vec::new(),
            end_window: 0,
            nend_bits: 0,
            nbits_total: CODE_BITS as i32 + 1,
            rng: CODE_TOP,
            low: 0,
            rem: -1,
            ext: 0,
            size,
            error: false,
        }
    }

    /// The buffer size in bytes.
    pub fn storage(&self) -> usize {
        self.size
    }

    /// The current range (the final range once [`Self::done`] ran).
    pub fn rng(&self) -> u32 {
        self.rng
    }

    fn carry_out(&mut self, c: u32) {
        if c != SYM_MAX {
            let carry = c >> SYM_BITS;
            if self.rem >= 0 {
                self.push((self.rem as u32).wrapping_add(carry) as u8);
            }
            if self.ext > 0 {
                let sym = (SYM_MAX + carry) & SYM_MAX;
                for _ in 0..self.ext {
                    self.push(sym as u8);
                }
                self.ext = 0;
            }
            self.rem = (c & SYM_MAX) as i32;
        } else {
            self.ext += 1;
        }
    }

    fn push(&mut self, b: u8) {
        if self.out.len() + self.end.len() < self.size {
            self.out.push(b);
        } else {
            self.error = true;
        }
    }

    fn normalize(&mut self) {
        while self.rng <= CODE_BOT {
            self.carry_out(self.low >> CODE_SHIFT);
            self.low = (self.low << SYM_BITS) & (CODE_TOP - 1);
            self.rng <<= SYM_BITS;
            self.nbits_total += SYM_BITS as i32;
        }
    }

    /// Codes the symbol with cumulative range `[fl, fh)` of total `ft`.
    pub fn encode(&mut self, fl: u32, fh: u32, ft: u32) {
        let ft = ft.max(1);
        let r = self.rng / ft;
        if fl > 0 {
            self.low = self.low.wrapping_add(self.rng.wrapping_sub(r.wrapping_mul(ft - fl)));
            self.rng = r.wrapping_mul(fh.wrapping_sub(fl));
        } else {
            self.rng = self.rng.wrapping_sub(r.wrapping_mul(ft.wrapping_sub(fh)));
        }
        self.normalize();
    }

    /// Like [`Self::encode`] with `ft = 1 << bits`.
    pub fn encode_bin(&mut self, fl: u32, fh: u32, bits: u32) {
        self.encode(fl, fh, 1 << bits);
    }

    /// Codes a binary symbol whose probability of being 1 is `1/2^logp`.
    pub fn bit_logp(&mut self, val: bool, logp: u32) {
        let r = self.rng;
        let s = r >> logp;
        let r2 = r - s;
        if val {
            self.low = self.low.wrapping_add(r2);
            self.rng = s;
        } else {
            self.rng = r2;
        }
        self.normalize();
    }

    /// Codes symbol `s` with an "inverse CDF" table (`ft = 1 << ftb`).
    pub fn icdf(&mut self, s: usize, icdf: &[u8], ftb: u32) {
        let r = self.rng >> ftb;
        let at = |i: usize| icdf.get(i).copied().unwrap_or(0) as u32;
        if s > 0 {
            self.low = self.low.wrapping_add(self.rng.wrapping_sub(r.wrapping_mul(at(s - 1))));
            self.rng = r.wrapping_mul(at(s - 1).wrapping_sub(at(s)));
        } else {
            self.rng = self.rng.wrapping_sub(r.wrapping_mul(at(s)));
        }
        self.normalize();
    }

    /// Codes `fl` uniformly distributed in `[0, ft)` (`ft >= 2`).
    pub fn uint(&mut self, fl: u32, ft: u32) {
        let ft1 = ft.saturating_sub(1);
        let fl = fl.min(ft1);
        let ftb = ilog(ft1);
        if ftb > 8 {
            let ftb = (ftb - 8) as u32;
            let top = (ft1 >> ftb) + 1;
            let v = fl >> ftb;
            self.encode(v, v + 1, top);
            self.bits(fl & ((1 << ftb) - 1), ftb);
        } else {
            self.encode(fl, fl + 1, ft.max(1));
        }
    }

    /// Writes `bits` raw bits (0..=25) at the end of the buffer.
    pub fn bits(&mut self, val: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        let mut window = self.end_window;
        let mut used = self.nend_bits;
        if used as u32 + bits > CODE_BITS {
            while used >= SYM_BITS as i32 {
                if self.out.len() + self.end.len() < self.size {
                    self.end.push((window & SYM_MAX) as u8);
                } else {
                    self.error = true;
                }
                window >>= SYM_BITS;
                used -= SYM_BITS as i32;
            }
        }
        window |= (val & ((1u32 << bits) - 1)) << used;
        used += bits as i32;
        self.end_window = window;
        self.nend_bits = used;
        self.nbits_total += bits as i32;
    }

    /// Pretends the frame's remaining bits were used (CELT silence frames).
    pub fn skip_to_end(&mut self, total: i32) {
        let t = self.tell();
        self.nbits_total += total - t;
    }

    /// Number of bits written so far (rounded up).
    pub fn tell(&self) -> i32 {
        self.nbits_total - ilog(self.rng)
    }

    /// Number of 1/8 bits written so far (rounded up).
    pub fn tell_frac(&self) -> u32 {
        let nbits = (self.nbits_total as u32) << BITRES;
        let mut l = ilog(self.rng) as u32;
        let mut r = self.rng >> (l.saturating_sub(16));
        for _ in 0..BITRES {
            r = (r * r) >> 15;
            let b = r >> 16;
            l = (l << 1) | b;
            r >>= b;
        }
        nbits.wrapping_sub(l)
    }

    /// Finalises the stream: a buffer of exactly `size` bytes (zero-padded in the middle).
    /// [`Self::error`] tells whether everything fitted.
    pub fn done(mut self) -> Vec<u8> {
        let mut l = CODE_BITS as i32 - ilog(self.rng);
        let mut msk = (CODE_TOP - 1) >> l;
        let mut end = self.low.wrapping_add(msk) & !msk;
        if (end | msk) >= self.low.wrapping_add(self.rng) {
            l += 1;
            msk >>= 1;
            end = self.low.wrapping_add(msk) & !msk;
        }
        while l > 0 {
            self.carry_out(end >> CODE_SHIFT);
            end = (end << SYM_BITS) & (CODE_TOP - 1);
            l -= SYM_BITS as i32;
        }
        if self.rem >= 0 || self.ext > 0 {
            self.carry_out(0);
        }
        let mut window = self.end_window;
        let mut used = self.nend_bits;
        while used >= SYM_BITS as i32 {
            if self.out.len() + self.end.len() < self.size {
                self.end.push((window & SYM_MAX) as u8);
            } else {
                self.error = true;
            }
            window >>= SYM_BITS;
            used -= SYM_BITS as i32;
        }
        let full = self.out.len() + self.end.len() >= self.size;
        let mut buf = self.out;
        buf.resize(self.size.saturating_sub(self.end.len()), 0);
        for &b in self.end.iter().rev() {
            buf.push(b);
        }
        // Leftover raw bits share the byte just before the raw-bit region: padding, or the last
        // range-coder byte, whose low `-l` bits are free.
        if !self.error && used > 0 {
            match self.size.checked_sub(self.end.len() + 1).and_then(|i| buf.get_mut(i)) {
                None => self.error = true,
                Some(byte) => {
                    let free = -l;
                    if full && free < used {
                        window &= (1u32 << free.clamp(0, 31)) - 1;
                        self.error = true;
                    }
                    *byte |= window as u8;
                }
            }
        }
        buf
    }
}

/// What the CELT layer codes symbols through: the decoder reads them, the encoder writes the
/// values it is given. The value arguments are ignored when decoding; the return value is the
/// symbol (decoded, or the one written).
pub(crate) trait Coder {
    const ENCODE: bool;
    fn tell(&self) -> i32;
    fn tell_frac(&self) -> u32;
    fn storage(&self) -> usize;
    fn bit_logp(&mut self, val: bool, logp: u32) -> bool;
    fn uint(&mut self, val: u32, ft: u32) -> u32;
    fn bits(&mut self, val: u32, bits: u32) -> u32;
    /// Decoding: the cumulative frequency of the next symbol. Encoding: 0.
    fn decode(&mut self, ft: u32) -> u32;
    /// Decoding: consumes the symbol `[fl, fh)`. Encoding: writes it.
    fn update(&mut self, fl: u32, fh: u32, ft: u32);
}

impl Coder for RangeDecoder<'_> {
    const ENCODE: bool = false;
    #[inline]
    fn tell(&self) -> i32 {
        RangeDecoder::tell(self)
    }
    #[inline]
    fn tell_frac(&self) -> u32 {
        RangeDecoder::tell_frac(self)
    }
    #[inline]
    fn storage(&self) -> usize {
        RangeDecoder::storage(self)
    }
    #[inline]
    fn bit_logp(&mut self, _: bool, logp: u32) -> bool {
        RangeDecoder::bit_logp(self, logp)
    }
    #[inline]
    fn uint(&mut self, _: u32, ft: u32) -> u32 {
        RangeDecoder::uint(self, ft)
    }
    #[inline]
    fn bits(&mut self, _: u32, bits: u32) -> u32 {
        RangeDecoder::bits(self, bits)
    }
    #[inline]
    fn decode(&mut self, ft: u32) -> u32 {
        RangeDecoder::decode(self, ft)
    }
    #[inline]
    fn update(&mut self, fl: u32, fh: u32, ft: u32) {
        RangeDecoder::update(self, fl, fh, ft)
    }
}

impl Coder for RangeEncoder {
    const ENCODE: bool = true;
    fn tell(&self) -> i32 {
        RangeEncoder::tell(self)
    }
    fn tell_frac(&self) -> u32 {
        RangeEncoder::tell_frac(self)
    }
    fn storage(&self) -> usize {
        self.size
    }
    fn bit_logp(&mut self, val: bool, logp: u32) -> bool {
        RangeEncoder::bit_logp(self, val, logp);
        val
    }
    fn uint(&mut self, val: u32, ft: u32) -> u32 {
        let v = val.min(ft.saturating_sub(1));
        RangeEncoder::uint(self, v, ft);
        v
    }
    fn bits(&mut self, val: u32, bits: u32) -> u32 {
        let v = if bits >= 32 { val } else { val & ((1u32 << bits) - 1) };
        RangeEncoder::bits(self, v, bits);
        v
    }
    fn decode(&mut self, _: u32) -> u32 {
        0
    }
    fn update(&mut self, fl: u32, fh: u32, ft: u32) {
        self.encode(fl, fh, ft)
    }
}

#[cfg(test)]
pub(crate) mod enc {
    pub use super::RangeEncoder;
}

#[cfg(test)]
mod tests {
    use super::enc::RangeEncoder;
    use super::*;

    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            self.0 >> 8
        }
    }

    #[test]
    fn roundtrip_mixed_symbols() {
        let mut rng = Lcg(1);
        for trial in 0..200 {
            let n = 1 + (rng.next() % 300) as usize;
            let mut ops = Vec::new();
            for _ in 0..n {
                match rng.next() % 5 {
                    0 => {
                        let ft = 2 + rng.next() % 1000;
                        let v = rng.next() % ft;
                        ops.push((0u8, v, ft));
                    }
                    1 => {
                        let logp = 1 + rng.next() % 15;
                        ops.push((1, rng.next().is_multiple_of(7) as u32, logp));
                    }
                    2 => {
                        let bits = 1 + rng.next() % 24;
                        ops.push((2, rng.next() & ((1 << bits) - 1), bits));
                    }
                    3 => {
                        let ft = 2 + rng.next() % 100_000_000;
                        ops.push((3, rng.next() % ft, ft));
                    }
                    _ => ops.push((4, rng.next() % 4, 0)),
                }
            }
            let icdf: [u8; 4] = [200, 120, 30, 0];
            let mut e = RangeEncoder::new(8 * n + 16);
            for &(k, v, p) in &ops {
                match k {
                    0 => e.encode(v, v + 1, p),
                    1 => e.bit_logp(v != 0, p),
                    2 => e.bits(v, p),
                    3 => e.uint(v, p),
                    _ => e.icdf(v as usize, &icdf, 8),
                }
            }
            let buf = e.done();
            let mut d = RangeDecoder::new(&buf);
            for (i, &(k, v, p)) in ops.iter().enumerate() {
                let got = match k {
                    0 => {
                        let s = d.decode(p);
                        d.update(s, s + 1, p);
                        s
                    }
                    1 => d.bit_logp(p) as u32,
                    2 => d.bits(p),
                    3 => d.uint(p),
                    _ => d.icdf(&icdf, 8) as u32,
                };
                assert_eq!(got, v, "trial {trial} op {i} kind {k}");
            }
            assert!(!d.error);
        }
    }

    #[test]
    fn tell_starts_at_one_bit() {
        let buf = [0u8; 8];
        let d = RangeDecoder::new(&buf);
        assert_eq!(d.tell(), 1);
        assert_eq!(d.tell_frac(), 8);
    }

    #[test]
    fn empty_and_garbage_never_panic() {
        let mut d = RangeDecoder::new(&[]);
        for i in 0..100 {
            d.bits(1 + i % 20);
            d.uint(3 + i * 77);
            d.icdf(&[250, 100, 0], 8);
            d.bit_logp(1 + i % 15);
        }
        let _ = d.tell_frac();
    }
}
