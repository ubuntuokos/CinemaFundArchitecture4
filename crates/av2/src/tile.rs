//! Tile layout (AV2 v1.0.0 § 5.18.7.3 `tile_params()`, § 5.18.7.5 `uniform_spacing()`,
//! § 5.18.7.7 `tile_log2()`). The sequence header reads it once for the maximum frame size
//! (§ 5.4.2); the frame header reads or reuses it per frame.

use cfa4_bitstream::BitReader;

use crate::seq_tools::{SuperblockSize, ns};
use crate::{Error, Result};

/// `MAX_TILE_WIDTH`, in luma samples.
const MAX_TILE_WIDTH: u64 = 4096;
/// `MAX_TILE_AREA`, in luma samples.
const MAX_TILE_AREA: u64 = 4096 * 2304;
/// `MAX_TILE_COLS`, `MAX_TILE_ROWS`.
pub const MAX_TILE_COLS: u32 = 64;
pub const MAX_TILE_ROWS: u32 = 64;
/// `seq_level_idx` without level limits.
const LEVEL_UNCONSTRAINED: u8 = 31;
/// `Tile_Width_Scaling_Factor[seq_tier][seq_level_idx]` for the defined levels 0..=21.
const TILE_WIDTH_SCALING: [[u8; 22]; 2] =
    [[4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 8, 8, 8], [4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 8, 8, 8, 16, 16, 16, 16]];
/// `Tile_Area_Scaling_Factor[seq_tier][seq_level_idx]` for the defined levels 0..=21.
const TILE_AREA_SCALING: [[u8; 22]; 2] =
    [[4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 8, 8, 8, 16, 16, 16, 16], [4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 16, 16, 16, 16, 32, 32, 32, 32]];

impl SuperblockSize {
    /// `Mi_Width_Log2[ sbSize ]`.
    pub fn mi_log2(self) -> u32 {
        match self {
            SuperblockSize::Sb64 => 4,
            SuperblockSize::Sb128 => 5,
            SuperblockSize::Sb256 => 6,
        }
    }
}

/// The level limits `tile_params()` uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileLimits {
    pub level_idx: u8,
    pub tier: u8,
}

/// What `tile_params()` returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileParams {
    pub sb_row_starts: Vec<u32>,
    pub sb_rows: u32,
    pub tile_rows: u32,
    pub tile_rows_log2: u32,
    pub sb_col_starts: Vec<u32>,
    pub sb_cols: u32,
    pub tile_cols: u32,
    pub tile_cols_log2: u32,
    pub uniform_tile_spacing: bool,
    pub sb_shift: u32,
}

/// `tile_log2( blkSize, target )`: the smallest `k` with `blkSize << k >= target`.
pub fn tile_log2(blk_size: u64, target: u64) -> u32 {
    let blk = blk_size.max(1);
    let mut k = 0;
    while k < 63 && (blk << k) < target {
        k += 1;
    }
    k
}

/// `uniform_spacing( tileLog2, mis, sbSize )`.
pub(crate) fn uniform_spacing(tile_log2: u32, mis: u32, sb: SuperblockSize) -> Result<Vec<u32>> {
    let shift = sb.mi_log2();
    let sbs = (mis + (1 << shift) - 1) >> shift;
    let full_sbs = mis >> shift;
    let tiles = 1u32.checked_shl(tile_log2).ok_or(Error::Invalid("tile count"))?;
    let tile_sb = full_sbs.checked_shr(tile_log2).unwrap_or(0);
    let extra_sbs = if tile_sb == 0 { sbs } else { full_sbs - (tile_sb << tile_log2) };
    let mut starts = Vec::new();
    let mut start = 0u32;
    let mut i = 0u32;
    while i < tiles && start < sbs {
        starts.push(start);
        start += tile_sb;
        if i < extra_sbs {
            start += 1;
        }
        i += 1;
    }
    Ok(starts)
}

/// Reads `increment_tile_*_log2` flags from `min` up to at most `max`.
fn increments(r: &mut BitReader<'_>, min: u32, max: u32, is_bridge: bool) -> Result<u32> {
    let mut log2 = min;
    if !is_bridge {
        while log2 < max {
            if r.read_flag()? {
                log2 += 1;
            } else {
                break;
            }
        }
    }
    Ok(log2)
}

fn count(v: &[u32]) -> Result<u32> {
    u32::try_from(v.len()).map_err(|_| Error::Invalid("tile count"))
}

/// `tile_params( frameWidth, frameHeight, uniformSbSize, sbSize, isBridge )`.
pub fn tile_params(
    r: &mut BitReader<'_>,
    (frame_width, frame_height): (u32, u32),
    uniform_sb: SuperblockSize,
    sb: SuperblockSize,
    is_bridge: bool,
    limits: TileLimits,
) -> Result<TileParams> {
    let mi_cols = 2 * ((frame_width.saturating_add(7)) >> 3);
    let mi_rows = 2 * ((frame_height.saturating_add(7)) >> 3);
    let sb_shift = sb.mi_log2();
    let sb_cols = (mi_cols + (1 << sb_shift) - 1) >> sb_shift;
    let sb_rows = (mi_rows + (1 << sb_shift) - 1) >> sb_shift;
    let sb_area = u64::from(sb_rows) * u64::from(sb_cols);
    let (max_tile_width_sb, mut max_tile_area_sb) = if limits.level_idx == LEVEL_UNCONSTRAINED {
        (u64::from(sb_cols), sb_area)
    } else {
        let tier = usize::from(limits.tier.min(1));
        let level = usize::from(limits.level_idx);
        let (Some(w), Some(a)) = (TILE_WIDTH_SCALING[tier].get(level), TILE_AREA_SCALING[tier].get(level)) else {
            return Err(Error::Invalid("reserved seq_level_idx"));
        };
        ((u64::from(*w) * MAX_TILE_WIDTH) >> (sb_shift + 4), (u64::from(*a) * MAX_TILE_AREA) >> (2 * (sb_shift + 2) + 2))
    };
    let min_log2_tile_cols = tile_log2(max_tile_width_sb, u64::from(sb_cols));
    let max_log2_tile_cols = tile_log2(1, u64::from(sb_cols.min(MAX_TILE_COLS)));
    let max_log2_tile_rows = tile_log2(1, u64::from(sb_rows.min(MAX_TILE_ROWS)));
    let min_log2_tiles = min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_area));
    let uniform_tile_spacing = is_bridge || r.read_flag()?;

    let (sb_col_starts, sb_row_starts, out_shift);
    if uniform_tile_spacing {
        out_shift = uniform_sb.mi_log2();
        let cols_log2 = increments(r, min_log2_tile_cols, max_log2_tile_cols, is_bridge)?;
        sb_col_starts = uniform_spacing(cols_log2, mi_cols, uniform_sb)?;
        let cols_log2 = tile_log2(1, u64::from(count(&sb_col_starts)?));
        let min_log2_tile_rows = min_log2_tiles.saturating_sub(cols_log2);
        let rows_log2 = increments(r, min_log2_tile_rows, max_log2_tile_rows, is_bridge)?;
        sb_row_starts = uniform_spacing(rows_log2, mi_rows, uniform_sb)?;
    } else {
        out_shift = sb_shift;
        let mut cols = Vec::new();
        let (mut widest, mut start) = (1u32, 0u32);
        while start < sb_cols {
            cols.push(start);
            let n = u64::from(sb_cols - start).min(max_tile_width_sb);
            let size = ns(r, u32::try_from(n).map_err(|_| Error::Invalid("tile width"))?)? + 1;
            widest = widest.max(size);
            start = start.saturating_add(size);
        }
        if min_log2_tiles > 0 {
            max_tile_area_sb = sb_area >> (min_log2_tiles + 1);
        } else {
            max_tile_area_sb = sb_area;
        }
        let max_tile_height_sb = (max_tile_area_sb / u64::from(widest)).max(1);
        let mut rows = Vec::new();
        start = 0;
        while start < sb_rows {
            rows.push(start);
            let n = u64::from(sb_rows - start).min(max_tile_height_sb);
            let size = ns(r, u32::try_from(n).map_err(|_| Error::Invalid("tile height"))?)? + 1;
            start = start.saturating_add(size);
        }
        sb_col_starts = cols;
        sb_row_starts = rows;
    }
    let tile_cols = count(&sb_col_starts)?;
    let tile_rows = count(&sb_row_starts)?;
    if tile_cols > MAX_TILE_COLS || tile_rows > MAX_TILE_ROWS {
        return Err(Error::Invalid("too many tiles"));
    }
    Ok(TileParams {
        sb_row_starts,
        sb_rows,
        tile_rows,
        tile_rows_log2: tile_log2(1, u64::from(tile_rows)),
        sb_col_starts,
        sb_cols,
        tile_cols,
        tile_cols_log2: tile_log2(1, u64::from(tile_cols)),
        uniform_tile_spacing,
        sb_shift: out_shift,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_bitstream::BitWriter;

    const L5_0: TileLimits = TileLimits { level_idx: 12, tier: 0 };

    fn parse(bits: &[bool], size: (u32, u32), sb: SuperblockSize, limits: TileLimits) -> (TileParams, usize) {
        let mut w = BitWriter::new();
        for &b in bits {
            w.write_bit(b);
        }
        w.write_bits(0, 16);
        let data = w.finish();
        let mut r = BitReader::new(&data);
        let p = tile_params(&mut r, size, sb, sb, false, limits).unwrap();
        (p, r.position())
    }

    #[test]
    fn single_uniform_tile_takes_three_bits() {
        // 352x288 with 64x64 superblocks: 6x5 superblocks, one tile, two terminating zeros
        let (p, used) = parse(&[true, false, false], (352, 288), SuperblockSize::Sb64, L5_0);
        assert_eq!(used, 3);
        assert_eq!((p.sb_cols, p.sb_rows, p.tile_cols, p.tile_rows), (6, 5, 1, 1));
        assert_eq!((p.sb_col_starts, p.sb_row_starts), (vec![0], vec![0]));
    }

    #[test]
    fn uniform_increments_split_columns_and_rows() {
        // 1920x1080, Sb128: 15x9 superblocks; two column steps, one row step
        let (p, used) = parse(&[true, true, true, false, true, false], (1920, 1080), SuperblockSize::Sb128, L5_0);
        assert_eq!(used, 6);
        assert_eq!((p.tile_cols, p.tile_cols_log2, p.tile_rows, p.tile_rows_log2), (4, 2, 2, 1));
        assert_eq!(p.sb_col_starts, vec![0, 4, 8, 12]);
        assert_eq!(p.sb_row_starts, vec![0, 4]);
    }

    #[test]
    fn explicit_sizes_use_ns() {
        // 256x128 Sb64: 4x2 superblocks, unconstrained level. Column widths: ns(4)=1 → 2 SBs,
        // ns(2)=1 → 2 SBs; rows: max height = 8/2 = 4, ns(2)=1 → 2 SBs.
        let unconstrained = TileLimits { level_idx: 31, tier: 0 };
        let bits = [false, false, true, true, true];
        let (p, used) = parse(&bits, (256, 128), SuperblockSize::Sb64, unconstrained);
        assert_eq!(used, 5);
        assert_eq!((p.sb_col_starts, p.sb_row_starts), (vec![0, 2], vec![0]));
        assert!(!p.uniform_tile_spacing);
    }

    #[test]
    fn reserved_level_is_an_error() {
        let data = [0xFFu8; 4];
        let mut r = BitReader::new(&data);
        let limits = TileLimits { level_idx: 25, tier: 0 };
        assert!(tile_params(&mut r, (64, 64), SuperblockSize::Sb64, SuperblockSize::Sb64, false, limits).is_err());
    }

    #[test]
    fn tile_log2_matches_definition() {
        assert_eq!(tile_log2(1, 1), 0);
        assert_eq!(tile_log2(1, 5), 3);
        assert_eq!(tile_log2(64, 6), 0);
        assert_eq!(tile_log2(0, 1 << 40), 40);
    }
}
