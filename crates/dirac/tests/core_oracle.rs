//! Dirac core syntax streams written here with random content, decoded by us and by ffmpeg's
//! Dirac decoder (an external oracle): every sample must be equal. ffmpeg has no Dirac encoder,
//! so this writer covers the syntax itself: intra pictures with arithmetic or exp-Golomb coded
//! coefficients in codeblocks with quantiser deltas, and inter pictures from one or two
//! references with superblock splitting, DC blocks, motion vectors at every precision, global
//! motion and every block size preset.
//!
//! Avoided because ffmpeg departs from the specification there (each is checked on its own
//! below): non-default reference weights (ffmpeg scales most samples twice), zero residuals
//! (ffmpeg transforms the previous picture's coefficients again) and odd block overlaps. The
//! picture sizes are whole superblocks. The oracle test skips when ffmpeg is not installed.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use cfa4_dirac::Decoder;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u32) as i32
    }
    fn chance(&mut self, num: u32, den: u32) -> bool {
        self.below(den) < num
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bit: u32,
}

impl Writer {
    fn bit(&mut self, b: bool) {
        if self.bit == 0 {
            self.bytes.push(0);
        }
        if b {
            *self.bytes.last_mut().unwrap() |= 0x80 >> self.bit;
        }
        self.bit = (self.bit + 1) % 8;
    }
    fn bits(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.bit(v >> i & 1 == 1);
        }
    }
    fn uint(&mut self, v: u64) {
        let x = v + 1;
        let n = 63 - x.leading_zeros();
        for i in (0..n).rev() {
            self.bit(false);
            self.bit(x >> i & 1 == 1);
        }
        self.bit(true);
    }
    fn sint(&mut self, v: i64) {
        self.uint(v.unsigned_abs());
        if v != 0 {
            self.bit(v < 0);
        }
    }
    fn align(&mut self) {
        self.bit = 0;
    }
    /// A length-prefixed byte block.
    fn block(&mut self, data: &[u8]) {
        self.uint(data.len() as u64);
        self.align();
        self.bytes.extend_from_slice(data);
    }
}

// ---- the arithmetic encoder (Dirac specification Annex B) ----

const ZPZN_F1: usize = 0;
const NPZN_F1: usize = 2;
const ZP_F2: usize = 4;
const ZP_F5: usize = 7;
const COEFF_DATA: usize = 14;
const SIGN_ZERO: usize = 16;
const ZERO_BLOCK: usize = 18;
const DELTA_Q_F: usize = 19;
const DELTA_Q_DATA: usize = 20;
const NEXT: [usize; 22] = [4, 4, 9, 9, 5, 6, 7, 8, 8, 10, 11, 12, 13, 13, 0, 0, 0, 0, 0, 19, 0, 0];
const PROB: [u16; 256] = [
    0, 2, 5, 8, 11, 15, 20, 24, 29, 35, 41, 47, 53, 60, 67, 74, 82, 89, 97, 106, 114, 123, 132, 141, 150, 160, 170, 180, 190, 201, 211, 222, 233, 244, 256,
    267, 279, 291, 303, 315, 327, 340, 353, 366, 379, 392, 405, 419, 433, 447, 461, 475, 489, 504, 518, 533, 548, 563, 578, 593, 609, 624, 640, 656, 672, 688,
    705, 721, 738, 754, 771, 788, 805, 822, 840, 857, 875, 892, 910, 928, 946, 964, 983, 1001, 1020, 1038, 1057, 1076, 1095, 1114, 1133, 1153, 1172, 1192,
    1211, 1231, 1251, 1271, 1291, 1311, 1332, 1352, 1373, 1393, 1414, 1435, 1456, 1477, 1498, 1520, 1541, 1562, 1584, 1606, 1628, 1649, 1671, 1694, 1716, 1738,
    1760, 1783, 1806, 1828, 1851, 1874, 1897, 1920, 1935, 1942, 1949, 1955, 1961, 1968, 1974, 1980, 1985, 1991, 1996, 2001, 2006, 2011, 2016, 2021, 2025, 2029,
    2033, 2037, 2040, 2044, 2047, 2050, 2053, 2056, 2058, 2061, 2063, 2065, 2066, 2068, 2069, 2070, 2071, 2072, 2072, 2072, 2072, 2072, 2072, 2071, 2070, 2069,
    2068, 2066, 2065, 2063, 2060, 2058, 2055, 2052, 2049, 2045, 2042, 2038, 2033, 2029, 2024, 2019, 2013, 2008, 2002, 1996, 1989, 1982, 1975, 1968, 1960, 1952,
    1943, 1934, 1925, 1916, 1906, 1896, 1885, 1874, 1863, 1851, 1839, 1827, 1814, 1800, 1786, 1772, 1757, 1742, 1727, 1710, 1694, 1676, 1659, 1640, 1622, 1602,
    1582, 1561, 1540, 1518, 1495, 1471, 1447, 1422, 1396, 1369, 1341, 1312, 1282, 1251, 1219, 1186, 1151, 1114, 1077, 1037, 995, 952, 906, 857, 805, 750, 690,
    625, 553, 471, 376, 255,
];

/// Exact interval arithmetic: the code value's bits with carries.
struct Arith {
    bits: Vec<u8>,
    scale: usize,
    range: u32,
    ctx: [u16; 22],
}

impl Arith {
    fn new() -> Self {
        Self { bits: vec![0; 16], scale: 0, range: 0xFFFF, ctx: [0x8000; 22] }
    }
    fn add(&mut self, v: u32) {
        let end = self.scale + 16;
        if self.bits.len() < end {
            self.bits.resize(end, 0);
        }
        let mut carry = 0u32;
        for k in (0..end).rev() {
            let add = if k >= self.scale { (v >> (15 - (k - self.scale))) & 1 } else { 0 };
            let s = u32::from(self.bits[k]) + add + carry;
            self.bits[k] = (s & 1) as u8;
            carry = s >> 1;
            if k < self.scale && carry == 0 {
                break;
            }
        }
    }
    fn bit(&mut self, ctx: usize, bit: bool) {
        let p = self.ctx[ctx];
        let rtp = (self.range * u32::from(p)) >> 16;
        if bit {
            self.add(rtp);
            self.range -= rtp;
        } else {
            self.range = rtp;
        }
        let i = usize::from(p >> 8);
        self.ctx[ctx] = if bit { p - PROB[i] } else { p + PROB[255 - i] };
        while self.range <= 0x4000 {
            self.range <<= 1;
            self.scale += 1;
        }
    }
    fn uint(&mut self, follow: usize, data: usize, v: u32) {
        let v = v + 1;
        let top = 31 - v.leading_zeros();
        let mut f = follow;
        for k in (0..top).rev() {
            self.bit(f, false);
            self.bit(data, (v >> k) & 1 == 1);
            f = NEXT[f];
        }
        self.bit(f, true);
    }
    fn int(&mut self, follow: usize, data: usize, v: i32) {
        self.uint(follow, data, v.unsigned_abs());
        if v != 0 {
            self.bit(data + 1, v < 0);
        }
    }
    fn finish(mut self) -> Vec<u8> {
        let r = self.range - 1;
        self.add(r);
        self.bits
            .chunks(8)
            .map(|c| {
                let mut b = 0u8;
                for (k, &bit) in c.iter().enumerate() {
                    b |= bit << (7 - k);
                }
                if c.len() < 8 {
                    b |= 0xFF >> c.len();
                }
                b
            })
            .collect()
    }
}

// ---- stream structure ----

#[derive(Clone)]
struct Seq {
    w: u32,
    h: u32,
    /// 0 4:4:4, 1 4:2:2, 2 4:2:0
    cdf: u64,
    /// Signal range preset (2: 8-bit video, 3: 10-bit video).
    range: u64,
}

impl Seq {
    fn chroma(&self) -> (u32, u32) {
        match self.cdf {
            0 => (self.w, self.h),
            1 => (self.w / 2, self.h),
            _ => (self.w / 2, self.h / 2),
        }
    }
}

fn parse_info(code: u8, body: &[u8], next: bool, previous: usize) -> Vec<u8> {
    let mut v = b"BBCD".to_vec();
    v.push(code);
    let n = if next { (13 + body.len()) as u32 } else { 0 };
    v.extend_from_slice(&n.to_be_bytes());
    v.extend_from_slice(&(previous as u32).to_be_bytes());
    v.extend_from_slice(body);
    v
}

fn sequence_header(s: &Seq) -> Vec<u8> {
    let mut w = Writer::default();
    w.uint(2); // major version
    w.uint(2); // minor
    w.uint(8); // main profile
    w.uint(0);
    w.uint(0); // base video format: custom
    w.bit(true);
    w.uint(u64::from(s.w));
    w.uint(u64::from(s.h));
    w.bit(true);
    w.uint(s.cdf);
    w.bit(false); // scan format
    w.bit(false); // frame rate
    w.bit(false); // pixel aspect ratio
    w.bit(true); // clean area: the whole frame
    w.uint(u64::from(s.w));
    w.uint(u64::from(s.h));
    w.uint(0);
    w.uint(0);
    w.bit(true);
    w.uint(s.range);
    w.bit(false); // colour spec
    w.uint(0); // frame coding
    w.align();
    w.bytes
}

/// What a picture's random content may contain.
#[derive(Clone)]
struct Content {
    wavelet: u64,
    depth: u32,
    /// Codeblocks per level (index 0: the DC band); `None`: one each.
    codeblocks: Option<Vec<(u64, u64)>>,
    codeblock_mode: bool,
    max_coeff: u32,
    max_quant: i64,
    zero_band: u32,
}

#[derive(Clone)]
struct MotionParams {
    /// Block parameters index 1..=4, or 0 with (xblen, yblen, xbsep, ybsep).
    preset: u64,
    custom: (u64, u64, u64, u64),
    precision: u64,
    global: bool,
    max_mv: i32,
    dc_blocks: u32,
    /// Reference weights (precision, weight 1, weight 2); `None`: the defaults.
    weights: Option<(u64, i64, i64)>,
    /// Every block in this mode (1, 2: one reference, 3: both); `None`: random modes.
    mode: Option<u8>,
}

struct Pic {
    number: u32,
    refs: Vec<i64>,
    retire: Option<i64>,
    arith: bool,
    motion: Option<MotionParams>,
    /// An inter picture without residual.
    zero_res: bool,
}

impl Pic {
    fn code(&self) -> u8 {
        let mut c = if self.arith { 0x08 } else { 0x48 };
        if self.retire.is_some() {
            c |= 0x04;
        }
        c | self.refs.len() as u8
    }
}

fn band_sizes(depth: u32, w: u32, h: u32) -> Vec<(usize, usize)> {
    let pw = (w as usize).div_ceil(1 << depth) << depth;
    let ph = (h as usize).div_ceil(1 << depth) << depth;
    let mut out = vec![(pw >> depth, ph >> depth)];
    for level in 1..=depth {
        out.extend([(pw >> (depth - level + 1), ph >> (depth - level + 1)); 3]);
    }
    out
}

/// One subband's coded values in decode order, written with the decoder's contexts.
fn subband_arith(
    rng: &mut Rng,
    c: &Content,
    size: (usize, usize),
    parent: Option<(&[i32], usize)>,
    orientation: usize,
    cb: (u64, u64),
    quant: &mut i64,
) -> (Vec<u8>, Vec<i32>) {
    let (w, h) = size;
    let mut vals = vec![0i32; w * h];
    let mut a = Arith::new();
    let (cw, ch) = (cb.0 as usize, cb.1 as usize);
    let one = cw + ch == 2;
    let mut top = 0;
    for cy in 0..ch {
        let bottom = h * (cy + 1) / ch;
        let mut left = 0;
        for cx in 0..cw {
            let right = w * (cx + 1) / cw;
            let skip = !one && rng.chance(1, 4);
            if !one {
                a.bit(ZERO_BLOCK, skip);
            }
            if !skip {
                if c.codeblock_mode {
                    let d = rng.range(-2, 2).clamp(-(*quant as i32), (c.max_quant - *quant) as i32);
                    *quant += i64::from(d);
                    a.int(DELTA_Q_F, DELTA_Q_DATA, d);
                }
                for y in top..bottom {
                    for x in left..right {
                        let get = |x: usize, y: usize| vals[y * w + x];
                        let mut ctx = ZPZN_F1;
                        if let Some((p, pw)) = parent
                            && p[(y >> 1) * pw + (x >> 1)] != 0
                        {
                            ctx = NPZN_F1;
                        }
                        let up = if y > 0 { get(x, y - 1) } else { 0 };
                        let mut sp = if orientation == 1 { up } else { 0 };
                        if x > 0 {
                            let ul = if y > 0 { get(x - 1, y - 1) } else { 0 };
                            ctx += usize::from(get(x - 1, y) == 0 && up == 0 && ul == 0);
                            if orientation == 2 {
                                sp = get(x - 1, y);
                            }
                        } else {
                            ctx += usize::from(up == 0);
                        }
                        let v = if rng.chance(2, 5) { 0 } else { rng.range(-(c.max_coeff as i32), c.max_coeff as i32) };
                        a.uint(ctx, COEFF_DATA, v.unsigned_abs());
                        if v != 0 {
                            a.bit((SIGN_ZERO as i32 + i32::from(sp > 0) - i32::from(sp < 0)) as usize, v < 0);
                        }
                        vals[y * w + x] = v;
                    }
                }
            }
            left = right;
        }
        top = bottom;
    }
    (a.finish(), vals)
}

fn subband_vlc(rng: &mut Rng, c: &Content, size: (usize, usize), cb: (u64, u64), quant: &mut i64) -> Vec<u8> {
    let (w, h) = size;
    let mut b = Writer::default();
    let (cw, ch) = (cb.0 as usize, cb.1 as usize);
    let one = cw + ch == 2;
    let mut top = 0;
    for cy in 0..ch {
        let bottom = h * (cy + 1) / ch;
        let mut left = 0;
        for cx in 0..cw {
            let right = w * (cx + 1) / cw;
            let skip = !one && rng.chance(1, 4);
            if !one {
                b.bit(skip);
            }
            if !skip {
                if c.codeblock_mode {
                    let d = rng.range(-2, 2).clamp(-(*quant as i32), (c.max_quant - *quant) as i32);
                    *quant += i64::from(d);
                    b.sint(i64::from(d));
                }
                for _ in 0..(bottom - top) * (right - left) {
                    let v = if rng.chance(2, 5) { 0 } else { rng.range(-(c.max_coeff as i32), c.max_coeff as i32) };
                    b.sint(i64::from(v));
                }
            }
            left = right;
        }
        top = bottom;
    }
    b.bytes
}

fn transform_data(w: &mut Writer, rng: &mut Rng, s: &Seq, c: &Content, arith: bool) {
    w.uint(c.wavelet);
    w.uint(u64::from(c.depth));
    match &c.codeblocks {
        Some(cbs) => {
            w.bit(true);
            for &(x, y) in cbs {
                w.uint(x);
                w.uint(y);
            }
            w.uint(u64::from(c.codeblock_mode));
        }
        None => w.bit(false),
    }
    let (cw, ch) = s.chroma();
    for (pw, ph) in [(s.w, s.h), (cw, ch), (cw, ch)] {
        let sizes = band_sizes(c.depth, pw, ph);
        let mut coded: Vec<Vec<i32>> = Vec::new();
        for (i, &size) in sizes.iter().enumerate() {
            w.align();
            let level = if i == 0 { 0 } else { (i - 1) / 3 };
            let orientation = if i == 0 { 0 } else { (i - 1) % 3 + 1 };
            if rng.below(100) < c.zero_band {
                w.uint(0);
                coded.push(vec![0; size.0 * size.1]);
                continue;
            }
            let cb = c.codeblocks.as_ref().map_or((1, 1), |v| v[if i == 0 { 0 } else { level + 1 }]);
            let mut quant = i64::from(rng.below(c.max_quant as u32 + 1));
            let q0 = quant;
            let (data, vals) = if arith {
                let parent = if level > 0 { Some((coded[i - 3].as_slice(), sizes[i - 3].0)) } else { None };
                subband_arith(rng, c, size, parent, orientation, cb, &mut quant)
            } else {
                (subband_vlc(rng, c, size, cb, &mut quant), vec![1; size.0 * size.1])
            };
            coded.push(vals);
            w.uint(data.len() as u64);
            w.uint(q0 as u64);
            w.align();
            w.bytes.extend_from_slice(&data);
        }
    }
}

/// The encoder's view of the block motion field (only what decides what is coded next).
fn motion_data(w: &mut Writer, rng: &mut Rng, s: &Seq, m: &MotionParams, num_refs: usize) {
    w.align();
    w.uint(m.preset);
    let (xblen, yblen, xbsep, ybsep) = match m.preset {
        0 => {
            let (a, b, c, d) = m.custom;
            for v in [a, b, c, d] {
                w.uint(v);
            }
            m.custom
        }
        i => {
            let l = [4, 12, 16, 24][i as usize - 1];
            (l, l, 4 * i, 4 * i)
        }
    };
    let _ = (xblen, yblen);
    w.uint(m.precision);
    w.bit(m.global);
    if m.global {
        for _ in 0..num_refs {
            w.bit(true);
            w.sint(i64::from(rng.range(-6, 6)));
            w.sint(i64::from(rng.range(-6, 6)));
            w.bit(true);
            w.uint(6);
            for _ in 0..4 {
                w.sint(i64::from(rng.range(-3, 3)));
            }
            let perspective = rng.chance(1, 2);
            w.bit(perspective);
            if perspective {
                w.uint(8);
                w.sint(i64::from(rng.range(-1, 1)));
                w.sint(i64::from(rng.range(-1, 1)));
            }
        }
    }
    w.uint(0); // prediction mode
    w.bit(m.weights.is_some());
    if let Some((precision, w1, w2)) = m.weights {
        w.uint(precision);
        w.sint(w1);
        if num_refs == 2 {
            w.sint(w2);
        }
    }
    w.align();
    let sbw = (s.w as usize).div_ceil(4 * xbsep as usize);
    let sbh = (s.h as usize).div_ceil(4 * ybsep as usize);
    let blw = 4 * sbw;
    // superblock splits
    let mut split = vec![0u8; sbw * sbh];
    let mut a = Arith::new();
    for y in 0..sbh {
        for x in 0..sbw {
            let at = |x: usize, y: usize| split[y * sbw + x];
            let p = if x == 0 && y == 0 {
                0
            } else if y == 0 {
                at(x - 1, y)
            } else if x == 0 {
                at(x, y - 1)
            } else {
                [0, 0, 1, 1, 1, 2, 2][usize::from(at(x - 1, y) + at(x, y - 1) + at(x - 1, y - 1))]
            };
            let v = rng.below(3) as u8;
            a.uint(ZP_F5, 0, u32::from(v));
            split[y * sbw + x] = (v + p) % 3;
        }
    }
    w.block(&a.finish());
    // block modes and values
    let mut refs_of = vec![0u8; blw * 4 * sbh];
    let mut modes = Arith::new();
    let mut mvs: Vec<[Arith; 2]> = (0..num_refs).map(|_| [Arith::new(), Arith::new()]).collect();
    let mut dcs = [Arith::new(), Arith::new(), Arith::new()];
    let pred = |refs: &[u8], x: usize, y: usize, mask: u8| -> u8 {
        let at = |x: usize, y: usize| refs[y * blw + x] & mask;
        if x == 0 && y == 0 {
            0
        } else if y == 0 {
            at(x - 1, y)
        } else if x == 0 {
            at(x, y - 1)
        } else {
            (((u32::from(at(x - 1, y)) + u32::from(at(x, y - 1)) + u32::from(at(x - 1, y - 1))) >> 1) as u8) & mask
        }
    };
    for sy in 0..sbh {
        for sx in 0..sbw {
            let sp = split[sy * sbw + sx];
            let (count, step) = (1usize << sp, 4usize >> sp);
            for q in 0..count {
                for p in 0..count {
                    let (bx, by) = (4 * sx + p * step, 4 * sy + q * step);
                    // the wanted mode: DC now and then, else one or both references
                    let want: u8 = if let Some(mode) = m.mode {
                        mode
                    } else if rng.below(100) < m.dc_blocks {
                        0
                    } else if num_refs == 2 {
                        1 + rng.below(3) as u8
                    } else {
                        1
                    };
                    let mut r = pred(&refs_of, bx, by, 1);
                    modes.bit(0, (want & 1) != r);
                    r = want & 1;
                    if num_refs == 2 {
                        let p2 = pred(&refs_of, bx, by, 2);
                        modes.bit(1, (want & 2) != p2);
                        r |= want & 2;
                    }
                    if r == 0 {
                        for d in &mut dcs {
                            d.int(ZP_F5, 0, rng.range(-12, 12));
                        }
                    } else {
                        if m.global {
                            let g = rng.chance(1, 4) as u8 * 4;
                            let pg = pred(&refs_of, bx, by, 4);
                            modes.bit(2, g != pg);
                            r |= g;
                        }
                        for (i, pair) in mvs.iter_mut().enumerate() {
                            if r & (i as u8 + 1) != 0 && r & 4 == 0 {
                                for a in pair.iter_mut() {
                                    a.int(ZP_F2, 0, rng.range(-m.max_mv, m.max_mv));
                                }
                            }
                        }
                    }
                    for y in by..by + step {
                        for x in bx..bx + step {
                            refs_of[y * blw + x] = r;
                        }
                    }
                }
            }
        }
    }
    w.block(&modes.finish());
    for pair in mvs {
        let [x, y] = pair;
        w.block(&x.finish());
        w.block(&y.finish());
    }
    for d in dcs {
        w.block(&d.finish());
    }
}

fn picture(rng: &mut Rng, s: &Seq, p: &Pic, c: &Content) -> Vec<u8> {
    let mut w = Writer::default();
    w.bits(u64::from(p.number), 32);
    for &r in &p.refs {
        w.sint(r);
    }
    if let Some(r) = p.retire {
        w.sint(r);
    }
    if let Some(m) = &p.motion {
        motion_data(&mut w, rng, s, m, p.refs.len());
    }
    w.align();
    if !p.refs.is_empty() {
        w.bit(p.zero_res);
    }
    if !p.zero_res {
        transform_data(&mut w, rng, s, c, p.arith);
    }
    w.align();
    w.bytes
}

fn stream(rng: &mut Rng, s: &Seq, pics: &[Pic], c: &Content) -> Vec<u8> {
    let mut units = vec![(0x00, sequence_header(s))];
    for p in pics {
        units.push((p.code(), picture(rng, s, p, c)));
    }
    units.push((0x10, Vec::new()));
    let mut out = Vec::new();
    let mut prev = 0;
    let n = units.len();
    for (i, (code, body)) in units.into_iter().enumerate() {
        let unit = parse_info(code, &body, i + 1 < n, prev);
        prev = unit.len();
        out.extend(unit);
    }
    out
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn ffmpeg_decode(name: &str, data: &[u8]) -> Option<Vec<u8>> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/dirac");
    std::fs::create_dir_all(&d).ok()?;
    let src = d.join(format!("core_{name}.drc"));
    std::fs::write(&src, data).ok()?;
    let out = Command::new("ffmpeg").args(["-v", "error", "-f", "dirac", "-i"]).arg(&src).args(["-f", "rawvideo", "-"]).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Our decode in display order, as bytes.
fn ours(name: &str, data: &[u8]) -> Vec<u8> {
    let mut pics = Decoder::new().decode(data).unwrap_or_else(|e| panic!("{name}: {e}"));
    pics.sort_by_key(|p| p.picture_number);
    pics.iter().flat_map(|p| p.planes.iter().flatten().map(|&v| v as u8)).collect()
}

/// I0 P2 B1 P4 B3 (references 0, 2, 4; picture 4 retires 0), then an intra picture 5.
fn gop(arith: bool, motion: &MotionParams, two_refs: bool) -> Vec<Pic> {
    let i = |n: u32| Pic { number: n, refs: vec![], retire: Some(0), arith, motion: None, zero_res: false };
    let p =
        |n: u32, back: i64, retire: i64| Pic { number: n, refs: vec![-back], retire: Some(retire), arith: true, motion: Some(motion.clone()), zero_res: false };
    let b = |n: u32| {
        let refs = if two_refs { vec![-1, 1] } else { vec![-1] };
        Pic { number: n, refs, retire: None, arith: true, motion: Some(motion.clone()), zero_res: false }
    };
    vec![i(0), p(2, 2, 0), b(1), p(4, 2, -4), b(3), i(5)]
}

#[test]
fn synthetic_core_streams_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("SKIPPED (Dirac core synthetic oracle): ffmpeg is not installed");
        return;
    }
    let content = Content { wavelet: 1, depth: 3, codeblocks: None, codeblock_mode: false, max_coeff: 6, max_quant: 16, zero_band: 10 };
    let motion = MotionParams { preset: 2, custom: (0, 0, 0, 0), precision: 2, global: false, max_mv: 6, dc_blocks: 10, weights: None, mode: None };
    let mut cases: Vec<(String, Seq, Vec<Pic>, Content)> = Vec::new();
    let s420 = Seq { w: 96, h: 64, cdf: 2, range: 2 };
    // intra only: every wavelet, arithmetic and exp-Golomb coding, codeblocks and quantiser deltas
    for wavelet in 0..=5 {
        for arith in [true, false] {
            let mut c = content.clone();
            c.wavelet = wavelet;
            c.depth = 1 + (wavelet as u32 % 4);
            if wavelet % 2 == 1 {
                c.codeblocks = Some((0..=c.depth).map(|l| (1 + u64::from(l), 1 + u64::from(l) / 2)).collect());
                c.codeblock_mode = true;
            }
            let pics = vec![
                Pic { number: 0, refs: vec![], retire: Some(0), arith, motion: None, zero_res: false },
                Pic { number: 1, refs: vec![], retire: None, arith, motion: None, zero_res: false },
            ];
            // FFmpeg's edge extension differs on subbands only 1–2 samples high: keep them taller
            cases.push((format!("intra_w{wavelet}_{}", if arith { "ac" } else { "vlc" }), Seq { w: 128, h: 128, cdf: 2, range: 2 }, pics, c));
        }
    }
    // inter: every block preset and precision, global motion, 4:2:2 and 4:4:4. FFmpeg keeps its
    // four edge-emulation buffers `align(width, 16)` apart in rows of the frame stride, so for some
    // widths (64, 128) they overlap and its sub-pel prediction at the picture edge reads another
    // plane; the widths here keep them apart.
    for preset in 1..=4u64 {
        for precision in 0..=3u64 {
            let mut m = motion.clone();
            (m.preset, m.precision, m.max_mv) = (preset, precision, 3 << precision);
            // FFmpeg needs the picture size a multiple of 4 block separations
            let s = if preset == 4 { Seq { w: 192, h: 128, cdf: 2, range: 2 } } else { Seq { w: 96, h: 96, cdf: 2, range: 2 } };
            let mut c = content.clone();
            c.codeblocks = Some(vec![(1, 1), (2, 2), (3, 2), (4, 4)]);
            c.codeblock_mode = precision % 2 == 1;
            cases.push((format!("inter_p{preset}_mv{precision}"), s, gop(true, &m, true), c));
        }
    }
    let mut m = motion.clone();
    m.global = true;
    cases.push(("inter_global".into(), Seq { w: 96, h: 64, cdf: 2, range: 2 }, gop(true, &m, true), content.clone()));
    let mut m = motion.clone();
    (m.preset, m.custom) = (0, (8, 8, 4, 4));
    cases.push(("inter_custom_blocks".into(), Seq { w: 80, h: 48, cdf: 0, range: 2 }, gop(true, &m, false), content.clone()));
    let mut m = motion.clone();
    (m.precision, m.dc_blocks) = (3, 40);
    cases.push(("inter_422_dc".into(), Seq { w: 96, h: 64, cdf: 1, range: 2 }, gop(true, &m, true), content.clone()));
    cases.push(("inter_vlc_intra".into(), s420.clone(), gop(false, &motion, true), content.clone()));

    let mut rng = Rng(0xD1AC_0001);
    let mut failed = Vec::new();
    for (name, s, pics, c) in &cases {
        let data = stream(&mut rng, s, pics, c);
        let Some(want) = ffmpeg_decode(name, &data) else {
            failed.push(format!("{name}: ffmpeg could not decode it"));
            continue;
        };
        let got = ours(name, &data);
        if got.len() != want.len() {
            failed.push(format!("{name}: {} bytes, ffmpeg {}", got.len(), want.len()));
        } else if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            failed.push(format!("{name}: byte {i} differs (ours {}, ffmpeg {})", got[i], want[i]));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// Every block from the same place in the references without residual: the output is the
/// weighted reference sample (OBMC weights sum to 64), which checks the reference weights and
/// zero residuals that ffmpeg handles differently from the specification.
#[test]
fn reference_weights_and_zero_residual() {
    let s = Seq { w: 96, h: 64, cdf: 2, range: 2 };
    let c = Content { wavelet: 1, depth: 3, codeblocks: None, codeblock_mode: false, max_coeff: 6, max_quant: 16, zero_band: 10 };
    let still =
        |weights, mode| MotionParams { preset: 2, custom: (0, 0, 0, 0), precision: 2, global: false, max_mv: 0, dc_blocks: 0, weights, mode: Some(mode) };
    let intra = |n: u32| Pic { number: n, refs: vec![], retire: Some(0), arith: true, motion: None, zero_res: false };
    let inter = |n: u32, refs: Vec<i64>, m: MotionParams| Pic { number: n, refs, retire: None, arith: true, motion: Some(m), zero_res: true };
    let pics = vec![
        intra(0),
        intra(2),
        inter(1, vec![-1, 1], still(Some((3, 3, 5)), 3)),
        inter(3, vec![-1], still(Some((2, 2, 0)), 1)),
        inter(4, vec![-2], still(None, 1)),
        inter(5, vec![-3, -5], still(None, 3)),
    ];
    let data = stream(&mut Rng(0xD1AC_0002), &s, &pics, &c);
    let mut got = Decoder::new().decode(&data).expect("decode");
    got.sort_by_key(|p| p.picture_number);
    let numbers: Vec<u32> = got.iter().map(|p| p.picture_number).collect();
    assert_eq!(numbers, [0, 1, 2, 3, 4, 5]);
    let (a, b) = (&got[0].planes, &got[2].planes);
    let check = |n: usize, f: &dyn Fn(i32, i32) -> i32| {
        for k in 0..3 {
            for (i, &v) in got[n].planes[k].iter().enumerate() {
                let want = f(i32::from(a[k][i]), i32::from(b[k][i])).clamp(0, 255);
                assert_eq!(i32::from(v), want, "picture {n} component {k} sample {i}");
            }
        }
    };
    // two references: (3a + 5b + 4) >> 3
    check(1, &|a, b| (3 * a + 5 * b + 4) >> 3);
    // one reference: the weights' sum, (2 + 1) b with precision 2
    check(3, &|_, b| (3 * b + 2) >> 2);
    // default weights: the reference itself, or the rounded mean of both
    check(4, &|_, b| b);
    check(5, &|a, b| (a + b + 1) >> 1);
}

#[test]
fn deep_inter_pictures_are_unsupported() {
    let s = Seq { w: 96, h: 64, cdf: 2, range: 3 };
    let c = Content { wavelet: 1, depth: 3, codeblocks: None, codeblock_mode: false, max_coeff: 6, max_quant: 16, zero_band: 10 };
    let m = MotionParams { preset: 2, custom: (0, 0, 0, 0), precision: 2, global: false, max_mv: 6, dc_blocks: 10, weights: None, mode: None };
    let intra = stream(&mut Rng(3), &s, &[Pic { number: 0, refs: vec![], retire: Some(0), arith: true, motion: None, zero_res: false }], &c);
    let pics = Decoder::new().decode(&intra).expect("10-bit intra");
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].luma_depth, 10);
    let data = stream(&mut Rng(3), &s, &gop(true, &m, true), &c);
    assert!(matches!(Decoder::new().decode(&data), Err(cfa4_dirac::Error::Unsupported(_))));
}

/// Damaged core syntax streams (flipped bytes, cuts, noise) return errors or garbage pictures,
/// never a panic.
#[test]
fn damaged_core_streams_never_panic() {
    let c = Content {
        wavelet: 1,
        depth: 3,
        codeblocks: Some(vec![(1, 1), (2, 2), (3, 2), (4, 4)]),
        codeblock_mode: true,
        max_coeff: 6,
        max_quant: 16,
        zero_band: 10,
    };
    let mut m = MotionParams { preset: 2, custom: (0, 0, 0, 0), precision: 3, global: true, max_mv: 24, dc_blocks: 10, weights: Some((2, 3, -1)), mode: None };
    let mut rng = Rng(0xBAD_D1AC);
    let mut sources = vec![stream(&mut rng, &Seq { w: 64, h: 48, cdf: 1, range: 2 }, &gop(true, &m, true), &c)];
    m.global = false;
    sources.push(stream(&mut rng, &Seq { w: 48, h: 32, cdf: 2, range: 2 }, &gop(false, &m, false), &c));
    for src in &sources {
        for round in 0..300 {
            let mut d = src.clone();
            match round % 3 {
                0 => {
                    for _ in 0..1 + rng.below(4) {
                        let at = rng.below(d.len() as u32) as usize;
                        d[at] ^= 1 << rng.below(8);
                    }
                }
                1 => d.truncate(rng.below(d.len() as u32) as usize),
                _ => {
                    // noise over the picture data, headers kept
                    let start = 13 + rng.below(d.len() as u32 - 13) as usize;
                    for b in d.iter_mut().skip(start).take(1 + rng.below(64) as usize) {
                        *b = rng.next() as u8;
                    }
                }
            }
            let _ = Decoder::new().decode(&d);
        }
    }
}
