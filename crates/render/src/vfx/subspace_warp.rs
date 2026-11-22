//! Warp Stabilizer, **Subspace Warp** method.
//!
//! A hand-held shot of a scene with depth moves different parts of the picture differently, so no
//! single homography can hold it still. Subspace Warp stabilizes it with a mesh:
//!
//! 1. **Analysis** — the clip is decoded small and long feature trajectories are followed through
//!    it ([`crate::trajectories`]).
//! 2. **Smoothing** — the trajectories are smoothed together in a low-dimensional subspace
//!    ([`crate::subspace`], after Liu et al., ACM TOG 2011), which keeps tracks at different depths
//!    consistent. This gives, for every frame, where each tracked point is and where the smoothed
//!    camera wants it.
//! 3. **Mesh** — per frame, a grid of control points is fitted to those pairs. Every control point
//!    takes a Gaussian-weighted local affine fit of the pairs around it, pulled towards the
//!    frame's global affine fit where tracks are sparse, so the warp is local where the clip has
//!    parallax and rigid where it has not.
//! 4. **Framing** — as the other methods: Stabilize Only, Crop, Crop + Auto-scale (the smallest
//!    zoom about the centre that hides every border the mesh exposes, within Maximum Scale),
//!    Synthesize Edges. Preserve Scale has no meaning for a mesh and is ignored.
//!
//! The analysis is kept in memory only for now.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cfa4_project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::subspace::{self, Params, Smoothed};
use crate::track::{Gray, Prepared};
use crate::trajectories::{self, Trajectories};

/// Control grid cells across and down.
const CELLS_X: usize = 16;
const CELLS_Y: usize = 12;

/// A mesh warp of one frame: for each node of a regular grid over the stabilized frame, where the
/// source picture has to be sampled (analysis pixels).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mesh {
    pub w: f64,
    pub h: f64,
    /// `(CELLS_X + 1) × (CELLS_Y + 1)` source positions, row-major.
    pub nodes: Vec<Vec2>,
}

impl Mesh {
    pub(crate) fn identity(w: f64, h: f64) -> Self {
        let nodes = (0..=CELLS_Y).flat_map(|j| (0..=CELLS_X).map(move |i| Vec2::new(w * i as f64 / CELLS_X as f64, h * j as f64 / CELLS_Y as f64))).collect();
        Self { w, h, nodes }
    }

    /// Source position for the stabilized position `p` (bilinear in the node grid; clamped to the
    /// grid outside).
    pub(crate) fn eval(&self, p: Vec2) -> Vec2 {
        let gx = (p.x / self.w * CELLS_X as f64).clamp(0.0, CELLS_X as f64 - 1e-9);
        let gy = (p.y / self.h * CELLS_Y as f64).clamp(0.0, CELLS_Y as f64 - 1e-9);
        let (i, j) = (gx.floor() as usize, gy.floor() as usize);
        let (tx, ty) = (gx - i as f64, gy - j as f64);
        let at = |i: usize, j: usize| self.nodes[j * (CELLS_X + 1) + i];
        let lerp = |a: Vec2, b: Vec2, t: f64| Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let (top, bot) = (lerp(at(i, j), at(i + 1, j), tx), lerp(at(i, j + 1), at(i + 1, j + 1), tx));
        // outside the frame the last cell is extended linearly
        let base = lerp(top, bot, ty);
        let ex = p.x / self.w * CELLS_X as f64 - gx;
        let ey = p.y / self.h * CELLS_Y as f64 - gy;
        if ex == 0.0 && ey == 0.0 {
            return base;
        }
        let dx = Vec2::new(at(i + 1, j).x - at(i, j).x, at(i + 1, j).y - at(i, j).y);
        let dy = Vec2::new(at(i, j + 1).x - at(i, j).x, at(i, j + 1).y - at(i, j).y);
        Vec2::new(base.x + dx.x * ex + dy.x * ey, base.y + dx.y * ex + dy.y * ey)
    }

    fn finite(&self) -> bool {
        self.nodes.iter().all(|n| n.x.is_finite() && n.y.is_finite())
    }
}

/// Solve `n x = r` for the symmetric 3×3 `n` and two right-hand sides; `None` when singular.
fn solve3(n: [[f64; 3]; 3], r: [[f64; 3]; 2]) -> Option<[[f64; 3]; 2]> {
    let det = n[0][0] * (n[1][1] * n[2][2] - n[1][2] * n[2][1]) - n[0][1] * (n[1][0] * n[2][2] - n[1][2] * n[2][0])
        + n[0][2] * (n[1][0] * n[2][1] - n[1][1] * n[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = [
        [(n[1][1] * n[2][2] - n[1][2] * n[2][1]) / det, (n[0][2] * n[2][1] - n[0][1] * n[2][2]) / det, (n[0][1] * n[1][2] - n[0][2] * n[1][1]) / det],
        [(n[1][2] * n[2][0] - n[1][0] * n[2][2]) / det, (n[0][0] * n[2][2] - n[0][2] * n[2][0]) / det, (n[0][2] * n[1][0] - n[0][0] * n[1][2]) / det],
        [(n[1][0] * n[2][1] - n[1][1] * n[2][0]) / det, (n[0][1] * n[2][0] - n[0][0] * n[2][1]) / det, (n[0][0] * n[1][1] - n[0][1] * n[1][0]) / det],
    ];
    let mut out = [[0.0; 3]; 2];
    for c in 0..2 {
        for i in 0..3 {
            out[c][i] = (0..3).map(|k| inv[i][k] * r[c][k]).sum();
        }
    }
    Some(out)
}

/// Weighted least-squares affine map `dst → src` (rows: x then y; columns: ∂/∂x, ∂/∂y, offset at
/// `centre`), pulled towards `prior` by `ridge` (relative to the total weight).
fn local_affine(pairs: &[(Vec2, Vec2)], centre: Vec2, sigma: f64, prior: Option<&[[f64; 3]; 2]>, ridge: f64) -> Option<[[f64; 3]; 2]> {
    let mut n = [[0.0; 3]; 3];
    let mut r = [[0.0; 3]; 2];
    let mut wsum = 0.0;
    for (src, dst) in pairs {
        let (ux, uy) = (dst.x - centre.x, dst.y - centre.y);
        let w = (-(ux * ux + uy * uy) / (2.0 * sigma * sigma)).exp();
        if w < 1e-6 {
            continue;
        }
        let row = [ux, uy, 1.0];
        for a in 0..3 {
            for b in 0..3 {
                n[a][b] += w * row[a] * row[b];
            }
            r[0][a] += w * row[a] * src.x;
            r[1][a] += w * row[a] * src.y;
        }
        wsum += w;
    }
    if wsum < 1e-9 {
        return None;
    }
    if let Some(p) = prior {
        // prior in centred form: Jacobian as is, offset = prior map at the centre
        let off = [p[0][0] * centre.x + p[0][1] * centre.y + p[0][2], p[1][0] * centre.x + p[1][1] * centre.y + p[1][2]];
        let lam = [ridge * wsum * sigma * sigma, ridge * wsum * sigma * sigma, ridge * wsum * 0.01];
        for a in 0..3 {
            n[a][a] += lam[a];
        }
        for c in 0..2 {
            r[c][0] += lam[0] * p[c][0];
            r[c][1] += lam[1] * p[c][1];
            r[c][2] += lam[2] * off[c];
        }
    }
    solve3(n, r)
}

/// The mesh of one frame from its `(source, stabilized)` point pairs.
pub(crate) fn fit_mesh(pairs: &[(Vec2, Vec2)], w: f64, h: f64) -> Mesh {
    let mesh = Mesh::identity(w, h);
    if pairs.len() < 6 {
        return mesh;
    }
    // global affine over every pair (centred at the frame centre, large sigma, no prior)
    let c = Vec2::new(w / 2.0, h / 2.0);
    let Some(g) = local_affine(pairs, c, 1e9, None, 0.0) else { return mesh };
    // the global map as a prior in absolute form: src = J·dst + t with t = offset − J·centre
    let prior = [[g[0][0], g[0][1], g[0][2] - g[0][0] * c.x - g[0][1] * c.y], [g[1][0], g[1][1], g[1][2] - g[1][0] * c.x - g[1][1] * c.y]];
    let sigma = 0.22 * w.max(h);
    let nodes: Vec<Vec2> = (0..mesh.nodes.len())
        .map(|k| {
            let v = Vec2::new(w * (k % (CELLS_X + 1)) as f64 / CELLS_X as f64, h * (k / (CELLS_X + 1)) as f64 / CELLS_Y as f64);
            match local_affine(pairs, v, sigma, Some(&prior), 0.25) {
                Some(a) => Vec2::new(a[0][2], a[1][2]),
                None => Vec2::new(prior[0][0] * v.x + prior[0][1] * v.y + prior[0][2], prior[1][0] * v.x + prior[1][1] * v.y + prior[1][2]),
            }
        })
        .collect();
    let m = Mesh { w, h, nodes };
    if m.finite() { m } else { mesh }
}

/// Smallest zoom about the centre (≥ 1) that hides the borders the mesh exposes, within `max`.
pub(crate) fn cover_scale(mesh: &Mesh, max: f64) -> f64 {
    let (w, h) = (mesh.w, mesh.h);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let covered = |z: f64| {
        let steps = 24;
        (0..=steps).all(|s| {
            let t = s as f64 / steps as f64;
            [(t * w, 0.0), (t * w, h), (0.0, t * h), (w, t * h)].iter().all(|&(x, y)| {
                let q = mesh.eval(Vec2::new(cx + (x - cx) / z, cy + (y - cy) / z));
                q.x >= -0.5 && q.y >= -0.5 && q.x <= w + 0.5 && q.y <= h + 0.5
            })
        })
    };
    if covered(1.0) {
        return 1.0;
    }
    let (mut lo, mut hi) = (1.0, max.max(1.0));
    if !covered(hi) {
        return hi;
    }
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if covered(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Resample `img` through `mesh` (analysis pixels, scaled by `s` to the layer) with the output
/// zoomed about its centre by `zoom`. Pixels whose source falls outside stay clear.
pub(crate) fn warp(img: &Image, mesh: &Mesh, s: f64, zoom: f64) -> Image {
    let (w, h) = (img.w, img.h);
    let mut out = Image::new(w, h);
    if w == 0 || h == 0 {
        return out;
    }
    let (cx, cy) = (mesh.w / 2.0, mesh.h / 2.0);
    out.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let a = Vec2::new((x as f64 + 0.5) / s, (y as f64 + 0.5) / s);
            let q = mesh.eval(Vec2::new(cx + (a.x - cx) / zoom, cy + (a.y - cy) / zoom));
            let (qx, qy) = (q.x * s, q.y * s);
            if qx < -1.0 || qy < -1.0 || qx > w as f64 + 1.0 || qy > h as f64 + 1.0 {
                continue;
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&img.sample_bilinear(qx as f32, qy as f32));
        }
    });
    out
}

/// Everything one setting of the effect needs.
struct Plan {
    meshes: Vec<Mesh>,
    cover_crop: f64,
    cover_auto: f64,
}

type TrajCache = Mutex<HashMap<u64, Arc<Trajectories>>>;
fn traj_cache() -> &'static TrajCache {
    static C: OnceLock<TrajCache> = OnceLock::new();
    C.get_or_init(Default::default)
}
type PlanCache = Mutex<HashMap<u64, Arc<Plan>>>;
fn plan_cache() -> &'static PlanCache {
    static C: OnceLock<PlanCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Forget every cached analysis and plan (called with `clear_stabilizer_cache`).
pub(super) fn clear_cache() {
    if let Ok(mut c) = traj_cache().lock() {
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

fn trajectories_of(env: &dyn FxEnv, now: f64, detailed: bool) -> Option<(Arc<Trajectories>, f64)> {
    let fps = env.frame_rate().max(1.0);
    let key = [0x5355_4253u64, env.clip_key(), detailed as u64].into_iter().fold(0xcbf2_9ce4_8422_2325, mix);
    if let Some(t) = traj_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Some((t, fps));
    }
    let n = ((env.clip_seconds() * fps).round() as usize).clamp(1, 20_000);
    let src_w = env.source_size().0.max(1) as f32;
    let scale = ((if detailed { 960.0 } else { 480.0 }) / src_w).min(1.0);
    let frames: Vec<Option<Prepared>> = (0..n)
        .into_par_iter()
        .map(|k| {
            if cfa4_media::cancel::cancelled() {
                return None;
            }
            env.source_frame(k as f64 / fps - now, scale).map(|f| Prepared::new(gray_of(&f)))
        })
        .collect();
    let tr = Arc::new(trajectories::analyse(&frames, &|| cfa4_media::cancel::cancelled())?);
    if let Ok(mut c) = traj_cache().lock() {
        if c.len() > 16 {
            c.clear();
        }
        c.insert(key, tr.clone());
    }
    Some((tr, fps))
}

fn plan(env: &dyn FxEnv, tr: &Trajectories, detailed: bool, smooth_frames: f64, no_motion: bool, max_scale: f64) -> Option<Arc<Plan>> {
    let key = [0x5355_504Cu64, env.clip_key(), detailed as u64, smooth_frames.to_bits(), no_motion as u64, max_scale.to_bits()]
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, mix);
    if let Some(p) = plan_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Some(p);
    }
    let window = if no_motion { tr.frames.min(90) } else { ((smooth_frames * 6.0).round() as usize).clamp(16, 90) };
    let sigma = if no_motion { window as f64 } else { smooth_frames.clamp(1.0, window as f64 / 3.0) };
    let smoothed: Smoothed = subspace::smooth(tr, &Params { window, rank: 6, sigma }).unwrap_or_else(|| Smoothed { frames: vec![Vec::new(); tr.frames] });
    let (aw, ah) = (tr.w as f64, tr.h as f64);
    let meshes: Vec<Mesh> = smoothed.frames.par_iter().map(|pairs| fit_mesh(pairs, aw, ah)).collect();
    let worst = |max: f64| meshes.par_iter().map(|m| cover_scale(m, max)).reduce(|| 1.0, f64::max);
    let p = Arc::new(Plan { cover_crop: worst(4.0), cover_auto: worst(max_scale), meshes });
    if let Ok(mut c) = plan_cache().lock() {
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, p.clone());
    }
    Some(p)
}

/// Warp Stabilizer with Method = Subspace Warp: see the module docs. Leaves the image unchanged
/// without an environment or when the clip can't be analysed.
pub fn warp_stabilizer(img: &mut Image, e: &EffectInstance, cx: &FxCtx<'_>) {
    let Some(env) = cx.env else { return };
    let detailed = bv(e, "detailed");
    let Some((tr, fps)) = trajectories_of(env, cx.seconds, detailed) else { return };
    let k = ((cx.seconds * fps).round().max(0.0) as usize).min(tr.frames - 1);
    let smooth = fv(e, "smoothness", cx) as f64 / 100.0 * fps;
    let crop_less = fv(e, "crop_less", cx) as f64 / 100.0;
    let max_scale = (fv(e, "max_scale", cx) as f64 / 100.0).max(1.0);
    let Some(plan) = plan(env, &tr, detailed, smooth * (0.5 + crop_less), chv(e, "result") == 1, max_scale) else { return };
    let framing = chv(e, "framing");
    let safe = 1.0 + fv(e, "action_safe", cx) as f64 / 100.0;
    let extra = fv(e, "additional_scale", cx) as f64 / 100.0;
    let zoom = match framing {
        2 => plan.cover_auto * safe,
        _ => 1.0,
    } * extra;
    let s = img.w as f64 / tr.w as f64;
    let (lw, lh) = (img.w as f64, img.h as f64);
    let mesh = &plan.meshes[k];
    let src = img.clone();
    *img = warp(&src, mesh, s, zoom);
    match framing {
        1 => {
            let (mx, my) = (lw * (1.0 - 1.0 / plan.cover_crop) / 2.0, lh * (1.0 - 1.0 / plan.cover_crop) / 2.0);
            crate::effects::crop(img, (mx / lw) as f32, (my / lh) as f32, (mx / lw) as f32, (my / lh) as f32, 0.0);
        }
        3 => {
            let (cx0, cy0) = (mesh.w / 2.0, mesh.h / 2.0);
            let w = img.w;
            img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                for x in 0..w {
                    let p = &mut row[x * 4..x * 4 + 4];
                    if p[3] < 0.999 {
                        let a = Vec2::new((x as f64 + 0.5) / s, (y as f64 + 0.5) / s);
                        let q = mesh.eval(Vec2::new(cx0 + (a.x - cx0) / zoom, cy0 + (a.y - cy0) / zoom));
                        let mut b = src.sample_bilinear_clamped((q.x * s) as f32, (q.y * s) as f32);
                        over(&mut b, [p[0], p[1], p[2], p[3]]);
                        p.copy_from_slice(&b);
                    }
                }
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_pairs(f: impl Fn(Vec2) -> Vec2) -> Vec<(Vec2, Vec2)> {
        (0..120).map(|i| Vec2::new(((i * 37) % 190) as f64 + 5.0, ((i * 61) % 120) as f64 + 4.0)).map(|d| (f(d), d)).collect()
    }

    #[test]
    fn mesh_without_motion_is_the_identity_and_a_shift_is_a_shift() {
        let id = fit_mesh(&grid_pairs(|d| d), 200.0, 128.0);
        for (a, b) in id.nodes.iter().zip(&Mesh::identity(200.0, 128.0).nodes) {
            assert!((*a - *b).length() < 1e-3, "{a:?} {b:?}");
        }
        let shift = fit_mesh(&grid_pairs(|d| Vec2::new(d.x + 6.0, d.y - 3.0)), 200.0, 128.0);
        assert!((shift.eval(Vec2::new(100.0, 64.0)) - Vec2::new(106.0, 61.0)).length() < 1e-3);
        assert_eq!(fit_mesh(&[], 200.0, 128.0), Mesh::identity(200.0, 128.0));
    }

    #[test]
    fn mesh_follows_a_depth_dependent_field_better_than_one_affine() {
        // the left half moves 8 px, the right half 2 px (parallax)
        let f = |d: Vec2| Vec2::new(d.x + if d.x < 100.0 { 8.0 } else { 2.0 }, d.y);
        let pairs: Vec<(Vec2, Vec2)> = (0..400).map(|i| Vec2::new(((i * 37) % 196) as f64 + 2.0, ((i * 61) % 124) as f64 + 2.0)).map(|d| (f(d), d)).collect();
        let mesh = fit_mesh(&pairs, 200.0, 128.0);
        let err = |p: Vec2| (mesh.eval(p) - f(p)).length();
        assert!(err(Vec2::new(30.0, 60.0)) < 2.0 && err(Vec2::new(170.0, 60.0)) < 2.0, "{} {}", err(Vec2::new(30.0, 60.0)), err(Vec2::new(170.0, 60.0)));
    }

    #[test]
    fn cover_scale_grows_with_the_shift_and_warp_moves_the_picture() {
        let id = Mesh::identity(64.0, 48.0);
        assert_eq!(cover_scale(&id, 3.0), 1.0);
        let far = Mesh { nodes: Mesh::identity(64.0, 48.0).nodes.iter().map(|n| Vec2::new(n.x + 30.0, n.y)).collect(), ..id.clone() };
        assert_eq!(cover_scale(&far, 3.0), 3.0);
        let near = Mesh { nodes: id.nodes.iter().map(|n| Vec2::new(n.x + 3.0, n.y)).collect(), ..id.clone() };
        let z = cover_scale(&near, 3.0);
        assert!(z > 1.0 && z < 1.5, "{z}");
        // warp: a mesh shifting the source by +4 px moves the picture 4 px left
        let mut img = Image::new(64, 48);
        for y in 0..48 {
            for x in 0..64 {
                img.px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&[x as f32 / 64.0, 0.0, 0.0, 1.0]);
            }
        }
        let out = warp(&img, &Mesh { nodes: id.nodes.iter().map(|n| Vec2::new(n.x + 4.0, n.y)).collect(), ..id }, 1.0, 1.0);
        assert!((out.get(20, 10)[0] - 24.0 / 64.0).abs() < 1e-4);
        assert_eq!(out.get(62, 10)[3], 0.0);
    }
}
