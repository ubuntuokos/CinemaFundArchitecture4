//! Subspace smoothing of feature trajectories: the second step of the Subspace Warp stabilizer
//! (after [`crate::trajectories`]), following Liu et al., "Subspace Video Stabilization",
//! ACM TOG 2011, with a sliding window instead of their whole-clip moving factorization.
//!
//! In every window of [`Params::window`] frames the trajectories that span it form a matrix with
//! one column per track and `2·window` rows (x then y per frame). Its dominant left singular
//! vectors `u_i` are the *eigen-trajectories*: the few time curves every track is a mixture of
//! (camera motion seen at different depths). Each `u_i` is smoothed in time with a Gaussian, and
//! each track is rebuilt from the smoothed curves with its own mixing weights `c`, found by least
//! squares on the frames where the track is actually alive. So
//!
//! * noise that is not shared between tracks is dropped (rank truncation),
//! * a track that is alive for only part of the window still gets a complete, consistent path,
//! * a track at another depth moves with its own mixture, which is what makes the result a
//!   3-D-aware correction and not one global transform.
//!
//! Windows overlap by half and are blended with triangular weights. When a window has too few
//! spanning tracks, its tracks are smoothed one by one instead. The singular vectors come from the
//! eigen-decomposition of the small `2·window × 2·window` Gram matrix (cyclic Jacobi).

use crate::trajectories::Trajectories;
use cfa4_geom::Vec2;
use rayon::prelude::*;

/// Smoothing settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Frames per factorization window (at least 8).
    pub window: usize,
    /// Eigen-trajectories kept (at most 12).
    pub rank: usize,
    /// Gaussian sigma of the path smoothing, in frames.
    pub sigma: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self { window: 40, rank: 6, sigma: 8.0 }
    }
}

/// The result: for every frame the positions of the tracks alive there, as they are (`src`) and
/// as the smoothed paths want them (`dst`). Warping `src → dst` stabilizes the frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Smoothed {
    pub frames: Vec<Vec<(Vec2, Vec2)>>,
}

fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    let r = (sigma * 3.0).ceil().max(1.0) as usize;
    let k: Vec<f64> = (0..=2 * r).map(|i| (-((i as f64 - r as f64).powi(2)) / (2.0 * sigma * sigma)).exp()).collect();
    let s: f64 = k.iter().sum();
    k.into_iter().map(|v| v / s).collect()
}

/// `v` smoothed with `kernel`, the ends extended by their last value.
fn smooth_1d(v: &[f64], kernel: &[f64]) -> Vec<f64> {
    let r = kernel.len() / 2;
    (0..v.len())
        .map(|i| {
            kernel
                .iter()
                .enumerate()
                .map(|(j, w)| {
                    let t = (i + j).saturating_sub(r).min(v.len() - 1);
                    w * v[t]
                })
                .sum()
        })
        .collect()
}

/// Eigenvalues (descending) and eigenvectors (rows) of the symmetric `n × n` matrix `a`
/// (cyclic Jacobi rotations).
fn eigen_sym(mut a: Vec<f64>, n: usize) -> (Vec<f64>, Vec<Vec<f64>>) {
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..60 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| a[i * n + j].powi(2)).sum();
        let diag: f64 = (0..n).map(|i| a[i * n + i].powi(2)).sum();
        if off <= 1e-22 * diag.max(1e-300) {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| a[y * n + y].total_cmp(&a[x * n + x]));
    let vals = order.iter().map(|&i| a[i * n + i]).collect();
    let vecs = order.iter().map(|&i| (0..n).map(|k| v[k * n + i]).collect()).collect();
    (vals, vecs)
}

/// Solve the small dense system `a x = b` (`a` is `n × n`, row-major) by Gaussian elimination
/// with partial pivoting; `None` when singular.
fn solve(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs()))?;
        if a[piv * n + col].abs() < 1e-12 {
            return None;
        }
        if piv != col {
            for k in 0..n {
                a.swap(piv * n + k, col * n + k);
            }
            b.swap(piv, col);
        }
        for r in col + 1..n {
            let f = a[r * n + col] / a[col * n + col];
            for k in col..n {
                a[r * n + k] -= f * a[col * n + k];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|k| a[i * n + k] * x[k]).sum();
        x[i] = (b[i] - s) / a[i * n + i];
    }
    Some(x)
}

/// One window's contribution: `(track index, frame, smoothed position, weight)`.
fn window_pass(tr: &Trajectories, a: usize, params: &Params, kernel: &[f64]) -> Vec<(usize, usize, Vec2, f64)> {
    let w = params.window.min(tr.frames - a);
    let b = a + w;
    let weight = |k: usize| ((k - a + 1).min(b - k)) as f64;
    let rank = params.rank.clamp(1, 12);
    let mut out = Vec::new();
    // the tracks that span the whole window build the basis
    let full: Vec<usize> = (0..tr.tracks.len()).filter(|&i| tr.tracks[i].start <= a && tr.tracks[i].end() >= b).collect();
    let basis = (full.len() >= rank * 3 && w >= 8).then(|| {
        let n = 2 * w;
        let mut gram = vec![0.0; n * n];
        let cols: Vec<Vec<f64>> = full
            .iter()
            .map(|&i| (a..b).map(|k| tr.tracks[i].at(k).map_or(0.0, |p| p.x)).chain((a..b).map(|k| tr.tracks[i].at(k).map_or(0.0, |p| p.y))).collect())
            .collect();
        for c in &cols {
            for r in 0..n {
                for s in r..n {
                    gram[r * n + s] += c[r] * c[s];
                }
            }
        }
        for r in 0..n {
            for s in 0..r {
                gram[r * n + s] = gram[s * n + r];
            }
        }
        let (_, vecs) = eigen_sym(gram, n);
        let r = rank.min(vecs.len());
        let raw: Vec<Vec<f64>> = vecs.into_iter().take(r).collect();
        let smooth: Vec<Vec<f64>> = raw.iter().map(|u| smooth_1d(&u[..w], kernel).into_iter().chain(smooth_1d(&u[w..], kernel)).collect()).collect();
        (raw, smooth)
    });
    for (i, t) in tr.tracks.iter().enumerate() {
        let (lo, hi) = (t.start.max(a), t.end().min(b));
        if hi <= lo || hi - lo < (w / 3).max(4) {
            continue;
        }
        let obs: Vec<usize> = (lo..hi).collect();
        let smoothed: Vec<Vec2> = match &basis {
            Some((raw, smooth)) => {
                let r = raw.len();
                // mixing weights on the observed frames: (Uᵀ U + εI) c = Uᵀ m
                let mut ata = vec![0.0; r * r];
                let mut atb = vec![0.0; r];
                for &k in &obs {
                    let p = t.at(k).unwrap_or_default();
                    for x in 0..r {
                        let (ux, uy) = (raw[x][k - a], raw[x][w + k - a]);
                        atb[x] += ux * p.x + uy * p.y;
                        for y in 0..r {
                            ata[x * r + y] += ux * raw[y][k - a] + uy * raw[y][w + k - a];
                        }
                    }
                }
                for x in 0..r {
                    ata[x * r + x] += 1e-6 * obs.len() as f64;
                }
                match solve(ata, atb, r) {
                    Some(c) => {
                        (a..b).map(|k| Vec2::new((0..r).map(|x| c[x] * smooth[x][k - a]).sum(), (0..r).map(|x| c[x] * smooth[x][w + k - a]).sum())).collect()
                    }
                    None => continue,
                }
            }
            None => {
                let xs: Vec<f64> = obs.iter().map(|&k| t.at(k).map_or(0.0, |p| p.x)).collect();
                let ys: Vec<f64> = obs.iter().map(|&k| t.at(k).map_or(0.0, |p| p.y)).collect();
                let (sx, sy) = (smooth_1d(&xs, kernel), smooth_1d(&ys, kernel));
                // placed on the whole window grid; frames outside the observed range stay unused
                (a..b).map(|k| if (lo..hi).contains(&k) { Vec2::new(sx[k - lo], sy[k - lo]) } else { Vec2::default() }).collect()
            }
        };
        for &k in &obs {
            out.push((i, k, smoothed[k - a], weight(k)));
        }
    }
    out
}

/// Smooth all trajectories. `None` for a clip with fewer than 8 frames or without tracks.
pub fn smooth(tr: &Trajectories, params: &Params) -> Option<Smoothed> {
    if tr.frames < 8 || tr.tracks.is_empty() {
        return None;
    }
    let params = Params { window: params.window.max(8), ..*params };
    let kernel = gaussian_kernel(params.sigma.max(0.5));
    let step = (params.window / 2).max(1);
    let mut starts: Vec<usize> = (0..tr.frames.saturating_sub(params.window / 2)).step_by(step).collect();
    if let Some(&last) = starts.last()
        && last + params.window < tr.frames
    {
        starts.push(tr.frames - params.window);
    }
    let parts: Vec<Vec<(usize, usize, Vec2, f64)>> = starts.par_iter().map(|&a| window_pass(tr, a, &params, &kernel)).collect();
    let mut acc: std::collections::HashMap<(usize, usize), (Vec2, f64)> = std::collections::HashMap::new();
    for (i, k, p, w) in parts.into_iter().flatten() {
        let e = acc.entry((i, k)).or_insert((Vec2::default(), 0.0));
        e.0 = Vec2::new(e.0.x + p.x * w, e.0.y + p.y * w);
        e.1 += w;
    }
    let mut frames = vec![Vec::new(); tr.frames];
    for (i, t) in tr.tracks.iter().enumerate() {
        for k in t.start..t.end() {
            if let Some(&(sum, w)) = acc.get(&(i, k)) {
                frames[k].push((t.pts[k - t.start], Vec2::new(sum.x / w, sum.y / w)));
            }
        }
    }
    Some(Smoothed { frames })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectories::Track;

    /// Deterministic pseudo-random test signal in -1..1 for (track, frame, channel).
    fn noise(track: u64, frame: u64, channel: u64) -> f64 {
        let x = (track as f64 * 12.9898 + frame as f64 * 78.233 + channel as f64 * 37.719).sin() * 43758.5453;
        (x - x.floor()) * 2.0 - 1.0
    }

    /// The smooth camera path and the shake on top of it (frame `k`).
    fn smooth_cam(k: f64) -> Vec2 {
        Vec2::new(0.8 * k, 12.0 * (k * 0.05).sin())
    }
    fn shake(k: usize) -> Vec2 {
        Vec2::new(3.0 * noise(0, k as u64, 1), 3.0 * noise(1, k as u64, 2))
    }

    /// Tracks of a scene with depth: each moves with `depth ×` the camera path. Half of them live
    /// only part of the clip. `jitter` is per-track measurement noise.
    fn scene(frames: usize, jitter: f64) -> Trajectories {
        let mut tracks = Vec::new();
        for i in 0..120u64 {
            let depth = 0.5 + (i % 7) as f64 * 0.25;
            let base = Vec2::new(20.0 + (i % 11) as f64 * 12.0, 15.0 + (i / 11) as f64 * 9.0);
            let (start, end) = if i % 2 == 0 { (0, frames) } else { ((i as usize * 3) % (frames / 3), frames / 2 + (i as usize * 5) % (frames / 2)) };
            let pts = (start..end)
                .map(|k| {
                    let c = smooth_cam(k as f64);
                    let s = shake(k);
                    Vec2::new(base.x + depth * (c.x + s.x) + jitter * noise(i, k as u64, 3), base.y + depth * (c.y + s.y) + jitter * noise(i, k as u64, 4))
                })
                .collect();
            tracks.push(Track { start, pts });
        }
        Trajectories { w: 200, h: 120, frames, tracks }
    }

    fn truth(i: usize, k: usize) -> Vec2 {
        let i = i as u64;
        let depth = 0.5 + (i % 7) as f64 * 0.25;
        let base = Vec2::new(20.0 + (i % 11) as f64 * 12.0, 15.0 + (i / 11) as f64 * 9.0);
        let c = smooth_cam(k as f64);
        Vec2::new(base.x + depth * c.x, base.y + depth * c.y)
    }

    fn rms(v: impl Iterator<Item = f64>) -> f64 {
        let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x * x, n + 1));
        (s / n.max(1) as f64).sqrt()
    }

    #[test]
    fn jacobi_finds_the_eigenpairs_of_a_symmetric_matrix() {
        let a = vec![4.0, 1.0, 0.0, 1.0, 3.0, 0.5, 0.0, 0.5, 1.0];
        let (vals, vecs) = eigen_sym(a.clone(), 3);
        assert!(vals[0] >= vals[1] && vals[1] >= vals[2]);
        for (l, v) in vals.iter().zip(&vecs) {
            for r in 0..3 {
                let av: f64 = (0..3).map(|c| a[r * 3 + c] * v[c]).sum();
                assert!((av - l * v[r]).abs() < 1e-9);
            }
        }
        assert!((vals.iter().sum::<f64>() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn smoothing_removes_shake_and_measurement_noise_and_keeps_the_camera_path() {
        let tr = scene(120, 0.4);
        let out = smooth(&tr, &Params { window: 40, rank: 4, sigma: 6.0 }).unwrap();
        assert_eq!(out.frames.len(), 120);
        // positions of (track, frame) in the output, in the same order as the tracks list them
        let mut raw_err = Vec::new();
        let mut new_err = Vec::new();
        for k in 0..120 {
            let alive: Vec<usize> = (0..tr.tracks.len()).filter(|&i| tr.tracks[i].at(k).is_some()).collect();
            // tracks missing from the output (too short in every window) are skipped
            let mut it = out.frames[k].iter();
            for &i in &alive {
                if let Some(&(src, dst)) = it.next() {
                    if (src.x - tr.tracks[i].at(k).unwrap().x).abs() > 1e-9 {
                        break;
                    }
                    let t = truth(i, k);
                    raw_err.push((src.x - t.x).hypot(src.y - t.y));
                    new_err.push((dst.x - t.x).hypot(dst.y - t.y));
                }
            }
        }
        assert!(new_err.len() > 4000, "{} samples", new_err.len());
        let (raw, new) = (rms(raw_err.into_iter()), rms(new_err.into_iter()));
        assert!(new < raw * 0.45, "smoothed error {new} vs raw {raw}");
    }

    #[test]
    fn partially_alive_tracks_get_a_consistent_path_and_tiny_inputs_are_refused() {
        let tr = scene(100, 0.0);
        let out = smooth(&tr, &Params::default()).unwrap();
        // every track alive in the middle frame has a destination there, also the short ones
        let alive = tr.alive(50);
        assert!(out.frames[50].len() as f64 >= alive as f64 * 0.9, "{} of {alive}", out.frames[50].len());
        assert!(smooth(&Trajectories { w: 10, h: 10, frames: 4, tracks: Vec::new() }, &Params::default()).is_none());
        assert!(smooth(&Trajectories { w: 10, h: 10, frames: 50, tracks: Vec::new() }, &Params::default()).is_none());
    }

    #[test]
    fn too_few_spanning_tracks_fall_back_to_per_track_smoothing() {
        let mut tr = scene(60, 0.0);
        tr.tracks.truncate(6);
        let out = smooth(&tr, &Params { window: 30, rank: 4, sigma: 5.0 }).unwrap();
        let moved: Vec<f64> = out.frames[30].iter().map(|(s, d)| (s.x - d.x).hypot(s.y - d.y)).collect();
        assert!(!moved.is_empty() && moved.iter().all(|m| m.is_finite() && *m < 8.0));
    }
}
