// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/uavs3d.c`, `source/decore/inter_pred.c`), Copyright (c) 2018-2022 Peking
// University Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation.
// See LICENSE.uavs3d.
//! Coding unit decoding: the CU tree, prediction and reconstruction.

use std::sync::Arc;

use crate::Error;
use crate::core::{Core, Picture, Plane};
use crate::cuutil::*;
use crate::header::SliceType;
use crate::intra::{get_nbr, predict, predict_chroma, tscpm};
use crate::itrans::itrans;
use crate::mc::{mc_chroma, mc_luma};
use crate::types::*;

/// Writes a `w x h` block of samples into a plane.
fn put_block(plane: &mut Plane, x: usize, y: usize, w: usize, h: usize, src: &[i32]) {
    for j in 0..h {
        let row = (y + j) * plane.w + x;
        for i in 0..w {
            plane.data[row + i] = src[j * w + i] as u16;
        }
    }
}

/// Adds a residual to a prediction, clips, and writes the block into a plane.
#[allow(clippy::too_many_arguments)]
fn recon_block(plane: &mut Plane, x: usize, y: usize, w: usize, h: usize, pred: &[i32], pred_stride: usize, resi: Option<&[i16]>, bit_depth: u32) {
    let max = (1i32 << bit_depth) - 1;
    for j in 0..h {
        let row = (y + j) * plane.w + x;
        for i in 0..w {
            let p = pred[j * pred_stride + i];
            let v = match resi {
                Some(r) => clip3(0, max, p + i32::from(r[j * w + i])),
                None => p,
            };
            plane.data[row + i] = v as u16;
        }
    }
}

fn avg(a: &[i32], b: &[i32]) -> Vec<i32> {
    a.iter().zip(b).map(|(&p, &q)| (p + q + 1) >> 1).collect()
}

impl Core {
    fn ref_pic(&self, refi: i8, lidx: usize) -> Result<Arc<Picture>, Error> {
        self.refp
            .get(refi.max(0) as usize)
            .and_then(|r| r[lidx].as_ref())
            .map(|r| Arc::clone(&r.pic))
            .ok_or(Error::Invalid("reference picture index out of range"))
    }

    /// `com_mc`: the prediction of the luma (unless the chroma tree) and chroma blocks of the CU.
    fn inter_pred(&self) -> Result<[Vec<i32>; 3], Error> {
        let (x, y, w, h) = (self.cu_pix_x, self.cu_pix_y, self.cu_width, self.cu_height);
        let bd = self.seq.bit_depth_internal;
        let (max_x, max_y) = (self.seq.pic_width as i32 + 4, self.seq.pic_height as i32 + 4);
        let refi = self.refi;
        let valid = |r: i8| r >= 0;
        let lidx_base = if valid(refi[0]) { 0 } else { 1 };
        let bi = valid(refi[0]) && valid(refi[1]) && (!Arc::ptr_eq(&self.ref_pic(refi[0], 0)?, &self.ref_pic(refi[1], 1)?) || self.mv[0] != self.mv[1]);
        let mut out: [Vec<i32>; 3] = Default::default();
        let (cw, ch) = (w >> 1, h >> 1);
        if self.tree_status != TREE_C {
            let mut preds: Vec<Vec<i32>> = Vec::new();
            for i in 0..=usize::from(bi) {
                let lidx = if i == 0 { lidx_base } else { 1 - lidx_base };
                let rp = self.ref_pic(refi[lidx], lidx)?;
                let gx = ((x as i32) << 2) + i32::from(self.mv[lidx][0]);
                let gy = ((y as i32) << 2) + i32::from(self.mv[lidx][1]);
                let mut buf = vec![0i32; w * h];
                mc_luma(&rp.planes[0], gx, gy, w, h, max_x, max_y, bd, false, &mut buf);
                preds.push(buf);
            }
            out[0] = if bi { avg(&preds[0], &preds[1]) } else { preds.swap_remove(0) };
        }
        if self.tree_status != TREE_L {
            for c in 1..3 {
                let mut preds: Vec<Vec<i32>> = Vec::new();
                for i in 0..=usize::from(bi) {
                    let lidx = if i == 0 { lidx_base } else { 1 - lidx_base };
                    let rp = self.ref_pic(refi[lidx], lidx)?;
                    let gx = ((x as i32) << 2) + i32::from(self.mv[lidx][0]);
                    let gy = ((y as i32) << 2) + i32::from(self.mv[lidx][1]);
                    let mut buf = vec![0i32; cw * ch];
                    mc_chroma(&rp.planes[c], gx, gy, cw, ch, max_x >> 1, max_y >> 1, bd, false, &mut buf);
                    preds.push(buf);
                }
                out[c] = if bi { avg(&preds[0], &preds[1]) } else { preds.swap_remove(0) };
            }
        }
        Ok(out)
    }

    /// `com_mc_affine`
    fn affine_pred(&self) -> Result<[Vec<i32>; 3], Error> {
        let (x, y, w, h) = (self.cu_pix_x, self.cu_pix_y, self.cu_width, self.cu_height);
        let bd = self.seq.bit_depth_internal;
        let (max_x, max_y) = (self.seq.pic_width as i32 + 4, self.seq.pic_height as i32 + 4);
        let refi = self.refi;
        let valid = |r: i8| r >= 0;
        let bi = valid(refi[0]) && valid(refi[1]);
        let lidx_base = if valid(refi[0]) { 0 } else { 1 };
        let sb = if self.pichdr.affine_subblock_size_idx == 1 || bi { 8 } else { 4 };
        let (cw, ch) = (w >> 1, h >> 1);
        let mut out: [Vec<i32>; 3] = Default::default();

        let mut luma: Vec<Vec<i32>> = Vec::new();
        let mut chroma: [Vec<Vec<i32>>; 2] = Default::default();
        for i in 0..=usize::from(bi) {
            let lidx = if i == 0 { lidx_base } else { 1 - lidx_base };
            let rp = self.ref_pic(refi[lidx], lidx)?;
            let asb = &self.affine_sb_mv[lidx];
            // luma
            let mut buf = vec![0i32; w * h];
            let mut blk = vec![0i32; sb * sb];
            let mut idx = 0usize;
            for by in (0..h).step_by(sb) {
                for bx in (0..w).step_by(sb) {
                    let v = asb.get(idx).ok_or(Error::Invalid("affine block too large"))?;
                    let gx = (((x + bx) as i32) << 4) + v[0];
                    let gy = (((y + by) as i32) << 4) + v[1];
                    idx += 1;
                    mc_luma(&rp.planes[0], gx, gy, sb, sb, max_x, max_y, bd, true, &mut blk);
                    for j in 0..sb {
                        buf[(by + j) * w + bx..(by + j) * w + bx + sb].copy_from_slice(&blk[j * sb..(j + 1) * sb]);
                    }
                }
            }
            luma.push(buf);
            // chroma: 4x4 chroma samples per 8x8 luma samples
            for c in 0..2 {
                let mut cbuf = vec![0i32; cw * ch];
                let mut cblk = vec![0i32; 16];
                for by in (0..h).step_by(8) {
                    for bx in (0..w).step_by(8) {
                        let (mvx, mvy) = if sb == 4 {
                            let i0 = (by >> 2) * (w >> 2) + (bx >> 2);
                            let i1 = i0 + (w >> 2);
                            (
                                (asb[i0][0] + asb[i0 + 1][0] + asb[i1][0] + asb[i1 + 1][0] + 2) >> 2,
                                (asb[i0][1] + asb[i0 + 1][1] + asb[i1][1] + asb[i1 + 1][1] + 2) >> 2,
                            )
                        } else {
                            let i0 = (by >> 3) * (w >> 3) + (bx >> 3);
                            (asb[i0][0], asb[i0][1])
                        };
                        let gx = (((x + bx) as i32) << 4) + mvx;
                        let gy = (((y + by) as i32) << 4) + mvy;
                        mc_chroma(&rp.planes[1 + c], gx, gy, 4, 4, max_x >> 1, max_y >> 1, bd, true, &mut cblk);
                        for j in 0..4 {
                            let o = ((by >> 1) + j) * cw + (bx >> 1);
                            cbuf[o..o + 4].copy_from_slice(&cblk[j * 4..j * 4 + 4]);
                        }
                    }
                }
                chroma[c].push(cbuf);
            }
        }
        out[0] = if bi { avg(&luma[0], &luma[1]) } else { luma.swap_remove(0) };
        for c in 0..2 {
            out[1 + c] = if bi { avg(&chroma[c][0], &chroma[c][1]) } else { chroma[c].swap_remove(0) };
        }
        Ok(out)
    }

    /// Parses and inverse transforms one block of coefficients.
    fn residual(&mut self, log2_w: u32, log2_h: u32, ch_type: usize, nxn: Option<usize>, sec: u32, alt: bool) -> Vec<i16> {
        let n = 1usize << (log2_w + log2_h);
        let mut coef = vec![0i16; n];
        self.parse_run_length_cc(&mut coef, log2_w, log2_h, ch_type);
        let mut resi = vec![0i16; n];
        itrans(&coef, &mut resi, log2_w, log2_h, self.seq.bit_depth_internal, nxn, sec, alt);
        resi
    }

    /// Reads the raw samples of an IPCM block of one plane.
    fn ipcm_block(&mut self, comp: usize, x: usize, y: usize, w: usize, h: usize, left: &mut i32) {
        let bd_in = self.seq.bit_depth_input as i32;
        let shift = self.seq.bit_depth_internal - self.seq.bit_depth_input;
        let (tb_w, tb_h) = (w.min(32), h.min(32));
        for ty in (0..h).step_by(tb_h) {
            for tx in (0..w).step_by(tb_w) {
                for j in 0..tb_h {
                    for i in 0..tb_w {
                        let v = self.lbac.ipcm(left, bd_in) << shift;
                        let pl = &mut self.pic.planes[comp];
                        pl.data[(y + ty + j) * pl.w + x + tx + i] = v as u16;
                    }
                }
            }
        }
    }

    /// `dec_cu`
    pub(crate) fn dec_cu(&mut self, x: usize, y: usize, log2w: u32, log2h: u32) -> Result<(), Error> {
        let bd = self.seq.bit_depth_internal;
        let i_scu = self.seq.i_scu as i32;
        let (cw, ch) = (1usize << log2w, 1usize << log2h);
        self.cu_log2w = log2w;
        self.cu_log2h = log2h;
        self.cu_width = cw;
        self.cu_height = ch;
        self.cu_pix_x = x;
        self.cu_pix_y = y;

        if self.tree_status != TREE_C {
            self.parse_cu_header();
        } else {
            self.parse_cu_header_chroma();
        }
        if self.tree_status != TREE_C {
            self.update_map();
            if !self.pichdr.loop_filter_disable_flag {
                self.deblock_set_edge();
            }
        }

        if self.cu_mode != MODE_INTRA {
            if self.refi[0] < 0 && self.refi[1] < 0 {
                self.refi[0] = 0;
            }
            for l in 0..2 {
                if self.refi[l] >= 0 {
                    if self.num_refp[l] == 0 {
                        return Err(Error::Invalid("reference list is empty"));
                    }
                    self.refi[l] = self.refi[l].min(self.num_refp[l] as i8 - 1);
                }
            }
            let pred = if self.affine_flag != 0 { self.affine_pred()? } else { self.inter_pred()? };
            if self.tree_status != TREE_C {
                let (tb_w, tb_h) = tb_width_height(cw, ch, self.tb_part);
                let (lw, lh) = (log2_u(tb_w), log2_u(tb_h));
                for k in 0..part_num(self.tb_part) {
                    let (ox, oy) = tb_start_pos(cw, ch, self.tb_part, k);
                    let resi = if self.cbfy[k] != 0 && self.cbfy.iter().any(|&c| c != 0) {
                        let nxn = (self.tb_part == SIZE_NXN).then_some(k);
                        Some(self.residual(lw, lh, 0, nxn, 0, false))
                    } else {
                        None
                    };
                    let po = oy * cw + ox;
                    recon_block(&mut self.pic.planes[0], x + ox, y + oy, tb_w, tb_h, &pred[0][po..], cw, resi.as_deref(), bd);
                }
            }
            if self.tree_status != TREE_L {
                let (cxw, cxh) = (cw >> 1, ch >> 1);
                let mut resi: [Option<Vec<i16>>; 2] = [None, None];
                for c in 0..2 {
                    if self.cbfc[c] != 0 {
                        resi[c] = Some(self.residual(log2w - 1, log2h - 1, 1 + c, None, 0, false));
                    }
                }
                for c in 0..2 {
                    recon_block(&mut self.pic.planes[1 + c], x >> 1, y >> 1, cxw, cxh, &pred[1 + c], cxw, resi[c].as_deref(), bd);
                }
            }
            return Ok(());
        }

        // intra
        if self.tree_status != TREE_C {
            if self.ipm[0] as u8 == IPD_IPCM {
                if self.lbac.bin_trm() != 1 {
                    return Err(Error::Invalid("IPCM without terminating bin"));
                }
                self.lbac.ipcm_start();
                let mut left = 8;
                self.ipcm_block(0, x, y, cw, ch, &mut left);
                if left != 8 {
                    return Err(Error::Invalid("IPCM data not byte aligned"));
                }
                if self.tree_status == TREE_L || self.ipm_c != IPD_DM_C {
                    let cur = self.lbac.cur;
                    self.lbac.init_at(cur);
                }
            } else {
                let (tb_w, tb_h) = tb_width_height(cw, ch, self.tb_part);
                let (lw, lh) = (log2_u(tb_w), log2_u(tb_h));
                let sec_enable = self.seq.secondary_transform_enable_flag;
                for tb_idx in 0..part_num(self.tb_part) {
                    let mode = self.ipm[pb_idx_by_tb(self.pb_part, tb_idx)] as u8;
                    let (ox, oy) = tb_start_pos(cw, ch, self.tb_part, tb_idx);
                    let (tx, ty) = (x + ox, y + oy);
                    let tb_scup = ((ty >> 2) as i32) * i_scu + (tx >> 2) as i32;
                    let avail = avail_intra(&self.maps, tb_scup);
                    let nbr = get_nbr(&self.pic.planes[0], tx, ty, tb_w, tb_h, 4, avail, tb_scup, &self.maps, bd);
                    let mut pred = vec![0i32; tb_w * tb_h];
                    predict(&nbr, mode, tb_w, tb_h, bd, avail, self.ipf_flag, &mut pred);
                    let resi = if self.cbfy[tb_idx] != 0 {
                        let mut sec = 0u32;
                        if sec_enable && (lw > 2 || lh > 2) {
                            let up = avail & AVAIL_UP != 0;
                            let le = avail & AVAIL_LE != 0;
                            let vt = mode < IPD_HOR && up;
                            let ht = ((mode > IPD_VER && mode < IPD_CNT) || mode <= IPD_BI) && le;
                            sec = (u32::from(vt) << 1) | u32::from(ht);
                        }
                        let nxn = (self.tb_part == SIZE_NXN).then_some(tb_idx);
                        Some(self.residual(lw, lh, 0, nxn, sec, sec_enable))
                    } else {
                        None
                    };
                    recon_block(&mut self.pic.planes[0], tx, ty, tb_w, tb_h, &pred, tb_w, resi.as_deref(), bd);
                }
            }
        }

        if self.tree_status != TREE_L {
            let (cxw, cxh) = (cw >> 1, ch >> 1);
            let (cx, cy) = (x >> 1, y >> 1);
            if self.ipm[0] as u8 == IPD_IPCM && self.ipm_c == IPD_DM_C {
                if self.tree_status == TREE_C {
                    if self.lbac.bin_trm() != 1 {
                        return Err(Error::Invalid("IPCM without terminating bin"));
                    }
                    self.lbac.ipcm_start();
                }
                let mut left = 8;
                self.ipcm_block(1, cx, cy, cxw, cxh, &mut left);
                self.ipcm_block(2, cx, cy, cxw, cxh, &mut left);
                if left != 8 {
                    return Err(Error::Invalid("IPCM data not byte aligned"));
                }
                let cur = self.lbac.cur;
                self.lbac.init_at(cur);
            } else {
                let avail = avail_intra(&self.maps, self.cu_scup);
                let nu = get_nbr(&self.pic.planes[1], cx, cy, cxw, cxh, 2, avail, self.cu_scup, &self.maps, bd);
                let nv = get_nbr(&self.pic.planes[2], cx, cy, cxw, cxh, 2, avail, self.cu_scup, &self.maps, bd);
                let mut pred_u = vec![0i32; cxw * cxh];
                let mut pred_v = vec![0i32; cxw * cxh];
                if self.ipm_c == IPD_TSCPM_C {
                    let nl = get_nbr(&self.pic.planes[0], x, y, cw, ch, 4, avail, self.cu_scup, &self.maps, bd);
                    let mut rec = vec![0i32; cw * ch];
                    let pl = &self.pic.planes[0];
                    for j in 0..ch {
                        for i in 0..cw {
                            rec[j * cw + i] = i32::from(pl.data[(y + j) * pl.w + x + i]);
                        }
                    }
                    tscpm(&nu, &nv, &nl, &rec, cxw, cxh, avail & AVAIL_UP != 0, avail & AVAIL_LE != 0, bd, &mut pred_u, &mut pred_v);
                } else {
                    let ipm = self.ipm[0] as u8;
                    predict_chroma(&nu, self.ipm_c, ipm, cxw, cxh, bd, avail, &mut pred_u);
                    predict_chroma(&nv, self.ipm_c, ipm, cxw, cxh, bd, avail, &mut pred_v);
                }
                let mut resi: [Option<Vec<i16>>; 2] = [None, None];
                for c in 0..2 {
                    if self.cbfc[c] != 0 {
                        resi[c] = Some(self.residual(log2w - 1, log2h - 1, 1 + c, None, 0, false));
                    }
                }
                recon_block(&mut self.pic.planes[1], cx, cy, cxw, cxh, &pred_u, cxw, resi[0].as_deref(), bd);
                recon_block(&mut self.pic.planes[2], cx, cy, cxw, cxh, &pred_v, cxw, resi[1].as_deref(), bd);
            }
        }
        let _ = put_block;
        Ok(())
    }

    /// `decode_cu_tree`
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_cu_tree(
        &mut self,
        x0: usize,
        y0: usize,
        log2w: u32,
        log2h: u32,
        mut qt_depth: u32,
        mut bet_depth: u32,
        mut cons_pred_mode: u8,
        tree_status: u8,
    ) -> Result<(), Error> {
        let (cw, ch) = (1usize << log2w, 1usize << log2h);
        let i_scu = self.seq.i_scu;
        self.tree_scup = ((y0 >> 2) * i_scu + (x0 >> 2)) as i32;
        let mut split_mode = SPLIT_NONE;
        if cw > 4 || ch > 4 {
            let tab = split_available(&self.seq, x0, y0, cw, ch, qt_depth, bet_depth, self.slice_type);
            split_mode = self.parse_split_mode(tab, cw, ch);
        }
        if split_mode != SPLIT_NONE {
            let child_status = if tree_status == TREE_LC && is_separate_tree(cw, ch, split_mode) { TREE_L } else { tree_status };
            let ss = split_struct(split_mode, x0, y0, cw, ch);
            if cons_pred_mode == NO_MODE_CONS && self.slice_type != SliceType::I && cons_allow(cw, ch, split_mode) {
                cons_pred_mode = self.parse_cons_pred_mode_child();
            }
            if split_mode == SPLIT_QUAD {
                qt_depth += 1;
            } else {
                bet_depth += 1;
            }
            for p in 0..ss.count {
                if ss.x[p] < self.seq.pic_width && ss.y[p] < self.seq.pic_height {
                    self.decode_cu_tree(ss.x[p], ss.y[p], ss.log_w[p], ss.log_h[p], qt_depth, bet_depth, cons_pred_mode, child_status)?;
                }
            }
            if child_status == TREE_L && tree_status == TREE_LC {
                self.tree_status = TREE_C;
                self.cu_scup = ((y0 >> 2) * i_scu + (x0 >> 2)) as i32;
                self.dec_cu(x0, y0, log2w, log2h)?;
            }
        } else {
            self.tree_status = tree_status;
            self.cons_pred_mode = cons_pred_mode;
            self.cu_scup = self.tree_scup;
            self.dec_cu(x0, y0, log2w, log2h)?;
            if self.seq.num_of_hmvp_cand != 0 && self.cu_mode != MODE_INTRA && self.affine_flag == 0 {
                self.update_hmvp_candidates();
            }
        }
        Ok(())
    }
}
