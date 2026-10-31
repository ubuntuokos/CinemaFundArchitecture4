//! CABAC slice data and macroblock layer parsing (7.3.4, 7.3.5 with 9.3 binarisations and context
//! index derivations).

use crate::cabac::Cabac;
use crate::error::{Result, ensure, invalid};
use crate::mbtypes::*;
use crate::picture::MbKind;
use crate::slice::SliceType;
use crate::slicedec::{MbCur, SliceDecoder, nnz_of, nnz_of_mut};

/// Table 9-43: significant_coeff_flag ctxIdxInc for 8x8 frame blocks.
#[rustfmt::skip]
const SIG8_FRAME: [u8; 63] = [
    0, 1, 2, 3, 4, 5, 5, 4, 4, 3, 3, 4, 4, 4, 5, 5, 4, 4, 4, 4, 3, 3, 6, 7, 7, 7, 8, 9, 10, 9, 8, 7,
    7, 6, 11, 12, 13, 11, 6, 7, 8, 9, 14, 10, 9, 8, 6, 11, 12, 13, 11, 6, 9, 14, 10, 9, 11, 12, 13, 11, 14, 10, 12,
];
/// Table 9-43: significant_coeff_flag ctxIdxInc for 8x8 field blocks.
#[rustfmt::skip]
const SIG8_FIELD: [u8; 63] = [
    0, 1, 1, 2, 2, 3, 3, 4, 5, 6, 7, 7, 7, 8, 4, 5, 6, 9, 10, 10, 8, 11, 12, 11, 9, 9, 10, 10, 8, 11, 12, 11,
    9, 9, 10, 10, 8, 11, 12, 11, 9, 9, 10, 10, 8, 13, 13, 9, 9, 10, 10, 8, 13, 13, 9, 9, 10, 10, 14, 14, 14, 14, 14,
];
/// Table 9-43: last_significant_coeff_flag ctxIdxInc for 8x8 blocks.
#[rustfmt::skip]
const LAST8: [u8; 63] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8,
];

/// ctxIdxInc = levelListIdx (ctxBlockCat 0, 1, 2, 4).
const IDENT_CTX: [u8; 64] = {
    let mut t = [0u8; 64];
    let mut i = 0;
    while i < 64 {
        t[i] = i as u8;
        i += 1;
    }
    t
};
/// ctxIdxInc = Min(levelListIdx / NumC8x8, 2) for chroma DC (ctxBlockCat 3): 4:2:0 (NumC8x8 = 1)
/// and 4:2:2 (NumC8x8 = 2).
const CHROMA_DC_CTX: [u8; 4] = [0, 1, 2, 2];
const CHROMA_DC_CTX_422: [u8; 8] = [0, 0, 1, 1, 2, 2, 2, 2];

/// ctxIdx bases (coded_block_flag, significant_coeff_flag, last_significant_coeff_flag,
/// coeff_abs_level_minus1) of frame coded blocks per ctxBlockCat 0..=13: the ctxIdxOffset of
/// Table 9-34 plus the ctxIdxBlockCatOffset of Table 9-40.
const CAT_CTX: [(usize, usize, usize, usize); 14] = [
    (85, 105, 166, 227),
    (85 + 4, 105 + 15, 166 + 15, 227 + 10),
    (85 + 8, 105 + 29, 166 + 29, 227 + 20),
    (85 + 12, 105 + 44, 166 + 44, 227 + 30),
    (85 + 16, 105 + 47, 166 + 47, 227 + 39),
    (1012, 402, 417, 426),
    (460, 484, 572, 952),
    (460 + 4, 484 + 15, 572 + 15, 952 + 10),
    (460 + 8, 484 + 29, 572 + 29, 952 + 20),
    (1012 + 4, 660, 690, 708),
    (472, 528, 616, 982),
    (472 + 4, 528 + 15, 616 + 15, 982 + 10),
    (472 + 8, 528 + 29, 616 + 29, 982 + 20),
    (1012 + 8, 718, 748, 766),
];

/// [`CAT_CTX`] for field macroblocks (field pictures and MBAFF field macroblocks): the
/// significant_coeff_flag and last_significant_coeff_flag contexts of field coded blocks.
const CAT_CTX_FIELD: [(usize, usize, usize, usize); 14] = [
    (85, 277, 338, 227),
    (85 + 4, 277 + 15, 338 + 15, 227 + 10),
    (85 + 8, 277 + 29, 338 + 29, 227 + 20),
    (85 + 12, 277 + 44, 338 + 44, 227 + 30),
    (85 + 16, 277 + 47, 338 + 47, 227 + 39),
    (1012, 436, 451, 426),
    (460, 776, 864, 952),
    (460 + 4, 776 + 15, 864 + 15, 952 + 10),
    (460 + 8, 776 + 29, 864 + 29, 952 + 20),
    (1012 + 4, 675, 699, 708),
    (472, 820, 908, 982),
    (472 + 4, 820 + 15, 908 + 15, 982 + 10),
    (472 + 8, 820 + 29, 908 + 29, 982 + 20),
    (1012 + 8, 733, 757, 766),
];

/// ctxBlockCat of (Intra16x16 DC, Intra16x16 AC, 4x4, 8x8) blocks per colour component Y, Cb, Cr
/// (Table 9-42; Cb and Cr only in 4:4:4).
const LUMA_CATS: [[usize; 4]; 3] = [[0, 1, 2, 5], [6, 7, 8, 9], [10, 11, 12, 13]];

/// The sample location of 4x4 block coordinate `b` relative to a macroblock: its first sample, or
/// for a block left of / above the macroblock the sample next to it (-1), which is what the
/// neighbour derivation of MBAFF frames needs (6.4.12).
#[inline(always)]
fn blk_loc(b: i32) -> i32 {
    if b < 0 { -1 } else { 4 * b }
}

impl<S: crate::deblock::Lanes> SliceDecoder<'_, S> {
    /// Decode slice_data() with CABAC. `rbsp` is the whole slice RBSP.
    pub fn decode_cabac(&mut self, rbsp: &[u8]) -> Result<()> {
        let start = self.sh.header_bits.div_ceil(8) * 8;
        let init = if self.sh.slice_type.is_intra() { 3 } else { self.sh.cabac_init_idc as usize };
        let mut c = Cabac::new(rbsp, start, self.sh.qp(self.pps), init)?;
        let total = self.mb_w * self.mb_h;
        let mbaff = self.mbaff;
        let mut addr = self.sh.first_mb_in_slice as usize * (1 + mbaff as usize);
        let intra_slice = self.sh.slice_type.is_intra();
        // MBAFF: the bottom macroblock's mb_skip_flag, decoded ahead after a skipped top one
        let mut bottom_skip = None;
        loop {
            ensure!(addr < total, "macroblock address {addr} out of range");
            let top = mbaff && addr.is_multiple_of(2);
            if top {
                // until mb_field_decoding_flag is decoded, the inferred value applies (7.4.4)
                self.field_mb = self.infer_field(addr);
            }
            self.start_mb(addr);
            let skip = !intra_slice && bottom_skip.take().unwrap_or_else(|| self.decode_skip_flag(&mut c));
            if skip {
                if top {
                    // a skipped top macroblock takes the flag of its pair, coded with the bottom
                    // macroblock when that one is not skipped (7.3.4)
                    self.mb_mut().kind = if self.sh.slice_type.is_b() { MbKind::BSkip } else { MbKind::PSkip };
                    self.start_mb(addr + 1);
                    let next = self.decode_skip_flag(&mut c);
                    if !next {
                        self.field_mb = self.decode_field_flag(&mut c);
                    }
                    bottom_skip = Some(next);
                    self.start_mb(addr);
                }
                self.decode_skip()?;
            } else {
                if top {
                    self.field_mb = self.decode_field_flag(&mut c);
                    self.start_mb(addr);
                }
                self.decode_mb_cabac(&mut c)?;
            }
            self.pic.mb_done(addr);
            ensure!(!c.overrun(), "CABAC read past end of slice data");
            // MBAFF: end_of_slice_flag follows bottom macroblocks only
            if !top && c.decode_terminate() == 1 {
                break;
            }
            addr += 1;
        }
        Ok(())
    }

    /// mb_field_decoding_flag (ctxIdx 70..=72: the left and above pairs are field pairs).
    fn decode_field_flag(&mut self, c: &mut Cabac) -> bool {
        let pairs = self.pair_nb;
        let cond = |n: Option<usize>| n.and_then(|m| self.pic.mbs.get(m)).is_some_and(|m| m.field) as usize;
        c.decode_decision(70 + cond(pairs[0]) + cond(pairs[1])) == 1
    }

    fn decode_skip_flag(&mut self, c: &mut Cabac) -> bool {
        let cond = |n: Option<usize>| n.map(|m| !self.pic.mbs[m].kind_is_skip()).unwrap_or(false) as usize;
        let inc = cond(self.nb[0]) + cond(self.nb[1]);
        let base = if self.sh.slice_type.is_b() { 24 } else { 11 };
        c.decode_decision(base + inc) == 1
    }

    /// Intra mb_type suffix / I-slice mb_type (Table 9-36). `base` is 3 (I slice) or 17/32 (prefix
    /// offsets in P/B slices). Returns the I-slice mb_type value 0..=25.
    fn decode_mb_type_i(&mut self, c: &mut Cabac, base: usize) -> u32 {
        let islice = base == 3;
        let b0 = if islice {
            let cond = |n: Option<usize>| n.map(|m| !matches!(self.pic.mbs[m].kind, MbKind::I4x4 | MbKind::I8x8)).unwrap_or(false) as usize;
            let inc = cond(self.nb[0]) + cond(self.nb[1]);
            c.decode_decision(base + inc)
        } else {
            c.decode_decision(base)
        };
        if b0 == 0 {
            return 0;
        }
        if c.decode_terminate() == 1 {
            return 25;
        }
        let (c_luma, c_chroma, c_chroma2, c_pred0, c_pred1) =
            if islice { (base + 3, base + 4, base + 5, base + 6, base + 7) } else { (base + 1, base + 2, base + 2, base + 3, base + 3) };
        let luma = c.decode_decision(c_luma);
        let mut chroma = c.decode_decision(c_chroma);
        if chroma != 0 {
            chroma += c.decode_decision(c_chroma2);
        }
        let p0 = c.decode_decision(c_pred0);
        let p1 = c.decode_decision(c_pred1);
        1 + (p0 * 2 + p1) + 4 * chroma + 12 * luma
    }

    fn decode_mb_type(&mut self, c: &mut Cabac) -> Result<MbTypeInfo> {
        let info = match self.sh.slice_type {
            SliceType::I | SliceType::Si => intra_mb_type(self.decode_mb_type_i(c, 3)),
            SliceType::P | SliceType::Sp => {
                if c.decode_decision(14) == 1 {
                    intra_mb_type(self.decode_mb_type_i(c, 17))
                } else if c.decode_decision(15) == 0 {
                    p_mb_type(if c.decode_decision(16) == 0 { 0 } else { 3 })
                } else {
                    p_mb_type(if c.decode_decision(17) == 1 { 1 } else { 2 })
                }
            }
            SliceType::B => {
                let cond = |n: Option<usize>| n.map(|m| !matches!(self.pic.mbs[m].kind, MbKind::BSkip | MbKind::BDirect16x16)).unwrap_or(false) as usize;
                let inc = cond(self.nb[0]) + cond(self.nb[1]);
                if c.decode_decision(27 + inc) == 0 {
                    b_mb_type(0)
                } else if c.decode_decision(27 + 3) == 0 {
                    b_mb_type(1 + c.decode_decision(27 + 5))
                } else {
                    let b2 = c.decode_decision(27 + 4);
                    let b3 = c.decode_decision(27 + 5);
                    let b4 = c.decode_decision(27 + 5);
                    let b5 = c.decode_decision(27 + 5);
                    let v = (b2 << 3) | (b3 << 2) | (b4 << 1) | b5;
                    match v {
                        0b1101 => intra_mb_type(self.decode_mb_type_i(c, 32)),
                        0b1111 => b_mb_type(22),
                        0b1110 => b_mb_type(11),
                        _ if b2 == 0 => b_mb_type(3 + v),
                        _ => {
                            let b6 = c.decode_decision(27 + 5);
                            b_mb_type(12 + (((v & 7) << 1) | b6))
                        }
                    }
                }
            }
        };
        match info {
            Some(i) => Ok(i),
            None => invalid("invalid CABAC mb_type"),
        }
    }

    fn decode_sub_mb_type(&mut self, c: &mut Cabac) -> Result<SubMbInfo> {
        let r = if self.sh.slice_type.is_b() {
            if c.decode_decision(36) == 0 {
                b_sub_mb_type(0)
            } else if c.decode_decision(37) == 0 {
                b_sub_mb_type(1 + c.decode_decision(39))
            } else if c.decode_decision(38) == 1 {
                if c.decode_decision(39) == 1 {
                    b_sub_mb_type(11 + c.decode_decision(39))
                } else {
                    let b4 = c.decode_decision(39);
                    let b5 = c.decode_decision(39);
                    b_sub_mb_type(7 + (b4 << 1) + b5)
                }
            } else {
                let b3 = c.decode_decision(39);
                let b4 = c.decode_decision(39);
                b_sub_mb_type(3 + (b3 << 1) + b4)
            }
        } else if c.decode_decision(21) == 1 {
            p_sub_mb_type(0)
        } else if c.decode_decision(22) == 0 {
            p_sub_mb_type(1)
        } else {
            p_sub_mb_type(if c.decode_decision(23) == 1 { 2 } else { 3 })
        };
        match r {
            Some(s) => Ok(s),
            None => invalid("invalid CABAC sub_mb_type"),
        }
    }

    /// ref_idx_lX for the partition whose top-left 4x4 block is (x, y) (9.3.3.1.1.6).
    fn decode_ref_idx(&mut self, c: &mut Cabac, list: usize, x: usize, y: usize) -> u32 {
        let cond = |xx: i32, yy: i32| -> usize {
            let Some((m, xw, yw)) = self.nb_loc(blk_loc(xx), blk_loc(yy), 16, 16) else { return 0 };
            let st = &self.pic.mbs[m];
            let b8 = (yw / 8) * 2 + xw / 8;
            if st.kind.is_intra() || st.kind_is_skip() || st.direct8x8 & (1 << b8) != 0 {
                return 0;
            }
            // a field neighbour of an MBAFF frame macroblock: field indices count double
            let zero = if self.mbaff && !self.field_mb && st.field { 1 } else { 0 };
            (st.ref_idx[list][b8] > zero) as usize
        };
        let inc = cond(x as i32 - 1, y as i32) + 2 * cond(x as i32, y as i32 - 1);
        let mut v = 0u32;
        if c.decode_decision(54 + inc) == 1 {
            v = 1;
            if c.decode_decision(54 + 4) == 1 {
                v = 2;
                while c.decode_decision(54 + 5) == 1 {
                    v += 1;
                    if v > 32 {
                        break;
                    }
                }
            }
        }
        v
    }

    /// One mvd component for the (sub-)partition with top-left 4x4 block (x, y) (9.3.3.1.1.7, UEG3).
    fn decode_mvd(&mut self, c: &mut Cabac, list: usize, comp: usize, x: usize, y: usize) -> i32 {
        let absn = |xx: i32, yy: i32| -> u32 {
            let Some((m, xw, yw)) = self.nb_loc(blk_loc(xx), blk_loc(yy), 16, 16) else { return 0 };
            let st = &self.pic.mbs[m];
            let v = st.mvd[list][(yw / 4) * 4 + xw / 4][comp] as u32;
            // vertical mvds of MBAFF neighbours in the other (frame / field) units (9.3.3.1.1.7)
            match (comp == 1 && self.mbaff, self.field_mb, st.field) {
                (true, false, true) => v * 2,
                (true, true, false) => v / 2,
                _ => v,
            }
        };
        let sum = absn(x as i32 - 1, y as i32) + absn(x as i32, y as i32 - 1);
        let inc = if sum > 32 {
            2
        } else if sum > 2 {
            1
        } else {
            0
        };
        let base = if comp == 0 { 40 } else { 47 };
        if c.decode_decision(base + inc) == 0 {
            return 0;
        }
        let mut prefix = 1;
        while prefix < 9 && c.decode_decision(base + (prefix + 2).min(6)) == 1 {
            prefix += 1;
        }
        let mut val = prefix as i32;
        if prefix >= 9 {
            // EG3 suffix
            let mut k = 3;
            while c.decode_bypass() == 1 {
                val += 1 << k;
                k += 1;
                if k > 24 {
                    break;
                }
            }
            while k > 0 {
                k -= 1;
                val += (c.decode_bypass() as i32) << k;
            }
        }
        if c.decode_bypass() == 1 { -val } else { val }
    }

    fn set_cur_ref(&mut self, list: usize, x: usize, y: usize, w: usize, h: usize, r: i8) {
        let st = self.mb_mut();
        for yy in (y..y + h).step_by(2) {
            for xx in (x..x + w).step_by(2) {
                st.ref_idx[list][(yy >> 1) * 2 + (xx >> 1)] = r;
            }
        }
    }

    fn set_cur_mvd(&mut self, list: usize, x: usize, y: usize, w: usize, h: usize, d: [i32; 2]) {
        let a = [d[0].unsigned_abs().min(127) as u8, d[1].unsigned_abs().min(127) as u8];
        let st = self.mb_mut();
        for yy in y..y + h {
            for xx in x..x + w {
                st.mvd[list][yy * 4 + xx] = a;
            }
        }
    }

    fn decode_cbp(&mut self, c: &mut Cabac) -> u8 {
        let mut luma = 0u8;
        for b8 in 0..4usize {
            let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            // A
            let (x, y) = (4 * bx as i32, 4 * by as i32);
            let ca = if bx > 0 { ((luma >> (b8 - 1)) & 1 == 0) as usize } else { self.cbp_luma_cond(self.nb_loc(x - 1, y, 16, 16)) };
            let cb = if by > 0 { ((luma >> (b8 - 2)) & 1 == 0) as usize } else { self.cbp_luma_cond(self.nb_loc(x, y - 1, 16, 16)) };
            let bin = c.decode_decision(73 + ca + 2 * cb);
            luma |= (bin as u8) << b8;
        }
        let cond = |n: Option<usize>, bin: usize| -> usize {
            let Some(m) = n else { return 0 };
            let st = &self.pic.mbs[m];
            if st.kind == MbKind::IPcm {
                return 1;
            }
            if st.kind_is_skip() {
                return 0;
            }
            let ch = st.cbp >> 4;
            if bin == 0 { (ch != 0) as usize } else { (ch == 2) as usize }
        };
        let inc0 = cond(self.nb[0], 0) + 2 * cond(self.nb[1], 0);
        let mut chroma = 0u8;
        // the chroma suffix is present for 4:2:0 and 4:2:2 only (9.3.2.6)
        if matches!(self.chroma.0, 1 | 2) && c.decode_decision(77 + inc0) == 1 {
            let inc1 = cond(self.nb[0], 1) + 2 * cond(self.nb[1], 1) + 4;
            chroma = 1 + c.decode_decision(77 + inc1) as u8;
        }
        luma | (chroma << 4)
    }

    /// condTermFlagN for coded_block_pattern luma bin from the neighbouring location `n` (see
    /// [`SliceDecoder::nb_loc`]) in another MB.
    fn cbp_luma_cond(&self, n: Option<(usize, usize, usize)>) -> usize {
        let Some((m, xw, yw)) = n else { return 0 };
        let b8n = (yw / 8) * 2 + xw / 8;
        let st = &self.pic.mbs[m];
        if st.kind == MbKind::IPcm {
            return 0;
        }
        if st.kind_is_skip() {
            return 1;
        }
        ((st.cbp >> (b8n & 3)) & 1 == 0) as usize
    }

    fn decode_qp_delta(&mut self, c: &mut Cabac) -> Result<i32> {
        let inc = self.prev_qp_delta_nz as usize;
        if c.decode_decision(60 + inc) == 0 {
            return Ok(0);
        }
        let mut k = 1u32;
        let mut ctx = 62;
        while c.decode_decision(ctx) == 1 {
            k += 1;
            ctx = 63;
            ensure!(k <= 128, "mb_qp_delta too large");
        }
        let v = k.div_ceil(2) as i32;
        Ok(if k % 2 == 1 { v } else { -v })
    }

    /// coded_block_flag ctxIdxInc condition for the 4x4 block (bx, by) (relative to the current
    /// MB, -1 = in the left / above MB) of component `p` coded like luma (ctxBlockCat 1, 2, 7, 8,
    /// 11, 12; 9.3.3.1.1.9).
    fn cbf_luma_cond(&self, p: usize, bx: i32, by: i32, intra_cur: bool) -> usize {
        if bx >= 0 && by >= 0 {
            return (nnz_of(self.mb(), p)[(by * 4 + bx) as usize] != 0) as usize;
        }
        let Some((m, xw, yw)) = self.nb_loc(blk_loc(bx), blk_loc(by), 16, 16) else { return intra_cur as usize };
        let st = &self.pic.mbs[m];
        (nnz_of(st, p)[(yw / 4) * 4 + xw / 4] != 0) as usize
    }

    /// coded_block_flag ctxIdxInc condition for an 8x8 block of component `p` (ctxBlockCat 5, 9,
    /// 13; 4:4:4 only): the 8x8 block `b8n` of macroblock `n` (None = the neighbour is not
    /// available) counts only when that macroblock used the 8x8 transform.
    fn cbf8_cond(&self, p: usize, n: Option<(usize, usize, usize)>, intra_cur: bool) -> usize {
        let Some((m, xw, yw)) = n else { return intra_cur as usize };
        let b8n = (yw / 8) * 2 + xw / 8;
        let st = &self.pic.mbs[m];
        if st.kind == MbKind::IPcm {
            return 1;
        }
        if !st.transform_8x8 || st.kind_is_skip() {
            return 0;
        }
        (nnz_of(st, p)[(b8n >> 1) * 8 + (b8n & 1) * 2] != 0) as usize
    }

    fn cbf_dc_cond(&self, n: Option<usize>, bit: u8, intra_cur: bool) -> usize {
        let Some(m) = n else { return intra_cur as usize };
        let st = &self.pic.mbs[m];
        if st.kind == MbKind::IPcm {
            return 1;
        }
        (st.cbf_dc & (1 << bit) != 0) as usize
    }

    /// coded_block_flag ctxIdxInc condition for a 4:2:0 / 4:2:2 chroma AC block (ctxBlockCat 4).
    fn cbf_chroma_ac_cond(&self, comp: usize, bx: i32, by: i32, intra_cur: bool) -> usize {
        if bx >= 0 && by >= 0 {
            return (self.mb().nnz_c[comp][(by * 4 + bx) as usize] != 0) as usize;
        }
        let (mw, mh) = (self.chroma.mb_w() as i32, self.chroma.mb_h() as i32);
        let Some((m, xw, yw)) = self.nb_loc(blk_loc(bx), blk_loc(by), mw, mh) else { return intra_cur as usize };
        let st = &self.pic.mbs[m];
        (st.nnz_c[comp][(yw / 4) * 4 + xw / 4] != 0) as usize
    }

    /// residual_block_cabac for ctxBlockCat `cat` (0..=13). Coefficients are written to `coeff`
    /// in list order (indices 0..max_num). `cbf_inc` is the coded_block_flag ctxIdxInc (ignored for
    /// 8x8 blocks outside 4:4:4, which have no coded_block_flag). Returns the number of non-zero
    /// coefficients.
    fn residual_block_cabac(&mut self, c: &mut Cabac, coeff: &mut [i32], cat: usize, max_num: usize, cbf_inc: usize) -> u8 {
        let field = self.field_mb;
        let Some(&(cbf_base, sig_base, last_base, abs_base)) = (if field { &CAT_CTX_FIELD } else { &CAT_CTX }).get(cat) else { return 0 };
        let max_num = max_num.min(coeff.len());
        if max_num == 0 {
            return 0;
        }
        if (max_num != 64 || self.chroma.0 == 3) && c.decode_decision(cbf_base + cbf_inc) == 0 {
            return 0;
        }
        let (sig_tab, last_tab): (&[u8], &[u8]) = match cat {
            3 if max_num == 8 => (&CHROMA_DC_CTX_422, &CHROMA_DC_CTX_422),
            3 => (&CHROMA_DC_CTX, &CHROMA_DC_CTX),
            5 | 9 | 13 if field => (&SIG8_FIELD, &LAST8),
            5 | 9 | 13 => (&SIG8_FRAME, &LAST8),
            _ => (&IDENT_CTX, &IDENT_CTX),
        };
        // significance map: positions of the significant coefficients in scan order
        let mut pos = [0u8; 64];
        let mut n = 0usize;
        let last_i = max_num - 1;
        let mut found_last = false;
        for i in 0..last_i {
            if c.decode_decision(sig_base + sig_tab[i] as usize) == 1 {
                pos[n] = i as u8;
                n += 1;
                if c.decode_decision(last_base + last_tab[i] as usize) == 1 {
                    found_last = true;
                    break;
                }
            }
        }
        if !found_last {
            pos[n] = last_i as u8;
            n += 1;
        }
        let mut eq1 = 0usize;
        let mut gt1 = 0usize;
        let gt1_cap = if cat == 3 { 3 } else { 4 };
        for &p in pos[..n].iter().rev() {
            let inc0 = if gt1 != 0 { 0 } else { (1 + eq1).min(4) };
            let mut abs = 1i32;
            if c.decode_decision(abs_base + inc0) == 1 {
                let ctx = abs_base + 5 + gt1.min(gt1_cap);
                let mut prefix = 1;
                while prefix < 14 && c.decode_decision(ctx) == 1 {
                    prefix += 1;
                }
                abs = prefix + 1;
                if prefix == 14 {
                    let mut k = 0;
                    let mut suf = 0i32;
                    while c.decode_bypass() == 1 {
                        suf += 1 << k;
                        k += 1;
                        if k > 24 {
                            break;
                        }
                    }
                    while k > 0 {
                        k -= 1;
                        suf += (c.decode_bypass() as i32) << k;
                    }
                    abs += suf;
                }
                gt1 += 1;
            } else {
                eq1 += 1;
            }
            coeff[p as usize] = if c.decode_bypass() == 1 { -abs } else { abs };
        }
        n as u8
    }

    fn residual_cabac(&mut self, c: &mut Cabac, cbp: u8) -> Result<()> {
        let intra = self.mb().kind.is_intra();
        self.residual_luma_cabac(c, cbp, 0);
        match self.chroma.0 {
            1 | 2 => {
                let ndc = 2 * self.chroma.mb_h() / 4;
                let cbp_c = cbp >> 4;
                if cbp_c != 0 {
                    for comp in 0..2 {
                        let bit = 1 + comp as u8;
                        let inc = self.cbf_dc_cond(self.nb[0], bit, intra) + 2 * self.cbf_dc_cond(self.nb[1], bit, intra);
                        let mut lv = [0i32; 8];
                        let dc = &mut lv[..ndc];
                        if self.residual_block_cabac(c, dc, 3, ndc, inc) > 0 {
                            self.mb_mut().cbf_dc |= 1 << bit;
                            self.put_chroma_dc(comp, dc);
                        }
                    }
                }
                if cbp_c == 2 {
                    for comp in 0..2 {
                        for b in 0..ndc {
                            let (bx, by) = ((b & 1) as i32, (b >> 1) as i32);
                            let raster = (by * 4 + bx) as usize;
                            let inc = self.cbf_chroma_ac_cond(comp, bx - 1, by, intra) + 2 * self.cbf_chroma_ac_cond(comp, bx, by - 1, intra);
                            let mut lv = [0i32; 16];
                            let n = self.residual_block_cabac(c, &mut lv[1..], 4, 15, inc);
                            self.mb_mut().nnz_c[comp][raster] = n;
                            if n > 0 {
                                self.put_chroma_ac(comp, raster, &lv);
                            }
                        }
                    }
                }
            }
            3 => {
                self.residual_luma_cabac(c, cbp, 1);
                self.residual_luma_cabac(c, cbp, 2);
            }
            _ => {}
        }
        Ok(())
    }

    /// residual_luma() (7.3.5.3.1) of component `p` (Y, or Cb / Cr in 4:4:4) with CABAC.
    fn residual_luma_cabac(&mut self, c: &mut Cabac, cbp: u8, p: usize) {
        let kind = self.mb().kind;
        let intra = kind.is_intra();
        let t8 = self.mb().transform_8x8;
        let [cat_dc, cat_ac, cat4, cat8] = LUMA_CATS[p.min(2)];
        if kind == MbKind::I16x16 {
            let bit = p as u8;
            let inc = self.cbf_dc_cond(self.nb[0], bit, intra) + 2 * self.cbf_dc_cond(self.nb[1], bit, intra);
            let mut lv = [0i32; 16];
            if self.residual_block_cabac(c, &mut lv, cat_dc, 16, inc) > 0 {
                self.mb_mut().cbf_dc |= 1 << bit;
                self.put_luma_dc(p, &lv);
            }
        }
        for b8 in 0..4 {
            let (bx8, by8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            if cbp & (1 << b8) == 0 {
                continue;
            }
            if t8 {
                let inc = if self.chroma.0 == 3 {
                    let (x, y) = (4 * bx8 as i32, 4 * by8 as i32);
                    let na = self.nb_loc(x - 1, y, 16, 16);
                    let nb = self.nb_loc(x, y - 1, 16, 16);
                    self.cbf8_cond(p, na, intra) + 2 * self.cbf8_cond(p, nb, intra)
                } else {
                    0
                };
                let mut lv = [0i32; 64];
                let n = self.residual_block_cabac(c, &mut lv, cat8, 64, inc);
                // outside 4:4:4 the 8x8 block has no coded_block_flag: it counts as coded
                let flag = (n > 0 || self.chroma.0 != 3) as u8;
                let nnz = nnz_of_mut(self.mb_mut(), p);
                for r in [by8 * 4 + bx8, by8 * 4 + bx8 + 1, (by8 + 1) * 4 + bx8, (by8 + 1) * 4 + bx8 + 1] {
                    nnz[r] = flag;
                }
                if n > 0 {
                    self.put_luma8(p, b8, &lv);
                }
            } else {
                for i4 in 0..4 {
                    let (bx, by) = (bx8 + (i4 & 1), by8 + (i4 >> 1));
                    let raster = by * 4 + bx;
                    let inc = self.cbf_luma_cond(p, bx as i32 - 1, by as i32, intra) + 2 * self.cbf_luma_cond(p, bx as i32, by as i32 - 1, intra);
                    let mut lv = [0i32; 16];
                    let n = if kind == MbKind::I16x16 {
                        self.residual_block_cabac(c, &mut lv[1..], cat_ac, 15, inc)
                    } else {
                        self.residual_block_cabac(c, &mut lv, cat4, 16, inc)
                    };
                    nnz_of_mut(self.mb_mut(), p)[raster] = n;
                    if n > 0 {
                        self.put_luma4(p, raster, &lv, kind == MbKind::I16x16);
                    }
                }
            }
        }
    }

    /// Parse and reconstruct one macroblock_layer() with CABAC.
    fn decode_mb_cabac(&mut self, c: &mut Cabac) -> Result<()> {
        let mut info = self.decode_mb_type(c)?;
        self.cur = MbCur { info, ..Default::default() };
        if info.kind == MbKind::IPcm {
            // pcm alignment + samples, then re-initialise the arithmetic decoder
            let pos = c.bit_pos().div_ceil(8);
            let (samples, bytes) = self.read_pcm(c.data(), pos)?;
            c.set_bit_pos((pos + bytes) * 8);
            c.init_engine()?;
            self.finish_pcm();
            self.write_pcm(&samples);
            self.prev_qp_delta_nz = false;
            return Ok(());
        }
        let intra = info.kind.is_intra();
        if info.kind == MbKind::I4x4 && self.pps.transform_8x8_mode && self.decode_t8x8_flag(c) {
            info.kind = MbKind::I8x8;
            self.cur.info.kind = MbKind::I8x8;
            self.mb_mut().transform_8x8 = true;
        }
        self.mb_mut().kind = info.kind;
        // MBAFF field macroblocks index the fields of the list entries (7.4.5.1)
        let k = if self.mbaff && self.field_mb { 2 } else { 1 };
        let n_ref = self.sh.num_ref_idx_active.map(|n| n * k);
        match info.kind {
            MbKind::I4x4 | MbKind::I8x8 => {
                let n = if info.kind == MbKind::I8x8 { 4 } else { 16 };
                for i in 0..n {
                    self.cur.rem_mode[i] = if c.decode_decision(68) == 1 {
                        -1
                    } else {
                        let b0 = c.decode_decision(69);
                        let b1 = c.decode_decision(69);
                        let b2 = c.decode_decision(69);
                        (b0 | (b1 << 1) | (b2 << 2)) as i8
                    };
                }
                self.derive_intra_modes(info.kind == MbKind::I8x8);
                if matches!(self.chroma.0, 1 | 2) {
                    let m = self.decode_chroma_mode(c);
                    self.mb_mut().intra_chroma_mode = m;
                }
            }
            MbKind::I16x16 => {
                if matches!(self.chroma.0, 1 | 2) {
                    let m = self.decode_chroma_mode(c);
                    self.mb_mut().intra_chroma_mode = m;
                }
            }
            MbKind::BDirect16x16 => {
                self.mb_mut().direct8x8 = 0xf;
            }
            _ => {
                if info.part == Part::P8x8 {
                    for p in 0..4 {
                        self.cur.sub[p] = self.decode_sub_mb_type(c)?;
                    }
                    let direct_mask: u8 = (0..4).filter(|&p| self.cur.sub[p].direct).map(|p| 1u8 << p).sum();
                    self.mb_mut().direct8x8 = direct_mask;
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            let (x, y) = ((p & 1) * 2, (p >> 1) * 2);
                            let r = if n_ref[l] > 1 && !info.ref0 { self.decode_ref_idx(c, l, x, y) as i8 } else { 0 };
                            self.cur.ref_idx[l][p] = r;
                            self.set_cur_ref(l, x, y, 2, 2, r);
                        }
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
                            for j in 0..sub.shape.num_parts() {
                                let (sx, sy, w, h) = sub.shape.rect(j);
                                let (x, y) = (x8 + sx, y8 + sy);
                                let dx = self.decode_mvd(c, l, 0, x, y);
                                let dy = self.decode_mvd(c, l, 1, x, y);
                                self.set_cur_mvd(l, x, y, w, h, [dx, dy]);
                                self.cur.mvd[l][p * 4 + j] = [dx as i16, dy as i16];
                            }
                        }
                    }
                } else {
                    let np = info.part.num_parts();
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) == 0 {
                                continue;
                            }
                            let (x, y, w, h) = info.part.rect(p);
                            let r = if n_ref[l] > 1 { self.decode_ref_idx(c, l, x, y) as i8 } else { 0 };
                            self.cur.ref_idx[l][p] = r;
                            self.set_cur_ref(l, x, y, w, h, r);
                        }
                    }
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) == 0 {
                                continue;
                            }
                            let (x, y, w, h) = info.part.rect(p);
                            let dx = self.decode_mvd(c, l, 0, x, y);
                            let dy = self.decode_mvd(c, l, 1, x, y);
                            self.set_cur_mvd(l, x, y, w, h, [dx, dy]);
                            self.cur.mvd[l][p * 4] = [dx as i16, dy as i16];
                        }
                    }
                }
            }
        }
        let cbp = if info.kind == MbKind::I16x16 { info.i16_cbp } else { self.decode_cbp(c) };
        self.cur.cbp = cbp;
        self.mb_mut().cbp = cbp;
        if !intra && cbp & 15 != 0 && self.pps.transform_8x8_mode && self.no_sub_8x8_lt() {
            let t = self.decode_t8x8_flag(c);
            self.mb_mut().transform_8x8 = t;
        }
        if !intra {
            // ref_idx written during parsing is replaced by the derived motion
            self.derive_inter_motion()?;
        }
        if cbp != 0 || info.kind == MbKind::I16x16 {
            let d = self.decode_qp_delta(c)?;
            self.apply_qp_delta(d)?;
            self.prev_qp_delta_nz = d != 0;
        } else {
            self.prev_qp_delta_nz = false;
        }
        self.store_qp();
        self.residual_cabac(c, cbp)?;
        if intra {
            self.reconstruct_intra();
        } else {
            self.predict_inter();
            self.reconstruct_inter_residual();
        }
        Ok(())
    }

    fn decode_t8x8_flag(&mut self, c: &mut Cabac) -> bool {
        let cond = |n: Option<usize>| n.map(|m| self.pic.mbs[m].transform_8x8).unwrap_or(false) as usize;
        let inc = cond(self.nb[0]) + cond(self.nb[1]);
        c.decode_decision(399 + inc) == 1
    }

    fn decode_chroma_mode(&mut self, c: &mut Cabac) -> u8 {
        let cond = |n: Option<usize>| {
            n.map(|m| {
                let st = &self.pic.mbs[m];
                st.kind.is_intra() && st.kind != MbKind::IPcm && st.intra_chroma_mode != 0
            })
            .unwrap_or(false) as usize
        };
        let inc = cond(self.nb[0]) + cond(self.nb[1]);
        if c.decode_decision(64 + inc) == 0 {
            return 0;
        }
        if c.decode_decision(67) == 0 {
            return 1;
        }
        if c.decode_decision(67) == 0 { 2 } else { 3 }
    }
}
