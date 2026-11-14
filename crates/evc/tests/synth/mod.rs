// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! Synthetic EVC Baseline streams for the decoder tests.
//!
//! The arithmetic coder mirrors the reference encoder (`xeve_eco.c`:
//! `xeve_sbac_encode_bin`, `sbac_encode_bin_ep`, `xeve_sbac_encode_bin_trm`,
//! `sbac_carry_propagate`, `sbac_put_byte`, `xeve_sbac_finish`). The syntax
//! writers produce what the Baseline decoder reads; the coding decisions are
//! random, so every syntax path is exercised.

#![allow(dead_code)]

/// Deterministic xorshift generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }

    /// True with probability `p` percent.
    pub fn pct(&mut self, p: u32) -> bool {
        self.below(100) < p
    }
}

/// MSB-first bit writer.
#[derive(Default)]
pub struct BitWriter {
    pub out: Vec<u8>,
    cur: u32,
    n: u32,
}

impl BitWriter {
    pub fn put(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.bit((v >> i) & 1);
        }
    }

    pub fn bit(&mut self, b: u32) {
        self.cur = (self.cur << 1) | (b & 1);
        self.n += 1;
        if self.n == 8 {
            self.out.push(self.cur as u8);
            self.cur = 0;
            self.n = 0;
        }
    }

    pub fn flag(&mut self, b: bool) {
        self.bit(u32::from(b));
    }

    pub fn ue(&mut self, v: u32) {
        let v1 = u64::from(v) + 1;
        let len = 64 - v1.leading_zeros();
        for _ in 0..len - 1 {
            self.bit(0);
        }
        for i in (0..len).rev() {
            self.bit(((v1 >> i) & 1) as u32);
        }
    }

    pub fn se(&mut self, v: i32) {
        let u = if v > 0 { 2 * v as u32 - 1 } else { 2 * v.unsigned_abs() };
        self.ue(u);
    }

    pub fn aligned(&self) -> bool {
        self.n == 0
    }

    pub fn align_zero(&mut self) {
        while self.n != 0 {
            self.bit(0);
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.align_zero();
        self.out
    }
}

const PROB_INIT: u16 = 512;

/// Context models, in the layout of the decoder (`sbac::Ctx`).
#[derive(Clone)]
pub struct Models {
    split_cu_flag: u16,
    run: [u16; 4],
    last: [u16; 2],
    level: [u16; 4],
    cbf_luma: u16,
    cbf_cb: u16,
    cbf_cr: u16,
    cbf_all: u16,
    pred_mode: u16,
    direct_mode_flag: u16,
    inter_dir: [u16; 2],
    intra_dir: [u16; 2],
    mvp_idx: [u16; 3],
    mvd: u16,
    refi: [u16; 2],
    delta_qp: u16,
    skip_flag: u16,
}

impl Models {
    fn new() -> Self {
        Self {
            split_cu_flag: PROB_INIT,
            run: [PROB_INIT; 4],
            last: [PROB_INIT; 2],
            level: [PROB_INIT; 4],
            cbf_luma: PROB_INIT,
            cbf_cb: PROB_INIT,
            cbf_cr: PROB_INIT,
            cbf_all: PROB_INIT,
            pred_mode: PROB_INIT,
            direct_mode_flag: PROB_INIT,
            inter_dir: [PROB_INIT; 2],
            intra_dir: [PROB_INIT; 2],
            mvp_idx: [PROB_INIT; 3],
            mvd: PROB_INIT,
            refi: [PROB_INIT; 2],
            delta_qp: PROB_INIT,
            skip_flag: PROB_INIT,
        }
    }
}

/// The SBAC encoder of `xeve_eco.c`.
pub struct SbacEnc {
    bw: BitWriter,
    range: u32,
    code: u32,
    code_bits: i32,
    pending_byte: u32,
    is_pending_byte: bool,
    stacked_ff: u32,
    stacked_zero: u32,
    pub m: Models,
}

fn encode_model(e: &mut SbacEnc, bin: u32, model: u16) -> u16 {
    let mut state = u32::from(model >> 1);
    let mut mps = u32::from(model & 1);
    let lps = ((state * e.range) >> 9).max(437);
    e.range -= lps;
    if bin != mps {
        if e.range >= lps {
            e.code += e.range;
            e.range = lps;
        }
        state += (512 - state + 16) >> 5;
        if state > 256 {
            mps = 1 - mps;
            state = 512 - state;
        }
    } else {
        state -= (state + 16) >> 5;
    }
    e.renorm();
    ((state << 1) + mps) as u16
}

macro_rules! enc {
    ($e:expr, $bin:expr, $($field:tt)+) => {{
        let m = $e.m.$($field)+;
        let n = encode_model($e, $bin, m);
        $e.m.$($field)+ = n;
    }};
}

impl SbacEnc {
    pub fn new(bw: BitWriter) -> Self {
        Self { bw, range: 16384, code: 0, code_bits: 11, pending_byte: 0, is_pending_byte: false, stacked_ff: 0, stacked_zero: 0, m: Models::new() }
    }

    fn put_byte(&mut self, b: u32) {
        if self.is_pending_byte {
            if self.pending_byte == 0 {
                self.stacked_zero += 1;
            } else {
                while self.stacked_zero > 0 {
                    self.bw.put(0, 8);
                    self.stacked_zero -= 1;
                }
                self.bw.put(self.pending_byte, 8);
            }
        }
        self.pending_byte = b;
        self.is_pending_byte = true;
    }

    fn carry(&mut self) {
        let out = self.code >> 17;
        self.code &= (1 << 17) - 1;
        if out < 0xFF {
            while self.stacked_ff != 0 {
                self.put_byte(0xFF);
                self.stacked_ff -= 1;
            }
            self.put_byte(out);
        } else if out > 0xFF {
            self.pending_byte += 1;
            while self.stacked_ff != 0 {
                self.put_byte(0x00);
                self.stacked_ff -= 1;
            }
            self.put_byte(out & 0xFF);
        } else {
            self.stacked_ff += 1;
        }
    }

    fn renorm(&mut self) {
        while self.range < 8192 {
            self.range <<= 1;
            self.code <<= 1;
            self.code_bits -= 1;
            if self.code_bits == 0 {
                self.carry();
                self.code_bits = 8;
            }
        }
    }

    pub fn ep(&mut self, bin: u32) {
        self.range >>= 1;
        if bin != 0 {
            self.code += self.range;
        }
        self.range <<= 1;
        self.code <<= 1;
        self.code_bits -= 1;
        if self.code_bits == 0 {
            self.carry();
            self.code_bits = 8;
        }
    }

    pub fn trm(&mut self, bin: u32) {
        self.range -= 1;
        if bin != 0 {
            self.code += self.range;
            self.range = 1;
        }
        self.renorm();
    }

    pub fn finish(mut self) -> Vec<u8> {
        let mut tmp = (self.code + self.range - 1) & (0xFFFF_FFFFu32 << 14);
        if tmp < self.code {
            tmp += 8192;
        }
        self.code = tmp << self.code_bits;
        self.carry();
        self.code <<= 8;
        self.carry();
        while self.stacked_zero > 0 {
            self.bw.put(0, 8);
            self.stacked_zero -= 1;
        }
        if self.pending_byte != 0 {
            self.bw.put(self.pending_byte, 8);
        } else if self.code_bits < 4 {
            self.bw.put(0, (4 - self.code_bits) as u32);
            self.bw.align_zero();
        }
        self.bw.finish()
    }

    // Syntax elements.

    /// `sbac_write_unary_sym` over models `base..base + num`.
    fn unary(&mut self, sym: u32, which: Unary) {
        let mut idx = 0usize;
        self.unary_bin(which, idx, u32::from(sym != 0));
        if sym == 0 {
            return;
        }
        let mut s = sym;
        while s > 0 {
            s -= 1;
            if idx + 1 < which.num() {
                idx += 1;
            }
            self.unary_bin(which, idx, u32::from(s != 0));
        }
    }

    fn unary_bin(&mut self, which: Unary, idx: usize, bin: u32) {
        match which {
            Unary::Run(c) => enc!(self, bin, run[usize::from(c) * 2 + idx]),
            Unary::Level(c) => enc!(self, bin, level[usize::from(c) * 2 + idx]),
            Unary::IntraDir => enc!(self, bin, intra_dir[idx]),
            Unary::DeltaQp => enc!(self, bin, delta_qp),
        }
    }

    pub fn split(&mut self, b: bool) {
        enc!(self, u32::from(b), split_cu_flag);
    }

    pub fn mvp_idx(&mut self, v: u32) {
        for i in 0..3u32 {
            let b = u32::from(i != v);
            enc!(self, b, mvp_idx[i as usize]);
            if b == 0 {
                break;
            }
        }
    }

    fn abs_mvd(&mut self, v: u32) {
        if v == 0 {
            enc!(self, 1, mvd);
            return;
        }
        enc!(self, 0, mvd);
        let mut len = 1u32;
        while (1u64 << (len + 1)) - 1 <= u64::from(v) {
            len += 1;
        }
        for i in 0..len {
            let b = u32::from(i == len - 1);
            if i == 0 {
                enc!(self, b, mvd);
            } else {
                self.ep(b);
            }
        }
        let rest = v - ((1 << len) - 1);
        for i in (0..len).rev() {
            self.ep((rest >> i) & 1);
        }
    }

    pub fn mvd(&mut self, d: [i16; 2]) {
        for v in d {
            self.abs_mvd(u32::from(v.unsigned_abs()));
            if v != 0 {
                self.ep(u32::from(v < 0));
            }
        }
    }

    pub fn refi(&mut self, r: u32, num_refp: u32) {
        if num_refp > 1 {
            enc!(self, u32::from(r > 0), refi[0]);
            if r > 0 && num_refp > 2 {
                enc!(self, u32::from(r > 1), refi[1]);
                if r > 1 {
                    let mut k = 2;
                    while k < num_refp - 1 {
                        self.ep(u32::from(k < r));
                        if k >= r {
                            break;
                        }
                        k += 1;
                    }
                }
            }
        }
    }

    pub fn dqp(&mut self, dqp: i32) {
        self.unary(dqp.unsigned_abs(), Unary::DeltaQp);
        if dqp != 0 {
            self.ep(u32::from(dqp < 0));
        }
    }
}

#[derive(Clone, Copy)]
enum Unary {
    Run(bool),
    Level(bool),
    IntraDir,
    DeltaQp,
}

impl Unary {
    fn num(self) -> usize {
        match self {
            Unary::DeltaQp => 1,
            _ => 2,
        }
    }
}

// --- High-level stream description -----------------------------------------

#[derive(Clone, Debug)]
pub struct SeqCfg {
    pub width: u32,
    pub height: u32,
    pub chroma: bool,
    pub bit_depth: u32,
    pub log2_sub_gop_length: u32,
    pub max_num_ref_pics: u32,
    pub crop: Option<[u32; 4]>,
    pub cu_qp_delta: bool,
    pub cip: bool,
}

impl Default for SeqCfg {
    fn default() -> Self {
        Self { width: 64, height: 64, chroma: true, bit_depth: 8, log2_sub_gop_length: 0, max_num_ref_pics: 1, crop: None, cu_qp_delta: false, cip: false }
    }
}

pub const SLICE_B: u32 = 0;
pub const SLICE_P: u32 = 1;
pub const SLICE_I: u32 = 2;

fn nal_header(bw: &mut BitWriter, nal_type: u32, tid: u32) {
    bw.bit(0);
    bw.put(nal_type + 1, 6);
    bw.put(tid, 3);
    bw.put(0, 5);
    bw.bit(0);
}

pub fn sps(c: &SeqCfg) -> Vec<u8> {
    let mut bw = BitWriter::default();
    nal_header(&mut bw, 24, 0);
    bw.ue(0); // sps_seq_parameter_set_id
    bw.put(0, 8); // profile_idc: Baseline
    bw.put(30, 8); // level_idc
    bw.put(0, 32); // toolset_idc_h
    bw.put(0, 32); // toolset_idc_l
    bw.ue(u32::from(c.chroma));
    bw.ue(c.width);
    bw.ue(c.height);
    bw.ue(c.bit_depth - 8);
    bw.ue(c.bit_depth - 8);
    for _ in 0..13 {
        bw.bit(0); // btt, suco, the Main tool flags, rpl, pocs, …
    }
    bw.ue(c.log2_sub_gop_length);
    if c.log2_sub_gop_length == 0 {
        bw.ue(0); // log2_ref_pic_gap_length
    }
    bw.ue(c.max_num_ref_pics);
    bw.flag(c.crop.is_some());
    if let Some(cr) = c.crop {
        for v in cr {
            bw.ue(v);
        }
    }
    if c.chroma {
        bw.bit(0); // chroma_qp_table_present_flag
    }
    bw.bit(0); // vui_parameters_present_flag
    bw.bit(1); // rbsp stop bit
    bw.finish()
}

pub fn pps(c: &SeqCfg) -> Vec<u8> {
    let mut bw = BitWriter::default();
    nal_header(&mut bw, 25, 0);
    bw.ue(0); // pps_pic_parameter_set_id
    bw.ue(0); // pps_seq_parameter_set_id
    bw.ue(0); // num_ref_idx_default_active_minus1[0]
    bw.ue(0); // num_ref_idx_default_active_minus1[1]
    bw.ue(0); // additional_lt_poc_lsb_len
    bw.bit(0); // rpl1_idx_present_flag
    bw.bit(1); // single_tile_in_pic_flag
    bw.ue(0); // tile_id_len_minus1
    bw.bit(0); // explicit_tile_id_flag
    bw.bit(0); // pic_dra_enabled_flag
    bw.bit(0); // arbitrary_slice_present_flag
    bw.flag(c.cip);
    bw.flag(c.cu_qp_delta);
    if c.cu_qp_delta {
        bw.ue(0); // log2_cu_qp_delta_area_minus6
    }
    bw.bit(1);
    bw.finish()
}

/// How the coding units of a picture are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// Random decisions.
    Random,
    /// 64×64 (or the largest fitting) intra DC units without residual.
    FlatIntra,
    /// Skip units that pick the collocated (zero) motion vector of an intra
    /// reference.
    SkipCollocated,
}

#[derive(Clone, Debug)]
pub struct PicCfg {
    pub slice_type: u32,
    pub idr: bool,
    pub tid: u32,
    /// Active references per list, as the decoder derives them.
    pub num_refp: [u32; 2],
    pub qp: u32,
    pub qp_u_offset: i32,
    pub qp_v_offset: i32,
    pub deblock: bool,
    pub content: Content,
}

pub fn slice(c: &SeqCfg, p: &PicCfg, rng: &mut Rng) -> Vec<u8> {
    let mut bw = BitWriter::default();
    let nut = if p.idr { 1 } else { 0 };
    nal_header(&mut bw, nut, p.tid);
    bw.ue(0); // slice_pic_parameter_set_id
    bw.ue(p.slice_type);
    if p.idr {
        bw.bit(0); // no_output_of_prior_pics_flag
    }
    if p.slice_type != SLICE_I {
        bw.bit(0); // num_ref_idx_active_override_flag
    }
    bw.flag(p.deblock);
    bw.put(p.qp, 6);
    bw.se(p.qp_u_offset);
    bw.se(p.qp_v_offset);
    bw.align_zero();
    let mut e = SbacEnc::new(bw);
    let w_lcu = c.width.div_ceil(64);
    let h_lcu = c.height.div_ceil(64);
    for lcu in 0..w_lcu * h_lcu {
        let x = (lcu % w_lcu) * 64;
        let y = (lcu / w_lcu) * 64;
        tree(&mut e, c, p, rng, x, y, 6);
    }
    e.trm(1);
    e.finish()
}

fn tree(e: &mut SbacEnc, c: &SeqCfg, p: &PicCfg, rng: &mut Rng, x: u32, y: u32, log2: u32) {
    let size = 1 << log2;
    let inside = x + size <= c.width && y + size <= c.height;
    let split = if size >= 8 {
        let s = match p.content {
            Content::Random => !inside || rng.pct(55),
            _ => !inside,
        };
        e.split(s);
        s
    } else {
        false
    };
    if split {
        let half = size / 2;
        for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
            if x + dx < c.width && y + dy < c.height {
                tree(e, c, p, rng, x + dx, y + dy, log2 - 1);
            }
        }
    } else {
        cu(e, c, p, rng, log2);
    }
}

fn cu(e: &mut SbacEnc, c: &SeqCfg, p: &PicCfg, rng: &mut Rng, log2: u32) {
    let b = p.slice_type == SLICE_B;
    let (skip, intra) = match p.content {
        Content::FlatIntra => (false, true),
        Content::SkipCollocated => (true, false),
        Content::Random => {
            if p.slice_type == SLICE_I {
                (false, true)
            } else {
                let r = rng.below(3);
                (r == 0, r == 1)
            }
        }
    };
    if p.slice_type != SLICE_I {
        enc!(e, u32::from(skip), skip_flag);
    }
    if skip {
        let pick = |rng: &mut Rng| if p.content == Content::SkipCollocated { 3 } else { rng.below(4) };
        let i0 = pick(rng);
        e.mvp_idx(i0);
        if b {
            let i1 = pick(rng);
            e.mvp_idx(i1);
        }
        return;
    }
    if p.slice_type != SLICE_I {
        enc!(e, u32::from(intra), pred_mode);
    }
    if intra {
        // Any mode: in a flat picture every intra prediction is flat too.
        e.unary(rng.below(5), Unary::IntraDir);
    } else {
        let direct = b && rng.pct(25);
        if b {
            enc!(e, u32::from(direct), direct_mode_flag);
        }
        if !direct {
            // PRED_L0 = 0, PRED_L1 = 1, PRED_BI = 2.
            let dir = if b { rng.below(3) } else { 0 };
            if b {
                match dir {
                    2 => enc!(e, 0, inter_dir[0]),
                    1 => {
                        enc!(e, 1, inter_dir[0]);
                        enc!(e, 1, inter_dir[1]);
                    }
                    _ => {
                        enc!(e, 1, inter_dir[0]);
                        enc!(e, 0, inter_dir[1]);
                    }
                }
            }
            for l in 0..2 {
                if ((dir + 1) >> l) & 1 == 1 {
                    let n = p.num_refp[l];
                    e.refi(rng.below(n.max(1)), n);
                    e.mvp_idx(rng.below(4));
                    let mut d = [0i16; 2];
                    for v in &mut d {
                        *v = if rng.pct(30) { 0 } else { rng.below(65) as i16 - 32 };
                    }
                    e.mvd(d);
                }
            }
        }
    }
    coefs(e, c, p, rng, log2, intra);
}

fn coefs(e: &mut SbacEnc, c: &SeqCfg, p: &PicCfg, rng: &mut Rng, log2: u32, intra: bool) {
    let flat = p.content == Content::FlatIntra;
    if !intra {
        let any = rng.pct(70);
        enc!(e, u32::from(any), cbf_all);
        if !any {
            return;
        }
    }
    let mut cbf = [false; 3];
    if c.chroma {
        cbf[1] = !flat && rng.pct(40);
        cbf[2] = !flat && rng.pct(40);
        enc!(e, u32::from(cbf[1]), cbf_cb);
        enc!(e, u32::from(cbf[2]), cbf_cr);
    }
    if !intra && !cbf[1] && !cbf[2] {
        cbf[0] = true;
    } else {
        cbf[0] = !flat && rng.pct(60);
        enc!(e, u32::from(cbf[0]), cbf_luma);
    }
    if c.cu_qp_delta && cbf.iter().any(|&f| f) {
        let d = rng.below(7) as i32 - 3;
        e.dqp(d);
    }
    for comp in 0..3 {
        if cbf[comp] {
            let l = if comp == 0 { log2 } else { log2 - 1 };
            run_length(e, rng, l, comp != 0);
        }
    }
}

fn run_length(e: &mut SbacEnc, rng: &mut Rng, log2: u32, chroma: bool) {
    let num = 1u32 << (2 * log2);
    let mut pos = 0u32;
    loop {
        // Runs mostly short, sometimes long, never past the block.
        let max_run = num - 1 - pos;
        let run = if rng.pct(80) { rng.below(4) } else { rng.below(max_run + 1) }.min(max_run);
        e.unary(run, Unary::Run(chroma));
        pos += run;
        let level = if rng.pct(85) { 1 } else { 1 + rng.below(40) };
        e.unary(level - 1, Unary::Level(chroma));
        e.ep(rng.below(2));
        if pos >= num - 1 {
            return;
        }
        pos += 1;
        let last = rng.pct(35);
        enc!(e, u32::from(last), last[usize::from(chroma)]);
        if last {
            return;
        }
    }
}

/// Joins NAL units into the length-prefixed stream format of the reference
/// tools.
pub fn stream(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for n in nals {
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}
