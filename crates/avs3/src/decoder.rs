// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/uavs3d.c`, `source/decore/pic_manager.c`), Copyright (c) 2018-2022 Peking
// University Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation.
// See LICENSE.uavs3d.
//! Picture decoding (patches, LCU rows) and the decoded picture buffer.

use std::sync::Arc;

use crate::Error;
use crate::bits::{Bits, demulate};
use crate::core::{Core, Maps, Picture, Plane, RefPic, SaoParam};
use crate::header::{DoiState, MAX_QUANT_BASE, MAX_REFS, PatchHeader, PicHeader, SliceType, parse_ext_and_user_data};
use crate::lbac::{Ctx as LbacCtx, Lbac};
use crate::types::*;

/// The most pictures the buffer holds before the stream is rejected.
const MAX_DPB: usize = MAX_REFS + 8;
const DOI_CYCLE_LENGTH: i64 = 256;

impl Core {
    pub(crate) fn new(seq: crate::header::Sequence) -> Core {
        let maps = Maps::new(&seq);
        Core {
            seq,
            pichdr: PicHeader::default(),
            pathdr: PatchHeader::default(),
            lbac: Lbac::empty(),
            ctx: LbacCtx::new(),
            pic: Picture::default(),
            maps,
            slice_type: SliceType::I,
            num_refp: [0; 2],
            refp: Vec::new(),
            cu_mode: 0,
            pb_part: 0,
            tb_part: 0,
            cbfy: [0; 4],
            cbfc: [0; 2],
            refi: [REFI_INVALID; 2],
            ipm: [0; 4],
            ipm_c: 0,
            ipf_flag: false,
            affine_flag: 0,
            mv: [[0; 2]; 2],
            affine_mv: [[[0; 2]; 4]; 2],
            affine_sb_mv: [[[0; 2]; 256]; 2],
            cu_scup: 0,
            cu_pix_x: 0,
            cu_pix_y: 0,
            cu_log2w: 0,
            cu_log2h: 0,
            cu_width: 0,
            cu_height: 0,
            tree_scup: 0,
            tree_status: TREE_LC,
            cons_pred_mode: NO_MODE_CONS,
            lcu_x: 0,
            lcu_y: 0,
            lcu_pix_x: 0,
            lcu_pix_y: 0,
            lcu_qp_y: 0,
            lcu_qp_u: 0,
            lcu_qp_v: 0,
            hmvp_cands_cnt: 0,
            hmvp_cands_list: [Motion::default(); ALLOWED_HMVP_NUM],
            sao_param_map: Vec::new(),
            alf_enable_map: Vec::new(),
        }
    }

    fn begin_picture(&mut self, pichdr: PicHeader, refp: Vec<[Option<RefPic>; 2]>, num_refp: [usize; 2]) {
        let (w, h) = (self.seq.pic_width, self.seq.pic_height);
        self.slice_type = pichdr.slice_type;
        self.pichdr = pichdr;
        self.refp = refp;
        self.num_refp = num_refp;
        self.maps = Maps::new(&self.seq);
        self.pic = Picture { planes: [Plane::new(w, h), Plane::new(w / 2, h / 2), Plane::new(w / 2, h / 2)], ..Picture::default() };
        for (i, r) in self.refp.iter().enumerate().take(MAX_REFS) {
            if let Some(r) = &r[0] {
                self.pic.list_dist[i] = r.dist as i16;
            }
        }
        self.sao_param_map = vec![[SaoParam::default(); 3]; self.seq.f_lcu];
        self.alf_enable_map = vec![[false; 3]; self.seq.f_lcu];
        self.affine_flag = 0;
        self.hmvp_cands_cnt = 0;
    }

    /// `dec_pic`: decodes the patches (`units` are the de-emulated slice units) of the picture started by
    /// [`begin_picture`](Self::begin_picture), then runs the loop filters.
    fn decode_patches(&mut self, units: &[Vec<u8>]) -> Result<(), Error> {
        let mut unit_iter = units.iter();
        let mut new_patch = true;
        let mut left_lcu = self.seq.f_lcu;
        let (mut patch_start_x, mut patch_end_x, mut patch_end_y) = (0usize, 0usize, 0usize);
        let mut lcu_qp = 0i32;
        let mut lcu_qp_delta = 0i32;
        let wl = self.seq.pic_width_in_lcu;
        loop {
            if new_patch {
                let unit = unit_iter.next().ok_or(Error::Truncated)?;
                let mut b = Bits::new(unit);
                let ph = PatchHeader::parse(&mut b, &self.seq, &self.pichdr)?;
                if ph.index >= self.seq.patch_columns * self.seq.patch_rows {
                    return Err(Error::Invalid("patch index out of range"));
                }
                self.pathdr = ph;
                if ph.index == 0 || self.pichdr.loop_filter_disable_flag || !self.seq.cross_patch_loop_filter {
                    self.maps.scu.fill(Scu::default());
                } else {
                    for s in self.maps.scu.iter_mut() {
                        *s = Scu { cbf: s.cbf, ..Scu::default() };
                    }
                }
                self.lbac.set_data(&unit[b.byte_pos()..]);
                self.lbac.init_at(0);
                self.ctx = LbacCtx::new();
                let (patch_x, patch_y) = (ph.index % self.seq.patch_columns, ph.index / self.seq.patch_columns);
                patch_start_x = self.seq.patch_column_width[..patch_x].iter().sum();
                let patch_start_y: usize = self.seq.patch_row_height[..patch_y].iter().sum();
                self.lcu_x = patch_start_x;
                self.lcu_y = patch_start_y;
                patch_end_x = patch_start_x + self.seq.patch_column_width[patch_x];
                patch_end_y = patch_start_y + self.seq.patch_row_height[patch_y];
                for j in patch_start_y..patch_end_y {
                    for i in patch_start_x..patch_end_x {
                        self.maps.patch[j * wl + i] = ph.index as i8;
                    }
                }
                lcu_qp = ph.slice_qp;
                lcu_qp_delta = 0;
            } else {
                self.lcu_x += 1;
                if self.lcu_x >= patch_end_x {
                    self.lcu_x = patch_start_x;
                    self.lcu_y += 1;
                    if self.lcu_y >= patch_end_y {
                        return Err(Error::Invalid("patch overrun"));
                    }
                }
            }
            let log2 = self.seq.log2_max_cuwh;
            self.lcu_pix_x = self.lcu_x << log2;
            self.lcu_pix_y = self.lcu_y << log2;
            let lcu_idx = self.lcu_y * wl + self.lcu_x;
            if lcu_idx >= self.seq.f_lcu {
                return Err(Error::Invalid("LCU out of range"));
            }
            if self.lcu_x == patch_start_x {
                self.hmvp_cands_cnt = 0;
            }
            if self.pathdr.fixed_slice_qp_flag {
                lcu_qp = self.pathdr.slice_qp;
            } else {
                lcu_qp_delta = self.parse_lcu_delta_qp(lcu_qp_delta);
                lcu_qp += lcu_qp_delta;
            }
            let off = self.seq.bit_depth_2_qp_offset;
            lcu_qp = clip3(0, MAX_QUANT_BASE + off, lcu_qp);
            let adj = |delta: i32| -> i32 {
                let a = lcu_qp + delta - off;
                if a >= 0 { i32::from(crate::deblock::QP_CHROMA_ADJUST[a.min(MAX_QUANT_BASE) as usize]) } else { a }
            };
            let (adj_cb, adj_cr) = (adj(self.pichdr.chroma_quant_param_delta_cb), adj(self.pichdr.chroma_quant_param_delta_cr));
            self.lcu_qp_y = lcu_qp;
            self.lcu_qp_u = (adj_cb + off).max(0);
            self.lcu_qp_v = (adj_cr + off).max(0);
            self.maps.qp[lcu_idx] = lcu_qp as i8;

            if self.seq.sao_enable_flag {
                self.parse_sao_param(lcu_idx);
            }
            if self.seq.alf_enable_flag {
                for c in 0..3 {
                    self.alf_enable_map[lcu_idx][c] = self.pichdr.alf_param.alf_flag[c] && self.parse_alf_enable();
                }
            }
            let (px, py) = (self.lcu_pix_x, self.lcu_pix_y);
            self.decode_cu_tree(px, py, log2, log2, 0, 0, NO_MODE_CONS, TREE_LC)?;

            new_patch = self.lbac.bin_trm() != 0;
            left_lcu -= 1;
            if left_lcu == 0 {
                break;
            }
        }

        if !self.pichdr.loop_filter_disable_flag {
            self.deblock_picture();
        }
        if self.seq.sao_enable_flag {
            self.sao_picture();
        }
        if self.seq.alf_enable_flag && self.pichdr.alf_param.pic_alf_enable {
            self.alf_picture();
        }
        self.pic.map_mv = std::mem::take(&mut self.maps.mv);
        self.pic.map_refi = std::mem::take(&mut self.maps.refi);
        Ok(())
    }
}

/// A picture of the buffer.
struct Slot {
    pic: Arc<Picture>,
    doi: i64,
    ptr: i64,
    output_delay: i64,
    is_ref: bool,
    is_output: bool,
    tag: i64,
}

/// A picture ready for output.
pub(crate) struct OutPic {
    pub pic: Arc<Picture>,
    pub poc: i64,
    pub tag: i64,
}

/// The decoder state across pictures.
#[derive(Default)]
pub(crate) struct Ctx {
    pub seq: Option<crate::header::Sequence>,
    core: Option<Core>,
    dpb: Vec<Slot>,
    doi: DoiState,
    cur_doi: i64,
}

/// The start codes of `data`: the code byte and the offset of its `00 00 01`.
fn codes(data: &[u8]) -> Vec<(u8, usize)> {
    data.windows(4).enumerate().filter_map(|(i, w)| (w[0] == 0 && w[1] == 0 && w[2] == 1).then_some((w[3], i))).collect()
}

impl Ctx {
    pub(crate) fn reset(&mut self) {
        self.dpb.clear();
        self.doi = DoiState::default();
    }

    /// Decodes one packet (a sequence header, or a picture with all its patches).
    pub(crate) fn decode_packet(&mut self, packet: &[u8], tag: i64) -> Result<(), Error> {
        let cs = codes(packet);
        let Some(&(code, at)) = cs.first() else {
            return Ok(());
        };
        match code {
            0xB0 => self.decode_sequence(packet.get(at..).unwrap_or_default()),
            0xB1 | 0xB7 => Ok(()),
            0xB3 | 0xB6 => self.decode_picture(packet, &cs, tag),
            _ => Ok(()),
        }
    }

    fn decode_sequence(&mut self, data: &[u8]) -> Result<(), Error> {
        let d = demulate(data);
        let mut b = Bits::new(&d);
        let seq = crate::header::Sequence::parse(&mut b)?;
        parse_ext_and_user_data(&mut b, None);
        if let Some(old) = &self.seq {
            if old.horizontal_size != seq.horizontal_size
                || old.vertical_size != seq.vertical_size
                || old.log2_max_cu_width_height != seq.log2_max_cu_width_height
                || old.encoding_precision != seq.encoding_precision
            {
                return Err(Error::Unsupported("sequence parameters changed inside the stream"));
            }
            self.seq = Some(seq.clone());
            if let Some(c) = self.core.as_mut() {
                c.seq = seq;
            }
        } else {
            self.core = Some(Core::new(seq.clone()));
            self.seq = Some(seq);
        }
        Ok(())
    }

    fn decode_picture(&mut self, packet: &[u8], cs: &[(u8, usize)], tag: i64) -> Result<(), Error> {
        let seq = self.seq.clone().ok_or(Error::Invalid("picture before the sequence header"))?;
        // the header unit runs up to the first slice start code
        let first_slice = cs.iter().position(|&(c, _)| c <= 0x8F);
        let (_, hdr_at) = cs[0];
        let hdr_end = first_slice.map_or(packet.len(), |i| cs[i].1);
        let hdr = demulate(packet.get(hdr_at..hdr_end).unwrap_or_default());
        let mut b = Bits::new(&hdr);
        let mut doi = self.doi;
        let mut ph = PicHeader::parse(&mut b, &seq, &mut doi)?;
        parse_ext_and_user_data(&mut b, Some(&mut ph));
        if doi.wrapped {
            for s in &mut self.dpb {
                s.doi -= DOI_CYCLE_LENGTH;
            }
        }
        self.doi = doi;
        let cur_doi_raw = i64::from(ph.decode_order_index);

        // mark the reference pictures
        for s in &mut self.dpb {
            if s.is_ref {
                let keep = ph.rpl_l0.delta_doi[..ph.rpl_l0.num].iter().any(|&d| s.doi == cur_doi_raw - i64::from(d))
                    || ph.rpl_l1.delta_doi[..ph.rpl_l1.num].iter().any(|&d| s.doi == cur_doi_raw - i64::from(d));
                if !keep {
                    s.is_ref = false;
                }
            }
        }
        // the active reference lists
        let mut refp: Vec<[Option<RefPic>; 2]> = (0..MAX_REFS).map(|_| [None, None]).collect();
        let mut num_refp = [0usize; 2];
        for lidx in 0..2 {
            if lidx == 1 && ph.slice_type == SliceType::P {
                break;
            }
            let rpl = if lidx == 0 { &ph.rpl_l0 } else { &ph.rpl_l1 };
            for i in 0..rpl.active.min(MAX_REFS) {
                let want = cur_doi_raw - i64::from(rpl.delta_doi[i]);
                let slot = self.dpb.iter().find(|s| s.doi == want && s.is_ref).ok_or(Error::Invalid("missing reference picture"))?;
                let mut dist = (ph.ptr - slot.ptr) as i16;
                if dist == 0 {
                    dist = 1;
                }
                refp[i][lidx] = Some(RefPic { pic: Arc::clone(&slot.pic), dist: i32::from(dist) });
                num_refp[lidx] += 1;
            }
        }
        if ph.slice_type == SliceType::I {
            num_refp = [0; 2];
        }
        // free pictures that are neither references nor waiting for output
        self.dpb.retain(|s| s.is_ref || s.is_output);
        if self.dpb.len() >= MAX_DPB {
            return Err(Error::Invalid("decoded picture buffer overflow"));
        }

        // the slice units
        let mut units: Vec<Vec<u8>> = Vec::new();
        for (k, &(c, at)) in cs.iter().enumerate() {
            if c <= 0x8E {
                let end = cs.get(k + 1).map_or(packet.len(), |n| n.1);
                units.push(demulate(packet.get(at..end).unwrap_or_default()));
            }
        }
        let core = self.core.as_mut().ok_or(Error::Invalid("no decoder state"))?;
        let ptr = ph.ptr;
        let output_delay = i64::from(ph.output_delay);
        core.begin_picture(ph, refp, num_refp);
        core.decode_patches(&units)?;
        let pic = Arc::new(std::mem::take(&mut core.pic));
        core.refp.clear();
        self.dpb.push(Slot { pic, doi: cur_doi_raw, ptr, output_delay, is_ref: true, is_output: true, tag });
        self.cur_doi = cur_doi_raw;
        Ok(())
    }

    /// `com_picman_out_pic`: the next picture due for output (`flush`: bumping at the end of the stream).
    pub(crate) fn next_output(&mut self, flush: bool) -> Option<OutPic> {
        let mut best: Option<usize> = None;
        for (i, s) in self.dpb.iter().enumerate() {
            if !s.is_output {
                continue;
            }
            let eligible = flush || s.doi + s.output_delay <= self.cur_doi;
            if eligible && best.is_none_or(|b| self.dpb[b].ptr >= s.ptr) {
                best = Some(i);
            }
        }
        let i = best?;
        let s = &mut self.dpb[i];
        s.is_output = false;
        Some(OutPic { pic: Arc::clone(&s.pic), poc: s.ptr, tag: s.tag })
    }
}
