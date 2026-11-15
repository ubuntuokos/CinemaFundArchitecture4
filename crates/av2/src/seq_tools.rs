//! The tool configuration of the sequence header (AV2 v1.0.0 § 5.4.1 after the cropping window,
//! § 5.4.2 – 5.4.10, 5.4.13): decoder model, layer dependency maps, partition, segment, intra,
//! inter, screen content, transform/quantizer/entropy, filter and tile configuration, and the
//! film grain flag. The names follow the specification; derived variables are kept where later
//! syntax depends on them.

use cfa4_bitstream::BitReader;

use crate::tile::{TileLimits, TileParams, tile_params};
use crate::{Error, Result};

/// `MAX_NUM_MLAYERS`.
pub const MAX_NUM_MLAYERS: usize = 8;
/// `MAX_NUM_TLAYERS`.
pub const MAX_NUM_TLAYERS: usize = 4;
/// `MOTION_MODES`.
pub const MOTION_MODES: usize = 5;
/// `INTERINTRA` (first motion mode with a sequence enable flag).
const INTERINTRA: usize = 1;
/// `DELTAWARP`.
const DELTAWARP: usize = 3;
/// `MAX_REF_MV_STACK_SIZE`.
const MAX_REF_MV_STACK_SIZE: u32 = 6;
/// `MAX_REF_BV_STACK_SIZE`.
const MAX_REF_BV_STACK_SIZE: u32 = 4;
/// `REFS_PER_FRAME`.
const REFS_PER_FRAME: u8 = 7;
/// `SEG_LVL_MAX`.
pub const SEG_LVL_MAX: usize = 3;
/// `MAXQ_BITS` = `MAXQ_8_BITS + 4 * MAXQ_OFFSET` = 255 + 4 · 24.
const MAXQ_BITS: i32 = 351;
/// `Segmentation_Feature_Bits`, `_Signed`, `_Max` (§ 5.4.9).
const SEG_FEATURE_BITS: [u32; SEG_LVL_MAX] = [9, 0, 0];
const SEG_FEATURE_SIGNED: [bool; SEG_LVL_MAX] = [true, false, false];
const SEG_FEATURE_MAX: [i32; SEG_LVL_MAX] = [MAXQ_BITS, 0, 0];
/// `DELTA_DCQUANT_MIN` = `DELTA_DCQUANT_MAX - (1 << DELTA_DCQUANT_BITS) + 1` = 8 − 32 + 1.
const DELTA_DCQUANT_MIN: i32 = -23;
/// `SELECT_SCREEN_CONTENT_TOOLS`, `SELECT_INTEGER_MV`.
pub const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;
pub const SELECT_INTEGER_MV: u8 = 2;

/// `get_seq_sb_size()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuperblockSize {
    Sb64,
    Sb128,
    Sb256,
}

/// `DrlReorder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrlReorder {
    Disabled,
    Constraint,
    Always,
}

/// `CdefOnSkipTxfm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdefOnSkipTxfm {
    AlwaysOn,
    Disabled,
    Adaptive,
}

/// `seq_decoder_model_info()` (§ 5.4.13).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecoderModelInfo {
    pub decoder_buffer_delay: u32,
    pub encoder_buffer_delay: u32,
    pub low_delay_mode: bool,
}

/// Decoder model fields of the sequence header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecoderModel {
    /// `seq_initial_display_delay_minus_1 + 1`, when present.
    pub initial_display_delay: Option<u8>,
    /// `num_units_in_decoding_tick`, when `decoder_model_info_present_flag`.
    pub num_units_in_decoding_tick: Option<u32>,
    pub info: Option<DecoderModelInfo>,
}

/// `seg_info()` (§ 5.4.9).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SegInfo {
    pub enabled: Vec<[bool; SEG_LVL_MAX]>,
    pub data: Vec<[i32; SEG_LVL_MAX]>,
}

/// § 5.4.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionConfig {
    pub sb_size: SuperblockSize,
    pub enable_sdp: bool,
    pub enable_extended_sdp: bool,
    pub enable_ext_partitions: bool,
    pub enable_uneven_4way_partitions: bool,
    /// `MaxPbAspectRatio`.
    pub max_pb_aspect_ratio: u8,
}

/// § 5.4.4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentConfig {
    pub enable_ext_seg: bool,
    /// `MaxSegments`.
    pub max_segments: u8,
    /// `seq_allow_seg_info_change` and the sequence `seg_info()`, when present.
    pub seq_seg_info: Option<(bool, SegInfo)>,
}

/// § 5.4.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntraConfig {
    pub enable_dip: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_mrls: bool,
    pub enable_cfl_intra: bool,
    pub cfl_ds_filter_index: u8,
    pub enable_mhccp: bool,
    pub enable_ibp: bool,
}

/// § 5.4.6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterConfig {
    pub seq_enabled_motion_modes: [bool; MOTION_MODES],
    pub seq_frame_motion_modes_present: bool,
    pub enable_six_param_warp_delta: bool,
    pub enable_masked_compound: bool,
    pub enable_ref_frame_mvs: bool,
    pub reduced_ref_frame_mvs_mode: bool,
    /// `OrderHintBits` (0 with a single picture header).
    pub order_hint_bits: u8,
    pub enable_refmvbank: bool,
    pub drl_reorder: DrlReorder,
    pub explicit_ref_frame_map: bool,
    /// `NumRefFrames`.
    pub num_ref_frames: u8,
    /// `ActiveNumRefFrames`.
    pub active_num_ref_frames: u8,
    pub long_term_frame_id_bits: u8,
    pub seq_max_drl_bits_minus_1: u32,
    pub allow_frame_max_drl_bits: bool,
    pub seq_max_bvp_drl_bits_minus_1: u32,
    pub allow_frame_max_bvp_drl_bits: bool,
    pub num_same_ref_compound: u8,
    pub enable_tip: bool,
    /// `EnableTipOutput`.
    pub enable_tip_output: bool,
    pub enable_tip_hole_fill: bool,
    pub enable_mv_traj: bool,
    pub enable_bawp: bool,
    pub enable_cwp: bool,
    pub enable_imp_msk_bld: bool,
    pub enable_df_sub_pu: bool,
    pub enable_tip_explicit_qp: bool,
    /// `enable_opfl_refine` (0 = `REFINE_NONE`).
    pub enable_opfl_refine: u8,
    pub enable_refinemv: bool,
    pub enable_tip_refinemv: bool,
    pub enable_bru: bool,
    pub enable_adaptive_mvd: bool,
    pub enable_mvd_sign_derive: bool,
    pub enable_flex_mvres: bool,
    pub enable_global_motion: bool,
    pub enable_short_refresh_frame_flags: bool,
}

/// § 5.4.7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenContentConfig {
    /// 0, 1 or [`SELECT_SCREEN_CONTENT_TOOLS`].
    pub seq_force_screen_content_tools: u8,
    /// 0, 1 or [`SELECT_INTEGER_MV`].
    pub seq_force_integer_mv: u8,
}

/// § 5.4.8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransformQuantEntropyConfig {
    pub enable_fsc: bool,
    pub enable_idtx_intra: bool,
    pub enable_intra_ist: bool,
    pub enable_inter_ist: bool,
    pub enable_chroma_dctonly: bool,
    pub enable_inter_ddt: bool,
    pub reduced_tx_part_set: bool,
    pub enable_cctx: bool,
    pub enable_tcq: bool,
    pub choose_tcq_per_frame: bool,
    pub enable_parity_hiding: bool,
    pub enable_avg_cdf: bool,
    pub avg_cdf_type: bool,
    pub separate_uv_delta_q: bool,
    pub equal_ac_dc_q: bool,
    /// `BaseYDcDeltaQ`, `BaseUVDcDeltaQ`, `BaseUVAcDeltaQ`.
    pub base_y_dc_delta_q: i32,
    pub base_uv_dc_delta_q: i32,
    pub base_uv_ac_delta_q: i32,
    pub y_dc_delta_q_enabled: bool,
    pub uv_dc_delta_q_enabled: bool,
    pub uv_ac_delta_q_enabled: bool,
}

/// § 5.4.10. `lr_tools_disable[plane][tool]` is kept for `RESTORE_PC_WIENER` and
/// `RESTORE_WIENER_NONSEP`, as `[luma, chroma]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterConfig {
    pub disable_loopfilters_across_tiles: bool,
    pub enable_cdef: bool,
    pub enable_gdf: bool,
    pub gdf_unit_matches_sb_size: bool,
    pub enable_restoration: bool,
    pub lr_disable_pc_wiener: [bool; 2],
    pub lr_disable_wiener_nonsep: [bool; 2],
    pub enable_ccso: bool,
    pub ccso_unit_matches_sb_size: bool,
    pub cdef_on_skip_txfm: CdefOnSkipTxfm,
    pub df_par_bits_minus_2: u8,
}

/// `sequence_tile_config()` (§ 5.4.2) when `seq_tile_info_present_flag` is 1: the layout for the
/// maximum frame size and superblock size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqTileInfo {
    pub allow_tile_info_change: bool,
    pub params: TileParams,
}

/// Everything of `sequence_header_obu()` after the cropping window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceTools {
    pub decoder_model: DecoderModel,
    /// `MLayerDependencyMap[curr][ref]`.
    pub mlayer_dependency_map: [[bool; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS],
    /// `TLayerDependencyMap[mlayer][curr][ref]`.
    pub tlayer_dependency_map: [[[bool; MAX_NUM_TLAYERS]; MAX_NUM_TLAYERS]; MAX_NUM_MLAYERS],
    pub partition: PartitionConfig,
    pub segment: SegmentConfig,
    pub intra: IntraConfig,
    pub inter: InterConfig,
    pub screen_content: ScreenContentConfig,
    pub tqe: TransformQuantEntropyConfig,
    pub filter: FilterConfig,
    /// The sequence tile layout, when `seq_tile_info_present_flag` (§ 5.4.2).
    pub seq_tile_info: Option<SeqTileInfo>,
    pub film_grain_params_present: bool,
}

/// The leading fields the tool syntax depends on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context {
    pub single_picture_header: bool,
    pub monochrome: bool,
    pub max_tlayer_id: u8,
    pub max_mlayer_id: u8,
    /// `max_frame_width_minus_1 + 1`, `max_frame_height_minus_1 + 1`.
    pub max_frame_size: (u32, u32),
    pub limits: TileLimits,
}

fn flag(r: &mut BitReader<'_>) -> Result<bool> {
    Ok(r.read_flag()?)
}

fn bits_u8(r: &mut BitReader<'_>, n: u32) -> Result<u8> {
    u8::try_from(r.read_bits(n)?).map_err(|_| Error::Invalid("field wider than 8 bits"))
}

/// `ns(n)` (§ 4.11.8), `n >= 1`.
pub(crate) fn ns(r: &mut BitReader<'_>, n: u32) -> Result<u32> {
    if n == 0 {
        return Err(Error::Invalid("ns(0)"));
    }
    let w = n.ilog2() + 1;
    let m = (1u32 << w) - n;
    let v = r.read_bits(w - 1)?;
    if v < m {
        return Ok(v);
    }
    let extra = r.read_bits(1)?;
    Ok((v << 1) - m + extra)
}

/// `su(n)` (§ 4.11.7), `1 <= n <= 32`.
pub(crate) fn su(r: &mut BitReader<'_>, n: u32) -> Result<i32> {
    let value = i64::from(r.read_bits(n)?);
    let sign_mask = 1i64 << (n - 1);
    let v = if value & sign_mask != 0 { value - 2 * sign_mask } else { value };
    i32::try_from(v).map_err(|_| Error::Invalid("su value out of range"))
}

/// `seg_info( numSegments )` (§ 5.4.9).
pub(crate) fn seg_info(r: &mut BitReader<'_>, num_segments: u8) -> Result<SegInfo> {
    let mut info = SegInfo::default();
    for _ in 0..num_segments {
        let (mut enabled, mut data) = ([false; SEG_LVL_MAX], [0i32; SEG_LVL_MAX]);
        for j in 0..SEG_LVL_MAX {
            enabled[j] = flag(r)?;
            if enabled[j] {
                let bits = SEG_FEATURE_BITS[j];
                let limit = SEG_FEATURE_MAX[j];
                data[j] = if SEG_FEATURE_SIGNED[j] {
                    su(r, 1 + bits)?.clamp(-limit, limit)
                } else {
                    // f(0) reads nothing and yields 0
                    i32::try_from(r.read_bits(bits)?).map_err(|_| Error::Invalid("segment feature"))?.clamp(0, limit)
                };
            }
        }
        info.enabled.push(enabled);
        info.data.push(data);
    }
    Ok(info)
}

fn uvlc(r: &mut BitReader<'_>) -> Result<u32> {
    crate::seq::uvlc(r)
}

impl SequenceTools {
    pub(crate) fn parse(r: &mut BitReader<'_>, cx: Context) -> Result<Self> {
        let decoder_model = if cx.single_picture_header {
            DecoderModel::default()
        } else {
            let initial_display_delay = if flag(r)? { Some(bits_u8(r, 4)? + 1) } else { None };
            let (num_units_in_decoding_tick, info) = if flag(r)? {
                let ticks = r.read_bits(32)?;
                let info = if flag(r)? {
                    Some(DecoderModelInfo { decoder_buffer_delay: uvlc(r)?, encoder_buffer_delay: uvlc(r)?, low_delay_mode: flag(r)? })
                } else {
                    None
                };
                (Some(ticks), info)
            } else {
                (None, None)
            };
            DecoderModel { initial_display_delay, num_units_in_decoding_tick, info }
        };

        let (max_t, max_m) = (usize::from(cx.max_tlayer_id), usize::from(cx.max_mlayer_id));
        let mut tmap = [[[false; MAX_NUM_TLAYERS]; MAX_NUM_TLAYERS]; MAX_NUM_MLAYERS];
        for (m, layer) in tmap.iter_mut().enumerate() {
            for (curr, row) in layer.iter_mut().enumerate() {
                for (rf, v) in row.iter_mut().enumerate() {
                    *v = rf <= curr && curr <= max_t && m <= max_m;
                }
            }
        }
        let mut mmap = [[false; MAX_NUM_MLAYERS]; MAX_NUM_MLAYERS];
        for (curr, row) in mmap.iter_mut().enumerate() {
            for (rf, v) in row.iter_mut().enumerate() {
                *v = rf <= curr && curr <= max_m;
            }
        }
        if max_m > 0 && flag(r)? {
            for curr in 1..=max_m {
                for rf in (0..=curr).rev() {
                    mmap[curr][rf] = flag(r)?;
                }
            }
        }
        if max_t > 0 && flag(r)? {
            let multi = max_m > 0 && flag(r)?;
            for m in 0..=max_m {
                for curr in 1..=max_t {
                    for rf in (0..=curr).rev() {
                        tmap[m][curr][rf] = if multi || m == 0 { flag(r)? } else { tmap[0][curr][rf] };
                    }
                }
            }
        }
        let partition = Self::partition(r, cx)?;
        let segment = {
            let enable_ext_seg = flag(r)?;
            let max_segments = if enable_ext_seg { 16 } else { 8 };
            let seq_seg_info = if flag(r)? {
                let allow_change = flag(r)?;
                Some((allow_change, seg_info(r, max_segments)?))
            } else {
                None
            };
            SegmentConfig { enable_ext_seg, max_segments, seq_seg_info }
        };
        let intra = IntraConfig {
            enable_dip: flag(r)?,
            enable_intra_edge_filter: flag(r)?,
            enable_mrls: flag(r)?,
            enable_cfl_intra: flag(r)?,
            cfl_ds_filter_index: if cx.monochrome { 0 } else { bits_u8(r, 2)? },
            enable_mhccp: flag(r)?,
            enable_ibp: flag(r)?,
        };
        let inter = Self::inter(r, cx)?;
        let screen_content = Self::screen_content(r, cx)?;
        let tqe = Self::tqe(r, cx)?;
        let filter = Self::filter(r, cx, partition.sb_size)?;
        let seq_tile_info = if flag(r)? {
            let allow_tile_info_change = flag(r)?;
            let sb = partition.sb_size;
            Some(SeqTileInfo { allow_tile_info_change, params: tile_params(r, cx.max_frame_size, sb, sb, false, cx.limits)? })
        } else {
            None
        };
        let film_grain_params_present = flag(r)?;
        Ok(SequenceTools {
            decoder_model,
            mlayer_dependency_map: mmap,
            tlayer_dependency_map: tmap,
            partition,
            segment,
            intra,
            inter,
            screen_content,
            tqe,
            filter,
            seq_tile_info,
            film_grain_params_present,
        })
    }

    fn partition(r: &mut BitReader<'_>, cx: Context) -> Result<PartitionConfig> {
        let sb_size = if flag(r)? {
            SuperblockSize::Sb256
        } else if flag(r)? {
            SuperblockSize::Sb128
        } else {
            SuperblockSize::Sb64
        };
        let enable_sdp = !cx.monochrome && flag(r)?;
        let enable_extended_sdp = enable_sdp && !cx.single_picture_header && flag(r)?;
        let enable_ext_partitions = flag(r)?;
        let enable_uneven_4way_partitions = enable_ext_partitions && flag(r)?;
        let max_pb_aspect_ratio = if flag(r)? { 1 << (bits_u8(r, 1)? + 1) } else { 8 };
        Ok(PartitionConfig { sb_size, enable_sdp, enable_extended_sdp, enable_ext_partitions, enable_uneven_4way_partitions, max_pb_aspect_ratio })
    }

    fn drl_reorder(r: &mut BitReader<'_>) -> Result<DrlReorder> {
        Ok(if flag(r)? {
            DrlReorder::Disabled
        } else if flag(r)? {
            DrlReorder::Constraint
        } else {
            DrlReorder::Always
        })
    }

    fn inter(r: &mut BitReader<'_>, cx: Context) -> Result<InterConfig> {
        if cx.single_picture_header {
            let enable_refmvbank = flag(r)?;
            let drl_reorder = Self::drl_reorder(r)?;
            let seq_max_bvp_drl_bits_minus_1 = ns(r, MAX_REF_BV_STACK_SIZE - 1)?;
            let allow_frame_max_bvp_drl_bits = flag(r)?;
            let enable_bawp = flag(r)?;
            return Ok(InterConfig {
                seq_enabled_motion_modes: [false; MOTION_MODES],
                seq_frame_motion_modes_present: false,
                enable_six_param_warp_delta: false,
                enable_masked_compound: false,
                enable_ref_frame_mvs: false,
                reduced_ref_frame_mvs_mode: false,
                order_hint_bits: 0,
                enable_refmvbank,
                drl_reorder,
                explicit_ref_frame_map: false,
                num_ref_frames: 2,
                active_num_ref_frames: 2,
                long_term_frame_id_bits: 0,
                seq_max_drl_bits_minus_1: 0,
                allow_frame_max_drl_bits: false,
                seq_max_bvp_drl_bits_minus_1,
                allow_frame_max_bvp_drl_bits,
                num_same_ref_compound: 0,
                enable_tip: false,
                enable_tip_output: false,
                enable_tip_hole_fill: false,
                enable_mv_traj: false,
                enable_bawp,
                enable_cwp: false,
                enable_imp_msk_bld: false,
                enable_df_sub_pu: false,
                enable_tip_explicit_qp: false,
                enable_opfl_refine: 0,
                enable_refinemv: false,
                enable_tip_refinemv: false,
                enable_bru: false,
                enable_adaptive_mvd: false,
                enable_mvd_sign_derive: false,
                enable_flex_mvres: false,
                enable_global_motion: false,
                enable_short_refresh_frame_flags: false,
            });
        }
        let mut modes = [false; MOTION_MODES];
        for m in modes.iter_mut().skip(INTERINTRA) {
            *m = flag(r)?;
        }
        let any_mode = modes.iter().any(|&m| m);
        let seq_frame_motion_modes_present = any_mode && flag(r)?;
        let enable_six_param_warp_delta = modes[DELTAWARP] && flag(r)?;
        let enable_masked_compound = flag(r)?;
        let enable_ref_frame_mvs = flag(r)?;
        let reduced_ref_frame_mvs_mode = enable_ref_frame_mvs && flag(r)?;
        let order_hint_bits = bits_u8(r, 4)? + 1;
        let enable_refmvbank = flag(r)?;
        let drl_reorder = Self::drl_reorder(r)?;
        let explicit_ref_frame_map = flag(r)?;
        let num_ref_frames = if flag(r)? { bits_u8(r, 4)? + 1 } else { 8 };
        let active_num_ref_frames = REFS_PER_FRAME.min(num_ref_frames);
        let long_term_frame_id_bits = bits_u8(r, 3)?;
        let seq_max_drl_bits_minus_1 = ns(r, MAX_REF_MV_STACK_SIZE - 1)?;
        let allow_frame_max_drl_bits = flag(r)?;
        let seq_max_bvp_drl_bits_minus_1 = ns(r, MAX_REF_BV_STACK_SIZE - 1)?;
        let allow_frame_max_bvp_drl_bits = flag(r)?;
        let num_same_ref_compound = bits_u8(r, 2)?;
        let enable_tip = flag(r)?;
        let (enable_tip_output, enable_tip_hole_fill) = if enable_tip {
            let disable_tip_output = flag(r)?;
            (!disable_tip_output, flag(r)?)
        } else {
            (false, false)
        };
        let enable_mv_traj = flag(r)?;
        let enable_bawp = flag(r)?;
        let enable_cwp = flag(r)?;
        let enable_imp_msk_bld = flag(r)?;
        let enable_df_sub_pu = flag(r)?;
        let enable_tip_explicit_qp = enable_tip_output && enable_df_sub_pu && flag(r)?;
        let enable_opfl_refine = bits_u8(r, 2)?;
        let enable_refinemv = flag(r)?;
        let enable_tip_refinemv = enable_tip && (enable_opfl_refine != 0 || enable_refinemv) && flag(r)?;
        Ok(InterConfig {
            seq_enabled_motion_modes: modes,
            seq_frame_motion_modes_present,
            enable_six_param_warp_delta,
            enable_masked_compound,
            enable_ref_frame_mvs,
            reduced_ref_frame_mvs_mode,
            order_hint_bits,
            enable_refmvbank,
            drl_reorder,
            explicit_ref_frame_map,
            num_ref_frames,
            active_num_ref_frames,
            long_term_frame_id_bits,
            seq_max_drl_bits_minus_1,
            allow_frame_max_drl_bits,
            seq_max_bvp_drl_bits_minus_1,
            allow_frame_max_bvp_drl_bits,
            num_same_ref_compound,
            enable_tip,
            enable_tip_output,
            enable_tip_hole_fill,
            enable_mv_traj,
            enable_bawp,
            enable_cwp,
            enable_imp_msk_bld,
            enable_df_sub_pu,
            enable_tip_explicit_qp,
            enable_opfl_refine,
            enable_refinemv,
            enable_tip_refinemv,
            enable_bru: flag(r)?,
            enable_adaptive_mvd: flag(r)?,
            enable_mvd_sign_derive: flag(r)?,
            enable_flex_mvres: flag(r)?,
            enable_global_motion: flag(r)?,
            enable_short_refresh_frame_flags: flag(r)?,
        })
    }

    fn screen_content(r: &mut BitReader<'_>, cx: Context) -> Result<ScreenContentConfig> {
        if cx.single_picture_header {
            return Ok(ScreenContentConfig { seq_force_screen_content_tools: SELECT_SCREEN_CONTENT_TOOLS, seq_force_integer_mv: SELECT_INTEGER_MV });
        }
        let tools = if flag(r)? { SELECT_SCREEN_CONTENT_TOOLS } else { bits_u8(r, 1)? };
        let integer_mv = if tools > 0 { if flag(r)? { SELECT_INTEGER_MV } else { bits_u8(r, 1)? } } else { SELECT_INTEGER_MV };
        Ok(ScreenContentConfig { seq_force_screen_content_tools: tools, seq_force_integer_mv: integer_mv })
    }

    fn tqe(r: &mut BitReader<'_>, cx: Context) -> Result<TransformQuantEntropyConfig> {
        let mono = cx.monochrome;
        let enable_fsc = flag(r)?;
        let enable_idtx_intra = enable_fsc || flag(r)?;
        let enable_intra_ist = flag(r)?;
        let enable_inter_ist = flag(r)?;
        let enable_chroma_dctonly = !mono && flag(r)?;
        // not read with a single picture header; no inference is given, so it stays off
        let enable_inter_ddt = !cx.single_picture_header && flag(r)?;
        let reduced_tx_part_set = flag(r)?;
        let enable_cctx = !mono && flag(r)?;
        let enable_tcq = flag(r)?;
        let choose_tcq_per_frame = enable_tcq && !cx.single_picture_header && flag(r)?;
        let enable_parity_hiding = !(enable_tcq && !choose_tcq_per_frame) && flag(r)?;
        let (enable_avg_cdf, avg_cdf_type) = if cx.single_picture_header {
            (true, true)
        } else {
            let on = flag(r)?;
            (on, on && flag(r)?)
        };
        let separate_uv_delta_q = !mono && flag(r)?;
        let equal_ac_dc_q = flag(r)?;
        let (mut y_dc, mut uv_dc, mut uv_ac) = (0, 0, 0);
        let (mut y_dc_on, mut uv_dc_on, mut uv_ac_on) = (false, false, false);
        if !equal_ac_dc_q {
            y_dc = DELTA_DCQUANT_MIN + i32::from(bits_u8(r, 5)?);
            y_dc_on = flag(r)?;
        }
        if !mono {
            if !equal_ac_dc_q {
                uv_dc = DELTA_DCQUANT_MIN + i32::from(bits_u8(r, 5)?);
                uv_dc_on = flag(r)?;
            }
            uv_ac = DELTA_DCQUANT_MIN + i32::from(bits_u8(r, 5)?);
            uv_ac_on = flag(r)?;
            if equal_ac_dc_q {
                uv_dc = uv_ac;
            }
        }
        Ok(TransformQuantEntropyConfig {
            enable_fsc,
            enable_idtx_intra,
            enable_intra_ist,
            enable_inter_ist,
            enable_chroma_dctonly,
            enable_inter_ddt,
            reduced_tx_part_set,
            enable_cctx,
            enable_tcq,
            choose_tcq_per_frame,
            enable_parity_hiding,
            enable_avg_cdf,
            avg_cdf_type,
            separate_uv_delta_q,
            equal_ac_dc_q,
            base_y_dc_delta_q: y_dc,
            base_uv_dc_delta_q: uv_dc,
            base_uv_ac_delta_q: uv_ac,
            y_dc_delta_q_enabled: y_dc_on,
            uv_dc_delta_q_enabled: uv_dc_on,
            uv_ac_delta_q_enabled: uv_ac_on,
        })
    }

    fn filter(r: &mut BitReader<'_>, cx: Context, sb: SuperblockSize) -> Result<FilterConfig> {
        let disable_loopfilters_across_tiles = flag(r)?;
        let enable_cdef = flag(r)?;
        let enable_gdf = flag(r)?;
        let gdf_unit_matches_sb_size = enable_gdf && sb == SuperblockSize::Sb64 && flag(r)?;
        let enable_restoration = flag(r)?;
        let (mut pc, mut nonsep) = ([false; 2], [false; 2]);
        if enable_restoration {
            pc[0] = flag(r)?;
            nonsep[0] = flag(r)?;
            pc[1] = true;
            nonsep[1] = if flag(r)? { flag(r)? } else { nonsep[0] };
        }
        let enable_ccso = flag(r)?;
        let ccso_unit_matches_sb_size = enable_ccso && flag(r)?;
        let cdef_on_skip_txfm = if cx.single_picture_header {
            CdefOnSkipTxfm::Adaptive
        } else if flag(r)? {
            CdefOnSkipTxfm::AlwaysOn
        } else if flag(r)? {
            CdefOnSkipTxfm::Disabled
        } else {
            CdefOnSkipTxfm::Adaptive
        };
        let df_par_bits_minus_2 = bits_u8(r, 2)?;
        Ok(FilterConfig {
            disable_loopfilters_across_tiles,
            enable_cdef,
            enable_gdf,
            gdf_unit_matches_sb_size,
            enable_restoration,
            lr_disable_pc_wiener: pc,
            lr_disable_wiener_nonsep: nonsep,
            enable_ccso,
            ccso_unit_matches_sb_size,
            cdef_on_skip_txfm,
            df_par_bits_minus_2,
        })
    }
}
