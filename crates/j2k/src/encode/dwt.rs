//! Forward discrete wavelet transform (T.800 Annex F.4): the analysis inverse of
//! [`crate::dwt`]'s synthesis, 5/3 in integers and 9/7 in floats, by lifting with whole-sample
//! symmetric extension. The image origin is 0, so every row and column starts at an even
//! position (low-pass samples first).
//!
//! Each lifting step runs over whole slices (rows for the vertical transform, the
//! de-interleaved low-pass and high-pass halves of a row for the horizontal one), which the
//! compiler vectorises.

use super::Coeff;

/// 9/7 lifting parameters (Table F.4).
const ALPHA: f32 = -1.586_134_3;
const BETA: f32 = -0.052_980_118;
const GAMMA: f32 = 0.882_911_1;
const DELTA: f32 = 0.443_506_87;
const K: f32 = 1.230_174_1;

impl Coeff for i32 {
    const STEPS: usize = 2;

    /// The 5/3 lifting steps (F.4.8.1): predict, then update.
    fn lift(step: usize, v: &mut [i32], a: &[i32], b: &[i32]) {
        let n = v.len().min(a.len()).min(b.len());
        let (v, a, b) = (&mut v[..n], &a[..n], &b[..n]);
        if step == 0 {
            for ((v, &a), &b) in v.iter_mut().zip(a).zip(b) {
                *v = v.wrapping_sub(a.wrapping_add(b) >> 1);
            }
        } else {
            for ((v, &a), &b) in v.iter_mut().zip(a).zip(b) {
                *v = v.wrapping_add(a.wrapping_add(b).wrapping_add(2) >> 2);
            }
        }
    }
}

impl Coeff for f32 {
    const STEPS: usize = 4;

    /// The 9/7 lifting steps (F.4.8.2).
    fn lift(step: usize, v: &mut [f32], a: &[f32], b: &[f32]) {
        let c = match step {
            0 => ALPHA,
            1 => BETA,
            2 => GAMMA,
            _ => DELTA,
        };
        let n = v.len().min(a.len()).min(b.len());
        let (v, a, b) = (&mut v[..n], &a[..n], &b[..n]);
        for ((v, &a), &b) in v.iter_mut().zip(a).zip(b) {
            *v += c * (a + b);
        }
    }

    fn scale(lo: &mut [f32], hi: &mut [f32]) {
        lo.iter_mut().for_each(|v| *v *= 1.0 / K);
        hi.iter_mut().for_each(|v| *v *= K);
    }
}

/// Neighbours of row (or sample) `k` in a signal of `n` with whole-sample symmetric extension.
fn neighbours(k: usize, n: usize) -> (usize, usize) {
    let a = if k == 0 { 1 } else { k - 1 };
    let b = if k + 1 == n { n - 2 } else { k + 1 };
    (a, b)
}

/// Lifting step `step` on row `k` of the `w`-sample rows in `data`, from rows `a` and `b`.
fn lift_row<T: Coeff>(data: &mut [T], w: usize, step: usize, k: usize, a: usize, b: usize) {
    let (head, tail) = data.split_at_mut((k * w).min(data.len()));
    let (row, after) = tail.split_at_mut(w.min(tail.len()));
    let neighbour = |r: usize| -> &[T] {
        if r < k {
            head.get(r * w..(r + 1) * w).unwrap_or(&[])
        } else {
            let at = (r - k - 1) * w;
            after.get(at..at + w).unwrap_or(&[])
        }
    };
    T::lift(step, row, neighbour(a), neighbour(b));
}

/// One-dimensional analysis of the `n` rows of `w` samples in `data`, along the rows' index:
/// the rows stay interleaved (low-pass at even, high-pass at odd indices). The lifting steps are
/// pipelined in one sweep: at row `t`, step `s` runs on row `t - s` (its neighbours then hold
/// step `s - 1` and not yet step `s + 1`), so each row is touched while it is in the cache.
fn vertical<T: Coeff>(data: &mut [T], w: usize, n: usize) {
    if n < 2 || w == 0 {
        return;
    }
    let scale = |data: &mut [T], r: usize| {
        if let Some(row) = data.get_mut(r * w..(r + 1) * w) {
            if r.is_multiple_of(2) {
                T::scale(row, &mut []);
            } else {
                T::scale(&mut [], row);
            }
        }
    };
    for t in 0..n + T::STEPS {
        for step in 0..T::STEPS {
            // step 0, 2: odd rows; 1, 3: even rows
            let Some(k) = t.checked_sub(step) else { break };
            if k < n && k % 2 != step % 2 {
                let (a, b) = neighbours(k, n);
                lift_row(data, w, step, k, a, b);
            }
        }
        if let Some(r) = t.checked_sub(T::STEPS) {
            scale(data, r);
        }
    }
}

/// One-dimensional analysis of `row`, de-interleaved: low-pass samples first, then high-pass.
/// `lo` and `hi` are scratch buffers.
fn horizontal<T: Coeff>(row: &mut [T], lo: &mut Vec<T>, hi: &mut Vec<T>) {
    let w = row.len();
    if w < 2 {
        return;
    }
    lo.clear();
    hi.clear();
    lo.extend(row.iter().step_by(2));
    hi.extend(row.iter().skip(1).step_by(2));
    let (nl, nh) = (lo.len(), hi.len());
    for step in 0..T::STEPS {
        if step % 2 == 0 {
            // odd samples: x[2i+1] from x[2i] and x[2i+2] (the last mirrors to x[2i])
            let m = nh.min(nl - 1);
            T::lift(step, &mut hi[..m], &lo[..m], &lo[1..=m]);
            if m < nh {
                T::lift(step, &mut hi[m..nh], &lo[m..nh], &lo[m..nh]);
            }
        } else {
            // even samples: x[2i] from x[2i-1] and x[2i+1] (the first and an odd width's last
            // mirror)
            T::lift(step, &mut lo[..1], &hi[..1], &hi[..1]);
            let m = nl.min(nh);
            if m > 1 {
                T::lift(step, &mut lo[1..m], &hi[..m - 1], &hi[1..m]);
            }
            if nl > nh {
                T::lift(step, &mut lo[nh..nl], &hi[nh - 1..nh], &hi[nh - 1..nh]);
            }
        }
    }
    T::scale(lo, hi);
    let (l, h) = row.split_at_mut(nl);
    l.copy_from_slice(lo);
    h.copy_from_slice(hi);
}

/// One decomposition level of a `w` × `h` region: (LL, HL, LH, HH), each row-major. The
/// columns are transformed first, then the rows, the inverse of the decoder's order.
pub(super) fn level<T: Coeff>(region: &[T], w: usize, h: usize) -> [Vec<T>; 4] {
    let (lw, hw) = (w.div_ceil(2), w / 2);
    let (lh, hh) = (h.div_ceil(2), h / 2);
    let mut v = region.get(..w * h).unwrap_or(region).to_vec();
    v.resize(w * h, T::default());
    vertical(&mut v, w, h);
    if w > 1 {
        let rows = |row: &mut [T], (lo, hi): &mut (Vec<T>, Vec<T>)| horizontal(row, lo, hi);
        #[cfg(feature = "threads")]
        {
            use rayon::prelude::*;
            v.par_chunks_exact_mut(w).for_each_init(|| (Vec::with_capacity(lw), Vec::with_capacity(hw)), |s, row| rows(row, s));
        }
        #[cfg(not(feature = "threads"))]
        {
            let mut s = (Vec::with_capacity(lw), Vec::with_capacity(hw));
            v.chunks_exact_mut(w).for_each(|row| rows(row, &mut s));
        }
    }
    let mut bands = [Vec::with_capacity(lw * lh), Vec::with_capacity(hw * lh), Vec::with_capacity(lw * hh), Vec::with_capacity(hw * hh)];
    for (y, row) in v.chunks_exact(w.max(1)).enumerate() {
        let [b0, b1, b2, b3] = &mut bands;
        let (lo, hi) = if y % 2 == 0 { (b0, b1) } else { (b2, b3) };
        let (l, r) = row.split_at(lw.min(row.len()));
        lo.extend_from_slice(l);
        hi.extend_from_slice(r);
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
        for (w, h) in [(1, 1), (1, 5), (6, 1), (2, 2), (3, 3), (7, 5), (16, 9), (33, 17), (2, 7), (5, 2)] {
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
