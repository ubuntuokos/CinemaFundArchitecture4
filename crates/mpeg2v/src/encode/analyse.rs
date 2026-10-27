//! Macroblock analysis: motion search, the intra / inter decision, prediction, forward DCT and
//! the spatial activity rate control uses. Independent per macroblock row, so rows can run in
//! parallel.

use crate::idct::basis;
use crate::mc::{Frame, predict};
use crate::vlc::{MB_BWD, MB_FWD};

/// Motion vectors stay in [-RANGE, RANGE - 1] half samples (f_code 3).
pub(super) const F_CODE: u32 = 3;
pub(super) const RANGE: i32 = 16 << (F_CODE - 1);

/// What analysis decided for one macroblock.
#[derive(Clone)]
pub(super) struct Plan {
    pub intra: bool,
    /// Prediction directions of an inter macroblock: `MB_FWD` and / or `MB_BWD`.
    pub dir: i16,
    /// Field motion compensation (interlaced): one vector per field of the macroblock and
    /// direction, vertical components in field lines.
    pub field_mc: bool,
    /// Vectors in half samples, `[r][s]`: `r` the field (field MC) or 0 (frame MC), `s` 0
    /// forward, 1 backward.
    pub mvs: [[[i32; 2]; 2]; 2],
    /// The reference field of each field vector, `[r][s]`.
    pub sel: [[usize; 2]; 2],
    /// Field DCT (interlaced): luma blocks 0 and 1 hold the top field's lines.
    pub field_dct: bool,
    /// Forward DCT of the samples (intra) or of the prediction error (inter), raster order.
    pub coef: [[i32; 64]; 8],
    /// Spatial activity (1 + the smallest luma block variance), for adaptive quantisation.
    pub act: f32,
    /// Expected share of the picture's bits (coefficient magnitude), for rate control.
    pub weight: f32,
}

impl Plan {
    /// The frame vector of direction `s` (field MC: the first field's, in frame units).
    pub fn frame_mv(&self, s: usize) -> [i32; 2] {
        let v = self.mvs[0][s];
        if self.field_mc { [v[0], v[1] * 2] } else { v }
    }
}

/// Picture-wide analysis inputs.
pub(super) struct Ctx<'a> {
    pub cur: &'a Frame,
    /// The anchor before (P and B pictures) and after (B pictures) the picture.
    pub fwd: Option<&'a Frame>,
    pub bwd: Option<&'a Frame>,
    /// Chroma MB height (8 for 4:2:0, 16 for 4:2:2).
    pub cmbh: usize,
    pub nblocks: usize,
    pub mb_width: usize,
    /// Temporal vector predictors towards `fwd` and `bwd`, one per macroblock (from the last P
    /// picture's vectors).
    pub preds_f: &'a [[i32; 2]],
    pub preds_b: &'a [[i32; 2]],
    /// Weight of vector bits against SAD.
    pub lambda: u32,
    /// Interlaced frame pictures: field motion compensation and field DCT are candidates.
    pub interlaced: bool,
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

/// Approximate bits of a vector component difference.
#[inline]
fn mv_bits(d: i32) -> u32 {
    if d == 0 { 1 } else { 3 + 2 * (32 - d.unsigned_abs().leading_zeros()) }
}

/// Where a 16-wide luma block (and its chroma) sits: in the frame (frame prediction) or in one
/// field of the current macroblock (field prediction, 8 lines from field line `8·mby`).
#[derive(Clone, Copy)]
struct Geo {
    /// Luma position and height in the (frame or field) plane.
    x: usize,
    y: usize,
    h: usize,
    /// Chroma position and height.
    cx: usize,
    cy: usize,
    ch: usize,
    /// Chroma vertical sub-sampling (4:2:0).
    shy: bool,
    /// A field of the reference (`Some(parity)`) or the whole frame.
    field: Option<usize>,
}

impl Geo {
    fn frame(mbx: usize, mby: usize, cmbh: usize) -> Geo {
        Geo { x: mbx * 16, y: mby * 16, h: 16, cx: mbx * 8, cy: mby * cmbh, ch: cmbh, shy: cmbh == 8, field: None }
    }

    /// Field `sel` of the reference, for the 8 lines of one field of macroblock (`mbx`, `mby`).
    fn field(mbx: usize, mby: usize, cmbh: usize, sel: usize) -> Geo {
        Geo { x: mbx * 16, y: mby * 8, h: 8, cx: mbx * 8, cy: mby * cmbh / 2, ch: cmbh / 2, shy: cmbh == 8, field: Some(sel) }
    }

    fn chroma_mv(&self, mv: [i32; 2]) -> [i32; 2] {
        [mv[0] / 2, if self.shy { mv[1] / 2 } else { mv[1] }]
    }

    /// Whether prediction with `mv` stays inside reference `r`: luma and the chroma vector the
    /// decoder derives (§7.6.3.7), as conforming streams must.
    fn fits(&self, r: &Frame, mv: [i32; 2]) -> bool {
        if !(-RANGE..RANGE).contains(&mv[0]) || !(-RANGE..RANGE).contains(&mv[1]) {
            return false;
        }
        let inside = |x: usize, y: usize, v: [i32; 2], bw: i32, bh: usize, w: usize, h: usize| {
            let (px, py) = (x as i32 + (v[0] >> 1), y as i32 + (v[1] >> 1));
            px >= 0 && py >= 0 && px + bw + (v[0] & 1) <= w as i32 && py + bh as i32 + (v[1] & 1) <= h as i32
        };
        let (l, c) = (r.plane(0, self.field), r.plane(1, self.field));
        inside(self.x, self.y, mv, 16, self.h, l.w, l.h) && inside(self.cx, self.cy, self.chroma_mv(mv), 8, self.ch, c.w, c.h)
    }

    /// The decoder's prediction from `r` with `mv` into macroblock lines `first + k·step` (frame
    /// prediction: 0 and 1; field prediction: the field of the macroblock and 2).
    #[allow(clippy::too_many_arguments)]
    fn predict(&self, r: &Frame, mv: [i32; 2], first: usize, step: usize, y: &mut [u8; 256], c: &mut [[u8; 128]; 2], avg: bool) {
        let (x0, y0) = (self.x as i32 + (mv[0] >> 1), self.y as i32 + (mv[1] >> 1));
        predict(y, first * 16, 16 * step, r.plane(0, self.field), x0, y0, mv[0] & 1 != 0, mv[1] & 1 != 0, 16, self.h, avg);
        let cmv = self.chroma_mv(mv);
        let (cx, cy) = (self.cx as i32 + (cmv[0] >> 1), self.cy as i32 + (cmv[1] >> 1));
        for (k, plane) in c.iter_mut().enumerate() {
            predict(plane, first * 8, 8 * step, r.plane(1 + k, self.field), cx, cy, cmv[0] & 1 != 0, cmv[1] & 1 != 0, 8, self.ch, avg);
        }
    }
}

/// The source luma block a search matches: 16 wide, `h` lines from (`x`, `y`) of `cur` (the frame
/// or one field).
struct Src {
    px: [u8; 256],
    h: usize,
}

impl Src {
    fn of(cur: &Frame, field: Option<usize>, x: usize, y: usize, h: usize) -> Src {
        let p = cur.plane(0, field);
        let mut px = [0u8; 256];
        for l in 0..h {
            let at = p.off + (y + l) * p.stride + x;
            if let (Some(s), Some(d)) = (p.data.get(at..at + 16), px.get_mut(l * 16..l * 16 + 16)) {
                d.copy_from_slice(s);
            }
        }
        Src { px, h }
    }

    /// SAD against the whole-sample block at (`rx`, `ry`) of reference plane `field`, stopping
    /// early above `limit`.
    fn sad(&self, r: &Frame, field: Option<usize>, rx: usize, ry: usize, limit: u32) -> u32 {
        let p = r.plane(0, field);
        let mut sum = 0u32;
        for l in 0..self.h {
            let at = p.off + (ry + l) * p.stride + rx;
            let Some(b) = p.data.get(at..at + 16) else { return u32::MAX };
            sum += self.px[l * 16..l * 16 + 16].iter().zip(b).map(|(&a, &q)| a.abs_diff(q) as u32).sum::<u32>();
            if sum > limit {
                break;
            }
        }
        sum
    }

    /// SAD of the half-sample prediction `mv`.
    fn sad_half(&self, r: &Frame, g: &Geo, mv: [i32; 2]) -> u32 {
        let mut p = [0u8; 256];
        let (x0, y0) = (g.x as i32 + (mv[0] >> 1), g.y as i32 + (mv[1] >> 1));
        predict(&mut p, 0, 16, r.plane(0, g.field), x0, y0, mv[0] & 1 != 0, mv[1] & 1 != 0, 16, self.h, false);
        self.px[..16 * self.h].iter().zip(&p).map(|(&a, &b)| a.abs_diff(b) as u32).sum()
    }
}

/// Motion search of `src` in reference `r` placed by `g`: the best of the predictors, a diamond
/// search from it at whole samples, then half-sample refinement. Returns the vector (half
/// samples), its SAD and its cost (SAD plus the vector bits against the first predictor).
fn search(lambda: u32, src: &Src, r: &Frame, g: &Geo, preds: &[[i32; 2]]) -> ([i32; 2], u32, u32) {
    let pmv = preds.first().copied().unwrap_or([0, 0]);
    let cost_of = |v: [i32; 2], sad: u32| sad.saturating_add(lambda * (mv_bits(v[0] - pmv[0]) + mv_bits(v[1] - pmv[1])));
    // whole-sample candidates (d in samples)
    let eval = |d: [i32; 2], limit: u32| -> Option<u32> {
        let v = [2 * d[0], 2 * d[1]];
        if !g.fits(r, v) {
            return None;
        }
        let sad = src.sad(r, g.field, (g.x as i32 + d[0]) as usize, (g.y as i32 + d[1]) as usize, limit);
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
    let mut best_cost = cost_of(center, src.sad_half(r, g, center));
    for o in [[-1, -1], [0, -1], [1, -1], [-1, 0], [1, 0], [-1, 1], [0, 1], [1, 1]] {
        let c = [center[0] + o[0], center[1] + o[1]];
        if g.fits(r, c) {
            let cost = cost_of(c, src.sad_half(r, g, c));
            if cost < best_cost {
                v = c;
                best_cost = cost;
            }
        }
    }
    (v, src.sad_half(r, g, v), best_cost)
}

/// One direction's prediction: frame vector, or a vector and reference field per field.
#[derive(Clone, Copy)]
struct Motion {
    field: bool,
    mv: [[i32; 2]; 2],
    sel: [usize; 2],
    sad: u32,
    /// SAD plus the vector bits.
    cost: u32,
}

impl Motion {
    /// Predict the macroblock from `r` into the buffers.
    fn predict(&self, r: &Frame, mbx: usize, mby: usize, cmbh: usize, y: &mut [u8; 256], c: &mut [[u8; 128]; 2], avg: bool) {
        if self.field {
            for f in 0..2 {
                Geo::field(mbx, mby, cmbh, self.sel[f]).predict(r, self.mv[f], f, 2, y, c, avg);
            }
        } else {
            Geo::frame(mbx, mby, cmbh).predict(r, self.mv[0], 0, 1, y, c, avg);
        }
    }
}

/// The best frame and field (interlaced) predictions of macroblock (`mbx`, `mby`) from `r`.
fn motions(ctx: &Ctx, r: &Frame, mbx: usize, mby: usize, preds: &[[i32; 2]], frame_src: &Src, field_src: &[Src; 2]) -> (Motion, Option<Motion>) {
    let g = Geo::frame(mbx, mby, ctx.cmbh);
    let (mv, sad, cost) = search(ctx.lambda, frame_src, r, &g, preds);
    let frame = Motion { field: false, mv: [mv, [0, 0]], sel: [0, 0], sad, cost };
    if !ctx.interlaced {
        return (frame, None);
    }
    // each field of the macroblock from the better reference field (vertical in field lines)
    let fpreds: Vec<[i32; 2]> = preds.iter().map(|p| [p[0], p[1] >> 1]).collect();
    // two field select bits
    let mut field = Motion { field: true, mv: [[0, 0]; 2], sel: [0, 1], sad: 0, cost: 2 * ctx.lambda };
    for (f, src) in field_src.iter().enumerate() {
        let mut best: Option<([i32; 2], u32, u32, usize)> = None;
        for sel in [f, 1 - f] {
            let (mv, sad, cost) = search(ctx.lambda, src, r, &Geo::field(mbx, mby, ctx.cmbh, sel), &fpreds);
            if best.is_none_or(|b| cost < b.2) {
                best = Some((mv, sad, cost, sel));
            }
        }
        let (mv, sad, cost, sel) = best.unwrap_or(([0, 0], u32::MAX, u32::MAX, f));
        field.mv[f] = mv;
        field.sel[f] = sel;
        field.sad = field.sad.saturating_add(sad);
        field.cost = field.cost.saturating_add(cost);
    }
    (frame, Some(field))
}

/// Samples of block `i` (0-3 luma, then chroma) of a macroblock buffer; `field` takes the luma
/// blocks (and 4:2:2 chroma blocks) from alternate lines: field DCT.
fn block_samples(y: &[u8; 256], c: &[[u8; 128]; 2], cmbh: usize, i: usize, field: bool, out: &mut [i32; 64]) {
    if i < 4 {
        let x0 = (i & 1) * 8;
        let (y0, step) = if field { (i >> 1, 2) } else { ((i >> 1) * 8, 1) };
        for r in 0..8 {
            for k in 0..8 {
                out[r * 8 + k] = y[(y0 + r * step) * 16 + x0 + k] as i32;
            }
        }
    } else {
        // 4, 6: Cb; 5, 7: Cr (4:2:2: 6 and 7 are the lower halves)
        let plane = &c[i & 1];
        let k = (i - 4) >> 1;
        let (y0, step) = if field && cmbh == 16 { (k, 2) } else { (k * 8, 1) };
        for r in 0..8 {
            for x in 0..8 {
                out[r * 8 + x] = plane[(y0 + r * step) * 8 + x] as i32;
            }
        }
    }
}

/// Whether field DCT codes the luma of `d` (16×16) in fewer bits than frame DCT: the summed
/// magnitude of the transformed blocks either way, a usual proxy for their cost.
fn prefers_field_dct(d: &[i32; 256]) -> bool {
    let cost = |field: bool| {
        let (mut px, mut co) = ([0i32; 64], [0i32; 64]);
        let mut sum = 0u32;
        for i in 0..4 {
            let x0 = (i & 1) * 8;
            let (y0, step) = if field { (i >> 1, 2) } else { ((i >> 1) * 8, 1) };
            for r in 0..8 {
                for k in 0..8 {
                    px[r * 8 + k] = d[(y0 + r * step) * 16 + x0 + k];
                }
            }
            fdct(&px, &mut co);
            sum = sum.saturating_add(co.iter().map(|c| c.unsigned_abs()).sum::<u32>());
        }
        sum
    };
    cost(true) < cost(false)
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
            block_samples(&sy, &sc, ctx.cmbh, i, false, &mut px);
            let mean = px.iter().sum::<i32>() as f32 / 64.0;
            let var = px.iter().map(|&p| (p as f32 - mean) * (p as f32 - mean)).sum::<f32>() / 64.0;
            min_var = min_var.min(var);
        }
        let mean16 = (sy.iter().map(|&p| p as u32).sum::<u32>() + 128) / 256;
        let intra_cost = sy.iter().map(|&p| (p as u32).abs_diff(mean16)).sum::<u32>();
        let mut plan = Plan {
            intra: true,
            dir: 0,
            field_mc: false,
            mvs: [[[0; 2]; 2]; 2],
            sel: [[0, 0], [1, 1]],
            field_dct: false,
            coef: [[0; 64]; 8],
            act: 1.0 + min_var,
            weight: 1.0,
        };
        if ctx.fwd.is_some() || ctx.bwd.is_some() {
            let frame_src = Src { px: sy, h: 16 };
            let field_src =
                if ctx.interlaced { [0, 1].map(|f| Src::of(ctx.cur, Some(f), mbx * 16, mby * 8, 8)) } else { [0, 1].map(|_| Src { px: [0; 256], h: 0 }) };
            let left = out.last().filter(|p| !p.intra);
            let at = mby * ctx.mb_width + mbx;
            let candidates = |s: usize, temporal: Option<&[i32; 2]>| {
                let mut preds: Vec<[i32; 2]> = Vec::with_capacity(3);
                preds.push(left.filter(|p| p.dir & [MB_FWD, MB_BWD][s] != 0).map_or([0, 0], |p| p.frame_mv(s)));
                preds.extend(temporal.copied());
                preds.push([0, 0]);
                preds
            };
            let f = ctx.fwd.map(|r| motions(ctx, r, mbx, mby, &candidates(0, ctx.preds_f.get(at)), &frame_src, &field_src));
            let b = ctx.bwd.map(|r| motions(ctx, r, mbx, mby, &candidates(1, ctx.preds_b.get(at)), &frame_src, &field_src));
            // the cheapest of forward, backward and their average, all frame or all field
            // prediction (one motion type per macroblock)
            let sad_of = |y: &[u8; 256]| sy.iter().zip(y).map(|(&a, &b)| a.abs_diff(b) as u32).sum::<u32>();
            let mut best: Option<(u32, u32, i16, Motion, Motion)> = None;
            let mut consider = |sad: u32, cost: u32, dir: i16, mf: Motion, mb: Motion| {
                if best.is_none_or(|b| cost < b.1) {
                    best = Some((sad, cost, dir, mf, mb));
                }
            };
            for field in [false, true] {
                let pick = |m: &Option<(Motion, Option<Motion>)>| m.and_then(|(fr, fi)| if field { fi } else { Some(fr) });
                let (mf, mb) = (pick(&f), pick(&b));
                if let (Some(m), Some(r)) = (mf, ctx.fwd) {
                    m.predict(r, mbx, mby, ctx.cmbh, &mut py, &mut pc, false);
                    let sad = sad_of(&py);
                    consider(sad, m.cost, MB_FWD, m, m);
                }
                if let (Some(m), Some(r)) = (mb, ctx.bwd) {
                    m.predict(r, mbx, mby, ctx.cmbh, &mut py, &mut pc, false);
                    let sad = sad_of(&py);
                    consider(sad, m.cost, MB_BWD, m, m);
                }
                if let (Some(a), Some(c), Some(ra), Some(rc)) = (mf, mb, ctx.fwd, ctx.bwd) {
                    a.predict(ra, mbx, mby, ctx.cmbh, &mut py, &mut pc, false);
                    c.predict(rc, mbx, mby, ctx.cmbh, &mut py, &mut pc, true);
                    let sad = sad_of(&py);
                    // the vector bits of both, on top of the averaged prediction's SAD
                    let bits = |m: Motion| m.cost.saturating_sub(m.sad);
                    consider(sad, sad.saturating_add(bits(a)).saturating_add(bits(c)), MB_FWD | MB_BWD, a, c);
                }
            }
            if let Some((sad, _, dir, mf, mb)) = best
                && sad <= intra_cost.saturating_add(512)
            {
                plan.intra = false;
                plan.dir = dir;
                plan.field_mc = mf.field;
                for (s, m) in [(0, mf), (1, mb)] {
                    if dir & [MB_FWD, MB_BWD][s] != 0 {
                        for f in 0..2 {
                            plan.mvs[f][s] = m.mv[f];
                            plan.sel[f][s] = m.sel[f];
                        }
                    }
                }
                let mut avg = false;
                if let (Some(r), true) = (ctx.fwd, dir & MB_FWD != 0) {
                    mf.predict(r, mbx, mby, ctx.cmbh, &mut py, &mut pc, false);
                    avg = true;
                }
                if let (Some(r), true) = (ctx.bwd, dir & MB_BWD != 0) {
                    mb.predict(r, mbx, mby, ctx.cmbh, &mut py, &mut pc, avg);
                }
            }
        }
        // the luma to transform: the samples (intra) or the prediction error
        let mut d = [0i32; 256];
        for (k, v) in d.iter_mut().enumerate() {
            *v = sy[k] as i32 - if plan.intra { 0 } else { py[k] as i32 };
        }
        plan.field_dct = ctx.interlaced && prefers_field_dct(&d);
        let mut pb = [0i32; 64];
        for i in 0..ctx.nblocks {
            block_samples(&sy, &sc, ctx.cmbh, i, plan.field_dct, &mut px);
            if !plan.intra {
                block_samples(&py, &pc, ctx.cmbh, i, plan.field_dct, &mut pb);
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
