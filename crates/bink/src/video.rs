//! The Bink 1 video decoder (`binkvideo`): revisions `b` (the old format, in-place on the
//! previous frame) and `f` to `k`, 4:2:0 with an optional alpha plane.

use std::sync::OnceLock;

use crate::bits::Bits;
use crate::tables::*;
use crate::{Error, Result};

const FLAG_ALPHA: u32 = 0x0010_0000;

/// The largest frame accepted, in pixels.
pub const MAX_PIXELS: u64 = 7680 * 4800;

// ---- sources (bundles) of the current format
const SRC_BLOCK_TYPES: usize = 0;
const SRC_SUB_BLOCK_TYPES: usize = 1;
const SRC_COLORS: usize = 2;
const SRC_PATTERN: usize = 3;
const SRC_X_OFF: usize = 4;
const SRC_Y_OFF: usize = 5;
const SRC_INTRA_DC: usize = 6;
const SRC_INTER_DC: usize = 7;
const SRC_RUN: usize = 8;
const NB_SRC: usize = 9;

// ---- sources of the old format (`a`, `b`)
const BB_BLOCK_TYPES: usize = 0;
const BB_COLORS: usize = 1;
const BB_PATTERN: usize = 2;
const BB_X_OFF: usize = 3;
const BB_Y_OFF: usize = 4;
const BB_INTRA_DC: usize = 5;
const BB_INTER_DC: usize = 6;
const BB_INTRA_Q: usize = 7;
const BB_INTER_Q: usize = 8;
const BB_INTER_COEFS: usize = 9;
const BB_NB_SRC: usize = 10;
const BB_SIZES: [u8; BB_NB_SRC] = [4, 8, 8, 5, 5, 11, 11, 4, 4, 7];
const BB_SIGNED: [bool; BB_NB_SRC] = [false, false, false, true, true, false, true, false, false, false];

// block types
const SKIP_BLOCK: i32 = 0;
const SCALED_BLOCK: i32 = 1;
const MOTION_BLOCK: i32 = 2;
const RUN_BLOCK: i32 = 3;
const RESIDUE_BLOCK: i32 = 4;
const INTRA_BLOCK: i32 = 5;
const FILL_BLOCK: i32 = 6;
const INTER_BLOCK: i32 = 7;
const PATTERN_BLOCK: i32 = 8;
const RAW_BLOCK: i32 = 9;

const RLE_LENS: [usize; 4] = [4, 8, 12, 32];
const DC_START_BITS: u32 = 11;

/// A decoded frame: 8-bit planes Y, U, V (and A), cropped to the picture size; the chroma
/// planes are `(width + 1) / 2` by `(height + 1) / 2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Vec<u8>>,
    /// False when the packet ended before all planes were coded: the planes not coded are the
    /// previous frame's (ffmpeg leaves them undefined).
    pub complete: bool,
}

// ---------------------------------------------------------------- Huffman trees

struct Vlc {
    maxbits: u32,
    /// (symbol, length) per `maxbits`-bit little-endian peek; length 0 marks a hole.
    table: Vec<(u8, u8)>,
}

fn vlcs() -> &'static [Vlc; 16] {
    static V: OnceLock<[Vlc; 16]> = OnceLock::new();
    V.get_or_init(|| {
        std::array::from_fn(|t| {
            let maxbits = TREE_LENS[t][15] as u32;
            let mut table = vec![(0u8, 0u8); 1 << maxbits];
            for sym in 0..16 {
                let len = TREE_LENS[t][sym] as u32;
                let code = TREE_BITS[t][sym] as usize;
                let mut hi = 0usize;
                while hi < (1usize << (maxbits - len)) {
                    table[code | (hi << len)] = (sym as u8, len as u8);
                    hi += 1;
                }
            }
            Vlc { maxbits, table }
        })
    })
}

#[derive(Clone, Copy, Default)]
struct Tree {
    vlc_num: usize,
    syms: [u8; 16],
}

impl Tree {
    fn get(&self, gb: &mut Bits<'_>) -> u8 {
        let v = &vlcs()[self.vlc_num];
        let (sym, len) = v.table[gb.peek(v.maxbits) as usize];
        gb.skip(len.max(1) as usize);
        self.syms[sym as usize]
    }
}

fn merge(gb: &mut Bits<'_>, dst: &mut [u8], src: &[u8], size: usize) {
    let (mut a, mut b) = (0usize, size);
    let (mut size1, mut size2) = (size, size);
    let mut d = 0;
    loop {
        if gb.bit() == 0 {
            dst[d] = src[a];
            a += 1;
            size1 -= 1;
        } else {
            dst[d] = src[b];
            b += 1;
            size2 -= 1;
        }
        d += 1;
        if size1 == 0 || size2 == 0 {
            break;
        }
    }
    while size1 > 0 {
        dst[d] = src[a];
        d += 1;
        a += 1;
        size1 -= 1;
    }
    while size2 > 0 {
        dst[d] = src[b];
        d += 1;
        b += 1;
        size2 -= 1;
    }
}

fn read_tree(gb: &mut Bits<'_>) -> Result<Tree> {
    if gb.left() < 4 {
        return Err(Error::Invalid("tree: out of data"));
    }
    let mut tree = Tree { vlc_num: gb.get(4) as usize, syms: [0; 16] };
    if tree.vlc_num == 0 {
        for (i, s) in tree.syms.iter_mut().enumerate() {
            *s = i as u8;
        }
        return Ok(tree);
    }
    if gb.bit() != 0 {
        let mut len = gb.get(3) as usize;
        let mut seen = [false; 16];
        for i in 0..=len {
            tree.syms[i] = gb.get(4) as u8;
            seen[tree.syms[i] as usize] = true;
        }
        let mut i = 0;
        while i < 16 && len < 15 {
            if !seen[i] {
                len += 1;
                tree.syms[len] = i as u8;
            }
            i += 1;
        }
    } else {
        let len = gb.get(2) as usize;
        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        for (i, v) in a.iter_mut().enumerate() {
            *v = i as u8;
        }
        let (mut inp, mut out) = (&mut a, &mut b);
        for i in 0..=len {
            let size = 1usize << i;
            let mut t = 0;
            while t < 16 {
                merge(gb, &mut out[t..], &inp[t..], size);
                t += size << 1;
            }
            std::mem::swap(&mut inp, &mut out);
        }
        tree.syms = *inp;
    }
    Ok(tree)
}

// ---------------------------------------------------------------- bundles

#[derive(Default)]
struct Bundle {
    len: u32,
    tree: Tree,
    data: Vec<u8>,
    cur_dec: Option<usize>,
    cur_ptr: usize,
}

impl Bundle {
    fn reset(&mut self) {
        self.cur_dec = Some(0);
        self.cur_ptr = 0;
    }
    fn u8_at(&mut self) -> i32 {
        let v = self.data.get(self.cur_ptr).copied().unwrap_or(0);
        self.cur_ptr += 1;
        v as i32
    }
    fn i8_at(&mut self) -> i32 {
        self.u8_at() as u8 as i8 as i32
    }
    fn i16_at(&mut self) -> i32 {
        let lo = self.data.get(self.cur_ptr).copied().unwrap_or(0);
        let hi = self.data.get(self.cur_ptr + 1).copied().unwrap_or(0);
        self.cur_ptr += 2;
        i16::from_le_bytes([lo, hi]) as i32
    }
}

/// The start of a bundle read: `Ok(None)` when nothing is to be decoded.
fn check_read(gb: &mut Bits<'_>, b: &mut Bundle) -> Option<(usize, u32)> {
    let cur = b.cur_dec?;
    if cur > b.cur_ptr {
        return None;
    }
    let t = gb.get(b.len);
    if t == 0 {
        b.cur_dec = None;
        return None;
    }
    Some((cur, t))
}

fn bad(msg: &'static str) -> Error {
    Error::Invalid(msg)
}

fn log2(x: u32) -> u32 {
    31 - x.leading_zeros()
}

// ---------------------------------------------------------------- DSP

const A1: i32 = 2896;
const A2: i32 = 2217;
const A3: i32 = 3784;
const A4: i32 = -5352;

#[inline]
fn mul(x: i32, y: i32) -> i32 {
    ((x as u32).wrapping_mul(y as u32) as i32) >> 11
}

#[inline]
fn idct_1d(s: [i32; 8], munge: impl Fn(i32) -> i32) -> [i32; 8] {
    let a0 = s[0].wrapping_add(s[4]);
    let a1 = s[0].wrapping_sub(s[4]);
    let a2 = s[2].wrapping_add(s[6]);
    let a3 = mul(A1, s[2].wrapping_sub(s[6]));
    let a4 = s[5].wrapping_add(s[3]);
    let a5 = s[5].wrapping_sub(s[3]);
    let a6 = s[1].wrapping_add(s[7]);
    let a7 = s[1].wrapping_sub(s[7]);
    let b0 = a4.wrapping_add(a6);
    let b1 = mul(A3, a5.wrapping_add(a7));
    let b2 = mul(A4, a5).wrapping_sub(b0).wrapping_add(b1);
    let b3 = mul(A1, a6.wrapping_sub(a4)).wrapping_sub(b2);
    let b4 = mul(A2, a7).wrapping_add(b3).wrapping_sub(b1);
    let w = i32::wrapping_add;
    let s_ = i32::wrapping_sub;
    [
        munge(w(w(a0, a2), b0)),
        munge(w(s_(w(a1, a3), a2), b2)),
        munge(w(w(s_(a1, a3), a2), b3)),
        munge(s_(s_(a0, a2), b4)),
        munge(w(s_(a0, a2), b4)),
        munge(s_(w(s_(a1, a3), a2), b3)),
        munge(s_(s_(w(a1, a3), a2), b2)),
        munge(s_(w(a0, a2), b0)),
    ]
}

fn idct_cols(block: &[i32; 64]) -> [i32; 64] {
    let mut temp = [0i32; 64];
    for i in 0..8 {
        let col: [i32; 8] = std::array::from_fn(|r| block[r * 8 + i]);
        if col[1..].iter().all(|&v| v == 0) {
            for r in 0..8 {
                temp[r * 8 + i] = col[0];
            }
        } else {
            let o = idct_1d(col, |x| x);
            for r in 0..8 {
                temp[r * 8 + i] = o[r];
            }
        }
    }
    temp
}

fn idct_rows(temp: &[i32; 64]) -> [i32; 64] {
    let mut out = [0i32; 64];
    for r in 0..8 {
        let row: [i32; 8] = std::array::from_fn(|c| temp[r * 8 + c]);
        let o = idct_1d(row, |x| x.wrapping_add(0x7F) >> 8);
        out[r * 8..r * 8 + 8].copy_from_slice(&o);
    }
    out
}

// ---------------------------------------------------------------- planes

#[derive(Clone)]
struct Plane {
    stride: usize,
    /// Visible size in the picture.
    w: usize,
    h: usize,
    data: Vec<u8>,
}

impl Plane {
    fn new(w: usize, h: usize, bw: usize, bh: usize) -> Plane {
        let stride = bw * 8 + 8;
        Plane { stride, w, h, data: vec![0; stride * (bh * 8 + 8)] }
    }
    fn put_block(&mut self, dst: usize, rows: impl Fn(usize) -> [u8; 8]) {
        for r in 0..8 {
            let row = rows(r);
            let at = dst + r * self.stride;
            if let Some(s) = self.data.get_mut(at..at + 8) {
                s.copy_from_slice(&row);
            }
        }
    }
    fn get8(&self, at: usize) -> [u8; 8] {
        let mut o = [0u8; 8];
        if let Some(s) = self.data.get(at..at + 8) {
            o.copy_from_slice(s);
        }
        o
    }
    fn set(&mut self, at: usize, v: u8) {
        if let Some(p) = self.data.get_mut(at) {
            *p = v;
        }
    }
    fn fill(&mut self, dst: usize, v: u8, n: usize) {
        for r in 0..n {
            let at = dst + r * self.stride;
            if let Some(s) = self.data.get_mut(at..at + n) {
                s.fill(v);
            }
        }
    }
    fn idct_put(&mut self, dst: usize, block: &[i32; 64]) {
        let o = idct_rows(&idct_cols(block));
        self.put_block(dst, |r| std::array::from_fn(|c| o[r * 8 + c] as u8));
    }
    fn idct_add(&mut self, dst: usize, block: &[i32; 64]) {
        let o = idct_rows(&idct_cols(block));
        let cur: Vec<[u8; 8]> = (0..8).map(|r| self.get8(dst + r * self.stride)).collect();
        self.put_block(dst, |r| std::array::from_fn(|c| cur[r][c].wrapping_add(o[r * 8 + c] as u8)));
    }
    fn add_pixels8(&mut self, dst: usize, block: &[i16; 64]) {
        let cur: Vec<[u8; 8]> = (0..8).map(|r| self.get8(dst + r * self.stride)).collect();
        self.put_block(dst, |r| std::array::from_fn(|c| cur[r][c].wrapping_add(block[r * 8 + c] as u8)));
    }
    /// 16x16 from an 8x8 block, every pixel doubled both ways.
    fn scale_block(&mut self, dst: usize, src: &[u8; 64]) {
        for j in 0..8 {
            for i in 0..8 {
                let v = src[j * 8 + i];
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    self.set(dst + (2 * j + dy) * self.stride + 2 * i + dx, v);
                }
            }
        }
    }
    fn cropped(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(self.w * self.h);
        for y in 0..self.h {
            o.extend_from_slice(&self.data[y * self.stride..y * self.stride + self.w]);
        }
        o
    }
}

// ---------------------------------------------------------------- decoder

/// The decoder of one stream.
pub struct Decoder {
    width: usize,
    height: usize,
    version: u8,
    has_alpha: bool,
    swap_planes: bool,
    frame_num: u32,
    /// Set when a frame leaves pixels undefined (a skipped 16x16 part without its block).
    undefined: bool,
    last: Option<Vec<Plane>>,
    bundle: Vec<Bundle>,
    col_high: [Tree; 16],
    col_lastval: usize,
    binkb_intra_quant: Vec<[i32; 64]>,
    binkb_inter_quant: Vec<[i32; 64]>,
}

fn plane_blocks(width: usize, height: usize, chroma: bool) -> (usize, usize) {
    if chroma { ((width + 15) >> 4, (height + 15) >> 4) } else { ((width + 7) >> 3, (height + 7) >> 3) }
}

impl Decoder {
    /// A decoder for a `width` x `height` stream. `tag` is the file signature (`BIKi`...: the
    /// fourth byte is the revision) and `flags` the 32-bit extradata word of the header.
    pub fn new(width: u32, height: u32, tag: [u8; 4], flags: u32) -> Result<Decoder> {
        if tag[..3] != *b"BIK" {
            return Err(Error::Unsupported("not a Bink 1 stream"));
        }
        let version = tag[3];
        if !matches!(version, b'b' | b'f' | b'g' | b'h' | b'i' | b'k') {
            return Err(Error::Unsupported("unknown Bink revision"));
        }
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Unsupported("frame size"));
        }
        let (w, h) = (width as usize, height as usize);
        let blocks = ((w + 7) >> 3) * ((h + 7) >> 3);
        let (mut intra, mut inter) = (Vec::new(), Vec::new());
        if version == b'b' {
            let inv: Vec<usize> = {
                let mut inv = vec![0usize; 64];
                for (i, &s) in BINK_SCAN.iter().enumerate() {
                    inv[s as usize] = i;
                }
                inv
            };
            const S: [i64; 64] = [
                1073741824, 1489322693, 1402911301, 1262586814, 1073741824, 843633538, 581104888, 296244703, 1489322693, 2065749918, 1945893874, 1751258219,
                1489322693, 1170153332, 806015634, 410903207, 1402911301, 1945893874, 1832991949, 1649649171, 1402911301, 1102260336, 759250125, 387062357,
                1262586814, 1751258219, 1649649171, 1484645031, 1262586814, 992008094, 683307060, 348346918, 1073741824, 1489322693, 1402911301, 1262586814,
                1073741824, 843633538, 581104888, 296244703, 843633538, 1170153332, 1102260336, 992008094, 843633538, 662838617, 456571181, 232757969,
                581104888, 806015634, 759250125, 683307060, 581104888, 456571181, 314491699, 160326478, 296244703, 410903207, 387062357, 348346918, 296244703,
                232757969, 160326478, 81733730,
            ];
            let c: i64 = 1 << 30;
            for j in 0..16 {
                let (mut a, mut b) = ([0i32; 64], [0i32; 64]);
                for i in 0..64 {
                    let k = inv[i];
                    let den = BINKB_DEN[j] as i64 * (c >> 12);
                    a[k] = (BINKB_INTRA_SEED[i] as i64 * S[i] * BINKB_NUM[j] as i64 / den) as i32;
                    b[k] = (BINKB_INTER_SEED[i] as i64 * S[i] * BINKB_NUM[j] as i64 / den) as i32;
                }
                intra.push(a);
                inter.push(b);
            }
        }
        let nb = if version > b'b' { NB_SRC } else { BB_NB_SRC };
        let bundle = (0..nb.max(BB_NB_SRC)).map(|_| Bundle { data: vec![0; blocks * 64], ..Default::default() }).collect();
        Ok(Decoder {
            width: w,
            height: h,
            version,
            has_alpha: flags & FLAG_ALPHA != 0,
            swap_planes: version >= b'h',
            frame_num: 0,
            undefined: false,
            last: None,
            bundle,
            col_high: [Tree::default(); 16],
            col_lastval: 0,
            binkb_intra_quant: intra,
            binkb_inter_quant: inter,
        })
    }

    /// Forget the previous frame's position (a seek); the next frame counts as the first.
    pub fn flush(&mut self) {
        self.frame_num = 0;
    }

    fn new_frame(&self) -> Vec<Plane> {
        let mut v = Vec::new();
        for p in 0..4 {
            if p == 3 && !self.has_alpha {
                break;
            }
            let chroma = p == 1 || p == 2;
            let (bw, bh) = plane_blocks(self.width, self.height, chroma);
            let (w, h) = if chroma { (self.width.div_ceil(2), self.height.div_ceil(2)) } else { (self.width, self.height) };
            v.push(Plane::new(w, h, bw, bh));
        }
        v
    }

    /// Decode one video packet.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Picture> {
        let mut gb = Bits::new(pkt);
        let bits_count = pkt.len() * 8;
        let new = self.version > b'b';
        let mut frame = if new { self.new_frame() } else { self.last.take().unwrap_or_else(|| self.new_frame()) };
        let last = if new { Some(self.last.take().unwrap_or_else(|| self.new_frame())) } else { None };
        self.undefined = false;
        let res = self.decode_planes(&mut gb, &mut frame, last.as_deref(), bits_count);
        let mut complete = true;
        if let Ok(done) = res
            && done < 3
        {
            complete = false;
            let prev = last.as_deref().unwrap_or(&[]);
            for p in 0..frame.len() {
                // plane indices are coded 0, 1, 2 (swapped) and alpha first
                let coded = p == 3 || (0..done).any(|k| (if k == 0 || !self.swap_planes { k } else { k ^ 3 }) == p);
                if !coded && let Some(q) = prev.get(p) {
                    frame[p] = q.clone();
                }
            }
        }
        if let Err(e) = res {
            // keep the reference chain on errors
            self.last = Some(if new { last.unwrap() } else { frame });
            return Err(e);
        }
        complete &= !self.undefined;
        let pic = Picture { width: self.width as u32, height: self.height as u32, planes: frame.iter().map(Plane::cropped).collect(), complete };
        self.last = Some(frame);
        Ok(pic)
    }

    fn decode_planes(&mut self, gb: &mut Bits<'_>, frame: &mut [Plane], last: Option<&[Plane]>, bits_count: usize) -> Result<usize> {
        if self.has_alpha {
            if self.version >= b'i' {
                gb.skip(32);
            }
            self.decode_plane(gb, frame, last, 3, false)?;
        }
        if self.version >= b'i' {
            gb.skip(32);
        }
        self.frame_num += 1;
        let mut done = 0;
        for plane in 0..3usize {
            let idx = if plane == 0 || !self.swap_planes { plane } else { plane ^ 3 };
            if self.version > b'b' {
                self.decode_plane(gb, frame, last, idx, plane != 0)?;
            } else {
                self.binkb_decode_plane(gb, frame, idx, self.frame_num == 1, plane != 0)?;
            }
            done += 1;
            if gb.count() >= bits_count {
                break;
            }
        }
        Ok(done)
    }

    // ---- bundle readers

    fn init_lengths(&mut self, width: usize, bw: usize) {
        let width = width.div_ceil(8) * 8;
        let l = |x: usize| log2(x as u32 + 511) + 1;
        self.bundle[SRC_BLOCK_TYPES].len = l(width >> 3);
        self.bundle[SRC_SUB_BLOCK_TYPES].len = l(width >> 4);
        self.bundle[SRC_COLORS].len = l(bw * 64);
        for s in [SRC_INTRA_DC, SRC_INTER_DC, SRC_X_OFF, SRC_Y_OFF] {
            self.bundle[s].len = l(width >> 3);
        }
        self.bundle[SRC_PATTERN].len = l(bw << 3);
        self.bundle[SRC_RUN].len = l(bw * 48);
    }

    fn read_bundle(&mut self, gb: &mut Bits<'_>, n: usize) -> Result<()> {
        if n == SRC_COLORS {
            for i in 0..16 {
                self.col_high[i] = read_tree(gb)?;
            }
            self.col_lastval = 0;
        }
        if n != SRC_INTRA_DC && n != SRC_INTER_DC {
            self.bundle[n].tree = read_tree(gb)?;
        }
        self.bundle[n].reset();
        Ok(())
    }

    fn read_runs(&mut self, gb: &mut Bits<'_>) -> Result<()> {
        let b = &mut self.bundle[SRC_RUN];
        let Some((cur, t)) = check_read(gb, b) else { return Ok(()) };
        let end = cur + t as usize;
        if end > b.data.len() {
            return Err(bad("run value out of bounds"));
        }
        if gb.left() < 1 {
            return Err(bad("out of data"));
        }
        if gb.bit() != 0 {
            let v = gb.get(4) as u8;
            b.data[cur..end].fill(v);
        } else {
            for i in cur..end {
                b.data[i] = b.tree.get(gb);
            }
        }
        b.cur_dec = Some(end);
        Ok(())
    }

    fn read_motion_values(&mut self, gb: &mut Bits<'_>, n: usize) -> Result<()> {
        let b = &mut self.bundle[n];
        let Some((cur, t)) = check_read(gb, b) else { return Ok(()) };
        let end = cur + t as usize;
        if end > b.data.len() {
            return Err(bad("too many motion values"));
        }
        if gb.left() < 1 {
            return Err(bad("out of data"));
        }
        let signed = |gb: &mut Bits<'_>, v: i32| {
            if v != 0 {
                let sign = -(gb.bit() as i32);
                (v ^ sign) - sign
            } else {
                v
            }
        };
        if gb.bit() != 0 {
            let v = gb.get(4) as i32;
            let v = signed(gb, v);
            b.data[cur..end].fill(v as u8);
        } else {
            for i in cur..end {
                let v = b.tree.get(gb) as i32;
                b.data[i] = signed(gb, v) as u8;
            }
        }
        b.cur_dec = Some(end);
        Ok(())
    }

    fn read_block_types(&mut self, gb: &mut Bits<'_>, n: usize) -> Result<()> {
        let version = self.version;
        let b = &mut self.bundle[n];
        let Some((cur, mut t)) = check_read(gb, b) else { return Ok(()) };
        if version == b'k' {
            t ^= 0xBB;
            if t == 0 {
                b.cur_dec = None;
                return Ok(());
            }
        }
        let end = cur + t as usize;
        if end > b.data.len() {
            return Err(bad("too many block type values"));
        }
        if gb.left() < 1 {
            return Err(bad("out of data"));
        }
        let mut pos = cur;
        if gb.bit() != 0 {
            let v = gb.get(4) as u8;
            b.data[cur..end].fill(v);
            pos = end;
        } else {
            let mut last = 0u8;
            while pos < end {
                let v = b.tree.get(gb);
                if v < 12 {
                    last = v;
                    b.data[pos] = v;
                    pos += 1;
                } else {
                    let run = RLE_LENS[(v - 12) as usize];
                    if end - pos < run {
                        return Err(bad("block type run"));
                    }
                    b.data[pos..pos + run].fill(last);
                    pos += run;
                }
            }
        }
        b.cur_dec = Some(pos);
        Ok(())
    }

    fn read_patterns(&mut self, gb: &mut Bits<'_>) -> Result<()> {
        let b = &mut self.bundle[SRC_PATTERN];
        let Some((cur, t)) = check_read(gb, b) else { return Ok(()) };
        let end = cur + t as usize;
        if end > b.data.len() {
            return Err(bad("too many pattern values"));
        }
        for i in cur..end {
            if gb.left() < 2 {
                return Err(bad("out of data"));
            }
            let lo = b.tree.get(gb);
            let hi = b.tree.get(gb);
            b.data[i] = lo | (hi << 4);
        }
        b.cur_dec = Some(end);
        Ok(())
    }

    fn read_colors(&mut self, gb: &mut Bits<'_>) -> Result<()> {
        let version = self.version;
        let b = &mut self.bundle[SRC_COLORS];
        let Some((cur, t)) = check_read(gb, b) else { return Ok(()) };
        let end = cur + t as usize;
        if end > b.data.len() {
            return Err(bad("too many color values"));
        }
        if gb.left() < 1 {
            return Err(bad("out of data"));
        }
        let fix = |v: i32| -> u8 {
            if version < b'i' {
                let sign = ((v as u8) as i8 as i32) >> 7;
                (((v & 0x7F) ^ sign) - sign + 0x80) as u8
            } else {
                v as u8
            }
        };
        if gb.bit() != 0 {
            self.col_lastval = self.col_high[self.col_lastval].get(gb) as usize;
            let v = b.tree.get(gb) as i32;
            let v = fix(((self.col_lastval as i32) << 4) | v);
            b.data[cur..end].fill(v);
        } else {
            for i in cur..end {
                if gb.left() < 2 {
                    return Err(bad("out of data"));
                }
                self.col_lastval = self.col_high[self.col_lastval].get(gb) as usize;
                let v = b.tree.get(gb) as i32;
                b.data[i] = fix(((self.col_lastval as i32) << 4) | v);
            }
        }
        b.cur_dec = Some(end);
        Ok(())
    }

    fn read_dcs(&mut self, gb: &mut Bits<'_>, n: usize, start_bits: u32, has_sign: bool) -> Result<()> {
        let b = &mut self.bundle[n];
        let Some((cur, len)) = check_read(gb, b) else { return Ok(()) };
        let mut len = len as i64;
        if gb.left() < (start_bits - has_sign as u32) as isize {
            return Err(bad("out of data"));
        }
        let mut v = gb.get(start_bits - has_sign as u32) as i32;
        if v != 0 && has_sign {
            let sign = -(gb.bit() as i32);
            v = (v ^ sign) - sign;
        }
        let cap = b.data.len() / 2;
        let mut dst = cur / 2;
        let put = |b: &mut Bundle, dst: &mut usize, v: i32| {
            let le = (v as i16).to_le_bytes();
            b.data[*dst * 2] = le[0];
            b.data[*dst * 2 + 1] = le[1];
            *dst += 1;
        };
        if cap < dst + 1 {
            return Err(bad("dc bundle full"));
        }
        put(b, &mut dst, v);
        len -= 1;
        let mut i = 0i64;
        while i < len {
            let len2 = (len - i).min(8) as usize;
            if cap < dst + len2 {
                return Err(bad("dc bundle full"));
            }
            let bsize = gb.get(4);
            if bsize != 0 {
                for _ in 0..len2 {
                    let mut v2 = gb.get(bsize) as i32;
                    if v2 != 0 {
                        let sign = -(gb.bit() as i32);
                        v2 = (v2 ^ sign) - sign;
                    }
                    v += v2;
                    put(b, &mut dst, v);
                    if !(-32768..=32767).contains(&v) {
                        return Err(bad("dc value out of bounds"));
                    }
                }
            } else {
                for _ in 0..len2 {
                    put(b, &mut dst, v);
                }
            }
            i += 8;
        }
        b.cur_dec = Some(dst * 2);
        Ok(())
    }

    fn get_value(&mut self, n: usize) -> i32 {
        let b = &mut self.bundle[n];
        if n < SRC_X_OFF || n == SRC_RUN {
            b.u8_at()
        } else if n == SRC_X_OFF || n == SRC_Y_OFF {
            b.i8_at()
        } else {
            b.i16_at()
        }
    }

    // ---- block coefficient readers

    /// Returns the quantizer index, and the coefficient positions read.
    fn read_dct_coeffs(&mut self, gb: &mut Bits<'_>, block: &mut [i32; 64], q: i32) -> Result<(usize, Vec<usize>)> {
        let mut coef_list = [0i32; 128];
        let mut mode_list = [0i32; 128];
        let mut coef_idx = Vec::with_capacity(64);
        let (mut list_start, mut list_end) = (64usize, 64usize);
        if gb.left() < 4 {
            return Err(bad("out of data"));
        }
        for (c, m) in [(4, 0), (24, 0), (44, 0), (1, 3), (2, 3), (3, 3)] {
            coef_list[list_end] = c;
            mode_list[list_end] = m;
            list_end += 1;
        }
        let mut bits = gb.get(4) as i32 - 1;
        while bits >= 0 {
            let mut list_pos = list_start;
            while list_pos < list_end {
                if (mode_list[list_pos] | coef_list[list_pos]) == 0 || gb.bit() == 0 {
                    list_pos += 1;
                    continue;
                }
                let mut ccoef = coef_list[list_pos];
                let mode = mode_list[list_pos];
                match mode {
                    0 | 2 => {
                        if mode == 0 {
                            coef_list[list_pos] = ccoef + 4;
                            mode_list[list_pos] = 1;
                        } else {
                            coef_list[list_pos] = 0;
                            mode_list[list_pos] = 0;
                            list_pos += 1;
                        }
                        for _ in 0..4 {
                            if gb.bit() != 0 {
                                list_start -= 1;
                                coef_list[list_start] = ccoef;
                                mode_list[list_start] = 3;
                            } else {
                                let t = coef_value(gb, bits);
                                block[BINK_SCAN[(ccoef & 63) as usize] as usize] = t;
                                coef_idx.push((ccoef & 63) as usize);
                            }
                            ccoef += 1;
                        }
                    }
                    1 => {
                        mode_list[list_pos] = 2;
                        for _ in 0..3 {
                            ccoef += 4;
                            coef_list[list_end] = ccoef;
                            mode_list[list_end] = 2;
                            list_end += 1;
                        }
                    }
                    _ => {
                        let t = coef_value(gb, bits);
                        block[BINK_SCAN[(ccoef & 63) as usize] as usize] = t;
                        coef_idx.push((ccoef & 63) as usize);
                        coef_list[list_pos] = 0;
                        mode_list[list_pos] = 0;
                        list_pos += 1;
                    }
                }
            }
            bits -= 1;
        }
        let quant_idx = if q == -1 { gb.get(4) as i32 } else { q };
        if !(0..16).contains(&quant_idx) {
            return Err(bad("quant index out of range"));
        }
        Ok((quant_idx as usize, coef_idx))
    }
}

fn coef_value(gb: &mut Bits<'_>, bits: i32) -> i32 {
    if bits == 0 {
        1 - ((gb.bit() as i32) << 1)
    } else {
        let t = (gb.get(bits as u32) | (1 << bits)) as i32;
        let sign = -(gb.bit() as i32);
        (t ^ sign) - sign
    }
}

fn unquantize(block: &mut [i32; 64], quant: &[i32; 64], coef_idx: &[usize]) {
    let q = |v: i32, q: i32| ((v as u32).wrapping_mul(q as u32) as i32) >> 11;
    block[0] = q(block[0], quant[0]);
    for &idx in coef_idx {
        let p = BINK_SCAN[idx] as usize;
        block[p] = q(block[p], quant[idx]);
    }
}

/// The residue of a motion block.
fn read_residue(gb: &mut Bits<'_>, block: &mut [i16; 64], mut masks_count: i32) {
    let mut coef_list = [0i32; 128];
    let mut mode_list = [0i32; 128];
    let (mut list_start, mut list_end) = (64usize, 64usize);
    let mut nz_coeff = [0usize; 64];
    let mut nz_count = 0usize;
    for (c, m) in [(4, 0), (24, 0), (44, 0), (0, 2)] {
        coef_list[list_end] = c;
        mode_list[list_end] = m;
        list_end += 1;
    }
    let mut mask: i32 = 1 << gb.get(3);
    while mask != 0 {
        for i in 0..nz_count {
            if gb.bit() == 0 {
                continue;
            }
            let p = nz_coeff[i];
            if block[p] < 0 {
                block[p] = block[p].wrapping_sub(mask as i16);
            } else {
                block[p] = block[p].wrapping_add(mask as i16);
            }
            masks_count -= 1;
            if masks_count < 0 {
                return;
            }
        }
        let mut list_pos = list_start;
        while list_pos < list_end {
            if (coef_list[list_pos] | mode_list[list_pos]) == 0 || gb.bit() == 0 {
                list_pos += 1;
                continue;
            }
            let mut ccoef = coef_list[list_pos];
            let mode = mode_list[list_pos];
            match mode {
                0 | 2 => {
                    if mode == 0 {
                        coef_list[list_pos] = ccoef + 4;
                        mode_list[list_pos] = 1;
                    } else {
                        coef_list[list_pos] = 0;
                        mode_list[list_pos] = 0;
                        list_pos += 1;
                    }
                    for _ in 0..4 {
                        if gb.bit() != 0 {
                            list_start -= 1;
                            coef_list[list_start] = ccoef;
                            mode_list[list_start] = 3;
                        } else {
                            let p = BINK_SCAN[(ccoef & 63) as usize] as usize;
                            nz_coeff[nz_count] = p;
                            nz_count += 1;
                            let sign = -(gb.bit() as i32);
                            block[p] = ((mask ^ sign) - sign) as i16;
                            masks_count -= 1;
                            if masks_count < 0 {
                                return;
                            }
                        }
                        ccoef += 1;
                    }
                }
                1 => {
                    mode_list[list_pos] = 2;
                    for _ in 0..3 {
                        ccoef += 4;
                        coef_list[list_end] = ccoef;
                        mode_list[list_end] = 2;
                        list_end += 1;
                    }
                }
                _ => {
                    let p = BINK_SCAN[(ccoef & 63) as usize] as usize;
                    nz_coeff[nz_count] = p;
                    nz_count += 1;
                    let sign = -(gb.bit() as i32);
                    block[p] = ((mask ^ sign) - sign) as i16;
                    coef_list[list_pos] = 0;
                    mode_list[list_pos] = 0;
                    list_pos += 1;
                    masks_count -= 1;
                    if masks_count < 0 {
                        return;
                    }
                }
            }
        }
        mask >>= 1;
    }
}

impl Decoder {
    fn put_pixels(&mut self, dst_plane: &mut Plane, dst: usize, prev_plane: &Plane, prev: usize, ref_end: usize) -> Result<()> {
        let xoff = self.get_value(SRC_X_OFF) as isize;
        let yoff = self.get_value(SRC_Y_OFF) as isize;
        let r = prev as isize + xoff + yoff * prev_plane.stride as isize;
        if r < 0 || r as usize > ref_end {
            return Err(bad("copy out of bounds"));
        }
        let rows: Vec<[u8; 8]> = (0..8).map(|y| prev_plane.get8(r as usize + y * prev_plane.stride)).collect();
        dst_plane.put_block(dst, |y| rows[y]);
        Ok(())
    }

    fn decode_plane(&mut self, gb: &mut Bits<'_>, frame: &mut [Plane], last: Option<&[Plane]>, plane_idx: usize, is_chroma: bool) -> Result<()> {
        let (bw, bh) = plane_blocks(self.width, self.height, is_chroma);
        let width = self.width >> is_chroma as usize;
        let height = self.height >> is_chroma as usize;
        if self.version == b'k' && gb.bit() != 0 {
            let fill = gb.get(8) as u8;
            let p = &mut frame[plane_idx];
            for y in 0..height {
                let at = y * p.stride;
                p.data[at..at + width].fill(fill);
            }
            gb.align32();
            return Ok(());
        }
        self.init_lengths(width.max(8), bw);
        for i in 0..NB_SRC {
            self.read_bundle(gb, i)?;
        }
        let fallback;
        let prev_plane: &Plane = match last {
            Some(l) => &l[plane_idx],
            None => {
                fallback = frame[plane_idx].clone();
                &fallback
            }
        };
        let stride = frame[plane_idx].stride;
        let ref_end = (bw - 1 + prev_plane.stride * (bh - 1)) * 8;
        let _ = stride;
        let dstp = &mut frame[plane_idx];
        let stride = dstp.stride;
        let mut ublock = [0u8; 64];
        let mut covered = vec![false; (bw + 1) * (bh + 1)];
        let cw = bw + 1;
        for by in 0..bh {
            self.read_block_types(gb, SRC_BLOCK_TYPES)?;
            self.read_block_types(gb, SRC_SUB_BLOCK_TYPES)?;
            self.read_colors(gb)?;
            self.read_patterns(gb)?;
            self.read_motion_values(gb, SRC_X_OFF)?;
            self.read_motion_values(gb, SRC_Y_OFF)?;
            self.read_dcs(gb, SRC_INTRA_DC, DC_START_BITS, false)?;
            self.read_dcs(gb, SRC_INTER_DC, DC_START_BITS, true)?;
            self.read_runs(gb)?;
            let mut bx = 0;
            while bx < bw {
                let at = 8 * by * stride + 8 * bx;
                let mut blk = self.get_value(SRC_BLOCK_TYPES);
                if ((by & 1) != 0 || (bx & 1) != 0) && blk == SCALED_BLOCK {
                    if !covered[by * cw + bx] {
                        self.undefined = true;
                    }
                    bx += 2;
                    continue;
                }
                covered[by * cw + bx] = true;
                match blk {
                    SKIP_BLOCK => {
                        let rows: Vec<[u8; 8]> = (0..8).map(|y| prev_plane.get8(at + y * prev_plane.stride)).collect();
                        dstp.put_block(at, |y| rows[y]);
                    }
                    SCALED_BLOCK => {
                        blk = self.get_value(SRC_SUB_BLOCK_TYPES);
                        match blk {
                            RUN_BLOCK => {
                                if gb.left() < 4 {
                                    return Err(bad("out of data"));
                                }
                                let scan = &PATTERNS[gb.get(4) as usize];
                                self.run_block(gb, scan, |_, pos, v| ublock[pos] = v)?;
                            }
                            INTRA_BLOCK => {
                                let mut dct = [0i32; 64];
                                dct[0] = self.get_value(SRC_INTRA_DC);
                                let (qi, ci) = self.read_dct_coeffs(gb, &mut dct, -1)?;
                                unquantize(&mut dct, &INTRA_QUANT[qi], &ci);
                                let o = idct_rows(&idct_cols(&dct));
                                for (d, s) in ublock.iter_mut().zip(o) {
                                    *d = s as u8;
                                }
                            }
                            FILL_BLOCK => {
                                let v = self.get_value(SRC_COLORS) as u8;
                                dstp.fill(at, v, 16);
                            }
                            PATTERN_BLOCK => {
                                let col = [self.get_value(SRC_COLORS) as u8, self.get_value(SRC_COLORS) as u8];
                                for j in 0..8 {
                                    let mut v = self.get_value(SRC_PATTERN);
                                    for i in 0..8 {
                                        ublock[i + j * 8] = col[(v & 1) as usize];
                                        v >>= 1;
                                    }
                                }
                            }
                            RAW_BLOCK => {
                                for j in 0..8 {
                                    for i in 0..8 {
                                        ublock[i + j * 8] = self.get_value(SRC_COLORS) as u8;
                                    }
                                }
                            }
                            _ => return Err(bad("incorrect 16x16 block type")),
                        }
                        if blk != FILL_BLOCK {
                            dstp.scale_block(at, &ublock);
                        }
                        for (dy, dx) in [(0, 1), (1, 0), (1, 1)] {
                            covered[(by + dy) * cw + bx + dx] = true;
                        }
                        bx += 2;
                        continue;
                    }
                    MOTION_BLOCK => self.put_pixels(dstp, at, prev_plane, at, ref_end)?,
                    RUN_BLOCK => {
                        let scan = &PATTERNS[gb.get(4) as usize];
                        self.run_block(gb, scan, |_, pos, v| {
                            let (x, y) = (pos & 7, pos >> 3);
                            dstp.set(at + x + y * stride, v);
                        })?;
                    }
                    RESIDUE_BLOCK => {
                        self.put_pixels(dstp, at, prev_plane, at, ref_end)?;
                        let mut block = [0i16; 64];
                        let v = gb.get(7);
                        read_residue(gb, &mut block, v as i32);
                        dstp.add_pixels8(at, &block);
                    }
                    INTRA_BLOCK => {
                        let mut dct = [0i32; 64];
                        dct[0] = self.get_value(SRC_INTRA_DC);
                        let (qi, ci) = self.read_dct_coeffs(gb, &mut dct, -1)?;
                        unquantize(&mut dct, &INTRA_QUANT[qi], &ci);
                        dstp.idct_put(at, &dct);
                    }
                    FILL_BLOCK => {
                        let v = self.get_value(SRC_COLORS) as u8;
                        dstp.fill(at, v, 8);
                    }
                    INTER_BLOCK => {
                        self.put_pixels(dstp, at, prev_plane, at, ref_end)?;
                        let mut dct = [0i32; 64];
                        dct[0] = self.get_value(SRC_INTER_DC);
                        let (qi, ci) = self.read_dct_coeffs(gb, &mut dct, -1)?;
                        unquantize(&mut dct, &INTER_QUANT[qi], &ci);
                        dstp.idct_add(at, &dct);
                    }
                    PATTERN_BLOCK => {
                        let col = [self.get_value(SRC_COLORS) as u8, self.get_value(SRC_COLORS) as u8];
                        for i in 0..8 {
                            let mut v = self.get_value(SRC_PATTERN);
                            for j in 0..8 {
                                dstp.set(at + i * stride + j, col[(v & 1) as usize]);
                                v >>= 1;
                            }
                        }
                    }
                    RAW_BLOCK => {
                        for i in 0..8 {
                            let row: [u8; 8] =
                                std::array::from_fn(|k| self.bundle[SRC_COLORS].data.get(self.bundle[SRC_COLORS].cur_ptr + i * 8 + k).copied().unwrap_or(0));
                            dstp.put_block_row(at + i * stride, row);
                        }
                        self.bundle[SRC_COLORS].cur_ptr += 64;
                    }
                    _ => return Err(bad("unknown block type")),
                }
                bx += 1;
            }
        }
        gb.align32();
        Ok(())
    }

    /// The runs of a run block (the shared loop of both block kinds): `put(self, position, v)`.
    fn run_block(&mut self, gb: &mut Bits<'_>, scan: &[u8; 64], mut put: impl FnMut(&mut Self, usize, u8)) -> Result<()> {
        let mut i = 0usize;
        let mut s = 0usize;
        loop {
            let run = self.get_value(SRC_RUN) as usize + 1;
            i += run;
            if i > 64 {
                return Err(bad("run out of bounds"));
            }
            if gb.bit() != 0 {
                let v = self.get_value(SRC_COLORS) as u8;
                for _ in 0..run {
                    put(self, scan[s] as usize, v);
                    s += 1;
                }
            } else {
                for _ in 0..run {
                    let v = self.get_value(SRC_COLORS) as u8;
                    put(self, scan[s] as usize, v);
                    s += 1;
                }
            }
            if i >= 63 {
                break;
            }
        }
        if i == 63 {
            let v = self.get_value(SRC_COLORS) as u8;
            put(self, scan[s] as usize, v);
        }
        Ok(())
    }
}

impl Plane {
    fn put_block_row(&mut self, at: usize, row: [u8; 8]) {
        if let Some(s) = self.data.get_mut(at..at + 8) {
            s.copy_from_slice(&row);
        }
    }
}

// ---------------------------------------------------------------- the old format (`a`, `b`)

impl Decoder {
    fn bb_value(&mut self, n: usize) -> i32 {
        let b = &mut self.bundle[n];
        if BB_SIZES[n] <= 8 { if BB_SIGNED[n] { b.i8_at() } else { b.u8_at() } } else { b.i16_at() }
    }

    fn bb_read_bundle(&mut self, gb: &mut Bits<'_>, n: usize) -> Result<()> {
        let bits = BB_SIZES[n] as u32;
        let mask = 1i32 << (bits - 1);
        let signed = BB_SIGNED[n];
        let b = &mut self.bundle[n];
        let Some((cur, len)) = check_read(gb, b) else { return Ok(()) };
        let wide = bits > 8;
        let len = len as usize;
        if b.data.len() - cur.min(b.data.len()) < len * (1 + wide as usize) {
            return Err(bad("binkb bundle full"));
        }
        let mut at = cur;
        for _ in 0..len {
            let mut v = gb.get(bits) as i32;
            if signed {
                v -= mask;
            }
            if wide {
                let le = (v as i16).to_le_bytes();
                b.data[at] = le[0];
                b.data[at + 1] = le[1];
                at += 2;
            } else {
                b.data[at] = v as u8;
                at += 1;
            }
        }
        b.cur_dec = Some(at);
        Ok(())
    }

    fn bb_motion(&mut self, p: &mut Plane, at: usize, ybias: i32, ref_end: usize) {
        let xoff = self.bb_value(BB_X_OFF) as isize;
        let yoff = (self.bb_value(BB_Y_OFF) + ybias) as isize;
        let r = at as isize + xoff + yoff * p.stride as isize;
        if r < 0 || r as usize > ref_end {
            return; // "Reference block is out of bounds": a warning only
        }
        let rows: Vec<[u8; 8]> = (0..8).map(|y| p.get8(r as usize + y * p.stride)).collect();
        p.put_block(at, |y| rows[y]);
    }

    fn binkb_decode_plane(&mut self, gb: &mut Bits<'_>, frame: &mut [Plane], plane_idx: usize, is_key: bool, is_chroma: bool) -> Result<()> {
        let (bw, bh) = plane_blocks(self.width, self.height, is_chroma);
        let ybias = if is_key { -15 } else { 0 };
        for b in self.bundle.iter_mut().take(BB_NB_SRC) {
            b.reset();
            b.len = 13;
        }
        let p = &mut frame[plane_idx];
        let stride = p.stride;
        let ref_end = ((bh - 1) * stride + bw - 1) * 8;
        for by in 0..bh {
            for i in 0..BB_NB_SRC {
                self.bb_read_bundle(gb, i)?;
            }
            for bx in 0..bw {
                let at = 8 * by * stride + 8 * bx;
                let blk = self.bb_value(BB_BLOCK_TYPES);
                match blk {
                    0 => {}
                    1 => {
                        let scan = &PATTERNS[gb.get(4) as usize];
                        let (mut i, mut s) = (0usize, 0usize);
                        let put = |p: &mut Plane, s: &mut usize, v: i32| {
                            let pos = scan[(*s).min(63)] as usize;
                            p.set(at + (pos & 7) + (pos >> 3) * stride, v as u8);
                            *s += 1;
                        };
                        loop {
                            let mode = gb.bit();
                            let run = gb.get(BINKB_RUNBITS[i.min(63)] as u32) as usize + 1;
                            i += run;
                            if i > 64 {
                                return Err(bad("run out of bounds"));
                            }
                            if mode != 0 {
                                let v = self.bb_value(BB_COLORS);
                                for _ in 0..run {
                                    put(p, &mut s, v);
                                }
                            } else {
                                for _ in 0..run {
                                    let v = self.bb_value(BB_COLORS);
                                    put(p, &mut s, v);
                                }
                            }
                            if i >= 63 {
                                break;
                            }
                        }
                        if i == 63 {
                            let v = self.bb_value(BB_COLORS);
                            put(p, &mut s, v);
                        }
                    }
                    2 | 4 => {
                        if blk == 4 {
                            self.bb_motion(p, at, ybias, ref_end);
                        }
                        let mut dct = [0i32; 64];
                        dct[0] = self.bb_value(if blk == 2 { BB_INTRA_DC } else { BB_INTER_DC });
                        let qp = self.bb_value(if blk == 2 { BB_INTRA_Q } else { BB_INTER_Q });
                        let (qi, ci) = self.read_dct_coeffs(gb, &mut dct, qp)?;
                        if blk == 2 {
                            unquantize(&mut dct, &self.binkb_intra_quant[qi], &ci);
                            p.idct_put(at, &dct);
                        } else {
                            unquantize(&mut dct, &self.binkb_inter_quant[qi], &ci);
                            p.idct_add(at, &dct);
                        }
                    }
                    3 => {
                        self.bb_motion(p, at, ybias, ref_end);
                        let mut block = [0i16; 64];
                        let v = self.bb_value(BB_INTER_COEFS);
                        read_residue(gb, &mut block, v);
                        p.add_pixels8(at, &block);
                    }
                    5 => {
                        let v = self.bb_value(BB_COLORS) as u8;
                        p.fill(at, v, 8);
                    }
                    6 => {
                        let col = [self.bb_value(BB_COLORS) as u8, self.bb_value(BB_COLORS) as u8];
                        for i in 0..8 {
                            let mut v = self.bb_value(BB_PATTERN);
                            for j in 0..8 {
                                p.set(at + i * stride + j, col[(v & 1) as usize]);
                                v >>= 1;
                            }
                        }
                    }
                    7 => self.bb_motion(p, at, ybias, ref_end),
                    8 => {
                        let b = &mut self.bundle[BB_COLORS];
                        for i in 0..8 {
                            let row: [u8; 8] = std::array::from_fn(|k| b.data.get(b.cur_ptr + i * 8 + k).copied().unwrap_or(0));
                            p.put_block_row(at + i * stride, row);
                        }
                        b.cur_ptr += 64;
                    }
                    _ => return Err(bad("unknown block type")),
                }
            }
        }
        gb.align32();
        Ok(())
    }
}
