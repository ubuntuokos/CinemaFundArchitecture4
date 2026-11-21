// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/parser.c`, LBAC part), Copyright (c) 2018-2022 Peking University Shenzhen
// Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! The arithmetic decoder (LBAC) and its context models.

use crate::plps::TBL_PLPS;

const CABAC_BITS: i32 = 16;
const CABAC_MASK: i32 = (1 << CABAC_BITS) - 1;
const PROB_MASK: i32 = (1 << 11) - 1;
const HALF_RANGE: i32 = 256;
const MAX_RANGE: i32 = 511;

pub(crate) type Model = u16;

/// All context models (`com_lbac_all_ctx_t`).
#[derive(Clone)]
pub(crate) struct Ctx {
    pub skip_flag: [Model; 4],
    pub skip_idx: [Model; 11],
    pub direct_flag: [Model; 2],
    pub umve_flag: [Model; 1],
    pub umve_base_idx: [Model; 1],
    pub umve_step_idx: [Model; 1],
    pub umve_dir_idx: [Model; 2],
    pub inter_dir: [Model; 3],
    pub intra_dir: [Model; 10],
    pub pred_mode: [Model; 6],
    pub cons_mode: [Model; 1],
    pub ipf_flag: [Model; 1],
    pub refi: [Model; 3],
    pub mvr_idx: [Model; 4],
    pub affine_mvr_idx: [Model; 2],
    pub mvp_from_hmvp_flag: [Model; 1],
    pub mvd: [[Model; 3]; 2],
    pub ctp_zero_flag: [Model; 2],
    pub cbf: [Model; 3],
    pub tb_split: [Model; 1],
    pub run: [Model; 24],
    pub last1: [Model; 12],
    pub last2: [Model; 22],
    pub level: [Model; 24],
    pub split_flag: [Model; 4],
    pub bt_split_flag: [Model; 9],
    pub split_dir: [Model; 5],
    pub split_mode: [Model; 3],
    pub affine_flag: [Model; 1],
    pub affine_mrg_idx: [Model; 4],
    pub smvd_flag: [Model; 1],
    pub part_size: [Model; 6],
    pub sao_merge_flag: [Model; 3],
    pub sao_mode: [Model; 1],
    pub sao_offset: [Model; 1],
    pub alf_lcu_enable: [Model; 1],
    pub lcu_qp_delta: [Model; 4],
}

const INIT: Model = ((1 << 10) - 1) << 1;

impl Ctx {
    pub(crate) fn new() -> Self {
        Self {
            skip_flag: [INIT; 4],
            skip_idx: [INIT; 11],
            direct_flag: [INIT; 2],
            umve_flag: [INIT; 1],
            umve_base_idx: [INIT; 1],
            umve_step_idx: [INIT; 1],
            umve_dir_idx: [INIT; 2],
            inter_dir: [INIT; 3],
            intra_dir: [INIT; 10],
            pred_mode: [INIT; 6],
            cons_mode: [INIT; 1],
            ipf_flag: [INIT; 1],
            refi: [INIT; 3],
            mvr_idx: [INIT; 4],
            affine_mvr_idx: [INIT; 2],
            mvp_from_hmvp_flag: [INIT; 1],
            mvd: [[INIT; 3]; 2],
            ctp_zero_flag: [INIT; 2],
            cbf: [INIT; 3],
            tb_split: [INIT; 1],
            run: [INIT; 24],
            last1: [INIT; 12],
            last2: [INIT; 22],
            level: [INIT; 24],
            split_flag: [INIT; 4],
            bt_split_flag: [INIT; 9],
            split_dir: [INIT; 5],
            split_mode: [INIT; 3],
            affine_flag: [INIT; 1],
            affine_mrg_idx: [INIT; 4],
            smvd_flag: [INIT; 1],
            part_size: [INIT; 6],
            sao_merge_flag: [INIT; 3],
            sao_mode: [INIT; 1],
            sao_offset: [INIT; 1],
            alf_lcu_enable: [INIT; 1],
            lcu_qp_delta: [INIT; 4],
        }
    }
}

fn get_shift(v: i32) -> i32 {
    (v as u32).leading_zeros() as i32 - 23
}

pub(crate) fn get_log2(v: i32) -> i32 {
    31 - (v as u32).leading_zeros() as i32
}

fn next_model(range_lps: i32, s: i32) -> Model {
    let idx = 2048 + (range_lps & 0xf000) + s;
    TBL_PLPS.get(idx as usize).copied().unwrap_or(4096) as Model
}

/// The arithmetic decoder over one slice unit.
pub(crate) struct Lbac {
    pub range: i32,
    pub low: i32,
    data: Vec<u8>,
    pub cur: usize,
    pub end: usize,
}

impl Lbac {
    pub(crate) fn empty() -> Self {
        Self { range: MAX_RANGE, low: 0, data: Vec::new(), cur: 0, end: 0 }
    }

    /// Sets the data of a slice unit (it is padded so reads past the end are harmless).
    pub(crate) fn set_data(&mut self, data: &[u8]) {
        self.data.clear();
        self.data.extend_from_slice(data);
        self.end = data.len();
        self.data.extend_from_slice(&[0; 8]);
    }

    fn byte(&self, i: usize) -> i32 {
        i32::from(self.data.get(i).copied().unwrap_or(0))
    }

    /// `lbac_init` at `cur` (an offset into the unit).
    pub(crate) fn init_at(&mut self, cur: usize) {
        self.range = MAX_RANGE;
        self.cur = cur + 3;
        self.low = (self.byte(cur) << 18) | (self.byte(cur + 1) << 10) | (self.byte(cur + 2) << 2) | 2;
    }

    fn refill(&mut self, low: i32) -> i32 {
        let x = low ^ low.wrapping_sub(1);
        let i = 7 - get_shift(x >> 15);
        let x = -CABAC_MASK + (self.byte(self.cur) << 9) + (self.byte(self.cur + 1) << 1);
        self.cur = (self.cur + 2).min(self.end);
        low.wrapping_add(x << i)
    }

    #[inline]
    pub(crate) fn bin(&mut self, model: &mut Model) -> u32 {
        let range_lps = i32::from(*model);
        let mut s = range_lps & PROB_MASK;
        let mut r_mps = self.range - (s >> 3);
        let s_flag = i32::from(r_mps < HALF_RANGE);
        r_mps |= 0x100;
        let scaled = r_mps << (CABAC_BITS + 1 - s_flag);
        self.range = (self.range << s_flag) - r_mps;
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.range += (r_mps - self.range) & !lps_mask;
        s ^= lps_mask;
        *model = next_model(range_lps, s);
        let bin = (s & 1) as u32;
        let sh = get_shift(self.range);
        self.range <<= sh;
        self.low = self.low.wrapping_shl((sh + s_flag) as u32);
        if self.low & CABAC_MASK == 0 {
            self.low = self.refill(self.low);
        }
        bin
    }

    /// Two models combined (`lbac_dec_binW`).
    pub(crate) fn bin_w(&mut self, m1: &mut Model, m2: &mut Model) -> u32 {
        let range_lps1 = i32::from(*m1);
        let range_lps2 = i32::from(*m2);
        let mut s1 = range_lps1 & PROB_MASK;
        let mut s2 = range_lps2 & PROB_MASK;
        let prob_lps1 = (s1 >> 1) as u16;
        let prob_lps2 = (s2 >> 1) as u16;
        let cmps1 = *m1 & 1;
        let cmps2 = *m2 & 1;
        let (cmps, prob_lps);
        if cmps1 == cmps2 {
            cmps = cmps1;
            prob_lps = (prob_lps1 + prob_lps2) >> 1;
        } else if prob_lps1 < prob_lps2 {
            cmps = cmps1;
            prob_lps = (256u16 << 2) - 1 - ((prob_lps2 - prob_lps1) >> 1);
        } else {
            cmps = cmps2;
            prob_lps = (256u16 << 2) - 1 - ((prob_lps1 - prob_lps2) >> 1);
        }
        let mut r_mps = self.range - i32::from(prob_lps >> 2);
        let s_flag = i32::from(r_mps < HALF_RANGE);
        r_mps |= 0x100;
        let scaled = r_mps << (CABAC_BITS + 1 - s_flag);
        self.range = (self.range << s_flag) - r_mps;
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.range += (r_mps - self.range) & !lps_mask;
        let bin = cmps ^ (lps_mask & 1) as u16;
        s1 ^= if bin == cmps1 { 0 } else { -1 };
        s2 ^= if bin == cmps2 { 0 } else { -1 };
        *m1 = next_model(range_lps1, s1);
        *m2 = next_model(range_lps2, s2);
        let sh = get_shift(self.range);
        self.range <<= sh;
        self.low = self.low.wrapping_shl((sh + s_flag) as u32);
        if self.low & CABAC_MASK == 0 {
            self.low = self.refill(self.low);
        }
        u32::from(bin)
    }

    /// An equiprobable bin; returns 1 or 0 (`lbac_dec_bin_ep() & 1`).
    #[inline]
    pub(crate) fn bin_ep(&mut self) -> u32 {
        let scaled = self.range << CABAC_BITS;
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.low = self.low.wrapping_shl(1);
        if self.low & CABAC_MASK == 0 {
            self.low = self.refill(self.low);
        }
        (lps_mask & 1) as u32
    }

    pub(crate) fn bin_trm(&mut self) -> u32 {
        let mut r_mps = self.range - 1;
        let s_flag = i32::from(r_mps < HALF_RANGE);
        r_mps |= 0x100;
        let scaled = r_mps << (CABAC_BITS + 1 - s_flag);
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        let bin = (lps_mask & 1) as u32;
        self.range = (self.range << s_flag) - r_mps;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.range += (r_mps - self.range) & !lps_mask;
        let sh = get_shift(self.range);
        self.range <<= sh;
        self.low = self.low.wrapping_shl((sh + s_flag) as u32);
        if self.low & CABAC_MASK == 0 {
            self.low = self.refill(self.low);
        }
        bin
    }

    /// `lbac_read_bins_ep_msb`: `n` equiprobable bins, first bin most significant.
    pub(crate) fn bins_ep_msb(&mut self, n: u32) -> u32 {
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | self.bin_ep();
        }
        val
    }

    /// `lbac_read_bins_ep_msb_ext`: first bin least significant.
    pub(crate) fn bins_ep_lsb(&mut self, n: u32) -> u32 {
        let mut val = 0u32;
        for i in 0..n {
            val |= self.bin_ep() << i;
        }
        val
    }

    pub(crate) fn unary_ep(&mut self) -> u32 {
        // the C code counts the 0 bins (`bin = !lps_mask`)
        let mut val = 0;
        loop {
            let bin = self.bin_ep() ^ 1;
            val += bin;
            if bin == 0 || self.cur >= self.end {
                break;
            }
        }
        val
    }

    pub(crate) fn truncate_unary_ep(&mut self, max_num: u32) -> u32 {
        let mut val = 0;
        loop {
            let bin = self.bin_ep() ^ 1;
            val += bin;
            if !(val + 1 < max_num && bin != 0) {
                break;
            }
        }
        val
    }

    /// `lbac_read_bins_msb`: bins with consecutive context models.
    pub(crate) fn bins_msb(&mut self, models: &mut [Model], n: u32) -> u32 {
        let mut val = 0;
        for m in models.iter_mut().take(n as usize) {
            val = (val << 1) | self.bin(m);
        }
        val
    }

    pub(crate) fn truncate_unary(&mut self, models: &mut [Model], num_ctx: u32, max_num: u32) -> u32 {
        let mut val = 0u32;
        loop {
            let idx = val.min(num_ctx - 1) as usize;
            let bin = self.bin(&mut models[idx]);
            val += bin ^ 1;
            // note: the C code reads the inverted bin (`bin = !(s & 1)`), so a decoded 0 continues
            if !(val + 1 < max_num && (bin ^ 1) != 0) {
                break;
            }
        }
        val
    }

    /// `dec_parse_ipcm_start`: positions `cur` at the first byte of the PCM samples.
    pub(crate) fn ipcm_start(&mut self) {
        let low = self.low;
        let x = low ^ low.wrapping_sub(1);
        let mut flag_idx = get_log2(x);
        while flag_idx <= 8 {
            self.cur = self.cur.saturating_sub(1);
            flag_idx += 8;
        }
    }

    /// `dec_parse_ipcm`: `bits` raw bits; `bit_left` tracks the bits left in the current byte.
    pub(crate) fn ipcm(&mut self, bit_left: &mut i32, bits: i32) -> i32 {
        let mut bits = bits;
        let t1 = self.byte(self.cur) & ((1 << *bit_left) - 1);
        bits -= *bit_left;
        *bit_left = 8 - bits;
        self.cur = (self.cur + 1).min(self.end);
        if bits != 0 {
            let t2 = self.byte(self.cur) >> *bit_left;
            if *bit_left == 0 {
                self.cur = (self.cur + 1).min(self.end);
                *bit_left = 8;
            }
            (t1 << bits) | t2
        } else {
            t1
        }
    }
}
