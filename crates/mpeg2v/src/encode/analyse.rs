//! Macroblock analysis: motion search, the intra / inter decision, prediction, forward DCT and
//! the spatial activity rate control uses. Independent per macroblock row, so rows can run in
//! parallel.

use crate::idct::basis;
use crate::mc::{Frame, predict};

/// Motion vectors stay in [-RANGE, RANGE - 1] half samples (f_code 3).
pub(super) const F_CODE: u32 = 3;
pub(super) const RANGE: i32 = 16 << (F_CODE - 1);

/// What analysis decided for one macroblock.
#[derive(Clone)]
pub(super) struct Plan {
    pub intra: bool,
    /// Forward vector in half samples (P pictures, inter).
    pub mv: [i32; 2],
    /// Forward DCT of the samples (intra) or of the prediction error (inter), raster order.
    pub coef: [[i32; 64]; 8],
    /// Spatial activity (1 + the smallest luma block variance), for adaptive quantisation.
    pub act: f32,
    /// Expected share of the picture's bits (coefficient magnitude), for rate control.
    pub weight: f32,
}

/// Picture-wide analysis inputs.
pub(super) struct Ctx<'a> {
    pub cur: &'a Frame,
    pub reference: Option<&'a Frame>,
    /// Chroma MB height (8 for 4:2:0, 16 for 4:2:2).
    pub cmbh: usize,
    pub nblocks: usize,
    pub mb_width: usize,
    /// The previous P picture's vectors (temporal predictors), one per macroblock.
    pub prev_mvs: &'a [[i32; 2]],
    /// Weight of vector bits against SAD.
    pub lambda: u32,
}

/// Forward 8×8 DCT (the transpose of the decoder's IDCT basis), rounded.
pub(super) fn fdct(px: &[i32; 64], out: &mut [i32; 64]) {
    let m = basis();
    // rows: tmp[y][u] = Σ_x M[x][u] f[y][x]
    let mut tmp = [[0f32; 8]; 8];
    for y in 0..8 {
        let f: [f32; 8] = std::array::from_fn(|x| px[y * 8 + x] as f32);
        for u in 0..8 {
            let mut s = 0f32;
            for x in 0..8 {
                s += m[x][u] * f[x];
            }
            tmp[y][u] = s;
        }
    }
    // columns: F[v][u] = Σ_y M[y][v] tmp[y][u]
    for v in 0..8 {
        for u in 0..8 {
            let mut s = 0f32;
            for (y, row) in tmp.iter().enumerate() {
                s += m[y][v] * row[u];
            }
            out[v * 8 + u] = s.round() as i32;
        }
    }
}

/// Sum of absolute differences of a 16×16 block of `cur` at (`x`, `y`) against `r` at (`rx`, `ry`)
/// (both inside their planes), stopping early above `limit`.
#[inline]
fn sad16(cur: &Frame, x: usize, y: usize, r: &Frame, rx: usize, ry: usize, limit: u32) -> u32 {
    let (cs, rs) = (cur.w, r.w);
    let mut sum = 0u32;
    for l in 0..16 {
        let (Some(a), Some(b)) = (cur.planes[0].get((y + l) * cs + x..(y + l) * cs + x + 16), r.planes[0].get((ry + l) * rs + rx..(ry + l) * rs + rx + 16))
        else {
            return u32::MAX;
        };
        sum += a.iter().zip(b).map(|(&p, &q)| p.abs_diff(q) as u32).sum::<u32>();
        if sum > limit {
            break;
        }
    }
    sum
}

/// Approximate bits of a vector component difference.
#[inline]
fn mv_bits(d: i32) -> u32 {
    if d == 0 { 1 } else { 3 + 2 * (32 - d.unsigned_abs().leading_zeros()) }
}

/// Whether frame prediction with `mv` (half samples) stays inside the reference: luma and the
/// chroma vector the decoder derives (§7.6.3.7), as conforming streams must.
pub(super) fn fits(r: &Frame, mbx: usize, mby: usize, cmbh: usize, mv: [i32; 2]) -> bool {
    if !(-RANGE..RANGE).contains(&mv[0]) || !(-RANGE..RANGE).contains(&mv[1]) {
        return false;
    }
    let inside = |x: i32, y: i32, v: [i32; 2], bw: i32, bh: i32, w: usize, h: usize| {
        let (px, py) = (x + (v[0] >> 1), y + (v[1] >> 1));
        px >= 0 && py >= 0 && px + bw + (v[0] & 1) <= w as i32 && py + bh + (v[1] & 1) <= h as i32
    };
    let cmv = chroma_mv(mv, cmbh);
    inside((mbx * 16) as i32, (mby * 16) as i32, mv, 16, 16, r.w, r.h) && inside((mbx * 8) as i32, (mby * cmbh) as i32, cmv, 8, cmbh as i32, r.cw, r.ch)
}

fn chroma_mv(mv: [i32; 2], cmbh: usize) -> [i32; 2] {
    [mv[0] / 2, if cmbh == 8 { mv[1] / 2 } else { mv[1] }]
}

/// The decoder's frame prediction of a macroblock (luma 16×16, chroma 8×`cmbh`).
pub(super) fn predict_mb(r: &Frame, mbx: usize, mby: usize, cmbh: usize, mv: [i32; 2], y: &mut [u8; 256], c: &mut [[u8; 128]; 2]) {
    let (x0, y0) = ((mbx * 16) as i32 + (mv[0] >> 1), (mby * 16) as i32 + (mv[1] >> 1));
    predict(y, 0, 16, r.plane(0, None), x0, y0, mv[0] & 1 != 0, mv[1] & 1 != 0, 16, 16, false);
    let cmv = chroma_mv(mv, cmbh);
    let (cx, cy) = ((mbx * 8) as i32 + (cmv[0] >> 1), (mby * cmbh) as i32 + (cmv[1] >> 1));
    for (k, plane) in c.iter_mut().enumerate() {
        predict(plane, 0, 8, r.plane(1 + k, None), cx, cy, cmv[0] & 1 != 0, cmv[1] & 1 != 0, 8, cmbh, false);
    }
}

/// SAD of the half-sample prediction `mv` against the source macroblock.
fn sad_half(ctx: &Ctx, r: &Frame, mbx: usize, mby: usize, mv: [i32; 2]) -> u32 {
    let mut p = [0u8; 256];
    let (x0, y0) = ((mbx * 16) as i32 + (mv[0] >> 1), (mby * 16) as i32 + (mv[1] >> 1));
    predict(&mut p, 0, 16, r.plane(0, None), x0, y0, mv[0] & 1 != 0, mv[1] & 1 != 0, 16, 16, false);
    let cur = ctx.cur;
    let mut sum = 0u32;
    for l in 0..16 {
        let at = (mby * 16 + l) * cur.w + mbx * 16;
        if let Some(a) = cur.planes[0].get(at..at + 16) {
            sum += a.iter().zip(&p[l * 16..l * 16 + 16]).map(|(&s, &q)| s.abs_diff(q) as u32).sum::<u32>();
        }
    }
    sum
}

/// Motion search: the best of the predictors, a diamond search from it at whole samples, then
/// half-sample refinement. Returns the vector (half samples) and its SAD.
fn search(ctx: &Ctx, r: &Frame, mbx: usize, mby: usize, preds: &[[i32; 2]]) -> ([i32; 2], u32) {
    let (x, y) = (mbx * 16, mby * 16);
    let pmv = preds.first().copied().unwrap_or([0, 0]);
    let cost_of = |v: [i32; 2], sad: u32| sad.saturating_add(ctx.lambda * (mv_bits(v[0] - pmv[0]) + mv_bits(v[1] - pmv[1])));
    // whole-sample candidates (d in samples)
    let eval = |d: [i32; 2], limit: u32| -> Option<u32> {
        let v = [2 * d[0], 2 * d[1]];
        if !fits(r, mbx, mby, ctx.cmbh, v) {
            return None;
        }
        let sad = sad16(ctx.cur, x, y, r, (x as i32 + d[0]) as usize, (y as i32 + d[1]) as usize, limit);
        Some(cost_of(v, sad))
    };
    let mut best = [0i32; 2];
    let mut best_cost = eval(best, u32::MAX).unwrap_or(u32::MAX);
    for p in preds {
        let d = [p[0] >> 1, p[1] >> 1];
        if d != best
            && let Some(c) = eval(d, best_cost)
            && c < best_cost
        {
            best = d;
            best_cost = c;
        }
    }
    // large diamond, then small diamond
    for pattern in [&[[0, -2], [1, -1], [2, 0], [1, 1], [0, 2], [-1, 1], [-2, 0], [-1, -1]][..], &[[0, -1], [1, 0], [0, 1], [-1, 0]][..]] {
        for _ in 0..48 {
            let center = best;
            for o in pattern {
                let d = [center[0] + o[0], center[1] + o[1]];
                if let Some(c) = eval(d, best_cost)
                    && c < best_cost
                {
                    best = d;
                    best_cost = c;
                }
            }
            if best == center {
                break;
            }
        }
    }
    // half samples
    let center = [2 * best[0], 2 * best[1]];
    let mut v = center;
    let mut best_cost = cost_of(center, sad_half(ctx, r, mbx, mby, center));
    for o in [[-1, -1], [0, -1], [1, -1], [-1, 0], [1, 0], [-1, 1], [0, 1], [1, 1]] {
        let c = [center[0] + o[0], center[1] + o[1]];
        if fits(r, mbx, mby, ctx.cmbh, c) {
            let cost = cost_of(c, sad_half(ctx, r, mbx, mby, c));
            if cost < best_cost {
                v = c;
                best_cost = cost;
            }
        }
    }
    (v, sad_half(ctx, r, mbx, mby, v))
}

/// Samples of block `i` (0-3 luma, then chroma) of a macroblock buffer.
fn block_samples(y: &[u8; 256], c: &[[u8; 128]; 2], i: usize, out: &mut [i32; 64]) {
    if i < 4 {
        let (x0, y0) = ((i & 1) * 8, (i >> 1) * 8);
        for r in 0..8 {
            for k in 0..8 {
                out[r * 8 + k] = y[(y0 + r) * 16 + x0 + k] as i32;
            }
        }
    } else {
        // 4, 6: Cb; 5, 7: Cr (4:2:2: 6 and 7 are the lower halves)
        let plane = &c[i & 1];
        let y0 = ((i - 4) >> 1) * 8;
        for r in 0..8 {
            for k in 0..8 {
                out[r * 8 + k] = plane[(y0 + r) * 8 + k] as i32;
            }
        }
    }
}

/// Copy the source macroblock out of the (padded) current frame.
fn source_mb(cur: &Frame, mbx: usize, mby: usize, cmbh: usize, y: &mut [u8; 256], c: &mut [[u8; 128]; 2]) {
    for l in 0..16 {
        let at = (mby * 16 + l) * cur.w + mbx * 16;
        if let Some(s) = cur.planes[0].get(at..at + 16) {
            y[l * 16..l * 16 + 16].copy_from_slice(s);
        }
    }
    for (k, out) in c.iter_mut().enumerate() {
        for l in 0..cmbh {
            let at = (mby * cmbh + l) * cur.cw + mbx * 8;
            if let Some(s) = cur.planes[1 + k].get(at..at + 8) {
                out[l * 8..l * 8 + 8].copy_from_slice(s);
            }
        }
    }
}

/// Analyse macroblock row `mby`.
pub(super) fn analyse_row(ctx: &Ctx, mby: usize) -> Vec<Plan> {
    let mut out: Vec<Plan> = Vec::with_capacity(ctx.mb_width);
    let mut sy = [0u8; 256];
    let mut sc = [[0u8; 128]; 2];
    let mut py = [0u8; 256];
    let mut pc = [[0u8; 128]; 2];
    let mut px = [0i32; 64];
    for mbx in 0..ctx.mb_width {
        source_mb(ctx.cur, mbx, mby, ctx.cmbh, &mut sy, &mut sc);
        // activity and the intra cost (deviation from the mean)
        let mut min_var = f32::MAX;
        for i in 0..4 {
            block_samples(&sy, &sc, i, &mut px);
            let mean = px.iter().sum::<i32>() as f32 / 64.0;
            let var = px.iter().map(|&p| (p as f32 - mean) * (p as f32 - mean)).sum::<f32>() / 64.0;
            min_var = min_var.min(var);
        }
        let mean16 = (sy.iter().map(|&p| p as u32).sum::<u32>() + 128) / 256;
        let intra_cost = sy.iter().map(|&p| (p as u32).abs_diff(mean16)).sum::<u32>();
        let mut plan = Plan { intra: true, mv: [0, 0], coef: [[0; 64]; 8], act: 1.0 + min_var, weight: 1.0 };
        if let Some(r) = ctx.reference {
            let mut preds: Vec<[i32; 2]> = Vec::with_capacity(3);
            preds.push(out.last().filter(|p| !p.intra).map_or([0, 0], |p| p.mv));
            if let Some(&t) = ctx.prev_mvs.get(mby * ctx.mb_width + mbx) {
                preds.push(t);
            }
            preds.push([0, 0]);
            let (mv, sad) = search(ctx, r, mbx, mby, &preds);
            if sad <= intra_cost.saturating_add(512) {
                plan.intra = false;
                plan.mv = mv;
                predict_mb(r, mbx, mby, ctx.cmbh, mv, &mut py, &mut pc);
            }
        }
        let mut pb = [0i32; 64];
        for i in 0..ctx.nblocks {
            block_samples(&sy, &sc, i, &mut px);
            if !plan.intra {
                block_samples(&py, &pc, i, &mut pb);
                for (s, p) in px.iter_mut().zip(&pb) {
                    *s -= p;
                }
            }
            fdct(&px, &mut plan.coef[i]);
            plan.weight += plan.coef[i][plan.intra as usize..].iter().map(|c| c.unsigned_abs()).sum::<u32>() as f32 / 64.0;
        }
        out.push(plan);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idct::idct;

    #[test]
    fn fdct_inverts_the_decoders_idct() {
        let mut px = [0i32; 64];
        for (i, v) in px.iter_mut().enumerate() {
            *v = ((i * 37 + i / 8 * 11) % 255) as i32 - 100;
        }
        let mut f = [0i32; 64];
        fdct(&px, &mut f);
        idct(&mut f, false);
        for (a, b) in px.iter().zip(&f) {
            assert!((a - b).abs() <= 1, "{a} {b}");
        }
        // a flat block: DC = 8 · mean
        let mut f = [0i32; 64];
        fdct(&[50; 64], &mut f);
        assert_eq!(f[0], 400);
        assert!(f[1..].iter().all(|&c| c == 0));
    }
}
