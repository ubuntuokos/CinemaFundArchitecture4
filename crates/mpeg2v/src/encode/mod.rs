//! MPEG-2 video encoder (ITU-T H.262 | ISO/IEC 13818-2).
//!
//! - Main Profile (4:2:0) and 4:2:2 Profile at the lowest level that holds the picture size,
//!   rate and bitrate (Main, High-1440, High; 4:2:2 Main and High).
//! - Progressive frame pictures, I and P pictures (low delay: decode order is display order),
//!   a closed GOP with a sequence header at every I picture.
//! - Motion search at whole samples (predictors plus diamond search), half-sample refinement,
//!   ±32 samples (f_code 3); intra / inter / skip decisions per macroblock.
//! - Intra VLC table one, the non-linear quantiser scale, intra DC precision 8-11 bits, the
//!   default quantiser matrices.
//! - Constant quantiser, or an average bitrate (picture targets after Test Model 5, an even
//!   quantiser per picture, optional adaptive quantisation by spatial activity).
//!
//! Every coded picture is decoded by this crate's own decoder to form the next reference, so
//! encoder and decoder reconstructions agree by construction.
//!
//! ```no_run
//! use cfa4_mpeg2v::encode::{Config, Encoder, Input, Rate};
//! let mut cfg = Config::new(1920, 1080, 25, 1);
//! cfg.rate = Rate::Bitrate { bits_per_second: 25_000_000 };
//! let mut enc = Encoder::new(cfg)?;
//! # let (y, cb, cr) = (vec![0u8; 1920 * 1080], vec![0u8; 960 * 540], vec![0u8; 960 * 540]);
//! let packet = enc.encode(&Input { y: &y, cb: &cb, cr: &cr, y_stride: 1920, c_stride: 960 })?;
//! let mut es = packet.data;
//! es.extend_from_slice(&enc.finish());
//! # Ok::<(), cfa4_mpeg2v::Error>(())
//! ```

mod analyse;
mod rate;
mod write;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::headers::{DEFAULT_INTRA, SCAN};
use crate::mc::Frame;
use crate::slice::NON_LINEAR_Q;
use crate::vlc::{MB_FWD, MB_INTRA, MB_PATTERN, MB_QUANT, MBA_ESCAPE};
use crate::{ChromaFormat, Decoder, Error, PictureType, Result};

use analyse::{Ctx, F_CODE, Plan, analyse_row};
use write::{BitWriter, Code, EncTables, MAX_TABLE_LEVEL, tables};

/// How the quantiser is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    /// A constant quantiser_scale_code 1..=31 on the non-linear scale (Table 7-6: 1-8 step 1,
    /// then up to 112). Lower is better quality.
    Quantizer(u8),
    /// Average bitrate (VBR, vbv_delay 0xFFFF).
    Bitrate { bits_per_second: u64 },
}

/// aspect_ratio_information (Table 6-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AspectRatio {
    /// Square samples.
    Square,
    /// 4:3 display.
    Dar4x3,
    /// 16:9 display.
    Dar16x9,
    /// 2.21:1 display.
    Dar221x100,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    /// 4:2:0 (Main Profile) or 4:2:2 (4:2:2 Profile).
    pub chroma: ChromaFormat,
    /// One of the MPEG-2 frame rates: 24000/1001, 24, 25, 30000/1001, 30, 50, 60000/1001, 60.
    pub fps_num: u32,
    pub fps_den: u32,
    pub rate: Rate,
    /// Pictures per GOP (an I picture every `gop` pictures).
    pub gop: u32,
    /// Intra DC precision in bits: 8-10 (Main Profile), 8-11 (4:2:2 Profile).
    pub intra_dc_precision: u8,
    pub aspect: AspectRatio,
    /// (colour_primaries, transfer_characteristics, matrix_coefficients) for a sequence display
    /// extension (ITU-T H.273 code points).
    pub colour: Option<(u8, u8, u8)>,
    /// Bitrate mode: coarser quantisers for busy macroblocks, finer for flat ones (TM5 step 3).
    /// Usually looks better but measures a lower PSNR.
    pub adaptive_quantization: bool,
    /// Analyse macroblock rows in parallel (with the `threads` feature).
    pub threads: bool,
}

impl Config {
    /// 4:2:0, quantiser 6, a GOP of 15, 9-bit intra DC, square samples.
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Config {
        Config {
            width,
            height,
            chroma: ChromaFormat::Yuv420,
            fps_num,
            fps_den,
            rate: Rate::Quantizer(6),
            gop: 15,
            intra_dc_precision: 9,
            aspect: AspectRatio::Square,
            colour: None,
            adaptive_quantization: false,
            threads: true,
        }
    }
}

/// One picture: planar 8-bit samples (chroma `width.div_ceil(2)` wide, `height.div_ceil(2)`
/// (4:2:0) or `height` (4:2:2) high).
pub struct Input<'a> {
    pub y: &'a [u8],
    pub cb: &'a [u8],
    pub cr: &'a [u8],
    pub y_stride: usize,
    pub c_stride: usize,
}

/// One coded picture (with the sequence and GOP headers before I pictures).
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    pub keyframe: bool,
    pub picture_type: PictureType,
    /// Picture number (decode order is display order).
    pub index: u64,
}

/// Level limits (Tables 8-8 to 8-13): profile_and_level_indication, maximum size, luma
/// samples/s, bitrate and VBV buffer size (bits).
struct Level {
    pli: u8,
    w: u32,
    h: u32,
    samples: u64,
    rate: u64,
    vbv: u64,
}

const LEVELS_420: [Level; 3] = [
    Level { pli: 0x48, w: 720, h: 576, samples: 10_368_000, rate: 15_000_000, vbv: 1_835_008 },
    Level { pli: 0x46, w: 1440, h: 1152, samples: 47_001_600, rate: 60_000_000, vbv: 7_340_032 },
    Level { pli: 0x44, w: 1920, h: 1152, samples: 62_668_800, rate: 80_000_000, vbv: 9_781_248 },
];
const LEVELS_422: [Level; 2] = [
    Level { pli: 0x85, w: 720, h: 608, samples: 11_059_200, rate: 50_000_000, vbv: 9_437_184 },
    Level { pli: 0x82, w: 1920, h: 1088, samples: 62_668_800, rate: 300_000_000, vbv: 47_185_920 },
];

/// frame_rate_code for an exact MPEG-2 frame rate.
fn frame_rate_code(num: u32, den: u32) -> Option<u8> {
    if num == 0 || den == 0 {
        return None;
    }
    let g = crate::headers::gcd(num, den);
    let want = (num / g, den / g);
    (1..=8).find(|&c| crate::headers::frame_rate(c, 0, 0) == Some(want))
}

/// The quantiser_scale_code whose non-linear scale is nearest to `qs`.
fn nearest_code(qs: f64) -> u8 {
    let mut best = 1u8;
    for c in 2..32u8 {
        if (NON_LINEAR_Q[c as usize] as f64 - qs).abs() < (NON_LINEAR_Q[best as usize] as f64 - qs).abs() {
            best = c;
        }
    }
    best
}

pub struct Encoder {
    cfg: Config,
    frc: u8,
    pli: u8,
    /// bit_rate in units of 400 bit/s and vbv_buffer_size in units of 16384 bits.
    bit_rate_units: u32,
    vbv_units: u32,
    mb_width: usize,
    mb_height: usize,
    /// Chroma macroblock height (8 or 16).
    cmbh: usize,
    nblocks: usize,
    frames: u64,
    /// The reconstructed previous picture.
    reference: Option<Arc<Frame>>,
    prev_mvs: Vec<[i32; 2]>,
    dec: Decoder,
    rc: Option<rate::Rc>,
    /// Average quantiser_scale of the last picture (motion search weight).
    last_scale: f64,
    /// intra_vlc_format of the next picture, and what the intra blocks of the current one cost
    /// with table zero and table one.
    intra_vlc: bool,
    vlc_cost: [u64; 2],
}

impl Encoder {
    pub fn new(cfg: Config) -> Result<Encoder> {
        let (w, h) = (cfg.width, cfg.height);
        if w == 0 || h == 0 || w > 16383 || h > 16383 || w % 4096 == 0 || h % 4096 == 0 {
            return Err(Error::Unsupported(format!("MPEG-2 picture size {w}x{h}")));
        }
        let frc = frame_rate_code(cfg.fps_num, cfg.fps_den).ok_or_else(|| {
            Error::Unsupported(format!("MPEG-2 frame rate {}/{} (24000/1001, 24, 25, 30000/1001, 30, 50, 60000/1001 or 60)", cfg.fps_num, cfg.fps_den))
        })?;
        let (levels, max_dc): (&[Level], u8) = match cfg.chroma {
            ChromaFormat::Yuv420 => (&LEVELS_420, 10),
            ChromaFormat::Yuv422 => (&LEVELS_422, 11),
            ChromaFormat::Yuv444 => return Err(Error::Unsupported("MPEG-2 encoding of 4:4:4".into())),
        };
        if !(8..=max_dc).contains(&cfg.intra_dc_precision) {
            return Err(Error::Unsupported(format!("intra DC precision {} bits (8..={max_dc})", cfg.intra_dc_precision)));
        }
        match cfg.rate {
            Rate::Quantizer(q) if !(1..=31).contains(&q) => return Err(Error::Unsupported(format!("quantiser_scale_code {q} (1..=31)"))),
            Rate::Bitrate { bits_per_second } if !(16_000..=1_000_000_000).contains(&bits_per_second) => {
                return Err(Error::Unsupported(format!("bitrate {bits_per_second} bit/s")));
            }
            _ => {}
        }
        if cfg.gop == 0 {
            return Err(Error::Unsupported("a GOP of 0 pictures".into()));
        }
        let fps = cfg.fps_num as f64 / cfg.fps_den as f64;
        let bitrate = match cfg.rate {
            Rate::Bitrate { bits_per_second } => Some(bits_per_second),
            Rate::Quantizer(_) => None,
        };
        let samples = (w as u64 * h as u64 * cfg.fps_num as u64).div_ceil(cfg.fps_den as u64);
        let fitting = levels.iter().find(|l| w <= l.w && h <= l.h && samples <= l.samples && bitrate.is_none_or(|b| b <= l.rate));
        // beyond the highest level: still decodable, signalled as the highest level
        let level = fitting.or(levels.last()).ok_or_else(|| Error::Unsupported("no level".into()))?;
        let bit_rate_units = bitrate.unwrap_or(level.rate).div_ceil(400).clamp(1, (1 << 30) - 1) as u32;
        let vbv_units = (level.vbv / 16384).clamp(1, (1 << 18) - 1) as u32;
        let mb_width = (w as usize).div_ceil(16);
        let mb_height = (h as usize).div_ceil(16);
        let cmbh = if cfg.chroma == ChromaFormat::Yuv420 { 8 } else { 16 };
        let mut dec = Decoder::new();
        dec.set_threads(cfg.threads);
        Ok(Encoder {
            frc,
            pli: level.pli,
            bit_rate_units,
            vbv_units,
            mb_width,
            mb_height,
            cmbh,
            nblocks: if cmbh == 8 { 6 } else { 8 },
            frames: 0,
            reference: None,
            prev_mvs: vec![[0, 0]; mb_width * mb_height],
            dec,
            rc: bitrate.map(|b| rate::Rc::new(b, fps, cfg.adaptive_quantization)),
            last_scale: 8.0,
            intra_vlc: true,
            vlc_cost: [0; 2],
            cfg,
        })
    }

    /// profile_and_level_indication of the stream.
    pub fn profile_and_level(&self) -> u8 {
        self.pli
    }

    /// The bitrate the sequence header signals (bit/s).
    pub fn signalled_bit_rate(&self) -> u64 {
        self.bit_rate_units as u64 * 400
    }

    /// Encode the next picture.
    pub fn encode(&mut self, input: &Input) -> Result<Packet> {
        let cur = self.padded(input)?;
        let index = self.frames;
        let gop_pos = (index % self.cfg.gop as u64) as u32;
        let reference = if gop_pos == 0 { None } else { self.reference.clone() };
        let ptype = if reference.is_some() { PictureType::P } else { PictureType::I };
        let mut w = BitWriter::default();
        if ptype == PictureType::I {
            self.sequence_headers(&mut w);
            self.gop_header(&mut w, index);
            if let Some(rc) = self.rc.as_mut() {
                rc.start_gop(self.cfg.gop);
            }
        }
        self.picture_header(&mut w, gop_pos, ptype);
        // analysis
        let lambda = (self.last_scale * 0.6).round().clamp(1.0, 64.0) as u32;
        let ctx = Ctx {
            cur: &cur,
            reference: reference.as_deref(),
            cmbh: self.cmbh,
            nblocks: self.nblocks,
            mb_width: self.mb_width,
            prev_mvs: &self.prev_mvs,
            lambda,
        };
        let plans = self.analyse(&ctx);
        let avg_act = plans.iter().flatten().map(|p| p.act as f64).sum::<f64>() / (self.mb_width * self.mb_height).max(1) as f64;
        // slices
        let start = w.bits();
        if let Some(rc) = self.rc.as_mut() {
            let weights: Vec<f64> = plans.iter().flatten().map(|p| p.weight as f64).collect();
            rc.start_picture(ptype == PictureType::I, &weights, avg_act);
        }
        // the first picture of each type: a trial run measures its complexity
        if self.rc.as_ref().is_some_and(|rc| rc.needs_trial()) {
            let mut trial = BitWriter::default();
            for (row, plans) in plans.iter().enumerate() {
                self.slice(&mut trial, row, plans, ptype, 0);
            }
            if let Some(rc) = self.rc.as_mut() {
                rc.calibrate(trial.bits());
            }
        }
        let mut scale_sum = 0f64;
        self.vlc_cost = [0; 2];
        for (row, plans) in plans.iter().enumerate() {
            scale_sum += self.slice(&mut w, row, plans, ptype, start);
        }
        if let Some(rc) = self.rc.as_mut() {
            rc.end_picture(w.bits() - start);
            self.last_scale = rc.average_scale();
        } else {
            self.last_scale = scale_sum / (self.mb_width * self.mb_height).max(1) as f64;
        }
        // the intra VLC table that would have been cheaper serves the next picture
        if self.vlc_cost[0] != self.vlc_cost[1] {
            self.intra_vlc = self.vlc_cost[1] < self.vlc_cost[0];
        }
        w.align();
        let data = w.out;
        // reconstruction: decode the picture
        let errors = self.dec.errors();
        self.dec.decode(&data, index as i64)?;
        if self.dec.errors() != errors {
            return Err(Error::Invalid(format!("encoder produced an undecodable picture: {}", self.dec.last_error().unwrap_or("?"))));
        }
        self.reference = Some(self.dec.last_anchor().ok_or_else(|| Error::Invalid("no reconstructed picture".into()))?);
        self.prev_mvs = plans.iter().flatten().map(|p| if p.intra { [0, 0] } else { p.mv }).collect();
        self.frames += 1;
        Ok(Packet { data, keyframe: ptype == PictureType::I, picture_type: ptype, index })
    }

    /// The sequence_end_code that closes the stream.
    pub fn finish(&mut self) -> Vec<u8> {
        vec![0, 0, 1, crate::headers::SEQUENCE_END]
    }

    fn analyse(&self, ctx: &Ctx) -> Vec<Vec<Plan>> {
        #[cfg(feature = "threads")]
        if self.cfg.threads {
            use rayon::prelude::*;
            return (0..self.mb_height).into_par_iter().map(|r| analyse_row(ctx, r)).collect();
        }
        (0..self.mb_height).map(|r| analyse_row(ctx, r)).collect()
    }

    /// The input copied into a macroblock-aligned frame, edges replicated.
    fn padded(&self, input: &Input) -> Result<Frame> {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let (cw, ch) = (w.div_ceil(2), if self.cmbh == 8 { h.div_ceil(2) } else { h });
        let (pw, ph) = (self.mb_width * 16, self.mb_height * 16);
        let mut f = Frame::new(pw, ph, pw / 2, self.mb_height * self.cmbh);
        let planes: [(&[u8], usize, usize, usize); 3] =
            [(input.y, input.y_stride, w, h), (input.cb, input.c_stride, cw, ch), (input.cr, input.c_stride, cw, ch)];
        for (k, (src, stride, sw, sh)) in planes.into_iter().enumerate() {
            if stride < sw || src.len() < stride * (sh - 1) + sw {
                return Err(Error::Invalid(format!("plane {k}: {} bytes, stride {stride} for {sw}x{sh}", src.len())));
            }
            let (dw, dh) = if k == 0 { (f.w, f.h) } else { (f.cw, f.ch) };
            let dst = &mut f.planes[k];
            for y in 0..dh {
                let s = &src[y.min(sh - 1) * stride..][..sw];
                let d = &mut dst[y * dw..(y + 1) * dw];
                d[..sw].copy_from_slice(s);
                let last = s[sw - 1];
                d[sw..].fill(last);
            }
        }
        Ok(f)
    }

    fn sequence_headers(&self, w: &mut BitWriter) {
        let c = &self.cfg;
        w.start(crate::headers::SEQUENCE_HEADER);
        w.put(c.width & 0xFFF, 12);
        w.put(c.height & 0xFFF, 12);
        w.put(
            match c.aspect {
                AspectRatio::Square => 1,
                AspectRatio::Dar4x3 => 2,
                AspectRatio::Dar16x9 => 3,
                AspectRatio::Dar221x100 => 4,
            },
            4,
        );
        w.put(self.frc as u32, 4);
        w.put(self.bit_rate_units & 0x3FFFF, 18);
        w.put(1, 1); // marker_bit
        w.put(self.vbv_units & 0x3FF, 10);
        w.put(0, 1); // constrained_parameters_flag
        w.put(0, 1); // load_intra_quantiser_matrix
        w.put(0, 1); // load_non_intra_quantiser_matrix
        // sequence extension
        w.start(crate::headers::EXTENSION);
        w.put(1, 4);
        w.put(self.pli as u32, 8);
        w.put(1, 1); // progressive_sequence
        w.put(if self.cmbh == 8 { 1 } else { 2 }, 2);
        w.put(c.width >> 12, 2);
        w.put(c.height >> 12, 2);
        w.put(self.bit_rate_units >> 18, 12);
        w.put(1, 1); // marker_bit
        w.put(self.vbv_units >> 10, 8);
        w.put(1, 1); // low_delay
        w.put(0, 2); // frame_rate_extension_n
        w.put(0, 5); // frame_rate_extension_d
        if let Some((p, t, m)) = c.colour {
            w.start(crate::headers::EXTENSION);
            w.put(2, 4);
            w.put(5, 3); // video_format: unspecified
            w.put(1, 1); // colour_description
            w.put(p as u32, 8);
            w.put(t as u32, 8);
            w.put(m as u32, 8);
            w.put(c.width, 14);
            w.put(1, 1); // marker_bit
            w.put(c.height, 14);
        }
    }

    fn gop_header(&self, w: &mut BitWriter, index: u64) {
        let fps = (self.cfg.fps_num as u64).div_ceil(self.cfg.fps_den.max(1) as u64).max(1);
        let secs = index / fps;
        w.start(crate::headers::GROUP_START);
        w.put(0, 1); // drop_frame_flag
        w.put((secs / 3600 % 24) as u32, 5);
        w.put((secs / 60 % 60) as u32, 6);
        w.put(1, 1); // marker_bit
        w.put((secs % 60) as u32, 6);
        w.put((index % fps) as u32, 6);
        w.put(1, 1); // closed_gop
        w.put(0, 1); // broken_link
    }

    fn picture_header(&self, w: &mut BitWriter, tref: u32, ptype: PictureType) {
        w.start(crate::headers::PICTURE_START);
        w.put(tref & 1023, 10);
        w.put(if ptype == PictureType::I { 1 } else { 2 }, 3);
        w.put(0xFFFF, 16); // vbv_delay: variable bitrate
        if ptype == PictureType::P {
            w.put(0, 1); // full_pel_forward_vector
            w.put(7, 3); // forward_f_code (MPEG-2: in the extension)
        }
        w.put(0, 1); // extra_bit_picture
        // picture coding extension
        w.start(crate::headers::EXTENSION);
        w.put(8, 4);
        let f = if ptype == PictureType::P { F_CODE } else { 15 };
        for v in [f, f, 15, 15] {
            w.put(v, 4);
        }
        w.put((self.cfg.intra_dc_precision - 8) as u32, 2);
        w.put(3, 2); // picture_structure: frame
        w.put(0, 1); // top_field_first
        w.put(1, 1); // frame_pred_frame_dct
        w.put(0, 1); // concealment_motion_vectors
        w.put(1, 1); // q_scale_type: non-linear
        w.put(self.intra_vlc as u32, 1); // intra_vlc_format
        w.put(0, 1); // alternate_scan
        w.put(0, 1); // repeat_first_field
        w.put((self.cmbh == 8) as u32, 1); // chroma_420_type = progressive_frame for 4:2:0
        w.put(1, 1); // progressive_frame
        w.put(0, 1); // composite_display_flag
    }

    /// The quantiser_scale_code for macroblock `j` of the picture.
    fn mb_code(&mut self, j: usize, bits: u64, act: f32) -> u8 {
        match (self.rc.as_mut(), self.cfg.rate) {
            (Some(rc), _) => nearest_code(rc.scale(j, bits, act)),
            (None, Rate::Quantizer(q)) => q.clamp(1, 31),
            (None, _) => 8,
        }
    }

    /// Code one slice (a macroblock row). Returns the sum of the quantiser scales used.
    fn slice(&mut self, w: &mut BitWriter, row: usize, plans: &[Plan], ptype: PictureType, pic_start: u64) -> f64 {
        let t = tables();
        let mbw = self.mb_width;
        let dc_reset = 1i32 << (self.cfg.intra_dc_precision - 1);
        let mut code = self.mb_code(row * mbw, w.bits() - pic_start, plans.first().map_or(1.0, |p| p.act));
        w.start(1 + (row & 127) as u8);
        if self.cfg.height > 2800 {
            w.put((row >> 7) as u32, 3); // slice_vertical_position_extension
        }
        w.put(code as u32, 5);
        w.put(0, 1); // extra_bit_slice
        let mut dc = [dc_reset; 3];
        let mut pmv = [0i32; 2];
        let mut skipped = 0usize;
        let mut scale_sum = 0f64;
        let mut levels = [[0i32; 64]; 8];
        for (mbx, plan) in plans.iter().enumerate() {
            let want = if mbx == 0 { code } else { self.mb_code(row * mbw + mbx, w.bits() - pic_start, plan.act) };
            let qs = NON_LINEAR_Q[want as usize] as i32;
            if plan.intra {
                let quant = want != code;
                code = want;
                for (b, lv) in plan.coef.iter().zip(levels.iter_mut()).take(self.nblocks) {
                    quantize_intra(b, qs, lv);
                }
                address_increment(w, t, skipped + 1);
                skipped = 0;
                let flags = MB_INTRA | if quant { MB_QUANT } else { 0 };
                w.code(if ptype == PictureType::I { t.mbtype_i[flags as usize] } else { t.mbtype_p[flags as usize] });
                if quant {
                    w.put(code as u32, 5);
                }
                let m = 8 >> (self.cfg.intra_dc_precision - 8);
                let max = (1 << self.cfg.intra_dc_precision) - 1;
                for (i, (b, lv)) in plan.coef.iter().zip(&levels).take(self.nblocks).enumerate() {
                    let cc = if i < 4 { 0 } else { 1 + (i & 1) };
                    let v = ((b[0] + m / 2) / m).clamp(0, max);
                    write_dc(w, t, cc, v - dc[cc]);
                    dc[cc] = v;
                    write_coefficients(w, t, lv, true, self.intra_vlc, &mut self.vlc_cost);
                }
                pmv = [0, 0];
                scale_sum += qs as f64;
                continue;
            }
            let mut cbp = 0u32;
            for (i, (b, lv)) in plan.coef.iter().zip(levels.iter_mut()).take(self.nblocks).enumerate() {
                if quantize_inter(b, qs, lv) {
                    cbp |= 1 << (self.nblocks - 1 - i);
                }
            }
            dc = [dc_reset; 3];
            let zero = plan.mv == [0, 0];
            if cbp == 0 && zero && mbx > 0 && mbx + 1 < mbw {
                skipped += 1;
                pmv = [0, 0];
                scale_sum += NON_LINEAR_Q[code as usize] as f64;
                continue;
            }
            address_increment(w, t, skipped + 1);
            skipped = 0;
            let quant = cbp != 0 && want != code;
            if cbp != 0 {
                code = want;
            }
            scale_sum += NON_LINEAR_Q[code as usize] as f64;
            let mut flags = if cbp == 0 {
                MB_FWD
            } else if zero {
                MB_PATTERN
            } else {
                MB_FWD | MB_PATTERN
            };
            if quant {
                flags |= MB_QUANT;
            }
            w.code(t.mbtype_p[flags as usize]);
            if quant {
                w.put(code as u32, 5);
            }
            if flags & MB_FWD != 0 {
                for c in 0..2 {
                    write_motion(w, t, plan.mv[c] - pmv[c]);
                }
                pmv = plan.mv;
            } else {
                pmv = [0, 0];
            }
            if cbp != 0 {
                if self.nblocks == 8 {
                    w.code(t.cbp[(cbp >> 2) as usize]);
                    w.put(cbp & 3, 2);
                } else {
                    w.code(t.cbp[cbp as usize]);
                }
                for (i, lv) in levels.iter().take(self.nblocks).enumerate() {
                    if cbp & (1 << (self.nblocks - 1 - i)) != 0 {
                        write_coefficients(w, t, lv, false, false, &mut self.vlc_cost);
                    }
                }
            }
        }
        scale_sum
    }
}

fn address_increment(w: &mut BitWriter, t: &EncTables, mut n: usize) {
    while n > 33 {
        w.code(t.mba[MBA_ESCAPE as usize]);
        n -= 33;
    }
    w.code(t.mba[n]);
}

/// Intra AC levels (raster order; the DC is coded separately): nearest level, rounding offset
/// 3/8 towards zero.
fn quantize_intra(coef: &[i32; 64], qs: i32, out: &mut [i32; 64]) {
    out[0] = 0;
    for k in 1..64 {
        let d = DEFAULT_INTRA[k] as i32 * qs;
        let a = coef[k].abs();
        let l = ((128 * a + 3 * d) / (8 * d)).min(2047);
        out[k] = if coef[k] < 0 { -l } else { l };
    }
}

/// Non-intra levels with the flat matrix: reconstruction is (2·level + 1)·qs / 2, so
/// `floor(|F| / qs)` puts it in the middle of each step (and a dead zone of one step).
/// Returns whether any level is non-zero.
fn quantize_inter(coef: &[i32; 64], qs: i32, out: &mut [i32; 64]) -> bool {
    let mut any = false;
    for k in 0..64 {
        let a = coef[k].abs();
        let l = (a / qs).min(2047);
        out[k] = if coef[k] < 0 { -l } else { l };
        any |= l != 0;
    }
    any
}

fn write_dc(w: &mut BitWriter, t: &EncTables, cc: usize, diff: i32) {
    let size = 32 - diff.unsigned_abs().leading_zeros();
    let table = if cc == 0 { &t.dc_luma } else { &t.dc_chroma };
    w.code(table.get(size as usize).copied().unwrap_or_default());
    if size > 0 {
        let v = if diff > 0 { diff } else { diff + (1 << size) - 1 };
        w.put(v as u32, size);
    }
}

/// The run/level pairs of a block in zigzag order (from the first AC coefficient for intra
/// blocks), then end_of_block. Intra blocks use table one when `one`, and add what they would
/// cost with table zero and table one to `cost`.
fn write_coefficients(w: &mut BitWriter, t: &EncTables, levels: &[i32; 64], intra: bool, one: bool, cost: &mut [u64; 2]) {
    let tab = &t.dct[one as usize];
    let code_of = |tab: &[[Code; MAX_TABLE_LEVEL + 1]], run: usize, a: usize| tab.get(run).and_then(|r| r.get(a)).copied().filter(|c| c.1 > 0);
    let mut run = 0usize;
    let mut first = !intra;
    for &pos in &SCAN[0][intra as usize..] {
        let l = levels[pos as usize];
        if l == 0 {
            run += 1;
            continue;
        }
        let sign = (l < 0) as u32;
        let a = l.unsigned_abs() as usize;
        if intra {
            for (k, c) in cost.iter_mut().enumerate() {
                *c += code_of(&t.dct[k], run, a).map_or(24, |c| c.1 as u64 + 1);
            }
        }
        if first && run == 0 && a == 1 {
            w.put(0b10 | sign, 2);
        } else {
            match code_of(tab, run, a) {
                Some(c) => {
                    w.code(c);
                    w.put(sign, 1);
                }
                None => {
                    w.put(0b000001, 6);
                    w.put(run as u32, 6);
                    w.put((l & 0xFFF) as u32, 12);
                }
            }
        }
        first = false;
        run = 0;
    }
    if intra {
        cost[0] += 2;
        cost[1] += 4;
    }
    w.code(if one { Code(0b0110, 4) } else { Code(0b10, 2) });
}

/// A motion vector difference (half samples), brought into [-16f, 16f - 1] (§7.6.3.1).
fn write_motion(w: &mut BitWriter, t: &EncTables, mut d: i32) {
    let fc = F_CODE;
    let f = 1i32 << (fc - 1);
    if d < -16 * f {
        d += 32 * f;
    } else if d > 16 * f - 1 {
        d -= 32 * f;
    }
    if d == 0 {
        w.code(t.motion[16]);
        return;
    }
    let a = d.abs() - 1;
    let code = (a >> (fc - 1)) + 1;
    w.code(t.motion.get((16 + code * d.signum()) as usize).copied().unwrap_or_default());
    w.put((a & (f - 1)) as u32, fc - 1);
}
