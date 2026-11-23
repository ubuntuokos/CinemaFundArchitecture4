//! Sorenson Video 3 (SVQ3) video decoder, ported from FFmpeg's (n7.1.1) `libavcodec/svq3.c` with
//! the H.264 intra predictors, the half- and third-pel interpolation and the Exp-Golomb readers it
//! uses. Pure safe Rust, no dependencies but `thiserror`.
//!
//! SVQ3 is an H.264 predecessor: 16x16 macroblocks, intra 4x4 / 16x16 prediction, integer 4x4
//! transforms, P and B pictures with full-, half- and third-pel motion and 16x16 .. 4x4
//! partitions. The decoder keeps three picture slots (current, last reference, next reference);
//! with B pictures (`low_delay` unset in the sequence header) output is delayed by one reference
//! picture, so [`Decoder::decode`] returns the picture to show, which is not always the one just
//! decoded, and [`Decoder::flush`] returns the last one.

mod inflate;
mod pred;
mod tables;

use pred::{pred4x4, pred8x8, pred16x16};
use tables::{
    CHROMA_DC_SCAN, GOLOMB_TO_INTER_CBP, GOLOMB_TO_INTRA4X4_CBP, I_MB_TYPE_INFO, SCAN8, SVQ3_DCT_TABLES, SVQ3_DEQUANT_COEFF, SVQ3_PRED_0, SVQ3_PRED_1,
    SVQ3_SCAN, ZIGZAG_SCAN, chroma_qp,
};
use thiserror::Error;

/// Why a frame or the sequence header was rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid SVQ3 data: {0}")]
    Invalid(&'static str),
    #[error("unsupported SVQ3 feature: {0}")]
    Unsupported(&'static str),
}

/// A decoded picture: planar YUV 4:2:0 (full range, "YUVJ420P"), cropped to the stream size.
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Y, Cb, Cr; chroma planes are `ceil(width/2) x ceil(height/2)`.
    pub planes: Vec<Vec<u8>>,
    /// An intra picture.
    pub key: bool,
}

/// A picture to show together with the tag given to [`Decoder::decode`] with its packet.
#[derive(Clone, Debug)]
pub struct Output {
    pub picture: Picture,
    pub tag: i64,
}

// --- bit reading ---------------------------------------------------------------------------------

/// MSB-first bit reader over an owned buffer; reads past the end return zero bits like FFmpeg's
/// zero-padded buffers, `left()` goes negative.
#[derive(Default)]
struct Bits {
    buf: Vec<u8>,
    size: usize,
    pos: usize,
}

impl Bits {
    fn new(buf: Vec<u8>, size_bits: usize) -> Bits {
        Bits { buf, size: size_bits, pos: 0 }
    }

    fn left(&self) -> i64 {
        self.size as i64 - self.pos as i64
    }

    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let byte = self.pos >> 3;
        let mut v = 0u64;
        for k in 0..8 {
            v = (v << 8) | u64::from(self.buf.get(byte + k).copied().unwrap_or(0));
        }
        ((v << (self.pos & 7)) >> (64 - n)) as u32
    }

    fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    fn get1(&mut self) -> u32 {
        self.get(1)
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    /// `get_interleaved_ue_golomb`: the data bits are interleaved with "more" flags.
    fn ue(&mut self) -> u32 {
        let mut ret = 1u32;
        loop {
            if self.left() <= 0 || self.get1() == 1 {
                break;
            }
            ret = (ret << 1) | self.get1();
            if ret >= 0x0800_0000 {
                break;
            }
        }
        ret - 1
    }

    /// `get_interleaved_se_golomb`.
    fn se(&mut self) -> i32 {
        let ue = i64::from(self.ue());
        (if ue & 1 == 1 { (ue + 1) >> 1 } else { -(ue >> 1) }) as i32
    }

    /// `skip_1stop_8data_bits`.
    fn skip_1stop_8data_bits(&mut self) -> Result<(), Error> {
        if self.left() <= 0 {
            return Err(Error::Invalid("header ends early"));
        }
        while self.get1() == 1 {
            self.skip(8);
            if self.left() <= 0 {
                return Err(Error::Invalid("header ends early"));
            }
        }
        Ok(())
    }
}

// --- pictures ---------------------------------------------------------------------------------------

/// Margin around each plane, so that intra prediction may read its (unused) edge neighbours.
const MARGIN: usize = 16;

#[derive(Clone, Default)]
struct Plane {
    data: Vec<u8>,
    stride: usize,
    origin: usize,
}

impl Plane {
    fn new(w: usize, h: usize, fill: u8) -> Plane {
        let stride = w + 2 * MARGIN;
        Plane { data: vec![fill; stride * (h + 2 * MARGIN)], stride, origin: MARGIN * stride + MARGIN }
    }

    fn at(&self, x: usize, y: usize) -> usize {
        self.origin + y * self.stride + x
    }
}

#[derive(Default)]
struct Slot {
    planes: [Plane; 3],
    /// Holds a picture (FFmpeg's `f->data[0] != NULL`).
    valid: bool,
    motion_val: [Vec<[i16; 2]>; 2],
    mb_type: Vec<u32>,
    key: bool,
    tag: i64,
}

/// The kind of a coded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicType {
    /// Intra: refers to no other picture.
    I,
    /// Predicted from the previous reference picture.
    P,
    /// Predicted from the previous and the next reference pictures; no picture refers to it.
    B,
}

const MB_INTRA4X4: u32 = 0x0001;
const MB_INTRA16X16: u32 = 0x0002;
const MB_16X16: u32 = 0x0008;
const MB_SKIP: u32 = 0x0800;

fn is_intra(t: u32) -> bool {
    t & 7 != 0
}
fn is_inter(t: u32) -> bool {
    t & 0x78 != 0
}

const PART_NOT_AVAILABLE: i8 = -2;

const FULLPEL_MODE: i32 = 1;
const HALFPEL_MODE: i32 = 2;
const THIRDPEL_MODE: i32 = 3;
const PREDICT_MODE: i32 = 4;

const VERT_PRED: i8 = 0;
const DC_PRED: i8 = 2;
const DIAG_DOWN_LEFT_PRED: i8 = 3;
const VERT_LEFT_PRED: i8 = 7;
const LEFT_DC_PRED: i8 = 9;
const TOP_DC_PRED: i8 = 10;
const DC_128_PRED: i8 = 11;

const DC_PRED8X8: i32 = 0;
const LEFT_DC_PRED8X8: i32 = 4;
const TOP_DC_PRED8X8: i32 = 5;
const DC_128_PRED8X8: i32 = 6;

fn clip(a: i32, lo: i32, hi: i32) -> i32 {
    if a < lo {
        lo
    } else if a > hi {
        hi
    } else {
        a
    }
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

fn clip_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// FFmpeg's `ff_h264_check_intra4x4_pred_mode` (its result is ignored by SVQ3).
fn check_intra4x4_pred_mode(cache: &mut [i8; 40], top_avail: u32, left_avail: u32) {
    const TOP: [i8; 12] = [-1, 0, LEFT_DC_PRED, -1, -1, -1, -1, -1, 0, 0, 0, 0];
    const LEFT: [i8; 12] = [0, -1, TOP_DC_PRED, 0, -1, -1, -1, 0, -1, DC_128_PRED, 0, 0];
    let base = usize::from(SCAN8[0]);
    if top_avail & 0x8000 == 0 {
        for i in 0..4 {
            let Some(&status) = usize::try_from(cache[base + i]).ok().and_then(|m| TOP.get(m)) else { return };
            if status < 0 {
                return;
            } else if status != 0 {
                cache[base + i] = status;
            }
        }
    }
    if left_avail & 0x8888 != 0x8888 {
        const MASK: [u32; 4] = [0x8000, 0x2000, 0x80, 0x20];
        for i in 0..4 {
            if left_avail & MASK[i] == 0 {
                let Some(&status) = usize::try_from(cache[base + 8 * i]).ok().and_then(|m| LEFT.get(m)) else { return };
                if status < 0 {
                    return;
                } else if status != 0 {
                    cache[base + 8 * i] = status;
                }
            }
        }
    }
}

/// FFmpeg's `ff_h264_check_intra_pred_mode` (the 8-bit, non-MBAFF case); `None` when the mode
/// cannot be used with the available neighbours.
fn check_intra_pred_mode(top_avail: u32, left_avail: u32, mut mode: i32) -> Option<i32> {
    const TOP: [i8; 4] = [LEFT_DC_PRED8X8 as i8, 1, -1, -1];
    const LEFT: [i8; 5] = [TOP_DC_PRED8X8 as i8, -1, 2, -1, DC_128_PRED8X8 as i8];
    if !(0..=3).contains(&mode) {
        return None;
    }
    if top_avail & 0x8000 == 0 {
        mode = i32::from(TOP[mode as usize]);
        if mode < 0 {
            return None;
        }
    }
    if left_avail & 0x8080 != 0x8080 {
        mode = i32::from(*LEFT.get(mode as usize)?);
        if mode < 0 {
            return None;
        }
    }
    Some(mode)
}

// --- transforms ----------------------------------------------------------------------------------

/// `svq3_luma_dc_dequant_idct_c`: the 16 DC coefficients of an Intra16x16 macroblock into the
/// first coefficient of each of its 16 blocks.
fn luma_dc_dequant_idct(output: &mut [i16], input: &[i16], qp: usize) {
    let qmul = SVQ3_DEQUANT_COEFF[qp];
    let mut temp = [0i32; 16];
    for i in 0..4 {
        let inp = |k: usize| i32::from(input[4 * i + k]);
        let z0 = 13 * (inp(0) + inp(2));
        let z1 = 13 * (inp(0) - inp(2));
        let z2 = 7 * inp(1) - 17 * inp(3);
        let z3 = 17 * inp(1) + 7 * inp(3);
        temp[4 * i] = z0 + z3;
        temp[4 * i + 1] = z1 + z2;
        temp[4 * i + 2] = z1 - z2;
        temp[4 * i + 3] = z0 - z3;
    }
    const X_OFFSET: [usize; 4] = [0, 16, 64, 80];
    let q = |v: i32| (v as u32).wrapping_mul(qmul).wrapping_add(0x80000) as i32 >> 20;
    for (i, &offset) in X_OFFSET.iter().enumerate() {
        let z0 = 13 * (temp[i] + temp[8 + i]);
        let z1 = 13 * (temp[i] - temp[8 + i]);
        let z2 = 7 * temp[4 + i] - 17 * temp[12 + i];
        let z3 = 17 * temp[4 + i] + 7 * temp[12 + i];
        output[offset] = q(z0 + z3) as i16;
        output[32 + offset] = q(z1 + z2) as i16;
        output[128 + offset] = q(z1 - z2) as i16;
        output[160 + offset] = q(z0 - z3) as i16;
    }
}

/// `svq3_add_idct_c`: inverse transform of one 4x4 block added to the picture; clears the block.
/// `dc` is 0 for a plain block, 1 for an Intra16x16 block (its DC from the DC transform) and 2 for
/// a chroma block.
fn add_idct(dst: &mut [u8], pos: usize, stride: usize, block: &mut [i16], qp: usize, dc: i32) {
    let qmul = SVQ3_DEQUANT_COEFF[qp] as i32;
    let mut dcv = 0i32;
    if dc != 0 {
        let b0 = i32::from(block[0]);
        dcv = 169i32.wrapping_mul(if dc == 1 { 1538i32.wrapping_mul(b0) } else { qmul.wrapping_mul(b0 >> 3) / 2 });
        block[0] = 0;
    }
    for i in 0..4 {
        let b = |k: usize| i32::from(block[k + 4 * i]);
        let z0 = 13 * (b(0) + b(2));
        let z1 = 13 * (b(0) - b(2));
        let z2 = 7 * b(1) - 17 * b(3);
        let z3 = 17 * b(1) + 7 * b(3);
        block[4 * i] = (z0 + z3) as i16;
        block[4 * i + 1] = (z1 + z2) as i16;
        block[4 * i + 2] = (z1 - z2) as i16;
        block[4 * i + 3] = (z0 - z3) as i16;
    }
    let rr = (dcv as u32).wrapping_add(0x80000);
    for i in 0..4 {
        let b = |k: usize| i32::from(block[i + 4 * k]);
        let z0 = (13 * (b(0) + b(2))) as u32;
        let z1 = (13 * (b(0) - b(2))) as u32;
        let z2 = (7 * b(1) - 17 * b(3)) as u32;
        let z3 = (17 * b(1) + 7 * b(3)) as u32;
        let q = |v: u32| (v.wrapping_mul(qmul as u32).wrapping_add(rr)) as i32 >> 20;
        let mut put = |row: usize, v: i32| {
            let p = &mut dst[pos + i + stride * row];
            *p = clip_u8(i32::from(*p) + v);
        };
        put(0, q(z0.wrapping_add(z3)));
        put(1, q(z1.wrapping_add(z2)));
        put(2, q(z1.wrapping_sub(z2)));
        put(3, q(z0.wrapping_sub(z3)));
    }
    block[..16].fill(0);
}

/// `h264_chroma_dc_dequant_idct` (8-bit): the 2x2 chroma DC block at `block[0, 16, 32, 48]`.
fn chroma_dc_dequant_idct(block: &mut [i16], qmul: i32) {
    let (a, b, c, d) = (i32::from(block[0]), i32::from(block[16]), i32::from(block[32]), i32::from(block[48]));
    let e = a.wrapping_sub(b);
    let a = a.wrapping_add(b);
    let b = c.wrapping_sub(d);
    let c = c.wrapping_add(d);
    let q = |v: i32| (v.wrapping_mul(qmul) >> 7) as i16;
    block[0] = q(a.wrapping_add(c));
    block[16] = q(e.wrapping_add(b));
    block[32] = q(a.wrapping_sub(c));
    block[48] = q(e.wrapping_sub(b));
}

/// `svq3_decode_block`: the (run, level) coded coefficients of one block into `block` through the
/// scan of `ty` (1 = plain, 2 = intra with two passes, 3 = chroma DC).
fn decode_block(gb: &mut Bits, block: &mut [i16], mut index: usize, ty: usize) -> Result<(), ()> {
    let intra = (3 * ty) >> 2;
    let mut limit = 16usize >> intra;
    while index < 16 {
        loop {
            let mut vlc = gb.ue();
            if vlc == 0 {
                break;
            }
            if (vlc as i32) < 0 {
                return Err(());
            }
            let sign: i32 = if vlc & 1 == 1 { 0 } else { -1 };
            vlc = (vlc + 1) >> 1;
            let (run, level);
            if ty == 3 {
                if vlc < 3 {
                    run = 0;
                    level = vlc as i32;
                } else if vlc < 4 {
                    run = 1;
                    level = 1;
                } else {
                    run = (vlc & 3) as usize;
                    level = ((vlc + 9) >> 2) as i32 - run as i32;
                }
            } else if vlc < 16 {
                let (r, l) = SVQ3_DCT_TABLES[intra][vlc as usize];
                run = usize::from(r);
                level = i32::from(l);
            } else if intra != 0 {
                run = (vlc & 7) as usize;
                level = (vlc >> 3) as i32
                    + if run == 0 {
                        8
                    } else if run < 2 {
                        2
                    } else if run < 5 {
                        0
                    } else {
                        -1
                    };
            } else {
                run = (vlc & 0xF) as usize;
                level = (vlc >> 4) as i32
                    + if run == 0 {
                        4
                    } else if run < 3 {
                        2
                    } else if run < 10 {
                        1
                    } else {
                        0
                    };
            }
            index += run;
            if index >= limit {
                return Err(());
            }
            let pos = match ty {
                1 => usize::from(ZIGZAG_SCAN[index]),
                2 => usize::from(SVQ3_SCAN[index]),
                _ => usize::from(CHROMA_DC_SCAN[index]),
            };
            block[pos] = ((level ^ sign) - sign) as i16;
            index += 1;
        }
        if ty != 2 {
            break;
        }
        index = limit;
        limit += 8;
    }
    Ok(())
}

// --- motion compensation --------------------------------------------------------------------------------

/// Copies the `(w+1) x (h+1)` source block at (`mx`, `my`) into `out` (stride 17), repeating the
/// picture's edge samples for the parts outside `edge_w x edge_h` (`emulated_edge_mc`).
fn fetch_block(plane: &Plane, mx: i32, my: i32, w: usize, h: usize, edge_w: i32, edge_h: i32, out: &mut [u8; 17 * 17]) {
    for dy in 0..=h {
        let sy = clip(my + dy as i32, 0, (edge_h - 1).max(0)) as usize;
        for dx in 0..=w {
            let sx = clip(mx + dx as i32, 0, (edge_w - 1).max(0)) as usize;
            out[dy * 17 + dx] = plane.data[plane.at(sx, sy)];
        }
    }
}

/// Half-pel (`hpeldsp` put / avg, rounding up) or third-pel (`tpeldsp`) prediction of a `w x h`
/// block from `src` (stride 17) into the picture, `dxy` as in FFmpeg's function tables.
#[allow(clippy::too_many_arguments)]
fn interpolate(dst: &mut Plane, dx: usize, dy: usize, src: &[u8; 17 * 17], w: usize, h: usize, dxy: usize, thirdpel: bool, avg: bool) {
    let base = dst.at(dx, dy);
    // FFmpeg's 2-pixel wide "avg" half-pel xy2 predictor is a plain "put" ("FIXME non put")
    let avg = avg && !(w == 2 && !thirdpel && dxy == 3);
    for y in 0..h {
        for x in 0..w {
            let s = |ox: usize, oy: usize| i32::from(src[(y + oy) * 17 + x + ox]);
            let v = if thirdpel {
                match dxy {
                    0 => s(0, 0),
                    1 => ((2 * s(0, 0) + s(1, 0) + 1) * 683) >> 11,
                    2 => ((s(0, 0) + 2 * s(1, 0) + 1) * 683) >> 11,
                    4 => ((2 * s(0, 0) + s(0, 1) + 1) * 683) >> 11,
                    5 => ((4 * s(0, 0) + 3 * s(1, 0) + 3 * s(0, 1) + 2 * s(1, 1) + 6) * 2731) >> 15,
                    6 => ((3 * s(0, 0) + 4 * s(1, 0) + 2 * s(0, 1) + 3 * s(1, 1) + 6) * 2731) >> 15,
                    8 => ((s(0, 0) + 2 * s(0, 1) + 1) * 683) >> 11,
                    9 => ((3 * s(0, 0) + 2 * s(1, 0) + 4 * s(0, 1) + 3 * s(1, 1) + 6) * 2731) >> 15,
                    _ => ((2 * s(0, 0) + 3 * s(1, 0) + 3 * s(0, 1) + 4 * s(1, 1) + 6) * 2731) >> 15,
                }
            } else {
                match dxy {
                    0 => s(0, 0),
                    1 => (s(0, 0) + s(1, 0) + 1) >> 1,
                    2 => (s(0, 0) + s(0, 1) + 1) >> 1,
                    _ => (s(0, 0) + s(1, 0) + s(0, 1) + s(1, 1) + 2) >> 2,
                }
            };
            let d = &mut dst.data[base + y * dst.stride + x];
            *d = if avg { ((i32::from(*d) + v + 1) >> 1) as u8 } else { v as u8 };
        }
    }
}

// --- the decoder ------------------------------------------------------------------------------------

/// The SVQ3 decoder.
pub struct Decoder {
    width: usize,
    height: usize,
    frames: [Slot; 3],
    cur: usize,
    next: usize,
    last: usize,
    gb: Bits,
    gb_slice: Bits,
    halfpel_flag: bool,
    thirdpel_flag: bool,
    has_watermark: bool,
    watermark_key: u32,
    adaptive_quant: bool,
    low_delay: bool,
    h_edge_pos: i32,
    v_edge_pos: i32,
    last_frame_output: bool,
    slice_num: i32,
    qscale: i32,
    cbp: i32,
    frame_num: i32,
    frame_num_offset: i32,
    prev_frame_num_offset: i32,
    prev_frame_num: i32,
    pict_type: PicType,
    slice_type: PicType,
    mb_x: usize,
    mb_y: usize,
    mb_xy: usize,
    mb_width: usize,
    mb_height: usize,
    mb_stride: usize,
    mb_num: usize,
    b_stride: usize,
    mb2br_xy: Vec<usize>,
    chroma_pred_mode: i32,
    intra16x16_pred_mode: i32,
    intra4x4_pred_mode_cache: [i8; 40],
    intra4x4_pred_mode: Vec<i8>,
    top_samples_available: u32,
    left_samples_available: u32,
    mv_cache: [[[i16; 2]; 40]; 2],
    ref_cache: [[i8; 40]; 2],
    mb: Vec<i16>,
    mb_luma_dc: [i16; 32],
    non_zero_count_cache: [u8; 120],
}

fn bad<T>(what: &'static str) -> Result<T, Error> {
    Err(Error::Invalid(what))
}

impl Decoder {
    /// A decoder for streams of `width x height` (used unless the sequence header in `extradata`
    /// gives the size). `extradata` is the QuickTime image description's `SMI ` atom (or anything
    /// holding the `SEQH` sequence header); an empty slice gives FFmpeg's defaults (half- and
    /// third-pel motion, B pictures).
    pub fn new(mut width: u32, mut height: u32, extradata: &[u8]) -> Result<Decoder, Error> {
        let mut halfpel_flag = true;
        let mut thirdpel_flag = true;
        let mut has_watermark = false;
        let mut watermark_key = 0u32;
        let mut low_delay = false;

        // prowl for the "SEQH" marker in the extradata
        let marker = (0..extradata.len()).take_while(|m| m + 8 < extradata.len()).find(|&m| &extradata[m..m + 4] == b"SEQH");
        if let Some(m) = marker {
            let ex = &extradata[m..];
            let size = u32::from_be_bytes([ex[4], ex[5], ex[6], ex[7]]) as usize;
            if size > ex.len() - 8 {
                return bad("sequence header size");
            }
            let mut gb = Bits::new(ex[8..8 + size].to_vec(), size * 8);
            let (w, h) = match gb.get(3) {
                0 => (160, 120),
                1 => (128, 96),
                2 => (176, 144),
                3 => (352, 288),
                4 => (704, 576),
                5 => (240, 180),
                6 => (320, 240),
                _ => (gb.get(12), gb.get(12)),
            };
            width = w;
            height = h;
            halfpel_flag = gb.get1() == 1;
            thirdpel_flag = gb.get1() == 1;
            gb.skip(4); // unknown fields
            low_delay = gb.get1() == 1;
            gb.skip(1); // unknown field
            gb.skip_1stop_8data_bits()?;
            has_watermark = gb.get1() == 1;
            if has_watermark {
                let watermark_width = u64::from(gb.ue());
                let watermark_height = u64::from(gb.ue());
                let _u1 = gb.ue();
                gb.skip(8 + 2);
                let _u4 = gb.ue();
                let offset = (gb.pos + 7) >> 3;
                if watermark_height == 0 || watermark_width * 4 > u64::from(u32::MAX) / watermark_height {
                    return bad("watermark size");
                }
                let buf_len = (watermark_width * watermark_height * 4) as usize;
                if offset > size || buf_len > 1 << 28 {
                    return bad("watermark size");
                }
                let Some(buf) = inflate::zlib_decompress(&ex[8 + offset..8 + size], buf_len) else {
                    return bad("could not uncompress watermark logo");
                };
                // crc16-ccitt (init 0, MSB first) of the logo, twice
                let mut crc = 0u16;
                for &b in &buf {
                    crc ^= u16::from(b) << 8;
                    for _ in 0..8 {
                        crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
                    }
                }
                watermark_key = (u32::from(crc) << 16) | u32::from(crc);
            }
        }
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return bad("frame size");
        }
        let (width, height) = (width as usize, height as usize);
        let mb_width = width.div_ceil(16);
        let mb_height = height.div_ceil(16);
        let mb_stride = mb_width + 1;
        let mut mb2br_xy = vec![0usize; mb_stride * (mb_height + 1)];
        for y in 0..mb_height {
            for x in 0..mb_width {
                let mb_xy = x + y * mb_stride;
                mb2br_xy[mb_xy] = 8 * (mb_xy % (2 * mb_stride));
            }
        }
        Ok(Decoder {
            width,
            height,
            frames: Default::default(),
            cur: 0,
            last: 1,
            next: 2,
            gb: Bits::default(),
            gb_slice: Bits::default(),
            halfpel_flag,
            thirdpel_flag,
            has_watermark,
            watermark_key,
            adaptive_quant: false,
            low_delay,
            h_edge_pos: (mb_width * 16) as i32,
            v_edge_pos: (mb_height * 16) as i32,
            last_frame_output: false,
            slice_num: 0,
            qscale: 0,
            cbp: 0,
            frame_num: 0,
            frame_num_offset: 0,
            prev_frame_num_offset: 0,
            prev_frame_num: 0,
            pict_type: PicType::I,
            slice_type: PicType::I,
            mb_x: 0,
            mb_y: 0,
            mb_xy: 0,
            mb_width,
            mb_height,
            mb_stride,
            mb_num: mb_width * mb_height,
            b_stride: 4 * mb_width,
            mb2br_xy,
            chroma_pred_mode: 0,
            intra16x16_pred_mode: 0,
            intra4x4_pred_mode_cache: [0; 40],
            intra4x4_pred_mode: vec![0; mb_stride * 2 * 8],
            top_samples_available: 0,
            left_samples_available: 0,
            mv_cache: [[[0; 2]; 40]; 2],
            ref_cache: [[0; 40]; 2],
            mb: vec![0; 16 * 48 * 2],
            mb_luma_dc: [0; 32],
            non_zero_count_cache: [0; 120],
        })
    }

    /// The stream's picture size.
    pub fn size(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    /// Whether the stream may hold B pictures (the sequence header did not say `low_delay`):
    /// output then lags decoding by one reference picture.
    pub fn has_b_frames(&self) -> bool {
        !self.low_delay
    }

    /// Forgets all pictures (after a seek).
    pub fn reset(&mut self) {
        for s in &mut self.frames {
            s.valid = false;
        }
        self.last_frame_output = false;
        self.frame_num = 0;
        self.prev_frame_num = 0;
        self.frame_num_offset = 0;
        self.prev_frame_num_offset = 0;
    }

    /// The last picture still held back (`None` for streams without B pictures, or once drained).
    pub fn flush(&mut self) -> Option<Output> {
        if self.frames[self.next].valid && !self.low_delay && !self.last_frame_output {
            self.last_frame_output = true;
            return Some(self.output(self.next));
        }
        None
    }

    fn output(&self, slot: usize) -> Output {
        let s = &self.frames[slot];
        let (w, h) = (self.width, self.height);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let planes = [(w, h), (cw, ch), (cw, ch)]
            .iter()
            .zip(&s.planes)
            .map(|(&(pw, ph), p)| {
                let mut v = Vec::with_capacity(pw * ph);
                for y in 0..ph {
                    let o = p.at(0, y);
                    v.extend_from_slice(&p.data[o..o + pw]);
                }
                v
            })
            .collect();
        Output { picture: Picture { width: w as u32, height: h as u32, planes, key: s.key }, tag: s.tag }
    }

    /// Allocates (or reuses) the buffers of `slot` and marks it as holding a picture.
    fn get_buffer(&mut self, slot: usize) {
        let big_mb_num = self.mb_stride * (self.mb_height + 3) + 2;
        let b4 = self.b_stride * self.mb_height * 4 + 8;
        let (aw, ah) = (self.mb_width * 16, self.mb_height * 16);
        let s = &mut self.frames[slot];
        if s.mb_type.is_empty() {
            s.mb_type = vec![0; big_mb_num];
            s.motion_val = [vec![[0; 2]; b4], vec![[0; 2]; b4]];
        }
        if s.planes[0].data.is_empty() {
            s.planes = [Plane::new(aw, ah, 0), Plane::new(aw / 2, ah / 2, 128), Plane::new(aw / 2, ah / 2, 128)];
        }
        s.valid = true;
    }

    /// FFmpeg's stand-in for a reference picture that is missing (a stream starting with a
    /// delta picture): black luma and neutral chroma over the visible rows only, what lies below
    /// them (up to the macroblock-aligned size) stays zero as in a fresh FFmpeg frame buffer.
    fn fill_missing_reference(&mut self, slot: usize) {
        self.frames[slot].valid = false;
        self.get_buffer(slot);
        let rows = self.height / 2;
        let p = &mut self.frames[slot].planes;
        p[0].data.fill(0);
        for plane in &mut p[1..] {
            plane.data.fill(0);
            let (stride, origin) = (plane.stride, plane.origin);
            for y in 0..rows.min(self.mb_height * 8) {
                plane.data[origin + y * stride..origin + y * stride + self.mb_width * 8].fill(0x80);
            }
        }
    }

    /// Decodes one packet. Returns the picture to show now, if any, with the tag the decoder was
    /// given when it decoded it.
    pub fn decode(&mut self, data: &[u8], tag: i64) -> Result<Option<Output>, Error> {
        if data.is_empty() {
            return Ok(self.flush());
        }
        self.mb_x = 0;
        self.mb_y = 0;
        self.mb_xy = 0;
        self.gb = Bits::new(data.to_vec(), data.len() * 8);

        self.slice_header()?;
        if data.len() < self.mb_width * self.mb_height / 8 {
            return bad("packet too small");
        }
        self.pict_type = self.slice_type;
        if self.pict_type != PicType::B {
            std::mem::swap(&mut self.next, &mut self.last);
        }
        let cur = self.cur;
        self.frames[cur].valid = false;
        self.frames[cur].key = self.pict_type == PicType::I;
        self.frames[cur].tag = tag;
        self.get_buffer(cur);

        if self.pict_type != PicType::I {
            if !self.frames[self.last].valid {
                self.fill_missing_reference(self.last);
            }
            if self.pict_type == PicType::B && !self.frames[self.next].valid {
                self.fill_missing_reference(self.next);
            }
        }
        self.mb.fill(0);
        self.mb_luma_dc.fill(0);

        if self.pict_type == PicType::B {
            self.frame_num_offset = self.slice_num - self.prev_frame_num;
            if self.frame_num_offset < 0 {
                self.frame_num_offset += 256;
            }
            if self.frame_num_offset == 0 || self.frame_num_offset >= self.prev_frame_num_offset {
                return bad("error in B-frame picture id");
            }
        } else {
            self.prev_frame_num = self.frame_num;
            self.frame_num = self.slice_num;
            self.prev_frame_num_offset = self.frame_num - self.prev_frame_num;
            if self.prev_frame_num_offset < 0 {
                self.prev_frame_num_offset += 256;
            }
        }

        for m in 0..2 {
            for i in 0..4 {
                for j in -1i32..5 {
                    let idx = (usize::from(SCAN8[0]) + 8 * i).wrapping_add_signed(j as isize);
                    if j < 4 {
                        self.ref_cache[m][idx] = 1;
                    } else if i < 3 {
                        self.ref_cache[m][idx] = PART_NOT_AVAILABLE;
                    }
                }
            }
        }

        self.mb_y = 0;
        while self.mb_y < self.mb_height {
            self.mb_x = 0;
            while self.mb_x < self.mb_width {
                self.mb_xy = self.mb_x + self.mb_y * self.mb_stride;
                let left = self.gb_slice.left();
                // a slice of another type than the picture is only reported by FFmpeg
                if left <= 7 && (self.gb_slice.pos & 7 == 0 || self.gb_slice.peek((left & 7) as u32) == 0) {
                    self.slice_header()?;
                }
                let mut mb_type = self.gb_slice.ue();
                if self.pict_type == PicType::I {
                    mb_type = mb_type.wrapping_add(8);
                } else if self.pict_type == PicType::B && mb_type >= 4 {
                    mb_type = mb_type.wrapping_add(4);
                }
                if mb_type > 33 {
                    return bad("macroblock type");
                }
                self.decode_mb(mb_type)?;
                if mb_type != 0 || self.cbp != 0 {
                    self.hl_decode_mb()?;
                }
                if self.pict_type != PicType::B && !self.low_delay {
                    let t = if self.pict_type == PicType::P && mb_type < 8 { mb_type - 1 } else { u32::MAX };
                    self.frames[cur].mb_type[self.mb_xy] = t;
                }
                self.mb_x += 1;
            }
            self.mb_y += 1;
        }

        if data.len() as i64 * 8 - (self.gb_slice.pos as i64) < 0 {
            return bad("frame data overrun");
        }

        let show = if self.pict_type == PicType::B || self.low_delay {
            Some(cur)
        } else if self.frames[self.last].valid {
            Some(self.last)
        } else {
            None
        };
        // no picture right after a seek (the "last" one is not there)
        let out = if self.frames[self.last].valid || self.low_delay { show.map(|s| self.output(s)) } else { None };

        if self.pict_type != PicType::B {
            std::mem::swap(&mut self.cur, &mut self.next);
        } else {
            self.frames[cur].valid = false;
        }
        Ok(out)
    }

    /// Cuts the next slice (header byte, length field, data, and the watermark scrambling) out of
    /// `gb`: its header byte and a reader over its data.
    fn read_slice(&self, gb: &mut Bits) -> Result<(i32, Bits), Error> {
        let header = gb.get(8) as i32;
        if ((header & 0x9F) != 1 && (header & 0x9F) != 2) || (header & 0x60) == 0 {
            return bad("unsupported slice header");
        }
        let length = (header >> 5 & 3) as usize;
        let slice_length = gb.peek(8 * length as u32) as usize;
        let slice_bits = slice_length * 8;
        let slice_bytes = slice_length + length - 1;
        gb.skip(8);
        if slice_bytes as i64 * 8 > gb.left() {
            return bad("slice after bitstream end");
        }
        let start = gb.pos / 8;
        let mut sb = gb.buf[start..start + slice_bytes].to_vec();
        sb.resize(slice_bytes + 8, 0);
        sb.copy_within(slice_length..slice_length + length - 1, 0);
        if self.watermark_key != 0 {
            let h = u32::from_le_bytes([sb[1], sb[2], sb[3], sb[4]]) ^ self.watermark_key;
            sb[1..5].copy_from_slice(&h.to_le_bytes());
        }
        gb.skip(slice_bytes * 8);
        Ok((header, Bits::new(sb, slice_bits)))
    }

    /// The type of the picture in `data` (that of its first slice); `None` if that cannot be
    /// read. Only an intra picture can start decoding, only a B picture can be dropped.
    pub fn frame_type(&self, data: &[u8]) -> Option<PicType> {
        let mut gb = Bits::new(data.to_vec(), data.len() * 8);
        let (_, mut slice) = self.read_slice(&mut gb).ok()?;
        match slice.ue() {
            0 => Some(PicType::P),
            1 => Some(PicType::B),
            2 => Some(PicType::I),
            _ => None,
        }
    }

    fn slice_header(&mut self) -> Result<(), Error> {
        let mb_xy = self.mb_xy;
        let mut gb = std::mem::take(&mut self.gb);
        let r = self.read_slice(&mut gb);
        self.gb = gb;
        let (header, slice) = r?;
        self.gb_slice = slice;

        let slice_id = self.gb_slice.ue();
        self.slice_type = match slice_id {
            0 => PicType::P,
            1 => PicType::B,
            2 => PicType::I,
            _ => return bad("illegal slice type"),
        };
        if (header & 0x9F) == 2 {
            let i = if self.mb_num < 64 { 6 } else { 1 + (self.mb_num - 1).ilog2() };
            self.gb_slice.get(i);
        } else if self.gb_slice.get1() == 1 {
            return Err(Error::Unsupported("media key encryption"));
        }
        self.slice_num = self.gb_slice.get(8) as i32;
        self.qscale = self.gb_slice.get(5) as i32;
        self.adaptive_quant = self.gb_slice.get1() == 1;
        self.gb_slice.skip(1); // unknown
        if self.has_watermark {
            self.gb_slice.skip(1);
        }
        self.gb_slice.skip(1);
        self.gb_slice.skip(2);
        self.gb_slice.skip_1stop_8data_bits()?;

        // reset intra predictors and invalidate motion vector references
        let (mb_x, mb_y, stride, mb_width) = (self.mb_x, self.mb_y, self.mb_stride, self.mb_width);
        if mb_x > 0 {
            self.fill_modes(self.mb2br_xy[mb_xy - 1] + 3, 4);
            self.fill_modes(self.mb2br_xy[mb_xy - mb_x], 8 * mb_x);
        }
        if mb_y > 0 {
            self.fill_modes(self.mb2br_xy[mb_xy - stride], 8 * (mb_width - mb_x));
            if mb_x > 0 {
                let i = self.mb2br_xy[mb_xy - stride - 1] + 3;
                self.fill_modes(i, 1);
            }
        }
        Ok(())
    }

    fn fill_modes(&mut self, start: usize, len: usize) {
        let end = (start + len).min(self.intra4x4_pred_mode.len());
        if start < end {
            self.intra4x4_pred_mode[start..end].fill(-1);
        }
    }

    // --- motion vectors ------------------------------------------------------------------------------

    /// `svq3_pred_motion`: the predicted vector of block `n`.
    fn pred_motion(&self, n: usize, part_width: usize, list: usize, r: i8) -> (i32, i32) {
        let index8 = usize::from(SCAN8[n]);
        let top_ref = self.ref_cache[list][index8 - 8];
        let left_ref = self.ref_cache[list][index8 - 1];
        let a = self.mv_cache[list][index8 - 1];
        let b = self.mv_cache[list][index8 - 8];
        let topright_ref = self.ref_cache[list][index8 - 8 + part_width];
        let (c, diagonal_ref) = if topright_ref != PART_NOT_AVAILABLE {
            (self.mv_cache[list][index8 - 8 + part_width], topright_ref)
        } else {
            (self.mv_cache[list][index8 - 8 - 1], self.ref_cache[list][index8 - 8 - 1])
        };
        let (a0, a1, b0, b1, c0, c1) = (i32::from(a[0]), i32::from(a[1]), i32::from(b[0]), i32::from(b[1]), i32::from(c[0]), i32::from(c[1]));
        let match_count = i32::from(diagonal_ref == r) + i32::from(top_ref == r) + i32::from(left_ref == r);
        if match_count > 1 {
            (mid_pred(a0, b0, c0), mid_pred(a1, b1, c1))
        } else if match_count == 1 {
            if left_ref == r {
                (a0, a1)
            } else if top_ref == r {
                (b0, b1)
            } else {
                (c0, c1)
            }
        } else if top_ref == PART_NOT_AVAILABLE && diagonal_ref == PART_NOT_AVAILABLE && left_ref != PART_NOT_AVAILABLE {
            (a0, a1)
        } else {
            (mid_pred(a0, b0, c0), mid_pred(a1, b1, c1))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mc_dir_part(&mut self, x: i32, y: i32, width: i32, height: i32, mut mx: i32, mut my: i32, dxy: usize, thirdpel: bool, dir: usize, avg: bool) {
        let src = if dir == 0 { self.last } else { self.next };
        let cur = self.cur;
        mx += x;
        my += y;
        if mx < 0 || mx >= self.h_edge_pos - width - 1 || my < 0 || my >= self.v_edge_pos - height - 1 {
            mx = clip(mx, -16, self.h_edge_pos - width + 15);
            my = clip(my, -16, self.v_edge_pos - height + 15);
        }
        let mut tmp = [0u8; 17 * 17];
        let (w, h) = (width as usize, height as usize);
        fetch_block(&self.frames[src].planes[0], mx, my, w, h, self.h_edge_pos, self.v_edge_pos, &mut tmp);
        interpolate(&mut self.frames[cur].planes[0], x as usize, y as usize, &tmp, w, h, dxy, thirdpel, avg);

        mx = (mx + i32::from(mx < x)) >> 1;
        my = (my + i32::from(my < y)) >> 1;
        let (w, h) = (w >> 1, h >> 1);
        for plane in 1..3 {
            fetch_block(&self.frames[src].planes[plane], mx, my, w, h, self.h_edge_pos >> 1, self.v_edge_pos >> 1, &mut tmp);
            interpolate(&mut self.frames[cur].planes[plane], (x >> 1) as usize, (y >> 1) as usize, &tmp, w, h, dxy, thirdpel, avg);
        }
    }

    /// `svq3_mc_dir`: motion vectors and prediction of a macroblock's partitions in one direction.
    fn mc_dir(&mut self, size: i32, mode: i32, dir: usize, avg: bool) -> Result<(), Error> {
        let part_width: i32 = if (size & 5) == 4 { 4 } else { 16 >> (size & 1) };
        let part_height: i32 = 16 >> ((size + 1) as u32 / 3);
        let extra_width = if mode == PREDICT_MODE { -16 * 6 } else { 0 };
        let h_edge_pos = 6 * (self.h_edge_pos - part_width) - extra_width;
        let v_edge_pos = 6 * (self.v_edge_pos - part_height) - extra_width;
        let cur = self.cur;

        let mut i = 0;
        while i < 16 {
            let mut j = 0;
            while j < 16 {
                let b_xy = (4 * self.mb_x + (j >> 2) as usize) + (4 * self.mb_y + (i >> 2) as usize) * self.b_stride;
                let x = 16 * self.mb_x as i32 + j;
                let y = 16 * self.mb_y as i32 + i;
                let k = ((j >> 2 & 1) + (i >> 1 & 2) + (j >> 1 & 4) + (i & 8)) as usize;
                let (mut mx, mut my);
                if mode != PREDICT_MODE {
                    (mx, my) = self.pred_motion(k, (part_width >> 2) as usize, dir, 1);
                } else {
                    let mv = self.frames[self.next].motion_val[0][b_xy];
                    mx = i32::from(mv[0]) * 2;
                    my = i32::from(mv[1]) * 2;
                    if self.prev_frame_num_offset == 0 {
                        return bad("zero frame distance");
                    }
                    if dir == 0 {
                        mx = (mx * self.frame_num_offset / self.prev_frame_num_offset + 1) >> 1;
                        my = (my * self.frame_num_offset / self.prev_frame_num_offset + 1) >> 1;
                    } else {
                        mx = (mx * (self.frame_num_offset - self.prev_frame_num_offset) / self.prev_frame_num_offset + 1) >> 1;
                        my = (my * (self.frame_num_offset - self.prev_frame_num_offset) / self.prev_frame_num_offset + 1) >> 1;
                    }
                }
                // clip motion vector prediction to frame border
                mx = clip(mx, extra_width - 6 * x, h_edge_pos - 6 * x);
                my = clip(my, extra_width - 6 * y, v_edge_pos - 6 * y);

                // get (optional) motion vector differential
                let (dx, dy);
                if mode == PREDICT_MODE {
                    dx = 0;
                    dy = 0;
                } else {
                    dy = self.gb_slice.se();
                    dx = self.gb_slice.se();
                    if dx != i32::from(dx as i16) || dy != i32::from(dy as i16) {
                        return bad("invalid MV vlc");
                    }
                }

                // compute motion vector
                if mode == THIRDPEL_MODE {
                    mx = ((mx + 1) >> 1).wrapping_add(dx);
                    my = ((my + 1) >> 1).wrapping_add(dy);
                    let fx = ((mx.wrapping_add(0x30000)) as u32 / 3) as i32 - 0x10000;
                    let fy = ((my.wrapping_add(0x30000)) as u32 / 3) as i32 - 0x10000;
                    let dxy = (mx - 3 * fx) + 4 * (my - 3 * fy);
                    self.mc_dir_part(x, y, part_width, part_height, fx, fy, dxy as usize, true, dir, avg);
                    mx += mx;
                    my += my;
                } else if mode == HALFPEL_MODE || mode == PREDICT_MODE {
                    mx = (((mx + 1 + 0x30000) as u32 / 3) as i32).wrapping_add(dx).wrapping_sub(0x10000);
                    my = (((my + 1 + 0x30000) as u32 / 3) as i32).wrapping_add(dy).wrapping_sub(0x10000);
                    let dxy = (mx & 1) + 2 * (my & 1);
                    self.mc_dir_part(x, y, part_width, part_height, mx >> 1, my >> 1, dxy as usize, false, dir, avg);
                    mx = mx.wrapping_mul(3);
                    my = my.wrapping_mul(3);
                } else {
                    mx = (((mx + 3 + 0x60000) as u32 / 6) as i32).wrapping_add(dx).wrapping_sub(0x10000);
                    my = (((my + 3 + 0x60000) as u32 / 6) as i32).wrapping_add(dy).wrapping_sub(0x10000);
                    self.mc_dir_part(x, y, part_width, part_height, mx, my, 0, false, dir, avg);
                    mx = mx.wrapping_mul(6);
                    my = my.wrapping_mul(6);
                }

                // update mv_cache
                let mv = [mx as i16, my as i16];
                if mode != PREDICT_MODE {
                    let s8 = usize::from(SCAN8[k]);
                    let cache = &mut self.mv_cache[dir];
                    if part_height == 8 && i < 8 {
                        cache[s8 + 8] = mv;
                        if part_width == 8 && j < 8 {
                            cache[s8 + 1 + 8] = mv;
                        }
                    }
                    if part_width == 8 && j < 8 {
                        cache[s8 + 1] = mv;
                    }
                    if part_width == 4 || part_height == 4 {
                        cache[s8] = mv;
                    }
                }

                // write back motion vectors
                let mvs = &mut self.frames[cur].motion_val[dir];
                for yy in 0..(part_height >> 2) as usize {
                    for xx in 0..(part_width >> 2) as usize {
                        mvs[b_xy + xx + yy * self.b_stride] = mv;
                    }
                }
                j += part_width;
            }
            i += part_height;
        }
        Ok(())
    }

    // --- macroblocks --------------------------------------------------------------------------------------

    fn zero_motion(&mut self, b_xy: usize, lists: usize) {
        for list in 0..lists {
            for i in 0..4 {
                let o = b_xy + i * self.b_stride;
                self.frames[self.cur].motion_val[list][o..o + 4].fill([0, 0]);
            }
        }
    }

    /// `svq3_decode_mb`.
    fn decode_mb(&mut self, mut mb_type: u32) -> Result<(), Error> {
        let mb_xy = self.mb_xy;
        let (mb_x, mb_y, stride) = (self.mb_x, self.mb_y, self.mb_stride);
        let b_xy = 4 * mb_x + 4 * mb_y * self.b_stride;
        let cur = self.cur;
        let mut cbp = 0i32;
        let s0 = usize::from(SCAN8[0]);

        self.top_samples_available = if mb_y == 0 { 0x33FF } else { 0xFFFF };
        self.left_samples_available = if mb_x == 0 { 0x5F5F } else { 0xFFFF };

        if mb_type == 0 {
            // SKIP
            if self.pict_type == PicType::P || self.frames[self.next].mb_type[mb_xy] == u32::MAX {
                let (x, y) = (16 * mb_x as i32, 16 * mb_y as i32);
                self.mc_dir_part(x, y, 16, 16, 0, 0, 0, false, 0, false);
                if self.pict_type == PicType::B {
                    self.mc_dir_part(x, y, 16, 16, 0, 0, 0, false, 1, true);
                }
                mb_type = MB_SKIP;
            } else {
                let t = self.frames[self.next].mb_type[mb_xy].min(6) as i32;
                self.mc_dir(t, PREDICT_MODE, 0, false)?;
                self.mc_dir(t, PREDICT_MODE, 1, true)?;
                mb_type = MB_16X16;
            }
        } else if mb_type < 8 {
            // INTER
            let mode = if self.thirdpel_flag && self.halfpel_flag == (self.gb_slice.get1() == 0) {
                THIRDPEL_MODE
            } else if self.halfpel_flag && self.thirdpel_flag == (self.gb_slice.get1() == 0) {
                HALFPEL_MODE
            } else {
                FULLPEL_MODE
            };

            // fill caches
            for m in 0..2 {
                let left_ok = mb_x > 0 && self.intra4x4_pred_mode[self.mb2br_xy[mb_xy - 1] + 6] != -1;
                for i in 0..4 {
                    self.mv_cache[m][s0 - 1 + i * 8] = if left_ok { self.frames[cur].motion_val[m][b_xy - 1 + i * self.b_stride] } else { [0, 0] };
                }
                if mb_y > 0 {
                    for i in 0..4 {
                        self.mv_cache[m][s0 - 8 + i] = self.frames[cur].motion_val[m][b_xy - self.b_stride + i];
                    }
                    let above = self.intra4x4_pred_mode[self.mb2br_xy[mb_xy - stride]] == -1;
                    self.ref_cache[m][s0 - 8..s0 - 4].fill(if above { PART_NOT_AVAILABLE } else { 1 });

                    if mb_x < self.mb_width - 1 {
                        self.mv_cache[m][s0 + 4 - 8] = self.frames[cur].motion_val[m][b_xy - self.b_stride + 4];
                        self.ref_cache[m][s0 + 4 - 8] =
                            if self.intra4x4_pred_mode[self.mb2br_xy[mb_xy - stride + 1] + 6] == -1 || above { PART_NOT_AVAILABLE } else { 1 };
                    } else {
                        self.ref_cache[m][s0 + 4 - 8] = PART_NOT_AVAILABLE;
                    }
                    if mb_x > 0 {
                        self.mv_cache[m][s0 - 1 - 8] = self.frames[cur].motion_val[m][b_xy - self.b_stride - 1];
                        self.ref_cache[m][s0 - 1 - 8] =
                            if self.intra4x4_pred_mode[self.mb2br_xy[mb_xy - stride - 1] + 3] == -1 { PART_NOT_AVAILABLE } else { 1 };
                    } else {
                        self.ref_cache[m][s0 - 1 - 8] = PART_NOT_AVAILABLE;
                    }
                } else {
                    self.ref_cache[m][s0 - 8 - 1..s0 - 8 - 1 + 8].fill(PART_NOT_AVAILABLE);
                }
                if self.pict_type != PicType::B {
                    break;
                }
            }

            // decode motion vector(s) and form prediction(s)
            if self.pict_type == PicType::P {
                self.mc_dir(mb_type as i32 - 1, mode, 0, false)?;
            } else {
                if mb_type != 2 {
                    self.mc_dir(0, mode, 0, false)?;
                } else {
                    self.zero_motion(b_xy, 1);
                }
                if mb_type != 1 {
                    self.mc_dir(0, mode, 1, mb_type == 3)?;
                } else {
                    for i in 0..4 {
                        let o = b_xy + i * self.b_stride;
                        self.frames[cur].motion_val[1][o..o + 4].fill([0, 0]);
                    }
                }
            }
            mb_type = MB_16X16;
        } else if mb_type == 8 || mb_type == 33 {
            // INTRA4x4
            let i4x4_at = self.mb2br_xy[mb_xy];
            self.intra4x4_pred_mode_cache = [-1; 40];

            if mb_type == 8 {
                if mb_x > 0 {
                    for i in 0..4 {
                        self.intra4x4_pred_mode_cache[s0 - 1 + i * 8] = self.intra4x4_pred_mode[self.mb2br_xy[mb_xy - 1] + 6 - i];
                    }
                    if self.intra4x4_pred_mode_cache[s0 - 1] == -1 {
                        self.left_samples_available = 0x5F5F;
                    }
                }
                if mb_y > 0 {
                    let above = self.mb2br_xy[mb_xy - stride];
                    for i in 0..4 {
                        self.intra4x4_pred_mode_cache[4 + i] = self.intra4x4_pred_mode[above + i];
                    }
                    if self.intra4x4_pred_mode_cache[4] == -1 {
                        self.top_samples_available = 0x33FF;
                    }
                }

                // decode prediction codes for luma blocks
                for i in (0..16).step_by(2) {
                    let vlc = self.gb_slice.ue();
                    if vlc >= 25 {
                        return bad("luma prediction");
                    }
                    let s8 = usize::from(SCAN8[i]);
                    let c = &mut self.intra4x4_pred_mode_cache;
                    let (left0, top0, top1) = (c[s8 - 1], c[s8 - 8], c[s8 - 7]);
                    let pred = |t: i8, l: i8, k: u8| -> Option<i8> {
                        let t = usize::try_from(i32::from(t) + 1).ok()?;
                        let l = usize::try_from(i32::from(l) + 1).ok()?;
                        SVQ3_PRED_1.get(t)?.get(l)?.get(usize::from(k)).copied()
                    };
                    let p = SVQ3_PRED_0[vlc as usize];
                    let v1 = pred(top0, left0, p[0]).unwrap_or(-1);
                    c[s8] = v1;
                    let v2 = pred(top1, v1, p[1]).unwrap_or(-1);
                    c[s8 + 1] = v2;
                    if v1 == -1 || v2 == -1 {
                        return bad("weird prediction");
                    }
                }
            } else {
                // DC_128_PRED block type
                for i in 0..4 {
                    self.intra4x4_pred_mode_cache[s0 + 8 * i..s0 + 8 * i + 4].fill(DC_PRED);
                }
            }

            let c = self.intra4x4_pred_mode_cache;
            let i4 = &mut self.intra4x4_pred_mode;
            i4[i4x4_at..i4x4_at + 4].copy_from_slice(&c[4 + 8 * 4..4 + 8 * 4 + 4]);
            i4[i4x4_at + 4] = c[7 + 8 * 3];
            i4[i4x4_at + 5] = c[7 + 8 * 2];
            i4[i4x4_at + 6] = c[7 + 8];

            if mb_type == 8 {
                check_intra4x4_pred_mode(&mut self.intra4x4_pred_mode_cache, self.top_samples_available, self.left_samples_available);
                self.top_samples_available = if mb_y == 0 { 0x33FF } else { 0xFFFF };
                self.left_samples_available = if mb_x == 0 { 0x5F5F } else { 0xFFFF };
            } else {
                for i in 0..4 {
                    self.intra4x4_pred_mode_cache[s0 + 8 * i..s0 + 8 * i + 4].fill(DC_128_PRED);
                }
                self.top_samples_available = 0x33FF;
                self.left_samples_available = 0x5F5F;
            }
            mb_type = MB_INTRA4X4;
        } else {
            // INTRA16x16
            let (pred_mode, c) = I_MB_TYPE_INFO[mb_type as usize - 8];
            let dir = i32::from(pred_mode);
            let dir = (dir >> 1) ^ (3 * (dir & 1)) ^ 1;
            let Some(m) = check_intra_pred_mode(self.top_samples_available, self.left_samples_available, dir) else {
                return bad("intra 16x16 prediction mode");
            };
            self.intra16x16_pred_mode = m;
            cbp = i32::from(c);
            mb_type = MB_INTRA16X16;
        }

        if !is_inter(mb_type) && self.pict_type != PicType::I {
            self.zero_motion(b_xy, if self.pict_type == PicType::B { 2 } else { 1 });
        }
        if mb_type & MB_INTRA4X4 == 0 {
            let o = self.mb2br_xy[mb_xy];
            self.intra4x4_pred_mode[o..o + 8].fill(DC_PRED);
        }
        if mb_type & MB_SKIP == 0 || self.pict_type == PicType::B {
            self.non_zero_count_cache[8..8 + 14 * 8].fill(0);
        }

        if mb_type & MB_INTRA16X16 == 0 && (mb_type & MB_SKIP == 0 || self.pict_type == PicType::B) {
            let vlc = self.gb_slice.ue();
            if vlc >= 48 {
                return bad("cbp_vlc");
            }
            cbp = i32::from(if is_intra(mb_type) { GOLOMB_TO_INTRA4X4_CBP[vlc as usize] } else { GOLOMB_TO_INTER_CBP[vlc as usize] });
        }
        if mb_type & MB_INTRA16X16 != 0 || (self.pict_type != PicType::I && self.adaptive_quant && cbp != 0) {
            self.qscale += self.gb_slice.se();
            if self.qscale as u32 > 31 {
                return bad("qscale");
            }
        }
        if mb_type & MB_INTRA16X16 != 0 {
            self.mb_luma_dc[..16].fill(0);
            if decode_block(&mut self.gb_slice, &mut self.mb_luma_dc, 0, 1).is_err() {
                return bad("intra luma dc");
            }
        }

        if cbp != 0 {
            let index = usize::from(mb_type & MB_INTRA16X16 != 0);
            let ty = if self.qscale < 24 && mb_type & MB_INTRA4X4 != 0 { 2 } else { 1 };
            for i in 0..4usize {
                if cbp & (1 << i) != 0 {
                    for j in 0..4usize {
                        let k = if index == 1 { (j & 1) + 2 * (i & 1) + 2 * (j & 2) + 4 * (i & 2) } else { 4 * i + j };
                        self.non_zero_count_cache[usize::from(SCAN8[k])] = 1;
                        if decode_block(&mut self.gb_slice, &mut self.mb[16 * k..16 * k + 16], index, ty).is_err() {
                            return bad("block");
                        }
                    }
                }
            }
            if cbp & 0x30 != 0 {
                for i in 1..3usize {
                    if decode_block(&mut self.gb_slice, &mut self.mb[256 * i..256 * i + 64], 0, 3).is_err() {
                        return bad("chroma dc block");
                    }
                }
                if cbp & 0x20 != 0 {
                    for i in 1..3usize {
                        for j in 0..4usize {
                            let k = 16 * i + j;
                            self.non_zero_count_cache[usize::from(SCAN8[k])] = 1;
                            if decode_block(&mut self.gb_slice, &mut self.mb[16 * k..16 * k + 16], 1, 1).is_err() {
                                return bad("chroma ac block");
                            }
                        }
                    }
                }
            }
        }

        self.cbp = cbp;
        self.frames[cur].mb_type[mb_xy] = mb_type;

        if is_intra(mb_type) {
            let Some(m) = check_intra_pred_mode(self.top_samples_available, self.left_samples_available, DC_PRED8X8) else {
                return bad("chroma prediction mode");
            };
            self.chroma_pred_mode = m;
        }
        Ok(())
    }

    /// `hl_decode_mb`: prediction and residual of the macroblock into the current picture.
    fn hl_decode_mb(&mut self) -> Result<(), Error> {
        let (mb_x, mb_y, mb_xy) = (self.mb_x, self.mb_y, self.mb_xy);
        let cur = self.cur;
        let mb_type = self.frames[cur].mb_type[mb_xy];
        let s0 = usize::from(SCAN8[0]);
        let planes = &mut self.frames[cur].planes;
        let ls = planes[0].stride;
        let uvls = planes[1].stride;
        let dest_y = planes[0].at(mb_x * 16, mb_y * 16);
        let dest_c = planes[1].at(mb_x * 8, mb_y * 8);
        let off = |n: usize, stride: usize| {
            let d = usize::from(SCAN8[n]) - s0;
            4 * (d & 7) + 4 * stride * (d >> 3)
        };
        let qscale = self.qscale as usize;

        if is_intra(mb_type) {
            for p in planes[1..3].iter_mut() {
                pred8x8(self.chroma_pred_mode, &mut p.data, dest_c, uvls);
            }
            let y = &mut planes[0].data;
            if mb_type & MB_INTRA4X4 != 0 {
                for i in 0..16 {
                    let ptr = dest_y + off(i, ls);
                    let dir = self.intra4x4_pred_mode_cache[usize::from(SCAN8[i])];
                    let topright = (dir == DIAG_DOWN_LEFT_PRED || dir == VERT_LEFT_PRED).then(|| ptr + 4 - ls);
                    if !(VERT_PRED..=DC_128_PRED).contains(&dir) {
                        return bad("intra 4x4 mode");
                    }
                    pred4x4(dir as usize, y, ptr, ls, topright);
                    if self.non_zero_count_cache[usize::from(SCAN8[i])] != 0 {
                        add_idct(y, ptr, ls, &mut self.mb[i * 16..i * 16 + 16], qscale, 0);
                    }
                }
            } else {
                pred16x16(self.intra16x16_pred_mode, y, dest_y, ls);
                let (mb, dc) = (&mut self.mb, &self.mb_luma_dc);
                luma_dc_dequant_idct(mb, dc, qscale);
            }
        }

        if mb_type & MB_INTRA4X4 == 0 {
            let y = &mut planes[0].data;
            for i in 0..16 {
                if self.non_zero_count_cache[usize::from(SCAN8[i])] != 0 || self.mb[i * 16] != 0 {
                    add_idct(y, dest_y + off(i, ls), ls, &mut self.mb[i * 16..i * 16 + 16], qscale, i32::from(is_intra(mb_type)));
                }
            }
        }

        if self.cbp & 0x30 != 0 {
            chroma_dc_dequant_idct(&mut self.mb[256..], 1024);
            chroma_dc_dequant_idct(&mut self.mb[512..], 1024);
            let cq = (chroma_qp(qscale + 12) - 12) as usize;
            for j in 1..3usize {
                let data = &mut planes[j].data;
                for i in j * 16..j * 16 + 4 {
                    if self.non_zero_count_cache[usize::from(SCAN8[i])] != 0 || self.mb[i * 16] != 0 {
                        add_idct(data, dest_c + off(i & 3, uvls), uvls, &mut self.mb[i * 16..i * 16 + 16], cq, 2);
                    }
                }
            }
        }
        Ok(())
    }
}
