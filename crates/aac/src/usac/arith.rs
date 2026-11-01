//! Context-based arithmetic coding of spectral coefficients (ISO/IEC 23003-3 §7.4, "spectral
//! noiseless coding"), used by the FD core and by TCX.
//!
//! Ported from libxaac `ixheaacd_arith_dec.c` (Apache-2.0, see `super` for the notice).

use cfa4_bitstream::BitReader;

use super::arith_tables::{ARI_CF_M, ARI_CF_R, ARI_HASH_M, ARI_HASH_M_PKI, ARI_LOOKUP_M};
use crate::{Error, Result};

const ARITH_ESCAPE: usize = 16;
const ESC_NB_OFFSET: [u32; 8] = [0, 131072, 262144, 393216, 524288, 655360, 786432, 917504];
/// Context entries: 2 guard entries + up to 512 2-tuples + 2.
const CTX_LEN: usize = 1024 / 2 + 4;
/// Largest magnitude a decoded coefficient may have.
const MAX_QUANT: i32 = 8191;

/// Per-channel context of the spectral noiseless decoder (`q[0]`/`q[1]` of the standard).
#[derive(Clone)]
pub struct ArithContext {
    prev: [i8; CTX_LEN],
    pres: [i8; CTX_LEN],
    /// Number of spectral lines of the previous transform (`previous_lg`).
    prev_n: usize,
}

impl Default for ArithContext {
    fn default() -> Self {
        ArithContext { prev: [0; CTX_LEN], pres: [0; CTX_LEN], prev_n: 0 }
    }
}

impl ArithContext {
    /// `arith_map_context()`: reset, or resample the context to a transform of `n` lines.
    pub fn map(&mut self, n: usize, reset: bool) {
        if reset {
            self.prev = [0; CTX_LEN];
            self.pres = [0; CTX_LEN];
        } else if self.prev_n > 0 {
            let (prev_tmp, pres_tmp) = (self.prev, self.pres);
            let pn = self.prev_n.min(1024);
            let ratio = pn as f32 / n as f32;
            for i in 0..n / 2 {
                let k = ((i as f32 * ratio) as usize).min(CTX_LEN - 3);
                self.prev[2 + i] = prev_tmp[2 + k];
                self.pres[2 + i] = pres_tmp[2 + k];
            }
            if let (Some(a), Some(&b)) = (self.prev.get_mut(n / 2 + 2), prev_tmp.get(pn / 2 + 2)) {
                *a = b;
            }
            if let (Some(a), Some(&b)) = (self.pres.get_mut(n / 2 + 3), pres_tmp.get(pn / 2 + 3)) {
                *a = b;
            }
        }
        self.prev_n = n;
    }

    /// The context of a transform whose coefficients are all zero (`max_spec_coefficients == 0`).
    pub fn set_all_zero(&mut self, n: usize) {
        for v in self.pres.iter_mut().skip(2).take(n / 2) {
            *v = 1;
        }
    }
}

/// Bit source of the arithmetic decoder: reads ahead of the frame's `BitReader` and yields zeros
/// past the end of the data.
struct Lookahead<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Lookahead<'_> {
    fn bit(&mut self) -> u32 {
        let b = self.data.get(self.pos / 8).map_or(0, |&v| (v >> (7 - self.pos % 8)) & 1);
        self.pos += 1;
        b as u32
    }
}

struct State {
    low: i32,
    high: i32,
    value: i32,
}

fn get_pk(c: u32) -> usize {
    let (mut i_min, mut i_max): (isize, isize) = (-1, ARI_HASH_M.len() as isize - 1);
    while i_max - i_min > 1 {
        let i = i_min + ((i_max - i_min) >> 1);
        let j = ARI_HASH_M[i as usize];
        if c < j {
            i_max = i;
        } else if c > j {
            i_min = i;
        } else {
            return ARI_HASH_M_PKI[i as usize] as usize;
        }
    }
    ARI_LOOKUP_M[i_max as usize] as usize
}

fn decode_symbol(src: &mut Lookahead, bit_count: &mut usize, s: &mut State, cum_freq: &[u16]) -> usize {
    let range = s.high - s.low + 1;
    let cumulative = (((s.value - s.low + 1) << 14) - 1) / range;
    // binary search for the first symbol whose cumulative frequency is <= `cumulative`
    let mut p: isize = -1;
    let mut cfl = cum_freq.len();
    loop {
        let q = p + (cfl >> 1) as isize;
        if q >= 0 && (cum_freq.get(q as usize).copied().unwrap_or(0) as i32) > cumulative {
            p = q;
            cfl += 1;
        }
        cfl >>= 1;
        if cfl <= 1 {
            break;
        }
    }
    let symbol = (p + 1) as usize;
    if symbol > 0 {
        s.high = s.low + ((range * cum_freq.get(symbol - 1).copied().unwrap_or(0) as i32) >> 14) - 1;
    }
    s.low += (range * cum_freq.get(symbol).copied().unwrap_or(0) as i32) >> 14;
    loop {
        if s.high < 32768 {
        } else if s.low >= 32768 {
            s.value -= 32768;
            s.low -= 32768;
            s.high -= 32768;
        } else if s.low >= 16384 && s.high < 49152 {
            s.value -= 16384;
            s.low -= 16384;
            s.high -= 16384;
        } else {
            break;
        }
        s.low += s.low;
        s.high = 2 * s.high + 1;
        s.value = (s.value << 1) | src.bit() as i32;
        *bit_count += 1;
    }
    symbol
}

/// Decode one transform of `n_lines` spectral lines (`n_coded` of them coded, the rest zero) into
/// `quant` and update `ctx`. `ctx.map()` must have been called for this transform size.
pub fn decode_spectrum(br: &mut BitReader, ctx: &mut ArithContext, n_coded: usize, n_lines: usize, quant: &mut [i32]) -> Result<()> {
    let n = n_coded / 2;
    let pres_n = n_lines / 2;
    if pres_n + 2 > CTX_LEN || quant.len() < n_lines || n > pres_n {
        return Err(Error::Bitstream("arithmetic coder: transform too long"));
    }
    quant[..n_lines].fill(0);
    for i in 0..pres_n {
        ctx.prev[2 + i] = ctx.pres[2 + i];
        ctx.pres[2 + i] = 1;
    }
    let mut src = Lookahead { data: br.data(), pos: br.position() };
    let mut value = 0i32;
    for _ in 0..16 {
        value = (value << 1) | src.bit() as i32;
    }
    let mut s = State { low: 0, high: 65535, value };
    let mut bit_count = 16usize;

    let mut c: i32 = (ctx.prev[2] as i32) << 12;
    for i in 0..n {
        // arith_get_context()
        c = (c >> 4) + ((ctx.prev[2 + i + 1] as i32) << 12);
        c = (c & 0xFFF0) + ctx.pres[2 + i - 1] as i32;
        let mut s1 = c;
        if i > 3 && (ctx.pres[2 + i - 1] as i32 + ctx.pres[2 + i - 2] as i32 + ctx.pres[2 + i - 3] as i32) < 5 {
            s1 += 0x10000;
        }
        let mut lev = 0usize;
        let mut esc_nb = 0usize;
        let mut m;
        loop {
            let pki = get_pk(s1 as u32 + ESC_NB_OFFSET[esc_nb]);
            let cf = ARI_CF_M.get(pki).ok_or(Error::Bitstream("arithmetic coder: bad context"))?;
            m = decode_symbol(&mut src, &mut bit_count, &mut s, cf);
            if m < ARITH_ESCAPE {
                break;
            }
            lev += 1;
            if lev > 23 {
                return Err(Error::Bitstream("arithmetic coder: too many escapes"));
            }
            esc_nb = lev.min(7);
        }
        if m == 0 {
            if esc_nb > 0 {
                break; // ARITH_STOP: the remaining 2-tuples are zero
            }
            ctx.pres[2 + i] = 1;
            continue;
        }
        let mut b = (m >> 2) as i32;
        let mut a = (m & 3) as i32;
        for _ in 0..lev {
            let lsbidx = if a == 0 {
                1
            } else if b == 0 {
                0
            } else {
                2
            };
            let r = decode_symbol(&mut src, &mut bit_count, &mut s, &ARI_CF_R[lsbidx]) as i32;
            a = (a << 1) | (r & 1);
            b = (b << 1) | ((r >> 1) & 1);
        }
        if a > MAX_QUANT || b > MAX_QUANT {
            return Err(Error::Bitstream("arithmetic coder: coefficient out of range"));
        }
        quant[2 * i] = a;
        quant[2 * i + 1] = b;
        ctx.pres[2 + i] = (a + b + 1).min(0xF) as i8;
    }
    // the arithmetic decoder reads 14 bits beyond the end of its code word
    let consumed = bit_count.saturating_sub(14);
    br.skip(consumed)?;
    for v in quant.iter_mut().take(2 * pres_n) {
        // sign bit: 1 = positive
        if *v != 0 && br.read_bits(1)? == 0 {
            *v = -*v;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_bitstream::BitWriter;

    /// Encoder side of the arithmetic coder (after libxaac `iusace_arith_enc.c`), for round trips.
    struct Enc {
        low: i32,
        high: i32,
        follow: i32,
    }

    impl Enc {
        fn put(bw: &mut BitWriter, bit: u32, follow: &mut i32) {
            bw.write_bits(bit, 1);
            while *follow > 0 {
                bw.write_bits(1 - bit, 1);
                *follow -= 1;
            }
        }
        fn encode(&mut self, bw: &mut BitWriter, symbol: usize, cf: &[u16]) {
            let range = self.high - self.low + 1;
            if symbol > 0 {
                self.high = self.low + ((range * cf[symbol - 1] as i32) >> 14) - 1;
            }
            self.low += (range * cf[symbol] as i32) >> 14;
            loop {
                if self.high < 32768 {
                    Self::put(bw, 0, &mut self.follow);
                } else if self.low >= 32768 {
                    Self::put(bw, 1, &mut self.follow);
                    self.low -= 32768;
                    self.high -= 32768;
                } else if self.low >= 16384 && self.high < 49152 {
                    self.follow += 1;
                    self.low -= 16384;
                    self.high -= 16384;
                } else {
                    break;
                }
                self.low += self.low;
                self.high = 2 * self.high + 1;
            }
        }
        fn done(&mut self, bw: &mut BitWriter) {
            self.follow += 1;
            if self.low < 16384 {
                Self::put(bw, 0, &mut self.follow);
            } else {
                Self::put(bw, 1, &mut self.follow);
            }
        }
    }

    fn encode(bw: &mut BitWriter, prev: &mut [i32; CTX_LEN], pres: &mut [i32; CTX_LEN], quant: &[i32], n: usize, nt: usize) {
        let mut e = Enc { low: 0, high: 65535, follow: 0 };
        let mut c: i32 = prev[2] << 12;
        let mut stop_at = None;
        if let Some(last) = (0..n).rev().find(|&j| quant[2 * j] != 0 || quant[2 * j + 1] != 0) {
            if last + 1 < n {
                stop_at = Some(last + 1);
            }
        } else {
            stop_at = Some(0);
        }
        let mut i = 0;
        while i < n {
            c = (c >> 4) + (prev[2 + i + 1] << 12);
            c = (c & 0xFFF0) + pres[2 + i - 1];
            let mut t = c;
            if i > 3 && pres[2 + i - 1] + pres[2 + i - 2] + pres[2 + i - 3] < 5 {
                t += 0x10000;
            }
            if stop_at == Some(i) {
                e.encode(bw, ARITH_ESCAPE, &ARI_CF_M[get_pk(t as u32)]);
                e.encode(bw, 0, &ARI_CF_M[get_pk(t as u32 + (1 << 17))]);
                break;
            }
            let (mut a1, mut b1) = (quant[2 * i].abs(), quant[2 * i + 1].abs());
            pres[2 + i] = (a1 + b1 + 1).min(15);
            let mut qs = Vec::new();
            let mut esc_nb = 0u32;
            while a1 > 3 || b1 > 3 {
                e.encode(bw, ARITH_ESCAPE, &ARI_CF_M[get_pk(t as u32 + (esc_nb << 17))]);
                qs.push((a1 & 1) | ((b1 & 1) << 1));
                a1 >>= 1;
                b1 >>= 1;
                esc_nb = (esc_nb + 1).min(7);
            }
            e.encode(bw, (a1 + (b1 << 2)) as usize, &ARI_CF_M[get_pk(t as u32 + (esc_nb << 17))]);
            let (mut a2, mut b2) = (a1, b1);
            for &q in qs.iter().rev() {
                let lsbidx = if a2 == 0 {
                    1
                } else if b2 == 0 {
                    0
                } else {
                    2
                };
                e.encode(bw, q as usize, &ARI_CF_R[lsbidx]);
                a2 = (a2 << 1) | (q & 1);
                b2 = (b2 << 1) | ((q >> 1) & 1);
            }
            i += 1;
        }
        e.done(bw);
        for v in pres.iter_mut().skip(2 + i.min(nt)).take(nt.saturating_sub(i)) {
            *v = 1;
        }
        for &q in &quant[..2 * n] {
            if q != 0 {
                bw.write_bits((q > 0) as u32, 1);
            }
        }
        for k in 0..nt {
            prev[2 + k] = pres[2 + k];
            pres[2 + k] = 1;
        }
    }

    #[test]
    fn round_trip() {
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as i32
        };
        let (mut eprev, mut epres) = ([0i32; CTX_LEN], [0i32; CTX_LEN]);
        let mut ctx = ArithContext::default();
        ctx.map(1024, true);
        for frame in 0..6 {
            let n_coded = [1024, 800, 512, 0, 960, 1024][frame];
            let quant: Vec<i32> = (0..1024)
                .map(|k| {
                    if k >= n_coded || k > 700 {
                        return 0;
                    }
                    let r = rnd() % 100;
                    let mag = if r < 50 {
                        0
                    } else if r < 85 {
                        1 + rnd() % 3
                    } else if r < 98 {
                        4 + rnd() % 30
                    } else {
                        rnd() % 3000
                    };
                    if rnd() % 2 == 0 { mag } else { -mag }
                })
                .collect();
            let mut bw = BitWriter::new();
            if n_coded > 0 {
                encode(&mut bw, &mut eprev, &mut epres, &quant, n_coded / 2, 512);
            }
            let bits = bw.bit_len();
            bw.write_bits(0b1011_0110, 8);
            let data = bw.finish();
            let mut br = BitReader::new(&data);
            let mut out = vec![0i32; 1024];
            if n_coded > 0 {
                decode_spectrum(&mut br, &mut ctx, n_coded, 1024, &mut out).unwrap();
            } else {
                ctx.set_all_zero(1024);
                for k in 0..512 {
                    eprev[2 + k] = epres[2 + k];
                    epres[2 + k] = 1;
                }
            }
            assert_eq!(out, quant, "frame {frame}");
            assert_eq!(br.position(), bits, "frame {frame}");
            ctx.map(1024, false);
        }
    }
}
