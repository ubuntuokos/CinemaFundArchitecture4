//! Inverse discrete wavelet transform (T.800 Annex F): the reversible 5/3 filter in integers and
//! the irreversible 9/7 filter in floats, by lifting, with symmetric extension (F.3.7).

/// 9/7 lifting parameters (Table F.4).
const ALPHA: f32 = -1.586_134_3;
const BETA: f32 = -0.052_980_118;
const GAMMA: f32 = 0.882_911_1;
const DELTA: f32 = 0.443_506_87;
const K: f32 = 1.230_174_1;

/// A resolution level's rectangle on its own grid: [u0, u1) × [v0, v1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rect {
    pub u0: u32,
    pub v0: u32,
    pub u1: u32,
    pub v1: u32,
}

impl Rect {
    pub fn w(&self) -> usize {
        (self.u1 - self.u0) as usize
    }
    pub fn h(&self) -> usize {
        (self.v1 - self.v0) as usize
    }
}

/// Index `k` of a signal of length `n` with whole-sample symmetric extension.
fn reflect(k: isize, n: usize) -> usize {
    let n = n as isize;
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut k = k.rem_euclid(period);
    if k >= n {
        k = period - k;
    }
    k as usize
}

pub(crate) trait Sample: Copy + Default {
    /// 1D_SR on `x`, whose first sample sits at position `i0` of the signal.
    fn synthesis(x: &mut [Self], i0: u32);
}

impl Sample for i32 {
    fn synthesis(x: &mut [i32], i0: u32) {
        let n = x.len();
        if n == 1 {
            if i0 % 2 == 1 {
                x[0] /= 2;
            }
            return;
        }
        if n == 0 {
            return;
        }
        let first_even = (i0 % 2) as usize;
        let at = |x: &[i32], k: isize| x.get(reflect(k, n)).copied().unwrap_or(0);
        let mut k = first_even;
        while k < n {
            let s = at(x, k as isize - 1).wrapping_add(at(x, k as isize + 1)).wrapping_add(2);
            x[k] = x[k].wrapping_sub(s.div_euclid(4));
            k += 2;
        }
        let mut k = 1 - first_even;
        while k < n {
            let s = at(x, k as isize - 1).wrapping_add(at(x, k as isize + 1));
            x[k] = x[k].wrapping_add(s.div_euclid(2));
            k += 2;
        }
    }
}

impl Sample for f32 {
    fn synthesis(x: &mut [f32], i0: u32) {
        let n = x.len();
        if n == 1 {
            if i0 % 2 == 1 {
                x[0] /= 2.0;
            }
            return;
        }
        if n == 0 {
            return;
        }
        let first_even = (i0 % 2) as usize;
        for (k, v) in x.iter_mut().enumerate() {
            *v *= if (k + 2 - first_even).is_multiple_of(2) { K } else { 1.0 / K };
        }
        let lift = |x: &mut [f32], start: usize, c: f32| {
            let mut k = start;
            while k < n {
                let a = x.get(reflect(k as isize - 1, n)).copied().unwrap_or(0.0);
                let b = x.get(reflect(k as isize + 1, n)).copied().unwrap_or(0.0);
                x[k] -= c * (a + b);
                k += 2;
            }
        };
        lift(x, first_even, DELTA);
        lift(x, 1 - first_even, GAMMA);
        lift(x, first_even, BETA);
        lift(x, 1 - first_even, ALPHA);
    }
}

/// One level of 2D synthesis (2D_SR, F.3.3): the four subbands of resolution `r` (LL is the
/// resolution below) interleaved onto `rect`, then filtered along rows and columns.
pub(crate) fn level<T: Sample>(ll: &[T], hl: &[T], lh: &[T], hh: &[T], rect: Rect) -> Vec<T> {
    let (w, h) = (rect.w(), rect.h());
    let mut out = vec![T::default(); w * h];
    // band widths: low ceil(u1/2) - ceil(u0/2), high floor(u1/2) - floor(u0/2)
    let (lw, hw) = ((rect.u1.div_ceil(2) - rect.u0.div_ceil(2)) as usize, (rect.u1 / 2 - rect.u0 / 2) as usize);
    let (lx0, hx0, ly0, hy0) = (rect.u0.div_ceil(2), rect.u0 / 2, rect.v0.div_ceil(2), rect.v0 / 2);
    for (j, row) in out.chunks_mut(w.max(1)).enumerate().take(h) {
        let v = rect.v0 + j as u32;
        let (vodd, by) = if v.is_multiple_of(2) { (false, (v / 2 - ly0) as usize) } else { (true, ((v - 1) / 2 - hy0) as usize) };
        for (i, o) in row.iter_mut().enumerate() {
            let u = rect.u0 + i as u32;
            let (src, bw, bx) = match (u % 2 == 1, vodd) {
                (false, false) => (ll, lw, (u / 2 - lx0) as usize),
                (true, false) => (hl, hw, ((u - 1) / 2 - hx0) as usize),
                (false, true) => (lh, lw, (u / 2 - lx0) as usize),
                (true, true) => (hh, hw, ((u - 1) / 2 - hx0) as usize),
            };
            *o = src.get(by * bw + bx).copied().unwrap_or_default();
        }
    }
    if w == 0 || h == 0 {
        return out;
    }
    for row in out.chunks_mut(w) {
        T::synthesis(row, rect.u0);
    }
    let mut col = vec![T::default(); h];
    for i in 0..w {
        for (j, c) in col.iter_mut().enumerate() {
            *c = out[j * w + i];
        }
        T::synthesis(&mut col, rect.v0);
        for (j, c) in col.iter().enumerate() {
            out[j * w + i] = *c;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflection_is_whole_sample_symmetric() {
        let idx: Vec<usize> = (-3..8).map(|k| reflect(k, 4)).collect();
        assert_eq!(idx, [3, 2, 1, 0, 1, 2, 3, 2, 1, 0, 1]);
    }

    /// 5/3 analysis (F.4.8.1), the inverse of the synthesis, for the round trip.
    fn analysis53(x: &mut [i32], i0: u32) {
        let n = x.len();
        let first_even = (i0 % 2) as usize;
        let at = |x: &[i32], k: isize| x[reflect(k, n)];
        let mut k = 1 - first_even;
        while k < n {
            x[k] -= (at(x, k as isize - 1) + at(x, k as isize + 1)).div_euclid(2);
            k += 2;
        }
        let mut k = first_even;
        while k < n {
            x[k] += (at(x, k as isize - 1) + at(x, k as isize + 1) + 2).div_euclid(4);
            k += 2;
        }
    }

    #[test]
    fn the_53_synthesis_inverts_the_analysis_for_any_offset_and_length() {
        for n in 2..12usize {
            for i0 in 0..3u32 {
                let orig: Vec<i32> = (0..n as i32).map(|k| (k * 37 % 23) - 11).collect();
                let mut x = orig.clone();
                analysis53(&mut x, i0);
                i32::synthesis(&mut x, i0);
                assert_eq!(x, orig, "n {n} i0 {i0}");
            }
        }
    }
}
