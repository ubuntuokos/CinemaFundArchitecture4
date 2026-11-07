//! Encoding of one tile column: partition and mode decisions, reconstruction, and the tile's
//! symbols. Every context, candidate list and reconstruction step mirrors the decoder
//! (`crate::tile`), so that the decoder's picture is the encoder's reconstruction.

use super::boolenc::BoolEncoder;
use super::fdct;
use crate::frame::{Frame, MiGrid, MiInfo, Mv};
use crate::inter::{self, RefPlane};
use crate::intra::{self, IntraEdge};
use crate::probs::FrameContext;
use crate::tables::*;
use crate::transform;

/// What the frame encoder hands every tile column.
pub struct FrameCtx<'a> {
    pub key: bool,
    pub mi_cols: usize,
    pub mi_rows: usize,
    pub sb_cols: usize,
    pub sb_rows: usize,
    pub fc: &'a FrameContext,
    /// Quantizers: [plane > 0][dc = 0 / ac = 1].
    pub q: [[i32; 2]; 2],
    /// The LAST reference as the decoder holds it (inter frames).
    pub last: Option<&'a Frame>,
    /// The previous frame's mode info when the decoder uses its motion vectors.
    pub prev_mi: Option<&'a MiGrid>,
    /// Source planes, padded by edge replication to whole superblocks: `src[p][y * src_stride[p] + x]`.
    pub src: [&'a [u16]; 3],
    pub src_stride: [usize; 3],
    /// The LAST reference's luma, edge-extended by `ref_pad` samples on every side, for the
    /// integer motion search.
    pub ref_luma: &'a [u16],
    pub ref_stride: usize,
    pub ref_pad: usize,
    /// Rate-distortion weight of one bit, in SATD units.
    pub lambda: f64,
    /// 0 (thorough) to 2 (fast): how many modes and motion candidates are tried.
    pub speed: u8,
}

/// Mode decision for one block.
#[derive(Clone, Copy, Debug)]
struct Choice {
    inter: bool,
    y_mode: u8,
    uv_mode: u8,
    mv: Mv,
}

enum Tree {
    /// Outside the frame.
    Outside,
    Leaf(Choice),
    Split(Box<[Tree; 4]>),
}

/// Intra modes tried, by speed.
const INTRA_MODES: [&[u8]; 3] = [
    &[DC_PRED, V_PRED, H_PRED, TM_PRED, D45_PRED, D135_PRED, D117_PRED, D153_PRED, D207_PRED, D63_PRED],
    &[DC_PRED, V_PRED, H_PRED, TM_PRED, D135_PRED, D45_PRED],
    &[DC_PRED, V_PRED, H_PRED, TM_PRED],
];

/// The coded coefficients of one transform block: levels in scan order up to the end of block.
struct TxCoefs {
    levels: Vec<i32>,
}

pub struct TileEncoder<'a> {
    f: &'a FrameCtx<'a>,
    mi_col_start: usize,
    mi_col_end: usize,
    pub planes: [Vec<u16>; 3],
    pub strides: [usize; 3],
    x_off: [usize; 3],
    above_nonzero: [Vec<u8>; 3],
    left_nonzero: [Vec<u8>; 3],
    above_partition: Vec<u8>,
    left_partition: Vec<u8>,
    // MV prediction state (as the decoder's).
    ref_list_mv: [Mv; 2],
    ref_mv_count: usize,
    mode_context: [u8; 4],
    // Scratch.
    pred: Vec<u16>,
    mc_tmp: Vec<u16>,
    mc_win: Vec<u16>,
    res: Vec<i32>,
    fwd: Vec<f64>,
    deq: Vec<i32>,
    token_cache: Vec<u8>,
}

/// Sum of absolute 4×4 Hadamard-transformed differences of a `w` × `h` region (w, h multiples
/// of 4), halved.
fn satd(a: &[u16], a_stride: usize, b: &[u16], b_stride: usize, w: usize, h: usize) -> u64 {
    let mut total = 0u64;
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            let mut d = [0i32; 16];
            for y in 0..4 {
                for x in 0..4 {
                    let ia = (by + y) * a_stride + bx + x;
                    let ib = (by + y) * b_stride + bx + x;
                    d[y * 4 + x] = i32::from(a.get(ia).copied().unwrap_or(0)) - i32::from(b.get(ib).copied().unwrap_or(0));
                }
            }
            for y in 0..4 {
                let (a0, a1, a2, a3) = (d[y * 4], d[y * 4 + 1], d[y * 4 + 2], d[y * 4 + 3]);
                let (s0, s1, d0, d1) = (a0 + a1, a2 + a3, a0 - a1, a2 - a3);
                d[y * 4] = s0 + s1;
                d[y * 4 + 1] = s0 - s1;
                d[y * 4 + 2] = d0 + d1;
                d[y * 4 + 3] = d0 - d1;
            }
            for x in 0..4 {
                let (a0, a1, a2, a3) = (d[x], d[4 + x], d[8 + x], d[12 + x]);
                let (s0, s1, d0, d1) = (a0 + a1, a2 + a3, a0 - a1, a2 - a3);
                total += ((s0 + s1).unsigned_abs() + (s0 - s1).unsigned_abs() + (d0 + d1).unsigned_abs() + (d0 - d1).unsigned_abs()) as u64;
            }
        }
    }
    total / 2
}

/// Bits of one motion vector component difference (in 1/8 samples), roughly.
fn mv_comp_bits(d: i32) -> f64 {
    if d == 0 { 1.0 } else { 3.0 + 2.0 * f64::from((d.unsigned_abs() >> 1).max(1)).log2() }
}

impl<'a> TileEncoder<'a> {
    pub fn new(f: &'a FrameCtx<'a>, mi_col_start: usize, mi_col_end: usize) -> Self {
        let strip_w = (mi_col_end - mi_col_start).div_ceil(8) * 64;
        let strip_h = f.sb_rows * 64;
        let x0 = mi_col_start * 8;
        let an = f.sb_cols * 16 + 16;
        let ln = f.sb_rows * 16 + 16;
        TileEncoder {
            f,
            mi_col_start,
            mi_col_end,
            planes: [vec![0; strip_w * strip_h], vec![0; (strip_w / 2) * (strip_h / 2)], vec![0; (strip_w / 2) * (strip_h / 2)]],
            strides: [strip_w, strip_w / 2, strip_w / 2],
            x_off: [x0, x0 / 2, x0 / 2],
            above_nonzero: [vec![0; an], vec![0; an], vec![0; an]],
            left_nonzero: [vec![0; ln], vec![0; ln], vec![0; ln]],
            above_partition: vec![0; f.sb_cols * 8 + 8],
            left_partition: vec![0; f.sb_rows * 8 + 8],
            ref_list_mv: [Mv::ZERO; 2],
            ref_mv_count: 0,
            mode_context: [0; 4],
            pred: vec![0; 64 * 64],
            mc_tmp: vec![0; 64 * 71],
            mc_win: vec![0; 71 * 71],
            res: vec![0; 32 * 32],
            fwd: vec![0.0; 32 * 32],
            deq: vec![0; 32 * 32],
            token_cache: vec![0; 1024],
        }
    }

    // ------------------------------------------------------------------ sample access helpers

    #[inline]
    fn src_at(&self, p: usize, x: usize, y: usize) -> u16 {
        self.f.src[p].get(y * self.f.src_stride[p] + x).copied().unwrap_or(0)
    }

    /// Copy the source of the `w` × `h` region at plane position (`x`, `y`) into the reconstruction.
    fn fill_source(&mut self, p: usize, x: usize, y: usize, w: usize, h: usize) {
        let (stride, off) = (self.strides[p], self.x_off[p]);
        for i in 0..h {
            let s = (y + i) * self.f.src_stride[p] + x;
            let d = (y + i) * stride + x - off;
            if let (Some(src), Some(dst)) = (self.f.src[p].get(s..s + w), self.planes[p].get_mut(d..d + w)) {
                dst.copy_from_slice(src);
            }
        }
    }

    /// SATD between the source and the reconstruction buffer over a region.
    fn satd_recon(&self, p: usize, x: usize, y: usize, w: usize, h: usize) -> u64 {
        let so = y * self.f.src_stride[p] + x;
        let ro = y * self.strides[p] + x - self.x_off[p];
        satd(self.f.src[p].get(so..).unwrap_or_default(), self.f.src_stride[p], self.planes[p].get(ro..).unwrap_or_default(), self.strides[p], w, h)
    }

    fn intra_edge(&self, p: usize, avail_l: bool, avail_u: bool, x4: usize, y4: usize, n4w: usize, tx: u8) -> IntraEdge {
        let s = (p > 0) as usize;
        IntraEdge {
            have_left: avail_l || x4 > 0,
            have_above: avail_u || y4 > 0,
            not_on_right: x4 + (1 << tx) < n4w,
            tx_size: tx,
            max_x: ((self.f.mi_cols * 8) >> s) - 1,
            max_y: ((self.f.mi_rows * 8) >> s) - 1,
            bit_depth: 8,
        }
    }

    // ------------------------------------------------------------------------- tile driver

    /// Encode this tile column (one tile row) into `bw`, writing mode info into `mi`.
    pub fn encode(&mut self, bw: &mut BoolEncoder, mi: &mut [MiInfo]) {
        for r in (0..self.f.mi_rows).step_by(8) {
            for p in 0..3 {
                let sy = (p > 0) as usize;
                let s = (r * 2) >> sy;
                let e = (s + (16 >> sy)).min(self.left_nonzero[p].len());
                if let Some(l) = self.left_nonzero[p].get_mut(s..e) {
                    l.fill(0);
                }
            }
            if let Some(l) = self.left_partition.get_mut(r..r + 8) {
                l.fill(0);
            }
            for c in (self.mi_col_start..self.mi_col_end).step_by(8) {
                // decisions on the source, then the real coding
                for p in 0..3 {
                    let s = (p > 0) as usize;
                    let (x, y) = ((c * 8) >> s, (r * 8) >> s);
                    self.fill_source(p, x, y, 64 >> s, 64 >> s);
                }
                let (_, tree) = self.estimate(mi, r, c, BLOCK_64X64);
                self.code_partition(bw, mi, r, c, BLOCK_64X64, &tree);
            }
        }
    }

    // ---------------------------------------------------------------------- decisions

    fn partition_ctx(&self, r: usize, c: usize, bsize: u8) -> usize {
        let num8x8 = NUM_8X8_WIDE[bsize as usize] as usize;
        let bsl = MI_WIDTH_LOG2[bsize as usize] as usize;
        let boffset = 3 - bsl;
        let (mut above, mut left) = (0u8, 0u8);
        for i in 0..num8x8 {
            above |= self.above_partition.get(c + i).copied().unwrap_or(0);
            left |= self.left_partition.get(r + i).copied().unwrap_or(0);
        }
        bsl * 4 + (((left >> boffset) & 1) as usize) * 2 + ((above >> boffset) & 1) as usize
    }

    fn partition_probs(&self, ctx: usize) -> [u8; 3] {
        if self.f.key {
            KF_PARTITION_PROBS.get(ctx * 3..ctx * 3 + 3).and_then(|p| p.first_chunk::<3>()).copied().unwrap_or([128; 3])
        } else {
            self.f.fc.partition.get(ctx).copied().unwrap_or([128; 3])
        }
    }

    /// Best coding of the block at (`r`, `c`) of size `bsize`: its cost and decisions.
    fn estimate(&mut self, mi: &[MiInfo], r: usize, c: usize, bsize: u8) -> (f64, Tree) {
        if r >= self.f.mi_rows || c >= self.mi_col_end {
            return (0.0, Tree::Outside);
        }
        let n8 = NUM_8X8_WIDE[bsize as usize] as usize;
        let inside = r + n8 <= self.f.mi_rows && c + n8 <= self.mi_col_end;
        let probs = self.partition_probs(self.partition_ctx(r, c, bsize));
        let bit_cost = |p: u8, one: bool| -> f64 {
            let p = f64::from(if one { 256 - u32::from(p) } else { u32::from(p) }) / 256.0;
            -p.max(1.0 / 256.0).log2()
        };
        let mut best: Option<(f64, Tree)> = None;
        if inside {
            let (j, choice) = self.best_mode(mi, r, c, bsize);
            let pbits = bit_cost(probs[0], false);
            best = Some((j + self.f.lambda * pbits, Tree::Leaf(choice)));
        }
        if bsize > BLOCK_8X8 {
            // the speed setting skips splitting blocks whose whole-block prediction is good
            let flat = best.as_ref().is_some_and(|(j, _)| self.f.speed >= 2 && *j < self.f.lambda * 8.0 * (n8 * n8) as f64);
            if !flat || !inside {
                let sub = SUBSIZE_LOOKUP[3][bsize as usize];
                let half = n8 / 2;
                let kids = [(r, c), (r, c + half), (r + half, c), (r + half, c + half)];
                let mut total = 0.0;
                let mut trees: Vec<Tree> = Vec::with_capacity(4);
                for (kr, kc) in kids {
                    let (j, t) = self.estimate(mi, kr, kc, sub);
                    total += j;
                    trees.push(t);
                }
                let split_bits = bit_cost(probs[0], true) + bit_cost(probs[1], true) + bit_cost(probs[2], true);
                total += self.f.lambda * split_bits;
                let better = best.as_ref().is_none_or(|(j, _)| total < *j);
                if better && let Ok(arr) = <[Tree; 4]>::try_from(trees) {
                    best = Some((total, Tree::Split(Box::new(arr))));
                }
            }
        }
        best.unwrap_or((f64::MAX / 4.0, Tree::Outside))
    }

    /// Mode decision for a whole block: cost and choice.
    fn best_mode(&mut self, mi: &[MiInfo], r: usize, c: usize, bsize: u8) -> (f64, Choice) {
        let avail_u = r > 0;
        let avail_l = c > self.mi_col_start;
        let (y_cost, y_mode) = self.best_intra(0, r, c, bsize, avail_l, avail_u);
        let (uv_cost, uv_mode) = self.best_intra(1, r, c, bsize, avail_l, avail_u);
        let intra = (y_cost + uv_cost + self.f.lambda * 6.0, Choice { inter: false, y_mode, uv_mode, mv: Mv::ZERO });
        if self.f.key || self.f.last.is_none() {
            return intra;
        }
        let (mv_cost, mv) = self.motion_search(mi, r, c, bsize);
        let inter = (mv_cost + self.f.lambda * 2.0, Choice { inter: true, y_mode: ZEROMV, uv_mode: DC_PRED, mv });
        if inter.0 <= intra.0 { inter } else { intra }
    }

    /// The best intra mode for luma (`p` 0) or chroma (`p` 1, both chroma planes) of a block,
    /// predicted transform block by transform block from the source as neighbours.
    fn best_intra(&mut self, p: usize, r: usize, c: usize, bsize: u8, avail_l: bool, avail_u: bool) -> (f64, u8) {
        let modes = INTRA_MODES[(self.f.speed as usize).min(2)];
        let modes: &[u8] = if p > 0 { &[DC_PRED, V_PRED, H_PRED, TM_PRED] } else { modes };
        let mut best = (f64::MAX, DC_PRED);
        for &mode in modes {
            let mut cost = 0u64;
            for plane in if p == 0 { 0..1 } else { 1..3 } {
                cost += self.intra_cost(plane, r, c, bsize, mode, avail_l, avail_u);
            }
            let bits = if mode == DC_PRED { 1.0 } else { 3.0 };
            let j = cost as f64 + self.f.lambda * bits;
            if j < best.0 {
                best = (j, mode);
            }
        }
        best
    }

    fn plane_geometry(&self, plane: usize, r: usize, c: usize, bsize: u8) -> (usize, usize, usize, usize, u8) {
        let s = (plane > 0) as usize;
        let psize = if plane > 0 { SS_SIZE_LOOKUP[bsize as usize][1][1] } else { bsize } as usize;
        let n4w = NUM_4X4_WIDE[psize] as usize;
        let n4h = NUM_4X4_HIGH[psize] as usize;
        let tx = if plane > 0 { MAX_TXSIZE[bsize as usize].min(MAX_TXSIZE[psize]) } else { MAX_TXSIZE[bsize as usize] };
        ((c * 8) >> s, (r * 8) >> s, n4w, n4h, tx)
    }

    /// Prediction error of an intra mode over a block, each transform block predicted from
    /// source neighbours (the block's region holds the source on return).
    #[allow(clippy::too_many_arguments)]
    fn intra_cost(&mut self, plane: usize, r: usize, c: usize, bsize: u8, mode: u8, avail_l: bool, avail_u: bool) -> u64 {
        let (bx, by, n4w, n4h, tx) = self.plane_geometry(plane, r, c, bsize);
        let step = 1usize << tx;
        let n = 4usize << tx;
        let mut cost = 0;
        for y4 in (0..n4h).step_by(step) {
            for x4 in (0..n4w).step_by(step) {
                let (x, y) = (bx + 4 * x4, by + 4 * y4);
                let e = self.intra_edge(plane, avail_l, avail_u, x4, y4, n4w, tx);
                let (stride, off) = (self.strides[plane], self.x_off[plane]);
                intra::predict(&mut self.planes[plane], stride, off, x, y, mode, &e);
                cost += self.satd_recon(plane, x, y, n, n);
                self.fill_source(plane, x, y, n, n);
            }
        }
        cost
    }

    /// Motion search on luma against LAST: the best motion vector (1/8 samples, even) and its cost.
    fn motion_search(&mut self, mi: &[MiInfo], r: usize, c: usize, bsize: u8) -> (f64, Mv) {
        let n = 8 * NUM_8X8_WIDE[bsize as usize] as usize;
        let (bx, by) = (c * 8, r * 8);
        let pad = self.f.ref_pad as i32;
        // predictors: zero and the neighbours' vectors
        let mut starts = vec![Mv::ZERO];
        for (nr, nc) in [(r as isize, c as isize - 1), (r as isize - 1, c as isize), (r as isize - 1, c as isize + 1)] {
            if nr >= 0 && nc >= self.mi_col_start as isize && (nc as usize) < self.mi_col_end && (nr as usize) < self.f.mi_rows {
                let m = mi.get(nr as usize * self.f.mi_cols + nc as usize).copied().unwrap_or_default();
                if m.ref_frame[0] > INTRA_FRAME {
                    starts.push(m.mv[0][3]);
                }
            }
        }
        let pred_mv = starts.get(1).copied().unwrap_or(Mv::ZERO);
        let range = (pad - 8 - n as i32).max(0);
        let clamp_px = |v: i32| v.clamp(-range, range);
        let lambda = self.f.lambda;
        let bits = |mv: Mv| mv_comp_bits(i32::from(mv.row) - i32::from(pred_mv.row)) + mv_comp_bits(i32::from(mv.col) - i32::from(pred_mv.col));
        // integer search: SAD on the padded reference
        let sad = |dx: i32, dy: i32| -> u64 {
            let stride = self.f.ref_stride;
            let mut s = 0u64;
            for i in 0..n {
                let ry = (by + i) as i32 + dy + pad;
                let ro = ry as usize * stride + (bx as i32 + dx + pad) as usize;
                let so = (by + i) * self.f.src_stride[0] + bx;
                if let (Some(a), Some(b)) = (self.f.ref_luma.get(ro..ro + n), self.f.src[0].get(so..so + n)) {
                    s += a.iter().zip(b).map(|(&x, &y)| u64::from(x.abs_diff(y))).sum::<u64>();
                }
            }
            s
        };
        let mut best = (u64::MAX, 0i32, 0i32);
        for s in &starts {
            let (dx, dy) = (clamp_px((i32::from(s.col) + 4) >> 3), clamp_px((i32::from(s.row) + 4) >> 3));
            let v = sad(dx, dy);
            if v < best.0 {
                best = (v, dx, dy);
            }
        }
        let steps: &[i32] = if self.f.speed == 0 { &[16, 8, 4, 2, 1] } else { &[8, 4, 2, 1] };
        for &st in steps {
            loop {
                let mut moved = false;
                for (ox, oy) in [(st, 0), (-st, 0), (0, st), (0, -st)] {
                    let (dx, dy) = (clamp_px(best.1 + ox), clamp_px(best.2 + oy));
                    let v = sad(dx, dy);
                    if v < best.0 {
                        best = (v, dx, dy);
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        // sub-sample refinement with the real filters
        let mut mv = Mv::new(best.2 * 8, best.1 * 8);
        let mut cost = self.inter_satd(r, c, bsize, mv) as f64 + lambda * bits(mv);
        for st in [4, 2] {
            let centre = mv;
            for (oy, ox) in [(-st, 0), (st, 0), (0, -st), (0, st), (-st, -st), (-st, st), (st, -st), (st, st)] {
                let cand = Mv::new(i32::from(centre.row) + oy, i32::from(centre.col) + ox);
                let j = self.inter_satd(r, c, bsize, cand) as f64 + lambda * bits(cand);
                if j < cost {
                    cost = j;
                    mv = cand;
                }
            }
        }
        // the zero vector and the predictor as coded alternatives (cheaper to signal)
        for cand in [Mv::ZERO, pred_mv] {
            let j = self.inter_satd(r, c, bsize, cand) as f64 + lambda * (bits(cand) * 0.25);
            if j < cost {
                cost = j;
                mv = cand;
            }
        }
        (cost, mv)
    }

    /// SATD of the luma inter prediction of a block with `mv`.
    fn inter_satd(&mut self, r: usize, c: usize, bsize: u8, mv: Mv) -> u64 {
        let n = 8 * NUM_8X8_WIDE[bsize as usize] as usize;
        if !self.predict_inter(r, c, bsize, 0, mv) {
            return u64::MAX / 4;
        }
        let so = (r * 8) * self.f.src_stride[0] + c * 8;
        satd(self.f.src[0].get(so..).unwrap_or_default(), self.f.src_stride[0], &self.pred, n, n, n)
    }

    /// Inter prediction (8.5.2) of plane `plane` of the block into `self.pred` (stride = width).
    fn predict_inter(&mut self, r: usize, c: usize, bsize: u8, plane: usize, mv: Mv) -> bool {
        let Some(f) = self.f.last else { return false };
        let s = (plane > 0) as i32;
        let bh = NUM_8X8_HIGH[bsize as usize] as i32;
        let bw = NUM_8X8_WIDE[bsize as usize] as i32;
        let (ri, ci) = (r as i32, c as i32);
        let to_top = (-(ri * 8 * 16)) >> s;
        let to_bottom = (((self.f.mi_rows as i32 - bh - ri) * 8) * 16) >> s;
        let to_left = (-(ci * 8 * 16)) >> s;
        let to_right = (((self.f.mi_cols as i32 - bw - ci) * 8) * 16) >> s;
        let spel_left = (INTERP_EXTEND + ((bw * 8) >> s)) << 4;
        let spel_right = spel_left - 16;
        let spel_top = (INTERP_EXTEND + ((bh * 8) >> s)) << 4;
        let spel_bottom = spel_top - 16;
        let cmv_row = ((2 * i32::from(mv.row)) >> s).clamp(to_top - spel_top, (to_bottom + spel_bottom).max(to_top - spel_top));
        let cmv_col = ((2 * i32::from(mv.col)) >> s).clamp(to_left - spel_left, (to_right + spel_right).max(to_left - spel_left));
        let (x, y) = ((c * 8) >> s, (r * 8) >> s);
        let (w, h) = ((bw as usize * 8) >> s, (bh as usize * 8) >> s);
        let rp = RefPlane { frame: f, plane, last_x: ((f.info.width as i32 + s) >> s) - 1, last_y: ((f.info.height as i32 + s) >> s) - 1 };
        inter::predict(&rp, ((x as i32) << 4) + cmv_col, ((y as i32) << 4) + cmv_row, 16, 16, w, h, 0, 8, &mut self.pred, &mut self.mc_tmp, &mut self.mc_win);
        true
    }

    // ----------------------------------------------------------------------------- coding

    fn code_partition(&mut self, bw: &mut BoolEncoder, mi: &mut [MiInfo], r: usize, c: usize, bsize: u8, tree: &Tree) {
        if r >= self.f.mi_rows || c >= self.f.mi_cols {
            return;
        }
        let num8x8 = NUM_8X8_WIDE[bsize as usize] as usize;
        let half = num8x8 >> 1;
        let has_rows = (r + half) < self.f.mi_rows;
        let has_cols = (c + half) < self.f.mi_cols;
        let probs = self.partition_probs(self.partition_ctx(r, c, bsize));
        let (partition, leaf) = match tree {
            Tree::Leaf(ch) => (0u8, Some(*ch)),
            _ => (3u8, None),
        };
        if has_rows && has_cols {
            bw.write_tree(&PARTITION_TREE, &probs, partition);
        } else if has_cols {
            bw.write_bool(partition == 3, probs[1]);
        } else if has_rows {
            bw.write_bool(partition == 3, probs[2]);
        }
        let subsize = SUBSIZE_LOOKUP[partition as usize][bsize as usize];
        match (leaf, tree) {
            (Some(ch), _) => self.code_block(bw, mi, r, c, bsize, ch),
            (None, Tree::Split(kids)) => {
                let pos = [(r, c), (r, c + half), (r + half, c), (r + half, c + half)];
                for ((kr, kc), t) in pos.into_iter().zip(kids.iter()) {
                    self.code_partition(bw, mi, kr, kc, subsize, t);
                }
            }
            _ => {
                // a forced split without a decision below (cannot happen for blocks inside the
                // frame): code every child with DC prediction
                let pos = [(r, c), (r, c + half), (r + half, c), (r + half, c + half)];
                for (kr, kc) in pos {
                    let t = if subsize == BLOCK_8X8 || kr + half / 2 >= self.f.mi_rows || kc + half / 2 >= self.f.mi_cols {
                        Tree::Leaf(Choice { inter: false, y_mode: DC_PRED, uv_mode: DC_PRED, mv: Mv::ZERO })
                    } else {
                        Tree::Outside
                    };
                    self.code_partition(bw, mi, kr, kc, subsize, &t);
                }
            }
        }
        if bsize == BLOCK_8X8 || partition != 3 {
            let a = 15 >> B_WIDTH_LOG2[subsize as usize];
            let l = 15 >> B_HEIGHT_LOG2[subsize as usize];
            let ae = (c + num8x8).min(self.above_partition.len());
            if let Some(s) = self.above_partition.get_mut(c.min(ae)..ae) {
                s.fill(a);
            }
            let le = (r + num8x8).min(self.left_partition.len());
            if let Some(s) = self.left_partition.get_mut(r.min(le)..le) {
                s.fill(l);
            }
        }
    }

    fn mi_get(&self, mi: &[MiInfo], r: usize, c: usize) -> MiInfo {
        mi.get(r * self.f.mi_cols + c).copied().unwrap_or_default()
    }

    /// Reconstruct and code one block.
    fn code_block(&mut self, bw: &mut BoolEncoder, mi: &mut [MiInfo], r: usize, c: usize, bsize: u8, ch: Choice) {
        let avail_u = r > 0;
        let avail_l = c > self.mi_col_start;
        let above = avail_u.then(|| self.mi_get(mi, r - 1, c));
        let left = avail_l.then(|| self.mi_get(mi, r, c - 1));
        let mut info = MiInfo { sb_size: bsize, tx_size: MAX_TXSIZE[bsize as usize], ..MiInfo::default() };
        // motion vector prediction and the inter mode
        let mut inter_mode = ZEROMV;
        let mut best_mv = Mv::ZERO;
        let mut mv = ch.mv;
        if ch.inter {
            self.find_mv_refs(mi, r, c, bsize);
            let (nearest, near) = self.find_best_ref_mvs(r, c, bsize);
            best_mv = nearest;
            inter_mode = if mv == nearest {
                NEARESTMV
            } else if mv == near {
                NEARMV
            } else if mv == Mv::ZERO {
                ZEROMV
            } else {
                NEWMV
            };
            if inter_mode == NEWMV {
                // the difference must be even and within the coded range
                let d = |v: i16, b: i16| (i32::from(v) - i32::from(b)).clamp(-(1 << 13) + 2, (1 << 13) - 2) & !1;
                mv = Mv::new(i32::from(best_mv.row) + d(mv.row, best_mv.row), i32::from(best_mv.col) + d(mv.col, best_mv.col));
                if mv == nearest {
                    inter_mode = NEARESTMV;
                }
            }
            info.ref_frame = [LAST_FRAME, NONE];
            info.y_mode = inter_mode;
            info.sub_modes = [DC_PRED; 4];
            info.mv = [[mv; 4], [Mv::ZERO; 4]];
            info.interp_filter = 0;
        } else {
            info.y_mode = ch.y_mode;
            info.sub_modes = [ch.y_mode; 4];
        }
        // reconstruction: every transform block's levels, then the skip flag
        let mut coded: Vec<Vec<(usize, usize, Option<TxCoefs>)>> = Vec::with_capacity(3);
        let mut any = false;
        for plane in 0..3 {
            let (bx, by, n4w, n4h, tx) = self.plane_geometry(plane, r, c, bsize);
            if ch.inter && self.predict_inter(r, c, bsize, plane, mv) {
                let (w, h) = (n4w * 4, n4h * 4);
                let (stride, off) = (self.strides[plane], self.x_off[plane]);
                for i in 0..h {
                    let o = (by + i) * stride + bx - off;
                    if let (Some(d), Some(s)) = (self.planes[plane].get_mut(o..o + w), self.pred.get(i * w..i * w + w)) {
                        d.copy_from_slice(s);
                    }
                }
            }
            let mut blocks = Vec::new();
            let step = 1usize << tx;
            for y4 in (0..n4h).step_by(step) {
                for x4 in (0..n4w).step_by(step) {
                    let (x, y) = (bx + 4 * x4, by + 4 * y4);
                    let tx_type = if plane > 0 || tx == TX_32X32 || ch.inter { DCT_DCT } else { MODE2TXFM[ch.y_mode as usize] };
                    if !ch.inter {
                        let mode = if plane > 0 { ch.uv_mode } else { ch.y_mode };
                        let e = self.intra_edge(plane, avail_l, avail_u, x4, y4, n4w, tx);
                        let (stride, off) = (self.strides[plane], self.x_off[plane]);
                        intra::predict(&mut self.planes[plane], stride, off, x, y, mode, &e);
                    }
                    let coefs = self.quantize_block(plane, x, y, tx, tx_type, ch.inter);
                    if let Some(t) = &coefs {
                        any |= !t.levels.is_empty();
                    }
                    blocks.push((x, y, coefs));
                }
            }
            coded.push(blocks);
        }
        let skip = !any;
        info.skip = skip;
        // mode info
        if self.f.key {
            let ctx = above.map_or(0, |m| m.skip as usize) + left.map_or(0, |m| m.skip as usize);
            bw.write_bool(skip, self.f.fc.skip[ctx.min(2)]);
            let a = above.map_or(DC_PRED, |m| m.sub_modes[2]);
            let l = left.map_or(DC_PRED, |m| m.sub_modes[1]);
            let o = (a as usize * 10 + l as usize) * 9;
            bw.write_tree(&INTRA_MODE_TREE, KF_Y_MODE_PROBS.get(o..o + 9).unwrap_or(&[128; 9]), ch.y_mode);
            let o = ch.y_mode as usize * 9;
            bw.write_tree(&INTRA_MODE_TREE, KF_UV_MODE_PROBS.get(o..o + 9).unwrap_or(&[128; 9]), ch.uv_mode);
        } else {
            let left_ref = left.map_or([INTRA_FRAME, NONE], |m| m.ref_frame);
            let above_ref = above.map_or([INTRA_FRAME, NONE], |m| m.ref_frame);
            let (left_intra, above_intra) = (left_ref[0] <= INTRA_FRAME, above_ref[0] <= INTRA_FRAME);
            let ctx = above.map_or(0, |m| m.skip as usize) + left.map_or(0, |m| m.skip as usize);
            bw.write_bool(skip, self.f.fc.skip[ctx.min(2)]);
            let ctx = if avail_u && avail_l {
                if left_intra && above_intra { 3 } else { (left_intra || above_intra) as usize }
            } else if avail_u || avail_l {
                2 * (if avail_u { above_intra } else { left_intra }) as usize
            } else {
                0
            };
            bw.write_bool(ch.inter, self.f.fc.is_inter[ctx]);
            if ch.inter {
                let ctx = single_ref_p1_ctx(avail_u, avail_l, above_ref, left_ref);
                bw.write_bool(false, self.f.fc.single_ref[ctx][0]);
                let ctx = (self.mode_context[LAST_FRAME as usize] as usize).min(6);
                bw.write_tree(&INTER_MODE_TREE, &self.f.fc.inter_mode[ctx], inter_mode - NEARESTMV);
                if inter_mode == NEWMV {
                    self.write_mv(bw, mv, best_mv);
                }
            } else {
                let ctx = SIZE_GROUP[bsize as usize] as usize;
                bw.write_tree(&INTRA_MODE_TREE, &self.f.fc.y_mode[ctx], ch.y_mode);
                bw.write_tree(&INTRA_MODE_TREE, &self.f.fc.uv_mode[ch.y_mode as usize], ch.uv_mode);
            }
        }
        // tokens
        for (plane, blocks) in coded.iter().enumerate() {
            let tx = self.plane_geometry(plane, r, c, bsize).4;
            let step = 1usize << tx;
            for (x, y, coefs) in blocks {
                let nonzero = match coefs {
                    Some(t) if !skip => {
                        let tx_type = if plane > 0 || tx == TX_32X32 || ch.inter { DCT_DCT } else { MODE2TXFM[ch.y_mode as usize] };
                        self.write_tokens(bw, plane, *x, *y, tx, tx_type, ch.inter, &t.levels);
                        !t.levels.is_empty()
                    }
                    _ => false,
                };
                let (ax, ly) = (x >> 2, y >> 2);
                let an = &mut self.above_nonzero[plane];
                let e = (ax + step).min(an.len());
                if let Some(s) = an.get_mut(ax.min(e)..e) {
                    s.fill(nonzero as u8);
                }
                let ln = &mut self.left_nonzero[plane];
                let e = (ly + step).min(ln.len());
                if let Some(s) = ln.get_mut(ly.min(e)..e) {
                    s.fill(nonzero as u8);
                }
            }
        }
        let n8 = NUM_8X8_WIDE[bsize as usize] as usize;
        for yy in r..(r + n8).min(self.f.mi_rows) {
            for xx in c..(c + n8).min(self.mi_col_end) {
                if let Some(m) = mi.get_mut(yy * self.f.mi_cols + xx) {
                    *m = info;
                }
            }
        }
    }

    /// Residual of the transform block at (`x`, `y`) against the prediction in the
    /// reconstruction buffer: transformed, quantized, and added back (the reconstruction the
    /// decoder makes). `None` for a block outside the frame.
    #[allow(clippy::too_many_arguments)]
    fn quantize_block(&mut self, plane: usize, x: usize, y: usize, tx: u8, tx_type: u8, inter: bool) -> Option<TxCoefs> {
        let n = 4usize << tx;
        let (stride, off) = (self.strides[plane], self.x_off[plane]);
        for i in 0..n {
            for j in 0..n {
                let s = i32::from(self.src_at(plane, x + j, y + i));
                let p = i32::from(self.planes[plane].get((y + i) * stride + x + j - off).copied().unwrap_or(0));
                self.res[i * n + j] = s - p;
            }
        }
        fdct::basis(tx, tx_type).forward(&self.res[..n * n], &mut self.fwd[..n * n]);
        let q = self.f.q[(plane > 0) as usize];
        let dq_shift = (tx == TX_32X32) as i64;
        let sc = scan(tx, tx_type);
        let mut levels = Vec::new();
        let mut eob = 0usize;
        let mut max_row = 0usize;
        let log2n = 2 + tx as usize;
        // rounding: a dead zone that leans towards zero more for inter blocks
        let bias = if inter { 0.30 } else { 0.38 };
        for (k, &pos) in sc.iter().enumerate().take(n * n) {
            let pos = pos as usize;
            let qv = i64::from(if pos == 0 { q[0] } else { q[1] }).max(1);
            let v = self.fwd.get(pos).copied().unwrap_or(0.0);
            let scaled = v.abs() * (1i64 << dq_shift) as f64 / qv as f64;
            let mag = ((scaled + bias).floor() as i64).min(16_384 + 66);
            let level = if v < 0.0 { -mag } else { mag };
            levels.push(level as i32);
            // the decoder dequantizes the magnitude, then applies the sign
            let d = ((mag * qv) >> dq_shift) as i32;
            self.deq[pos] = if level < 0 { -d } else { d };
            if level != 0 {
                eob = k + 1;
                max_row = max_row.max(pos >> log2n);
            }
        }
        levels.truncate(eob);
        if eob > 0 {
            let o = y * stride + x - off;
            if let Some(dst) = self.planes[plane].get_mut(o..) {
                transform::inverse_transform_add(&self.deq, tx, tx_type, false, eob, max_row + 1, 8, dst, stride);
            }
        }
        for &pos in sc.iter().take(n * n) {
            if let Some(d) = self.deq.get_mut(pos as usize) {
                *d = 0;
            }
        }
        Some(TxCoefs { levels })
    }

    /// tokens (6.4.24), written.
    #[allow(clippy::too_many_arguments)]
    fn write_tokens(&mut self, bw: &mut BoolEncoder, plane: usize, x: usize, y: usize, tx: u8, tx_type: u8, inter: bool, levels: &[i32]) {
        let seg_eob = 16usize << (tx << 1);
        let sc = scan(tx, tx_type);
        let nb = neighbors(tx, tx_type);
        let bands: &[u8] = if tx == 0 { &COEFBAND_4X4 } else { &COEFBAND_8X8PLUS };
        let ptype = (plane > 0) as usize;
        let probs = &self.f.fc.coef[tx as usize][ptype][inter as usize];
        let s = (plane > 0) as usize;
        let max_x4 = (2 * self.f.mi_cols) >> s;
        let max_y4 = (2 * self.f.mi_rows) >> s;
        let n = 1usize << tx;
        let (x4, y4) = (x >> 2, y >> 2);
        let (mut above, mut left) = (0u8, 0u8);
        for i in 0..n {
            if x4 + i < max_x4 {
                above |= self.above_nonzero[plane].get(x4 + i).copied().unwrap_or(0);
            }
            if y4 + i < max_y4 {
                left |= self.left_nonzero[plane].get(y4 + i).copied().unwrap_or(0);
            }
        }
        let ctx0 = (above + left) as usize;
        let pareto = pareto_full();
        let eob = levels.len();
        let mut check_eob = true;
        let tc = &mut self.token_cache;
        let mut c = 0usize;
        while c < seg_eob {
            let pos = sc.get(c).copied().unwrap_or(0) as usize;
            let band = bands.get(c).copied().unwrap_or(5) as usize;
            let ctx = if c == 0 {
                ctx0
            } else {
                let (a, b) = nb.get(c).copied().unwrap_or((0, 0));
                ((1 + tc[a as usize] + tc[b as usize]) >> 1) as usize
            };
            let p = probs[band][ctx];
            if check_eob {
                bw.write_bool(c < eob, p[0]);
                if c >= eob {
                    break;
                }
            }
            let v = levels.get(c).copied().unwrap_or(0);
            let mag = v.unsigned_abs();
            if mag == 0 {
                bw.write_bool(false, p[1]);
                tc[pos] = 0;
                check_eob = false;
                c += 1;
                continue;
            }
            bw.write_bool(true, p[1]);
            check_eob = true;
            let token: usize = match mag {
                1 => 1,
                2 => 2,
                3 => 3,
                4 => 4,
                5..=6 => 5,
                7..=10 => 6,
                11..=18 => 7,
                19..=34 => 8,
                35..=66 => 9,
                _ => 10,
            };
            if token == 1 {
                bw.write_bool(false, p[2]);
            } else {
                bw.write_bool(true, p[2]);
                let pp = &pareto[p[2].max(1) as usize - 1];
                match token {
                    2 => {
                        bw.write_bool(false, pp[0]);
                        bw.write_bool(false, pp[1]);
                    }
                    3 | 4 => {
                        bw.write_bool(false, pp[0]);
                        bw.write_bool(true, pp[1]);
                        bw.write_bool(token == 4, pp[2]);
                    }
                    5 | 6 => {
                        bw.write_bool(true, pp[0]);
                        bw.write_bool(false, pp[3]);
                        bw.write_bool(token == 6, pp[4]);
                    }
                    7 | 8 => {
                        bw.write_bool(true, pp[0]);
                        bw.write_bool(true, pp[3]);
                        bw.write_bool(false, pp[5]);
                        bw.write_bool(token == 8, pp[6]);
                    }
                    _ => {
                        bw.write_bool(true, pp[0]);
                        bw.write_bool(true, pp[3]);
                        bw.write_bool(true, pp[5]);
                        bw.write_bool(token == 10, pp[7]);
                    }
                }
                let (cat, num_extra, base) = EXTRA_BITS[token];
                let extra = mag.saturating_sub(u32::from(base)).min((1 << num_extra) - 1);
                let cp = CAT_PROBS[cat as usize];
                for e in 0..num_extra as usize {
                    bw.write_bool((extra >> (num_extra as usize - 1 - e)) & 1 == 1, cp.get(e).copied().unwrap_or(128));
                }
            }
            tc[pos] = ENERGY_CLASS[token];
            bw.write_bool(v < 0, 128);
            c += 1;
        }
    }

    /// A NEWMV vector relative to `best` (6.4.19 / 6.4.20, without high precision).
    fn write_mv(&self, bw: &mut BoolEncoder, mv: Mv, best: Mv) {
        let fc = self.f.fc;
        let diff = [i32::from(mv.row) - i32::from(best.row), i32::from(mv.col) - i32::from(best.col)];
        let joint = match (diff[0] != 0, diff[1] != 0) {
            (false, false) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (true, true) => 3,
        };
        bw.write_tree(&MV_JOINT_TREE, &fc.mv_joint, joint);
        for (comp, &d) in diff.iter().enumerate() {
            if d == 0 {
                continue;
            }
            bw.write_bool(d < 0, fc.mv_sign[comp]);
            // magnitude - 1 = offset; without high precision the offset is odd (hp bit 1)
            let offset = d.unsigned_abs() - 1;
            if offset < 16 {
                bw.write_tree(&MV_CLASS_TREE, &fc.mv_class[comp], 0);
                bw.write_bool((offset >> 3) & 1 == 1, fc.mv_class0_bit[comp]);
                bw.write_tree(&MV_FR_TREE, &fc.mv_class0_fr[comp][(offset >> 3) as usize & 1], ((offset >> 1) & 3) as u8);
            } else {
                let class = (31 - (offset >> 3).leading_zeros()) as usize;
                let class = class.min(10);
                bw.write_tree(&MV_CLASS_TREE, &fc.mv_class[comp], class as u8);
                let rest = offset - (2u32 << (class + 2));
                let dbits = rest >> 3;
                for i in 0..class {
                    bw.write_bool((dbits >> i) & 1 == 1, fc.mv_bits[comp][i]);
                }
                bw.write_tree(&MV_FR_TREE, &fc.mv_fr[comp], ((rest >> 1) & 3) as u8);
            }
        }
    }

    // ------------------------------------------------- motion vector prediction (6.5), mirrored

    fn is_inside(&self, r: isize, c: isize) -> bool {
        r >= 0 && (r as usize) < self.f.mi_rows && c >= self.mi_col_start as isize && (c as usize) < self.mi_col_end
    }

    fn add_mv_ref_list(&mut self, mv: Mv) {
        if self.ref_mv_count >= 2 || (self.ref_mv_count > 0 && mv == self.ref_list_mv[0]) {
            return;
        }
        self.ref_list_mv[self.ref_mv_count] = mv;
        self.ref_mv_count += 1;
    }

    fn if_same_ref_frame_add(&mut self, m: &MiInfo, ref_frame: i8) {
        for j in 0..2 {
            if m.ref_frame[j] == ref_frame {
                self.add_mv_ref_list(m.mv[j][3]);
                return;
            }
        }
    }

    fn if_diff_ref_frame_add(&mut self, m: &MiInfo, ref_frame: i8) {
        // every reference has sign bias 0: no vector is inverted
        let mvs = [m.mv[0][3], m.mv[1][3]];
        if m.ref_frame[0] > INTRA_FRAME && m.ref_frame[0] != ref_frame {
            self.add_mv_ref_list(mvs[0]);
        }
        if m.ref_frame[1] > INTRA_FRAME && m.ref_frame[1] != ref_frame && mvs[0] != mvs[1] {
            self.add_mv_ref_list(mvs[1]);
        }
    }

    fn clamp_mv(&self, r: usize, c: usize, bsize: u8, mv: Mv, border: i32) -> Mv {
        let bh = NUM_8X8_HIGH[bsize as usize] as i32;
        let bw = NUM_8X8_WIDE[bsize as usize] as i32;
        let to_top = -((r as i32 * 8) * 8);
        let to_bottom = ((self.f.mi_rows as i32 - bh - r as i32) * 8) * 8;
        let to_left = -((c as i32 * 8) * 8);
        let to_right = ((self.f.mi_cols as i32 - bw - c as i32) * 8) * 8;
        Mv::new(
            i32::from(mv.row).clamp(to_top - border, (to_bottom + border).max(to_top - border)),
            i32::from(mv.col).clamp(to_left - border, (to_right + border).max(to_left - border)),
        )
    }

    /// find_mv_refs (6.5.1) for LAST.
    fn find_mv_refs(&mut self, mi: &[MiInfo], r: usize, c: usize, bsize: u8) {
        let ref_frame = LAST_FRAME;
        self.ref_mv_count = 0;
        self.ref_list_mv = [Mv::ZERO; 2];
        let mut different_ref_found = false;
        let mut context_counter = 0usize;
        let o = bsize as usize * 16;
        let search = MV_REF_BLOCKS.get(o..o + 16).unwrap_or(&[0; 16]);
        let (ri, ci) = (r as isize, c as isize);
        let at = |i: usize| (ri + search[i * 2] as isize, ci + search[i * 2 + 1] as isize);
        for i in 0..2 {
            let (cr, cc) = at(i);
            if self.is_inside(cr, cc) {
                let m = self.mi_get(mi, cr as usize, cc as usize);
                different_ref_found = true;
                context_counter += MODE_2_COUNTER[m.y_mode as usize] as usize;
                for j in 0..2 {
                    if m.ref_frame[j] == ref_frame {
                        self.add_mv_ref_list(m.mv[j][3]);
                        break;
                    }
                }
            }
        }
        for i in 2..8 {
            let (cr, cc) = at(i);
            if self.is_inside(cr, cc) {
                different_ref_found = true;
                let m = self.mi_get(mi, cr as usize, cc as usize);
                self.if_same_ref_frame_add(&m, ref_frame);
            }
        }
        let prev = self.f.prev_mi.and_then(|g| g.mi.get(r * g.cols + c).copied());
        if let Some(p) = &prev {
            self.if_same_ref_frame_add(p, ref_frame);
        }
        if different_ref_found {
            for i in 0..8 {
                let (cr, cc) = at(i);
                if self.is_inside(cr, cc) {
                    let m = self.mi_get(mi, cr as usize, cc as usize);
                    self.if_diff_ref_frame_add(&m, ref_frame);
                }
            }
        }
        if let Some(p) = &prev {
            self.if_diff_ref_frame_add(p, ref_frame);
        }
        self.mode_context[ref_frame as usize] = COUNTER_TO_CONTEXT[context_counter.min(18)];
        for i in 0..2 {
            self.ref_list_mv[i] = self.clamp_mv(r, c, bsize, self.ref_list_mv[i], MV_BORDER);
        }
    }

    /// find_best_ref_mvs (6.5.12) without high precision: (nearest, near).
    fn find_best_ref_mvs(&mut self, r: usize, c: usize, bsize: u8) -> (Mv, Mv) {
        for i in 0..2 {
            let mv = self.ref_list_mv[i];
            let (mut dr, mut dc) = (i32::from(mv.row), i32::from(mv.col));
            if dr & 1 != 0 {
                dr += if dr > 0 { -1 } else { 1 };
            }
            if dc & 1 != 0 {
                dc += if dc > 0 { -1 } else { 1 };
            }
            let border = (BORDERINPIXELS - INTERP_EXTEND) << 3;
            self.ref_list_mv[i] = self.clamp_mv(r, c, bsize, Mv::new(dr, dc), border);
        }
        (self.ref_list_mv[0], self.ref_list_mv[1])
    }
}

/// The context of single_ref_p1 (9.3.1).
fn single_ref_p1_ctx(avail_u: bool, avail_l: bool, a: [i8; 2], l: [i8; 2]) -> usize {
    let (above_intra, left_intra) = (a[0] <= INTRA_FRAME, l[0] <= INTRA_FRAME);
    let (above_single, left_single) = (a[1] <= NONE, l[1] <= NONE);
    (if avail_u && avail_l {
        if above_intra && left_intra {
            2
        } else if left_intra {
            if above_single { 4 * (a[0] == LAST_FRAME) as u8 } else { 1 + (a[0] == LAST_FRAME || a[1] == LAST_FRAME) as u8 }
        } else if above_intra {
            if left_single { 4 * (l[0] == LAST_FRAME) as u8 } else { 1 + (l[0] == LAST_FRAME || l[1] == LAST_FRAME) as u8 }
        } else if above_single && left_single {
            2 * (a[0] == LAST_FRAME) as u8 + 2 * (l[0] == LAST_FRAME) as u8
        } else if !above_single && !left_single {
            1 + (a[0] == LAST_FRAME || a[1] == LAST_FRAME || l[0] == LAST_FRAME || l[1] == LAST_FRAME) as u8
        } else {
            let rfs = if above_single { a[0] } else { l[0] };
            let crf1 = if above_single { l[0] } else { a[0] };
            let crf2 = if above_single { l[1] } else { a[1] };
            if rfs == LAST_FRAME { 3 + (crf1 == LAST_FRAME || crf2 == LAST_FRAME) as u8 } else { (crf1 == LAST_FRAME || crf2 == LAST_FRAME) as u8 }
        }
    } else if avail_u {
        if above_intra {
            2
        } else if above_single {
            4 * (a[0] == LAST_FRAME) as u8
        } else {
            1 + (a[0] == LAST_FRAME || a[1] == LAST_FRAME) as u8
        }
    } else if avail_l {
        if left_intra {
            2
        } else if left_single {
            4 * (l[0] == LAST_FRAME) as u8
        } else {
            1 + (l[0] == LAST_FRAME || l[1] == LAST_FRAME) as u8
        }
    } else {
        2
    }) as usize
}
