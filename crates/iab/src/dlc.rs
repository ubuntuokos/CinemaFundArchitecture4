//! AudioDataDLC: sample-rate scalable lossless coding (Annex B, 9.7, 10.8).
//!
//! A 48 kHz base layer and, at 96 kHz, an extension layer over the interpolated base layer; each
//! layer is up to three regions of forward-adaptive linear prediction (lattice coefficients
//! converted to direct form, Q20) over sub-blocks of Rice / Golomb or sign-magnitude PCM
//! residuals. The arithmetic is the normative 32 / 64-bit integer arithmetic of Annex B, wrapping
//! where hostile values would overflow.

use cfa4_bitstream::{BitReader, BitWriter};

use crate::frame::Ctx;
use crate::{Error, Result};

/// Half-band interpolation filter taps of odd index 1, 3, …, 31 (Table B.1; even taps are zero
/// except the centre, which the filter applies as a plain delay).
const INTERP: [i64; 16] = [-138, 305, -618, 1128, -1952, 3377, -6450, 20688, 20688, -6450, 3377, -1952, 1128, -618, 305, -138];
/// Q20 one.
const ONE: i32 = 1 << 20;

/// One predictor region: sub-blocks covered and direct-form coefficients a[1..=order] (Q20).
struct Region {
    blocks: usize,
    a: Vec<i32>,
}

/// Lattice (reflection) codes → direct-form coefficients (B.7).
fn direct_form(k_codes: &[u16]) -> Vec<i32> {
    let order = k_codes.len();
    let mut a = vec![0i32; order + 1];
    let mut tmp = vec![0i32; order + 1];
    a[0] = ONE;
    for j in 1..=order {
        let k = (i32::from(k_codes[j - 1]) - 512) << 11;
        a[j] = 0;
        for i in 1..=j {
            let acc = i64::from(k) * i64::from(a[j - i]);
            tmp[i] = a[i].wrapping_add((acc >> 20) as i32);
        }
        a[1..=j].copy_from_slice(&tmp[1..=j]);
    }
    a
}

fn read_regions(r: &mut BitReader, blocks: usize) -> Result<Vec<Region>> {
    let n = r.read_bits(2)?;
    let mut regions = Vec::new();
    let mut total = 0;
    for _ in 0..n {
        let len = r.read_bits(4)? as usize;
        let order = r.read_bits(5)? as usize;
        let mut k = Vec::with_capacity(order);
        for _ in 0..order {
            k.push(r.read_bits(10)? as u16);
        }
        total += len;
        regions.push(Region { blocks: len, a: direct_form(&k) });
    }
    if n > 0 && total != blocks {
        return Err(Error::Invalid("DLC predictor regions do not cover the frame"));
    }
    Ok(regions)
}

/// The residuals of `blocks` sub-blocks of `size` samples (10.8.11-10.8.16).
fn read_residuals(r: &mut BitReader, blocks: usize, size: usize) -> Result<Vec<i32>> {
    let mut out = Vec::with_capacity(blocks * size);
    for _ in 0..blocks {
        if r.read_bit()? {
            let rem = r.read_bits(5)?;
            for _ in 0..size {
                let mut q: u64 = 0;
                while r.read_bit()? {
                    q += 1;
                    if q << rem > i32::MAX as u64 {
                        return Err(Error::Invalid("DLC Rice quotient overflows"));
                    }
                }
                let v = (q << rem) + u64::from(r.read_bits(rem)?);
                out.push(signed(r, v)?);
            }
        } else {
            let depth = r.read_bits(5)?;
            for _ in 0..size {
                let v = u64::from(r.read_bits(depth)?);
                out.push(signed(r, v)?);
            }
        }
    }
    Ok(out)
}

/// A magnitude and, when non-zero, its sign bit.
fn signed(r: &mut BitReader, v: u64) -> Result<i32> {
    let m = i32::try_from(v).map_err(|_| Error::Invalid("DLC residual overflows"))?;
    Ok(if m != 0 && r.read_bit()? { -m } else { m })
}

/// Inverse prediction (B.8).
fn predict(regions: &[Region], residual: &[i32], size: usize) -> Vec<i32> {
    if regions.is_empty() {
        return residual.to_vec();
    }
    let mut buf = [0i32; 64];
    let mut i1 = 0usize;
    let mut out = Vec::with_capacity(residual.len());
    let mut n = 0;
    for reg in regions {
        for _ in 0..reg.blocks * size {
            let mut i2 = i1;
            let mut acc: i64 = 0;
            for &a in reg.a.iter().skip(1) {
                acc = acc.wrapping_sub(i64::from(buf[i2]).wrapping_mul(i64::from(a)));
                i2 = i2.wrapping_sub(1) & 63;
            }
            let v = residual.get(n).copied().unwrap_or(0).wrapping_add((acc >> 20) as i32);
            i1 = (i1 + 1) & 63;
            buf[i1] = v;
            out.push(v);
            n += 1;
        }
    }
    out
}

/// The 48 kHz base layer upsampled to 96 kHz (B.9): even outputs are the base delayed by eight
/// samples, odd outputs the half-band interpolation between them.
pub(crate) fn upsample(base: &[i32], n96: usize) -> Vec<i32> {
    let mut buf = [0i32; 64];
    let mut i1 = 0usize;
    let mut out = vec![0i32; n96];
    let mut k = 0;
    let mut n = 0;
    while n < n96 {
        buf[i1] = base.get(k).copied().unwrap_or(0);
        out[n] = buf[i1.wrapping_sub(8) & 63];
        let mut i2 = i1;
        let mut acc: i64 = 0;
        for c in INTERP {
            acc = acc.wrapping_add(i64::from(buf[i2]) * c);
            i2 = i2.wrapping_sub(1) & 63;
        }
        if let Some(o) = out.get_mut(n + 1) {
            *o = (acc >> 15) as i32;
        }
        i1 = (i1 + 1) & 63;
        n += 2;
        k += 1;
    }
    out
}

/// Decodes the AudioDataDLC fields after AudioDataID and DLCSize into left-justified 32-bit
/// samples at the frame's rate.
pub(crate) fn decode(r: &mut BitReader, ctx: &Ctx) -> Result<Vec<i32>> {
    let (blocks, size48) = ctx.rate.dlc_blocks().ok_or(Error::Invalid("AudioDataDLC at a 1001 frame rate"))?;
    let rate96 = r.read_bits(2)? == 1;
    if rate96 != ctx.rate96 {
        return Err(Error::Invalid("DLCSampleRate differs from the IAFrame SampleRate"));
    }
    let shift = r.read_bits(5)?;
    let regions = read_regions(r, blocks)?;
    let residual = read_residuals(r, blocks, size48)?;
    let base = predict(&regions, &residual, size48);
    let mut out = if rate96 {
        let size96 = size48 * 2;
        let regions = read_regions(r, blocks)?;
        let residual = read_residuals(r, blocks, size96)?;
        let mut ext = predict(&regions, &residual, size96);
        for (e, u) in ext.iter_mut().zip(upsample(&base, blocks * size96)) {
            *e = e.wrapping_add(u);
        }
        ext
    } else {
        base
    };
    for v in &mut out {
        *v = v.wrapping_shl(shift);
    }
    Ok(out)
}

// ---- encoder ----

/// Highest predictor order the encoder tries.
const MAX_ORDER: usize = 16;

/// Bits to code `r` with Rice parameter `k`.
fn rice_bits(r: &[i32], k: u32) -> u64 {
    r.iter().map(|&v| (u64::from(v.unsigned_abs()) >> k) + 1 + u64::from(k) + u64::from(v != 0)).sum()
}

/// Bits of the magnitude of the largest residual.
fn depth_of(r: &[i32]) -> u32 {
    let m = r.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
    32 - m.leading_zeros()
}

fn write_residuals(w: &mut BitWriter, residual: &[i32], blocks: usize, size: usize) {
    for b in 0..blocks {
        let r = residual.get(b * size..(b + 1) * size).unwrap_or_default();
        let depth = depth_of(r);
        let pcm_bits = 5 + r.len() as u64 * u64::from(depth) + r.iter().filter(|&&v| v != 0).count() as u64;
        let (k, rice) = (0..31u32).map(|k| (k, 5 + rice_bits(r, k))).min_by_key(|&(_, b)| b).unwrap_or((0, u64::MAX));
        if rice < pcm_bits {
            w.write_bit(true);
            w.write_bits(k, 5);
            for &v in r {
                let m = v.unsigned_abs();
                for _ in 0..m >> k {
                    w.write_bit(true);
                }
                w.write_bit(false);
                w.write_bits(m & ((1u32 << k) - 1), k);
                if v != 0 {
                    w.write_bit(v < 0);
                }
            }
        } else {
            w.write_bit(false);
            w.write_bits(depth, 5);
            for &v in r {
                w.write_bits(v.unsigned_abs(), depth);
                if v != 0 {
                    w.write_bit(v < 0);
                }
            }
        }
    }
}

/// Reflection coefficients of `x` by Levinson-Durbin, quantised to 10-bit codes.
fn lattice_codes(x: &[i32], order: usize) -> Vec<u16> {
    let xf: Vec<f64> = x.iter().map(|&v| f64::from(v)).collect();
    let mut ac = vec![0f64; order + 1];
    for (lag, c) in ac.iter_mut().enumerate() {
        *c = xf.iter().zip(xf.iter().skip(lag)).map(|(a, b)| a * b).sum();
    }
    if ac[0] <= 0.0 {
        return vec![512; order];
    }
    ac[0] *= 1.0 + 1e-9;
    let mut a = vec![0f64; order + 1];
    let mut err = ac[0];
    let mut codes = Vec::with_capacity(order);
    for i in 1..=order {
        let acc: f64 = (1..i).map(|j| a[j] * ac[i - j]).sum();
        // e[n] = x[n] + sum a[j] x[n - j], the decoder's convention; k is (code - 512) / 512
        let k = if err > 0.0 { -(ac[i] + acc) / err } else { 0.0 };
        let code = (k * 512.0).round().clamp(-511.0, 511.0) as i32 + 512;
        let kq = f64::from(code - 512) / 512.0;
        codes.push(code as u16);
        let prev = a.clone();
        for j in 1..i {
            a[j] = prev[j] + kq * prev[i - j];
        }
        a[i] = kq;
        err *= 1.0 - kq * kq;
    }
    codes
}

/// Forward prediction: the residual whose inverse prediction gives back `x` exactly.
fn residual_of(x: &[i32], a: &[i32]) -> Vec<i32> {
    let mut buf = [0i32; 64];
    let mut i1 = 0usize;
    let mut out = Vec::with_capacity(x.len());
    for &v in x {
        let mut i2 = i1;
        let mut acc: i64 = 0;
        for &c in a.iter().skip(1) {
            acc = acc.wrapping_sub(i64::from(buf[i2]).wrapping_mul(i64::from(c)));
            i2 = i2.wrapping_sub(1) & 63;
        }
        out.push(v.wrapping_sub((acc >> 20) as i32));
        i1 = (i1 + 1) & 63;
        buf[i1] = v;
    }
    out
}

/// Writes one layer: a single predictor region (or none) and the residual sub-blocks.
fn write_layer(w: &mut BitWriter, x: &[i32], blocks: usize, size: usize) -> Result<()> {
    // sign-magnitude residuals hold at most 31 magnitude bits
    if x.contains(&i32::MIN) {
        return Err(Error::Invalid("a DLC layer sample of -2^31 cannot be coded"));
    }
    let cost = |r: &[i32]| -> u64 {
        (0..blocks).map(|b| r.get(b * size..(b + 1) * size).map_or(0, |s| (0..31).map(|k| rice_bits(s, k)).min().unwrap_or(u64::MAX))).sum()
    };
    let mut best: (u64, Vec<u16>, Vec<i32>) = (cost(x), Vec::new(), x.to_vec());
    for order in [2, 4, 8, MAX_ORDER] {
        let codes = lattice_codes(x, order);
        let r = residual_of(x, &direct_form(&codes));
        if r.contains(&i32::MIN) {
            continue;
        }
        // the coefficients cost 10 bits each
        let c = cost(&r) + 10 * order as u64;
        if c < best.0 {
            best = (c, codes, r);
        }
    }
    let (_, codes, residual) = best;
    if codes.is_empty() {
        w.write_bits(0, 2);
    } else {
        w.write_bits(1, 2);
        w.write_bits(blocks as u32, 4);
        w.write_bits(codes.len() as u32, 5);
        for c in &codes {
            w.write_bits(u32::from(*c), 10);
        }
    }
    write_residuals(w, &residual, blocks, size);
    Ok(())
}

/// Encodes left-justified samples (exactly [`crate::FrameRate::samples`] of them) as the
/// AudioDataDLC fields after AudioDataID and DLCSize, byte aligned.
pub(crate) fn encode(samples: &[i32], ctx: &Ctx) -> Result<Vec<u8>> {
    let (blocks, size48) = ctx.rate.dlc_blocks().ok_or(Error::Invalid("AudioDataDLC at a 1001 frame rate"))?;
    let n = ctx.rate.samples(ctx.rate96, 0);
    if samples.len() != n {
        return Err(Error::Invalid("sample count does not match the frame rate"));
    }
    // the low bits every sample has clear, put back by ShiftBits
    let shift = samples.iter().fold(0i32, |a, &v| a | v).trailing_zeros().min(31);
    let x: Vec<i32> = samples.iter().map(|&v| v >> shift).collect();
    let mut w = BitWriter::new();
    w.write_bits(u32::from(ctx.rate96), 2);
    w.write_bits(shift, 5);
    if ctx.rate96 {
        // base layer: x[2k + 16] smoothed (any filter keeps the coding lossless; this one lines
        // the base up with the decoder's eight-sample interpolation delay)
        let n48 = blocks * size48;
        let at = |i: usize| i64::from(x.get(i).or(x.last()).copied().unwrap_or(0));
        let base: Vec<i32> = (0..n48)
            .map(|k| {
                let j = 2 * k + 16;
                ((at(j.saturating_sub(1)) + 2 * at(j) + at(j + 1)) / 4) as i32
            })
            .collect();
        write_layer(&mut w, &base, blocks, size48)?;
        let up = upsample(&base, n);
        let ext: Vec<i32> = x.iter().zip(&up).map(|(a, b)| a.wrapping_sub(*b)).collect();
        write_layer(&mut w, &ext, blocks, size48 * 2)?;
    } else {
        write_layer(&mut w, &x, blocks, size48)?;
    }
    Ok(w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FrameRate;

    fn ctx(rate: FrameRate, rate96: bool) -> Ctx {
        Ctx { rate, rate96, bit_depth: 24, index: 0 }
    }

    fn roundtrip(x: &[i32], c: &Ctx) -> Vec<i32> {
        let bytes = encode(x, c).unwrap();
        decode(&mut BitReader::new(&bytes), c).unwrap()
    }

    #[test]
    fn direct_form_of_one_reflection_coefficient_is_itself() {
        // k = (768 - 512) / 512 = 0.5 → a = [1, 0.5] in Q20
        assert_eq!(direct_form(&[768]), vec![ONE, ONE / 2]);
        // two stages: a2 = k2, a1 = k1 + k2 k1
        let a = direct_form(&[768, 256]);
        assert_eq!(a, vec![ONE, ONE / 2 - ONE / 4, -ONE / 2]);
    }

    #[test]
    fn upsampling_a_constant_settles_at_the_constant() {
        let up = upsample(&[1 << 12; 100], 200);
        // after the filter fills (16 base samples) the even phase is the input and the odd
        // phase the taps' sum, 32680 / 32768 of it
        for (i, v) in up.iter().enumerate().skip(40) {
            let want = if i % 2 == 0 { 1 << 12 } else { (4096 * 32680) >> 15 };
            assert_eq!(*v, want, "{i}");
        }
        assert!(up[..16].iter().step_by(2).all(|&v| v == 0), "the even phase is delayed by eight base samples");
        assert_eq!(up[16], 1 << 12);
    }

    #[test]
    fn lossless_at_48_and_96_khz() {
        for (rate, r96) in [(FrameRate::R24, false), (FrameRate::R25, true), (FrameRate::R48, false), (FrameRate::R120, true)] {
            let c = ctx(rate, r96);
            let n = rate.samples(r96, 0);
            let x: Vec<i32> = (0..n).map(|i| ((((i as f64) * 0.05).sin() * 4e6) as i32 + (i as i32 * 7919 % 301) - 150) << 8).collect();
            assert_eq!(roundtrip(&x, &c), x, "{rate:?} 96k {r96}");
            let silence = vec![0; n];
            assert_eq!(roundtrip(&silence, &c), silence);
            // 24-bit full scale both ways
            let extremes: Vec<i32> = (0..n).map(|i| if i % 3 == 0 { i32::MIN } else { 0x7FFF_FF00 }).collect();
            assert_eq!(roundtrip(&extremes, &c), extremes);
        }
    }

    #[test]
    fn a_32_bit_minimum_is_refused_not_miscoded() {
        let c = ctx(FrameRate::R24, false);
        let mut x = vec![1; 2000];
        x[5] = i32::MIN;
        assert!(encode(&x, &c).is_err());
    }

    #[test]
    fn prediction_shrinks_smooth_signals() {
        let c = ctx(FrameRate::R24, false);
        let x: Vec<i32> = (0..2000).map(|i| (((i as f64) * 0.01).sin() * 8e6) as i32 * 256).collect();
        let bytes = encode(&x, &c).unwrap();
        // 24-bit PCM would take 6000 bytes; 10-bit reflection coefficients limit the gain on
        // very low tones
        assert!(bytes.len() < 6000 * 3 / 4, "{} bytes for 2000 smooth samples", bytes.len());
    }

    #[test]
    fn hostile_codes_are_errors_not_panics() {
        let c = ctx(FrameRate::R24, false);
        // a Rice block whose unary quotient never ends
        let mut w = BitWriter::new();
        w.write_bits(0, 2);
        w.write_bits(0, 5);
        w.write_bits(0, 2);
        w.write_bit(true);
        w.write_bits(31, 5);
        for _ in 0..64 {
            w.write_bits(u32::MAX, 32);
        }
        assert!(decode(&mut BitReader::new(&w.finish()), &c).is_err());
        // predictor regions that do not add up to the sub-blocks
        let mut w = BitWriter::new();
        w.write_bits(0, 2);
        w.write_bits(0, 5);
        w.write_bits(1, 2);
        w.write_bits(3, 4);
        w.write_bits(0, 5);
        assert_eq!(decode(&mut BitReader::new(&w.finish()), &c), Err(Error::Invalid("DLC predictor regions do not cover the frame")));
        // maximal coefficients and residuals wrap instead of overflowing
        let mut w = BitWriter::new();
        w.write_bits(0, 2);
        w.write_bits(31, 5);
        w.write_bits(1, 2);
        w.write_bits(10, 4);
        w.write_bits(31, 5);
        for _ in 0..31 {
            w.write_bits(1023, 10);
        }
        for _ in 0..10 {
            w.write_bit(false);
            w.write_bits(31, 5);
            for _ in 0..200 {
                w.write_bits(0x7FFF_FFFF, 31);
                w.write_bit(true);
            }
        }
        assert_eq!(decode(&mut BitReader::new(&w.finish()), &c).map(|v| v.len()), Ok(2000));
    }
}
