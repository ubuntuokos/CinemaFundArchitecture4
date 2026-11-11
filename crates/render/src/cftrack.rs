//! Correlation-filter object tracker: the optional **Correlation** mode of `masks.track`.
//!
//! The default mask tracker ([`crate::track`]) follows Shi–Tomasi corners inside the mask; it is
//! exact on textured, rigid content but loses an object with little interior detail (a face in
//! soft light, a plain shirt, a car body) or one that deforms. This tracker follows the object's
//! *appearance* instead, the way the classic single-object trackers (KCF, CSRT) do. Implemented
//! from the papers' equations, no code reused:
//!
//! 1. **Model** — a kernelized correlation filter (Henriques, Caseiro, Martins & Batista, "High-
//!    Speed Tracking with Kernelized Correlation Filters", TPAMI 2015): ridge regression over every
//!    cyclic shift of a window 2.5× the mask's bounding box, solved in the Fourier domain with a
//!    Gaussian kernel (σ 0.5), λ 10⁻⁴ and a Gaussian label (σ 0.1 × target size in cells).
//! 2. **Features** — 4×4-pixel cells, 9 unsigned gradient-orientation bins with linear bin
//!    interpolation, L2-normalised per cell (HOG-like, Dalal & Triggs 2005) plus the cell's mean
//!    intensity: 10 channels on a power-of-two grid (16–64 cells on the long side, a cell ≈ 4 px).
//! 3. **Spatial reliability** — after CSRT (Lukežič et al., "Discriminative Correlation Filter
//!    with Channel and Spatial Reliability", CVPR 2017), the filter is not trained on the whole
//!    window: the Hann window is multiplied by a reliability map built from the mask's own shape
//!    (1 inside, falling off outside), so the background around the object counts less.
//! 4. **Scale** — a separate one-dimensional correlation filter over 16 scales 2 % apart, after
//!    DSST (Danelljan, Häger, Khan & Felsberg, "Accurate Scale Estimation for Robust Visual
//!    Tracking", BMVC 2014): the target box is sampled at each scale into an 8×8-cell feature
//!    vector, and the filter's peak (refined by a parabola) gives the scale change. Translation is
//!    found first at the current scale, then the scale at the new position.
//! 5. **Confidence** — the response's peak-to-sidelobe ratio (Bolme et al., "MOSSE", CVPR 2010).
//!    Below [`LOST_PSR`] the step reports the object as lost (occluded, out of frame); below
//!    [`UPDATE_PSR`] the model is not updated so an occluder is not learnt.
//!
//! The tracker estimates position and (optionally) uniform scale; it does not estimate rotation.
//! Draw the mask around the whole object: the filter follows its outline and look. It is
//! stateful: create it on the first frame with [`CorrTracker::new`] and feed it the next
//! frames in tracking order with [`CorrTracker::update`].

use cfa4_geom::{Affine, Vec2};

use crate::track::{Gray, dist_to_poly, point_in};

const CELL: usize = 4;
const BINS: usize = 9;
const CHANNELS: usize = BINS + 1;
/// Window size relative to the target's bounding box.
const PADDING: f64 = 2.5;
/// Feature cells on the window's long side: a power of two near the window's long side / [`CELL`]
/// (so one cell ≈ 4 frame pixels), within these bounds.
const GRID_MIN: usize = 16;
const GRID_MAX: usize = 64;
const LAMBDA: f32 = 1e-4;
const KERNEL_SIGMA: f32 = 0.5;
const LABEL_SIGMA: f64 = 0.1;
/// Model interpolation factor per frame.
const LEARN: f32 = 0.075;
/// Scale filter: scales sampled (a power of two), their ratio, the label width (in steps), the
/// sample grid (cells per side) and its learning rate.
const N_SCALES: usize = 16;
const SCALE_STEP: f64 = 1.02;
const SCALE_SIGMA: f64 = 1.0;
const SCALE_GRID: usize = 8;
const SCALE_LAMBDA: f32 = 1e-2;
const LEARN_SCALE: f32 = 0.025;
/// Peak-to-sidelobe ratio below which the object counts as lost.
pub const LOST_PSR: f32 = 6.0;
/// Peak-to-sidelobe ratio below which the model is not updated.
pub const UPDATE_PSR: f32 = 10.0;
/// Smallest target side, in frame pixels.
const MIN_TARGET: f64 = 6.0;

// ------------------------------------------------------------------ complex FFT

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct C {
    re: f32,
    im: f32,
}

impl C {
    const ZERO: C = C { re: 0.0, im: 0.0 };
    #[inline]
    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    #[inline]
    fn mul(self, o: C) -> C {
        C::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
    #[inline]
    fn conj(self) -> C {
        C::new(self.re, -self.im)
    }
    #[inline]
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
    #[inline]
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
    #[inline]
    fn scale(self, s: f32) -> C {
        C::new(self.re * s, self.im * s)
    }
    /// `self / o`.
    #[inline]
    fn div(self, o: C) -> C {
        let d = o.re * o.re + o.im * o.im;
        if d <= f32::MIN_POSITIVE {
            return C::ZERO;
        }
        C::new((self.re * o.re + self.im * o.im) / d, (self.im * o.re - self.re * o.im) / d)
    }
}

/// In-place iterative radix-2 FFT (`buf.len()` a power of two); the inverse is scaled by 1/n.
fn fft1(buf: &mut [C], inverse: bool) {
    let n = buf.len();
    if n < 2 {
        return;
    }
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            buf.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = sign * std::f64::consts::TAU / len as f64;
        for k in 0..half {
            let (s, c) = (step * k as f64).sin_cos();
            let w = C::new(c as f32, s as f32);
            for i in (0..n).step_by(len) {
                let u = buf[i + k];
                let v = buf[i + k + half].mul(w);
                buf[i + k] = u.add(v);
                buf[i + k + half] = u.sub(v);
            }
        }
        len <<= 1;
    }
    if inverse {
        let s = 1.0 / n as f32;
        for v in buf.iter_mut() {
            *v = v.scale(s);
        }
    }
}

/// 2-D FFT of a `w`×`h` row-major grid.
fn fft2(data: &mut [C], w: usize, h: usize, inverse: bool) {
    for row in data.chunks_mut(w) {
        fft1(row, inverse);
    }
    let mut col = vec![C::ZERO; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = data[y * w + x];
        }
        fft1(&mut col, inverse);
        for y in 0..h {
            data[y * w + x] = col[y];
        }
    }
}

fn fft2_real(v: &[f32], w: usize, h: usize) -> Vec<C> {
    let mut d: Vec<C> = v.iter().map(|&r| C::new(r, 0.0)).collect();
    fft2(&mut d, w, h, false);
    d
}

// ------------------------------------------------------------------ tracker

/// One tracking step's result.
#[derive(Clone, Debug)]
pub struct CorrStep {
    /// Maps the previous frame's pixels to this frame's (translation and uniform scale about the
    /// target centre).
    pub transform: Affine,
    /// Peak-to-sidelobe ratio of the detection (higher is surer; see [`LOST_PSR`]).
    pub confidence: f32,
}

/// Multi-channel features of a window: `CHANNELS` planes of `gw * gh` cells.
type Features = Vec<Vec<f32>>;

/// A correlation-filter tracker following one object.
pub struct CorrTracker {
    gw: usize,
    gh: usize,
    frame: (usize, usize),
    center: Vec2,
    /// Window size (frame pixels) at scale 1.
    base_win: (f64, f64),
    scale: f64,
    /// Hann window × spatial reliability, per cell.
    window: Vec<f32>,
    yf: Vec<C>,
    model_xf: Vec<Vec<C>>,
    model_xx: f32,
    model_alphaf: Vec<C>,
    /// Target box (frame pixels) at scale 1.
    target: (f64, f64),
    scale_filter: Option<ScaleFilter>,
}

fn pow2_near(x: f64) -> usize {
    let e = x.max(1.0).log2().round() as u32;
    1usize << e.min(10)
}

impl CorrTracker {
    /// Start tracking the object inside `region` (a polygon in `frame`'s pixels). `None` when the
    /// region is degenerate or outside the frame.
    pub fn new(frame: &Gray, region: &[Vec2], estimate_scale: bool) -> Option<Self> {
        if region.len() < 3 || frame.w < 8 || frame.h < 8 || region.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return None;
        }
        let lo = region.iter().fold(Vec2::new(f64::INFINITY, f64::INFINITY), |a, p| Vec2::new(a.x.min(p.x), a.y.min(p.y)));
        let hi = region.iter().fold(Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY), |a, p| Vec2::new(a.x.max(p.x), a.y.max(p.y)));
        let center = (lo + hi) * 0.5;
        if center.x < 0.0 || center.y < 0.0 || center.x > frame.w as f64 || center.y > frame.h as f64 {
            return None;
        }
        let (bw, bh) = ((hi.x - lo.x).max(MIN_TARGET), (hi.y - lo.y).max(MIN_TARGET));
        let base_win = (bw * PADDING, bh * PADDING);
        let long = pow2_near(base_win.0.max(base_win.1) / CELL as f64).clamp(GRID_MIN, GRID_MAX);
        let (gw, gh) = if base_win.0 >= base_win.1 {
            (long, pow2_near(long as f64 * base_win.1 / base_win.0).clamp(8, long))
        } else {
            (pow2_near(long as f64 * base_win.0 / base_win.1).clamp(8, long), long)
        };
        // Hann window × reliability from the mask shape (in window coordinates).
        let feather = 0.15 * (bw * bw + bh * bh).sqrt();
        let hann = |i: usize, n: usize| 0.5 * (1.0 - (std::f64::consts::TAU * (i as f64 + 0.5) / n as f64).cos());
        let mut window = vec![0.0f32; gw * gh];
        for cy in 0..gh {
            for cx in 0..gw {
                let p = Vec2::new(center.x + ((cx as f64 + 0.5) / gw as f64 - 0.5) * base_win.0, center.y + ((cy as f64 + 0.5) / gh as f64 - 0.5) * base_win.1);
                let rel = if point_in(region, p.x, p.y) { 1.0 } else { (-dist_to_poly(region, p) / feather).exp() };
                window[cy * gw + cx] = (hann(cx, gw) * hann(cy, gh) * (0.3 + 0.7 * rel)) as f32;
            }
        }
        // Gaussian label peaked at the origin (circularly).
        let sigma = (gw as f64 * gh as f64).sqrt() / PADDING * LABEL_SIGMA;
        let mut y = vec![0.0f32; gw * gh];
        for cy in 0..gh {
            let dy = ((cy + gh / 2) % gh) as f64 - (gh / 2) as f64;
            for cx in 0..gw {
                let dx = ((cx + gw / 2) % gw) as f64 - (gw / 2) as f64;
                y[cy * gw + cx] = (-0.5 * (dx * dx + dy * dy) / (sigma * sigma)).exp() as f32;
            }
        }
        let yf = fft2_real(&y, gw, gh);
        let mut t = CorrTracker {
            gw,
            gh,
            frame: (frame.w, frame.h),
            center,
            base_win,
            scale: 1.0,
            window,
            yf,
            model_xf: Vec::new(),
            model_xx: 0.0,
            model_alphaf: Vec::new(),
            target: (bw, bh),
            scale_filter: estimate_scale.then(ScaleFilter::new),
        };
        if let Some(sf) = t.scale_filter.as_mut() {
            sf.learn(frame, center, (bw, bh), 1.0, 1.0);
        }
        let x = features(frame, center, base_win, gw, gh);
        let (xf, xx) = t.spectrum(&x);
        if !xx.is_finite() || xx <= 1e-9 {
            return None;
        }
        let alphaf = t.train(&xf, xx);
        t.model_xf = xf;
        t.model_xx = xx;
        t.model_alphaf = alphaf;
        Some(t)
    }

    /// The target's centre (frame pixels).
    pub fn center(&self) -> Vec2 {
        self.center
    }

    /// The target's scale relative to the first frame.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Find the object in the next frame. `None` when it is lost (low confidence) or the frame
    /// size changed.
    pub fn update(&mut self, frame: &Gray) -> Option<CorrStep> {
        if (frame.w, frame.h) != self.frame {
            return None;
        }
        let win = (self.base_win.0 * self.scale, self.base_win.1 * self.scale);
        let z = features(frame, self.center, win, self.gw, self.gh);
        let (zf, zz) = self.spectrum(&z);
        let kf = self.kernel(&self.model_xf, self.model_xx, &zf, zz);
        let mut r: Vec<C> = kf.iter().zip(&self.model_alphaf).map(|(k, a)| k.mul(*a)).collect();
        fft2(&mut r, self.gw, self.gh, true);
        let resp: Vec<f32> = r.iter().map(|c| c.re).collect();
        let (peak_i, peak) = resp.iter().copied().enumerate().fold((0, f32::NEG_INFINITY), |b, (i, v)| if v > b.1 { (i, v) } else { b });
        if !peak.is_finite() {
            return None;
        }
        let confidence = psr(&resp, self.gw, self.gh);
        if confidence.is_nan() || confidence < LOST_PSR {
            return None;
        }
        let (dx, dy) = self.subpixel(&resp, peak_i);
        let old = self.center;
        let new = Vec2::new(old.x + dx * win.0 / self.gw as f64, old.y + dy * win.1 / self.gh as f64);
        if !new.x.is_finite() || !new.y.is_finite() {
            return None;
        }
        self.center = new;
        let s = match self.scale_filter.as_ref() {
            Some(sf) => sf.detect(frame, new, self.target, self.scale),
            None => 1.0,
        };
        self.scale = (self.scale * s).clamp(0.05, 20.0);
        if confidence >= UPDATE_PSR {
            if let Some(sf) = self.scale_filter.as_mut() {
                sf.learn(frame, new, self.target, self.scale, LEARN_SCALE);
            }
            let win = (self.base_win.0 * self.scale, self.base_win.1 * self.scale);
            let x = features(frame, new, win, self.gw, self.gh);
            let (xf, xx) = self.spectrum(&x);
            if xx.is_finite() && xx > 1e-9 {
                let alphaf = self.train(&xf, xx);
                for (m, n) in self.model_alphaf.iter_mut().zip(&alphaf) {
                    *m = m.scale(1.0 - LEARN).add(n.scale(LEARN));
                }
                for (mc, nc) in self.model_xf.iter_mut().zip(&xf) {
                    for (m, n) in mc.iter_mut().zip(nc) {
                        *m = m.scale(1.0 - LEARN).add(n.scale(LEARN));
                    }
                }
                // ‖x‖² of the interpolated model, by Parseval.
                let n = (self.gw * self.gh) as f32;
                self.model_xx = self.model_xf.iter().flatten().map(|c| c.re * c.re + c.im * c.im).sum::<f32>() / n;
            }
        }
        let transform = Affine::translate(new.x, new.y).then_apply(&Affine::scale(s, s)).then_apply(&Affine::translate(-old.x, -old.y));
        Some(CorrStep { transform, confidence })
    }

    /// Per-channel spectra of windowed features and their total energy.
    fn spectrum(&self, f: &Features) -> (Vec<Vec<C>>, f32) {
        let mut xx = 0.0f32;
        let xf = f
            .iter()
            .map(|ch| {
                let w: Vec<f32> = ch.iter().zip(&self.window).map(|(v, w)| v * w).collect();
                xx += w.iter().map(|v| v * v).sum::<f32>();
                fft2_real(&w, self.gw, self.gh)
            })
            .collect();
        (xf, xx)
    }

    /// Fourier-domain Gaussian kernel correlation k(s) = κ(x, z shifted by s).
    fn kernel(&self, xf: &[Vec<C>], xx: f32, zf: &[Vec<C>], zz: f32) -> Vec<C> {
        let n = self.gw * self.gh;
        let mut acc = vec![C::ZERO; n];
        for (xc, zc) in xf.iter().zip(zf) {
            for ((a, x), z) in acc.iter_mut().zip(xc).zip(zc) {
                *a = a.add(x.conj().mul(*z));
            }
        }
        fft2(&mut acc, self.gw, self.gh, true);
        let denom = (n * CHANNELS) as f32;
        let k: Vec<f32> = acc.iter().map(|c| (-((xx + zz - 2.0 * c.re).max(0.0) / denom) / (KERNEL_SIGMA * KERNEL_SIGMA)).exp()).collect();
        fft2_real(&k, self.gw, self.gh)
    }

    fn train(&self, xf: &[Vec<C>], xx: f32) -> Vec<C> {
        let kf = self.kernel(xf, xx, xf, xx);
        self.yf.iter().zip(&kf).map(|(y, k)| y.div(k.add(C::new(LAMBDA, 0.0)))).collect()
    }

    /// Peak displacement in cells (signed, circular) with a parabolic sub-cell refinement.
    fn subpixel(&self, r: &[f32], i: usize) -> (f64, f64) {
        let (gw, gh) = (self.gw, self.gh);
        let (px, py) = (i % gw, i / gw);
        let at = |x: usize, y: usize| r[(y % gh) * gw + (x % gw)] as f64;
        // a parabola through the logs (exact for a Gaussian peak); plain values if any is ≤ 0
        let para = |l: f64, c: f64, r: f64| {
            let (l, c, r) = if l > 1e-9 && c > 1e-9 && r > 1e-9 { (l.ln(), c.ln(), r.ln()) } else { (l, c, r) };
            let d = l - 2.0 * c + r;
            if d.abs() < 1e-12 { 0.0 } else { (0.5 * (l - r) / d).clamp(-0.5, 0.5) }
        };
        let ox = para(at(px + gw - 1, py), at(px, py), at(px + 1, py));
        let oy = para(at(px, py + gh - 1), at(px, py), at(px, py + 1));
        let wrap = |p: usize, n: usize| if p > n / 2 { p as f64 - n as f64 } else { p as f64 };
        (wrap(px, gw) + ox, wrap(py, gh) + oy)
    }
}

/// Features of the window of size `win` (frame pixels) centred on `c`.
fn features(frame: &Gray, c: Vec2, win: (f64, f64), gw: usize, gh: usize) -> Features {
    let (pw, ph) = (gw * CELL, gh * CELL);
    // Patch, area-averaged when the window is larger than the patch.
    let (sx, sy) = (win.0 / pw as f64, win.1 / ph as f64);
    let sub = sx.max(sy).ceil().clamp(1.0, 4.0) as usize;
    let mut patch = vec![0.0f32; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            let mut acc = 0.0f32;
            for j in 0..sub {
                for i in 0..sub {
                    let fx = c.x + ((x as f64 + (i as f64 + 0.5) / sub as f64) / pw as f64 - 0.5) * win.0 - 0.5;
                    let fy = c.y + ((y as f64 + (j as f64 + 0.5) / sub as f64) / ph as f64 - 0.5) * win.1 - 0.5;
                    acc += frame.sample(fx as f32, fy as f32);
                }
            }
            patch[y * pw + x] = acc / (sub * sub) as f32;
        }
    }
    let at = |x: isize, y: isize| patch[(y.clamp(0, ph as isize - 1) as usize) * pw + x.clamp(0, pw as isize - 1) as usize];
    let mut out: Features = vec![vec![0.0f32; gw * gh]; CHANNELS];
    for y in 0..ph {
        for x in 0..pw {
            let (xi, yi) = (x as isize, y as isize);
            let gx = (at(xi + 1, yi) - at(xi - 1, yi)) * 0.5;
            let gy = (at(xi, yi + 1) - at(xi, yi - 1)) * 0.5;
            let mag = (gx * gx + gy * gy).sqrt();
            let cell = (y / CELL) * gw + x / CELL;
            out[BINS][cell] += patch[y * pw + x];
            if mag > 0.0 {
                let mut th = gy.atan2(gx);
                if th < 0.0 {
                    th += std::f32::consts::PI;
                }
                let b = th / std::f32::consts::PI * BINS as f32 - 0.5;
                let lo = b.floor();
                let frac = b - lo;
                let lo = (lo as isize).rem_euclid(BINS as isize) as usize;
                out[lo][cell] += mag * (1.0 - frac);
                out[(lo + 1) % BINS][cell] += mag * frac;
            }
        }
    }
    let area = (CELL * CELL) as f32;
    for cell in 0..gw * gh {
        let norm = ((0..BINS).map(|b| out[b][cell] * out[b][cell]).sum::<f32>() + 0.01).sqrt();
        for ch in out.iter_mut().take(BINS) {
            ch[cell] /= norm;
        }
        out[BINS][cell] = out[BINS][cell] / area - 0.5;
    }
    out
}

/// Signed scale exponent of circular index `i` (label peak at 0).
fn scale_exponent(i: usize) -> f64 {
    if i > N_SCALES / 2 { i as f64 - N_SCALES as f64 } else { i as f64 }
}

/// The DSST-style scale filter: per feature dimension, the numerator spectrum over scales, and
/// the shared denominator.
struct ScaleFilter {
    num: Vec<Vec<C>>,
    den: Vec<f32>,
    gf: Vec<C>,
    win: Vec<f32>,
}

impl ScaleFilter {
    fn new() -> Self {
        let mut g: Vec<C> = (0..N_SCALES).map(|i| C::new((-0.5 * scale_exponent(i).powi(2) / (SCALE_SIGMA * SCALE_SIGMA)).exp() as f32, 0.0)).collect();
        fft1(&mut g, false);
        let half = (N_SCALES / 2) as f64 + 1.0;
        let win = (0..N_SCALES).map(|i| (0.5 * (1.0 + (std::f64::consts::PI * scale_exponent(i) / half).cos())) as f32).collect();
        Self { num: Vec::new(), den: Vec::new(), gf: g, win }
    }

    /// Spectra over scales of every feature dimension of the target box at `scale` × each step.
    fn sample(&self, frame: &Gray, c: Vec2, target: (f64, f64), scale: f64) -> Vec<Vec<C>> {
        let dims = CHANNELS * SCALE_GRID * SCALE_GRID;
        let mut x = vec![vec![C::ZERO; N_SCALES]; dims];
        for i in 0..N_SCALES {
            let f = scale * SCALE_STEP.powf(scale_exponent(i));
            let feat = features(frame, c, (target.0 * f, target.1 * f), SCALE_GRID, SCALE_GRID);
            for (d, v) in feat.iter().flatten().enumerate() {
                x[d][i] = C::new(v * self.win[i], 0.0);
            }
        }
        for row in &mut x {
            fft1(row, false);
        }
        x
    }

    fn learn(&mut self, frame: &Gray, c: Vec2, target: (f64, f64), scale: f64, rate: f32) {
        let xf = self.sample(frame, c, target, scale);
        let num: Vec<Vec<C>> = xf.iter().map(|row| row.iter().zip(&self.gf).map(|(x, g)| g.conj().mul(*x)).collect()).collect();
        let mut den = vec![0.0f32; N_SCALES];
        for row in &xf {
            for (d, x) in den.iter_mut().zip(row) {
                *d += x.re * x.re + x.im * x.im;
            }
        }
        if self.num.is_empty() || rate >= 1.0 {
            self.num = num;
            self.den = den;
            return;
        }
        for (m, n) in self.num.iter_mut().flatten().zip(num.iter().flatten()) {
            *m = m.scale(1.0 - rate).add(n.scale(rate));
        }
        for (m, n) in self.den.iter_mut().zip(&den) {
            *m = *m * (1.0 - rate) + n * rate;
        }
    }

    /// The scale change at `c` relative to `scale`.
    fn detect(&self, frame: &Gray, c: Vec2, target: (f64, f64), scale: f64) -> f64 {
        let zf = self.sample(frame, c, target, scale);
        let mut r = vec![C::ZERO; N_SCALES];
        for (a, z) in self.num.iter().zip(&zf) {
            for (o, (a, z)) in r.iter_mut().zip(a.iter().zip(z)) {
                *o = o.add(a.conj().mul(*z));
            }
        }
        for (o, d) in r.iter_mut().zip(&self.den) {
            *o = o.scale(1.0 / (d + SCALE_LAMBDA));
        }
        fft1(&mut r, true);
        let v: Vec<f64> = r.iter().map(|c| c.re as f64).collect();
        let (i, peak) = v.iter().copied().enumerate().fold((0, f64::NEG_INFINITY), |b, (i, x)| if x > b.1 { (i, x) } else { b });
        if !peak.is_finite() {
            return 1.0;
        }
        let (l, rr) = (v[(i + N_SCALES - 1) % N_SCALES], v[(i + 1) % N_SCALES]);
        let d = l - 2.0 * peak + rr;
        let off = if d.abs() < 1e-12 { 0.0 } else { (0.5 * (l - rr) / d).clamp(-0.5, 0.5) };
        SCALE_STEP.powf(scale_exponent(i) + off)
    }
}

/// Peak-to-sidelobe ratio: (peak − mean) / std of the response outside a 7×7 cells around it.
fn psr(r: &[f32], gw: usize, gh: usize) -> f32 {
    let (i, peak) = r.iter().copied().enumerate().fold((0, f32::NEG_INFINITY), |b, (i, v)| if v > b.1 { (i, v) } else { b });
    let (px, py) = ((i % gw) as isize, (i / gw) as isize);
    let near = |x: usize, y: usize| {
        let dx = (x as isize - px).rem_euclid(gw as isize);
        let dy = (y as isize - py).rem_euclid(gh as isize);
        dx.min(gw as isize - dx) <= 3 && dy.min(gh as isize - dy) <= 3
    };
    let (mut s, mut s2, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for y in 0..gh {
        for x in 0..gw {
            if !near(x, y) {
                let v = r[y * gw + x] as f64;
                s += v;
                s2 += v * v;
                n += 1.0;
            }
        }
    }
    if n < 2.0 {
        return 0.0;
    }
    let mean = s / n;
    let std = (s2 / n - mean * mean).max(0.0).sqrt();
    if std < 1e-9 {
        return 0.0;
    }
    ((peak as f64 - mean) / std) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{Prepared, track_step};
    use cfa4_project::TrackMethod;

    fn hash(u: f64, v: f64, cell: f64) -> f64 {
        let (iu, iv) = ((u / cell).floor() as i64, (v / cell).floor() as i64);
        let mut x = (iu.wrapping_mul(73_856_093) ^ iv.wrapping_mul(19_349_663)) as u64;
        x ^= x >> 13;
        x = x.wrapping_mul(0x5bd1_e995);
        (x >> 40) as f64 / (1u64 << 24) as f64
    }

    /// Textured background with an object (`obj` gives its value in object coordinates, or
    /// `None` outside it) placed at `c` with scale `s`.
    fn frame(w: usize, h: usize, c: Vec2, s: f64, obj: &dyn Fn(Vec2) -> Option<f64>) -> Gray {
        let mut px = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let p = Vec2::new(x as f64 + ox - 0.5, y as f64 + oy - 0.5);
                    let o = (p - c) * (1.0 / s);
                    acc += obj(o).unwrap_or_else(|| 0.3 + 0.15 * hash(p.x + 700.0, p.y, 9.0) + 0.05 * ((p.x * 0.13).sin() * (p.y * 0.11).cos()));
                }
                px[y * w + x] = (acc / 4.0) as f32;
            }
        }
        Gray::new(w, h, px)
    }

    fn textured(o: Vec2) -> Option<f64> {
        (o.length() < 40.0).then(|| 0.55 + 0.3 * hash(o.x + 200.0, o.y + 300.0, 5.0) + 0.1 * (o.x * 0.3).sin())
    }

    /// A plain bright disc: no corners inside it at all.
    fn plain(o: Vec2) -> Option<f64> {
        (o.length() < 40.0).then_some(0.85)
    }

    fn circle(c: Vec2, r: f64) -> Vec<Vec2> {
        (0..32).map(|i| c + Vec2::new((i as f64 / 32.0 * std::f64::consts::TAU).cos() * r, (i as f64 / 32.0 * std::f64::consts::TAU).sin() * r)).collect()
    }

    fn path(k: usize) -> Vec2 {
        Vec2::new(110.0 + 3.0 * k as f64, 120.0 + 1.5 * k as f64 + 4.0 * (k as f64 * 0.4).sin())
    }

    #[test]
    fn fft_round_trip_and_delta() {
        let (w, h) = (8, 4);
        let v: Vec<f32> = (0..w * h).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut f = fft2_real(&v, w, h);
        fft2(&mut f, w, h, true);
        for (a, b) in v.iter().zip(&f) {
            assert!((a - b.re).abs() < 1e-5 && b.im.abs() < 1e-5);
        }
        let mut d = vec![0.0f32; w * h];
        d[0] = 1.0;
        assert!(fft2_real(&d, w, h).iter().all(|c| (c.re - 1.0).abs() < 1e-6 && c.im.abs() < 1e-6));
    }

    #[test]
    fn follows_a_moving_object() {
        let (w, h) = (320, 260);
        let f0 = frame(w, h, path(0), 1.0, &textured);
        let mut t = CorrTracker::new(&f0, &circle(path(0), 35.0), false).expect("tracker");
        let mut probe = path(0);
        let mut worst = 0.0f64;
        for k in 1..=25 {
            let st = t.update(&frame(w, h, path(k), 1.0, &textured)).expect("tracked");
            probe = st.transform.apply(probe);
            worst = worst.max((probe - path(k)).length());
        }
        assert!(worst < 1.5, "max error {worst} px");
    }

    #[test]
    fn follows_a_plain_object_the_point_tracker_loses() {
        let (w, h) = (320, 260);
        let region = circle(path(0), 35.0);
        let f0 = frame(w, h, path(0), 1.0, &plain);
        let f1 = frame(w, h, path(1), 1.0, &plain);
        // No corners inside the mask: the point tracker gives up.
        assert!(track_step(&Prepared::new(f0.clone()), &Prepared::new(f1), &region, TrackMethod::Position).is_none());
        let mut t = CorrTracker::new(&f0, &region, false).expect("tracker");
        let mut worst = 0.0f64;
        for k in 1..=25 {
            t.update(&frame(w, h, path(k), 1.0, &plain)).expect("tracked");
            worst = worst.max((t.center() - path(k)).length());
        }
        assert!(worst < 2.0, "max error {worst} px");
    }

    #[test]
    fn follows_a_turning_object() {
        let (w, h) = (320, 260);
        let rot = |k: usize| {
            let a = (1.5 * k as f64).to_radians();
            move |o: Vec2| textured(Vec2::new(o.x * a.cos() + o.y * a.sin(), -o.x * a.sin() + o.y * a.cos()))
        };
        let mut t = CorrTracker::new(&frame(w, h, path(0), 1.0, &rot(0)), &circle(path(0), 35.0), false).expect("tracker");
        let mut worst = 0.0f64;
        for k in 1..=20 {
            t.update(&frame(w, h, path(k), 1.0, &rot(k))).expect("tracked");
            worst = worst.max((t.center() - path(k)).length());
        }
        assert!(worst < 2.0, "max error {worst} px");
    }

    #[test]
    fn estimates_scale() {
        let (w, h) = (320, 260);
        let c = Vec2::new(160.0, 130.0);
        let sc = |k: usize| 1.0 + 0.015 * k as f64;
        let mut t = CorrTracker::new(&frame(w, h, c, 1.0, &textured), &circle(c, 35.0), true).expect("tracker");
        for k in 1..=20 {
            t.update(&frame(w, h, c, sc(k), &textured)).expect("tracked");
        }
        let rel = t.scale() / sc(20);
        assert!((rel - 1.0).abs() < 0.06, "scale {} vs {}", t.scale(), sc(20));
        assert!((t.center() - c).length() < 2.0, "centre {:?}", t.center());
    }

    #[test]
    fn reports_a_lost_object() {
        let (w, h) = (320, 260);
        let f0 = frame(w, h, path(0), 1.0, &textured);
        let mut t = CorrTracker::new(&f0, &circle(path(0), 35.0), false).expect("tracker");
        let gone = frame(w, h, path(1), 1.0, &|_| None);
        // the object left the picture: only background where it was
        assert!(t.update(&gone).is_none());
        let mut t = CorrTracker::new(&f0, &circle(path(0), 35.0), false).expect("tracker");
        let flat = Gray::new(w, h, vec![0.4; w * h]);
        assert!(t.update(&flat).is_none());
        // a different-size frame is refused, not misread
        assert!(t.update(&Gray::new(100, 100, vec![0.4; 100 * 100])).is_none());
    }

    #[test]
    fn rejects_degenerate_regions() {
        let f = Gray::new(64, 64, vec![0.5; 64 * 64]);
        assert!(CorrTracker::new(&f, &[Vec2::ZERO, Vec2::new(1.0, 1.0)], false).is_none());
        assert!(CorrTracker::new(&f, &circle(Vec2::new(-500.0, 30.0), 10.0), false).is_none());
        assert!(CorrTracker::new(&f, &[Vec2::new(f64::NAN, 0.0), Vec2::ZERO, Vec2::new(1.0, 1.0)], false).is_none());
    }
}
