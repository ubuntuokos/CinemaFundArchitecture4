//! Small per-block syntax reads of the intra mode info (spec §5.20.5.10–5.20.5.11, §5.20.7.25,
//! §5.20.10): `skip_flag`, the quantizer index delta, the CDEF index, `ccso_blk`, `use_gdf` and
//! the CFL alphas, with the §8.3.2 CDF selection.
//!
//! Neighbour-derived contexts and the frame-level grids (`cdef_idx`, `CcsoBlks`, `GdfBlks`) belong
//! to the block decoder; they are inputs and outputs here.

use crate::cdf_default::{
    DEFAULT_CCSO_BLK_CDF, DEFAULT_CDEF_INDEX0_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF, DEFAULT_CFL_ALPHA_CDF, DEFAULT_CFL_INDEX_CDF, DEFAULT_CFL_MHCCP_CDF, DEFAULT_CFL_MH_DIR_CDF,
    DEFAULT_CFL_SIGN_CDF, DEFAULT_DELTA_Q_CDF, DEFAULT_SKIP_CDF, DEFAULT_USE_GDF_CDF,
};
use crate::cdf_set::CdfSet;
use crate::symbol::SymbolDecoder;
use crate::{Error, Result};

pub const SKIP_CONTEXTS: usize = 6;
pub const DELTA_Q_SMALL: u32 = 7;
pub const CFL_EXPLICIT: u32 = 0;
pub const CFL_DERIVED_ALPHA: u32 = 1;
pub const CFL_MULTI: u32 = 2;
pub const CFL_SIGN_ZERO: u32 = 0;
pub const CFL_SIGN_NEG: u32 = 1;
pub const CFL_SIGN_POS: u32 = 2;

/// `Size_Group` (size_group.h).
pub const SIZE_GROUP: [u8; 29] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2];

/// `skip_flag`: `neighbour_skips` is the sum of `Skips` over the neighbours in `NPosBuf`.
pub fn read_skip(sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet, neighbour_skips: usize, skip_mode: bool) -> Result<bool> {
    let ctx = neighbour_skips + if skip_mode { SKIP_CONTEXTS >> 1 } else { 0 };
    Ok(sym.read_symbol(cdfs.cdf_mut(&DEFAULT_SKIP_CDF, &[ctx])?)? != 0)
}

/// Frame-level quantizer delta parameters.
#[derive(Debug, Clone, Copy)]
pub struct DeltaQParams {
    pub delta_q_present: bool,
    pub delta_q_res: u32,
    pub max_q: i32,
}

/// `read_delta_qindex`: returns the new `CurrentQIndex`. `read` is
/// `!(MiSize == SbSize && skip_flag) && ReadDeltas`.
pub fn read_delta_qindex(sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet, p: &DeltaQParams, current_q: i32, read: bool) -> Result<i32> {
    let mut q = current_q;
    if read {
        let mut abs = sym.read_symbol(cdfs.cdf_mut(&DEFAULT_DELTA_Q_CDF, &[])?)?;
        if abs == DELTA_Q_SMALL {
            let rem_bits = sym.read_literal(3) + 1;
            let abs_bits = sym.read_literal(rem_bits);
            abs = abs_bits + (1 << rem_bits) + DELTA_Q_SMALL - 2;
        }
        if abs != 0 {
            let neg = sym.read_literal(1) != 0;
            let abs = i64::from(abs);
            let reduced = if neg { -abs } else { abs };
            let shifted = reduced.checked_shl(p.delta_q_res).ok_or(Error::Invalid("delta_q_res"))?;
            q = (i64::from(q) + shifted).clamp(1, i64::from(p.max_q)) as i32;
        }
    }
    if p.delta_q_present {
        q = q.clamp(1, p.max_q);
    }
    Ok(q)
}

/// The `cdef_index0` context from the left and above 64x64 `cdef_idx` values (`None` when that
/// neighbour is unavailable).
pub fn cdef_index0_ctx(left: Option<i8>, above: Option<i8>) -> usize {
    let mut ctx = 0;
    let mut cnt = 0;
    for v in [left, above].into_iter().flatten() {
        ctx += usize::from(v == 0);
        cnt += 1;
    }
    if ctx != 0 && cnt == ctx {
        ctx += 1;
    }
    ctx
}

/// Reads a new `cdef_idx` for a 64x64 unit (the `cdef_idx == -1` branch of `read_cdef`).
pub fn read_cdef_idx(sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet, cdef_strengths: u32, ctx: usize) -> Result<u8> {
    if cdef_strengths == 1 {
        return Ok(0);
    }
    if sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CDEF_INDEX0_CDF, &[ctx])?)? != 0 {
        return Ok(0);
    }
    if cdef_strengths == 2 {
        return Ok(1);
    }
    let table = match cdef_strengths {
        3 => &DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF,
        4 => &DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
        5 => &DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF,
        6 => &DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF,
        7 => &DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF,
        8 => &DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF,
        _ => return Err(Error::Invalid("cdef strengths")),
    };
    Ok(sym.read_symbol(cdfs.cdf_mut(table, &[])?)? as u8 + 1)
}

/// `ccso_blk` for `plane`; `left_blk` is `CcsoBlks` of the block to the left when it lies inside
/// the tile.
pub fn read_ccso_blk(sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet, plane: usize, left_blk: Option<u8>) -> Result<u8> {
    let ctx = left_blk.map_or(0, |b| 2 * usize::from(b));
    Ok(sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CCSO_BLK_CDF, &[plane, ctx])?)? as u8)
}

/// `use_gdf`.
pub fn read_use_gdf(sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet) -> Result<bool> {
    Ok(sym.read_symbol(cdfs.cdf_mut(&DEFAULT_USE_GDF_CDF, &[])?)? != 0)
}

/// Inputs of `read_cfl_alphas`.
#[derive(Debug, Clone, Copy)]
pub struct CflInputs {
    pub enable_cfl_intra: bool,
    /// `is_mhccp_allowed()`.
    pub mhccp_allowed: bool,
    pub mi_size: u8,
}

/// Result of `read_cfl_alphas`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CflAlphas {
    pub cfl_mhccp: bool,
    pub cfl_index: u32,
    pub cfl_mh_dir: u32,
    pub alpha_u: i32,
    pub alpha_v: i32,
}

/// `read_cfl_alphas` (§5.20.7.32).
pub fn read_cfl_alphas(sym: &mut SymbolDecoder<'_>, cdfs: &mut CdfSet, p: &CflInputs) -> Result<CflAlphas> {
    let cfl_mhccp = if !p.enable_cfl_intra {
        true
    } else if p.mhccp_allowed {
        sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CFL_MHCCP_CDF, &[])?)? != 0
    } else {
        false
    };
    let cfl_index = if cfl_mhccp { CFL_MULTI } else { sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CFL_INDEX_CDF, &[])?)? };
    let mut out = CflAlphas { cfl_mhccp, cfl_index, cfl_mh_dir: 0, alpha_u: 0, alpha_v: 0 };
    if cfl_index == CFL_MULTI {
        let group = usize::from(*SIZE_GROUP.get(usize::from(p.mi_size)).ok_or(Error::Invalid("block size"))?);
        out.cfl_mh_dir = sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CFL_MH_DIR_CDF, &[group])?)?;
    }
    if cfl_index != CFL_EXPLICIT {
        return Ok(out);
    }
    let signs = sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CFL_SIGN_CDF, &[])?)?;
    let sign_u = (signs + 1) / 3;
    let sign_v = (signs + 1) % 3;
    if sign_u != CFL_SIGN_ZERO {
        let ctx = ((sign_u - 1) * 3 + sign_v) as usize;
        let a = 1 + sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CFL_ALPHA_CDF, &[ctx])?)? as i32;
        out.alpha_u = if sign_u == CFL_SIGN_NEG { -a } else { a };
    }
    if sign_v != CFL_SIGN_ZERO {
        let ctx = ((sign_v - 1) * 3 + sign_u) as usize;
        let a = 1 + sym.read_symbol(cdfs.cdf_mut(&DEFAULT_CFL_ALPHA_CDF, &[ctx])?)? as i32;
        out.alpha_v = if sign_v == CFL_SIGN_NEG { -a } else { a };
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Vec<u8> {
        (0..96u32).map(|i| (i * 151 + 29) as u8).collect()
    }

    #[test]
    fn cdef_ctx() {
        assert_eq!(cdef_index0_ctx(None, None), 0);
        assert_eq!(cdef_index0_ctx(Some(0), None), 2);
        assert_eq!(cdef_index0_ctx(Some(0), Some(3)), 1);
        assert_eq!(cdef_index0_ctx(Some(0), Some(0)), 3);
        assert_eq!(cdef_index0_ctx(Some(2), Some(3)), 0);
    }

    #[test]
    fn size_group_covers_all_sizes() {
        assert_eq!(SIZE_GROUP.len(), crate::block_tables::BLOCK_SIZES);
        assert!(SIZE_GROUP.iter().all(|&g| g < 4));
    }

    #[test]
    fn reads_stay_in_range() {
        let d = data();
        for seed in 0..32usize {
            let mut sym = SymbolDecoder::new(&d[seed..], false).unwrap();
            let mut cdfs = CdfSet::new(120);
            let dq = DeltaQParams { delta_q_present: true, delta_q_res: 2, max_q: 255 };
            let mut q = 100;
            for strengths in 1..=8u32 {
                let _ = read_skip(&mut sym, &mut cdfs, 2, seed % 2 == 0).unwrap();
                q = read_delta_qindex(&mut sym, &mut cdfs, &dq, q, true).unwrap();
                assert!((1..=255).contains(&q));
                let c = read_cdef_idx(&mut sym, &mut cdfs, strengths, 1).unwrap();
                assert!(u32::from(c) < strengths);
                assert!(read_ccso_blk(&mut sym, &mut cdfs, 2, Some(1)).unwrap() < 2);
                let _ = read_use_gdf(&mut sym, &mut cdfs).unwrap();
                for mhccp_allowed in [false, true] {
                    let a = read_cfl_alphas(&mut sym, &mut cdfs, &CflInputs { enable_cfl_intra: true, mhccp_allowed, mi_size: 6 }).unwrap();
                    assert!(a.alpha_u.abs() <= 16 && a.alpha_v.abs() <= 16);
                    if a.cfl_index == CFL_EXPLICIT {
                        assert!(a.alpha_u != 0 || a.alpha_v != 0);
                    }
                }
            }
        }
    }
}
