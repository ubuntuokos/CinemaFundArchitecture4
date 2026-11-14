// Ported from FFmpeg n7.1.1 libavcodec/rv10.c, ituh263dec.c, h263.c, h263dsp.c, mpegvideo.c,
// mpegvideo_dec.c, mpv_reconstruct_mb_template.c, mpegvideo_motion.c, mpeg4video.c,
// simple_idct_template.c and hpel_template.c (Copyright (c) 2000-2002 Fabrice Bellard, Michael
// Niedermayer and the FFmpeg developers), LGPL-2.1-or-later; see docs/licensing.md §5. CFA4
// changes GPL-3.0-or-later.

//! Pure-Rust decoder for **RealVideo 1.0** (`RV10`) and **RealVideo 2.0** (`RV20`): the H.263
//! based RealVideo codecs. I, P and B pictures, advanced intra coding, modified quantisation, the
//! deblocking loop filter, four-vector macroblocks, direct-mode B macroblocks, slices and
//! reference picture resampling (picture size changes). The output is the same as ffmpeg's `rv10`
//! / `rv20` decoders with `-idct simple -flags +bitexact`, pixel for pixel.
//!
//! [`Decoder::decode`] takes one frame as the RealMedia demuxer assembles it (a slice count, a
//! slice table and the slices); `cfa4-realmedia` produces them.
//!
//! **RealVideo 3.0** (`RV30`) and **RealVideo 4.0** (`RV40`), the H.264-like codecs, are in
//! [`rv34`], which takes packets in the same layout.

pub mod rv34;
mod tables;

use std::sync::Arc;
use std::sync::OnceLock;
use tables::*;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid RealVideo stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported RealVideo stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Largest picture side.
pub const MAX_SIDE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Rv10,
    Rv20,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictType {
    I,
    P,
    B,
}

/// A decoded picture: 8-bit 4:2:0, tight planes of the display size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    /// `(width + 1) / 2` × `(height + 1) / 2`.
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub pict_type: PictType,
    /// Which frame of the stream this is, counted in decode order from 0 (see
    /// [`Decoder::frames_started`]): pictures come out in display order, so a caller maps them
    /// back to its packets with this.
    pub decode_index: u64,
}

// ---------------------------------------------------------------------------------------------
// Bit reading and VLCs

/// MSB-first bit reader; reads past the end return zeros.
struct Gb<'a> {
    d: &'a [u8],
    pos: usize,
    /// The bits the reader counts as its buffer (`get_bits_left`).
    size: usize,
}

impl<'a> Gb<'a> {
    fn new(d: &'a [u8], size_bits: usize) -> Self {
        Self { d, pos: 0, size: size_bits }
    }
    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let byte = self.pos >> 3;
        let mut w = 0u64;
        for i in 0..5 {
            w = (w << 8) | u64::from(byte.checked_add(i).and_then(|k| self.d.get(k)).copied().unwrap_or(0));
        }
        let v = (w << (24 + (self.pos & 7))) >> (64 - n);
        v as u32
    }
    fn skip(&mut self, n: u32) {
        self.pos = self.pos.saturating_add(n as usize);
    }
    fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n);
        v
    }
    fn get1(&mut self) -> bool {
        self.get(1) == 1
    }
    fn sget(&mut self, n: u32) -> i32 {
        let v = self.get(n);
        ((v << (32 - n)) as i32) >> (32 - n)
    }
    fn count(&self) -> usize {
        self.pos
    }
    fn left(&self) -> isize {
        self.size as isize - self.pos as isize
    }
}

struct Vlc {
    bits: u32,
    t: Vec<(i16, u8)>,
}

impl Vlc {
    fn new(entries: &[(u32, u8, i16)]) -> Self {
        let bits = u32::from(entries.iter().map(|e| e.1).max().unwrap_or(1));
        let mut t = vec![(0i16, 0u8); 1 << bits];
        for &(code, len, sym) in entries {
            if len == 0 {
                continue;
            }
            let shift = bits - u32::from(len);
            let start = (code as usize) << shift;
            for e in t.iter_mut().skip(start).take(1 << shift) {
                *e = (sym, len);
            }
        }
        Self { bits, t }
    }
    fn read(&self, gb: &mut Gb<'_>) -> Option<i16> {
        let &(s, l) = self.t.get(gb.peek(self.bits) as usize)?;
        if l == 0 {
            return None;
        }
        gb.skip(u32::from(l));
        Some(s)
    }
}

struct Vlcs {
    intra_mcbpc: Vlc,
    inter_mcbpc: Vlc,
    cbpy: Vlc,
    mv: Vlc,
    mbtype_b: Vlc,
    cbpc_b: Vlc,
    rl_inter: Vlc,
    rl_aic: Vlc,
    dc_lum: Vlc,
    dc_chrom: Vlc,
}

fn dc_vlc(len_count: &[u16; 15], runs: &[(u8, u8)]) -> Vlc {
    let mut syms = Vec::new();
    for &(sym, run) in runs {
        let mut cur = sym;
        for _ in 0..=run {
            syms.push(cur);
            cur = cur.wrapping_sub(1);
        }
    }
    let mut entries = Vec::new();
    let mut code: u64 = 0;
    let mut k = 0;
    for (i, &n) in len_count.iter().enumerate() {
        let len = i as u8 + 2;
        for _ in 0..n {
            let sym = syms.get(k).copied().unwrap_or(0);
            entries.push(((code >> (32 - u32::from(len))) as u32, len, i16::from(sym)));
            code += 1u64 << (32 - u32::from(len));
            k += 1;
        }
    }
    Vlc::new(&entries)
}

fn vlcs() -> &'static Vlcs {
    static V: OnceLock<Vlcs> = OnceLock::new();
    V.get_or_init(|| {
        let idx = |i: usize| i as i16;
        Vlcs {
            intra_mcbpc: Vlc::new(&(0..9).map(|i| (u32::from(INTRA_MCBPC_CODE[i]), INTRA_MCBPC_BITS[i], idx(i))).collect::<Vec<_>>()),
            inter_mcbpc: Vlc::new(&(0..28).map(|i| (u32::from(INTER_MCBPC_CODE[i]), INTER_MCBPC_BITS[i], idx(i))).collect::<Vec<_>>()),
            cbpy: Vlc::new(&CBPY.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), l, idx(i))).collect::<Vec<_>>()),
            mv: Vlc::new(&MVTAB.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), l, idx(i))).collect::<Vec<_>>()),
            mbtype_b: Vlc::new(&MBTYPE_B.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), l, idx(i))).collect::<Vec<_>>()),
            cbpc_b: Vlc::new(&CBPC_B.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), l, idx(i))).collect::<Vec<_>>()),
            rl_inter: Vlc::new(&INTER_VLC.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), l, idx(i))).collect::<Vec<_>>()),
            rl_aic: Vlc::new(&INTRA_AIC_VLC.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), l, idx(i))).collect::<Vec<_>>()),
            dc_lum: dc_vlc(&RV_LUM_LEN_COUNT, &RV_SYM_RUN_LEN),
            dc_chrom: dc_vlc(&RV_CHROM_LEN_COUNT, &RV_SYM_RUN_LEN[..17]),
        }
    })
}

// ---------------------------------------------------------------------------------------------
// IDCT (ffmpeg's simple IDCT, 8 bit)

const W1: i32 = 22725;
const W2: i32 = 21407;
const W3: i32 = 19266;
const W4: i32 = 16383;
const W5: i32 = 12873;
const W6: i32 = 8867;
const W7: i32 = 4520;

#[inline]
fn m(c: i32, v: i16) -> u32 {
    (c as u32).wrapping_mul(i32::from(v) as u32)
}

fn idct_row(r: &mut [i16]) {
    if r[1..8].iter().all(|&v| v == 0) {
        let t = (i32::from(r[0]) * 8) as i16;
        r[..8].fill(t);
        return;
    }
    let mut a0 = m(W4, r[0]).wrapping_add(1 << 10);
    let (mut a1, mut a2, mut a3) = (a0, a0, a0);
    a0 = a0.wrapping_add(m(W2, r[2]));
    a1 = a1.wrapping_add(m(W6, r[2]));
    a2 = a2.wrapping_sub(m(W6, r[2]));
    a3 = a3.wrapping_sub(m(W2, r[2]));
    let mut b0 = m(W1, r[1]).wrapping_add(m(W3, r[3]));
    let mut b1 = m(W3, r[1]).wrapping_add(m(-W7, r[3]));
    let mut b2 = m(W5, r[1]).wrapping_add(m(-W1, r[3]));
    let mut b3 = m(W7, r[1]).wrapping_add(m(-W5, r[3]));
    a0 = a0.wrapping_add(m(W4, r[4])).wrapping_add(m(W6, r[6]));
    a1 = a1.wrapping_add(m(-W4, r[4])).wrapping_sub(m(W2, r[6]));
    a2 = a2.wrapping_add(m(-W4, r[4])).wrapping_add(m(W2, r[6]));
    a3 = a3.wrapping_add(m(W4, r[4])).wrapping_sub(m(W6, r[6]));
    b0 = b0.wrapping_add(m(W5, r[5])).wrapping_add(m(W7, r[7]));
    b1 = b1.wrapping_add(m(-W1, r[5])).wrapping_add(m(-W5, r[7]));
    b2 = b2.wrapping_add(m(W7, r[5])).wrapping_add(m(W3, r[7]));
    b3 = b3.wrapping_add(m(W3, r[5])).wrapping_add(m(-W1, r[7]));
    let s = |v: u32| ((v as i32) >> 11) as i16;
    r[0] = s(a0.wrapping_add(b0));
    r[7] = s(a0.wrapping_sub(b0));
    r[1] = s(a1.wrapping_add(b1));
    r[6] = s(a1.wrapping_sub(b1));
    r[2] = s(a2.wrapping_add(b2));
    r[5] = s(a2.wrapping_sub(b2));
    r[3] = s(a3.wrapping_add(b3));
    r[4] = s(a3.wrapping_sub(b3));
}

/// One column, the 8 outputs top to bottom (before clipping).
fn idct_col(b: &[i16; 64], c: usize) -> [i32; 8] {
    let col = |k: usize| b[c + 8 * k];
    let mut a0 = (W4 as u32).wrapping_mul((i32::from(col(0)) + ((1 << 19) / W4)) as u32);
    let (mut a1, mut a2, mut a3) = (a0, a0, a0);
    a0 = a0.wrapping_add(m(W2, col(2)));
    a1 = a1.wrapping_add(m(W6, col(2)));
    a2 = a2.wrapping_add(m(-W6, col(2)));
    a3 = a3.wrapping_add(m(-W2, col(2)));
    let mut b0 = m(W1, col(1));
    let mut b1 = m(W3, col(1));
    let mut b2 = m(W5, col(1));
    let mut b3 = m(W7, col(1));
    b0 = b0.wrapping_add(m(W3, col(3)));
    b1 = b1.wrapping_add(m(-W7, col(3)));
    b2 = b2.wrapping_add(m(-W1, col(3)));
    b3 = b3.wrapping_add(m(-W5, col(3)));
    a0 = a0.wrapping_add(m(W4, col(4)));
    a1 = a1.wrapping_add(m(-W4, col(4)));
    a2 = a2.wrapping_add(m(-W4, col(4)));
    a3 = a3.wrapping_add(m(W4, col(4)));
    b0 = b0.wrapping_add(m(W5, col(5)));
    b1 = b1.wrapping_add(m(-W1, col(5)));
    b2 = b2.wrapping_add(m(W7, col(5)));
    b3 = b3.wrapping_add(m(W3, col(5)));
    a0 = a0.wrapping_add(m(W6, col(6)));
    a1 = a1.wrapping_add(m(-W2, col(6)));
    a2 = a2.wrapping_add(m(W2, col(6)));
    a3 = a3.wrapping_add(m(-W6, col(6)));
    b0 = b0.wrapping_add(m(W7, col(7)));
    b1 = b1.wrapping_add(m(-W5, col(7)));
    b2 = b2.wrapping_add(m(W3, col(7)));
    b3 = b3.wrapping_add(m(-W1, col(7)));
    let s = |v: u32| (v as i32) >> 20;
    [
        s(a0.wrapping_add(b0)),
        s(a1.wrapping_add(b1)),
        s(a2.wrapping_add(b2)),
        s(a3.wrapping_add(b3)),
        s(a3.wrapping_sub(b3)),
        s(a2.wrapping_sub(b2)),
        s(a1.wrapping_sub(b1)),
        s(a0.wrapping_sub(b0)),
    ]
}

fn idct(block: &mut [i16; 64], dst: &mut [u8], off: usize, stride: usize, add: bool) {
    for r in 0..8 {
        idct_row(&mut block[r * 8..r * 8 + 8]);
    }
    for c in 0..8 {
        let out = idct_col(block, c);
        for (k, &v) in out.iter().enumerate() {
            if let Some(p) = dst.get_mut(off + k * stride + c) {
                *p = if add { (i32::from(*p) + v).clamp(0, 255) } else { v.clamp(0, 255) } as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Pictures and motion compensation

#[derive(Clone)]
struct Plane {
    d: Vec<u8>,
    w: usize,
    h: usize,
}

impl Plane {
    fn new(w: usize, h: usize, v: u8) -> Self {
        Self { d: vec![v; w * h], w, h }
    }
    #[inline]
    fn at(&self, x: isize, y: isize) -> i32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        i32::from(self.d[y * self.w + x])
    }
}

const MB_SKIP: u8 = 1;
const MB_INTRA: u8 = 2;
const MB_8X8: u8 = 4;

#[derive(Clone)]
struct Pic {
    planes: [Plane; 3],
    /// Per 8×8 block, forward and backward, with a one-entry border (zero).
    motion: [Vec<[i16; 2]>; 2],
    mb_type: Vec<u8>,
    qscale: Vec<u8>,
    pict_type: PictType,
    mb_w: usize,
    /// [`Decoder::frames_started`] when this picture was started.
    index: u64,
}

impl Pic {
    fn new(mb_w: usize, mb_h: usize, pict_type: PictType, fill: u8) -> Self {
        let mv = (2 * mb_w + 2) * (2 * mb_h + 1);
        Self {
            planes: [Plane::new(mb_w * 16, mb_h * 16, fill), Plane::new(mb_w * 8, mb_h * 8, 128), Plane::new(mb_w * 8, mb_h * 8, 128)],
            motion: [vec![[0; 2]; mv], vec![[0; 2]; mv]],
            mb_type: vec![0; mb_w * mb_h],
            qscale: vec![0; mb_w * mb_h],
            pict_type,
            mb_w,
            index: 0,
        }
    }
    /// Index of the 8×8 block at block row `r`, column `c` (each may be −1).
    fn mvi(&self, r: isize, c: isize) -> usize {
        ((r + 1) * (2 * self.mb_w as isize + 2) + c + 1).max(0) as usize
    }
}

/// One half-pel block prediction (`put` / `avg`, rounding or not).
#[allow(clippy::too_many_arguments)]
fn mc(dst: &mut Plane, dx: usize, dy: usize, r: &Plane, sx: isize, sy: isize, bw: usize, bh: usize, dxy: i32, rnd: bool, avg: bool) {
    let (hx, hy) = (dxy & 1 != 0, dxy & 2 != 0);
    for j in 0..bh {
        for i in 0..bw {
            let (x, y) = (sx + i as isize, sy + j as isize);
            let a = r.at(x, y);
            let p = match (hx, hy) {
                (false, false) => a,
                (true, false) => (a + r.at(x + 1, y) + i32::from(rnd)) >> 1,
                (false, true) => (a + r.at(x, y + 1) + i32::from(rnd)) >> 1,
                (true, true) => (a + r.at(x + 1, y) + r.at(x, y + 1) + r.at(x + 1, y + 1) + 1 + i32::from(rnd)) >> 2,
            };
            if let Some(d) = dst.d.get_mut((dy + j) * dst.w + dx + i) {
                *d = if avg { ((i32::from(*d) + p + 1) >> 1) as u8 } else { p as u8 };
            }
        }
    }
}

fn round_chroma(x: i32) -> i32 {
    CHROMA_ROUND[(x & 15) as usize] + (x >> 3)
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

fn loop_filter_px(p0: i32, p1: i32, p2: i32, p3: i32, strength: i32) -> [u8; 4] {
    let d = (p0 - p3 + 4 * (p2 - p1)) / 8;
    let d1 = if d < -2 * strength {
        0
    } else if d < -strength {
        -2 * strength - d
    } else if d < strength {
        d
    } else if d < 2 * strength {
        2 * strength - d
    } else {
        0
    };
    let clip = |p: i32| if p & 256 != 0 { !(p >> 31) } else { p };
    let q1 = clip(p1 + d1);
    let q2 = clip(p2 - d1);
    let ad1 = d1.abs() >> 1;
    let d2 = ((p0 - p3) / 4).clamp(-ad1, ad1);
    [(p0 - d2) as u8, q1 as u8, q2 as u8, (p3 + d2) as u8]
}

/// Filters the vertical edge left of column `x`, rows `y..y + 8`.
fn h_loop_filter(p: &mut Plane, x: usize, y: usize, q: u8) {
    if x < 2 || x + 1 >= p.w {
        return;
    }
    let s = i32::from(LOOP_FILTER_STRENGTH[usize::from(q & 31)]);
    for r in y..(y + 8).min(p.h) {
        let o = r * p.w + x;
        let v = loop_filter_px(i32::from(p.d[o - 2]), i32::from(p.d[o - 1]), i32::from(p.d[o]), i32::from(p.d[o + 1]), s);
        p.d[o - 2..o + 2].copy_from_slice(&v);
    }
}

/// Filters the horizontal edge above row `y`, columns `x..x + 8`.
fn v_loop_filter(p: &mut Plane, x: usize, y: usize, q: u8) {
    if y < 2 || y + 1 >= p.h {
        return;
    }
    let s = i32::from(LOOP_FILTER_STRENGTH[usize::from(q & 31)]);
    let w = p.w;
    for c in x..(x + 8).min(w) {
        let o = y * w + c;
        let v = loop_filter_px(i32::from(p.d[o - 2 * w]), i32::from(p.d[o - w]), i32::from(p.d[o]), i32::from(p.d[o + w]), s);
        p.d[o - 2 * w] = v[0];
        p.d[o - w] = v[1];
        p.d[o] = v[2];
        p.d[o + w] = v[3];
    }
}

// ---------------------------------------------------------------------------------------------
// The decoder

#[derive(Clone, Copy, PartialEq, Eq)]
enum MbEnd {
    Ok,
    End,
}

const MV_FWD: u8 = 1;
const MV_BWD: u8 = 2;

/// A RealVideo 1 / 2 decoder.
pub struct Decoder {
    codec: Codec,
    extradata: Vec<u8>,
    sub_id: u32,
    rv10_version: u8,
    long_vectors: bool,
    low_delay: bool,
    orig_w: u32,
    orig_h: u32,
    width: u32,
    height: u32,
    mb_w: usize,
    mb_h: usize,

    pict_type: PictType,
    qscale: i32,
    chroma_qscale: i32,
    y_dc_scale: i32,
    c_dc_scale: i32,
    aic: bool,
    modified_quant: bool,
    loop_filter: bool,
    no_rounding: bool,
    last_dc: [i32; 3],
    first_dc_coded: [bool; 3],
    time: i32,
    pp_time: i32,
    pb_time: i32,
    last_non_b_time: i32,
    direct_scale: [[i32; 64]; 2],

    mb_x: usize,
    mb_y: usize,
    resync_mb_x: usize,
    first_slice_line: bool,
    mb_num_left: usize,

    dc_val: [Vec<i32>; 3],
    ac_val: [Vec<[i16; 16]>; 3],
    ac_pred: bool,
    aic_dir: bool,

    mb_intra: bool,
    mv_dir: u8,
    mv_8x8: bool,
    mv: [[[i32; 2]; 4]; 2],
    blocks: [[i16; 64]; 6],
    last_index: [i32; 6],

    cur: Option<Pic>,
    decoded: Vec<bool>,
    last: Option<Arc<Pic>>,
    next: Option<Arc<Pic>>,
    frames_started: u64,
    /// Index of the reference picture [`Decoder::flush`] returned.
    flushed: Option<u64>,
}

fn rv_major(x: u32) -> u32 {
    x >> 28
}
fn rv_minor(x: u32) -> u32 {
    (x >> 20) & 0xFF
}
fn rv_micro(x: u32) -> u32 {
    (x >> 12) & 0xFF
}

impl Decoder {
    /// A decoder for `codec` with the stream's coded size and its codec data (the 8 or more bytes
    /// after the frame rate in the RealMedia `VIDO` header).
    pub fn new(codec: Codec, width: u32, height: u32, extradata: &[u8]) -> Result<Self> {
        if extradata.len() < 8 {
            return Err(Error::Invalid("codec data too small"));
        }
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::Unsupported("picture size"));
        }
        let sub_id = u32::from_be_bytes([extradata[4], extradata[5], extradata[6], extradata[7]]);
        let mut d = Self {
            codec,
            extradata: extradata.to_vec(),
            sub_id,
            rv10_version: 0,
            long_vectors: extradata[3] & 1 != 0,
            low_delay: true,
            orig_w: width,
            orig_h: height,
            width: 0,
            height: 0,
            mb_w: 0,
            mb_h: 0,
            pict_type: PictType::I,
            qscale: 1,
            chroma_qscale: 1,
            y_dc_scale: 8,
            c_dc_scale: 8,
            aic: false,
            modified_quant: false,
            loop_filter: false,
            no_rounding: false,
            last_dc: [0; 3],
            first_dc_coded: [false; 3],
            time: 0,
            pp_time: 0,
            pb_time: 0,
            last_non_b_time: 0,
            direct_scale: [[0; 64]; 2],
            mb_x: 0,
            mb_y: 0,
            resync_mb_x: 0,
            first_slice_line: false,
            mb_num_left: 0,
            dc_val: [Vec::new(), Vec::new(), Vec::new()],
            ac_val: [Vec::new(), Vec::new(), Vec::new()],
            ac_pred: false,
            aic_dir: false,
            mb_intra: false,
            mv_dir: 0,
            mv_8x8: false,
            mv: [[[0; 2]; 4]; 2],
            blocks: [[0; 64]; 6],
            last_index: [0; 6],
            cur: None,
            decoded: Vec::new(),
            last: None,
            next: None,
            frames_started: 0,
            flushed: None,
        };
        match rv_major(sub_id) {
            1 => {
                d.rv10_version = if rv_micro(sub_id) != 0 { 3 } else { 1 };
                if rv_micro(sub_id) == 2 {
                    return Err(Error::Unsupported("RealVideo 1 overlapped block motion compensation"));
                }
            }
            2 => {
                if rv_minor(sub_id) >= 2 {
                    d.low_delay = false;
                }
            }
            _ => return Err(Error::Unsupported("RealVideo 1/2 version")),
        }
        d.set_size(width, height);
        Ok(d)
    }

    fn set_size(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        self.mb_w = (w as usize).div_ceil(16);
        self.mb_h = (h as usize).div_ceil(16);
        let (bw, bh) = (2 * self.mb_w + 2, 2 * self.mb_h + 1);
        let (cw, ch) = (self.mb_w + 2, self.mb_h + 1);
        self.dc_val = [vec![1024; bw * bh], vec![1024; cw * ch], vec![1024; cw * ch]];
        self.ac_val = [vec![[0; 16]; bw * bh], vec![[0; 16]; cw * ch], vec![[0; 16]; cw * ch]];
        self.cur = None;
        self.last = None;
        self.next = None;
    }

    fn set_qscale(&mut self, q: i32) {
        let q = q.clamp(1, 31);
        self.qscale = q;
        self.chroma_qscale = if self.modified_quant { i32::from(H263_CHROMA_QSCALE[q as usize]) } else { q };
        if self.aic {
            self.y_dc_scale = i32::from(AIC_DC_SCALE[q as usize]);
            self.c_dc_scale = i32::from(AIC_DC_SCALE[self.chroma_qscale as usize]);
        } else {
            self.y_dc_scale = i32::from(MPEG1_DC_SCALE);
            self.c_dc_scale = i32::from(MPEG1_DC_SCALE);
        }
    }

    fn rv10_header(&mut self, gb: &mut Gb<'_>) -> Result<usize> {
        let _marker = gb.get1();
        self.pict_type = if gb.get1() { PictType::P } else { PictType::I };
        if gb.get1() {
            return Err(Error::Unsupported("PB-frames"));
        }
        self.qscale = gb.get(5) as i32;
        if self.qscale == 0 {
            return Err(Error::Invalid("quantiser 0"));
        }
        if self.pict_type == PictType::I && self.rv10_version == 3 {
            for v in &mut self.last_dc {
                *v = gb.get(8) as i32;
            }
        }
        let mb_num = self.mb_w * self.mb_h;
        let mb_xy = self.mb_x + self.mb_y * self.mb_w;
        let count = if gb.peek(12) == 0 || (mb_xy != 0 && mb_xy < mb_num) {
            self.mb_x = gb.get(6) as usize;
            self.mb_y = gb.get(6) as usize;
            gb.get(12) as usize
        } else {
            self.mb_x = 0;
            self.mb_y = 0;
            mb_num
        };
        gb.skip(3);
        Ok(count)
    }

    fn rv20_header(&mut self, gb: &mut Gb<'_>, whole_size: usize) -> Result<usize> {
        self.pict_type = [PictType::I, PictType::I, PictType::P, PictType::B][gb.get(2) as usize];
        if self.low_delay && self.pict_type == PictType::B {
            return Err(Error::Invalid("B-frame in a low-delay stream"));
        }
        if self.last.is_none() && self.pict_type == PictType::B {
            return Err(Error::Invalid("B-frame before two references"));
        }
        if gb.get1() {
            return Err(Error::Invalid("reserved bit set"));
        }
        self.qscale = gb.get(5) as i32;
        if self.qscale == 0 {
            return Err(Error::Invalid("quantiser 0"));
        }
        if rv_minor(self.sub_id) >= 2 {
            gb.skip(1); // loop filter flag (the loop filter is always on)
        }
        let mut seq = if rv_minor(self.sub_id) <= 1 { (gb.get(8) << 7) as i32 } else { (gb.get(13) << 2) as i32 };
        let rpr_max = u32::from(self.extradata[1] & 7);
        if rpr_max != 0 {
            let rpr_bits = 32 - rpr_max.leading_zeros();
            let f = gb.get(rpr_bits) as usize;
            let (nw, nh) = if f != 0 {
                let (a, b) = (6 + 2 * f, 7 + 2 * f);
                match (self.extradata.get(a), self.extradata.get(b)) {
                    (Some(&w), Some(&h)) => (4 * u32::from(w), 4 * u32::from(h)),
                    _ => return Err(Error::Invalid("codec data too small for the picture size")),
                }
            } else {
                (self.orig_w, self.orig_h)
            };
            if nw != self.width || nh != self.height {
                if nw == 0 || nh == 0 || nw > MAX_SIDE || nh > MAX_SIDE {
                    return Err(Error::Invalid("picture size"));
                }
                if whole_size < (nw as usize).div_ceil(16) * (nh as usize).div_ceil(16) / 8 {
                    return Err(Error::Invalid("frame too small"));
                }
                self.set_size(nw, nh);
            }
        }
        let mb_num = self.mb_w * self.mb_h;
        let mut i = 0;
        while i < 6 && mb_num - 1 > usize::from(MBA_MAX[i]) {
            i += 1;
        }
        let mb_pos = gb.get(u32::from(MBA_LENGTH[i])) as usize;
        self.mb_x = mb_pos % self.mb_w;
        self.mb_y = mb_pos / self.mb_w;

        seq |= self.time & !0x7FFF;
        if seq - self.time > 0x4000 {
            seq -= 0x8000;
        }
        if seq - self.time < -0x4000 {
            seq += 0x8000;
        }
        if seq != self.time {
            if self.pict_type != PictType::B {
                self.time = seq;
                self.pp_time = self.time - self.last_non_b_time;
                self.last_non_b_time = self.time;
            } else {
                self.time = seq;
                self.pb_time = self.pp_time - (self.last_non_b_time - self.time);
            }
        }
        if self.pict_type == PictType::B {
            if self.pp_time <= self.pb_time || self.pp_time <= self.pp_time - self.pb_time || self.pp_time <= 0 {
                return Err(Error::Invalid("B-frame out of order"));
            }
            for k in 0..64 {
                let v = k as i32 - 32;
                self.direct_scale[0][k] = v * self.pb_time / self.pp_time;
                self.direct_scale[1][k] = v * (self.pb_time - self.pp_time) / self.pp_time;
            }
        }
        self.no_rounding = gb.get1();
        if rv_minor(self.sub_id) <= 1 && self.pict_type == PictType::B {
            gb.skip(5);
        }
        self.aic = self.pict_type == PictType::I;
        self.modified_quant = true;
        self.loop_filter = true;
        Ok(mb_num.saturating_sub(mb_pos))
    }

    fn decode_dquant(&mut self, gb: &mut Gb<'_>) {
        let q = if self.modified_quant {
            if gb.get1() {
                let t = usize::from(gb.get1());
                i32::from(MODIFIED_QUANT[t][self.qscale.clamp(0, 31) as usize])
            } else {
                gb.get(5) as i32
            }
        } else {
            self.qscale + [-1, -2, 1, 2][gb.get(2) as usize]
        };
        self.set_qscale(q);
    }

    fn decode_motion(&self, gb: &mut Gb<'_>, pred: i32) -> Option<i32> {
        let code = i32::from(vlcs().mv.read(gb)?);
        if code == 0 {
            return Some(pred);
        }
        let sign = gb.get1();
        let mut val = if sign { -code } else { code };
        val += pred;
        if !self.long_vectors {
            val = (val << 26) >> 26;
        } else {
            if pred < -31 && val < -63 {
                val += 64;
            }
            if pred > 32 && val > 63 {
                val -= 64;
            }
        }
        Some(val)
    }

    /// `ff_h263_pred_motion`: the predictor and the index of the block's vector.
    fn pred_motion(&self, cur: &mut Pic, block: usize, dir: usize) -> (i32, i32, usize) {
        let r = (2 * self.mb_y + (block >> 1)) as isize;
        let c = (2 * self.mb_x + (block & 1)) as isize;
        let off = [2isize, 1, 1, -1][block];
        let mv = |cur: &Pic, rr: isize, cc: isize| {
            let v = cur.motion[dir].get(cur.mvi(rr, cc)).copied().unwrap_or([0, 0]);
            (i32::from(v[0]), i32::from(v[1]))
        };
        let me = cur.mvi(r, c);
        let a = mv(cur, r, c - 1);
        let (px, py) = if self.first_slice_line && block < 3 {
            match block {
                0 => {
                    if self.mb_x == self.resync_mb_x {
                        (0, 0)
                    } else {
                        a
                    }
                }
                1 => a,
                _ => {
                    let b = mv(cur, r - 1, c);
                    let cc = mv(cur, r - 1, c + off);
                    let mut a = a;
                    if self.mb_x == self.resync_mb_x {
                        let ai = cur.mvi(r, c - 1);
                        if let Some(v) = cur.motion[dir].get_mut(ai) {
                            *v = [0, 0];
                        }
                        a = (0, 0);
                    }
                    (mid_pred(a.0, b.0, cc.0), mid_pred(a.1, b.1, cc.1))
                }
            }
        } else {
            let b = mv(cur, r - 1, c);
            let cc = mv(cur, r - 1, c + off);
            (mid_pred(a.0, b.0, cc.0), mid_pred(a.1, b.1, cc.1))
        };
        (px, py, me)
    }

    fn decode_dc(&self, gb: &mut Gb<'_>, n: usize) -> Option<i32> {
        if n < 4 {
            if gb.peek(7) == 0x7F {
                gb.skip(18);
                return Some(255);
            }
            vlcs().dc_lum.read(gb).map(i32::from)
        } else {
            match gb.peek(9) {
                0x1FE => {
                    gb.skip(18);
                    Some(255)
                }
                0x1FF => None,
                _ => vlcs().dc_chrom.read(gb).map(i32::from),
            }
        }
    }

    fn pred_acdc(&mut self, n: usize) {
        let (x, y, plane, scale) =
            if n < 4 { (2 * self.mb_x + (n & 1), 2 * self.mb_y + (n >> 1), 0, self.y_dc_scale) } else { (self.mb_x, self.mb_y, n - 3, self.c_dc_scale) };
        let wrap = if n < 4 { 2 * self.mb_w + 2 } else { self.mb_w + 2 };
        let idx = |xx: isize, yy: isize| ((yy + 1) * wrap as isize + xx + 1).max(0) as usize;
        let me = idx(x as isize, y as isize);
        let left = idx(x as isize - 1, y as isize);
        let top = idx(x as isize, y as isize - 1);
        let dcv = &self.dc_val[plane];
        let mut a = dcv.get(left).copied().unwrap_or(1024);
        let mut c = dcv.get(top).copied().unwrap_or(1024);
        if self.first_slice_line && n != 3 {
            if n != 2 {
                c = 1024;
            }
            if n != 1 && self.mb_x == self.resync_mb_x {
                a = 1024;
            }
        }
        let block = &mut self.blocks[n];
        let pred_dc;
        if self.ac_pred {
            let mut p = 1024;
            if self.aic_dir {
                if a != 1024 {
                    let src = self.ac_val[plane].get(left).copied().unwrap_or([0; 16]);
                    for i in 1..8 {
                        block[i << 3] = block[i << 3].wrapping_add(src[i]);
                    }
                    p = a;
                }
            } else if c != 1024 {
                let src = self.ac_val[plane].get(top).copied().unwrap_or([0; 16]);
                for i in 1..8 {
                    block[i] = block[i].wrapping_add(src[i + 8]);
                }
                p = c;
            }
            pred_dc = p;
        } else if a != 1024 && c != 1024 {
            pred_dc = (a + c) >> 1;
        } else if a != 1024 {
            pred_dc = a;
        } else {
            pred_dc = c;
        }
        let mut dc = i32::from(block[0]) * scale + pred_dc;
        dc = i32::from(dc as i16);
        dc = if dc < 0 { 0 } else { dc | 1 };
        block[0] = dc as i16;
        if let Some(v) = self.dc_val[plane].get_mut(me) {
            *v = i32::from(block[0]);
        }
        if let Some(acv) = self.ac_val[plane].get_mut(me) {
            for i in 1..8 {
                acv[i] = block[i << 3];
                acv[8 + i] = block[i];
            }
        }
    }

    fn decode_block(&mut self, gb: &mut Gb<'_>, n: usize, coded: bool) -> Result<()> {
        let v = vlcs();
        let mut aic_tab = false;
        let mut scan = &ZIGZAG;
        let i: i32;
        if self.aic && self.mb_intra {
            aic_tab = true;
            i = 0;
            if self.ac_pred {
                scan = if self.aic_dir { &ALT_VERTICAL } else { &ALT_HORIZONTAL };
            }
        } else if self.mb_intra {
            let level;
            if self.codec == Codec::Rv10 && self.rv10_version == 3 && self.pict_type == PictType::I {
                let comp = n.saturating_sub(3);
                let mut l = self.last_dc[comp];
                if self.first_dc_coded[comp] {
                    let diff = self.decode_dc(gb, n).ok_or(Error::Invalid("DC code"))?;
                    l = (l + diff) & 0xff;
                    self.last_dc[comp] = l;
                } else {
                    self.first_dc_coded[comp] = true;
                }
                level = l;
            } else {
                let l = gb.get(8) as i32;
                level = if l == 255 { 128 } else { l };
            }
            self.blocks[n][0] = level as i16;
            i = 1;
        } else {
            i = 0;
        }
        if !coded {
            if self.mb_intra && self.aic {
                self.pred_acdc(n);
                self.last_index[n] = 63;
            } else {
                self.last_index[n] = i - 1;
            }
            return Ok(());
        }
        let rl = if aic_tab { &v.rl_aic } else { &v.rl_inter };
        let (runs, levels) = if aic_tab { (&INTRA_AIC_RUN, &INTRA_AIC_LEVEL) } else { (&INTER_RUN, &INTER_LEVEL) };
        let mut pos = i - 1;
        loop {
            let sym = rl.read(gb).ok_or(Error::Invalid("coefficient code"))? as usize;
            let (run, level);
            if sym == 102 {
                run = gb.get(7) as i32 + 1;
                let mut l = gb.get(8) as u8 as i8 as i32;
                if l == -128 {
                    if self.codec == Codec::Rv10 {
                        l = gb.sget(12);
                    } else {
                        let lo = gb.get(5) as i32;
                        l = lo | (gb.sget(6) * 32);
                    }
                }
                level = l;
            } else {
                run = i32::from(runs[sym]) + 1 + if sym >= RL_LAST { 192 } else { 0 };
                let l = i32::from(levels[sym]);
                level = if gb.get1() { -l } else { l };
            }
            pos += run;
            if pos >= 64 {
                pos = pos - run + ((run - 1) & 63) + 1;
                if pos < 64 {
                    self.blocks[n][usize::from(scan[pos as usize])] = level as i16;
                    break;
                }
                return Err(Error::Invalid("coefficient run overflow"));
            }
            self.blocks[n][usize::from(scan[pos as usize])] = level as i16;
        }
        if self.mb_intra && self.aic {
            self.pred_acdc(n);
            pos = 63;
        }
        self.last_index[n] = pos;
        Ok(())
    }

    fn set_direct_mv(&mut self) {
        let Some(next) = self.next.clone() else { return };
        let mb_xy = self.mb_x + self.mb_y * self.mb_w;
        let col_8x8 = next.mb_type.get(mb_xy).is_some_and(|t| t & MB_8X8 != 0);
        let one = |d: &Self, i: usize| -> [[i32; 2]; 2] {
            let r = (2 * d.mb_y + (i >> 1)) as isize;
            let c = (2 * d.mb_x + (i & 1)) as isize;
            let p = next.motion[0].get(next.mvi(r, c)).copied().unwrap_or([0, 0]);
            let (pp, pb) = (i32::from(d.pp_time as u16), i32::from(d.pb_time as u16));
            let mut out = [[0; 2]; 2];
            for k in 0..2 {
                let pv = i32::from(p[k]);
                if (pv + 32) as u32 > 63 || pp == 0 {
                    if pp != 0 {
                        out[0][k] = pv * pb / pp;
                        out[1][k] = pv * (pb - pp) / pp;
                    }
                } else {
                    out[0][k] = d.direct_scale[0][(pv + 32) as usize];
                    out[1][k] = d.direct_scale[1][(pv + 32) as usize];
                }
            }
            out
        };
        self.mv_8x8 = true;
        if col_8x8 {
            for i in 0..4 {
                let o = one(self, i);
                self.mv[0][i] = o[0];
                self.mv[1][i] = o[1];
            }
        } else {
            let o = one(self, 0);
            for i in 0..4 {
                self.mv[0][i] = o[0];
                self.mv[1][i] = o[1];
            }
        }
    }

    /// `ff_h263_decode_mb`.
    fn decode_mb(&mut self, gb: &mut Gb<'_>, cur: &mut Pic) -> Result<MbEnd> {
        let v = vlcs();
        let xy = self.mb_x + self.mb_y * self.mb_w;
        let cbpc;
        let dquant;
        self.mv_8x8 = false;
        self.mv_dir = MV_FWD;
        'parse: {
            match self.pict_type {
                PictType::P => {
                    let c = loop {
                        if gb.get1() {
                            self.mb_intra = false;
                            self.last_index = [-1; 6];
                            self.mv[0][0] = [0, 0];
                            cur.mb_type[xy] = MB_SKIP;
                            break 'parse;
                        }
                        let c = v.inter_mcbpc.read(gb).ok_or(Error::Invalid("MCBPC code"))?;
                        if c != 20 {
                            break c as i32;
                        }
                    };
                    self.blocks = [[0; 64]; 6];
                    dquant = c & 8 != 0;
                    self.mb_intra = c & 4 != 0;
                    if self.mb_intra {
                        cbpc = c;
                    } else {
                        let cbpy = i32::from(v.cbpy.read(gb).ok_or(Error::Invalid("CBPY code"))?) ^ 0xF;
                        let cbp = (c & 3) | (cbpy << 2);
                        if dquant {
                            self.decode_dquant(gb);
                        }
                        if c & 16 == 0 {
                            cur.mb_type[xy] = 0;
                            let (px, py, _) = self.pred_motion(cur, 0, 0);
                            let mx = self.decode_motion(gb, px).ok_or(Error::Invalid("motion vector code"))?;
                            let my = self.decode_motion(gb, py).ok_or(Error::Invalid("motion vector code"))?;
                            self.mv[0][0] = [mx, my];
                        } else {
                            cur.mb_type[xy] = MB_8X8;
                            self.mv_8x8 = true;
                            for i in 0..4 {
                                let (px, py, idx) = self.pred_motion(cur, i, 0);
                                let mx = self.decode_motion(gb, px).ok_or(Error::Invalid("motion vector code"))?;
                                let my = self.decode_motion(gb, py).ok_or(Error::Invalid("motion vector code"))?;
                                self.mv[0][i] = [mx, my];
                                if let Some(m) = cur.motion[0].get_mut(idx) {
                                    *m = [mx as i16, my as i16];
                                }
                            }
                        }
                        self.decode_blocks(gb, cbp)?;
                        break 'parse;
                    }
                }
                PictType::B => {
                    let r = (2 * self.mb_y) as isize;
                    let c = (2 * self.mb_x) as isize;
                    for d in 0..2 {
                        for (rr, cc) in [(r, c), (r, c + 1), (r + 1, c), (r + 1, c + 1)] {
                            let i = cur.mvi(rr, cc);
                            if let Some(m) = cur.motion[d].get_mut(i) {
                                *m = [0, 0];
                            }
                        }
                    }
                    let t = loop {
                        let t = v.mbtype_b.read(gb).ok_or(Error::Invalid("B macroblock type"))? as usize;
                        if t != 12 {
                            break t;
                        }
                    };
                    self.mb_intra = t >= 13;
                    let has_cbp = t >= 13 || t % 3 != 0;
                    let quant = t == 14 || (t < 12 && t % 3 == 2);
                    let mut cbp = 0;
                    if has_cbp {
                        self.blocks = [[0; 64]; 6];
                        let c = i32::from(v.cbpc_b.read(gb).ok_or(Error::Invalid("CBPC code"))?);
                        if self.mb_intra {
                            dquant = quant;
                            cbpc = c;
                            cur.mb_type[xy] = MB_INTRA;
                            return self.intra_mb(gb, cur, cbpc, dquant);
                        }
                        let cbpy = i32::from(v.cbpy.read(gb).ok_or(Error::Invalid("CBPY code"))?) ^ 0xF;
                        cbp = (c & 3) | (cbpy << 2);
                    } else {
                        self.blocks = [[0; 64]; 6];
                    }
                    if quant {
                        self.decode_dquant(gb);
                    }
                    if t < 3 {
                        self.mv_dir = MV_FWD | MV_BWD;
                        self.set_direct_mv();
                    } else {
                        self.mv_dir = 0;
                        let fwd = (3..6).contains(&t) || (9..12).contains(&t);
                        let bwd = (6..12).contains(&t);
                        for (dir, on) in [(0usize, fwd), (1usize, bwd)] {
                            if !on {
                                continue;
                            }
                            let (px, py, _) = self.pred_motion(cur, 0, dir);
                            self.mv_dir |= if dir == 0 { MV_FWD } else { MV_BWD };
                            let mx = self.decode_motion(gb, px).ok_or(Error::Invalid("motion vector code"))?;
                            let my = self.decode_motion(gb, py).ok_or(Error::Invalid("motion vector code"))?;
                            self.mv[dir][0] = [mx, my];
                            for (rr, cc) in [(r, c), (r, c + 1), (r + 1, c), (r + 1, c + 1)] {
                                let i = cur.mvi(rr, cc);
                                if let Some(m) = cur.motion[dir].get_mut(i) {
                                    *m = [mx as i16, my as i16];
                                }
                            }
                        }
                    }
                    cur.mb_type[xy] = if self.mv_8x8 { MB_8X8 } else { 0 };
                    self.decode_blocks(gb, cbp)?;
                    break 'parse;
                }
                PictType::I => {
                    let c = loop {
                        let c = v.intra_mcbpc.read(gb).ok_or(Error::Invalid("MCBPC code"))?;
                        if c != 8 {
                            break i32::from(c);
                        }
                    };
                    self.blocks = [[0; 64]; 6];
                    dquant = c & 4 != 0;
                    self.mb_intra = true;
                    cbpc = c;
                }
            }
            cur.mb_type[xy] = MB_INTRA;
            return self.intra_mb(gb, cur, cbpc, dquant);
        }
        self.mb_end(gb)
    }

    fn intra_mb(&mut self, gb: &mut Gb<'_>, cur: &mut Pic, cbpc: i32, dquant: bool) -> Result<MbEnd> {
        let xy = self.mb_x + self.mb_y * self.mb_w;
        cur.mb_type[xy] = MB_INTRA;
        self.mb_intra = true;
        if self.aic {
            self.ac_pred = gb.get1();
            if self.ac_pred {
                self.aic_dir = gb.get1();
            }
        } else {
            self.ac_pred = false;
        }
        let cbpy = i32::from(vlcs().cbpy.read(gb).ok_or(Error::Invalid("CBPY code"))?);
        let cbp = (cbpc & 3) | (cbpy << 2);
        if dquant {
            self.decode_dquant(gb);
        }
        self.decode_blocks(gb, cbp)?;
        self.mb_end(gb)
    }

    fn decode_blocks(&mut self, gb: &mut Gb<'_>, mut cbp: i32) -> Result<()> {
        for i in 0..6 {
            self.decode_block(gb, i, cbp & 32 != 0)?;
            cbp += cbp;
        }
        Ok(())
    }

    fn mb_end(&self, gb: &mut Gb<'_>) -> Result<MbEnd> {
        let left = gb.left();
        if left < 0 {
            return Err(Error::Invalid("slice overread"));
        }
        let mut v = gb.peek(16);
        if left < 16 {
            v >>= 16 - left;
        }
        Ok(if v == 0 { MbEnd::End } else { MbEnd::Ok })
    }

    fn update_motion_val(&self, cur: &mut Pic) {
        if self.mv_8x8 {
            return;
        }
        let mv = if self.mb_intra { [0, 0] } else { [self.mv[0][0][0] as i16, self.mv[0][0][1] as i16] };
        let r = (2 * self.mb_y) as isize;
        let c = (2 * self.mb_x) as isize;
        for (rr, cc) in [(r, c), (r, c + 1), (r + 1, c), (r + 1, c + 1)] {
            let i = cur.mvi(rr, cc);
            if let Some(m) = cur.motion[0].get_mut(i) {
                *m = mv;
            }
        }
    }

    /// Motion compensation of the current macroblock from `r` in direction `dir`.
    fn motion(&self, cur: &mut Pic, r: &Pic, dir: usize, rnd: bool, avg: bool) {
        let (bx, by) = (self.mb_x * 16, self.mb_y * 16);
        let (w, h) = (self.width as isize, self.height as isize);
        if !self.mv_8x8 {
            let [mx, my] = self.mv[dir][0];
            let dxy = ((my & 1) << 1) | (mx & 1);
            let sx = bx as isize + (mx >> 1) as isize;
            let sy = by as isize + (my >> 1) as isize;
            mc(&mut cur.planes[0], bx, by, &r.planes[0], sx, sy, 16, 16, dxy, rnd, avg);
            let uvdxy = dxy | (my & 2) | ((mx & 2) >> 1);
            for p in 1..3 {
                mc(&mut cur.planes[p], bx / 2, by / 2, &r.planes[p], sx >> 1, sy >> 1, 8, 8, uvdxy, rnd, avg);
            }
            return;
        }
        let (mut sumx, mut sumy) = (0, 0);
        for i in 0..4 {
            let [mx, my] = self.mv[dir][i];
            let ox = bx + (i & 1) * 8;
            let oy = by + (i >> 1) * 8;
            let mut dxy = 0;
            let sx = (ox as isize + (mx >> 1) as isize).clamp(-16, w);
            if sx != w {
                dxy |= mx & 1;
            }
            let sy = (oy as isize + (my >> 1) as isize).clamp(-16, h);
            if sy != h {
                dxy |= (my & 1) << 1;
            }
            mc(&mut cur.planes[0], ox, oy, &r.planes[0], sx, sy, 8, 8, dxy, rnd, avg);
            sumx += mx;
            sumy += my;
        }
        let mx = round_chroma(sumx);
        let my = round_chroma(sumy);
        let mut dxy = ((my & 1) << 1) | (mx & 1);
        let sx = ((self.mb_x * 8) as isize + (mx >> 1) as isize).clamp(-8, w >> 1);
        if sx == w >> 1 {
            dxy &= !1;
        }
        let sy = ((self.mb_y * 8) as isize + (my >> 1) as isize).clamp(-8, h >> 1);
        if sy == h >> 1 {
            dxy &= !2;
        }
        for p in 1..3 {
            mc(&mut cur.planes[p], bx / 2, by / 2, &r.planes[p], sx, sy, 8, 8, dxy, rnd, avg);
        }
    }

    fn dequant(&mut self, n: usize, intra: bool) {
        let q = if n < 4 { self.qscale } else { self.chroma_qscale };
        let qmul = q << 1;
        let block = &mut self.blocks[n];
        let mut qadd = (q - 1) | 1;
        let start = if intra {
            if !self.aic {
                block[0] = (i32::from(block[0]) * if n < 4 { self.y_dc_scale } else { self.c_dc_scale }) as i16;
            } else {
                qadd = 0;
            }
            1
        } else {
            0
        };
        for v in block.iter_mut().skip(start) {
            let l = i32::from(*v);
            if l != 0 {
                *v = if l < 0 { l * qmul - qadd } else { l * qmul + qadd } as i16;
            }
        }
    }

    fn reconstruct(&mut self, cur: &mut Pic) {
        let xy = self.mb_x + self.mb_y * self.mb_w;
        cur.qscale[xy] = self.qscale as u8;
        let (bx, by) = (self.mb_x * 16, self.mb_y * 16);
        let offs = |n: usize| -> (usize, usize, usize) { if n < 4 { (0, bx + (n & 1) * 8, by + (n >> 1) * 8) } else { (n - 3, bx / 2, by / 2) } };
        if !self.mb_intra {
            let rnd = !self.no_rounding || self.pict_type == PictType::B;
            let mut avg = false;
            if self.mv_dir & MV_FWD != 0 {
                if let Some(r) = self.last.clone() {
                    self.motion(cur, &r, 0, rnd, false);
                }
                avg = true;
            }
            if self.mv_dir & MV_BWD != 0
                && let Some(r) = self.next.clone()
            {
                self.motion(cur, &r, 1, rnd, avg);
            }
            for n in 0..6 {
                if self.last_index[n] >= 0 {
                    self.dequant(n, false);
                    let (p, x, y) = offs(n);
                    let pl = &mut cur.planes[p];
                    let stride = pl.w;
                    idct(&mut self.blocks[n], &mut pl.d, y * stride + x, stride, true);
                }
            }
        } else {
            for n in 0..6 {
                self.dequant(n, true);
                let (p, x, y) = offs(n);
                let pl = &mut cur.planes[p];
                let stride = pl.w;
                idct(&mut self.blocks[n], &mut pl.d, y * stride + x, stride, false);
            }
        }
    }

    /// `ff_h263_loop_filter` for the current macroblock.
    fn h263_loop_filter(&self, cur: &mut Pic) {
        let xy = self.mb_x + self.mb_y * self.mb_w;
        let (x, y) = (self.mb_x * 16, self.mb_y * 16);
        let (cx, cy) = (x / 2, y / 2);
        let skip = |i: usize| cur.mb_type.get(i).is_some_and(|t| t & MB_SKIP != 0);
        let qs = |i: usize| cur.qscale.get(i).copied().unwrap_or(0);
        let cq = |q: u8| H263_CHROMA_QSCALE[usize::from(q & 31)];
        let qp_c = if !skip(xy) {
            let q = self.qscale as u8;
            v_loop_filter(&mut cur.planes[0], x, y + 8, q);
            v_loop_filter(&mut cur.planes[0], x + 8, y + 8, q);
            q
        } else {
            0
        };
        if self.mb_y > 0 {
            let up = xy - self.mb_w;
            let qp_tt = if skip(up) { 0 } else { qs(up) };
            let qp_tc = if qp_c != 0 { qp_c } else { qp_tt };
            if qp_tc != 0 {
                let c = cq(qp_tc);
                v_loop_filter(&mut cur.planes[0], x, y, qp_tc);
                v_loop_filter(&mut cur.planes[0], x + 8, y, qp_tc);
                v_loop_filter(&mut cur.planes[1], cx, cy, c);
                v_loop_filter(&mut cur.planes[2], cx, cy, c);
            }
            if qp_tt != 0 {
                h_loop_filter(&mut cur.planes[0], x + 8, y - 8, qp_tt);
            }
            if self.mb_x > 0 {
                let qp_dt = if qp_tt != 0 || skip(up - 1) { qp_tt } else { qs(up - 1) };
                if qp_dt != 0 {
                    let c = cq(qp_dt);
                    h_loop_filter(&mut cur.planes[0], x, y - 8, qp_dt);
                    h_loop_filter(&mut cur.planes[1], cx, cy - 8, c);
                    h_loop_filter(&mut cur.planes[2], cx, cy - 8, c);
                }
            }
        }
        if qp_c != 0 {
            h_loop_filter(&mut cur.planes[0], x + 8, y, qp_c);
            if self.mb_y + 1 == self.mb_h {
                h_loop_filter(&mut cur.planes[0], x + 8, y + 8, qp_c);
            }
        }
        if self.mb_x > 0 {
            let qp_lc = if qp_c != 0 || skip(xy - 1) { qp_c } else { qs(xy - 1) };
            if qp_lc != 0 {
                h_loop_filter(&mut cur.planes[0], x, y, qp_lc);
                if self.mb_y + 1 == self.mb_h {
                    let c = cq(qp_lc);
                    h_loop_filter(&mut cur.planes[0], x, y + 8, qp_lc);
                    h_loop_filter(&mut cur.planes[1], cx, cy, c);
                    h_loop_filter(&mut cur.planes[2], cx, cy, c);
                }
            }
        }
    }

    /// Ends the picture being decoded; it becomes a reference unless it is a B-picture.
    fn finish(&mut self) -> Option<Pic> {
        let mut cur = self.cur.take()?;
        // Conceal macroblocks that were not decoded (damaged or missing slices).
        let reference = self.last.clone();
        for (k, &done) in self.decoded.iter().enumerate() {
            if done {
                continue;
            }
            let (mx, my) = (k % self.mb_w, k / self.mb_w);
            for p in 0..3 {
                let s = if p == 0 { 16 } else { 8 };
                let w = cur.planes[p].w;
                for yy in my * s..(my + 1) * s {
                    for xx in mx * s..(mx + 1) * s {
                        let v = reference.as_ref().and_then(|r| r.planes[p].d.get(yy * w + xx).copied()).unwrap_or(128);
                        if let Some(d) = cur.planes[p].d.get_mut(yy * w + xx) {
                            *d = v;
                        }
                    }
                }
            }
        }
        if cur.pict_type != PictType::B {
            self.next = Some(Arc::new(cur.clone()));
        }
        Some(cur)
    }

    fn start_frame(&mut self) {
        let fill = 128;
        if self.pict_type != PictType::B {
            self.last = self.next.take();
        }
        if self.last.is_none() && self.pict_type != PictType::I {
            self.last = Some(Arc::new(Pic::new(self.mb_w, self.mb_h, PictType::I, fill)));
        }
        if self.next.is_none() && self.pict_type == PictType::B {
            self.next = Some(Arc::new(Pic::new(self.mb_w, self.mb_h, PictType::I, fill)));
        }
        if self.pict_type == PictType::I {
            for v in &mut self.dc_val {
                v.fill(1024);
            }
            for v in &mut self.ac_val {
                v.fill([0; 16]);
            }
        }
        let mut cur = Pic::new(self.mb_w, self.mb_h, self.pict_type, 0);
        cur.index = self.frames_started;
        self.frames_started = self.frames_started.saturating_add(1);
        self.cur = Some(cur);
        self.decoded = vec![false; self.mb_w * self.mb_h];
    }

    /// One slice (`rv10_decode_packet`); returns the bits the slice used.
    fn decode_slice(&mut self, data: &[u8], size: usize, size2: usize, whole_size: usize) -> Result<usize> {
        let mut active = size * 8;
        let mut gb = Gb::new(data, size.max(size2) * 8);
        let count = match self.codec {
            Codec::Rv10 => self.rv10_header(&mut gb)?,
            Codec::Rv20 => self.rv20_header(&mut gb, whole_size)?,
        };
        if self.mb_x >= self.mb_w || self.mb_y >= self.mb_h {
            return Err(Error::Invalid("slice position"));
        }
        let mb_pos = self.mb_y * self.mb_w + self.mb_x;
        if count > self.mb_w * self.mb_h - mb_pos {
            return Err(Error::Invalid("slice macroblock count"));
        }
        if whole_size < self.mb_w * self.mb_h / 8 {
            return Err(Error::Invalid("frame too small"));
        }
        if (self.mb_x == 0 && self.mb_y == 0) || self.cur.is_none() {
            if self.cur.is_some() {
                let (mx, my) = (self.mb_x, self.mb_y);
                let _ = self.finish();
                self.mb_x = mx;
                self.mb_y = my;
            }
            self.start_frame();
        } else if self.cur.as_ref().is_some_and(|c| c.pict_type != self.pict_type) {
            return Err(Error::Invalid("slice type mismatch"));
        }
        match self.codec {
            Codec::Rv10 => {
                if self.mb_y == 0 {
                    self.first_slice_line = true;
                }
            }
            Codec::Rv20 => {
                self.first_slice_line = true;
                self.resync_mb_x = self.mb_x;
            }
        }
        self.set_qscale(self.qscale);
        self.first_dc_coded = [false; 3];
        let mut cur = self.cur.take().ok_or(Error::Invalid("no picture"))?;
        let r = self.decode_mbs(&mut gb, &mut cur, count, &mut active, size2);
        self.cur = Some(cur);
        r?;
        Ok(active)
    }

    fn decode_mbs(&mut self, gb: &mut Gb<'_>, cur: &mut Pic, count: usize, active: &mut usize, size2: usize) -> Result<()> {
        let mut left = count;
        while left > 0 {
            self.mb_num_left = left;
            self.mv_dir = MV_FWD;
            self.mv_8x8 = false;
            let mut ret = self.decode_mb(gb, cur)?;
            if *active >= gb.count() {
                let mut v = gb.peek(16);
                if gb.count() + 16 > *active {
                    v >>= gb.count() + 16 - *active;
                }
                if v == 0 {
                    ret = MbEnd::End;
                }
            }
            if *active < gb.count() && 8 * size2 >= gb.count() {
                *active = size2 * 8;
                ret = MbEnd::Ok;
            }
            if *active < gb.count() {
                return Err(Error::Invalid("slice overread"));
            }
            if self.pict_type != PictType::B {
                self.update_motion_val(cur);
            }
            self.reconstruct(cur);
            if self.loop_filter {
                self.h263_loop_filter(cur);
            }
            if let Some(d) = self.decoded.get_mut(self.mb_x + self.mb_y * self.mb_w) {
                *d = true;
            }
            self.mb_x += 1;
            if self.mb_x == self.mb_w {
                self.mb_x = 0;
                self.mb_y += 1;
            }
            if self.mb_x == self.resync_mb_x {
                self.first_slice_line = false;
            }
            if ret == MbEnd::End {
                break;
            }
            left -= 1;
        }
        Ok(())
    }

    fn picture(&self, p: &Pic) -> Picture {
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let crop = |pl: &Plane, w: usize, h: usize| {
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                out.extend_from_slice(pl.d.get(y * pl.w..y * pl.w + w).unwrap_or(&[]));
            }
            out
        };
        Picture {
            width: self.width,
            height: self.height,
            y: crop(&p.planes[0], w, h),
            u: crop(&p.planes[1], cw, ch),
            v: crop(&p.planes[2], cw, ch),
            pict_type: p.pict_type,
            decode_index: p.index,
        }
    }

    /// Decodes one frame as the RealMedia demuxer assembles it. Returns the next picture in
    /// display order, if one is ready (streams with B-frames output each anchor one frame late).
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Option<Picture>> {
        let Some((&n, rest)) = pkt.split_first() else { return Ok(None) };
        let slice_count = usize::from(n) + 1;
        if rest.len() <= 8 * slice_count {
            return Err(Error::Invalid("slice count"));
        }
        let (hdr, buf) = rest.split_at(8 * slice_count);
        let off = |i: usize| -> usize { hdr.get(i * 8 + 4..i * 8 + 8).map_or(usize::MAX, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) };
        let buf_size = buf.len();
        let mut i = 0;
        let mut first_err = None;
        while i < slice_count {
            let offset = off(i);
            if offset >= buf_size {
                first_err.get_or_insert(Error::Invalid("slice offset"));
                break;
            }
            let size = if i + 1 == slice_count { buf_size - offset } else { off(i + 1).wrapping_sub(offset) };
            let size2 = if i + 2 >= slice_count { buf_size - offset } else { off(i + 2).wrapping_sub(offset) };
            if size == 0 || size2 == 0 || size > buf_size || size2 > buf_size || offset + size.max(size2) > buf_size {
                first_err.get_or_insert(Error::Invalid("slice size"));
                break;
            }
            match self.decode_slice(&buf[offset..], size, size2, buf_size) {
                Ok(bits) => {
                    if bits > 8 * size {
                        i += 1;
                    }
                }
                Err(e) => {
                    first_err.get_or_insert(e);
                    if self.cur.is_none() {
                        break;
                    }
                }
            }
            i += 1;
        }
        let complete = self.cur.is_some() && (self.mb_y >= self.mb_h || first_err.is_some());
        if !complete {
            return match first_err {
                Some(e) => Err(e),
                None => Ok(None),
            };
        }
        let pict = self.pict_type;
        let Some(cur) = self.finish() else { return Ok(None) };
        self.mb_x = 0;
        self.mb_y = 0;
        if pict == PictType::B || self.low_delay {
            return Ok(Some(self.picture(&cur)));
        }
        Ok(self.last.as_ref().filter(|l| self.flushed != Some(l.index)).map(|l| self.picture(l)))
    }

    /// The last reference picture of a stream with B-frames, which [`Decoder::decode`] holds back
    /// until the next reference picture (ffmpeg drops it at the end of a stream). The decoder
    /// keeps it as its reference, so decoding can go on; it is not returned a second time.
    pub fn flush(&mut self) -> Option<Picture> {
        if self.low_delay {
            return None;
        }
        let n = self.next.clone()?;
        if self.flushed == Some(n.index) {
            return None;
        }
        self.flushed = Some(n.index);
        Some(self.picture(&n))
    }

    /// How many frames this decoder has started (a packet that starts a frame raises it by one).
    pub fn frames_started(&self) -> u64 {
        self.frames_started
    }

    /// The current picture size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// The picture type of a packet (the slice table and the first slice header), without decoding.
pub fn picture_type(codec: Codec, pkt: &[u8]) -> Option<PictType> {
    let n = usize::from(*pkt.first()?) + 1;
    let off = u32::from_le_bytes(pkt.get(5..9)?.try_into().ok()?) as usize;
    let b = *pkt.get(1 + 8 * n + off)?;
    Some(match codec {
        Codec::Rv10 => {
            if b & 0x40 != 0 {
                PictType::P
            } else {
                PictType::I
            }
        }
        Codec::Rv20 => match b >> 6 {
            0 | 1 => PictType::I,
            2 => PictType::P,
            _ => PictType::B,
        },
    })
}
