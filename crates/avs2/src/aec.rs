// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/aec.cc`), Copyright (C) 2018~ VCL, NELVT, Peking University
// (Falei Luo et al.), GPL-2.0-or-later.
//! The AVS2 arithmetic decoder (AEC): the binary decoding engine and its contexts. The syntax
//! elements read with it are in `cu.rs`.

/// One adaptive context: `LG_PMPS` (11 bits), `MPS`, `cycno` (2 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ctx {
    lg_pmps: u16,
    mps: u8,
    cycno: u8,
}

const LG_PMPS_SHIFTNO: u32 = 2;
const B_BITS: u32 = 10;
const QUARTER: u32 = 1 << (B_BITS - 2);
const AEC_VALUE_BOUND: u32 = 254;
const TAB_CWR: [u32; 7] = [3, 3, 4, 5, 5, 5, 5];
const TAB_LG_PMPS_OFFSET: [u32; 6] = [0, 0, 0, 197, 95, 46];

impl Ctx {
    const INIT: Ctx = Ctx { lg_pmps: ((QUARTER << LG_PMPS_SHIFTNO) - 1) as u16, mps: 0, cycno: 0 };

    fn update_mps(&mut self) {
        let cwr = TAB_CWR[usize::from(self.cycno & 3)];
        if self.cycno == 0 {
            self.cycno = 1;
        }
        let l = u32::from(self.lg_pmps);
        self.lg_pmps = (l - ((l >> cwr) + (l >> (cwr + 2)))) as u16;
    }

    fn update_lps(&mut self) {
        let mut cycno = u32::from(self.cycno & 3);
        let cwr = TAB_CWR[cycno as usize];
        let mut lg_pmps = u32::from(self.lg_pmps) + TAB_LG_PMPS_OFFSET.get(cwr as usize).copied().unwrap_or(0);
        let mut mps = self.mps;
        if cycno != 3 {
            cycno += 1;
        }
        if lg_pmps >= (256 << LG_PMPS_SHIFTNO) {
            lg_pmps = (512 << LG_PMPS_SHIFTNO) - 1 - lg_pmps;
            mps ^= 1;
        }
        // MAKE_CONTEXT keeps 11 bits of LG_PMPS in the 16-bit word
        *self = Ctx { lg_pmps: (lg_pmps & 0x7FF) as u16, mps, cycno: cycno as u8 };
    }
}

// Offsets of the context groups in the flat context array (`context_set_t`, in field order).
pub(crate) const CTX_CU_TYPE: usize = 0;
pub(crate) const CTX_INTRA_PU_TYPE: usize = 6;
pub(crate) const CTX_CU_SPLIT: usize = 7;
pub(crate) const CTX_TU_SPLIT: usize = 10;
pub(crate) const CTX_AMP: usize = 13;
pub(crate) const CTX_REF_IDX: usize = 15;
pub(crate) const CTX_CBP: usize = 18;
pub(crate) const CTX_MVD: usize = 27;
pub(crate) const CTX_PU_TYPE_INDEX: usize = 33;
pub(crate) const CTX_B_PU_TYPE_MIN: usize = 48;
pub(crate) const CTX_CU_SUBTYPE: usize = 50;
pub(crate) const CTX_WPM: usize = 54;
pub(crate) const CTX_DELTA_QP: usize = 57;
pub(crate) const CTX_INTRA_LUMA: usize = 61;
pub(crate) const CTX_INTRA_CHROMA: usize = 68;
/// `coeff_run[2][3][11]`.
pub(crate) const CTX_COEFF_RUN: usize = 71;
pub(crate) const CTX_COEFF_LEVEL: usize = 137;
pub(crate) const CTX_LAST_CG: usize = 177;
pub(crate) const CTX_SIG_CG: usize = 189;
pub(crate) const CTX_LAST_POS: usize = 192;
pub(crate) const CTX_SAO_MERGE: usize = 252;
pub(crate) const CTX_SAO_MODE: usize = 255;
pub(crate) const CTX_SAO_OFFSET: usize = 256;
pub(crate) const CTX_ALF_LCU: usize = 258;
pub(crate) const NUM_CTX: usize = 270;

/// The arithmetic decoder state (`aec_t`).
pub(crate) struct Aec<'a> {
    buf: &'a [u8],
    byte_buf: u64,
    byte_pos: usize,
    bits_to_go: i32,
    /// Set when the decoder ran out of data or saw an impossible value; reads then return 0.
    pub bit_error: bool,
    val_bound: bool,
    val_domain: bool,
    s1: u32,
    t1: u32,
    value_s: u32,
    value_t: u32,
    pub ctx: [Ctx; NUM_CTX],
}

impl<'a> Aec<'a> {
    /// `aec_start_decoding` + `aec_init_contexts`: decoding starts at byte `byte_pos` of `buf`.
    pub(crate) fn new(buf: &'a [u8], byte_pos: usize) -> Self {
        let mut a = Aec {
            buf,
            byte_buf: 0,
            byte_pos,
            bits_to_go: 0,
            bit_error: false,
            val_bound: false,
            val_domain: true,
            s1: 0,
            t1: QUARTER - 1,
            value_s: 0,
            value_t: 0,
            ctx: [Ctx::INIT; NUM_CTX],
        };
        a.next_n_bits(B_BITS - 1);
        a
    }

    /// `aec_init_contexts`.
    pub(crate) fn init_contexts(&mut self) {
        self.ctx = [Ctx::INIT; NUM_CTX];
    }

    /// `aec_bits_read`.
    pub(crate) fn bits_read(&self) -> i64 {
        (self.byte_pos as i64) * 8 - i64::from(self.bits_to_go)
    }

    /// `aec_get_next_bit`: true on error.
    fn next_bit(&mut self) -> bool {
        self.bits_to_go -= 1;
        if self.bits_to_go < 0 {
            let diff = self.buf.len().saturating_sub(self.byte_pos);
            let p = self.buf.get(self.byte_pos..).unwrap_or(&[]);
            if diff > 7 {
                self.byte_buf = p.iter().take(8).fold(0u64, |a, &b| (a << 8) | u64::from(b));
                self.bits_to_go = 63;
                self.byte_pos += 8;
            } else if diff > 0 {
                self.bits_to_go += (diff as i32) << 3;
                self.byte_pos += ((self.bits_to_go + 1) >> 3) as usize;
                self.byte_buf = p.iter().take(diff).fold(0u64, |a, &b| (a << 8) | u64::from(b));
            } else {
                self.bit_error = true;
                return true;
            }
        }
        let next = ((self.byte_buf >> self.bits_to_go) & 1) as u32;
        self.value_t = (self.value_t << 1) | next;
        false
    }

    /// `aec_get_next_n_bit`: true on error.
    fn next_n_bits(&mut self, n: u32) -> bool {
        if self.bits_to_go >= n as i32 {
            self.bits_to_go -= n as i32;
            let next = ((self.byte_buf >> self.bits_to_go) & ((1u64 << n) - 1)) as u32;
            self.value_t = (self.value_t << n) | next;
            false
        } else {
            for _ in 0..n {
                self.next_bit();
            }
            self.bit_error
        }
    }

    /// The renormalisation that starts each decision when the value left the R domain.
    /// Returns the new `value_s` or `None` on error.
    fn refill(&mut self, mut value_s: u32) -> Option<u32> {
        if self.val_domain || (self.s1 == AEC_VALUE_BOUND && self.val_bound) {
            value_s = 0;
            self.s1 = 0;
            while self.value_t < QUARTER && value_s < AEC_VALUE_BOUND {
                if self.next_bit() {
                    return None;
                }
                value_s += 1;
            }
            self.val_bound = self.value_t < QUARTER;
            self.value_t &= 0xff;
        }
        Some(value_s)
    }

    /// One decision with context `c` given the current `value_s`: (bit, new value_s), or `None`
    /// on error.
    fn decide(&mut self, c: usize, value_s: u32) -> Option<u32> {
        let ctx = self.ctx.get(c).copied().unwrap_or(Ctx::INIT);
        let lg_pmps = u32::from(ctx.lg_pmps) >> LG_PMPS_SHIFTNO;
        let mut bit = u32::from(ctx.mps);
        let s_flag = u32::from(self.t1 < lg_pmps);
        let s2 = self.s1 + s_flag;
        let t2 = self.t1.wrapping_sub(lg_pmps).wrapping_add(s_flag << 8);
        let is_lps = (s2 > value_s || (s2 == value_s && self.value_t >= t2)) && !self.val_bound;
        self.val_domain = is_lps;
        if is_lps {
            let mut t_rlps = if s_flag == 0 { lg_pmps } else { self.t1 + lg_pmps };
            bit ^= 1;
            if s2 == value_s {
                self.value_t = self.value_t.wrapping_sub(t2);
            } else {
                if self.next_bit() {
                    return None;
                }
                self.value_t = self.value_t.wrapping_add(256u32.wrapping_sub(t2));
            }
            let mut n_bits = 0;
            while t_rlps < QUARTER {
                t_rlps <<= 1;
                n_bits += 1;
            }
            if n_bits > 0 && self.next_n_bits(n_bits) {
                return None;
            }
            self.s1 = 0;
            self.t1 = t_rlps & 0xff;
            if let Some(x) = self.ctx.get_mut(c) {
                x.update_lps();
            }
        } else {
            self.s1 = s2;
            self.t1 = t2;
            if let Some(x) = self.ctx.get_mut(c) {
                x.update_mps();
            }
        }
        Some(bit)
    }

    /// `biari_decode_symbol`.
    pub(crate) fn symbol(&mut self, c: usize) -> u32 {
        let Some(value_s) = self.refill(self.value_s) else { return 0 };
        if self.value_s > AEC_VALUE_BOUND {
            self.bit_error = true;
            self.value_s = value_s;
            return 0;
        }
        let Some(bit) = self.decide(c, value_s) else { return 0 };
        self.value_s = value_s;
        bit
    }

    /// `biari_decode_symbol_eq_prob`.
    pub(crate) fn eq_prob(&mut self) -> u32 {
        if self.val_domain || (self.s1 == AEC_VALUE_BOUND && self.val_bound) {
            self.s1 = 0;
            if self.next_bit() {
                return 0;
            }
            if self.value_t >= 256 + self.t1 {
                self.value_t -= 256 + self.t1;
                1
            } else {
                0
            }
        } else {
            let s2 = self.s1 + 1;
            let t2 = self.t1;
            let is_lps = s2 > self.value_s || ((s2 == self.value_s && self.value_t >= t2) && !self.val_bound);
            self.val_domain = is_lps;
            if is_lps {
                if s2 == self.value_s {
                    self.value_t = self.value_t.wrapping_sub(t2);
                } else {
                    if self.next_bit() {
                        return 0;
                    }
                    self.value_t = self.value_t.wrapping_add(256u32.wrapping_sub(t2));
                }
                1
            } else {
                self.s1 = s2;
                self.t1 = t2;
                0
            }
        }
    }

    /// `biari_decode_final`.
    pub(crate) fn final_bit(&mut self) -> u32 {
        let lg_pmps = 1u32;
        if self.val_domain || (self.s1 == AEC_VALUE_BOUND && self.val_bound) {
            self.s1 = 0;
            self.value_s = 0;
            while self.value_t < QUARTER && self.value_s < AEC_VALUE_BOUND {
                if self.next_bit() {
                    return 0;
                }
                self.value_s += 1;
            }
            self.val_bound = self.value_t < QUARTER;
            self.value_t &= 0xff;
        }
        let s_flag = u32::from(self.t1 < lg_pmps);
        let s2 = self.s1 + s_flag;
        let t2 = self.t1.wrapping_sub(lg_pmps).wrapping_add(s_flag << 8);
        let is_lps = (s2 > self.value_s || (s2 == self.value_s && self.value_t >= t2)) && !self.val_bound;
        self.val_domain = is_lps;
        if is_lps {
            if s2 == self.value_s {
                self.value_t = self.value_t.wrapping_sub(t2);
            } else {
                if self.next_bit() {
                    return 0;
                }
                self.value_t = self.value_t.wrapping_add(256u32.wrapping_sub(t2));
            }
            // t_rlps = 1 shifted up to QUARTER: 8 bits
            if self.next_n_bits(8) {
                return 0;
            }
            self.s1 = 0;
            self.t1 = 0;
        } else {
            self.s1 = s2;
            self.t1 = t2;
        }
        u32::from(is_lps)
    }

    /// `biari_decode_symbol_continue0`: a run of decisions with one context until a 1 or
    /// `max_num` decisions; returns the number of zeros.
    pub(crate) fn continue0(&mut self, c: usize, max_num: u32) -> u32 {
        let mut value_s = self.value_s;
        let mut bit = 0;
        let mut i = 0;
        while i < max_num && bit == 0 {
            let Some(v) = self.refill(value_s) else { return 0 };
            value_s = v;
            if value_s > AEC_VALUE_BOUND {
                self.bit_error = true;
                return 0;
            }
            let Some(b) = self.decide(c, value_s) else { return 0 };
            bit = b;
            i += 1;
        }
        self.value_s = value_s;
        i - bit
    }

    /// `biari_decode_symbol_continu0_ext`: like [`Aec::continue0`], the context index growing
    /// with the decision number up to `c + max_ctx_inc`.
    pub(crate) fn continue0_ext(&mut self, c: usize, max_ctx_inc: u32, max_num: u32) -> u32 {
        let mut bit = 0;
        let mut i = 0;
        while i < max_num && bit == 0 {
            let cc = c + i.min(max_ctx_inc) as usize;
            let Some(v) = self.refill(self.value_s) else { return 0 };
            self.value_s = v;
            if self.value_s > AEC_VALUE_BOUND {
                self.bit_error = true;
                return 0;
            }
            // davs2 reads the renormalisation bits one at a time here; the result is the same
            let Some(b) = self.decide(cc, self.value_s) else { return 0 };
            bit = b;
            i += 1;
        }
        i - bit
    }

    /// `unary_bin_max_decode`.
    pub(crate) fn unary_max(&mut self, c: usize, ctx_offset: usize, max_symbol: u32) -> u32 {
        let symbol = self.symbol(c);
        if symbol == 1 {
            0
        } else if max_symbol == 1 {
            symbol
        } else {
            1 + self.continue0(c + ctx_offset, max_symbol - 1)
        }
    }
}
