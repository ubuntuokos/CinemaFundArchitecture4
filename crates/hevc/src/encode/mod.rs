//! H.265 / HEVC encoder: Main and Main 10 profiles, 4:2:0, I and P pictures.
//!
//! The encoder is built on the decoder's own pieces: intra prediction, motion compensation, merge /
//! AMVP derivation, scaling and inverse transforms are the decoder's functions, and every coded
//! picture is decoded by an internal [`Decoder`] whose output (with deblocking) becomes the
//! reference for later pictures. The reconstruction can therefore never drift from what any
//! conforming decoder produces.
//!
//! Coding tools: CTUs of 32x32 with CUs from 32x32 to 8x8 chosen by rate-distortion cost, transform
//! units the size of the CU (DCT 8..32, 4x4 chroma), intra 2Nx2N with the 35 luma modes and the DM
//! chroma mode, inter 2Nx2N with skip, merge (temporal candidates included) and AMVP with a
//! hexagon search and quarter-sample refinement, up to four reference pictures, deblocking. Frame
//! level rate control: constant QP, constant quality or average bitrate with a VBV.

mod cabac;
mod ctu;
mod fwd;
mod headers;
mod rc;
#[cfg(test)]
mod tests;

use crate::Decoder;
use crate::cabac::{Contexts, init_contexts};
use crate::params::{Layout, Pps, Sps};
use crate::picture::{Frame, RefPic, Sample};
use crate::slice::{NalHeader, SliceHeader};
use crate::slicedec::PicState;
use crate::spec_tables::NUM_CTX;
use ctu::{Coder, RefPlane, Source};
use headers::{SeqParams, SliceParams};
use std::collections::VecDeque;
use std::sync::Arc;

/// Encoder errors.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("invalid dimensions {0}x{1} (must be even and non-zero)")]
    InvalidDimensions(u32, u32),
    #[error("invalid frame rate {0}/{1}")]
    InvalidFrameRate(u32, u32),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("input frame planes too small for the configured size")]
    BadFrame,
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preset {
    Speed,
    #[default]
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateControl {
    /// Constant quality; roughly the QP of P pictures (0–51, lower is better).
    Crf(f32),
    /// Constant bitrate target (1-pass ABR with a 1-second VBV at `kbps`).
    Cbr { kbps: u32 },
    /// Variable bitrate: average `target_kbps`, VBV-limited to `max_kbps`.
    Vbr { target_kbps: u32, max_kbps: u32 },
    /// Constant QP (I pictures use QP - 2).
    Qp(u8),
}

/// Colour description written to the VUI (ITU-T H.273 code points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorConfig {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

impl Default for ColorConfig {
    /// BT.709, limited range.
    fn default() -> Self {
        ColorConfig { primaries: 1, transfer: 1, matrix: 1, full_range: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PacketFormat {
    /// Start-code prefixed NAL units, parameter sets in-band on keyframes.
    #[default]
    AnnexB,
    /// 4-byte big-endian length-prefixed NAL units (MP4 samples); parameter sets live in
    /// [`Encoder::hvcc`].
    LengthPrefixed,
}

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// 8 (Main) or 10 (Main 10).
    pub bit_depth: u8,
    pub preset: Preset,
    pub rate: RateControl,
    /// Maximum distance between IDR pictures (0: only the first picture).
    pub keyint: u32,
    /// Reference pictures of P pictures (1–4).
    pub refs: u8,
    pub color: ColorConfig,
    pub format: PacketFormat,
    /// Sample aspect ratio (1:1 by default).
    pub sar: (u16, u16),
    /// HDR static metadata sent in an SEI with every IDR: the 24-byte SMPTE ST 2086 mastering
    /// display payload (payloadType 137) …
    pub mastering_display: Option<[u8; 24]>,
    /// … and (MaxCLL, MaxFALL) content light level (payloadType 144).
    pub content_light: Option<(u16, u16)>,
    /// Slices per picture, coded in parallel. 0 picks one per worker thread (at most 8, with at
    /// least 4 CTU rows each). The output depends on the slice count, never on the thread count.
    pub slices: u32,
}

impl EncoderConfig {
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        EncoderConfig {
            width,
            height,
            fps_num,
            fps_den,
            bit_depth: 8,
            preset: Preset::Balanced,
            rate: RateControl::Crf(28.0),
            keyint: 250,
            refs: 2,
            color: ColorConfig::default(),
            format: PacketFormat::AnnexB,
            sar: (1, 1),
            mastering_display: None,
            content_light: None,
            slices: 0,
        }
    }
}

/// A planar 4:2:0 input picture of the configured size: 8-bit samples (scaled up for Main 10), or
/// samples at the configured bit depth in 16-bit words.
#[derive(Clone, Copy, Debug)]
pub enum YuvFrame<'a> {
    U8 { y: &'a [u8], u: &'a [u8], v: &'a [u8], y_stride: usize, uv_stride: usize },
    U16 { y: &'a [u16], u: &'a [u16], v: &'a [u16], y_stride: usize, uv_stride: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    Idr,
    P,
}

/// One coded picture (access unit).
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub keyframe: bool,
    pub frame_type: FrameType,
    pub qp: u8,
}

/// H.265 encoder.
pub struct Encoder {
    cfg: EncoderConfig,
    core: Core,
}

enum Core {
    E8(Box<State<u8>>),
    E16(Box<State<u16>>),
}

struct State<S> {
    seq: SeqParams,
    vps: Vec<u8>,
    sps_nal: Vec<u8>,
    pps_nal: Vec<u8>,
    sei: Option<Vec<u8>>,
    sps: Arc<Sps>,
    pps: Arc<Pps>,
    layout: Arc<Layout>,
    dec: Decoder,
    /// Reference pictures, most recent first.
    dpb: VecDeque<(RefPic, RefPlane<S>)>,
    poc: i32,
    frames: u64,
    rc: rc::RateControl,
    prev_low: Option<rc::LowRes>,
    preset: Preset,
    refs: usize,
    keyint: u32,
    /// Slices per picture.
    slices: usize,
    /// Plane buffers reused between pictures (one per slice).
    scratch: Vec<PicState<S>>,
}

/// One slice of the picture being coded.
struct SliceJob<S> {
    /// CTU rows [start, end).
    rows: (usize, usize),
    address: u32,
    header: Vec<u8>,
    sh: SliceHeader,
    pic: PicState<S>,
}

/// Distance of the integer motion search border from the picture.
const PAD: usize = 80;

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Self, EncodeError> {
        let (w, h) = (cfg.width, cfg.height);
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || w > 8192 || h > 4320 {
            return Err(EncodeError::InvalidDimensions(w, h));
        }
        if cfg.fps_num == 0 || cfg.fps_den == 0 {
            return Err(EncodeError::InvalidFrameRate(cfg.fps_num, cfg.fps_den));
        }
        if cfg.bit_depth != 8 && cfg.bit_depth != 10 {
            return Err(EncodeError::InvalidConfig(format!("bit depth {} (8 or 10)", cfg.bit_depth)));
        }
        let core = if cfg.bit_depth == 8 { Core::E8(Box::new(State::new(&cfg)?)) } else { Core::E16(Box::new(State::new(&cfg)?)) };
        Ok(Encoder { cfg, core })
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// VPS, SPS and PPS NAL units (without start codes or length prefixes).
    pub fn parameter_sets(&self) -> [Vec<u8>; 3] {
        let (v, s, p) = match &self.core {
            Core::E8(st) => (&st.vps, &st.sps_nal, &st.pps_nal),
            Core::E16(st) => (&st.vps, &st.sps_nal, &st.pps_nal),
        };
        [v.clone(), s.clone(), p.clone()]
    }

    /// HEVCDecoderConfigurationRecord (ISO/IEC 14496-15 8.3.3.1) for an `hvcC` box.
    pub fn hvcc(&self) -> Vec<u8> {
        let [vps, sps, pps] = self.parameter_sets();
        let main10 = self.cfg.bit_depth > 8;
        let level = match &self.core {
            Core::E8(st) => st.seq.level_idc,
            Core::E16(st) => st.seq.level_idc,
        };
        let mut out = vec![1u8];
        out.push(if main10 { 2 } else { 1 }); // profile_space 0, tier 0, profile_idc
        let compat: u32 = if main10 { 1 << 29 } else { (1 << 30) | (1 << 29) };
        out.extend(compat.to_be_bytes());
        out.extend([0x90, 0, 0, 0, 0, 0]); // progressive, frame only
        out.push(level);
        out.extend([0xf0, 0x00]); // min_spatial_segmentation_idc 0
        out.push(0xfc); // parallelismType 0
        out.push(0xfc | 1); // chroma_format_idc 4:2:0
        let bd = (self.cfg.bit_depth - 8) & 7;
        out.push(0xf8 | bd);
        out.push(0xf8 | bd);
        out.extend([0, 0]); // avgFrameRate
        out.push((1 << 3) | (1 << 2) | 3); // 1 temporal layer, nested, 4-byte lengths
        out.push(3);
        for (t, n) in [(32u8, &vps), (33, &sps), (34, &pps)] {
            out.push(0x80 | t);
            out.extend(1u16.to_be_bytes());
            out.extend((n.len() as u16).to_be_bytes());
            out.extend_from_slice(n);
        }
        out
    }

    /// Encode one picture; returns its access unit (pictures are coded in display order, so each
    /// call returns one packet).
    pub fn encode(&mut self, frame: &YuvFrame, pts: i64) -> Result<Vec<Packet>, EncodeError> {
        let cfg = &self.cfg;
        let p = match &mut self.core {
            Core::E8(st) => st.encode(cfg, frame, pts)?,
            Core::E16(st) => st.encode(cfg, frame, pts)?,
        };
        Ok(vec![p])
    }

    /// End of stream: nothing is held back.
    pub fn flush(&mut self) -> Result<Vec<Packet>, EncodeError> {
        Ok(Vec::new())
    }
}

/// Slices per picture of `ctb_rows` CTU rows.
fn slice_count(cfg: &EncoderConfig, ctb_rows: usize) -> usize {
    let n = if cfg.slices > 0 {
        cfg.slices as usize
    } else {
        #[cfg(feature = "threads")]
        let threads = rayon::current_num_threads();
        #[cfg(not(feature = "threads"))]
        let threads = 1;
        threads.min(8).min(ctb_rows / 4)
    };
    n.clamp(1, ctb_rows.max(1))
}

fn internal(e: impl std::fmt::Display) -> EncodeError {
    EncodeError::Internal(e.to_string())
}

impl<S: Sample> State<S> {
    fn new(cfg: &EncoderConfig) -> Result<Self, EncodeError> {
        let log2_ctb = 5;
        let log2_min_cb = 3;
        let align = 1u32 << log2_min_cb;
        let (coded_w, coded_h) = (cfg.width.div_ceil(align) * align, cfg.height.div_ceil(align) * align);
        let fps = cfg.fps_num as f64 / cfg.fps_den as f64;
        let refs = cfg.refs.clamp(1, 4) as usize;
        let c = cfg.color;
        let seq = SeqParams {
            coded_w,
            coded_h,
            width: cfg.width,
            height: cfg.height,
            bit_depth: cfg.bit_depth as u32,
            log2_ctb,
            log2_min_cb,
            log2_max_tb: 5,
            log2_max_poc_lsb: 8,
            max_dec_pic_buffering: refs as u32 + 1,
            level_idc: headers::level_for(coded_w, coded_h, fps),
            fps: (cfg.fps_num, cfg.fps_den),
            sar: cfg.sar,
            color: Some((c.primaries, c.transfer, c.matrix, c.full_range)),
            sao: false,
            tmvp: true,
            amp: false,
            sign_hiding: false,
            cu_qp_delta: false,
        };
        let vps = headers::vps(&seq);
        let sps_nal = headers::sps(&seq);
        let pps_nal = headers::pps(&seq);
        let sps = Arc::new(Sps::parse(&cfa4_bitstream::unescape_rbsp(&sps_nal[2..])).map_err(internal)?);
        let pps = Arc::new(Pps::parse(&cfa4_bitstream::unescape_rbsp(&pps_nal[2..])).map_err(internal)?);
        let layout = Arc::new(Layout::new(&sps, &pps).map_err(internal)?);
        let (kind, vbv) = match cfg.rate {
            RateControl::Qp(q) => (rc::RcKind::Qp(q.min(51)), (None, None)),
            RateControl::Crf(c) => (rc::RcKind::Crf(if c.is_finite() { c.clamp(0.0, 51.0) } else { 28.0 }), (None, None)),
            RateControl::Cbr { kbps } => (rc::RcKind::Abr { kbps: kbps.max(1) }, (Some(kbps.max(1)), Some(kbps.max(1)))),
            RateControl::Vbr { target_kbps, max_kbps } => {
                let max = max_kbps.max(target_kbps).max(1);
                (rc::RcKind::Abr { kbps: target_kbps.max(1) }, (Some(max), Some(max * 2)))
            }
        };
        Ok(State {
            sei: headers::hdr_sei(cfg.mastering_display.as_ref(), cfg.content_light),
            seq,
            vps,
            sps_nal,
            pps_nal,
            sps,
            pps,
            layout,
            dec: Decoder::with_threads(1),
            dpb: VecDeque::new(),
            poc: 0,
            frames: 0,
            rc: rc::RateControl::new(kind, fps, vbv.0, vbv.1),
            prev_low: None,
            preset: cfg.preset,
            refs,
            keyint: cfg.keyint,
            slices: slice_count(cfg, coded_h.div_ceil(1 << log2_ctb) as usize),
            scratch: Vec::new(),
        })
    }

    /// The input at the coded size with edge replication, at the coded bit depth.
    fn source(&self, f: &YuvFrame) -> Result<Source, EncodeError> {
        let (w, h) = (self.seq.coded_w as usize, self.seq.coded_h as usize);
        let (vw, vh) = (self.seq.width as usize, self.seq.height as usize);
        let bd = self.seq.bit_depth;
        let mut src = Source { y: vec![0; w * h], u: vec![0; w * h / 4], v: vec![0; w * h / 4], w, h };
        fn fill<T: Copy>(dst: &mut [u16], dw: usize, dh: usize, s: &[T], stride: usize, vw: usize, vh: usize, conv: impl Fn(T) -> u16) -> bool {
            if vw == 0 || vh == 0 || stride < vw || s.len() < (vh - 1) * stride + vw {
                return false;
            }
            for y in 0..dh {
                let sy = y.min(vh - 1);
                let row = &s[sy * stride..sy * stride + vw];
                for x in 0..dw {
                    dst[y * dw + x] = conv(row[x.min(vw - 1)]);
                }
            }
            true
        }
        let max = (1u32 << bd) - 1;
        let ok = match *f {
            YuvFrame::U8 { y, u, v, y_stride, uv_stride } => {
                let up = bd - 8;
                let c = |s: u8| (s as u16) << up;
                fill(&mut src.y, w, h, y, y_stride, vw, vh, c)
                    && fill(&mut src.u, w / 2, h / 2, u, uv_stride, vw / 2, vh / 2, c)
                    && fill(&mut src.v, w / 2, h / 2, v, uv_stride, vw / 2, vh / 2, c)
            }
            YuvFrame::U16 { y, u, v, y_stride, uv_stride } => {
                let c = |s: u16| (s as u32).min(max) as u16;
                fill(&mut src.y, w, h, y, y_stride, vw, vh, c)
                    && fill(&mut src.u, w / 2, h / 2, u, uv_stride, vw / 2, vh / 2, c)
                    && fill(&mut src.v, w / 2, h / 2, v, uv_stride, vw / 2, vh / 2, c)
            }
        };
        if ok { Ok(src) } else { Err(EncodeError::BadFrame) }
    }

    fn encode(&mut self, cfg: &EncoderConfig, frame: &YuvFrame, pts: i64) -> Result<Packet, EncodeError> {
        let src = self.source(frame)?;
        let idr = self.frames == 0 || (self.keyint > 0 && self.frames.is_multiple_of(self.keyint as u64));
        if idr {
            self.dpb.clear();
            self.poc = 0;
            self.frames = 0;
        }
        let bd = self.seq.bit_depth;
        let low = rc::LowRes::new(&src.y, src.w, src.h, bd);
        let cplx = rc::complexity(&low, self.prev_low.as_ref(), idr);
        let kind = if idr { rc::Kind::I } else { rc::Kind::P };
        let qp_f = self.rc.frame_qp(kind, cplx);
        let qp = (qp_f.round() as i32).clamp(0, 51);
        let refs_n = self.dpb.len().min(self.refs);
        let (max_merge, intra_rd, me_range, fast_intra) = match self.preset {
            Preset::Speed => (2, 1, 16, true),
            Preset::Balanced => (3, 2, 32, true),
            Preset::Quality => (5, 3, 64, false),
        };
        let nal_type = if idr { 19 } else { 1 };
        let nal_hdr = NalHeader { nal_type, layer_id: 0, temporal_id: 0 };
        let refs: [Vec<RefPic>; 2] = [self.dpb.iter().take(refs_n).map(|(r, _)| r.clone()).collect(), Vec::new()];
        let (cw, ch) = (self.seq.coded_w as usize, self.seq.coded_h as usize);
        let frame_ref = Arc::new(Frame::new(0, self.poc, cw, ch, self.sps.log2_ctb, bd, bd, (1, 1)));
        let qp_bd = 6 * (bd as i32 - 8);
        let qpi = qp.clamp(-qp_bd, 57);
        let qp_c = crate::tables::qpc(1, qpi) + qp_bd;
        let lambda = 0.57 * 2f64.powf((qp as f64 - 12.0) / 3.0) * (1u64 << (2 * (bd - 8))) as f64;
        let ref_planes: Vec<&RefPlane<S>> = self.dpb.iter().take(refs_n).map(|(_, p)| p).collect();
        let ctx_init = {
            let mut c: Contexts = [0; NUM_CTX.next_power_of_two()];
            init_contexts(&mut c, qp, if idr { 0 } else { 1 });
            c
        };
        // slices of whole CTU rows, each coded independently (in parallel with `threads`)
        let (wc, hc) = (self.layout.width_ctbs as usize, self.layout.height_ctbs as usize);
        let n_slices = self.slices.clamp(1, hc.max(1));
        let ctbs = (wc * hc) as u32;
        let address_bits = u32::BITS - ctbs.saturating_sub(1).leading_zeros();
        let mut jobs: Vec<SliceJob<S>> = Vec::with_capacity(n_slices);
        for i in 0..n_slices {
            let (r0, r1) = (i * hc / n_slices, (i + 1) * hc / n_slices);
            let address = (r0 * wc) as u32;
            let sp = SliceParams {
                idr,
                intra: idr,
                poc: self.poc,
                ref_deltas: self.dpb.iter().take(refs_n).map(|(r, _)| r.poc - self.poc).collect(),
                qp,
                max_merge_cand: max_merge,
                address,
                address_bits,
            };
            let header = headers::slice_header(&self.seq, &sp);
            let (sps, pps) = (self.sps.clone(), self.pps.clone());
            let (sh, _, _) = SliceHeader::parse(&header, nal_hdr, |_| Ok((pps.clone(), sps.clone())), None).map_err(internal)?;
            // picture state (the planes are the reconstruction before in-loop filtering)
            let pic = match self.scratch.pop() {
                Some(mut p) => {
                    p.frame = frame_ref.clone();
                    p.poc = self.poc;
                    p.ctb_addr.fill(u32::MAX);
                    p.ctb_slice.fill(u32::MAX);
                    p
                }
                None => PicState::new(frame_ref.clone(), self.sps.clone(), self.pps.clone(), self.layout.clone(), None),
            };
            jobs.push(SliceJob { rows: (r0, r1), address, header, sh, pic });
        }
        let code = |job: &mut SliceJob<S>| -> Vec<u8> {
            let mut w = cabac::Writer::new(ctx_init);
            let mut coder = Coder {
                pic: &mut job.pic,
                sps: &self.sps,
                pps: &self.pps,
                sh: &job.sh,
                refs: &refs,
                col: refs[0].first(),
                ref_planes: &ref_planes,
                src: &src,
                qp_y: qp,
                qp: [qp + qp_bd, qp_c, qp_c],
                lambda,
                sqrt_lambda: lambda.sqrt(),
                me_range,
                intra_rd_modes: intra_rd,
                fast_intra,
                slice_addr: job.address,
                refs_buf: Default::default(),
                mc: Default::default(),
                pred0: vec![0; 64 * 64],
                pred1: vec![0; 64 * 64],
                diff: vec![0; 64 * 64],
                coef: vec![0; 64 * 64],
            };
            let (r0, r1) = job.rows;
            for ry in r0..r1 {
                for rx in 0..wc {
                    coder.ctu(rx, ry, &mut w);
                    let last = ry + 1 == r1 && rx + 1 == wc;
                    cabac::Bins::terminate(&mut w, last as u32);
                }
            }
            let mut rbsp = std::mem::take(&mut job.header);
            rbsp.extend(w.finish());
            headers::nal(nal_type, &rbsp)
        };
        #[cfg(feature = "threads")]
        let slices: Vec<Vec<u8>> = {
            use rayon::prelude::*;
            jobs.par_iter_mut().map(code).collect()
        };
        #[cfg(not(feature = "threads"))]
        let slices: Vec<Vec<u8>> = jobs.iter_mut().map(code).collect();
        self.scratch.extend(jobs.into_iter().map(|j| j.pic));
        let mut nals: Vec<&[u8]> = Vec::new();
        if idr {
            nals.push(&self.vps);
            nals.push(&self.sps_nal);
            nals.push(&self.pps_nal);
            if let Some(s) = &self.sei {
                nals.push(s);
            }
        }
        nals.extend(slices.iter().map(|s| s.as_slice()));
        // decode it: the decoder's picture (deblocked) becomes the reference
        let mut annexb = Vec::new();
        for n in &nals {
            annexb.extend([0, 0, 0, 1]);
            annexb.extend_from_slice(n);
        }
        self.dec.decode(&annexb, pts).map_err(internal)?;
        let decoded = self.dec.last_frame().ok_or_else(|| internal("no decoded picture"))?;
        decoded.wait_complete();
        let r = RefPic { frame: decoded, poc: self.poc, long_term: false };
        let plane = RefPlane::new(&r, cw, ch, PAD);
        self.dpb.push_front((r, plane));
        self.dpb.truncate(self.refs);
        let data = match cfg.format {
            PacketFormat::AnnexB => annexb,
            PacketFormat::LengthPrefixed => {
                let mut out = Vec::new();
                for n in nals.iter().filter(|n| !matches!(n.first().map(|b| (b >> 1) & 0x3f), Some(32..=34))) {
                    out.extend((n.len() as u32).to_be_bytes());
                    out.extend_from_slice(n);
                }
                out
            }
        };
        self.rc.update(kind, qp as f64, cplx, (slices.iter().map(Vec::len).sum::<usize>() * 8) as u64);
        self.prev_low = Some(low);
        self.poc += 1;
        self.frames += 1;
        Ok(Packet { data, pts, dts: pts, keyframe: idr, frame_type: if idr { FrameType::Idr } else { FrameType::P }, qp: qp as u8 })
    }
}
