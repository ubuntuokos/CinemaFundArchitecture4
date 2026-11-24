//! Intra mode helpers of the mode info syntax (spec §5.20.5.5–5.20.5.8): the mode enumerations,
//! `get_intra_y_mode_set`, `get_intra_uv_mode_set` and `neg_deinterleave`.
//!
//! The symbol reads and their CDF contexts follow in later modules; these are the pure mappings
//! from a decoded mode index to the prediction mode and angle delta.

use crate::block_tables::{BLOCK_8X8, NUM_4X4_HIGH, NUM_4X4_WIDE};

pub const DC_PRED: u8 = 0;
pub const V_PRED: u8 = 1;
pub const H_PRED: u8 = 2;
pub const D45_PRED: u8 = 3;
pub const D135_PRED: u8 = 4;
pub const D113_PRED: u8 = 5;
pub const D157_PRED: u8 = 6;
pub const D203_PRED: u8 = 7;
pub const D67_PRED: u8 = 8;
pub const SMOOTH_PRED: u8 = 9;
pub const SMOOTH_V_PRED: u8 = 10;
pub const SMOOTH_H_PRED: u8 = 11;
pub const PAETH_PRED: u8 = 12;
pub const UV_CFL_PRED: u8 = 13;

pub const INTRA_MODES: usize = 13;
pub const NON_DIRECTIONAL_MODES_COUNT: u32 = 5;
pub const DIRECTIONAL_MODES_COUNT: usize = 56;
pub const MODE_INDEX_COUNT: u32 = 8;
pub const FIRST_MODE_COUNT: u32 = 13;
pub const SECOND_MODE_COUNT: u32 = 16;
pub const TOTAL_ANGLE_DELTA_COUNT: u32 = 7;
pub const MAX_ANGLE_DELTA: i32 = 3;
pub const CHROMA_MODE_COUNT: u32 = 8;
pub const UV_INTRA_MODES_CFL_NOT_ALLOWED: usize = 13;
pub const FSC_MAX: u32 = 32;

pub const REORDERED_Y_MODE: [u8; INTRA_MODES] =
    [DC_PRED, SMOOTH_PRED, SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED, D45_PRED, D67_PRED, V_PRED, D113_PRED, D135_PRED, D157_PRED, H_PRED, D203_PRED];

pub const DEFAULT_MODE_LIST_Y: [u8; DIRECTIONAL_MODES_COUNT] = [
    17, 45, 3, 10, 24, 31, 38, 52, 15, 19, 43, 47, 1, 5, 8, 12, 22, 26, 29, 33, 36, 40, 50, 54, 16, 18, 44, 46, 2, 4, 9, 11, 23, 25, 30, 32, 37, 39, 51, 53,
    14, 20, 42, 48, 0, 6, 7, 13, 21, 27, 28, 34, 35, 41, 49, 55,
];

pub const DEFAULT_MODE_LIST_UV: [u8; UV_INTRA_MODES_CFL_NOT_ALLOWED] =
    [DC_PRED, SMOOTH_PRED, SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED, V_PRED, H_PRED, D45_PRED, D135_PRED, D67_PRED, D113_PRED, D157_PRED, D203_PRED];

/// Result of `read_intra_y_mode` after the mode index is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YModeInfo {
    pub y_mode: u8,
    pub angle_delta: i32,
    /// `IntraJointMode`, the value neighbours see through `get_joint_mode`.
    pub joint_mode: u8,
}

/// `is_directional_mode`.
pub fn is_directional_mode(mode: u8) -> bool {
    (V_PRED..=D67_PRED).contains(&mode)
}

/// `get_intra_y_mode_set`. `joint` holds `get_joint_mode(0)` (left) and `get_joint_mode(1)`
/// (above), already resolved by the caller (DC_PRED when outside the tile).
/// Returns `None` for a `mode_idx` outside the valid range.
pub fn get_intra_y_mode_set(mode_idx: u32, mi_size: u8, joint: [u8; 2]) -> Option<u32> {
    if mode_idx < NON_DIRECTIONAL_MODES_COUNT {
        return Some(mode_idx);
    }
    let mut idx = mode_idx - NON_DIRECTIONAL_MODES_COUNT;
    let mut selected = [false; DIRECTIONAL_MODES_COUNT];
    let mut dir_modes = [0i32; 2];
    let mut count = 0usize;
    let ndm = NON_DIRECTIONAL_MODES_COUNT;
    let dm = DIRECTIONAL_MODES_COUNT as i32;
    if mi_size >= BLOCK_8X8 {
        for &j in &joint {
            let j = u32::from(j);
            if j >= ndm {
                let mode = (j - ndm) as i32;
                if count == 0 || mode != dir_modes[0] {
                    if idx == 0 {
                        return Some(mode as u32 + ndm);
                    }
                    idx -= 1;
                    *selected.get_mut(mode as usize)? = true;
                    *dir_modes.get_mut(count)? = mode;
                    count += 1;
                }
            }
        }
        let size = usize::from(mi_size);
        let area = u32::from(*NUM_4X4_WIDE.get(size)?) * u32::from(*NUM_4X4_HIGH.get(size)?) * 16;
        if area > 64 {
            for i in 1..=4 {
                for &base in dir_modes.iter().take(count) {
                    for sgn in [-1, 1] {
                        let mut mode = base + i * sgn;
                        if mode < 0 {
                            mode += dm;
                        } else if mode >= dm {
                            mode -= dm;
                        }
                        let sel = selected.get_mut(mode as usize)?;
                        if !*sel {
                            if idx == 0 {
                                return Some(mode as u32 + ndm);
                            }
                            idx -= 1;
                            *sel = true;
                        }
                    }
                }
            }
        }
    }
    for &mode in &DEFAULT_MODE_LIST_Y {
        if !selected[usize::from(mode)] {
            if idx == 0 {
                return Some(u32::from(mode) + ndm);
            }
            idx -= 1;
        }
    }
    None
}

/// Mode, angle delta and joint mode from `modeDelta` (the output of `get_intra_y_mode_set`).
pub fn y_mode_from_delta(mode_delta: u32) -> Option<YModeInfo> {
    let joint_mode = u8::try_from(mode_delta).ok()?;
    if mode_delta < NON_DIRECTIONAL_MODES_COUNT {
        return Some(YModeInfo { y_mode: *REORDERED_Y_MODE.get(mode_delta as usize)?, angle_delta: 0, joint_mode });
    }
    let d = mode_delta - NON_DIRECTIONAL_MODES_COUNT;
    let y_mode = *REORDERED_Y_MODE.get((d / TOTAL_ANGLE_DELTA_COUNT + NON_DIRECTIONAL_MODES_COUNT) as usize)?;
    let angle_delta = (d % TOTAL_ANGLE_DELTA_COUNT) as i32 - MAX_ANGLE_DELTA;
    Some(YModeInfo { y_mode, angle_delta, joint_mode })
}

/// `get_intra_uv_mode_set`.
pub fn get_intra_uv_mode_set(mode_idx: u32, y_mode: u8) -> Option<u8> {
    let mut idx = mode_idx;
    let dir = is_directional_mode(y_mode);
    if dir {
        if idx == 0 {
            return Some(y_mode);
        }
        idx -= 1;
    }
    for &mode in &DEFAULT_MODE_LIST_UV {
        if mode != y_mode || !dir {
            if idx == 0 {
                return Some(mode);
            }
            idx -= 1;
        }
    }
    None
}

/// `neg_deinterleave`, with a guard for out-of-range input (returns `None`).
pub fn neg_deinterleave(diff: i32, reference: i32, max: i32) -> Option<i32> {
    if reference == 0 {
        return Some(diff);
    }
    if reference >= max - 1 {
        return Some(max - diff - 1);
    }
    if 2 * reference < max {
        if diff <= 2 * reference {
            return Some(if diff & 1 != 0 { reference + ((diff + 1) >> 1) } else { reference - (diff >> 1) });
        }
        return Some(diff);
    }
    if diff <= 2 * (max - reference - 1) {
        return Some(if diff & 1 != 0 { reference + ((diff + 1) >> 1) } else { reference - (diff >> 1) });
    }
    Some(max - (diff + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_tables::{BLOCK_4X4, BLOCK_16X16};

    #[test]
    fn y_mode_set_is_a_permutation_without_neighbours() {
        for size in [BLOCK_4X4, BLOCK_8X8, BLOCK_16X16] {
            let mut seen = [false; 61];
            for idx in 0..61 {
                let m = get_intra_y_mode_set(idx, size, [DC_PRED; 2]).unwrap() as usize;
                assert!(!seen[m], "duplicate {m} size {size}");
                seen[m] = true;
            }
        }
    }

    #[test]
    fn y_mode_set_is_a_permutation_with_neighbours() {
        let mut seen = [false; 61];
        for idx in 0..61 {
            let m = get_intra_y_mode_set(idx, BLOCK_16X16, [5 + 17, 5 + 18]).unwrap() as usize;
            assert!(!seen[m]);
            seen[m] = true;
        }
        // neighbour modes come first, then the +-1 ring
        assert_eq!(get_intra_y_mode_set(5, BLOCK_16X16, [22, 23]), Some(22));
        assert_eq!(get_intra_y_mode_set(6, BLOCK_16X16, [22, 23]), Some(23));
        assert_eq!(get_intra_y_mode_set(7, BLOCK_16X16, [22, 23]), Some(21));
        assert_eq!(get_intra_y_mode_set(61, BLOCK_16X16, [22, 23]), None);
    }

    #[test]
    fn small_blocks_ignore_neighbours() {
        assert_eq!(get_intra_y_mode_set(5, BLOCK_4X4, [30, 31]), Some(DEFAULT_MODE_LIST_Y[0] as u32 + 5));
    }

    #[test]
    fn mode_from_delta() {
        assert_eq!(y_mode_from_delta(1), Some(YModeInfo { y_mode: SMOOTH_PRED, angle_delta: 0, joint_mode: 1 }));
        // delta 5 -> first directional entry, angle delta -3
        assert_eq!(y_mode_from_delta(5), Some(YModeInfo { y_mode: D45_PRED, angle_delta: -3, joint_mode: 5 }));
        // last: 5 + 55 = 60 -> directional index 55 -> Reordered[7 + 5], angle delta 3
        assert_eq!(y_mode_from_delta(60).unwrap().y_mode, D203_PRED);
        assert_eq!(y_mode_from_delta(60).unwrap().angle_delta, 3);
    }

    #[test]
    fn uv_mode_set() {
        // non-directional luma: plain default list
        assert_eq!(get_intra_uv_mode_set(0, DC_PRED), Some(DC_PRED));
        assert_eq!(get_intra_uv_mode_set(12, DC_PRED), Some(D203_PRED));
        assert_eq!(get_intra_uv_mode_set(13, DC_PRED), None);
        // directional luma: luma mode first, then the list without it
        assert_eq!(get_intra_uv_mode_set(0, H_PRED), Some(H_PRED));
        let mut seen = [false; 13];
        for i in 0..13 {
            let m = get_intra_uv_mode_set(i, H_PRED).unwrap() as usize;
            assert!(!seen[m]);
            seen[m] = true;
        }
    }

    #[test]
    fn deinterleave() {
        assert_eq!(neg_deinterleave(3, 0, 8), Some(3));
        assert_eq!(neg_deinterleave(0, 7, 8), Some(7));
        // max 8, ref 2: 0->2, 1->3, 2->1, 3->4, 4->0, 5->5
        let got: Vec<i32> = (0..6).map(|d| neg_deinterleave(d, 2, 8).unwrap()).collect();
        assert_eq!(got, vec![2, 3, 1, 4, 0, 5]);
        // beyond the interleaved range falls through to identity
        assert_eq!(neg_deinterleave(7, 2, 8), Some(7));
        // ref in the upper half
        let got: Vec<i32> = (0..8).map(|d| neg_deinterleave(d, 5, 8).unwrap()).collect();
        let mut s = got.clone();
        s.sort();
        assert_eq!(s, (0..8).collect::<Vec<_>>());
    }
}
