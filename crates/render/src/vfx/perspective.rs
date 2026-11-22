//! Warp Stabilizer, **Perspective** method.
//!
//! The Position and Position, Scale, Rotation methods fit a similarity to each frame step; they
//! cannot undo the keystone and tilt a hand-held shot picks up when the camera pitches or yaws.
//! Perspective fits a full homography instead:
//!
//! 1. **Analysis** — the clip is decoded small and, for every frame step, the Shi–Tomasi features
//!    tracked with pyramidal Lucas–Kanade (the mask tracker in [`crate::track`]) are fitted with
//!    a RANSAC homography ([`crate::homography`]). A step whose homography is unsupported (fewer
//!    than [`homography::MIN_INLIERS`] agreeing points) or implausible (a corner moving more than
//!    a quarter of the frame in one frame) falls back to the similarity fit, and to no motion if
//!    that fails too.
//! 2. **Camera path** — the steps accumulate into each frame's pose relative to the first frame,
//!    written as where the first frame's four corners land (8 numbers). The path is smoothed
//!    with a Gaussian over those numbers (Smoothness, Crop Less ↔ Smooth More), or held at their
//!    mean (No Motion).
//! 3. **Correction** — the homography taking the four corners to their smoothed positions, after
//!    the inverse of the raw pose: current frame → smoothed frame.
//! 4. **Framing** — as the other methods: Stabilize Only, Crop, Crop + Auto-scale (the smallest
//!    zoom about the centre that hides every border the corrections expose, within Maximum
//!    Scale), Synthesize Edges (uncovered pixels from the clamped source).
//!
//! Preserve Scale has no meaning for a projective path and is ignored here. The analysis is kept
//! in memory only for now (the other methods' analysis is stored in the media cache).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cfa4_project::{EffectInstance, TrackMethod};
use rayon::prelude::*;

use super::*;
use crate::homography::{self, Homography};
use crate::track::{Gray, Prepared, track_pairs};

/// A clip's analysed camera motion: per-frame homographies (frame k → k+1) in analysis pixels.
#[derive(Clone, Debug)]
pub struct PerspectivePath {
    pub w: usize,
    pub h: usize,
    pub steps: Vec<Homography>,
    pub fps: f64,
}

/// The corrections of one setting of the effect and what framing needs from them.
struct Plan {
    corr: Vec<Homography>,
    /// Zoom that hides every border (cap 4×): Stabilize, Crop.
    cover_crop: f64,
    /// The same, capped at Maximum Scale: Stabilize, Crop, Auto-scale.
    cover_auto: f64,
}

type PathCache = Mutex<HashMap<u64, Arc<PerspectivePath>>>;
fn path_cache() -> &'static PathCache {
    static C: OnceLock<PathCache> = OnceLock::new();
    C.get_or_init(Default::default)
}
type PlanCache = Mutex<HashMap<u64, Arc<Plan>>>;
fn plan_cache() -> &'static PlanCache {
    static C: OnceLock<PlanCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Forget every cached analysis and plan (called with `clear_stabilizer_cache`).
pub(super) fn clear_cache() {
    if let Ok(mut c) = path_cache().lock() {
        c.clear();
    }
    if let Ok(mut c) = plan_cache().lock() {
        c.clear();
    }
}

fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x0100_0000_01b3)
}

fn gray_of(img: &Image) -> Gray {
    let px = img.px.par_chunks(4).map(|p| cfa4_color::linear_to_srgb(luma([p[0], p[1], p[2]]).clamp(0.0, 1.0))).collect();
    Gray::new(img.w, img.h, px)
}

/// The largest distance any frame corner moves under `h`, `None` if one leaves the plane.
fn corner_shift(h: &Homography, w: f64, hh: f64) -> Option<f64> {
    let mut worst = 0.0f64;
    for c in [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, hh), Vec2::new(0.0, hh)] {
        worst = worst.max((h.apply(c)? - c).length());
    }
    Some(worst)
}

/// One frame step from matched points: a homography when enough agree, else a similarity.
fn fit_step(src: &[Vec2], dst: &[Vec2], w: f64, h: f64) -> Homography {
    if let Some((hom, _)) = homography::fit_robust(src, dst)
        && corner_shift(&hom, w, h).is_some_and(|s| s <= 0.25 * w.min(h))
    {
        return hom;
    }
    crate::track::fit_robust(TrackMethod::PositionScaleRotation, src, dst).map(|(a, _)| Homography::from_affine(&a)).unwrap_or(Homography::IDENTITY)
}

fn perspective_path(env: &dyn FxEnv, now: f64, detailed: bool) -> Option<Arc<PerspectivePath>> {
    let key = [0x5045_5253u64, env.clip_key(), detailed as u64].into_iter().fold(0xcbf2_9ce4_8422_2325, mix);
    if let Some(p) = path_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Some(p);
    }
    let fps = env.frame_rate().max(1.0);
    let n = ((env.clip_seconds() * fps).round() as usize).clamp(1, 20_000);
    let src_w = env.source_size().0.max(1) as f32;
    let target_w = if detailed { 960.0 } else { 480.0 };
    let scale = (target_w / src_w).min(1.0);
    let frames: Vec<Option<Prepared>> = (0..n)
        .into_par_iter()
        .map(|k| {
            if cfa4_media::cancel::cancelled() {
                return None;
            }
            env.source_frame(k as f64 / fps - now, scale).map(|f| Prepared::new(gray_of(&f)))
        })
        .collect();
    if cfa4_media::cancel::cancelled() {
        return None;
    }
    let (w, h) = frames.iter().flatten().next().map(|p| (p.base().w, p.base().h))?;
    let m = 8.0;
    let region = [Vec2::new(m, m), Vec2::new(w as f64 - m, m), Vec2::new(w as f64 - m, h as f64 - m), Vec2::new(m, h as f64 - m)];
    let steps: Vec<Homography> = (0..n.saturating_sub(1))
        .into_par_iter()
        .map(|k| match (&frames[k], &frames[k + 1]) {
            (Some(a), Some(b)) if a.base().w == b.base().w => {
                track_pairs(a, b, &region).map_or(Homography::IDENTITY, |(src, dst, _)| fit_step(&src, &dst, w as f64, h as f64))
            }
            _ => Homography::IDENTITY,
        })
        .collect();
    let path = Arc::new(PerspectivePath { w, h, steps, fps });
    if let Ok(mut c) = path_cache().lock() {
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, path.clone());
    }
    Some(path)
}

/// Per-frame correction homographies (current frame px → stabilised px, analysis pixels).
pub(crate) fn corrections(path: &PerspectivePath, smooth_frames: f64, no_motion: bool) -> Vec<Homography> {
    let n = path.steps.len() + 1;
    let (w, h) = (path.w as f64, path.h as f64);
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)];
    let flat = |p: [Vec2; 4]| -> [f64; 8] { [p[0].x, p[0].y, p[1].x, p[1].y, p[2].x, p[2].y, p[3].x, p[3].y] };
    let unflat = |v: &[f64; 8]| [Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]), Vec2::new(v[4], v[5]), Vec2::new(v[6], v[7])];
    // pose of frame k relative to frame 0: cum[k] maps frame-0 pixels to frame-k pixels
    let mut cum = vec![Homography::IDENTITY; n];
    for k in 1..n {
        cum[k] = path.steps[k - 1].then(&cum[k - 1]);
    }
    let mut params: Vec<[f64; 8]> = Vec::with_capacity(n);
    for c in &cum {
        let mapped: Option<Vec<Vec2>> = corners.iter().map(|q| c.apply(*q).filter(|v| v.x.is_finite() && v.y.is_finite())).collect();
        let p = match mapped {
            Some(v) => flat([v[0], v[1], v[2], v[3]]),
            None => params.last().copied().unwrap_or_else(|| flat(corners)),
        };
        params.push(p);
    }
    let smoothed: Vec<[f64; 8]> = if no_motion {
        let mut mean = [0.0; 8];
        for p in &params {
            for (m, v) in mean.iter_mut().zip(p) {
                *m += v / n as f64;
            }
        }
        vec![mean; n]
    } else {
        let sigma = smooth_frames.max(0.01);
        let r = (sigma * 3.0).ceil() as isize;
        (0..n as isize)
            .map(|k| {
                let mut acc = [0.0; 8];
                let mut ws = 0.0;
                for j in -r..=r {
                    let i = (k + j).clamp(0, n as isize - 1) as usize;
                    let wgt = (-(j as f64).powi(2) / (2.0 * sigma * sigma)).exp();
                    for (a, v) in acc.iter_mut().zip(&params[i]) {
                        *a += v * wgt;
                    }
                    ws += wgt;
                }
                acc.map(|v| v / ws)
            })
            .collect()
    };
    (0..n)
        .map(|k| {
            let smooth_pose = Homography::from_points(&corners, &unflat(&smoothed[k]));
            let raw_inv = cum[k].inverse();
            match (smooth_pose, raw_inv) {
                (Some(sp), Some(ri)) => {
                    let c = sp.then(&ri);
                    if c.is_finite() { c } else { Homography::IDENTITY }
                }
                _ => Homography::IDENTITY,
            }
        })
        .collect()
}

/// Smallest zoom about the centre (≥ 1) that hides the borders a correction exposes.
pub(crate) fn cover_scale(corr: &Homography, w: f64, h: f64, max: f64) -> f64 {
    let Some(inv) = corr.inverse() else { return 1.0 };
    let (cx, cy) = (w / 2.0, h / 2.0);
    let covered = |z: f64| {
        [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().all(|&(x, y)| {
            inv.apply(Vec2::new(cx + (x - cx) / z, cy + (y - cy) / z)).is_some_and(|q| q.x >= -0.5 && q.y >= -0.5 && q.x <= w + 0.5 && q.y <= h + 0.5)
        })
    };
    if covered(1.0) {
        return 1.0;
    }
    let (mut lo, mut hi) = (1.0, max.max(1.0));
    if !covered(hi) {
        return hi;
    }
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if covered(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

fn plan(env: &dyn FxEnv, path: &PerspectivePath, detailed: bool, smooth_frames: f64, no_motion: bool, max_scale: f64) -> Arc<Plan> {
    let key = [0x504C_414Eu64, env.clip_key(), detailed as u64, smooth_frames.to_bits(), no_motion as u64, max_scale.to_bits()]
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, mix);
    if let Some(p) = plan_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return p;
    }
    let corr = corrections(path, smooth_frames, no_motion);
    let (aw, ah) = (path.w as f64, path.h as f64);
    let worst = |max: f64| corr.iter().map(|c| cover_scale(c, aw, ah, max)).fold(1.0, f64::max);
    let p = Arc::new(Plan { cover_crop: worst(4.0), cover_auto: worst(max_scale), corr });
    if let Ok(mut c) = plan_cache().lock() {
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, p.clone());
    }
    p
}

/// Warp Stabilizer with Method = Perspective: see the module docs. Leaves the image unchanged
/// without an environment or when the clip can't be analysed.
pub fn warp_stabilizer(img: &mut Image, e: &EffectInstance, cx: &FxCtx<'_>) {
    let Some(env) = cx.env else { return };
    let detailed = bv(e, "detailed");
    let Some(path) = perspective_path(env, cx.seconds, detailed) else { return };
    let n = path.steps.len() + 1;
    let k = ((cx.seconds * path.fps).round().max(0.0) as usize).min(n - 1);
    let smooth = fv(e, "smoothness", cx) as f64 / 100.0 * path.fps;
    let crop_less = fv(e, "crop_less", cx) as f64 / 100.0;
    let max_scale = (fv(e, "max_scale", cx) as f64 / 100.0).max(1.0);
    let plan = plan(env, &path, detailed, smooth * (0.5 + crop_less), chv(e, "result") == 1, max_scale);
    let framing = chv(e, "framing");
    let safe = 1.0 + fv(e, "action_safe", cx) as f64 / 100.0;
    let extra = fv(e, "additional_scale", cx) as f64 / 100.0;
    let zoom = match framing {
        2 => plan.cover_auto * safe,
        _ => 1.0,
    } * extra;
    // analysis px → layer px
    let s = img.w as f64 / path.w as f64;
    let (lw, lh) = (img.w as f64, img.h as f64);
    let z = Affine::translate(lw / 2.0, lh / 2.0).then_apply(&Affine::scale(zoom, zoom)).then_apply(&Affine::translate(-lw / 2.0, -lh / 2.0));
    let m = Homography::from_affine(&z).then(&plan.corr[k].conjugated_by_scale(s));
    let src = img.clone();
    *img = homography::warp(&src, &m);
    match framing {
        1 => {
            // crop to the area every frame covers
            let (mx, my) = (lw * (1.0 - 1.0 / plan.cover_crop) / 2.0, lh * (1.0 - 1.0 / plan.cover_crop) / 2.0);
            crate::effects::crop(img, (mx / lw) as f32, (my / lh) as f32, (mx / lw) as f32, (my / lh) as f32, 0.0);
        }
        3 => {
            // synthesise edges: fill uncovered pixels from the clamped source
            if let Some(inv) = m.inverse() {
                let w = img.w;
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let p = &mut row[x * 4..x * 4 + 4];
                        if p[3] < 0.999
                            && let Some(q) = inv.apply(Vec2::new(x as f64 + 0.5, y as f64 + 0.5))
                        {
                            let mut b = src.sample_bilinear_clamped(q.x as f32, q.y as f32);
                            over(&mut b, [p[0], p[1], p[2], p[3]]);
                            p.copy_from_slice(&b);
                        }
                    }
                });
            }
        }
        _ => {}
    }
}
