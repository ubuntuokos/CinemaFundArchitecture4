//! Slice data decoding: macroblock parsing (CAVLC), motion vector derivation, and reconstruction.

use crate::cavlc;
use crate::deblock;
use crate::error::{Result, ensure, invalid};
use crate::inter::{self, Weight};
use crate::intra::{self, Avail};
use crate::mbtypes::*;
use crate::params::{Pps, Sps};
use crate::picture::*;
use crate::slice::{SliceHeader, SliceType};
use crate::tables::*;
use crate::transform::{self, LevelScale};
use cfa4_bitstream::BitReader;

/// Per-slice data kept with the picture (deblocking, motion field export).
#[derive(Clone, Debug, Default)]
pub struct SliceInfo {
    pub disable_deblocking_filter_idc: u32,
    pub alpha_offset: i32,
    pub beta_offset: i32,
    /// [`RefPic::key`] of the entries of RefPicList0/1.
    pub ref_ids: [Vec<u32>; 2],
    /// MBAFF frame: the reference indices of field macroblocks select fields of the list
    /// entries (8.4.2.1).
    pub mbaff: bool,
}

impl SliceInfo {
    /// Identity ([`RefPic::key`]) of the picture that reference index `ri` of list `l` selects
    /// for macroblock `st` (`u32::MAX` for none, `u32::MAX - 1` when out of range).
    #[inline]
    pub fn ref_key(&self, st: &MbState, l: usize, ri: i8) -> u32 {
        if ri < 0 {
            return u32::MAX;
        }
        if self.mbaff && st.field {
            let Some(&k) = self.ref_ids[l].get((ri >> 1) as usize) else { return u32::MAX - 1 };
            return (k & !3) | ((st.bottom as u32) ^ (ri as u32 & 1));
        }
        self.ref_ids[l].get(ri as usize).copied().unwrap_or(u32::MAX - 1)
    }
}

/// The picture under construction: private reconstruction buffers plus the shared [`Frame`] that
/// finished (deblocked) macroblock rows are published into.
pub struct PicState<S> {
    /// The frame, or one field of it for a field picture.
    pub planes: Planes<S>,
    /// Per macroblock address (MBAFF: `2 * pair + bottom`).
    pub mbs: Vec<MbState>,
    pub slices: Vec<SliceInfo>,
    pub mb_w: usize,
    /// Macroblock rows of this picture (half the frame's for a field).
    pub mb_h: usize,
    pub poc: i32,
    pub decoded_mbs: usize,
    pub frame: FrameRef,
    /// Bits per sample (8, or 9..=14 with `S = u16`).
    pub bit_depth: u32,
    /// Largest sample value, `(1 << bit_depth) - 1`.
    pub max: i32,
    /// Field picture: the field (0 top, 1 bottom) of `frame` being decoded.
    pub field: Option<u8>,
    /// MBAFF frame: macroblocks are decoded, deblocked and published in pairs.
    pub mbaff: bool,
    /// Decoded macroblocks per tracking row (a macroblock row, or a pair row in MBAFF frames).
    row_count: Vec<u16>,
    /// Leading tracking rows that are fully reconstructed / deblocked / published.
    rows_recon: usize,
    rows_deblocked: usize,
    rows_published: usize,
    /// Draft mode for a non-reference picture: rows are published without deblocking.
    pub skip_deblock: bool,
}

impl<S: deblock::Lanes> PicState<S> {
    pub fn new(frame: FrameRef, field: Option<u8>, mbaff: bool) -> Self {
        let mb_w = frame.mb_w;
        let mb_h = if field.is_some() { frame.mb_h() / 2 } else { frame.mb_h() };
        let bit_depth = frame.bit_depth;
        let mbaff = mbaff && field.is_none() && mb_h % 2 == 0;
        let rows = if mbaff { mb_h / 2 } else { mb_h };
        PicState {
            planes: Planes::new(mb_w * 16, mb_h * 16, bit_depth, frame.chroma),
            mbs: vec![MbState::default(); mb_w * mb_h],
            slices: Vec::new(),
            mb_w,
            mb_h,
            poc: frame.poc,
            decoded_mbs: 0,
            frame,
            bit_depth,
            max: (1i32 << bit_depth) - 1,
            field,
            mbaff,
            row_count: vec![0; rows],
            rows_recon: 0,
            rows_deblocked: 0,
            rows_published: 0,
            skip_deblock: false,
        }
    }

    /// Reuse the buffers of a finished picture of the same size for a new frame.
    pub fn reset(&mut self, frame: FrameRef, field: Option<u8>, mbaff: bool) {
        // start_mb() re-initialises each macroblock before use; only availability must be reset
        for m in &mut self.mbs {
            m.slice_num = u32::MAX;
        }
        self.slices.clear();
        self.poc = frame.poc;
        self.decoded_mbs = 0;
        self.frame = frame;
        self.field = field;
        self.mbaff = mbaff && field.is_none() && self.mb_h.is_multiple_of(2);
        self.row_count = vec![0; if self.mbaff { self.mb_h / 2 } else { self.mb_h }];
        self.rows_recon = 0;
        self.rows_deblocked = 0;
        self.rows_published = 0;
        self.skip_deblock = false;
    }

    /// Macroblock rows of the frame (field) a picture of `frame` with `field` decodes.
    pub fn rows_for(frame: &Frame, field: Option<u8>) -> usize {
        if field.is_some() { frame.mb_h() / 2 } else { frame.mb_h() }
    }

    /// Macroblocks per tracking row.
    #[inline]
    fn row_mbs(&self) -> usize {
        if self.mbaff { 2 * self.mb_w } else { self.mb_w }
    }

    /// Record a decoded macroblock and deblock / publish rows that became final.
    #[inline]
    pub fn mb_done(&mut self, addr: usize) {
        self.decoded_mbs += 1;
        let per = self.row_mbs();
        let r = addr / per;
        let Some(c) = self.row_count.get_mut(r) else { return };
        *c += 1;
        if *c as usize >= per && r == self.rows_recon {
            while self.rows_recon < self.row_count.len() && self.row_count[self.rows_recon] as usize >= per {
                self.rows_recon += 1;
            }
            self.flush_rows(false);
        }
    }

    /// Deblock and publish rows. With `finished`, everything remaining is processed (also rows with
    /// missing macroblocks, so that waiting readers never block forever).
    pub fn flush_rows(&mut self, finished: bool) {
        let nrows = self.row_count.len();
        let per = self.row_mbs();
        // Deblocking row r must wait until row r + 1 is reconstructed (its intra prediction reads the
        // unfiltered bottom line of row r); row r is final once row r + 1 is deblocked.
        let deblock_limit = if finished { nrows } else { self.rows_recon.saturating_sub(1) };
        while self.rows_deblocked < deblock_limit {
            let r = self.rows_deblocked;
            if !self.skip_deblock {
                // in order of increasing macroblock addresses (8.7)
                for addr in r * per..(r + 1) * per {
                    deblock::deblock_mb(self, addr);
                }
            }
            self.rows_deblocked += 1;
        }
        let publish_limit = if finished { nrows } else { self.rows_deblocked.saturating_sub(1) };
        while self.rows_published < publish_limit {
            let r = self.rows_published;
            self.rows_published += 1;
            match (self.field, self.mbaff, self.frame.interlaced) {
                (Some(p), _, _) => self.publish_unit(2 * r + p as usize, RowLines { first: 16 * r, cfirst: self.planes.chroma.mb_h() * r, step: 1 }, r, 1),
                (None, true, _) => {
                    for p in 0..2 {
                        self.publish_unit(2 * r + p, self.frame.unit_lines(2 * r + p), r, p);
                    }
                }
                // a pair of macroblock rows holds both fields' lines of two row units
                (None, false, true) => {
                    if r % 2 == 1 || finished {
                        let u = r / 2;
                        for i in [2 * u, 2 * u + 1] {
                            self.publish_unit(i, self.frame.unit_lines(i), i, 1);
                        }
                    }
                }
                (None, false, false) => self.publish_unit(r, self.frame.unit_lines(r), r, 1),
            }
        }
    }

    /// Publish row unit `unit` of the frame from `lines` of the planes, with the motion of
    /// macroblock row `row` (MBAFF: `bottom` 0/1 selects the pair row's top or bottom
    /// macroblocks; otherwise `bottom` is ignored).
    fn publish_unit(&self, unit: usize, lines: RowLines, row: usize, bottom: usize) {
        if self.frame.is_published(unit) {
            return;
        }
        let slices = &self.slices;
        let ids = |st: &MbState, l: usize, ri: i8| slices.get(st.slice_num as usize).map_or(u32::MAX, |s| s.ref_key(st, l, ri));
        let (mb_w, mbaff) = (self.mb_w, self.mbaff);
        let mbs = &self.mbs;
        let mb_at = |x: usize| if mbaff { mbs.get(2 * (row * mb_w + x) + bottom) } else { mbs.get(row * mb_w + x) };
        self.frame.publish(unit, Frame::make_row(&self.planes, lines, &mb_at, &ids));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WpMode {
    Default,
    Explicit,
    Implicit,
}

/// Prediction weight tables of a slice.
struct Weights {
    mode: WpMode,
    luma_denom: i32,
    chroma_denom: i32,
    /// [list][ref] -> (weight, offset) for luma and two chroma components.
    luma: [Vec<(i32, i32)>; 2],
    chroma: [Vec<[(i32, i32); 2]>; 2],
    /// Implicit weights (w0, w1) indexed by ref0 * n1 + ref1.
    implicit: Vec<(i32, i32)>,
    n1: usize,
}

/// Current macroblock syntax (prediction part).
#[derive(Clone, Copy, Default)]
pub(crate) struct MbCur {
    pub info: MbTypeInfo,
    pub sub: [SubMbInfo; 4],
    /// rem_intra_pred_mode per block, or -1 when prev_intra_pred_mode_flag was set.
    pub rem_mode: [i8; 16],
    pub cbp: u8,
    pub ref_idx: [[i8; 4]; 2],
    /// mvd per (partition * 4 + sub-partition).
    pub mvd: [[[i16; 2]; 16]; 2],
}

/// Coefficient scratch for one macroblock.
pub(crate) struct Scratch<S> {
    /// Per colour component Y, Cb, Cr: 16 4x4 blocks (raster block index * 16 + raster coefficient,
    /// the raster index of 4:2:0 / 4:2:2 chroma blocks counting four blocks per row) or 4 8x8 blocks
    /// (b8 * 64 + raster).
    pub coef: [[i32; 256]; 3],
    /// Blocks with coefficients per component: raster 4x4 bits, or b8 bits in 8x8 mode.
    pub blk_nz: [u16; 3],
    pub pred: [[S; 256]; 2],
    /// Chroma predictions [list][Cb, Cr], stride 16.
    pub pred_c: [[[S; 256]; 2]; 2],
}

impl<S: Sample> Scratch<S> {
    pub fn new() -> Box<Self> {
        Box::new(Scratch { coef: [[0; 256]; 3], blk_nz: [0; 3], pred: [[S::default(); 256]; 2], pred_c: [[[S::default(); 256]; 2]; 2] })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Nb {
    pub avail: bool,
    pub ref_idx: i8,
    pub mv: [i16; 2],
}

const NB_NA: Nb = Nb { avail: false, ref_idx: -1, mv: [0, 0] };

/// vertMvScale of a co-located block (8.4.1.2.1).
#[derive(Clone, Copy, PartialEq, Eq)]
enum VScale {
    One,
    FrmToFld,
    FldToFrm,
}

pub struct SliceDecoder<'a, S> {
    pub sh: &'a SliceHeader,
    pub pps: &'a Pps,
    pub sps: &'a Sps,
    pub pic: &'a mut PicState<S>,
    pub refs: &'a [Vec<RefPic>; 2],
    pub ls: &'a LevelScale,
    pub slice_num: u32,
    pub mb_w: usize,
    pub mb_h: usize,
    pub qp: i32,
    pub chroma_qp_offset: [i32; 2],
    // current macroblock
    pub mb_addr: usize,
    pub mb_x: usize,
    pub mb_y: usize,
    /// Neighbouring macroblocks A (left), B (above), C (above right), D (above left) when available.
    pub nb: [Option<usize>; 4],
    pub mv_done: u16,
    pub cur: MbCur,
    pub s: Box<Scratch<S>>,
    weights: Weights,
    /// Temporal direct: DistScaleFactor per refIdxL0, and whether pic0 is long-term.
    tdirect: Vec<(i32, bool)>,
    /// CABAC: previous MB in decoding order had a non-zero mb_qp_delta.
    pub prev_qp_delta_nz: bool,
    /// QpBdOffsetY = QpBdOffsetC = 6 * (BitDepth - 8): QP'  = QPY + offset, and QPY itself may be
    /// as low as -offset (7.4.5).
    pub qp_bd_offset: i32,
    /// Intra prediction constants of this bit depth.
    depth: intra::Depth,
    /// Chroma sampling (ChromaArrayType).
    pub chroma: Chroma,
    /// MbaffFrameFlag: the picture is coded in macroblock pairs.
    pub mbaff: bool,
    /// Field picture: its parity (0 top, 1 bottom).
    pub pic_field: Option<u8>,
    /// The current macroblock is a field macroblock (of an MBAFF field pair or a field picture).
    pub field_mb: bool,
    /// MBAFF: the current macroblock is the bottom macroblock of its pair.
    pub bottom: bool,
    /// MBAFF: top macroblock addresses of the neighbouring pairs A, B, C, D (6.4.9) when available.
    pub(crate) pair_nb: [Option<usize>; 4],
    /// Where the samples of the current macroblock are: luma, chroma.
    view: [View; 2],
    /// Inverse scans of the current macroblock: zig-zag, or the field scans (8.5.6, 8.5.7).
    pub scan4: &'static [u8; 16],
    pub scan8: &'static [u8; 64],
    /// MBAFF field macroblocks of each parity: implicit weights (indexed like `Weights::implicit`
    /// over the field lists) and temporal direct DistScaleFactors per field refIdxL0.
    implicit_field: [Vec<(i32, i32)>; 2],
    tdirect_field: [Vec<(i32, bool)>; 2],
}

/// Sample addressing of one colour component of the current macroblock: its samples are
/// `plane[off + (y0 + y) * stride + x0 + x]`. Field macroblocks of MBAFF frames address the lines
/// of their field (`off` selects the parity, `stride` skips the other field's lines), so that the
/// line above is the same field's line above, as the neighbour derivation of Table 6-4 specifies.
#[derive(Clone, Copy, Default)]
struct View {
    off: usize,
    stride: usize,
    x0: usize,
    y0: usize,
}

/// 4x4 field scan (Table 8-13), as raster positions.
pub const FIELD4: [u8; 16] = [0, 4, 1, 8, 12, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15];
/// 8x8 field scan (Table 8-14), as raster positions.
#[rustfmt::skip]
pub const FIELD8: [u8; 64] = [
    0, 8, 16, 1, 9, 24, 32, 17, 2, 25, 40, 48, 56, 33, 10, 3,
    18, 41, 49, 57, 26, 11, 4, 19, 34, 42, 50, 58, 27, 12, 5, 20,
    35, 43, 51, 59, 28, 13, 6, 21, 36, 44, 52, 60, 29, 14, 22, 37,
    45, 53, 61, 30, 7, 15, 38, 46, 54, 62, 23, 31, 39, 47, 55, 63,
];

/// Interpolate a bw x bh luma block at (px, py) with motion vector `mv` from reference `f` (the
/// frame, or its field `field`) into `y`, and the corresponding chroma blocks (8.4.2.2) into `c`
/// (stride `cs`). `cdy` is added to the vertical vector of 4:2:0 chroma (Table 8-10).
#[allow(clippy::too_many_arguments)]
#[inline]
fn predict_block<S: Sample>(
    f: &Frame,
    field: Option<u8>,
    mv: [i16; 2],
    cdy: i16,
    px: usize,
    py: usize,
    bw: usize,
    bh: usize,
    y: (&mut [S], usize),
    c: [&mut [S]; 2],
    cs: usize,
    max: i32,
) {
    const LW: usize = 24;
    let mut win = [S::default(); LW * 21];
    let ix = px as i32 + (mv[0] as i32 >> 2);
    let iy = py as i32 + (mv[1] as i32 >> 2);
    f.luma_window(field, ix - 2, iy - 2, bw + 5, bh + 5, &mut win, LW);
    inter::mc_luma_win(&win, LW, (mv[0] & 3) as u32, (mv[1] & 3) as u32, bw, bh, y.0, y.1, max);
    if f.chroma.0 != 1 {
        predict_chroma_other(f, field, mv, px, py, bw, bh, c, cs, max);
        return;
    }
    let mvy = mv[1].wrapping_add(cdy);
    let cx = (px / 2) as i32 + (mv[0] as i32 >> 3);
    let cy = (py / 2) as i32 + (mvy as i32 >> 3);
    let (cw, ch) = (bw / 2, bh / 2);
    let (fx, fy) = ((mv[0] & 7) as u32, (mvy & 7) as u32);
    for (comp, out) in c.into_iter().enumerate() {
        let mut w = [S::default(); 16 * 9];
        f.chroma_window::<S, 16, 3>(field, comp, cx, cy, cw + 1, ch + 1, &mut w, 16);
        inter::mc_chroma_win(&w, 16, fx, fy, cw, ch, out, cs);
    }
}

/// Chroma part of [`predict_block`] for 4:2:2, 4:4:4 and monochrome (kept out of the 4:2:0 path).
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn predict_chroma_other<S: Sample>(
    f: &Frame,
    field: Option<u8>,
    mv: [i16; 2],
    px: usize,
    py: usize,
    bw: usize,
    bh: usize,
    c: [&mut [S]; 2],
    cs: usize,
    max: i32,
) {
    match f.chroma.0 {
        // 4:4:4: Cb and Cr use the luma interpolation
        3 => {
            const LW: usize = 24;
            let mut win = [S::default(); LW * 21];
            let ix = px as i32 + (mv[0] as i32 >> 2);
            let iy = py as i32 + (mv[1] as i32 >> 2);
            let (fx, fy) = ((mv[0] & 3) as u32, (mv[1] & 3) as u32);
            for (comp, out) in c.into_iter().enumerate() {
                f.chroma_window::<S, LW, 4>(field, comp, ix - 2, iy - 2, bw + 5, bh + 5, &mut win, LW);
                inter::mc_luma_win(&win, LW, fx, fy, bw, bh, out, cs, max);
            }
        }
        // 4:2:2: full vertical resolution, the quarter-sample vertical vector in eighths (8-229, 8-230)
        2 => {
            let cx = (px / 2) as i32 + (mv[0] as i32 >> 3);
            let cy = py as i32 + (mv[1] as i32 >> 2);
            let (fx, fy) = ((mv[0] & 7) as u32, ((mv[1] & 3) << 1) as u32);
            for (comp, out) in c.into_iter().enumerate() {
                let mut w = [S::default(); 16 * 17];
                f.chroma_window::<S, 16, 4>(field, comp, cx, cy, bw / 2 + 1, bh + 1, &mut w, 16);
                inter::mc_chroma_win(&w, 16, fx, fy, bw / 2, bh, out, cs);
            }
        }
        // monochrome: the neutral chroma planes are never written
        _ => {}
    }
}

/// TotalCoeff (CAVLC) or coded_block_flag (CABAC) per 4x4 block of colour component `p`.
#[inline(always)]
pub(crate) fn nnz_of(st: &MbState, p: usize) -> &[u8; 16] {
    match p {
        1 => &st.nnz_c[0],
        2 => &st.nnz_c[1],
        _ => &st.nnz,
    }
}

#[inline(always)]
pub(crate) fn nnz_of_mut(st: &mut MbState, p: usize) -> &mut [u8; 16] {
    match p {
        1 => &mut st.nnz_c[0],
        2 => &mut st.nnz_c[1],
        _ => &mut st.nnz,
    }
}

/// The sample plane of colour component `p` (0 = Y, 1 = Cb, 2 = Cr) and its stride.
#[inline(always)]
fn plane_of<S>(pl: &mut Planes<S>, p: usize) -> (&mut [S], usize) {
    match p {
        0 => (&mut pl.y[..], pl.width),
        1 => (&mut pl.cb[..], pl.cwidth),
        _ => (&mut pl.cr[..], pl.cwidth),
    }
}

/// Temporal direct DistScaleFactor (8-199, 8-200) for the current picture `cur` between `poc0`
/// and `poc1`, and whether mvL0 = mvCol is used instead (long-term or equal POCs).
fn dist_scale(cur: i32, poc0: i32, poc1: i32, long_term: bool) -> (i32, bool) {
    let tb = (cur - poc0).clamp(-128, 127);
    let td = (poc1 - poc0).clamp(-128, 127);
    if td == 0 || long_term {
        (256, true)
    } else {
        let tx = (16384 + (td / 2).abs()) / td;
        (((tb * tx + 32) >> 6).clamp(-1024, 1023), false)
    }
}

/// Implicit bi-prediction weights (w0, w1) (8.4.3).
fn implicit_weights(cur: i32, poc0: i32, poc1: i32, long_term: bool) -> (i32, i32) {
    let tb = (cur - poc0).clamp(-128, 127);
    let td = (poc1 - poc0).clamp(-128, 127);
    if td == 0 || long_term {
        return (32, 32);
    }
    let tx = (16384 + (td / 2).abs()) / td;
    let dsf = ((tb * tx + 32) >> 6).clamp(-1024, 1023);
    if (dsf >> 2) < -64 || (dsf >> 2) > 128 { (32, 32) } else { (64 - (dsf >> 2), dsf >> 2) }
}

#[inline(always)]
fn median(a: i16, b: i16, c: i16) -> i16 {
    a.max(b).min(a.min(b).max(c))
}

impl<'a, S: deblock::Lanes> SliceDecoder<'a, S> {
    /// `field_poc`: TopFieldOrderCnt and BottomFieldOrderCnt of the current frame (or the field's
    /// POC twice for a field picture).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sh: &'a SliceHeader,
        pps: &'a Pps,
        sps: &'a Sps,
        pic: &'a mut PicState<S>,
        refs: &'a [Vec<RefPic>; 2],
        ls: &'a LevelScale,
        field_poc: [i32; 2],
    ) -> Result<Self> {
        let slice_num = pic.slices.len() as u32;
        let chroma_qp_offset = [pps.chroma_qp_index_offset, pps.second_chroma_qp_index_offset];
        let mbaff = pic.mbaff;
        pic.slices.push(SliceInfo {
            disable_deblocking_filter_idc: sh.disable_deblocking_filter_idc,
            alpha_offset: sh.slice_alpha_c0_offset_div2 * 2,
            beta_offset: sh.slice_beta_offset_div2 * 2,
            ref_ids: [refs[0].iter().map(|r| r.key()).collect(), refs[1].iter().map(|r| r.key()).collect()],
            mbaff,
        });
        let weights = Self::build_weights(sh, pps, refs, pic.poc, sps.bit_depth_luma);
        let mut tdirect = Vec::new();
        let temporal = sh.slice_type == SliceType::B && !sh.direct_spatial_mv_pred && !refs[1].is_empty();
        if temporal {
            let poc1 = refs[1][0].poc();
            tdirect = refs[0].iter().map(|r0| dist_scale(pic.poc, r0.poc(), poc1, r0.long_term)).collect();
        }
        // MBAFF field macroblocks predict from the fields of the list entries: entry 2i is the
        // field of entry i with the macroblock's parity, 2i + 1 the other one (8.4.2.1)
        let mut implicit_field = [Vec::new(), Vec::new()];
        let mut tdirect_field = [Vec::new(), Vec::new()];
        if mbaff {
            for parity in 0..2 {
                let fpoc = |r: &RefPic, i: usize| r.field_poc[parity ^ (i & 1)];
                let cur = field_poc[parity];
                if weights.mode == WpMode::Implicit {
                    for i0 in 0..2 * refs[0].len() {
                        for i1 in 0..2 * refs[1].len() {
                            let (r0, r1) = (&refs[0][i0 / 2], &refs[1][i1 / 2]);
                            implicit_field[parity].push(implicit_weights(cur, fpoc(r0, i0), fpoc(r1, i1), r0.long_term || r1.long_term));
                        }
                    }
                }
                if temporal {
                    let poc1 = refs[1][0].field_poc[parity];
                    tdirect_field[parity] = (0..2 * refs[0].len()).map(|i| dist_scale(cur, fpoc(&refs[0][i / 2], i), poc1, refs[0][i / 2].long_term)).collect();
                }
            }
        }
        let (mb_w, mb_h) = (pic.mb_w, pic.mb_h);
        let chroma = pic.planes.chroma;
        let pic_field = pic.field;
        Ok(SliceDecoder {
            sh,
            pps,
            sps,
            pic,
            refs,
            ls,
            slice_num,
            mb_w,
            mb_h,
            qp: sh.qp(pps),
            chroma_qp_offset,
            mb_addr: 0,
            mb_x: 0,
            mb_y: 0,
            nb: [None; 4],
            mv_done: 0,
            cur: MbCur::default(),
            s: Scratch::new(),
            weights,
            tdirect,
            prev_qp_delta_nz: false,
            qp_bd_offset: 6 * (sps.bit_depth_luma - 8) as i32,
            depth: intra::Depth::new(sps.bit_depth_luma),
            chroma,
            mbaff,
            pic_field,
            field_mb: pic_field.is_some(),
            bottom: false,
            pair_nb: [None; 4],
            view: [View::default(); 2],
            scan4: if pic_field.is_some() { &FIELD4 } else { &ZIGZAG4 },
            scan8: if pic_field.is_some() { &FIELD8 } else { &ZIGZAG8 },
            implicit_field,
            tdirect_field,
        })
    }

    fn build_weights(sh: &SliceHeader, pps: &Pps, refs: &[Vec<RefPic>; 2], cur_poc: i32, bit_depth: u32) -> Weights {
        // explicit offsets are in units of 1 << (BitDepth - 8) (8.4.2.3.2)
        let os = 1i32 << bit_depth.saturating_sub(8);
        let mut w = Weights {
            mode: WpMode::Default,
            luma_denom: 0,
            chroma_denom: 0,
            luma: [Vec::new(), Vec::new()],
            chroma: [Vec::new(), Vec::new()],
            implicit: Vec::new(),
            n1: refs[1].len(),
        };
        let explicit = (pps.weighted_pred && sh.slice_type.is_p()) || (pps.weighted_bipred_idc == 1 && sh.slice_type.is_b());
        if explicit {
            if let Some(t) = &sh.pred_weight_table {
                w.mode = WpMode::Explicit;
                w.luma_denom = t.luma_log2_denom as i32;
                w.chroma_denom = t.chroma_log2_denom as i32;
                for (l, entries) in [&t.l0, &t.l1].into_iter().enumerate() {
                    w.luma[l] = entries.iter().map(|e| (e.luma_weight, e.luma_offset * os)).collect();
                    w.chroma[l] =
                        entries.iter().map(|e| [(e.chroma_weight[0], e.chroma_offset[0] * os), (e.chroma_weight[1], e.chroma_offset[1] * os)]).collect();
                }
            }
        } else if pps.weighted_bipred_idc == 2 && sh.slice_type.is_b() {
            w.mode = WpMode::Implicit;
            for r0 in refs[0].iter() {
                for r1 in refs[1].iter() {
                    w.implicit.push(implicit_weights(cur_poc, r0.poc(), r1.poc(), r0.long_term || r1.long_term));
                }
            }
        }
        w
    }

    // ------------------------------------------------------------------------------------------
    // Neighbour helpers
    // ------------------------------------------------------------------------------------------

    /// Set up the current macroblock address and neighbour availability. In MBAFF frames
    /// `self.field_mb` must hold the macroblock's mb_field_decoding_flag.
    pub fn start_mb(&mut self, addr: usize) {
        self.mb_addr = addr;
        self.mv_done = 0;
        let sn = self.slice_num;
        let w = self.mb_w;
        if !self.mbaff {
            self.mb_x = addr % w;
            self.mb_y = addr / w;
            let mbs = &self.pic.mbs;
            let (x, y) = (self.mb_x, self.mb_y);
            let ok = |a: usize| if mbs[a].slice_num == sn { Some(a) } else { None };
            self.nb = [
                if x > 0 { ok(addr - 1) } else { None },
                if y > 0 { ok(addr - w) } else { None },
                if y > 0 && x + 1 < w { ok(addr - w + 1) } else { None },
                if y > 0 && x > 0 { ok(addr - w - 1) } else { None },
            ];
            let (field, bottom) = (self.pic_field.is_some(), self.pic_field == Some(1));
            self.pic.mbs[addr] = MbState { slice_num: sn, field, bottom, ..MbState::default() };
            self.set_views();
            return;
        }
        // MBAFF: address 2 * pair + bottom (6.4.10), mb_y counts frame macroblock rows
        let pair = addr / 2;
        self.bottom = addr & 1 == 1;
        self.mb_x = pair % w;
        let py = pair / w;
        self.mb_y = 2 * py + self.bottom as usize;
        let x = self.mb_x;
        let mbs = &self.pic.mbs;
        let ok = |p: usize| if mbs.get(2 * p).is_some_and(|m| m.slice_num == sn) { Some(2 * p) } else { None };
        self.pair_nb = [
            if x > 0 { ok(pair - 1) } else { None },
            if py > 0 { ok(pair - w) } else { None },
            if py > 0 && x + 1 < w { ok(pair - w + 1) } else { None },
            if py > 0 && x > 0 { ok(pair - w - 1) } else { None },
        ];
        self.pic.mbs[addr] = MbState { slice_num: sn, field: self.field_mb, bottom: self.bottom, ..MbState::default() };
        self.nb = [(-1, 0), (0, -1), (16, -1), (-1, -1)].map(|(x, y)| self.nb_loc_mbaff(x, y, 16, 16).map(|n| n.0));
        (self.scan4, self.scan8) = if self.field_mb { (&FIELD4, &FIELD8) } else { (&ZIGZAG4, &ZIGZAG8) };
        self.set_views();
    }

    /// The inferred mb_field_decoding_flag of the pair whose top macroblock is `top` (7.4.4):
    /// that of the left pair, else of the pair above (in the same slice), else frame.
    pub fn infer_field(&self, top: usize) -> bool {
        let (pair, w, sn) = (top / 2, self.mb_w, self.slice_num);
        let get = |p: usize| self.pic.mbs.get(2 * p).filter(|m| m.slice_num == sn).map(|m| m.field);
        let left = if pair % w > 0 { get(pair - 1) } else { None };
        let above = if pair >= w { get(pair - w) } else { None };
        left.or(above).unwrap_or(false)
    }

    /// Sample addressing of the current macroblock (see [`View`]).
    fn set_views(&mut self) {
        let (w, cw) = (self.pic.planes.width, self.pic.planes.cwidth);
        let (cmw, cmh) = (self.chroma.mb_w(), self.chroma.mb_h());
        let x = self.mb_x;
        self.view = if self.mbaff && self.field_mb {
            let (py, b) = (self.mb_y / 2, self.bottom as usize);
            [View { off: b * w, stride: 2 * w, x0: x * 16, y0: py * 16 }, View { off: b * cw, stride: 2 * cw, x0: x * cmw, y0: py * cmh }]
        } else {
            [View { off: 0, stride: w, x0: x * 16, y0: self.mb_y * 16 }, View { off: 0, stride: cw, x0: x * cmw, y0: self.mb_y * cmh }]
        };
    }

    /// The neighbouring location (6.4.12) of location (xn, yn) relative to the current macroblock,
    /// for a component with `maxw` x `maxh` samples per macroblock: the macroblock containing it
    /// (the current one for locations inside it) and (xW, yW) in that macroblock.
    #[inline]
    pub(crate) fn nb_loc(&self, xn: i32, yn: i32, maxw: i32, maxh: i32) -> Option<(usize, usize, usize)> {
        if self.mbaff {
            return self.nb_loc_mbaff(xn, yn, maxw, maxh);
        }
        let n = if xn < 0 {
            if yn < 0 {
                3
            } else if yn < maxh {
                0
            } else {
                return None;
            }
        } else if xn < maxw {
            if yn < 0 {
                1
            } else if yn < maxh {
                return Some((self.mb_addr, xn as usize, yn as usize));
            } else {
                return None;
            }
        } else if yn < 0 {
            2
        } else {
            return None;
        };
        let m = self.nb[n]?;
        Some((m, xn.rem_euclid(maxw) as usize, yn.rem_euclid(maxh) as usize))
    }

    /// [`Self::nb_loc`] in MBAFF frames (6.4.12.2, Table 6-4). Every neighbouring location is the
    /// physically adjacent sample of the current macroblock's frame or field lines, so the
    /// location is mapped to a line of the macroblock pair rows and from there into the
    /// macroblock (frame or field) of the pair containing it.
    fn nb_loc_mbaff(&self, xn: i32, yn: i32, maxw: i32, maxh: i32) -> Option<(usize, usize, usize)> {
        let ph = 2 * maxh;
        let b = self.bottom as i32;
        // line relative to the top line of the current pair
        let yf = if self.field_mb { b + 2 * yn } else { maxh * b + yn };
        if yf >= ph {
            return None;
        }
        let cur_top = self.mb_addr & !1;
        let (top, xw, yp) = if yf < 0 {
            let n = if xn < 0 {
                3
            } else if xn < maxw {
                1
            } else {
                2
            };
            (self.pair_nb[n]?, xn.rem_euclid(maxw), yf + ph)
        } else if xn < 0 {
            (self.pair_nb[0]?, xn + maxw, yf)
        } else if xn < maxw {
            (cur_top, xn, yf)
        } else {
            return None;
        };
        let field = if top == cur_top { self.field_mb } else { self.pic.mbs.get(top)?.field };
        let (m, yw) = if field { (top + (yp & 1) as usize, yp >> 1) } else { (top + (yp >= maxh) as usize, yp % maxh) };
        // the other macroblock of the current pair is only available when decoded before it
        if top == cur_top && m > self.mb_addr {
            return None;
        }
        Some((m, xw as usize, yw as usize))
    }

    /// Luma location (x, y) relative to the current macroblock is available for intra prediction
    /// (constrained_intra_pred aware; locations inside the current macroblock count as available).
    fn intra_avail_at(&self, x: i32, y: i32) -> bool {
        match self.nb_loc(x, y, 16, 16) {
            None => false,
            Some((m, _, _)) => m == self.mb_addr || !self.pps.constrained_intra_pred || self.pic.mbs[m].kind.is_intra(),
        }
    }

    /// The left neighbouring luma samples of rows y0..y0 + h are available (in MBAFF frames they
    /// may come from both macroblocks of the left pair).
    fn intra_left_avail(&self, y0: i32, h: i32) -> bool {
        if !self.mbaff {
            return self.intra_nb(0);
        }
        self.intra_avail_at(-1, y0) && self.intra_avail_at(-1, y0 + 1) && self.intra_avail_at(-1, y0 + h - 1)
    }

    #[inline]
    pub fn mb(&self) -> &MbState {
        &self.pic.mbs[self.mb_addr]
    }
    #[inline]
    pub fn mb_mut(&mut self) -> &mut MbState {
        &mut self.pic.mbs[self.mb_addr]
    }

    /// Neighbour MB usable for intra prediction (constrained_intra_pred aware).
    fn intra_nb(&self, n: usize) -> bool {
        match self.nb[n] {
            None => false,
            Some(a) => !self.pps.constrained_intra_pred || self.pic.mbs[a].kind.is_intra(),
        }
    }

    /// QPC from QPY (8.5.8): qPi below 30 (and the negative values high bit depths allow) map to
    /// themselves, above it through the table.
    fn chroma_qp(&self, qp: i32, c: usize) -> i32 {
        let qpi = (qp + self.chroma_qp_offset[c]).clamp(-self.qp_bd_offset, 51);
        if qpi < 0 { qpi } else { QPC_TABLE[qpi as usize] as i32 }
    }

    /// nC for a 4x4 block (raster index) of component `p` coded like luma (Y, or Cb / Cr in
    /// 4:4:4) - 9.2.1.
    fn nc_luma(&self, p: usize, raster: usize) -> i32 {
        self.nc_grid(p, raster, 4, 4)
    }

    /// nC for a 4:2:0 / 4:2:2 chroma AC block of component `c` (raster index in a 4-wide grid).
    fn nc_chroma(&self, c: usize, raster: usize) -> i32 {
        self.nc_grid(1 + c, raster, 2, self.chroma.mb_h() / 4)
    }

    /// nC from the TotalCoeff of the blocks left of and above block `raster` of component `p`,
    /// whose blocks form a `w` x `h` grid per macroblock.
    fn nc_grid(&self, p: usize, raster: usize, w: usize, h: usize) -> i32 {
        let (bx, by) = (raster & 3, raster >> 2);
        let nnz = |st: &MbState, r: usize| nnz_of(st, p).get(r).copied().unwrap_or(0);
        let cur = self.mb();
        let (a, b) = if self.mbaff {
            // the neighbouring blocks (6.4.11.4, 6.4.11.5) of a pair may sit at other rows
            let (mw, mh) = (4 * w as i32, 4 * h as i32);
            let at = |x: i32, y: i32| self.nb_loc(x, y, mw, mh).map(|(m, xw, yw)| nnz(&self.pic.mbs[m], (yw / 4) * 4 + xw / 4));
            (if bx > 0 { Some(nnz(cur, raster - 1)) } else { at(-1, 4 * by as i32) }, if by > 0 { Some(nnz(cur, raster - 4)) } else { at(4 * bx as i32, -1) })
        } else {
            (
                if bx > 0 { Some(nnz(cur, raster - 1)) } else { self.nb[0].map(|m| nnz(&self.pic.mbs[m], by * 4 + w - 1)) },
                if by > 0 { Some(nnz(cur, raster - 4)) } else { self.nb[1].map(|m| nnz(&self.pic.mbs[m], (h - 1) * 4 + bx)) },
            )
        };
        match (a, b) {
            (Some(a), Some(b)) => (a as i32 + b as i32 + 1) >> 1,
            (Some(a), None) => a as i32,
            (None, Some(b)) => b as i32,
            _ => 0,
        }
    }

    /// Motion data of the neighbouring 4x4 block at (x, y) (4x4 units, relative to the current MB).
    pub(crate) fn nb_motion(&self, list: usize, x: i32, y: i32) -> Nb {
        if self.mbaff && !((0..4).contains(&x) && (0..4).contains(&y)) {
            return self.nb_motion_mbaff(list, x, y);
        }
        let (mb, bx, by) = if x < 0 {
            if y < 0 {
                (self.nb[3], 3, 3)
            } else if y > 3 {
                (None, 0, 0)
            } else {
                (self.nb[0], 3, y as usize)
            }
        } else if x > 3 {
            if y < 0 { (self.nb[2], 0, 3) } else { (None, 0, 0) }
        } else if y < 0 {
            (self.nb[1], x as usize, 3)
        } else {
            let r = (y * 4 + x) as usize;
            if self.mv_done & (1 << r) == 0 {
                return NB_NA;
            }
            (Some(self.mb_addr), x as usize, y as usize)
        };
        let Some(m) = mb else { return NB_NA };
        let st = &self.pic.mbs[m];
        let b8 = (by >> 1) * 2 + (bx >> 1);
        let r = st.ref_idx[list][b8];
        if r < 0 {
            return Nb { avail: true, ref_idx: -1, mv: [0, 0] };
        }
        Nb { avail: true, ref_idx: r, mv: st.mv[list][by * 4 + bx] }
    }

    /// [`Self::nb_motion`] outside the current macroblock of an MBAFF frame: the neighbour's
    /// motion in the current macroblock's frame or field units (8.4.1.3.1).
    #[inline(never)]
    fn nb_motion_mbaff(&self, list: usize, x: i32, y: i32) -> Nb {
        if y > 3 || (x > 3 && y >= 0) {
            return NB_NA;
        }
        // the sample next to the macroblock (6.4.12): -1, not the 4x4 block's first sample
        self.nb_motion_at(list, if x < 0 { -1 } else { 4 * x }, if y < 0 { -1 } else { 4 * y })
    }

    /// Motion of the block covering luma sample (xs, ys) relative to the current macroblock of an
    /// MBAFF frame, in the current macroblock's frame or field units (8.4.1.3.1).
    fn nb_motion_at(&self, list: usize, xs: i32, ys: i32) -> Nb {
        let Some((m, xw, yw)) = self.nb_loc_mbaff(xs, ys, 16, 16) else { return NB_NA };
        let Some(st) = self.pic.mbs.get(m) else { return NB_NA };
        let (bx, by) = (xw / 4, yw / 4);
        let r = st.ref_idx[list][(by >> 1) * 2 + (bx >> 1)];
        if r < 0 {
            return Nb { avail: true, ref_idx: -1, mv: [0, 0] };
        }
        let mut mv = st.mv[list][by * 4 + bx];
        let r = match (self.field_mb, st.field) {
            (true, false) => {
                mv[1] /= 2;
                r.saturating_mul(2)
            }
            (false, true) => {
                mv[1] = mv[1].wrapping_mul(2);
                r >> 1
            }
            _ => r,
        };
        Nb { avail: true, ref_idx: r, mv }
    }

    /// Returns (A, B, C) neighbour motion for a partition at (x, y) of width w (4x4 units), with C
    /// replaced by D when unavailable.
    fn nb_abc(&self, list: usize, x: usize, y: usize, w: usize) -> (Nb, Nb, Nb) {
        let (x, y, w) = (x as i32, y as i32, w as i32);
        let a = self.nb_motion(list, x - 1, y);
        let b = self.nb_motion(list, x, y - 1);
        let mut c = self.nb_motion(list, x + w, y - 1);
        if !c.avail {
            // D is the sample above-left of the partition; in MBAFF frames its line decides
            // which macroblock of a field pair to the left holds it
            c = if self.mbaff && x == 0 && y > 0 { self.nb_motion_at(list, -1, 4 * y - 1) } else { self.nb_motion(list, x - 1, y - 1) };
        }
        (a, b, c)
    }

    /// Luma motion vector prediction (8.4.1.3). `shape`: 1 = 16x8, 2 = 8x16, 0 = other.
    #[allow(clippy::too_many_arguments)]
    fn mvp(&self, list: usize, x: usize, y: usize, w: usize, ref_idx: i8, shape: u8, part: usize) -> [i16; 2] {
        let (a, b, c) = self.nb_abc(list, x, y, w);
        if shape == 1 {
            if part == 0 && b.ref_idx == ref_idx {
                return b.mv;
            }
            if part == 1 && a.ref_idx == ref_idx {
                return a.mv;
            }
        } else if shape == 2 {
            if part == 0 && a.ref_idx == ref_idx {
                return a.mv;
            }
            if part == 1 && c.ref_idx == ref_idx {
                return c.mv;
            }
        }
        Self::median_pred(a, b, c, ref_idx)
    }

    fn median_pred(a: Nb, mut b: Nb, mut c: Nb, ref_idx: i8) -> [i16; 2] {
        if !b.avail && !c.avail && a.avail {
            b = a;
            c = a;
        }
        let ma = a.ref_idx == ref_idx;
        let mb = b.ref_idx == ref_idx;
        let mc = c.ref_idx == ref_idx;
        match (ma, mb, mc) {
            (true, false, false) => a.mv,
            (false, true, false) => b.mv,
            (false, false, true) => c.mv,
            _ => [median(a.mv[0], b.mv[0], c.mv[0]), median(a.mv[1], b.mv[1], c.mv[1])],
        }
    }

    /// Store motion for a rectangle (4x4 units) of the current MB and mark it decoded.
    fn set_motion(&mut self, x: usize, y: usize, w: usize, h: usize, refs: [i8; 2], mvs: [[i16; 2]; 2]) {
        let st = &mut self.pic.mbs[self.mb_addr];
        for yy in y..y + h {
            for xx in x..x + w {
                let r = yy * 4 + xx;
                for l in 0..2 {
                    st.mv[l][r] = if refs[l] >= 0 { mvs[l] } else { [0, 0] };
                }
                self.mv_done |= 1 << r;
            }
        }
        for l in 0..2 {
            // ref_idx is per 8x8; rectangles never straddle 8x8 blocks with different refs
            for yy in (y..y + h).step_by(1) {
                for xx in x..x + w {
                    st.ref_idx[l][(yy >> 1) * 2 + (xx >> 1)] = refs[l];
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Direct prediction (8.4.1.2)
    // ------------------------------------------------------------------------------------------

    /// Co-located (mvCol, refIdxCol, [`RefPic::key`] of the referenced picture, vertMvScale) for
    /// 4x4 block `raster` of the current MB (8.4.1.2.1, Tables 8-6 and 8-8).
    fn colocated(&self, raster: usize) -> ([i16; 2], i8, u32, VScale) {
        const NONE: ([i16; 2], i8, u32, VScale) = ([0, 0], -1, u32::MAX, VScale::One);
        let Some(col) = self.refs[1].first() else { return NONE };
        let raster = if self.sps.direct_8x8_inference {
            let (bx, by) = (raster & 3, raster >> 2);
            (if by >= 2 { 12 } else { 0 }) + if bx >= 2 { 3 } else { 0 }
        } else {
            raster
        };
        let (xcol, ycol) = ((raster & 3) * 4, (raster >> 2) * 4);
        let f = &col.frame;
        if f.mb_w != self.mb_w {
            return NONE;
        }
        let x = self.mb_x;
        // a frame macroblock with a complementary field pair: the field closer in output order
        let near = || ((col.field_poc[0] - self.pic.poc).abs() >= (col.field_poc[1] - self.pic.poc).abs()) as usize;
        // field decoding flag of the pair at the current position of an MBAFF colPic
        let pair_field = |pair_row: usize| f.row::<S>(2 * pair_row).and_then(|m| m.field.get(x).copied()).unwrap_or(false);
        // the row unit holding mbAddrCol, yM and vertMvScale; units are frame macroblock rows of
        // frames (MBAFF: 2 * pair row + bottom) and 2 * row + parity of field pairs
        let (unit, ym, scale) = match (self.pic_field, f.coding) {
            (Some(p), Coding::Fields) => (2 * self.mb_y + col.field.unwrap_or(p) as usize, ycol, VScale::One),
            (Some(p), Coding::Mbaff) if pair_field(self.mb_y) => (2 * self.mb_y + p as usize, ycol, VScale::One),
            (Some(_), _) => (2 * self.mb_y + ycol / 8, (2 * ycol) % 16, VScale::FrmToFld),
            (None, Coding::Fields) if self.mbaff => {
                let py = self.mb_y / 2;
                if self.field_mb {
                    (2 * py + self.bottom as usize, ycol, VScale::One)
                } else {
                    (2 * py + near(), 8 * self.bottom as usize + 4 * (ycol / 8), VScale::FldToFrm)
                }
            }
            (None, Coding::Fields) => (2 * (self.mb_y / 2) + near(), 8 * (self.mb_y % 2) + 4 * (ycol / 8), VScale::FldToFrm),
            (None, _) if self.mbaff => {
                let (py, b) = (self.mb_y / 2, self.bottom as usize);
                match (self.field_mb, pair_field(py)) {
                    (false, false) => (2 * py + b, ycol, VScale::One),
                    (false, true) => (2 * py + near(), 8 * b + 4 * (ycol / 8), VScale::FldToFrm),
                    (true, false) => (2 * py + ycol / 8, (2 * ycol) % 16, VScale::FrmToFld),
                    (true, true) => (2 * py + b, ycol, VScale::One),
                }
            }
            (None, _) => (self.mb_y, ycol, VScale::One),
        };
        let Some(m) = f.row::<S>(unit) else { return NONE };
        if m.intra.get(x).is_none_or(|&i| i) {
            return NONE;
        }
        let b8 = x * 4 + (ym / 8) * 2 + xcol / 8;
        let r = x * 16 + (ym / 4) * 4 + xcol / 4;
        let l = if m.ref_idx[0].get(b8).is_some_and(|&r| r >= 0) { 0 } else { 1 };
        match (m.mv[l].get(r), m.ref_idx[l].get(b8), m.ref_id[l].get(b8)) {
            (Some(&mv), Some(&ri), Some(&id)) => (mv, ri, id, scale),
            _ => NONE,
        }
    }

    /// MapColToList0 (8.4.1.2.3): the refIdxL0 referencing the picture `key` the co-located
    /// block refers to (0 when the current list lacks it).
    fn map_col_to_list0(&self, key: u32) -> i8 {
        let frame = key >> 2;
        let pos = if self.mbaff && self.field_mb {
            // a field of a frame entry: a frame reference maps to the field of the current parity
            let b = self.bottom as u32;
            let par = if key & 3 == 3 { b } else { key & 1 };
            self.refs[0].iter().position(|r| r.key() >> 2 == frame).map(|i| 2 * i + (par != b) as usize)
        } else if let Some(p) = self.pic_field {
            let target = if key & 3 == 3 { (key & !3) | p as u32 } else { key };
            self.refs[0].iter().position(|r| r.key() == target)
        } else {
            // a frame, or the frame containing a referenced field
            self.refs[0].iter().position(|r| r.key() >> 2 == frame)
        };
        pos.map_or(0, |i| i.min(127) as i8)
    }

    /// Spatial direct: MB-level reference indices, predicted mvs and directZeroPrediction (8.4.1.2.2).
    fn spatial_direct_base(&self) -> ([i8; 2], [[i16; 2]; 2], bool) {
        let mut refs = [-1i8; 2];
        let mut nbs = [(NB_NA, NB_NA, NB_NA); 2];
        let min_pos = |x: i8, y: i8| if x >= 0 && y >= 0 { x.min(y) } else { x.max(y) };
        for (l, nb) in nbs.iter_mut().enumerate() {
            *nb = self.nb_abc(l, 0, 0, 4);
            refs[l] = min_pos(nb.0.ref_idx, min_pos(nb.1.ref_idx, nb.2.ref_idx));
        }
        let mut mvs = [[0i16; 2]; 2];
        if refs[0] < 0 && refs[1] < 0 {
            return ([0, 0], mvs, true);
        }
        for l in 0..2 {
            if refs[l] >= 0 {
                let (a, b, c) = nbs[l];
                mvs[l] = Self::median_pred(a, b, c, refs[l]);
            }
        }
        (refs, mvs, false)
    }

    /// Derive direct-mode motion for the 8x8 blocks in `b8_mask` of the current MB.
    fn derive_direct(&mut self, b8_mask: u8) -> Result<()> {
        let spatial = self.sh.direct_spatial_mv_pred;
        let (base_refs, base_mvs, direct_zero) = if spatial { self.spatial_direct_base() } else { ([0, 0], [[0, 0]; 2], false) };
        let col_short_term = self.refs[1].first().map(|r| !r.long_term).unwrap_or(false);
        for b8 in 0..4 {
            if b8_mask & (1 << b8) == 0 {
                continue;
            }
            let (x8, y8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            let unit = if self.sps.direct_8x8_inference { 2 } else { 1 };
            for sub in (0..4).filter(|s| unit == 1 || *s == 0) {
                let (x, y) = (x8 + (sub & 1), y8 + (sub >> 1));
                let raster = y * 4 + x;
                let (mv_col, ref_col, key_col, scale) = self.colocated(raster);
                let (refs, mvs) = if spatial {
                    let col_zero = col_short_term && ref_col == 0 && (-1..=1).contains(&mv_col[0]) && (-1..=1).contains(&mv_col[1]);
                    let mut mvs = base_mvs;
                    for l in 0..2 {
                        if direct_zero || base_refs[l] < 0 || (base_refs[l] == 0 && col_zero) {
                            mvs[l] = [0, 0];
                        }
                    }
                    (base_refs, mvs)
                } else {
                    let ref0 = if ref_col < 0 { 0 } else { self.map_col_to_list0(key_col) };
                    let mut mv_col = mv_col;
                    match scale {
                        VScale::One => {}
                        VScale::FrmToFld => mv_col[1] /= 2,
                        VScale::FldToFrm => mv_col[1] = mv_col[1].wrapping_mul(2),
                    }
                    let td = if self.mbaff && self.field_mb { &self.tdirect_field[self.bottom as usize] } else { &self.tdirect };
                    let (dsf, ltr) = td.get(ref0 as usize).copied().unwrap_or((256, true));
                    let (mv0, mv1) = if ltr {
                        (mv_col, [0, 0])
                    } else {
                        let s = |c: i16| ((dsf * c as i32 + 128) >> 8) as i16;
                        let m0 = [s(mv_col[0]), s(mv_col[1])];
                        (m0, [m0[0] - mv_col[0], m0[1] - mv_col[1]])
                    };
                    ([ref0, 0], [mv0, mv1])
                };
                self.set_motion(x, y, unit, unit, refs, mvs);
            }
        }
        let st = &mut self.pic.mbs[self.mb_addr];
        st.direct8x8 |= b8_mask;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------
    // Motion vector derivation for a parsed inter macroblock
    // ------------------------------------------------------------------------------------------

    /// P_Skip motion (8.4.1.1).
    pub fn derive_p_skip(&mut self) {
        let a = self.nb_motion(0, -1, 0);
        let b = self.nb_motion(0, 0, -1);
        let mv = if !a.avail || !b.avail || (a.ref_idx == 0 && a.mv == [0, 0]) || (b.ref_idx == 0 && b.mv == [0, 0]) {
            [0, 0]
        } else {
            self.mvp(0, 0, 0, 4, 0, 0, 0)
        };
        self.set_motion(0, 0, 4, 4, [0, -1], [mv, [0, 0]]);
    }

    /// Derive mvs of the current (non-skip) inter MB from `self.cur` (ref_idx, mvd).
    pub fn derive_inter_motion(&mut self) -> Result<()> {
        let info = self.cur.info;
        if info.kind == MbKind::BDirect16x16 || info.kind == MbKind::BSkip {
            return self.derive_direct(0xf);
        }
        let n = [self.num_refs(0), self.num_refs(1)];
        if info.part != Part::P8x8 {
            let shape = match info.part {
                Part::P16x8 => 1,
                Part::P8x16 => 2,
                _ => 0,
            };
            for p in 0..info.part.num_parts() {
                let (x, y, w, h) = info.part.rect(p);
                let mut refs = [-1i8; 2];
                let mut mvs = [[0i16; 2]; 2];
                for l in 0..2 {
                    if info.pred[p] & (1 << l) != 0 {
                        let r = self.cur.ref_idx[l][p];
                        ensure!((r as usize) < n[l], "ref_idx {r} out of range (list {l}, size {})", n[l]);
                        let mvp = self.mvp(l, x, y, w, r, shape, p);
                        let d = self.cur.mvd[l][p * 4];
                        refs[l] = r;
                        mvs[l] = [mvp[0].wrapping_add(d[0]), mvp[1].wrapping_add(d[1])];
                    }
                }
                self.set_motion(x, y, w, h, refs, mvs);
            }
            return Ok(());
        }
        for p in 0..4 {
            let sub = self.cur.sub[p];
            let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
            if sub.direct {
                self.derive_direct(1 << p)?;
                continue;
            }
            let mut refs = [-1i8; 2];
            for (l, r) in refs.iter_mut().enumerate() {
                if sub.pred & (1 << l) != 0 {
                    let v = self.cur.ref_idx[l][p];
                    ensure!((v as usize) < n[l], "ref_idx {v} out of range (list {l}, size {})", n[l]);
                    *r = v;
                }
            }
            for j in 0..sub.shape.num_parts() {
                let (sx, sy, w, h) = sub.shape.rect(j);
                let (x, y) = (x8 + sx, y8 + sy);
                let mut mvs = [[0i16; 2]; 2];
                for l in 0..2 {
                    if refs[l] >= 0 {
                        let mvp = self.mvp(l, x, y, w, refs[l], 0, 0);
                        let d = self.cur.mvd[l][p * 4 + j];
                        mvs[l] = [mvp[0].wrapping_add(d[0]), mvp[1].wrapping_add(d[1])];
                    }
                }
                self.set_motion(x, y, w, h, refs, mvs);
            }
        }
        Ok(())
    }

    /// Reference indices the current macroblock may use in list `l`: the list's entries, or
    /// both fields of each in MBAFF field macroblocks.
    pub(crate) fn num_refs(&self, l: usize) -> usize {
        self.refs[l].len() << (self.mbaff && self.field_mb) as usize
    }

    // ------------------------------------------------------------------------------------------
    // Inter prediction
    // ------------------------------------------------------------------------------------------

    fn weight_for(&self, refs: [i8; 2], comp: usize) -> Weight {
        let w = &self.weights;
        let field = self.mbaff && self.field_mb;
        match w.mode {
            WpMode::Default => Weight::Default,
            WpMode::Implicit => {
                if refs[0] >= 0 && refs[1] >= 0 {
                    // MBAFF field macroblocks: weights from the field POCs (8.4.2.3.1)
                    let (table, n1) = if field { (&self.implicit_field[self.bottom as usize], 2 * w.n1) } else { (&w.implicit, w.n1) };
                    let (w0, w1) = table.get(refs[0] as usize * n1 + refs[1] as usize).copied().unwrap_or((32, 32));
                    Weight::Weighted { log_wd: 5, w0, w1, o0: 0, o1: 0 }
                } else {
                    Weight::Default
                }
            }
            WpMode::Explicit => {
                let get = |l: usize| -> (i32, i32) {
                    if refs[l] < 0 {
                        return (0, 0);
                    }
                    // refIdxL0WP = refIdxL0 >> 1 in MBAFF field macroblocks
                    let r = if field { refs[l] as usize >> 1 } else { refs[l] as usize };
                    if comp == 0 {
                        w.luma[l].get(r).copied().unwrap_or((1 << w.luma_denom, 0))
                    } else {
                        w.chroma[l].get(r).map(|c| c[comp - 1]).unwrap_or((1 << w.chroma_denom, 0))
                    }
                };
                let (w0, o0) = get(0);
                let (w1, o1) = get(1);
                let log_wd = if comp == 0 { w.luma_denom } else { w.chroma_denom };
                Weight::Weighted { log_wd, w0, w1, o0, o1 }
            }
        }
    }

    /// Motion-compensated prediction of a rectangle (4x4 units) written into the picture.
    fn mc_rect(&mut self, x4: usize, y4: usize, w4: usize, h4: usize) {
        let st = &self.pic.mbs[self.mb_addr];
        let r = y4 * 4 + x4;
        let b8 = (y4 >> 1) * 2 + (x4 >> 1);
        let refs = [st.ref_idx[0][b8], st.ref_idx[1][b8]];
        let mvs = [st.mv[0][r], st.mv[1][r]];
        let (bw, bh) = (w4 * 4, h4 * 4);
        let wts = [self.weight_for(refs, 0), self.weight_for(refs, 1), self.weight_for(refs, 2)];
        let [yv, cv] = self.view;
        let px = yv.x0 + x4 * 4;
        let py = yv.y0 + y4 * 4;
        let picked = [self.ref_pic(0, refs[0]), self.ref_pic(1, refs[1])];
        let (use0, use1) = (picked[0].is_some(), picked[1].is_some());
        let (ys, cs) = (yv.stride, cv.stride);
        let max = self.pic.max;
        let (sx, sy) = (self.chroma.sx(), self.chroma.sy());
        let (cx, cy) = (px >> sx, py >> sy);
        let (yo, co) = (yv.off + py * ys + px, cv.off + cy * cs + cx);
        if use0 != use1 && wts.iter().all(|w| *w == Weight::Default) {
            // single-list unweighted prediction: interpolate straight into the picture
            let Some((f, field, cdy)) = picked[if use0 { 0 } else { 1 }] else { return };
            let l = if use0 { 0 } else { 1 };
            let planes = &mut self.pic.planes;
            let (Some(y), Some(cb), Some(cr)) = (planes.y.get_mut(yo..), planes.cb.get_mut(co..), planes.cr.get_mut(co..)) else { return };
            predict_block(f, field, mvs[l], cdy, px, py, bw, bh, (y, ys), [cb, cr], cs, max);
            return;
        }
        let s = &mut *self.s;
        for (l, p) in picked.iter().enumerate() {
            let Some((f, field, cdy)) = *p else { continue };
            let [pc0, pc1] = &mut s.pred_c[l];
            predict_block(f, field, mvs[l], cdy, px, py, bw, bh, (&mut s.pred[l][..], 16), [&mut pc0[..], &mut pc1[..]], 16, max);
        }
        let Some(ydst) = self.pic.planes.y.get_mut(yo..) else { return };
        inter::weighted_store(
            ydst,
            ys,
            if use0 { Some(&s.pred[0][..]) } else { None },
            if use1 { Some(&s.pred[1][..]) } else { None },
            16,
            bw,
            bh,
            wts[0],
            max,
        );
        if !self.chroma.coded() {
            return;
        }
        for c in 0..2 {
            let wc = wts[c + 1];
            let a = if use0 { Some(&s.pred_c[0][c][..]) } else { None };
            let b = if use1 { Some(&s.pred_c[1][c][..]) } else { None };
            let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
            let Some(dst) = plane.get_mut(co..) else { return };
            inter::weighted_store(dst, cs, a, b, 16, bw >> sx, bh >> sy, wc, max);
        }
    }

    /// The reference picture that index `ri` of list `l` selects for the current macroblock: the
    /// frame, the field of it read (MBAFF field macroblocks address the fields of the list
    /// entries, 8.4.2.1), and the 4:2:0 chroma vertical offset between fields of different
    /// parity (Table 8-10).
    #[inline]
    fn ref_pic(&self, l: usize, ri: i8) -> Option<(&'a Frame, Option<u8>, i16)> {
        if ri < 0 {
            return None;
        }
        let refs: &'a [Vec<RefPic>; 2] = self.refs;
        let (rp, field, cur) = if self.mbaff && self.field_mb {
            let b = self.bottom as u8;
            let rp = refs[l].get(ri as usize >> 1)?;
            (rp, Some(b ^ (ri as u8 & 1)), b)
        } else {
            let rp = refs[l].get(ri as usize)?;
            (rp, rp.field, self.pic_field.unwrap_or(0))
        };
        let cdy = field.map_or(0, |f| 2 * (cur as i16 - f as i16));
        Some((&rp.frame, field, cdy))
    }

    /// All 16 blocks of the current MB share references and motion vectors.
    fn uniform_motion(&self) -> bool {
        let st = self.mb();
        (0..2).all(|l| {
            let r = st.ref_idx[l];
            r[1] == r[0] && r[2] == r[0] && r[3] == r[0] && st.mv[l].iter().all(|m| *m == st.mv[l][0])
        })
    }

    /// Inter prediction for the whole current MB, using the stored motion.
    pub fn predict_inter(&mut self) {
        let info = self.cur.info;
        let direct_unit = if self.sps.direct_8x8_inference { 2 } else { 1 };
        match info.kind {
            MbKind::PSkip => self.mc_rect(0, 0, 4, 4),
            MbKind::BSkip | MbKind::BDirect16x16 if self.uniform_motion() => self.mc_rect(0, 0, 4, 4),
            MbKind::BSkip | MbKind::BDirect16x16 => {
                for y in (0..4).step_by(direct_unit) {
                    for x in (0..4).step_by(direct_unit) {
                        self.mc_rect(x, y, direct_unit, direct_unit);
                    }
                }
            }
            _ => {
                if info.part != Part::P8x8 {
                    for p in 0..info.part.num_parts() {
                        let (x, y, w, h) = info.part.rect(p);
                        self.mc_rect(x, y, w, h);
                    }
                } else {
                    for p in 0..4 {
                        let sub = self.cur.sub[p];
                        let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
                        if sub.direct {
                            for y in (0..2).step_by(direct_unit) {
                                for x in (0..2).step_by(direct_unit) {
                                    self.mc_rect(x8 + x, y8 + y, direct_unit, direct_unit);
                                }
                            }
                        } else {
                            for j in 0..sub.shape.num_parts() {
                                let (sx, sy, w, h) = sub.shape.rect(j);
                                self.mc_rect(x8 + sx, y8 + sy, w, h);
                            }
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Intra prediction mode derivation (8.3.1.1 / 8.3.2.1)
    // ------------------------------------------------------------------------------------------

    /// Intra4x4PredMode / Intra8x8PredMode of the neighbouring block left of (`left`) or above
    /// the block at (bx, by) (4x4 units) of the current macroblock; None when
    /// dcPredModePredictedFlag must be set.
    fn intra_mode_nb(&self, left: bool, bx: usize, by: usize, is8x8: bool) -> Option<u8> {
        let (x, y) = (4 * bx as i32, 4 * by as i32);
        let (m, xw, yw) = if left { self.nb_loc(x - 1, y, 16, 16)? } else { self.nb_loc(x, y - 1, 16, 16)? };
        let st = &self.pic.mbs[m];
        if self.pps.constrained_intra_pred && !st.kind.is_intra() {
            return None;
        }
        // an Intra_8x8 block takes the 4x4 block of an Intra_4x4 neighbour next to its first
        // row or column (8.3.2.1: n = 1 for A, 2 for B)
        let (nx, ny) = match (is8x8, left) {
            (true, true) => (2 * (xw / 8) + 1, 2 * (yw / 8)),
            (true, false) => (2 * (xw / 8), 2 * (yw / 8) + 1),
            _ => (xw / 4, yw / 4),
        };
        Some(match st.kind {
            MbKind::I4x4 | MbKind::I8x8 => st.intra_modes[ny * 4 + nx],
            _ => 2,
        })
    }

    /// Resolve Intra4x4PredMode / Intra8x8PredMode for the current MB from `self.cur.rem_mode`.
    pub fn derive_intra_modes(&mut self, is8x8: bool) {
        let step = if is8x8 { 2 } else { 1 };
        let nblk = if is8x8 { 4 } else { 16 };
        for i in 0..nblk {
            let (bx, by) = if is8x8 {
                ((i & 1) * 2, (i >> 1) * 2)
            } else {
                let (x, y) = BLK4_XY[i];
                (x as usize, y as usize)
            };
            let a = if bx > 0 { Some(self.mb().intra_modes[by * 4 + bx - 1]) } else { self.intra_mode_nb(true, bx, by, is8x8) };
            let b = if by > 0 { Some(self.mb().intra_modes[(by - 1) * 4 + bx]) } else { self.intra_mode_nb(false, bx, by, is8x8) };
            let pred = match (a, b) {
                (Some(a), Some(b)) => a.min(b),
                _ => 2,
            };
            let rem = self.cur.rem_mode[i];
            let mode = if rem < 0 {
                pred
            } else if (rem as u8) < pred {
                rem as u8
            } else {
                rem as u8 + 1
            };
            let st = self.mb_mut();
            for yy in by..by + step {
                for xx in bx..bx + step {
                    st.intra_modes[yy * 4 + xx] = mode;
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Reconstruction
    // ------------------------------------------------------------------------------------------

    /// The plane of colour component `p` from the current macroblock's view origin (offset by the
    /// field parity in MBAFF field macroblocks), its stride and the macroblock's top-left sample.
    #[inline]
    fn mb_plane(&mut self, p: usize) -> (&mut [S], usize, usize, usize) {
        let v = self.view[(p > 0) as usize];
        let (plane, _) = plane_of(&mut self.pic.planes, p);
        (plane.get_mut(v.off..).unwrap_or_default(), v.stride, v.x0, v.y0)
    }

    /// Add the residual of 4x4 block `raster` of component `p` (raster index in a 4-wide grid).
    fn add_block4(&mut self, p: usize, raster: usize) {
        if self.s.blk_nz[p] & (1 << raster) == 0 {
            return;
        }
        let max = self.pic.max;
        let v = self.view[(p > 0) as usize];
        let (px, py) = (v.x0 + (raster & 3) * 4, v.y0 + (raster >> 2) * 4);
        let Some(blk) = self.s.coef[p].get_mut(raster * 16..).and_then(|c| c.first_chunk_mut::<16>()) else {
            return;
        };
        let (plane, _) = plane_of(&mut self.pic.planes, p);
        if let Some(dst) = plane.get_mut(v.off + py * v.stride + px..) {
            transform::idct4_add(blk, dst, v.stride, max);
        }
        blk.fill(0);
    }

    /// Add the residual of 8x8 block `b8` of component `p` (coded like luma).
    fn add_block8(&mut self, p: usize, b8: usize) {
        if self.s.blk_nz[p] & (1 << b8) == 0 {
            return;
        }
        let max = self.pic.max;
        let v = self.view[(p > 0) as usize];
        let (px, py) = (v.x0 + (b8 & 1) * 8, v.y0 + (b8 >> 1) * 8);
        let Some(blk) = self.s.coef[p].get_mut(b8 * 64..).and_then(|c| c.first_chunk_mut::<64>()) else {
            return;
        };
        let (plane, _) = plane_of(&mut self.pic.planes, p);
        if let Some(dst) = plane.get_mut(v.off + py * v.stride + px..) {
            transform::idct8_add(blk, dst, v.stride, max);
        }
        blk.fill(0);
    }

    /// Add the residual of the 4:2:0 / 4:2:2 chroma blocks.
    fn add_chroma(&mut self) {
        for p in 1..3 {
            let mut m = self.s.blk_nz[p];
            while m != 0 {
                let r = m.trailing_zeros() as usize;
                m &= m - 1;
                self.add_block4(p, r);
            }
            self.s.blk_nz[p] = 0;
        }
    }

    /// Colour components coded like luma: Y, plus Cb and Cr in 4:4:4.
    #[inline]
    fn luma_like(&self) -> usize {
        if self.chroma.0 == 3 { 3 } else { 1 }
    }

    /// Neighbouring sample availability of the luma block at (bx, by) (4x4 units) of `n4` x `n4`
    /// 4x4 blocks; top_right only for blocks in the top row.
    fn luma_avail(&self, bx: usize, by: usize, n4: usize) -> Avail {
        if !self.mbaff {
            let a = self.intra_nb(0);
            let b = self.intra_nb(1);
            let top_left = match (bx > 0, by > 0) {
                (true, true) => true,
                (false, true) => a,
                (true, false) => b,
                (false, false) => self.intra_nb(3),
            };
            let top_right = by == 0 && if bx + n4 < 4 { b } else { self.intra_nb(2) };
            return Avail { left: bx > 0 || a, top: by > 0 || b, top_left, top_right };
        }
        let (x, y) = (4 * bx as i32, 4 * by as i32);
        Avail {
            left: bx > 0 || self.intra_left_avail(y, 4 * n4 as i32),
            top: by > 0 || self.intra_avail_at(x, -1),
            top_left: (bx > 0 && by > 0) || self.intra_avail_at(x - 1, y - 1),
            top_right: by == 0 && self.intra_avail_at(x + 4 * n4 as i32, -1),
        }
    }

    fn luma_avail4(&self, bx: usize, by: usize) -> Avail {
        let mut av = self.luma_avail(bx, by, 1);
        if by > 0 {
            av.top_right = bx < 3 && RASTER_TO_BLK4[(by - 1) * 4 + bx + 1] < RASTER_TO_BLK4[by * 4 + bx];
        }
        av
    }

    /// Reconstruct an intra MB (prediction + residual) after its syntax has been parsed.
    pub fn reconstruct_intra(&mut self) {
        // 4:4:4: Cb and Cr are predicted and reconstructed like luma, with the luma modes (8.3.4.5)
        for p in 0..self.luma_like() {
            self.reconstruct_intra_luma(p);
        }
        if matches!(self.chroma.0, 1 | 2) {
            let mut av = self.luma_avail(0, 0, 4);
            // MBAFF with constrained intra prediction: the halves of the left column may come
            // from different macroblocks; chroma DC predicts each 4x4 block from its own rows
            let (upper, lower) = if self.mbaff { (self.intra_left_avail(0, 8), self.intra_left_avail(8, 8)) } else { (av.left, av.left) };
            av.left = upper;
            let mode = self.mb().intra_chroma_mode;
            let h = self.chroma.mb_h();
            let depth = self.depth;
            for p in 1..3 {
                let (plane, stride, cx, cy) = self.mb_plane(p);
                intra::pred_chroma(plane, stride, cx, cy, mode, av, lower, depth, h);
            }
            self.add_chroma();
        }
        self.s.blk_nz = [0; 3];
    }

    /// Intra prediction + residual of component `p` with the luma processes (8.3.1 - 8.3.3).
    fn reconstruct_intra_luma(&mut self, p: usize) {
        let kind = self.mb().kind;
        let depth = self.depth;
        match kind {
            MbKind::I4x4 => {
                for &(bx, by) in BLK4_XY.iter() {
                    let (bx, by) = (bx as usize, by as usize);
                    let av = self.luma_avail4(bx, by);
                    let mode = self.mb().intra_modes[by * 4 + bx];
                    let (plane, stride, x0, y0) = self.mb_plane(p);
                    intra::pred4x4(plane, stride, x0 + bx * 4, y0 + by * 4, mode, av, depth);
                    self.add_block4(p, by * 4 + bx);
                }
            }
            MbKind::I8x8 => {
                for b8 in 0..4 {
                    let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
                    let mut av = self.luma_avail(bx, by, 2);
                    if b8 == 2 {
                        av.top_right = true;
                    }
                    let mode = self.mb().intra_modes[by * 4 + bx];
                    let (plane, stride, x0, y0) = self.mb_plane(p);
                    intra::pred8x8(plane, stride, x0 + bx * 4, y0 + by * 4, mode, av, depth);
                    self.add_block8(p, b8);
                }
            }
            MbKind::I16x16 => {
                let av = self.luma_avail(0, 0, 4);
                let mode = self.cur.info.i16_mode;
                let (plane, stride, x0, y0) = self.mb_plane(p);
                intra::pred16x16(plane, stride, x0, y0, mode, Avail { top_right: false, ..av }, depth);
                for r in 0..16 {
                    self.add_block4(p, r);
                }
            }
            _ => {}
        }
    }

    /// Add the residual of an inter MB (prediction already written).
    pub fn reconstruct_inter_residual(&mut self) {
        let t8 = self.mb().transform_8x8;
        for p in 0..self.luma_like() {
            if t8 {
                for b8 in 0..4 {
                    self.add_block8(p, b8);
                }
            } else if self.s.blk_nz[p] != 0 {
                for r in 0..16 {
                    self.add_block4(p, r);
                }
            }
        }
        if matches!(self.chroma.0, 1 | 2) {
            self.add_chroma();
        }
        self.s.blk_nz = [0; 3];
    }

    /// Number of I_PCM samples: 256 luma plus 2 * MbWidthC * MbHeightC chroma.
    fn pcm_count(&self) -> usize {
        256 + if self.chroma.coded() { 2 * self.chroma.mb_w() * self.chroma.mb_h() } else { 0 }
    }

    /// Write I_PCM samples.
    pub fn write_pcm(&mut self, samples: &[u16; 768]) {
        let max = self.pic.max;
        let (cw, ch) = (self.chroma.mb_w(), self.chroma.mb_h());
        let n = cw * ch;
        for p in 0..if self.chroma.coded() { 3 } else { 1 } {
            let (w, h, src) = if p == 0 { (16, 16, &samples[..256]) } else { (cw, ch, &samples[256 + (p - 1) * n..256 + p * n]) };
            let (plane, stride, x0, y0) = self.mb_plane(p);
            for r in 0..h {
                let Some(dst) = plane.get_mut((y0 + r) * stride + x0..(y0 + r) * stride + x0 + w) else { return };
                for (d, &v) in dst.iter_mut().zip(&src[r * w..r * w + w]) {
                    *d = S::from_i32(i32::from(v).min(max));
                }
            }
        }
    }

    /// Read the PCM samples of an I_PCM macroblock (7.3.5), `bit_depth` bits each, from the
    /// byte-aligned position `pos` of `data`; returns them and how many bytes they occupied.
    pub fn read_pcm(&self, data: &[u8], pos: usize) -> Result<([u16; 768], usize)> {
        let bd = self.pic.bit_depth;
        let count = self.pcm_count();
        let bytes = count * bd as usize / 8;
        let Some(src) = data.get(pos..pos + bytes) else {
            return invalid("truncated I_PCM macroblock");
        };
        let mut out = [0u16; 768];
        if bd == 8 {
            for (o, &v) in out.iter_mut().zip(src) {
                *o = u16::from(v);
            }
        } else {
            let mut r = BitReader::new(src);
            for o in out.iter_mut().take(count) {
                *o = r.read_bits(bd)? as u16;
            }
        }
        Ok((out, bytes))
    }

    /// Common state for an I_PCM MB.
    pub fn finish_pcm(&mut self) {
        let qpc = [self.chroma_qp(0, 0) as i8, self.chroma_qp(0, 1) as i8];
        let st = self.mb_mut();
        st.kind = MbKind::IPcm;
        st.qp = 0;
        st.qpc = qpc;
        st.nnz = [16; 16];
        st.nnz_c = [[16; 16]; 2];
        st.cbf_dc = 7;
        st.cbp = 0x2f;
        st.nz_mask = 0xffff;
        st.ref_idx = [[-1; 4]; 2];
    }

    /// Update QP after mb_qp_delta.
    pub fn apply_qp_delta(&mut self, delta: i32) -> Result<()> {
        // -(26 + QpBdOffsetY / 2) ..= 25 + QpBdOffsetY / 2 (7.4.5)
        let half = self.qp_bd_offset / 2;
        ensure!((-26 - half..=25 + half).contains(&delta), "mb_qp_delta {delta} out of range");
        let range = 52 + self.qp_bd_offset;
        self.qp = (self.qp + delta + 2 * range - 52) % range - self.qp_bd_offset;
        Ok(())
    }

    pub fn store_qp(&mut self) {
        let qp = self.qp;
        let qpc = [self.chroma_qp(qp, 0) as i8, self.chroma_qp(qp, 1) as i8];
        let st = self.mb_mut();
        st.qp = qp as i8;
        st.qpc = qpc;
    }

    // ------------------------------------------------------------------------------------------
    // Residual storage helpers (shared by CAVLC and CABAC)
    // ------------------------------------------------------------------------------------------

    /// QP' of colour component `p`: QP'Y, or QP'C of Cb / Cr.
    #[inline]
    fn qp_prime(&self, p: usize) -> i32 {
        if p == 0 { self.qp + self.qp_bd_offset } else { self.chroma_qp(self.qp, p - 1) + self.qp_bd_offset }
    }

    /// 1 for an inter macroblock (selects the Inter scaling lists).
    #[inline]
    fn inter(&self) -> usize {
        !self.mb().kind.is_intra() as usize
    }

    /// Store a 4x4 block of component `p` (coded like luma) given in scan order (levels[0..16]);
    /// `ac_only` skips position 0.
    pub fn put_luma4(&mut self, p: usize, raster: usize, levels: &[i32; 16], ac_only: bool) {
        let scan = self.scan4;
        let qp = self.qp_prime(p);
        // ScalingList4x4[iYCbCr + 3 * mbIsInterFlag] (8.5.9)
        let Some(ls) = self.ls.ls4.get(p + 3 * self.inter()).map(|l| &l[(qp % 6) as usize]) else { return };
        let Some(blk) = self.s.coef[p].get_mut(raster * 16..raster * 16 + 16) else { return };
        let mut any = false;
        for (k, &lv) in levels.iter().enumerate().skip(ac_only as usize) {
            if lv != 0 {
                let pos = scan[k] as usize;
                blk[pos] = transform::scale4(lv, ls[pos], qp);
                any = true;
            }
        }
        if any {
            self.s.blk_nz[p] |= 1 << raster;
            if p == 0 {
                self.pic.mbs[self.mb_addr].nz_mask |= 1 << raster;
            }
        }
    }

    /// Store an 8x8 block of component `p` given in scan order.
    pub fn put_luma8(&mut self, p: usize, b8: usize, levels: &[i32; 64]) {
        let scan = self.scan8;
        let qp = self.qp_prime(p);
        // ScalingList8x8[2 * iYCbCr + mbIsInterFlag]
        let Some(ls) = self.ls.ls8.get(2 * p + self.inter()).map(|l| &l[(qp % 6) as usize]) else { return };
        let Some(blk) = self.s.coef[p].get_mut(b8 * 64..b8 * 64 + 64) else { return };
        let mut any = false;
        for (k, &lv) in levels.iter().enumerate() {
            if lv != 0 {
                let pos = scan[k] as usize;
                blk[pos] = transform::scale8(lv, ls[pos], qp);
                any = true;
            }
        }
        if any {
            self.s.blk_nz[p] |= 1 << b8;
            if p == 0 {
                let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
                let m = (0b11u16 << (by * 4 + bx)) | (0b11u16 << ((by + 1) * 4 + bx));
                self.pic.mbs[self.mb_addr].nz_mask |= m;
            }
        }
    }

    /// Intra16x16 DC levels of component `p` in scan order -> scaled DC of each 4x4 block.
    pub fn put_luma_dc(&mut self, p: usize, levels: &[i32; 16]) {
        let scan = self.scan4;
        let mut c = [0i32; 16];
        for (k, &lv) in levels.iter().enumerate() {
            c[scan[k] as usize] = lv;
        }
        let qp = self.qp_prime(p);
        let Some(ls00) = self.ls.ls4.get(p).map(|l| l[(qp % 6) as usize][0]) else { return };
        transform::luma_dc_dequant(&mut c, qp, ls00);
        for (r, &dc) in c.iter().enumerate() {
            if dc != 0 {
                self.s.coef[p][r * 16] = dc;
                self.s.blk_nz[p] |= 1 << r;
            }
        }
    }

    /// Chroma DC levels of component `c` (4 for 4:2:0, 8 for 4:2:2, in parsing order).
    pub fn put_chroma_dc(&mut self, c: usize, levels: &[i32]) {
        let qpc = self.qp_prime(1 + c);
        let list = 1 + c + 3 * self.inter();
        let mut dc = [0i32; 8];
        let n = if let Ok(lv) = <&[i32; 8]>::try_from(levels) {
            let mut v = *lv;
            let qp_dc = qpc + 3;
            transform::chroma_dc_dequant_422(&mut v, qp_dc, self.ls.ls4[list][(qp_dc % 6) as usize][0]);
            dc = v;
            8
        } else if let Ok(lv) = <&[i32; 4]>::try_from(levels) {
            let mut v = *lv;
            transform::chroma_dc_dequant_420(&mut v, qpc, self.ls.ls4[list][(qpc % 6) as usize][0]);
            dc[..4].copy_from_slice(&v);
            4
        } else {
            return;
        };
        for (b, &v) in dc.iter().enumerate().take(n) {
            if v != 0 {
                // chroma4x4BlkIdx b sits at column b & 1, row b >> 1
                let raster = (b >> 1) * 4 + (b & 1);
                self.s.coef[1 + c][raster * 16] = v;
                self.s.blk_nz[1 + c] |= 1 << raster;
            }
        }
    }

    /// Chroma AC levels (scan positions 1..16 in levels[1..]) for block `raster` (4-wide grid) of
    /// component `c`.
    pub fn put_chroma_ac(&mut self, c: usize, raster: usize, levels: &[i32; 16]) {
        let scan = self.scan4;
        let qpc = self.qp_prime(1 + c);
        let list = 1 + c + 3 * self.inter();
        let ls = &self.ls.ls4[list][(qpc % 6) as usize];
        let Some(blk) = self.s.coef[1 + c].get_mut(raster * 16..raster * 16 + 16) else { return };
        let mut any = false;
        for (k, &lv) in levels.iter().enumerate().skip(1) {
            if lv != 0 {
                let p = scan[k] as usize;
                blk[p] = transform::scale4(lv, ls[p], qpc);
                any = true;
            }
        }
        if any {
            self.s.blk_nz[1 + c] |= 1 << raster;
        }
    }

    // ------------------------------------------------------------------------------------------
    // CAVLC macroblock layer
    // ------------------------------------------------------------------------------------------

    /// Decode a P_Skip or B_Skip macroblock at the current address.
    pub fn decode_skip(&mut self) -> Result<()> {
        let b = self.sh.slice_type.is_b();
        self.cur = MbCur::default();
        self.cur.info.kind = if b { MbKind::BSkip } else { MbKind::PSkip };
        self.mb_mut().kind = self.cur.info.kind;
        self.store_qp();
        if b {
            self.derive_direct(0xf)?;
        } else {
            ensure!(!self.refs[0].is_empty(), "P_Skip with empty reference list");
            self.derive_p_skip();
        }
        self.predict_inter();
        self.prev_qp_delta_nz = false;
        Ok(())
    }

    fn read_te(r: &mut BitReader, range: u32) -> Result<u32> {
        Ok(if range > 1 { r.read_ue()? } else { !r.read_bit()? as u32 })
    }

    /// Parse and reconstruct one macroblock_layer() with CAVLC.
    pub fn decode_mb_cavlc(&mut self, r: &mut BitReader) -> Result<()> {
        let mb_type = r.read_ue()?;
        let st = self.sh.slice_type;
        let info = match st {
            SliceType::I | SliceType::Si => intra_mb_type(mb_type),
            SliceType::P | SliceType::Sp => {
                if mb_type < 5 {
                    p_mb_type(mb_type)
                } else {
                    intra_mb_type(mb_type - 5)
                }
            }
            SliceType::B => {
                if mb_type < 23 {
                    b_mb_type(mb_type)
                } else {
                    intra_mb_type(mb_type - 23)
                }
            }
        };
        let Some(mut info) = info else { return invalid(format!("mb_type {mb_type} out of range")) };
        self.cur = MbCur { info, ..Default::default() };
        if info.kind == MbKind::IPcm {
            r.byte_align();
            let pos = r.byte_pos();
            let (samples, bytes) = self.read_pcm(r.data(), pos)?;
            r.skip(bytes * 8)?;
            self.finish_pcm();
            self.write_pcm(&samples);
            return Ok(());
        }
        let mut transform_8x8 = false;
        if info.kind == MbKind::I4x4 && self.pps.transform_8x8_mode && r.read_flag()? {
            transform_8x8 = true;
            info.kind = MbKind::I8x8;
            self.cur.info.kind = MbKind::I8x8;
        }
        self.mb_mut().kind = info.kind;
        self.mb_mut().transform_8x8 = transform_8x8;
        // MBAFF field macroblocks index the fields of the list entries (7.4.5.1)
        let k = if self.mbaff && self.field_mb { 2 } else { 1 };
        let n_ref = [self.sh.num_ref_idx_active[0] * k, self.sh.num_ref_idx_active[1] * k];
        match info.kind {
            MbKind::I4x4 | MbKind::I8x8 => {
                let n = if info.kind == MbKind::I8x8 { 4 } else { 16 };
                for i in 0..n {
                    self.cur.rem_mode[i] = if r.read_flag()? { -1 } else { r.read_bits(3)? as i8 };
                }
                self.derive_intra_modes(info.kind == MbKind::I8x8);
                if matches!(self.chroma.0, 1 | 2) {
                    let m = r.read_ue()?;
                    ensure!(m <= 3, "intra_chroma_pred_mode out of range");
                    self.mb_mut().intra_chroma_mode = m as u8;
                }
            }
            MbKind::I16x16 => {
                if matches!(self.chroma.0, 1 | 2) {
                    let m = r.read_ue()?;
                    ensure!(m <= 3, "intra_chroma_pred_mode out of range");
                    self.mb_mut().intra_chroma_mode = m as u8;
                }
            }
            MbKind::BDirect16x16 => {}
            _ => {
                if info.part == Part::P8x8 {
                    for p in 0..4 {
                        let t = r.read_ue()?;
                        let sub = if st.is_b() { b_sub_mb_type(t) } else { p_sub_mb_type(t) };
                        let Some(sub) = sub else { return invalid(format!("sub_mb_type {t} out of range")) };
                        self.cur.sub[p] = sub;
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            self.cur.ref_idx[l][p] = if n_ref[l] > 1 && !info.ref0 { Self::read_te(r, n_ref[l] - 1)? as i8 } else { 0 };
                        }
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            for j in 0..sub.shape.num_parts() {
                                let x = r.read_se()?;
                                let y = r.read_se()?;
                                self.cur.mvd[l][p * 4 + j] = [x as i16, y as i16];
                            }
                        }
                    }
                } else {
                    let np = info.part.num_parts();
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) != 0 {
                                self.cur.ref_idx[l][p] = if n_ref[l] > 1 { Self::read_te(r, n_ref[l] - 1)? as i8 } else { 0 };
                            }
                        }
                    }
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) != 0 {
                                let x = r.read_se()?;
                                let y = r.read_se()?;
                                self.cur.mvd[l][p * 4] = [x as i16, y as i16];
                            }
                        }
                    }
                }
            }
        }
        // coded_block_pattern
        let cbp = if info.kind == MbKind::I16x16 {
            info.i16_cbp
        } else {
            let code = r.read_ue()? as usize;
            let table: &[(u8, u8)] = if matches!(self.chroma.0, 1 | 2) { &CBP_ME } else { &CBP_ME_NO_CHROMA };
            let Some(&(i, p)) = table.get(code) else { return invalid("coded_block_pattern out of range") };
            if info.kind.is_intra() { i } else { p }
        };
        self.cur.cbp = cbp;
        self.mb_mut().cbp = cbp;
        if !info.kind.is_intra() && cbp & 15 != 0 && self.pps.transform_8x8_mode && self.no_sub_8x8_lt() {
            let t = r.read_flag()?;
            self.mb_mut().transform_8x8 = t;
        }
        if !info.kind.is_intra() {
            self.derive_inter_motion()?;
        }
        if cbp != 0 || info.kind == MbKind::I16x16 {
            let d = r.read_se()?;
            self.apply_qp_delta(d)?;
        }
        self.store_qp();
        self.residual_cavlc(r, cbp)?;
        if info.kind.is_intra() {
            self.reconstruct_intra();
        } else {
            self.predict_inter();
            self.reconstruct_inter_residual();
        }
        Ok(())
    }

    /// Whether transform_size_8x8_flag may be present for an inter MB (NoSubMbPartSizeLessThan8x8 etc.).
    pub fn no_sub_8x8_lt(&self) -> bool {
        let info = self.cur.info;
        match info.kind {
            MbKind::BDirect16x16 => self.sps.direct_8x8_inference,
            MbKind::Inter if info.part == Part::P8x8 => {
                self.cur.sub.iter().all(|s| if s.direct { self.sps.direct_8x8_inference } else { s.shape == SubPart::S8x8 })
            }
            _ => true,
        }
    }

    fn residual_cavlc(&mut self, r: &mut BitReader, cbp: u8) -> Result<()> {
        // residual_luma() for Y, then chroma DC / AC (4:2:0, 4:2:2) or Cb and Cr like luma (4:4:4)
        self.residual_luma_cavlc(r, cbp, 0)?;
        match self.chroma.0 {
            1 | 2 => {
                let ndc = 2 * self.chroma.mb_h() / 4;
                let cbp_c = cbp >> 4;
                if cbp_c & 3 != 0 {
                    for c in 0..2 {
                        let mut lv = [0i32; 8];
                        let dc = &mut lv[..ndc];
                        cavlc::residual_block(r, dc, 0, ndc - 1, ndc, if ndc == 4 { -1 } else { -2 })?;
                        self.put_chroma_dc(c, dc);
                    }
                }
                if cbp_c & 2 != 0 {
                    for c in 0..2 {
                        for b in 0..ndc {
                            let raster = (b >> 1) * 4 + (b & 1);
                            let nc = self.nc_chroma(c, raster);
                            let mut lv = [0i32; 16];
                            let tc = cavlc::residual_block(r, &mut lv[1..], 0, 14, 15, nc)?;
                            self.mb_mut().nnz_c[c][raster] = tc;
                            self.put_chroma_ac(c, raster, &lv);
                        }
                    }
                }
            }
            3 => {
                self.residual_luma_cavlc(r, cbp, 1)?;
                self.residual_luma_cavlc(r, cbp, 2)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// residual_luma() (7.3.5.3.1) of component `p` with CAVLC.
    fn residual_luma_cavlc(&mut self, r: &mut BitReader, cbp: u8, p: usize) -> Result<()> {
        let kind = self.mb().kind;
        let t8 = self.mb().transform_8x8;
        if kind == MbKind::I16x16 {
            let nc = self.nc_luma(p, 0);
            let mut lv = [0i32; 16];
            cavlc::residual_block(r, &mut lv, 0, 15, 16, nc)?;
            self.put_luma_dc(p, &lv);
        }
        for b8 in 0..4 {
            let (bx8, by8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            if cbp & (1 << b8) == 0 {
                continue;
            }
            if t8 {
                let mut lv8 = [0i32; 64];
                for i4 in 0..4 {
                    let raster = (by8 + (i4 >> 1)) * 4 + bx8 + (i4 & 1);
                    let nc = self.nc_luma(p, raster);
                    let mut lv = [0i32; 16];
                    let tc = cavlc::residual_block(r, &mut lv, 0, 15, 16, nc)?;
                    nnz_of_mut(self.mb_mut(), p)[raster] = tc;
                    for (i, &v) in lv.iter().enumerate() {
                        lv8[4 * i + i4] = v;
                    }
                }
                self.put_luma8(p, b8, &lv8);
            } else {
                for i4 in 0..4 {
                    let raster = (by8 + (i4 >> 1)) * 4 + bx8 + (i4 & 1);
                    let nc = self.nc_luma(p, raster);
                    let mut lv = [0i32; 16];
                    let tc = if kind == MbKind::I16x16 {
                        cavlc::residual_block(r, &mut lv[1..], 0, 14, 15, nc)?
                    } else {
                        cavlc::residual_block(r, &mut lv, 0, 15, 16, nc)?
                    };
                    nnz_of_mut(self.mb_mut(), p)[raster] = tc;
                    self.put_luma4(p, raster, &lv, kind == MbKind::I16x16);
                }
            }
        }
        Ok(())
    }

    /// Decode slice_data() with CAVLC. `r` is positioned after the slice header.
    pub fn decode_cavlc(&mut self, r: &mut BitReader) -> Result<()> {
        if self.mbaff {
            return self.decode_cavlc_mbaff(r);
        }
        let total = self.mb_w * self.mb_h;
        let mut addr = self.sh.first_mb_in_slice as usize;
        let intra_slice = self.sh.slice_type.is_intra();
        loop {
            ensure!(addr < total, "macroblock address {addr} out of range");
            if !intra_slice {
                let run = r.read_ue()? as usize;
                ensure!(addr + run <= total, "mb_skip_run out of range");
                for _ in 0..run {
                    self.start_mb(addr);
                    self.decode_skip()?;
                    self.pic.mb_done(addr);
                    addr += 1;
                }
                if run > 0 && !r.more_rbsp_data() {
                    break;
                }
                ensure!(addr < total, "macroblock address {addr} out of range");
            }
            self.start_mb(addr);
            self.decode_mb_cavlc(r)?;
            self.pic.mb_done(addr);
            addr += 1;
            if !r.more_rbsp_data() {
                break;
            }
        }
        Ok(())
    }

    /// slice_data() of an MBAFF frame with CAVLC (7.3.4): macroblocks are coded in pairs, and
    /// mb_field_decoding_flag precedes the first non-skipped macroblock of each pair.
    fn decode_cavlc_mbaff(&mut self, r: &mut BitReader) -> Result<()> {
        let total = self.mb_w * self.mb_h;
        let mut addr = 2 * self.sh.first_mb_in_slice as usize;
        let intra_slice = self.sh.slice_type.is_intra();
        loop {
            ensure!(addr < total, "macroblock address {addr} out of range");
            if !intra_slice {
                let run = r.read_ue()? as usize;
                ensure!(addr + run <= total, "mb_skip_run out of range");
                let end = addr + run;
                while addr < end {
                    if addr.is_multiple_of(2) {
                        // a skipped pair infers the flag; a skipped top macroblock followed by a
                        // coded bottom one takes the bottom's flag, which comes right after the run
                        self.field_mb = if addr + 1 < end { self.infer_field(addr) } else { r.read_flag()? };
                    }
                    self.start_mb(addr);
                    self.decode_skip()?;
                    self.pic.mb_done(addr);
                    addr += 1;
                }
                if run > 0 && !r.more_rbsp_data() {
                    break;
                }
                ensure!(addr < total, "macroblock address {addr} out of range");
            }
            // a bottom macroblock shares the flag of its top one (read early if that was skipped)
            if addr.is_multiple_of(2) {
                self.field_mb = r.read_flag()?;
            }
            self.start_mb(addr);
            self.decode_mb_cavlc(r)?;
            self.pic.mb_done(addr);
            addr += 1;
            if !r.more_rbsp_data() {
                break;
            }
        }
        Ok(())
    }
}
