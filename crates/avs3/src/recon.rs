// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! Reconstruction of a coding unit and the coding tree walk.

use crate::blocks::{self, NO_SPLIT, SIZE_NXN, SPLIT_QUAD};
use crate::core::{Core, MODE_INTRA, NO_MODE_CONS, TREE_C, TREE_L, TREE_LC};
use crate::header::SliceType;
use crate::intra::{AVAIL_LE, AVAIL_UP, IPD_BI, IPD_CNT, IPD_DM_C, IPD_HOR, IPD_IPCM, IPD_VER};
use crate::transform::itrans;

impl Core {
    /// Writes `pred` (+ `resi`) of a `w` × `h` block at (`x`, `y`) of plane `comp`.
    fn put(&mut self, comp: usize, x: i32, y: i32, w: i32, h: i32, pred: &[u16], resi: Option<&[i16]>) {
        let max = (1i32 << self.seq.bit_depth) - 1;
        let Some(pl) = self.cur.planes.get_mut(comp) else {
            return;
        };
        for j in 0..h {
            let yy = y + j;
            if yy < 0 || yy as usize >= pl.h {
                continue;
            }
            for i in 0..w {
                let xx = x + i;
                if xx < 0 || xx as usize >= pl.w {
                    continue;
                }
                let k = (j * w + i) as usize;
                let p = i32::from(pred.get(k).copied().unwrap_or(0));
                let v = match resi {
                    Some(r) => (p + i32::from(r.get(k).copied().unwrap_or(0))).clamp(0, max),
                    None => p,
                };
                pl.data[yy as usize * pl.w + xx as usize] = v as u16;
            }
        }
    }

    fn residual(&mut self, comp: usize, lw: u32, lh: u32, blk: usize, nxn: bool, sec: u32) -> Vec<i16> {
        let mut coef = self.dec_coefs(lw, lh, comp);
        let mut resi = vec![0i16; 1 << (lw + lh)];
        itrans(nxn, blk, &mut coef, &mut resi, lw, lh, self.seq.bit_depth as i32, sec, false);
        resi
    }

    /// Residual of a luma intra block (the alternative 4x4 transform is allowed here).
    fn residual_intra_luma(&mut self, lw: u32, lh: u32, blk: usize, nxn: bool, sec: u32) -> Vec<i16> {
        let mut coef = self.dec_coefs(lw, lh, 0);
        let mut resi = vec![0i16; 1 << (lw + lh)];
        itrans(nxn, blk, &mut coef, &mut resi, lw, lh, self.seq.bit_depth as i32, sec, self.seq.secondary_transform);
        resi
    }

    fn sub_block(src: &[u16], stride: i32, x: i32, y: i32, w: i32, h: i32) -> Vec<u16> {
        let mut out = Vec::with_capacity((w * h) as usize);
        for j in 0..h {
            for i in 0..w {
                out.push(src.get(((y + j) * stride + x + i) as usize).copied().unwrap_or(0));
            }
        }
        out
    }

    fn ipcm_block(&mut self, comp: usize, x: i32, y: i32, w: i32, h: i32, left: &mut i32, bits: i32, shift: u32) {
        let tb_w = w.min(32);
        let tb_h = h.min(32);
        let mut ty = 0;
        while ty < h {
            let mut tx = 0;
            while tx < w {
                let mut blk = Vec::with_capacity((tb_w * tb_h) as usize);
                for _ in 0..tb_h * tb_w {
                    blk.push((self.lbac.ipcm_read(left, bits) << shift) as u16);
                }
                self.put(comp, x + tx, y + ty, tb_w, tb_h, &blk, None);
                tx += tb_w;
            }
            ty += tb_h;
        }
    }

    /// `dec_cu`: parses and reconstructs one coding unit (or the chroma part of a separate tree).
    pub(crate) fn dec_cu(&mut self, x: i32, y: i32, lw: u32, lh: u32) {
        self.log2w = lw;
        self.log2h = lh;
        self.cu_w = 1 << lw;
        self.cu_h = 1 << lh;
        self.cu_x = x;
        self.cu_y = y;
        let (cw, ch) = (self.cu_w, self.cu_h);
        let bd = self.seq.bit_depth as i32;
        if self.tree_status != TREE_C {
            self.dec_cu_header();
            self.update_map();
            if !self.ph.loop_filter_disable {
                self.deblock_set_edge();
            }
        } else {
            self.dec_cu_header_chroma();
        }

        if self.cu_mode != MODE_INTRA {
            if self.refi == [-1, -1] {
                self.refi[0] = 0;
            }
            for l in 0..2 {
                self.refi[l] = self.refi[l].min(self.num_refp[l] as i8 - 1);
            }
            let pred = if self.affine_flag != 0 { self.mc_affine() } else { self.mc() };
            if self.tree_status != TREE_C {
                if self.cbfy != [0; 4] {
                    let (tw, th) = blocks::tb_size(cw, ch, self.tb_part);
                    let (ltw, lth) = (tw.trailing_zeros(), th.trailing_zeros());
                    for k in 0..blocks::PART_NUM[self.tb_part as usize] {
                        let (tx, ty) = blocks::tb_start(cw, ch, self.tb_part, k);
                        let p = Self::sub_block(&pred.y, cw, tx, ty, tw, th);
                        if self.cbfy[k] != 0 {
                            let r = self.residual(0, ltw, lth, k, self.tb_part == SIZE_NXN, 0);
                            self.put(0, x + tx, y + ty, tw, th, &p, Some(&r));
                        } else {
                            self.put(0, x + tx, y + ty, tw, th, &p, None);
                        }
                    }
                } else {
                    self.put(0, x, y, cw, ch, &pred.y, None);
                }
            }
            if self.tree_status != TREE_L {
                let (cx, cy, w2, h2) = (x >> 1, y >> 1, cw >> 1, ch >> 1);
                for (i, p) in [&pred.u, &pred.v].into_iter().enumerate() {
                    if self.cbfc[i] != 0 {
                        let r = self.residual(i + 1, lw - 1, lh - 1, 0, false, 0);
                        self.put(i + 1, cx, cy, w2, h2, p, Some(&r));
                    } else {
                        self.put(i + 1, cx, cy, w2, h2, p, None);
                    }
                }
            }
            return;
        }

        // intra
        let ipcm0 = i32::from(self.ipm[0]) == IPD_IPCM;
        let in_bits = self.seq.output_bit_depth() as i32;
        let shift = (bd - in_bits).max(0) as u32;
        if self.tree_status != TREE_C {
            if ipcm0 {
                let _ = self.lbac.bin_trm();
                self.lbac.ipcm_start();
                let mut left = 8;
                self.ipcm_block(0, x, y, cw, ch, &mut left, in_bits, shift);
                if self.tree_status == TREE_L || i32::from(self.ipm_c) != IPD_DM_C {
                    self.lbac.restart();
                }
            } else {
                let (tw, th) = blocks::tb_size(cw, ch, self.tb_part);
                let (ltw, lth) = (tw.trailing_zeros(), th.trailing_zeros());
                let sec_on = self.seq.secondary_transform;
                for k in 0..blocks::PART_NUM[self.tb_part as usize] {
                    let mode = i32::from(self.ipm[blocks::pb_idx_by_tb(self.pb_part, k).min(3)]);
                    let (tx, ty) = blocks::tb_start(cw, ch, self.tb_part, k);
                    let (bx, by) = (x + tx, y + ty);
                    let scup = (by >> 2) * self.i_scu + (bx >> 2);
                    let avail = self.avail_intra(scup);
                    let pred = self.ipred_luma(bx, by, tw, th, mode, self.ipf_flag, scup);
                    if self.cbfy[k] != 0 {
                        let mut sec = 0u32;
                        if sec_on && (ltw > 2 || lth > 2) {
                            let vt = mode < IPD_HOR && avail & AVAIL_UP != 0;
                            let ht = ((mode > IPD_VER && mode < IPD_CNT) || mode <= IPD_BI) && avail & AVAIL_LE != 0;
                            sec = (u32::from(vt) << 1) | u32::from(ht);
                        }
                        let r = self.residual_intra_luma(ltw, lth, k, self.tb_part == SIZE_NXN, sec);
                        self.put(0, bx, by, tw, th, &pred, Some(&r));
                    } else {
                        self.put(0, bx, by, tw, th, &pred, None);
                    }
                }
            }
        }
        if self.tree_status != TREE_L {
            let (cx, cy, w2, h2) = (x >> 1, y >> 1, cw >> 1, ch >> 1);
            if ipcm0 && i32::from(self.ipm_c) == IPD_DM_C {
                if self.tree_status == TREE_C {
                    let _ = self.lbac.bin_trm();
                    self.lbac.ipcm_start();
                }
                let mut left = 8;
                self.ipcm_block(1, cx, cy, w2, h2, &mut left, in_bits, shift);
                self.ipcm_block(2, cx, cy, w2, h2, &mut left, in_bits, shift);
                self.lbac.restart();
            } else {
                let preds = self.ipred_chroma(x, y, cw, ch, i32::from(self.ipm_c), i32::from(self.ipm[0]), self.cu_scup);
                for (i, p) in preds.iter().enumerate() {
                    if self.cbfc[i] != 0 {
                        let r = self.residual(i + 1, lw - 1, lh - 1, 0, false, 0);
                        self.put(i + 1, cx, cy, w2, h2, p, Some(&r));
                    } else {
                        self.put(i + 1, cx, cy, w2, h2, p, None);
                    }
                }
            }
        }
    }

    /// `decode_cu_tree`.
    pub(crate) fn decode_cu_tree(&mut self, x0: i32, y0: i32, lw: u32, lh: u32, qt: u32, bet: u32, cons: u8, tree: u8) {
        let (w, h) = (1i32 << lw, 1i32 << lh);
        self.tree_scup = (y0 >> 2) * self.i_scu + (x0 >> 2);
        let mut split = NO_SPLIT;
        if w > 4 || h > 4 {
            let tab = blocks::split_available(&self.seq, x0, y0, w, h, qt, bet, self.slice_type);
            split = self.dec_split_mode(tab, w, h);
        }
        if split != NO_SPLIT {
            let sp = blocks::split_struct(split, x0, y0, w, h);
            let child_tree = if tree == TREE_LC && blocks::is_separate_tree(w, h, split) { TREE_L } else { tree };
            let mut cons = cons;
            if cons == NO_MODE_CONS && self.slice_type != SliceType::I && blocks::cons_allow(w, h, split) {
                cons = self.dec_cons_pred_mode_child();
            }
            let (qt, bet) = if split == SPLIT_QUAD { (qt + 1, bet) } else { (qt, bet + 1) };
            for p in 0..sp.count {
                if sp.x[p] < self.seq.pic_width as i32 && sp.y[p] < self.seq.pic_height as i32 {
                    self.decode_cu_tree(sp.x[p], sp.y[p], sp.log_w[p], sp.log_h[p], qt, bet, cons, child_tree);
                }
            }
            if child_tree == TREE_L && tree == TREE_LC {
                self.tree_status = TREE_C;
                self.cu_scup = (y0 >> 2) * self.i_scu + (x0 >> 2);
                self.dec_cu(x0, y0, lw, lh);
            }
        } else {
            self.tree_status = tree;
            self.cons_pred_mode = cons;
            self.cu_scup = self.tree_scup;
            self.dec_cu(x0, y0, lw, lh);
            if self.seq.num_hmvp_cand != 0 && self.cu_mode != MODE_INTRA && self.affine_flag == 0 {
                self.update_hmvp();
            }
        }
    }
}
