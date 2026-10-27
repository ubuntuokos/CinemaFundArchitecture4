//! Forward discrete wavelet transform (T.800 Annex F.4): the analysis inverse of
//! [`crate::dwt`]'s synthesis, 5/3 in integers and 9/7 in floats, by lifting with whole-sample
//! symmetric extension. The image origin is 0, so every row and column starts at an even
//! position (low-pass samples first).

use super::Coeff;

/// 9/7 lifting parameters (Table F.4).
const ALPHA: f32 = -1.586_134_3;
const BETA: f32 = -0.052_980_118;
const GAMMA: f32 = 0.882_911_1;
const DELTA: f32 = 0.443_506_87;
const K: f32 = 1.230_174_1;

/// Neighbours of row (or sample) `k` in a signal of `n` with whole-sample symmetric extension.
fn neighbours(k: usize, n: usize) -> (usize, usize) {
    let a = if k == 0 { 1 } else { k - 1 };
    let b = if k + 1 == n { n - 2 } else { k + 1 };
    (a, b)
}

impl Coeff for i32 {
    /// 1D_SD of the 5/3 (F.4.8.1) on rows of `w` samples, as operations on whole rows.
    fn analysis_rows(data: &mut [i32], w: usize, n: usize) {
        if n < 2 {
            return;
        }
        for (odd, f) in [(true, 0u8), (false, 1u8)] {
            let mut k = usize::from(odd);
            while k < n {
                let (a, b) = neighbours(k, n);
                for x in 0..w {
                    let (va, vb) = (data[a * w + x], data[b * w + x]);
                    let v = &mut data[k * w + x];
                    *v = if f == 0 { v.wrapping_sub(va.wrapping_add(vb) >> 1) } else { v.wrapping_add(va.wrapping_add(vb).wrapping_add(2) >> 2) };
                }
                k += 2;
            }
        }
    }
}

impl Coeff for f32 {
    /// 1D_SD of the 9/7 (F.4.8.2) on rows of `w` samples.
    fn analysis_rows(data: &mut [f32], w: usize, n: usize) {
        if n < 2 {
            return;
        }
        for (odd, c) in [(true, ALPHA), (false, BETA), (true, GAMMA), (false, DELTA)] {
            let mut k = usize::from(odd);
            while k < n {
                let (a, b) = neighbours(k, n);
                for x in 0..w {
                    let s = data[a * w + x] + data[b * w + x];
                    data[k * w + x] += c * s;
                }
                k += 2;
            }
        }
        for (k, row) in data.chunks_exact_mut(w).take(n).enumerate() {
            let s = if k % 2 == 0 { 1.0 / K } else { K };
            row.iter_mut().for_each(|v| *v *= s);
        }
    }
}

/// One decomposition level of a `w` × `h` region: (LL, HL, LH, HH), each row-major. The
/// columns are transformed first, then the rows, the inverse of the decoder's order.
pub(super) fn level<T: Coeff>(region: &[T], w: usize, h: usize) -> [Vec<T>; 4] {
    let (lw, hw) = (w.div_ceil(2), w / 2);
    let (lh, hh) = (h.div_ceil(2), h / 2);
    // vertical: rows are the samples
    let mut v = region.to_vec();
    T::analysis_rows(&mut v, w, h);
    // horizontal: each row on its own, as a column of one sample
    let horizontal = |row: &mut [T]| T::analysis_rows(row, 1, w);
    if w > 0 {
        #[cfg(feature = "threads")]
        {
            use rayon::prelude::*;
            v.par_chunks_exact_mut(w).for_each(horizontal);
        }
        #[cfg(not(feature = "threads"))]
        v.chunks_exact_mut(w).for_each(horizontal);
    }
    let mut bands = [vec![T::default(); lw * lh], vec![T::default(); hw * lh], vec![T::default(); lw * hh], vec![T::default(); hw * hh]];
    for (y, row) in v.chunks_exact(w.max(1)).enumerate() {
        let by = y / 2;
        let [b0, b1, b2, b3] = &mut bands;
        let (lo, hi) = if y % 2 == 0 { (b0, b1) } else { (b2, b3) };
        for (dst, src) in lo[by * lw..(by + 1) * lw].iter_mut().zip(row.iter().step_by(2)) {
            *dst = *src;
        }
        for (dst, src) in hi[by * hw..(by + 1) * hw].iter_mut().zip(row.iter().skip(1).step_by(2)) {
            *dst = *src;
        }
    }
    bands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dwt::{Rect, level as synthesis};

    /// Analysis then the decoder's synthesis gives the input back: exactly for the 5/3, to float
    /// precision for the 9/7, at every size and parity.
    #[test]
    fn synthesis_inverts_analysis() {
        for (w, h) in [(1, 1), (1, 5), (6, 1), (7, 5), (16, 9), (33, 17)] {
            let src: Vec<i32> = (0..w * h).map(|k| ((k * 7919) % 255) as i32 - 128).collect();
            let [ll, hl, lh, hh] = level(&src, w, h);
            let rect = Rect { u0: 0, v0: 0, u1: w as u32, v1: h as u32 };
            assert_eq!(synthesis(&ll, &hl, &lh, &hh, rect), src, "5/3 {w}x{h}");
            let srcf: Vec<f32> = src.iter().map(|&v| v as f32).collect();
            let [ll, hl, lh, hh] = level(&srcf, w, h);
            let back = synthesis(&ll, &hl, &lh, &hh, rect);
            assert!(back.iter().zip(&srcf).all(|(a, b)| (a - b).abs() < 1e-3), "9/7 {w}x{h}");
        }
    }
}
