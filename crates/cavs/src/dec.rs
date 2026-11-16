//! Picture decoding: headers, macroblocks, motion vector prediction, residuals and the in-loop
//! filter, re-expressed from FFmpeg n7.1.1 `cavs.c` and `cavsdec.c` (Stefan Gehrer). Where the C
//! code keeps state between macroblocks, slices or pictures (border lines, predictors, the
//! coefficient block after a damaged residual), the port keeps the same state, so the output
//! matches FFmpeg's also on streams that rely on it.

use crate::Error;
use crate::bits::Bits;
use crate::dsp::{self, Edge, RefPlane};
use crate::tables::*;

pub(crate) const PIC_I_START: u32 = 0xb3;
pub(crate) const PIC_PB_START: u32 = 0xb6;

pub(crate) const PIC_I: u32 = 1;
pub(crate) const PIC_P: u32 = 2;
pub(crate) const PIC_B: u32 = 3;

const NOT_AVAIL: i16 = -1;
const REF_INTRA: i16 = -2;
const REF_DIR: i16 = -3;

const A_AVAIL: u32 = 1;
const B_AVAIL: u32 = 2;
const C_AVAIL: u32 = 4;
const D_AVAIL: u32 = 8;

const ESCAPE_CODE: u32 = 59;
/// FFmpeg's `AVERROR_INVALIDDATA`, which ends up as a skip count on a broken code.
const AVERROR_INVALIDDATA: i64 = -1_094_995_529;

// macroblock types
const I_8X8: u32 = 0;
const P_SKIP: u32 = 1;
const P_16X16: u32 = 2;
const P_16X8: u32 = 3;
const P_8X16: u32 = 4;
const P_8X8: u32 = 5;
const B_SKIP: u32 = 6;
const B_DIRECT: u32 = 7;
const B_FWD_16X16: u32 = 8;
const B_BWD_16X16: u32 = 9;
const B_SYM_16X16: u32 = 10;
const B_8X8: u32 = 29;

// sub-macroblock types of B_8X8
const B_SUB_DIRECT: u32 = 0;
const B_SUB_FWD: u32 = 1;
const B_SUB_BWD: u32 = 2;
const B_SUB_SYM: u32 = 3;

// motion vector prediction modes
const MV_PRED_MEDIAN: u32 = 0;
const MV_PRED_LEFT: u32 = 1;
const MV_PRED_TOP: u32 = 2;
const MV_PRED_TOPRIGHT: u32 = 3;
const MV_PRED_PSKIP: u32 = 4;
const MV_PRED_BSKIP: u32 = 5;

// partition sizes
const BLK_16X16: u32 = 0;
const BLK_16X8: u32 = 1;
const BLK_8X16: u32 = 2;
const BLK_8X8: u32 = 3;

// motion vector cache: D3 B2 B3 C2 / A1 X0 X1 - / A3 X2 X3 -, then the same backward
const MV_BWD_OFFS: usize = 12;
const MV_STRIDE: usize = 4;
const MV_FWD_D3: usize = 0;
const MV_FWD_B2: usize = 1;
const MV_FWD_B3: usize = 2;
const MV_FWD_C2: usize = 3;
const MV_FWD_A1: usize = 4;
const MV_FWD_X0: usize = 5;
const MV_FWD_X1: usize = 6;
const MV_FWD_A3: usize = 8;
const MV_FWD_X2: usize = 9;
const MV_FWD_X3: usize = 10;
const MV_BWD_B3: usize = MV_BWD_OFFS + MV_FWD_B3;
const MV_BWD_C2: usize = MV_BWD_OFFS + MV_FWD_C2;
const MV_BWD_A1: usize = MV_BWD_OFFS + MV_FWD_A1;
const MV_BWD_X0: usize = MV_BWD_OFFS + MV_FWD_X0;
const MV_BWD_X1: usize = MV_BWD_OFFS + MV_FWD_X1;
const MV_BWD_X2: usize = MV_BWD_OFFS + MV_FWD_X2;
const MV_SCAN: [usize; 4] = [MV_FWD_X0, MV_FWD_X1, MV_FWD_X2, MV_FWD_X3];
const SCAN3X3: [usize; 4] = [4, 5, 7, 8];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Mv {
    x: i16,
    y: i16,
    dist: i16,
    r: i16,
}

const UN_MV: Mv = Mv { x: 0, y: 0, dist: 1, r: NOT_AVAIL };
const INTRA_MV: Mv = Mv { x: 0, y: 0, dist: 1, r: REF_INTRA };
const DIR_MV: Mv = Mv { x: 0, y: 0, dist: 1, r: REF_DIR };

/// The sequence header fields the decoder uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Seq {
    pub profile: u32,
    pub level: u32,
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: u32,
    pub frame_rate_code: u32,
    pub low_delay: bool,
    pub mb_w: usize,
    pub mb_h: usize,
}

/// The largest picture the decoder takes (luma samples of the macroblock-aligned picture).
const MAX_SAMPLES: usize = 35_651_584;

/// Reads a sequence header (after its start code). FFmpeg reads only the Jizhun profile.
pub(crate) fn parse_seq(data: &[u8]) -> Result<Seq, Error> {
    let mut b = Bits::new(data);
    let profile = b.get(8);
    if profile != 0x20 {
        return Err(Error::Unsupported("only the Jizhun profile (AVS1-P2) is supported"));
    }
    let level = b.get(8);
    b.skip(1); // progressive_sequence
    let width = b.get(14);
    let height = b.get(14);
    if width == 0 || height == 0 {
        return Err(Error::Malformed("invalid picture size"));
    }
    b.skip(2); // chroma_format
    b.skip(3); // sample_precision
    let aspect_ratio = b.get(4);
    let mut frame_rate_code = b.get(4);
    if frame_rate_code == 0 || frame_rate_code > 13 {
        frame_rate_code = 1;
    }
    b.skip(18); // bit_rate_lower
    b.skip(1); // marker_bit
    b.skip(12); // bit_rate_upper
    let low_delay = b.bit();
    if b.left() < 0 {
        return Err(Error::Truncated);
    }
    let mb_w = (width as usize).div_ceil(16);
    let mb_h = (height as usize).div_ceil(16);
    if mb_w * mb_h * 256 > MAX_SAMPLES {
        return Err(Error::Unsupported("picture too large"));
    }
    Ok(Seq { profile, level, width, height, aspect_ratio, frame_rate_code, low_delay, mb_w, mb_h })
}

/// The three planes of a macroblock-aligned picture.
#[derive(Clone, Debug, Default)]
pub(crate) struct Planes {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Planes {
    fn new(mb_w: usize, mb_h: usize) -> Self {
        let luma = mb_w * mb_h * 256;
        Self { y: vec![0; luma], u: vec![0; luma / 4], v: vec![0; luma / 4] }
    }
}

/// A reference picture.
#[derive(Clone, Debug)]
pub(crate) struct Pic {
    pub planes: Planes,
    pub tag: i64,
}

#[derive(Clone, Copy)]
enum Plane {
    Y,
    U,
    V,
}

/// The decoder state of FFmpeg's `AVSContext`.
pub(crate) struct Ctx {
    pub seq: Option<Seq>,
    // lines kept across macroblock rows, slices and pictures
    top_qp: Vec<u8>,
    top_mv: [Vec<Mv>; 2],
    top_pred_y: Vec<i32>,
    top_border_y: Vec<u8>,
    top_border_u: Vec<u8>,
    top_border_v: Vec<u8>,
    col_mv: Vec<Mv>,
    col_type: Vec<u8>,
    block: [i16; 64],
    left_border_y: [u8; 26],
    left_border_u: [u8; 10],
    left_border_v: [u8; 10],
    intern_border_y: [u8; 26],
    topleft_y: u8,
    topleft_u: u8,
    topleft_v: u8,
    mv: [Mv; 24],
    pred_mode_y: [i32; 9],
    // pictures
    pub dpb: [Option<Pic>; 2],
    pub dpb_poc: [i32; 2],
    cur: Planes,
    cur_poc: i32,
    pict_type: u32,
    dist: [i32; 2],
    scale_den: [i32; 2],
    direct_den: [i32; 2],
    sym_factor: i32,
    pub stream_revision: u32,
    pub got_keyframe: bool,
    // picture and slice header fields
    pic_structure: bool,
    skip_mode_flag: bool,
    loop_filter_disable: bool,
    alpha_offset: i32,
    beta_offset: i32,
    ref_flag: bool,
    qp: i32,
    qp_fixed: bool,
    pic_qp_fixed: bool,
    cbp: u32,
    // macroblock position
    flags: u32,
    mbx: usize,
    mby: usize,
    mbidx: usize,
    /// Macroblock row the sample pointers are on (`cy`; a slice header moves `mby` only).
    prow: usize,
    left_qp: i32,
}

impl Default for Ctx {
    fn default() -> Self {
        let mut mv = [Mv::default(); 24];
        mv[7] = UN_MV;
        mv[19] = UN_MV;
        Self {
            seq: None,
            top_qp: Vec::new(),
            top_mv: [Vec::new(), Vec::new()],
            top_pred_y: Vec::new(),
            top_border_y: Vec::new(),
            top_border_u: Vec::new(),
            top_border_v: Vec::new(),
            col_mv: Vec::new(),
            col_type: Vec::new(),
            block: [0; 64],
            left_border_y: [0; 26],
            left_border_u: [0; 10],
            left_border_v: [0; 10],
            intern_border_y: [0; 26],
            topleft_y: 0,
            topleft_u: 0,
            topleft_v: 0,
            mv,
            pred_mode_y: [0; 9],
            dpb: [None, None],
            dpb_poc: [0; 2],
            cur: Planes::default(),
            cur_poc: 0,
            pict_type: PIC_I,
            dist: [0; 2],
            scale_den: [0; 2],
            direct_den: [0; 2],
            sym_factor: 0,
            stream_revision: 0,
            got_keyframe: false,
            pic_structure: true,
            skip_mode_flag: false,
            loop_filter_disable: false,
            alpha_offset: 0,
            beta_offset: 0,
            ref_flag: false,
            qp: 0,
            qp_fixed: false,
            pic_qp_fixed: false,
            cbp: 0,
            flags: 0,
            mbx: 0,
            mby: 0,
            mbidx: 0,
            prow: 0,
            left_qp: 0,
        }
    }
}

fn set_mvs(mv: &mut [Mv; 24], i: usize, size: u32) {
    let m = mv[i];
    match size {
        BLK_16X16 => {
            mv[i + MV_STRIDE] = m;
            mv[i + MV_STRIDE + 1] = m;
            mv[i + 1] = m;
        }
        BLK_16X8 => mv[i + 1] = m,
        BLK_8X16 => mv[i + MV_STRIDE] = m,
        _ => {}
    }
}

fn get_bs(mv: &[Mv; 24], p: usize, q: usize, b: bool) -> u8 {
    let (mp, mq) = (mv[p], mv[q]);
    if mp.r == REF_INTRA || mq.r == REF_INTRA {
        return 2;
    }
    let differ = |a: Mv, c: Mv| (i32::from(a.x) - i32::from(c.x)).abs() >= 4 || (i32::from(a.y) - i32::from(c.y)).abs() >= 4 || a.r != c.r;
    if differ(mp, mq) {
        return 1;
    }
    if b && differ(mv[p + MV_BWD_OFFS], mv[q + MV_BWD_OFFS]) {
        return 1;
    }
    0
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// `get_ue_code`: a k-th order exp-Golomb code.
fn ue_code(b: &mut Bits<'_>, order: u32) -> Option<u32> {
    let v = b.ue()?;
    if u64::from(v) >= (1u64 << 31) >> order {
        return None;
    }
    if order > 0 { Some((v << order) + b.get(order)) } else { Some(v) }
}

impl Ctx {
    /// Takes a sequence header: the first one sets the picture size (later ones must keep it).
    pub fn sequence(&mut self, s: Seq) -> Result<(), Error> {
        if let Some(old) = &self.seq
            && (old.width != s.width || old.height != s.height)
        {
            return Err(Error::Unsupported("picture size changes within the stream"));
        }
        let first = self.seq.is_none();
        let (mb_w, mb_h) = (s.mb_w, s.mb_h);
        self.seq = Some(s);
        if first {
            self.top_qp = vec![0; mb_w];
            self.top_mv = [vec![Mv::default(); mb_w * 2 + 1], vec![Mv::default(); mb_w * 2 + 1]];
            self.top_pred_y = vec![0; mb_w * 2];
            self.top_border_y = vec![0; (mb_w + 1) * 16];
            self.top_border_u = vec![0; mb_w * 10];
            self.top_border_v = vec![0; mb_w * 10];
            self.col_mv = vec![Mv::default(); mb_w * mb_h * 4];
            self.col_type = vec![0; mb_w * mb_h];
        }
        Ok(())
    }

    fn dims(&self) -> (usize, usize) {
        self.seq.as_ref().map_or((0, 0), |s| (s.mb_w, s.mb_h))
    }

    /// Decodes a picture (`data` starts after its start code). A decoded B picture is returned;
    /// a decoded I or P picture becomes the newest reference.
    pub fn decode_pic(&mut self, data: &[u8], stc: u32, tag: i64) -> Result<Option<Pic>, Error> {
        let (mb_w, mb_h) = self.dims();
        let low_delay = self.seq.as_ref().ok_or(Error::Malformed("picture before a sequence header"))?.low_delay;
        let mut b = Bits::new(data);
        b.skip(16); // bbv_delay
        if stc == PIC_PB_START {
            self.pict_type = b.get(2) + PIC_I;
            if self.pict_type > PIC_B {
                return Err(Error::Malformed("illegal picture type"));
            }
            if self.dpb[0].is_none() || (self.dpb[1].is_none() && self.pict_type == PIC_B) {
                return Err(Error::Malformed("missing reference picture"));
            }
        } else {
            self.pict_type = PIC_I;
            if b.bit() {
                b.skip(24); // time_code
            }
            // the first sample clips were all progressive and had no low_delay
            if low_delay || b.peek(9) & 1 == 0 || b.peek(11) & 3 != 0 {
                self.stream_revision = 1;
            }
            if self.stream_revision > 0 {
                b.skip(1); // marker_bit
            }
        }
        if b.left() < 23 {
            return Err(Error::Truncated);
        }
        self.cur = Planes::new(mb_w, mb_h);
        self.init_pic();
        self.cur_poc = b.get(8) as i32 * 2;

        // temporal distances and motion vector scaling
        self.dist[0] = if self.pict_type != PIC_B { (self.cur_poc - self.dpb_poc[0]) & 511 } else { (self.dpb_poc[0] - self.cur_poc) & 511 };
        self.dist[1] = (self.cur_poc - self.dpb_poc[1]) & 511;
        self.scale_den = self.dist.map(|d| if d != 0 { 512 / d } else { 0 });
        if self.pict_type == PIC_B {
            self.sym_factor = self.dist[0] * self.scale_den[1];
            if self.sym_factor.abs() > 32768 {
                return Err(Error::Malformed("sym_factor too large"));
            }
        } else {
            self.direct_den = self.dist.map(|d| if d != 0 { 16384 / d } else { 0 });
        }

        if low_delay {
            let _ = b.ue(); // bbv_check_times
        }
        let progressive = b.bit();
        self.pic_structure = true;
        if !progressive {
            self.pic_structure = b.bit();
        }
        if !self.pic_structure && stc == PIC_PB_START {
            b.skip(1); // advanced_pred_mode_disable
        }
        b.skip(1); // top_field_first
        b.skip(1); // repeat_first_field
        self.qp_fixed = b.bit();
        self.pic_qp_fixed = self.qp_fixed;
        self.qp = b.get(6) as i32;
        if self.pict_type == PIC_I {
            if !progressive && !self.pic_structure {
                b.skip(1);
            }
            b.skip(4); // reserved
        } else {
            if !(self.pict_type == PIC_B && self.pic_structure) {
                self.ref_flag = b.bit();
            }
            b.skip(4); // reserved
            self.skip_mode_flag = b.bit();
        }
        self.loop_filter_disable = b.bit();
        if !self.loop_filter_disable && b.bit() {
            self.alpha_offset = b.se().unwrap_or(0);
            self.beta_offset = b.se().unwrap_or(0);
            if !(-64..=64).contains(&self.alpha_offset) || !(-64..=64).contains(&self.beta_offset) {
                self.alpha_offset = 0;
                self.beta_offset = 0;
                return Err(Error::Malformed("deblocking offsets out of range"));
            }
        } else {
            self.alpha_offset = 0;
            self.beta_offset = 0;
        }

        let mut skip_count: i64 = -1;
        match self.pict_type {
            PIC_I => loop {
                self.check_for_slice(&mut b);
                self.decode_mb_i(&mut b, 0)?;
                if !self.next_mb() {
                    break;
                }
            },
            PIC_P => loop {
                if self.check_for_slice(&mut b) {
                    skip_count = -1;
                }
                if self.skip_mode_flag && skip_count < 0 {
                    if b.left() < 1 {
                        return Err(Error::Truncated);
                    }
                    skip_count = b.ue().map_or(AVERROR_INVALIDDATA, i64::from);
                }
                let skip = self.skip_mode_flag && {
                    let c = skip_count;
                    skip_count -= 1;
                    c != 0
                };
                if skip {
                    self.decode_mb_p(&mut b, P_SKIP);
                } else {
                    if b.left() < 1 {
                        return Err(Error::Truncated);
                    }
                    let code = b.ue().ok_or(Error::Malformed("macroblock type"))?;
                    let mb_type = code + P_SKIP + u32::from(self.skip_mode_flag);
                    if mb_type > P_8X8 {
                        self.decode_mb_i(&mut b, mb_type - P_8X8 - 1)?;
                    } else {
                        self.decode_mb_p(&mut b, mb_type);
                    }
                }
                if !self.next_mb() {
                    break;
                }
            },
            _ => loop {
                if self.check_for_slice(&mut b) {
                    skip_count = -1;
                }
                if self.skip_mode_flag && skip_count < 0 {
                    if b.left() < 1 {
                        return Err(Error::Truncated);
                    }
                    skip_count = b.ue().map_or(AVERROR_INVALIDDATA, i64::from);
                }
                let skip = self.skip_mode_flag && {
                    let c = skip_count;
                    skip_count -= 1;
                    c != 0
                };
                if skip {
                    self.decode_mb_b(&mut b, B_SKIP)?;
                } else {
                    if b.left() < 1 {
                        return Err(Error::Truncated);
                    }
                    let code = b.ue().ok_or(Error::Malformed("macroblock type"))?;
                    let mb_type = code + B_SKIP + u32::from(self.skip_mode_flag);
                    if mb_type > B_8X8 {
                        self.decode_mb_i(&mut b, mb_type - B_8X8 - 1)?;
                    } else {
                        self.decode_mb_b(&mut b, mb_type)?;
                    }
                }
                if !self.next_mb() {
                    break;
                }
            },
        }
        let cur = std::mem::take(&mut self.cur);
        let pic = Pic { planes: cur, tag };
        if self.pict_type == PIC_B {
            return Ok(Some(pic));
        }
        // DPB[1] ← DPB[0] ← current
        self.dpb[1] = self.dpb[0].take();
        self.dpb_poc[1] = self.dpb_poc[0];
        self.dpb[0] = Some(pic);
        self.dpb_poc[0] = self.cur_poc;
        Ok(None)
    }

    fn init_pic(&mut self) {
        for i in (0..=20).step_by(4) {
            self.mv[i] = UN_MV;
        }
        self.mv[MV_BWD_X0] = DIR_MV;
        set_mvs(&mut self.mv, MV_BWD_X0, BLK_16X16);
        self.mv[MV_FWD_X0] = DIR_MV;
        set_mvs(&mut self.mv, MV_FWD_X0, BLK_16X16);
        self.pred_mode_y[3] = -1;
        self.pred_mode_y[6] = -1;
        self.mbx = 0;
        self.mby = 0;
        self.mbidx = 0;
        self.prow = 0;
        self.flags = 0;
    }

    fn check_for_slice(&mut self, b: &mut Bits<'_>) -> bool {
        if self.mbx != 0 {
            return false;
        }
        let mut align = ((8 - b.count() % 8) % 8) as u32;
        if align == 0 && b.peek(8) == 0x80 {
            align = 8;
        }
        if b.peek(24 + align) & 0xff_ffff != 1 {
            return false;
        }
        b.skip(24 + align as usize);
        let stc = b.get(8) as usize;
        if stc >= self.dims().1 {
            return false;
        }
        self.slice_header(b, stc);
        true
    }

    fn slice_header(&mut self, b: &mut Bits<'_>, stc: usize) {
        self.mby = stc;
        self.mbidx = stc * self.dims().0;
        self.flags &= !(B_AVAIL | C_AVAIL);
        if !self.pic_qp_fixed {
            self.qp_fixed = b.bit();
            self.qp = b.get(6) as i32;
        }
        // inter pictures and a second field's slices can carry weighting parameters, which
        // FFmpeg does not read (a set flag desynchronises it, so it does here as well)
        if self.pict_type != PIC_I || (!self.pic_structure && self.mby >= self.dims().0 / 2) {
            let _ = b.bit();
        }
    }

    fn init_mb(&mut self) {
        let mbx = self.mbx;
        for i in 0..3 {
            self.mv[MV_FWD_B2 + i] = self.top_mv[0].get(mbx * 2 + i).copied().unwrap_or(UN_MV);
            self.mv[MV_FWD_B2 + MV_BWD_OFFS + i] = self.top_mv[1].get(mbx * 2 + i).copied().unwrap_or(UN_MV);
        }
        self.pred_mode_y[1] = self.top_pred_y.get(mbx * 2).copied().unwrap_or(-1);
        self.pred_mode_y[2] = self.top_pred_y.get(mbx * 2 + 1).copied().unwrap_or(-1);
        if self.flags & B_AVAIL == 0 {
            self.mv[MV_FWD_B2] = UN_MV;
            self.mv[MV_FWD_B3] = UN_MV;
            self.mv[MV_FWD_B2 + MV_BWD_OFFS] = UN_MV;
            self.mv[MV_BWD_B3] = UN_MV;
            self.pred_mode_y[1] = -1;
            self.pred_mode_y[2] = -1;
            self.flags &= !(C_AVAIL | D_AVAIL);
        } else if mbx > 0 {
            self.flags |= D_AVAIL;
        }
        if mbx + 1 == self.dims().0 {
            self.flags &= !C_AVAIL;
        }
        if self.flags & C_AVAIL == 0 {
            self.mv[MV_FWD_C2] = UN_MV;
            self.mv[MV_BWD_C2] = UN_MV;
        }
        if self.flags & D_AVAIL == 0 {
            self.mv[MV_FWD_D3] = UN_MV;
            self.mv[MV_FWD_D3 + MV_BWD_OFFS] = UN_MV;
        }
    }

    /// Saves the predictors and moves to the next macroblock; false at the end of the picture.
    fn next_mb(&mut self) -> bool {
        let (mb_w, mb_h) = self.dims();
        self.flags |= A_AVAIL;
        for i in (0..=20).step_by(4) {
            self.mv[i] = self.mv[i + 2];
        }
        let m = self.mbx * 2;
        if let Some(t) = self.top_mv[0].get_mut(m..m + 2) {
            t[0] = self.mv[MV_FWD_X2];
            t[1] = self.mv[MV_FWD_X3];
        }
        if let Some(t) = self.top_mv[1].get_mut(m..m + 2) {
            t[0] = self.mv[MV_BWD_X2];
            t[1] = self.mv[MV_BWD_X2 + 1];
        }
        self.mbidx += 1;
        self.mbx += 1;
        if self.mbx == mb_w {
            self.flags = B_AVAIL | C_AVAIL;
            self.pred_mode_y[3] = -1;
            self.pred_mode_y[6] = -1;
            for i in (0..=20).step_by(4) {
                self.mv[i] = UN_MV;
            }
            self.mbx = 0;
            self.mby += 1;
            self.prow = self.mby;
            if self.mby >= mb_h {
                return false;
            }
        }
        true
    }

    // -----------------------------------------------------------------------------------------
    // sample positions

    fn ls(&self) -> usize {
        self.dims().0 * 16
    }

    fn cs(&self) -> usize {
        self.dims().0 * 8
    }

    /// Offset of the current macroblock's luma and chroma samples.
    fn cy(&self) -> usize {
        self.prow * 16 * self.ls() + self.mbx * 16
    }

    fn cc(&self) -> usize {
        self.prow * 8 * self.cs() + self.mbx * 8
    }

    fn y_at(&self, off: usize) -> u8 {
        self.cur.y.get(off).copied().unwrap_or(0)
    }

    // -----------------------------------------------------------------------------------------
    // residuals

    /// Decodes, dequantises and adds one 8×8 block (`decode_residual_block`). On an error the
    /// coefficient block keeps what was written, as in FFmpeg.
    fn residual_block(&mut self, b: &mut Bits<'_>, tabs: &[Vlc2d], esc_order: u32, qp: usize, plane: Plane, off: usize, stride: usize) -> Result<(), Error> {
        let mut level_buf = [0i16; 65];
        let mut run_buf = [0u8; 65];
        let mut r = 0usize;
        let last = tabs.len().saturating_sub(1);
        let mut n = 0usize;
        while n < 65 {
            let t = tabs.get(r.min(last)).ok_or(Error::Malformed("residual table"))?;
            let code = ue_code(b, t.golomb_order).ok_or(Error::Malformed("residual code"))?;
            let (level, run);
            if code >= ESCAPE_CODE {
                run = ((code - ESCAPE_CODE) >> 1) + 1;
                if run > 64 {
                    return Err(Error::Malformed("residual run too large"));
                }
                let esc = ue_code(b, esc_order).ok_or(Error::Malformed("escape code"))?;
                if esc > 32767 {
                    return Err(Error::Malformed("escape code too large"));
                }
                let add = if run > t.max_run as u32 { 1 } else { i32::from(t.level_add.get(run as usize).copied().unwrap_or(0)) };
                let mut l = esc as i32 + add;
                while tabs.get(r.min(last)).is_some_and(|t| l > t.inc_limit) && r < last {
                    r += 1;
                }
                let mask = -((code & 1) as i32);
                l = (l ^ mask) - mask;
                level = l;
            } else {
                let e = t.rltab[code as usize];
                if e[0] == 0 {
                    break;
                }
                level = i32::from(e[0]);
                run = e[1] as u32;
                r += e[2] as usize;
            }
            level_buf[n] = level as i16;
            run_buf[n] = run as u8;
            n += 1;
        }
        // inverse scan and dequantisation
        let mul = i32::from(DEQUANT_MUL[qp & 63]);
        let shift = u32::from(DEQUANT_SHIFT[qp & 63]);
        let round = 1i32 << (shift - 1);
        let mut pos: i32 = -1;
        for k in (0..n).rev() {
            pos += i32::from(run_buf[k]);
            if pos > 63 {
                return Err(Error::Malformed("coefficient position out of the block"));
            }
            let v = i32::from(level_buf[k]).wrapping_mul(mul).wrapping_add(round) >> shift;
            self.block[usize::from(ZIGZAG[pos as usize])] = v as i16;
        }
        let dst = match plane {
            Plane::Y => &mut self.cur.y,
            Plane::U => &mut self.cur.u,
            Plane::V => &mut self.cur.v,
        };
        dsp::idct8_add(dst, off, stride, &mut self.block);
        self.block = [0; 64];
        Ok(())
    }

    fn residual_chroma(&mut self, b: &mut Bits<'_>) -> Result<(), Error> {
        let qp = usize::from(CHROMA_QP[(self.qp & 63) as usize]);
        let (cc, cs) = (self.cc(), self.cs());
        if self.cbp & (1 << 4) != 0 {
            self.residual_block(b, &CHROMA_DEC, 0, qp, Plane::U, cc, cs)?;
        }
        if self.cbp & (1 << 5) != 0 {
            self.residual_block(b, &CHROMA_DEC, 0, qp, Plane::V, cc, cs)?;
        }
        Ok(())
    }

    /// `decode_residual_inter`; FFmpeg ignores its errors.
    fn residual_inter(&mut self, b: &mut Bits<'_>) {
        let Some(cbp) = b.ue() else { return };
        let Some(e) = CBP_TAB.get(cbp as usize) else { return };
        self.cbp = u32::from(e[1]);
        if self.cbp != 0 && !self.qp_fixed {
            self.qp = self.qp.wrapping_add(b.se().unwrap_or(0)) & 63;
        }
        let (cy, ls) = (self.cy(), self.ls());
        let scan = [0, 8, 8 * ls, 8 * ls + 8];
        for (block, &o) in scan.iter().enumerate() {
            if self.cbp & (1 << block) != 0 {
                let _ = self.residual_block(b, &INTER_DEC, 0, self.qp as usize, Plane::Y, cy + o, ls);
            }
        }
        let _ = self.residual_chroma(b);
    }

    // -----------------------------------------------------------------------------------------
    // intra macroblocks

    fn load_intra_pred_luma(&mut self, block: usize) -> ([u8; 18], [u8; 18]) {
        let mut top = [0u8; 18];
        let (cy, ls, mbx) = (self.cy(), self.ls(), self.mbx);
        let tb = |s: &Self, i: usize| s.top_border_y.get(i).copied().unwrap_or(0);
        let left: [u8; 18];
        match block {
            0 => {
                self.left_border_y[0] = self.left_border_y[1];
                let fill = self.left_border_y[16];
                self.left_border_y[17..26].fill(fill);
                for i in 0..16 {
                    top[1 + i] = tb(self, mbx * 16 + i);
                }
                top[17] = top[16];
                top[0] = top[1];
                if self.flags & A_AVAIL != 0 && self.flags & B_AVAIL != 0 {
                    top[0] = self.topleft_y;
                    self.left_border_y[0] = self.topleft_y;
                }
                left = first18(&self.left_border_y[..]);
            }
            1 => {
                for i in 0..8 {
                    self.intern_border_y[i + 1] = self.y_at(cy + 7 + i * ls);
                }
                let fill = self.intern_border_y[8];
                self.intern_border_y[9..18].fill(fill);
                self.intern_border_y[0] = self.intern_border_y[1];
                for i in 0..8 {
                    top[1 + i] = tb(self, mbx * 16 + 8 + i);
                }
                if self.flags & C_AVAIL != 0 {
                    for i in 0..8 {
                        top[9 + i] = tb(self, (mbx + 1) * 16 + i);
                    }
                } else {
                    let f = top[8];
                    top[9..18].fill(f);
                }
                top[17] = top[16];
                top[0] = top[1];
                if self.flags & B_AVAIL != 0 {
                    let v = tb(self, mbx * 16 + 7);
                    top[0] = v;
                    self.intern_border_y[0] = v;
                }
                left = first18(&self.intern_border_y[..]);
            }
            2 => {
                for i in 0..16 {
                    top[1 + i] = self.y_at(cy + 7 * ls + i);
                }
                top[17] = top[16];
                top[0] = top[1];
                if self.flags & A_AVAIL != 0 {
                    top[0] = self.left_border_y[8];
                }
                left = first18(&self.left_border_y[8..]);
            }
            _ => {
                for i in 0..8 {
                    self.intern_border_y[i + 9] = self.y_at(cy + 7 + (i + 8) * ls);
                }
                let fill = self.intern_border_y[16];
                self.intern_border_y[17..26].fill(fill);
                for i in 0..9 {
                    top[i] = self.y_at(cy + 7 + 7 * ls + i);
                }
                let f = top[8];
                top[9..18].fill(f);
                left = first18(&self.intern_border_y[8..]);
            }
        }
        (top, left)
    }

    fn load_intra_pred_chroma(&mut self) {
        let m = self.mbx * 10;
        if self.top_border_u.len() < m + 10 {
            return;
        }
        self.left_border_u[9] = self.left_border_u[8];
        self.left_border_v[9] = self.left_border_v[8];
        if self.flags & C_AVAIL != 0 {
            self.top_border_u[m + 9] = self.top_border_u.get(m + 11).copied().unwrap_or(0);
            self.top_border_v[m + 9] = self.top_border_v.get(m + 11).copied().unwrap_or(0);
        } else {
            self.top_border_u[m + 9] = self.top_border_u[m + 8];
            self.top_border_v[m + 9] = self.top_border_v[m + 8];
        }
        if self.flags & A_AVAIL != 0 && self.flags & B_AVAIL != 0 {
            self.top_border_u[m] = self.topleft_u;
            self.left_border_u[0] = self.topleft_u;
            self.top_border_v[m] = self.topleft_v;
            self.left_border_v[0] = self.topleft_v;
        } else {
            self.left_border_u[0] = self.left_border_u[1];
            self.left_border_v[0] = self.left_border_v[1];
            self.top_border_u[m] = self.top_border_u[m + 1];
            self.top_border_v[m] = self.top_border_v[m + 1];
        }
    }

    fn modify_mb_i(&mut self, pred_mode_uv: &mut u32) {
        // save the modes before they get modified
        self.pred_mode_y[3] = self.pred_mode_y[5];
        self.pred_mode_y[6] = self.pred_mode_y[8];
        let m = self.mbx * 2;
        if let Some(t) = self.top_pred_y.get_mut(m..m + 2) {
            t[0] = self.pred_mode_y[7];
            t[1] = self.pred_mode_y[8];
        }
        let modify = |table: &[i8], mode: i32| -> i32 {
            let v = table.get(usize::try_from(mode).unwrap_or(usize::MAX)).copied().unwrap_or(-1);
            if v < 0 { 0 } else { i32::from(v) }
        };
        if self.flags & A_AVAIL == 0 {
            self.pred_mode_y[4] = modify(&LEFT_MODIFIER_L, self.pred_mode_y[4]);
            self.pred_mode_y[7] = modify(&LEFT_MODIFIER_L, self.pred_mode_y[7]);
            *pred_mode_uv = modify(&LEFT_MODIFIER_C, *pred_mode_uv as i32) as u32;
        }
        if self.flags & B_AVAIL == 0 {
            self.pred_mode_y[4] = modify(&TOP_MODIFIER_L, self.pred_mode_y[4]);
            self.pred_mode_y[5] = modify(&TOP_MODIFIER_L, self.pred_mode_y[5]);
            *pred_mode_uv = modify(&TOP_MODIFIER_C, *pred_mode_uv as i32) as u32;
        }
    }

    fn decode_mb_i(&mut self, b: &mut Bits<'_>, cbp_code: u32) -> Result<(), Error> {
        self.init_mb();
        for &pos in &SCAN3X3 {
            let na = self.pred_mode_y[pos - 1];
            let nb = self.pred_mode_y[pos - 3];
            let mut predpred = na.min(nb);
            if predpred == -1 {
                predpred = dsp::INTRA_L_LP;
            }
            if !b.bit() {
                let rem = b.get(2) as i32;
                predpred = rem + i32::from(rem >= predpred);
            }
            self.pred_mode_y[pos] = predpred;
        }
        let mut pred_mode_uv = b.ue31().unwrap_or(u32::MAX);
        if pred_mode_uv > 6 {
            return Err(Error::Malformed("illegal intra chroma prediction mode"));
        }
        self.modify_mb_i(&mut pred_mode_uv);

        let mut cbp_code = cbp_code;
        if self.pict_type == PIC_I {
            cbp_code = b.ue().unwrap_or(u32::MAX);
        }
        let e = CBP_TAB.get(cbp_code as usize).ok_or(Error::Malformed("illegal intra coded block pattern"))?;
        self.cbp = u32::from(e[0]);
        if self.cbp != 0 && !self.qp_fixed {
            self.qp = self.qp.wrapping_add(b.se().unwrap_or(0)) & 63;
        }

        let (cy, ls) = (self.cy(), self.ls());
        let scan = [0, 8, 8 * ls, 8 * ls + 8];
        for block in 0..4 {
            let d = cy + scan[block];
            let (top, left) = self.load_intra_pred_luma(block);
            dsp::intra_luma(self.pred_mode_y[SCAN3X3[block]], &mut self.cur.y, d, ls, &top, &left);
            if self.cbp & (1 << block) != 0 {
                self.residual_block(b, &INTRA_DEC, 1, self.qp as usize, Plane::Y, d, ls)?;
            }
        }

        self.load_intra_pred_chroma();
        let (cc, cs, m) = (self.cc(), self.cs(), self.mbx * 10);
        let top_u = first18(self.top_border_u.get(m..m + 10).unwrap_or(&[]));
        let top_v = first18(self.top_border_v.get(m..m + 10).unwrap_or(&[]));
        let left_u = first18(&self.left_border_u[..]);
        let left_v = first18(&self.left_border_v[..]);
        dsp::intra_chroma(pred_mode_uv, &mut self.cur.u, cc, cs, &top_u, &left_u);
        dsp::intra_chroma(pred_mode_uv, &mut self.cur.v, cc, cs, &top_v, &left_v);

        self.residual_chroma(b)?;
        self.filter(I_8X8);
        // set_mv_intra
        self.mv[MV_FWD_X0] = INTRA_MV;
        set_mvs(&mut self.mv, MV_FWD_X0, BLK_16X16);
        self.mv[MV_BWD_X0] = INTRA_MV;
        set_mvs(&mut self.mv, MV_BWD_X0, BLK_16X16);
        if self.pict_type != PIC_B
            && let Some(t) = self.col_type.get_mut(self.mbidx)
        {
            *t = I_8X8 as u8;
        }
        Ok(())
    }

    fn set_intra_mode_default(&mut self) {
        let mode = if self.stream_revision > 0 { -1 } else { dsp::INTRA_L_LP };
        self.pred_mode_y[3] = mode;
        self.pred_mode_y[6] = mode;
        let m = self.mbx * 2;
        if let Some(t) = self.top_pred_y.get_mut(m..m + 2) {
            t[0] = mode;
            t[1] = mode;
        }
    }

    // -----------------------------------------------------------------------------------------
    // motion vectors

    fn scale_mv(&self, src: Mv, distp: i16) -> (i32, i32) {
        let den = i64::from(self.scale_den[usize::from(src.r > 0)]);
        let s = |v: i16| ((i64::from(v) * i64::from(distp) * den + 256 + i64::from(v >> 15)) >> 9) as i32;
        (s(src.x), s(src.y))
    }

    fn mv_pred_median(&mut self, np: usize, a: Mv, bb: Mv, c: Mv) {
        let dist = self.mv[np].dist;
        let (ax, ay) = self.scale_mv(a, dist);
        let (bx, by) = self.scale_mv(bb, dist);
        let (cx, cy) = self.scale_mv(c, dist);
        let len_ab = (ax - bx).abs() + (ay - by).abs();
        let len_bc = (bx - cx).abs() + (by - cy).abs();
        let len_ca = (cx - ax).abs() + (cy - ay).abs();
        let len_mid = mid_pred(len_ab, len_bc, len_ca);
        let (x, y) = if len_mid == len_ab {
            (cx, cy)
        } else if len_mid == len_bc {
            (ax, ay)
        } else {
            (bx, by)
        };
        self.mv[np].x = x as i16;
        self.mv[np].y = y as i16;
    }

    /// `ff_cavs_mv`: predicts the vector at `np` (with `nc` as the top-right candidate) and
    /// adds the coded difference.
    fn pred_mv(&mut self, b: &mut Bits<'_>, np: usize, nc: usize, mode: u32, size: u32, r: i16) {
        self.mv[np].r = r;
        self.mv[np].dist = self.dist[usize::from(r > 0)] as i16;
        let a = self.mv[np - 1];
        let bb = self.mv[np - 4];
        let mut c = self.mv[nc];
        if c.r == NOT_AVAIL || np == MV_FWD_X3 || np == MV_FWD_X3 + MV_BWD_OFFS {
            c = self.mv[np - 5];
        }
        let p2 = if mode == MV_PRED_PSKIP && (a.r == NOT_AVAIL || bb.r == NOT_AVAIL || (a.x | a.y | a.r) == 0 || (bb.x | bb.y | bb.r) == 0) {
            Some(UN_MV)
        } else if a.r >= 0 && bb.r < 0 && c.r < 0 {
            Some(a)
        } else if a.r < 0 && bb.r >= 0 && c.r < 0 {
            Some(bb)
        } else if a.r < 0 && bb.r < 0 && c.r >= 0 {
            Some(c)
        } else if mode == MV_PRED_LEFT && a.r == r {
            Some(a)
        } else if mode == MV_PRED_TOP && bb.r == r {
            Some(bb)
        } else if mode == MV_PRED_TOPRIGHT && c.r == r {
            Some(c)
        } else {
            None
        };
        match p2 {
            Some(p) => {
                self.mv[np].x = p.x;
                self.mv[np].y = p.y;
            }
            None => self.mv_pred_median(np, a, bb, c),
        }
        if mode < MV_PRED_PSKIP {
            let mx = b.se().unwrap_or(0).wrapping_add(i32::from(self.mv[np].x));
            let my = b.se().unwrap_or(0).wrapping_add(i32::from(self.mv[np].y));
            if i32::from(mx as i16) == mx && i32::from(my as i16) == my {
                self.mv[np].x = mx as i16;
                self.mv[np].y = my as i16;
            }
        }
        set_mvs(&mut self.mv, np, size);
    }

    fn mv_pred_direct(&mut self, fw: usize, col: Mv) {
        let bw = fw + MV_BWD_OFFS;
        let den = self.direct_den[usize::from(col.r > 0)] as u32;
        self.mv[fw].dist = self.dist[1] as i16;
        self.mv[bw].dist = self.dist[0] as i16;
        self.mv[fw].r = 1;
        self.mv[bw].r = 0;
        let fd = self.mv[fw].dist as i32 as u32;
        let bd = self.mv[bw].dist as i32 as u32;
        let scale = |v: i16, dist: u32| {
            let m = (i32::from(v) >> 31) as u32;
            let prod = den.wrapping_mul(i32::from(v) as u32).wrapping_mul(dist);
            (den.wrapping_add(prod ^ m).wrapping_sub(m).wrapping_sub(1) >> 14) ^ m
        };
        let mx = (i32::from(col.x) >> 31) as u32;
        let my = (i32::from(col.y) >> 31) as u32;
        self.mv[fw].x = scale(col.x, fd).wrapping_sub(mx) as i16;
        self.mv[bw].x = mx.wrapping_sub(scale(col.x, bd)) as i16;
        self.mv[fw].y = scale(col.y, fd).wrapping_sub(my) as i16;
        self.mv[bw].y = my.wrapping_sub(scale(col.y, bd)) as i16;
    }

    fn mv_pred_sym(&mut self, src: usize, size: u32) {
        let dst = src + MV_BWD_OFFS;
        let s = self.mv[src];
        self.mv[dst].x = (-((i32::from(s.x) * self.sym_factor + 256) >> 9)) as i16;
        self.mv[dst].y = (-((i32::from(s.y) * self.sym_factor + 256) >> 9)) as i16;
        self.mv[dst].r = 0;
        self.mv[dst].dist = self.dist[0] as i16;
        set_mvs(&mut self.mv, dst, size);
    }

    fn col(&self, block: usize) -> Mv {
        self.col_mv.get(self.mbidx * 4 + block).copied().unwrap_or_default()
    }

    fn col_intra(&self) -> bool {
        self.col_type.get(self.mbidx).copied().unwrap_or(0) == 0
    }

    // -----------------------------------------------------------------------------------------
    // inter macroblocks

    fn decode_mb_p(&mut self, b: &mut Bits<'_>, mb_type: u32) {
        self.init_mb();
        let rf = |b: &mut Bits<'_>, flag: bool| -> i16 { if flag { 0 } else { i16::from(b.bit()) } };
        let flag = self.ref_flag;
        match mb_type {
            P_SKIP => self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_PSKIP, BLK_16X16, 0),
            P_16X16 => {
                let r0 = rf(b, flag);
                self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_MEDIAN, BLK_16X16, r0);
            }
            P_16X8 => {
                let r0 = rf(b, flag);
                let r2 = rf(b, flag);
                self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_TOP, BLK_16X8, r0);
                self.pred_mv(b, MV_FWD_X2, MV_FWD_A1, MV_PRED_LEFT, BLK_16X8, r2);
            }
            P_8X16 => {
                let r0 = rf(b, flag);
                let r1 = rf(b, flag);
                self.pred_mv(b, MV_FWD_X0, MV_FWD_B3, MV_PRED_LEFT, BLK_8X16, r0);
                self.pred_mv(b, MV_FWD_X1, MV_FWD_C2, MV_PRED_TOPRIGHT, BLK_8X16, r1);
            }
            _ => {
                let r: [i16; 4] = std::array::from_fn(|_| rf(b, flag));
                self.pred_mv(b, MV_FWD_X0, MV_FWD_B3, MV_PRED_MEDIAN, BLK_8X8, r[0]);
                self.pred_mv(b, MV_FWD_X1, MV_FWD_C2, MV_PRED_MEDIAN, BLK_8X8, r[1]);
                self.pred_mv(b, MV_FWD_X2, MV_FWD_X1, MV_PRED_MEDIAN, BLK_8X8, r[2]);
                self.pred_mv(b, MV_FWD_X3, MV_FWD_X0, MV_PRED_MEDIAN, BLK_8X8, r[3]);
            }
        }
        self.inter(mb_type);
        self.set_intra_mode_default();
        // store_mvs
        let base = self.mbidx * 4;
        if let Some(c) = self.col_mv.get_mut(base..base + 4) {
            for (k, &i) in MV_SCAN.iter().enumerate() {
                c[k] = self.mv[i];
            }
        }
        if mb_type != P_SKIP {
            self.residual_inter(b);
        }
        self.filter(mb_type);
        if let Some(t) = self.col_type.get_mut(self.mbidx) {
            *t = mb_type as u8;
        }
    }

    fn decode_mb_b(&mut self, b: &mut Bits<'_>, mb_type: u32) -> Result<(), Error> {
        self.init_mb();
        self.mv[MV_FWD_X0] = DIR_MV;
        set_mvs(&mut self.mv, MV_FWD_X0, BLK_16X16);
        self.mv[MV_BWD_X0] = DIR_MV;
        set_mvs(&mut self.mv, MV_BWD_X0, BLK_16X16);
        match mb_type {
            B_SKIP | B_DIRECT => {
                if self.col_intra() {
                    self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_BSKIP, BLK_16X16, 1);
                    self.pred_mv(b, MV_BWD_X0, MV_BWD_C2, MV_PRED_BSKIP, BLK_16X16, 0);
                } else {
                    for block in 0..4 {
                        let col = self.col(block);
                        self.mv_pred_direct(MV_SCAN[block], col);
                    }
                }
            }
            B_FWD_16X16 => self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_MEDIAN, BLK_16X16, 1),
            B_SYM_16X16 => {
                self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_MEDIAN, BLK_16X16, 1);
                self.mv_pred_sym(MV_FWD_X0, BLK_16X16);
            }
            B_BWD_16X16 => self.pred_mv(b, MV_BWD_X0, MV_BWD_C2, MV_PRED_MEDIAN, BLK_16X16, 0),
            B_8X8 => {
                const TMP: usize = 7;
                let mut flags = 0usize;
                let sub: [u32; 4] = std::array::from_fn(|_| b.get(2));
                for block in 0..4 {
                    let s = MV_SCAN[block];
                    match sub[block] {
                        B_SUB_DIRECT => {
                            if self.col_intra() {
                                if flags == 0 {
                                    // a co-located intra macroblock: one 16×16 prediction
                                    if block > 0 {
                                        self.mv[TMP] = self.mv[MV_FWD_X0];
                                        self.mv[TMP + MV_BWD_OFFS] = self.mv[MV_BWD_X0];
                                    }
                                    self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_BSKIP, BLK_8X8, 1);
                                    self.pred_mv(b, MV_BWD_X0, MV_BWD_C2, MV_PRED_BSKIP, BLK_8X8, 0);
                                    if block > 0 {
                                        flags = s;
                                        self.mv[flags] = self.mv[MV_FWD_X0];
                                        self.mv[flags + MV_BWD_OFFS] = self.mv[MV_BWD_X0];
                                        self.mv[MV_FWD_X0] = self.mv[TMP];
                                        self.mv[MV_BWD_X0] = self.mv[TMP + MV_BWD_OFFS];
                                    } else {
                                        flags = MV_FWD_X0;
                                    }
                                } else {
                                    self.mv[s] = self.mv[flags];
                                    self.mv[s + MV_BWD_OFFS] = self.mv[flags + MV_BWD_OFFS];
                                }
                            } else {
                                let col = self.col(block);
                                self.mv_pred_direct(s, col);
                            }
                        }
                        B_SUB_FWD => self.pred_mv(b, s, s - 3, MV_PRED_MEDIAN, BLK_8X8, 1),
                        B_SUB_SYM => {
                            self.pred_mv(b, s, s - 3, MV_PRED_MEDIAN, BLK_8X8, 1);
                            self.mv_pred_sym(s, BLK_8X8);
                        }
                        _ => {}
                    }
                }
                for block in 0..4 {
                    if sub[block] == B_SUB_BWD {
                        let s = MV_SCAN[block] + MV_BWD_OFFS;
                        self.pred_mv(b, s, s - 3, MV_PRED_MEDIAN, BLK_8X8, 0);
                    }
                }
            }
            _ => {
                if mb_type <= B_SYM_16X16 || mb_type > B_8X8 {
                    return Err(Error::Malformed("invalid macroblock type in a B picture"));
                }
                let flags = PARTITION_FLAGS[mb_type as usize];
                if mb_type & 1 == 1 {
                    // 16×8
                    if flags & FWD0 != 0 {
                        self.pred_mv(b, MV_FWD_X0, MV_FWD_C2, MV_PRED_TOP, BLK_16X8, 1);
                    }
                    if flags & SYM0 != 0 {
                        self.mv_pred_sym(MV_FWD_X0, BLK_16X8);
                    }
                    if flags & FWD1 != 0 {
                        self.pred_mv(b, MV_FWD_X2, MV_FWD_A1, MV_PRED_LEFT, BLK_16X8, 1);
                    }
                    if flags & SYM1 != 0 {
                        self.mv_pred_sym(MV_FWD_X2, BLK_16X8);
                    }
                    if flags & BWD0 != 0 {
                        self.pred_mv(b, MV_BWD_X0, MV_BWD_C2, MV_PRED_TOP, BLK_16X8, 0);
                    }
                    if flags & BWD1 != 0 {
                        self.pred_mv(b, MV_BWD_X2, MV_BWD_A1, MV_PRED_LEFT, BLK_16X8, 0);
                    }
                } else {
                    // 8×16
                    if flags & FWD0 != 0 {
                        self.pred_mv(b, MV_FWD_X0, MV_FWD_B3, MV_PRED_LEFT, BLK_8X16, 1);
                    }
                    if flags & SYM0 != 0 {
                        self.mv_pred_sym(MV_FWD_X0, BLK_8X16);
                    }
                    if flags & FWD1 != 0 {
                        self.pred_mv(b, MV_FWD_X1, MV_FWD_C2, MV_PRED_TOPRIGHT, BLK_8X16, 1);
                    }
                    if flags & SYM1 != 0 {
                        self.mv_pred_sym(MV_FWD_X1, BLK_8X16);
                    }
                    if flags & BWD0 != 0 {
                        self.pred_mv(b, MV_BWD_X0, MV_BWD_B3, MV_PRED_LEFT, BLK_8X16, 0);
                    }
                    if flags & BWD1 != 0 {
                        self.pred_mv(b, MV_BWD_X1, MV_BWD_C2, MV_PRED_TOPRIGHT, BLK_8X16, 0);
                    }
                }
            }
        }
        self.inter(mb_type);
        self.set_intra_mode_default();
        if mb_type != B_SKIP {
            self.residual_inter(b);
        }
        self.filter(mb_type);
        Ok(())
    }

    /// Motion compensation of a macroblock (`ff_cavs_inter`).
    fn inter(&mut self, mb_type: u32) {
        if PARTITION_FLAGS.get(mb_type as usize).copied().unwrap_or(0) == 0 {
            self.mc_part(16, 0, 0, MV_FWD_X0);
        } else {
            self.mc_part(8, 0, 0, MV_FWD_X0);
            self.mc_part(8, 4, 0, MV_FWD_X1);
            self.mc_part(8, 0, 4, MV_FWD_X2);
            self.mc_part(8, 4, 4, MV_FWD_X3);
        }
    }

    /// `mc_part_std`: forward prediction, then backward (averaged when both exist). Offsets are
    /// in chroma samples.
    fn mc_part(&mut self, size: usize, xo: usize, yo: usize, mvi: usize) {
        let (mb_w, mb_h) = self.dims();
        let (ls, cs) = (self.ls(), self.cs());
        let dy = self.cy() + xo * 2 + yo * 2 * ls;
        let dc = self.cc() + xo + yo * cs;
        let x_off = (xo + 8 * self.mbx) as i32;
        let y_off = (yo + 8 * self.mby) as i32;
        let (pw, ph) = ((16 * mb_w) as i32, (16 * mb_h) as i32);
        let mut avg = false;
        for (m, refi) in [(self.mv[mvi], None), (self.mv[mvi + MV_BWD_OFFS], Some(0usize))] {
            if m.r < 0 {
                continue;
            }
            let idx = refi.unwrap_or(m.r as usize);
            if let Some(pic) = self.dpb.get(idx).and_then(Option::as_ref) {
                let mx = i32::from(m.x) + x_off * 8;
                let my = i32::from(m.y) + y_off * 8;
                let xy = ((mx & 3) + ((my & 3) << 2)) as u32;
                let p = &pic.planes;
                let yref = RefPlane { data: &p.y, stride: ls, width: pw, height: ph };
                dsp::luma_mc(&yref, mx >> 2, my >> 2, xy, size, &mut self.cur.y, dy, ls, avg);
                for (src, dst) in [(&p.u, &mut self.cur.u), (&p.v, &mut self.cur.v)] {
                    let cref = RefPlane { data: src, stride: cs, width: pw >> 1, height: ph >> 1 };
                    dsp::chroma_mc(&cref, mx >> 3, my >> 3, mx & 7, my & 7, size / 2, size / 2, dst, dc, cs, avg);
                }
            }
            avg = true;
        }
    }

    // -----------------------------------------------------------------------------------------
    // in-loop filter

    fn edge_params(&self, qp_avg: i32) -> Edge {
        let a = (qp_avg + self.alpha_offset).clamp(0, 63) as usize;
        let bt = (qp_avg + self.beta_offset).clamp(0, 63) as usize;
        Edge { alpha: i32::from(ALPHA_TAB[a]), beta: i32::from(BETA_TAB[bt]), tc: i32::from(TC_TAB[a]) }
    }

    /// `ff_cavs_filter`: saves the un-deblocked borders for intra prediction, then filters the
    /// macroblock's edges.
    fn filter(&mut self, mb_type: u32) {
        let (cy, cc, ls, cs) = (self.cy(), self.cc(), self.ls(), self.cs());
        let (m16, m10) = (self.mbx * 16, self.mbx * 10);
        self.topleft_y = self.top_border_y.get(m16 + 15).copied().unwrap_or(0);
        self.topleft_u = self.top_border_u.get(m10 + 8).copied().unwrap_or(0);
        self.topleft_v = self.top_border_v.get(m10 + 8).copied().unwrap_or(0);
        for i in 0..16 {
            let v = self.y_at(cy + 15 * ls + i);
            if let Some(t) = self.top_border_y.get_mut(m16 + i) {
                *t = v;
            }
        }
        for i in 0..8 {
            let u = self.cur.u.get(cc + 7 * cs + i).copied().unwrap_or(0);
            let v = self.cur.v.get(cc + 7 * cs + i).copied().unwrap_or(0);
            if let Some(t) = self.top_border_u.get_mut(m10 + 1 + i) {
                *t = u;
            }
            if let Some(t) = self.top_border_v.get_mut(m10 + 1 + i) {
                *t = v;
            }
        }
        for i in 0..8 {
            self.left_border_y[i * 2 + 1] = self.y_at(cy + 15 + (i * 2) * ls);
            self.left_border_y[i * 2 + 2] = self.y_at(cy + 15 + (i * 2 + 1) * ls);
            self.left_border_u[i + 1] = self.cur.u.get(cc + 7 + i * cs).copied().unwrap_or(0);
            self.left_border_v[i + 1] = self.cur.v.get(cc + 7 + i * cs).copied().unwrap_or(0);
        }
        if !self.loop_filter_disable {
            let mut bs = [0u8; 8];
            if mb_type == I_8X8 {
                bs = [2; 8];
            } else {
                let f = PARTITION_FLAGS.get(mb_type as usize).copied().unwrap_or(0);
                let bb = mb_type > P_8X8;
                let mv = &self.mv;
                if f & SPLITV != 0 {
                    bs[2] = get_bs(mv, MV_FWD_X0, MV_FWD_X1, bb);
                    bs[3] = get_bs(mv, MV_FWD_X2, MV_FWD_X3, bb);
                }
                if f & SPLITH != 0 {
                    bs[6] = get_bs(mv, MV_FWD_X0, MV_FWD_X2, bb);
                    bs[7] = get_bs(mv, MV_FWD_X1, MV_FWD_X3, bb);
                }
                bs[0] = get_bs(mv, MV_FWD_A1, MV_FWD_X0, bb);
                bs[1] = get_bs(mv, MV_FWD_A3, MV_FWD_X2, bb);
                bs[4] = get_bs(mv, MV_FWD_B2, MV_FWD_X0, bb);
                bs[5] = get_bs(mv, MV_FWD_B3, MV_FWD_X1, bb);
            }
            if bs.iter().any(|&v| v != 0) {
                let qp = self.qp;
                let cqp = |q: i32| i32::from(CHROMA_QP[(q & 63) as usize]);
                if self.flags & A_AVAIL != 0 {
                    let e = self.edge_params((qp + self.left_qp + 1) >> 1);
                    dsp::filter_luma(&mut self.cur.y, cy, ls, true, e, bs[0], bs[1]);
                    let e = self.edge_params((cqp(qp) + cqp(self.left_qp) + 1) >> 1);
                    dsp::filter_chroma(&mut self.cur.u, cc, cs, true, e, bs[0], bs[1]);
                    dsp::filter_chroma(&mut self.cur.v, cc, cs, true, e, bs[0], bs[1]);
                }
                let e = self.edge_params(qp);
                dsp::filter_luma(&mut self.cur.y, cy + 8, ls, true, e, bs[2], bs[3]);
                dsp::filter_luma(&mut self.cur.y, cy + 8 * ls, ls, false, e, bs[6], bs[7]);
                if self.flags & B_AVAIL != 0 {
                    let top = i32::from(self.top_qp.get(self.mbx).copied().unwrap_or(0));
                    let e = self.edge_params((qp + top + 1) >> 1);
                    dsp::filter_luma(&mut self.cur.y, cy, ls, false, e, bs[4], bs[5]);
                    let e = self.edge_params((cqp(qp) + cqp(top) + 1) >> 1);
                    dsp::filter_chroma(&mut self.cur.u, cc, cs, false, e, bs[4], bs[5]);
                    dsp::filter_chroma(&mut self.cur.v, cc, cs, false, e, bs[4], bs[5]);
                }
            }
        }
        self.left_qp = self.qp;
        if let Some(t) = self.top_qp.get_mut(self.mbx) {
            *t = self.qp as u8;
        }
    }
}

/// The first 18 values of `s` (zeros past its end).
fn first18(s: &[u8]) -> [u8; 18] {
    std::array::from_fn(|i| s.get(i).copied().unwrap_or(0))
}
