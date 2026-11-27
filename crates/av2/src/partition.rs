//! Block partition syntax (spec §5.20.3): `partition_implied`, the allowed-partition rules and
//! `read_partition` with the CDF selection of §8.3.2 for its syntax elements.
//!
//! This module reads one partition decision. The recursion of `decode_partition` and the block
//! syntax below it follow in later modules.

use crate::block_tables::*;
use crate::cdf_default::{
    DEFAULT_DO_EXT_PARTITION_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF, DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF, DEFAULT_RECT_TYPE_CDF,
};
use crate::cdf_set::CdfSet;
use crate::symbol::SymbolDecoder;
use crate::{Error, Result};

pub const PARTITION_NONE: u8 = 0;
pub const PARTITION_HORZ: u8 = 1;
pub const PARTITION_VERT: u8 = 2;
pub const PARTITION_HORZ_3: u8 = 3;
pub const PARTITION_VERT_3: u8 = 4;
pub const PARTITION_HORZ_4A: u8 = 5;
pub const PARTITION_HORZ_4B: u8 = 6;
pub const PARTITION_VERT_4A: u8 = 7;
pub const PARTITION_VERT_4B: u8 = 8;
pub const PARTITION_SPLIT: u8 = 9;
/// `EXT_PARTITION_TYPES`.
pub const EXT_PARTITION_TYPES: usize = 10;

pub const RECT_HORZ: u8 = 0;
pub const RECT_VERT: u8 = 1;
pub const RECT_INVALID: u8 = 2;

/// `TreeType`: which planes a partition tree codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeType {
    Shared,
    Luma,
    Chroma,
}

/// `RegionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionType {
    Intra,
    Mixed,
}

/// Frame and sequence parameters the partition syntax depends on.
#[derive(Clone, Copy, Debug)]
pub struct PartitionGeom {
    pub mi_rows: u32,
    pub mi_cols: u32,
    pub sub_x: bool,
    pub sub_y: bool,
    pub num_planes: u32,
    pub frame_is_intra: bool,
    pub enable_ext_partitions: bool,
    pub enable_uneven_4way_partitions: bool,
    /// `MaxPbAspectRatio`.
    pub max_pb_aspect_ratio: u32,
}

/// The decoder state variables read by the partition syntax.
#[derive(Clone, Copy, Debug)]
pub struct PartitionState {
    pub tree_type: TreeType,
    pub region_type: RegionType,
    /// `PlaneStart` (0 or 1 selects the CDF set).
    pub plane_start: usize,
    /// `bru_mode == BRU_ACTIVE`.
    pub bru_active: bool,
}

/// What `read_partition` needs to know about already decoded neighbours.
pub trait PartitionNeighbours {
    /// `AboveMiSizes[ plane_start ][ c ]`.
    fn above_mi_size(&self, plane_start: usize, c: u32) -> u8;
    /// `LeftMiSizes[ plane_start ][ r ]`.
    fn left_mi_size(&self, plane_start: usize, r: u32) -> u8;
    /// `MiSizes[ plane_start ][ r ][ c ]`.
    fn mi_size(&self, plane_start: usize, r: u32, c: u32) -> u8;
    /// `ChromaPartitionKnown[ r ][ c ]` and then `LumaPartitions[ r ][ c ]`.
    fn forced_chroma_partition(&self, r: u32, c: u32) -> Option<u8>;
}

const PARTITION_SIZE_ADJUST: [u8; 29] = [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0];
const PARTITION_SIZE_ADJUST_RECT_TYPE: [u8; 29] = [0, 0, 0, 0, 1, 2, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 13, 14, 13, 14, 0, 0, 0, 0];

/// `Block_Width[ b ]`.
pub fn block_width(b: u8) -> u32 {
    4 * u32::from(NUM_4X4_WIDE.get(usize::from(b)).copied().unwrap_or(1))
}

/// `Block_Height[ b ]`.
pub fn block_height(b: u8) -> u32 {
    4 * u32::from(NUM_4X4_HIGH.get(usize::from(b)).copied().unwrap_or(1))
}

fn n4w(b: u8) -> u32 {
    u32::from(NUM_4X4_WIDE.get(usize::from(b)).copied().unwrap_or(1))
}

fn n4h(b: u8) -> u32 {
    u32::from(NUM_4X4_HIGH.get(usize::from(b)).copied().unwrap_or(1))
}

fn mi_w_log2(b: u8) -> i32 {
    i32::from(MI_WIDTH_LOG2.get(usize::from(b)).copied().unwrap_or(0))
}

fn mi_h_log2(b: u8) -> i32 {
    i32::from(MI_HEIGHT_LOG2.get(usize::from(b)).copied().unwrap_or(0))
}

/// `Partition_Subsize[ p ][ b ]`.
pub fn partition_subsize(p: u8, b: u8) -> u8 {
    PARTITION_SUBSIZE.get(usize::from(p)).and_then(|row| row.get(usize::from(b))).copied().unwrap_or(BLOCK_INVALID)
}

fn h_midsize(b: u8) -> u8 {
    H_PARTITION_MIDSIZE.get(usize::from(b)).copied().unwrap_or(BLOCK_INVALID)
}

/// `get_plane_residual_size( subsize, 1 )`.
pub fn chroma_residual_size(g: &PartitionGeom, sub: u8) -> u8 {
    if sub == BLOCK_INVALID {
        return BLOCK_INVALID;
    }
    let i = usize::from(sub) * 4 + usize::from(g.sub_x) * 2 + usize::from(g.sub_y);
    SUBSAMPLED_SIZE.get(i).copied().unwrap_or(BLOCK_INVALID)
}

fn is_horz(p: u8) -> bool {
    matches!(p, PARTITION_HORZ | PARTITION_HORZ_3 | PARTITION_HORZ_4A | PARTITION_HORZ_4B)
}

fn is_vert(p: u8) -> bool {
    matches!(p, PARTITION_VERT | PARTITION_VERT_3 | PARTITION_VERT_4A | PARTITION_VERT_4B)
}

fn block_coded(g: &PartitionGeom, r: u32, c: u32) -> bool {
    r < g.mi_rows && c < g.mi_cols
}

/// `rect_type_implied_by_bsize( bSize )`.
pub fn rect_type_implied_by_bsize(st: &PartitionState, b: u8) -> u8 {
    if matches!(b, BLOCK_4X8 | BLOCK_64X128 | BLOCK_128X256 | BLOCK_4X16) {
        return RECT_HORZ;
    }
    if matches!(b, BLOCK_8X4 | BLOCK_128X64 | BLOCK_256X128 | BLOCK_16X4) {
        return RECT_VERT;
    }
    if st.tree_type == TreeType::Chroma {
        if matches!(b, BLOCK_8X16 | BLOCK_8X32) {
            return RECT_HORZ;
        }
        if matches!(b, BLOCK_16X8 | BLOCK_32X8) {
            return RECT_VERT;
        }
    }
    RECT_INVALID
}

fn check_chroma(g: &PartitionGeom, st: &PartitionState, b: u8) -> bool {
    if chroma_residual_size(g, b) == BLOCK_INVALID {
        return false;
    }
    st.tree_type == TreeType::Luma && block_width(b) >= 64 && block_height(b) >= 64
}

fn is_chroma_offset_for_subsize(g: &PartitionGeom, sub: u8) -> bool {
    (g.sub_y && mi_h_log2(sub) == 0) || (g.sub_x && mi_w_log2(sub) == 0)
}

/// `is_chroma_offset_for_partition( p, bSize )`.
pub fn is_chroma_offset_for_partition(g: &PartitionGeom, p: u8, b: u8) -> bool {
    if is_chroma_offset_for_subsize(g, partition_subsize(p, b)) {
        return true;
    }
    if p == PARTITION_HORZ_3 {
        let middle_chroma = b == BLOCK_8X32 && g.sub_x;
        if !middle_chroma && is_chroma_offset_for_subsize(g, h_midsize(b)) {
            return true;
        }
    }
    false
}

fn is_ext_partition_allowed(g: &PartitionGeom, st: &PartitionState, b: u8, rect: u8) -> bool {
    if !g.enable_ext_partitions {
        return false;
    }
    st.tree_type != TreeType::Chroma
        || (rect == RECT_HORZ && block_height(b) > 16 && block_width(b) > 8)
        || (rect == RECT_VERT && block_width(b) > 16 && block_height(b) > 8)
}

fn is_uneven_4way_partition_allowed(g: &PartitionGeom, st: &PartitionState, b: u8, rect: u8) -> bool {
    if !g.enable_uneven_4way_partitions {
        return false;
    }
    st.tree_type != TreeType::Chroma || (rect == RECT_HORZ && block_height(b) == 64) || (rect == RECT_VERT && block_width(b) == 64)
}

/// `is_partition_allowed(...)`.
#[allow(clippy::too_many_arguments)]
pub fn is_partition_allowed(g: &PartitionGeom, st: &PartitionState, r: u32, c: u32, p: u8, b: u8, mut chroma_offset: bool, has_chroma: bool) -> bool {
    let sub = partition_subsize(p, b);
    if sub == BLOCK_INVALID {
        return false;
    }
    if !g.frame_is_intra && st.region_type == RegionType::Mixed && sub == BLOCK_4X4 {
        return false;
    }
    let rect = rect_type_implied_by_bsize(st, b);
    if (rect == RECT_VERT && is_horz(p)) || (rect == RECT_HORZ && is_vert(p)) {
        return false;
    }
    let (bw, bh) = (block_width(sub), block_height(sub));
    let ratio = g.max_pb_aspect_ratio;
    if bw > bh * ratio || bh > bw * ratio {
        if p == PARTITION_NONE {
            return false;
        }
        if bw >= bh * 8 || bh >= bw * 8 {
            return false;
        }
    }
    let half_w = n4w(b) >> 1;
    let half_h = n4h(b) >> 1;
    if has_chroma && st.tree_type != TreeType::Chroma && !chroma_offset {
        chroma_offset = is_chroma_offset_for_partition(g, p, b);
    }
    if ((has_chroma && !chroma_offset && st.tree_type != TreeType::Luma) || check_chroma(g, st, b)) && chroma_residual_size(g, sub) == BLOCK_INVALID {
        return false;
    }
    match p {
        PARTITION_HORZ_3 => {
            if !is_ext_partition_allowed(g, st, b, RECT_HORZ) {
                return false;
            }
        }
        PARTITION_VERT_3 => {
            if !is_ext_partition_allowed(g, st, b, RECT_VERT) {
                return false;
            }
        }
        PARTITION_HORZ_4A | PARTITION_HORZ_4B => {
            if !is_ext_partition_allowed(g, st, b, RECT_HORZ) || !is_uneven_4way_partition_allowed(g, st, b, RECT_HORZ) {
                return false;
            }
        }
        PARTITION_VERT_4A | PARTITION_VERT_4B => {
            if !is_ext_partition_allowed(g, st, b, RECT_VERT) || !is_uneven_4way_partition_allowed(g, st, b, RECT_VERT) {
                return false;
            }
        }
        PARTITION_NONE => {
            let has_rows = r + half_h < g.mi_rows;
            let has_cols = c + half_w < g.mi_cols;
            if (st.tree_type != TreeType::Chroma || b != BLOCK_8X8) && (!has_rows || !has_cols) {
                return false;
            }
        }
        _ => {}
    }
    if has_chroma && st.tree_type != TreeType::Luma && g.num_planes > 1 && chroma_offset {
        match p {
            PARTITION_HORZ => return block_coded(g, r + half_h, c),
            PARTITION_VERT => return block_coded(g, r, c + half_w),
            PARTITION_HORZ_3 => return block_coded(g, r + 3 * (half_h >> 1), c),
            PARTITION_VERT_3 => return block_coded(g, r, c + 3 * (half_w >> 1)),
            PARTITION_HORZ_4A | PARTITION_HORZ_4B => return block_coded(g, r + 7 * n4h(sub), c),
            PARTITION_VERT_4A | PARTITION_VERT_4B => return block_coded(g, r, c + 7 * n4w(sub)),
            _ => {}
        }
    }
    true
}

/// `init_allowed_partitions(...)`: the allowed set and its size.
pub fn init_allowed_partitions(
    g: &PartitionGeom,
    st: &PartitionState,
    r: u32,
    c: u32,
    b: u8,
    chroma_offset: bool,
    has_chroma: bool,
) -> ([bool; EXT_PARTITION_TYPES], usize) {
    let mut allowed = [false; EXT_PARTITION_TYPES];
    let mut num = 0;
    for (p, slot) in allowed.iter_mut().enumerate() {
        let good = is_partition_allowed(g, st, r, c, p as u8, b, chroma_offset, has_chroma);
        *slot = good;
        num += usize::from(good);
    }
    if num == 0 {
        allowed[usize::from(PARTITION_NONE)] = true;
        num = 1;
    }
    (allowed, num)
}

/// `partition_implied_at_boundary( r, c, bSize )`.
pub fn partition_implied_at_boundary(g: &PartitionGeom, r: u32, c: u32, b: u8) -> (bool, u8) {
    let (w4, h4) = (n4w(b), n4h(b));
    let has_rows = r + (h4 >> 1) < g.mi_rows;
    let has_cols = c + (w4 >> 1) < g.mi_cols;
    if has_rows && has_cols {
        return (false, PARTITION_NONE);
    }
    let mut implied = PARTITION_NONE;
    if w4 == h4 {
        implied = if has_rows { PARTITION_VERT } else { PARTITION_HORZ };
    } else if h4 > w4 {
        if !has_rows {
            implied = PARTITION_HORZ;
        } else {
            let sub_has_cols = c + (w4 >> 2) < g.mi_cols;
            if w4 >= 4 && !sub_has_cols {
                implied = PARTITION_HORZ;
            }
        }
    } else if !has_cols {
        implied = PARTITION_VERT;
    } else {
        let sub_has_rows = r + (h4 >> 2) < g.mi_rows;
        if h4 >= 4 && !sub_has_rows {
            implied = PARTITION_VERT;
        }
    }
    (implied != PARTITION_NONE, implied)
}

/// `partition_implied( r, c, bSize )`.
pub fn partition_implied(g: &PartitionGeom, st: &PartitionState, nb: &impl PartitionNeighbours, r: u32, c: u32, b: u8) -> (bool, u8) {
    if b == BLOCK_4X4 || b >= BLOCK_4X32 {
        return (true, PARTITION_NONE);
    }
    if st.tree_type == TreeType::Chroma && b == BLOCK_8X8 {
        return (true, PARTITION_NONE);
    }
    if st.tree_type == TreeType::Chroma
        && b == BLOCK_64X64
        && let Some(p) = nb.forced_chroma_partition(r, c)
    {
        return (true, p);
    }
    partition_implied_at_boundary(g, r, c, b)
}

/// `Rect_Part_Table[ do_ext_partition ][ do_uneven_4way_partition ][ type ][ rectType ]`.
fn rect_part_table(ext: bool, uneven: bool, ty: u32, rect: u8) -> u8 {
    let horz = rect == RECT_HORZ;
    match (ext, uneven, ty) {
        (false, _, _) => {
            if horz {
                PARTITION_HORZ
            } else {
                PARTITION_VERT
            }
        }
        (true, false, _) => {
            if horz {
                PARTITION_HORZ_3
            } else {
                PARTITION_VERT_3
            }
        }
        (true, true, 0) => {
            if horz {
                PARTITION_HORZ_4A
            } else {
                PARTITION_VERT_4A
            }
        }
        (true, true, _) => {
            if horz {
                PARTITION_HORZ_4B
            } else {
                PARTITION_VERT_4B
            }
        }
    }
}

/// Everything `read_partition` needs besides the symbol decoder and the CDFs.
pub struct PartitionRead<'a, N: PartitionNeighbours> {
    pub geom: &'a PartitionGeom,
    pub state: &'a PartitionState,
    pub neighbours: &'a N,
    pub avail_u: bool,
    pub avail_l: bool,
}

impl<N: PartitionNeighbours> PartitionRead<'_, N> {
    /// `read_partition( r, c, bSize, chromaOffset, hasChroma )`.
    pub fn read(&self, sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet, r: u32, c: u32, b: u8, chroma_offset: bool, has_chroma: bool) -> Result<u8> {
        let (g, st) = (self.geom, self.state);
        if usize::from(b) >= BLOCK_SIZES {
            return Err(Error::Invalid("block size"));
        }
        let (implied, partition) = partition_implied(g, st, self.neighbours, r, c, b);
        let (allowed, num_allowed) = init_allowed_partitions(g, st, r, c, b, chroma_offset, has_chroma);
        if implied && allowed.get(usize::from(partition)).copied().unwrap_or(false) {
            return Ok(partition);
        }
        if num_allowed == 1
            && let Some(p) = allowed.iter().position(|&a| a)
        {
            return Ok(p as u8);
        }
        if !st.bru_active {
            return Ok(PARTITION_NONE);
        }
        let ps = st.plane_start;
        if allowed[usize::from(PARTITION_NONE)] {
            let ctx = self.ctx_do_split(r, c, b);
            if sym.read_symbol(cdfs.cdf_mut(&DEFAULT_DO_SPLIT_CDF, &[ps, ctx])?)? == 0 {
                return Ok(PARTITION_NONE);
            }
        }
        if allowed[usize::from(PARTITION_SPLIT)] {
            let ctx = self.ctx_do_square_split(r, c, b);
            if sym.read_symbol(cdfs.cdf_mut(&DEFAULT_DO_SQUARE_SPLIT_CDF, &[ps, ctx])?)? != 0 {
                return Ok(PARTITION_SPLIT);
            }
        }
        let mut rect = rect_type_implied_by_bsize(st, b);
        let any = |ps: &[u8]| ps.iter().any(|&p| allowed[usize::from(p)]);
        if rect == RECT_INVALID {
            let allow_horz = any(&[PARTITION_HORZ, PARTITION_HORZ_3, PARTITION_HORZ_4A, PARTITION_HORZ_4B]);
            let allow_vert = any(&[PARTITION_VERT, PARTITION_VERT_3, PARTITION_VERT_4A, PARTITION_VERT_4B]);
            if !allow_horz {
                rect = RECT_VERT;
            } else if !allow_vert {
                rect = RECT_HORZ;
            }
        }
        if rect == RECT_INVALID {
            let ctx = self.ctx_rect_type(r, c, b);
            let v = sym.read_symbol(cdfs.cdf_mut(&DEFAULT_RECT_TYPE_CDF, &[ps, ctx])?)?;
            rect = if v == 0 { RECT_HORZ } else { RECT_VERT };
        }
        let (non_ext, ext3, ext4) = if rect == RECT_HORZ {
            (allowed[usize::from(PARTITION_HORZ)], allowed[usize::from(PARTITION_HORZ_3)], any(&[PARTITION_HORZ_4A, PARTITION_HORZ_4B]))
        } else {
            (allowed[usize::from(PARTITION_VERT)], allowed[usize::from(PARTITION_VERT_3)], any(&[PARTITION_VERT_4A, PARTITION_VERT_4B]))
        };
        let ext_ctx = self.ctx_ext(r, c, b, rect);
        let do_ext = if non_ext && (ext3 || ext4) { sym.read_symbol(cdfs.cdf_mut(&DEFAULT_DO_EXT_PARTITION_CDF, &[ps, ext_ctx])?)? != 0 } else { ext3 || ext4 };
        let mut uneven = false;
        let mut uneven_type = 0;
        if do_ext {
            uneven = if ext3 && ext4 { sym.read_symbol(cdfs.cdf_mut(&DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF, &[ps, ext_ctx])?)? != 0 } else { ext4 };
            if uneven {
                uneven_type = sym.read_literal(1);
            }
        }
        Ok(rect_part_table(do_ext, uneven, uneven_type, rect))
    }

    fn ctx_do_split(&self, r: u32, c: u32, b: u8) -> usize {
        let ps = self.state.plane_start;
        let bsw = mi_w_log2(b).max(1);
        let bsh = mi_h_log2(b).max(1);
        let ctx1 = usize::from(mi_h_log2(self.neighbours.left_mi_size(ps, r)) < bsh);
        let ctx2 = usize::from(mi_w_log2(self.neighbours.above_mi_size(ps, c)) < bsw);
        usize::from(PARTITION_SIZE_ADJUST.get(usize::from(b)).copied().unwrap_or(0)) * 4 + ctx1 * 2 + ctx2
    }

    fn ctx_do_square_split(&self, r: u32, c: u32, b: u8) -> usize {
        let ps = self.state.plane_start;
        let (bsw, bsh) = (mi_w_log2(b), mi_h_log2(b));
        let above = self.avail_u && r > 0 && mi_w_log2(self.neighbours.mi_size(ps, r - 1, c)) < bsw;
        let left = self.avail_l && c > 0 && mi_h_log2(self.neighbours.mi_size(ps, r, c - 1)) < bsh;
        (if b == BLOCK_256X256 { 4 } else { 0 }) + usize::from(left) * 2 + usize::from(above)
    }

    fn ctx_rect_type(&self, r: u32, c: u32, b: u8) -> usize {
        let ps = self.state.plane_start;
        let bsw = mi_w_log2(b).max(1);
        let bsh = mi_h_log2(b).max(1);
        let ctx1 = usize::from(mi_h_log2(self.neighbours.left_mi_size(ps, r)) < bsh);
        let ctx2 = usize::from(mi_w_log2(self.neighbours.above_mi_size(ps, c)) < bsw);
        usize::from(PARTITION_SIZE_ADJUST_RECT_TYPE.get(usize::from(b)).copied().unwrap_or(0)) * 4 + ctx1 * 2 + ctx2
    }

    fn ctx_ext(&self, r: u32, c: u32, b: u8, rect: u8) -> usize {
        let ps = self.state.plane_start;
        let nb = self.neighbours;
        let (ctx1, ctx2) = if rect == RECT_HORZ {
            let bsh = (mi_h_log2(b) - 1).max(1);
            (usize::from(mi_h_log2(nb.left_mi_size(ps, r)) < bsh), usize::from(mi_h_log2(nb.left_mi_size(ps, r + (n4h(b) >> 1))) < bsh))
        } else {
            let bsw = (mi_w_log2(b) - 1).max(1);
            (usize::from(mi_w_log2(nb.above_mi_size(ps, c)) < bsw), usize::from(mi_w_log2(nb.above_mi_size(ps, c + (n4w(b) >> 1))) < bsw))
        };
        usize::from(PARTITION_SIZE_ADJUST.get(usize::from(b)).copied().unwrap_or(0)) * 4 + ctx1 * 2 + ctx2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat;
    impl PartitionNeighbours for Flat {
        fn above_mi_size(&self, _: usize, _: u32) -> u8 {
            BLOCK_64X64
        }
        fn left_mi_size(&self, _: usize, _: u32) -> u8 {
            BLOCK_64X64
        }
        fn mi_size(&self, _: usize, _: u32, _: u32) -> u8 {
            BLOCK_64X64
        }
        fn forced_chroma_partition(&self, _: u32, _: u32) -> Option<u8> {
            None
        }
    }

    fn geom(rows: u32, cols: u32) -> PartitionGeom {
        PartitionGeom {
            mi_rows: rows,
            mi_cols: cols,
            sub_x: true,
            sub_y: true,
            num_planes: 3,
            frame_is_intra: true,
            enable_ext_partitions: true,
            enable_uneven_4way_partitions: true,
            max_pb_aspect_ratio: 8,
        }
    }

    fn state() -> PartitionState {
        PartitionState { tree_type: TreeType::Shared, region_type: RegionType::Mixed, plane_start: 0, bru_active: true }
    }

    #[test]
    fn implied_partitions_at_frame_boundary() {
        let g = geom(20, 20);
        // Inside: nothing implied.
        assert_eq!(partition_implied_at_boundary(&g, 0, 0, BLOCK_64X64), (false, PARTITION_NONE));
        // 64x64 at (16, 0): rows run out, columns fit -> horizontal cut.
        assert_eq!(partition_implied_at_boundary(&g, 16, 0, BLOCK_64X64), (true, PARTITION_HORZ));
        // columns run out, rows fit -> vertical cut.
        assert_eq!(partition_implied_at_boundary(&g, 0, 16, BLOCK_64X64), (true, PARTITION_VERT));
        // both run out -> horizontal cut.
        assert_eq!(partition_implied_at_boundary(&g, 16, 16, BLOCK_64X64), (true, PARTITION_HORZ));
    }

    #[test]
    fn allowed_set_of_a_plain_interior_block() {
        let g = geom(128, 128);
        let (allowed, n) = init_allowed_partitions(&g, &state(), 0, 0, BLOCK_32X32, false, true);
        assert!(allowed[usize::from(PARTITION_NONE)]);
        // The square split exists only for 128x128 and 256x256 (Partition_Subsize); smaller
        // squares are cut with the rectangular partitions.
        assert!(!allowed[usize::from(PARTITION_SPLIT)], "{allowed:?}");
        assert!(allowed[usize::from(PARTITION_HORZ_3)] && allowed[usize::from(PARTITION_VERT_4B)]);
        assert_eq!(n, allowed.iter().filter(|&&a| a).count());
        let (big, _) = init_allowed_partitions(&g, &state(), 0, 0, BLOCK_128X128, false, true);
        assert!(big[usize::from(PARTITION_SPLIT)]);
        // 8x8 blocks have no 4-way cut.
        let (a8, _) = init_allowed_partitions(&g, &state(), 0, 0, BLOCK_8X8, false, true);
        assert!(!a8[usize::from(PARTITION_HORZ_4A)]);
    }

    #[test]
    fn read_partition_is_deterministic_and_total() {
        let data: Vec<u8> = (0..128u32).map(|i| (i.wrapping_mul(97) ^ 0x3C) as u8).collect();
        let g = geom(64, 64);
        let st = state();
        let pr = PartitionRead { geom: &g, state: &st, neighbours: &Flat, avail_u: true, avail_l: true };
        let run = || {
            let mut sym = SymbolDecoder::new(&data, false).unwrap();
            let mut cdfs = CdfSet::new(100);
            (0..40)
                .map(|i| pr.read(&mut sym, &mut cdfs, 0, 0, [BLOCK_64X64, BLOCK_32X32, BLOCK_16X16, BLOCK_16X32][i % 4], false, true).unwrap())
                .collect::<Vec<_>>()
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert!(a.iter().all(|&p| p <= PARTITION_SPLIT));
    }

    #[test]
    fn rect_part_table_matches_spec() {
        assert_eq!(rect_part_table(false, false, 0, RECT_HORZ), PARTITION_HORZ);
        assert_eq!(rect_part_table(false, false, 0, RECT_VERT), PARTITION_VERT);
        assert_eq!(rect_part_table(true, false, 0, RECT_VERT), PARTITION_VERT_3);
        assert_eq!(rect_part_table(true, true, 0, RECT_HORZ), PARTITION_HORZ_4A);
        assert_eq!(rect_part_table(true, true, 1, RECT_VERT), PARTITION_VERT_4B);
    }

    #[test]
    fn chroma_residual_sizes() {
        let g = geom(8, 8);
        assert_eq!(chroma_residual_size(&g, BLOCK_8X8), BLOCK_4X4);
        assert_eq!(chroma_residual_size(&g, BLOCK_4X8), BLOCK_4X4);
        assert_eq!(chroma_residual_size(&g, BLOCK_64X64), BLOCK_32X32);
    }
}
