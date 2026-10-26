//! VP9 encoder (profile 0: 8-bit 4:2:0), clean-room from the VP9 Bitstream & Decoding Process
//! Specification: every syntax element is written the way `crate::header` and `crate::tile`
//! read it.
//!
//! The stream is deliberately simple, so that the decoder never adapts a probability:
//! key frames and inter frames predicting from the previous frame only (LAST), the default
//! probabilities throughout (`refresh_frame_context` 0, `frame_parallel_decoding_mode` 1),
//! the largest transform each block allows (`ALLOW_32X32`), quarter-sample motion vectors, the
//! regular 8-tap filter, one tile row and as few tile columns as the frame width allows. Blocks
//! are 8×8 to 64×64 (partitions NONE or SPLIT), chosen by rate-distortion search.
//!
//! The encoder decodes every frame it writes with its own [`Decoder`] and predicts the next
//! frame from that (loop filtered) picture, so encoder and decoder never drift apart.

mod boolenc;
mod fdct;
mod tile;

use crate::frame::{Frame, MiInfo};
use crate::probs::FrameContext;
use crate::spec_tables::{AC_QLOOKUP, DC_QLOOKUP};
use crate::tables::{MAX_TILE_WIDTH_B64, MIN_TILE_WIDTH_B64};
use crate::{Decoder, Error, Picture, Result};
use boolenc::BoolEncoder;
use cfa4_bitstream::BitWriter;
use tile::{FrameCtx, TileEncoder};

/// How many bits the encoder spends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    /// A constant quantizer index, 1 (best) to 255 (smallest).
    Quantizer(u8),
    /// An average bit rate: the quantizer follows the bits spent.
    Bitrate { bits_per_second: u64, fps_num: u32, fps_den: u32 },
}

/// Encoder settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub rate: Rate,
    /// A key frame at least every this many frames (0 = only the first).
    pub keyframe_interval: u32,
    /// 0 (thorough) to 2 (fast).
    pub speed: u8,
    /// VP9 colour space code (1 = BT.601, 2 = BT.709, 5 = BT.2020).
    pub color_space: u8,
    pub full_range: bool,
}

impl Config {
    pub fn new(width: u32, height: u32) -> Config {
        Config { width, height, rate: Rate::Quantizer(60), keyframe_interval: 120, speed: 1, color_space: 2, full_range: false }
    }
}

/// One encoded frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub data: Vec<u8>,
    pub key: bool,
}

/// One 8-bit 4:2:0 input picture: planes with their row strides (in samples).
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub uv_stride: usize,
}

/// Samples of the motion search reference beyond each edge of the frame.
const REF_PAD: usize = 160;

/// The rate controller of [`Rate::Bitrate`].
#[derive(Clone, Copy, Debug)]
struct RateState {
    /// Bits per frame.
    target: f64,
    /// The current quantizer index (fractional, so that small corrections accumulate).
    q: f64,
    /// Bits spent beyond the target so far (negative: below), bounded.
    debt: f64,
}

pub struct Encoder {
    cfg: Config,
    mi_cols: usize,
    mi_rows: usize,
    sb_cols: usize,
    sb_rows: usize,
    frames: u64,
    since_key: u32,
    rc: Option<RateState>,
    dec: Decoder,
    fc: FrameContext,
    /// Source planes padded to whole superblocks.
    src: [Vec<u16>; 3],
    ref_luma: Vec<u16>,
    last_picture: Option<Picture>,
}

/// get_tile_offset (6.4.1).
fn tile_offset(i: usize, mis: usize, log2: u32) -> usize {
    let sbs = (mis + 7) >> 3;
    (((i * sbs) >> log2) << 3).min(mis)
}

/// The smallest number of tile columns (log2) a frame of `sb_cols` superblocks allows, and the
/// largest.
fn tile_cols_range(sb_cols: u32) -> (u32, u32) {
    let mut min_log2 = 0;
    while (MAX_TILE_WIDTH_B64 << min_log2) < sb_cols {
        min_log2 += 1;
    }
    let mut max_log2 = 1;
    while (sb_cols >> max_log2) >= MIN_TILE_WIDTH_B64 {
        max_log2 += 1;
    }
    (min_log2, max_log2 - 1)
}

fn q_lookup(table: &[i32], q_idx: u8) -> i32 {
    table.get(usize::from(q_idx)).copied().unwrap_or(4)
}

/// The compressed header (6.3): the transform mode, and no probability update.
fn write_compressed_header(key: bool) -> Vec<u8> {
    let mut bw = BoolEncoder::new();
    // tx_mode: ALLOW_32X32 (tx_mode 3, then the TX_MODE_SELECT bit 0)
    bw.write_literal(3, 2);
    bw.write_literal(0, 1);
    // coefficient probabilities of transform sizes 4x4 .. 32x32: no update
    for _ in 0..4 {
        bw.write_literal(0, 1);
    }
    let no_update = |bw: &mut BoolEncoder, n: usize| {
        for _ in 0..n {
            bw.write_bool(false, 252);
        }
    };
    // skip probabilities
    no_update(&mut bw, 3);
    if !key {
        // inter_mode, is_inter, single_ref, y_mode, partition
        no_update(&mut bw, 7 * 3 + 4 + 5 * 2 + 4 * 9 + 16 * 3);
        // motion vector probabilities: joints, then per component sign, classes, class0 bit,
        // bits; then per component class0_fr and fr (no high precision)
        no_update(&mut bw, 3 + 2 * (1 + 10 + 1 + 10) + 2 * (2 * 3 + 3));
    }
    bw.finish()
}

/// Loop filter strength for a quantizer (a fit of filter level against the AC quantizer).
fn filter_level(ac_q: i32, key: bool) -> u32 {
    let guess = (i64::from(ac_q) * 20_723 + 1_015_158 + (1 << 17)) >> 18;
    let guess = if key { guess - 4 } else { guess };
    guess.clamp(0, 63) as u32
}

impl Encoder {
    pub fn new(cfg: Config) -> Result<Encoder> {
        if cfg.width == 0 || cfg.height == 0 || cfg.width > 16384 || cfg.height > 16384 {
            return Err(Error::Unsupported(format!("frame size {}x{}", cfg.width, cfg.height)));
        }
        if cfg.color_space == 7 {
            return Err(Error::Unsupported("RGB needs profile 1".into()));
        }
        let mi_cols = (cfg.width as usize).div_ceil(8);
        let mi_rows = (cfg.height as usize).div_ceil(8);
        let (sb_cols, sb_rows) = (mi_cols.div_ceil(8), mi_rows.div_ceil(8));
        let rc = match cfg.rate {
            Rate::Quantizer(_) => None,
            Rate::Bitrate { bits_per_second, fps_num, fps_den } => {
                if fps_num == 0 || fps_den == 0 || bits_per_second == 0 {
                    return Err(Error::Unsupported("bit rate or frame rate of zero".into()));
                }
                let target = bits_per_second as f64 * f64::from(fps_den) / f64::from(fps_num);
                // a first quantizer from the bits per sample
                let bps = target / (f64::from(cfg.width) * f64::from(cfg.height));
                let q = (110.0 - 40.0 * bps.max(1e-4).log2()).clamp(8.0, 240.0);
                Some(RateState { target, q, debt: 0.0 })
            }
        };
        let (w, h) = (sb_cols * 64, sb_rows * 64);
        Ok(Encoder {
            mi_cols,
            mi_rows,
            sb_cols,
            sb_rows,
            frames: 0,
            since_key: 0,
            rc,
            dec: Decoder::with_threads(1),
            fc: FrameContext::default(),
            src: [vec![0; w * h], vec![0; (w / 2) * (h / 2)], vec![0; (w / 2) * (h / 2)]],
            ref_luma: Vec::new(),
            last_picture: None,
            cfg,
        })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The picture the decoder shows for the last encoded frame.
    pub fn last_picture(&self) -> Option<&Picture> {
        self.last_picture.as_ref()
    }

    fn q_idx(&self) -> u8 {
        match (self.cfg.rate, &self.rc) {
            (Rate::Quantizer(q), _) => q.max(1),
            (_, Some(rc)) => rc.q.round().clamp(1.0, 255.0) as u8,
            _ => 60,
        }
    }

    /// Copy the input into the superblock-padded source planes, replicating the edges.
    fn load_source(&mut self, input: &Input) -> Result<()> {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let need = |stride: usize, pw: usize, ph: usize| stride.checked_mul(ph - 1).and_then(|v| v.checked_add(pw));
        let ok = |len: usize, stride: usize, pw: usize, ph: usize| stride >= pw && need(stride, pw, ph).is_some_and(|n| n <= len);
        if !ok(input.y.len(), input.y_stride, w, h) || !ok(input.u.len(), input.uv_stride, cw, ch) || !ok(input.v.len(), input.uv_stride, cw, ch) {
            return Err(Error::Invalid("input planes smaller than the frame".into()));
        }
        let planes = [(input.y, input.y_stride, w, h), (input.u, input.uv_stride, cw, ch), (input.v, input.uv_stride, cw, ch)];
        for (p, &(data, stride, pw, ph)) in planes.iter().enumerate() {
            let dw = (self.sb_cols * 64) >> (p > 0) as usize;
            let dh = (self.sb_rows * 64) >> (p > 0) as usize;
            let dst = &mut self.src[p];
            for y in 0..dh {
                let sy = y.min(ph - 1);
                let row = data.get(sy * stride..sy * stride + pw).unwrap_or_default();
                let Some(out) = dst.get_mut(y * dw..y * dw + dw) else { continue };
                let last = row.last().copied().unwrap_or(128);
                for (x, o) in out.iter_mut().enumerate() {
                    *o = u16::from(row.get(x).copied().unwrap_or(last));
                }
            }
        }
        Ok(())
    }

    /// The luma of the reference, edge-extended by `REF_PAD` on every side of the padded frame.
    fn load_reference(&mut self, f: &Frame) {
        let (w, h) = (f.info.width as usize, f.info.height as usize);
        let rw = self.sb_cols * 64 + 2 * REF_PAD;
        let rh = self.sb_rows * 64 + 2 * REF_PAD;
        self.ref_luma.resize(rw * rh, 0);
        for y in 0..rh {
            let sy = y.saturating_sub(REF_PAD).min(h.saturating_sub(1));
            let row = f.row(0, sy);
            let first = row.first().copied().unwrap_or(128);
            let last = row.get(w.saturating_sub(1)).copied().unwrap_or(first);
            let Some(out) = self.ref_luma.get_mut(y * rw..y * rw + rw) else { continue };
            for (x, o) in out.iter_mut().enumerate() {
                *o = if x < REF_PAD {
                    first
                } else if x - REF_PAD >= w {
                    last
                } else {
                    row.get(x - REF_PAD).copied().unwrap_or(last)
                };
            }
        }
    }

    /// Encode one frame (a key frame when `force_key`, at the key frame interval, or first).
    pub fn encode(&mut self, input: &Input, force_key: bool) -> Result<Packet> {
        self.load_source(input)?;
        let (last, prev_mi) = self.dec.last_reference();
        let interval_due = self.cfg.keyframe_interval > 0 && self.since_key >= self.cfg.keyframe_interval;
        let key = force_key || interval_due || last.is_none();
        let last = if key { None } else { last };
        if let Some(f) = &last {
            self.load_reference(f);
        }
        let q_idx = self.q_idx();
        let (dc, ac) = (q_lookup(&DC_QLOOKUP, q_idx), q_lookup(&AC_QLOOKUP, q_idx));
        let (min_log2, _) = tile_cols_range(self.sb_cols as u32);
        let tile_cols_log2 = min_log2;
        let lf_level = filter_level(ac, key);
        let compressed = write_compressed_header(key);
        let header_size = u16::try_from(compressed.len()).map_err(|_| Error::Invalid("compressed header too large".into()))?;
        let mut frame = self.write_uncompressed_header(key, q_idx, lf_level, tile_cols_log2, header_size);
        frame.extend_from_slice(&compressed);
        // tiles
        let src_stride = [self.sb_cols * 64, self.sb_cols * 32, self.sb_cols * 32];
        let ctx = FrameCtx {
            key,
            mi_cols: self.mi_cols,
            mi_rows: self.mi_rows,
            sb_cols: self.sb_cols,
            sb_rows: self.sb_rows,
            fc: &self.fc,
            q: [[dc, ac], [dc, ac]],
            last: last.as_deref(),
            prev_mi: if key { None } else { prev_mi.as_deref() },
            src: [&self.src[0], &self.src[1], &self.src[2]],
            src_stride,
            ref_luma: &self.ref_luma,
            ref_stride: self.sb_cols * 64 + 2 * REF_PAD,
            ref_pad: REF_PAD,
            lambda: f64::from(ac) / 8.0,
            speed: self.cfg.speed.min(2),
        };
        let mut mi = vec![MiInfo::default(); self.mi_rows * self.mi_cols];
        let tile_cols = 1usize << tile_cols_log2;
        for t in 0..tile_cols {
            let (s, e) = (tile_offset(t, self.mi_cols, tile_cols_log2), tile_offset(t + 1, self.mi_cols, tile_cols_log2));
            let mut bw = BoolEncoder::new();
            if e > s {
                TileEncoder::new(&ctx, s, e).encode(&mut bw, &mut mi);
            }
            let data = bw.finish();
            if t + 1 < tile_cols {
                let len = u32::try_from(data.len()).map_err(|_| Error::Invalid("tile too large".into()))?;
                frame.extend_from_slice(&len.to_be_bytes());
            }
            frame.extend_from_slice(&data);
        }
        drop(last);
        drop(prev_mi);
        // the reference for the next frame is what a decoder makes of this one
        let pts = i64::try_from(self.frames).unwrap_or(i64::MAX);
        let mut pics = self.dec.decode(&frame, pts)?;
        self.last_picture = pics.pop();
        self.frames += 1;
        self.since_key = if key { 1 } else { self.since_key.saturating_add(1) };
        if let Some(rc) = &mut self.rc {
            let bits = frame.len() as f64 * 8.0;
            // a key frame may spend more; what it overspends is paid back over the next frames
            let target = if key { rc.target * 4.0 } else { rc.target };
            rc.debt = (rc.debt + bits - target).clamp(-rc.target * 30.0, rc.target * 30.0);
            let err = (bits / target).max(1e-3).log2() * 6.0 + rc.debt / (rc.target * 10.0) * 6.0;
            rc.q = (rc.q + err.clamp(-12.0, 12.0)).clamp(1.0, 255.0);
        }
        Ok(Packet { data: frame, key })
    }

    /// The uncompressed header (6.2).
    fn write_uncompressed_header(&self, key: bool, q_idx: u8, lf_level: u32, tile_cols_log2: u32, header_size: u16) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(2, 2); // frame_marker
        w.write_bits(0, 2); // profile 0
        w.write_bit(false); // show_existing_frame
        w.write_bit(!key); // frame_type
        w.write_bit(true); // show_frame
        w.write_bit(false); // error_resilient_mode
        if key {
            w.write_bits(0x49_83_42, 24);
            w.write_bits(u32::from(self.cfg.color_space), 3);
            w.write_bit(self.cfg.full_range);
            w.write_bits(self.cfg.width - 1, 16);
            w.write_bits(self.cfg.height - 1, 16);
            w.write_bit(false); // render_and_frame_size_different
        } else {
            w.write_bits(0, 2); // reset_frame_context
            w.write_bits(1, 8); // refresh_frame_flags: slot 0
            for i in 0..3 {
                w.write_bits(i, 3); // ref_frame_idx
                w.write_bit(false); // sign bias
            }
            w.write_bit(true); // found_ref: the size of LAST
            w.write_bit(false); // render_and_frame_size_different
            w.write_bit(false); // allow_high_precision_mv
            w.write_bit(false); // is_filter_switchable
            w.write_bits(1, 2); // raw_interpolation_filter: EIGHTTAP
        }
        w.write_bit(false); // refresh_frame_context
        w.write_bit(true); // frame_parallel_decoding_mode
        w.write_bits(0, 2); // frame_context_idx
        // loop_filter_params
        w.write_bits(lf_level, 6);
        w.write_bits(0, 3); // sharpness
        w.write_bit(false); // loop_filter_delta_enabled
        // quantization_params
        w.write_bits(u32::from(q_idx), 8);
        for _ in 0..3 {
            w.write_bit(false); // delta_coded
        }
        w.write_bit(false); // segmentation_enabled
        // tile_info
        let (min_log2, max_log2) = tile_cols_range(self.sb_cols as u32);
        for _ in min_log2..tile_cols_log2 {
            w.write_bit(true);
        }
        if tile_cols_log2 < max_log2 {
            w.write_bit(false);
        }
        w.write_bit(false); // tile_rows_log2
        w.write_bits(u32::from(header_size), 16);
        w.finish()
    }
}

#[cfg(test)]
mod tests;
