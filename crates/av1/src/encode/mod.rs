//! AV1 encoder.
//!
//! The encoder is the decoder run with a policy: the tile syntax code (`tile.rs`,
//! `inter_info.rs`) asks, at every symbol it would read, for the value to code (`Elem`), and an
//! arithmetic encoder codes that value with the same CDF and context the decoder uses. Prediction,
//! reconstruction, context modelling, the motion vector stack, loop filters and reference updates
//! are then the decoder's own, so the encoder's reconstruction is the decoder's output by
//! construction.
//!
//! Coding tools used: 64x64 superblocks split down to 8x8 (PARTITION_NONE / SPLIT), the
//! largest transform of each block (DCT_DCT for luma, the mode's type for intra chroma), the 13
//! intra modes without angle deltas, single-reference inter prediction from the previous frame
//! (NEAREST / NEAR / GLOBAL / NEW motion vectors, quarter or eighth sample), deblocking with a
//! searched level, uniform tiles coded in parallel.

mod fwd;
mod obu;
mod policy;
mod rc;
mod search;

pub(crate) use policy::{Elem, FrameEnc, Source, TileEnc};

use crate::cdf::Cdfs;
use crate::decoder::Decoder;
use crate::frame::Plane;
use crate::par::Pool;
use crate::spec_tables::*;
use crate::state::{FrameShared, FrameState, TileState};
use crate::symbol::SymbolEncoder;
use crate::tile::TileDecoder;

use obu::{FrameParams, SeqParams};

/// Encoder errors.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("invalid dimensions {0}x{1} (1..=16384 x 1..=8704)")]
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
    /// Constant quality on the H.264 / HEVC QP scale (0–51, lower is better).
    Crf(f32),
    /// Constant bitrate target (1-pass ABR with a 1-second VBV at `kbps`).
    Cbr { kbps: u32 },
    /// Variable bitrate: average `target_kbps`, VBV-limited to `max_kbps`.
    Vbr { target_kbps: u32, max_kbps: u32 },
    /// Constant quantizer index (base_q_idx 1–255; key frames use a finer one).
    QIndex(u8),
}

/// Colour description (ITU-T H.273 code points).
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

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// 8 or 10 (Main profile, 4:2:0).
    pub bit_depth: u8,
    pub preset: Preset,
    pub rate: RateControl,
    /// Maximum distance between key frames (0: only the first frame).
    pub keyint: u32,
    pub color: ColorConfig,
    /// HDR static metadata sent with every key frame: the 24-byte SMPTE ST 2086 mastering
    /// display payload in the H.265 SEI layout (green, blue, red primaries and white point in
    /// 0.00002 units, max / min luminance in 0.0001 cd/m²) …
    pub mastering_display: Option<[u8; 24]>,
    /// … and (MaxCLL, MaxFALL) content light level.
    pub content_light: Option<(u16, u16)>,
    /// Tiles per frame, coded in parallel (rounded to a power of two). 0 picks one per worker
    /// thread (at most 8). The output depends on the tile count, never on the thread count.
    pub tiles: u32,
    /// Worker threads (0: all available cores).
    pub threads: usize,
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
            color: ColorConfig::default(),
            mastering_display: None,
            content_light: None,
            tiles: 0,
            threads: 0,
        }
    }
}

/// A planar 4:2:0 input picture of the configured size: 8-bit samples (scaled up for 10-bit
/// coding), or samples at the configured bit depth in 16-bit words.
#[derive(Clone, Copy, Debug)]
pub enum YuvFrame<'a> {
    U8 { y: &'a [u8], u: &'a [u8], v: &'a [u8], y_stride: usize, uv_stride: usize },
    U16 { y: &'a [u16], u: &'a [u16], v: &'a [u16], y_stride: usize, uv_stride: usize },
}

/// One temporal unit (a shown frame).
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    pub pts: i64,
    pub keyframe: bool,
    /// base_q_idx of the frame.
    pub qindex: u8,
}

/// What a frame's tiles are coded from.
pub(crate) struct FrameJobEnc {
    pub src: Source,
    pub fe: FrameEnc,
}

/// AV1 encoder.
pub struct Encoder {
    cfg: EncoderConfig,
    seq: SeqParams,
    seq_obu: Vec<u8>,
    metadata: Vec<u8>,
    dec: Decoder,
    rc: rc::RateControl,
    prev_low: Option<rc::LowRes>,
    frames: u64,
    since_key: u64,
    tile_cols_log2: u32,
    tile_rows_log2: u32,
}

fn internal(e: impl std::fmt::Display) -> EncodeError {
    EncodeError::Internal(e.to_string())
}

fn default_threads() -> usize {
    #[cfg(all(feature = "threads", not(target_arch = "wasm32")))]
    {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16)
    }
    #[cfg(not(all(feature = "threads", not(target_arch = "wasm32"))))]
    {
        1
    }
}

/// qindex whose quantizer step matches the H.264 / HEVC step of `qp` (8-bit scale).
fn qp_to_qindex(qp: f64) -> u8 {
    // H.264 Qstep = 0.625 * 2^(QP / 6); AV1's orthonormal step is ac_q / 8.
    let step = 0.625 * 2f64.powf(qp / 6.0) * 8.0;
    let mut best = (f64::MAX, 1u8);
    for (q, &a) in AC_QLOOKUP[0].iter().enumerate().skip(1) {
        let d = ((a as f64).ln() - step.ln()).abs();
        if d < best.0 {
            best = (d, q as u8);
        }
    }
    best.1
}

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Self, EncodeError> {
        let (w, h) = (cfg.width, cfg.height);
        if w == 0 || h == 0 || w > 16384 || h > 8704 {
            return Err(EncodeError::InvalidDimensions(w, h));
        }
        if cfg.fps_num == 0 || cfg.fps_den == 0 {
            return Err(EncodeError::InvalidFrameRate(cfg.fps_num, cfg.fps_den));
        }
        if cfg.bit_depth != 8 && cfg.bit_depth != 10 {
            return Err(EncodeError::InvalidConfig(format!("bit depth {} (8 or 10)", cfg.bit_depth)));
        }
        let c = cfg.color;
        if c.primaries == 1 && c.transfer == 13 && c.matrix == 0 {
            return Err(EncodeError::InvalidConfig("sRGB (identity matrix) needs 4:4:4".into()));
        }
        let fps = cfg.fps_num as f64 / cfg.fps_den as f64;
        let seq = SeqParams {
            width: w,
            height: h,
            bit_depth: cfg.bit_depth,
            subsampling_x: 1,
            subsampling_y: 1,
            mono: false,
            primaries: c.primaries,
            transfer: c.transfer,
            matrix: c.matrix,
            full_range: c.full_range,
            level: obu::level_for(w, h, fps),
            enable_cdef: true,
        };
        let mut seq_obu = Vec::new();
        obu::obu(obu::OBU_SEQUENCE_HEADER, &obu::sequence_header(&seq), &mut seq_obu);
        let (kind, vbv) = match cfg.rate {
            RateControl::QIndex(q) => (rc::RcKind::Qp(q.max(1)), (None, None)),
            RateControl::Crf(c) => (rc::RcKind::Crf(if c.is_finite() { c.clamp(0.0, 51.0) } else { 28.0 }), (None, None)),
            RateControl::Cbr { kbps } => (rc::RcKind::Abr { kbps: kbps.max(1) }, (Some(kbps.max(1)), Some(kbps.max(1)))),
            RateControl::Vbr { target_kbps, max_kbps } => {
                let max = max_kbps.max(target_kbps).max(1);
                (rc::RcKind::Abr { kbps: target_kbps.max(1) }, (Some(max), Some(max.saturating_mul(2))))
            }
        };
        let threads = if cfg.threads == 0 { default_threads() } else { cfg.threads };
        let tiles = if cfg.tiles > 0 { cfg.tiles as usize } else { threads.min(8) };
        let (cols_log2, rows_log2) = tile_layout(w, h, tiles);
        Ok(Encoder {
            metadata: hdr_metadata(cfg.mastering_display.as_ref(), cfg.content_light),
            seq,
            seq_obu,
            dec: Decoder::for_encoder(threads),
            rc: rc::RateControl::new(kind, fps, vbv.0, vbv.1),
            prev_low: None,
            frames: 0,
            since_key: 0,
            tile_cols_log2: cols_log2,
            tile_rows_log2: rows_log2,
            cfg,
        })
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// The sequence header OBU (sent with every key frame).
    pub fn sequence_header_obu(&self) -> Vec<u8> {
        self.seq_obu.clone()
    }

    /// AV1CodecConfigurationRecord (AV1-ISOBMFF 2.3) for an `av1C` box.
    pub fn av1c(&self) -> Vec<u8> {
        let s = &self.seq;
        let mut out = vec![0x81, (s.profile() << 5) | (s.level & 31)];
        let ss = (s.subsampling_x << 3) | (s.subsampling_y << 2);
        out.push(((s.bit_depth > 8) as u8) << 6 | ((s.bit_depth == 12) as u8) << 5 | (s.mono as u8) << 4 | ss);
        out.push(0);
        out.extend_from_slice(&self.seq_obu);
        out
    }

    /// Encode one picture; returns its temporal unit (frames are coded in display order, so each
    /// call returns one packet).
    pub fn encode(&mut self, frame: &YuvFrame, pts: i64) -> Result<Vec<Packet>, EncodeError> {
        let src = self.source(frame)?;
        let key = self.frames == 0 || (self.cfg.keyint > 0 && self.since_key >= self.cfg.keyint as u64);
        if key {
            self.since_key = 0;
        }
        let bd = self.cfg.bit_depth as u32;
        let low = rc::LowRes::new(&src.planes[0].data, src.planes[0].stride, src.height[0].min(src.planes[0].rows), bd);
        let cplx = rc::complexity(&low, self.prev_low.as_ref(), key);
        let kind = if key { rc::Kind::I } else { rc::Kind::P };
        let qp = self.rc.frame_qp(kind, cplx);
        let qindex = match self.cfg.rate {
            RateControl::QIndex(q) => {
                if key {
                    q.max(1).saturating_sub(q / 8).max(1)
                } else {
                    q.max(1)
                }
            }
            _ => qp_to_qindex(qp),
        };
        let bd_idx = ((bd - 8) >> 1) as usize;
        let ac_q = AC_QLOOKUP[bd_idx.min(2)][qindex as usize] as f64;
        let (effort, range) = match self.cfg.preset {
            Preset::Speed => (0, 16),
            Preset::Balanced => (1, 32),
            Preset::Quality => (2, 64),
        };
        let fe = FrameEnc {
            key,
            lambda: 0.0375 * ac_q,
            lambda_sse: 0.0015 * ac_q * ac_q,
            split_bits: 2.0,
            rdoq: true,
            effort,
            range,
            round_intra: 0.36,
            round_inter: 0.22,
        };
        let mut fp = FrameParams {
            key,
            order_hint: (self.since_key % (1 << obu::ORDER_HINT_BITS)) as u32,
            refresh_frame_flags: if key { 0xFF } else { 0x01 },
            base_q_idx: qindex,
            lf: [0; 4],
            cdef: None,
            tile_cols_log2: self.tile_cols_log2,
            tile_rows_log2: self.tile_rows_log2,
            context_update_tile_id: 0,
            allow_high_precision_mv: qindex < 100,
        };
        let mut headers = Vec::new();
        if key {
            headers.extend_from_slice(&self.seq_obu);
        }
        obu::obu(obu::OBU_FRAME_HEADER, &obu::frame_header(&self.seq, &fp), &mut headers);
        let job = FrameJobEnc { src, fe };
        let coded = self.dec.encode_frame(&headers, &job).map_err(internal)?;
        fp.lf = coded.filters.lf;
        fp.cdef = coded.filters.cdef;
        // the frame OBU: the header (with the chosen loop filter levels), then the tile group
        let mut frame = obu::frame_header(&self.seq, &fp);
        let n = coded.tiles.len();
        if n > 1 {
            frame.push(0); // tile_start_and_end_present_flag = 0, byte_alignment( )
        }
        for (i, t) in coded.tiles.iter().enumerate() {
            if i + 1 < n {
                let size = t.len().checked_sub(1).ok_or_else(|| internal("empty tile"))? as u32;
                frame.extend_from_slice(&size.to_le_bytes());
            }
            frame.extend_from_slice(t);
        }
        let mut data = vec![(obu::OBU_TEMPORAL_DELIMITER << 3) | 2, 0];
        if key {
            data.extend_from_slice(&self.seq_obu);
            data.extend_from_slice(&self.metadata);
        }
        obu::obu(obu::OBU_FRAME, &frame, &mut data);
        self.rc.update(kind, qp, cplx, data.len() as u64 * 8);
        self.prev_low = Some(low);
        self.frames += 1;
        self.since_key += 1;
        Ok(vec![Packet { data, pts, keyframe: key, qindex }])
    }

    /// End of stream: nothing is held back.
    pub fn flush(&mut self) -> Result<Vec<Packet>, EncodeError> {
        Ok(Vec::new())
    }

    /// The input in the frame buffer geometry with edge replication, at the coded bit depth.
    fn source(&self, f: &YuvFrame) -> Result<Source, EncodeError> {
        let (vw, vh) = (self.cfg.width as usize, self.cfg.height as usize);
        let aw = vw.div_ceil(128) * 128 + 160;
        let ah = vh.div_ceil(128) * 128 + 160;
        let bd = self.cfg.bit_depth as u32;
        let (cw, ch) = (vw.div_ceil(2), vh.div_ceil(2));
        let mut planes: [Plane; 3] = [Plane::new(aw, ah), Plane::new(aw / 2, ah / 2), Plane::new(aw / 2, ah / 2)];
        fn fill<T: Copy>(dst: &mut Plane, s: &[T], stride: usize, vw: usize, vh: usize, conv: impl Fn(T) -> u16) -> bool {
            let need = vh.checked_sub(1).and_then(|r| r.checked_mul(stride)).and_then(|n| n.checked_add(vw));
            if vw == 0 || vh == 0 || stride < vw || need.is_none_or(|n| s.len() < n) {
                return false;
            }
            let (dw, dh) = (dst.stride, dst.rows);
            for y in 0..dh {
                let sy = y.min(vh - 1);
                let row = &s[sy * stride..sy * stride + vw];
                let d = &mut dst.data[y * dw..(y + 1) * dw];
                for (x, o) in d.iter_mut().enumerate() {
                    *o = conv(row[x.min(vw - 1)]);
                }
            }
            true
        }
        let max = (1u32 << bd) - 1;
        let [py, pu, pv] = &mut planes;
        let ok = match *f {
            YuvFrame::U8 { y, u, v, y_stride, uv_stride } => {
                let up = bd - 8;
                let c = |s: u8| (s as u16) << up;
                fill(py, y, y_stride, vw, vh, c) && fill(pu, u, uv_stride, cw, ch, c) && fill(pv, v, uv_stride, cw, ch, c)
            }
            YuvFrame::U16 { y, u, v, y_stride, uv_stride } => {
                let c = |s: u16| (s as u32).min(max) as u16;
                fill(py, y, y_stride, vw, vh, c) && fill(pu, u, uv_stride, cw, ch, c) && fill(pv, v, uv_stride, cw, ch, c)
            }
        };
        if !ok {
            return Err(EncodeError::BadFrame);
        }
        Ok(Source { planes, width: [vw, cw, cw], height: [vh, ch, ch] })
    }
}

/// Tile columns / rows (log2) for about `tiles` tiles: columns first (at least 4 superblocks
/// wide), then rows (at least 4 superblocks high).
fn tile_layout(w: u32, h: u32, tiles: usize) -> (u32, u32) {
    let (min_cols, max_cols, max_rows, min_tiles) = obu::tile_limits(w, h);
    let sb_cols = (2 * w.div_ceil(8)).div_ceil(16);
    let sb_rows = (2 * h.div_ceil(8)).div_ceil(16);
    let want = (usize::BITS - tiles.max(1).saturating_sub(1).leading_zeros()).min(6);
    let mut cols = 0;
    while cols < want && cols < max_cols && (sb_cols >> (cols + 1)) >= 4 {
        cols += 1;
    }
    let cols = cols.max(min_cols);
    let mut rows = 0;
    while cols + rows < want && rows < max_rows && (sb_rows >> (rows + 1)) >= 4 {
        rows += 1;
    }
    let rows = rows.max(min_tiles.saturating_sub(cols));
    (cols, rows)
}

/// HDR metadata OBUs (6.7.3 / 6.7.4) from the H.265 SEI-layout inputs.
fn hdr_metadata(mdcv: Option<&[u8; 24]>, cll: Option<(u16, u16)>) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some((max_cll, max_fall)) = cll {
        let mut w = obu::BitWriter::default();
        w.f(8, 1); // metadata_type: METADATA_TYPE_HDR_CLL (leb128, one byte)
        w.f(16, max_cll as u32);
        w.f(16, max_fall as u32);
        w.trailing_bits();
        obu::obu(obu::OBU_METADATA, &w.finish(), &mut out);
    }
    if let Some(m) = mdcv {
        let be16 = |i: usize| u16::from_be_bytes([m[i], m[i + 1]]) as u64;
        let be32 = |i: usize| u32::from_be_bytes([m[i], m[i + 1], m[i + 2], m[i + 3]]) as u64;
        // 0.00002 units -> 0.16 fixed point
        let xy = |v: u64| ((v * 65536 + 25000) / 50000).min(65535) as u32;
        let mut w = obu::BitWriter::default();
        w.f(8, 2); // METADATA_TYPE_HDR_MDCV
        // SEI order is green, blue, red; AV1's is red, green, blue
        for i in [2usize, 0, 1] {
            w.f(16, xy(be16(i * 4)));
            w.f(16, xy(be16(i * 4 + 2)));
        }
        w.f(16, xy(be16(12)));
        w.f(16, xy(be16(14)));
        // 0.0001 cd/m² -> 24.8 and 18.14 fixed point
        w.f(32, ((be32(16) * 256 + 5000) / 10000).min(u32::MAX as u64) as u32);
        w.f(32, ((be32(20) * 16384 + 5000) / 10000).min(u32::MAX as u64) as u32);
        w.trailing_bits();
        obu::obu(obu::OBU_METADATA, &w.finish(), &mut out);
    }
    out
}

/// Code a frame's tiles with the encoder's policy (in parallel when there are several); returns
/// the final CDFs of tile context_update_tile_id and every tile's data.
pub(crate) fn encode_tiles(pool: &Pool, fs: &mut FrameState, cdfs: &Cdfs, enc: &FrameJobEnc) -> crate::Result<(Option<Box<Cdfs>>, Vec<Vec<u8>>)> {
    let ti = fs.fh.tile_info.clone();
    let n = ti.cols * ti.rows;
    let ctx_tile = ti.context_update_tile_id as usize;
    let run = |sh: &FrameShared, t: &mut TileState, num: usize| -> crate::Result<(Option<Box<Cdfs>>, Vec<u8>)> {
        t.intra_frame_y_mode_cdf = DEFAULT_INTRA_FRAME_Y_MODE_CDF;
        let sd = SymbolEncoder::new(sh.fh.disable_cdf_update);
        let mut td = TileDecoder::new(sh, t, sd, Box::new(cdfs.clone()), num / ti.cols, num % ti.cols);
        td.enc = Some(Box::new(TileEnc::new(&enc.src, &enc.fe)));
        td.decode_tile()?;
        let TileDecoder { cdf, sd, .. } = td;
        Ok(((num == ctx_tile).then_some(cdf), sd.finish()))
    };
    let mut saved = None;
    let mut tiles = Vec::with_capacity(n);
    if n > 1 && pool.is_parallel() {
        let mut states: Vec<TileState> = (0..n)
            .map(|num| {
                let (row, col) = (num / ti.cols, num % ti.cols);
                let (r0, r1) = (ti.mi_row_starts[row] as usize, ti.mi_row_starts[row + 1] as usize);
                let (c0, c1) = (ti.mi_col_starts[col] as usize, ti.mi_col_starts[col + 1] as usize);
                fs.tile_state(r0, r1.min(fs.mi.rows), c0, c1.min(fs.mi.cols))
            })
            .collect();
        let sh = &fs.sh;
        let results = pool.map_mut(&mut states, |i, t| run(sh, t, i));
        for t in &states {
            fs.merge_tile_state(t);
        }
        for r in results {
            let (c, data) = r?;
            if c.is_some() {
                saved = c;
            }
            tiles.push(data);
        }
        return Ok((saved, tiles));
    }
    let mut t = fs.take_tile_state();
    let mut res = Ok(());
    for num in 0..n {
        match run(&fs.sh, &mut t, num) {
            Ok((c, data)) => {
                if c.is_some() {
                    saved = c;
                }
                tiles.push(data);
            }
            Err(e) => {
                res = Err(e);
                break;
            }
        }
    }
    fs.put_tile_state(t);
    res.map(|_| (saved, tiles))
}

/// Squared error of the frame's visible samples against the source.
fn frame_sse(fs: &FrameState, src: &Source) -> u64 {
    let mut total = 0u64;
    for p in 0..fs.num_planes {
        let cur = &fs.cur.planes[p];
        let s = &src.planes[p];
        for y in 0..src.height[p] {
            let w = src.width[p];
            let a = &cur.row(y)[..w];
            let b = &s.row(y)[..w];
            total += a.iter().zip(b).map(|(&x, &y)| ((x as i64 - y as i64) * (x as i64 - y as i64)) as u64).sum::<u64>();
        }
    }
    total
}

/// Pick the deblocking level (the same for every direction and plane) by trying levels around a
/// guess from the quantizer; leaves it in the frame header and returns loop_filter_level[].
pub(crate) fn choose_loop_filter(fs: &mut FrameState, enc: &FrameJobEnc, pool: &Pool) -> [u8; 4] {
    let q = fs.fh.quant.base_q_idx as i32;
    let guess = if enc.fe.key { q / 4 } else { q / 5 }.clamp(0, 63);
    let set = |fs: &mut FrameState, l: i32| {
        let l = l.clamp(0, 63) as u32;
        fs.sh.fh.lf.level = [l, l, l, l];
    };
    if enc.fe.effort == 0 {
        set(fs, guess);
        let l = guess as u8;
        return [l; 4];
    }
    let base = fs.cur.clone();
    let eval = |fs: &mut FrameState, l: i32| -> u64 {
        set(fs, l);
        fs.cur = base.clone();
        if l > 0 {
            crate::postfilter::loop_filter(fs, pool);
        }
        frame_sse(fs, &enc.src)
    };
    let mut best = (eval(fs, guess), guess);
    let mut step = 8;
    while step >= 1 {
        let mut moved = true;
        while moved {
            moved = false;
            for l in [best.1 - step, best.1 + step] {
                if !(0..=63).contains(&l) {
                    continue;
                }
                let e = eval(fs, l);
                if e < best.0 {
                    best = (e, l);
                    moved = true;
                }
            }
        }
        step /= 2;
    }
    fs.cur = base;
    set(fs, best.1);
    let l = best.1 as u8;
    [l; 4]
}

/// The post-filter settings chosen for a frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Filters {
    pub lf: [u8; 4],
    pub cdef: Option<obu::CdefChoice>,
}

/// Squared error of each plane of the frame `cur` against the source.
fn plane_sse(cur: &crate::frame::FrameBuf, src: &Source, planes: usize) -> [u64; 3] {
    let mut out = [0u64; 3];
    for (p, o) in out.iter_mut().enumerate().take(planes) {
        let c = &cur.planes[p];
        let s = &src.planes[p];
        for y in 0..src.height[p] {
            let w = src.width[p];
            let a = &c.row(y)[..w];
            let b = &s.row(y)[..w];
            *o += a.iter().zip(b).map(|(&x, &y)| ((x as i64 - y as i64) * (x as i64 - y as i64)) as u64).sum::<u64>();
        }
    }
    out
}

/// Pick one CDEF strength for the frame (luma and chroma separately) by filtering the deblocked
/// reconstruction with a few candidates; leaves it in the frame header. `fs.cur` is left as it
/// was (the post-filters run afterwards).
pub(crate) fn choose_cdef(fs: &mut FrameState, enc: &FrameJobEnc, pool: &Pool) -> Option<obu::CdefChoice> {
    if !fs.seq.enable_cdef || fs.fh.coded_lossless || fs.fh.allow_intrabc {
        return None;
    }
    let q = fs.fh.quant.base_q_idx;
    let damping = (3 + q / 64).min(6) as u8;
    let set = |fs: &mut FrameState, c: obu::CdefChoice| {
        let p = &mut fs.sh.fh.cdef;
        p.damping = c.damping as u32;
        p.bits = 0;
        p.y_pri[0] = c.y_pri as u32;
        p.y_sec[0] = c.y_sec as u32;
        p.uv_pri[0] = c.uv_pri as u32;
        p.uv_sec[0] = c.uv_sec as u32;
    };
    let unfiltered = fs.cur.clone();
    let lvl = fs.fh.lf.level;
    if lvl[0] != 0 || lvl[1] != 0 {
        crate::postfilter::loop_filter(fs, pool);
    }
    let planes = fs.num_planes;
    let base = plane_sse(&fs.cur, &enc.src, planes);
    // (primary, secondary) candidates, tried on luma and chroma at once
    let cands: &[(u8, u8)] = match enc.fe.effort {
        0 => &[(2, 1), (4, 1), (6, 2)],
        1 => &[(1, 0), (2, 1), (3, 1), (4, 2), (6, 2), (8, 2), (10, 4)],
        _ => &[(1, 0), (1, 1), (2, 0), (2, 1), (3, 1), (4, 1), (4, 2), (5, 2), (6, 2), (8, 2), (8, 4), (10, 4), (12, 4), (15, 4)],
    };
    let mut best_y = (base[0], 0u8, 0u8);
    let mut best_uv = (base[1] + base[2], 0u8, 0u8);
    for &(pri, sec) in cands {
        set(fs, obu::CdefChoice { damping, y_pri: pri, y_sec: sec, uv_pri: pri, uv_sec: sec });
        let Some(out) = crate::cdef::apply(fs, pool) else { continue };
        let e = plane_sse(&out, &enc.src, planes);
        if e[0] < best_y.0 {
            best_y = (e[0], pri, sec);
        }
        if e[1] + e[2] < best_uv.0 {
            best_uv = (e[1] + e[2], pri, sec);
        }
    }
    fs.cur = unfiltered;
    let c = obu::CdefChoice { damping, y_pri: best_y.1, y_sec: best_y.2, uv_pri: best_uv.1, uv_sec: best_uv.2 };
    set(fs, c);
    Some(c)
}

#[cfg(test)]
mod tests;
