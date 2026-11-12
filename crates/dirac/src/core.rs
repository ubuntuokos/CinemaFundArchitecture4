// Ported from FFmpeg n7.1.1 libavcodec/diracdec.c and diracdsp.c (Copyright (C) 2007 Marco
// Gerards, 2009 David Conrad, 2011 Jordi Ortiz), LGPL-2.1-or-later; see docs/licensing.md §5.
// CFA4 changes GPL-3.0-or-later.

//! Dirac core syntax pictures (Dirac specification 2.2.3 §11–§15): intra and inter pictures with
//! arithmetic or exp-Golomb coded wavelet coefficients in codeblocks, block motion data
//! (superblock splitting, prediction modes, motion vectors, DC blocks, global motion), and
//! overlapped block motion compensation from up to two reference pictures at up to 1/8 pel.
//!
//! Inter pictures need 8-bit video (as FFmpeg's decoder, the test oracle); intra pictures take
//! any depth.

use crate::arith::{self, Arith};
use crate::bits::Bits;
use crate::header::{ChromaFormat, Sequence};
use crate::picture::{self, Band, Coeffs, Quant};
use crate::{Error, Result};

/// Deepest core transform (FFmpeg's `MAX_DWT_LEVELS`).
const MAX_DEPTH: u64 = 5;
/// Largest block length (FFmpeg's `MAX_BLOCKSIZE`).
const MAX_BLOCK: u64 = 32;
/// Largest quantiser index of core syntax coefficients (FFmpeg's `DIRAC_MAX_QUANT_INDEX` - 1).
const MAX_QUANT: i64 = 115;
/// Edge of the reference planes MC may read before it clamps (FFmpeg's `EDGE_WIDTH / 2`).
const EDGE: i64 = 8;

/// What the picture's parse code says (§9.6.1).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Kind {
    pub num_refs: usize,
    pub arith: bool,
    pub reference: bool,
    /// Schrödinger before 1.0.8 left out the quantiser delta of single-codeblock subbands.
    pub old_delta_quant: bool,
}

impl Kind {
    pub fn new(parse_code: u8, old_delta_quant: bool) -> Self {
        Self { num_refs: usize::from(parse_code & 3), arith: parse_code & 0x48 == 0x08, reference: parse_code & 0x0C == 0x0C, old_delta_quant }
    }
}

/// A decoded picture kept as a reference: its samples and, once MC needs them, the half-pel
/// planes (H, V, C) of each component.
pub(crate) struct Reference {
    pub number: u32,
    /// The components as bytes (references serve 8-bit inter pictures only).
    bytes: [Vec<u8>; 3],
    hpel: Option<Box<[[Vec<u8>; 3]; 3]>>,
}

impl Reference {
    pub fn new(number: u32, planes: &[Vec<u16>; 3]) -> Self {
        let bytes = planes.clone().map(|p| p.into_iter().map(|v| v.min(255) as u8).collect());
        Self { number, bytes, hpel: None }
    }
}

/// The header of a core picture: its number, the references it predicts from, the picture it
/// retires.
pub(crate) struct Header {
    pub number: u32,
    pub refs: Vec<u32>,
    pub retire: Option<u32>,
}

#[derive(Clone, Copy, Default)]
struct Global {
    pan_tilt: [i64; 2],
    zrs: [[i64; 2]; 2],
    perspective: [i64; 2],
    zrs_exp: u32,
    perspective_exp: u32,
}

/// Picture prediction parameters (§11.2).
struct Prediction {
    xblen: usize,
    yblen: usize,
    xbsep: usize,
    ybsep: usize,
    precision: u32,
    global: Option<[Global; 2]>,
    weight_log2: u32,
    weights: [i32; 2],
}

/// One block's motion data. `mv` doubles as the three DC values of a DC block (as in FFmpeg,
/// where they share storage: DC 0, 1, 2 are `mv[0][0]`, `mv[0][1]`, `mv[1][0]`).
#[derive(Clone, Copy, Default)]
struct Block {
    mv: [[i16; 2]; 2],
    refs: u8,
}

const REF1: u8 = 1;
const REF2: u8 = 2;
const GLOBAL: u8 = 4;

impl Block {
    fn dc(&self, c: usize) -> i16 {
        match c {
            0 => self.mv[0][0],
            1 => self.mv[0][1],
            _ => self.mv[1][0],
        }
    }

    fn dc_mut(&mut self, c: usize) -> &mut i16 {
        match c {
            0 => &mut self.mv[0][0],
            1 => &mut self.mv[0][1],
            _ => &mut self.mv[1][0],
        }
    }
}

/// The block motion field of a picture.
struct Motion {
    pred: Prediction,
    blwidth: usize,
    blheight: usize,
    blocks: Vec<Block>,
}

pub(crate) fn header(b: &mut Bits, kind: Kind) -> Result<Header> {
    let number = b.read_nbits(32)? as u32;
    let mut refs = Vec::with_capacity(kind.num_refs);
    for _ in 0..kind.num_refs {
        refs.push(number.wrapping_add(b.read_sint()? as u32));
    }
    let retire = if kind.reference { Some(number.wrapping_add(b.read_sint()? as u32)) } else { None };
    Ok(Header { number, refs, retire })
}

fn prediction(b: &mut Bits, seq: &Sequence, num_refs: usize) -> Result<Prediction> {
    const DEFAULT_BLEN: [u64; 4] = [4, 12, 16, 24];
    b.byte_align();
    let index = b.read_uint()?;
    let (xblen, yblen, xbsep, ybsep) = match index {
        0 => (b.read_uint()?, b.read_uint()?, b.read_uint()?, b.read_uint()?),
        1..=4 => {
            let l = DEFAULT_BLEN[index as usize - 1];
            (l, l, 4 * index, 4 * index)
        }
        _ => return Err(Error::Invalid("block parameters index")),
    };
    let (cx, cy) = chroma_shift(seq.video.chroma);
    if xblen == 0 || yblen == 0 || xblen % (1 << cx) != 0 || yblen % (1 << cy) != 0 {
        return Err(Error::Invalid("block length"));
    }
    if xbsep == 0 || ybsep == 0 || xbsep < xblen / 2 || ybsep < yblen / 2 || xbsep > xblen || ybsep > yblen {
        return Err(Error::Invalid("block separation"));
    }
    if xblen.max(yblen) > MAX_BLOCK {
        return Err(Error::Unsupported("blocks longer than 32"));
    }
    let precision = b.read_uint()?;
    if precision > 3 {
        return Err(Error::Invalid("motion vector precision"));
    }
    let global = if b.read_bool()? {
        let mut g = [Global::default(); 2];
        for gr in g.iter_mut().take(num_refs) {
            if b.read_bool()? {
                gr.pan_tilt = [b.read_sint()?, b.read_sint()?];
            }
            if b.read_bool()? {
                gr.zrs_exp = u32::try_from(b.read_uint()?).map_err(|_| Error::Invalid("global motion"))?;
                gr.zrs = [[b.read_sint()?, b.read_sint()?], [b.read_sint()?, b.read_sint()?]];
            } else {
                gr.zrs = [[1, 0], [0, 1]];
            }
            if b.read_bool()? {
                gr.perspective_exp = u32::try_from(b.read_uint()?).map_err(|_| Error::Invalid("global motion"))?;
                gr.perspective = [b.read_sint()?, b.read_sint()?];
            }
            if u64::from(gr.zrs_exp) + u64::from(gr.perspective_exp) > 30 {
                return Err(Error::Invalid("global motion exponents"));
            }
            let big = |v: i64| v.unsigned_abs() > 1 << 30;
            if gr.pan_tilt.iter().chain(gr.perspective.iter()).chain(gr.zrs.iter().flatten()).any(|&v| big(v)) {
                return Err(Error::Invalid("global motion parameter"));
            }
        }
        Some(g)
    } else {
        None
    };
    if b.read_uint()? != 0 {
        return Err(Error::Invalid("picture prediction mode"));
    }
    let (mut weight_log2, mut weights) = (1, [1, 1]);
    if b.read_bool()? {
        let l = b.read_uint()?;
        if !(1..=8).contains(&l) {
            return Err(Error::Invalid("reference weight precision"));
        }
        weight_log2 = l as u32;
        let w = |v: i64| i32::try_from(v).ok().filter(|v| v.unsigned_abs() <= 1 << 16).ok_or(Error::Invalid("reference weight"));
        weights[0] = w(b.read_sint()?)?;
        if num_refs == 2 {
            weights[1] = w(b.read_sint()?)?;
        }
    }
    Ok(Prediction {
        xblen: xblen as usize,
        yblen: yblen as usize,
        xbsep: xbsep as usize,
        ybsep: ybsep as usize,
        precision: precision as u32,
        global,
        weight_log2,
        weights,
    })
}

fn chroma_shift(c: ChromaFormat) -> (u32, u32) {
    match c {
        ChromaFormat::C444 => (0, 0),
        ChromaFormat::C422 => (1, 0),
        ChromaFormat::C420 => (1, 1),
    }
}

fn mid3(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// `(x + 1) / 3` rounded down, as FFmpeg's `divide3`.
fn divide3(x: i32) -> i32 {
    (((i64::from(x) + 1) * 21845 + 10922) >> 16) as i32
}

impl Motion {
    fn at(&self, x: usize, y: usize) -> Block {
        self.blocks.get(y * self.blwidth + x).copied().unwrap_or_default()
    }

    fn pred_mode(&self, x: usize, y: usize, mask: u8) -> u8 {
        if x == 0 && y == 0 {
            return 0;
        }
        if y == 0 {
            return self.at(x - 1, y).refs & mask;
        }
        if x == 0 {
            return self.at(x, y - 1).refs & mask;
        }
        let sum = u32::from(self.at(x - 1, y).refs & mask) + u32::from(self.at(x, y - 1).refs & mask) + u32::from(self.at(x - 1, y - 1).refs & mask);
        ((sum >> 1) as u8) & mask
    }

    fn pred_dc(&self, x: usize, y: usize) -> [i16; 3] {
        let mut dc = [0i16; 3];
        let mut n = 0;
        let mut add = |b: Block| {
            for (c, d) in dc.iter_mut().enumerate() {
                *d = d.wrapping_add(b.dc(c));
            }
            n += 1;
        };
        if x > 0 && self.at(x - 1, y).refs & 3 == 0 {
            add(self.at(x - 1, y));
        }
        if y > 0 && self.at(x, y - 1).refs & 3 == 0 {
            add(self.at(x, y - 1));
        }
        if x > 0 && y > 0 && self.at(x - 1, y - 1).refs & 3 == 0 {
            add(self.at(x - 1, y - 1));
        }
        match n {
            2 => dc.map(|d| ((i32::from(d) + 1) >> 1) as i16),
            3 => dc.map(|d| divide3(i32::from(d)) as i16),
            _ => dc,
        }
    }

    fn pred_mv(&self, x: usize, y: usize, r: usize) -> [i16; 2] {
        let refmask = r as u8 + 1;
        let mask = refmask | GLOBAL;
        let mut p: Vec<[i16; 2]> = Vec::with_capacity(3);
        if x > 0 && self.at(x - 1, y).refs & mask == refmask {
            p.push(self.at(x - 1, y).mv[r]);
        }
        if y > 0 && self.at(x, y - 1).refs & mask == refmask {
            p.push(self.at(x, y - 1).mv[r]);
        }
        if x > 0 && y > 0 && self.at(x - 1, y - 1).refs & mask == refmask {
            p.push(self.at(x - 1, y - 1).mv[r]);
        }
        let c = |k: usize, i: usize| p.get(k).map_or(0, |v| i32::from(v[i]));
        match p.len() {
            1 => [c(0, 0) as i16, c(0, 1) as i16],
            2 => [((c(0, 0) + c(1, 0) + 1) >> 1) as i16, ((c(0, 1) + c(1, 1) + 1) >> 1) as i16],
            3 => [mid3(c(0, 0), c(1, 0), c(2, 0)) as i16, mid3(c(0, 1), c(1, 1), c(2, 1)) as i16],
            _ => [0, 0],
        }
    }
}

fn global_mv(g: &Global, x: usize, y: usize) -> [i16; 2] {
    let (x, y) = (x as i64, y as i64);
    let (ez, ep) = (g.zrs_exp, g.perspective_exp);
    let m = (1i64 << ep).wrapping_sub(g.perspective[0].wrapping_mul(x).wrapping_add(g.perspective[1].wrapping_mul(y)));
    let a = &g.zrs;
    let mx = m.wrapping_mul(a[0][0].wrapping_mul(x).wrapping_add(a[0][1].wrapping_mul(y)).wrapping_add((1i64 << ez).wrapping_mul(g.pan_tilt[0])));
    let my = m.wrapping_mul(a[1][0].wrapping_mul(x).wrapping_add(a[1][1].wrapping_mul(y)).wrapping_add((1i64 << ez).wrapping_mul(g.pan_tilt[1])));
    let s = ez + ep;
    let r = 1i64 << s;
    [(mx.wrapping_add(r) >> s) as i16, (my.wrapping_add(r) >> s) as i16]
}

/// Block motion data (§12).
fn motion(b: &mut Bits, seq: &Sequence, num_refs: usize) -> Result<Motion> {
    let pred = prediction(b, seq, num_refs)?;
    b.byte_align();
    let (w, h) = (seq.luma_width as usize, seq.luma_height as usize);
    let sbwidth = w.div_ceil(4 * pred.xbsep);
    let sbheight = h.div_ceil(4 * pred.ybsep);
    let (blwidth, blheight) = (4 * sbwidth, 4 * sbheight);
    if blwidth.saturating_mul(blheight) > 1 << 22 {
        return Err(Error::Unsupported("too many blocks"));
    }
    let mut a = Arith::new(b.block()?);
    let mut split = vec![0u8; sbwidth * sbheight];
    for y in 0..sbheight {
        for x in 0..sbwidth {
            let v = a.uint(arith::SB_F1, arith::SB_DATA);
            if v > 2 {
                return Err(Error::Invalid("superblock split"));
            }
            let at = |x: usize, y: usize| split.get(y * sbwidth + x).copied().unwrap_or(0);
            let p = if x == 0 && y == 0 {
                0
            } else if y == 0 {
                at(x - 1, y)
            } else if x == 0 {
                at(x, y - 1)
            } else {
                [0, 0, 1, 1, 1, 2, 2].get(usize::from(at(x - 1, y) + at(x, y - 1) + at(x - 1, y - 1))).copied().unwrap_or(0)
            };
            if let Some(s) = split.get_mut(y * sbwidth + x) {
                *s = ((v as u8) + p) % 3;
            }
        }
    }
    if a.damaged() {
        return Err(Error::Invalid("superblock data overread"));
    }
    let mut modes = Arith::new(b.block()?);
    let mut mvs: Vec<[Arith; 2]> = Vec::with_capacity(num_refs);
    for _ in 0..num_refs {
        mvs.push([Arith::new(b.block()?), Arith::new(b.block()?)]);
    }
    let mut dcs = [Arith::new(b.block()?), Arith::new(b.block()?), Arith::new(b.block()?)];
    let mut m = Motion { pred, blwidth, blheight, blocks: vec![Block::default(); blwidth * blheight] };
    for sy in 0..sbheight {
        for sx in 0..sbwidth {
            let s = split.get(sy * sbwidth + sx).copied().unwrap_or(0);
            let (count, step) = (1usize << s, 4usize >> s);
            for q in 0..count {
                for p in 0..count {
                    let (bx, by) = (4 * sx + p * step, 4 * sy + q * step);
                    let blk = block(&m, &mut modes, &mut mvs, &mut dcs, num_refs, bx, by);
                    for y in by..by + step {
                        for x in bx..bx + step {
                            if let Some(d) = m.blocks.get_mut(y * blwidth + x) {
                                *d = blk;
                            }
                        }
                    }
                }
            }
        }
    }
    if modes.damaged() || mvs.iter().flatten().any(Arith::damaged) || dcs.iter().any(Arith::damaged) {
        return Err(Error::Invalid("block data overread"));
    }
    Ok(m)
}

fn block(m: &Motion, modes: &mut Arith, mvs: &mut [[Arith; 2]], dcs: &mut [Arith; 3], num_refs: usize, x: usize, y: usize) -> Block {
    let mut blk = m.at(x, y);
    blk.refs = m.pred_mode(x, y, REF1) ^ u8::from(modes.bit(arith::PMODE_REF1));
    if num_refs == 2 {
        blk.refs |= m.pred_mode(x, y, REF2);
        blk.refs ^= u8::from(modes.bit(arith::PMODE_REF2)) << 1;
    }
    if blk.refs == 0 {
        let dc = m.pred_dc(x, y);
        for (c, a) in dcs.iter_mut().enumerate() {
            *blk.dc_mut(c) = dc[c].wrapping_add(a.int(arith::DC_F1, arith::DC_DATA) as i16);
        }
        return blk;
    }
    if m.pred.global.is_some() {
        blk.refs |= m.pred_mode(x, y, GLOBAL);
        blk.refs ^= u8::from(modes.bit(arith::GLOBAL_BLOCK)) << 2;
    }
    for r in 0..num_refs {
        if blk.refs & (r as u8 + 1) == 0 {
            continue;
        }
        if blk.refs & GLOBAL != 0
            && let Some(g) = m.pred.global.as_ref().and_then(|g| g.get(r))
        {
            blk.mv[r] = global_mv(g, x, y);
        } else {
            let p = m.pred_mv(x, y, r);
            if let Some([ax, ay]) = mvs.get_mut(r) {
                blk.mv[r] = [p[0].wrapping_add(ax.int(arith::MV_F1, arith::MV_DATA) as i16), p[1].wrapping_add(ay.int(arith::MV_F1, arith::MV_DATA) as i16)];
            }
        }
    }
    blk
}

/// Transform parameters of a core picture (§11.3): `None` for a zero residual.
struct CoreTransform {
    wavelet: u64,
    depth: u32,
    codeblocks: Vec<(usize, usize)>,
    codeblock_mode: bool,
}

fn transform(b: &mut Bits, seq: &Sequence, num_refs: usize) -> Result<Option<CoreTransform>> {
    b.byte_align();
    if num_refs > 0 && b.read_bool()? {
        return Ok(None);
    }
    let wavelet = b.read_uint()?;
    if wavelet > 6 {
        return Err(Error::Invalid("wavelet index"));
    }
    let depth = b.read_uint()?;
    if !(1..=MAX_DEPTH).contains(&depth) {
        return Err(Error::Unsupported("transform depth"));
    }
    let depth = depth as u32;
    let mut codeblocks = vec![(1, 1); depth as usize + 1];
    let mut codeblock_mode = false;
    if b.read_bool()? {
        for (i, cb) in codeblocks.iter_mut().enumerate() {
            let shift = depth - i as u32;
            let (w, h) = (b.read_uint()?, b.read_uint()?);
            if w < 1 || w > u64::from(seq.luma_width >> shift) || h < 1 || h > u64::from(seq.luma_height >> shift) {
                return Err(Error::Invalid("codeblock count"));
            }
            *cb = (w as usize, h as usize);
        }
        let mode = b.read_uint()?;
        if mode > 1 {
            return Err(Error::Invalid("codeblock mode"));
        }
        codeblock_mode = mode == 1;
    }
    Ok(Some(CoreTransform { wavelet, depth, codeblocks, codeblock_mode }))
}

/// The coefficients of one subband being decoded.
struct Subband<'a> {
    band: Band,
    coeffs: &'a mut [i32],
    /// The parent subband (same orientation, one level coarser) and its width.
    parent: Option<(&'a [i32], usize)>,
    /// 1: HL, 2: LH (sign prediction from above or from the left), 0 otherwise.
    orientation: usize,
    codeblocks: (usize, usize),
    quant: i64,
}

struct CoefficientCoding<'q> {
    quant: &'q Quant,
    arith: bool,
    inter: bool,
    codeblock_mode: bool,
    old_delta_quant: bool,
    /// 8-bit video keeps coefficients in 16 bits, as FFmpeg does.
    narrow: bool,
}

impl CoefficientCoding<'_> {
    fn dequant(&self, v: u32, q: i64) -> i32 {
        let v = self.quant.inverse_with(i64::from(v), q as usize, self.inter);
        if self.narrow { v as i16 as i32 } else { v }
    }

    fn arith_subband(&self, s: &mut Subband, data: &[u8]) -> Result<()> {
        let mut a = Arith::new(data);
        let (cw, ch) = s.codeblocks;
        let one = cw + ch == 2;
        let (w, h) = (s.band.w, s.band.h);
        let mut top = 0;
        for cy in 0..ch {
            let bottom = h * (cy + 1) / ch;
            let mut left = 0;
            for cx in 0..cw {
                let right = w * (cx + 1) / cw;
                let mut skip = false;
                if !one && a.bit(arith::ZERO_BLOCK) {
                    skip = true;
                }
                if !skip {
                    if self.codeblock_mode && !(self.old_delta_quant && one) {
                        s.quant += i64::from(a.int(arith::DELTA_Q_F, arith::DELTA_Q_DATA));
                    }
                    if !(0..=MAX_QUANT).contains(&s.quant) {
                        return Err(Error::Invalid("quantiser index"));
                    }
                    for y in top..bottom {
                        if a.damaged() {
                            return Err(Error::Invalid("coefficient data overread"));
                        }
                        for x in left..right {
                            let v = self.arith_coeff(s, &mut a, x, y);
                            if let Some(c) = s.coeffs.get_mut(y * w + x) {
                                *c = v;
                            }
                        }
                    }
                }
                left = right;
            }
            top = bottom;
        }
        Ok(())
    }

    fn arith_coeff(&self, s: &Subband, a: &mut Arith, x: usize, y: usize) -> i32 {
        let w = s.band.w;
        let get = |x: usize, y: usize| s.coeffs.get(y * w + x).copied().unwrap_or(0);
        let mut ctx = arith::ZPZN_F1;
        if let Some((p, pw)) = s.parent
            && p.get((y >> 1) * pw + (x >> 1)).copied().unwrap_or(0) != 0
        {
            ctx = arith::NPZN_F1;
        }
        let up = if y > 0 { get(x, y - 1) } else { 0 };
        let mut sign_pred = if s.orientation == 1 { up } else { 0 };
        if x > 0 {
            let (l, ul) = (get(x - 1, y), if y > 0 { get(x - 1, y - 1) } else { 0 });
            ctx += usize::from(l == 0 && up == 0 && ul == 0);
            if s.orientation == 2 {
                sign_pred = l;
            }
        } else {
            ctx += usize::from(up == 0);
        }
        let v = a.uint(ctx, arith::COEFF_DATA);
        if v == 0 {
            return 0;
        }
        let m = self.dequant(v, s.quant);
        let sctx = (arith::SIGN_ZERO as i32 + i32::from(sign_pred > 0) - i32::from(sign_pred < 0)) as usize;
        if a.bit(sctx) { m.wrapping_neg() } else { m }
    }

    fn vlc_subband(&self, s: &mut Subband, data: &[u8]) -> Result<()> {
        let mut b = Bits::new(data);
        b.bits_left = (data.len() as u64).saturating_mul(8);
        let (cw, ch) = s.codeblocks;
        let one = cw + ch == 2;
        let (w, h) = (s.band.w, s.band.h);
        let mut top = 0;
        for cy in 0..ch {
            let bottom = h * (cy + 1) / ch;
            let mut left = 0;
            for cx in 0..cw {
                let right = w * (cx + 1) / cw;
                if one || b.read_bitb()? == 0 {
                    if self.codeblock_mode && !(self.old_delta_quant && one) {
                        s.quant += b.read_sintb()?;
                    }
                    if !(0..=MAX_QUANT).contains(&s.quant) {
                        return Err(Error::Invalid("quantiser index"));
                    }
                    for y in top..bottom {
                        for x in left..right {
                            let v = b.read_sintb()?;
                            let m = self.dequant(v.unsigned_abs().min(u64::from(u32::MAX)) as u32, s.quant);
                            if let Some(c) = s.coeffs.get_mut(y * w + x) {
                                *c = if v < 0 { m.wrapping_neg() } else { m };
                            }
                        }
                    }
                }
                left = right;
            }
            top = bottom;
        }
        Ok(())
    }
}

/// Transform data of one component (§13.4.1): every subband's dequantised coefficients.
fn component(b: &mut Bits, t: &CoreTransform, bands: &[Band], coding: &CoefficientCoding, intra: bool) -> Result<Coeffs> {
    let mut coeffs: Coeffs = bands.iter().map(|b| vec![0i32; b.w * b.h]).collect();
    for (i, band) in bands.iter().enumerate() {
        b.byte_align();
        let length = b.read_uint()?;
        if length == 0 {
            continue;
        }
        let quant = i64::try_from(b.read_uint()?).unwrap_or(i64::MAX);
        if quant > MAX_QUANT {
            return Err(Error::Invalid("quantiser index"));
        }
        b.byte_align();
        let data = b.take(length);
        // level 0 holds the LL band and the first three; each further level three more
        let level = if i == 0 { 0 } else { (i - 1) / 3 };
        let orientation = if i == 0 { 0 } else { (i - 1) % 3 + 1 };
        let codeblocks = t.codeblocks.get(if i == 0 { 0 } else { level + 1 }).copied().unwrap_or((1, 1));
        let (done, rest) = coeffs.split_at_mut(i);
        let Some(target) = rest.first_mut() else { break };
        let parent = if level > 0 { done.get(i - 3).zip(bands.get(i - 3)).map(|(p, pb)| (p.as_slice(), pb.w)) } else { None };
        let mut s = Subband { band: *band, coeffs: target, parent, orientation, codeblocks, quant };
        if coding.arith {
            coding.arith_subband(&mut s, data)?;
        } else {
            coding.vlc_subband(&mut s, data)?;
        }
        if i == 0 && intra {
            picture::dc_prediction(target, band.w);
        }
    }
    Ok(coeffs)
}

/// The residual of each component (`None`: zero residual), as `pw`-wide arrays.
type Residual = [Option<(Vec<i32>, usize)>; 3];

fn residual(b: &mut Bits, seq: &Sequence, kind: Kind) -> Result<Residual> {
    let Some(t) = transform(b, seq, kind.num_refs)? else { return Ok([None, None, None]) };
    let tr = picture::core_transform(t.wavelet, t.depth)?;
    let quant = Quant::new();
    let narrow = seq.luma_depth <= 8 && seq.color_diff_depth <= 8;
    let coding = CoefficientCoding {
        quant: &quant,
        arith: kind.arith,
        inter: kind.num_refs > 0,
        codeblock_mode: t.codeblock_mode,
        old_delta_quant: kind.old_delta_quant,
        narrow,
    };
    let dims = [(seq.luma_width, seq.luma_height), (seq.color_diff_width, seq.color_diff_height), (seq.color_diff_width, seq.color_diff_height)];
    let mut out: Residual = [None, None, None];
    for (c, o) in out.iter_mut().enumerate() {
        let bands = picture::bands(&tr, dims[c].0, dims[c].1)?;
        let coeffs = component(b, &t, &bands, &coding, kind.num_refs == 0)?;
        *o = Some(picture::idwt(&tr, &bands, coeffs)?);
    }
    Ok(out)
}

/// The geometry of one component's blocks.
#[derive(Clone, Copy)]
struct PlaneGeom {
    w: usize,
    h: usize,
    xblen: usize,
    yblen: usize,
    xbsep: usize,
    ybsep: usize,
    xoff: usize,
    yoff: usize,
    /// Chroma subsampling of the motion vectors.
    shift: (u32, u32),
}

/// FFmpeg's OBMC weight ramp (`weight()`): position `i` of a block `blen` long whose blocks
/// overlap by `2 * offset`.
fn ramp(i: usize, blen: usize, offset: usize) -> u32 {
    let roll = |i: usize| -> u32 { if offset == 1 { if i > 0 { 5 } else { 3 } } else { (1 + (6 * i + offset - 1) / (2 * offset - 1)) as u32 } };
    if i < 2 * offset {
        roll(i)
    } else if i + 2 * offset > blen - 1 {
        roll(blen - 1 - i)
    } else {
        8
    }
}

/// The weights along one dimension of a block: `first` / `last` blocks keep full weight on their
/// outer half.
fn weights_1d(blen: usize, offset: usize, first: bool, last: bool) -> Vec<u32> {
    (0..blen).map(|i| if (first && i < blen >> 1) || (last && i >= blen >> 1) { 8 } else { ramp(i, blen, offset) }).collect()
}

/// A reference component extended by clamping its coordinates (the edges FFmpeg draws).
struct RefPlane<'a> {
    data: &'a [u8],
    w: usize,
    h: usize,
}

impl RefPlane<'_> {
    #[inline]
    fn at(&self, x: i64, y: i64) -> u8 {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        self.data.get(y * self.w + x).copied().unwrap_or(0)
    }
}

fn hpel_filter(v: [i32; 8]) -> u8 {
    let s = 21 * (v[3] + v[4]) - 7 * (v[2] + v[5]) + 3 * (v[1] + v[6]) - (v[0] + v[7]) + 16;
    (s >> 5).clamp(0, 255) as u8
}

/// The half-pel planes H, V and C of a `w` x `h` component (`dirac_hpel_filter`).
fn hpel_planes(f: &[u8], w: usize, h: usize) -> [Vec<u8>; 3] {
    let p = RefPlane { data: f, w, h };
    let (mut hp, mut vp, mut cp) = (vec![0u8; w * h], vec![0u8; w * h], vec![0u8; w * h]);
    // V over x in [-3, w + 5) so that C can filter across the edge
    let vw = w + 8;
    let mut vrow = vec![0u8; vw];
    for y in 0..h {
        let yi = y as i64;
        for (k, v) in vrow.iter_mut().enumerate() {
            let x = k as i64 - 3;
            let taps = std::array::from_fn(|t| i32::from(p.at(x, yi + t as i64 - 3)));
            *v = hpel_filter(taps);
        }
        for x in 0..w {
            if let Some(d) = vp.get_mut(y * w + x) {
                *d = vrow.get(x + 3).copied().unwrap_or(0);
            }
            let taps = std::array::from_fn(|t| i32::from(vrow.get(x + t).copied().unwrap_or(0)));
            if let Some(d) = cp.get_mut(y * w + x) {
                *d = hpel_filter(taps);
            }
            let xi = x as i64;
            let taps = std::array::from_fn(|t| i32::from(p.at(xi + t as i64 - 3, yi)));
            if let Some(d) = hp.get_mut(y * w + x) {
                *d = hpel_filter(taps);
            }
        }
    }
    [hp, vp, cp]
}

const EPEL: [[[u32; 4]; 4]; 4] = [
    [[16, 0, 0, 0], [12, 4, 0, 0], [8, 8, 0, 0], [4, 12, 0, 0]],
    [[12, 0, 4, 0], [9, 3, 3, 1], [6, 6, 2, 2], [3, 9, 1, 3]],
    [[8, 0, 8, 0], [6, 2, 6, 2], [4, 4, 4, 4], [2, 6, 2, 6]],
    [[4, 0, 12, 0], [3, 1, 9, 3], [2, 2, 6, 6], [1, 3, 3, 9]],
];

/// The prediction of one block from one reference (`mc_subpel` and the pixel functions):
/// `out` gets `xblen` x `yblen` samples.
fn predict(planes: [&RefPlane; 4], g: &PlaneGeom, precision: u32, mv: [i16; 2], x0: i64, y0: i64, out: &mut [u8]) {
    let (mut mx, mut my) = (i32::from(mv[0]), i32::from(mv[1]));
    mx >>= g.shift.0;
    my >>= g.shift.1;
    let mask = (1i32 << precision) - 1;
    let (fx, fy) = ((mx & mask) << (3 - precision), (my & mask) << (3 - precision));
    let (mut x, mut y) = (x0 + i64::from(mx >> precision), y0 + i64::from(my >> precision));
    // Each source: plane index and offset of its true position from the declared one.
    let mut src: [(usize, i64, i64); 4] = [(0, 0, 0), (1, 0, 0), (2, 0, 0), (3, 0, 0)];
    let mut n = 1;
    let mut weights = [0u32; 4];
    let epel = (fx | fy) & 1 == 1;
    if (fx | fy) & 3 == 0 {
        src[0] = (((fy >> 1) + (fx >> 2)) as usize, 0, 0);
    } else {
        n = 4;
        if fx > 4 {
            src[0].1 += 1;
            src[2].1 += 1;
            x += 1;
            for s in &mut src {
                s.1 -= 1;
            }
        }
        if fy > 4 {
            src[0].2 += 1;
            src[1].2 += 1;
            y += 1;
            for s in &mut src {
                s.2 -= 1;
            }
        }
        if !epel {
            if fx & 3 == 0 {
                if fx == 0 {
                    src[1] = src[2];
                } else {
                    src[0] = src[3];
                }
                n = 2;
            } else if fy & 3 == 0 {
                let k = (fy >> 1) as usize;
                (src[0], src[1]) = (src[k], src[k + 1]);
                n = 2;
            }
        } else {
            if fx > 4 {
                src.swap(0, 1);
                src.swap(2, 3);
            }
            if fy > 4 {
                src.swap(0, 2);
                src.swap(1, 3);
            }
            weights = EPEL[(fy & 3) as usize][(fx & 3) as usize];
        }
    }
    // Reads outside the edge go through FFmpeg's emulated edge: the declared position is clamped
    // to the edge and the source's offset kept.
    let (ew, eh) = (g.w as i64 + EDGE, g.h as i64 + EDGE);
    let sample = |s: (usize, i64, i64), i: usize, j: usize| -> u32 {
        let xi = (x + i as i64).clamp(0, ew - 1) + s.1;
        let yj = (y + j as i64).clamp(0, eh - 1) + s.2;
        planes.get(s.0).map_or(0, |p| u32::from(p.at(xi, yj)))
    };
    for j in 0..g.yblen {
        for i in 0..g.xblen {
            let v = if epel {
                (0..4).map(|k| sample(src[k], i, j) * weights[k]).sum::<u32>().wrapping_add(8) >> 4
            } else {
                match n {
                    1 => sample(src[0], i, j),
                    2 => (sample(src[0], i, j) + sample(src[1], i, j) + 1) >> 1,
                    _ => (sample(src[0], i, j) + sample(src[1], i, j) + sample(src[2], i, j) + sample(src[3], i, j) + 2) >> 2,
                }
            };
            if let Some(o) = out.get_mut(j * g.xblen + i) {
                *o = v as u8;
            }
        }
    }
}

/// Overlapped block motion compensation of component `c` plus its residual (8-bit video).
fn compensate(m: &Motion, refs: &[&Reference], c: usize, g: &PlaneGeom, residual: Option<&(Vec<i32>, usize)>) -> Vec<u16> {
    let p = &m.pred;
    let wx = [weights_1d(g.xblen, g.xoff, true, false), weights_1d(g.xblen, g.xoff, false, false), weights_1d(g.xblen, g.xoff, false, true)];
    let wy = [weights_1d(g.yblen, g.yoff, true, false), weights_1d(g.yblen, g.yoff, false, false), weights_1d(g.yblen, g.yoff, false, true)];
    let aw = m.blwidth * g.xbsep + g.xblen;
    let ah = m.blheight * g.ybsep + g.yblen;
    let mut acc = vec![0u16; aw * ah];
    // the reference planes: F, H, V, C (H, V and C only with sub-pel vectors)
    let planes: Vec<[RefPlane; 4]> = refs
        .iter()
        .map(|r| {
            let f = r.bytes.get(c).map_or(&[][..], Vec::as_slice);
            let hp = |k: usize| r.hpel.as_ref().and_then(|h| h.get(c)).and_then(|h| h.get(k)).map_or(f, Vec::as_slice);
            [
                RefPlane { data: f, w: g.w, h: g.h },
                RefPlane { data: hp(0), w: g.w, h: g.h },
                RefPlane { data: hp(1), w: g.w, h: g.h },
                RefPlane { data: hp(2), w: g.w, h: g.h },
            ]
        })
        .collect();
    let weighted = p.weight_log2 > 1 || p.weights != [1, 1];
    let round = 1i32 << (p.weight_log2 - 1);
    let n = g.xblen * g.yblen;
    let (mut p0, mut p1) = (vec![0u8; n], vec![0u8; n]);
    for by in 0..m.blheight {
        let wyv = &wy[if by == 0 {
            0
        } else if by == m.blheight - 1 {
            2
        } else {
            1
        }];
        for bx in 0..m.blwidth {
            let wxv = &wx[if bx == 0 {
                0
            } else if bx == m.blwidth - 1 {
                2
            } else {
                1
            }];
            let blk = m.at(bx, by);
            let (x0, y0) = ((bx * g.xbsep) as i64 - g.xoff as i64, (by * g.ybsep) as i64 - g.yoff as i64);
            let (ax, ay) = (bx * g.xbsep, by * g.ybsep);
            let dc = match blk.refs & 3 {
                0 => Some(i32::from(blk.dc(c)) + 128),
                r => {
                    let first = if r == 2 { 1 } else { 0 };
                    if let Some(pl) = planes.get(first) {
                        predict([&pl[0], &pl[1], &pl[2], &pl[3]], g, p.precision, blk.mv[first], x0, y0, &mut p0);
                    }
                    if r == 3 {
                        if let Some(pl) = planes.get(1) {
                            predict([&pl[0], &pl[1], &pl[2], &pl[3]], g, p.precision, blk.mv[1], x0, y0, &mut p1);
                        }
                        for (a, &b) in p0.iter_mut().zip(&p1) {
                            *a = if weighted {
                                ((i32::from(*a) * p.weights[0] + i32::from(b) * p.weights[1] + round) >> p.weight_log2).clamp(0, 255) as u8
                            } else {
                                ((u32::from(*a) + u32::from(b) + 1) >> 1) as u8
                            };
                        }
                    } else if weighted {
                        let w = p.weights[0] + p.weights[1];
                        for a in p0.iter_mut() {
                            *a = ((i32::from(*a) * w + round) >> p.weight_log2).clamp(0, 255) as u8;
                        }
                    }
                    None
                }
            };
            for j in 0..g.yblen {
                let row = (ay + j) * aw + ax;
                let Some(&wj) = wyv.get(j) else { continue };
                for i in 0..g.xblen {
                    let w = wj * wxv.get(i).copied().unwrap_or(0);
                    let v = match dc {
                        Some(d) => d.wrapping_mul(w as i32) as u16,
                        None => (u32::from(p0.get(j * g.xblen + i).copied().unwrap_or(0)) * w) as u16,
                    };
                    if let Some(a) = acc.get_mut(row + i) {
                        *a = a.wrapping_add(v);
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(g.w * g.h);
    for y in 0..g.h {
        for x in 0..g.w {
            let mc = i32::from(acc.get((y + g.yoff) * aw + x + g.xoff).copied().unwrap_or(0));
            let r = residual.map_or(0, |(v, pw)| v.get(y * pw + x).copied().unwrap_or(0) as i16 as i32);
            out.push((((mc + 32) >> 6) + r).clamp(0, 255) as u16);
        }
    }
    out
}

/// Decode a core syntax picture from the bytes after its parse info header. `refs` are the
/// reference pictures held; returns the header, the picture's components and the bytes used.
pub(crate) fn decode(seq: &Sequence, kind: Kind, data: &[u8], refs: &mut [Reference]) -> Result<(Header, [Vec<u16>; 3], usize)> {
    let mut b = Bits::new(data);
    let hdr = header(&mut b, kind)?;
    let dims = [(seq.luma_width, seq.luma_height), (seq.color_diff_width, seq.color_diff_height), (seq.color_diff_width, seq.color_diff_height)];
    if kind.num_refs == 0 {
        let res = residual(&mut b, seq, kind)?;
        let depths = [seq.luma_depth, seq.color_diff_depth, seq.color_diff_depth];
        let mut planes: [Vec<u16>; 3] = Default::default();
        for (c, plane) in planes.iter_mut().enumerate() {
            let (w, h) = (dims[c].0 as usize, dims[c].1 as usize);
            let (pic, pw) = res[c].as_ref().ok_or(Error::Invalid("intra picture without residual"))?;
            let half = 1i32 << (depths[c] - 1);
            let max = (1i32 << depths[c]) - 1;
            let narrow = depths[c] <= 8;
            plane.reserve(w * h);
            for y in 0..h {
                for x in 0..w {
                    let v = pic.get(y * pw + x).copied().unwrap_or(0);
                    let v = if narrow { i32::from(v as i16) } else { v };
                    plane.push((v.saturating_add(half)).clamp(0, max) as u16);
                }
            }
        }
        return Ok((hdr, planes, b.bytes_used()));
    }
    if seq.luma_depth != 8 || seq.color_diff_depth != 8 {
        return Err(Error::Unsupported("inter pictures deeper than 8 bits"));
    }
    if !kind.arith {
        return Err(Error::Unsupported("inter pictures without arithmetic coding"));
    }
    if refs.is_empty() {
        return Err(Error::Invalid("inter picture without reference pictures"));
    }
    // the closest reference picture to each wanted number (FFmpeg's choice when one is missing)
    let picks: Vec<usize> = hdr
        .refs
        .iter()
        .map(|&want| {
            let mut best = 0;
            let mut dist = u64::MAX;
            for (i, r) in refs.iter().enumerate() {
                let d = (i64::from(r.number) - i64::from(want)).unsigned_abs();
                if d < dist {
                    (best, dist) = (i, d);
                }
            }
            best
        })
        .collect();
    let m = motion(&mut b, seq, kind.num_refs)?;
    let res = residual(&mut b, seq, kind)?;
    if m.pred.precision > 0 {
        for &i in &picks {
            if let Some(r) = refs.get_mut(i)
                && r.hpel.is_none()
            {
                let h = std::array::from_fn(|c| hpel_planes(r.bytes.get(c).map_or(&[][..], Vec::as_slice), dims[c].0 as usize, dims[c].1 as usize));
                r.hpel = Some(Box::new(h));
            }
        }
    }
    let chosen: Vec<&Reference> = picks.iter().filter_map(|&i| refs.get(i)).collect();
    let (cx, cy) = chroma_shift(seq.video.chroma);
    let mut planes: [Vec<u16>; 3] = Default::default();
    for (c, plane) in planes.iter_mut().enumerate() {
        let (sx, sy) = if c == 0 { (0, 0) } else { (cx, cy) };
        let p = &m.pred;
        let (xblen, yblen, xbsep, ybsep) = (p.xblen >> sx, p.yblen >> sy, p.xbsep >> sx, p.ybsep >> sy);
        if xbsep == 0 || ybsep == 0 {
            return Err(Error::Invalid("chroma block separation"));
        }
        let g = PlaneGeom {
            w: dims[c].0 as usize,
            h: dims[c].1 as usize,
            xblen,
            yblen,
            xbsep,
            ybsep,
            xoff: (xblen - xbsep) / 2,
            yoff: (yblen - ybsep) / 2,
            shift: (sx, sy),
        };
        *plane = compensate(&m, &chosen, c, &g, res[c].as_ref());
    }
    Ok((hdr, planes, b.bytes_used()))
}
