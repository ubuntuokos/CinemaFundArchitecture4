// Ported from FFmpeg n7.1.1 libavcodec/rv34.c, rv30.c and rv40.c (Copyright (c) 2007 Mike
// Melanson, Konstantin Shishkov), LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes
// GPL-3.0-or-later.

//! Pure-Rust decoder for **RealVideo 3.0** (`RV30`) and **RealVideo 4.0** (`RV40`): the H.264-like
//! RealVideo codecs. I, P and B pictures, 4×4 and 16×16 intra prediction, the RV30 third-pel and
//! RV40 quarter-pel motion compensation, RV40 weighted bi-prediction, direct and skipped B
//! macroblocks, slices, picture size changes and both loop filters. The output is the same as
//! ffmpeg's `rv30` / `rv40` decoders, pixel for pixel.
//!
//! [`Decoder::decode`] takes one frame as the RealMedia demuxer assembles it (a slice count, a
//! slice table and the slices); `cfa4-realmedia` produces them.

mod dsp;
mod pred;
mod tables;
mod vlc;

use std::sync::Arc;

use crate::{Error, Gb, MAX_SIDE, PictType, Picture, Plane, Result, mid_pred};
use tables::*;
use vlc::{VlcSet, se, tables as vlcs, ue};

/// The two codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Rv30,
    Rv40,
}

// Macroblock types (`RV34_MB_*`).
const MB_INTRA: usize = 0;
const MB_INTRA16: usize = 1;
const MB_P_16X16: usize = 2;
const MB_P_8X8: usize = 3;
const MB_B_FORWARD: usize = 4;
const MB_B_BACKWARD: usize = 5;
const MB_SKIP: usize = 6;
const MB_B_DIRECT: usize = 7;
const MB_P_16X8: usize = 8;
const MB_P_8X16: usize = 9;
const MB_B_BIDIR: usize = 10;
const MB_P_MIX16X16: usize = 11;

// lavc macroblock type flags (mpegutils.h).
const T_INTRA4X4: u32 = 1;
const T_INTRA16X16: u32 = 1 << 1;
const T_16X16: u32 = 1 << 3;
const T_16X8: u32 = 1 << 4;
const T_8X16: u32 = 1 << 5;
const T_8X8: u32 = 1 << 6;
const T_DIRECT2: u32 = 1 << 8;
const T_FORWARD: u32 = 1 << 12;
const T_BACKWARD: u32 = 1 << 13;
const T_SKIP: u32 = 1 << 17;
const T_SEPARATE_DC: u32 = 0x0100_0000;

const fn is_intra(t: u32) -> bool {
    t & 7 != 0
}

const TO_LAVC: [u32; 12] = [
    T_INTRA4X4,
    T_INTRA16X16 | T_SEPARATE_DC,
    T_16X16 | T_FORWARD,
    T_8X8 | T_FORWARD,
    T_16X16 | T_FORWARD,
    T_16X16 | T_BACKWARD,
    T_SKIP,
    T_DIRECT2 | T_16X16,
    T_16X8 | T_FORWARD,
    T_8X16 | T_FORWARD,
    T_16X16 | T_FORWARD | T_BACKWARD,
    T_16X16 | T_FORWARD | T_SEPARATE_DC,
];

const PART_W: [usize; 12] = [2, 2, 2, 1, 2, 2, 2, 2, 2, 1, 2, 2];
const PART_H: [usize; 12] = [2, 2, 2, 1, 2, 2, 2, 2, 1, 2, 2, 2];
const AVAIL_INDEXES: [usize; 4] = [6, 7, 10, 11];
const NUM_MVS: [usize; 12] = [0, 0, 1, 4, 1, 1, 0, 0, 2, 2, 2, 1];
const ITTRANS: [i32; 9] =
    [pred::DC, pred::VERT, pred::HOR, pred::DIAG_DOWN_RIGHT, pred::DIAG_DOWN_LEFT, pred::VERT_RIGHT, pred::VERT_LEFT, pred::HOR_UP, pred::HOR_DOWN];
const ITTRANS16: [i32; 4] = [pred::DC8, pred::VERT8, pred::HOR8, pred::PLANE8];
const CHROMA_COEFFS: [i32; 3] = [0, 3, 5];

#[derive(Clone, Copy, Default)]
struct SliceInfo {
    /// 0 (I), 2 (P) or 3 (B).
    ty: u32,
    quant: usize,
    vlc_set: u32,
    start: usize,
    end: usize,
    width: u32,
    height: u32,
    pts: i32,
}

/// A picture with what later pictures and the loop filter need of it.
struct Frame {
    planes: [Plane; 3],
    /// Forward and backward vectors per 8×8 block, `b8_stride` wide, after one spare row and entry.
    mv: [Vec<[i16; 2]>; 2],
    mb_type: Vec<u32>,
    qscale: Vec<u8>,
    pict_type: PictType,
    index: u64,
}

impl Frame {
    fn new(mb_w: usize, mb_h: usize, pict_type: PictType, luma: u8) -> Self {
        let b8 = 2 * mb_w + 1;
        let n = b8 * (2 * mb_h + 2) + 2;
        Self {
            planes: [Plane::new(mb_w * 16, mb_h * 16, luma), Plane::new(mb_w * 8, mb_h * 8, 128), Plane::new(mb_w * 8, mb_h * 8, 128)],
            mv: [vec![[0; 2]; n], vec![[0; 2]; n]],
            mb_type: vec![0; mb_w * mb_h],
            qscale: vec![0; mb_w * mb_h],
            pict_type,
            index: 0,
        }
    }
}

/// Which VLC set the macroblock uses.
#[derive(Clone, Copy)]
struct VlcSel {
    inter: bool,
    idx: usize,
}

/// A RealVideo 3 / 4 decoder.
pub struct Decoder {
    rv30: bool,
    extradata: Vec<u8>,
    max_rpr: u32,
    orig_w: u32,
    orig_h: u32,
    width: u32,
    height: u32,
    mb_w: usize,
    mb_h: usize,
    b8: isize,

    pict_type: PictType,
    si: SliceInfo,
    mb_x: usize,
    mb_y: usize,
    resync_x: usize,
    resync_y: usize,
    first_slice_line: bool,
    mb_num_left: isize,
    mb_skip_run: u32,
    qscale: usize,

    it_stride: usize,
    /// Four rows of the previous macroblock row's 4×4 intra types, then four of the current one.
    it_hist: Vec<i8>,
    mb_types: Vec<u8>,
    cbp_luma: Vec<u16>,
    cbp_chroma: Vec<u8>,
    deblock: Vec<u16>,
    avail: [u32; 12],
    block_type: usize,
    luma_vlc: usize,
    chroma_vlc: usize,
    is16: bool,
    dmv: [[i32; 2]; 4],
    cur_vlcs: VlcSel,
    block: [i16; 16],

    cur_pts: i32,
    last_pts: i32,
    next_pts: i32,
    scaled_weight: bool,
    weight1: i32,
    weight2: i32,
    mv_weight1: i32,
    mv_weight2: i32,
    tmp: [[Plane; 3]; 2],

    cur: Option<Frame>,
    decoded: Vec<bool>,
    last: Option<Arc<Frame>>,
    next: Option<Arc<Frame>>,
    frames_started: u64,
    flushed: Option<u64>,
}

fn pts_diff(a: i32, b: i32) -> i32 {
    (a - b + 8192) & 0x1FFF
}

fn rl32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn rb32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// `get_slice_offset`: the slice table entry `n` of a packet, or `buf_size` past the last.
fn slice_offset(hdr: &[u8], n: usize, count: usize, buf_size: usize) -> Option<usize> {
    if n < count {
        let flag = if n == 0 { rl32(hdr, 0) } else { rl32(hdr, n * 8) };
        let at = n * 8 + 4;
        let v = if flag == Some(1) { rl32(hdr, at)? } else { rb32(hdr, at)? };
        Some(v as usize)
    } else {
        Some(buf_size)
    }
}

/// `ff_rv34_get_start_offset`.
fn start_bits(mb_size: usize) -> u32 {
    let i = MB_MAX_SIZES.iter().take(5).position(|&m| m as usize + 1 >= mb_size).unwrap_or(5);
    MB_BITS_SIZES.get(i).copied().unwrap_or(14)
}

impl Decoder {
    /// A decoder for `codec` with the stream's coded size and its codec data (the bytes after the
    /// frame rate in the RealMedia `VIDO` header).
    pub fn new(codec: Codec, width: u32, height: u32, extradata: &[u8]) -> Result<Self> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::Invalid("picture size"));
        }
        let rv30 = codec == Codec::Rv30;
        if rv30 && extradata.len() < 2 {
            return Err(Error::Invalid("RV30 codec data too short"));
        }
        let max_rpr = if rv30 { u32::from(extradata.get(1).copied().unwrap_or(0) & 7) } else { 0 };
        let mut d = Self {
            rv30,
            extradata: extradata.to_vec(),
            max_rpr,
            orig_w: width,
            orig_h: height,
            width: 0,
            height: 0,
            mb_w: 0,
            mb_h: 0,
            b8: 0,
            pict_type: PictType::I,
            si: SliceInfo::default(),
            mb_x: 0,
            mb_y: 0,
            resync_x: 0,
            resync_y: 0,
            first_slice_line: true,
            mb_num_left: 0,
            mb_skip_run: 0,
            qscale: 0,
            it_stride: 0,
            it_hist: Vec::new(),
            mb_types: Vec::new(),
            cbp_luma: Vec::new(),
            cbp_chroma: Vec::new(),
            deblock: Vec::new(),
            avail: [0; 12],
            block_type: 0,
            luma_vlc: 0,
            chroma_vlc: 0,
            is16: false,
            dmv: [[0; 2]; 4],
            cur_vlcs: VlcSel { inter: false, idx: 0 },
            block: [0; 16],
            cur_pts: 0,
            last_pts: 0,
            next_pts: 0,
            scaled_weight: false,
            weight1: 0,
            weight2: 0,
            mv_weight1: 0,
            mv_weight2: 0,
            tmp: std::array::from_fn(|_| [Plane::new(16, 16, 0), Plane::new(8, 8, 0), Plane::new(8, 8, 0)]),
            cur: None,
            decoded: Vec::new(),
            last: None,
            next: None,
            frames_started: 0,
            flushed: None,
        };
        d.resize(width, height);
        Ok(d)
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        self.mb_w = (w as usize).div_ceil(16);
        self.mb_h = (h as usize).div_ceil(16);
        self.b8 = (2 * self.mb_w + 1) as isize;
        let n = self.mb_w * self.mb_h;
        self.it_stride = self.mb_w * 4 + 4;
        self.it_hist = vec![-1; self.it_stride * 8];
        self.mb_types = vec![0; n];
        self.cbp_luma = vec![0; n];
        self.cbp_chroma = vec![0; n];
        self.deblock = vec![0; n];
    }

    // -----------------------------------------------------------------------------------------
    // Headers

    fn parse_slice_header(&self, gb: &mut Gb<'_>) -> Result<SliceInfo> {
        let mut si = SliceInfo::default();
        let (mut w, mut h) = (self.width, self.height);
        if self.rv30 {
            if gb.get(3) != 0 {
                return Err(Error::Invalid("slice header"));
            }
            si.ty = gb.get(2);
            if si.ty == 1 {
                si.ty = 0;
            }
            if gb.get1() {
                return Err(Error::Invalid("slice header"));
            }
            si.quant = gb.get(5) as usize;
            gb.skip(1);
            si.pts = gb.get(13) as i32;
            let rpr_bits = 32 - self.max_rpr.leading_zeros().min(31);
            let rpr = gb.get(rpr_bits.max(1));
            if rpr != 0 {
                if rpr > self.max_rpr {
                    return Err(Error::Invalid("RPR index"));
                }
                let i = 6 + 2 * rpr as usize;
                let (Some(&a), Some(&b)) = (self.extradata.get(i), self.extradata.get(i + 1)) else {
                    return Err(Error::Invalid("RV30 codec data too short"));
                };
                w = u32::from(a) << 2;
                h = u32::from(b) << 2;
            } else {
                w = self.orig_w;
                h = self.orig_h;
            }
        } else {
            if gb.get1() {
                return Err(Error::Invalid("slice header"));
            }
            si.ty = gb.get(2);
            if si.ty == 1 {
                si.ty = 0;
            }
            si.quant = gb.get(5) as usize;
            if gb.get(2) != 0 {
                return Err(Error::Invalid("slice header"));
            }
            si.vlc_set = gb.get(2);
            gb.skip(1);
            si.pts = gb.get(13) as i32;
            if si.ty == 0 || !gb.get1() {
                w = dimension(gb, &RV40_STANDARD_WIDTHS)?;
                h = dimension(gb, &RV40_STANDARD_HEIGHTS)?;
            }
        }
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(Error::Invalid("picture size"));
        }
        si.width = w;
        si.height = h;
        let mb_size = (w as usize).div_ceil(16) * (h as usize).div_ceil(16);
        si.start = gb.get(start_bits(mb_size)) as usize;
        if self.rv30 {
            gb.skip(1);
        }
        Ok(si)
    }

    // -----------------------------------------------------------------------------------------
    // Frame decoding

    /// Decodes one frame; returns a picture in display order (B-pictures at once, reference
    /// pictures when the next one is decoded).
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Option<Picture>> {
        let Some((&n, rest)) = pkt.split_first() else { return Ok(None) };
        let slice_count = usize::from(n) + 1;
        if rest.len() < 8 * slice_count {
            return Err(Error::Invalid("slice count"));
        }
        let (hdr, buf) = rest.split_at(8 * slice_count);
        let buf_size = buf.len();
        let off0 = slice_offset(hdr, 0, slice_count, buf_size).ok_or(Error::Invalid("slice table"))?;
        if off0 > buf_size {
            return Err(Error::Invalid("slice offset"));
        }
        let first = {
            let mut gb = Gb::new(buf.get(off0..).unwrap_or(&[]), (buf_size - off0) * 8);
            self.parse_slice_header(&mut gb)?
        };
        if first.start != 0 {
            return Err(Error::Invalid("first slice header"));
        }
        let faulty_b = self.last.is_none() && first.ty == 3;

        // A frame that did not finish is ended as it is.
        let mut out = None;
        if self.cur.is_some() {
            out = self.end_frame(false);
        }
        if first.width != self.width || first.height != self.height {
            self.resize(first.width, first.height);
            self.last = None;
            self.next = None;
        }
        if faulty_b {
            return Err(Error::Invalid("B-frame without references"));
        }
        self.pict_type = match first.ty {
            0 => PictType::I,
            3 => PictType::B,
            _ => PictType::P,
        };
        self.start_frame();
        self.cur_pts = first.pts;
        if self.pict_type != PictType::B {
            self.last_pts = self.next_pts;
            self.next_pts = self.cur_pts;
        } else {
            let refdist = pts_diff(self.next_pts, self.last_pts);
            let dist0 = pts_diff(self.cur_pts, self.last_pts);
            let dist1 = pts_diff(self.next_pts, self.cur_pts);
            if refdist == 0 {
                self.mv_weight1 = 8192;
                self.mv_weight2 = 8192;
                self.weight1 = 8192;
                self.weight2 = 8192;
                self.scaled_weight = false;
            } else {
                self.mv_weight1 = (dist0 << 14) / refdist;
                self.mv_weight2 = (dist1 << 14) / refdist;
                if (self.mv_weight1 | self.mv_weight2) & 511 != 0 {
                    self.weight1 = self.mv_weight1;
                    self.weight2 = self.mv_weight2;
                    self.scaled_weight = false;
                } else {
                    self.weight1 = self.mv_weight1 >> 9;
                    self.weight2 = self.mv_weight2 >> 9;
                    self.scaled_weight = true;
                }
            }
        }
        self.mb_x = 0;
        self.mb_y = 0;

        let mut last = false;
        for i in 0..slice_count {
            let (Some(offset), Some(offset1)) = (slice_offset(hdr, i, slice_count, buf_size), slice_offset(hdr, i + 1, slice_count, buf_size)) else { break };
            if offset > offset1 || offset1 > buf_size {
                break;
            }
            let mut size = offset1 - offset;
            let mut end = self.mb_w * self.mb_h;
            if i + 1 < slice_count {
                let Some(offset2) = slice_offset(hdr, i + 2, slice_count, buf_size) else { break };
                if offset2 < offset1 || offset2 > buf_size {
                    break;
                }
                let mut gb = Gb::new(buf.get(offset1..).unwrap_or(&[]), (buf_size - offset1) * 8);
                match self.parse_slice_header(&mut gb) {
                    Ok(si) => end = si.start,
                    Err(_) => size = offset2 - offset,
                }
            }
            match self.decode_slice(end, buf.get(offset..).unwrap_or(&[]), size) {
                Ok(true) => {
                    last = true;
                    break;
                }
                Ok(false) => {}
                Err(_) => {}
            }
        }
        if last {
            out = self.end_frame(true).or(out);
        }
        Ok(out)
    }

    /// The last reference picture, which [`Decoder::decode`] holds back until the next reference
    /// picture (what ffmpeg returns when it is flushed). The decoder keeps it as its reference;
    /// it is not returned a second time.
    pub fn flush(&mut self) -> Option<Picture> {
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

    fn start_frame(&mut self) {
        if self.pict_type != PictType::B {
            self.last = self.next.take();
        }
        if self.last.is_none() && self.pict_type != PictType::I {
            self.last = Some(Arc::new(Frame::new(self.mb_w, self.mb_h, PictType::I, 128)));
        }
        if self.next.is_none() && self.pict_type == PictType::B {
            self.next = Some(Arc::new(Frame::new(self.mb_w, self.mb_h, PictType::I, 128)));
        }
        let mut cur = Frame::new(self.mb_w, self.mb_h, self.pict_type, 0);
        cur.index = self.frames_started;
        self.frames_started = self.frames_started.saturating_add(1);
        self.cur = Some(cur);
        self.decoded = vec![false; self.mb_w * self.mb_h];
    }

    /// Ends the picture being decoded (`complete`: every row decoded); it becomes a reference
    /// unless it is a B-picture. Returns what ffmpeg's `finish_frame` outputs.
    fn end_frame(&mut self, complete: bool) -> Option<Picture> {
        let rows = if complete { self.mb_h } else { self.mb_y.saturating_sub(1) };
        for row in 0..rows {
            if self.rv30 {
                self.rv30_loop_filter(row);
            } else {
                self.rv40_loop_filter(row);
            }
        }
        let mut cur = self.cur.take()?;
        // Conceal macroblocks that were not decoded (damaged or missing slices).
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
                        let v = self.last.as_ref().and_then(|r| r.planes[p].d.get(yy * w + xx).copied()).unwrap_or(128);
                        if let Some(d) = cur.planes[p].d.get_mut(yy * w + xx) {
                            *d = v;
                        }
                    }
                }
            }
        }
        if cur.pict_type == PictType::B {
            return complete.then(|| self.picture(&cur));
        }
        let cur = Arc::new(cur);
        self.next = Some(cur);
        if !complete {
            return None;
        }
        self.last.as_ref().filter(|l| self.flushed != Some(l.index)).map(|l| self.picture(l))
    }

    fn picture(&self, f: &Frame) -> Picture {
        let (w, h) = (self.width as usize, self.height as usize);
        let crop = |p: &Plane, w: usize, h: usize| -> Vec<u8> {
            let mut v = Vec::with_capacity(w * h);
            for y in 0..h {
                v.extend_from_slice(p.d.get(y * p.w..y * p.w + w).unwrap_or(&[]));
            }
            v.resize(w * h, 0);
            v
        };
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        Picture {
            width: self.width,
            height: self.height,
            y: crop(&f.planes[0], w, h),
            u: crop(&f.planes[1], cw, ch),
            v: crop(&f.planes[2], cw, ch),
            pict_type: f.pict_type,
            decode_index: f.index,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Slices

    /// `rv34_decode_slice`; `Ok(true)` when the picture is complete.
    fn decode_slice(&mut self, end: usize, data: &[u8], size: usize) -> Result<bool> {
        let mut gb = Gb::new(data, size * 8);
        let mut si = self.parse_slice_header(&mut gb)?;
        let ty = match si.ty {
            0 => PictType::I,
            3 => PictType::B,
            _ => PictType::P,
        };
        if ty != self.pict_type {
            return Err(Error::Invalid("slice type mismatch"));
        }
        if si.width != self.width || si.height != self.height {
            return Err(Error::Invalid("slice size mismatch"));
        }
        si.end = end;
        self.si = si;
        self.qscale = si.quant;
        self.mb_num_left = si.end as isize - si.start as isize;
        self.mb_skip_run = 0;
        if si.start != self.mb_x + self.mb_y * self.mb_w {
            self.mb_x = si.start % self.mb_w.max(1);
            self.mb_y = si.start / self.mb_w.max(1);
        }
        self.it_hist.iter_mut().for_each(|v| *v = -1);
        self.first_slice_line = true;
        self.resync_x = self.mb_x;
        self.resync_y = self.mb_y;
        while !self.slice_end(&gb) {
            let k = self.mb_x + self.mb_y * self.mb_w;
            if si.ty != 0 {
                self.decode_inter_mb(&mut gb)?;
            } else {
                self.decode_intra_mb(&mut gb)?;
            }
            if let Some(d) = self.decoded.get_mut(k) {
                *d = true;
            }
            self.mb_x += 1;
            if self.mb_x == self.mb_w {
                self.mb_x = 0;
                self.mb_y += 1;
                let s = self.it_stride * 4;
                self.it_hist.copy_within(s..2 * s, 0);
                self.it_hist[s..2 * s].iter_mut().for_each(|v| *v = -1);
            }
            if self.mb_x == self.resync_x {
                self.first_slice_line = false;
            }
            self.mb_num_left -= 1;
        }
        Ok(self.mb_y == self.mb_h)
    }

    fn slice_end(&self, gb: &Gb<'_>) -> bool {
        if self.mb_y >= self.mb_h || self.mb_num_left <= 0 {
            return true;
        }
        if self.mb_skip_run > 1 {
            return false;
        }
        let bits = gb.left();
        bits <= 0 || (bits < 8 && gb.peek(bits as u32) == 0)
    }

    // -----------------------------------------------------------------------------------------
    // Macroblocks

    fn mb_pos(&self) -> usize {
        self.mb_x + self.mb_y * self.mb_w
    }

    fn cur_mb_type(&self, pos: usize) -> u32 {
        self.cur.as_ref().and_then(|c| c.mb_type.get(pos).copied()).unwrap_or(0)
    }

    fn set_cur_mb_type(&mut self, t: u32) {
        let pos = self.mb_pos();
        if let Some(v) = self.cur.as_mut().and_then(|c| c.mb_type.get_mut(pos)) {
            *v = t;
        }
    }

    fn fill_avail(&mut self) {
        self.avail = [0; 12];
        for i in [6, 7, 10, 11] {
            self.avail[i] = 1;
        }
        let pos = self.mb_pos();
        let (mw, x) = (self.mb_w as isize, self.mb_x as isize);
        let dist = (x - self.resync_x as isize) + (self.mb_y as isize - self.resync_y as isize) * mw;
        if x != 0 && dist != 0 {
            let t = self.cur_mb_type(pos.wrapping_sub(1));
            self.avail[5] = t;
            self.avail[9] = t;
        }
        if dist >= mw {
            let t = self.cur_mb_type(pos.wrapping_sub(self.mb_w));
            self.avail[2] = t;
            self.avail[3] = t;
        }
        if x + 1 < mw && dist >= mw - 1 {
            self.avail[4] = self.cur_mb_type(pos.wrapping_sub(self.mb_w) + 1);
        }
        if x != 0 && dist > mw {
            self.avail[1] = self.cur_mb_type(pos.wrapping_sub(self.mb_w + 1));
        }
    }

    fn set_qscale_table(&mut self) {
        let (pos, q) = (self.mb_pos(), self.qscale as u8);
        if let Some(v) = self.cur.as_mut().and_then(|c| c.qscale.get_mut(pos)) {
            *v = q;
        }
    }

    /// Offset of the current macroblock's intra types in `it_hist`.
    fn it_base(&self) -> usize {
        self.it_stride * 4 + self.mb_x * 4 + 4
    }

    fn fill_intra_types(&mut self, t: i8) {
        let base = self.it_base();
        for j in 0..4 {
            for i in 0..4 {
                if let Some(v) = self.it_hist.get_mut(base + j * self.it_stride + i) {
                    *v = t;
                }
            }
        }
    }

    fn decode_intra_mb(&mut self, gb: &mut Gb<'_>) -> Result<()> {
        self.fill_avail();
        self.qscale = self.si.quant;
        let cbp = self.intra_mb_header(gb);
        let pos = self.mb_pos();
        self.store_cbp(pos, &cbp);
        if let Some(d) = self.deblock.get_mut(pos) {
            *d = 0xFFFF;
        }
        self.set_qscale_table();
        let cbp = cbp?;
        if self.is16 {
            self.output_i16x16(gb, cbp);
        } else {
            self.output_intra(gb, cbp);
        }
        Ok(())
    }

    fn store_cbp(&mut self, pos: usize, cbp: &Result<u32>) {
        let v = cbp.as_ref().map_or(u32::MAX, |&c| c);
        if let Some(c) = self.cbp_luma.get_mut(pos) {
            *c = v as u16;
        }
        if let Some(c) = self.cbp_chroma.get_mut(pos) {
            *c = (v >> 16) as u8;
        }
    }

    fn decode_inter_mb(&mut self, gb: &mut Gb<'_>) -> Result<()> {
        self.fill_avail();
        self.qscale = self.si.quant;
        let cbp = self.inter_mb_header(gb);
        let pos = self.mb_pos();
        self.store_cbp(pos, &cbp);
        let coef = self.set_deblock_coef() | u32::from(self.cbp_luma.get(pos).copied().unwrap_or(0));
        if let Some(d) = self.deblock.get_mut(pos) {
            *d = coef as u16;
        }
        self.set_qscale_table();
        let mut cbp = cbp?;
        if is_intra(self.cur_mb_type(pos)) {
            if self.is16 {
                self.output_i16x16(gb, cbp);
            } else {
                self.output_intra(gb, cbp);
            }
            return Ok(());
        }
        let (x0, y0) = (self.mb_x * 16, self.mb_y * 16);
        let q = self.qscale;
        if self.is16 {
            // Only for MB_P_MIX16X16.
            let q_dc = QSCALE_TAB.get(usize::from(self.luma_dc_quant(false, q))).copied().unwrap_or(0);
            let q_ac = QSCALE_TAB.get(q).copied().unwrap_or(0);
            let mut block16 = [0i16; 16];
            if decode_block(&mut block16, gb, self.vlcs(), 3, 0, q_dc, q_dc, q_ac) != 0 {
                dsp::inv_transform(&mut block16);
            } else {
                dsp::inv_transform_dc(&mut block16);
            }
            for j in 0..4 {
                for i in 0..4 {
                    let dc = i32::from(block16[i + j * 4]);
                    let has_ac = if cbp & 1 != 0 {
                        let mut b = self.block;
                        let r = decode_block(&mut b, gb, self.vlcs(), self.luma_vlc, 0, q_ac, q_ac, q_ac);
                        self.block = b;
                        r
                    } else {
                        0
                    };
                    cbp >>= 1;
                    let Some(cur) = self.cur.as_mut() else { return Ok(()) };
                    if has_ac != 0 {
                        self.block[0] = dc as i16;
                        dsp::idct_add(&mut cur.planes[0], x0 + 4 * i, y0 + 4 * j, &mut self.block);
                    } else {
                        dsp::idct_dc_add(&mut cur.planes[0], x0 + 4 * i, y0 + 4 * j, dc);
                    }
                }
            }
            self.cur_vlcs = choose_vlc_set(self.si.quant, self.si.vlc_set, true);
        } else {
            let q_ac = QSCALE_TAB.get(q).copied().unwrap_or(0);
            for j in 0..4 {
                for i in 0..4 {
                    if cbp & 1 != 0 {
                        self.process_block(gb, 0, x0 + 4 * i, y0 + 4 * j, self.luma_vlc, 0, q_ac, q_ac);
                    }
                    cbp >>= 1;
                }
            }
        }
        self.chroma_residual(gb, cbp);
        Ok(())
    }

    fn chroma_residual(&mut self, gb: &mut Gb<'_>, mut cbp: u32) {
        let q = self.qscale;
        let q_dc = QSCALE_TAB.get(usize::from(CHROMA_QUANT.get(32 + q).copied().unwrap_or(0))).copied().unwrap_or(0);
        let q_ac = QSCALE_TAB.get(usize::from(CHROMA_QUANT.get(q).copied().unwrap_or(0))).copied().unwrap_or(0);
        let (x0, y0) = (self.mb_x * 8, self.mb_y * 8);
        for p in 1..3 {
            for i in 0..4 {
                if cbp & 1 != 0 {
                    self.process_block(gb, p, x0 + (i & 1) * 4, y0 + (i & 2) * 2, self.chroma_vlc, 1, q_dc, q_ac);
                }
                cbp >>= 1;
            }
        }
    }

    fn luma_dc_quant(&self, intra: bool, q: usize) -> u8 {
        if self.rv30 { RV30_LUMA_DC_QUANT.get(q).copied().unwrap_or(0) } else { RV40_LUMA_DC_QUANT.get(if intra { q } else { 32 + q }).copied().unwrap_or(0) }
    }

    fn vlcs(&self) -> &'static VlcSet {
        let t = vlcs();
        let set = if self.cur_vlcs.inter { t.inter.get(self.cur_vlcs.idx) } else { t.intra.get(self.cur_vlcs.idx) };
        set.unwrap_or(&t.intra[0])
    }

    /// `rv34_process_block`.
    #[allow(clippy::too_many_arguments)]
    fn process_block(&mut self, gb: &mut Gb<'_>, plane: usize, x: usize, y: usize, fc: usize, sc: usize, q_dc: i32, q_ac: i32) {
        let mut b = self.block;
        let has_ac = decode_block(&mut b, gb, self.vlcs(), fc, sc, q_dc, q_ac, q_ac);
        self.block = b;
        let Some(cur) = self.cur.as_mut() else { return };
        let Some(p) = cur.planes.get_mut(plane) else { return };
        if has_ac != 0 {
            dsp::idct_add(p, x, y, &mut self.block);
        } else {
            dsp::idct_dc_add(p, x, y, i32::from(self.block[0]));
            self.block[0] = 0;
        }
    }

    fn intra_mb_header(&mut self, gb: &mut Gb<'_>) -> Result<u32> {
        self.is16 = gb.get1();
        if self.is16 {
            self.set_cur_mb_type(T_INTRA16X16);
            self.block_type = MB_INTRA16;
            let t = gb.get(2) as i8;
            self.fill_intra_types(t);
            self.luma_vlc = 2;
        } else {
            if !self.rv30 {
                gb.skip(1);
            }
            self.set_cur_mb_type(T_INTRA4X4);
            self.block_type = MB_INTRA;
            self.decode_intra_types(gb)?;
            self.luma_vlc = 1;
        }
        self.chroma_vlc = 0;
        self.cur_vlcs = choose_vlc_set(self.si.quant, self.si.vlc_set, false);
        decode_cbp(gb, self.vlcs(), usize::from(self.is16))
    }

    fn inter_mb_header(&mut self, gb: &mut Gb<'_>) -> Result<u32> {
        let bt = self.decode_mb_info(gb)?;
        self.block_type = bt;
        let pos = self.mb_pos();
        let lavc = TO_LAVC.get(bt).copied().unwrap_or(0);
        self.set_cur_mb_type(lavc);
        let mut internal = bt;
        if bt == MB_SKIP {
            internal = if self.pict_type == PictType::P { MB_P_16X16 } else { MB_B_DIRECT };
        }
        if let Some(t) = self.mb_types.get_mut(pos) {
            *t = internal as u8;
        }
        self.is16 = lavc & T_INTRA16X16 != 0;
        self.decode_mv(gb, bt)?;
        if bt == MB_SKIP {
            self.fill_intra_types(0);
            return Ok(0);
        }
        self.chroma_vlc = 1;
        self.luma_vlc = 0;
        if is_intra(lavc) {
            if self.is16 {
                let t = gb.get(2) as i8;
                self.fill_intra_types(t);
                self.luma_vlc = 2;
            } else {
                self.decode_intra_types(gb)?;
                self.luma_vlc = 1;
            }
            self.chroma_vlc = 0;
            self.cur_vlcs = choose_vlc_set(self.si.quant, self.si.vlc_set, false);
        } else {
            self.fill_intra_types(0);
            self.cur_vlcs = choose_vlc_set(self.si.quant, self.si.vlc_set, true);
            if internal == MB_P_MIX16X16 {
                self.is16 = true;
                self.chroma_vlc = 1;
                self.luma_vlc = 2;
                self.cur_vlcs = choose_vlc_set(self.si.quant, self.si.vlc_set, false);
            }
        }
        decode_cbp(gb, self.vlcs(), usize::from(self.is16))
    }

    fn decode_mb_info(&mut self, gb: &mut Gb<'_>) -> Result<usize> {
        if self.rv30 {
            const P: [usize; 6] = [MB_SKIP, MB_P_16X16, MB_P_8X8, usize::MAX, MB_INTRA, MB_INTRA16];
            const B: [usize; 6] = [MB_SKIP, MB_B_DIRECT, MB_B_FORWARD, MB_B_BACKWARD, MB_INTRA, MB_INTRA16];
            let mut code = ue(gb) as usize;
            if code > 11 {
                return Err(Error::Invalid("macroblock type"));
            }
            if code > 5 {
                code -= 6;
            }
            let t = if self.pict_type != PictType::B { P[code] } else { B[code] };
            if t == usize::MAX {
                return Err(Error::Invalid("macroblock type"));
            }
            return Ok(t);
        }
        if self.mb_skip_run == 0 {
            let r = ue(gb).saturating_add(1);
            if r > (self.mb_w * self.mb_h) as u32 {
                return Err(Error::Invalid("skip run"));
            }
            self.mb_skip_run = r;
        }
        self.mb_skip_run -= 1;
        if self.mb_skip_run != 0 {
            return Ok(MB_SKIP);
        }
        let pos = self.mb_pos();
        let mut prev = 0usize;
        let mt = |p: usize| self.mb_types.get(p).copied().map_or(0, usize::from);
        if self.avail[2] != 0 {
            let mut blocks = [0u32; 12];
            let mut count = 0;
            if self.avail[5] != 0 {
                blocks[mt(pos.wrapping_sub(1)).min(11)] += 1;
            }
            blocks[mt(pos.wrapping_sub(self.mb_w)).min(11)] += 1;
            if self.avail[4] != 0 {
                blocks[mt(pos.wrapping_sub(self.mb_w) + 1).min(11)] += 1;
            }
            if self.avail[1] != 0 {
                blocks[mt(pos.wrapping_sub(self.mb_w + 1)).min(11)] += 1;
            }
            for (i, &b) in blocks.iter().enumerate() {
                if b > count {
                    count = b;
                    prev = i;
                    if count > 1 {
                        break;
                    }
                }
            }
        } else if self.avail[5] != 0 {
            prev = mt(pos.wrapping_sub(1));
        }
        let t = vlcs();
        let q = if self.pict_type == PictType::P {
            let v = t.ptype.get(usize::from(BLOCK_NUM_TO_PTYPE.get(prev).copied().unwrap_or(0))).ok_or(Error::Invalid("ptype"))?;
            let q = v.read(gb).ok_or(Error::Invalid("ptype"))?;
            if q < 0xFF {
                return Ok(usize::from(q));
            }
            v.read(gb);
            0
        } else {
            let v = t.btype.get(usize::from(BLOCK_NUM_TO_BTYPE.get(prev).copied().unwrap_or(0))).ok_or(Error::Invalid("btype"))?;
            let q = v.read(gb).ok_or(Error::Invalid("btype"))?;
            if q < 0xFF {
                return Ok(usize::from(q));
            }
            v.read(gb);
            0
        };
        Ok(q)
    }

    fn decode_intra_types(&mut self, gb: &mut Gb<'_>) -> Result<()> {
        let s = self.it_stride as isize;
        let base = self.it_base() as isize;
        let get = |h: &[i8], k: isize| -> i32 { usize::try_from(k).ok().and_then(|k| h.get(k)).map_or(-1, |&v| i32::from(v)) };
        if self.rv30 {
            for i in 0..4isize {
                let row = base + i * s;
                for j in (0..4isize).step_by(2) {
                    let code = ue(gb).saturating_mul(2);
                    if code > 80 * 2 {
                        return Err(Error::Invalid("intra prediction code"));
                    }
                    for k in 0..2isize {
                        let at = row + j + k;
                        let a = get(&self.it_hist, at - s) + 1;
                        let b = get(&self.it_hist, at - 1) + 1;
                        let c = i32::from(RV30_ITYPE_CODE.get(code as usize + k as usize).copied().unwrap_or(0));
                        let v = RV30_ITYPE_FROM_CONTEXT.get((a * 90 + b * 9 + c).max(0) as usize).copied().unwrap_or(9);
                        if let Some(d) = self.it_hist.get_mut(at as usize) {
                            *d = v;
                        }
                        if v == 9 {
                            return Err(Error::Invalid("intra prediction mode"));
                        }
                    }
                }
            }
            return Ok(());
        }
        let t = vlcs();
        for i in 0..4isize {
            let row = base + i * s;
            if i == 0 && self.first_slice_line {
                let pattern = i32::from(t.aic_top.read(gb).ok_or(Error::Invalid("intra types"))?);
                let v = [(pattern >> 2) & 2, (pattern >> 1) & 2, pattern & 2, (pattern << 1) & 2];
                for (k, &v) in v.iter().enumerate() {
                    if let Some(d) = self.it_hist.get_mut(row as usize + k) {
                        *d = v as i8;
                    }
                }
                continue;
            }
            let mut j = 0isize;
            while j < 4 {
                let at = row + j;
                let a = get(&self.it_hist, at - s + 1);
                let b = get(&self.it_hist, at - s);
                let c = get(&self.it_hist, at - 1);
                let pattern = a + b * 16 + c * 256;
                let k = RV40_AIC_TABLE_INDEX.iter().position(|&p| p == pattern);
                if let (true, Some(k)) = (j < 3, k) {
                    let v = t.aic_mode2.get(k).and_then(|v| v.read(gb)).ok_or(Error::Invalid("intra types"))?;
                    if let Some(d) = self.it_hist.get_mut(at as usize) {
                        *d = (v >> 4) as i8;
                    }
                    if let Some(d) = self.it_hist.get_mut(at as usize + 1) {
                        *d = (v & 0xF) as i8;
                    }
                    j += 2;
                    continue;
                }
                let v = if b != -1 && c != -1 {
                    let idx = (b + c * 10).max(0) as usize;
                    let vlc = t.aic_mode1.get(idx).and_then(|v| v.as_ref()).ok_or(Error::Invalid("intra types"))?;
                    i32::from(vlc.read(gb).ok_or(Error::Invalid("intra types"))?)
                } else {
                    match c {
                        -1 => {
                            if b < 2 {
                                i32::from(!gb.get1())
                            } else {
                                0
                            }
                        }
                        0 | 2 => i32::from(!gb.get1()) << 1,
                        _ => 0,
                    }
                };
                if let Some(d) = self.it_hist.get_mut(at as usize) {
                    *d = v as i8;
                }
                j += 1;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Motion vectors

    fn mv_index(&self, pos: isize) -> usize {
        (pos + self.b8 + 1).max(0) as usize
    }

    fn mv_get(&self, dir: usize, pos: isize) -> [i32; 2] {
        let i = self.mv_index(pos);
        let v = self.cur.as_ref().and_then(|c| c.mv.get(dir)).and_then(|m| m.get(i)).copied().unwrap_or([0; 2]);
        [i32::from(v[0]), i32::from(v[1])]
    }

    fn mv_set(&mut self, dir: usize, pos: isize, v: [i32; 2]) {
        let i = self.mv_index(pos);
        if let Some(m) = self.cur.as_mut().and_then(|c| c.mv.get_mut(dir)).and_then(|m| m.get_mut(i)) {
            *m = [v[0] as i16, v[1] as i16];
        }
    }

    fn zero_8x2(&mut self, dir: usize, pos: isize) {
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            self.mv_set(dir, pos + dx + dy * self.b8, [0, 0]);
        }
    }

    fn mv_base(&self) -> isize {
        (self.mb_x * 2) as isize + (self.mb_y * 2) as isize * self.b8
    }

    /// `rv34_pred_mv`.
    fn pred_mv(&mut self, block_type: usize, sub: usize, dmv_no: usize) {
        let mut mv_pos = self.mv_base();
        let av = AVAIL_INDEXES[sub] as isize;
        let avail = |k: isize| self.avail.get((av + k) as usize).copied().unwrap_or(0) != 0;
        let mut c_off = PART_W[block_type] as isize;
        mv_pos += (sub & 1) as isize + (sub >> 1) as isize * self.b8;
        if sub == 3 {
            c_off = -1;
        }
        let a = if avail(-1) { self.mv_get(0, mv_pos - 1) } else { [0, 0] };
        let b = if avail(-4) { self.mv_get(0, mv_pos - self.b8) } else { a };
        let c = if !avail(c_off - 4) {
            if avail(-4) && (avail(-1) || self.rv30) { self.mv_get(0, mv_pos - self.b8 - 1) } else { a }
        } else {
            self.mv_get(0, mv_pos - self.b8 + c_off)
        };
        let dmv = self.dmv[dmv_no];
        let mx = mid_pred(a[0], b[0], c[0]).wrapping_add(dmv[0]);
        let my = mid_pred(a[1], b[1], c[1]).wrapping_add(dmv[1]);
        for j in 0..PART_H[block_type] {
            for i in 0..PART_W[block_type] {
                self.mv_set(0, mv_pos + i as isize + j as isize * self.b8, [mx, my]);
            }
        }
    }

    fn calc_add_mv(&self, dir: usize, val: i32) -> i32 {
        let mul = if dir != 0 { -self.mv_weight2 } else { self.mv_weight1 };
        ((val as u32).wrapping_mul(mul as u32) as i32).wrapping_add(0x2000) >> 14
    }

    /// `rv34_pred_mv_b`.
    fn pred_mv_b(&mut self, block_type: usize, dir: usize) {
        let pos = self.mb_pos();
        let mv_pos = self.mv_base();
        let ty = self.cur_mb_type(pos);
        let mask = if dir != 0 { T_BACKWARD } else { T_FORWARD };
        let (mut a, mut b, mut c) = ([0; 2], [0; 2], [0; 2]);
        let (mut ha, mut hb, mut hc) = (false, false, false);
        if self.avail[5] & ty & mask != 0 {
            a = self.mv_get(dir, mv_pos - 1);
            ha = true;
        }
        if self.avail[2] & ty & mask != 0 {
            b = self.mv_get(dir, mv_pos - self.b8);
            hb = true;
        }
        if self.avail[2] != 0 && self.avail[4] & ty & mask != 0 {
            c = self.mv_get(dir, mv_pos - self.b8 + 2);
            hc = true;
        } else if self.mb_x + 1 == self.mb_w && self.avail[1] & ty & mask != 0 {
            c = self.mv_get(dir, mv_pos - self.b8 - 1);
            hc = true;
        }
        let n = u32::from(ha) + u32::from(hb) + u32::from(hc);
        let (mut mx, mut my) = if n != 3 {
            let (mut x, mut y) = (a[0] + b[0] + c[0], a[1] + b[1] + c[1]);
            if n == 2 {
                x /= 2;
                y /= 2;
            }
            (x, y)
        } else {
            (mid_pred(a[0], b[0], c[0]), mid_pred(a[1], b[1], c[1]))
        };
        mx = mx.wrapping_add(self.dmv[dir][0]);
        my = my.wrapping_add(self.dmv[dir][1]);
        for j in 0..2 {
            for i in 0..2 {
                self.mv_set(dir, mv_pos + i + j * self.b8, [mx, my]);
            }
        }
        if block_type == MB_B_BACKWARD || block_type == MB_B_FORWARD {
            self.zero_8x2(1 - dir, mv_pos);
        }
    }

    /// `rv34_pred_mv_rv3`.
    fn pred_mv_rv3(&mut self) {
        let mv_pos = self.mv_base();
        let avail = |k: isize| self.avail.get((6 + k) as usize).copied().unwrap_or(0) != 0;
        let a = if avail(-1) { self.mv_get(0, mv_pos - 1) } else { [0, 0] };
        let b = if avail(-4) { self.mv_get(0, mv_pos - self.b8) } else { a };
        let c = if !avail(-2) { if avail(-4) && avail(-1) { self.mv_get(0, mv_pos - self.b8 - 1) } else { a } } else { self.mv_get(0, mv_pos - self.b8 + 2) };
        let mx = mid_pred(a[0], b[0], c[0]).wrapping_add(self.dmv[0][0]);
        let my = mid_pred(a[1], b[1], c[1]).wrapping_add(self.dmv[0][1]);
        for j in 0..2 {
            for i in 0..2 {
                for k in 0..2 {
                    self.mv_set(k, mv_pos + i + j * self.b8, [mx, my]);
                }
            }
        }
    }

    /// `rv34_decode_mv`: motion vector differences, reconstruction and motion compensation.
    fn decode_mv(&mut self, gb: &mut Gb<'_>, block_type: usize) -> Result<()> {
        let mv_pos = self.mv_base();
        self.dmv = [[0; 2]; 4];
        for i in 0..NUM_MVS[block_type] {
            let (Some(x), Some(y)) = (se(gb), se(gb)) else {
                self.dmv[i] = [0, 0];
                return Err(Error::Invalid("motion vector"));
            };
            self.dmv[i] = [x, y];
        }
        match block_type {
            MB_INTRA | MB_INTRA16 => {
                self.zero_8x2(0, mv_pos);
            }
            MB_SKIP if self.pict_type == PictType::P => {
                self.zero_8x2(0, mv_pos);
                self.mc_1mv(block_type, 0, 0, 0, 2, 2, 0);
            }
            MB_SKIP | MB_B_DIRECT => {
                let pos = self.mb_pos();
                let next_bt = self.next.as_ref().and_then(|n| n.mb_type.get(pos).copied()).unwrap_or(0);
                if is_intra(next_bt) || next_bt & T_SKIP != 0 {
                    self.zero_8x2(0, mv_pos);
                    self.zero_8x2(1, mv_pos);
                } else {
                    for j in 0..2isize {
                        for i in 0..2isize {
                            let p = mv_pos + i + j * self.b8;
                            let nv = self.next.as_ref().and_then(|n| n.mv[0].get(self.mv_index(p)).copied()).unwrap_or([0; 2]);
                            for l in 0..2 {
                                let v = [self.calc_add_mv(l, i32::from(nv[0])), self.calc_add_mv(l, i32::from(nv[1]))];
                                self.mv_set(l, p, v);
                            }
                        }
                    }
                }
                if next_bt & (T_16X8 | T_8X16 | T_8X8) == 0 {
                    self.mc_2mv(block_type);
                } else {
                    self.mc_2mv_skip();
                }
                self.zero_8x2(0, mv_pos);
            }
            MB_P_16X16 | MB_P_MIX16X16 => {
                self.pred_mv(block_type, 0, 0);
                self.mc_1mv(block_type, 0, 0, 0, 2, 2, 0);
            }
            MB_B_FORWARD | MB_B_BACKWARD => {
                self.dmv[1] = self.dmv[0];
                let dir = usize::from(block_type == MB_B_BACKWARD);
                if self.rv30 {
                    self.pred_mv_rv3();
                } else {
                    self.pred_mv_b(block_type, dir);
                }
                self.mc_1mv(block_type, 0, 0, 0, 2, 2, dir);
            }
            MB_P_16X8 | MB_P_8X16 => {
                self.pred_mv(block_type, 0, 0);
                self.pred_mv(block_type, 1 + usize::from(block_type == MB_P_16X8), 1);
                if block_type == MB_P_16X8 {
                    self.mc_1mv(block_type, 0, 0, 0, 2, 1, 0);
                    self.mc_1mv(block_type, 0, 8, self.b8, 2, 1, 0);
                } else {
                    self.mc_1mv(block_type, 0, 0, 0, 1, 2, 0);
                    self.mc_1mv(block_type, 8, 0, 1, 1, 2, 0);
                }
            }
            MB_B_BIDIR => {
                self.pred_mv_b(block_type, 0);
                self.pred_mv_b(block_type, 1);
                self.mc_2mv(block_type);
            }
            MB_P_8X8 => {
                for i in 0..4usize {
                    self.pred_mv(block_type, i, i);
                    let off = (i & 1) as isize + (i >> 1) as isize * self.b8;
                    self.mc_1mv(block_type, (i & 1) << 3, (i & 2) << 2, off, 1, 1, 0);
                }
            }
            _ => {}
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Motion compensation

    /// `rv34_mc`: the `width`×`height` (in 8×8 blocks) partition at (`xoff`, `yoff`) from the
    /// previous (`dir` 0) or next reference into the picture (or, `weighted`, into `tmp[dir]`).
    #[allow(clippy::too_many_arguments)]
    fn mc(&mut self, xoff: usize, yoff: usize, mv_off: isize, width: usize, height: usize, dir: usize, weighted: bool, avg: bool) {
        let mv = self.mv_get(dir, self.mv_base() + mv_off);
        let (mx, my, lx, ly, umx, umy, uvmx, uvmy);
        if self.rv30 {
            let d3 = |v: i32| (v + (3 << 24)) / 3 - (1 << 24);
            let m3 = |v: i32| (v + (3 << 24)) % 3;
            mx = d3(mv[0]);
            my = d3(mv[1]);
            lx = m3(mv[0]);
            ly = m3(mv[1]);
            let (cmx, cmy) = (mv[0] / 2, mv[1] / 2);
            umx = d3(cmx);
            umy = d3(cmy);
            uvmx = CHROMA_COEFFS[m3(cmx) as usize];
            uvmy = CHROMA_COEFFS[m3(cmy) as usize];
        } else {
            mx = mv[0] >> 2;
            my = mv[1] >> 2;
            lx = mv[0] & 3;
            ly = mv[1] & 3;
            let (cx, cy) = (mv[0] / 2, mv[1] / 2);
            umx = cx >> 2;
            umy = cy >> 2;
            let (mut ux, mut uy) = ((cx & 3) << 1, (cy & 3) << 1);
            if ux == 6 && uy == 6 {
                ux = 4;
                uy = 4;
            }
            uvmx = ux;
            uvmy = uy;
        }
        let Some(src) = (if dir != 0 { self.next.clone() } else { self.last.clone() }) else { return };
        let sx = (self.mb_x * 16 + xoff) as isize + mx as isize;
        let sy = (self.mb_y * 16 + yoff) as isize + my as isize;
        let usx = (self.mb_x * 8 + (xoff >> 1)) as isize + umx as isize;
        let usy = (self.mb_y * 8 + (yoff >> 1)) as isize + umy as isize;
        let (bw, bh) = (width * 8, height * 8);
        let rv30 = self.rv30;
        if weighted {
            let t = &mut self.tmp[dir];
            dsp::luma_mc(rv30, &mut t[0], xoff, yoff, bw, bh, &src.planes[0], sx, sy, lx, ly, false);
            for p in 1..3 {
                dsp::chroma_mc(rv30, &mut t[p], xoff >> 1, yoff >> 1, bw / 2, bh / 2, &src.planes[p], usx, usy, uvmx, uvmy, false);
            }
        } else {
            let (dx, dy) = (self.mb_x * 16 + xoff, self.mb_y * 16 + yoff);
            let Some(cur) = self.cur.as_mut() else { return };
            dsp::luma_mc(rv30, &mut cur.planes[0], dx, dy, bw, bh, &src.planes[0], sx, sy, lx, ly, avg);
            for p in 1..3 {
                dsp::chroma_mc(rv30, &mut cur.planes[p], dx >> 1, dy >> 1, bw / 2, bh / 2, &src.planes[p], usx, usy, uvmx, uvmy, avg);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mc_1mv(&mut self, _block_type: usize, xoff: usize, yoff: usize, mv_off: isize, width: usize, height: usize, dir: usize) {
        self.mc(xoff, yoff, mv_off, width, height, dir, false, false);
    }

    fn weight(&mut self) {
        let (x, y) = (self.mb_x * 16, self.mb_y * 16);
        let (w1, w2, sc) = (self.weight1, self.weight2, self.scaled_weight);
        let Some(cur) = self.cur.as_mut() else { return };
        let [t0, t1] = &self.tmp;
        dsp::weight(&mut cur.planes[0], x, y, 16, &t0[0], &t1[0], w1, w2, sc);
        for p in 1..3 {
            dsp::weight(&mut cur.planes[p], x / 2, y / 2, 8, &t0[p], &t1[p], w1, w2, sc);
        }
    }

    fn mc_2mv(&mut self, block_type: usize) {
        let weighted = !self.rv30 && block_type != MB_B_BIDIR && self.weight1 != 8192;
        self.mc(0, 0, 0, 2, 2, 0, weighted, false);
        if !weighted {
            self.mc(0, 0, 0, 2, 2, 1, false, true);
        } else {
            self.mc(0, 0, 0, 2, 2, 1, true, false);
            self.weight();
        }
    }

    fn mc_2mv_skip(&mut self) {
        let weighted = !self.rv30 && self.weight1 != 8192;
        for j in 0..2usize {
            for i in 0..2usize {
                let off = i as isize + j as isize * self.b8;
                self.mc(i * 8, j * 8, off, 1, 1, 0, weighted, false);
                self.mc(i * 8, j * 8, off, 1, 1, 1, weighted, !weighted);
            }
        }
        if weighted {
            self.weight();
        }
    }

    // -----------------------------------------------------------------------------------------
    // Intra reconstruction

    fn intra_type(&self, i: usize, j: usize) -> i32 {
        let v = self.it_hist.get(self.it_base() + j * self.it_stride + i).copied().unwrap_or(0);
        ITTRANS.get(usize::try_from(v).unwrap_or(0)).copied().unwrap_or(pred::DC)
    }

    /// `rv34_pred_4x4_block` on `plane` at (`x`, `y`).
    #[allow(clippy::too_many_arguments)]
    fn pred_4x4(&mut self, plane: usize, x: usize, y: usize, mut itype: i32, up: bool, left: bool, down: bool, right: bool) {
        if !up && !left {
            itype = pred::DC_128;
        } else if !up {
            if itype == pred::VERT {
                itype = pred::HOR;
            }
            if itype == pred::DC {
                itype = pred::LEFT_DC;
            }
        } else if !left {
            if itype == pred::HOR {
                itype = pred::VERT;
            }
            if itype == pred::DC {
                itype = pred::TOP_DC;
            }
            if itype == pred::DIAG_DOWN_LEFT {
                itype = pred::DIAG_DOWN_LEFT_NODOWN;
            }
        }
        if !down {
            if itype == pred::DIAG_DOWN_LEFT {
                itype = pred::DIAG_DOWN_LEFT_NODOWN;
            }
            if itype == pred::HOR_UP {
                itype = pred::HOR_UP_NODOWN;
            }
            if itype == pred::VERT_LEFT {
                itype = pred::VERT_LEFT_NODOWN;
            }
        }
        let Some(p) = self.cur.as_mut().and_then(|c| c.planes.get_mut(plane)) else { return };
        let (xi, yi) = (x as isize, y as isize);
        let tr = if !right && up { [p.at(xi + 3, yi - 1); 4] } else { std::array::from_fn(|k| p.at(xi + 4 + k as isize, yi - 1)) };
        pred::pred4x4(p, x, y, itype, tr);
    }

    fn output_i16x16(&mut self, gb: &mut Gb<'_>, mut cbp: u32) {
        let q = self.qscale;
        let mut q_dc = QSCALE_TAB.get(usize::from(self.luma_dc_quant(true, q))).copied().unwrap_or(0);
        let mut q_ac = QSCALE_TAB.get(q).copied().unwrap_or(0);
        let (x0, y0) = (self.mb_x * 16, self.mb_y * 16);
        let mut block16 = [0i16; 16];
        if decode_block(&mut block16, gb, self.vlcs(), 3, 0, q_dc, q_dc, q_ac) != 0 {
            dsp::inv_transform(&mut block16);
        } else {
            dsp::inv_transform_dc(&mut block16);
        }
        let it0 = self.it_hist.get(self.it_base()).copied().unwrap_or(0);
        let mut itype = ITTRANS16.get(usize::try_from(it0).unwrap_or(0)).copied().unwrap_or(pred::DC8);
        let (up, left) = (self.avail[2] != 0, self.avail[5] != 0);
        let luma = adjust_pred16(itype, up, left);
        if let Some(c) = self.cur.as_mut() {
            pred::pred_block(&mut c.planes[0], x0, y0, 16, luma);
        }
        for j in 0..4 {
            for i in 0..4 {
                let dc = i32::from(block16[i + j * 4]);
                let has_ac = if cbp & 1 != 0 {
                    let mut b = self.block;
                    let r = decode_block(&mut b, gb, self.vlcs(), self.luma_vlc, 0, q_ac, q_ac, q_ac);
                    self.block = b;
                    r
                } else {
                    0
                };
                cbp >>= 1;
                let Some(cur) = self.cur.as_mut() else { return };
                if has_ac != 0 {
                    self.block[0] = dc as i16;
                    dsp::idct_add(&mut cur.planes[0], x0 + 4 * i, y0 + 4 * j, &mut self.block);
                } else {
                    dsp::idct_dc_add(&mut cur.planes[0], x0 + 4 * i, y0 + 4 * j, dc);
                }
            }
        }
        if itype == pred::PLANE8 {
            itype = pred::DC8;
        }
        let chroma = adjust_pred16(itype, up, left);
        q_dc = QSCALE_TAB.get(usize::from(CHROMA_QUANT.get(32 + q).copied().unwrap_or(0))).copied().unwrap_or(0);
        q_ac = QSCALE_TAB.get(usize::from(CHROMA_QUANT.get(q).copied().unwrap_or(0))).copied().unwrap_or(0);
        let (cx, cy) = (self.mb_x * 8, self.mb_y * 8);
        for p in 1..3 {
            if let Some(c) = self.cur.as_mut() {
                pred::pred_block(&mut c.planes[p], cx, cy, 8, chroma);
            }
            for i in 0..4 {
                if cbp & 1 != 0 {
                    self.process_block(gb, p, cx + (i & 1) * 4, cy + (i & 2) * 2, self.chroma_vlc, 1, q_dc, q_ac);
                }
                cbp >>= 1;
            }
        }
    }

    fn output_intra(&mut self, gb: &mut Gb<'_>, mut cbp: u32) {
        let mut avail = [false; 48];
        if self.avail[1] != 0 {
            avail[0] = true;
        }
        if self.avail[2] != 0 {
            avail[1] = true;
            avail[2] = true;
        }
        if self.avail[3] != 0 {
            avail[3] = true;
            avail[4] = true;
        }
        if self.avail[4] != 0 {
            avail[5] = true;
        }
        if self.avail[5] != 0 {
            avail[8] = true;
            avail[16] = true;
        }
        if self.avail[9] != 0 {
            avail[24] = true;
            avail[32] = true;
        }
        let q = self.qscale;
        let q_ac = QSCALE_TAB.get(q).copied().unwrap_or(0);
        let (x0, y0) = (self.mb_x * 16, self.mb_y * 16);
        for j in 0..4 {
            for (i, idx) in (0..4).zip(9 + j * 8..) {
                let it = self.intra_type(i, j);
                self.pred_4x4(0, x0 + 4 * i, y0 + 4 * j, it, avail[idx - 8], avail[idx - 1], avail[idx + 7], avail[idx - 7]);
                avail[idx] = true;
                if cbp & 1 != 0 {
                    self.process_block(gb, 0, x0 + 4 * i, y0 + 4 * j, self.luma_vlc, 0, q_ac, q_ac);
                }
                cbp >>= 1;
            }
        }
        let q_dc = QSCALE_TAB.get(usize::from(CHROMA_QUANT.get(32 + q).copied().unwrap_or(0))).copied().unwrap_or(0);
        let q_ac = QSCALE_TAB.get(usize::from(CHROMA_QUANT.get(q).copied().unwrap_or(0))).copied().unwrap_or(0);
        let (cx, cy) = (self.mb_x * 8, self.mb_y * 8);
        for p in 1..3 {
            for k in [6, 7, 10, 11] {
                self.avail[k] = 0;
            }
            for j in 0..2 {
                for i in 0..2 {
                    let ac = 6 + j * 4 + i;
                    let it = self.intra_type(i * 2, j * 2);
                    let a = |k: isize| self.avail.get((ac as isize + k) as usize).copied().unwrap_or(0) != 0;
                    let (up, left, right) = (a(-4), a(-1), a(-3));
                    self.pred_4x4(p, cx + 4 * i, cy + 4 * j, it, up, left, i == 0 && j == 0, right);
                    self.avail[ac] = 1;
                    if cbp & 1 != 0 {
                        self.process_block(gb, p, cx + 4 * i, cy + 4 * j, self.chroma_vlc, 1, q_dc, q_ac);
                    }
                    cbp >>= 1;
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Loop filters

    /// `rv34_set_deblock_coef`.
    fn set_deblock_coef(&mut self) -> u32 {
        let mut hmv = 0u32;
        let mut vmv = 0u32;
        let base = self.mv_base();
        let gt3 = |a: [i32; 2], b: [i32; 2]| (a[0] - b[0]).abs() > 3 || (a[1] - b[1]).abs() > 3;
        for (row, j) in [0u32, 8].into_iter().enumerate() {
            let mv = base + row as isize * self.b8;
            for i in 0..2isize {
                let cur = self.mv_get(0, mv + i);
                if gt3(cur, self.mv_get(0, mv + i - 1)) {
                    vmv |= 0x11 << (j + i as u32 * 2);
                }
                if (j != 0 || self.mb_y != 0) && gt3(cur, self.mv_get(0, mv + i - self.b8)) {
                    hmv |= 0x03 << (j + i as u32 * 2);
                }
            }
        }
        if self.first_slice_line {
            hmv &= !0x000F;
        }
        if self.mb_x == 0 {
            vmv &= !0x1111;
        }
        if self.rv30 {
            vmv |= (vmv & 0x4444) >> 1;
            hmv |= (hmv & 0x0F00) >> 4;
            let pos = self.mb_pos();
            if self.mb_x != 0
                && let Some(d) = self.deblock.get_mut(pos - 1)
            {
                *d |= ((vmv & 0x1111) << 3) as u16;
            }
            if !self.first_slice_line
                && let Some(d) = pos.checked_sub(self.mb_w).and_then(|p| self.deblock.get_mut(p))
            {
                *d |= ((hmv & 0xF) << 12) as u16;
            }
        }
        hmv | vmv
    }

    fn rv30_loop_filter(&mut self, row: usize) {
        let mw = self.mb_w;
        let Some(cur) = self.cur.as_mut() else { return };
        for x in 0..mw {
            let pos = row * mw + x;
            let t = cur.mb_type.get(pos).copied().unwrap_or(0);
            if (is_intra(t) || t & T_SEPARATE_DC != 0)
                && let Some(d) = self.deblock.get_mut(pos)
            {
                *d = 0xFFFF;
            }
            if is_intra(t)
                && let Some(c) = self.cbp_chroma.get_mut(pos)
            {
                *c = 0xFF;
            }
        }
        let lim = |q: u8| RV30_LOOP_FILT_LIM.get(usize::from(q)).copied().unwrap_or(0);
        let db = |p: usize| u32::from(self.deblock.get(p).copied().unwrap_or(0));
        let cc = |p: usize| u32::from(self.cbp_chroma.get(p).copied().unwrap_or(0));
        let mut left_lim = 0;
        for x in 0..mw {
            let pos = row * mw + x;
            let cur_lim = lim(cur.qscale.get(pos).copied().unwrap_or(0));
            if x != 0 {
                left_lim = lim(cur.qscale.get(pos - 1).copied().unwrap_or(0));
            }
            for j in (0..16).step_by(4) {
                for i in usize::from(x == 0)..4 {
                    let ij = i + j;
                    let l = if db(pos) & (1 << ij) != 0 {
                        cur_lim
                    } else if i == 0 && db(pos.wrapping_sub(1)) & (1 << (ij + 3)) != 0 {
                        left_lim
                    } else if i != 0 && db(pos) & (1 << (ij - 1)) != 0 {
                        cur_lim
                    } else {
                        0
                    };
                    if l != 0 {
                        dsp::rv30_filter(&mut cur.planes[0], x * 16 + i * 4, row * 16 + j, true, l);
                    }
                }
            }
            for k in 0..2 {
                let cur_cbp = (cc(pos) >> (k * 4)) & 0xF;
                let left_cbp = if x != 0 { (cc(pos - 1) >> (k * 4)) & 0xF } else { 0 };
                for j in (0..8).step_by(4) {
                    for i in usize::from(x == 0)..2 {
                        let ij = i + (j >> 1);
                        let l = if cur_cbp & (1 << ij) != 0 {
                            cur_lim
                        } else if i == 0 && left_cbp & (1 << (ij + 1)) != 0 {
                            left_lim
                        } else if i != 0 && cur_cbp & (1 << (ij - 1)) != 0 {
                            cur_lim
                        } else {
                            0
                        };
                        if l != 0 {
                            dsp::rv30_filter(&mut cur.planes[k + 1], x * 8 + i * 4, row * 8 + j, true, l);
                        }
                    }
                }
            }
        }
        let mut top_lim = 0;
        for x in 0..mw {
            let pos = row * mw + x;
            let cur_lim = lim(cur.qscale.get(pos).copied().unwrap_or(0));
            if row != 0 {
                top_lim = lim(cur.qscale.get(pos - mw).copied().unwrap_or(0));
            }
            for j in ((if row == 0 { 4 } else { 0 })..16).step_by(4) {
                for i in 0..4 {
                    let ij = i + j;
                    let l = if db(pos) & (1 << ij) != 0 {
                        cur_lim
                    } else if j == 0 && db(pos.wrapping_sub(mw)) & (1 << (ij + 12)) != 0 {
                        top_lim
                    } else if j != 0 && db(pos) & (1 << (ij - 4)) != 0 {
                        cur_lim
                    } else {
                        0
                    };
                    if l != 0 {
                        dsp::rv30_filter(&mut cur.planes[0], x * 16 + i * 4, row * 16 + j, false, l);
                    }
                }
            }
            for k in 0..2 {
                let cur_cbp = (cc(pos) >> (k * 4)) & 0xF;
                let top_cbp = if row != 0 { (cc(pos - mw) >> (k * 4)) & 0xF } else { 0 };
                for j in ((if row == 0 { 4 } else { 0 })..8).step_by(4) {
                    for i in 0..2 {
                        let ij = i + (j >> 1);
                        let l = if cc(pos) & (1 << ij) != 0 {
                            cur_lim
                        } else if j == 0 && top_cbp & (1 << (ij + 2)) != 0 {
                            top_lim
                        } else if j != 0 && cur_cbp & (1 << (ij - 2)) != 0 {
                            cur_lim
                        } else {
                            0
                        };
                        if l != 0 {
                            dsp::rv30_filter(&mut cur.planes[k + 1], x * 8 + i * 4, row * 8 + j, false, l);
                        }
                    }
                }
            }
        }
    }

    fn rv40_loop_filter(&mut self, row: usize) {
        const MASK_CUR: u32 = 0x0001;
        const MASK_RIGHT: u32 = 0x0008;
        const MASK_BOTTOM: u32 = 0x0010;
        const MASK_TOP: u32 = 0x1000;
        const MASK_Y_TOP_ROW: u32 = 0x000F;
        const MASK_Y_LAST_ROW: u32 = 0xF000;
        const MASK_Y_LEFT_COL: u32 = 0x1111;
        const MASK_Y_RIGHT_COL: u32 = 0x8888;
        const MASK_C_TOP_ROW: u32 = 0x0003;
        const MASK_C_LAST_ROW: u32 = 0x000C;
        const MASK_C_LEFT_COL: u32 = 0x0005;
        const MASK_C_RIGHT_COL: u32 = 0x000A;
        const POS_CUR: usize = 0;
        const POS_TOP: usize = 1;
        const POS_LEFT: usize = 2;
        const POS_BOTTOM: usize = 3;
        let (mw, mh) = (self.mb_w, self.mb_h);
        let small = u64::from(self.width) * u64::from(self.height) <= 176 * 144;
        let Some(cur) = self.cur.as_mut() else { return };
        for x in 0..mw {
            let pos = row * mw + x;
            let t = cur.mb_type.get(pos).copied().unwrap_or(0);
            if is_intra(t) || t & T_SEPARATE_DC != 0 {
                if let Some(d) = self.cbp_luma.get_mut(pos) {
                    *d = 0xFFFF;
                }
                if let Some(d) = self.deblock.get_mut(pos) {
                    *d = 0xFFFF;
                }
            }
            if is_intra(t)
                && let Some(c) = self.cbp_chroma.get_mut(pos)
            {
                *c = 0xFF;
            }
        }
        for mb_x in 0..mw {
            let pos = row * mw + mb_x;
            let q = usize::from(cur.qscale.get(pos).copied().unwrap_or(0)) & 31;
            let alpha = RV40_ALPHA_TAB[q];
            let beta = RV40_BETA_TAB[q];
            let beta_c = beta * 3;
            let beta_y = if small { beta_c + beta } else { beta_c };
            let avail = [true, row != 0, mb_x != 0, row + 1 < mh];
            let offs: [isize; 4] = [0, -(mw as isize), -1, mw as isize];
            let mut mvmasks = [0u32; 4];
            let mut mbtype = [0u32; 4];
            let mut cbp = [0u32; 4];
            let mut uvcbp = [[0u32; 2]; 4];
            let mut strong = [false; 4];
            let mut clip = [0i32; 4];
            for i in 0..4 {
                if avail[i] {
                    let p = (pos as isize + offs[i]) as usize;
                    mvmasks[i] = u32::from(self.deblock.get(p).copied().unwrap_or(0));
                    mbtype[i] = cur.mb_type.get(p).copied().unwrap_or(0);
                    cbp[i] = u32::from(self.cbp_luma.get(p).copied().unwrap_or(0));
                    let c = u32::from(self.cbp_chroma.get(p).copied().unwrap_or(0));
                    uvcbp[i] = [c & 0xF, c >> 4];
                } else {
                    mbtype[i] = mbtype[0];
                }
                strong[i] = is_intra(mbtype[i]) || mbtype[i] & T_SEPARATE_DC != 0;
                clip[i] = RV40_FILTER_CLIP_TBL[(usize::from(strong[i]) + 1) * 32 + q];
            }
            let y_to_deblock = mvmasks[POS_CUR] | (mvmasks[POS_BOTTOM] << 16);
            let mut y_h = y_to_deblock | ((cbp[POS_CUR] << 4) & !MASK_Y_TOP_ROW) | ((cbp[POS_TOP] & MASK_Y_LAST_ROW) >> 12);
            let mut y_v = y_to_deblock | ((cbp[POS_CUR] << 1) & !MASK_Y_LEFT_COL) | ((cbp[POS_LEFT] & MASK_Y_RIGHT_COL) >> 3);
            if mb_x == 0 {
                y_v &= !MASK_Y_LEFT_COL;
            }
            if row == 0 {
                y_h &= !MASK_Y_TOP_ROW;
            }
            if row == mh - 1 || strong[POS_CUR] || strong[POS_BOTTOM] {
                y_h &= !(MASK_Y_TOP_ROW << 16);
            }
            let mut c_to = [0u32; 2];
            let mut c_v = [0u32; 2];
            let mut c_h = [0u32; 2];
            for i in 0..2 {
                c_to[i] = (uvcbp[POS_BOTTOM][i] << 4) | uvcbp[POS_CUR][i];
                c_v[i] = c_to[i] | ((uvcbp[POS_CUR][i] << 1) & !MASK_C_LEFT_COL) | ((uvcbp[POS_LEFT][i] & MASK_C_RIGHT_COL) >> 1);
                c_h[i] = c_to[i] | ((uvcbp[POS_TOP][i] & MASK_C_LAST_ROW) >> 2) | (uvcbp[POS_CUR][i] << 2);
                if mb_x == 0 {
                    c_v[i] &= !MASK_C_LEFT_COL;
                }
                if row == 0 {
                    c_h[i] &= !MASK_C_TOP_ROW;
                }
                if row == mh - 1 || strong[POS_CUR] || strong[POS_BOTTOM] {
                    c_h[i] &= !(MASK_C_TOP_ROW << 4);
                }
            }
            let strong_cl = strong[POS_CUR] || strong[POS_LEFT];
            let strong_ct = strong[POS_CUR] || strong[POS_TOP];
            let y = &mut cur.planes[0];
            for j in (0..16).step_by(4) {
                for i in 0..4 {
                    let ij = i + j;
                    let (px, py) = (mb_x * 16 + i * 4, row * 16 + j);
                    let clip_cur = if y_to_deblock & (MASK_CUR << ij) != 0 { clip[POS_CUR] } else { 0 };
                    let dither = if j != 0 { ij } else { i * 4 };
                    if y_h & (MASK_BOTTOM << ij) != 0 {
                        let lq = if y_to_deblock & (MASK_BOTTOM << ij) != 0 { clip[POS_CUR] } else { 0 };
                        dsp::rv40_adaptive(y, px, py + 4, dither, lq, clip_cur, alpha, beta, beta_y, false, false, 0);
                    }
                    if y_v & (MASK_CUR << ij) != 0 && (i != 0 || !strong_cl) {
                        let clip_left = if i == 0 {
                            if mvmasks[POS_LEFT] & (MASK_RIGHT << j) != 0 { clip[POS_LEFT] } else { 0 }
                        } else if y_to_deblock & (MASK_CUR << (ij - 1)) != 0 {
                            clip[POS_CUR]
                        } else {
                            0
                        };
                        dsp::rv40_adaptive(y, px, py, dither, clip_cur, clip_left, alpha, beta, beta_y, false, false, 1);
                    }
                    if j == 0 && y_h & (MASK_CUR << i) != 0 && strong_ct {
                        let ct = if mvmasks[POS_TOP] & (MASK_TOP << i) != 0 { clip[POS_TOP] } else { 0 };
                        dsp::rv40_adaptive(y, px, py, dither, clip_cur, ct, alpha, beta, beta_y, false, true, 0);
                    }
                    if y_v & (MASK_CUR << ij) != 0 && i == 0 && strong_cl {
                        let clip_left = if mvmasks[POS_LEFT] & (MASK_RIGHT << j) != 0 { clip[POS_LEFT] } else { 0 };
                        dsp::rv40_adaptive(y, px, py, dither, clip_cur, clip_left, alpha, beta, beta_y, false, true, 1);
                    }
                }
            }
            for k in 0..2 {
                let c = &mut cur.planes[k + 1];
                for j in 0..2 {
                    for i in 0..2 {
                        let ij = i + j * 2;
                        let (px, py) = (mb_x * 8 + i * 4, row * 8 + j * 4);
                        let clip_cur = if c_to[k] & (MASK_CUR << ij) != 0 { clip[POS_CUR] } else { 0 };
                        if c_h[k] & (MASK_CUR << (ij + 2)) != 0 {
                            let clip_bot = if c_to[k] & (MASK_CUR << (ij + 2)) != 0 { clip[POS_CUR] } else { 0 };
                            dsp::rv40_adaptive(c, px, py + 4, i * 8, clip_bot, clip_cur, alpha, beta, beta_c, true, false, 0);
                        }
                        if c_v[k] & (MASK_CUR << ij) != 0 && (i != 0 || !strong_cl) {
                            let clip_left = if i == 0 {
                                if uvcbp[POS_LEFT][k] & (MASK_CUR << (2 * j + 1)) != 0 { clip[POS_LEFT] } else { 0 }
                            } else if c_to[k] & (MASK_CUR << (ij - 1)) != 0 {
                                clip[POS_CUR]
                            } else {
                                0
                            };
                            dsp::rv40_adaptive(c, px, py, j * 8, clip_cur, clip_left, alpha, beta, beta_c, true, false, 1);
                        }
                        if j == 0 && c_h[k] & (MASK_CUR << ij) != 0 && strong_ct {
                            let clip_top = if uvcbp[POS_TOP][k] & (MASK_CUR << (ij + 2)) != 0 { clip[POS_TOP] } else { 0 };
                            dsp::rv40_adaptive(c, px, py, i * 8, clip_cur, clip_top, alpha, beta, beta_c, true, true, 0);
                        }
                        if c_v[k] & (MASK_CUR << ij) != 0 && i == 0 && strong_cl {
                            let clip_left = if uvcbp[POS_LEFT][k] & (MASK_CUR << (2 * j + 1)) != 0 { clip[POS_LEFT] } else { 0 };
                            dsp::rv40_adaptive(c, px, py, j * 8, clip_cur, clip_left, alpha, beta, beta_c, true, true, 1);
                        }
                    }
                }
            }
        }
    }
}

/// `get_dimension` (RV40 picture sizes).
fn dimension(gb: &mut Gb<'_>, dim: &[i32]) -> Result<u32> {
    let t = gb.get(3) as usize;
    let mut val = dim.get(t).copied().unwrap_or(0);
    if val < 0 {
        val = dim.get((i64::from(gb.get(1)) - i64::from(val)) as usize).copied().unwrap_or(0);
    }
    if val == 0 {
        loop {
            if gb.left() < 8 {
                return Err(Error::Invalid("picture size"));
            }
            let t = gb.get(8) as i32;
            val += t << 2;
            if t != 0xFF {
                break;
            }
            if val > MAX_SIDE as i32 * 4 {
                return Err(Error::Invalid("picture size"));
            }
        }
    }
    Ok(val as u32)
}

fn choose_vlc_set(quant: usize, modifier: u32, inter: bool) -> VlcSel {
    let mut q = quant;
    if modifier == 2 && q < 19 {
        q += 10;
    } else if modifier != 0 && q < 26 {
        q += 5;
    }
    let idx = usize::from(QUANT_TO_VLC_SET.get(usize::from(inter) * 32 + q.min(31)).copied().unwrap_or(0));
    VlcSel { inter, idx }
}

fn adjust_pred16(mut itype: i32, up: bool, left: bool) -> i32 {
    if !up && !left {
        itype = pred::DC_128_8;
    } else if !up {
        if itype == pred::PLANE8 || itype == pred::VERT8 {
            itype = pred::HOR8;
        }
        if itype == pred::DC8 {
            itype = pred::LEFT_DC8;
        }
    } else if !left {
        if itype == pred::PLANE8 || itype == pred::HOR8 {
            itype = pred::VERT8;
        }
        if itype == pred::DC8 {
            itype = pred::TOP_DC8;
        }
    }
    itype
}

/// `rv34_decode_cbp`.
fn decode_cbp(gb: &mut Gb<'_>, vlc: &VlcSet, table: usize) -> Result<u32> {
    const MASKS: [u32; 3] = [0x100000, 0x010000, 0x110000];
    const SHIFTS: [u32; 4] = [0, 2, 8, 10];
    let code = u32::from(vlc.cbppattern.get(table).and_then(|v| v.read(gb)).ok_or(Error::Invalid("cbp"))?);
    let pattern = code & 0xF;
    let code = (code >> 4) as usize;
    let ones = usize::from(COUNT_ONES[pattern as usize]);
    let mut cbp = 0u32;
    for (k, mask) in [8u32, 4, 2, 1].into_iter().enumerate() {
        if pattern & mask != 0 {
            let v = vlc.cbp.get(table).and_then(|t| t.get(ones)).and_then(|v| v.read(gb)).ok_or(Error::Invalid("cbp"))?;
            cbp |= u32::from(v) << SHIFTS[k];
        }
    }
    let m3 = u32::from(MODULO_THREE.get(code).copied().unwrap_or(0));
    for i in 0..4 {
        let t = (m3 >> (6 - 2 * i)) & 3;
        if t == 1 {
            cbp |= MASKS[usize::from(gb.get1())] << i;
        }
        if t == 2 {
            cbp |= MASKS[2] << i;
        }
    }
    Ok(cbp)
}

/// `decode_coeff`.
fn decode_coeff(dst: &mut i16, coef: u32, esc: u32, gb: &mut Gb<'_>, vlc: &vlc::Vlc, q: i32) {
    if coef == 0 {
        return;
    }
    let mut c = coef as i32;
    if coef == esc {
        c = i32::from(vlc.read(gb).unwrap_or(0));
        if c > 23 {
            let n = (c - 23) as u32;
            c = 22 + ((1i32.wrapping_shl(n)) | gb.get(n.min(31)) as i32);
        }
        c += esc as i32;
    }
    if gb.get1() {
        c = c.wrapping_neg();
    }
    *dst = (c.wrapping_mul(q).wrapping_add(8) >> 4) as i16;
}

fn decode_subblock(dst: &mut [i16; 16], at: usize, code: usize, is_block2: bool, gb: &mut Gb<'_>, vlc: &vlc::Vlc, q: i32) {
    let flags = u32::from(MODULO_THREE.get(code).copied().unwrap_or(0));
    let mut c = |k: usize, coef: u32, esc: u32, gb: &mut Gb<'_>| {
        if let Some(d) = dst.get_mut(at + k) {
            decode_coeff(d, coef, esc, gb, vlc, q);
        }
    };
    c(0, flags >> 6, 3, gb);
    if is_block2 {
        c(4, (flags >> 4) & 3, 2, gb);
        c(1, (flags >> 2) & 3, 2, gb);
    } else {
        c(1, (flags >> 4) & 3, 2, gb);
        c(4, (flags >> 2) & 3, 2, gb);
    }
    c(5, flags & 3, 2, gb);
}

/// `rv34_decode_block`: returns whether the block has AC coefficients (`has_ac | pattern`).
#[allow(clippy::too_many_arguments)]
fn decode_block(dst: &mut [i16; 16], gb: &mut Gb<'_>, set: &VlcSet, fc: usize, sc: usize, q_dc: i32, q_ac1: i32, q_ac2: i32) -> u32 {
    let Some(code) = set.first.get(fc).and_then(|v| v.read(gb)) else { return 0 };
    let pattern = u32::from(code & 7);
    let code = usize::from(code >> 3);
    let flags = u32::from(MODULO_THREE.get(code).copied().unwrap_or(0));
    let mut has_ac = 1;
    if flags & 0x3F != 0 {
        decode_coeff(&mut dst[0], flags >> 6, 3, gb, &set.coeff, q_dc);
        decode_coeff(&mut dst[1], (flags >> 4) & 3, 2, gb, &set.coeff, q_ac1);
        decode_coeff(&mut dst[4], (flags >> 2) & 3, 2, gb, &set.coeff, q_ac1);
        decode_coeff(&mut dst[5], flags & 3, 2, gb, &set.coeff, q_ac2);
    } else {
        decode_coeff(&mut dst[0], flags >> 6, 3, gb, &set.coeff, q_dc);
        if pattern == 0 {
            return 0;
        }
        has_ac = 0;
    }
    if pattern & 4 != 0 {
        let code = set.second.get(sc).and_then(|v| v.read(gb)).map_or(0, usize::from);
        decode_subblock(dst, 2, code, false, gb, &set.coeff, q_ac2);
    }
    if pattern & 2 != 0 {
        let code = set.second.get(sc).and_then(|v| v.read(gb)).map_or(0, usize::from);
        decode_subblock(dst, 8, code, true, gb, &set.coeff, q_ac2);
    }
    if pattern & 1 != 0 {
        let code = set.third.get(sc).and_then(|v| v.read(gb)).map_or(0, usize::from);
        decode_subblock(dst, 10, code, false, gb, &set.coeff, q_ac2);
    }
    has_ac | pattern
}

/// The picture type of a packet (the slice table and the first slice header), without decoding.
pub fn picture_type(codec: Codec, pkt: &[u8]) -> Option<PictType> {
    let n = usize::from(*pkt.first()?) + 1;
    let hdr = pkt.get(1..1 + 8 * n)?;
    let buf_size = pkt.len().checked_sub(1 + 8 * n)?;
    let off = slice_offset(hdr, 0, n, buf_size)?;
    let b = *pkt.get(1 + 8 * n + off)?;
    let ty = match codec {
        Codec::Rv30 => (b >> 3) & 3,
        Codec::Rv40 => (b >> 5) & 3,
    };
    Some(match ty {
        0 | 1 => PictType::I,
        2 => PictType::P,
        _ => PictType::B,
    })
}
