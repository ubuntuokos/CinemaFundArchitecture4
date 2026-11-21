// After atracdenc 0.2.3 src/lib/mdct/mdct.h and mdct.cpp (Copyright (c) Daniil Cherednik;
// LGPL-2.1-or-later); the complex FFT (kissfft there) is our own radix-2 transform. See
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! The forward MDCT (`N` inputs, `N/2` outputs) through an `N/4`-point complex FFT, with
//! atracdenc's pre- and post-twiddle and scaling.

use std::f64::consts::PI;

/// A radix-2 forward complex FFT (`X[k] = Σ x[n]·e^(-2πi·kn/len)`, unnormalised).
#[derive(Clone)]
pub(crate) struct Fft {
    len: usize,
    twiddle: Vec<(f64, f64)>,
    rev: Vec<usize>,
}

impl Fft {
    pub(crate) fn new(len: usize) -> Self {
        assert!(len.is_power_of_two() && len >= 2, "FFT length must be a power of two");
        let bits = len.trailing_zeros();
        let rev = (0..len).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let twiddle = (0..len / 2)
            .map(|k| {
                let a = -2.0 * PI * k as f64 / len as f64;
                (a.cos(), a.sin())
            })
            .collect();
        Self { len, twiddle, rev }
    }

    pub(crate) fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.len;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (c, s) = self.twiddle[k * step];
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
    }
}

/// atracdenc's `TMDCT<N>`: `n` inputs, `n/2` coefficients.
#[derive(Clone)]
pub(crate) struct Mdct {
    n: usize,
    /// cos, sin pairs (`n/4` of each, interleaved as in atracdenc).
    sincos: Vec<f64>,
    fft: Fft,
}

impl Mdct {
    pub(crate) fn new(n: usize, scale: f64) -> Self {
        let alpha = 2.0 * PI / (8.0 * n as f64);
        let omega = 2.0 * PI / n as f64;
        let scale = (scale / n as f64).sqrt();
        let mut sincos = vec![0.0; n / 2];
        for i in 0..n / 4 {
            sincos[2 * i] = scale * (omega * i as f64 + alpha).cos();
            sincos[2 * i + 1] = scale * (omega * i as f64 + alpha).sin();
        }
        Self { n, sincos, fft: Fft::new(n / 4) }
    }

    pub(crate) fn forward(&self, input: &[f32], out: &mut [f32]) {
        let n = self.n;
        let (n2, n4) = (n / 2, n / 4);
        let (n34, n54) = (3 * n4, 5 * n4);
        let x = |i: usize| f64::from(input[i]);
        let mut re = vec![0.0; n4];
        let mut im = vec![0.0; n4];
        let mut k = 0;
        while k < n4 {
            let r0 = x(n34 - 1 - k) + x(n34 + k);
            let i0 = x(n4 + k) - x(n4 - 1 - k);
            let (c, s) = (self.sincos[k], self.sincos[k + 1]);
            re[k / 2] = r0 * c + i0 * s;
            im[k / 2] = i0 * c - r0 * s;
            k += 2;
        }
        while k < n2 {
            let r0 = x(n34 - 1 - k) - x(k - n4);
            let i0 = x(n4 + k) + x(n54 - 1 - k);
            let (c, s) = (self.sincos[k], self.sincos[k + 1]);
            re[k / 2] = r0 * c + i0 * s;
            im[k / 2] = i0 * c - r0 * s;
            k += 2;
        }
        self.fft.run(&mut re, &mut im);
        let mut k = 0;
        while k < n2 {
            let (r0, i0) = (re[k / 2], im[k / 2]);
            let (c, s) = (self.sincos[k], self.sincos[k + 1]);
            out[k] = (-r0 * c - i0 * s) as f32;
            out[n2 - 1 - k] = (-r0 * s + i0 * c) as f32;
            k += 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The textbook MDCT `X[k] = Σ x[n]·cos(2π/N·(n + 1/2 + N/4)·(k + 1/2))`.
    fn direct(input: &[f32]) -> Vec<f64> {
        let n = input.len();
        (0..n / 2)
            .map(|k| {
                let s: f64 = input
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| f64::from(v) * (2.0 * PI / n as f64 * (i as f64 + 0.5 + n as f64 / 4.0) * (k as f64 + 0.5)).cos())
                    .sum();
                s
            })
            .collect()
    }

    #[test]
    fn is_the_direct_transform_over_n() {
        let input: Vec<f32> = (0..512).map(|i| ((i * 37 % 101) as f32 / 50.0 - 1.0) * 0.3).collect();
        let m = Mdct::new(512, 1.0);
        let mut out = vec![0.0; 256];
        m.forward(&input, &mut out);
        let d = direct(&input);
        // atracdenc's transform with scale 1 is the textbook one divided by ±N
        let ratio = f64::from(out[3]) / d[3];
        assert!((ratio.abs() * 512.0 - 1.0).abs() < 1e-4, "scale {ratio}");
        for (a, b) in out.iter().zip(&d) {
            assert!((f64::from(*a) - b * ratio).abs() < 1e-6, "{a} vs {}", b * ratio);
        }
    }
}
