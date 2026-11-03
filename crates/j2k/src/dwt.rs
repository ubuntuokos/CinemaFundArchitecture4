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

pub(crate) trait Sample: Copy + Default + Send + Sync {
    /// A lifting step's coefficient.
    type Step: Copy + Send + Sync + 'static;
    /// The lifting steps of 1D_SR (F.3.8) in order: whether the step updates the even samples,
    /// and its coefficient for [`Sample::lift`].
    const STEPS: &'static [(bool, Self::Step)];
    /// One lifting update of `x` from its neighbours `a` and `b`.
    fn lift(step: Self::Step, x: Self, a: Self, b: Self) -> Self;
    /// Scales the low-pass (`low`) or high-pass samples before lifting (K and 1 / K for the 9/7).
    fn scale(x: &mut [Self], low: bool);
    /// The single sample of a length-1 signal at an odd position (F.3.7).
    fn halve(x: &mut [Self]);
}

impl Sample for i32 {
    /// 0: x − ⌊(a + b + 2) / 4⌋, 1: x + ⌊(a + b) / 2⌋.
    type Step = u8;
    const STEPS: &'static [(bool, u8)] = &[(true, 0), (false, 1)];
    #[inline]
    fn lift(step: u8, x: i32, a: i32, b: i32) -> i32 {
        if step == 0 { x.wrapping_sub(a.wrapping_add(b).wrapping_add(2) >> 2) } else { x.wrapping_add(a.wrapping_add(b) >> 1) }
    }
    fn scale(_: &mut [i32], _: bool) {}
    fn halve(x: &mut [i32]) {
        x.iter_mut().for_each(|v| *v /= 2);
    }
}

impl Sample for f32 {
    type Step = f32;
    const STEPS: &'static [(bool, f32)] = &[(true, DELTA), (false, GAMMA), (true, BETA), (false, ALPHA)];
    #[inline]
    fn lift(c: f32, x: f32, a: f32, b: f32) -> f32 {
        x - c * (a + b)
    }
    fn scale(x: &mut [f32], low: bool) {
        let k = if low { K } else { 1.0 / K };
        x.iter_mut().for_each(|v| *v *= k);
    }
    fn halve(x: &mut [f32]) {
        x.iter_mut().for_each(|v| *v /= 2.0);
    }
}

/// 1D_SR on interleaved `x`, whose first sample sits at position `i0` of the signal: the
/// textbook form, the reference for [`synthesis_split`] in the tests.
#[cfg(test)]
pub(crate) fn synthesis<T: Sample>(x: &mut [T], i0: u32) {
    let n = x.len();
    if n < 2 {
        if n == 1 && i0 % 2 == 1 {
            T::halve(x);
        }
        return;
    }
    let first_even = (i0 % 2) as usize;
    for (k, v) in x.iter_mut().enumerate() {
        T::scale(std::slice::from_mut(v), (k + first_even).is_multiple_of(2));
    }
    for &(on_even, c) in T::STEPS {
        let f = |x: T, a: T, b: T| T::lift(c, x, a, b);
        let mut k = if on_even { first_even } else { 1 - first_even };
        // the ends reflect (whole-sample symmetric extension); the inside does not need to
        if k == 0 {
            x[0] = f(x[0], x[1], x[1]);
            k = 2;
        }
        while k + 1 < n {
            x[k] = f(x[k], x[k - 1], x[k + 1]);
            k += 2;
        }
        if k == n - 1 {
            x[k] = f(x[k], x[k - 1], x[k - 1]);
        }
    }
}

/// One lifting step on deinterleaved samples: `target[i]` from `src[i + offset]` and
/// `src[i + offset + 1]` (`offset` 0 or -1), a neighbour past either end being its mirror image,
/// the other neighbour (whole-sample symmetric extension).
fn lift_step<T: Sample>(target: &mut [T], src: &[T], offset: isize, c: T::Step) {
    let (nt, ns) = (target.len(), src.len());
    if ns == 0 {
        return;
    }
    let get = |i: isize| src.get(i.clamp(0, ns as isize - 1) as usize).copied().unwrap_or_default();
    // the ends, where a neighbour is mirrored
    let edge = |x: T, i: usize| -> T {
        let (a, b) = (i as isize + offset, i as isize + offset + 1);
        let (a, b) = if a < 0 {
            (b, b)
        } else if b >= ns as isize {
            (a, a)
        } else {
            (a, b)
        };
        T::lift(c, x, get(a), get(b))
    };
    // inside: i with 0 <= i + offset and i + offset + 1 < ns
    let lo = usize::from(offset < 0);
    let hi = (ns as isize - 1 - offset).clamp(0, nt as isize) as usize;
    if lo > 0
        && let Some(t) = target.first_mut()
    {
        *t = edge(*t, 0);
    }
    if lo < hi {
        let s0 = (lo as isize + offset) as usize;
        let (a, b) = (&src[s0..], &src[s0 + 1..]);
        for ((t, &p), &q) in target[lo..hi].iter_mut().zip(a).zip(b) {
            *t = T::lift(c, *t, p, q);
        }
    }
    for (i, t) in target.iter_mut().enumerate().skip(hi.max(lo)) {
        *t = edge(*t, i);
    }
}

/// 1D_SR of one row given as its low-pass (`low`) and high-pass (`high`) samples, written
/// interleaved into `out`, whose first sample sits at position `i0` of the signal.
fn synthesis_split<T: Sample>(low: &mut [T], high: &mut [T], out: &mut [T], i0: u32) {
    let first_even = i0.is_multiple_of(2);
    if out.len() < 2 {
        if let Some(o) = out.first_mut() {
            *o = if first_even { low.first().copied().unwrap_or_default() } else { high.first().copied().unwrap_or_default() };
            if !first_even {
                T::halve(std::slice::from_mut(o));
            }
        }
        return;
    }
    T::scale(low, true);
    T::scale(high, false);
    // low-pass samples sit at even positions: their neighbours are high[i - 1], high[i] when
    // the row starts at an even position, high[i], high[i + 1] otherwise
    let (low_off, high_off) = if first_even { (-1, 0) } else { (0, -1) };
    for &(on_even, c) in T::STEPS {
        if on_even {
            lift_step(low, high, low_off, c);
        } else {
            lift_step(high, low, high_off, c);
        }
    }
    let (even, odd) = if first_even { (&*low, &*high) } else { (&*high, &*low) };
    for (pair, (&e, &o)) in out.chunks_mut(2).zip(even.iter().zip(odd.iter().chain(std::iter::repeat(&T::default())))) {
        if let [a, b] = pair {
            (*a, *b) = (e, o);
        } else if let [a] = pair {
            *a = e;
        }
    }
}

/// Vertical 1D_SR of the columns of `rows` (a strip of the image's rows, top first), whose
/// first row sits at `v0`, as operations on whole rows.
fn synthesis_columns<T: Sample>(rows: &mut [&mut [T]], v0: u32) {
    let h = rows.len();
    if h < 2 {
        if let Some(r) = rows.first_mut()
            && v0 % 2 == 1
        {
            T::halve(r);
        }
        return;
    }
    let first_even = (v0 % 2) as usize;
    for (j, row) in rows.iter_mut().enumerate() {
        T::scale(row, (j + first_even).is_multiple_of(2));
    }
    for &(on_even, c) in T::STEPS {
        let mut k = if on_even { first_even } else { 1 - first_even };
        while k < h {
            // neighbours with whole-sample symmetric extension
            let a = if k == 0 { 1 } else { k - 1 };
            let b = if k + 1 == h { h - 2 } else { k + 1 };
            let (before, rest) = rows.split_at_mut(k);
            let Some((row, after)) = rest.split_first_mut() else { break };
            let pick = |i: usize| -> Option<&[T]> { if i < k { before.get(i).map(|r| &**r) } else { after.get(i - k - 1).map(|r| &**r) } };
            if let (Some(ra), Some(rb)) = (pick(a), pick(b)) {
                for ((x, &p), &q) in row.iter_mut().zip(ra).zip(rb) {
                    *x = T::lift(c, *x, p, q);
                }
            }
            k += 2;
        }
    }
}

/// Columns per strip of the vertical pass (a strip's rows stay in cache).
const STRIP: usize = 64;

/// One level of 2D synthesis (2D_SR, F.3.3): the four subbands of resolution `r` (LL is the
/// resolution below) interleaved onto `rect`, then filtered along rows and columns.
pub(crate) fn level<T: Sample>(ll: &[T], hl: &[T], lh: &[T], hh: &[T], rect: Rect) -> Vec<T> {
    let (w, h) = (rect.w(), rect.h());
    let mut out = vec![T::default(); w * h];
    if w == 0 || h == 0 {
        return out;
    }
    // band widths: low ceil(u1/2) - ceil(u0/2), high floor(u1/2) - floor(u0/2)
    let (lw, hw) = ((rect.u1.div_ceil(2) - rect.u0.div_ceil(2)) as usize, (rect.u1 / 2 - rect.u0 / 2) as usize);
    let (ly0, hy0) = (rect.v0.div_ceil(2), rect.v0 / 2);
    let horizontal = |(j, row): (usize, &mut [T])| {
        let v = rect.v0 + j as u32;
        let (lo, hi) = if v.is_multiple_of(2) {
            let by = (v / 2 - ly0) as usize;
            (ll.get(by * lw..(by + 1) * lw), hl.get(by * hw..(by + 1) * hw))
        } else {
            let by = ((v - 1) / 2 - hy0) as usize;
            (lh.get(by * lw..(by + 1) * lw), hh.get(by * hw..(by + 1) * hw))
        };
        let (mut lo, mut hi) = (lo.map_or_else(|| vec![T::default(); lw], <[T]>::to_vec), hi.map_or_else(|| vec![T::default(); hw], <[T]>::to_vec));
        synthesis_split(&mut lo, &mut hi, row, rect.u0);
    };
    // the column strips: each holds its part of every row
    #[cfg(feature = "threads")]
    {
        use rayon::prelude::*;
        out.par_chunks_exact_mut(w).enumerate().for_each(horizontal);
    }
    #[cfg(not(feature = "threads"))]
    out.chunks_exact_mut(w).enumerate().for_each(horizontal);
    let mut strips: Vec<Vec<&mut [T]>> = (0..w.div_ceil(STRIP)).map(|_| Vec::with_capacity(h)).collect();
    for row in out.chunks_exact_mut(w) {
        for (strip, part) in strips.iter_mut().zip(row.chunks_mut(STRIP)) {
            strip.push(part);
        }
    }
    #[cfg(feature = "threads")]
    {
        use rayon::prelude::*;
        strips.par_iter_mut().for_each(|s| synthesis_columns(s, rect.v0));
    }
    #[cfg(not(feature = "threads"))]
    strips.iter_mut().for_each(|s| synthesis_columns(s, rect.v0));
    drop(strips);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
                synthesis(&mut x, i0);
                assert_eq!(x, orig, "n {n} i0 {i0}");
            }
        }
    }

    /// The deinterleaved lifting the decoder uses equals the textbook interleaved form, at every
    /// length and both parities of the first position.
    #[test]
    fn split_synthesis_matches_the_interleaved_form() {
        for n in 1..14usize {
            for i0 in 0..2u32 {
                let x: Vec<i32> = (0..n as i32).map(|k| (k * 53 % 29) - 14).collect();
                let first_even = i0 % 2 == 0;
                let split = |x: &[i32]| -> (Vec<i32>, Vec<i32>) {
                    let (a, b): (Vec<(usize, i32)>, Vec<(usize, i32)>) = x.iter().copied().enumerate().partition(|(k, _)| (k % 2 == 0) == first_even);
                    (a.into_iter().map(|p| p.1).collect(), b.into_iter().map(|p| p.1).collect())
                };
                let mut want = x.clone();
                synthesis(&mut want, i0);
                let (mut lo, mut hi) = split(&x);
                let mut got = vec![0; n];
                synthesis_split(&mut lo, &mut hi, &mut got, i0);
                assert_eq!(got, want, "5/3 n {n} i0 {i0}");

                let xf: Vec<f32> = x.iter().map(|&v| v as f32 * 0.37).collect();
                let mut want = xf.clone();
                synthesis(&mut want, i0);
                let (lo, hi) = split(&x);
                let (mut lo, mut hi): (Vec<f32>, Vec<f32>) = (lo.iter().map(|&v| v as f32 * 0.37).collect(), hi.iter().map(|&v| v as f32 * 0.37).collect());
                let mut got = vec![0.0; n];
                synthesis_split(&mut lo, &mut hi, &mut got, i0);
                assert!(got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-4), "9/7 n {n} i0 {i0}: {got:?} {want:?}");
            }
        }
    }
}
