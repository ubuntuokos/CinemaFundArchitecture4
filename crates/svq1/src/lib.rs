//! Sorenson Video 1 (SVQ1, FourCC `SVQ1`) decoder, a Rust re-expression of FFmpeg n7.1.1
//! `libavcodec/svq1dec.c`. Pure safe Rust, no dependencies but `thiserror`.
//!
//! Frames are vector-quantised in a quad-tree of 16x16 blocks with multi-stage codebooks; delta
//! frames add half-pel motion compensation with one or four vectors per block. The pictures are
//! planar YUV 4:1:0.

mod data;

use thiserror::Error;

/// SVQ1 decoding errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("SVQ1: invalid frame: {0}")]
    Invalid(&'static str),
}

fn inv<T>(why: &'static str) -> Result<T, Error> {
    Err(Error::Invalid(why))
}

/// A decoded picture: planar YUV 4:1:0, tightly packed (`Y`, `U`, `V`; chroma at a quarter of the
/// width and height, rounded up).
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Vec<u8>>,
    /// Whether this was a key (intra) frame.
    pub key: bool,
}

impl Picture {
    /// Width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if i == 0 { (w, h) } else { (w.div_ceil(4), h.div_ceil(4)) }
    }
}

// ---------------------------------------------------------------------------------------------
// Bits and VLCs (MSB first)

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0 }
    }
    /// The next `n` (at most 24) bits without consuming them; zeros past the end.
    fn peek(&self, n: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut v = 0u64;
        for k in 0..5 {
            v = v << 8 | u64::from(self.data.get(byte + k).copied().unwrap_or(0));
        }
        ((v >> (40 - (self.pos & 7) as u32 - n)) & ((1u64 << n) - 1)) as u32
    }
    fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }
    fn get1(&mut self) -> bool {
        self.get(1) != 0
    }
    fn skip(&mut self, n: usize) {
        self.pos += n;
    }
    fn left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }
}

/// A prefix code read MSB first: a primary lookup table and a list for the long codes.
struct Vlc {
    max: u8,
    bits: u8,
    /// `(symbol, length)` per `bits`-bit prefix; length 0 for prefixes of longer codes.
    table: Vec<(u16, u8)>,
    long: Vec<(u32, u8, u16)>,
}

impl Vlc {
    fn new(codes: &[(u32, u8)]) -> Vlc {
        let max = codes.iter().map(|c| c.1).max().unwrap_or(1);
        let bits = max.min(10);
        let mut table = vec![(0u16, 0u8); 1 << bits];
        let mut long = Vec::new();
        for (sym, &(code, len)) in codes.iter().enumerate() {
            if len <= bits {
                let first = (code << (bits - len)) as usize;
                for e in &mut table[first..first + (1 << (bits - len))] {
                    *e = (sym as u16, len);
                }
            } else {
                long.push((code, len, sym as u16));
            }
        }
        Vlc { max, bits, table, long }
    }

    fn get(&self, b: &mut Bits<'_>) -> Option<usize> {
        let w = b.peek(u32::from(self.max));
        let (sym, len) = self.table[(w >> (self.max - self.bits)) as usize];
        if len != 0 {
            b.skip(len as usize);
            return Some(sym as usize);
        }
        for &(code, len, sym) in &self.long {
            if w >> (self.max - len) == code {
                b.skip(len as usize);
                return Some(sym as usize);
            }
        }
        None
    }
}

struct Tables {
    block_type: Vlc,
    mv: Vlc,
    intra_multi: Vec<Vlc>,
    inter_multi: Vec<Vlc>,
    intra_mean: Vlc,
    inter_mean: Vlc,
}

impl Tables {
    fn new() -> Tables {
        Tables {
            block_type: Vlc::new(&data::BLOCK_TYPE),
            mv: Vlc::new(&data::MV_VLC),
            intra_multi: data::INTRA_MULTISTAGE.iter().map(|t| Vlc::new(t)).collect(),
            inter_multi: data::INTER_MULTISTAGE.iter().map(|t| Vlc::new(t)).collect(),
            intra_mean: Vlc::new(&data::INTRA_MEAN),
            inter_mean: Vlc::new(&data::INTER_MEAN),
        }
    }
}

const INTRA_CODEBOOKS: [&[u32]; 4] = [&data::INTRA_4X2, &data::INTRA_4X4, &data::INTRA_8X4, &data::INTRA_8X8];
const INTER_CODEBOOKS: [&[u32]; 4] = [&data::INTER_4X2, &data::INTER_4X4, &data::INTER_8X4, &data::INTER_8X8];

// ---------------------------------------------------------------------------------------------
// Vector decoding

/// One plane of a frame, with a stride that is a multiple of 16.
#[derive(Clone)]
struct Plane {
    stride: usize,
    pix: Vec<u8>,
}

impl Plane {
    fn new(width: usize, height: usize) -> Plane {
        let (stride, rows) = (width.next_multiple_of(16), height.next_multiple_of(16));
        Plane { stride, pix: vec![0; stride * rows] }
    }
}

#[derive(Clone)]
struct Frame {
    width: u32,
    height: u32,
    planes: [Plane; 3],
}

impl Frame {
    fn new(width: u32, height: u32) -> Frame {
        let (w, h) = (width as usize, height as usize);
        // the chroma planes hold the rounded-up size, the decoded area is ffmpeg's (rounded down)
        Frame { width, height, planes: [Plane::new(w, h), Plane::new(w.div_ceil(4), h.div_ceil(4)), Plane::new(w.div_ceil(4), h.div_ceil(4))] }
    }
}

/// The breadth-first walk of the vector quad-tree (`SVQ1_PROCESS_VECTOR`): calls `vector` with
/// the offset and the level of every leaf.
fn walk(b: &mut Bits<'_>, base: usize, pitch: usize, mut vector: impl FnMut(&mut Bits<'_>, usize, u32) -> Result<(), Error>) -> Result<(), Error> {
    let mut list = [0usize; 63];
    list[0] = base;
    let (mut i, mut m, mut n, mut level) = (0usize, 1usize, 1usize, 5u32);
    while i < n {
        while level > 0 {
            // process the next depth
            if i == m {
                m = n;
                level -= 1;
                if level == 0 {
                    break;
                }
            }
            // divide the block if the next bit is set
            if !b.get1() {
                break;
            }
            if n + 2 > list.len() {
                return inv("vector list overflow");
            }
            list[n] = list[i];
            list[n + 1] = list[i] + ((if level & 1 != 0 { pitch } else { 1 }) << ((level >> 1) + 1));
            n += 2;
            i += 1;
        }
        vector(b, list[i], level)?;
        i += 1;
    }
    Ok(())
}

/// Clip the two 16-bit lanes of `n` to 0..=255 (`SVQ1_ADD_CODEBOOK`).
fn clip_lanes(mut n: u32) -> u32 {
    if n & 0xFF00_FF00 != 0 {
        let n3 = ((n >> 15 & 0x0001_0001) | 0x0100_0100).wrapping_sub(0x0001_0001);
        n = n.wrapping_add(0x7F00_7F00);
        n |= ((!n >> 15 & 0x0001_0001) | 0x0100_0100).wrapping_sub(0x0001_0001);
        n &= n3 & 0x00FF_00FF;
    }
    n
}

/// Add the codebook entries of the stages to the two lane accumulators.
fn add_codebook(codebook: &[u32], entries: &[usize], pos: usize, n1: &mut u32, n2: &mut u32) {
    for &e in entries {
        let n3 = codebook[e + pos] ^ 0x8080_8080;
        *n1 = n1.wrapping_add((n3 & 0xFF00_FF00) >> 8);
        *n2 = n2.wrapping_add(n3 & 0x00FF_00FF);
    }
    *n1 = clip_lanes(*n1);
    *n2 = clip_lanes(*n2);
}

fn read_word(p: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]])
}

fn write_word(p: &mut [u8], o: usize, v: u32) {
    p[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// The codebook entries of a vector's stages (`SVQ1_CALC_CODEBOOK_ENTRIES`).
fn entries_of(b: &mut Bits<'_>, stages: usize, level: u32) -> [usize; 6] {
    let cache = if stages > 0 { b.get(4 * stages as u32) } else { 0 };
    let mut e = [0usize; 6];
    for (j, v) in e.iter_mut().enumerate().take(stages) {
        *v = (((cache >> (4 * (stages - j - 1))) & 0xF) as usize + 16 * j) << (level + 1);
    }
    e
}

fn vector_size(level: u32) -> (usize, usize) {
    (1 << ((4 + level) / 2), 1 << ((3 + level) / 2))
}

fn decode_block_intra(t: &Tables, b: &mut Bits<'_>, plane: &mut Plane, base: usize) -> Result<(), Error> {
    let pitch = plane.stride;
    walk(b, base, pitch, |b, off, level| {
        let (width, height) = vector_size(level);
        // the number of stages (-1 skips the vector, 0 is the mean only)
        let stages = t.intra_multi[level as usize].get(b).ok_or(Error::Invalid("bad stage code"))? as i32 - 1;
        if stages == -1 {
            for y in 0..height {
                let o = off + y * pitch;
                plane.pix[o..o + width].fill(0);
            }
            return Ok(());
        }
        let stages = stages as usize;
        if stages > 0 && level >= 4 {
            return inv("invalid vector");
        }
        let mean = t.intra_mean.get(b).ok_or(Error::Invalid("bad mean code"))? as u32;
        if stages == 0 {
            for y in 0..height {
                let o = off + y * pitch;
                plane.pix[o..o + width].fill(mean as u8);
            }
        } else {
            let codebook = INTRA_CODEBOOKS[level as usize];
            let entries = entries_of(b, stages, level);
            let mean = mean.wrapping_sub(stages as u32 * 128);
            let n4 = (mean << 16).wrapping_add(mean);
            for y in 0..height {
                for x in 0..width / 4 {
                    let (mut n1, mut n2) = (n4, n4);
                    add_codebook(codebook, &entries[..stages], y * (width / 4) + x, &mut n1, &mut n2);
                    write_word(&mut plane.pix, off + y * pitch + x * 4, n1 << 8 | n2);
                }
            }
        }
        Ok(())
    })
}

fn decode_block_non_intra(t: &Tables, b: &mut Bits<'_>, plane: &mut Plane, base: usize, buggy: bool) -> Result<(), Error> {
    let pitch = plane.stride;
    walk(b, base, pitch, |b, off, level| {
        let (width, height) = vector_size(level);
        let stages = t.inter_multi[level as usize].get(b).ok_or(Error::Invalid("bad stage code"))? as i32 - 1;
        if stages == -1 {
            return Ok(());
        }
        let stages = stages as usize;
        if stages > 0 && level >= 4 {
            return inv("invalid vector");
        }
        let mut mean = (t.inter_mean.get(b).ok_or(Error::Invalid("bad mean code"))? as u32).wrapping_sub(256);
        if buggy {
            if mean == (-128i32) as u32 {
                mean = 128;
            } else if mean == 128 {
                mean = (-128i32) as u32;
            }
        }
        let codebook: &[u32] = if stages > 0 { INTER_CODEBOOKS[level as usize] } else { &[] };
        let entries = entries_of(b, stages, level);
        let mean = mean.wrapping_sub(stages as u32 * 128);
        let n4 = (mean << 16).wrapping_add(mean);
        for y in 0..height {
            for x in 0..width / 4 {
                let o = off + y * pitch + x * 4;
                let n3 = read_word(&plane.pix, o);
                // add the mean to the prediction
                let mut n1 = n4.wrapping_add((n3 & 0xFF00_FF00) >> 8);
                let mut n2 = n4.wrapping_add(n3 & 0x00FF_00FF);
                add_codebook(codebook, &entries[..stages], y * (width / 4) + x, &mut n1, &mut n2);
                write_word(&mut plane.pix, o, n1 << 8 | n2);
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------------------------
// Motion

#[derive(Clone, Copy, Default)]
struct Mv {
    x: i32,
    y: i32,
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

fn sign_extend6(v: i32) -> i32 {
    (v << 26) >> 26
}

/// Decode a vector against the median of three predictors.
fn decode_mv(t: &Tables, b: &mut Bits<'_>, p: [Mv; 3]) -> Result<Mv, Error> {
    let mut comp = [0i32; 2];
    for (i, c) in comp.iter_mut().enumerate() {
        let mut diff = t.mv.get(b).ok_or(Error::Invalid("bad motion code"))? as i32;
        if diff != 0 && b.get1() {
            diff = -diff;
        }
        let pred = if i == 0 { mid_pred(p[0].x, p[1].x, p[2].x) } else { mid_pred(p[0].y, p[1].y, p[2].y) };
        *c = sign_extend6(diff.wrapping_add(pred));
    }
    Ok(Mv { x: comp[0], y: comp[1] })
}

/// `put_pixels` of ffmpeg's `hpeldsp` (rounding): an `size` x `size` block at half-pel `mode`
/// (bit 0 horizontal, bit 1 vertical).
fn put_pixels(dst: &mut [u8], doff: usize, src: &[u8], soff: usize, pitch: usize, size: usize, mode: usize) {
    for y in 0..size {
        let s = soff + y * pitch;
        let d = doff + y * pitch;
        for x in 0..size {
            let a = u32::from(src[s + x]);
            dst[d + x] = match mode {
                0 => a as u8,
                1 => ((a + u32::from(src[s + x + 1]) + 1) >> 1) as u8,
                2 => ((a + u32::from(src[s + pitch + x]) + 1) >> 1) as u8,
                _ => ((a + u32::from(src[s + x + 1]) + u32::from(src[s + pitch + x]) + u32::from(src[s + pitch + x + 1]) + 2) >> 2) as u8,
            };
        }
    }
}

struct Ctx<'a> {
    t: &'a Tables,
    prev: &'a Plane,
    buggy: bool,
    width: i32,
    height: i32,
}

fn decode_delta_block(c: &Ctx<'_>, b: &mut Bits<'_>, cur: &mut Plane, motion: &mut [Mv], x: i32, y: i32) -> Result<(), Error> {
    let pitch = cur.stride;
    let base = x as usize + y as usize * pitch;
    let bx = (x / 8) as usize;
    let block_type = c.t.block_type.get(b).ok_or(Error::Invalid("bad block type"))?;
    // reset the motion vectors
    if block_type == 0 || block_type == 3 {
        motion[0] = Mv::default();
        motion[bx + 2] = Mv::default();
        motion[bx + 3] = Mv::default();
    }
    match block_type {
        // skip
        0 => {
            for i in 0..16 {
                let o = base + i * pitch;
                cur.pix[o..o + 16].copy_from_slice(&c.prev.pix[o..o + 16]);
            }
        }
        // one vector
        1 => {
            let pm = [motion[0], if y == 0 { motion[0] } else { motion[bx + 2] }, if y == 0 { motion[0] } else { motion[bx + 4] }];
            let mv = decode_mv(c.t, b, pm)?;
            motion[0] = mv;
            motion[bx + 2] = mv;
            motion[bx + 3] = mv;
            let mx = mv.x.clamp(-2 * x, 2 * (c.width - x - 16));
            let my = mv.y.clamp(-2 * y, 2 * (c.height - y - 16));
            let soff = (x + (mx >> 1)) as usize + (y + (my >> 1)) as usize * pitch;
            put_pixels(&mut cur.pix, base, &c.prev.pix, soff, pitch, 16, ((my & 1) << 1 | (mx & 1)) as usize);
            decode_block_non_intra(c.t, b, cur, base, c.buggy)?;
        }
        // four vectors
        2 => {
            let p0 = motion[0];
            let (a, bb) = if y == 0 { (p0, p0) } else { (motion[bx + 2], motion[bx + 4]) };
            let mv0 = decode_mv(c.t, b, [p0, a, bb])?;
            // vector 1 (top right)
            let (q1, q2) = if y == 0 { (mv0, mv0) } else { (motion[bx + 3], motion[bx + 4]) };
            let mv1 = decode_mv(c.t, b, [mv0, q1, q2])?;
            motion[0] = mv1;
            // vector 2 (bottom left)
            let mv2 = decode_mv(c.t, b, [mv0, mv1, motion[bx + 1]])?;
            motion[bx + 2] = mv2;
            // vector 3 (bottom right)
            let mv3 = decode_mv(c.t, b, [mv0, mv1, mv2])?;
            motion[bx + 3] = mv3;
            let mut cur_off = base;
            for (i, v) in [mv0, mv1, mv2, mv3].into_iter().enumerate() {
                let mvx = (v.x + (i as i32 & 1) * 16).clamp(-2 * x, 2 * (c.width - x - 8));
                let mvy = (v.y + (i as i32 >> 1) * 16).clamp(-2 * y, 2 * (c.height - y - 8));
                let soff = (x + (mvx >> 1)) as usize + (y + (mvy >> 1)) as usize * pitch;
                // the source block sits at the position of the destination block
                put_pixels(&mut cur.pix, cur_off, &c.prev.pix, soff, pitch, 8, ((mvy & 1) << 1 | (mvx & 1)) as usize);
                if i & 1 != 0 {
                    cur_off += 8 * pitch - 8;
                } else {
                    cur_off += 8;
                }
            }
            decode_block_non_intra(c.t, b, cur, base, c.buggy)?;
        }
        // intra
        _ => decode_block_intra(c.t, b, cur, base)?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Frames

/// The SVQ1 decoder.
pub struct Decoder {
    tables: Tables,
    width: u32,
    height: u32,
    last_tempref: u32,
    prev: Option<Frame>,
    extradata: bool,
}

struct Header {
    key: bool,
    nonref: bool,
    buggy: bool,
}

impl Decoder {
    /// A decoder for a stream of `width` x `height` pictures (the size is replaced by the one in
    /// each key frame). `extradata` is whether the container carried codec extradata; without any,
    /// frames after a first one with temporal reference 0 use ffmpeg's workaround for old
    /// encoders.
    pub fn new(width: u32, height: u32, extradata: bool) -> Decoder {
        Decoder { tables: Tables::new(), width: (width + 3) & !3, height: (height + 3) & !3, last_tempref: 0xFF, prev: None, extradata }
    }

    /// Forget the reference picture (after a seek).
    pub fn reset(&mut self) {
        self.prev = None;
    }

    fn header(&mut self, b: &mut Bits<'_>, frame_code: u32) -> Result<Header, Error> {
        let tempref = b.get(8);
        let buggy = tempref == 0 && self.last_tempref == 0 && !self.extradata;
        self.last_tempref = tempref;
        let (key, nonref) = match b.get(2) {
            0 => (true, false),
            1 => (false, false),
            2 => (false, true),
            _ => return inv("frame type"),
        };
        let (mut width, mut height) = (self.width, self.height);
        if key {
            if frame_code == 0x50 || frame_code == 0x60 {
                b.skip(16); // checksum
            }
            if (frame_code ^ 0x10) >= 0x50 {
                // an embedded message
                let len = b.get(8) as usize;
                b.skip(8 * len);
            }
            b.skip(5);
            let code = b.get(3) as usize;
            if code == 7 {
                width = b.get(12);
                height = b.get(12);
                if width == 0 || height == 0 {
                    return inv("zero size");
                }
            } else {
                (width, height) = data::FRAME_SIZES[code];
            }
        }
        if b.get1() {
            b.skip(2);
            if b.get(2) != 0 {
                return inv("header flags");
            }
        }
        if b.get1() {
            b.skip(8);
            // skip_1stop_8data_bits
            if b.left() <= 0 {
                return inv("short header");
            }
            while b.get1() {
                b.skip(8);
                if b.left() <= 0 {
                    return inv("short header");
                }
            }
        }
        if b.left() <= 0 {
            return inv("short header");
        }
        self.width = width;
        self.height = height;
        Ok(Header { key, nonref, buggy })
    }

    /// Decode one frame.
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture, Error> {
        if data.len() < 3 {
            return inv("short frame");
        }
        let frame_code = Bits::new(data).get(22);
        if frame_code & !0x70 != 0 || frame_code & 0x60 == 0 {
            return inv("frame code");
        }
        let mut swapped = None;
        if frame_code != 0x20 {
            // swap some header bytes
            if data.len() < 9 * 4 {
                return inv("short frame");
            }
            let mut v = data.to_vec();
            let word = |v: &[u8], i: usize| u32::from_le_bytes([v[4 + 4 * i], v[5 + 4 * i], v[6 + 4 * i], v[7 + 4 * i]]);
            for i in 0..4 {
                let w = word(&v, i).rotate_left(16) ^ word(&v, 7 - i);
                v[4 + 4 * i..8 + 4 * i].copy_from_slice(&w.to_le_bytes());
            }
            swapped = Some(v);
        }
        let mut b = Bits::new(swapped.as_deref().unwrap_or(data));
        b.skip(22);
        let h = self.header(&mut b, frame_code)?;
        let (w, ht) = (self.width, self.height);
        let mut cur = Frame::new(w, ht);
        if !h.key {
            match &self.prev {
                Some(p) if p.width == w && p.height == ht => {}
                _ => return inv("missing reference frame"),
            }
        }
        let mut pmv = vec![Mv::default(); (w as usize).next_multiple_of(16) / 8 + 3];
        for i in 0..3 {
            let (pw, ph) = if i == 0 {
                (w.next_multiple_of(16) as usize, ht.next_multiple_of(16) as usize)
            } else {
                ((w as usize / 4).next_multiple_of(16), (ht as usize / 4).next_multiple_of(16))
            };
            let plane = &mut cur.planes[i];
            if h.key {
                for y in (0..ph).step_by(16) {
                    for x in (0..pw).step_by(16) {
                        decode_block_intra(&self.tables, &mut b, plane, x + y * plane.stride)?;
                        if b.left() < 0 {
                            return inv("truncated frame");
                        }
                    }
                }
            } else {
                let Some(prev) = &self.prev else { return inv("missing reference frame") };
                let ctx = Ctx { t: &self.tables, prev: &prev.planes[i], buggy: h.buggy, width: pw as i32, height: ph as i32 };
                pmv.fill(Mv::default());
                for y in (0..ph).step_by(16) {
                    for x in (0..pw).step_by(16) {
                        decode_delta_block(&ctx, &mut b, plane, &mut pmv, x as i32, y as i32)?;
                        if b.left() < 0 {
                            return inv("truncated frame");
                        }
                    }
                    pmv[0] = Mv::default();
                }
            }
        }
        let pic = crop(&cur, h.key);
        if !h.nonref {
            self.prev = Some(cur);
        }
        Ok(pic)
    }
}

/// The frame type of a packet (`0` key, `1` delta, `2` delta that no frame refers to), `None`
/// when it is not an SVQ1 frame. The type sits in front of the swapped header bytes.
pub fn frame_type(data: &[u8]) -> Option<u8> {
    let head: [u8; 4] = data.get(..4)?.try_into().ok()?;
    let w = u32::from_be_bytes(head);
    let frame_code = w >> 10;
    if frame_code & !0x70 != 0 || frame_code & 0x60 == 0 {
        return None;
    }
    match (w & 3) as u8 {
        3 => None,
        t => Some(t),
    }
}

fn crop(f: &Frame, key: bool) -> Picture {
    let (w, h) = (f.width as usize, f.height as usize);
    let planes = f
        .planes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (pw, ph) = if i == 0 { (w, h) } else { (w.div_ceil(4), h.div_ceil(4)) };
            (0..ph).flat_map(|y| p.pix[y * p.stride..y * p.stride + pw].iter().copied()).collect()
        })
        .collect();
    Picture { width: f.width, height: f.height, planes, key }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlc_round_trip() {
        let v = Vlc::new(&data::INTRA_MEAN);
        // every code decodes to its own index
        for (sym, &(code, len)) in data::INTRA_MEAN.iter().enumerate() {
            let mut bytes = vec![0u8; 8];
            let shifted = (u64::from(code)) << (64 - u32::from(len));
            bytes.copy_from_slice(&shifted.to_be_bytes());
            let mut b = Bits::new(&bytes);
            assert_eq!(v.get(&mut b), Some(sym));
            assert_eq!(b.pos, len as usize);
        }
    }

    #[test]
    fn rejects_garbage() {
        let mut d = Decoder::new(16, 16, false);
        assert!(d.decode(&[]).is_err());
        assert!(d.decode(&[0xFF; 64]).is_err());
        assert!(d.decode(&[0, 0, 8, 0, 0, 0, 0, 0]).is_err());
    }
}
