//! The encoder's answers to the tile syntax: block decisions (made when a block starts, against
//! the reconstruction so far) and the values of the syntax elements that code them.

use crate::frame::Plane;
use crate::intra::is_directional_mode;
use crate::spec_tables::*;
use crate::symbol::{SymbolIo, symbol_cost};
use crate::tile::TileDecoder;

use super::fwd::fwd;

/// A syntax element the tile syntax is about to code.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Elem {
    /// Anything the encoder never uses: coded as 0.
    Other,
    /// partition (mi_row, mi_col, bsize)
    Partition(usize, usize, usize),
    /// split_or_horz / split_or_vert at the frame edge
    SplitBool,
    Skip,
    /// intra_frame_y_mode / y_mode
    YMode,
    AngleDeltaY,
    UvMode,
    AngleDeltaUv,
    CflSign,
    CflAlphaU,
    CflAlphaV,
    /// The transform type, with the set's symbol -> type table.
    TxType(&'static [u8]),
    /// all_zero (plane, x, y, tx_sz): the transform block's position in the plane
    AllZero(usize, usize, usize, usize),
    EobPt,
    EobExtra,
    /// eob_extra_bit (bit position)
    EobBit(u32),
    /// coeff_base_eob (pos)
    CoeffBaseEob(usize),
    /// coeff_base (pos)
    CoeffBase(usize),
    /// coeff_br (pos, level so far)
    CoeffBr(usize, i32),
    DcSign,
    /// sign_bit (pos)
    Sign(usize),
    /// golomb_length_bit (pos, length so far)
    GolombLength(usize, u32),
    /// golomb_data_bit (pos, bit)
    GolombBit(usize, u32),
    IsInter,
    NewMv,
    ZeroMv,
    RefMv,
    /// drl_mode (idx)
    Drl(usize),
    MvJoint,
    MvSign(usize),
    MvClass(usize),
    MvClass0Bit(usize),
    MvClass0Fr(usize),
    MvClass0Hp(usize),
    /// mv_bit (comp, i)
    MvBit(usize, usize),
    MvFr(usize),
    MvHp(usize),
}

/// The source picture, padded to the frame buffer geometry by repeating the edge samples.
pub(crate) struct Source {
    pub planes: [Plane; 3],
    /// Visible plane sizes.
    pub width: [usize; 3],
    pub height: [usize; 3],
}

/// Per-frame encoder settings.
#[derive(Clone, Debug)]
pub(crate) struct FrameEnc {
    pub key: bool,
    /// Rate-distortion trade-off for SATD costs: distortion units per bit.
    pub lambda: f64,
    /// Squared error per bit for rate-distortion costs.
    pub lambda_sse: f64,
    /// Bits charged for splitting a block in the partition analysis.
    pub split_bits: f64,
    /// Rate-distortion optimised quantization (levels rounded down, trailing ones dropped).
    pub rdoq: bool,
    /// 0 fastest .. 2 best
    pub effort: u8,
    /// Integer motion search range (luma samples).
    pub range: i32,
    /// Quantizer rounding offsets (fractions of a step) for intra and inter blocks.
    pub round_intra: f32,
    pub round_inter: f32,
}

/// The decisions for the current block.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Plan {
    pub skip: bool,
    pub is_inter: bool,
    pub y_mode: usize,
    pub uv_mode: usize,
    /// NEARESTMV / NEARMV / GLOBALMV / NEWMV
    pub mode: usize,
    pub ref_mv_idx: usize,
    pub mv: [i32; 2],
}

/// Nodes of a 64x64 superblock's partition tree, 64x64 down to 8x8.
const NODES: usize = 1 + 4 + 16 + 64;
const LEVEL_BASE: [usize; 4] = [0, 1, 5, 21];

/// Per-tile encoder state.
pub(crate) struct TileEnc<'f> {
    pub src: &'f Source,
    pub fe: &'f FrameEnc,
    /// Partition tree of the current superblock (split flags).
    pub split: [bool; NODES],
    /// Motion vectors found by the analysis for the tree's nodes.
    pub amv: [[i32; 2]; NODES],
    pub sb: (usize, usize),
    pub plan: Plan,
    /// Quantized coefficients of the current transform block (the decoder's Quant layout).
    coef: Vec<i32>,
    eob: usize,
    mv_diff: [i32; 2],
    pub diff: Vec<i32>,
    fco: Vec<f32>,
    ftmp: Vec<f32>,
}

impl<'f> TileEnc<'f> {
    pub fn new(src: &'f Source, fe: &'f FrameEnc) -> TileEnc<'f> {
        TileEnc {
            src,
            fe,
            split: [false; NODES],
            amv: [[0; 2]; NODES],
            sb: (0, 0),
            plan: Plan::default(),
            coef: vec![0; 32 * 32],
            eob: 0,
            mv_diff: [0; 2],
            diff: vec![0; 64 * 64],
            fco: vec![0.0; 32 * 32],
            ftmp: Vec::new(),
        }
    }

    /// Tree node of the square block (r, c, bsize) of the current superblock.
    pub fn node(&self, r: usize, c: usize, bsize: usize) -> Option<usize> {
        let level = match bsize {
            BLOCK_64X64 => 0,
            BLOCK_32X32 => 1,
            BLOCK_16X16 => 2,
            BLOCK_8X8 => 3,
            _ => return None,
        };
        let unit = 16 >> level;
        let lr = r.checked_sub(self.sb.0)? / unit;
        let lc = c.checked_sub(self.sb.1)? / unit;
        if lr >= 1 << level || lc >= 1 << level {
            return None;
        }
        Some(LEVEL_BASE[level] + (lr << level) + lc)
    }
}

/// Sum of absolute Hadamard-transformed differences (8x8 blocks, 4x4 for 4-sample sides),
/// scaled to be comparable with a SAD.
pub(crate) fn satd(d: &[i32], stride: usize, w: usize, h: usize) -> u64 {
    let mut total = 0u64;
    if w >= 8 && h >= 8 {
        for by in (0..h).step_by(8) {
            for bx in (0..w).step_by(8) {
                let mut m = [[0i32; 8]; 8];
                for (i, row) in m.iter_mut().enumerate() {
                    let o = (by + i) * stride + bx;
                    let Some(src) = d.get(o..o + 8) else { return total };
                    row.copy_from_slice(src);
                    hadamard8(row);
                }
                let mut s = 0u64;
                for j in 0..8 {
                    let mut col = [0i32; 8];
                    for i in 0..8 {
                        col[i] = m[i][j];
                    }
                    hadamard8(&mut col);
                    s += col.iter().map(|v| v.unsigned_abs() as u64).sum::<u64>();
                }
                total += (s + 4) >> 3;
            }
        }
    } else {
        for by in (0..h).step_by(4) {
            for bx in (0..w).step_by(4) {
                let mut m = [[0i32; 4]; 4];
                for (i, row) in m.iter_mut().enumerate() {
                    let o = (by + i) * stride + bx;
                    let Some(src) = d.get(o..o + 4) else { return total };
                    row.copy_from_slice(src);
                    hadamard4(row);
                }
                let mut s = 0u64;
                for j in 0..4 {
                    let mut col = [m[0][j], m[1][j], m[2][j], m[3][j]];
                    hadamard4(&mut col);
                    s += col.iter().map(|v| v.unsigned_abs() as u64).sum::<u64>();
                }
                total += (s + 1) >> 2;
            }
        }
    }
    total
}

fn hadamard4(v: &mut [i32; 4]) {
    wht(v);
}

fn hadamard8(v: &mut [i32; 8]) {
    wht(v);
}

/// In-place Walsh-Hadamard transform (unnormalized).
fn wht<const N: usize>(v: &mut [i32; N]) {
    let mut len = 1;
    while len < N {
        let mut i = 0;
        while i < N {
            for j in i..i + len {
                let (a, b) = (v[j], v[j + len]);
                v[j] = a + b;
                v[j + len] = a - b;
            }
            i += 2 * len;
        }
        len *= 2;
    }
}

/// The coded components of a motion vector difference component (mv_sign, mv_class, the class
/// bits, mv_fr, mv_hp) for |v| = magnitude (1/8 units, non-zero).
pub(crate) struct MvParts {
    pub sign: u32,
    pub class: u32,
    /// class0_bit, or the integer bits for class > 0
    pub bits: u32,
    pub fr: u32,
    pub hp: u32,
}

pub(crate) fn mv_parts(v: i32) -> MvParts {
    let sign = (v < 0) as u32;
    let m = (v.unsigned_abs().max(1) - 1).min((1 << 14) - 1);
    let (class, rest) = if m < (CLASS0_SIZE as u32) << 3 {
        (0, m)
    } else {
        let c = (31 - m.leading_zeros()) - 3;
        (c, m - ((CLASS0_SIZE as u32) << (c + 2)))
    };
    MvParts { sign, class, bits: rest >> 3, fr: (rest >> 1) & 3, hp: rest & 1 }
}

impl<S: SymbolIo> TileDecoder<'_, '_, S> {
    /// The value to code for `e` (only asked when encoding).
    #[inline(always)]
    pub(crate) fn want(&mut self, e: Elem) -> u32 {
        if S::ENCODE { self.policy(e) } else { 0 }
    }

    fn policy(&mut self, e: Elem) -> u32 {
        let Some(mut enc) = self.enc.take() else { return 0 };
        let v = self.answer(&mut enc, e);
        self.enc = Some(enc);
        v
    }

    fn answer(&mut self, enc: &mut TileEnc, e: Elem) -> u32 {
        let p = enc.plan;
        match e {
            Elem::Other | Elem::CflSign | Elem::CflAlphaU | Elem::CflAlphaV => 0,
            Elem::Partition(r, c, bsize) => {
                if enc.node(r, c, bsize).is_some_and(|n| enc.split[n]) && bsize > BLOCK_8X8 {
                    PARTITION_SPLIT as u32
                } else {
                    PARTITION_NONE as u32
                }
            }
            Elem::SplitBool => 1,
            Elem::Skip => p.skip as u32,
            Elem::IsInter => p.is_inter as u32,
            Elem::YMode => p.y_mode as u32,
            Elem::UvMode => p.uv_mode as u32,
            Elem::AngleDeltaY | Elem::AngleDeltaUv => MAX_ANGLE_DELTA as u32,
            Elem::TxType(table) => table.iter().position(|&t| t as usize == DCT_DCT).unwrap_or(0) as u32,
            Elem::AllZero(plane, x, y, tx_sz) => {
                let tx_type = if plane == 0 { DCT_DCT } else { self.compute_tx_type(plane, tx_sz, x >> 2, y >> 2) };
                enc.eob = self.quantize_tx(enc, plane, x, y, tx_sz, tx_type);
                (enc.eob == 0) as u32
            }
            Elem::EobPt => {
                let (pt, _) = eob_pt(enc.eob);
                pt - 1
            }
            Elem::EobExtra => {
                let (pt, off) = eob_pt(enc.eob);
                (off >> (pt.saturating_sub(3))) & 1
            }
            Elem::EobBit(sh) => {
                let (_, off) = eob_pt(enc.eob);
                (off >> sh.min(31)) & 1
            }
            Elem::CoeffBaseEob(pos) => level(enc, pos).clamp(1, 3) - 1,
            Elem::CoeffBase(pos) => level(enc, pos).min(3),
            Elem::CoeffBr(pos, lv) => (level(enc, pos) as i64 - lv as i64).clamp(0, 3) as u32,
            Elem::DcSign => (enc.coef.first().copied().unwrap_or(0) < 0) as u32,
            Elem::Sign(pos) => (enc.coef.get(pos).copied().unwrap_or(0) < 0) as u32,
            Elem::GolombLength(pos, len) => {
                let x = level(enc, pos).saturating_sub(14).max(1);
                (len >= 32 - x.leading_zeros()) as u32
            }
            Elem::GolombBit(pos, k) => {
                let x = level(enc, pos).saturating_sub(14).max(1);
                (x >> k.min(31)) & 1
            }
            Elem::NewMv => (p.mode != NEWMV) as u32,
            Elem::ZeroMv => (p.mode != GLOBALMV) as u32,
            Elem::RefMv => (p.mode != NEARESTMV) as u32,
            Elem::Drl(idx) => (p.ref_mv_idx > idx) as u32,
            Elem::MvJoint => {
                let pred = self.b.pred_mv[0];
                enc.mv_diff = [p.mv[0] - pred[0], p.mv[1] - pred[1]];
                (enc.mv_diff[1] != 0) as u32 * MV_JOINT_HNZVZ as u32 + (enc.mv_diff[0] != 0) as u32 * MV_JOINT_HZVNZ as u32
            }
            Elem::MvSign(c) => mv_parts(enc.mv_diff[c & 1]).sign,
            Elem::MvClass(c) => mv_parts(enc.mv_diff[c & 1]).class,
            Elem::MvClass0Bit(c) => mv_parts(enc.mv_diff[c & 1]).bits,
            Elem::MvClass0Fr(c) | Elem::MvFr(c) => mv_parts(enc.mv_diff[c & 1]).fr,
            Elem::MvClass0Hp(c) | Elem::MvHp(c) => mv_parts(enc.mv_diff[c & 1]).hp,
            Elem::MvBit(c, i) => (mv_parts(enc.mv_diff[c & 1]).bits >> i.min(31)) & 1,
        }
    }

    /// Residual of the transform block at (x, y) of `plane` against the prediction in the current
    /// frame (zero outside the picture), into `enc.diff` (stride w).
    pub(crate) fn residual_of(&self, enc: &mut TileEnc, plane: usize, x: usize, y: usize, w: usize, h: usize) {
        let src = &enc.src.planes[plane];
        let cur = &self.t.cur.planes[plane];
        let vw = enc.src.width[plane];
        let vh = enc.src.height[plane];
        enc.diff.clear();
        enc.diff.resize(w * h, 0);
        for i in 0..h {
            let yy = y + i;
            if yy >= vh {
                break;
            }
            let n = w.min(vw.saturating_sub(x));
            let s = &src.row_from(yy, x)[..n];
            let p = &cur.row_from(yy, x)[..n];
            for (d, (&a, &b)) in enc.diff[i * w..i * w + n].iter_mut().zip(s.iter().zip(p)) {
                *d = a as i32 - b as i32;
            }
        }
    }

    /// DC and AC quantizer steps of `plane` and the dequantization divisor of `tx_sz`.
    pub(crate) fn quant_steps(&self, plane: usize, tx_sz: usize) -> (f32, f32, f32) {
        let fh = &self.fs.fh;
        let qindex = fh.get_qindex(false, self.b.segment_id, self.current_q_index);
        let (dc_delta, ac_delta) = match plane {
            0 => (fh.quant.delta_q_y_dc, 0),
            1 => (fh.quant.delta_q_u_dc, fh.quant.delta_q_u_ac),
            _ => (fh.quant.delta_q_v_dc, fh.quant.delta_q_v_ac),
        };
        let bd_idx = ((self.fs.bit_depth.saturating_sub(8)) >> 1).min(2) as usize;
        let dc_q = DC_QLOOKUP[bd_idx][(qindex + dc_delta).clamp(0, 255) as usize].max(1) as f32;
        let ac_q = AC_QLOOKUP[bd_idx][(qindex + ac_delta).clamp(0, 255) as usize].max(1) as f32;
        let dq_denom = match tx_sz {
            TX_32X32 | TX_16X32 | TX_32X16 | TX_16X64 | TX_64X16 => 2.0,
            TX_64X64 | TX_32X64 | TX_64X32 => 4.0,
            _ => 1.0,
        };
        (dc_q, ac_q, dq_denom)
    }

    /// Rate-distortion cost (squared error plus `lambda_sse` per bit) of coding the residual in
    /// `enc.diff` (one transform block of `tx_sz`, row stride its width) with DCT_DCT,
    /// with an estimated coefficient rate.
    pub(crate) fn rd_residual(&self, enc: &mut TileEnc, tx_sz: usize, round: f32) -> u64 {
        let Some(f) = fwd(tx_sz, DCT_DCT) else { return u64::MAX };
        let n = f.tw * f.th;
        enc.fco.resize(n, 0.0);
        f.apply(&enc.diff, &mut enc.fco, &mut enc.ftmp);
        let (dc_q, ac_q, dq_denom) = self.quant_steps(0, tx_sz);
        let energy: f64 = enc.diff.iter().map(|&d| d as f64 * d as f64).sum();
        let (mut kept, mut err, mut bits) = (0f64, 0f64, 256f64);
        for (i, &c) in enc.fco.iter().enumerate() {
            let step = if i == 0 { dc_q } else { ac_q };
            let a = c.abs();
            let l = (a * dq_denom / step + round).floor();
            let rec = l * step / dq_denom;
            kept += (a * a) as f64;
            err += ((a - rec) * (a - rec)) as f64;
            if l > 0.0 {
                bits += coef_bits(l);
            }
        }
        let g = f.gain as f64;
        let dist = (energy - g * kept).max(0.0) + g * err;
        (dist + enc.fe.lambda_sse * bits / 256.0) as u64
    }

    /// Quantize the residual of a transform block into `enc.coef`; returns eob.
    pub(crate) fn quantize_tx(&mut self, enc: &mut TileEnc, plane: usize, x: usize, y: usize, tx_sz: usize, tx_type: usize) -> usize {
        let Some(f) = fwd(tx_sz, tx_type) else { return 0 };
        self.residual_of(enc, plane, x, y, f.w, f.h);
        let n = f.tw * f.th;
        enc.fco.resize(n, 0.0);
        f.apply(&enc.diff, &mut enc.fco, &mut enc.ftmp);
        let (dc_q, ac_q, dq_denom) = self.quant_steps(plane, tx_sz);
        let round = if self.b.is_inter { enc.fe.round_inter } else { enc.fe.round_intra };
        let rdoq = enc.fe.rdoq;
        let lsse = enc.fe.lambda_sse;
        let gain = f.gain as f64;
        enc.coef.clear();
        enc.coef.resize(n, 0);
        for (i, (&c, q)) in enc.fco.iter().zip(enc.coef.iter_mut()).enumerate() {
            let step = if i == 0 { dc_q } else { ac_q };
            // dequantized value: level * step / dq_denom (masked to 24 bits by the decoder)
            let max_level = ((1u32 << 24) - 1) as f32 / step;
            let a = c.abs() * dq_denom / step;
            let mut l = (a + round).floor().min(max_level).min(0xFFFFF as f32);
            if rdoq && l >= 1.0 {
                // the rounded-down level when it is cheaper overall
                let lo = l - 1.0;
                let err = |v: f32| -> f64 {
                    let e = ((a - v) * step / dq_denom) as f64;
                    gain * e * e
                };
                let bits = |v: f32| if v > 0.0 { coef_bits(v) } else { 0.0 };
                if err(lo) + lsse * bits(lo) / 256.0 < err(l) + lsse * bits(l) / 256.0 {
                    l = lo;
                }
            }
            let l = l as i32;
            *q = if c < 0.0 { -l } else { l };
        }
        self.plane_tx_type = tx_type;
        let scan = self.get_scan(tx_sz);
        let mut eob = 0;
        for (k, &pos) in scan.iter().enumerate().take(n) {
            if enc.coef.get(pos as usize).is_some_and(|&v| v != 0) {
                eob = k + 1;
            }
        }
        if rdoq {
            // drop trailing isolated ones that cost more than they bring
            while eob > 0 {
                let Some(&pos) = scan.get(eob - 1) else { break };
                let pos = pos as usize;
                let (Some(&v), Some(&c)) = (enc.coef.get(pos), enc.fco.get(pos)) else { break };
                if v.abs() != 1 {
                    break;
                }
                let step = if pos == 0 { dc_q } else { ac_q };
                let rec = step / dq_denom;
                let a = c.abs();
                let gain_err = gain * ((a * a) as f64 - ((a - rec) * (a - rec)) as f64);
                if gain_err >= lsse * (coef_bits(1.0) + 512.0) / 256.0 {
                    break;
                }
                if let Some(q) = enc.coef.get_mut(pos) {
                    *q = 0;
                }
                eob -= 1;
                while eob > 0 && scan.get(eob - 1).and_then(|&p| enc.coef.get(p as usize)).is_some_and(|&v| v == 0) {
                    eob -= 1;
                }
            }
        }
        eob
    }

    /// Cost in 1/256 bits of a motion vector difference (row, col).
    pub(crate) fn mv_cost(&self, diff: [i32; 2]) -> u32 {
        let hp = self.fs.fh.allow_high_precision_mv;
        let c = &self.cdf;
        let joint = (diff[1] != 0) as usize * MV_JOINT_HNZVZ + (diff[0] != 0) as usize * MV_JOINT_HZVNZ;
        let mut bits = symbol_cost(&c.mv_joint[0], joint);
        for comp in 0..2 {
            if diff[comp] == 0 {
                continue;
            }
            let p = mv_parts(diff[comp]);
            bits += symbol_cost(&c.mv_sign[0][comp], p.sign as usize);
            bits += symbol_cost(&c.mv_class[0][comp], p.class as usize);
            if p.class == 0 {
                bits += symbol_cost(&c.mv_class0_bit[0][comp], p.bits as usize);
                bits += symbol_cost(&c.mv_class0_fr[0][comp][(p.bits & 1) as usize], p.fr as usize);
                if hp {
                    bits += symbol_cost(&c.mv_class0_hp[0][comp], p.hp as usize);
                }
            } else {
                for i in 0..p.class as usize {
                    bits += symbol_cost(&c.mv_bit[0][comp][i.min(9)], ((p.bits >> i) & 1) as usize);
                }
                bits += symbol_cost(&c.mv_fr[0][comp], p.fr as usize);
                if hp {
                    bits += symbol_cost(&c.mv_hp[0][comp], p.hp as usize);
                }
            }
        }
        bits
    }

    /// Superblock start: decide its partitioning.
    pub(crate) fn enc_superblock(&mut self, r: usize, c: usize) {
        let Some(mut enc) = self.enc.take() else { return };
        enc.sb = (r, c);
        enc.split = [false; NODES];
        enc.amv = [[0; 2]; NODES];
        self.analyse(&mut enc, r, c, BLOCK_64X64);
        self.enc = Some(enc);
    }

    /// Block start: choose its modes (`enc.plan`).
    pub(crate) fn enc_block(&mut self) {
        let Some(mut enc) = self.enc.take() else { return };
        let saved = self.b.clone();
        let plan = self.plan_block(&mut enc);
        enc.plan = plan;
        self.b = saved;
        self.enc = Some(enc);
    }

    fn plan_block(&mut self, enc: &mut TileEnc) -> Plan {
        let lambda = enc.fe.lambda_sse;
        let lam = |bits: u32| -> u64 { (lambda * bits as f64 / 256.0) as u64 };
        let key = self.fs.fh.frame_is_intra;
        // inter candidate
        let mut best: Option<(u64, Plan)> = None;
        if !key && let Some((j, p)) = self.plan_inter(enc) {
            let is_inter_bits = self.is_inter_cost(true);
            best = Some((j + lam(is_inter_bits), p));
        }
        let try_intra = key || enc.fe.effort > 0 || best.as_ref().is_none_or(|(j, _)| *j > (lambda * 24.0) as u64);
        if try_intra {
            let (j, p) = self.plan_intra(enc);
            let extra = if key { 0 } else { lam(self.is_inter_cost(false)) };
            if best.as_ref().is_none_or(|(bj, _)| j + extra < *bj) {
                best = Some((j + extra, p));
            }
        }
        let mut plan = best.map(|(_, p)| p).unwrap_or_default();
        if plan.is_inter {
            self.b.is_inter = true;
            self.b.ref_frame = [LAST_FRAME as i8, -1];
        }
        plan.skip = self.can_skip(enc, &plan);
        plan
    }

    fn block_area(&self) -> usize {
        self.b.bw4 * self.b.bh4 * 16
    }

    fn is_inter_cost(&self, inter: bool) -> u32 {
        let n = self.nb;
        let ctx = if self.b.avail_u && self.b.avail_l {
            if n.left_intra && n.above_intra { 3 } else { (n.left_intra || n.above_intra) as usize }
        } else if self.b.avail_u || self.b.avail_l {
            2 * (if self.b.avail_u { n.above_intra } else { n.left_intra }) as usize
        } else {
            0
        };
        symbol_cost(&self.cdf.is_inter[ctx], inter as usize)
    }

    /// Luma block geometry: (x, y, w, h) in samples, clipped to the coded area.
    fn luma_rect(&self) -> (usize, usize, usize, usize) {
        let x = self.b.mi_col * 4;
        let y = self.b.mi_row * 4;
        (x, y, self.b.bw4 * 4, self.b.bh4 * 4)
    }

    /// SATD of the source against the current frame over the block's luma (visible part).
    fn satd_luma(&self, enc: &mut TileEnc, x: usize, y: usize, w: usize, h: usize) -> u64 {
        self.residual_of(enc, 0, x, y, w, h);
        satd(&enc.diff, w, w, h)
    }

    /// The best intra modes for the block, with their cost.
    fn plan_intra(&mut self, enc: &mut TileEnc) -> (u64, Plan) {
        let lambda = enc.fe.lambda;
        let lam = |bits: u32| -> u64 { (lambda * bits as f64 / 256.0) as u64 };
        let key = self.fs.fh.frame_is_intra;
        self.b.is_inter = false;
        self.b.ref_frame = [INTRA_FRAME as i8, -1];
        self.b.use_filter_intra = false;
        self.b.palette_size_y = 0;
        self.b.palette_size_uv = 0;
        self.b.angle_delta_y = 0;
        self.b.angle_delta_uv = 0;
        let ms = self.b.mi_size;
        let tx = MAX_TX_SIZE_RECT[ms] as usize;
        self.b.tx_size = tx;
        let (x, y, w, h) = self.luma_rect();
        let tw = TX_WIDTH[tx] as usize;
        let th = TX_HEIGHT[tx] as usize;
        let ycdf: Vec<u16> = if key {
            let mi = &self.t.mi;
            let (r, c) = (self.b.mi_row, self.b.mi_col);
            let above = if self.b.avail_u { mi.y_mode[mi.idx(r - 1, c)] as usize } else { DC_PRED };
            let left = if self.b.avail_l { mi.y_mode[mi.idx(r, c - 1)] as usize } else { DC_PRED };
            self.t.intra_frame_y_mode_cdf[INTRA_MODE_CONTEXT[above] as usize][INTRA_MODE_CONTEXT[left] as usize].to_vec()
        } else {
            self.cdf.y_mode[SIZE_GROUP[ms] as usize].to_vec()
        };
        let modes: &[usize] = if enc.fe.effort == 0 || (!key && enc.fe.effort == 1) {
            &[DC_PRED, V_PRED, H_PRED, SMOOTH_PRED, PAETH_PRED]
        } else {
            &[DC_PRED, V_PRED, H_PRED, D45_PRED, D135_PRED, D113_PRED, D157_PRED, D203_PRED, D67_PRED, SMOOTH_PRED, SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED]
        };
        let mut ranked: Vec<(u64, usize, u32)> = Vec::with_capacity(modes.len());
        for &m in modes {
            self.b.y_mode = m;
            // one transform block per block (TX_MODE_LARGEST, blocks up to 64x64)
            let mut d = 0;
            let mut ty = 0;
            while ty < h {
                let mut tx_ = 0;
                while tx_ < w {
                    self.predict_intra_tx(0, x + tx_, y + ty, tx, tx_ >> 2, ty >> 2);
                    tx_ += tw;
                }
                ty += th;
            }
            d += self.satd_luma(enc, x, y, w, h);
            let mut bits = symbol_cost(&ycdf, m);
            if ms >= BLOCK_8X8 && is_directional_mode(m) {
                bits += symbol_cost(&self.cdf.angle_delta[m - V_PRED], MAX_ANGLE_DELTA);
            }
            ranked.push((d + lam(bits), m, bits));
        }
        // the best few by SATD, then by rate-distortion cost
        ranked.sort_unstable_by_key(|r| r.0);
        let keep = [1, 2, 3].get(enc.fe.effort as usize).copied().unwrap_or(3);
        let mut best = (u64::MAX, DC_PRED);
        for &(_, m, bits) in ranked.iter().take(keep) {
            self.b.y_mode = m;
            let mut ty = 0;
            while ty < h {
                let mut tx_ = 0;
                while tx_ < w {
                    self.predict_intra_tx(0, x + tx_, y + ty, tx, tx_ >> 2, ty >> 2);
                    tx_ += tw;
                }
                ty += th;
            }
            self.residual_of(enc, 0, x, y, w, h);
            let j = self.rd_residual(enc, tx, enc.fe.round_intra).saturating_add((enc.fe.lambda_sse * bits as f64 / 256.0) as u64);
            if j < best.0 {
                best = (j, m);
            }
        }
        self.b.y_mode = best.1;
        let mut plan = Plan { y_mode: best.1, uv_mode: DC_PRED, ..Default::default() };
        let total = best.0;
        if self.b.has_chroma {
            let cfl_allowed = if self.b.lossless && self.fs.plane_residual_size(ms, 1) == BLOCK_4X4 {
                true
            } else {
                !self.b.lossless && 4 * self.b.bw4.max(self.b.bh4) <= 32
            };
            let uvcdf: Vec<u16> = if cfl_allowed { self.cdf.uv_mode_cfl_allowed[best.1].to_vec() } else { self.cdf.uv_mode_cfl_not_allowed[best.1].to_vec() };
            let mut cands = vec![DC_PRED, V_PRED, H_PRED, SMOOTH_PRED, PAETH_PRED];
            if !cands.contains(&best.1) {
                cands.push(best.1);
            }
            let uv_tx = self.get_tx_size(1, tx);
            let (ssx, ssy) = (self.fs.ssx, self.fs.ssy);
            let cx = (self.b.mi_col >> ssx) * 4;
            let cy = (self.b.mi_row >> ssy) * 4;
            let psz = self.fs.plane_residual_size(ms, 1);
            let cw = 4 * NUM_4X4_BLOCKS_WIDE[psz] as usize;
            let ch = 4 * NUM_4X4_BLOCKS_HIGH[psz] as usize;
            let (uw, uh) = (TX_WIDTH[uv_tx] as usize, TX_HEIGHT[uv_tx] as usize);
            let mut cbest = (u64::MAX, DC_PRED);
            for &m in &cands {
                self.b.uv_mode = m;
                let mut d = 0;
                for plane in 1..3 {
                    self.predict_intra_tx(plane, cx, cy, uv_tx, 0, 0);
                    let (pw, ph) = (uw.min(cw), uh.min(ch));
                    self.residual_of(enc, plane, cx, cy, pw, ph);
                    d += satd(&enc.diff, pw, pw, ph);
                }
                let mut bits = symbol_cost(&uvcdf, m);
                if ms >= BLOCK_8X8 && is_directional_mode(m) {
                    bits += symbol_cost(&self.cdf.angle_delta[m - V_PRED], MAX_ANGLE_DELTA);
                }
                let j = d + lam(bits);
                if j < cbest.0 {
                    cbest = (j, m);
                }
            }
            plan.uv_mode = cbest.1;
            self.b.uv_mode = cbest.1;
        }
        (total, plan)
    }

    /// Whether the planned block quantizes to nothing (then it is coded as skip).
    fn can_skip(&mut self, enc: &mut TileEnc, plan: &Plan) -> bool {
        let ms = self.b.mi_size;
        let tx = MAX_TX_SIZE_RECT[ms] as usize;
        self.b.tx_size = tx;
        self.b.is_inter = plan.is_inter;
        self.b.y_mode = plan.y_mode;
        self.b.uv_mode = plan.uv_mode;
        self.b.angle_delta_y = 0;
        self.b.angle_delta_uv = 0;
        self.b.use_filter_intra = false;
        if plan.is_inter {
            self.b.ref_frame = [LAST_FRAME as i8, -1];
            for plane in 0..(1 + 2 * self.b.has_chroma as usize) {
                self.predict_block_inter(plane, plan.mv);
            }
        } else {
            self.b.ref_frame = [INTRA_FRAME as i8, -1];
        }
        let planes = 1 + 2 * self.b.has_chroma as usize;
        let (ssx, ssy) = (self.fs.ssx, self.fs.ssy);
        for plane in 0..planes {
            let tsz = self.get_tx_size(plane, tx);
            let (sub_x, sub_y) = if plane > 0 { (ssx, ssy) } else { (0, 0) };
            let psz = self.fs.plane_residual_size(ms, plane);
            let pw = 4 * NUM_4X4_BLOCKS_WIDE[psz] as usize;
            let ph = 4 * NUM_4X4_BLOCKS_HIGH[psz] as usize;
            let bx = (self.b.mi_col >> sub_x) * 4;
            let by = (self.b.mi_row >> sub_y) * 4;
            let (tw, th) = (TX_WIDTH[tsz] as usize, TX_HEIGHT[tsz] as usize);
            let max_x = (self.fs.fh.mi_cols as usize * 4) >> sub_x;
            let max_y = (self.fs.fh.mi_rows as usize * 4) >> sub_y;
            let mut ty = 0;
            while ty < ph {
                let mut tx_ = 0;
                while tx_ < pw {
                    let (xx, yy) = (bx + tx_, by + ty);
                    if xx < max_x && yy < max_y {
                        if !plan.is_inter {
                            self.predict_intra_tx(plane, xx, yy, tsz, tx_ >> 2, ty >> 2);
                        }
                        let tx_type = if plane == 0 { DCT_DCT } else { self.compute_tx_type_for(plane, tsz, plan) };
                        if self.quantize_tx(enc, plane, xx, yy, tsz, tx_type) > 0 {
                            return false;
                        }
                    }
                    tx_ += tw;
                }
                ty += th;
            }
        }
        true
    }

    /// The chroma transform type of a planned block (7.13.? compute_tx_type, before the block's
    /// luma has been coded): DCT_DCT for inter blocks (their luma uses DCT_DCT), the mode's type
    /// for intra.
    fn compute_tx_type_for(&self, plane: usize, tx_sz: usize, plan: &Plan) -> usize {
        if plan.is_inter || plane == 0 {
            return DCT_DCT;
        }
        let set = self.get_tx_set(tx_sz);
        let t = MODE_TO_TXFM[plan.uv_mode] as usize;
        if TX_TYPE_IN_SET_INTRA.get(set).is_some_and(|s| s.get(t).copied() == Some(1)) && TX_SIZE_SQR_UP[tx_sz] as usize <= TX_32X32 { t } else { DCT_DCT }
    }
}

/// eob_pt and the offset of eob within its group.
fn eob_pt(eob: usize) -> (u32, u32) {
    if eob <= 1 {
        return (1, 0);
    }
    let pt = 2 + (usize::BITS - 1 - (eob - 1).leading_zeros());
    let start = (1usize << (pt - 2)) + 1;
    (pt, (eob - start) as u32)
}

/// Estimated cost (1/256 bits) of a coefficient of level `l` > 0, with its share of the zeros
/// and end-of-block coding.
fn coef_bits(l: f32) -> f64 {
    let base = if l <= 1.0 {
        3.0
    } else if l <= 2.0 {
        4.5
    } else if l <= 14.0 {
        5.0 + 0.5 * (l - 3.0) as f64
    } else {
        11.0 + 2.0 * ((l - 14.0) as f64).log2()
    };
    256.0 * base
}

fn level(enc: &TileEnc, pos: usize) -> u32 {
    enc.coef.get(pos).map_or(0, |v| v.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eob_groups() {
        assert_eq!(eob_pt(1), (1, 0));
        assert_eq!(eob_pt(2), (2, 0));
        assert_eq!(eob_pt(3), (3, 0));
        assert_eq!(eob_pt(4), (3, 1));
        assert_eq!(eob_pt(5), (4, 0));
        assert_eq!(eob_pt(8), (4, 3));
        assert_eq!(eob_pt(9), (5, 0));
        assert_eq!(eob_pt(1024), (11, 511));
    }

    #[test]
    fn mv_parts_invert_the_decoder() {
        for v in [-3000i32, -17, -16, -9, -8, -1, 1, 2, 7, 8, 15, 16, 17, 31, 32, 100, 1023, 4000] {
            let p = mv_parts(v);
            let mag = if p.class == 0 {
                ((p.bits << 3) | (p.fr << 1) | p.hp) as i32 + 1
            } else {
                ((CLASS0_SIZE as i32) << (p.class + 2)) + ((p.bits << 3) | (p.fr << 1) | p.hp) as i32 + 1
            };
            assert_eq!(if p.sign == 1 { -mag } else { mag }, v);
        }
    }

    #[test]
    fn satd_of_flat_difference() {
        let d = vec![3i32; 64];
        // DC only: 8 * 8 * 3 / 8
        assert_eq!(satd(&d, 8, 8, 8), 24);
        // an 8x8 block of 4x4 SATDs: 4 * (16 * 3 / 4)
        assert_eq!(satd(&d, 8, 4, 4), 12);
    }
}
