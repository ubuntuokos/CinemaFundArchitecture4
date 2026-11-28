//! DNxHR and DNxHD encoder (RI and progressive HD profiles, CBR).
//!
//! Every macroblock is transformed once (orthonormal 8×8 FDCT). Quantisation follows equation
//! 8.2 exactly (`X̂ = ⌊p·|X| / (qsf·W)⌋`, which equation (8.1) reconstructs near the interval
//! midpoint); with `ffmpeg_compat` the amplitude nearest to FFmpeg's `(A + ¾)` reconstruction. Rate control: a binary search finds the smallest uniform quantisation scale factor
//! whose frame fits the CBR payload, then macroblocks get one finer step in scan order while the
//! budget allows.
//!
//! Interlaced DNxHD IDs (1241–1243) are field coded: each frame becomes two coding units of half
//! the Table C.1 frame size, field 1 (the top field, even lines) then field 2, each with its own
//! rate control (§7.2.2: `FFC`, `SST` = 1, `FFE` = 0).

use crate::bits::Writer;
use crate::dct::fdct8x8;
use crate::header::{self, EOF_SIGNATURE, FieldCode, FrameHeader, SCAN_INDEX_OFFSET};
use crate::tables::{CidInfo, VlcSet, ZIGZAG, cid_info, vlc};
use crate::{ChromaFormat, ColorVolume, Error, Frame, Profile, Result};

/// Encoder configuration.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub profile: Profile,
    pub width: u32,
    pub height: u32,
    /// Coded sample depth: 8 for LB/SQ/HQ; 10 or 12 for HQX and 444.
    pub bit_depth: u8,
    pub color_volume: ColorVolume,
    /// Pixel aspect ratio to signal (0/0 = not signalled).
    pub par: (u16, u16),
    /// Write what FFmpeg's decoder expects instead of SMPTE ST 2019-1: amplitudes for its
    /// `(A + ¾)` reconstruction and its 4:4:4 signalling (RGB with CLF = 0 in G, B, R order;
    /// Y′CbCr with CLF = 1 and ACF = 1). Default false: the standard (CFA4-DNX-SPEC-001).
    pub ffmpeg_compat: bool,
}

impl EncoderConfig {
    pub fn new(profile: Profile, width: u32, height: u32) -> EncoderConfig {
        let bit_depth = match profile.hd_raster() {
            Some((_, _, depth)) => depth,
            None if matches!(profile, Profile::Hqx | Profile::R444) => 10,
            None => 8,
        };
        EncoderConfig { profile, width, height, bit_depth, color_volume: ColorVolume::Bt709, par: (0, 0), ffmpeg_compat: false }
    }

    pub fn chroma(&self) -> ChromaFormat {
        if self.profile.is_444() { ChromaFormat::Yuv444 } else { ChromaFormat::Yuv422 }
    }
}

/// DNxHR encoder.
pub struct Encoder {
    cfg: EncoderConfig,
    info: &'static CidInfo,
    frame_size: u32,
    header_size: u32,
    last_q: u32,
}

/// Transformed macroblock data: per block, the DC value and the AC magnitudes pre-divided by
/// the weights (`p·|X|/W`, sign in the float's sign) in bitstream (zig-zag) order.
struct Prepared {
    nw: usize,
    ns: usize,
    nblk: usize,
    /// `[mb * nblk + k][r]`; `[..][0]` is the DC value.
    t: Vec<[f32; 64]>,
    /// DC bits of each macroblock (independent of the quantiser).
    dc_bits: Vec<u32>,
}

impl Encoder {
    /// An encoder with the default configuration; fails on rasters the profile can't code.
    pub fn new(profile: Profile, width: u32, height: u32) -> Result<Encoder> {
        Encoder::with_config(EncoderConfig::new(profile, width, height))
    }

    pub fn with_config(cfg: EncoderConfig) -> Result<Encoder> {
        if cfg.width == 0 || cfg.height == 0 || cfg.width > 16384 || cfg.height > 16384 {
            return Err(Error::Input(format!("unsupported raster {}x{}", cfg.width, cfg.height)));
        }
        if cfg.chroma() == ChromaFormat::Yuv422 && !cfg.width.is_multiple_of(2) {
            return Err(Error::Input("4:2:2 needs an even width".into()));
        }
        // DNxHD: one raster and depth per compression ID (Table C.1)
        if let Some((w, h, depth)) = cfg.profile.hd_raster() {
            if (cfg.width, cfg.height) != (w, h) {
                return Err(Error::Input(format!("{} codes {w}x{h} pictures, not {}x{}", cfg.profile.name(), cfg.width, cfg.height)));
            }
            if cfg.bit_depth != depth {
                return Err(Error::Input(format!("{} is {depth}-bit, not {}-bit", cfg.profile.name(), cfg.bit_depth)));
            }
        }
        let ok_depth = match cfg.profile {
            _ if cfg.profile.hd_raster().is_some() => true,
            Profile::Hqx | Profile::R444 => matches!(cfg.bit_depth, 10 | 12),
            _ => cfg.bit_depth == 8,
        };
        if !ok_depth {
            return Err(Error::Input(format!("{} does not support {}-bit", cfg.profile.name(), cfg.bit_depth)));
        }
        let info = cid_info(cfg.profile.cid()).ok_or_else(|| Error::Input(format!("no compression table for {}", cfg.profile.name())))?;
        let frame_size = cfg.profile.frame_size(cfg.width, cfg.height);
        let header_size = header::header_size_for(if cfg.profile.interlaced() { cfg.height / 2 } else { cfg.height });
        Ok(Encoder { cfg, info, frame_size, header_size, last_q: 8 })
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// Size of every encoded frame in bytes (CBR; both coding units of a field-coded frame).
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// Encode one frame. Its geometry and chroma format must match the configuration; samples
    /// at another depth are rescaled to the coded depth.
    pub fn encode(&mut self, f: &Frame) -> Result<Vec<u8>> {
        let cfg = &self.cfg;
        if f.width != cfg.width || f.height != cfg.height {
            return Err(Error::Input(format!("frame is {}x{}, encoder is {}x{}", f.width, f.height, cfg.width, cfg.height)));
        }
        if f.chroma != cfg.chroma() {
            return Err(Error::Input(format!("{:?} input for {}", f.chroma, cfg.profile.name())));
        }
        // R'G'B' is only defined for 4:4:4 (§6.3, CLF): a 4:2:2 RGB frame is wrong input
        if f.rgb && f.chroma != ChromaFormat::Yuv444 {
            return Err(Error::Input("RGB input requires 4:4:4 chroma".into()));
        }
        if !(8..=16).contains(&f.bit_depth) {
            return Err(Error::Input(format!("unsupported input depth {}", f.bit_depth)));
        }
        let n = f.width as usize * f.height as usize;
        let cn = f.chroma_width() as usize * f.chroma_height() as usize;
        if f.y.len() != n || f.cb.len() != cn || f.cr.len() != cn {
            return Err(Error::Input("plane sizes do not match the frame geometry".into()));
        }
        if !self.cfg.profile.interlaced() {
            return self.encode_unit(f, FieldCode::Frame, self.frame_size);
        }
        // field coding: field 1 (top, even lines) then field 2, each half the frame size
        let unit = self.frame_size / 2;
        let mut out = self.encode_unit(&field_of(f, 0), FieldCode::Field1, unit)?;
        out.extend(self.encode_unit(&field_of(f, 1), FieldCode::Field2, unit)?);
        Ok(out)
    }

    /// Encode `f` (a frame, or one field) as a single coding unit of `unit_size` bytes.
    fn encode_unit(&mut self, f: &Frame, field: FieldCode, unit_size: u32) -> Result<Vec<u8>> {
        let prep = self.prepare(f);
        let vlc = vlc(self.info.vlc);
        let budget_bits = (unit_size as u64 - self.header_size as u64 - 4) * 8;
        let q = self.rate_control(&prep, vlc, budget_bits);
        self.write(&prep, vlc, &q, f, field, unit_size)
    }

    fn mb_layout(&self) -> &'static [(u8, u8, u8)] {
        match self.cfg.chroma() {
            ChromaFormat::Yuv444 => {
                &[(0, 0, 0), (0, 8, 0), (1, 0, 0), (1, 8, 0), (2, 0, 0), (2, 8, 0), (0, 0, 8), (0, 8, 8), (1, 0, 8), (1, 8, 8), (2, 0, 8), (2, 8, 8)]
            }
            _ => &[(0, 0, 0), (0, 8, 0), (1, 0, 0), (2, 0, 0), (0, 0, 8), (0, 8, 8), (1, 0, 8), (2, 0, 8)],
        }
    }

    fn prepare(&self, f: &Frame) -> Prepared {
        let cfg = &self.cfg;
        let nw = f.width.div_ceil(16) as usize;
        let ns = f.height.div_ceil(16) as usize;
        let layout = self.mb_layout();
        let nblk = layout.len();
        let depth = cfg.bit_depth as i32;
        let shift = depth - f.bit_depth as i32;
        let mid = (1i32 << (depth - 1)) as f32;
        let inv_scale = if depth == 12 { 0.25f32 } else { 1.0 };
        let p = self.info.p as f32;
        let w = self.info.weights;
        let cmbw = if cfg.chroma() == ChromaFormat::Yuv444 { 16 } else { 8 };
        // coded channels Ch1/Ch2/Ch3: the Frame's G, B, R planes go out as R, G, B by the standard
        // (§6.3) and as G, B, R in FFmpeg's order; Y'CbCr as Y, Cb, Cr
        let (fw, fh, cw, ch) = (f.width as usize, f.height as usize, f.chroma_width() as usize, f.chroma_height() as usize);
        let planes: [(&[u16], usize, usize, usize); 3] = if f.rgb && !cfg.ffmpeg_compat {
            [(&f.cr, fw, fh, 16), (&f.y, fw, fh, 16), (&f.cb, fw, fh, 16)]
        } else {
            [(&f.y, fw, fh, 16), (&f.cb, cw, ch, cmbw), (&f.cr, cw, ch, cmbw)]
        };
        let sample = |pl: usize, x: usize, y: usize| -> f32 {
            let (d, pw, ph, _) = planes[pl];
            let v = d[y.min(ph - 1) * pw + x.min(pw - 1)] as i32;
            let v = if shift >= 0 { v << shift } else { (v + (1 << (-shift - 1))) >> -shift };
            v.min((1 << depth) - 1) as f32 - mid
        };
        let do_row = |s: usize| -> Vec<[f32; 64]> {
            let mut out = Vec::with_capacity(nw * nblk);
            for mb in 0..nw {
                for &(pl, xo, yo) in layout {
                    let pl = pl as usize;
                    let x0 = mb * planes[pl].3 + xo as usize;
                    let y0 = s * 16 + yo as usize;
                    let mut b = [0f32; 64];
                    for j in 0..8 {
                        for i in 0..8 {
                            b[j * 8 + i] = sample(pl, x0 + i, y0 + j);
                        }
                    }
                    fdct8x8(&mut b);
                    let wt = &w[(pl != 0) as usize];
                    let mut t = [0f32; 64];
                    let dc_max = if depth == 8 { 1023.0 } else { 4095.0 };
                    t[0] = (b[0] * inv_scale).round().clamp(-dc_max - 1.0, dc_max);
                    for r in 1..64 {
                        let z = ZIGZAG[r] as usize;
                        t[r] = b[z] * inv_scale * p / wt[z - 1] as f32;
                    }
                    out.push(t);
                }
            }
            out
        };
        #[cfg(feature = "threads")]
        let rows: Vec<Vec<[f32; 64]>> = {
            use rayon::prelude::*;
            (0..ns).into_par_iter().map(do_row).collect()
        };
        #[cfg(not(feature = "threads"))]
        let rows: Vec<Vec<[f32; 64]>> = (0..ns).map(do_row).collect();
        let t: Vec<[f32; 64]> = rows.into_iter().flatten().collect();
        // DC bits with per-row, per-component prediction
        let v = vlc(self.info.vlc);
        let mut dc_bits = vec![0u32; nw * ns];
        for s in 0..ns {
            let mut pred = [0i32; 3];
            for mb in 0..nw {
                let mut bits = 0;
                for (k, &(pl, _, _)) in layout.iter().enumerate() {
                    let dc = t[(s * nw + mb) * nblk + k][0] as i32;
                    let e = dc - pred[pl as usize];
                    pred[pl as usize] = dc;
                    let eta = dc_eta(e);
                    bits += v.dc_code[eta as usize].1 as u32 + eta;
                }
                dc_bits[s * nw + mb] = bits;
            }
        }
        Prepared { nw, ns, nblk, t, dc_bits }
    }

    /// Per-macroblock qsf values whose coded frame fits `budget_bits`.
    fn rate_control(&mut self, prep: &Prepared, vlc: &VlcSet, budget_bits: u64) -> Vec<u32> {
        let p_bits = if self.cfg.bit_depth == 8 { 4 } else { 6 };
        let max_amp = if p_bits == 4 { 1024 } else { 4096 };
        let nmb = prep.nw * prep.ns;
        let bias = quant_bias(&self.cfg);
        let mb_bits = |mb: usize, q: u32| -> u32 {
            let mut bits = 12 + prep.dc_bits[mb];
            for k in 0..prep.nblk {
                bits += ac_bits(&prep.t[mb * prep.nblk + k], q as f32, vlc, p_bits, max_amp, bias);
            }
            bits
        };
        let frame_bits = |q: u32| -> u64 {
            let row = |s: usize| -> u64 { row_bytes((0..prep.nw).map(|m| mb_bits(s * prep.nw + m, q) as u64).sum()) * 8 };
            #[cfg(feature = "threads")]
            {
                use rayon::prelude::*;
                (0..prep.ns).into_par_iter().map(row).sum()
            }
            #[cfg(not(feature = "threads"))]
            {
                (0..prep.ns).map(row).sum()
            }
        };
        // smallest feasible uniform q (bits are non-increasing in q)
        let (mut lo, mut hi) = (1u32, 2047u32);
        if frame_bits(1) <= budget_bits {
            hi = 1;
        } else {
            // bracket around the previous frame's choice first
            let g = self.last_q.clamp(2, 2047);
            if frame_bits(g) <= budget_bits {
                hi = g;
                let g2 = (g / 2).max(1);
                if g2 > 1 && frame_bits(g2) > budget_bits {
                    lo = g2;
                }
            } else {
                lo = g;
            }
            while hi - lo > 1 {
                let m = (lo + hi) / 2;
                if frame_bits(m) <= budget_bits { hi = m } else { lo = m }
            }
        }
        let q = hi;
        self.last_q = q;
        let mut qs = vec![q; nmb];
        if q > 1 {
            // refine: one finer step per macroblock while the budget allows
            let mut row_bits: Vec<u64> = (0..prep.ns).map(|s| (0..prep.nw).map(|m| mb_bits(s * prep.nw + m, q) as u64).sum()).collect();
            let mut total: u64 = row_bits.iter().map(|&b| row_bytes(b) * 8).sum();
            let cur: Vec<u32> = (0..nmb).map(|mb| mb_bits(mb, q)).collect();
            let finer: Vec<u32> = (0..nmb).map(|mb| mb_bits(mb, q - 1)).collect();
            // spread the refinement evenly over the frame: visit macroblocks in a strided order
            let stride = 7919usize;
            let mut idx = 0usize;
            for _ in 0..nmb {
                idx = (idx + stride) % nmb;
                let s = idx / prep.nw;
                let extra = finer[idx] as u64 - cur[idx].min(finer[idx]) as u64;
                let nb = row_bits[s] + extra;
                let nt = total - row_bytes(row_bits[s]) * 8 + row_bytes(nb) * 8;
                if nt <= budget_bits {
                    row_bits[s] = nb;
                    total = nt;
                    qs[idx] = q - 1;
                }
            }
        }
        qs
    }

    fn write(&self, prep: &Prepared, vlc: &VlcSet, qs: &[u32], f: &Frame, field: FieldCode, unit_size: u32) -> Result<Vec<u8>> {
        let rgb = f.rgb;
        let cfg = &self.cfg;
        let p_bits = if cfg.bit_depth == 8 { 4 } else { 6 };
        let max_amp = if p_bits == 4 { 1024 } else { 4096 };
        let layout = self.mb_layout();
        // 4:4:4 signalling. The standard (§7.2, §7.3.1.1): RGB is CLF = 1, Y'CbCr is CLF = 0, every
        // macroblock ACF = 0. FFmpeg (compat; see `FrameHeader::rgb_planes`): RGB is CLF = 0 with
        // ACF = 0, Y'CbCr is CLF = 1 with ACF = 1 on every macroblock.
        let is444 = cfg.chroma() == ChromaFormat::Yuv444;
        let (clf, acf) = if cfg.ffmpeg_compat { (is444 && !rgb, is444 && !rgb) } else { (is444 && rgb, false) };
        let bias = quant_bias(cfg);
        let write_row = |s: usize| -> Vec<u8> {
            let mut w = Writer::new();
            let mut pred = [0i32; 3];
            for mb in 0..prep.nw {
                let m = s * prep.nw + mb;
                let q = qs[m];
                w.put(q, 11);
                w.put(acf as u32, 1);
                for (k, &(pl, _, _)) in layout.iter().enumerate() {
                    let t = &prep.t[m * prep.nblk + k];
                    let dc = t[0] as i32;
                    let e = dc - pred[pl as usize];
                    pred[pl as usize] = dc;
                    let eta = dc_eta(e);
                    let (c, l) = vlc.dc_code[eta as usize];
                    w.put(c as u32, l as u32);
                    if eta > 0 {
                        let rho = if e > 0 { e } else { e + (1 << eta) - 1 };
                        w.put(rho as u32, eta);
                    }
                    write_ac(&mut w, t, q as f32, vlc, p_bits, max_amp, bias);
                }
            }
            w.finish_aligned(4)
        };
        #[cfg(feature = "threads")]
        let rows: Vec<Vec<u8>> = {
            use rayon::prelude::*;
            (0..prep.ns).into_par_iter().map(write_row).collect()
        };
        #[cfg(not(feature = "threads"))]
        let rows: Vec<Vec<u8>> = (0..prep.ns).map(write_row).collect();

        let h = FrameHeader {
            header_size: self.header_size,
            // header version 1 for the HD profile IDs, 3 for the resolution-independent ones
            version: if cfg.profile.hd_raster().is_some() { 1 } else { 3 },
            cid: self.info.cid,
            vbr: false,
            field,
            macf: false,
            crc: false,
            alpha: false,
            lossless_alpha: false,
            premultiplied_alpha: false,
            width: cfg.width,
            lines: f.height,
            par: cfg.par,
            bit_depth: cfg.bit_depth,
            interlaced: field != FieldCode::Frame,
            frame_encoding: field == FieldCode::Frame,
            chroma: cfg.chroma(),
            rgb: clf,
            color_volume: cfg.color_volume,
            timecode: None,
            mb_rows: prep.ns as u32,
        };
        let mut out = header::write(&h);
        out.reserve((unit_size as usize).saturating_sub(out.len()));
        let mut off = 0u32;
        for (s, r) in rows.iter().enumerate() {
            let o = SCAN_INDEX_OFFSET + 4 * s;
            out[o..o + 4].copy_from_slice(&off.to_be_bytes());
            off += r.len() as u32;
        }
        for r in rows {
            out.extend_from_slice(&r);
        }
        let end = unit_size as usize - 4;
        if out.len() > end {
            return Err(Error::Input("rate control overflow".into()));
        }
        out.resize(end, 0);
        out.extend_from_slice(&EOF_SIGNATURE);
        Ok(out)
    }
}

#[inline]
/// Field `parity` (0 = top, even lines; 1 = bottom) of `f` as a frame of half the height.
fn field_of(f: &Frame, parity: usize) -> Frame {
    let lines = |d: &[u16], w: usize| -> Vec<u16> { d.chunks_exact(w).skip(parity).step_by(2).flatten().copied().collect() };
    let (w, cw) = (f.width as usize, f.chroma_width() as usize);
    Frame {
        width: f.width,
        height: f.height / 2,
        chroma: f.chroma,
        rgb: f.rgb,
        bit_depth: f.bit_depth,
        y: lines(&f.y, w),
        cb: lines(&f.cb, cw),
        cr: lines(&f.cr, cw),
        alpha: None,
        interlace: f.interlace,
        color_volume: f.color_volume,
        cid: f.cid,
        par: f.par,
    }
}

fn row_bytes(bits: u64) -> u64 {
    bits.div_ceil(32) * 4
}

#[inline]
fn dc_eta(e: i32) -> u32 {
    if e == 0 { 0 } else { 32 - e.unsigned_abs().leading_zeros() }
}

/// Subtracted from `|t|/q` before truncation: 0 gives equation 8.2 (`⌊p·|X| / (qsf·W)⌋`), whose
/// interval equation (8.1) reconstructs near its midpoint; ¼ gives the amplitude nearest to
/// FFmpeg's `(A + 3/4)·step` reconstruction.
fn quant_bias(cfg: &EncoderConfig) -> f32 {
    if cfg.ffmpeg_compat { 0.25 } else { 0.0 }
}

/// Quantised amplitude (≥ 1) of a pre-divided magnitude `|t| ≥ q`.
#[inline(always)]
fn quant(t: f32, q: f32, max_amp: u32, bias: f32) -> u32 {
    ((t.abs() / q - bias) as u32).clamp(1, max_amp)
}

/// Bits of a block's AC coefficients plus EOB at quantiser `q`.
#[inline]
fn ac_bits(t: &[f32; 64], q: f32, v: &VlcSet, p_bits: u32, max_amp: u32, bias: f32) -> u32 {
    let mut bits = v.eob.1 as u32;
    let mut run = 0usize;
    for &x in &t[1..] {
        if x.abs() < q {
            run += 1;
            continue;
        }
        let a = quant(x, q, max_amp, bias);
        let p = (a - 1) >> 6;
        let base = ((a - 1) & 63) as usize;
        let frun = (run > 0) as usize;
        let findex = (p > 0) as usize;
        bits += v.amp_code[frun][findex][base].1 as u32 + 1;
        if findex == 1 {
            bits += p_bits;
        }
        if frun == 1 {
            bits += v.run_code[run].1 as u32;
        }
        run = 0;
    }
    bits
}

fn write_ac(w: &mut Writer, t: &[f32; 64], q: f32, v: &VlcSet, p_bits: u32, max_amp: u32, bias: f32) {
    let mut run = 0usize;
    for &x in &t[1..] {
        if x.abs() < q {
            run += 1;
            continue;
        }
        let a = quant(x, q, max_amp, bias);
        let p = (a - 1) >> 6;
        let base = ((a - 1) & 63) as usize;
        let frun = (run > 0) as usize;
        let findex = (p > 0) as usize;
        let (c, l) = v.amp_code[frun][findex][base];
        w.put(c as u32, l as u32);
        w.put((x < 0.0) as u32, 1);
        if findex == 1 {
            w.put(p, p_bits);
        }
        if frun == 1 {
            let (c, l) = v.run_code[run];
            w.put(c as u32, l as u32);
        }
        run = 0;
    }
    w.put(v.eob.0 as u32, v.eob.1 as u32);
}
