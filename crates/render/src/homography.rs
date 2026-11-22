//! Planar homographies: the projective 2-D transform (3×3, eight degrees of freedom) that maps
//! one view of a plane to another. Warp Stabilizer's Perspective method fits one per frame step
//! and warps with it.
//!
//! - **Fit** — the direct linear transform with Hartley's point normalisation (translate the
//!   centroid to the origin, scale the mean distance to √2; Hartley & Zisserman, *Multiple View
//!   Geometry*, §4.4), solved by the 8×8 normal equations with `h33 = 1` after normalisation.
//! - **Robust fit** — RANSAC over 4-point samples (Fischler & Bolles 1981) with a 1 px transfer
//!   error threshold and a fixed-seed generator (deterministic), then least squares on the inliers.
//! - **Warp** — inverse mapping with bilinear sampling, like [`crate::image::Image::transformed`].

use cfa4_geom::{Affine, Vec2};
use rayon::prelude::*;

use crate::image::Image;

/// A homography, row-major, acting on column vectors `(x, y, 1)`. Constructors keep `h33 = 1`
/// where they can, so [`Homography::apply`] sees a positive `w` for points in front of the plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

const RANSAC_THRESHOLD: f64 = 1.0;
const RANSAC_ITERS: usize = 400;
/// Fewest inliers a robust fit may rest on.
pub const MIN_INLIERS: usize = 10;

impl Homography {
    pub const IDENTITY: Homography = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn from_affine(a: &Affine) -> Self {
        Homography([[a.a, a.c, a.e], [a.b, a.d, a.f], [0.0, 0.0, 1.0]])
    }

    pub fn is_finite(&self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
    }

    /// Map a point; `None` when it lands at or behind the plane's horizon (`w ≤ 0`).
    pub fn apply(&self, p: Vec2) -> Option<Vec2> {
        let m = &self.0;
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        if w.is_nan() || w <= 1e-9 {
            return None;
        }
        Some(Vec2::new((m[0][0] * p.x + m[0][1] * p.y + m[0][2]) / w, (m[1][0] * p.x + m[1][1] * p.y + m[1][2]) / w))
    }

    /// `self ∘ o` (apply `o` first), scaled so that `h33 = 1` when it is not ≈ 0.
    pub fn then(&self, o: &Homography) -> Homography {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.0[i][k] * o.0[k][j]).sum();
            }
        }
        Homography(r).normalized()
    }

    fn normalized(self) -> Homography {
        let s = self.0[2][2];
        if s.abs() < 1e-12 || !s.is_finite() {
            return self;
        }
        Homography(self.0.map(|row| row.map(|v| v / s)))
    }

    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
        let adj =
            [[c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)], [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)], [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)]];
        let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
        if !det.is_finite() || det.abs() < 1e-18 {
            return None;
        }
        let inv = Homography(adj.map(|row| row.map(|v| v / det))).normalized();
        inv.is_finite().then_some(inv)
    }

    /// `S ∘ self ∘ S⁻¹` for the uniform scale `S` by `s`: the same transform in pixels `s` times
    /// larger.
    pub fn conjugated_by_scale(&self, s: f64) -> Homography {
        let f = |i: usize| if i < 2 { s } else { 1.0 };
        let mut r = self.0;
        for (i, row) in r.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v *= f(i) / f(j);
            }
        }
        Homography(r)
    }

    /// The homography taking the four points `src` to `dst`.
    pub fn from_points(src: &[Vec2; 4], dst: &[Vec2; 4]) -> Option<Homography> {
        fit_ls(src, dst)
    }
}

/// Hartley normalisation: the similarity moving the centroid to the origin and the mean distance
/// from it to √2, as (centroid, scale).
fn normalisation(p: &[Vec2]) -> Option<(Vec2, f64)> {
    let n = p.len() as f64;
    let c = p.iter().fold(Vec2::ZERO, |a, q| a + *q) * (1.0 / n);
    let d = p.iter().map(|q| (*q - c).length()).sum::<f64>() / n;
    (d.is_finite() && d > 1e-9).then(|| (c, std::f64::consts::SQRT_2 / d))
}

/// Solve `a x = b` (8×8, partial pivoting); `None` when singular.
fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let piv = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-10 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..8 {
            let f = a[r][col] / a[col][col];
            for k in col..8 {
                a[r][k] -= f * a[col][k];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = [0.0; 8];
    for r in (0..8).rev() {
        let s: f64 = (r + 1..8).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Least-squares homography taking `src[i]` to `dst[i]` (at least 4 points, no 3 collinear among
/// the minimal set). Exact for 4 points.
pub fn fit_ls(src: &[Vec2], dst: &[Vec2]) -> Option<Homography> {
    let n = src.len();
    if n < 4 || dst.len() != n {
        return None;
    }
    let (cs, ss) = normalisation(src)?;
    let (cd, sd) = normalisation(dst)?;
    let mut ata = [[0.0; 8]; 8];
    let mut atb = [0.0; 8];
    for (s, d) in src.iter().zip(dst) {
        let (x, y) = ((s.x - cs.x) * ss, (s.y - cs.y) * ss);
        let (u, v) = ((d.x - cd.x) * sd, (d.y - cd.y) * sd);
        for (row, rhs) in [([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u), ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v)] {
            for i in 0..8 {
                for j in 0..8 {
                    ata[i][j] += row[i] * row[j];
                }
                atb[i] += row[i] * rhs;
            }
        }
    }
    let h = solve8(ata, atb)?;
    let hn = Homography([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]]);
    // undo the normalisation: H = Td⁻¹ · Ĥ · Ts
    let ts = Homography([[ss, 0.0, -ss * cs.x], [0.0, ss, -ss * cs.y], [0.0, 0.0, 1.0]]);
    let td_inv = Homography([[1.0 / sd, 0.0, cd.x], [0.0, 1.0 / sd, cd.y], [0.0, 0.0, 1.0]]);
    let r = td_inv.then(&hn).then(&ts);
    (r.is_finite() && r.0[2][2].abs() > 1e-12 && (r.0[2][2] - 1.0).abs() < 1e-6).then_some(r)
}

/// Deterministic xorshift for RANSAC sampling.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

fn transfer_error(h: &Homography, s: Vec2, d: Vec2) -> f64 {
    h.apply(s).map_or(f64::INFINITY, |p| (p - d).length())
}

/// RANSAC + least-squares refit. Returns the homography and its inlier count; `None` when fewer
/// than [`MIN_INLIERS`] correspondences agree.
pub fn fit_robust(src: &[Vec2], dst: &[Vec2]) -> Option<(Homography, usize)> {
    let n = src.len();
    if n < MIN_INLIERS || dst.len() != n {
        return None;
    }
    let inliers_of = |h: &Homography| -> Vec<usize> { (0..n).filter(|&i| transfer_error(h, src[i], dst[i]) < RANSAC_THRESHOLD).collect() };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ n as u64);
    let mut best: Vec<usize> = Vec::new();
    for _ in 0..RANSAC_ITERS {
        let mut idx = [0usize; 4];
        let mut guard = 0;
        'pick: loop {
            for slot in 0..4 {
                idx[slot] = rng.below(n);
            }
            guard += 1;
            // distinct, and spread out (a cluster of near points gives a wild homography)
            let spread = (0..4).all(|a| (a + 1..4).all(|b| idx[a] != idx[b] && (src[idx[a]] - src[idx[b]]).length() >= 4.0));
            if spread {
                break 'pick;
            }
            if guard >= 16 {
                break;
            }
        }
        if guard >= 16 {
            continue;
        }
        let s = idx.map(|q| src[q]);
        let d = idx.map(|q| dst[q]);
        let Some(h) = fit_ls(&s, &d) else { continue };
        let inl = inliers_of(&h);
        if inl.len() > best.len() {
            best = inl;
            if best.len() == n {
                break;
            }
        }
    }
    if best.len() < MIN_INLIERS {
        return None;
    }
    let pick = |idx: &[usize]| -> (Vec<Vec2>, Vec<Vec2>) { idx.iter().map(|&q| (src[q], dst[q])).unzip() };
    for _ in 0..2 {
        let (s, d) = pick(&best);
        let h = fit_ls(&s, &d)?;
        let inl = inliers_of(&h);
        if inl.len() < best.len() {
            break;
        }
        best = inl;
    }
    let (s, d) = pick(&best);
    fit_ls(&s, &d).map(|h| (h, best.len()))
}

/// Resample `img` through `m` (source pixels → destination pixels) into an image of the same
/// size: inverse mapping with bilinear sampling. Pixels whose source falls outside stay clear.
pub fn warp(img: &Image, m: &Homography) -> Image {
    let (w, h) = (img.w, img.h);
    let Some(inv) = m.inverse() else { return Image::new(w, h) };
    if w == 0 || h == 0 {
        return Image::new(w, h);
    }
    let mut out = Image::new(w, h);
    out.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let Some(q) = inv.apply(Vec2::new(x as f64 + 0.5, y as f64 + 0.5)) else { continue };
            if q.x < -1.0 || q.y < -1.0 || q.x > w as f64 + 1.0 || q.y > h as f64 + 1.0 {
                continue;
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&img.sample_bilinear(q.x as f32, q.y as f32));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truth() -> Homography {
        Homography([[1.04, 0.03, 7.0], [-0.02, 0.98, -4.0], [3.0e-4, -2.0e-4, 1.0]])
    }

    fn pts(n: usize) -> Vec<Vec2> {
        (0..n).map(|i| Vec2::new(((i * 37) % 200) as f64 + 5.0, ((i * 61) % 150) as f64 + 5.0)).collect()
    }

    #[test]
    fn four_points_give_the_exact_homography() {
        let h = truth();
        let src = [Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), Vec2::new(200.0, 150.0), Vec2::new(0.0, 150.0)];
        let dst = src.map(|p| h.apply(p).unwrap());
        let f = Homography::from_points(&src, &dst).expect("fit");
        for p in pts(30) {
            let (a, b) = (h.apply(p).unwrap(), f.apply(p).unwrap());
            assert!((a - b).length() < 1e-6, "{a:?} {b:?}");
        }
    }

    #[test]
    fn collinear_or_degenerate_points_are_refused() {
        let line = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0), Vec2::new(30.0, 30.0)];
        assert!(Homography::from_points(&line, &line).is_none());
        let same = [Vec2::new(5.0, 5.0); 4];
        assert!(Homography::from_points(&same, &same).is_none());
    }

    #[test]
    fn inverse_composes_to_identity_and_scale_conjugation_commutes() {
        let h = truth();
        let id = h.then(&h.inverse().unwrap());
        for (i, row) in id.0.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9, "{id:?}");
            }
        }
        let big = h.conjugated_by_scale(2.5);
        for p in pts(10) {
            let a = big.apply(p * 2.5).unwrap();
            let b = h.apply(p).unwrap() * 2.5;
            assert!((a - b).length() < 1e-6);
        }
        assert!(Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]]).inverse().is_none());
    }

    #[test]
    fn points_behind_the_horizon_do_not_map() {
        let h = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-0.01, 0.0, 1.0]]);
        assert!(h.apply(Vec2::new(50.0, 10.0)).is_some());
        assert!(h.apply(Vec2::new(150.0, 10.0)).is_none());
    }

    #[test]
    fn ransac_rejects_outliers_and_noise() {
        let h = truth();
        let src = pts(80);
        let dst: Vec<Vec2> = src
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let q = h.apply(*p).unwrap();
                match i % 5 {
                    0 => q + Vec2::new(23.0, -17.0),                                                             // outlier
                    _ => q + Vec2::new(((i * 13) % 7) as f64 * 0.04 - 0.12, ((i * 29) % 5) as f64 * 0.05 - 0.1), // ≤ 0.15 px noise
                }
            })
            .collect();
        let (f, inl) = fit_robust(&src, &dst).expect("robust fit");
        assert_eq!(inl, 64);
        for p in pts(30) {
            assert!((f.apply(p).unwrap() - h.apply(p).unwrap()).length() < 0.3);
        }
        // not enough agreeing points
        assert!(fit_robust(&src[..6], &dst[..6]).is_none());
        let junk: Vec<Vec2> = (0..40).map(|i| Vec2::new(((i * 91) % 211) as f64, ((i * 53) % 197) as f64)).collect();
        assert!(fit_robust(&src[..40], &junk).is_none(), "random correspondences have no consensus");
    }

    #[test]
    fn warp_matches_the_affine_resampler_and_clears_what_is_outside() {
        let (w, h) = (48, 32);
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 13) % 31) as f32 / 31.0;
                img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[v, v * 0.5, 1.0 - v, 1.0]);
            }
        }
        let a = Affine::translate(5.0, -3.0);
        let via_h = warp(&img, &Homography::from_affine(&a));
        let via_a = img.transformed(w, h, &a);
        let worst = via_h.px.iter().zip(&via_a.px).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "{worst}");
        assert_eq!(via_h.get(0, 0)[3], 0.0, "the uncovered corner is clear");
        // a degenerate homography gives a clear image, not a panic
        let none = warp(&img, &Homography([[0.0; 3]; 3]));
        assert!(none.px.iter().all(|v| *v == 0.0));
    }
}
