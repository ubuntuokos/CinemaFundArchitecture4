//! Frame decoding: superblock and fragment coding flags, macroblock modes, motion vectors, block
//! quantizers, DCT tokens, DC prediction, reconstruction with motion compensation and the loop
//! filter. A port of FFmpeg n7.1.1 `libavcodec/vp3.c` (LGPL-2.1-or-later) following the Theora
//! specification, with coefficients kept per block instead of FFmpeg's token queues.
//!
//! Planes are kept in the specification's orientation (row 0 is the bottom of the picture);
//! [`crate::Decoder`] flips them on output.

use crate::bits::Bits;
use crate::dsp;
use crate::header::{Info, Setup};
use crate::vlc::Vlc;
use crate::{Chroma, Error, Result};

const MODE_INTER_NO_MV: u8 = 0;
const MODE_INTRA: u8 = 1;
const MODE_INTER_PLUS_MV: u8 = 2;
const MODE_INTER_LAST_MV: u8 = 3;
const MODE_INTER_PRIOR_LAST: u8 = 4;
const MODE_USING_GOLDEN: u8 = 5;
const MODE_GOLDEN_MV: u8 = 6;
const MODE_INTER_FOURMV: u8 = 7;
/// Not coded: copied from the previous frame.
const MODE_COPY: u8 = 8;

const MAX_LONG_RUN: u32 = 4129;

const MODE_ALPHABET: [[u8; 8]; 6] = [
    [MODE_INTER_LAST_MV, MODE_INTER_PRIOR_LAST, MODE_INTER_PLUS_MV, MODE_INTER_NO_MV, MODE_INTRA, MODE_USING_GOLDEN, MODE_GOLDEN_MV, MODE_INTER_FOURMV],
    [MODE_INTER_LAST_MV, MODE_INTER_PRIOR_LAST, MODE_INTER_NO_MV, MODE_INTER_PLUS_MV, MODE_INTRA, MODE_USING_GOLDEN, MODE_GOLDEN_MV, MODE_INTER_FOURMV],
    [MODE_INTER_LAST_MV, MODE_INTER_PLUS_MV, MODE_INTER_PRIOR_LAST, MODE_INTER_NO_MV, MODE_INTRA, MODE_USING_GOLDEN, MODE_GOLDEN_MV, MODE_INTER_FOURMV],
    [MODE_INTER_LAST_MV, MODE_INTER_PLUS_MV, MODE_INTER_NO_MV, MODE_INTER_PRIOR_LAST, MODE_INTRA, MODE_USING_GOLDEN, MODE_GOLDEN_MV, MODE_INTER_FOURMV],
    [MODE_INTER_NO_MV, MODE_INTER_LAST_MV, MODE_INTER_PRIOR_LAST, MODE_INTER_PLUS_MV, MODE_INTRA, MODE_USING_GOLDEN, MODE_GOLDEN_MV, MODE_INTER_FOURMV],
    [MODE_INTER_NO_MV, MODE_USING_GOLDEN, MODE_INTER_LAST_MV, MODE_INTER_PRIOR_LAST, MODE_INTER_PLUS_MV, MODE_INTRA, MODE_GOLDEN_MV, MODE_INTER_FOURMV],
];

const HILBERT: [(usize, usize); 16] =
    [(0, 0), (1, 0), (1, 1), (0, 1), (0, 2), (0, 3), (1, 3), (1, 2), (2, 2), (2, 3), (3, 3), (3, 2), (3, 1), (2, 1), (2, 0), (3, 0)];

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// EOB run tokens 0..=6: `(base, extra bits)`.
const EOB_RUN: [(u32, u32); 7] = [(1, 0), (2, 0), (3, 0), (4, 2), (8, 3), (16, 4), (0, 12)];
const ZERO_RUN_BASE: [u8; 32] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 10, 1, 2];
const ZERO_RUN_BITS: [u8; 32] = [0, 0, 0, 0, 0, 0, 0, 3, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 0, 1];
const COEFF_BITS: [u8; 32] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 3, 4, 5, 6, 10, 1, 1, 1, 1, 1, 1, 1, 2, 2];

/// The coefficient of `token` (7..=31) from its extra bits `v`.
fn token_coeff(token: usize, v: u32) -> i16 {
    let v = v as i32;
    let signed = |base: i32, half: i32| -> i16 {
        let mag = base + (v & (half - 1));
        (if v >= half { -mag } else { mag }) as i16
    };
    match token {
        7 | 8 => 0,
        9 => 1,
        10 => -1,
        11 => 2,
        12 => -2,
        13..=16 => signed(token as i32 - 10, 1),
        17 => signed(7, 2),
        18 => signed(9, 4),
        19 => signed(13, 8),
        20 => signed(21, 16),
        21 => signed(37, 32),
        22 => signed(69, 512),
        23..=29 => signed(1, 1),
        30 | 31 => signed(2, 2),
        _ => 0,
    }
}

/// `RSHIFT`: a shift that rounds half away from zero.
fn rshift(a: i32, b: u32) -> i32 {
    let half = (1 << b) >> 1;
    if a > 0 { (a + half) >> b } else { (a + half - 1) >> b }
}

/// A reference or output frame: the three planes in decoding orientation.
#[derive(Clone)]
pub(crate) struct Frame {
    pub(crate) planes: [Vec<u8>; 3],
}

#[derive(Clone, Copy, Default)]
struct Frag {
    coding: u8,
    qpi: u8,
    dc: i16,
}

/// Sizes derived from the identification header.
#[derive(Clone)]
pub(crate) struct Geometry {
    pub(crate) width: usize,
    pub(crate) height: usize,
    cx: u32,
    cy: u32,
    y_sb_w: usize,
    y_sb_h: usize,
    sb_count: usize,
    u_sb_start: usize,
    v_sb_start: usize,
    mb_w: usize,
    mb_h: usize,
    frag_w: [usize; 2],
    frag_h: [usize; 2],
    frag_start: [usize; 3],
    frag_count: usize,
    /// Fragment index of each position of each superblock, `usize::MAX` outside the plane.
    sb_frags: Vec<usize>,
}

impl Geometry {
    pub(crate) fn new(info: &Info) -> Self {
        let (width, height) = (info.width as usize, info.height as usize);
        let (cx, cy) = match info.chroma {
            Chroma::C420 => (1, 1),
            Chroma::C422 => (1, 0),
            Chroma::C444 => (0, 0),
        };
        let y_sb_w = width.div_ceil(32);
        let y_sb_h = height.div_ceil(32);
        let (c_w, c_h) = (width >> cx, height >> cy);
        let c_sb_w = c_w.div_ceil(32);
        let c_sb_h = c_h.div_ceil(32);
        let y_sb = y_sb_w * y_sb_h;
        let c_sb = c_sb_w * c_sb_h;
        let frag_w = [width / 8, (width / 8) >> cx];
        let frag_h = [height / 8, (height / 8) >> cy];
        let y_frags = frag_w[0] * frag_h[0];
        let c_frags = frag_w[1] * frag_h[1];
        let frag_start = [0, y_frags, y_frags + c_frags];
        let mut g = Self {
            width,
            height,
            cx,
            cy,
            y_sb_w,
            y_sb_h,
            sb_count: y_sb + 2 * c_sb,
            u_sb_start: y_sb,
            v_sb_start: y_sb + c_sb,
            mb_w: width.div_ceil(16),
            mb_h: height.div_ceil(16),
            frag_w,
            frag_h,
            frag_start,
            frag_count: y_frags + 2 * c_frags,
            sb_frags: Vec::new(),
        };
        let mut v = Vec::with_capacity(g.sb_count * 16);
        for plane in 0..3 {
            let (sw, sh) = if plane == 0 { (y_sb_w, y_sb_h) } else { (c_sb_w, c_sb_h) };
            let (fw, fh) = (frag_w[usize::from(plane > 0)], frag_h[usize::from(plane > 0)]);
            for sy in 0..sh {
                for sx in 0..sw {
                    for &(hx, hy) in &HILBERT {
                        let (x, y) = (4 * sx + hx, 4 * sy + hy);
                        v.push(if x < fw && y < fh { frag_start[plane] + y * fw + x } else { usize::MAX });
                    }
                }
            }
        }
        g.sb_frags = v;
        g
    }

    /// `(width, height)` in pixels of plane `p`.
    pub(crate) fn plane_size(&self, p: usize) -> (usize, usize) {
        if p == 0 { (self.width, self.height) } else { (self.width >> self.cx, self.height >> self.cy) }
    }

    pub(crate) fn blank_frame(&self, value: u8) -> Frame {
        let sz = |p| {
            let (w, h) = self.plane_size(p);
            vec![value; w * h]
        };
        Frame { planes: [sz(0), sz(1), sz(2)] }
    }
}

/// Dequantizer tables for one quantizer index: `[inter][plane][raster position transposed]`.
type Qmat = [[[i16; 64]; 3]; 2];

fn transpose(i: usize) -> usize {
    (i >> 3) | ((i & 7) << 3)
}

fn init_dequantizer(setup: &Setup, qps: &[usize; 3], qpi: usize, qmat: &mut [Qmat; 3]) {
    let qp = qps.get(qpi).copied().unwrap_or(0).min(63);
    let ac_scale = i32::from(setup.ac_scale[qp]);
    for inter in 0..2usize {
        for plane in 0..3usize {
            let dc_scale = i32::from(setup.dc_scale[qp]);
            let count = setup.qr_count[inter][plane];
            let mut sum = 0i32;
            let mut qri = 0usize;
            while qri < count {
                sum += i32::from(setup.qr_size[inter][plane][qri]);
                if qp as i32 <= sum {
                    break;
                }
                qri += 1;
            }
            let qri = qri.min(count.saturating_sub(1));
            let size = i32::from(setup.qr_size[inter][plane][qri]).max(1);
            let qistart = sum - size;
            let bmi = usize::from(setup.qr_base[inter][plane][qri]);
            let bmj = usize::from(setup.qr_base[inter][plane][qri + 1]);
            let empty = [0u8; 64];
            let (mi, mj) = (setup.base_matrix.get(bmi).unwrap_or(&empty), setup.base_matrix.get(bmj).unwrap_or(&empty));
            for i in 0..64 {
                let coeff = (2 * (sum - qp as i32) * i32::from(mi[i]) - 2 * (qistart - qp as i32) * i32::from(mj[i]) + size) / (2 * size);
                let qmin = 8 << (inter + usize::from(i == 0));
                let qscale = if i == 0 { dc_scale } else { ac_scale };
                let v = ((qscale * coeff) / 100 * 4).clamp(qmin, 4096);
                qmat[qpi][inter][plane][transpose(i)] = v as i16;
            }
            // all DC coefficients use the first quantizer's DC step
            qmat[qpi][inter][plane][0] = qmat[0][inter][plane][0];
        }
    }
}

/// A Huffman-coded long run (superblock coding, block quantizers): 1 to 4129 or more.
fn long_run(b: &mut Bits<'_>) -> u32 {
    let mut ones = 0;
    while ones < 6 && b.get1() {
        ones += 1;
    }
    match ones {
        0 => 1,
        1 => 2 + b.get(1),
        2 => 4 + b.get(1),
        3 => 6 + b.get(2),
        4 => 10 + b.get(3),
        5 => 18 + b.get(4),
        _ => 34 + b.get(12),
    }
}

/// A Huffman-coded short run (fragment coding): 1 to 30.
fn short_run(b: &mut Bits<'_>) -> u32 {
    let mut ones = 0;
    while ones < 5 && b.get1() {
        ones += 1;
    }
    match ones {
        0 => 1 + b.get(1),
        1 => 3 + b.get(1),
        2 => 5 + b.get(1),
        3 => 7 + b.get(2),
        4 => 11 + b.get(2),
        _ => 15 + b.get(4),
    }
}

/// The motion vector component code of the specification's table: values -31..=31 with the
/// lengths of FFmpeg's `motion_vector_vlc_table`, assigned left to right.
pub(crate) fn mv_vlc() -> Result<Vlc> {
    let mut e: Vec<(u8, i16)> = vec![(3, 0), (3, 1), (3, -1)];
    for m in 2i16..=31 {
        let len = match m {
            2..=3 => 4,
            4..=7 => 6,
            8..=15 => 7,
            _ => 8,
        };
        e.push((len, m));
        e.push((len, -m));
    }
    Vlc::new(&e)
}

/// The decoding state that lives for one frame, kept to reuse its allocations.
pub(crate) struct FrameDecoder {
    geo: Geometry,
    mv: Vlc,
    frags: Vec<Frag>,
    sb_coding: Vec<u8>,
    mb_coding: Vec<u8>,
    motion: [Vec<[i8; 2]>; 2],
    /// Fragments in coding order: all of plane 0, then 1, then 2 (each in superblock order).
    coded: Vec<usize>,
    plane_end: [usize; 3],
    keyframe: bool,
    qps: [usize; 3],
    nqps: usize,
    qmat: [Qmat; 3],
    /// Coefficient tokens by zigzag level per coded block.
    coefs: Vec<i16>,
    /// The block's first token was an end of block: it has no coefficients but the DC.
    dc_only: Vec<bool>,
    /// A zero run of the block's tokens ran past the last coefficient: FFmpeg stops dequantizing
    /// there, before it sets the block's DC (which then counts as 0).
    overflow: Vec<bool>,
}

impl FrameDecoder {
    pub(crate) fn new(info: &Info) -> Result<Self> {
        let geo = Geometry::new(info);
        let n = geo.frag_count;
        Ok(Self {
            mv: mv_vlc()?,
            frags: vec![Frag::default(); n],
            sb_coding: vec![0; geo.sb_count],
            mb_coding: vec![MODE_COPY; geo.mb_w * geo.mb_h],
            motion: [vec![[0; 2]; geo.frag_w[0] * geo.frag_h[0]], vec![[0; 2]; geo.frag_w[1] * geo.frag_h[1]]],
            coded: Vec::with_capacity(n),
            plane_end: [0; 3],
            keyframe: false,
            qps: [0; 3],
            nqps: 1,
            qmat: [[[[0; 64]; 3]; 2]; 3],
            coefs: Vec::new(),
            dc_only: Vec::new(),
            overflow: Vec::new(),
            geo,
        })
    }

    pub(crate) fn geometry(&self) -> &Geometry {
        &self.geo
    }

    /// Decodes a data packet (after its first bit, the packet type, has been checked) into a new
    /// frame. `last` and `golden` are the reference frames.
    pub(crate) fn decode(&mut self, setup: &Setup, packet: &[u8], last: Option<&Frame>, golden: Option<&Frame>) -> Result<(Frame, bool)> {
        let mut b = Bits::new(packet);
        b.skip(1);
        self.keyframe = !b.get1();
        self.qps = [0; 3];
        self.nqps = 0;
        loop {
            let q = b.get(6) as usize;
            if let Some(v) = self.qps.get_mut(self.nqps) {
                *v = q;
            }
            self.nqps += 1;
            if self.nqps >= 3 || !b.get1() {
                break;
            }
        }
        if self.keyframe {
            b.skip(3); // coding type and reserved bits
        } else if last.is_none() || golden.is_none() {
            return Err(Error::Invalid("inter frame without a reference frame"));
        }
        for qpi in 0..self.nqps {
            init_dequantizer(setup, &self.qps, qpi, &mut self.qmat);
        }
        for f in self.frags.iter_mut() {
            *f = Frag::default();
        }
        self.unpack_superblocks(&mut b)?;
        self.unpack_modes(&mut b)?;
        self.unpack_vectors(&mut b)?;
        self.unpack_block_qpis(&mut b)?;
        self.unpack_dct(setup, &mut b)?;
        let mut cur = self.geo.blank_frame(0);
        self.reconstruct(&mut cur, last, golden);
        let limit = i32::from(setup.filter_limit[self.qps[0].min(63)]);
        if limit > 0 {
            self.loop_filter(&mut cur, limit);
        }
        Ok((cur, self.keyframe))
    }

    fn unpack_superblocks(&mut self, b: &mut Bits<'_>) -> Result<()> {
        let n = self.geo.sb_count;
        let mut num_partial = 0usize;
        if self.keyframe {
            self.sb_coding.fill(2);
        } else {
            self.sb_coding.fill(0);
            let mut bit = u32::from(b.get1()) ^ 1;
            let mut run = 0u32;
            let mut cur = 0usize;
            while cur < n && b.left() > 0 {
                bit = if run == MAX_LONG_RUN { u32::from(b.get1()) } else { bit ^ 1 };
                run = long_run(b);
                let r = run as usize;
                if r > n - cur {
                    return Err(Error::Invalid("partially coded superblock run too long"));
                }
                self.sb_coding[cur..cur + r].fill(bit as u8);
                cur += r;
                if bit == 1 {
                    num_partial += r;
                }
            }
            if num_partial < n {
                let mut decoded = 0usize;
                let mut cur = 0usize;
                bit = u32::from(b.get1()) ^ 1;
                run = 0;
                while decoded < n - num_partial && b.left() > 0 {
                    bit = if run == MAX_LONG_RUN { u32::from(b.get1()) } else { bit ^ 1 };
                    run = long_run(b);
                    let mut j = 0;
                    while j < run {
                        let Some(sc) = self.sb_coding.get_mut(cur) else {
                            return Err(Error::Invalid("fully coded superblock run too long"));
                        };
                        if *sc == 0 {
                            *sc = (2 * bit) as u8;
                            j += 1;
                        }
                        cur += 1;
                    }
                    decoded += run as usize;
                }
            }
        }
        // the fragment coding flags
        let (mut bit, mut remaining) = (0u32, 0u32);
        if num_partial > 0 {
            bit = u32::from(b.get1()) ^ 1;
        }
        self.coded.clear();
        self.mb_coding.fill(MODE_COPY);
        let starts = [0, self.geo.u_sb_start, self.geo.v_sb_start];
        let ends = [self.geo.u_sb_start, self.geo.v_sb_start, self.geo.sb_count];
        for plane in 0..3 {
            for sb in starts[plane]..ends[plane] {
                for j in 0..16 {
                    let f = self.geo.sb_frags[sb * 16 + j];
                    if f == usize::MAX {
                        continue;
                    }
                    let mut coded = u32::from(self.sb_coding[sb]);
                    if coded == 1 {
                        if remaining == 0 {
                            bit ^= 1;
                            remaining = short_run(b);
                        }
                        remaining -= 1;
                        coded = bit;
                    }
                    if coded != 0 {
                        self.frags[f].coding = MODE_INTER_NO_MV;
                        self.coded.push(f);
                    } else {
                        self.frags[f].coding = MODE_COPY;
                    }
                }
            }
            self.plane_end[plane] = self.coded.len();
        }
        if self.keyframe {
            for f in self.frags.iter_mut() {
                f.coding = MODE_INTRA;
            }
        }
        Ok(())
    }

    /// The macroblocks in decoding order: `(mb_x, mb_y)` of each in each superblock.
    fn macroblocks(&self) -> Vec<(usize, usize)> {
        let mut v = Vec::with_capacity(self.geo.mb_w * self.geo.mb_h);
        for sb_y in 0..self.geo.y_sb_h {
            for sb_x in 0..self.geo.y_sb_w {
                for j in 0..4 {
                    let mb_x = 2 * sb_x + (j >> 1);
                    let mb_y = 2 * sb_y + (((j >> 1) + j) & 1);
                    if mb_x < self.geo.mb_w && mb_y < self.geo.mb_h {
                        v.push((mb_x, mb_y));
                    }
                }
            }
        }
        v
    }

    /// The luma fragment of block `k` (0..4) of the macroblock.
    fn luma_frag(&self, mb_x: usize, mb_y: usize, k: usize) -> usize {
        (2 * mb_y + (k >> 1)) * self.geo.frag_w[0] + 2 * mb_x + (k & 1)
    }

    fn unpack_modes(&mut self, b: &mut Bits<'_>) -> Result<()> {
        if self.keyframe {
            return Ok(());
        }
        let scheme = b.get(3) as usize;
        let mut custom = [MODE_INTER_NO_MV; 8];
        if scheme == 0 {
            for i in 0..8u8 {
                custom[b.get(3) as usize] = i;
            }
        }
        let alphabet = if scheme == 0 { custom } else { MODE_ALPHABET.get(scheme - 1).copied().unwrap_or(custom) };
        let g = self.geo.clone();
        let mut last_sb = usize::MAX;
        for (mb_x, mb_y) in self.macroblocks() {
            let sb = (mb_y / 2) * g.y_sb_w + mb_x / 2;
            if sb != last_sb {
                if b.left() == 0 {
                    return Err(Error::Invalid("mode data runs past the packet"));
                }
                last_sb = sb;
            }
            let any = (0..4).any(|k| self.frags[self.luma_frag(mb_x, mb_y, k)].coding != MODE_COPY);
            let mbi = mb_y * g.mb_w + mb_x;
            if !any {
                self.mb_coding[mbi] = MODE_INTER_NO_MV;
                continue;
            }
            let mode = if scheme == 7 {
                b.get(3) as u8
            } else {
                // unary: 0, 10, 110, ... 1111110, 1111111
                let mut ones = 0;
                while ones < 7 && b.get1() {
                    ones += 1;
                }
                alphabet[ones]
            };
            self.mb_coding[mbi] = mode;
            for k in 0..4 {
                let f = self.luma_frag(mb_x, mb_y, k);
                if self.frags[f].coding != MODE_COPY {
                    self.frags[f].coding = mode;
                }
            }
            let set = |frags: &mut Vec<Frag>, i: usize| {
                if let Some(f) = frags.get_mut(i)
                    && f.coding != MODE_COPY
                {
                    f.coding = mode;
                }
            };
            let (s1, s2) = (g.frag_start[1], g.frag_start[2]);
            let fw = g.frag_w[1];
            if g.cy == 1 {
                let f = mb_y * fw + mb_x;
                set(&mut self.frags, s1 + f);
                set(&mut self.frags, s2 + f);
            } else if g.cx == 1 {
                for k in 0..2 {
                    let f = (2 * mb_y + k) * fw + mb_x;
                    set(&mut self.frags, s1 + f);
                    set(&mut self.frags, s2 + f);
                }
            } else {
                for k in 0..4 {
                    let f = (2 * mb_y + (k >> 1)) * fw + 2 * mb_x + (k & 1);
                    set(&mut self.frags, s1 + f);
                    set(&mut self.frags, s2 + f);
                }
            }
        }
        Ok(())
    }

    fn unpack_vectors(&mut self, b: &mut Bits<'_>) -> Result<()> {
        if self.keyframe {
            return Ok(());
        }
        let coding_mode = b.get1();
        let g = self.geo.clone();
        let (mut last, mut prior_last) = ((0i32, 0i32), (0i32, 0i32));
        let mut last_sb = usize::MAX;
        for (mb_x, mb_y) in self.macroblocks() {
            let sb = (mb_y / 2) * g.y_sb_w + mb_x / 2;
            if sb != last_sb {
                if b.left() == 0 {
                    return Err(Error::Invalid("motion vectors run past the packet"));
                }
                last_sb = sb;
            }
            let mode = self.mb_coding[mb_y * g.mb_w + mb_x];
            if mode == MODE_COPY {
                continue;
            }
            let read = |b: &mut Bits<'_>| -> Result<(i32, i32)> {
                if coding_mode {
                    let fixed = |v: u32| -> i32 {
                        let m = (v >> 1) as i32;
                        if v & 1 != 0 { -m } else { m }
                    };
                    let x = fixed(b.get(6));
                    let y = fixed(b.get(6));
                    Ok((x, y))
                } else {
                    let x = self.mv.read(b).ok_or(Error::Invalid("motion vector code"))?;
                    let y = self.mv.read(b).ok_or(Error::Invalid("motion vector code"))?;
                    Ok((i32::from(x), i32::from(y)))
                }
            };
            let mut mvs = [(0i32, 0i32); 4];
            match mode {
                MODE_GOLDEN_MV | MODE_INTER_PLUS_MV => {
                    mvs[0] = read(b)?;
                    if mode == MODE_INTER_PLUS_MV {
                        prior_last = last;
                        last = mvs[0];
                    }
                }
                MODE_INTER_FOURMV => {
                    prior_last = last;
                    for k in 0..4 {
                        let f = self.luma_frag(mb_x, mb_y, k);
                        if self.frags[f].coding != MODE_COPY {
                            mvs[k] = read(b)?;
                            last = mvs[k];
                        }
                    }
                }
                MODE_INTER_LAST_MV => mvs[0] = last,
                MODE_INTER_PRIOR_LAST => {
                    mvs[0] = prior_last;
                    prior_last = last;
                    last = mvs[0];
                }
                _ => {}
            }
            let four = mode == MODE_INTER_FOURMV;
            for k in 0..4 {
                let f = self.luma_frag(mb_x, mb_y, k);
                let v = if four { mvs[k] } else { mvs[0] };
                if let Some(m) = self.motion[0].get_mut(f) {
                    *m = [v.0 as i8, v.1 as i8];
                }
            }
            let half = |v: i32| (v >> 1) | (v & 1);
            let fw = g.frag_w[1];
            if g.cy == 1 {
                let (mut x, mut y) = mvs[0];
                if four {
                    x = rshift(mvs[0].0 + mvs[1].0 + mvs[2].0 + mvs[3].0, 2);
                    y = rshift(mvs[0].1 + mvs[1].1 + mvs[2].1 + mvs[3].1, 2);
                }
                if let Some(m) = self.motion[1].get_mut(mb_y * fw + mb_x) {
                    *m = [half(x) as i8, half(y) as i8];
                }
            } else if g.cx == 1 {
                let (mut a, mut c) = (mvs[0], mvs[0]);
                if four {
                    a = (rshift(mvs[0].0 + mvs[1].0, 1), rshift(mvs[0].1 + mvs[1].1, 1));
                    c = (rshift(mvs[2].0 + mvs[3].0, 1), rshift(mvs[2].1 + mvs[3].1, 1));
                }
                for (k, v) in [a, c].into_iter().enumerate() {
                    if let Some(m) = self.motion[1].get_mut((2 * mb_y + k) * fw + mb_x) {
                        *m = [half(v.0) as i8, v.1 as i8];
                    }
                }
            } else {
                for k in 0..4 {
                    let v = if four { mvs[k] } else { mvs[0] };
                    if let Some(m) = self.motion[1].get_mut((2 * mb_y + (k >> 1)) * fw + 2 * mb_x + (k & 1)) {
                        *m = [v.0 as i8, v.1 as i8];
                    }
                }
            }
        }
        Ok(())
    }

    fn unpack_block_qpis(&mut self, b: &mut Bits<'_>) -> Result<()> {
        let total = self.coded.len();
        let mut num_blocks = total;
        let mut qpi = 0;
        while qpi + 1 < self.nqps && num_blocks > 0 {
            let (mut i, mut decoded, mut at_qpi) = (0usize, 0usize, 0usize);
            let mut bit = u32::from(b.get1()) ^ 1;
            let mut run = 0u32;
            loop {
                bit = if run == MAX_LONG_RUN { u32::from(b.get1()) } else { bit ^ 1 };
                run = long_run(b);
                decoded += run as usize;
                if bit == 0 {
                    at_qpi += run as usize;
                }
                let mut j = 0;
                while j < run {
                    let Some(&f) = self.coded.get(i) else {
                        return Err(Error::Invalid("block quantizer run too long"));
                    };
                    if usize::from(self.frags[f].qpi) == qpi {
                        self.frags[f].qpi += bit as u8;
                        j += 1;
                    }
                    i += 1;
                }
                if decoded >= num_blocks || b.left() == 0 {
                    break;
                }
            }
            num_blocks = num_blocks.saturating_sub(at_qpi);
            qpi += 1;
        }
        Ok(())
    }

    fn unpack_dct(&mut self, setup: &Setup, b: &mut Bits<'_>) -> Result<()> {
        let n = self.coded.len();
        self.coefs.clear();
        self.coefs.resize(n * 64, 0);
        self.dc_only.clear();
        self.dc_only.resize(n, false);
        self.overflow.clear();
        self.overflow.resize(n, false);
        // next token level of each block (64 when it has ended)
        let mut level_of = vec![0u8; n];
        if b.left() < 16 {
            return Err(Error::Invalid("DCT data missing"));
        }
        let dc_y = b.get(4) as usize;
        let dc_c = b.get(4) as usize;
        let mut ac_y = 0usize;
        let mut ac_c = 0usize;
        let mut eob_run: u32 = 0;
        let starts = [0, self.plane_end[0], self.plane_end[1]];
        for level in 0..64usize {
            if level == 1 {
                if b.left() < 8 {
                    return Err(Error::Invalid("DCT data missing"));
                }
                ac_y = b.get(4) as usize;
                ac_c = b.get(4) as usize;
            }
            let group = match level {
                0 => 0,
                1..=5 => 16,
                6..=14 => 32,
                15..=27 => 48,
                _ => 64,
            };
            for plane in 0..3 {
                let table_idx = group
                    + if level == 0 {
                        if plane == 0 { dc_y } else { dc_c }
                    } else if plane == 0 {
                        ac_y
                    } else {
                        ac_c
                    };
                let table = setup.huffman.get(table_idx).ok_or(Error::Invalid("Huffman table index"))?;
                for bi in starts[plane]..self.plane_end[plane] {
                    if usize::from(level_of[bi]) != level {
                        continue;
                    }
                    if eob_run > 0 {
                        eob_run -= 1;
                        level_of[bi] = 64;
                        if level == 0 {
                            self.dc_only[bi] = true;
                        }
                        continue;
                    }
                    if b.left() == 0 {
                        return Err(Error::Invalid("DCT tokens run past the packet"));
                    }
                    let token = table.read(b).ok_or(Error::Invalid("DCT token code"))? as usize;
                    if token <= 6 {
                        let (base, bits) = EOB_RUN[token];
                        let run = base + if bits > 0 { b.get(bits) } else { 0 };
                        eob_run = if run == 0 { u32::MAX } else { run } - 1;
                        level_of[bi] = 64;
                        if level == 0 {
                            self.dc_only[bi] = true;
                        }
                    } else {
                        let cbits = u32::from(COEFF_BITS[token]);
                        let coeff = token_coeff(token, if cbits > 0 { b.get(cbits) } else { 0 });
                        let zbits = u32::from(ZERO_RUN_BITS[token]);
                        let zero_run = usize::from(ZERO_RUN_BASE[token]) + if zbits > 0 { b.get(zbits) as usize } else { 0 };
                        let pos = level + zero_run;
                        if pos > 63 {
                            // FFmpeg clamps the run, and its dequantizer then gives up
                            if let Some(o) = self.overflow.get_mut(bi) {
                                *o = true;
                            }
                        } else if let Some(c) = self.coefs.get_mut(bi * 64 + pos) {
                            *c = coeff;
                        }
                        if level == 0 && zero_run == 0 {
                            self.frags[self.coded[bi]].dc = coeff;
                        }
                        level_of[bi] = (level + zero_run + 1).min(64) as u8;
                    }
                }
            }
        }
        // DC prediction, plane by plane
        for plane in 0..3 {
            self.reverse_dc_prediction(plane);
        }
        Ok(())
    }

    fn reverse_dc_prediction(&mut self, plane: usize) {
        const PUL: usize = 8;
        const PU: usize = 4;
        const PUR: usize = 2;
        const PL: usize = 1;
        const TRANSFORM: [[i32; 4]; 16] = [
            [0, 0, 0, 0],
            [0, 0, 0, 128],
            [0, 0, 128, 0],
            [0, 0, 53, 75],
            [0, 128, 0, 0],
            [0, 64, 0, 64],
            [0, 128, 0, 0],
            [0, 0, 53, 75],
            [128, 0, 0, 0],
            [0, 0, 0, 128],
            [64, 0, 64, 0],
            [0, 0, 53, 75],
            [0, 128, 0, 0],
            [-104, 116, 0, 116],
            [24, 80, 24, 0],
            [-104, 116, 0, 116],
        ];
        const COMPATIBLE: [u8; 9] = [1, 0, 1, 1, 1, 2, 2, 1, 3];
        let pi = usize::from(plane > 0);
        let (fw, fh) = (self.geo.frag_w[pi], self.geo.frag_h[pi]);
        let first = self.geo.frag_start[plane];
        let (mut vl, mut vul, mut vu, mut vur) = (0i32, 0i32, 0i32, 0i32);
        let mut last_dc = [0i32; 3];
        let mut i = first;
        for y in 0..fh {
            for x in 0..fw {
                let cur = self.frags[i];
                if cur.coding != MODE_COPY {
                    let ty = COMPATIBLE[usize::from(cur.coding).min(8)];
                    let compat = |frags: &[Frag], j: usize| COMPATIBLE[usize::from(frags[j].coding).min(8)] == ty;
                    let mut t = 0usize;
                    if x > 0 {
                        vl = i32::from(self.frags[i - 1].dc);
                        if compat(&self.frags, i - 1) {
                            t |= PL;
                        }
                    }
                    if y > 0 {
                        let u = i - fw;
                        vu = i32::from(self.frags[u].dc);
                        if compat(&self.frags, u) {
                            t |= PU;
                        }
                        if x > 0 {
                            vul = i32::from(self.frags[u - 1].dc);
                            if compat(&self.frags, u - 1) {
                                t |= PUL;
                            }
                        }
                        if x + 1 < fw {
                            vur = i32::from(self.frags[u + 1].dc);
                            if compat(&self.frags, u + 1) {
                                t |= PUR;
                            }
                        }
                    }
                    let predicted = if t == 0 {
                        last_dc[usize::from(ty).min(2)]
                    } else {
                        let m = TRANSFORM[t];
                        let mut p = (m[0] * vul + m[1] * vu + m[2] * vur + m[3] * vl) / 128;
                        if t == 15 || t == 13 {
                            if (p - vu).abs() > 128 {
                                p = vu;
                            } else if (p - vl).abs() > 128 {
                                p = vl;
                            } else if (p - vul).abs() > 128 {
                                p = vul;
                            }
                        }
                        p
                    };
                    let dc = (i32::from(cur.dc) + predicted) as i16;
                    self.frags[i].dc = dc;
                    last_dc[usize::from(ty).min(2)] = i32::from(dc);
                }
                i += 1;
            }
        }
    }

    /// Which plane and `(x, y)` in fragments the fragment index `i` is.
    fn locate(&self, i: usize) -> (usize, usize, usize) {
        let plane = if i >= self.geo.frag_start[2] { 2 } else { usize::from(i >= self.geo.frag_start[1]) };
        let fw = self.geo.frag_w[usize::from(plane > 0)].max(1);
        let local = i - self.geo.frag_start[plane];
        (plane, local % fw, local / fw)
    }

    fn reconstruct(&self, cur: &mut Frame, last: Option<&Frame>, golden: Option<&Frame>) {
        let mut scan = [0usize; 64];
        for (z, s) in scan.iter_mut().enumerate() {
            *s = transpose(usize::from(ZIGZAG[z]));
        }
        // the position of each coded fragment in the coding order
        let mut coded_pos = vec![usize::MAX; self.geo.frag_count];
        for (k, &f) in self.coded.iter().enumerate() {
            coded_pos[f] = k;
        }
        for i in 0..self.geo.frag_count {
            let frag = self.frags[i];
            let (plane, fx, fy) = self.locate(i);
            let (pw, ph) = self.geo.plane_size(plane);
            let (x8, y8) = (fx * 8, fy * 8);
            let off = y8 * pw + x8;
            if frag.coding == MODE_COPY {
                if let Some(l) = last {
                    for r in 0..8 {
                        let o = off + r * pw;
                        if let (Some(d), Some(s)) = (cur.planes[plane].get_mut(o..o + 8), l.planes[plane].get(o..o + 8)) {
                            d.copy_from_slice(s);
                        }
                    }
                }
                continue;
            }
            let inter = frag.coding != MODE_INTRA;
            if inter {
                let src = if frag.coding == MODE_USING_GOLDEN || frag.coding == MODE_GOLDEN_MV { golden } else { last };
                let Some(src) = src else { continue };
                let mut mv = (0i32, 0i32);
                if frag.coding > MODE_INTRA && frag.coding != MODE_USING_GOLDEN {
                    let pi = usize::from(plane > 0);
                    let m = self.motion[pi].get(fy * self.geo.frag_w[pi] + fx).copied().unwrap_or([0, 0]);
                    mv = (i32::from(m[0]), i32::from(m[1]));
                }
                predict(&mut cur.planes[plane], &src.planes[plane], pw, ph, x8, y8, mv);
            }
            // coefficients
            let k = coded_pos[i];
            let mut block = [0i16; 64];
            let qpi = usize::from(frag.qpi).min(2);
            let qm = &self.qmat[qpi][usize::from(inter)][plane];
            if let Some(c) = self.coefs.get(k * 64..k * 64 + 64) {
                for z in 1..64 {
                    if c[z] != 0 {
                        block[scan[z]] = (i32::from(c[z]) * i32::from(qm[scan[z]])) as i16;
                    }
                }
            }
            let overflow = self.overflow.get(k).copied().unwrap_or(false);
            let dc_q = self.qmat[0][usize::from(inter)][plane][0];
            block[0] = if overflow {
                // FFmpeg's dequantizer stopped before the DC: what stays is a DC-level token's
                // value times this block's own quantizer
                let c0 = self.coefs.get(k * 64).copied().unwrap_or(0);
                (i32::from(c0) * i32::from(qm[0])) as i16
            } else {
                (i32::from(frag.dc) * i32::from(dc_q)) as i16
            };
            let dst = &mut cur.planes[plane][off..];
            if !inter {
                dsp::idct_put(dst, pw, &block);
            } else if !overflow && self.dc_only.get(k).copied().unwrap_or(false) {
                dsp::idct_dc_add(dst, pw, block[0]);
            } else {
                dsp::idct_add(dst, pw, &block);
            }
        }
    }

    fn loop_filter(&self, cur: &mut Frame, limit: i32) {
        let bv = dsp::bounding_values(limit);
        for plane in 0..3 {
            let pi = usize::from(plane > 0);
            let (fw, fh) = (self.geo.frag_w[pi], self.geo.frag_h[pi]);
            let (pw, _) = self.geo.plane_size(plane);
            let start = self.geo.frag_start[plane];
            let p = &mut cur.planes[plane];
            for y in 0..fh {
                for x in 0..fw {
                    let f = start + y * fw + x;
                    if self.frags[f].coding == MODE_COPY {
                        continue;
                    }
                    if x > 0 {
                        dsp::h_loop_filter(p, pw, 8 * x, 8 * y, &bv);
                    }
                    if y > 0 {
                        dsp::v_loop_filter(p, pw, 8 * x, 8 * y, &bv);
                    }
                    if x + 1 < fw && self.frags[f + 1].coding == MODE_COPY {
                        dsp::h_loop_filter(p, pw, 8 * x + 8, 8 * y, &bv);
                    }
                    if y + 1 < fh && self.frags[f + fw].coding == MODE_COPY {
                        dsp::v_loop_filter(p, pw, 8 * x, 8 * y + 8, &bv);
                    }
                }
            }
        }
    }
}

/// The motion-compensated 8 x 8 prediction of the block at `(x8, y8)` from `src`, written to
/// `dst` (a whole plane of the same size). `mv` is in half pixels; reads outside the plane take
/// its nearest edge pixel.
fn predict(dst: &mut [u8], src: &[u8], pw: usize, ph: usize, x8: usize, y8: usize, mv: (i32, i32)) {
    let (mx, my) = mv;
    let ix = x8 as i32 + (mx >> 1);
    let iy = y8 as i32 + (my >> 1);
    let half = (mx & 1) | ((my & 1) << 1);
    let px = |x: i32, y: i32| -> i32 {
        let xc = x.clamp(0, pw as i32 - 1) as usize;
        let yc = y.clamp(0, ph as i32 - 1) as usize;
        src.get(yc * pw + xc).copied().map_or(0, i32::from)
    };
    let d = (mx ^ my) >> 31;
    for r in 0..8i32 {
        for c in 0..8i32 {
            let (x, y) = (ix + c, iy + r);
            let v = match half {
                0 => px(x, y),
                1 => (px(x, y) + px(x + 1, y)) >> 1,
                2 => (px(x, y) + px(x, y + 1)) >> 1,
                _ => (px(x - d, y) + px(x + 1 + d, y + 1)) >> 1,
            };
            if let Some(o) = dst.get_mut((y8 + r as usize) * pw + x8 + c as usize) {
                *o = v as u8;
            }
        }
    }
}
