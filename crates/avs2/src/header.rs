// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/header.cc`, `alf.cc`), Copyright (C) 2018~ VCL, NELVT, Peking University
// (Falei Luo et al.), GPL-2.0-or-later.
//! Sequence, picture and slice headers and the ALF parameters of a picture header.

use crate::Error;
use crate::bits::Bits;

pub(crate) const MAIN10_PROFILE: u32 = 0x22;

/// Most reference picture sets in a sequence header.
pub(crate) const GOP_NUM: usize = 32;
/// Most reference pictures of a picture.
pub(crate) const MAX_REFS: usize = 4;
pub(crate) const ALF_NUM_VARS: usize = 16;
pub(crate) const ALF_MAX_NUM_COEF: usize = 9;

/// The frame rates of `frame_rate_code` 1..=13 as (numerator, denominator).
pub const FRAME_RATES: [(u32, u32); 13] =
    [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1), (100, 1), (120, 1), (200, 1), (240, 1), (300, 1)];

/// A reference picture set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rps {
    pub ref_pic: [u32; MAX_REFS],
    pub remove_pic: [u32; 8],
    pub num_of_ref: usize,
    pub num_to_remove: usize,
    pub refered_by_others: bool,
}

fn read_rps(bs: &mut Bits<'_>) -> Result<Rps, Error> {
    let mut r = Rps { refered_by_others: bs.flag()?, num_of_ref: bs.u(3)? as usize, ..Rps::default() };
    if r.num_of_ref > MAX_REFS {
        return Err(Error::Invalid("number of reference pictures"));
    }
    for k in 0..r.num_of_ref {
        r.ref_pic[k] = bs.u(6)?;
    }
    r.num_to_remove = bs.u(3)? as usize;
    for k in 0..r.num_to_remove {
        r.remove_pic[k] = bs.u(6)?;
    }
    bs.u(1)?; // marker
    Ok(r)
}

/// The sequence header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub profile_id: u32,
    pub level_id: u32,
    pub progressive: bool,
    pub field_coding: bool,
    pub width: u32,
    pub height: u32,
    /// 0: 4:0:0, 1: 4:2:0.
    pub chroma_format: u32,
    /// Bits per sample of the decoded pictures (8 or 10).
    pub bit_depth: u32,
    /// Bits per sample of the source (`sample_precision`).
    pub sample_bit_depth: u32,
    pub aspect_ratio: u32,
    pub frame_rate_code: u32,
    pub low_delay: bool,
    pub temporal_id_exist: bool,
    pub log2_lcu_size: u32,
    pub weighted_quant: bool,
    pub background_picture: bool,
    pub mhp_skip: bool,
    pub dhp: bool,
    pub wsm: bool,
    pub amp: bool,
    pub nsqt: bool,
    pub sdip: bool,
    pub second_transform: bool,
    pub sao: bool,
    pub alf: bool,
    pub pmvr: bool,
    pub(crate) rps: Vec<Rps>,
    pub picture_reorder_delay: u32,
    pub cross_loop_filter: bool,
}

impl Sequence {
    /// `parse_sequence_header`: `data` starts after the `00 00 01 B0` start code.
    /// Whether `data` (a sequence header up to the next start code) is a well-formed AVS2
    /// sequence header: every marker bit set, 4:2:0, a known frame rate, and the header fits. AVS1
    /// uses the same start codes and profile 0x20, so sniffing needs this (an AVS1 header is too
    /// short and its marker bits fall elsewhere).
    pub(crate) fn plausible(data: &[u8]) -> bool {
        let check = || -> Result<bool, Error> {
            let mut bs = Bits::new(data);
            let profile_id = bs.u(8)?;
            bs.u(8)?; // level
            bs.u(2)?; // progressive_sequence, field_coded_sequence
            bs.u(28)?; // size
            let mut ok = bs.u(2)? == 1; // 4:2:0
            bs.u(3)?;
            if profile_id == MAIN10_PROFILE {
                bs.u(3)?;
            }
            bs.u(4)?;
            ok &= (1..=13).contains(&bs.u(4)?);
            bs.u(18)?;
            ok &= bs.u(1)? == 1;
            bs.u(12)?;
            bs.u(1)?;
            ok &= bs.u(1)? == 1;
            bs.u(19)?;
            ok &= (4..=6).contains(&bs.u(3)?);
            if bs.flag()? && bs.flag()? {
                for _ in 0..(16 + 64) {
                    bs.ue()?;
                }
            }
            bs.u(11)?;
            ok &= bs.u(1)? == 1;
            Ok(ok && Self::parse(data).is_ok())
        };
        check().unwrap_or(false)
    }

    pub(crate) fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut bs = Bits::new(data);
        let profile_id = bs.u(8)?;
        let level_id = bs.u(8)?;
        let progressive = bs.flag()?;
        let field_coding = bs.flag()?;
        let width = bs.u(14)?;
        let height = bs.u(14)?;
        if width < 16 || height < 16 {
            return Err(Error::Invalid("picture size"));
        }
        let chroma_format = bs.u(2)?;
        if chroma_format != 1 && chroma_format != 0 {
            return Err(Error::Unsupported("chroma format"));
        }
        let sample_precision = bs.u(3)?;
        let encoding_precision = if profile_id == MAIN10_PROFILE { bs.u(3)? } else { 1 };
        if !(1..=3).contains(&sample_precision) || !(1..=3).contains(&encoding_precision) {
            return Err(Error::Invalid("sample precision"));
        }
        let aspect_ratio = bs.u(4)?;
        let frame_rate_code = bs.u(4)?;
        bs.u(18)?; // bit_rate_lower
        bs.u(1)?;
        bs.u(12)?; // bit_rate_upper
        let low_delay = bs.flag()?;
        bs.u(1)?;
        let temporal_id_exist = bs.flag()?;
        bs.u(18)?; // bbv buffer size
        let log2_lcu_size = bs.u(3)?;
        if !(4..=6).contains(&log2_lcu_size) {
            return Err(Error::Invalid("LCU size"));
        }
        let weighted_quant = bs.flag()?;
        if weighted_quant && bs.flag()? {
            // load_seq_weight_quant_data_flag: 4x4 then 8x8 matrices
            for _ in 0..(16 + 64) {
                bs.ue()?;
            }
        }
        let background_picture = !bs.flag()?;
        let mhp_skip = bs.flag()?;
        let dhp = bs.flag()?;
        let wsm = bs.flag()?;
        let amp = bs.flag()?;
        let nsqt = bs.flag()?;
        let sdip = bs.flag()?;
        let second_transform = bs.flag()?;
        let sao = bs.flag()?;
        let alf = bs.flag()?;
        let pmvr = bs.flag()?;
        bs.u(1)?; // marker
        let num_of_rps = bs.u(6)? as usize;
        if num_of_rps > GOP_NUM {
            return Err(Error::Invalid("number of reference picture sets"));
        }
        let rps = (0..num_of_rps).map(|_| read_rps(&mut bs)).collect::<Result<Vec<_>, _>>()?;
        let picture_reorder_delay = if low_delay { 0 } else { bs.u(5)? };
        let cross_loop_filter = bs.flag()?;
        let frame_rate_code = frame_rate_code.clamp(1, 13);
        Ok(Self {
            profile_id,
            level_id,
            progressive,
            field_coding,
            width,
            height,
            chroma_format,
            bit_depth: 6 + (encoding_precision << 1),
            sample_bit_depth: 6 + (sample_precision << 1),
            aspect_ratio,
            frame_rate_code,
            low_delay,
            temporal_id_exist,
            log2_lcu_size,
            weighted_quant,
            background_picture,
            mhp_skip,
            dhp,
            wsm,
            amp,
            nsqt,
            sdip,
            second_transform,
            sao,
            alf,
            pmvr,
            rps,
            picture_reorder_delay,
            cross_loop_filter,
        })
    }

    /// The frame rate as (numerator, denominator).
    pub fn frame_rate(&self) -> (u32, u32) {
        FRAME_RATES.get(self.frame_rate_code.saturating_sub(1) as usize).copied().unwrap_or((25, 1))
    }
}

/// Picture types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    I,
    P,
    B,
    /// Forward (multi-hypothesis) picture.
    F,
    /// Background picture that is output.
    G,
    /// Background picture that is not output.
    Gb,
    /// P picture predicted from the background picture only.
    S,
}

impl FrameType {
    pub(crate) fn is_intra(self) -> bool {
        matches!(self, FrameType::I | FrameType::G | FrameType::Gb)
    }
}

/// ALF parameters of one component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AlfParam {
    pub filters_per_group: usize,
    pub filter_pattern: [bool; ALF_NUM_VARS],
    pub coeffmulti: [[i32; ALF_MAX_NUM_COEF]; ALF_NUM_VARS],
}

impl Default for AlfParam {
    fn default() -> Self {
        Self { filters_per_group: 1, filter_pattern: [false; ALF_NUM_VARS], coeffmulti: [[0; ALF_MAX_NUM_COEF]; ALF_NUM_VARS] }
    }
}

/// `vlc_read_alf_coeff`.
fn read_alf_coeff(bs: &mut Bits<'_>, p: &mut AlfParam, luma: bool) -> Result<(), Error> {
    if !luma {
        for c in p.coeffmulti[0].iter_mut() {
            *c = bs.se()?;
        }
        return Ok(());
    }
    let n = bs.ue()? as usize + 1;
    if n > ALF_NUM_VARS {
        return Err(Error::Invalid("ALF filter number"));
    }
    p.filters_per_group = n;
    p.filter_pattern = [false; ALF_NUM_VARS];
    let mut pre = 0usize;
    for f in 0..n {
        if f > 0 {
            let symbol = if n != 16 { bs.ue()? as usize } else { 1 };
            let at = symbol.saturating_add(pre);
            *p.filter_pattern.get_mut(at).ok_or(Error::Invalid("ALF region distance"))? = true;
            pre = at;
        }
        for c in p.coeffmulti[f].iter_mut() {
            *c = bs.se()?;
        }
    }
    Ok(())
}

/// A picture header.
#[derive(Clone, Debug)]
pub(crate) struct Picture {
    pub frame_type: FrameType,
    pub coi: u32,
    pub display_delay: u32,
    pub rps: Rps,
    /// `picture_structure`: true for a frame.
    pub frame: bool,
    pub fixed_picture_qp: bool,
    pub picture_qp: i32,
    pub background_reference: bool,
    pub loop_filter: bool,
    pub alpha_offset: i32,
    pub beta_offset: i32,
    pub chroma_quant_delta_u: i32,
    pub chroma_quant_delta_v: i32,
    pub alf_on: [bool; 3],
    pub alf: [AlfParam; 3],
}

impl Picture {
    /// `parse_picture_header_intra` / `_inter`: `data` starts after the start code (with the
    /// stuffing bits removed). `prev_alf` holds the ALF parameters of the previous picture: a
    /// component whose flag is off keeps them, as davs2 keeps `img_param` between pictures.
    /// Returns the header and the byte offset where the picture's first slice may start.
    pub(crate) fn parse(data: &[u8], intra: bool, seq: &Sequence, prev_alf: &[AlfParam; 3]) -> Result<(Self, usize), Error> {
        let mut bs = Bits::new(data);
        bs.u(32)?; // bbv_delay
        let mut frame_type = FrameType::I;
        let mut pic_struct = 0;
        let mut background_reference = false;
        if intra {
            if bs.flag()? {
                bs.u(24)?; // time_code
            }
            if seq.background_picture && bs.flag()? {
                frame_type = if bs.flag()? { FrameType::G } else { FrameType::Gb };
            }
        } else {
            pic_struct = bs.u(2)?;
            let mut background_pred = false;
            if seq.background_picture && (pic_struct == 1 || pic_struct == 3) {
                if pic_struct == 1 {
                    background_pred = bs.flag()?;
                }
                if !background_pred {
                    background_reference = bs.flag()?;
                }
            }
            frame_type = match pic_struct {
                1 if background_pred => FrameType::S,
                1 => FrameType::P,
                3 => FrameType::F,
                _ => FrameType::B,
            };
        }
        let coi = bs.u(8)?;
        if seq.temporal_id_exist {
            bs.u(3)?; // temporal_id
        }
        let mut display_delay = 0;
        if !seq.low_delay {
            display_delay = bs.ue()?;
            if display_delay >= 64 {
                return Err(Error::Invalid("picture output delay"));
            }
        }
        let rps = if bs.flag()? {
            let index = bs.u(5)? as usize;
            *seq.rps.get(index).ok_or(Error::Invalid("reference picture set index"))?
        } else {
            read_rps(&mut bs)?
        };
        if seq.low_delay {
            bs.ue()?; // bbv check times
        }
        let progressive_frame = bs.flag()?;
        let frame = if progressive_frame { true } else { bs.u(1)? == 1 };
        bs.flag()?; // top_field_first
        bs.flag()?; // repeat_first_field
        if seq.field_coding {
            bs.flag()?; // is_top_field
            bs.u(1)?;
        }
        let fixed_picture_qp = bs.flag()?;
        let picture_qp = bs.u(7)? as i32;
        if !intra {
            if !(pic_struct == 2 && frame) {
                bs.u(1)?; // reserved
            }
            bs.flag()?; // random_access_decodable_flag
        }
        let loop_filter = !bs.flag()?;
        let (mut alpha_offset, mut beta_offset) = (0, 0);
        if loop_filter && bs.flag()? {
            alpha_offset = bs.se()?;
            beta_offset = bs.se()?;
        }
        let (mut chroma_quant_delta_u, mut chroma_quant_delta_v) = (0, 0);
        if !bs.flag()? {
            chroma_quant_delta_u = bs.se()?;
            chroma_quant_delta_v = bs.se()?;
        }
        // davs2 clears the sequence's weighted-quantisation flag before reading the picture's
        // weighting data, so it never reads it: such streams are not decoded
        if seq.weighted_quant {
            return Err(Error::Unsupported("weighted quantisation"));
        }
        let mut alf_on = [false; 3];
        let mut alf = *prev_alf;
        if seq.alf {
            for on in alf_on.iter_mut() {
                *on = bs.flag()?;
            }
            for (c, p) in alf.iter_mut().enumerate() {
                if alf_on[c] {
                    read_alf_coeff(&mut bs, p, c == 0)?;
                }
            }
        }
        bs.align();
        let header = Self {
            frame_type,
            coi,
            display_delay,
            rps,
            frame,
            fixed_picture_qp,
            picture_qp,
            background_reference,
            loop_filter,
            alpha_offset,
            beta_offset,
            chroma_quant_delta_u,
            chroma_quant_delta_v,
            alf_on,
            alf,
        };
        Ok((header, bs.byte_pos()))
    }
}

/// A slice header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Slice {
    pub qp: i32,
    /// Whether CUs may change the QP.
    pub dqp: bool,
    pub sao_on: [bool; 3],
    /// Bit position (in `data`) where the slice data starts.
    pub data_bit_pos: usize,
}

impl Slice {
    /// `parse_slice_header`: `data` starts after `00 00 01`, at the slice's start code value.
    pub(crate) fn parse(data: &[u8], seq: &Sequence, pic: &Picture, sao: bool) -> Result<Self, Error> {
        let mut bs = Bits::new(data);
        let lcu = 1u32 << seq.log2_lcu_size;
        // The slice position is implied by decoding order (as in davs2), so it is only skipped.
        bs.u(8)?; // slice_vertical_position
        if seq.height > 144 * lcu {
            bs.u(3)?; // slice_vertical_position_extension
        }
        bs.u(8)?; // slice_horizontal_position
        if seq.width > 255 * lcu {
            bs.u(2)?; // slice_horizontal_position_extension
        }
        let (qp, dqp) = if !pic.fixed_picture_qp {
            let fixed = bs.flag()?;
            (bs.u(7)? as i32, !fixed)
        } else {
            (pic.picture_qp, false)
        };
        let mut sao_on = [false; 3];
        if sao {
            for s in sao_on.iter_mut() {
                *s = bs.flag()?;
            }
        }
        Ok(Self { qp, dqp, sao_on, data_bit_pos: bs.pos() })
    }
}
