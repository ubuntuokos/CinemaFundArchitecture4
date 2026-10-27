//! OBU packaging and the sequence / frame headers the encoder writes (spec 5.3, 5.5, 5.9), for
//! the subset of coding tools it uses.

use crate::spec_tables::*;

pub(crate) const OBU_SEQUENCE_HEADER: u8 = 1;
pub(crate) const OBU_TEMPORAL_DELIMITER: u8 = 2;
pub(crate) const OBU_FRAME_HEADER: u8 = 3;
pub(crate) const OBU_METADATA: u8 = 5;
pub(crate) const OBU_FRAME: u8 = 6;

/// Order hints are coded with this many bits.
pub(crate) const ORDER_HINT_BITS: u32 = 7;

/// MSB-first bit writer (the inverse of `BitReader`).
#[derive(Default)]
pub(crate) struct BitWriter {
    pub buf: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub fn bit(&mut self, b: bool) {
        if self.bits & 7 == 0 {
            self.buf.push(0);
        }
        if b && let Some(last) = self.buf.last_mut() {
            *last |= 0x80 >> (self.bits & 7);
        }
        self.bits += 1;
    }

    /// `f(n)`
    pub fn f(&mut self, n: u32, v: u32) {
        for i in (0..n.min(32)).rev() {
            self.bit((v >> i) & 1 == 1);
        }
    }

    /// `su(n)`
    pub fn su(&mut self, n: u32, v: i32) {
        let mask = if n >= 32 { u32::MAX } else { (1u32 << n) - 1 };
        self.f(n, v as u32 & mask);
    }

    /// `trailing_bits( )`: a one bit, then zeros to the byte boundary.
    pub fn trailing_bits(&mut self) {
        self.bit(true);
        self.byte_align();
    }

    pub fn byte_align(&mut self) {
        self.bits = self.bits.div_ceil(8) * 8;
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

pub(crate) fn leb128(mut v: usize, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// An OBU with `obu_has_size_field` set.
pub(crate) fn obu(obu_type: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.push((obu_type << 3) | 2);
    leb128(payload.len(), out);
    out.extend_from_slice(payload);
}

/// What the sequence header describes.
#[derive(Clone, Debug)]
pub(crate) struct SeqParams {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub subsampling_x: u8,
    pub subsampling_y: u8,
    pub mono: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
    /// seq_level_idx (31: no level constraints)
    pub level: u8,
    pub enable_cdef: bool,
}

impl SeqParams {
    pub fn profile(&self) -> u8 {
        if self.bit_depth == 12 || (self.subsampling_x == 1 && self.subsampling_y == 0) {
            2
        } else if self.subsampling_x == 0 && !self.mono {
            1
        } else {
            0
        }
    }
}

/// sequence_header_obu( ) payload.
pub(crate) fn sequence_header(s: &SeqParams) -> Vec<u8> {
    let mut w = BitWriter::default();
    let profile = s.profile();
    w.f(3, profile as u32);
    w.bit(false); // still_picture
    w.bit(false); // reduced_still_picture_header
    w.bit(false); // timing_info_present_flag
    w.bit(false); // initial_display_delay_present_flag
    w.f(5, 0); // operating_points_cnt_minus_1
    w.f(12, 0); // operating_point_idc[0]
    w.f(5, s.level as u32);
    if s.level > 7 {
        w.bit(false); // seq_tier
    }
    let wb = bits_for(s.width.saturating_sub(1));
    let hb = bits_for(s.height.saturating_sub(1));
    w.f(4, wb - 1);
    w.f(4, hb - 1);
    w.f(wb, s.width.saturating_sub(1));
    w.f(hb, s.height.saturating_sub(1));
    w.bit(false); // frame_id_numbers_present_flag
    w.bit(false); // use_128x128_superblock
    w.bit(false); // enable_filter_intra
    w.bit(true); // enable_intra_edge_filter
    w.bit(false); // enable_interintra_compound
    w.bit(false); // enable_masked_compound
    w.bit(false); // enable_warped_motion
    w.bit(false); // enable_dual_filter
    w.bit(true); // enable_order_hint
    w.bit(false); // enable_jnt_comp
    w.bit(false); // enable_ref_frame_mvs
    w.bit(false); // seq_choose_screen_content_tools
    w.f(1, 0); // seq_force_screen_content_tools
    w.f(3, ORDER_HINT_BITS - 1);
    w.bit(false); // enable_superres
    w.bit(s.enable_cdef);
    w.bit(false); // enable_restoration
    // color_config( )
    w.bit(s.bit_depth > 8); // high_bitdepth
    if profile == 2 && s.bit_depth > 8 {
        w.bit(s.bit_depth == 12); // twelve_bit
    }
    if profile != 1 {
        w.bit(s.mono);
    }
    w.bit(true); // color_description_present_flag
    w.f(8, s.primaries as u32);
    w.f(8, s.transfer as u32);
    w.f(8, s.matrix as u32);
    let srgb = s.primaries == 1 && s.transfer == 13 && s.matrix == 0;
    if s.mono {
        w.bit(s.full_range);
    } else if !srgb {
        w.bit(s.full_range);
        if profile == 2 && s.bit_depth == 12 {
            w.f(1, s.subsampling_x as u32);
            if s.subsampling_x == 1 {
                w.f(1, s.subsampling_y as u32);
            }
        }
        if s.subsampling_x == 1 && s.subsampling_y == 1 {
            w.f(2, 0); // chroma_sample_position: unknown
        }
    }
    if !s.mono {
        w.bit(false); // separate_uv_delta_q
    }
    w.bit(false); // film_grain_params_present
    w.trailing_bits();
    w.finish()
}

fn bits_for(v: u32) -> u32 {
    (32 - v.leading_zeros()).max(1)
}

/// The choices of one frame header.
#[derive(Clone, Debug)]
pub(crate) struct FrameParams {
    pub key: bool,
    pub order_hint: u32,
    pub refresh_frame_flags: u8,
    pub base_q_idx: u8,
    /// loop_filter_level[0..4]
    pub lf: [u8; 4],
    pub cdef: Option<CdefChoice>,
    pub tile_cols_log2: u32,
    pub tile_rows_log2: u32,
    /// Largest tile (its CDFs carry over to the next frame).
    pub context_update_tile_id: u32,
    pub allow_high_precision_mv: bool,
}

/// One CDEF strength for the frame (cdef_bits = 0).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CdefChoice {
    pub damping: u8,
    pub y_pri: u8,
    pub y_sec: u8,
    pub uv_pri: u8,
    pub uv_sec: u8,
}

/// Tile layout limits of a frame (5.9.15): (min cols log2, max cols log2, max rows log2,
/// min tiles log2).
pub(crate) fn tile_limits(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let mi_cols = 2 * width.div_ceil(8);
    let mi_rows = 2 * height.div_ceil(8);
    let sb_cols = (mi_cols + 15) >> 4;
    let sb_rows = (mi_rows + 15) >> 4;
    let max_tile_width_sb = MAX_TILE_WIDTH as u32 >> 6;
    let max_tile_area_sb = MAX_TILE_AREA as u32 >> 12;
    let min_cols = tile_log2(max_tile_width_sb, sb_cols);
    let max_cols = tile_log2(1, sb_cols.min(MAX_TILE_COLS as u32));
    let max_rows = tile_log2(1, sb_rows.min(MAX_TILE_ROWS as u32));
    let min_tiles = min_cols.max(tile_log2(max_tile_area_sb, sb_rows * sb_cols));
    (min_cols, max_cols, max_rows, min_tiles)
}

fn tile_log2(blk: u32, target: u32) -> u32 {
    let mut k = 0;
    while (blk.max(1) << k) < target {
        k += 1;
    }
    k
}

/// uncompressed_header( ) (frame_header_obu payload, byte aligned).
pub(crate) fn frame_header(s: &SeqParams, f: &FrameParams) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bit(false); // show_existing_frame
    w.f(2, if f.key { KEY_FRAME } else { INTER_FRAME } as u32);
    w.bit(true); // show_frame
    if !f.key {
        w.bit(false); // error_resilient_mode
    }
    w.bit(false); // disable_cdf_update
    w.bit(false); // frame_size_override_flag
    w.f(ORDER_HINT_BITS, f.order_hint & ((1 << ORDER_HINT_BITS) - 1));
    if !f.key {
        w.f(3, 0); // primary_ref_frame: LAST_FRAME's
        w.f(8, f.refresh_frame_flags as u32);
        w.bit(false); // frame_refs_short_signaling
        for _ in 0..REFS_PER_FRAME {
            w.f(3, 0); // ref_frame_idx: every reference is slot 0
        }
        w.bit(false); // render_and_frame_size_different
        w.bit(f.allow_high_precision_mv);
        w.bit(false); // is_filter_switchable
        w.f(2, 0); // interpolation_filter: EIGHTTAP
        w.bit(false); // is_motion_mode_switchable
    } else {
        w.bit(false); // render_and_frame_size_different
    }
    w.bit(false); // disable_frame_end_update_cdf
    // tile_info( )
    let (min_cols, max_cols, max_rows, min_tiles) = tile_limits(s.width, s.height);
    let cols_log2 = f.tile_cols_log2.clamp(min_cols, max_cols);
    w.bit(true); // uniform_tile_spacing_flag
    for _ in min_cols..cols_log2 {
        w.bit(true); // increment_tile_cols_log2
    }
    if cols_log2 < max_cols {
        w.bit(false);
    }
    let min_rows = min_tiles.saturating_sub(cols_log2);
    let rows_log2 = f.tile_rows_log2.clamp(min_rows, max_rows.max(min_rows));
    for _ in min_rows..rows_log2 {
        w.bit(true); // increment_tile_rows_log2
    }
    if rows_log2 < max_rows {
        w.bit(false);
    }
    if cols_log2 > 0 || rows_log2 > 0 {
        w.f(cols_log2 + rows_log2, f.context_update_tile_id);
        w.f(2, 3); // tile_size_bytes_minus_1
    }
    // quantization_params( )
    w.f(8, f.base_q_idx as u32);
    w.bit(false); // DeltaQYDc
    if !s.mono {
        w.bit(false); // DeltaQUDc
        w.bit(false); // DeltaQUAc
    }
    w.bit(false); // using_qmatrix
    w.bit(false); // segmentation_enabled
    if f.base_q_idx > 0 {
        w.bit(false); // delta_q_present
    }
    // loop_filter_params( )
    w.f(6, f.lf[0] as u32);
    w.f(6, f.lf[1] as u32);
    if !s.mono && (f.lf[0] != 0 || f.lf[1] != 0) {
        w.f(6, f.lf[2] as u32);
        w.f(6, f.lf[3] as u32);
    }
    w.f(3, 0); // loop_filter_sharpness
    w.bit(true); // loop_filter_delta_enabled
    w.bit(false); // loop_filter_delta_update
    // cdef_params( )
    if s.enable_cdef {
        let c = f.cdef.unwrap_or_default();
        w.f(2, c.damping.saturating_sub(3).min(3) as u32);
        w.f(2, 0); // cdef_bits
        w.f(4, c.y_pri.min(15) as u32);
        w.f(2, sec_code(c.y_sec));
        if !s.mono {
            w.f(4, c.uv_pri.min(15) as u32);
            w.f(2, sec_code(c.uv_sec));
        }
    }
    w.bit(false); // tx_mode_select: TX_MODE_LARGEST
    if !f.key {
        w.bit(false); // reference_select
    }
    w.bit(true); // reduced_tx_set
    if !f.key {
        for _ in 0..REFS_PER_FRAME {
            w.bit(false); // is_global
        }
    }
    w.byte_align();
    w.finish()
}

fn sec_code(v: u8) -> u32 {
    match v {
        4.. => 3,
        v => v as u32,
    }
}

/// seq_level_idx for a picture size and rate (Annex A limits; 31 when none fits).
pub(crate) fn level_for(width: u32, height: u32, fps: f64) -> u8 {
    // (seq_level_idx, MaxPicSize, MaxHSize, MaxVSize, MaxDisplayRate)
    const LEVELS: [(u8, u64, u32, u32, f64); 13] = [
        (0, 147_456, 2048, 1152, 4_423_680.0),
        (1, 278_784, 2816, 1584, 8_363_520.0),
        (4, 665_856, 4352, 2448, 19_975_680.0),
        (5, 1_065_024, 5504, 3096, 31_950_720.0),
        (8, 2_359_296, 6144, 3456, 70_778_880.0),
        (9, 2_359_296, 6144, 3456, 141_557_760.0),
        (12, 8_912_896, 8192, 4352, 267_386_880.0),
        (13, 8_912_896, 8192, 4352, 534_773_760.0),
        (14, 8_912_896, 8192, 4352, 1_069_547_520.0),
        (15, 8_912_896, 8192, 4352, 1_069_547_520.0),
        (16, 35_651_584, 16384, 8704, 1_069_547_520.0),
        (17, 35_651_584, 16384, 8704, 2_139_095_040.0),
        (18, 35_651_584, 16384, 8704, 4_278_190_080.0),
    ];
    let size = width as u64 * height as u64;
    let rate = size as f64 * fps.max(1.0);
    for &(idx, pic, hs, vs, disp) in &LEVELS {
        if size <= pic && width <= hs && height <= vs && rate <= disp {
            return idx;
        }
    }
    31
}
