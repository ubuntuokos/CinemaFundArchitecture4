//! Coding tree unit coding: rate-distortion mode decision (CU split, skip / merge / AMVP inter,
//! intra 2Nx2N), syntax writing (7.3.8) and reconstruction with the decoder's own prediction,
//! scaling and inverse transform, so the encoder's picture is exactly the decoder's.

use super::cabac::{BIT, Bins, Estimator, Writer};
use super::fwd;
use crate::intra::Refs;
use crate::mvpred::MvCtx;
use crate::params::{Pps, Sps};
use crate::picture::{RefPic, Sample};
use crate::slice::SliceHeader;
use crate::slicedec::{Blk, F_INTRA, F_SKIP, MvField, PartMode, PicState, motion_compensate, predict_intra, reconstruct};
use crate::spec_tables::*;
use crate::tables::{CTX_IDX_MAP, SCAN_4, scan_order};

/// Source picture at the coded size (edges replicated), 4:2:0.
pub struct Source {
    pub y: Vec<u16>,
    pub u: Vec<u16>,
    pub v: Vec<u16>,
    pub w: usize,
    pub h: usize,
}

impl Source {
    fn plane(&self, c: usize) -> (&[u16], usize) {
        match c {
            0 => (&self.y, self.w),
            1 => (&self.u, self.w / 2),
            _ => (&self.v, self.w / 2),
        }
    }
}

/// Luma plane of a reference picture with a replicated border, for integer motion search.
pub struct RefPlane<S> {
    pub data: Vec<S>,
    pub stride: usize,
    pub pad: usize,
}

impl<S: Sample> RefPlane<S> {
    pub fn new(r: &RefPic, w: usize, h: usize, pad: usize) -> Self {
        let stride = w + 2 * pad;
        let mut data = vec![S::from_u16(0); stride * (h + 2 * pad)];
        r.frame.copy_block::<S>(0, -(pad as i32), -(pad as i32), stride, h + 2 * pad, &mut data, stride);
        RefPlane { data, stride, pad }
    }
    /// The block at integer position (x, y) (may be up to `pad` outside the picture).
    #[inline]
    fn at(&self, x: i32, y: i32) -> &[S] {
        let o = (y + self.pad as i32) as usize * self.stride + (x + self.pad as i32) as usize;
        &self.data[o..]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Skip(u8),
    Merge(u8),
    Amvp { ref_idx: u8, mvd: [i32; 2], mvp: u8 },
    Intra(u8),
}

/// A coded CU with everything its syntax needs.
pub struct Leaf {
    x0: i32,
    y0: i32,
    log2: u32,
    mode: Mode,
    cbf: [bool; 3],
    /// Quantized levels per component (n x n luma, n/2 x n/2 chroma), row-major.
    levels: [Vec<i32>; 3],
}

pub enum Node {
    Leaf(Box<Leaf>),
    /// The sub-CUs inside the picture, in z-order.
    Split(Vec<Node>),
}

/// Saved reconstruction and block state of a region.
struct Saved<S> {
    planes: [Vec<S>; 3],
    blk: Vec<Blk>,
    mvf: Vec<MvField>,
}

/// Per-picture coding state and settings.
pub struct Coder<'a, S> {
    pub pic: &'a mut PicState<S>,
    pub sps: &'a Sps,
    pub pps: &'a Pps,
    pub sh: &'a SliceHeader,
    pub refs: &'a [Vec<RefPic>; 2],
    pub col: Option<&'a RefPic>,
    pub ref_planes: &'a [&'a RefPlane<S>],
    pub src: &'a Source,
    /// QpY and the QP' values of the three components.
    pub qp_y: i32,
    pub qp: [i32; 3],
    pub lambda: f64,
    pub sqrt_lambda: f64,
    /// Integer motion search range (luma samples).
    pub me_range: i32,
    /// Number of luma intra modes kept for full rate-distortion evaluation.
    pub intra_rd_modes: usize,
    /// Skip the intra search when an inter candidate codes without residual.
    pub fast_intra: bool,
    /// Address of the first CTB of the slice being coded.
    pub slice_addr: u32,
    pub refs_buf: Refs,
    pub mc: crate::inter::McScratch,
    pub pred0: Vec<i16>,
    pub pred1: Vec<i16>,
    pub diff: Vec<i32>,
    pub coef: Vec<i32>,
}

const MPM_FALLBACK: [u32; 3] = [0, 1, 26];

impl<'a, S: Sample> Coder<'a, S> {
    fn intra_slice(&self) -> bool {
        self.sh.is_intra()
    }

    fn mv_ctx(&self) -> MvCtx<'_, S> {
        MvCtx { pic: self.pic, sh: self.sh, pps: self.pps, sps: self.sps, refs: self.refs, col: self.col, no_backward_pred: true, slice_addr: self.slice_addr }
    }

    fn bit_depth(&self, c: usize) -> u32 {
        if c == 0 { self.pic.bd_y } else { self.pic.bd_c }
    }

    /// Code CTU (rx, ry): decide, then write its syntax with `w`.
    pub fn ctu(&mut self, rx: usize, ry: usize, w: &mut Writer) {
        let rs = ry * self.pic.wctb + rx;
        if let Some(a) = self.pic.ctb_addr.get_mut(rs) {
            *a = self.slice_addr;
        }
        if let Some(s) = self.pic.ctb_slice.get_mut(rs) {
            *s = self.slice_addr;
        }
        let ctb = 1i32 << self.sps.log2_ctb;
        let (x0, y0) = (rx as i32 * ctb, ry as i32 * ctb);
        let mut est = Estimator::new(w.ctx);
        let (_, node) = self.search(x0, y0, self.sps.log2_ctb, 0, &mut est);
        self.write_tree(w, x0, y0, self.sps.log2_ctb, 0, &node);
    }

    // ---------------------------------------------------------------------------------------
    // Mode decision

    fn search(&mut self, x0: i32, y0: i32, log2: u32, depth: u32, est: &mut Estimator) -> (f64, Node) {
        let n = 1i32 << log2;
        let (w, h) = (self.pic.width as i32, self.pic.height as i32);
        let inside = x0 + n <= w && y0 + n <= h;
        let can_split = log2 > self.sps.log2_min_cb;
        let start = est.clone();
        let start_bits = start.bits;
        let mut best: Option<(f64, Node, Estimator)> = None;
        if inside {
            let mut e = start.clone();
            if can_split {
                self.write_split(&mut e, x0, y0, depth, false);
            }
            let flag = (e.bits - start.bits) as f64 / BIT as f64 * self.lambda;
            let (cost, leaf, e_out) = self.best_leaf(x0, y0, log2, depth, &e);
            let cost = cost + flag;
            // early termination: a skipped CU, or an inter CU without residual, is not split further
            let skip = matches!(leaf.mode, Mode::Skip(_)) || (!matches!(leaf.mode, Mode::Intra(_)) && !leaf.cbf.iter().any(|&c| c));
            best = Some((cost, Node::Leaf(Box::new(leaf)), e_out));
            if skip || !can_split {
                let Some((c, node, e)) = best else { return (f64::MAX, Node::Split(Vec::new())) };
                *est = e;
                return (c, node);
            }
        }
        // split
        let saved = if inside { Some(self.save(x0, y0, n)) } else { None };
        let mut e = start;
        if inside {
            self.write_split(&mut e, x0, y0, depth, true);
        }
        let half = n >> 1;
        let mut children = Vec::with_capacity(4);
        // both alternatives count only the bits coded from `start` on (the split flags included)
        let mut cost = (e.bits - start_bits) as f64 / BIT as f64 * self.lambda;
        for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
            let (xs, ys) = (x0 + dx, y0 + dy);
            if xs >= w || ys >= h {
                continue;
            }
            let (c, node) = self.search(xs, ys, log2 - 1, depth + 1, &mut e);
            cost += c;
            children.push(node);
            if let Some((bc, _, _)) = &best
                && cost >= *bc
            {
                break;
            }
        }
        match best {
            Some((bc, node, be)) if bc <= cost => {
                if let Some(s) = saved {
                    self.restore(x0, y0, n, &s);
                }
                *est = be;
                (bc, node)
            }
            _ => {
                *est = e;
                (cost, Node::Split(children))
            }
        }
    }

    /// The best single CU at (x0, y0): returns its cost, data and the entropy state after it.
    fn best_leaf(&mut self, x0: i32, y0: i32, log2: u32, depth: u32, est: &Estimator) -> (f64, Leaf, Estimator) {
        let n = 1usize << log2;
        let mut best: Option<(f64, Leaf, Estimator, Saved<S>)> = None;
        let consider = |this: &mut Self, mode: Mode, mvf: MvField, best: &mut Option<(f64, Leaf, Estimator, Saved<S>)>| {
            let (cost, leaf, e) = this.eval(x0, y0, log2, depth, mode, mvf, est);
            if best.as_ref().is_none_or(|b| cost < b.0) {
                let saved = this.save(x0, y0, n as i32);
                *best = Some((cost, leaf, e, saved));
            }
        };
        if !self.intra_slice() {
            // merge candidates, ranked by luma SATD
            let mut cands: Vec<(u32, u8, MvField)> = Vec::new();
            let max = self.sh.max_num_merge_cand.max(1);
            for idx in 0..max {
                let f = self.mv_ctx().derive_merge(x0, y0, n as i32, x0, y0, n as i32, n as i32, 0, PartMode::P2Nx2N, idx as usize);
                if cands.iter().any(|c| c.2 == f) || !self.mvf_valid(&f) {
                    continue;
                }
                let cost = self.inter_satd(x0, y0, n, &f) + (self.sqrt_lambda * (idx + 1) as f64) as u32;
                cands.push((cost, idx as u8, f));
            }
            cands.sort_by_key(|c| c.0);
            for (k, &(_, idx, f)) in cands.iter().take(2).enumerate() {
                consider(self, Mode::Skip(idx), f, &mut best);
                if k == 0 {
                    consider(self, Mode::Merge(idx), f, &mut best);
                }
            }
            // motion search per reference
            for r in 0..self.refs[0].len().min(self.ref_planes.len()) {
                if let Some((mvf, mode)) = self.motion_search(x0, y0, n, r) {
                    consider(self, mode, mvf, &mut best);
                }
            }
        }
        // intra; with fast decisions not when an inter candidate already needs no residual
        let inter_clean = best.as_ref().is_some_and(|b| !b.1.cbf.iter().any(|&c| c));
        if !(self.fast_intra && inter_clean) {
            let keep = if self.intra_slice() { self.intra_rd_modes } else { self.intra_rd_modes.min(2) };
            for m in self.intra_candidates(x0, y0, log2, keep) {
                consider(self, Mode::Intra(m), MvField::default(), &mut best);
            }
        }
        let Some((cost, leaf, e, saved)) = best else {
            // unreachable in practice: intra always yields a candidate
            let leaf = Leaf { x0, y0, log2, mode: Mode::Intra(1), cbf: [false; 3], levels: [Vec::new(), Vec::new(), Vec::new()] };
            return (f64::MAX, leaf, est.clone());
        };
        self.restore(x0, y0, n as i32, &saved);
        (cost, leaf, e)
    }

    fn mvf_valid(&self, f: &MvField) -> bool {
        (0..2).all(|l| !f.pred(l) || (f.ref_idx[l] as usize) < self.refs[l].len()) && !f.pred(1)
    }

    /// Predict, code the residual and reconstruct one CU candidate; commits its block state.
    #[allow(clippy::too_many_arguments)]
    fn eval(&mut self, x0: i32, y0: i32, log2: u32, depth: u32, mode: Mode, mvf: MvField, est: &Estimator) -> (f64, Leaf, Estimator) {
        let n = 1usize << log2;
        let skip = matches!(mode, Mode::Skip(_));
        self.commit_state(x0, y0, n as i32, depth, mode, &mvf);
        let mut leaf = Leaf { x0, y0, log2, mode, cbf: [false; 3], levels: [Vec::new(), Vec::new(), Vec::new()] };
        if let Mode::Intra(m) = mode {
            predict_intra(self.pic, &mut self.refs_buf, true, false, self.slice_addr, x0, y0, log2, 0, m as u32);
            self.code_residual(x0, y0, log2, 0, true, &mut leaf);
            for c in 1..3 {
                predict_intra(self.pic, &mut self.refs_buf, true, false, self.slice_addr, x0 >> 1, y0 >> 1, log2 - 1, c, m as u32);
                self.code_residual(x0 >> 1, y0 >> 1, log2 - 1, c, true, &mut leaf);
            }
        } else {
            motion_compensate(self.pic, self.refs, None, false, &mut self.pred0, &mut self.pred1, &mut self.mc, x0, y0, n, n, &mvf);
            if !skip {
                self.code_residual(x0, y0, log2, 0, false, &mut leaf);
                for c in 1..3 {
                    self.code_residual(x0 >> 1, y0 >> 1, log2 - 1, c, false, &mut leaf);
                }
            }
        }
        // a merge CU without residual is a skipped CU; a merge CU whose chroma has no residual
        // must have luma residual (cbf_luma is then inferred)
        if let Mode::Merge(i) = leaf.mode
            && !leaf.cbf.iter().any(|&b| b)
        {
            leaf.mode = Mode::Skip(i);
            self.commit_state(x0, y0, n as i32, depth, leaf.mode, &mvf);
        }
        let dist = self.distortion(x0, y0, n);
        let mut e = est.clone();
        let b0 = e.bits;
        self.write_cu(&mut e, &leaf);
        let bits = (e.bits - b0) as f64 / BIT as f64;
        (dist as f64 + self.lambda * bits, leaf, e)
    }

    /// Block state the decoder would have after this CU (depth, flags, modes, motion).
    fn commit_state(&mut self, x0: i32, y0: i32, n: i32, depth: u32, mode: Mode, mvf: &MvField) {
        let (intra, skip) = (matches!(mode, Mode::Intra(_)), matches!(mode, Mode::Skip(_)));
        let ipm = if let Mode::Intra(m) = mode { m } else { 1 };
        let qp = self.qp_y as i8;
        let w4 = self.pic.w4;
        let (bx0, by0) = ((x0 >> 2) as usize, (y0 >> 2) as usize);
        let nb = (n >> 2) as usize;
        let f = if intra { MvField::default() } else { *mvf };
        for by in by0..by0 + nb {
            for bx in bx0..bx0 + nb {
                let i = by * w4 + bx;
                if let Some(b) = self.pic.blk.get_mut(i) {
                    b.depth = depth as u8;
                    b.flags = if intra { F_INTRA } else { 0 } | if skip { F_SKIP } else { 0 };
                    b.qp = qp;
                    b.ipm = ipm;
                    b.cpm = ipm;
                }
                if let Some(m) = self.pic.mvf.get_mut(i) {
                    *m = f;
                }
            }
        }
    }

    /// Residual of component `c` for the n x n block at component position (x, y) against the
    /// prediction in the picture: transform, quantize and reconstruct.
    fn code_residual(&mut self, x: i32, y: i32, log2: u32, c: usize, intra: bool, leaf: &mut Leaf) {
        let n = 1usize << log2;
        let bd = self.bit_depth(c);
        let stride = if c == 0 { self.pic.width } else { self.pic.cwidth };
        let (src, ss) = self.src.plane(c);
        let (xu, yu) = (x as usize, y as usize);
        for j in 0..n {
            let p = &self.pic.planes[c][(yu + j) * stride + xu..][..n];
            let s = &src[(yu + j) * ss + xu..][..n];
            for i in 0..n {
                self.diff[j * n + i] = s[i] as i32 - p[i].to_i32();
            }
        }
        let dst = intra && c == 0 && n == 4;
        fwd::forward(&self.diff[..n * n], &mut self.coef[..n * n], n, dst, bd);
        let nz = fwd::quantize(&mut self.coef[..n * n], n, self.qp[c], bd, intra);
        if nz == 0 {
            leaf.cbf[c] = false;
            leaf.levels[c].clear();
            return;
        }
        leaf.cbf[c] = true;
        leaf.levels[c] = self.coef[..n * n].to_vec();
        let (mut mx, mut my) = (0, 0);
        for j in 0..n {
            for i in 0..n {
                if self.coef[j * n + i] != 0 {
                    mx = mx.max(i);
                    my = my.max(j);
                }
            }
        }
        reconstruct(self.pic, &mut self.coef, intra, false, x, y, log2, c, self.qp[c], false, mx, my);
    }

    fn distortion(&self, x0: i32, y0: i32, n: usize) -> u64 {
        let mut d = 0;
        for c in 0..3 {
            let (x, y, m) = if c == 0 { (x0 as usize, y0 as usize, n) } else { (x0 as usize >> 1, y0 as usize >> 1, n >> 1) };
            let stride = if c == 0 { self.pic.width } else { self.pic.cwidth };
            let (src, ss) = self.src.plane(c);
            d += fwd::sse(&self.pic.planes[c][y * stride + x..], stride, &src[y * ss + x..], ss, m, m);
        }
        d
    }

    fn save(&self, x0: i32, y0: i32, n: i32) -> Saved<S> {
        let (pw, ph) = (self.pic.width as i32, self.pic.height as i32);
        let (w, h) = ((pw - x0).min(n).max(0) as usize, (ph - y0).min(n).max(0) as usize);
        let mut planes: [Vec<S>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (c, out) in planes.iter_mut().enumerate() {
            let (x, y, bw, bh, stride) =
                if c == 0 { (x0 as usize, y0 as usize, w, h, self.pic.width) } else { (x0 as usize >> 1, y0 as usize >> 1, w >> 1, h >> 1, self.pic.cwidth) };
            out.reserve(bw * bh);
            for j in 0..bh {
                out.extend_from_slice(&self.pic.planes[c][(y + j) * stride + x..][..bw]);
            }
        }
        let (bx, by, bw, bh) = ((x0 >> 2) as usize, (y0 >> 2) as usize, w.div_ceil(4), h.div_ceil(4));
        let mut blk = Vec::with_capacity(bw * bh);
        let mut mvf = Vec::with_capacity(bw * bh);
        for j in 0..bh {
            let o = (by + j) * self.pic.w4 + bx;
            blk.extend_from_slice(&self.pic.blk[o..o + bw]);
            mvf.extend_from_slice(&self.pic.mvf[o..o + bw]);
        }
        Saved { planes, blk, mvf }
    }

    fn restore(&mut self, x0: i32, y0: i32, n: i32, s: &Saved<S>) {
        let (pw, ph) = (self.pic.width as i32, self.pic.height as i32);
        let (w, h) = ((pw - x0).min(n).max(0) as usize, (ph - y0).min(n).max(0) as usize);
        for c in 0..3 {
            let (x, y, bw, bh, stride) =
                if c == 0 { (x0 as usize, y0 as usize, w, h, self.pic.width) } else { (x0 as usize >> 1, y0 as usize >> 1, w >> 1, h >> 1, self.pic.cwidth) };
            for j in 0..bh {
                self.pic.planes[c][(y + j) * stride + x..][..bw].copy_from_slice(&s.planes[c][j * bw..(j + 1) * bw]);
            }
        }
        let (bx, by, bw, bh) = ((x0 >> 2) as usize, (y0 >> 2) as usize, w.div_ceil(4), h.div_ceil(4));
        for j in 0..bh {
            let o = (by + j) * self.pic.w4 + bx;
            self.pic.blk[o..o + bw].copy_from_slice(&s.blk[j * bw..(j + 1) * bw]);
            self.pic.mvf[o..o + bw].copy_from_slice(&s.mvf[j * bw..(j + 1) * bw]);
        }
    }

    // ---------------------------------------------------------------------------------------
    // Intra analysis

    /// Luma modes worth a full evaluation: the `keep` best by SATD plus mode bits.
    fn intra_candidates(&mut self, x0: i32, y0: i32, log2: u32, keep: usize) -> Vec<u8> {
        let n = 1usize << log2;
        let mpm = self.mpm(x0, y0);
        let mut costs: Vec<(u32, u8)> = Vec::with_capacity(35);
        let mut tried = [false; 35];
        let mut try_mode = |this: &mut Self, m: u32, costs: &mut Vec<(u32, u8)>| {
            let Some(t) = tried.get_mut(m as usize) else { return };
            if *t {
                return;
            }
            *t = true;
            predict_intra(this.pic, &mut this.refs_buf, true, false, this.slice_addr, x0, y0, log2, 0, m);
            let satd = this.luma_satd_vs_src(x0, y0, n);
            let bits = if mpm[0] == m {
                2.0
            } else if mpm.contains(&m) {
                3.0
            } else {
                6.0
            };
            costs.push((satd + (this.sqrt_lambda * bits) as u32, m as u8));
        };
        // coarse to fine: planar, DC, every fourth angle and the most probable modes, then the
        // neighbours of the two best angles at distance 2 and 1
        for m in [0u32, 1].into_iter().chain((2..35).step_by(4)).chain(mpm) {
            try_mode(self, m, &mut costs);
        }
        for step in [2u32, 1] {
            costs.sort_by_key(|c| c.0);
            let best: Vec<u32> = costs.iter().filter(|c| c.1 >= 2).take(2).map(|c| c.1 as u32).collect();
            for b in best {
                for m in [b.saturating_sub(step), b + step] {
                    if (2..35).contains(&m) {
                        try_mode(self, m, &mut costs);
                    }
                }
            }
        }
        costs.sort_by_key(|c| c.0);
        costs.iter().take(keep.max(1)).map(|c| c.1).collect()
    }

    fn luma_satd_vs_src(&mut self, x0: i32, y0: i32, n: usize) -> u32 {
        let stride = self.pic.width;
        let ss = self.src.w;
        let (xu, yu) = (x0 as usize, y0 as usize);
        for j in 0..n {
            let p = &self.pic.planes[0][(yu + j) * stride + xu..][..n];
            let s = &self.src.y[(yu + j) * ss + xu..][..n];
            for i in 0..n {
                self.diff[j * n + i] = s[i] as i32 - p[i].to_i32();
            }
        }
        fwd::satd(&self.diff[..n * n], n, n)
    }

    /// Most probable modes (8.4.2).
    fn mpm(&self, xp: i32, yp: i32) -> [u32; 3] {
        let cand_a = if self.pic.zavail(xp, yp, xp - 1, yp, self.slice_addr) {
            let b = self.pic.blk_at(xp - 1, yp);
            if b.flags & F_INTRA != 0 { b.ipm as u32 } else { 1 }
        } else {
            1
        };
        let cand_b = if self.pic.zavail(xp, yp, xp, yp - 1, self.slice_addr) && yp > ((yp >> self.sps.log2_ctb) << self.sps.log2_ctb) {
            let b = self.pic.blk_at(xp, yp - 1);
            if b.flags & F_INTRA != 0 { b.ipm as u32 } else { 1 }
        } else {
            1
        };
        if cand_a == cand_b {
            if cand_a < 2 { MPM_FALLBACK } else { [cand_a, 2 + ((cand_a + 29) % 32), 2 + ((cand_a - 2 + 1) % 32)] }
        } else {
            let c = if cand_a != 0 && cand_b != 0 {
                0
            } else if cand_a != 1 && cand_b != 1 {
                1
            } else {
                26
            };
            [cand_a, cand_b, c]
        }
    }

    // ---------------------------------------------------------------------------------------
    // Inter analysis

    /// Luma SATD of the uni-prediction `f` against the source.
    fn inter_satd(&mut self, x0: i32, y0: i32, n: usize, f: &MvField) -> u32 {
        let Some(r) = self.refs[0].get(f.ref_idx[0].max(0) as usize) else { return u32::MAX };
        self.subpel_satd(r.frame.clone(), x0, y0, n, f.mv[0])
    }

    fn subpel_satd(&mut self, frame: crate::picture::FrameRef, x0: i32, y0: i32, n: usize, mv: [i16; 2]) -> u32 {
        crate::inter::mc_luma::<S>(&frame, x0, y0, mv, n, n, &mut self.pred0, &mut self.mc);
        let bd = self.pic.bd_y;
        let shift = 14 - bd;
        let off = if shift > 0 { 1i32 << (shift - 1) } else { 0 };
        let max = (1i32 << bd) - 1;
        let ss = self.src.w;
        let (xu, yu) = (x0 as usize, y0 as usize);
        for j in 0..n {
            let s = &self.src.y[(yu + j) * ss + xu..][..n];
            for i in 0..n {
                let p = ((self.pred0[j * n + i] as i32 + off) >> shift).clamp(0, max);
                self.diff[j * n + i] = s[i] as i32 - p;
            }
        }
        fwd::satd(&self.diff[..n * n], n, n)
    }

    /// Approximate bits of a motion vector difference component.
    fn mvd_bits(d: i32) -> f64 {
        let a = d.unsigned_abs();
        if a == 0 { 1.0 } else { 3.0 + 2.0 * (32 - a.leading_zeros()) as f64 }
    }

    /// Integer then fractional motion search of the n x n block at (x0, y0) in reference `r` of list 0.
    fn motion_search(&mut self, x0: i32, y0: i32, n: usize, r: usize) -> Option<(MvField, Mode)> {
        let ni = n as i32;
        let mvps = [0usize, 1].map(|k| self.mv_ctx().derive_amvp(x0, y0, ni, x0, y0, ni, ni, 0, 0, r, k));
        let planes: &'a [&'a RefPlane<S>] = self.ref_planes;
        let plane: &'a RefPlane<S> = planes.get(r)?;
        let frame = self.refs[0].get(r)?.frame.clone();
        let pad = plane.pad as i32;
        let (pw, ph) = (self.pic.width as i32, self.pic.height as i32);
        // integer positions keep the block inside the padded plane
        let (lo_x, hi_x) = (-x0 - pad + 4, pw - x0 + pad - ni - 4);
        let (lo_y, hi_y) = (-y0 - pad + 4, ph - y0 + pad - ni - 4);
        let lam = self.sqrt_lambda;
        let src: &'a Source = self.src;
        let ss = src.w;
        let mvp_cost = |mv: [i32; 2]| -> f64 {
            mvps.iter().map(|p| Self::mvd_bits(mv[0] - p[0] as i32) + Self::mvd_bits(mv[1] - p[1] as i32)).fold(f64::MAX, f64::min) * lam
        };
        let sad_at = |ix: i32, iy: i32| -> f64 {
            if ix < lo_x || ix > hi_x || iy < lo_y || iy > hi_y {
                return f64::MAX;
            }
            let a = plane.at(x0 + ix, y0 + iy);
            let s = &src.y[y0 as usize * ss + x0 as usize..];
            fwd::sad(a, plane.stride, s, ss, n, n) as f64 + mvp_cost([ix << 2, iy << 2])
        };
        // start: best of the predictors and zero
        let mut starts: Vec<(i32, i32)> = mvps.iter().map(|p| ((p[0] as i32 + 2) >> 2, (p[1] as i32 + 2) >> 2)).collect();
        starts.push((0, 0));
        let mut best = (0, 0);
        let mut best_c = f64::MAX;
        for &(sx, sy) in &starts {
            let c = sad_at(sx, sy);
            if c < best_c {
                best_c = c;
                best = (sx, sy);
            }
        }
        let center = best;
        // hexagon search, then a square refinement
        const HEX: [(i32, i32); 6] = [(-2, 0), (-1, -2), (1, -2), (2, 0), (1, 2), (-1, 2)];
        for _ in 0..(self.me_range / 2).max(1) {
            let c0 = best;
            for (dx, dy) in HEX {
                let p = (c0.0 + dx, c0.1 + dy);
                if (p.0 - center.0).abs() > self.me_range || (p.1 - center.1).abs() > self.me_range {
                    continue;
                }
                let c = sad_at(p.0, p.1);
                if c < best_c {
                    best_c = c;
                    best = p;
                }
            }
            if best == c0 {
                break;
            }
        }
        let c0 = best;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = sad_at(c0.0 + dx, c0.1 + dy);
                if c < best_c {
                    best_c = c;
                    best = (c0.0 + dx, c0.1 + dy);
                }
            }
        }
        // half then quarter sample refinement on SATD
        let mut mv = [best.0 << 2, best.1 << 2];
        let mut cost = self.subpel_satd(frame.clone(), x0, y0, n, [mv[0] as i16, mv[1] as i16]) as f64 + mvp_cost(mv);
        for step in [2, 1] {
            let c0 = mv;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let cand = [c0[0] + dx * step, c0[1] + dy * step];
                    if (cand[0] >> 2) < lo_x || (cand[0] >> 2) > hi_x || (cand[1] >> 2) < lo_y || (cand[1] >> 2) > hi_y {
                        continue;
                    }
                    let c = self.subpel_satd(frame.clone(), x0, y0, n, [cand[0] as i16, cand[1] as i16]) as f64 + mvp_cost(cand);
                    if c < cost {
                        cost = c;
                        mv = cand;
                    }
                }
            }
        }
        // predictor with the cheaper difference
        let bits = |p: &[i16; 2]| Self::mvd_bits(mv[0] - p[0] as i32) + Self::mvd_bits(mv[1] - p[1] as i32);
        let k = if bits(&mvps[1]) < bits(&mvps[0]) { 1 } else { 0 };
        let mvd = [mv[0] - mvps[k][0] as i32, mv[1] - mvps[k][1] as i32];
        let f = MvField { mv: [[mv[0] as i16, mv[1] as i16], [0, 0]], ref_idx: [r as i8, -1] };
        Some((f, Mode::Amvp { ref_idx: r as u8, mvd, mvp: k as u8 }))
    }

    // ---------------------------------------------------------------------------------------
    // Syntax

    fn write_split<B: Bins>(&self, b: &mut B, x0: i32, y0: i32, depth: u32, split: bool) {
        let mut inc = 0;
        if self.pic.zavail(x0, y0, x0 - 1, y0, self.slice_addr) && self.pic.blk_at(x0 - 1, y0).depth as u32 > depth {
            inc += 1;
        }
        if self.pic.zavail(x0, y0, x0, y0 - 1, self.slice_addr) && self.pic.blk_at(x0, y0 - 1).depth as u32 > depth {
            inc += 1;
        }
        b.decision(SPLIT_CU + inc, split as u32);
    }

    fn write_tree<B: Bins>(&self, b: &mut B, x0: i32, y0: i32, log2: u32, depth: u32, node: &Node) {
        let n = 1i32 << log2;
        let (w, h) = (self.pic.width as i32, self.pic.height as i32);
        let coded = x0 + n <= w && y0 + n <= h && log2 > self.sps.log2_min_cb;
        match node {
            Node::Leaf(leaf) => {
                if coded {
                    self.write_split(b, x0, y0, depth, false);
                }
                self.write_cu(b, leaf);
            }
            Node::Split(children) => {
                if coded {
                    self.write_split(b, x0, y0, depth, true);
                }
                let half = n >> 1;
                let mut it = children.iter();
                for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                    let (xs, ys) = (x0 + dx, y0 + dy);
                    if xs >= w || ys >= h {
                        continue;
                    }
                    if let Some(c) = it.next() {
                        self.write_tree(b, xs, ys, log2 - 1, depth + 1, c);
                    }
                }
            }
        }
    }

    fn write_merge_idx<B: Bins>(&self, b: &mut B, idx: u8) {
        let max = self.sh.max_num_merge_cand;
        if max <= 1 {
            return;
        }
        let idx = idx as u32;
        b.decision(MERGE_IDX, (idx > 0) as u32);
        if idx > 0 {
            for i in 1..max - 1 {
                b.bypass((idx > i) as u32);
                if idx == i {
                    break;
                }
            }
        }
    }

    fn write_cu<B: Bins>(&self, b: &mut B, leaf: &Leaf) {
        let (x0, y0) = (leaf.x0, leaf.y0);
        if !self.intra_slice() {
            let mut inc = 0;
            if self.pic.zavail(x0, y0, x0 - 1, y0, self.slice_addr) && self.pic.blk_at(x0 - 1, y0).flags & F_SKIP != 0 {
                inc += 1;
            }
            if self.pic.zavail(x0, y0, x0, y0 - 1, self.slice_addr) && self.pic.blk_at(x0, y0 - 1).flags & F_SKIP != 0 {
                inc += 1;
            }
            b.decision(CU_SKIP + inc, matches!(leaf.mode, Mode::Skip(_)) as u32);
        }
        if let Mode::Skip(i) = leaf.mode {
            self.write_merge_idx(b, i);
            return;
        }
        let intra = matches!(leaf.mode, Mode::Intra(_));
        if !self.intra_slice() {
            b.decision(PRED_MODE, intra as u32);
        }
        if !intra || leaf.log2 == self.sps.log2_min_cb {
            b.decision(PART_MODE, 1); // PART_2Nx2N
        }
        match leaf.mode {
            Mode::Intra(m) => {
                let m = m as u32;
                let mut list = self.mpm(x0, y0);
                if let Some(idx) = list.iter().position(|&c| c == m) {
                    b.decision(PREV_INTRA_LUMA, 1);
                    match idx {
                        0 => b.bypass(0),
                        1 => b.bypass_bits(0b10, 2),
                        _ => b.bypass_bits(0b11, 2),
                    }
                } else {
                    b.decision(PREV_INTRA_LUMA, 0);
                    list.sort_unstable();
                    let below = list.iter().filter(|&&c| c < m).count() as u32;
                    b.bypass_bits(m - below, 5);
                }
                b.decision(INTRA_CHROMA, 0); // DM: the luma mode
            }
            Mode::Merge(i) => {
                b.decision(MERGE_FLAG, 1);
                self.write_merge_idx(b, i);
            }
            Mode::Amvp { ref_idx, mvd, mvp } => {
                b.decision(MERGE_FLAG, 0);
                let nref = self.sh.num_ref_idx[0];
                if nref > 1 {
                    let ri = ref_idx as u32;
                    for i in 0..nref - 1 {
                        let bin = (ri > i) as u32;
                        if i < 2 {
                            b.decision(REF_IDX + i as usize, bin);
                        } else {
                            b.bypass(bin);
                        }
                        if bin == 0 {
                            break;
                        }
                    }
                }
                Self::write_mvd(b, mvd);
                b.decision(MVP_FLAG, mvp as u32);
            }
            Mode::Skip(_) => {}
        }
        let any = leaf.cbf.iter().any(|&c| c);
        if let Mode::Amvp { .. } = leaf.mode {
            b.decision(RQT_ROOT_CBF, any as u32);
            if !any {
                return;
            }
        }
        // transform_tree at depth 0 without a split (max_transform_hierarchy_depth = 0)
        b.decision(CBF_CHROMA, leaf.cbf[1] as u32);
        b.decision(CBF_CHROMA, leaf.cbf[2] as u32);
        if intra || leaf.cbf[1] || leaf.cbf[2] {
            b.decision(CBF_LUMA + 1, leaf.cbf[0] as u32);
        }
        let m = if let Mode::Intra(m) = leaf.mode { Some(m as u32) } else { None };
        for c in 0..3 {
            if leaf.cbf[c] {
                let log2 = if c == 0 { leaf.log2 } else { leaf.log2 - 1 };
                self.write_residual(b, &leaf.levels[c], log2, c, m);
            }
        }
    }

    fn write_mvd<B: Bins>(b: &mut B, mvd: [i32; 2]) {
        let a = [mvd[0].unsigned_abs(), mvd[1].unsigned_abs()];
        b.decision(ABS_MVD_GT0, (a[0] > 0) as u32);
        b.decision(ABS_MVD_GT0, (a[1] > 0) as u32);
        for &v in &a {
            if v > 0 {
                b.decision(ABS_MVD_GT1, (v > 1) as u32);
            }
        }
        for k in 0..2 {
            if a[k] > 0 {
                if a[k] > 1 {
                    Self::write_eg(b, a[k] - 2, 1);
                }
                b.bypass((mvd[k] < 0) as u32);
            }
        }
    }

    /// k-th order Exp-Golomb in bypass bins (9.3.3.3).
    fn write_eg<B: Bins>(b: &mut B, mut v: u32, mut k: u32) {
        while k < 31 && v >= (1 << k) {
            b.bypass(1);
            v -= 1 << k;
            k += 1;
        }
        b.bypass(0);
        b.bypass_bits(v, k);
    }

    /// residual_coding() (7.3.8.11) of the levels of an n x n block; `intra_mode` selects the scan.
    fn write_residual<B: Bins>(&self, b: &mut B, levels: &[i32], log2: u32, c: usize, intra_mode: Option<u32>) {
        let n = 1usize << log2;
        let scan_idx = match intra_mode {
            Some(m) if log2 == 2 || (log2 == 3 && c == 0) => {
                if (6..=14).contains(&m) {
                    2
                } else if (22..=30).contains(&m) {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        };
        let log2sb = log2 - 2;
        let sb_scan = scan_order(log2sb, scan_idx);
        let pos_scan = &SCAN_4[scan_idx];
        let sb_w = 1usize << log2sb;
        let at = |xs: usize, ys: usize, p: usize| -> i32 {
            let (x, y) = (xs * 4 + pos_scan[p].0 as usize, ys * 4 + pos_scan[p].1 as usize);
            levels.get(y * n + x).copied().unwrap_or(0)
        };
        // last significant coefficient in scan order
        let mut last: Option<(usize, usize)> = None;
        'find: for i in (0..sb_scan.len()).rev() {
            let (xs, ys) = (sb_scan[i].0 as usize, sb_scan[i].1 as usize);
            for p in (0..16).rev() {
                if at(xs, ys, p) != 0 {
                    last = Some((i, p));
                    break 'find;
                }
            }
        }
        let Some((last_sb, last_pos)) = last else { return };
        let (xs, ys) = (sb_scan[last_sb].0 as u32, sb_scan[last_sb].1 as u32);
        let mut lx = xs * 4 + pos_scan[last_pos].0 as u32;
        let mut ly = ys * 4 + pos_scan[last_pos].1 as u32;
        if scan_idx == 2 {
            std::mem::swap(&mut lx, &mut ly);
        }
        let cmax = (log2 << 1) - 1;
        let (off, shift) = if c == 0 { (3 * (log2 - 2) + ((log2 - 1) >> 2), (log2 + 1) >> 2) } else { (15, log2 - 2) };
        let prefix_of = |v: u32| -> u32 {
            if v < 4 {
                v
            } else {
                // smallest prefix p with (1 << nb) * (2 + (p & 1)) <= v, nb = (p >> 1) - 1
                let mut p = 4;
                while p < cmax {
                    let nb = ((p + 1) >> 1) - 1;
                    if ((1u32 << nb) * (2 + ((p + 1) & 1))) > v {
                        break;
                    }
                    p += 1;
                }
                p
            }
        };
        let (px, py) = (prefix_of(lx), prefix_of(ly));
        for (p, base) in [(px, LAST_X_PREFIX), (py, LAST_Y_PREFIX)] {
            for i in 0..p {
                b.decision(base + (off + (i >> shift)) as usize, 1);
            }
            if p < cmax {
                b.decision(base + (off + (p >> shift)) as usize, 0);
            }
        }
        for (p, v) in [(px, lx), (py, ly)] {
            if p > 3 {
                let nb = (p >> 1) - 1;
                let min = (1u32 << nb) * (2 + (p & 1));
                b.bypass_bits(v - min, nb);
            }
        }
        let mut csbf = [0u8; 64];
        let mut prev_g1: Option<u32> = None;
        let c_off_sig = if c == 0 { 0 } else { 27 };
        for i in (0..=last_sb).rev() {
            let (xs, ys) = (sb_scan[i].0 as usize, sb_scan[i].1 as usize);
            let right = if xs + 1 < sb_w { csbf[ys * 8 + xs + 1] } else { 0 };
            let below = if ys + 1 < sb_w { csbf[(ys + 1) * 8 + xs] } else { 0 };
            let any = (0..16).any(|p| at(xs, ys, p) != 0);
            let mut infer_dc = false;
            if i < last_sb && i > 0 {
                let inc = (right | below).min(1) as usize + if c > 0 { 2 } else { 0 };
                b.decision(CODED_SUB_BLOCK + inc, any as u32);
                csbf[ys * 8 + xs] = any as u8;
                infer_dc = true;
            } else {
                csbf[ys * 8 + xs] = 1;
            }
            if csbf[ys * 8 + xs] == 0 {
                continue;
            }
            // significance map, scan positions descending
            let mut sig = [0u8; 16];
            let mut nsig = 0;
            let start: i32 = if i == last_sb {
                sig[0] = last_pos as u8;
                nsig = 1;
                last_pos as i32 - 1
            } else {
                15
            };
            let prev_csbf = right + (below << 1);
            let mut np = start;
            while np >= 0 {
                let p = np as usize;
                let (xp, yp) = (pos_scan[p].0 as usize, pos_scan[p].1 as usize);
                let (xc, yc) = (xs * 4 + xp, ys * 4 + yp);
                let v = at(xs, ys, p);
                if np > 0 || !infer_dc {
                    let sig_ctx = if log2 == 2 {
                        CTX_IDX_MAP[(yc << 2) + xc] as usize
                    } else if xc + yc == 0 {
                        0
                    } else {
                        let mut s = match prev_csbf {
                            0 => {
                                if xp + yp == 0 {
                                    2
                                } else if xp + yp < 3 {
                                    1
                                } else {
                                    0
                                }
                            }
                            1 => {
                                if yp == 0 {
                                    2
                                } else if yp == 1 {
                                    1
                                } else {
                                    0
                                }
                            }
                            2 => {
                                if xp == 0 {
                                    2
                                } else if xp == 1 {
                                    1
                                } else {
                                    0
                                }
                            }
                            _ => 2,
                        };
                        if c == 0 {
                            if xs + ys > 0 {
                                s += 3;
                            }
                            s += if log2 == 3 { if scan_idx == 0 { 9 } else { 15 } } else { 21 };
                        } else {
                            s += if log2 == 3 { 9 } else { 12 };
                        }
                        s
                    };
                    b.decision(SIG_COEFF + c_off_sig + sig_ctx, (v != 0) as u32);
                    if v != 0 {
                        sig[nsig] = p as u8;
                        nsig += 1;
                        infer_dc = false;
                    }
                } else {
                    sig[nsig] = 0;
                    nsig += 1;
                }
                np -= 1;
            }
            if nsig == 0 {
                continue;
            }
            let lvl = |k: usize| -> i32 { at(xs, ys, sig[k] as usize) };
            // greater1 / greater2 flags
            let mut ctx_set = if i == 0 || c > 0 { 0 } else { 2 };
            if prev_g1 == Some(0) {
                ctx_set += 1;
            }
            let mut g1ctx = 1u32;
            let mut gt1 = [0u8; 16];
            let mut first_g1: Option<usize> = None;
            let c_off_g1 = if c > 0 { 16 } else { 0 };
            for k in 0..nsig.min(8) {
                let f = (lvl(k).abs() > 1) as u32;
                b.decision(GT1 + ctx_set * 4 + g1ctx.min(3) as usize + c_off_g1, f);
                gt1[k] = f as u8;
                if f == 1 {
                    g1ctx = 0;
                    if first_g1.is_none() {
                        first_g1 = Some(k);
                    }
                } else if g1ctx > 0 {
                    g1ctx += 1;
                }
            }
            prev_g1 = Some(g1ctx);
            let mut gt2 = 0;
            if let Some(k) = first_g1 {
                gt2 = (lvl(k).abs() > 2) as i32;
                b.decision(GT2 + ctx_set + if c > 0 { 4 } else { 0 }, gt2 as u32);
            }
            // signs (no sign data hiding)
            let mut signs = 0u32;
            for k in 0..nsig {
                signs = (signs << 1) | (lvl(k) < 0) as u32;
            }
            b.bypass_bits(signs, nsig as u32);
            // remaining levels
            let mut rice = 0u32;
            for k in 0..nsig {
                let mut base = 1 + gt1[k] as i32;
                if Some(k) == first_g1 {
                    base += gt2;
                }
                let thresh = if k < 8 { if Some(k) == first_g1 { 3 } else { 2 } } else { 1 };
                if base == thresh {
                    let level = lvl(k).abs();
                    let rem = (level - base) as u32;
                    Self::write_remaining(b, rem, rice);
                    if level > 3 * (1 << rice) {
                        rice = (rice + 1).min(4);
                    }
                }
            }
        }
    }

    /// coeff_abs_level_remaining binarization (9.3.3.11).
    fn write_remaining<B: Bins>(b: &mut B, v: u32, rice: u32) {
        if v < (4 << rice) {
            let p = v >> rice;
            for _ in 0..p {
                b.bypass(1);
            }
            b.bypass(0);
            b.bypass_bits(v & ((1 << rice) - 1), rice);
        } else {
            let mut p = 4u32;
            while p < 31 && v >= (((1u32 << (p + 1 - 3)) + 2) << rice) {
                p += 1;
            }
            let base = ((1u32 << (p - 3)) + 2) << rice;
            for _ in 0..p {
                b.bypass(1);
            }
            b.bypass(0);
            b.bypass_bits(v - base, p - 3 + rice);
        }
    }
}
