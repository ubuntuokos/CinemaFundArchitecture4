//! Partition analysis and motion search.
//!
//! The partition of each superblock is chosen before it is coded, from the source alone (intra
//! predictors built from neighbouring source samples, integer motion search against the
//! reference frame), each costed as squared error plus estimated coefficient bits after
//! quantizing its residual: the cost of a block is compared with the cost of its four quarters. Block
//! modes are then chosen against the actual reconstruction when the block is coded
//! (`policy.rs`); the motion vectors found here seed that search.

use crate::inter::{PRED_STRIDE, rounding};
use crate::spec_tables::*;
use crate::symbol::{SymbolIo, symbol_cost};
use crate::tile::TileDecoder;

use super::policy::{Plan, TileEnc, satd};

/// Cost (in 1/256 bits) of a motion vector difference in whole samples, before the coder's
/// CDFs are known: about two bits per doubling of each component.
fn mv_bits_estimate(d: [i32; 2]) -> u32 {
    let comp = |v: i32| -> u32 { if v == 0 { 0 } else { 256 * (2 + 2 * (32 - (v.unsigned_abs() >> 3).leading_zeros())) } };
    256 + comp(d[0]) + comp(d[1])
}

impl<S: SymbolIo> TileDecoder<'_, '_, S> {
    /// Analyse the square block (r, c, bsize) of the current superblock: returns its cost and
    /// records in `enc.split` whether it is better split.
    pub(crate) fn analyse(&mut self, enc: &mut TileEnc, r: usize, c: usize, bsize: usize) -> u64 {
        let (mi_rows, mi_cols) = (self.fs.fh.mi_rows as usize, self.fs.fh.mi_cols as usize);
        if r >= mi_rows || c >= mi_cols {
            return 0;
        }
        let n4 = NUM_4X4_BLOCKS_WIDE[bsize] as usize;
        let half = n4 >> 1;
        let node = enc.node(r, c, bsize);
        let forced = bsize > BLOCK_8X8 && !(r + half < mi_rows && c + half < mi_cols);
        let lambda = enc.fe.lambda_sse;
        let none = if forced { u64::MAX } else { self.analyse_block(enc, r, c, bsize, node) };
        if bsize <= BLOCK_8X8 {
            return none;
        }
        // Flat blocks are not split on the fastest setting.
        if !forced && enc.fe.effort == 0 && none < (lambda * 8.0) as u64 {
            return none;
        }
        let sub = PARTITION_SUBSIZE[PARTITION_SPLIT][bsize] as usize;
        let mut split = (lambda * enc.fe.split_bits) as u64;
        for (dr, dc) in [(0, 0), (0, half), (half, 0), (half, half)] {
            split = split.saturating_add(self.analyse(enc, r + dr, c + dc, sub));
            if split >= none {
                break;
            }
        }
        if split < none {
            if let Some(n) = node {
                enc.split[n] = true;
            }
            split
        } else {
            none
        }
    }

    /// Cost of coding the block whole: the best of a few intra predictors from the neighbouring
    /// source samples and (inter frames) the best integer motion vector.
    fn analyse_block(&mut self, enc: &mut TileEnc, r: usize, c: usize, bsize: usize, node: Option<usize>) -> u64 {
        let lambda = enc.fe.lambda_sse;
        let w = 4 * NUM_4X4_BLOCKS_WIDE[bsize] as usize;
        let h = 4 * NUM_4X4_BLOCKS_HIGH[bsize] as usize;
        let (x, y) = (c * 4, r * 4);
        let have_above = r > self.mi_row_start;
        let have_left = c > self.mi_col_start;
        let tx = MAX_TX_SIZE_RECT[bsize] as usize;
        self.source_intra_cost(enc, x, y, w, h, have_above, have_left);
        let intra = self.rd_residual(enc, tx, enc.fe.round_intra).saturating_add((lambda * 4.0) as u64);
        if self.fs.fh.frame_is_intra {
            return intra;
        }
        // integer motion search seeded by the neighbours' vectors
        let mut seeds: Vec<[i32; 2]> = vec![[0, 0]];
        let mi = &self.t.mi;
        if have_left {
            let m = mi.mv[mi.idx(r, c - 1)][0];
            seeds.push([m.row as i32, m.col as i32]);
        }
        if have_above {
            let m = mi.mv[mi.idx(r - 1, c)][0];
            seeds.push([m.row as i32, m.col as i32]);
        }
        if let Some(n) = node
            && n > 0
        {
            // the parent's vector
            let parent = match n {
                1..=4 => 0,
                5..=20 => {
                    let i = n - 5;
                    1 + ((i >> 3) << 1) + ((i & 3) >> 1)
                }
                _ => {
                    let i = n - 21;
                    5 + ((i >> 4) << 2) + ((i & 7) >> 1)
                }
            };
            seeds.push(enc.amv[parent]);
        }
        let (mv, _) = self.integer_search(enc, x, y, w, h, &seeds);
        if let Some(n) = node {
            enc.amv[n] = mv;
        }
        let inter = self.rd_residual(enc, tx, enc.fe.round_inter).saturating_add((lambda * (mv_bits_estimate(mv) as f64 / 256.0 + 2.0)) as u64);
        intra.min(inter)
    }

    /// Lowest SATD of DC / V / H / Paeth predictions built from the source's neighbouring
    /// samples; the residual of the best one is left in `enc.diff`.
    fn source_intra_cost(&self, enc: &mut TileEnc, x: usize, y: usize, w: usize, h: usize, have_above: bool, have_left: bool) -> u64 {
        let src = &enc.src.planes[0];
        let base = 1i32 << (self.fs.bit_depth - 1);
        let above: Vec<i32> = (0..w)
            .map(|i| {
                if have_above {
                    src.at(x + i, y - 1) as i32
                } else if have_left {
                    src.at(x - 1, y) as i32
                } else {
                    base - 1
                }
            })
            .collect();
        let left: Vec<i32> = (0..h)
            .map(|i| {
                if have_left {
                    src.at(x - 1, y + i) as i32
                } else if have_above {
                    src.at(x, y - 1) as i32
                } else {
                    base + 1
                }
            })
            .collect();
        let corner = if have_above && have_left {
            src.at(x - 1, y - 1) as i32
        } else if have_above {
            above[0]
        } else if have_left {
            left[0]
        } else {
            base
        };
        let dc = match (have_above, have_left) {
            (true, true) => (above.iter().sum::<i32>() + left.iter().sum::<i32>() + ((w + h) as i32 >> 1)) / (w + h) as i32,
            (true, false) => (above.iter().sum::<i32>() + (w as i32 >> 1)) / w as i32,
            (false, true) => (left.iter().sum::<i32>() + (h as i32 >> 1)) / h as i32,
            _ => base,
        };
        let mut best = (u64::MAX, 0);
        for mode in [0, 1, 2, 3, 4] {
            // 4: the best mode again, to leave its residual
            if mode == 4 && best.1 == 3 {
                break;
            }
            let mode = if mode == 4 { best.1 } else { mode };
            enc.diff.clear();
            enc.diff.resize(w * h, 0);
            for i in 0..h {
                let row = &src.row_from(y + i, x)[..w];
                for j in 0..w {
                    let p = match mode {
                        0 => dc,
                        1 => above[j],
                        2 => left[i],
                        _ => {
                            let (t, l) = (above[j], left[i]);
                            let pb = t + l - corner;
                            let (pl, pt, ptl) = ((pb - l).abs(), (pb - t).abs(), (pb - corner).abs());
                            if pl <= pt && pl <= ptl {
                                l
                            } else if pt <= ptl {
                                t
                            } else {
                                corner
                            }
                        }
                    };
                    enc.diff[i * w + j] = row[j] as i32 - p;
                }
            }
            let d = satd(&enc.diff, w, w, h);
            if d < best.0 {
                best = (d, mode);
            }
        }
        best.0
    }

    fn ref_idx(&self) -> i32 {
        self.fs.fh.ref_frame_idx[0] as i32
    }

    /// SAD of the source block at (x, y) against the reference at integer offset (dx, dy).
    fn sad_at(&self, enc: &TileEnc, x: usize, y: usize, w: usize, h: usize, dx: i32, dy: i32, limit: u64) -> u64 {
        let ref_idx = self.ref_idx();
        let refp = self.ref_plane(ref_idx, 0);
        let (last_x, last_y) = self.ref_plane_dims(ref_idx, 0);
        let src = &enc.src.planes[0];
        let (rx, ry) = (x as i32 + dx, y as i32 + dy);
        let inside = rx >= 0 && ry >= 0 && rx + w as i32 - 1 <= last_x && ry + h as i32 - 1 <= last_y;
        let mut sad = 0u64;
        for i in 0..h {
            let s = &src.row_from(y + i, x)[..w];
            if inside {
                let r = &refp.row_from((ry + i as i32) as usize, rx as usize)[..w];
                sad += s.iter().zip(r).map(|(&a, &b)| (a as i32 - b as i32).unsigned_abs() as u64).sum::<u64>();
            } else {
                let yy = (ry + i as i32).clamp(0, last_y) as usize;
                let row = refp.row(yy);
                for (j, &a) in s.iter().enumerate() {
                    let xx = (rx + j as i32).clamp(0, last_x) as usize;
                    sad += (a as i32 - row[xx] as i32).unsigned_abs() as u64;
                }
            }
            if sad > limit {
                return sad;
            }
        }
        sad
    }

    /// Integer-sample motion search (diamond steps from the best seed); returns the vector (1/8
    /// units) and its SATD.
    fn integer_search(&self, enc: &mut TileEnc, x: usize, y: usize, w: usize, h: usize, seeds: &[[i32; 2]]) -> ([i32; 2], u64) {
        let lambda = enc.fe.lambda;
        let range = enc.fe.range.max(1);
        let fw = self.fs.fh.frame_width as i32;
        let fhh = self.fs.fh.frame_height as i32;
        // keep the block within 64 samples of the picture and the search range
        let ok = |dx: i32, dy: i32| -> bool {
            dx.abs() <= range * 2
                && dy.abs() <= range * 2
                && x as i32 + dx > -(w as i32) - 64
                && y as i32 + dy > -(h as i32) - 64
                && x as i32 + dx < fw + 64
                && y as i32 + dy < fhh + 64
        };
        let cost = |s: &Self, enc: &TileEnc, dx: i32, dy: i32, limit: u64| -> u64 {
            let bits = mv_bits_estimate([dy * 8, dx * 8]);
            let pen = (lambda * bits as f64 / 256.0) as u64;
            s.sad_at(enc, x, y, w, h, dx, dy, limit.saturating_sub(pen)).saturating_add(pen)
        };
        let mut best = (0i32, 0i32);
        let mut best_cost = u64::MAX;
        for s in seeds {
            let (dx, dy) = ((s[1] + 4) >> 3, (s[0] + 4) >> 3);
            if !ok(dx, dy) {
                continue;
            }
            let c = cost(self, enc, dx, dy, best_cost);
            if c < best_cost {
                best_cost = c;
                best = (dx, dy);
            }
        }
        let center = best;
        let mut step = 1;
        while step * 2 <= range.min(16) {
            step *= 2;
        }
        while step >= 1 {
            loop {
                let mut moved = false;
                for (ox, oy) in [(step, 0), (-step, 0), (0, step), (0, -step)] {
                    let (dx, dy) = (best.0 + ox, best.1 + oy);
                    if !ok(dx, dy) || (dx - center.0).abs() > range || (dy - center.1).abs() > range {
                        continue;
                    }
                    let c = cost(self, enc, dx, dy, best_cost);
                    if c < best_cost {
                        best_cost = c;
                        best = (dx, dy);
                        moved = true;
                    }
                }
                if !moved || step > 1 {
                    break;
                }
            }
            step /= 2;
        }
        if enc.fe.effort >= 2 {
            // exhaustive refinement around the best point
            let b = best;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if (dx, dy) != (0, 0) && ok(b.0 + dx, b.1 + dy) {
                        let c = cost(self, enc, b.0 + dx, b.1 + dy, best_cost);
                        if c < best_cost {
                            best_cost = c;
                            best = (b.0 + dx, b.1 + dy);
                        }
                    }
                }
            }
        }
        let mv = [best.1 * 8, best.0 * 8];
        // SATD at the chosen vector, comparable with the intra costs
        let ref_idx = self.ref_idx();
        let refp = self.ref_plane(ref_idx, 0);
        let (last_x, last_y) = self.ref_plane_dims(ref_idx, 0);
        let src = &enc.src.planes[0];
        enc.diff.clear();
        enc.diff.resize(w * h, 0);
        for i in 0..h {
            let yy = (y as i32 + best.1 + i as i32).clamp(0, last_y) as usize;
            let row = refp.row(yy);
            let s = &src.row_from(y + i, x)[..w];
            for j in 0..w {
                let xx = (x as i32 + best.0 + j as i32).clamp(0, last_x) as usize;
                enc.diff[i * w + j] = s[j] as i32 - row[xx] as i32;
            }
        }
        (mv, satd(&enc.diff, w, w, h))
    }

    /// Inter prediction of the block's `plane` from LAST_FRAME with `mv` into `pred_buf[0]`.
    fn inter_pred(&mut self, plane: usize, mv: [i32; 2]) -> (usize, usize, usize, usize) {
        let (ssx, ssy) = if plane > 0 { (self.fs.ssx, self.fs.ssy) } else { (0, 0) };
        let x = (self.b.mi_col >> ssx) * 4;
        let y = (self.b.mi_row >> ssy) * 4;
        let psz = self.fs.plane_residual_size(self.b.mi_size, plane);
        let w = 4 * NUM_4X4_BLOCKS_WIDE[psz] as usize;
        let h = 4 * NUM_4X4_BLOCKS_HIGH[psz] as usize;
        let ref_idx = self.ref_idx();
        let rnd = rounding(false, self.fs.bit_depth);
        let (sx, sy, stx, sty) = self.mv_scaling(plane, ref_idx, x as i32, y as i32, mv);
        let mut pred = std::mem::take(&mut self.pred_buf[0]);
        let mut tmp = std::mem::take(&mut self.mc_tmp);
        let f = self.fs.fh.interpolation_filter;
        self.block_inter_prediction(plane, ref_idx, sx, sy, stx, sty, w, h, [f, f], rnd, &mut pred, &mut tmp);
        self.pred_buf[0] = pred;
        self.mc_tmp = tmp;
        (x, y, w, h)
    }

    /// Write the inter prediction of `plane` into the current frame.
    pub(crate) fn predict_block_inter(&mut self, plane: usize, mv: [i32; 2]) {
        let (x, y, w, h) = self.inter_pred(plane, mv);
        let max = (1i32 << self.fs.bit_depth) - 1;
        let pred = &self.pred_buf[0];
        let pl = &mut self.t.cur.planes[plane];
        for i in 0..h {
            let row = &mut pl.row_from_mut(y + i, x)[..w];
            for (j, o) in row.iter_mut().enumerate() {
                *o = pred[i * PRED_STRIDE + j].clamp(0, max) as u16;
            }
        }
    }

    /// Luma SATD of the inter prediction with `mv`.
    fn inter_satd(&mut self, enc: &mut TileEnc, mv: [i32; 2]) -> u64 {
        let (x, y, w, h) = self.inter_pred(0, mv);
        let max = (1i32 << self.fs.bit_depth) - 1;
        let src = &enc.src.planes[0];
        enc.diff.clear();
        enc.diff.resize(w * h, 0);
        let pred = &self.pred_buf[0];
        for i in 0..h {
            let s = &src.row_from(y + i, x)[..w];
            for j in 0..w {
                enc.diff[i * w + j] = s[j] as i32 - pred[i * PRED_STRIDE + j].clamp(0, max);
            }
        }
        satd(&enc.diff, w, w, h)
    }

    /// The best inter mode (LAST_FRAME) for the block, with its rate-distortion cost (the residual
    /// of its prediction is left in `enc.diff`).
    pub(crate) fn plan_inter(&mut self, enc: &mut TileEnc) -> Option<(u64, Plan)> {
        let lambda = enc.fe.lambda;
        let lam = |bits: u32| -> u64 { (lambda * bits as f64 / 256.0) as u64 };
        self.b.is_inter = true;
        self.b.ref_frame = [LAST_FRAME as i8, -1];
        self.b.interp_filter = [self.fs.fh.interpolation_filter; 2];
        self.b.motion_mode = SIMPLE as u8;
        self.b.compound_type = COMPOUND_AVERAGE;
        self.b.y_mode = NEARESTMV;
        self.find_mv_stack(false);
        let st = self.mvs.clone();
        let (cands, new_preds) = {
            let c = &self.cdf;
            let new_bits = |is_new: bool| symbol_cost(&c.new_mv[st.new_mv_context], (!is_new) as usize);
            let zero_bits = |is_zero: bool| symbol_cost(&c.zero_mv[st.zero_mv_context], (!is_zero) as usize);
            let ref_bits = |nearest: bool| symbol_cost(&c.ref_mv[st.ref_mv_context], (!nearest) as usize);
            let drl_bits = |idx_set: std::ops::Range<usize>, chosen: usize| -> u32 {
                let mut b = 0;
                for idx in idx_set {
                    if st.num_mv_found > idx + 1 {
                        let ctx = st.drl_ctx_stack[idx];
                        b += symbol_cost(&c.drl_mode[ctx], (chosen > idx) as usize);
                        if chosen <= idx {
                            break;
                        }
                    }
                }
                b
            };
            // (mode, ref_mv_idx, mv, bits)
            let mut cands: Vec<(usize, usize, [i32; 2], u32)> = Vec::new();
            cands.push((NEARESTMV, 0, st.ref_stack_mv[0][0], new_bits(false) + zero_bits(false) + ref_bits(true)));
            for idx in 1..4 {
                if idx > 1 && st.num_mv_found <= idx {
                    break;
                }
                cands.push((NEARMV, idx, st.ref_stack_mv[idx][0], new_bits(false) + zero_bits(false) + ref_bits(false) + drl_bits(1..3, idx)));
            }
            cands.push((GLOBALMV, 0, st.global_mvs[0], new_bits(false) + zero_bits(true)));
            let new_preds: Vec<(usize, [i32; 2], u32)> = (0..3)
                .filter(|&i| i == 0 || st.num_mv_found > i)
                .map(|i| {
                    let pos = if st.num_mv_found <= 1 { 0 } else { i };
                    (i, st.ref_stack_mv[pos][0], new_bits(true) + drl_bits(0..2, i))
                })
                .collect();
            (cands, new_preds)
        };
        let mut best: Option<(u64, Plan, u32)> = None;
        for &(mode, idx, mv, bits) in &cands {
            let j = self.inter_satd(enc, mv) + lam(bits);
            if best.as_ref().is_none_or(|(bj, _, _)| j < *bj) {
                best = Some((j, Plan { is_inter: true, mode, ref_mv_idx: idx, mv, ..Default::default() }, bits));
            }
        }
        // NEWMV: integer search seeded by the analysis, the stack and zero, then sub-sample
        // refinement on the actual interpolation
        let (x, y) = (self.b.mi_col * 4, self.b.mi_row * 4);
        let (w, h) = (self.b.bw4 * 4, self.b.bh4 * 4);
        let mut seeds: Vec<[i32; 2]> = vec![[0, 0]];
        for i in 0..st.num_mv_found.min(3) {
            seeds.push(st.ref_stack_mv[i][0]);
        }
        if let Some(n) = enc.node(self.b.mi_row, self.b.mi_col, self.b.mi_size) {
            seeds.push(enc.amv[n]);
        }
        let (imv, _) = self.integer_search(enc, x, y, w, h, &seeds);
        let mv_total = |s: &Self, mv: [i32; 2]| -> (u32, usize) {
            let mut b = (u32::MAX, 0);
            for &(i, p, mode_bits) in &new_preds {
                let bits = mode_bits + s.mv_cost([mv[0] - p[0], mv[1] - p[1]]);
                if bits < b.0 {
                    b = (bits, i);
                }
            }
            b
        };
        let hp = self.fs.fh.allow_high_precision_mv;
        let mut cur = imv;
        let (bits, _) = mv_total(self, cur);
        let mut cur_j = self.inter_satd(enc, cur) + lam(bits);
        let steps: &[i32] = if hp { &[4, 2, 1] } else { &[4, 2] };
        let steps = if enc.fe.effort == 0 { &steps[..steps.len().min(2)] } else { steps };
        for &s in steps {
            let base = cur;
            for (dr, dc) in [(-s, 0), (s, 0), (0, -s), (0, s), (-s, -s), (-s, s), (s, -s), (s, s)] {
                let mv = [base[0] + dr, base[1] + dc];
                let (bits, _) = mv_total(self, mv);
                let j = self.inter_satd(enc, mv) + lam(bits);
                if j < cur_j {
                    cur_j = j;
                    cur = mv;
                }
            }
        }
        let (bits, idx) = mv_total(self, cur);
        if best.as_ref().is_none_or(|(bj, _, _)| cur_j < *bj) {
            best = Some((cur_j, Plan { is_inter: true, mode: NEWMV, ref_mv_idx: idx, mv: cur, ..Default::default() }, bits));
        }
        let (_, plan, bits) = best?;
        self.inter_satd(enc, plan.mv);
        let tx = MAX_TX_SIZE_RECT[self.b.mi_size] as usize;
        let rd = self.rd_residual(enc, tx, enc.fe.round_inter).saturating_add((enc.fe.lambda_sse * bits as f64 / 256.0) as u64);
        Some((rd, plan))
    }
}
