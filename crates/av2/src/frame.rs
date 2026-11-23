//! Frame headers and tile group headers (AV2 v1.0.0 § 5.18, § 5.19).
//!
//! This first stage covers intra frames: closed-loop key frames and intra-only frames in regular
//! tile group OBUs, with the reference state their order hints need. Inter frames, open-loop key,
//! switch, RAS, bridge, TIP and show-existing frames, multi-frame headers and frame-level
//! Wiener filters are [`Error::Unsupported`] for now.

use cfa4_bitstream::BitReader;

use crate::obu::{Obu, ObuType};
use crate::seq::SequenceHeader;
use crate::seq_tools::{CdefOnSkipTxfm, SEG_LVL_MAX, SELECT_INTEGER_MV, SELECT_SCREEN_CONTENT_TOOLS, SuperblockSize, ns, seg_info};
use crate::tile::{TileLimits, TileParams, tile_log2, tile_params, uniform_spacing};
use crate::{Error, Result};

/// `NUM_REF_FRAMES`.
pub const NUM_REF_FRAMES: usize = 16;
/// `MAX_SEGMENTS`.
pub const MAX_SEGMENTS: usize = 16;
/// `SEG_LVL_ALT_Q`, `SEG_LVL_SKIP`.
const SEG_LVL_ALT_Q: usize = 0;
const SEG_LVL_SKIP: usize = 1;
/// `GDF_MIN_SIZE`.
const GDF_MIN_SIZE: u32 = 128;
/// `MI_SIZE_LOG2`.
const MI_SIZE_LOG2: u32 = 2;
/// `CCSO_INPUT_INTERVAL`.
const CCSO_INPUT_INTERVAL: u32 = 3;
/// `CCSO_Quant_Sz[ scale ][ quant ]`.
const CCSO_QUANT_SZ: [[u16; 4]; 4] = [[16, 8, 32, 0], [56, 40, 64, 128], [48, 24, 96, 192], [80, 112, 160, 256]];
/// `Ccso_Offset`.
const CCSO_OFFSET: [i32; 8] = [0, 1, -1, 3, -3, 7, -7, -10];
/// `RESTORATION_TILESIZE_MAX`.
const RESTORATION_TILESIZE_MAX: u32 = 512;
/// `RESTORE_*` frame restoration types.
pub const RESTORE_NONE: u8 = 0;
pub const RESTORE_PC_WIENER: u8 = 1;
pub const RESTORE_WIENER_NONSEP: u8 = 2;
pub const RESTORE_SWITCHABLE: u8 = 3;
/// `MAXQ_8_BITS`, `MAXQ_10_BITS`.
const MAXQ_8_BITS: i32 = 255;
const MAXQ_10_BITS: i32 = 255 + 2 * 24;

/// `FrameType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    Key,
    Inter,
    IntraOnly,
    Switch,
}

/// `quantization_params()` (§ 5.18.6.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuantParams {
    pub base_q_idx: u16,
    pub delta_q_y_dc: i32,
    pub delta_q_u_dc: i32,
    pub delta_q_u_ac: i32,
    pub delta_q_v_dc: i32,
    pub delta_q_v_ac: i32,
}

/// `segmentation_params()` (§ 5.18.7.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Segmentation {
    pub enabled: bool,
    pub update_map: bool,
    pub temporal_update: bool,
    pub feature_enabled: [[bool; SEG_LVL_MAX]; MAX_SEGMENTS],
    pub feature_data: [[i32; SEG_LVL_MAX]; MAX_SEGMENTS],
    pub seg_id_pre_skip: bool,
    pub last_active_seg_id: u8,
}

/// `setup_qm_params()` (§ 5.18.6.2) and the per-segment levels: `qm_y/u/v[ i ]`, then
/// `SegQMLevel[ plane ][ segmentId ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QmParams {
    pub levels: Vec<[u8; 3]>,
    pub seg_qm_level: Vec<[u8; 3]>,
}

/// `deblocking_filter_params()` (§ 5.18.5.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deblocking {
    pub apply: [bool; 4],
    pub delta_q: [i32; 4],
}

/// `gdf_params()` (§ 5.18.7.9) when `gdf_frame_enable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gdf {
    pub blk_size: u32,
    pub per_block: bool,
    pub pic_qc_idx: u8,
    pub pix_scale: u8,
}

/// `cdef_params()` (§ 5.18.7.10) when `cdef_frame_enable`: strengths as `[y_pri, y_sec, uv_pri, uv_sec]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cdef {
    pub damping: u8,
    pub on_skip_txfm: bool,
    pub strengths: Vec<[u8; 4]>,
}

/// `lr_params()` (§ 5.18.7.11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopRestoration {
    /// `FrameRestorationType[ plane ]`.
    pub types: [u8; 3],
    /// `LoopRestorationSize[ plane ]`.
    pub unit_size: [u32; 3],
}

/// `ccso_params()` (§ 5.18.7.12) for one plane with `ccso_planes[ plane ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CcsoPlane {
    pub bo_only: bool,
    pub scale_idx: u8,
    pub quant_idx: u8,
    pub ext_filter: u8,
    pub edge_clf: bool,
    pub max_band_log2: u8,
    /// `CcsoFilterOffset[ band ][ d0 ][ d1 ]`, flattened in loop order (d0, d1, band).
    pub offsets: Vec<i32>,
}

/// `frame_header_info()` of an intra frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub obu_type: ObuType,
    pub frame_type: FrameType,
    pub seq_header_id: u32,
    pub long_term_id: i32,
    pub immediate_output_frame: bool,
    pub implicit_output_frame: bool,
    pub sb_size: SuperblockSize,
    pub frame_size_override: bool,
    pub order_hint_lsbs: u32,
    pub order_hint: u32,
    pub refresh_frame_flags: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub mi_cols: u32,
    pub mi_rows: u32,
    pub allow_screen_content_tools: bool,
    pub force_integer_mv: bool,
    pub allow_intrabc: bool,
    pub allow_global_intrabc: bool,
    pub allow_local_intrabc: bool,
    pub max_bvp_drl_bits_minus_1: u32,
    pub disable_cdf_update: bool,
    pub tiles: TileParams,
    pub reuse_tile_info: bool,
    pub context_update_tile_id: u32,
    pub tile_size_bytes: u8,
    pub quant: QuantParams,
    pub segmentation: Segmentation,
    pub qm: Option<QmParams>,
    pub delta_q_present: bool,
    pub delta_q_res: u8,
    pub coded_lossless: bool,
    pub lossless: [bool; MAX_SEGMENTS],
    pub allow_tcq: bool,
    pub allow_parity_hiding: bool,
    pub deblocking: Deblocking,
    pub gdf: Option<Gdf>,
    pub cdef: Option<Cdef>,
    pub lr: LoopRestoration,
    pub ccso_luma_size_log2: u32,
    pub ccso: [Option<CcsoPlane>; 3],
    pub tx_mode_select: bool,
    pub reduced_tx_set: u8,
    /// `fgm_id` and `grain_seed` when `apply_grain`.
    pub film_grain: Option<(u8, u16)>,
    /// `NumFrameHeaderBits`.
    pub header_bits: usize,
}

impl FrameHeader {
    /// `MiColStarts[ i ]` for `i <= TileCols`.
    pub fn mi_col_starts(&self) -> Vec<u32> {
        mi_starts(&self.tiles.sb_col_starts, self.tiles.sb_shift, self.mi_cols)
    }

    /// `MiRowStarts[ i ]` for `i <= TileRows`.
    pub fn mi_row_starts(&self) -> Vec<u32> {
        mi_starts(&self.tiles.sb_row_starts, self.tiles.sb_shift, self.mi_rows)
    }
}

fn mi_starts(sb_starts: &[u32], shift: u32, end: u32) -> Vec<u32> {
    let mut v: Vec<u32> = sb_starts.iter().map(|s| s.checked_shl(shift).unwrap_or(u32::MAX)).collect();
    v.push(end);
    v
}

/// What `tile_group_obu()` (§ 5.19) says before the tile data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileGroup {
    /// The frame header, when this tile group carries the first copy of it.
    pub frame_header: Option<FrameHeader>,
    pub tg_start: u32,
    pub tg_end: u32,
    /// Byte offset of the tile data in the OBU payload.
    pub data_offset: usize,
    /// (offset, size) of each tile's data in the OBU payload.
    pub tiles: Vec<(usize, usize)>,
}

/// One reference slot as far as header parsing needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefSlot {
    pub valid: bool,
    pub order_hint: u32,
    pub frame_type: Option<FrameType>,
    pub tlayer_id: u8,
    pub mlayer_id: u8,
    pub implicit_output_frame: bool,
    pub immediate_output_frame: bool,
    pub long_term_id: i32,
    pub frame_width: u32,
    pub frame_height: u32,
}

/// The header-level decoder state across OBUs.
#[derive(Clone, Debug)]
pub struct HeaderParser {
    seq: Vec<Option<SequenceHeader>>,
    pub refs: [RefSlot; NUM_REF_FRAMES],
    first_picture_in_tu: bool,
    current: Option<FrameHeader>,
}

impl Default for HeaderParser {
    fn default() -> Self {
        Self::new()
    }
}

/// What [`HeaderParser::push`] did with an OBU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    SequenceHeader(u32),
    TemporalDelimiter,
    TileGroup(Box<TileGroup>),
    /// An OBU type that header parsing does not need.
    Skipped,
}

fn flag(r: &mut BitReader<'_>) -> Result<bool> {
    Ok(r.read_flag()?)
}

fn bits(r: &mut BitReader<'_>, n: u32) -> Result<u32> {
    if n > 32 {
        return Err(Error::Invalid("field wider than 32 bits"));
    }
    Ok(r.read_bits(n)?)
}

fn bits_u8(r: &mut BitReader<'_>, n: u32) -> Result<u8> {
    u8::try_from(bits(r, n)?).map_err(|_| Error::Invalid("field wider than 8 bits"))
}

fn uvlc(r: &mut BitReader<'_>) -> Result<u32> {
    crate::seq::uvlc(r)
}

/// `su(n)`.
fn su(r: &mut BitReader<'_>, n: u32) -> Result<i32> {
    crate::seq_tools::su(r, n)
}

/// `CeilLog2( x )`.
pub fn ceil_log2(x: u32) -> u32 {
    if x < 2 {
        return 0;
    }
    32 - (x - 1).leading_zeros()
}

/// `tu(mx)` (§ 4.11.9).
fn tu(r: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let mut v = 0;
    while v < max && flag(r)? {
        v += 1;
    }
    Ok(v)
}

/// `read_delta_q()` (§ 5.18.6.3).
fn read_delta_q(r: &mut BitReader<'_>) -> Result<i32> {
    if flag(r)? { su(r, 7) } else { Ok(0) }
}

fn block_width(sb: SuperblockSize) -> u32 {
    4 << sb.mi_log2()
}

impl HeaderParser {
    pub fn new() -> Self {
        HeaderParser { seq: vec![None; 16], refs: [RefSlot::default(); NUM_REF_FRAMES], first_picture_in_tu: true, current: None }
    }

    /// The sequence header with `id`, once seen.
    pub fn sequence_header(&self, id: u32) -> Option<&SequenceHeader> {
        self.seq.get(usize::try_from(id).ok()?).and_then(Option::as_ref)
    }

    /// Feed one OBU in stream order.
    pub fn push(&mut self, obu: &Obu<'_>) -> Result<Parsed> {
        let t = obu.header.obu_type;
        match t {
            ObuType::SequenceHeader => {
                let h = SequenceHeader::parse(obu.payload)?;
                let id = h.seq_header_id;
                let slot = self.seq.get_mut(usize::try_from(id).map_err(|_| Error::Invalid("seq_header_id"))?).ok_or(Error::Invalid("seq_header_id"))?;
                *slot = Some(h);
                Ok(Parsed::SequenceHeader(id))
            }
            ObuType::TemporalDelimiter => {
                self.first_picture_in_tu = true;
                Ok(Parsed::TemporalDelimiter)
            }
            ObuType::MultiFrameHeader => Err(Error::Unsupported("multi-frame header")),
            _ if t.is_tile_group() => self.tile_group(obu).map(|g| Parsed::TileGroup(Box::new(g))),
            ObuType::LeadingSef | ObuType::RegularSef | ObuType::LeadingTip | ObuType::RegularTip | ObuType::BridgeFrame => {
                Err(Error::Unsupported("show-existing, TIP and bridge frames"))
            }
            _ => Ok(Parsed::Skipped),
        }
    }

    /// `tile_group_obu( sz )` up to the tile data.
    fn tile_group(&mut self, obu: &Obu<'_>) -> Result<TileGroup> {
        let mut r = BitReader::new(obu.payload);
        let is_first = flag(&mut r)?;
        let present = is_first || flag(&mut r)?;
        let mut frame_header = None;
        if present {
            if is_first {
                let start = r.position();
                let mut h = self.frame_header_info(&mut r, obu)?;
                h.header_bits = r.position() - start;
                self.first_picture_in_tu = false;
                self.update_refs(&h, obu);
                self.current = Some(h.clone());
                frame_header = Some(h);
            } else {
                // frame_header_copy(): the same bits again
                let n = self.current.as_ref().ok_or(Error::Invalid("frame header copy before a frame header"))?.header_bits;
                r.skip(n)?;
            }
        }
        let h = self.current.as_ref().ok_or(Error::Invalid("tile group without a frame header"))?;
        let num_tiles = h.tiles.tile_cols * h.tiles.tile_rows;
        let (tg_start, tg_end) = if num_tiles > 1 && flag(&mut r)? {
            let n = h.tiles.tile_cols_log2 + h.tiles.tile_rows_log2;
            (bits(&mut r, n)?, bits(&mut r, n)?)
        } else {
            (0, num_tiles.saturating_sub(1))
        };
        if tg_start > tg_end || tg_end >= num_tiles {
            return Err(Error::Invalid("tile group range"));
        }
        // byte_alignment(): zero bits up to the next byte
        while !r.is_byte_aligned() {
            if flag(&mut r)? {
                return Err(Error::Invalid("byte alignment bit is one"));
            }
        }
        let data_offset = r.byte_pos();
        let mut tiles = Vec::new();
        let mut pos = data_offset;
        let size_bytes = usize::from(h.tile_size_bytes);
        for n in tg_start..=tg_end {
            let rest = obu.payload.len().checked_sub(pos).ok_or(Error::Truncated("tile data"))?;
            if n == tg_end {
                if rest == 0 {
                    return Err(Error::Truncated("tile data"));
                }
                tiles.push((pos, rest));
            } else {
                let field = obu.payload.get(pos..pos + size_bytes).ok_or(Error::Truncated("tile size"))?;
                let size = field.iter().rev().fold(0usize, |acc, &b| (acc << 8) | usize::from(b)) + 1;
                pos += size_bytes;
                if size > obu.payload.len() - pos {
                    return Err(Error::Invalid("tile size beyond the OBU"));
                }
                tiles.push((pos, size));
                pos += size;
            }
        }
        Ok(TileGroup { frame_header, tg_start, tg_end, data_offset, tiles })
    }

    /// The reference update for the header state (§ 7.20, the fields headers read back).
    fn update_refs(&mut self, h: &FrameHeader, obu: &Obu<'_>) {
        for (i, slot) in self.refs.iter_mut().enumerate() {
            if h.refresh_frame_flags & (1 << i) != 0 {
                *slot = RefSlot {
                    valid: true,
                    order_hint: h.order_hint,
                    frame_type: Some(h.frame_type),
                    tlayer_id: obu.header.tlayer_id,
                    mlayer_id: obu.header.mlayer_id,
                    implicit_output_frame: h.implicit_output_frame,
                    immediate_output_frame: h.immediate_output_frame,
                    long_term_id: h.long_term_id,
                    frame_width: h.frame_width,
                    frame_height: h.frame_height,
                };
            }
        }
    }

    /// `get_max_disp_order_hint( 1 )`.
    fn max_disp_order_hint(&self, seq: &SequenceHeader, obu: &Obu<'_>, num_ref_frames: usize) -> u32 {
        let (m, t) = (usize::from(obu.header.mlayer_id), usize::from(obu.header.tlayer_id));
        let tmap = &seq.tools.tlayer_dependency_map;
        let mmap = &seq.tools.mlayer_dependency_map;
        self.refs
            .iter()
            .take(num_ref_frames)
            .filter(|s| {
                s.valid
                    && tmap.get(m).and_then(|x| x.get(t)).and_then(|x| x.get(usize::from(s.tlayer_id))).copied().unwrap_or(false)
                    && mmap.get(m).and_then(|x| x.get(usize::from(s.mlayer_id))).copied().unwrap_or(false)
                    && (s.implicit_output_frame || s.immediate_output_frame)
            })
            .map(|s| s.order_hint)
            .max()
            .unwrap_or(0)
    }

    fn frame_header_info(&mut self, r: &mut BitReader<'_>, obu: &Obu<'_>) -> Result<FrameHeader> {
        let obu_type = obu.header.obu_type;
        match obu_type {
            ObuType::ClosedLoopKey | ObuType::RegularTileGroup => {}
            ObuType::OpenLoopKey => return Err(Error::Unsupported("open-loop key frames")),
            ObuType::Switch | ObuType::RasFrame => return Err(Error::Unsupported("switch and RAS frames")),
            _ => return Err(Error::Unsupported("leading frames")),
        }
        let key = obu_type == ObuType::ClosedLoopKey;
        if key && self.first_picture_in_tu {
            // startCVS
            for s in self.refs.iter_mut() {
                s.valid = false;
            }
        }
        let cur_mfh_id = uvlc(r)?;
        if cur_mfh_id != 0 {
            return Err(Error::Unsupported("multi-frame headers"));
        }
        let seq_header_id = uvlc(r)?;
        let seq = self.sequence_header(seq_header_id).ok_or(Error::Invalid("frame header names an unseen sequence header"))?.clone();
        let tools = &seq.tools;
        let inter = &tools.inter;
        let num_ref_frames = u32::from(inter.num_ref_frames).min(NUM_REF_FRAMES as u32);
        let all_frames = (1u32 << num_ref_frames) - 1;

        let (frame_type, long_term_id, immediate_output_frame, implicit_output_frame);
        if seq.single_picture_header {
            frame_type = FrameType::Key;
            long_term_id = -1;
            immediate_output_frame = true;
            implicit_output_frame = false;
        } else {
            frame_type = if key {
                FrameType::Key
            } else if flag(r)? {
                return Err(Error::Unsupported("inter frames"));
            } else {
                FrameType::IntraOnly
            };
            long_term_id = if frame_type == FrameType::Key {
                i32::try_from(bits(r, u32::from(inter.long_term_frame_id_bits))?).map_err(|_| Error::Invalid("long_term_id"))? - 1
            } else {
                -1
            };
            immediate_output_frame = flag(r)?;
            implicit_output_frame = !immediate_output_frame && !seq.monotonic_output_order && flag(r)?;
        }
        let sb_size = match tools.partition.sb_size {
            SuperblockSize::Sb256 => SuperblockSize::Sb128, // FrameIsIntra
            other => other,
        };
        let frame_size_override = !seq.single_picture_header && flag(r)?;
        let order_hint_lsbs = bits(r, u32::from(inter.order_hint_bits))?;
        let order_hint = if key {
            order_hint_lsbs
        } else {
            let max_disp = self.max_disp_order_hint(&seq, obu, num_ref_frames as usize);
            let ohb = u32::from(inter.order_hint_bits);
            let half = (1i64 << ohb) >> 1;
            let offset = i64::from(max_disp) - half - i64::from(order_hint_lsbs);
            let mut disp = i64::from(order_hint_lsbs);
            if offset >= 0 {
                disp += ((offset >> ohb) + 1) << ohb;
            }
            u32::try_from(disp).map_err(|_| Error::Invalid("order hint"))?
        };

        let refresh_frame_flags = if frame_type == FrameType::Key {
            if key && seq.max_mlayer_id == 0 {
                all_frames
            } else if inter.enable_short_refresh_frame_flags {
                1 << bits(r, ceil_log2(num_ref_frames))?
            } else {
                bits(r, num_ref_frames)?
            }
        } else if inter.enable_short_refresh_frame_flags {
            if flag(r)? { 1 << bits(r, ceil_log2(num_ref_frames))? } else { 0 }
        } else {
            bits(r, num_ref_frames)?
        };
        if refresh_frame_flags & !all_frames != 0 {
            return Err(Error::Invalid("refresh_frame_flags beyond NumRefFrames"));
        }

        // frame_size()
        let (frame_width, frame_height) = if frame_size_override {
            let (wb, hb) = seq.frame_size_bits;
            (bits(r, wb)?.saturating_add(1), bits(r, hb)?.saturating_add(1))
        } else {
            (seq.max_frame_width, seq.max_frame_height)
        };
        let mi_cols = 2 * (frame_width.saturating_add(7) >> 3);
        let mi_rows = 2 * (frame_height.saturating_add(7) >> 3);

        // screen_content_params()
        let sc = &tools.screen_content;
        let allow_screen_content_tools =
            if sc.seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS { flag(r)? } else { sc.seq_force_screen_content_tools != 0 };
        let force_integer_mv = allow_screen_content_tools && if sc.seq_force_integer_mv == SELECT_INTEGER_MV { flag(r)? } else { sc.seq_force_integer_mv != 0 };

        // intrabc_params()
        let allow_intrabc = flag(r)?;
        let (mut allow_global_intrabc, mut allow_local_intrabc) = (false, false);
        let mut max_bvp_drl_bits_minus_1 = inter.seq_max_bvp_drl_bits_minus_1;
        if allow_intrabc {
            allow_global_intrabc = flag(r)?;
            allow_local_intrabc = !allow_global_intrabc || flag(r)?;
            if inter.allow_frame_max_bvp_drl_bits && flag(r)? {
                max_bvp_drl_bits_minus_1 = ns(r, 2)?;
                if max_bvp_drl_bits_minus_1 >= inter.seq_max_bvp_drl_bits_minus_1 {
                    max_bvp_drl_bits_minus_1 += 1;
                }
            }
        }
        let disable_cdf_update = flag(r)?;

        // tile_info()
        let (tiles, reuse_tile_info) = Self::tile_info(r, &seq, (frame_width, frame_height), (mi_cols, mi_rows), sb_size)?;
        let (mut context_update_tile_id, mut tile_size_bytes) = (0, 4);
        if tiles.tile_cols > 1 || tiles.tile_rows > 1 {
            if !tools.tqe.enable_avg_cdf || !tools.tqe.avg_cdf_type {
                context_update_tile_id = bits(r, tiles.tile_rows_log2 + tiles.tile_cols_log2)?;
            }
            tile_size_bytes = bits_u8(r, 2)? + 1;
        }

        // quantization_params()
        let num_planes = seq.chroma_format.num_planes();
        let tqe = &tools.tqe;
        let base_q_idx = u16::try_from(bits(r, if seq.bit_depth == 8 { 8 } else { 9 })?).map_err(|_| Error::Invalid("base_q_idx"))?;
        let mut quant = QuantParams { base_q_idx, ..QuantParams::default() };
        if tqe.y_dc_delta_q_enabled {
            quant.delta_q_y_dc = read_delta_q(r)?;
        }
        if num_planes > 1 && (tqe.uv_ac_delta_q_enabled || tqe.uv_dc_delta_q_enabled) {
            let diff_uv_delta = tqe.separate_uv_delta_q && flag(r)?;
            if tqe.uv_dc_delta_q_enabled {
                quant.delta_q_u_dc = read_delta_q(r)?;
            }
            if tqe.uv_ac_delta_q_enabled {
                quant.delta_q_u_ac = read_delta_q(r)?;
            }
            if tqe.equal_ac_dc_q {
                quant.delta_q_u_dc = quant.delta_q_u_ac;
            }
            if diff_uv_delta {
                if tqe.uv_dc_delta_q_enabled {
                    quant.delta_q_v_dc = read_delta_q(r)?;
                }
                if tqe.uv_ac_delta_q_enabled {
                    quant.delta_q_v_ac = read_delta_q(r)?;
                }
                if tqe.equal_ac_dc_q {
                    quant.delta_q_v_dc = quant.delta_q_v_ac;
                }
            } else {
                quant.delta_q_v_dc = quant.delta_q_u_dc;
                quant.delta_q_v_ac = quant.delta_q_u_ac;
            }
        }

        // segmentation_params() with DerivedPrimaryRefFrame == PRIMARY_REF_NONE
        let max_segments = tools.segment.max_segments;
        let mut seg = Segmentation { enabled: flag(r)?, ..Default::default() };
        if seg.enabled {
            let (have, allow_change) = match &tools.segment.seq_seg_info {
                Some((allow, _)) => (true, *allow),
                None => (false, false),
            };
            let reuse = if allow_change { flag(r)? } else { have };
            let info = match (&tools.segment.seq_seg_info, reuse) {
                (Some((_, info)), true) => info.clone(),
                _ => seg_info(r, max_segments)?,
            };
            for (i, (e, d)) in info.enabled.iter().zip(info.data.iter()).enumerate().take(MAX_SEGMENTS) {
                seg.feature_enabled[i] = *e;
                seg.feature_data[i] = *d;
            }
            seg.update_map = true;
        }
        for i in 0..usize::from(max_segments).min(MAX_SEGMENTS) {
            for j in 0..SEG_LVL_MAX {
                if seg.feature_enabled[i][j] {
                    seg.last_active_seg_id = i as u8;
                    if j >= SEG_LVL_SKIP {
                        seg.seg_id_pre_skip = true;
                    }
                }
            }
        }

        // setup_qm_params()
        let qm_levels = if flag(r)? {
            let num = if seg.enabled { bits(r, 2)? + 1 } else { 1 };
            let mut levels = Vec::new();
            for _ in 0..num {
                let y = bits_u8(r, 4)?;
                let (u, v) = if num_planes > 1 {
                    if flag(r)? {
                        (y, y)
                    } else {
                        let u = bits_u8(r, 4)?;
                        (u, if tqe.separate_uv_delta_q { bits_u8(r, 4)? } else { u })
                    }
                } else {
                    (y, y)
                };
                levels.push([y, u, v]);
            }
            Some(levels)
        } else {
            None
        };

        // delta_q_params()
        let delta_q_present = base_q_idx > 0 && flag(r)?;
        let delta_q_res = if delta_q_present { bits_u8(r, 2)? } else { 0 };

        let max_q = if seq.bit_depth == 8 { MAXQ_8_BITS } else { MAXQ_10_BITS };
        let mut coded_lossless = true;
        let mut lossless = [false; MAX_SEGMENTS];
        let mut seg_qm_level = Vec::new();
        for (sid, l) in lossless.iter_mut().enumerate().take(usize::from(max_segments)) {
            let qindex = if seg.enabled && seg.feature_enabled[sid][SEG_LVL_ALT_Q] {
                (i32::from(base_q_idx) + seg.feature_data[sid][SEG_LVL_ALT_Q]).clamp(0, max_q)
            } else {
                i32::from(base_q_idx)
            };
            *l = qindex == 0
                && !delta_q_present
                && quant.delta_q_y_dc + tqe.base_y_dc_delta_q <= 0
                && quant.delta_q_u_dc + tqe.base_uv_dc_delta_q <= 0
                && quant.delta_q_v_dc + tqe.base_uv_dc_delta_q <= 0
                && quant.delta_q_u_ac + tqe.base_uv_ac_delta_q <= 0
                && quant.delta_q_v_ac + tqe.base_uv_ac_delta_q <= 0;
            if !*l {
                coded_lossless = false;
            }
            if let Some(levels) = &qm_levels {
                if *l {
                    seg_qm_level.push([15; 3]);
                } else {
                    let n = u32::try_from(levels.len()).map_err(|_| Error::Invalid("qm count"))?;
                    let idx = bits(r, ceil_log2(n))? as usize;
                    seg_qm_level.push(*levels.get(idx).ok_or(Error::Invalid("qm_index"))?);
                }
            }
        }
        let qm = qm_levels.map(|levels| QmParams { levels, seg_qm_level });
        let allow_tcq = if coded_lossless {
            false
        } else if tqe.choose_tcq_per_frame {
            flag(r)?
        } else {
            tqe.enable_tcq
        };
        let allow_parity_hiding = !coded_lossless && tqe.enable_parity_hiding && !allow_tcq && flag(r)?;

        let deblocking = if coded_lossless { Deblocking::default() } else { Self::deblocking(r, &seq)? };
        let mut h = FrameHeader {
            obu_type,
            frame_type,
            seq_header_id,
            long_term_id,
            immediate_output_frame,
            implicit_output_frame,
            sb_size,
            frame_size_override,
            order_hint_lsbs,
            order_hint,
            refresh_frame_flags,
            frame_width,
            frame_height,
            mi_cols,
            mi_rows,
            allow_screen_content_tools,
            force_integer_mv,
            allow_intrabc,
            allow_global_intrabc,
            allow_local_intrabc,
            max_bvp_drl_bits_minus_1,
            disable_cdf_update,
            tiles,
            reuse_tile_info,
            context_update_tile_id,
            tile_size_bytes,
            quant,
            segmentation: seg,
            qm,
            delta_q_present,
            delta_q_res,
            coded_lossless,
            lossless,
            allow_tcq,
            allow_parity_hiding,
            deblocking,
            gdf: None,
            cdef: None,
            lr: LoopRestoration::default(),
            ccso_luma_size_log2: 0,
            ccso: [None, None, None],
            tx_mode_select: false,
            reduced_tx_set: 0,
            film_grain: None,
            header_bits: 0,
        };
        h.gdf = Self::gdf(r, &seq, &h)?;
        h.cdef = Self::cdef(r, &seq, &h)?;
        h.lr = Self::lr(r, &seq, &h)?;
        Self::ccso(r, &seq, &mut h)?;
        h.tx_mode_select = !coded_lossless && flag(r)?;
        // frame_reference_mode(), skip_mode_params(), allow_bawp, allow_warpmv_mode: nothing for intra
        h.reduced_tx_set = bits_u8(r, 2)?;
        // global_motion_params(): nothing for intra
        // film_grain_config()
        if tools.film_grain_params_present && (immediate_output_frame || implicit_output_frame) {
            let apply = seq.single_picture_header || flag(r)?;
            if apply {
                let fgm_id = bits_u8(r, 3)?;
                let seed = u16::try_from(bits(r, 16)?).map_err(|_| Error::Invalid("grain_seed"))?;
                h.film_grain = Some((fgm_id, seed));
            }
        }
        Ok(h)
    }

    /// `tile_info()` (§ 5.18.7.2) without the trailing `context_update_tile_id` fields.
    fn tile_info(
        r: &mut BitReader<'_>,
        seq: &SequenceHeader,
        size: (u32, u32),
        (mi_cols, mi_rows): (u32, u32),
        sb_size: SuperblockSize,
    ) -> Result<(TileParams, bool)> {
        let seq_sb = seq.tools.partition.sb_size;
        let shift = sb_size.mi_log2();
        let sb_cols = (mi_cols + (1 << shift) - 1) >> shift;
        let sb_rows = (mi_rows + (1 << shift) - 1) >> shift;
        let eligible = |log2: u32, sbs: u32| {
            let n = 1u32.checked_shl(log2).unwrap_or(u32::MAX);
            let w = sbs.saturating_add(n - 1).checked_shr(log2).unwrap_or(0);
            let last = i64::from(sbs) - i64::from(n - 1) * i64::from(w);
            w >= 1 && last >= 1
        };
        let reuse = match &seq.tools.seq_tile_info {
            Some(info) => {
                let p = &info.params;
                let have = if p.uniform_tile_spacing {
                    eligible(p.tile_rows_log2, sb_rows) && eligible(p.tile_cols_log2, sb_cols)
                } else {
                    p.sb_cols == sb_cols && p.sb_rows == sb_rows
                };
                have && (!info.allow_tile_info_change || flag(r)?)
            }
            None => false,
        };
        let limits = TileLimits { level_idx: seq.level_idx, tier: seq.tier };
        let params = match (&seq.tools.seq_tile_info, reuse) {
            (Some(info), true) => {
                let p = &info.params;
                if p.uniform_tile_spacing {
                    let cols = uniform_spacing(p.tile_cols_log2, mi_cols, seq_sb)?;
                    let rows = uniform_spacing(p.tile_rows_log2, mi_rows, seq_sb)?;
                    let (tc, tr) = (cols.len() as u32, rows.len() as u32);
                    TileParams {
                        sb_rows,
                        tile_rows: tr,
                        tile_rows_log2: tile_log2(1, u64::from(tr)),
                        sb_cols,
                        tile_cols: tc,
                        tile_cols_log2: tile_log2(1, u64::from(tc)),
                        uniform_tile_spacing: true,
                        sb_shift: seq_sb.mi_log2(),
                        sb_col_starts: cols,
                        sb_row_starts: rows,
                    }
                } else {
                    TileParams { sb_shift: shift, ..p.clone() }
                }
            }
            _ => tile_params(r, size, seq_sb, sb_size, false, limits)?,
        };
        Ok((params, reuse))
    }

    /// `deblocking_filter_params()` for a frame that is not coded lossless (intra: no `allow_df_sub_pu`).
    fn deblocking(r: &mut BitReader<'_>, seq: &SequenceHeader) -> Result<Deblocking> {
        let mut d = Deblocking::default();
        d.apply[0] = flag(r)?;
        d.apply[1] = flag(r)?;
        if seq.chroma_format.num_planes() > 1 && (d.apply[0] || d.apply[1]) {
            d.apply[2] = flag(r)?;
            d.apply[3] = flag(r)?;
        }
        let par_bits = u32::from(seq.tools.filter.df_par_bits_minus_2) + 2;
        for i in 0..4 {
            d.delta_q[i] = if d.apply[i] {
                if flag(r)? {
                    i32::try_from(bits(r, par_bits)?).map_err(|_| Error::Invalid("df_delta_q"))? - (1 << (par_bits - 1))
                } else if i == 1 {
                    d.delta_q[0]
                } else {
                    0
                }
            } else {
                0
            };
        }
        Ok(d)
    }

    fn tile_starts_or(h: &FrameHeader) -> u32 {
        let cols = h.mi_col_starts();
        let rows = h.mi_row_starts();
        let tc = h.tiles.tile_cols as usize;
        let tr = h.tiles.tile_rows as usize;
        cols.iter().take(tc).chain(rows.iter().take(tr)).fold(0, |a, &v| a | v)
    }

    fn gdf(r: &mut BitReader<'_>, seq: &SequenceHeader, h: &FrameHeader) -> Result<Option<Gdf>> {
        let f = &seq.tools.filter;
        if h.coded_lossless || !f.enable_gdf {
            return Ok(None);
        }
        if !(seq.single_picture_header || flag(r)?) {
            return Ok(None);
        }
        let mut blk = block_width(h.sb_size).max(GDF_MIN_SIZE);
        if f.gdf_unit_matches_sb_size {
            blk = block_width(h.sb_size);
        } else if h.sb_size == SuperblockSize::Sb64 && Self::tile_starts_or(h) & 16 != 0 {
            blk = 64;
        }
        let multi = h.tiles.tile_rows > 1 || h.tiles.tile_cols > 1;
        let per_block = if h.mi_cols * 4 > blk || h.mi_rows * 4 > blk || (f.disable_loopfilters_across_tiles && multi) { flag(r)? } else { false };
        let pic_qc_idx = bits_u8(r, 2)?;
        let pix_scale = 1 + bits_u8(r, 2)?;
        Ok(Some(Gdf { blk_size: blk, per_block, pic_qc_idx, pix_scale }))
    }

    fn cdef(r: &mut BitReader<'_>, seq: &SequenceHeader, h: &FrameHeader) -> Result<Option<Cdef>> {
        let f = &seq.tools.filter;
        if h.coded_lossless || !f.enable_cdef {
            return Ok(None);
        }
        if !(seq.single_picture_header || flag(r)?) {
            return Ok(None);
        }
        let damping = bits_u8(r, 2)? + 3;
        let count = bits(r, 3)? + 1;
        let on_skip_txfm = match f.cdef_on_skip_txfm {
            CdefOnSkipTxfm::Adaptive => flag(r)?,
            CdefOnSkipTxfm::AlwaysOn => true,
            CdefOnSkipTxfm::Disabled => false,
        };
        let sec = |r: &mut BitReader<'_>| -> Result<u8> {
            let v = bits_u8(r, 2)?;
            Ok(if v == 3 { 4 } else { v })
        };
        let mut strengths = Vec::new();
        for _ in 0..count {
            let y_pri = if flag(r)? { 0 } else { bits_u8(r, 4)? };
            let y_sec = sec(r)?;
            let (uv_pri, uv_sec) = if seq.chroma_format.num_planes() > 1 {
                let p = if flag(r)? { 0 } else { bits_u8(r, 4)? };
                (p, sec(r)?)
            } else {
                (0, 0)
            };
            strengths.push([y_pri, y_sec, uv_pri, uv_sec]);
        }
        Ok(Some(Cdef { damping, on_skip_txfm, strengths }))
    }

    fn lr(r: &mut BitReader<'_>, seq: &SequenceHeader, h: &FrameHeader) -> Result<LoopRestoration> {
        let f = &seq.tools.filter;
        let mut lr = LoopRestoration::default();
        if h.coded_lossless || !f.enable_restoration {
            return Ok(lr);
        }
        let num_planes = usize::from(seq.chroma_format.num_planes());
        let (mut luma, mut chroma) = (false, false);
        for plane in 0..num_planes {
            let c = usize::from(plane > 0);
            let disabled = [false, f.lr_disable_pc_wiener[c], f.lr_disable_wiener_nonsep[c]];
            let mut index_to_tool = vec![RESTORE_NONE];
            for (i, tool) in [RESTORE_PC_WIENER, RESTORE_WIENER_NONSEP].into_iter().enumerate() {
                if !disabled[i + 1] {
                    index_to_tool.push(tool);
                }
            }
            let tools_count = index_to_tool.len() as u32;
            index_to_tool.push(RESTORE_SWITCHABLE);
            let allow_switchable = u32::from(tools_count > 2);
            let idx = ns(r, tools_count + allow_switchable)? as usize;
            let t = *index_to_tool.get(idx).ok_or(Error::Invalid("tool_index"))?;
            lr.types[plane] = t;
            if t != RESTORE_NONE {
                if plane == 0 {
                    luma = true;
                } else {
                    chroma = true;
                }
            }
            if (t == RESTORE_WIENER_NONSEP || t == RESTORE_SWITCHABLE) && flag(r)? {
                // frame_filters_on: the frame-level Wiener NS filter bank comes next
                return Err(Error::Unsupported("frame-level Wiener filters"));
            }
        }
        let (sx, sy) = seq.chroma_format.subsampling();
        let sub = u32::from(sx.max(sy));
        lr.unit_size[0] = RESTORATION_TILESIZE_MAX >> 3;
        lr.unit_size[1] = RESTORATION_TILESIZE_MAX >> (3 + sub);
        let shift = |r: &mut BitReader<'_>| -> Result<u32> {
            // half size, else max size (implied for 256x256 superblocks),
            // else quarter size (implied for 128x128), else eighth size
            if flag(r)? {
                return Ok(1);
            }
            if h.sb_size == SuperblockSize::Sb256 || flag(r)? {
                return Ok(0);
            }
            if h.sb_size == SuperblockSize::Sb128 || flag(r)? {
                return Ok(2);
            }
            Ok(3)
        };
        if luma {
            lr.unit_size[0] = RESTORATION_TILESIZE_MAX >> shift(r)?;
        }
        if chroma {
            lr.unit_size[1] = (RESTORATION_TILESIZE_MAX >> sub) >> shift(r)?;
        }
        lr.unit_size[2] = lr.unit_size[1];
        Ok(lr)
    }

    fn ccso(r: &mut BitReader<'_>, seq: &SequenceHeader, h: &mut FrameHeader) -> Result<()> {
        let f = &seq.tools.filter;
        if h.coded_lossless || !f.enable_ccso {
            return Ok(());
        }
        let a = Self::tile_starts_or(h);
        h.ccso_luma_size_log2 = if f.ccso_unit_matches_sb_size {
            h.sb_size.mi_log2() + MI_SIZE_LOG2
        } else if a & 63 == 0 {
            8
        } else if a & 31 == 0 {
            7
        } else {
            6
        };
        if !(seq.single_picture_header || flag(r)?) {
            return Ok(());
        }
        for plane in 0..usize::from(seq.chroma_format.num_planes()) {
            if !flag(r)? {
                continue;
            }
            // intra: no reuse_ccso / sb_reuse_ccso
            let bo_only = flag(r)?;
            let scale_idx = bits_u8(r, 2)?;
            let (mut quant_idx, mut ext_filter, mut edge_clf) = (0, 0, false);
            if !bo_only {
                quant_idx = bits_u8(r, 2)?;
                ext_filter = bits_u8(r, 3)?;
                let step = CCSO_QUANT_SZ[usize::from(scale_idx)][usize::from(quant_idx)];
                edge_clf = step != 0 && flag(r)?;
            }
            let max_band_log2 = bits_u8(r, 2 + u32::from(bo_only))?;
            let edges = if bo_only { 1 } else { CCSO_INPUT_INTERVAL - u32::from(edge_clf) };
            let bands = 1u32 << max_band_log2;
            let mut offsets = Vec::new();
            for _ in 0..edges * edges * bands {
                let idx = tu(r, 7)? as usize;
                offsets.push(CCSO_OFFSET[idx] * (i32::from(scale_idx) + 1));
            }
            if let Some(slot) = h.ccso.get_mut(plane) {
                *slot = Some(CcsoPlane { bo_only, scale_idx, quant_idx, ext_filter, edge_clf, max_band_log2, offsets });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_log2_matches_definition() {
        for (x, want) in [(0, 0), (1, 0), (2, 1), (3, 2), (4, 2), (5, 3), (8, 3), (9, 4), (16, 4)] {
            assert_eq!(ceil_log2(x), want, "{x}");
        }
    }

    #[test]
    fn tu_stops_at_max() {
        let data = [0xFF, 0x00];
        let mut r = BitReader::new(&data);
        assert_eq!(tu(&mut r, 7).unwrap(), 7);
        assert_eq!(r.position(), 7);
        let data = [0b1100_0000];
        let mut r = BitReader::new(&data);
        assert_eq!(tu(&mut r, 7).unwrap(), 2);
        assert_eq!(r.position(), 3);
    }
}
