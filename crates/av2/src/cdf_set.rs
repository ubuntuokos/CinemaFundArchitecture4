//! The set of all adaptive CDF arrays of a frame or tile (spec §8.2 and §7.5): initialisation
//! from the defaults, `save_cdfs` / `load_cdfs`, the exit-time copy and averaging of §8.2.4,
//! `blend_cdfs` and the frame-end update of §7.5.
//!
//! The arrays are stored in the order of [`crate::cdf_default::ALL`]; a table keeps the spec's
//! layout (`N + 1` entries per CDF: values, rate index, counter).

use crate::cdf_default::{ALL, CdfTable};

/// Names of the tables whose first dimension is the quantizer context (`init_coeff_cdfs`,
/// spec §7.20): only one of the four slices is used, chosen by `base_q_idx`.
const COEFF_TABLES: [&str; 27] = [
    "Default_Txb_Skip_Cdf",
    "Default_Eob_Pt_16_Cdf",
    "Default_Eob_Pt_32_Cdf",
    "Default_Eob_Pt_64_Cdf",
    "Default_Eob_Pt_128_Cdf",
    "Default_Eob_Pt_256_Cdf",
    "Default_Eob_Pt_512_Cdf",
    "Default_Eob_Pt_1024_Cdf",
    "Default_Eob_Extra_Cdf",
    "Default_Dc_Sign_Cdf",
    "Default_V_Txb_Skip_Cdf",
    "Default_Coeff_Base_Eob_Cdf",
    "Default_Coeff_Base_Lf_Eob_Cdf",
    "Default_Coeff_Base_Cdf",
    "Default_Coeff_Base_Lf_Cdf",
    "Default_Coeff_Base_Ph_Cdf",
    "Default_Coeff_Br_Cdf",
    "Default_Coeff_Br_Lf_Cdf",
    "Default_Coeff_Br_Uv_Cdf",
    "Default_Coeff_Base_Lf_Uv_Cdf",
    "Default_Coeff_Base_Lf_Eob_Uv_Cdf",
    "Default_Coeff_Base_Uv_Cdf",
    "Default_Coeff_Base_Eob_Uv_Cdf",
    "Default_Coeff_Base_Bob_Cdf",
    "Default_Coeff_Br_Idtx_Cdf",
    "Default_Coeff_Base_Idtx_Cdf",
    "Default_Idtx_Sign_Cdf",
];

/// All CDF arrays of one context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdfSet {
    arrays: Vec<Vec<u16>>,
}

impl CdfSet {
    /// `init_non_coeff_cdfs()` and `init_coeff_cdfs()` together: the default tables, with the
    /// coefficient tables taken from the quantizer context of `base_q_idx`.
    pub fn new(base_q_idx: u32) -> Self {
        let q = match base_q_idx {
            0..=90 => 0,
            91..=140 => 1,
            141..=190 => 2,
            _ => 3,
        };
        let arrays = ALL
            .iter()
            .map(|t| {
                if is_coeff(t) {
                    let per_q = t.data.len() / t.dims[0];
                    t.data[q * per_q..(q + 1) * per_q].to_vec()
                } else {
                    t.data.to_vec()
                }
            })
            .collect();
        CdfSet { arrays }
    }

    /// Number of arrays (always the number of default tables).
    pub fn len(&self) -> usize {
        self.arrays.len()
    }

    pub fn is_empty(&self) -> bool {
        self.arrays.is_empty()
    }

    /// One array by its index in [`ALL`]; `None` when out of range.
    pub fn array(&self, index: usize) -> Option<&[u16]> {
        self.arrays.get(index).map(Vec::as_slice)
    }

    /// Mutable array by index, for the symbol decoder.
    pub fn array_mut(&mut self, index: usize) -> Option<&mut [u16]> {
        self.arrays.get_mut(index).map(Vec::as_mut_slice)
    }

    /// `frame_end_update_cdf()` (§7.5): take the saved CDFs, with every counter scaled to
    /// `(3 * count) >> 2`.
    pub fn frame_end_update(saved: &CdfSet) -> CdfSet {
        let mut out = saved.clone();
        for (arr, t) in out.arrays.iter_mut().zip(ALL) {
            let n1 = cdf_len(t);
            for cdf in arr.chunks_mut(n1) {
                let last = n1 - 1;
                cdf[last] = ((3 * u32::from(cdf[last])) >> 2) as u16;
            }
        }
        out
    }

    /// `blend_cdfs( ctx )` (§ 5.18): blend this set with the saved context `saved`.
    pub fn blend(&mut self, saved: &CdfSet) {
        for ((arr, sv), t) in self.arrays.iter_mut().zip(&saved.arrays).zip(ALL) {
            let sz = cdf_len(t);
            for (cdf, scdf) in arr.chunks_mut(sz).zip(sv.chunks(sz)) {
                for i in 0..sz - 2 {
                    let v = (1u32 << 15) - ((((1u32 << 15) - u32::from(scdf[i])) + 7 * ((1u32 << 15) - u32::from(cdf[i])) + 4) >> 3);
                    cdf[i] = v as u16;
                }
                let i2 = sz - 1;
                cdf[i2] = ((u32::from(scdf[i2]) + 7 * u32::from(cdf[i2]) + 4) >> 3) as u16;
            }
        }
    }

    /// The `Saved*` update of `exit_symbol()` (§8.2.4) for one finished tile, applied to
    /// `self` (the saved set). `tile` is this tile's final CDF set.
    ///
    /// * Copy mode (`avg == None`): `self` becomes a copy of `tile`.
    /// * Average mode (`avg == Some((tile_num, num_log2))`): the `avg_cdf` accumulation; for
    ///   `tile_num == 0` the destination is first reset as the spec describes.
    pub fn save_from_tile(&mut self, tile: &CdfSet, avg: Option<(u32, u32)>) {
        let Some((tile_num, num_log2)) = avg else {
            *self = tile.clone();
            return;
        };
        for ((dst, src), t) in self.arrays.iter_mut().zip(&tile.arrays).zip(ALL) {
            let sz = cdf_len(t);
            for (cdf, tcdf) in dst.chunks_mut(sz).zip(src.chunks(sz)) {
                if tile_num == 0 {
                    for c in cdf.iter_mut().take(sz - 2) {
                        *c = 1 << 15;
                    }
                    cdf[sz - 2] = tcdf[sz - 2];
                    cdf[sz - 1] = 0;
                }
                for i in 0..sz - 2 {
                    cdf[i] = cdf[i].saturating_sub((((1u32 << 15) - u32::from(tcdf[i])) >> num_log2) as u16);
                }
                cdf[sz - 1] = cdf[sz - 1].saturating_add((u32::from(tcdf[sz - 1]) >> num_log2) as u16);
            }
        }
    }
}

/// Entries per CDF of a table (its last dimension, `N + 1`).
fn cdf_len(t: &CdfTable) -> usize {
    t.dims.last().copied().unwrap_or(1).max(3)
}

fn is_coeff(t: &CdfTable) -> bool {
    COEFF_TABLES.contains(&t.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coeff_tables_have_four_quantizer_contexts() {
        let n = ALL.iter().filter(|t| is_coeff(t)).count();
        assert_eq!(n, 27);
        for t in ALL.iter().filter(|t| is_coeff(t)) {
            assert_eq!(t.dims[0], 4, "{}", t.name);
        }
    }

    #[test]
    fn init_picks_the_quantizer_slice() {
        let skip = ALL.iter().position(|t| t.name == "Default_Txb_Skip_Cdf").unwrap();
        let t = ALL[skip];
        let per_q = t.data.len() / 4;
        for (q, idx) in [(0usize, 0u32), (0, 90), (1, 91), (1, 140), (2, 141), (2, 190), (3, 191), (3, 255)] {
            let s = CdfSet::new(idx);
            assert_eq!(s.array(skip).unwrap(), &t.data[q * per_q..(q + 1) * per_q]);
        }
        let plain = ALL.iter().position(|t| t.name == "Default_Skip_Cdf").unwrap();
        assert_eq!(CdfSet::new(0).array(plain), CdfSet::new(255).array(plain));
        assert_eq!(CdfSet::new(0).len(), 180);
    }

    #[test]
    fn frame_end_update_scales_counters() {
        let mut s = CdfSet::new(0);
        let plain = ALL.iter().position(|t| t.name == "Default_Skip_Cdf").unwrap();
        s.array_mut(plain).unwrap()[2] = 32;
        let u = CdfSet::frame_end_update(&s);
        assert_eq!(u.array(plain).unwrap()[2], 24);
        assert_eq!(u.array(plain).unwrap()[0], s.array(plain).unwrap()[0]);
    }

    #[test]
    fn blend_with_itself_is_identity_on_values() {
        let s = CdfSet::new(100);
        let mut b = s.clone();
        b.blend(&s);
        let plain = ALL.iter().position(|t| t.name == "Default_Skip_Cdf").unwrap();
        // values: 2^15 - ((2^15 - v) * 8 + 4 >> 3) == v; rate index entry is not touched.
        assert_eq!(b.array(plain).unwrap()[..2], s.array(plain).unwrap()[..2]);
    }

    #[test]
    fn averaging_two_equal_tiles_gives_the_tile() {
        let tile = CdfSet::new(50);
        let mut saved = CdfSet::new(0);
        saved.save_from_tile(&tile, Some((0, 1)));
        saved.save_from_tile(&tile, Some((1, 1)));
        let plain = ALL.iter().position(|t| t.name == "Default_Skip_Cdf").unwrap();
        let (a, b) = (saved.array(plain).unwrap(), tile.array(plain).unwrap());
        // a value is 2^15 - 2 * ((2^15 - v) >> 1): equal for even differences, off by one for odd.
        assert!(a[0].abs_diff(b[0]) <= 1);
        assert_eq!(a[1], b[1]);
        let mut copy = CdfSet::new(0);
        copy.save_from_tile(&tile, None);
        assert_eq!(copy, tile);
    }
}
