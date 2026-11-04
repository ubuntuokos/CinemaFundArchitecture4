//! Inverse CineForm 2/6 spatial wavelet.
//!
//! Filters, border rules, rounding and saturation follow the CineForm SDK
//! (`InvertSpatialQuant16s`, `InvertSpatialQuantDescale16s`, `InvertHorizontalStrip16s` and
//! `InvertHorizontalStrip16sToRow16u` in `spatial.c` / `InvertHorizontalStrip16s.c`;
//! (C) 2017 GoPro Inc, Apache-2.0 / MIT).

/// One wavelet: four `width × height` bands in the SDK's band order: 0 lowpass, 1 horizontal
/// highpass, 2 vertical highpass, 3 diagonal.
#[derive(Clone, Debug, Default)]
pub(crate) struct Wavelet {
    pub width: usize,
    pub height: usize,
    pub bands: [Option<Vec<i16>>; 4],
}

/// The last step of the horizontal synthesis filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Post {
    /// Divide by two (no prescale).
    Halve,
    /// Undo an encoder prescale: multiply by `1 << n` instead of halving (the SDK).
    Shift(u32),
    /// Undo an encoder prescale after halving: `(v >> 1) << (n + 1)` (ffmpeg's `cfhd`, which
    /// drops the bit the SDK keeps).
    HalveShift(u32),
}

#[inline]
fn sat(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// The lowpass part of the even and odd outputs at position `i` of `n`, from the lowpass
/// values at `i - 1`, `i`, `i + 1` (borders use the one-sided filters of the SDK).
#[inline]
pub(crate) fn lowpass_pair(n: usize, i: usize, l: impl Fn(usize) -> i32) -> (i32, i32) {
    let last = n.saturating_sub(1);
    let at = |k: usize| l(k.min(last));
    if i == 0 {
        let (l0, l1, l2) = (at(0), at(1), at(2));
        ((11 * l0 - 4 * l1 + l2 + 4) >> 3, (5 * l0 + 4 * l1 - l2 + 4) >> 3)
    } else if i == last {
        let (l0, l1, l2) = (at(i), at(i - 1), at(i.saturating_sub(2)));
        ((5 * l0 + 4 * l1 - l2 + 4) >> 3, (11 * l0 - 4 * l1 + l2 + 4) >> 3)
    } else {
        let (p, c, q) = (at(i - 1), at(i), at(i + 1));
        (((p - q + 4) >> 3) + c, ((q - p + 4) >> 3) + c)
    }
}

#[inline]
fn finish(v: i32, post: Post) -> i32 {
    match post {
        Post::Halve => v >> 1,
        Post::Shift(s) => v << s,
        Post::HalveShift(s) => (v >> 1) << (s + 1),
    }
}

/// Horizontal synthesis of one row: `low` and `high` (length `n`) into `2n` values passed to
/// `put(index, value)`.
#[inline]
fn horizontal(low: &[i16], high: &[i16], post: Post, mut put: impl FnMut(usize, i32)) {
    let n = low.len().min(high.len());
    let (low, high) = (low.get(..n).unwrap_or(&[]), high.get(..n).unwrap_or(&[]));
    if n < 3 {
        for (i, &h) in high.iter().enumerate() {
            let (e, o) = lowpass_pair(n, i, |k| low.get(k).map_or(0, |&v| i32::from(v)));
            let h = i32::from(h);
            put(2 * i, finish(e + h, post));
            put(2 * i + 1, finish(o - h, post));
        }
        return;
    }
    let edge = |i: usize, put: &mut dyn FnMut(usize, i32)| {
        let (e, o) = lowpass_pair(n, i, |k| low.get(k).map_or(0, |&v| i32::from(v)));
        let h = high.get(i).map_or(0, |&v| i32::from(v));
        put(2 * i, finish(e + h, post));
        put(2 * i + 1, finish(o - h, post));
    };
    edge(0, &mut put);
    for (i, (w, &h)) in low.windows(3).zip(high.get(1..).unwrap_or(&[])).enumerate() {
        let &[p, c, q] = w else { continue };
        let (p, c, q) = (i32::from(p), i32::from(c), i32::from(q));
        let h = i32::from(h);
        put(2 * i + 2, finish(((p - q + 4) >> 3) + c + h, post));
        put(2 * i + 3, finish(((q - p + 4) >> 3) + c - h, post));
    }
    edge(n - 1, &mut put);
}

/// Vertical synthesis: `low` and `high` (`width × height`) into `out` (`width × 2·height`).
fn vertical<'a>(low: &'a [i16], high: &'a [i16], width: usize, height: usize, out: &mut [i16]) {
    if width == 0 || low.len() < width * height || high.len() < width * height {
        return;
    }
    let row = |b: &'a [i16], r: usize| -> &'a [i16] { b.get(r * width..(r + 1) * width).unwrap_or(&[]) };
    for (r, rows) in out.chunks_exact_mut(2 * width).take(height).enumerate() {
        let (even, odd) = rows.split_at_mut(width);
        let h = row(high, r);
        if r == 0 || r + 1 == height || height < 3 {
            let at = |b: &[i16], r: usize, x: usize| b.get(r * width + x).map_or(0, |&v| i32::from(v));
            for (x, (pe, po)) in even.iter_mut().zip(odd.iter_mut()).enumerate() {
                let (e, o) = lowpass_pair(height, r, |k| at(low, k, x));
                let hv = at(high, r, x);
                *pe = sat((e + hv) >> 1);
                *po = sat((o - hv) >> 1);
            }
            continue;
        }
        let (p, c, q) = (row(low, r - 1), row(low, r), row(low, r + 1));
        for ((((pe, po), &p), (&c, &q)), &hv) in even.iter_mut().zip(odd.iter_mut()).zip(p).zip(c.iter().zip(q)).zip(h) {
            let (p, c, q, hv) = (i32::from(p), i32::from(c), i32::from(q), i32::from(hv));
            *pe = sat((((p - q + 4) >> 3) + c + hv) >> 1);
            *po = sat((((q - p + 4) >> 3) + c - hv) >> 1);
        }
    }
}

fn band(w: &Wavelet, i: usize) -> &[i16] {
    w.bands.get(i).and_then(|b| b.as_deref()).unwrap_or(&[])
}

/// The two vertical syntheses of a wavelet: horizontal-lowpass and horizontal-highpass rows,
/// `width × 2·height` each.
fn verticals(w: &Wavelet) -> (Vec<i16>, Vec<i16>) {
    let (width, height) = (w.width, w.height);
    let mut lowv = vec![0i16; width * 2 * height];
    let mut highv = vec![0i16; width * 2 * height];
    vertical(band(w, 0), band(w, 2), width, height, &mut lowv);
    vertical(band(w, 1), band(w, 3), width, height, &mut highv);
    (lowv, highv)
}

/// Inverse of one spatial wavelet: the `2·width × 2·height` lowpass band of the next level.
pub(crate) fn inverse_spatial(w: &Wavelet, post: Post) -> Vec<i16> {
    let width = w.width;
    let mut out = vec![0i16; 4 * width * w.height];
    if width == 0 {
        return out;
    }
    let (lowv, highv) = verticals(w);
    for ((o, l), h) in out.chunks_exact_mut(2 * width).zip(lowv.chunks_exact(width)).zip(highv.chunks_exact(width)) {
        horizontal(l, h, post, |i, v| {
            if let Some(d) = o.get_mut(i) {
                *d = sat(v);
            }
        });
    }
    out
}

/// Inverse of the last wavelet into `2·width × 2·height` output samples clamped to `0..=max`.
pub(crate) fn inverse_final(w: &Wavelet, max: i32) -> Vec<u16> {
    let width = w.width;
    let mut out = vec![0u16; 4 * width * w.height];
    if width == 0 {
        return out;
    }
    let (lowv, highv) = verticals(w);
    for ((o, l), h) in out.chunks_exact_mut(2 * width).zip(lowv.chunks_exact(width)).zip(highv.chunks_exact(width)) {
        horizontal(l, h, Post::Halve, |i, v| {
            if let Some(d) = o.get_mut(i) {
                *d = v.clamp(0, max) as u16;
            }
        });
    }
    out
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// The decoder's horizontal synthesis with halving, for encoder tests.
    pub(crate) fn horizontal_halve(low: &[i16], high: &[i16], out: &mut [i32]) {
        super::horizontal(low, high, super::Post::Halve, |i, v| {
            if let Some(d) = out.get_mut(i) {
                *d = v;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(v: i16, w: usize, h: usize) -> Wavelet {
        Wavelet { width: w, height: h, bands: [Some(vec![v; w * h]), Some(vec![0; w * h]), Some(vec![0; w * h]), Some(vec![0; w * h])] }
    }

    #[test]
    fn flat_signal_reconstructs_exactly() {
        assert!(inverse_spatial(&flat(400, 9, 5), Post::Halve).iter().all(|&v| v == 100));
        assert!(inverse_spatial(&flat(400, 9, 5), Post::Shift(1)).iter().all(|&v| v == 400));
        assert!(inverse_spatial(&flat(400, 9, 5), Post::HalveShift(1)).iter().all(|&v| v == 400));
        assert!(inverse_final(&flat(400, 4, 3), 1023).iter().all(|&v| v == 100));
        assert!(inverse_final(&flat(-400, 4, 3), 1023).iter().all(|&v| v == 0));
        assert!(inverse_final(&flat(8000, 4, 3), 1023).iter().all(|&v| v == 1023));
    }

    #[test]
    fn descale_variants_differ_only_in_the_dropped_bit() {
        // an odd intermediate value: the SDK keeps the low bit, ffmpeg drops it
        assert_eq!(finish(7, Post::Shift(1)), 14);
        assert_eq!(finish(7, Post::HalveShift(1)), 12);
        assert_eq!(finish(-7, Post::HalveShift(1)), -16);
        assert_eq!(finish(-7, Post::Halve), -4);
    }

    #[test]
    fn tiny_and_empty_bands_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (1, 3), (2, 1), (3, 2)] {
            let wv = Wavelet { width: w, height: h, bands: [Some(vec![7; w * h]), Some(vec![1; w * h]), None, Some(vec![-3; w * h])] };
            assert_eq!(inverse_spatial(&wv, Post::Halve).len(), 4 * w * h);
            assert_eq!(inverse_spatial(&wv, Post::Shift(1)).len(), 4 * w * h);
            assert_eq!(inverse_final(&wv, 1023).len(), 4 * w * h);
        }
    }
}
