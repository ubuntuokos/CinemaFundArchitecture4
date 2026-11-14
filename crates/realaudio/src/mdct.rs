//! The inverse MDCT FFmpeg's Cook decoder takes from `av_tx` (`AV_TX_FLOAT_MDCT` with
//! `AV_TX_FULL_IMDCT`): `N` coefficients to `2N` samples,
//! `y[n] = scale · Σ X[k] cos(π/N (n + ½ + N/2)(k + ½))`, computed through a DCT-IV with an
//! `N/2`-point complex FFT in `f64` (FFmpeg's own result depends on its SIMD code, so the output
//! matches it to rounding). `av_tx` with scale `s` is this transform with scale `-s`.

use std::f64::consts::PI;

#[derive(Clone)]
pub(crate) struct Imdct {
    n: usize,
    scale: f64,
    pre: Vec<(f64, f64)>,
    post: Vec<(f64, f64)>,
    twiddle: Vec<(f64, f64)>,
    rev: Vec<usize>,
}

impl Imdct {
    /// `n` must be a power of two of at least 4.
    pub(crate) fn new(n: usize, scale: f64) -> Self {
        let h = n / 2;
        let nf = n as f64;
        let pre = (0..h).map(|i| (-PI * (i as f64 + 0.25) / nf).sin_cos()).map(|(s, c)| (c, s)).collect();
        let post = (0..h).map(|k| (-PI * k as f64 / nf).sin_cos()).map(|(s, c)| (c, s)).collect();
        let twiddle = (0..h / 2).map(|k| (-2.0 * PI * k as f64 / h as f64).sin_cos()).map(|(s, c)| (c, s)).collect();
        let bits = h.trailing_zeros();
        let rev = (0..h).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        Self { n, scale, pre, post, twiddle, rev }
    }

    /// DCT-IV of `x` (length `n`) into `y`.
    fn dct4(&self, x: &[f32], y: &mut [f64]) {
        let n = self.n;
        let h = n / 2;
        let mut re = vec![0f64; h];
        let mut im = vec![0f64; h];
        for i in 0..h {
            let a = f64::from(x.get(2 * i).copied().unwrap_or(0.0));
            let b = f64::from(x.get(n - 1 - 2 * i).copied().unwrap_or(0.0));
            let (c, s) = self.pre.get(i).copied().unwrap_or((1.0, 0.0));
            let j = self.rev.get(i).copied().unwrap_or(0);
            if let (Some(r), Some(m)) = (re.get_mut(j), im.get_mut(j)) {
                *r = a * c - b * s;
                *m = a * s + b * c;
            }
        }
        let mut len = 2;
        while len <= h {
            let half = len / 2;
            let step = h / len;
            for start in (0..h).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = self.twiddle.get(k * step).copied().unwrap_or((1.0, 0.0));
                    let (a, b) = (start + k, start + k + half);
                    let (br, bi) = (re.get(b).copied().unwrap_or(0.0), im.get(b).copied().unwrap_or(0.0));
                    let (ar, ai) = (re.get(a).copied().unwrap_or(0.0), im.get(a).copied().unwrap_or(0.0));
                    let xr = br * wr - bi * wi;
                    let xi = br * wi + bi * wr;
                    if let (Some(r), Some(m)) = (re.get_mut(b), im.get_mut(b)) {
                        *r = ar - xr;
                        *m = ai - xi;
                    }
                    if let (Some(r), Some(m)) = (re.get_mut(a), im.get_mut(a)) {
                        *r = ar + xr;
                        *m = ai + xi;
                    }
                }
            }
            len <<= 1;
        }
        for k in 0..h {
            let (c, s) = self.post.get(k).copied().unwrap_or((1.0, 0.0));
            let (r, m) = (re.get(k).copied().unwrap_or(0.0), im.get(k).copied().unwrap_or(0.0));
            if let Some(v) = y.get_mut(2 * k) {
                *v = r * c - m * s;
            }
            if let Some(v) = y.get_mut(n - 1 - 2 * k) {
                *v = -(r * s + m * c);
            }
        }
    }

    /// `n` coefficients from `x` to `2n` samples in `out`.
    pub(crate) fn inverse(&self, x: &[f32], out: &mut [f32]) {
        let n = self.n;
        let mut u = vec![0f64; n];
        self.dct4(x, &mut u);
        for (i, o) in out.iter_mut().take(2 * n).enumerate() {
            // c(m) for m = i + n/2: u[m], -u[2n-1-m], -u[m-2n]
            let m = i + n / 2;
            let v = if m < n {
                u.get(m).copied().unwrap_or(0.0)
            } else if m < 2 * n {
                -u.get(2 * n - 1 - m).copied().unwrap_or(0.0)
            } else {
                -u.get(m - 2 * n).copied().unwrap_or(0.0)
            };
            *o = (v * self.scale) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_definition() {
        for n in [4usize, 16, 256] {
            let x: Vec<f32> = (0..n).map(|i| ((i * 7919 % 113) as f32 - 56.0) / 17.0).collect();
            let t = Imdct::new(n, 0.5);
            let mut y = vec![0f32; 2 * n];
            t.inverse(&x, &mut y);
            for (i, &v) in y.iter().enumerate() {
                let want: f64 = x
                    .iter()
                    .enumerate()
                    .map(|(k, &c)| f64::from(c) * (PI / n as f64 * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5)).cos())
                    .sum::<f64>()
                    * 0.5;
                assert!((f64::from(v) - want).abs() < 1e-4 * (1.0 + want.abs()), "n {n} i {i}: {v} vs {want}");
            }
        }
    }
}
