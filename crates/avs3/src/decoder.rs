// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! Picture-level decoding: patches and LCUs, the picture manager and the output order.

use std::sync::Arc;

use crate::Error;
use crate::bits::{demulate, start_codes};
use crate::core::{Core, Pic, RefInfo};
use crate::header::{DoiState, PatchHeader, Picture, Sequence, SliceType};
use crate::lbac::{Ctx as LbacCtx, Lbac};
use crate::tables::QP_CHROMA_ADJUST;

/// The largest coded picture (8192×4608 samples), against hostile headers.
const MAX_PIXELS: u64 = 8192 * 4608;
/// The most pictures the manager keeps (`MAX_PB_SIZE` in uavs3d is smaller; this only bounds memory).
const MAX_SLOTS: usize = 40;

/// A decoded picture held by the picture manager.
struct Slot {
    pic: Arc<Pic>,
    doi: i32,
    is_ref: bool,
    is_output: bool,
    output_delay: i32,
    ptr: i64,
    tag: i64,
}

/// A decoded picture ready for output.
pub(crate) struct OutPic {
    pub poc: i64,
    pub tag: i64,
    pub pic: Arc<Pic>,
}

/// The state of the picture being decoded between its header and its patches.
struct Current {
    ph: Picture,
    tag: i64,
    /// First LCU row of the next patch.
    next_row: u32,
    patches: usize,
}

#[derive(Default)]
pub(crate) struct Ctx {
    pub seq: Option<Sequence>,
    core: Option<Box<Core>>,
    doi: DoiState,
    slots: Vec<Slot>,
    cur: Option<Current>,
    /// Decoded pictures not yet matched by an output attempt (`ctx->output`).
    decoded: usize,
    cur_decoded_doi: i32,
}

impl Ctx {
    pub(crate) fn reset(&mut self) {
        self.slots.clear();
        self.cur = None;
        self.decoded = 0;
        self.doi = DoiState::default();
    }

    fn set_sequence(&mut self, seq: Sequence) -> Result<(), Error> {
        if u64::from(seq.pic_width) * u64::from(seq.pic_height) > MAX_PIXELS {
            return Err(Error::Unsupported("picture size"));
        }
        let same = self.seq.as_ref().is_some_and(|s| {
            s.pic_width == seq.pic_width
                && s.pic_height == seq.pic_height
                && s.log2_max_cu == seq.log2_max_cu
                && s.bit_depth == seq.bit_depth
                && s.patch_height == seq.patch_height
        });
        if !same || self.core.is_none() {
            self.slots.clear();
            self.core = Some(Box::new(Core::new(&seq)));
        } else if let Some(c) = self.core.as_mut() {
            c.seq = seq.clone();
        }
        self.seq = Some(seq);
        Ok(())
    }

    /// Decodes one packet: sequence header, sequence end or a picture header with its patches.
    pub(crate) fn decode_packet(&mut self, packet: &[u8], tag: i64) -> Result<(), Error> {
        let codes: Vec<(u8, usize)> = start_codes(packet).collect();
        let mut result = Ok(());
        for (k, &(code, at)) in codes.iter().enumerate() {
            let end = codes.get(k + 1).map_or(packet.len(), |&(_, n)| n);
            let unit = packet.get(at + 3..end).unwrap_or_default();
            let r = match code {
                0xB0 => {
                    self.finish_picture();
                    let dem = demulate(unit);
                    Sequence::parse(dem.get(1..).unwrap_or_default()).and_then(|s| self.set_sequence(s))
                }
                0xB1 | 0xB7 => {
                    self.finish_picture();
                    Ok(())
                }
                0xB3 | 0xB6 => {
                    self.finish_picture();
                    let dem = demulate(unit);
                    self.start_picture(code, dem.get(1..).unwrap_or_default(), tag)
                }
                0x00..=0x8E => {
                    let dem = demulate(unit);
                    self.decode_patch(code, dem.get(1..).unwrap_or_default())
                }
                _ => Ok(()),
            };
            if r.is_err() && result.is_ok() {
                result = r;
            }
        }
        self.finish_picture();
        result
    }

    fn start_picture(&mut self, code: u8, body: &[u8], tag: i64) -> Result<(), Error> {
        let seq = self.seq.clone().ok_or(Error::Invalid("picture before the sequence header"))?;
        let (ph, wrapped) = Picture::parse(body, code, &seq, &mut self.doi)?;
        if wrapped {
            for s in &mut self.slots {
                s.doi -= 256;
            }
        }
        let cur_doi = ph.doi as i32;
        // mark_refp
        let keep = |s: &Slot| {
            ph.rpl_l0.delta_doi.iter().take(ph.rpl_l0.num).any(|&d| s.doi == cur_doi - d)
                || ph.rpl_l1.delta_doi.iter().take(ph.rpl_l1.num).any(|&d| s.doi == cur_doi - d)
        };
        for s in &mut self.slots {
            if s.is_ref && !keep(s) {
                s.is_ref = false;
            }
        }
        // get_active_refp
        let find = |slots: &[Slot], d: i32| slots.iter().position(|s| s.is_ref && s.doi == cur_doi - d);
        let core = self.core.as_mut().ok_or(Error::Invalid("no sequence"))?;
        core.num_refp = [0, 0];
        for r in core.refp.iter_mut() {
            *r = [RefInfo::default(), RefInfo::default()];
        }
        let dist = |ref_ptr: i64| -> i32 {
            let d = i32::from((ph.ptr - ref_ptr) as i16);
            if d == 0 { 1 } else { d }
        };
        let mut lost = false;
        if ph.slice_type != SliceType::I {
            for i in 0..ph.rpl_l0.active.min(ph.rpl_l0.delta_doi.len()) {
                match find(&self.slots, ph.rpl_l0.delta_doi[i]) {
                    Some(j) => {
                        let s = &self.slots[j];
                        core.refp[i][0] = RefInfo { pic: Some(s.pic.clone()), dist: dist(s.ptr) };
                        core.num_refp[0] += 1;
                    }
                    None => lost = true,
                }
            }
            if ph.slice_type == SliceType::B {
                for i in 0..ph.rpl_l1.active.min(ph.rpl_l1.delta_doi.len()) {
                    match find(&self.slots, ph.rpl_l1.delta_doi[i]) {
                        Some(j) => {
                            let s = &self.slots[j];
                            core.refp[i][1] = RefInfo { pic: Some(s.pic.clone()), dist: dist(s.ptr) };
                            core.num_refp[1] += 1;
                        }
                        None => lost = true,
                    }
                }
            }
            if lost || core.num_refp[0] == 0 {
                // a leading picture after a seek, or a lost reference: skip the picture
                self.cur = None;
                return Ok(());
            }
        }
        core.slice_type = ph.slice_type;
        let mut pic = Core::new_pic(&seq);
        pic.ptr = ph.ptr;
        pic.doi = cur_doi;
        pic.tag = tag;
        for i in 0..core.num_refp[0] {
            pic.list_dist[i] = core.refp[i][0].dist as i16;
        }
        core.cur = pic;
        if !ph.loop_filter_disable {
            core.map_edge.iter_mut().for_each(|e| *e = 0);
        }
        core.ph = ph.clone();
        self.cur = Some(Current { ph, tag, next_row: 0, patches: 0 });
        Ok(())
    }

    fn decode_patch(&mut self, code: u8, body: &[u8]) -> Result<(), Error> {
        let Some(cur) = self.cur.as_mut() else {
            return Ok(());
        };
        let seq = self.seq.as_ref().ok_or(Error::Invalid("no sequence"))?;
        let core = self.core.as_mut().ok_or(Error::Invalid("no sequence"))?;
        let rows = seq.patch_rows();
        let idx = usize::from(code);
        let nrows = *rows.get(idx).ok_or(Error::Invalid("patch index"))?;
        if idx != cur.patches {
            return Err(Error::Invalid("patch order"));
        }
        let pat = PatchHeader::parse(u32::from(code), body, seq, &cur.ph)?;
        let data = body.get(pat.len..).unwrap_or_default().to_vec();
        // map_scu reset
        if idx == 0 || cur.ph.loop_filter_disable || !seq.cross_patch_loop_filter {
            core.map_scu.iter_mut().for_each(|v| *v = 0);
        } else {
            core.map_scu.iter_mut().for_each(|v| *v &= crate::core::SCU_CBF);
        }
        let start = cur.next_row;
        cur.next_row += nrows;
        cur.patches += 1;
        core.lbac = Lbac::new(data);
        core.ctx = LbacCtx::new();
        core.pat = pat;
        let (wl, hl) = (seq.w_lcu, seq.h_lcu);
        let end = (start + nrows).min(hl);
        for ly in start..end {
            for lx in 0..wl {
                if let Some(m) = core.map_patch.get_mut((ly * wl + lx) as usize) {
                    *m = idx as i8;
                }
            }
        }
        let off = seq.bit_depth_2_qp_offset;
        let mut lcu_qp = core.pat.slice_qp;
        let mut delta = 0;
        'rows: for ly in start..end {
            for lx in 0..wl {
                let lcu_idx = (ly * wl + lx) as usize;
                core.lcu_x = lx as i32;
                core.lcu_y = ly as i32;
                core.lcu_pix_x = (lx << seq.log2_max_cu) as i32;
                core.lcu_pix_y = (ly << seq.log2_max_cu) as i32;
                if lx == 0 {
                    core.hmvp_cnt = 0;
                }
                if core.pat.fixed_slice_qp {
                    lcu_qp = core.pat.slice_qp;
                } else {
                    delta = core.dec_lcu_delta_qp(delta);
                    lcu_qp += delta;
                }
                lcu_qp = lcu_qp.clamp(0, 63 + off);
                let adj = |d: i32| {
                    let mut a = lcu_qp + d - off;
                    if a >= 0 {
                        a = i32::from(QP_CHROMA_ADJUST[a.min(63) as usize]);
                    }
                    (a + off).max(0)
                };
                core.qp_y = lcu_qp;
                core.qp_u = adj(cur.ph.chroma_qp_delta_cb);
                core.qp_v = adj(cur.ph.chroma_qp_delta_cr);
                if let Some(q) = core.map_qp.get_mut(lcu_idx) {
                    *q = lcu_qp as i8;
                }
                if seq.sao_enable {
                    let p = core.dec_sao_param(lcu_idx);
                    if let Some(s) = core.sao_map.get_mut(lcu_idx) {
                        *s = p;
                    }
                }
                if seq.alf_enable {
                    let mut f = [false; 3];
                    for (c, v) in f.iter_mut().enumerate() {
                        if cur.ph.alf.flag[c] {
                            *v = core.dec_alf_enable();
                        }
                    }
                    if let Some(s) = core.alf_map.get_mut(lcu_idx) {
                        *s = f;
                    }
                }
                let lc = seq.log2_max_cu;
                core.decode_cu_tree(core.lcu_pix_x, core.lcu_pix_y, lc, lc, 0, 0, crate::core::NO_MODE_CONS, crate::core::TREE_LC);
                if core.lbac.bin_trm() != 0 {
                    break 'rows;
                }
            }
        }
        Ok(())
    }

    /// Runs the in-loop filters and files the decoded picture, if one is being decoded.
    fn finish_picture(&mut self) {
        let Some(cur) = self.cur.take() else {
            return;
        };
        let (Some(core), Some(seq)) = (self.core.as_mut(), self.seq.as_ref()) else {
            return;
        };
        if cur.patches == 0 {
            return;
        }
        if !cur.ph.loop_filter_disable {
            core.deblock_picture();
        }
        if seq.sao_enable {
            core.sao_picture();
        }
        if seq.alf_enable && cur.ph.alf.enable {
            core.alf_picture();
        }
        let pic = std::mem::replace(&mut core.cur, Core::new_pic(seq));
        let doi = pic.doi;
        self.slots.retain(|s| s.is_ref || s.is_output);
        while self.slots.len() >= MAX_SLOTS {
            self.slots.remove(0);
        }
        self.slots.push(Slot { ptr: pic.ptr, tag: cur.tag, doi, is_ref: true, is_output: true, output_delay: cur.ph.output_delay as i32, pic: Arc::new(pic) });
        self.cur_decoded_doi = doi;
        self.decoded += 1;
    }

    /// `com_picman_out_pic` (one output per decoded picture, or bumping when flushing).
    pub(crate) fn next_output(&mut self, flush: bool) -> Option<OutPic> {
        if flush {
            let j = self.slots.iter().enumerate().filter(|(_, s)| s.is_output).min_by_key(|(_, s)| s.ptr)?.0;
            return self.take(j);
        }
        if self.decoded == 0 {
            return None;
        }
        let cur = self.cur_decoded_doi;
        let mut best: Option<usize> = None;
        for (i, s) in self.slots.iter().enumerate() {
            if s.is_output && s.doi + s.output_delay <= cur && best.is_none_or(|b| s.ptr <= self.slots[b].ptr) {
                best = Some(i);
            }
        }
        let j = best?;
        self.decoded -= 1;
        self.take(j)
    }

    fn take(&mut self, j: usize) -> Option<OutPic> {
        let s = self.slots.get_mut(j)?;
        s.is_output = false;
        Some(OutPic { poc: s.ptr, tag: s.tag, pic: s.pic.clone() })
    }
}
