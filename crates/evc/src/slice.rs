// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! Picture decoding: coding tree, coding unit syntax, reconstruction and the
//! deblocking pass (`xevd.c`, `xevd_eco.c`, `xevd_util.c`, `xevd_df.c`).
//!
//! The reference decoder parses a whole slice before reconstructing it; the
//! port decodes and reconstructs each coding unit in turn, which reads the
//! same neighbour data (reconstruction only looks at coding units that are
//! already reconstructed, parsing only at ones already parsed).

use crate::bits::Bits;
use crate::dpb::{MAX_REFS, Pic, Plane, Refp};
use crate::dsp::{self, Nbr, RefPlane};
use crate::params::{NUT_IDR, NalHeader, SLICE_B, SLICE_I, SLICE_P, Sps};
use crate::sbac::{Model, Sbac, Unary};
use crate::tables::{DF_ST, MPM};
use crate::{Decoder, Error};

const MIN_CU_LOG2: u32 = 2;
const CTU_LOG2: u32 = 6;
const CTU: usize = 1 << CTU_LOG2;

// map_scu bits (MCU_*).
const IF: u32 = 1 << 15;
const SF: u32 = 1 << 23;
const CBFL: u32 = 1 << 24;
const COD: u32 = 1 << 31;

const MODE_INTRA: u8 = 0;
const MODE_INTER: u8 = 1;
const MODE_SKIP: u8 = 2;

const PRED_L0: u8 = 0;
const PRED_L1: u8 = 1;
const PRED_BI: u8 = 2;
const PRED_DIR: u8 = 4;

// Availability bits (AVAIL_*).
const AV_UP: u16 = 1 << 0;
const AV_LE: u16 = 1 << 1;
const AV_UP_RI: u16 = 1 << 6;

/// Per-sequence geometry and the maps of the picture being decoded.
pub(crate) struct Seq {
    w: usize,
    h: usize,
    chroma: bool,
    bit_depth: u32,
    w_lcu: usize,
    h_lcu: usize,
    w_scu: usize,
    h_scu: usize,
    map_scu: Vec<u32>,
    cod_eco: Vec<u8>,
    map_ipm: Vec<i8>,
    /// Scan tables `[log2_w - 1][log2_h - 1]`.
    scan: Vec<Vec<Vec<u16>>>,
}

impl Seq {
    pub(crate) fn new(sps: &Sps) -> Self {
        let w = sps.width as usize;
        let h = sps.height as usize;
        let w_scu = w.div_ceil(4);
        let h_scu = h.div_ceil(4);
        let f_scu = w_scu * h_scu;
        let mut scan = Vec::new();
        for lx in 1..=6u32 {
            let mut row = Vec::new();
            for ly in 1..=6u32 {
                row.push(zigzag(1 << lx, 1 << ly));
            }
            scan.push(row);
        }
        Self {
            w,
            h,
            chroma: sps.chroma_format_idc != 0,
            bit_depth: sps.bit_depth_luma,
            w_lcu: w.div_ceil(CTU),
            h_lcu: h.div_ceil(CTU),
            w_scu,
            h_scu,
            map_scu: vec![0; f_scu],
            cod_eco: vec![0; f_scu],
            map_ipm: vec![-1; f_scu],
            scan,
        }
    }

    pub(crate) fn matches(&self, sps: &Sps) -> bool {
        self.w == sps.width as usize && self.h == sps.height as usize && self.chroma == (sps.chroma_format_idc != 0) && self.bit_depth == sps.bit_depth_luma
    }

    fn scu(&self, i: isize) -> u32 {
        if i < 0 {
            return 0;
        }
        self.map_scu.get(i as usize).copied().unwrap_or(0)
    }
}

/// `init_scan` (zig-zag).
fn zigzag(sx: usize, sy: usize) -> Vec<u16> {
    let mut scan = Vec::with_capacity(sx * sy);
    scan.push(0u16);
    for l in 1..sx + sy - 1 {
        if l % 2 == 1 {
            let mut x = l.min(sx - 1) as isize;
            let mut y = l.saturating_sub(sx - 1) as isize;
            while x >= 0 && (y as usize) < sy {
                scan.push((y as usize * sx + x as usize) as u16);
                x -= 1;
                y += 1;
            }
        } else {
            let mut y = l.min(sy - 1) as isize;
            let mut x = l.saturating_sub(sy - 1) as isize;
            while y >= 0 && (x as usize) < sx {
                scan.push((y as usize * sx + x as usize) as u16);
                x += 1;
                y -= 1;
            }
        }
    }
    scan
}

/// State of the coding unit being decoded (`XEVD_CORE`). Some fields keep
/// their value from one coding unit to the next, as in the reference.
#[derive(Default)]
struct Core {
    pred_mode: u8,
    inter_dir: u8,
    mvp_idx: [u8; 2],
    mvd: [[i16; 2]; 2],
    refi: [i8; 2],
    mv: [[i16; 2]; 2],
    ipm: [u8; 2],
    qp: i32,
    qp_y: i32,
    qp_u: i32,
    qp_v: i32,
    is_coef: [bool; 3],
    is_coef_sub0: [bool; 3],
    qp_prev_eco: i32,
}

struct Ctx<'a> {
    seq: &'a mut Seq,
    sps: &'a Sps,
    cip: bool,
    cu_qp_delta: bool,
    slice_type: u32,
    sh_qp: i32,
    qp_u_offset: i32,
    qp_v_offset: i32,
    chroma_qp: &'a dyn Fn(usize, i32) -> i32,
    /// Picture being decoded.
    cur: &'a mut Pic,
    /// Reference pictures (all other pictures of the buffer, by id).
    refs: &'a [Option<&'a Pic>],
    refp: &'a [[Refp; 2]; MAX_REFS],
    num_refp: [usize; 2],
    poc: i32,
    core: Core,
}

impl Decoder {
    /// Decodes one slice NAL unit (a whole picture in the Baseline profile).
    /// Returns whether a picture was added to the buffer.
    pub(crate) fn decode_picture(&mut self, mut bs: Bits<'_>, hdr: NalHeader) -> Result<bool, Error> {
        let (Some(sps), Some(pps)) = (self.sps.clone(), self.pps.clone()) else {
            return Err(Error::Malformed("slice before the parameter sets"));
        };
        if self.seq.is_none() {
            return Err(Error::Malformed("slice before the parameter sets"));
        }
        let nut = hdr.nal_type;
        let is_idr = nut == NUT_IDR;
        crate::params::slice_header(&mut bs, &sps, &pps, &mut self.sh, nut)?;
        let tid = hdr.temporal_id;
        let slice_ref_flag = tid == 0 || u32::from(tid) < sps.log2_sub_gop_length;
        if is_idr {
            self.poc.prev_doc_offset = -1;
            self.poc.prev_poc_val = 0;
            self.poc.poc_val = 0;
        } else {
            self.poc.derive(tid, sps.log2_sub_gop_length)?;
        }
        let poc = self.poc.poc_val;
        let slice_type = self.sh.slice_type;
        if !is_idr && slice_type == SLICE_I && self.pic_cnt == 0 {
            self.cra_start_poc = poc;
        } else if is_idr {
            self.cra_start_poc = -1;
        }
        if self.cra_start_poc >= 0 && poc < self.cra_start_poc && !is_idr && slice_type != SLICE_I {
            // Leading picture of a random access point that cannot be decoded.
            return Ok(false);
        }
        if slice_type == SLICE_I {
            self.last_intra_poc = poc;
        }
        self.dpb.refp_init(&mut self.refp, sps.max_num_ref_pics as usize, slice_type, poc, tid, self.last_intra_poc)?;
        let Some(seq) = self.seq.as_mut() else {
            return Err(Error::Malformed("slice before the parameter sets"));
        };
        seq.map_scu.fill(0);
        seq.cod_eco.fill(0);
        let (w, h, chroma, f_scu) = (seq.w, seq.h, seq.chroma, seq.w_scu * seq.h_scu);
        let id = self.dpb.get_empty(|| new_pic(w, h, chroma, f_scu))?;
        // Every reference picture by id; the current one is taken out.
        let (cur, refs) = split_arena(&mut self.dpb.arena, id)?;
        cur.map_refi.fill([-1, -1]);
        cur.tag = self.tag;
        cur.map_mv.fill([[0; 2]; 2]);
        let base = self.chroma_qp_base;
        let table = self.chroma_qp;
        let chroma_map = move |i: usize, qp: i32| -> i32 {
            let k = qp + base;
            if (0..94).contains(&k) { table.get(i).map_or(0, |t| t[k as usize]) } else { 0 }
        };
        let bdc_m8 = sps.bit_depth_chroma as i32 - 8;
        let mut cx = Ctx {
            seq,
            sps: &sps,
            cip: pps.constrained_intra_pred_flag,
            cu_qp_delta: pps.cu_qp_delta_enabled_flag,
            slice_type,
            sh_qp: self.sh.qp,
            qp_u_offset: self.sh.qp_u_offset,
            qp_v_offset: self.sh.qp_v_offset,
            chroma_qp: &chroma_map,
            cur,
            refs: &refs,
            refp: &self.refp,
            num_refp: self.dpb.num_refp,
            poc,
            core: Core {
                qp_y: self.sh.qp + 6 * (sps.bit_depth_luma as i32 - 8),
                qp_u: chroma_map(0, self.sh.qp_u) + 6 * bdc_m8,
                qp_v: chroma_map(1, self.sh.qp_v) + 6 * bdc_m8,
                qp_prev_eco: self.sh.qp,
                ..Core::default()
            },
        };
        let mut sbac = Sbac::new(bs);
        let mut cus: Vec<(usize, usize, usize)> = Vec::new();
        let n_ctu = cx.seq.w_lcu * cx.seq.h_lcu;
        for lcu in 0..n_ctu {
            let x = (lcu % cx.seq.w_lcu) * CTU;
            let y = (lcu / cx.seq.w_lcu) * CTU;
            cx.tree(&mut sbac, x, y, CTU_LOG2, &mut cus)?;
            sbac.check()?;
        }
        if sbac.bin_trm()? != 1 {
            return Err(Error::Malformed("missing end of tile"));
        }
        // cabac_zero_words: the rest must be zero.
        while sbac.bs.left() >= 16 {
            if sbac.bs.bits(16) != 0 {
                return Err(Error::Malformed("data after the end of the tile"));
            }
        }
        if self.sh.deblocking_filter_on {
            cx.deblock(&cus, true);
            cx.deblock(&cus, false);
        }
        for p in &mut cx.cur.planes {
            p.expand();
        }
        let is_rap = is_idr || self.cra_start_poc == poc;
        self.dpb.put(id, is_rap, poc, tid, &self.refp, slice_ref_flag, self.ref_pic_gap_length);
        if self.cra_start_poc == poc {
            self.dpb.set_next_output(poc);
        }
        self.pic_cnt = self.pic_cnt.wrapping_add(1);
        Ok(true)
    }
}

fn new_pic(w: usize, h: usize, chroma: bool, f_scu: usize) -> Pic {
    let mut planes = vec![Plane::new(w, h, 4)];
    if chroma {
        planes.push(Plane::new(w.div_ceil(2), h.div_ceil(2), 2));
        planes.push(Plane::new(w.div_ceil(2), h.div_ceil(2), 2));
    }
    Pic {
        planes,
        poc: 0,
        temporal_id: 0,
        is_ref: false,
        need_for_out: false,
        list_poc: [0; MAX_REFS],
        map_mv: vec![[[0; 2]; 2]; f_scu],
        map_refi: vec![[-1; 2]; f_scu],
        tag: 0,
    }
}

/// Splits the picture arena into the picture `id` (mutable) and shared
/// references to all the others, indexed by id.
fn split_arena(arena: &mut [Pic], id: usize) -> Result<(&mut Pic, Vec<Option<&Pic>>), Error> {
    if id >= arena.len() {
        return Err(Error::Malformed("picture buffer out of sync"));
    }
    let (before, rest) = arena.split_at_mut(id);
    let (cur, after) = rest.split_first_mut().ok_or(Error::Malformed("picture buffer out of sync"))?;
    let mut refs: Vec<Option<&Pic>> = before.iter().map(Some).collect();
    refs.push(None);
    refs.extend(after.iter().map(Some));
    Ok((cur, refs))
}

impl Ctx<'_> {
    fn bdl(&self) -> u32 {
        self.sps.bit_depth_luma
    }

    fn bdc(&self) -> u32 {
        self.sps.bit_depth_chroma
    }

    /// `xevd_entropy_decode_tree` (quad-tree only in Baseline).
    fn tree(&mut self, sb: &mut Sbac<'_>, x: usize, y: usize, log2: u32, cus: &mut Vec<(usize, usize, usize)>) -> Result<(), Error> {
        let size = 1usize << log2;
        let split = if size >= 8 { sb.bin_ctx(Model::SplitCu) != 0 } else { false };
        if split {
            let half = size / 2;
            for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                if x + dx < self.seq.w && y + dy < self.seq.h {
                    self.tree(sb, x + dx, y + dy, log2 - 1, cus)?;
                }
            }
            Ok(())
        } else {
            cus.push((x, y, size));
            self.cu(sb, x, y, log2)
        }
    }

    fn chroma_qps(&mut self) {
        let lo = -6 * (self.bdc() as i32 - 8);
        let cb = (self.core.qp + self.qp_u_offset).clamp(lo, 57);
        let cr = (self.core.qp + self.qp_v_offset).clamp(lo, 57);
        let add = 6 * (self.bdc() as i32 - 8);
        self.core.qp_u = (self.chroma_qp)(0, cb) + add;
        self.core.qp_v = (self.chroma_qp)(1, cr) + add;
    }

    /// `xevd_eco_cu` followed by `xevd_recon_unit` and `xevd_set_dec_info`.
    fn cu(&mut self, sb: &mut Sbac<'_>, x: usize, y: usize, log2: u32) -> Result<(), Error> {
        let size = 1usize << log2;
        let w_scu = self.seq.w_scu;
        let (x_scu, y_scu) = (x >> MIN_CU_LOG2, y >> MIN_CU_LOG2);
        let scup = y_scu * w_scu + x_scu;
        let c = &mut self.core;
        c.pred_mode = MODE_INTRA;
        c.mvp_idx = [0, 0];
        c.inter_dir = PRED_L0;
        c.mvd = [[0; 2]; 2];
        let bd_m8 = self.sps.bit_depth_luma as i32 - 8;
        let n_c = if self.seq.chroma { 3 } else { 1 };
        let csize = size >> 1;
        let mut coef: [Vec<i16>; 3] = [vec![0; size * size], vec![0; csize * csize], vec![0; csize * csize]];
        if self.slice_type != SLICE_I && sb.bin_ctx(Model::Skip) != 0 {
            self.core.pred_mode = MODE_SKIP;
        }
        if self.core.pred_mode == MODE_SKIP {
            self.core.mvp_idx[0] = sb.mvp_idx() as u8;
            if self.slice_type == SLICE_B {
                self.core.mvp_idx[1] = sb.mvp_idx() as u8;
            }
            self.core.is_coef = [false; 3];
            self.core.is_coef_sub0 = [false; 3];
            self.core.qp = if self.cu_qp_delta { self.core.qp_prev_eco } else { self.sh_qp };
            self.core.qp_y = self.core.qp + 6 * bd_m8;
            self.chroma_qps();
        } else {
            let intra = self.slice_type == SLICE_I || sb.bin_ctx(Model::PredMode) != 0;
            self.core.pred_mode = if intra { MODE_INTRA } else { MODE_INTER };
            if !intra {
                if self.slice_type == SLICE_B && sb.bin_ctx(Model::DirectMode) != 0 {
                    self.core.inter_dir = PRED_DIR;
                }
                if self.core.inter_dir != PRED_DIR {
                    if self.slice_type == SLICE_B {
                        self.core.inter_dir = if sb.bin_ctx(Model::InterDir(0)) == 0 {
                            PRED_BI
                        } else if sb.bin_ctx(Model::InterDir(1)) != 0 {
                            PRED_L1
                        } else {
                            PRED_L0
                        };
                    }
                    for l in 0..2 {
                        if ((self.core.inter_dir + 1) >> l) & 1 == 1 {
                            let n = self.num_refp[l] as i32;
                            self.core.refi[l] = sb.refi(n) as i8;
                            self.core.mvp_idx[l] = sb.mvp_idx() as u8;
                            self.core.mvd[l] = sb.mvd()?;
                        }
                    }
                }
            } else {
                // xevd_get_mpm_b + xevd_eco_intra_dir_b.
                let mode_of = |s: &Self, n: usize| -> usize {
                    if s.seq.map_scu.get(n).is_some_and(|m| m & IF != 0) && s.seq.cod_eco.get(n).is_some_and(|&c| c != 0) {
                        (s.seq.map_ipm.get(n).copied().unwrap_or(-1) as i32 + 1).clamp(0, 5) as usize
                    } else {
                        0
                    }
                };
                let ipm_l = if x_scu > 0 { mode_of(self, scup - 1) } else { 0 };
                let ipm_u = if y_scu > 0 { mode_of(self, scup - w_scu) } else { 0 };
                let mpm = MPM[ipm_l][ipm_u];
                let t = sb.unary(Unary::IntraDir)?;
                let mut ipm = 0u8;
                for (i, &m) in mpm.iter().enumerate() {
                    if u32::from(m) == t {
                        ipm = i as u8;
                    }
                }
                self.core.ipm = [ipm, ipm];
                self.core.refi = [-1, -1];
                self.core.mv = [[0; 2]; 2];
            }
            self.coefs(sb, log2, &mut coef)?;
        }
        sb.check()?;
        // Reconstruction.
        let mut pred: [Vec<i16>; 3] = [vec![0; size * size], vec![0; csize * csize], vec![0; csize * csize]];
        let mut pred1: [Vec<i16>; 3] = [vec![0; size * size], vec![0; csize * csize], vec![0; csize * csize]];
        if self.core.pred_mode != MODE_SKIP {
            let qps = [self.core.qp_y, self.core.qp_u, self.core.qp_v];
            for comp in 0..n_c {
                if self.core.is_coef_sub0[comp] {
                    let l = if comp == 0 { log2 } else { log2 - 1 };
                    dsp::itdq(&mut coef[comp], l, l, qps[comp], self.bdl());
                }
            }
        }
        if self.core.pred_mode != MODE_INTRA {
            let avail = self.avail_inter(x_scu, y_scu, scup, size);
            if self.core.pred_mode == MODE_SKIP {
                self.skip_motion(scup, size, avail)?;
            } else if self.core.inter_dir == PRED_DIR {
                let br = scup + (size / 4 - 1) + (size / 4 - 1) * w_scu;
                self.mv_dir(br)?;
                self.core.refi = [0, 0];
            } else {
                self.inter_motion(scup, size, avail)?;
            }
            self.mc(x, y, size, &mut pred, &mut pred1)?;
        } else {
            for comp in 0..n_c {
                let nb = self.nbr(x, y, size, scup, comp);
                let (px, l) = if comp == 0 { (size, size) } else { (csize, csize) };
                dsp::ipred(&nb, self.core.ipm[usize::from(comp != 0)], px, l, &mut pred[comp]);
            }
        }
        for comp in 0..n_c {
            let (sz, ox, oy) = if comp == 0 { (size, x, y) } else { (csize, x >> 1, y >> 1) };
            let bd = if comp == 0 { self.bdl() } else { self.bdc() };
            let Some(plane) = self.cur.planes.get_mut(comp) else { continue };
            let off = oy * plane.stride + ox;
            let stride = plane.stride;
            let Some(dst) = plane.data.get_mut(off..) else {
                return Err(Error::Malformed("coding unit outside the picture"));
            };
            let res = if self.core.is_coef[comp] { Some(coef[comp].as_slice()) } else { None };
            dsp::recon(res, &pred[comp], sz, sz, dst, stride, bd);
        }
        self.set_dec_info(scup, size);
        // cod_eco and COD for the area of the coding unit.
        let n = size >> MIN_CU_LOG2;
        for j in 0..n {
            for i in 0..n {
                let k = scup + j * w_scu + i;
                if let Some(c) = self.seq.cod_eco.get_mut(k) {
                    *c = 1;
                }
                if let Some(m) = self.seq.map_scu.get_mut(k) {
                    *m |= COD;
                }
            }
        }
        Ok(())
    }

    /// `xevd_eco_coef` for coding units up to 64×64 (one transform block).
    fn coefs(&mut self, sb: &mut Sbac<'_>, log2: u32, coef: &mut [Vec<i16>; 3]) -> Result<(), Error> {
        let chroma = self.seq.chroma;
        let intra = self.core.pred_mode == MODE_INTRA;
        self.core.is_coef = [false; 3];
        self.core.is_coef_sub0 = [false; 3];
        let mut cbf = [false; 3];
        if !intra && sb.bin_ctx(Model::CbfAll) == 0 {
            self.core.qp = self.core.qp_prev_eco.rem_euclid(52);
            self.core.qp_y = self.core.qp + 6 * (self.bdl() as i32 - 8);
            return Ok(());
        }
        if chroma {
            cbf[1] = sb.bin_ctx(Model::CbfCb) != 0;
            cbf[2] = sb.bin_ctx(Model::CbfCr) != 0;
        }
        cbf[0] = if !intra && !cbf[1] && !cbf[2] { true } else { sb.bin_ctx(Model::CbfLuma) != 0 };
        if self.cu_qp_delta && (cbf[0] || cbf[1] || cbf[2]) {
            let dqp = sb.dqp()?;
            let qp = (self.core.qp_prev_eco + dqp + 52) % 52;
            if qp < 0 {
                return Err(Error::Malformed("QP delta out of range"));
            }
            self.core.qp = qp;
            self.core.qp_prev_eco = qp;
        } else {
            self.core.qp = (self.core.qp_prev_eco + 52) % 52;
        }
        self.core.qp_y = self.core.qp + 6 * (self.bdl() as i32 - 8);
        self.chroma_qps();
        for comp in 0..3 {
            if cbf[comp] {
                let l = if comp == 0 { log2 } else { log2 - 1 };
                self.run_length(sb, &mut coef[comp], l, comp != 0)?;
                self.core.is_coef_sub0[comp] = true;
                self.core.is_coef[comp] = true;
            }
        }
        Ok(())
    }

    /// `xevd_eco_run_length_cc`.
    fn run_length(&self, sb: &mut Sbac<'_>, coef: &mut [i16], log2: u32, chroma: bool) -> Result<(), Error> {
        let idx = (log2 as usize).saturating_sub(1);
        let scan = self.seq.scan.get(idx).and_then(|r| r.get(idx)).ok_or(Error::Malformed("transform size out of range"))?;
        let num = 1usize << (2 * log2);
        let mut pos = 0usize;
        loop {
            let run = sb.unary(Unary::Run(chroma))? as usize;
            pos += run;
            if pos >= num {
                return Err(Error::Malformed("coefficient run past the block"));
            }
            let level = sb.unary(Unary::Level(chroma))? as i32 + 1;
            let sign = sb.bin_ep();
            let v = if sign != 0 { -(level as i16) } else { level as i16 };
            if let Some(c) = scan.get(pos).and_then(|&p| coef.get_mut(usize::from(p))) {
                *c = v;
            }
            if pos >= num - 1 {
                return Ok(());
            }
            pos += 1;
            if sb.bin_ctx(Model::Last(chroma)) != 0 {
                return Ok(());
            }
            sb.check()?;
        }
    }

    /// `xevd_get_avail_inter`.
    fn avail_inter(&self, x_scu: usize, y_scu: usize, scup: usize, size: usize) -> u16 {
        let s = &self.seq;
        let w = s.w_scu as isize;
        let n = (size >> MIN_CU_LOG2) as isize;
        let p = scup as isize;
        let nif = |m: u32| m & IF == 0;
        let cod = |m: u32| m & COD != 0;
        let mut avail = 0u16;
        if x_scu > 0 && nif(s.scu(p - 1)) && cod(s.scu(p - 1)) {
            avail |= AV_LE;
        }
        if y_scu > 0 {
            if nif(s.scu(p - w)) {
                avail |= AV_UP;
            }
            let ur = s.scu(p - w + n);
            if x_scu + (n as usize) < s.w_scu && nif(ur) && cod(ur) {
                avail |= AV_UP_RI;
            }
        }
        avail
    }

    fn ref_pic(&self, refi: i8, list: usize) -> Result<&Pic, Error> {
        let r =
            usize::try_from(refi).ok().and_then(|i| self.refp.get(i)).and_then(|r| r[list].pic).ok_or(Error::Malformed("reference index without a picture"))?;
        self.refs.get(r).copied().flatten().ok_or(Error::Malformed("reference picture is the current picture"))
    }

    /// `xevd_get_motion`: the four motion vector predictor candidates.
    fn get_motion(&self, scup: usize, list: usize, size: usize, avail: u16) -> Result<[[i16; 2]; 4], Error> {
        let w = self.seq.w_scu;
        let mv_at = |k: Option<usize>| -> [i16; 2] { k.and_then(|k| self.cur.map_mv.get(k)).map_or([0, 0], |m| m[list]) };
        let mut c = [[1i16, 1]; 4];
        if avail & AV_LE != 0 {
            c[0] = mv_at(scup.checked_sub(1));
        }
        if avail & AV_UP != 0 {
            c[1] = mv_at(scup.checked_sub(w));
        }
        if avail & AV_UP_RI != 0 {
            c[2] = mv_at(scup.checked_sub(w).map(|k| k + (size >> MIN_CU_LOG2)));
        }
        let col = self.ref_pic(0, list)?;
        c[3] = col.map_mv.get(scup).map_or([0, 0], |m| m[0]);
        Ok(c)
    }

    fn skip_motion(&mut self, scup: usize, size: usize, avail: u16) -> Result<(), Error> {
        let i0 = usize::from(self.core.mvp_idx[0]).min(3);
        let c0 = self.get_motion(scup, 0, size, avail)?;
        self.core.refi[0] = 0;
        self.core.mv[0] = c0[i0];
        if self.slice_type == SLICE_P {
            self.core.refi[1] = -1;
            self.core.mv[1] = [0, 0];
        } else {
            let i1 = usize::from(self.core.mvp_idx[1]).min(3);
            let c1 = self.get_motion(scup, 1, size, avail)?;
            self.core.refi[1] = 0;
            self.core.mv[1] = c1[i1];
        }
        Ok(())
    }

    fn inter_motion(&mut self, scup: usize, size: usize, avail: u16) -> Result<(), Error> {
        for l in 0..2 {
            if ((self.core.inter_dir + 1) >> l) & 1 == 1 {
                let c = self.get_motion(scup, l, size, avail)?;
                let p = c[usize::from(self.core.mvp_idx[l]).min(3)];
                let d = self.core.mvd[l];
                self.core.mv[l] = [p[0].wrapping_add(d[0]), p[1].wrapping_add(d[1])];
            } else {
                self.core.refi[l] = -1;
                self.core.mv[l] = [0, 0];
            }
        }
        Ok(())
    }

    /// `xevd_get_mv_dir`: temporal direct mode from the first backward
    /// reference, at the bottom-right 4×4 unit of the coding unit.
    fn mv_dir(&mut self, scu_br: usize) -> Result<(), Error> {
        let r1 = self.refp[0][1];
        let r0 = self.refp[0][0];
        let col = self.ref_pic(0, 1)?;
        let mvc = col.map_mv.get(scu_br).map_or([0, 0], |m| m[0]);
        let dpoc_co = r1.poc.wrapping_sub(col.list_poc[0]) as i32;
        let poc = self.poc as u32;
        let dpoc_l0 = poc.wrapping_sub(r0.poc) as i32;
        let dpoc_l1 = r1.poc.wrapping_sub(poc) as i32;
        if dpoc_co == 0 {
            self.core.mv = [[0; 2]; 2];
        } else {
            for k in 0..2 {
                let m = i32::from(mvc[k]);
                self.core.mv[0][k] = dpoc_l0.wrapping_mul(m).wrapping_div(dpoc_co) as i16;
                self.core.mv[1][k] = dpoc_l1.wrapping_neg().wrapping_mul(m).wrapping_div(dpoc_co) as i16;
            }
        }
        Ok(())
    }

    /// `xevd_mc`.
    fn mc(&self, x: usize, y: usize, size: usize, pred: &mut [Vec<i16>; 3], pred1: &mut [Vec<i16>; 3]) -> Result<(), Error> {
        let refi = self.core.refi;
        let mv = self.core.mv;
        let (x4, y4, w4) = ((x as i32) << 2, (y as i32) << 2, (size as i32) << 2);
        let min = -(128 << 2);
        let max_x = (self.seq.w as i32 - 1 + 128) << 2;
        let max_y = (self.seq.h as i32 - 1 + 128) << 2;
        let mut mv_t = mv;
        for l in 0..2 {
            if refi[l] >= 0 {
                let (mx, my) = (i32::from(mv[l][0]), i32::from(mv[l][1]));
                if x4 + mx < min {
                    mv_t[l][0] = (min - x4) as i16;
                }
                if y4 + my < min {
                    mv_t[l][1] = (min - y4) as i16;
                }
                if x4 + mx + w4 - 4 > max_x {
                    mv_t[l][0] = (max_x - x4 - w4 + 4) as i16;
                }
                if y4 + my + w4 - 4 > max_y {
                    mv_t[l][1] = (max_y - y4 - w4 + 4) as i16;
                }
            }
        }
        let mut bidx = 0;
        let run = |pic: &Pic, l: usize, out: &mut [Vec<i16>; 3]| {
            let qx = i64::from(x4 + i32::from(mv_t[l][0]));
            let qy = i64::from(y4 + i32::from(mv_t[l][1]));
            for (comp, p) in pic.planes.iter().enumerate() {
                let rp = RefPlane { data: &p.data, stride: p.stride, w: p.w, h: p.h };
                if comp == 0 {
                    dsp::mc_luma(&rp, qx, qy, mv[l], size, size, self.bdl(), &mut out[0]);
                } else {
                    dsp::mc_chroma(&rp, qx, qy, mv[l], size / 2, size / 2, self.bdc(), &mut out[comp]);
                }
            }
        };
        if refi[0] >= 0 {
            run(self.ref_pic(refi[0], 0)?, 0, pred);
            bidx += 1;
        }
        if refi[0] >= 0 && refi[1] >= 0 {
            let p0 = self.ref_pic(refi[0], 0)?.poc;
            let p1 = self.ref_pic(refi[1], 1)?.poc;
            if p0 == p1 && mv_t[0] == mv_t[1] {
                return Ok(());
            }
        }
        if refi[1] >= 0 {
            let pic = self.ref_pic(refi[1], 1)?;
            if bidx == 0 {
                run(pic, 1, pred);
            } else {
                run(pic, 1, pred1);
            }
            bidx += 1;
        }
        if bidx == 2 {
            for (a, b) in pred.iter_mut().zip(pred1.iter()) {
                for (p, q) in a.iter_mut().zip(b.iter()) {
                    *p = ((i32::from(*p) + i32::from(*q) + 1) >> 1) as i16;
                }
            }
        }
        Ok(())
    }

    /// `xevd_get_nbr_b` for component `comp`.
    fn nbr(&self, x: usize, y: usize, size: usize, scup: usize, comp: usize) -> Nbr {
        let s = &self.seq;
        let w_scu = s.w_scu as isize;
        let p = scup as isize;
        let (cuw, unit, ox, oy) = if comp == 0 { (size, 4, x, y) } else { (size / 2, 2, x / 2, y / 2) };
        let n_units = 2 * (size >> MIN_CU_LOG2);
        let (x_scu, y_scu) = (x >> MIN_CU_LOG2, y >> MIN_CU_LOG2);
        // Intra neighbours use the luma bit depth for the default value.
        let dflt = 1i32 << (self.bdl() - 1);
        let plane = self.cur.planes.get(comp);
        let at = |px: isize, py: isize| -> i32 {
            match plane {
                Some(pl) if px >= 0 && py >= 0 => i32::from(pl.data.get(py as usize * pl.stride + px as usize).copied().unwrap_or(0)),
                _ => 0,
            }
        };
        let ok = |m: u32| m & COD != 0 && (!self.cip || m & IF != 0);
        let mut up = vec![dflt; 1 + n_units * unit + 1];
        let mut left = vec![dflt; 1 + n_units * unit + 1];
        let (ox, oy) = (ox as isize, oy as isize);
        // Up-left corner.
        let ul = s.scu(p - w_scu - 1);
        let corner = if x_scu > 0 && y_scu > 0 && ul & COD != 0 && (!self.cip || ul & IF != 0) { at(ox - 1, oy - 1) } else { dflt };
        up[0] = corner;
        left[0] = corner;
        for i in 0..n_units {
            let avail = y_scu > 0 && x_scu + i < s.w_scu && ok(s.scu(p - w_scu + i as isize));
            if avail {
                for j in 0..unit {
                    up[1 + i * unit + j] = at(ox + (i * unit + j) as isize, oy - 1);
                }
            }
            let avail = x_scu > 0 && y_scu + i < s.h_scu && ok(s.scu(p - 1 + i as isize * w_scu));
            if avail {
                for j in 0..unit {
                    left[1 + i * unit + j] = at(ox - 1, oy + (i * unit + j) as isize);
                }
            }
        }
        let _ = cuw;
        Nbr { left, up }
    }

    /// `xevd_set_dec_info`.
    fn set_dec_info(&mut self, scup: usize, size: usize) {
        let c = &self.core;
        let n = size >> MIN_CU_LOG2;
        let w_scu = self.seq.w_scu;
        let intra = c.pred_mode == MODE_INTRA;
        for j in 0..n {
            for i in 0..n {
                let k = scup + j * w_scu + i;
                if let Some(m) = self.seq.map_scu.get_mut(k) {
                    let mut v = *m;
                    if c.pred_mode == MODE_SKIP {
                        v |= SF;
                    } else {
                        v &= !SF;
                    }
                    if c.is_coef_sub0[0] {
                        v |= CBFL;
                    } else {
                        v &= !CBFL;
                    }
                    v = (v & 0xFF80_7F80) | ((c.qp as u32 & 0x7F) << 16) | (u32::from(intra) << 15);
                    *m = v;
                }
                if let Some(r) = self.cur.map_refi.get_mut(k) {
                    *r = c.refi;
                }
                if let Some(mv) = self.cur.map_mv.get_mut(k) {
                    *mv = c.mv;
                }
                if let Some(ipm) = self.seq.map_ipm.get_mut(k) {
                    *ipm = c.ipm[0] as i8;
                }
            }
        }
    }

    /// The deblocking pass over the picture, horizontal edges first, then
    /// vertical ones (`xevd_deblock`, one task).
    fn deblock(&mut self, cus: &[(usize, usize, usize)], hor: bool) {
        let w_scu = self.seq.w_scu;
        let rows_per_ctu = CTU >> MIN_CU_LOG2;
        let mut i = 0usize;
        for row in 0..self.seq.h_lcu {
            let t = row * rows_per_ctu;
            let b = ((row + 1) * rows_per_ctu).min(self.seq.h_scu);
            for k in t * w_scu..b * w_scu {
                if let Some(m) = self.seq.map_scu.get_mut(k) {
                    *m &= !COD;
                }
            }
            // Coding units are stored in decoding order, CTU row by row.
            while let Some(&(x, y, size)) = cus.get(i) {
                if y / CTU != row {
                    break;
                }
                if hor {
                    self.deblock_cu_hor(x, y, size);
                } else {
                    self.deblock_cu_ver(x, y, size);
                }
                i += 1;
            }
        }
    }

    fn edge_strength(&self, a: usize, b: usize) -> &'static [u8; 52] {
        let m0 = self.seq.map_scu.get(a).copied().unwrap_or(0);
        let m1 = self.seq.map_scu.get(b).copied().unwrap_or(0);
        let idx = if m0 & IF != 0 || m1 & IF != 0 {
            0
        } else if m0 & CBFL != 0 || m1 & CBFL != 0 {
            1
        } else {
            let r0 = self.cur.map_refi.get(a).copied().unwrap_or([-1, -1]);
            let r1 = self.cur.map_refi.get(b).copied().unwrap_or([-1, -1]);
            let v0 = self.cur.map_mv.get(a).copied().unwrap_or_default();
            let v1 = self.cur.map_mv.get(b).copied().unwrap_or_default();
            let mvs = |r: [i8; 2], v: [[i16; 2]; 2]| -> [[i32; 2]; 2] {
                let mut o = [[0i32; 2]; 2];
                for l in 0..2 {
                    if r[l] >= 0 {
                        o[l] = [i32::from(v[l][0]), i32::from(v[l][1])];
                    }
                }
                o
            };
            let (a0, a1) = (mvs(r0, v0), mvs(r1, v1));
            let far = |p: [i32; 2], q: [i32; 2]| (p[0] - q[0]).abs() >= 4 || (p[1] - q[1]).abs() >= 4;
            if r0[0] == r1[0] && r0[1] == r1[1] {
                if far(a0[0], a1[0]) || far(a0[1], a1[1]) { 2 } else { 3 }
            } else if r0[0] == r1[1] && r0[1] == r1[0] {
                if far(a0[0], a1[1]) || far(a0[1], a1[0]) { 2 } else { 3 }
            } else {
                2
            }
        };
        &DF_ST[idx]
    }

    fn chroma_st(&self, tbl: &[u8; 52], qp: i32) -> (i32, i32) {
        let bdc = self.bdc() as i32;
        let lo = -6 * (bdc - 8);
        let st = |off: i32, i: usize| -> i32 {
            let q = (qp + off).clamp(lo, 57);
            let m = (self.chroma_qp)(i, q);
            usize::try_from(m).ok().and_then(|m| tbl.get(m)).map_or(0, |&v| i32::from(v)) << (bdc - 8)
        };
        (st(self.qp_u_offset, 0), st(self.qp_v_offset, 1))
    }

    fn set_cod(&mut self, x: usize, y: usize, size: usize) {
        let w_scu = self.seq.w_scu;
        let n = size >> MIN_CU_LOG2;
        let p = (y >> MIN_CU_LOG2) * w_scu + (x >> MIN_CU_LOG2);
        for j in 0..n {
            for i in 0..n {
                if let Some(m) = self.seq.map_scu.get_mut(p + j * w_scu + i) {
                    *m |= COD;
                }
            }
        }
    }

    /// `xevd_deblock_cu_hor`.
    fn deblock_cu_hor(&mut self, x: usize, y: usize, size: usize) {
        let w_scu = self.seq.w_scu;
        let t = (y >> MIN_CU_LOG2) * w_scu + (x >> MIN_CU_LOG2);
        if y > 0 {
            for i in 0..size >> MIN_CU_LOG2 {
                let tbl = self.edge_strength(t + i, t + i - w_scu);
                let qp = ((self.seq.map_scu.get(t + i).copied().unwrap_or(0) >> 16) & 0x7F) as i32;
                let st = i32::from(tbl.get(qp as usize).copied().unwrap_or(0)) << (self.bdl() - 8);
                let (st_u, st_v) = if self.seq.chroma { self.chroma_st(tbl, qp) } else { (0, 0) };
                let (bdl, bdc) = (self.bdl(), self.bdc());
                if st != 0
                    && let Some(pl) = self.cur.planes.get_mut(0)
                {
                    let s = pl.stride;
                    let x0 = x + 4 * i;
                    dsp::deblock_luma(&mut pl.data, |k, j| (y as isize + k).try_into().ok().map(|r: usize| r * s + x0 + j), st, bdl);
                }
                if st_u != 0 || st_v != 0 {
                    for (comp, stc) in [(1, st_u), (2, st_v)] {
                        if stc == 0 {
                            continue;
                        }
                        if let Some(pl) = self.cur.planes.get_mut(comp) {
                            let s = pl.stride;
                            let (cx, cy) = (x / 2 + 2 * i, y / 2);
                            dsp::deblock_chroma(&mut pl.data, |k, j| (cy as isize + k).try_into().ok().map(|r: usize| r * s + cx + j), 2, stc, bdc);
                        }
                    }
                }
            }
        }
        self.set_cod(x, y, size);
    }

    /// `xevd_deblock_cu_ver`.
    fn deblock_cu_ver(&mut self, x: usize, y: usize, size: usize) {
        let w_scu = self.seq.w_scu;
        let n = size >> MIN_CU_LOG2;
        let t = (y >> MIN_CU_LOG2) * w_scu + (x >> MIN_CU_LOG2);
        let cod = |s: &Self, k: usize| s.seq.map_scu.get(k).is_some_and(|m| m & COD != 0);
        let mut edges = Vec::new();
        if x > 0 && cod(self, t - 1) {
            edges.push((x, 0usize));
        }
        if x + size < self.seq.w && cod(self, t + n) {
            edges.push((x + size, n));
        }
        for (ex, off) in edges {
            for i in 0..n {
                let cur = t + off + i * w_scu;
                let tbl = self.edge_strength(cur, cur - 1);
                let qp = ((self.seq.map_scu.get(cur).copied().unwrap_or(0) >> 16) & 0x7F) as i32;
                let st = i32::from(tbl.get(qp as usize).copied().unwrap_or(0)) << (self.bdl() - 8);
                let (st_u, st_v) = if self.seq.chroma { self.chroma_st(tbl, qp) } else { (0, 0) };
                let (bdl, bdc) = (self.bdl(), self.bdc());
                if st != 0
                    && let Some(pl) = self.cur.planes.get_mut(0)
                {
                    let s = pl.stride;
                    let y0 = y + 4 * i;
                    dsp::deblock_luma(&mut pl.data, |k, j| (ex as isize + k).try_into().ok().map(|c: usize| (y0 + j) * s + c), st, bdl);
                }
                for (comp, stc) in [(1, st_u), (2, st_v)] {
                    if stc == 0 {
                        continue;
                    }
                    if let Some(pl) = self.cur.planes.get_mut(comp) {
                        let s = pl.stride;
                        let (cx, cy) = (ex / 2, y / 2 + 2 * i);
                        dsp::deblock_chroma(&mut pl.data, |k, j| (cx as isize + k).try_into().ok().map(|c: usize| (cy + j) * s + c), 2, stc, bdc);
                    }
                }
            }
        }
        self.set_cod(x, y, size);
    }
}
