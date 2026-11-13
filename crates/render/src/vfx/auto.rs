//! Auto Color, Auto Contrast and Auto Levels: automatic tonal corrections from the frame's own
//! histogram, in display (sRGB-encoded) space.
//!
//! - **Auto Contrast** stretches all three channels by the same black and white points, taken from
//!   the luma histogram, so it changes contrast without adding or removing a colour cast.
//! - **Auto Levels** stretches each channel by its own black and white points; a cast that lives
//!   in the shadows or highlights goes away, and an unusual colour can gain one.
//! - **Auto Color** takes the black and white points from the darkest and brightest pixels by luma
//!   and uses *their* average colours as the per-channel points, so the shadows and highlights
//!   become neutral. Snap Neutral Midtones also bends each channel's gamma until the near-grey
//!   midtones are grey.
//!
//! Black Clip / White Clip are the share of pixels that may clip at either end. Temporal
//! Smoothing averages the correction over the neighbouring frames (at most [`MAX_SIDE`] each side,
//! evenly spaced across the window) so it does not pump; Scene Detect stops the window at a cut.
//! The analysis looks at a strided sample of at most [`MAX_SAMPLES`] pixels.

use cfa4_color::luma709;
use cfa4_project::EffectInstance;

use super::*;
use crate::effects::{dec, enc};

const BINS: usize = 1024;
/// Analysis sample cap (pixels).
const MAX_SAMPLES: usize = 1 << 18;
/// Black-to-white ranges narrower than this are left alone (caps the stretch at 20×, so a flat
/// frame does not turn its noise into full-range snow).
const MIN_RANGE: f32 = 0.05;
/// Neighbour frames read on each side for Temporal Smoothing.
const MAX_SIDE: usize = 4;
/// L1 distance between two normalised 32-bin luma histograms (0–2) that counts as a scene cut.
const SCENE_CUT: f32 = 0.6;
/// Snap Neutral Midtones: a midtone whose channels spread less than this counts as a tinted grey.
const NEAR_GREY: f32 = 0.25;
/// Snap Neutral Midtones: per-channel gamma limits.
const GAMMA_RANGE: (f32, f32) = (0.5, 2.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Color,
    Contrast,
    Levels,
}

/// Per-channel correction: `((v - lo) / (hi - lo)).clamp(0, 1) ^ g` in display space.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Map {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub g: [f32; 3],
}

impl Map {
    pub const IDENTITY: Map = Map { lo: [0.0; 3], hi: [1.0; 3], g: [1.0; 3] };

    #[inline]
    fn channel(&self, c: usize, v: f32) -> f32 {
        let (Some(&lo), Some(&hi), Some(&g)) = (self.lo.get(c), self.hi.get(c), self.g.get(c)) else { return v };
        let r = hi - lo;
        if r < MIN_RANGE {
            return v;
        }
        let t = ((v - lo) / r).clamp(0.0, 1.0);
        if (g - 1.0).abs() < 1e-6 { t } else { t.powf(g) }
    }

    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        [self.channel(0, c[0]), self.channel(1, c[1]), self.channel(2, c[2])]
    }

    fn mean(maps: &[Map]) -> Map {
        if maps.is_empty() {
            return Map::IDENTITY;
        }
        let n = maps.len() as f32;
        let mut m = Map { lo: [0.0; 3], hi: [0.0; 3], g: [0.0; 3] };
        for x in maps {
            for c in 0..3 {
                m.lo[c] += x.lo[c] / n;
                m.hi[c] += x.hi[c] / n;
                m.g[c] += x.g[c] / n;
            }
        }
        m
    }
}

/// The analysis controls.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    pub mode: Mode,
    /// Fractions (0–0.1) of the pixels allowed to clip to black / white.
    pub black_clip: f32,
    pub white_clip: f32,
    pub snap: bool,
}

/// Display-space colour and weight (alpha) of the sampled pixels.
fn samples(img: &Image) -> Vec<([f32; 3], f32)> {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() != w.saturating_mul(h).saturating_mul(4) {
        return Vec::new();
    }
    let area = w.saturating_mul(h);
    let step = ((area as f64 / MAX_SAMPLES as f64).sqrt().ceil() as usize).max(1);
    let mut out = Vec::with_capacity((w / step + 1).saturating_mul(h / step + 1).min(MAX_SAMPLES * 2));
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * 4;
            let Some(p) = img.px.get(i..i + 4) else { continue };
            let a = p[3];
            if !a.is_finite() || a <= 1e-3 {
                continue;
            }
            let c = enc(Image::unpremul([p[0], p[1], p[2], a]));
            if c.iter().all(|v| v.is_finite()) {
                out.push((c, a.min(1.0)));
            }
        }
    }
    out
}

#[inline]
fn bin(v: f32) -> usize {
    ((v.clamp(0.0, 1.0) * BINS as f32) as usize).min(BINS - 1)
}

/// Histograms of R, G, B and luma.
fn histograms(s: &[([f32; 3], f32)]) -> ([Vec<f32>; 4], f32) {
    let mut h = [vec![0.0f32; BINS], vec![0.0; BINS], vec![0.0; BINS], vec![0.0; BINS]];
    let mut total = 0.0;
    for (c, a) in s {
        let l = luma709(c[0], c[1], c[2]);
        for (k, v) in [c[0], c[1], c[2], l].into_iter().enumerate() {
            if let Some(slot) = h.get_mut(k).and_then(|hk| hk.get_mut(bin(v))) {
                *slot += a;
            }
        }
        total += a;
    }
    (h, total)
}

/// Black and white points of a histogram: the values below / above which `black` / `white` of
/// the weight lies.
fn clip_points(h: &[f32], total: f32, black: f32, white: f32) -> (f32, f32) {
    let (bt, wt) = (black * total, white * total);
    let mut acc = 0.0;
    let mut lo = 0.0;
    for (i, v) in h.iter().enumerate() {
        acc += v;
        if acc > bt {
            lo = i as f32 / BINS as f32;
            break;
        }
    }
    acc = 0.0;
    let mut hi = 1.0;
    for (i, v) in h.iter().enumerate().rev() {
        acc += v;
        if acc > wt {
            hi = (i + 1) as f32 / BINS as f32;
            break;
        }
    }
    (lo, hi.max(lo))
}

/// The correction one frame asks for.
pub(crate) fn analyse(img: &Image, s: Settings) -> Map {
    let px = samples(img);
    let (h, total) = histograms(&px);
    if total <= 0.0 {
        return Map::IDENTITY;
    }
    let black = s.black_clip.clamp(0.0, 0.1);
    let white = s.white_clip.clamp(0.0, 0.1);
    let luma = h.get(3).map(|v| clip_points(v, total, black, white)).unwrap_or((0.0, 1.0));
    let mut m = match s.mode {
        Mode::Contrast => Map { lo: [luma.0; 3], hi: [luma.1; 3], g: [1.0; 3] },
        Mode::Levels => {
            let mut m = Map::IDENTITY;
            for c in 0..3 {
                if let Some(hc) = h.get(c) {
                    let (lo, hi) = clip_points(hc, total, black, white);
                    m.lo[c] = lo;
                    m.hi[c] = hi;
                }
            }
            m
        }
        Mode::Color => {
            // the average colour of the darkest and brightest pixels becomes black and white
            let near = 1.0 / BINS as f32;
            let (mut dark, mut dw, mut bright, mut bw) = ([0.0f32; 3], 0.0f32, [0.0f32; 3], 0.0f32);
            for (c, a) in &px {
                let l = luma709(c[0], c[1], c[2]);
                if l < luma.0 + near {
                    for k in 0..3 {
                        dark[k] += c[k] * a;
                    }
                    dw += a;
                }
                if l >= luma.1 - near {
                    for k in 0..3 {
                        bright[k] += c[k] * a;
                    }
                    bw += a;
                }
            }
            let lo = if dw > 0.0 { dark.map(|v| (v / dw).clamp(0.0, 1.0)) } else { [luma.0; 3] };
            let hi = if bw > 0.0 { bright.map(|v| (v / bw).clamp(0.0, 1.0)) } else { [luma.1; 3] };
            Map { lo, hi, g: [1.0; 3] }
        }
    };
    if s.mode == Mode::Color && s.snap {
        m.g = neutral_gammas(&px, &m);
    }
    m
}

/// Gammas that make the average near-grey midtone (after the stretch) grey.
fn neutral_gammas(px: &[([f32; 3], f32)], m: &Map) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let (mut wsum, mut total) = (0.0f32, 0.0f32);
    for (c, a) in px {
        total += a;
        let s = m.apply(*c);
        let l = luma709(s[0], s[1], s[2]);
        let spread = s[0].max(s[1]).max(s[2]) - s[0].min(s[1]).min(s[2]);
        if (0.2..=0.8).contains(&l) && spread < NEAR_GREY {
            for k in 0..3 {
                sum[k] += s[k] * a;
            }
            wsum += a;
        }
    }
    // too few neutral pixels to judge (under 0.5 % of the frame): leave the midtones alone
    if wsum <= 0.005 * total || wsum <= 0.0 {
        return [1.0; 3];
    }
    let mean = sum.map(|v| v / wsum);
    let target = (mean[0] + mean[1] + mean[2]) / 3.0;
    mean.map(|v| {
        if v > 0.01 && v < 0.99 && target > 0.01 && target < 0.99 {
            let g = target.ln() / v.ln();
            if g.is_finite() { g.clamp(GAMMA_RANGE.0, GAMMA_RANGE.1) } else { 1.0 }
        } else {
            1.0
        }
    })
}

/// Coarse normalised luma histogram (scene-cut test).
fn coarse(img: &Image) -> [f32; 32] {
    let mut out = [0.0f32; 32];
    let px = samples(img);
    let mut total = 0.0;
    for (c, a) in &px {
        let l = luma709(c[0], c[1], c[2]).clamp(0.0, 1.0);
        let i = ((l * 32.0) as usize).min(31);
        if let Some(slot) = out.get_mut(i) {
            *slot += a;
        }
        total += a;
    }
    if total > 0.0 {
        for v in &mut out {
            *v /= total;
        }
    }
    out
}

fn distance(a: &[f32; 32], b: &[f32; 32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum()
}

fn settings(e: &EffectInstance, cx: &FxCtx<'_>, mode: Mode) -> Settings {
    Settings {
        mode,
        black_clip: fv(e, "black_clip", cx) / 100.0,
        white_clip: fv(e, "white_clip", cx) / 100.0,
        snap: mode == Mode::Color && bv(e, "snap_neutral"),
    }
}

/// The correction for this frame, averaged over the Temporal Smoothing window.
fn correction(img: &Image, e: &EffectInstance, cx: &FxCtx<'_>, s: Settings) -> Map {
    let here = analyse(img, s);
    let span = fv(e, "smoothing", cx).clamp(0.0, 10.0) as f64;
    let Some(env) = cx.env else { return here };
    if span.is_nan() || span <= 1e-3 {
        return here;
    }
    let fps = env.frame_rate();
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    let n = ((span * fps).ceil() as usize).clamp(1, MAX_SIDE);
    let dt = span / n as f64;
    let scene = bv(e, "scene_detect");
    let mut maps = vec![here];
    let first = if scene { Some(coarse(img)) } else { None };
    for dir in [-1.0f64, 1.0] {
        let mut prev = first;
        for k in 1..=n {
            let Some(f) = env.frame(dir * dt * k as f64, e) else { break };
            if let Some(p) = prev {
                let c = coarse(&f);
                if distance(&p, &c) > SCENE_CUT {
                    break;
                }
                prev = Some(c);
            }
            maps.push(analyse(&f, s));
        }
    }
    Map::mean(&maps)
}

/// Auto Color / Auto Contrast / Auto Levels.
pub fn auto_correct(img: &mut Image, e: &EffectInstance, cx: &FxCtx<'_>, mode: Mode) {
    let s = settings(e, cx, mode);
    let m = correction(img, e, cx, s);
    let keep = (fv(e, "blend", cx) / 100.0).clamp(0.0, 1.0);
    if m == Map::IDENTITY || keep >= 1.0 {
        return;
    }
    img.map_rgb(|c, _, _| {
        let d = enc(c);
        let o = m.apply(d);
        let mixed = [o[0] + (d[0] - o[0]) * keep, o[1] + (d[1] - o[1]) * keep, o[2] + (d[2] - o[2]) * keep];
        dec(mixed)
    });
}
