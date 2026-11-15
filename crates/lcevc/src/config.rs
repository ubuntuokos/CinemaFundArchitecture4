// SPDX-License-Identifier: BSD-3-Clause-Clear
// Copyright (c) V-Nova International Limited 2014-2026. All rights reserved.
// Ported to Rust for CFA4 from LCEVCdec 4.3.1 (https://github.com/v-novaltd/LCEVCdec, commit
// e295d843). As LCEVCdec requires, this file stays under the BSD-3-Clause-Clear licence and the
// additional licensing information in `crates/lcevc/LICENSE.LCEVCdec`: NO PATENT LICENCES ARE
// GRANTED by it.
//! Block syntax of an LCEVC access unit (LCEVCdec `config_parser.c`, `chunk_parser.c`): sequence
//! and global configuration, picture configuration and the encoded data chunks.

use crate::Error;
use crate::bits::{Bits, Bytes};

/// Chroma sampling of the enhanced picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    Mono,
    C420,
    C422,
    C444,
}

impl Chroma {
    /// Horizontal and vertical chroma subsampling shifts.
    pub fn shifts(self) -> (u32, u32) {
        match self {
            Chroma::Mono | Chroma::C444 => (0, 0),
            Chroma::C420 => (1, 1),
            Chroma::C422 => (1, 0),
        }
    }
}

/// Scaling between two levels of quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scaling {
    None,
    Horizontal,
    Both,
}

/// `LdeGlobalConfig`: state from the global configuration block, kept across pictures.
#[derive(Clone, Debug)]
pub(crate) struct Global {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) num_planes: usize,
    pub(crate) dds: bool,
    pub(crate) chroma: Chroma,
    pub(crate) base_depth: u32,
    pub(crate) enhanced_depth: u32,
    pub(crate) temporal_sw_modifier: u8,
    pub(crate) predicted_average: bool,
    pub(crate) temporal_reduced_signalling: bool,
    pub(crate) temporal_enabled: bool,
    /// Kernel magnitudes; taps 0 and 3 are negative.
    pub(crate) kernel: [u16; 4],
    /// Scaling from LOQ1 to LOQ0 (`[0]`) and from the base to LOQ1 (`[1]`).
    pub(crate) scaling: [Scaling; 2],
    pub(crate) user_data: Option<(usize, u32)>,
    pub(crate) chroma_sw_multiplier: u8,
    pub(crate) deblock_corner: u32,
    pub(crate) deblock_side: u32,
    /// Conformance window in luma samples: left, right, top, bottom.
    pub(crate) crop: [u32; 4],
}

impl Global {
    pub(crate) fn num_layers(&self) -> usize {
        if self.dds { 16 } else { 4 }
    }

    /// `ldePlaneDimensionsFromConfig`: size of `plane` at level of quality `loq` (0 = output,
    /// 1, 2 = base).
    pub(crate) fn plane_size(&self, loq: usize, plane: usize) -> (u32, u32) {
        let (mut w, mut h) = (self.width, self.height);
        for s in self.scaling.iter().take(loq) {
            if *s != Scaling::None {
                w = w.div_ceil(2);
                if *s == Scaling::Both {
                    h = h.div_ceil(2);
                }
            }
        }
        if plane > 0 {
            let (sx, sy) = self.chroma.shifts();
            if sx > 0 {
                w = w.div_ceil(2);
            }
            if sy > 0 {
                h = h.div_ceil(2);
            }
        }
        (w, h)
    }
}

/// Sequence-level values seen before the global configuration.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sequence {
    pub(crate) crop: Option<[u32; 4]>,
}

/// One chunk of entropy-coded data: a byte range of the unencapsulated access unit.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Chunk {
    pub(crate) enabled: bool,
    pub(crate) rle_only: bool,
    pub(crate) start: usize,
    pub(crate) size: usize,
}

/// `LdeFrameConfig`: one picture's configuration and chunks.
#[derive(Clone, Debug, Default)]
pub(crate) struct Frame {
    pub(crate) idr: bool,
    pub(crate) set: bool,
    pub(crate) entropy_enabled: bool,
    pub(crate) temporal_refresh: bool,
    pub(crate) temporal_signalling_present: bool,
    pub(crate) step_widths: [i32; 2],
    pub(crate) deblock_enabled: bool,
    pub(crate) dequant_offset: i32,
    pub(crate) dequant_offset_const: bool,
    pub(crate) loq_enabled: [bool; 2],
    /// Residual chunks `[plane][loq][layer]`.
    pub(crate) residual: Vec<[Vec<Chunk>; 2]>,
    pub(crate) temporal: Vec<Chunk>,
}

/// Quantisation matrix and dithering state that persists between pictures.
#[derive(Clone, Debug, Default)]
pub(crate) struct Persistent {
    pub(crate) qm: [[u8; 16]; 2],
    pub(crate) qm_set: bool,
    pub(crate) dither_enabled: bool,
}

const RESOLUTIONS: [(u32, u32); 51] = [
    (0, 0),
    (360, 200),
    (400, 240),
    (480, 320),
    (640, 360),
    (640, 480),
    (768, 480),
    (800, 600),
    (852, 480),
    (854, 480),
    (856, 480),
    (960, 540),
    (960, 640),
    (1024, 576),
    (1024, 600),
    (1024, 768),
    (1152, 864),
    (1280, 720),
    (1280, 800),
    (1280, 1024),
    (1360, 768),
    (1366, 768),
    (1400, 1050),
    (1440, 900),
    (1600, 1200),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2048, 1080),
    (2048, 1152),
    (2048, 1536),
    (2160, 1440),
    (2560, 1440),
    (2560, 1600),
    (2560, 2048),
    (3200, 1800),
    (3200, 2048),
    (3200, 2400),
    (3440, 1440),
    (3840, 1600),
    (3840, 2160),
    (3840, 2400),
    (4096, 2160),
    (4096, 3072),
    (5120, 2880),
    (5120, 3200),
    (5120, 4096),
    (6400, 4096),
    (6400, 4800),
    (7680, 4320),
    (7680, 4800),
];

/// Nearest, linear, cubic and modified cubic kernels (`kKernels`).
const KERNELS: [[u16; 4]; 4] = [[0, 16384, 0, 0], [0, 12288, 4096, 0], [1382, 14285, 3942, 461], [2360, 15855, 4165, 1276]];

/// Largest output picture accepted (samples per plane).
const MAX_SAMPLES: u64 = 1 << 26;

const DEFAULT_QM_DD_1D: [[u8; 4]; 2] = [[0, 2, 0, 0], [0, 3, 0, 32]];
const DEFAULT_QM_DD_2D: [[u8; 4]; 2] = [[32, 3, 0, 32], [0, 3, 0, 32]];
const DEFAULT_QM_DDS_1D: [[u8; 16]; 2] =
    [[13, 26, 19, 32, 52, 1, 78, 9, 13, 26, 19, 32, 150, 91, 91, 19], [0, 0, 0, 2, 52, 1, 78, 9, 26, 72, 0, 3, 150, 91, 91, 19]];
const DEFAULT_QM_DDS_2D: [[u8; 16]; 2] =
    [[13, 26, 19, 32, 52, 1, 78, 9, 26, 72, 0, 3, 150, 91, 91, 19], [0, 0, 0, 2, 52, 1, 78, 9, 26, 72, 0, 3, 150, 91, 91, 19]];

fn default_qm(g: &Global, loq: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    let one_d = g.scaling[0] == Scaling::Horizontal;
    let src: &[u8] = match (g.dds, one_d) {
        (true, true) => DEFAULT_QM_DDS_1D.get(loq).map_or(&[], |v| v.as_slice()),
        (true, false) => DEFAULT_QM_DDS_2D.get(loq).map_or(&[], |v| v.as_slice()),
        (false, true) => DEFAULT_QM_DD_1D.get(loq).map_or(&[], |v| v.as_slice()),
        (false, false) => DEFAULT_QM_DD_2D.get(loq).map_or(&[], |v| v.as_slice()),
    };
    for (o, s) in out.iter_mut().zip(src) {
        *o = *s;
    }
    out
}

fn depth(code: u8) -> u32 {
    8 + 2 * u32::from(code & 3)
}

fn parse_sequence(r: &mut Bytes<'_>, seq: &mut Sequence) -> Result<(), Error> {
    let a = r.u8()?;
    let b = r.u8()?;
    let crop = (b >> 5) & 1 != 0;
    if (a >> 4) == 15 || (a & 15) == 15 {
        r.u8()?;
    }
    seq.crop = if crop {
        let mut c = [0u32; 4];
        for v in &mut c {
            let m = r.multibyte()?;
            *v = u32::from(u16::try_from(m).map_err(|_| Error::Invalid("conformance window too large"))?);
        }
        Some(c)
    } else {
        None
    };
    Ok(())
}

fn parse_global(r: &mut Bytes<'_>, seq: &Sequence) -> Result<Global, Error> {
    let d0 = r.u8()?;
    let plane_mode = d0 >> 7 != 0;
    let res_type = usize::from((d0 >> 1) & 0x3F);
    let dds = d0 & 1 != 0;
    let (mut width, mut height) = match RESOLUTIONS.get(res_type) {
        Some(&wh) if res_type > 0 => wh,
        _ if res_type == 63 => (0, 0),
        _ => return Err(Error::Invalid("resolution type")),
    };
    let d1 = r.u8()?;
    let chroma = match d1 >> 6 {
        0 => Chroma::Mono,
        1 => Chroma::C420,
        2 => Chroma::C422,
        _ => Chroma::C444,
    };
    let base_depth = depth(d1 >> 4);
    let enhanced_depth = depth(d1 >> 2);
    let sw_modifier_signalled = (d1 >> 1) & 1 != 0;
    let predicted_average = d1 & 1 != 0;
    let d2 = r.u8()?;
    let temporal_reduced_signalling = d2 >> 7 != 0;
    let temporal_enabled = (d2 >> 6) & 1 != 0;
    let upsample = (d2 >> 3) & 7;
    if upsample > 4 {
        return Err(Error::Invalid("upsample type"));
    }
    let deblock_signalled = (d2 >> 2) & 1 != 0;
    let scaling_mode = |v: u8| match v & 3 {
        0 => Ok(Scaling::None),
        1 => Ok(Scaling::Horizontal),
        2 => Ok(Scaling::Both),
        _ => Err(Error::Invalid("scaling mode")),
    };
    let scaling_loq1 = scaling_mode(d2)?;
    let d3 = r.u8()?;
    let scaling_loq0 = scaling_mode(d3 >> 6)?;
    if (d3 >> 4) & 3 != 0 {
        return Err(Error::Unsupported("LCEVC tiles"));
    }
    let user_data_mode = (d3 >> 2) & 3;
    let chroma_sw_flag = d3 & 1 != 0;
    if enhanced_depth < base_depth {
        return Err(Error::Unsupported("enhancement depth below base depth"));
    }
    let num_planes = if plane_mode {
        match r.u8()? >> 4 {
            0 => 1,
            1 => 3,
            _ => return Err(Error::Invalid("plane type")),
        }
    } else {
        1
    };
    let temporal_sw_modifier = if sw_modifier_signalled { r.u8()? } else { 48 };
    let kernel =
        if upsample == 4 { [r.u16()?, r.u16()?, r.u16()?, r.u16()?] } else { *KERNELS.get(usize::from(upsample)).ok_or(Error::Invalid("upsample type"))? };
    let (deblock_corner, deblock_side) = if deblock_signalled {
        let d = r.u8()?;
        (16 - u32::from(d >> 4), 16 - u32::from(d & 15))
    } else {
        (16, 16)
    };
    if res_type == 63 {
        width = u32::from(r.u16()?);
        height = u32::from(r.u16()?);
        if width == 0 || height == 0 {
            return Err(Error::Invalid("zero resolution"));
        }
    }
    let chroma_sw_multiplier = if chroma_sw_flag { r.u8()? } else { 64 };
    let user_data = match user_data_mode {
        0 => None,
        m => Some((if dds { 5 } else { 1 }, if m == 1 { 2 } else { 6 })),
    };
    let g = Global {
        width,
        height,
        num_planes,
        dds,
        chroma,
        base_depth,
        enhanced_depth,
        temporal_sw_modifier,
        predicted_average,
        temporal_reduced_signalling,
        temporal_enabled,
        kernel,
        scaling: [scaling_loq0, scaling_loq1],
        user_data,
        chroma_sw_multiplier,
        deblock_corner,
        deblock_side,
        crop: [0; 4],
    };
    // validateResolution
    let with_chroma = g.num_planes > 1 && chroma != Chroma::Mono;
    let t = if dds { 4 } else { 2 };
    let hs = if scaling_loq0 != Scaling::None { 2 } else { 1 };
    let vs = if scaling_loq0 == Scaling::Both { 2 } else { 1 };
    let hc = if with_chroma && chroma != Chroma::C444 { 2 } else { 1 };
    let vc = if with_chroma && chroma == Chroma::C420 { 2 } else { 1 };
    if width % (t * hs * hc) != 0 || height % (t * vs * vc) != 0 {
        return Err(Error::Invalid("resolution is not a whole number of transforms"));
    }
    if u64::from(width) * u64::from(height) > MAX_SAMPLES {
        return Err(Error::Unsupported("picture too large"));
    }
    let mut g = g;
    if let Some(c) = seq.crop {
        let (sx, sy) = match chroma {
            Chroma::C420 => (1, 1),
            Chroma::C422 => (1, 0),
            _ => (0, 0),
        };
        g.crop = [c[0] << sx, c[1] << sx, c[2] << sy, c[3] << sy];
        if g.crop[0] + g.crop[1] >= width || g.crop[2] + g.crop[3] >= height {
            return Err(Error::Invalid("conformance window"));
        }
    }
    Ok(g)
}

fn parse_picture(r: &mut Bytes<'_>, g: &Global, f: &mut Frame, p: &mut Persistent) -> Result<(), Error> {
    let d = r.u8()?;
    f.entropy_enabled = d & 0x80 == 0;
    let mut qm_mode = 0u8;
    let mut sw_loq1_enabled = false;
    let mut dequant_offset_enabled = false;
    let mut dither_control_present = false;
    if (d >> 2) & 1 != 0 {
        return Err(Error::Unsupported("LCEVC field pictures"));
    }
    f.temporal_refresh = (d >> 1) & 1 != 0;
    if f.entropy_enabled {
        qm_mode = (d >> 4) & 7;
        dequant_offset_enabled = (d >> 3) & 1 != 0;
        f.temporal_signalling_present = g.temporal_enabled && !f.temporal_refresh;
        sw_loq1_enabled = d & 1 != 0;
        let v = r.u16()?;
        f.step_widths[0] = i32::from(v >> 1).clamp(1, 32767);
        dither_control_present = true;
        p.dither_enabled = v & 1 != 0;
    } else {
        f.temporal_signalling_present = d & 1 != 0;
    }
    // parseBlockPictureConfigMisc (bitstream version 2)
    if sw_loq1_enabled {
        let v = r.u16()?;
        f.step_widths[1] = i32::from(v >> 1).clamp(1, 32767);
        f.deblock_enabled = v & 1 != 0;
    } else {
        f.step_widths[1] = 32767;
    }
    let layers = g.num_layers();
    let read_qm = |r: &mut Bytes<'_>, row: &mut [u8; 16]| -> Result<(), Error> {
        for v in row.iter_mut().take(layers) {
            *v = r.u8()?;
        }
        Ok(())
    };
    // LOQ0
    match qm_mode {
        0 | 4 => {
            if f.idr || !p.qm_set {
                p.qm[0] = default_qm(g, 0);
            }
        }
        1 => p.qm[0] = default_qm(g, 0),
        2 | 3 | 5 => read_qm(r, &mut p.qm[0])?,
        _ => return Err(Error::Invalid("quant matrix mode")),
    }
    // LOQ1
    match qm_mode {
        0 | 3 => {
            if f.idr || !p.qm_set {
                p.qm[1] = default_qm(g, 1);
            }
        }
        1 => p.qm[1] = default_qm(g, 1),
        4 | 5 => read_qm(r, &mut p.qm[1])?,
        _ => p.qm[1] = p.qm[0],
    }
    p.qm_set = true;
    if dequant_offset_enabled {
        let v = r.u8()?;
        f.dequant_offset_const = v >> 7 != 0;
        f.dequant_offset = i32::from(v & 0x7F);
    } else {
        f.dequant_offset = -1;
    }
    if !dither_control_present && f.idr {
        p.dither_enabled = false;
    }
    if p.dither_enabled {
        return Err(Error::Unsupported("LCEVC dithering"));
    }
    f.set = true;
    Ok(())
}

fn parse_chunk(r: &mut Bytes<'_>, c: &mut Chunk, loq_enabled: &mut bool) -> Result<(), Error> {
    c.size = 0;
    if !c.enabled {
        return Ok(());
    }
    let size = r.multibyte()?;
    let size = usize::try_from(size).ok().filter(|&s| s > 0 && s <= i32::MAX as usize).ok_or(Error::Invalid("chunk size"))?;
    *loq_enabled = true;
    c.start = r.pos();
    c.size = size;
    let data = r.skip(size)?;
    if c.rle_only && data.last().is_some_and(|b| b & 0x80 != 0) {
        return Err(Error::Invalid("unterminated RLE chunk"));
    }
    Ok(())
}

fn parse_encoded_data(r: &mut Bytes<'_>, g: &Global, f: &mut Frame) -> Result<(), Error> {
    if !f.set {
        return Err(Error::Invalid("encoded data before picture configuration"));
    }
    let layers = g.num_layers();
    f.residual = (0..g.num_planes).map(|_| [vec![Chunk::default(); layers], vec![Chunk::default(); layers]]).collect();
    f.temporal = vec![Chunk::default(); g.num_planes];
    let mut bits = Bits::new(r.rest());
    for plane in 0..g.num_planes {
        if f.entropy_enabled {
            for loq in [1usize, 0] {
                let chunks = f.residual.get_mut(plane).and_then(|p| p.get_mut(loq)).ok_or(Error::Invalid("chunks"))?;
                for c in chunks.iter_mut() {
                    c.enabled = bits.read(1)? != 0;
                    c.rle_only = bits.read(1)? != 0;
                }
            }
        }
        if f.temporal_signalling_present {
            let c = f.temporal.get_mut(plane).ok_or(Error::Invalid("chunks"))?;
            c.enabled = bits.read(1)? != 0;
            c.rle_only = bits.read(1)? != 0;
        }
    }
    r.skip(bits.consumed_bytes())?;
    for plane in 0..g.num_planes {
        if f.entropy_enabled {
            for loq in [1usize, 0] {
                let mut enabled = f.loq_enabled.get(loq).copied().unwrap_or(false);
                let chunks = f.residual.get_mut(plane).and_then(|p| p.get_mut(loq)).ok_or(Error::Invalid("chunks"))?;
                for c in chunks.iter_mut() {
                    parse_chunk(r, c, &mut enabled)?;
                }
                if let Some(e) = f.loq_enabled.get_mut(loq) {
                    *e = enabled;
                }
            }
        }
        if f.temporal_signalling_present {
            let c = f.temporal.get_mut(plane).ok_or(Error::Invalid("chunks"))?;
            parse_chunk(r, c, &mut f.loq_enabled[0])?;
        }
    }
    Ok(())
}

/// Parses all blocks of one unencapsulated access unit. A new global configuration replaces
/// `global`.
pub(crate) fn parse(data: &[u8], idr: bool, seq: &mut Sequence, global: &mut Option<Global>, persistent: &mut Persistent) -> Result<Frame, Error> {
    let mut frame = Frame { idr, ..Frame::default() };
    let mut r = Bytes::new(data);
    while r.remaining() > 0 {
        let head = r.u8()?;
        let kind = head & 0x1F;
        let size = match head >> 5 {
            7 => usize::try_from(r.multibyte()?).map_err(|_| Error::Invalid("block size"))?,
            6 => return Err(Error::Invalid("reserved block size")),
            s => usize::from(s),
        };
        if size > r.remaining() {
            return Err(Error::Invalid("block size exceeds access unit"));
        }
        let start = r.pos();
        match kind {
            0 => parse_sequence(&mut r, seq)?,
            1 => {
                *global = None;
                *global = Some(parse_global(&mut r, seq)?);
            }
            2 => {
                let g = global.as_ref().ok_or(Error::Invalid("picture configuration before global configuration"))?;
                parse_picture(&mut r, g, &mut frame, persistent)?;
            }
            3 => {
                let g = global.as_ref().ok_or(Error::Invalid("encoded data before global configuration"))?;
                parse_encoded_data(&mut r, g, &mut frame)?;
            }
            4 => return Err(Error::Unsupported("LCEVC tiles")),
            _ => {
                r.skip(size)?;
            }
        }
        if r.pos() - start != size {
            return Err(Error::Invalid("block size mismatch"));
        }
    }
    Ok(frame)
}
