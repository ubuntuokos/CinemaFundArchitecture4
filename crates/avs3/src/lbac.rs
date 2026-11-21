// SPDX-License-Identifier: GPL-3.0-or-later
//! LBAC arithmetic decoder and the context-model set.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md).

use crate::tables::PLPS;

const CABAC_MASK: i32 = 0xffff;
const PROB_MASK: i32 = (1 << 11) - 1;
const MAX_RANGE: i32 = 511;
const INIT_MODEL: u16 = ((1 << 10) - 1) << 1;

/// All context models of one slice/patch. The array sizes follow `com_lbac_all_ctx_t`.
#[derive(Clone)]
pub struct Ctx {
    pub skip_flag: [u16; 4],
    pub skip_idx: [u16; 11],
    pub direct_flag: [u16; 2],
    pub umve_flag: u16,
    pub umve_base_idx: [u16; 1],
    pub umve_step_idx: [u16; 1],
    pub umve_dir_idx: [u16; 2],
    pub inter_dir: [u16; 3],
    pub intra_dir: [u16; 10],
    pub pred_mode: [u16; 6],
    pub cons_mode: [u16; 1],
    pub ipf_flag: [u16; 1],
    pub refi: [u16; 3],
    pub mvr_idx: [u16; 4],
    pub affine_mvr_idx: [u16; 2],
    pub mvp_from_hmvp_flag: [u16; 1],
    pub mvd: [[u16; 3]; 2],
    pub ctp_zero_flag: [u16; 2],
    pub cbf: [u16; 3],
    pub tb_split: [u16; 1],
    pub run: [u16; 24],
    pub last1: [u16; 12],
    pub last2: [u16; 22],
    pub level: [u16; 24],
    pub split_flag: [u16; 4],
    pub bt_split_flag: [u16; 9],
    pub split_dir: [u16; 5],
    pub split_mode: [u16; 3],
    pub affine_flag: [u16; 1],
    pub affine_mrg_idx: [u16; 4],
    pub smvd_flag: [u16; 1],
    pub part_size: [u16; 6],
    pub sao_merge_flag: [u16; 3],
    pub sao_mode: [u16; 1],
    pub sao_offset: [u16; 1],
    pub alf_lcu_enable: [u16; 1],
    pub lcu_qp_delta: [u16; 4],
}

impl Ctx {
    pub fn new() -> Ctx {
        Ctx {
            skip_flag: [INIT_MODEL; 4],
            skip_idx: [INIT_MODEL; 11],
            direct_flag: [INIT_MODEL; 2],
            umve_flag: INIT_MODEL,
            umve_base_idx: [INIT_MODEL; 1],
            umve_step_idx: [INIT_MODEL; 1],
            umve_dir_idx: [INIT_MODEL; 2],
            inter_dir: [INIT_MODEL; 3],
            intra_dir: [INIT_MODEL; 10],
            pred_mode: [INIT_MODEL; 6],
            cons_mode: [INIT_MODEL; 1],
            ipf_flag: [INIT_MODEL; 1],
            refi: [INIT_MODEL; 3],
            mvr_idx: [INIT_MODEL; 4],
            affine_mvr_idx: [INIT_MODEL; 2],
            mvp_from_hmvp_flag: [INIT_MODEL; 1],
            mvd: [[INIT_MODEL; 3]; 2],
            ctp_zero_flag: [INIT_MODEL; 2],
            cbf: [INIT_MODEL; 3],
            tb_split: [INIT_MODEL; 1],
            run: [INIT_MODEL; 24],
            last1: [INIT_MODEL; 12],
            last2: [INIT_MODEL; 22],
            level: [INIT_MODEL; 24],
            split_flag: [INIT_MODEL; 4],
            bt_split_flag: [INIT_MODEL; 9],
            split_dir: [INIT_MODEL; 5],
            split_mode: [INIT_MODEL; 3],
            affine_flag: [INIT_MODEL; 1],
            affine_mrg_idx: [INIT_MODEL; 4],
            smvd_flag: [INIT_MODEL; 1],
            part_size: [INIT_MODEL; 6],
            sao_merge_flag: [INIT_MODEL; 3],
            sao_mode: [INIT_MODEL; 1],
            sao_offset: [INIT_MODEL; 1],
            alf_lcu_enable: [INIT_MODEL; 1],
            lcu_qp_delta: [INIT_MODEL; 4],
        }
    }
}

impl Default for Ctx {
    fn default() -> Self {
        Ctx::new()
    }
}

#[inline]
fn get_shift(v: i32) -> i32 {
    (v as u32).leading_zeros() as i32 - 23
}

#[inline]
fn next_model(old: u16, s: i32) -> u16 {
    let idx = 2048i32.wrapping_add(i32::from(old & 0xf000)).wrapping_add(s);
    usize::try_from(idx).ok().and_then(|i| PLPS.get(i)).map_or(INIT_MODEL, |&v| v as u16)
}

/// The arithmetic decoder over one patch payload (emulation-prevention already removed).
pub struct Lbac {
    pub range: i32,
    pub low: i32,
    data: Vec<u8>,
    cur: usize,
}

impl Lbac {
    pub fn new(data: Vec<u8>) -> Lbac {
        let b = |i: usize| i32::from(data.get(i).copied().unwrap_or(0));
        let low = (b(0) << 18) | (b(1) << 10) | (b(2) << 2) | 2;
        let cur = 3.min(data.len());
        Lbac { range: MAX_RANGE, low, data, cur }
    }

    /// True once every payload byte has been consumed.
    #[inline]
    pub fn at_end(&self) -> bool {
        self.cur >= self.data.len()
    }

    #[inline]
    fn refill(&mut self, low: i32) -> i32 {
        let x = low ^ low.wrapping_sub(1);
        let i = (7 - get_shift(x >> 15)).clamp(0, 31);
        let b0 = i32::from(self.data.get(self.cur).copied().unwrap_or(0));
        let b1 = i32::from(self.data.get(self.cur + 1).copied().unwrap_or(0));
        let v = -CABAC_MASK + (b0 << 9) + (b1 << 1);
        self.cur = (self.cur + 2).min(self.data.len());
        low.wrapping_add(v << i)
    }

    #[inline]
    fn renorm(&mut self, s_flag: i32) {
        let sh = get_shift(self.range).clamp(0, 31);
        self.range <<= sh;
        self.low = self.low.wrapping_shl((sh + s_flag) as u32);
        if self.low & CABAC_MASK == 0 {
            self.low = self.refill(self.low);
        }
    }

    pub fn bin(&mut self, model: &mut u16) -> u32 {
        let range_lps = i32::from(*model);
        let mut s = range_lps & PROB_MASK;
        let mut rmps = self.range - (s >> 3);
        let s_flag = i32::from(rmps < 256);
        rmps |= 0x100;
        let scaled = rmps << (17 - s_flag);
        self.range = (self.range << s_flag).wrapping_sub(rmps);
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.range = self.range.wrapping_add((rmps.wrapping_sub(self.range)) & !lps_mask);
        s ^= lps_mask;
        *model = next_model(*model, s);
        let bin = (s & 1) as u32;
        self.renorm(s_flag);
        bin
    }

    /// Decode with the mix of two models (`lbac_dec_binW`).
    pub fn bin_w(&mut self, m1: &mut u16, m2: &mut u16) -> u32 {
        let mut s1 = i32::from(*m1) & PROB_MASK;
        let mut s2 = i32::from(*m2) & PROB_MASK;
        let prob_lps1 = s1 >> 1;
        let prob_lps2 = s2 >> 1;
        let cmps1 = i32::from(*m1) & 1;
        let cmps2 = i32::from(*m2) & 1;
        let (cmps, prob_lps);
        if cmps1 == cmps2 {
            cmps = cmps1;
            prob_lps = (prob_lps1 + prob_lps2) >> 1;
        } else if prob_lps1 < prob_lps2 {
            cmps = cmps1;
            prob_lps = (256 << 2) - 1 - ((prob_lps2 - prob_lps1) >> 1);
        } else {
            cmps = cmps2;
            prob_lps = (256 << 2) - 1 - ((prob_lps1 - prob_lps2) >> 1);
        }
        let mut rmps = self.range - (prob_lps >> 2);
        let s_flag = i32::from(rmps < 256);
        rmps |= 0x100;
        let scaled = rmps << (17 - s_flag);
        self.range = (self.range << s_flag).wrapping_sub(rmps);
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.range = self.range.wrapping_add((rmps.wrapping_sub(self.range)) & !lps_mask);
        let bin = cmps ^ (lps_mask & 1);
        s1 ^= if bin == cmps1 { 0 } else { -1 };
        s2 ^= if bin == cmps2 { 0 } else { -1 };
        *m1 = next_model(*m1, s1);
        *m2 = next_model(*m2, s2);
        self.renorm(s_flag);
        bin as u32
    }

    /// Equiprobable (bypass) bin.
    #[inline]
    pub fn bin_ep(&mut self) -> u32 {
        let scaled = self.range << 16;
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.low = self.low.wrapping_shl(1);
        if self.low & CABAC_MASK == 0 {
            self.low = self.refill(self.low);
        }
        (lps_mask & 1) as u32
    }

    /// Terminating bin.
    pub fn bin_trm(&mut self) -> u32 {
        let mut rmps = self.range - 1;
        let s_flag = i32::from(rmps < 256);
        rmps |= 0x100;
        let scaled = rmps << (17 - s_flag);
        let lps_mask = scaled.wrapping_sub(self.low) >> 31;
        let bin = (lps_mask & 1) as u32;
        self.range = (self.range << s_flag).wrapping_sub(rmps);
        self.low = self.low.wrapping_sub(scaled & lps_mask);
        self.range = self.range.wrapping_add((rmps.wrapping_sub(self.range)) & !lps_mask);
        self.renorm(s_flag);
        bin
    }

    /// `num_bin` bypass bins, MSB first.
    pub fn bins_ep_msb(&mut self, num_bin: u32) -> u32 {
        let mut val = 0u32;
        for _ in 0..num_bin.min(32) {
            val = (val << 1) | self.bin_ep();
        }
        val
    }

    /// `num_bin` bypass bins, LSB first.
    pub fn bins_ep_lsb(&mut self, num_bin: u32) -> u32 {
        let mut val = 0u32;
        for i in 0..num_bin.min(32) {
            val |= self.bin_ep() << i;
        }
        val
    }

    /// Unary code of bypass bins; stops at the end of the payload.
    pub fn unary_ep(&mut self) -> u32 {
        let mut val = 0u32;
        loop {
            let bin = self.bin_ep() ^ 1;
            val += bin;
            if bin == 0 || self.at_end() || val > 64 {
                break;
            }
        }
        val
    }

    /// Truncated unary code of bypass bins with maximum `max_num`.
    pub fn truncate_unary_ep(&mut self, max_num: u32) -> u32 {
        let mut val = 0u32;
        loop {
            let bin = self.bin_ep() ^ 1;
            val += bin;
            if !(val + 1 < max_num && bin != 0) {
                break;
            }
        }
        val
    }

    /// `num_bin` context-coded bins with consecutive models, MSB first.
    pub fn bins_msb(&mut self, models: &mut [u16], num_bin: usize) -> u32 {
        let mut val = 0u32;
        for m in models.iter_mut().take(num_bin) {
            val = (val << 1) | self.bin(m);
        }
        val
    }

    /// Truncated unary code over contexts `model[min(i, num_ctx-1)]`.
    pub fn truncate_unary(&mut self, models: &mut [u16], num_ctx: usize, max_num: u32) -> u32 {
        let mut val = 0u32;
        loop {
            let idx = (val as usize).min(num_ctx.saturating_sub(1));
            let bin = match models.get_mut(idx) {
                Some(m) => self.bin(m) ^ 1,
                None => 0,
            };
            val += bin;
            if !(val + 1 < max_num && bin != 0) {
                break;
            }
        }
        val
    }

    /// Position the byte reader at the first IPCM byte (`dec_parse_ipcm_start`).
    pub fn ipcm_start(&mut self) {
        let x = self.low ^ self.low.wrapping_sub(1);
        let mut flag_idx = 31 - (x as u32).leading_zeros() as i32;
        while flag_idx <= 8 {
            self.cur = self.cur.saturating_sub(1);
            flag_idx += 8;
        }
    }

    /// Read `bits` raw bits of an IPCM sample (`dec_parse_ipcm`).
    pub fn ipcm_read(&mut self, bit_left: &mut i32, mut bits: i32) -> i32 {
        let byte = |s: &Lbac| i32::from(s.data.get(s.cur).copied().unwrap_or(0));
        let t1 = byte(self) & ((1 << *bit_left) - 1);
        bits -= *bit_left;
        *bit_left = 8 - bits;
        self.cur = (self.cur + 1).min(self.data.len());
        if bits > 0 {
            let t2 = byte(self) >> *bit_left;
            if *bit_left == 0 {
                self.cur = (self.cur + 1).min(self.data.len());
                *bit_left = 8;
            }
            (t1 << bits) | t2
        } else {
            t1
        }
    }

    /// Restart the arithmetic decoder after IPCM samples (bytes already consumed).
    pub fn restart(&mut self) {
        let b = |s: &Lbac, i: usize| i32::from(s.data.get(s.cur + i).copied().unwrap_or(0));
        self.range = MAX_RANGE;
        self.low = (b(self, 0) << 18) | (b(self, 1) << 10) | (b(self, 2) << 2) | 2;
        self.cur = (self.cur + 3).min(self.data.len());
    }
}
