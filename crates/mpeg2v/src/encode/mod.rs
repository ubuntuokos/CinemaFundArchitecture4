//! MPEG-2 video encoder (ITU-T H.262 | ISO/IEC 13818-2).
//!
//! - Main Profile (4:2:0) and 4:2:2 Profile at the lowest level that holds the picture size,
//!   rate and bitrate (Main, High-1440, High; 4:2:2 Main and High).
//! - Progressive frame pictures: I and P pictures, and up to 7 B pictures between anchors
//!   (forward, backward or bi-directional prediction; reordered into decode order). Open GOPs,
//!   or closed ones whose leading B pictures predict from the I picture alone; a sequence header
//!   at every I picture.
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
//! let mut es = Vec::new();
//! // packets come in decode order; with B pictures, a picture may be held until its anchor
//! for p in enc.encode(&Input { y: &y, cb: &cb, cr: &cr, y_stride: 1920, c_stride: 960 })? {
//!     es.extend_from_slice(&p.data);
//! }
//! for p in enc.flush()? {
//!     es.extend_from_slice(&p.data);
//! }
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
use crate::vlc::{MB_BWD, MB_FWD, MB_INTRA, MB_PATTERN, MB_QUANT, MBA_ESCAPE};
use crate::{ChromaFormat, Decoder, Error, FieldOrder, PictureType, Result};

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
    /// B pictures between anchor pictures, 0-7 (2 gives the usual IBBP).
    pub b_frames: u32,
    /// Closed GOPs: the B pictures before each I picture predict from it alone, so every GOP
    /// decodes on its own (editing, chapter points). Open GOPs predict them from both sides.
    pub closed_gop: bool,
    /// Interlaced frame pictures with this field first (field or frame prediction and DCT per
    /// macroblock), or progressive (`None`).
    pub interlaced: Option<FieldOrder>,
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
    /// 4:2:0, quantiser 6, a closed GOP of 15 with I and P pictures only, 9-bit intra DC,
    /// square samples.
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Config {
        Config {
            width,
            height,
            chroma: ChromaFormat::Yuv420,
            fps_num,
            fps_den,
            rate: Rate::Quantizer(6),
            gop: 15,
            b_frames: 0,
            closed_gop: true,
            interlaced: None,
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

/// One coded picture (with the sequence and GOP headers before I pictures), in decode order.
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    pub keyframe: bool,
    pub picture_type: PictureType,
    /// Picture number in display order (the presentation time in frames).
    pub index: u64,
    /// Decode time in frames: the decode-order position, one frame earlier with B pictures so
    /// that every picture's decode time is at most its presentation time (-1 for the first).
    pub dts: i64,
}

/// A picture waiting for the anchor after it (B pictures), in display order.
struct Pending {
    index: u64,
    frame: Frame,
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
    /// Pictures received (display order) and coded (decode order).
    frames: u64,
    coded: u64,
    /// The reconstructed anchors: the older and the newer one.
    refs: [Option<Arc<Frame>>; 2],
    /// Display numbers of those anchors.
    anchor_index: [Option<u64>; 2],
    /// Display number of the current GOP's first picture (temporal_reference 0).
    gop_first: u64,
    /// B pictures waiting for their backward anchor.
    pending: Vec<Pending>,
    /// The last P picture's vectors (temporal predictors), over `mv_span` pictures.
    prev_mvs: Vec<[i32; 2]>,
    mv_span: u64,
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
        if cfg.b_frames > 7 {
            return Err(Error::Unsupported(format!("{} B pictures between anchors (0..=7)", cfg.b_frames)));
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
            coded: 0,
            refs: [None, None],
            anchor_index: [None, None],
            gop_first: 0,
            pending: Vec::new(),
            prev_mvs: vec![[0, 0]; mb_width * mb_height],
            mv_span: 1,
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

    /// Encode the next picture (display order). Returns the pictures that can be coded now, in
    /// decode order: none while B pictures wait for their anchor, then the anchor and those B
    /// pictures.
    pub fn encode(&mut self, input: &Input) -> Result<Vec<Packet>> {
        let frame = self.padded(input)?;
        let index = self.frames;
        self.frames += 1;
        let pos = index % u64::from(self.cfg.gop);
        if pos != 0 && !pos.is_multiple_of(u64::from(self.cfg.b_frames) + 1) {
            self.pending.push(Pending { index, frame });
            return Ok(Vec::new());
        }
        self.code_anchor(frame, index, pos == 0)
    }

    /// Code the pictures still waiting at the end of the stream: the last one becomes a P
    /// picture, the others B pictures before it.
    pub fn flush(&mut self) -> Result<Vec<Packet>> {
        let Some(last) = self.pending.pop() else { return Ok(Vec::new()) };
        self.code_anchor(last.frame, last.index, false)
    }

    /// The sequence_end_code that closes the stream.
    pub fn finish(&mut self) -> Vec<u8> {
        vec![0, 0, 1, crate::headers::SEQUENCE_END]
    }

    /// Code an anchor (I or P) and then the B pictures waiting before it.
    fn code_anchor(&mut self, frame: Frame, index: u64, intra: bool) -> Result<Vec<Packet>> {
        let pending = std::mem::take(&mut self.pending);
        let mut out = Vec::with_capacity(1 + pending.len());
        if intra {
            // the GOP starts with the B pictures displayed before its I picture
            self.gop_first = pending.first().map_or(index, |p| p.index);
        }
        let ptype = if intra { PictureType::I } else { PictureType::P };
        let past = self.refs[1].clone();
        out.push(self.code_picture(&frame, index, ptype, if intra { None } else { past.as_ref() }, None)?);
        // the anchors move on: the B pictures predict from the previous and this one
        let anchor = self.dec.last_anchor().ok_or_else(|| Error::Invalid("no reconstructed picture".into()))?;
        self.refs = [self.refs[1].take(), Some(anchor)];
        self.anchor_index = [self.anchor_index[1], Some(index)];
        let (fwd, bwd) = (if intra && self.cfg.closed_gop { None } else { self.refs[0].clone() }, self.refs[1].clone());
        for p in pending {
            out.push(self.code_picture(&p.frame, p.index, PictureType::B, fwd.as_ref(), bwd.as_ref())?);
        }
        Ok(out)
    }

    /// The temporal predictors of a picture at `index` towards an anchor at `anchor`: the last
    /// P picture's vectors scaled to the distance.
    fn scaled_mvs(&self, index: u64, anchor: Option<u64>) -> Vec<[i32; 2]> {
        let d = anchor.map_or(0, |a| index as i64 - a as i64);
        let span = self.mv_span.max(1) as i64;
        self.prev_mvs.iter().map(|v| [(v[0] as i64 * d / span) as i32, (v[1] as i64 * d / span) as i32]).collect()
    }

    /// Code one picture predicted from `fwd` (the anchor before) and / or `bwd` (the anchor
    /// after).
    fn code_picture(&mut self, cur: &Frame, index: u64, ptype: PictureType, fwd: Option<&Arc<Frame>>, bwd: Option<&Arc<Frame>>) -> Result<Packet> {
        let tref = index.saturating_sub(self.gop_first) as u32;
        let mut w = BitWriter::default();
        if ptype == PictureType::I {
            self.sequence_headers(&mut w);
            self.gop_header(&mut w, self.gop_first, self.cfg.closed_gop || self.gop_first == index);
            if let Some(rc) = self.rc.as_mut() {
                let m = u64::from(self.cfg.b_frames) + 1;
                let n = u64::from(self.cfg.gop);
                let np = (n - 1) / m;
                rc.start_gop(np as u32, (n - 1 - np) as u32);
            }
        }
        self.picture_header(&mut w, tref, ptype);
        // analysis
        let lambda = (self.last_scale * 0.6).round().clamp(1.0, 64.0) as u32;
        let (preds_f, preds_b) = if ptype == PictureType::B {
            (self.scaled_mvs(index, self.anchor_index[0]), self.scaled_mvs(index, self.anchor_index[1]))
        } else {
            (self.prev_mvs.clone(), Vec::new())
        };
        let ctx = Ctx {
            cur,
            fwd: fwd.map(|f| &**f),
            bwd: bwd.map(|b| &**b),
            cmbh: self.cmbh,
            nblocks: self.nblocks,
            mb_width: self.mb_width,
            preds_f: &preds_f,
            preds_b: &preds_b,
            lambda,
            interlaced: self.cfg.interlaced.is_some(),
        };
        let plans = self.analyse(&ctx);
        let avg_act = plans.iter().flatten().map(|p| p.act as f64).sum::<f64>() / (self.mb_width * self.mb_height).max(1) as f64;
        // slices
        let start = w.bits();
        let kind = match ptype {
            PictureType::I => rate::Kind::I,
            PictureType::B => rate::Kind::B,
            _ => rate::Kind::P,
        };
        if let Some(rc) = self.rc.as_mut() {
            let weights: Vec<f64> = plans.iter().flatten().map(|p| p.weight as f64).collect();
            rc.start_picture(kind, &weights, avg_act);
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
        // reconstruction: decode the picture (B pictures too: the decoder checks every one)
        let errors = self.dec.errors();
        self.dec.decode(&data, index as i64)?;
        if self.dec.errors() != errors {
            return Err(Error::Invalid(format!("encoder produced an undecodable picture: {}", self.dec.last_error().unwrap_or("?"))));
        }
        match ptype {
            PictureType::I => {
                self.prev_mvs.iter_mut().for_each(|v| *v = [0, 0]);
                self.mv_span = 1;
            }
            PictureType::P => {
                self.prev_mvs = plans.iter().flatten().map(|p| if p.intra { [0, 0] } else { p.frame_mv(0) }).collect();
                self.mv_span = self.anchor_index[1].map_or(1, |a| index.saturating_sub(a).max(1));
            }
            _ => {}
        }
        let dts = self.coded as i64 - i64::from(self.cfg.b_frames > 0);
        self.coded += 1;
        Ok(Packet { data, keyframe: ptype == PictureType::I, picture_type: ptype, index, dts })
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
        w.put(self.cfg.interlaced.is_none() as u32, 1); // progressive_sequence
        w.put(if self.cmbh == 8 { 1 } else { 2 }, 2);
        w.put(c.width >> 12, 2);
        w.put(c.height >> 12, 2);
        w.put(self.bit_rate_units >> 18, 12);
        w.put(1, 1); // marker_bit
        w.put(self.vbv_units >> 10, 8);
        w.put((self.cfg.b_frames == 0) as u32, 1); // low_delay
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

    /// A GOP header whose time code is that of picture `index`.
    fn gop_header(&self, w: &mut BitWriter, index: u64, closed: bool) {
        let fps = (self.cfg.fps_num as u64).div_ceil(self.cfg.fps_den.max(1) as u64).max(1);
        let secs = index / fps;
        w.start(crate::headers::GROUP_START);
        w.put(0, 1); // drop_frame_flag
        w.put((secs / 3600 % 24) as u32, 5);
        w.put((secs / 60 % 60) as u32, 6);
        w.put(1, 1); // marker_bit
        w.put((secs % 60) as u32, 6);
        w.put((index % fps) as u32, 6);
        w.put(closed as u32, 1); // closed_gop
        w.put(0, 1); // broken_link
    }

    fn picture_header(&self, w: &mut BitWriter, tref: u32, ptype: PictureType) {
        w.start(crate::headers::PICTURE_START);
        w.put(tref & 1023, 10);
        w.put(
            match ptype {
                PictureType::I => 1,
                PictureType::B => 3,
                _ => 2,
            },
            3,
        );
        w.put(0xFFFF, 16); // vbv_delay: variable bitrate
        if ptype != PictureType::I {
            w.put(0, 1); // full_pel_forward_vector
            w.put(7, 3); // forward_f_code (MPEG-2: in the extension)
        }
        if ptype == PictureType::B {
            w.put(0, 1); // full_pel_backward_vector
            w.put(7, 3); // backward_f_code
        }
        w.put(0, 1); // extra_bit_picture
        // picture coding extension
        w.start(crate::headers::EXTENSION);
        w.put(8, 4);
        let f = if ptype == PictureType::I { 15 } else { F_CODE };
        let b = if ptype == PictureType::B { F_CODE } else { 15 };
        for v in [f, f, b, b] {
            w.put(v, 4);
        }
        w.put((self.cfg.intra_dc_precision - 8) as u32, 2);
        let progressive = self.cfg.interlaced.is_none();
        w.put(3, 2); // picture_structure: frame
        w.put((self.cfg.interlaced == Some(FieldOrder::TopFirst)) as u32, 1); // top_field_first
        w.put(progressive as u32, 1); // frame_pred_frame_dct
        w.put(0, 1); // concealment_motion_vectors
        w.put(1, 1); // q_scale_type: non-linear
        w.put(self.intra_vlc as u32, 1); // intra_vlc_format
        w.put(!progressive as u32, 1); // alternate_scan
        w.put(0, 1); // repeat_first_field
        w.put((self.cmbh == 8 && progressive) as u32, 1); // chroma_420_type = progressive_frame for 4:2:0
        w.put(progressive as u32, 1); // progressive_frame
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
        let interlaced = self.cfg.interlaced.is_some();
        let scan = &SCAN[interlaced as usize];
        let mut code = self.mb_code(row * mbw, w.bits() - pic_start, plans.first().map_or(1.0, |p| p.act));
        w.start(1 + (row & 127) as u8);
        if self.cfg.height > 2800 {
            w.put((row >> 7) as u32, 3); // slice_vertical_position_extension
        }
        w.put(code as u32, 5);
        w.put(0, 1); // extra_bit_slice
        let mut dc = [dc_reset; 3];
        // motion vector predictors PMV[r][s] as the decoder keeps them (§7.6.3.4), and the
        // previous macroblock's prediction directions when it used frame prediction (B
        // pictures skip by repeating them)
        let mut pmv = [[[0i32; 2]; 2]; 2];
        let mut prev_dir: Option<i16> = None;
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
                w.code(match ptype {
                    PictureType::I => t.mbtype_i[flags as usize],
                    PictureType::B => t.mbtype_b[flags as usize],
                    _ => t.mbtype_p[flags as usize],
                });
                if interlaced {
                    w.put(plan.field_dct as u32, 1); // dct_type
                }
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
                    write_coefficients(w, t, lv, scan, true, self.intra_vlc, &mut self.vlc_cost);
                }
                pmv = [[[0; 2]; 2]; 2];
                prev_dir = None;
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
            let mut dir = plan.dir;
            if ptype == PictureType::B {
                // a skipped B macroblock repeats the previous one's frame prediction (§7.6.6.4)
                let same = !plan.field_mc
                    && prev_dir == Some(dir)
                    && (dir & MB_FWD == 0 || plan.mvs[0][0] == pmv[0][0])
                    && (dir & MB_BWD == 0 || plan.mvs[0][1] == pmv[0][1]);
                if cbp == 0 && same && mbx > 0 && mbx + 1 < mbw {
                    skipped += 1;
                    scale_sum += NON_LINEAR_Q[code as usize] as f64;
                    continue;
                }
            } else {
                // P: skipped, or "MC not coded", when frame prediction has a zero vector
                let zero = !plan.field_mc && plan.mvs[0][0] == [0, 0];
                if cbp == 0 && zero && mbx > 0 && mbx + 1 < mbw {
                    skipped += 1;
                    pmv = [[[0; 2]; 2]; 2];
                    scale_sum += NON_LINEAR_Q[code as usize] as f64;
                    continue;
                }
                if zero && cbp != 0 {
                    dir = 0;
                }
            }
            address_increment(w, t, skipped + 1);
            skipped = 0;
            let quant = cbp != 0 && want != code;
            if cbp != 0 {
                code = want;
            }
            scale_sum += NON_LINEAR_Q[code as usize] as f64;
            let flags = dir | if cbp != 0 { MB_PATTERN } else { 0 } | if quant { MB_QUANT } else { 0 };
            w.code(if ptype == PictureType::B { t.mbtype_b[flags as usize] } else { t.mbtype_p[flags as usize] });
            if interlaced && dir != 0 {
                w.put(if plan.field_mc { 1 } else { 2 }, 2); // frame_motion_type
            }
            if interlaced && cbp != 0 {
                w.put(plan.field_dct as u32, 1); // dct_type
            }
            if quant {
                w.put(code as u32, 5);
            }
            for (s, bit) in [MB_FWD, MB_BWD].into_iter().enumerate() {
                if dir & bit != 0 {
                    write_vectors(w, t, plan, s, &mut pmv);
                }
            }
            if dir == 0 {
                pmv = [[[0; 2]; 2]; 2];
            }
            prev_dir = if plan.field_mc { None } else { Some(dir) };
            write_pattern(w, t, cbp, self.nblocks, &levels, scan, &mut self.vlc_cost);
        }
        scale_sum
    }
}

/// The vectors of direction `s` of a macroblock against the predictors, which they replace
/// (§7.6.3): one frame vector, or a field select and a vector per field (vertical predicted
/// from half the stored predictor, stored doubled).
fn write_vectors(w: &mut BitWriter, t: &EncTables, plan: &Plan, s: usize, pmv: &mut [[[i32; 2]; 2]; 2]) {
    if plan.field_mc {
        for r in 0..2 {
            w.put(plan.sel[r][s] as u32, 1); // motion_vertical_field_select
            let v = plan.mvs[r][s];
            write_motion(w, t, v[0] - pmv[r][s][0]);
            write_motion(w, t, v[1] - (pmv[r][s][1] >> 1));
            pmv[r][s] = [v[0], v[1] * 2];
        }
    } else {
        let v = plan.mvs[0][s];
        for c in 0..2 {
            write_motion(w, t, v[c] - pmv[0][s][c]);
        }
        pmv[0][s] = v;
        pmv[1][s] = v;
    }
}

/// coded_block_pattern and the coded blocks of a non-intra macroblock (nothing when `cbp` is 0).
fn write_pattern(w: &mut BitWriter, t: &EncTables, cbp: u32, nblocks: usize, levels: &[[i32; 64]; 8], scan: &[u8; 64], cost: &mut [u64; 2]) {
    if cbp == 0 {
        return;
    }
    if nblocks == 8 {
        w.code(t.cbp[(cbp >> 2) as usize]);
        w.put(cbp & 3, 2);
    } else {
        w.code(t.cbp[cbp as usize]);
    }
    for (i, lv) in levels.iter().take(nblocks).enumerate() {
        if cbp & (1 << (nblocks - 1 - i)) != 0 {
            write_coefficients(w, t, lv, scan, false, false, cost);
        }
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

/// The run/level pairs of a block in `scan` order (from the first AC coefficient for intra
/// blocks), then end_of_block. Intra blocks use table one when `one`, and add what they would
/// cost with table zero and table one to `cost`.
fn write_coefficients(w: &mut BitWriter, t: &EncTables, levels: &[i32; 64], scan: &[u8; 64], intra: bool, one: bool, cost: &mut [u64; 2]) {
    let tab = &t.dct[one as usize];
    let code_of = |tab: &[[Code; MAX_TABLE_LEVEL + 1]], run: usize, a: usize| tab.get(run).and_then(|r| r.get(a)).copied().filter(|c| c.1 > 0);
    let mut run = 0usize;
    let mut first = !intra;
    for &pos in &scan[intra as usize..] {
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
