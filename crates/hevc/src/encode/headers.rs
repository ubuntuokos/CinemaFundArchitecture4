//! Parameter set, slice segment header and SEI writers (7.3.2 - 7.3.6, D.2).

use cfa4_bitstream::{BitWriter, escape_rbsp};

/// Sequence-level settings shared by the parameter sets and slice headers.
#[derive(Clone, Debug)]
pub struct SeqParams {
    /// Coded size (a multiple of the minimum CU size) and the visible size.
    pub coded_w: u32,
    pub coded_h: u32,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub log2_ctb: u32,
    pub log2_min_cb: u32,
    pub log2_max_tb: u32,
    pub log2_max_poc_lsb: u32,
    pub max_dec_pic_buffering: u32,
    pub level_idc: u8,
    pub fps: (u32, u32),
    pub sar: (u16, u16),
    /// (colour_primaries, transfer_characteristics, matrix_coeffs, full_range).
    pub color: Option<(u8, u8, u8, bool)>,
    pub sao: bool,
    pub tmvp: bool,
    pub amp: bool,
    pub sign_hiding: bool,
    pub cu_qp_delta: bool,
}

/// A NAL unit (Annex B payload without start code) from its header fields and RBSP.
pub fn nal(nal_type: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![nal_type << 1, 1];
    out.extend(escape_rbsp(rbsp));
    out
}

fn profile_tier_level(w: &mut BitWriter, p: &SeqParams) {
    let profile = if p.bit_depth > 8 { 2 } else { 1 };
    w.write_bits(0, 2); // general_profile_space
    w.write_bit(false); // general_tier_flag (Main)
    w.write_bits(profile, 5);
    // compatibility: Main is also decodable by Main 10 decoders
    let compat = if profile == 1 { (1u32 << 30) | (1 << 29) } else { 1 << 29 };
    w.write_bits(compat, 32);
    w.write_bit(true); // progressive_source
    w.write_bit(false); // interlaced_source
    w.write_bit(false); // non_packed_constraint
    w.write_bit(true); // frame_only_constraint
    w.write_bits(0, 32); // 43 reserved bits + general_inbld / reserved
    w.write_bits(0, 12);
    w.write_bits(p.level_idc as u32, 8);
}

/// Level (general_level_idc = 30 x level) for a picture size and rate (Table A.8).
pub fn level_for(w: u32, h: u32, fps: f64) -> u8 {
    let ps = w as u64 * h as u64;
    let sr = ps as f64 * fps;
    // (level_idc, MaxLumaPs, MaxLumaSr)
    const LEVELS: [(u8, u64, f64); 13] = [
        (30, 36_864, 552_960.0),
        (60, 122_880, 3_686_400.0),
        (63, 245_760, 7_372_800.0),
        (90, 552_960, 16_588_800.0),
        (93, 983_040, 33_177_600.0),
        (120, 2_228_224, 66_846_720.0),
        (123, 2_228_224, 133_693_440.0),
        (150, 8_912_896, 267_386_880.0),
        (153, 8_912_896, 534_773_760.0),
        (156, 8_912_896, 1_069_547_520.0),
        (180, 35_651_584, 1_069_547_520.0),
        (183, 35_651_584, 2_139_095_040.0),
        (186, 35_651_584, 4_278_190_080.0),
    ];
    // each dimension is also limited to sqrt(8 * MaxLumaPs)
    let side = w.max(h) as f64;
    LEVELS.iter().find(|&&(_, mps, msr)| ps <= mps && sr <= msr && side * side <= 8.0 * mps as f64).map_or(186, |l| l.0)
}

pub fn vps(p: &SeqParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // vps_video_parameter_set_id
    w.write_bit(true); // vps_base_layer_internal_flag
    w.write_bit(true); // vps_base_layer_available_flag
    w.write_bits(0, 6); // vps_max_layers_minus1
    w.write_bits(0, 3); // vps_max_sub_layers_minus1
    w.write_bit(true); // vps_temporal_id_nesting_flag
    w.write_bits(0xffff, 16);
    profile_tier_level(&mut w, p);
    w.write_bit(true); // vps_sub_layer_ordering_info_present_flag
    w.write_ue(p.max_dec_pic_buffering - 1);
    w.write_ue(0); // vps_max_num_reorder_pics
    w.write_ue(0); // vps_max_latency_increase_plus1
    w.write_bits(0, 6); // vps_max_layer_id
    w.write_ue(0); // vps_num_layer_sets_minus1
    let timing = p.fps.0 > 0 && p.fps.1 > 0;
    w.write_bit(timing);
    if timing {
        w.write_bits(p.fps.1, 32); // vps_num_units_in_tick
        w.write_bits(p.fps.0, 32); // vps_time_scale
        w.write_bit(false); // vps_poc_proportional_to_timing_flag
        w.write_ue(0); // vps_num_hrd_parameters
    }
    w.write_bit(false); // vps_extension_flag
    w.rbsp_trailing();
    nal(32, &w.finish())
}

pub fn sps(p: &SeqParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // sps_video_parameter_set_id
    w.write_bits(0, 3); // sps_max_sub_layers_minus1
    w.write_bit(true); // sps_temporal_id_nesting_flag
    profile_tier_level(&mut w, p);
    w.write_ue(0); // sps_seq_parameter_set_id
    w.write_ue(1); // chroma_format_idc 4:2:0
    w.write_ue(p.coded_w);
    w.write_ue(p.coded_h);
    let crop = p.coded_w != p.width || p.coded_h != p.height;
    w.write_bit(crop);
    if crop {
        // offsets in chroma sample units (SubWidthC = SubHeightC = 2)
        w.write_ue(0);
        w.write_ue((p.coded_w - p.width) / 2);
        w.write_ue(0);
        w.write_ue((p.coded_h - p.height) / 2);
    }
    w.write_ue(p.bit_depth - 8);
    w.write_ue(p.bit_depth - 8);
    w.write_ue(p.log2_max_poc_lsb - 4);
    w.write_bit(true); // sps_sub_layer_ordering_info_present_flag
    w.write_ue(p.max_dec_pic_buffering - 1);
    w.write_ue(0); // sps_max_num_reorder_pics
    w.write_ue(0); // sps_max_latency_increase_plus1
    w.write_ue(p.log2_min_cb - 3);
    w.write_ue(p.log2_ctb - p.log2_min_cb);
    w.write_ue(0); // log2_min_luma_transform_block_size_minus2
    w.write_ue(p.log2_max_tb - 2);
    w.write_ue(0); // max_transform_hierarchy_depth_inter
    w.write_ue(0); // max_transform_hierarchy_depth_intra
    w.write_bit(false); // scaling_list_enabled_flag
    w.write_bit(p.amp);
    w.write_bit(p.sao);
    w.write_bit(false); // pcm_enabled_flag
    w.write_ue(0); // num_short_term_ref_pic_sets
    w.write_bit(false); // long_term_ref_pics_present_flag
    w.write_bit(p.tmvp);
    w.write_bit(true); // strong_intra_smoothing_enabled_flag
    w.write_bit(true); // vui_parameters_present_flag
    vui(&mut w, p);
    w.write_bit(false); // sps_extension_present_flag
    w.rbsp_trailing();
    nal(33, &w.finish())
}

fn vui(w: &mut BitWriter, p: &SeqParams) {
    let sar = p.sar.0 > 0 && p.sar.1 > 0 && p.sar != (1, 1);
    w.write_bit(sar); // aspect_ratio_info_present_flag
    if sar {
        w.write_bits(255, 8); // EXTENDED_SAR
        w.write_bits(p.sar.0 as u32, 16);
        w.write_bits(p.sar.1 as u32, 16);
    }
    w.write_bit(false); // overscan_info_present_flag
    w.write_bit(p.color.is_some()); // video_signal_type_present_flag
    if let Some((prim, trc, mat, full)) = p.color {
        w.write_bits(5, 3); // video_format: unspecified
        w.write_bit(full);
        w.write_bit(true); // colour_description_present_flag
        w.write_bits(prim as u32, 8);
        w.write_bits(trc as u32, 8);
        w.write_bits(mat as u32, 8);
    }
    w.write_bit(false); // chroma_loc_info_present_flag
    w.write_bit(false); // neutral_chroma_indication_flag
    w.write_bit(false); // field_seq_flag
    w.write_bit(false); // frame_field_info_present_flag
    w.write_bit(false); // default_display_window_flag
    let timing = p.fps.0 > 0 && p.fps.1 > 0;
    w.write_bit(timing); // vui_timing_info_present_flag
    if timing {
        w.write_bits(p.fps.1, 32);
        w.write_bits(p.fps.0, 32);
        w.write_bit(false); // vui_poc_proportional_to_timing_flag
        w.write_bit(false); // vui_hrd_parameters_present_flag
    }
    w.write_bit(false); // bitstream_restriction_flag
}

pub fn pps(p: &SeqParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // pps_pic_parameter_set_id
    w.write_ue(0); // pps_seq_parameter_set_id
    w.write_bit(false); // dependent_slice_segments_enabled_flag
    w.write_bit(false); // output_flag_present_flag
    w.write_bits(0, 3); // num_extra_slice_header_bits
    w.write_bit(p.sign_hiding);
    w.write_bit(false); // cabac_init_present_flag
    w.write_ue(0); // num_ref_idx_l0_default_active_minus1
    w.write_ue(0); // num_ref_idx_l1_default_active_minus1
    w.write_se(0); // init_qp_minus26
    w.write_bit(false); // constrained_intra_pred_flag
    w.write_bit(false); // transform_skip_enabled_flag
    w.write_bit(p.cu_qp_delta);
    if p.cu_qp_delta {
        w.write_ue(0); // diff_cu_qp_delta_depth: one quantization group per CTU
    }
    w.write_se(0); // pps_cb_qp_offset
    w.write_se(0); // pps_cr_qp_offset
    w.write_bit(false); // pps_slice_chroma_qp_offsets_present_flag
    w.write_bit(false); // weighted_pred_flag
    w.write_bit(false); // weighted_bipred_flag
    w.write_bit(false); // transquant_bypass_enabled_flag
    w.write_bit(false); // tiles_enabled_flag
    w.write_bit(false); // entropy_coding_sync_enabled_flag
    w.write_bit(true); // pps_loop_filter_across_slices_enabled_flag
    w.write_bit(false); // deblocking_filter_control_present_flag
    w.write_bit(false); // pps_scaling_list_data_present_flag
    w.write_bit(false); // lists_modification_present_flag
    w.write_ue(0); // log2_parallel_merge_level_minus2
    w.write_bit(false); // slice_segment_header_extension_present_flag
    w.write_bit(false); // pps_extension_present_flag
    w.rbsp_trailing();
    nal(34, &w.finish())
}

/// What the slice header of one picture says.
pub struct SliceParams {
    pub idr: bool,
    pub intra: bool,
    pub poc: i32,
    /// Negative POC deltas of the reference pictures (closest first); all used by the picture.
    pub ref_deltas: Vec<i32>,
    pub qp: i32,
    pub max_merge_cand: u32,
    /// slice_segment_address (the first CTB of the slice) and its width in bits.
    pub address: u32,
    pub address_bits: u32,
}

/// Slice segment header RBSP up to and including byte_alignment(); slice data follows.
pub fn slice_header(p: &SeqParams, s: &SliceParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bit(s.address == 0); // first_slice_segment_in_pic_flag
    if s.idr {
        w.write_bit(false); // no_output_of_prior_pics_flag
    }
    w.write_ue(0); // slice_pic_parameter_set_id
    if s.address != 0 {
        // dependent slice segments are off, so no dependent_slice_segment_flag
        w.write_bits(s.address, s.address_bits);
    }
    w.write_ue(if s.intra { 2 } else { 1 }); // slice_type
    if !s.idr {
        let lsb_mask = (1i64 << p.log2_max_poc_lsb) - 1;
        w.write_bits((s.poc as i64 & lsb_mask) as u32, p.log2_max_poc_lsb);
        w.write_bit(false); // short_term_ref_pic_set_sps_flag
        // st_ref_pic_set(0): num_negative_pics, num_positive_pics, then the deltas
        w.write_ue(s.ref_deltas.len() as u32);
        w.write_ue(0);
        let mut prev = 0;
        for &d in &s.ref_deltas {
            w.write_ue((prev - d - 1) as u32); // delta_poc_s0_minus1
            w.write_bit(true); // used_by_curr_pic_s0_flag
            prev = d;
        }
        if p.tmvp {
            w.write_bit(true); // slice_temporal_mvp_enabled_flag
        }
    }
    if p.sao {
        w.write_bit(true); // slice_sao_luma_flag
        w.write_bit(true); // slice_sao_chroma_flag
    }
    if !s.intra {
        let n = s.ref_deltas.len() as u32;
        w.write_bit(true); // num_ref_idx_active_override_flag
        w.write_ue(n - 1);
        if p.tmvp && n > 1 {
            w.write_ue(0); // collocated_ref_idx
        }
        w.write_ue(5 - s.max_merge_cand);
    }
    w.write_se(s.qp - 26); // slice_qp_delta
    // slice_loop_filter_across_slices_enabled_flag (deblocking is on)
    w.write_bit(true);
    // byte_alignment()
    w.write_bit(true);
    w.align_zero();
    w.finish()
}

/// Prefix SEI NAL with mastering display colour volume (payload 137) and/or content light level
/// (payload 144).
pub fn hdr_sei(mastering: Option<&[u8; 24]>, cll: Option<(u16, u16)>) -> Option<Vec<u8>> {
    if mastering.is_none() && cll.is_none() {
        return None;
    }
    let mut w = BitWriter::new();
    if let Some(m) = mastering {
        w.write_bits(137, 8);
        w.write_bits(24, 8);
        w.write_bytes(m);
    }
    if let Some((max_cll, max_fall)) = cll {
        w.write_bits(144, 8);
        w.write_bits(4, 8);
        w.write_bits(max_cll as u32, 16);
        w.write_bits(max_fall as u32, 16);
    }
    w.rbsp_trailing();
    Some(nal(39, &w.finish()))
}
