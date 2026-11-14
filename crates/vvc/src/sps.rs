//! The leading fields of the sequence parameter set (H.266 § 7.3.2.4), with
//! `profile_tier_level()` (§ 7.3.3.1), `general_constraints_info()` (§ 7.3.3.2) and the
//! subpicture layout skipped over, up to `sps_bitdepth_minus8`.

use cfa4_bitstream::{BitReader, unescape_rbsp};

use crate::nal::{HEADER_LEN, NalHeader, NalType};
use crate::{Error, Result};

/// Fixed-length flags and fields of `general_constraints_info()` between `gci_present_flag` and
/// `gci_num_additional_bits` (§ 7.3.3.2): 71 bits.
const GCI_FIXED_BITS: usize = 71;
/// Largest picture side accepted (H.266 level limits stay far below it).
const MAX_SIDE: u32 = 65_536;

/// Leading fields of a sequence parameter set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sps {
    pub sps_id: u8,
    pub vps_id: u8,
    /// `sps_max_sublayers_minus1 + 1`.
    pub max_sublayers: u8,
    /// 0 = 4:0:0, 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4.
    pub chroma_format_idc: u8,
    /// `CtbLog2SizeY` (5..=7).
    pub log2_ctu_size: u8,
    /// `general_profile_idc` (1 = Main 10, 2 = Main 12, 10 = Main 12 Intra, 17 = Multilayer Main 10,
    /// 33 = Main 4:4:4 10, …); `None` without `profile_tier_level()` in the SPS.
    pub profile_idc: Option<u8>,
    pub tier_high: bool,
    /// `general_level_idc` (level × 16, e.g. 83 = level 5.1).
    pub level_idc: Option<u8>,
    /// `sps_pic_width_max_in_luma_samples` / `…_height_…`: the coded size.
    pub coded_width: u32,
    pub coded_height: u32,
    /// Output size after the conformance window.
    pub width: u32,
    pub height: u32,
    /// Conformance window in luma samples: left, right, top, bottom.
    pub crop: [u32; 4],
    /// `sps_subpic_info_present_flag`.
    pub subpictures: bool,
    /// `BitDepth` (8..=16).
    pub bit_depth: u8,
}

/// `SubWidthC`, `SubHeightC` of a chroma format (Table 2).
fn chroma_subsampling(chroma_format_idc: u8) -> (u32, u32) {
    match chroma_format_idc {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    }
}

/// `Ceil(Log2(n))` for n ≥ 1.
fn ceil_log2(n: u32) -> u32 {
    if n <= 1 { 0 } else { 32 - (n - 1).leading_zeros() }
}

fn skip_bits(r: &mut BitReader<'_>, n: usize) -> Result<()> {
    r.skip(n).map_err(Error::from)
}

fn byte_align(r: &mut BitReader<'_>) {
    if !r.is_byte_aligned() {
        r.byte_align();
    }
}

/// `profile_tier_level(1, max_sublayers_minus1)`: returns profile, tier and level.
fn profile_tier_level(r: &mut BitReader<'_>, max_sublayers_minus1: u32) -> Result<(u8, bool, u8)> {
    let profile = r.read_bits(7)? as u8;
    let tier_high = r.read_flag()?;
    let level = r.read_bits(8)? as u8;
    r.read_flag()?; // ptl_frame_only_constraint_flag
    r.read_flag()?; // ptl_multilayer_enabled_flag
    // general_constraints_info()
    if r.read_flag()? {
        skip_bits(r, GCI_FIXED_BITS)?;
        let additional = r.read_bits(8)? as usize;
        skip_bits(r, additional)?;
    }
    byte_align(r);
    let mut present = [false; 8];
    for i in (0..max_sublayers_minus1 as usize).rev() {
        present[i] = r.read_flag()?;
    }
    byte_align(r); // ptl_reserved_zero_bit
    for i in (0..max_sublayers_minus1 as usize).rev() {
        if present[i] {
            r.read_bits(8)?; // sublayer_level_idc
        }
    }
    let sub_profiles = r.read_bits(8)? as usize;
    skip_bits(r, sub_profiles * 32)?;
    Ok((profile, tier_high, level))
}

/// Skips the subpicture layout (`sps_subpic_info_present_flag` = 1).
fn skip_subpic_info(r: &mut BitReader<'_>, width: u32, height: u32, ctb: u32) -> Result<()> {
    let num_minus1 = r.read_ue()?;
    // Each subpicture holds at least one CTU.
    let ctus = width.div_ceil(ctb) as u64 * height.div_ceil(ctb) as u64;
    if num_minus1 as u64 >= ctus {
        return Err(Error::Invalid("sps_num_subpics_minus1"));
    }
    let (mut independent, mut same_size) = (true, false);
    if num_minus1 > 0 {
        independent = r.read_flag()?;
        same_size = r.read_flag()?;
    }
    let x_bits = ceil_log2(width.div_ceil(ctb)) as usize;
    let y_bits = ceil_log2(height.div_ceil(ctb)) as usize;
    if num_minus1 > 0 {
        for i in 0..=num_minus1 {
            if !same_size || i == 0 {
                if i > 0 && width > ctb {
                    skip_bits(r, x_bits)?;
                }
                if i > 0 && height > ctb {
                    skip_bits(r, y_bits)?;
                }
                if i < num_minus1 && width > ctb {
                    skip_bits(r, x_bits)?;
                }
                if i < num_minus1 && height > ctb {
                    skip_bits(r, y_bits)?;
                }
            }
            if !independent {
                skip_bits(r, 2)?; // treated_as_pic, loop_filter_across_subpic
            }
        }
    }
    let id_len = r.read_ue()? as usize + 1;
    if id_len > 16 {
        return Err(Error::Invalid("sps_subpic_id_len_minus1"));
    }
    if r.read_flag()? && r.read_flag()? {
        // sps_subpic_id_mapping_explicitly_signalled_flag, sps_subpic_id_mapping_present_flag
        skip_bits(r, (num_minus1 as usize + 1) * id_len)?;
    }
    Ok(())
}

impl Sps {
    /// Parses a whole SPS NAL unit (header included, emulation prevention bytes not removed).
    pub fn parse(nal: &[u8]) -> Result<Sps> {
        let h = NalHeader::parse(nal)?;
        if h.nal_type != NalType::Sps {
            return Err(Error::Invalid("not an SPS NAL unit"));
        }
        let rbsp = unescape_rbsp(nal.get(HEADER_LEN..).unwrap_or_default());
        Sps::parse_rbsp(&rbsp)
    }

    /// Parses the SPS RBSP (after the NAL unit header, emulation prevention bytes removed).
    pub fn parse_rbsp(rbsp: &[u8]) -> Result<Sps> {
        let mut r = BitReader::new(rbsp);
        let sps_id = r.read_bits(4)? as u8;
        let vps_id = r.read_bits(4)? as u8;
        let max_sublayers_minus1 = r.read_bits(3)?;
        if max_sublayers_minus1 > 6 {
            return Err(Error::Invalid("sps_max_sublayers_minus1"));
        }
        let chroma_format_idc = r.read_bits(2)? as u8;
        let log2_ctu_size_minus5 = r.read_bits(2)?;
        if log2_ctu_size_minus5 > 2 {
            return Err(Error::Invalid("sps_log2_ctu_size_minus5"));
        }
        let log2_ctu_size = log2_ctu_size_minus5 as u8 + 5;
        let (mut profile_idc, mut tier_high, mut level_idc) = (None, false, None);
        if r.read_flag()? {
            let (p, t, l) = profile_tier_level(&mut r, max_sublayers_minus1)?;
            profile_idc = Some(p);
            tier_high = t;
            level_idc = Some(l);
        }
        r.read_flag()?; // sps_gdr_enabled_flag
        if r.read_flag()? {
            r.read_flag()?; // sps_res_change_in_clvs_allowed_flag
        }
        let coded_width = r.read_ue()?;
        let coded_height = r.read_ue()?;
        if coded_width == 0 || coded_height == 0 || coded_width > MAX_SIDE || coded_height > MAX_SIDE {
            return Err(Error::Invalid("picture size"));
        }
        let (sub_w, sub_h) = chroma_subsampling(chroma_format_idc);
        let mut crop = [0u32; 4];
        if r.read_flag()? {
            for (k, c) in crop.iter_mut().enumerate() {
                let unit = if k < 2 { sub_w } else { sub_h };
                *c = r.read_ue()?.checked_mul(unit).ok_or(Error::Invalid("conformance window"))?;
            }
        }
        let width = coded_width.checked_sub(crop[0].saturating_add(crop[1])).filter(|w| *w > 0);
        let height = coded_height.checked_sub(crop[2].saturating_add(crop[3])).filter(|h| *h > 0);
        let (Some(width), Some(height)) = (width, height) else {
            return Err(Error::Invalid("conformance window"));
        };
        let subpictures = r.read_flag()?;
        if subpictures {
            skip_subpic_info(&mut r, coded_width, coded_height, 1 << log2_ctu_size)?;
        }
        let bitdepth_minus8 = r.read_ue()?;
        if bitdepth_minus8 > 8 {
            return Err(Error::Invalid("sps_bitdepth_minus8"));
        }
        Ok(Sps {
            sps_id,
            vps_id,
            max_sublayers: max_sublayers_minus1 as u8 + 1,
            chroma_format_idc,
            log2_ctu_size,
            profile_idc,
            tier_high,
            level_idc,
            coded_width,
            coded_height,
            width,
            height,
            crop,
            subpictures,
            bit_depth: bitdepth_minus8 as u8 + 8,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use cfa4_bitstream::{BitWriter, escape_rbsp};

    use super::*;

    /// Options of [`synthetic_sps`].
    #[derive(Clone, Copy)]
    pub(crate) struct Opts {
        pub gci: bool,
        pub sublayers_minus1: u32,
        pub crop: Option<[u32; 4]>,
        pub subpics: bool,
        pub width: u32,
        pub height: u32,
    }

    impl Default for Opts {
        fn default() -> Self {
            Opts { gci: false, sublayers_minus1: 0, crop: None, subpics: false, width: 1920, height: 1080 }
        }
    }

    /// An SPS NAL unit with the fields this module reads, then a plausible tail.
    pub(crate) fn synthetic_sps(o: Opts) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bits(0, 4); // sps_id
        w.write_bits(0, 4); // vps_id
        w.write_bits(o.sublayers_minus1, 3);
        w.write_bits(1, 2); // 4:2:0
        w.write_bits(2, 2); // CTU 128
        w.write_bit(true); // ptl present
        w.write_bits(1, 7); // Main 10
        w.write_bit(false);
        w.write_bits(83, 8); // level 5.1
        w.write_bit(true); // frame only
        w.write_bit(false);
        w.write_bit(o.gci);
        if o.gci {
            for _ in 0..GCI_FIXED_BITS {
                w.write_bit(false);
            }
            w.write_bits(3, 8); // three additional bits
            w.write_bits(0, 3);
        }
        w.align_zero();
        for i in (0..o.sublayers_minus1).rev() {
            w.write_bit(i == 0);
        }
        w.align_zero();
        if o.sublayers_minus1 > 0 {
            w.write_bits(80, 8); // sublayer 0 level
        }
        w.write_bits(1, 8); // one sub-profile
        w.write_bits(0xdead_beef, 32);
        w.write_bit(false); // gdr
        w.write_bit(false); // rpr
        w.write_ue(o.width);
        w.write_ue(o.height);
        match o.crop {
            Some(c) => {
                w.write_bit(true);
                for v in c {
                    w.write_ue(v);
                }
            }
            None => w.write_bit(false),
        }
        w.write_bit(o.subpics);
        if o.subpics {
            // Four subpictures of a 2x2 CTU grid at 128-sample CTUs, not same size, dependent.
            w.write_ue(3);
            w.write_bit(false); // independent
            w.write_bit(false); // same size
            let bits = ceil_log2(o.width.div_ceil(128));
            let ybits = ceil_log2(o.height.div_ceil(128));
            for i in 0..4u32 {
                if i > 0 {
                    w.write_bits(i % 2, bits);
                    w.write_bits(i / 2, ybits);
                }
                if i < 3 {
                    w.write_bits(0, bits);
                    w.write_bits(0, ybits);
                }
                w.write_bit(true);
                w.write_bit(false);
            }
            w.write_ue(7); // id len 8
            w.write_bit(true);
            w.write_bit(true);
            for i in 0..4u32 {
                w.write_bits(i, 8);
            }
        }
        w.write_ue(2); // 10 bit
        w.write_bits(0x5a, 8); // more SPS fields
        w.rbsp_trailing();
        let mut nal = vec![0x00, 15 << 3 | 1];
        nal.extend(escape_rbsp(&w.finish()));
        nal
    }

    #[test]
    fn plain_sps() {
        let s = Sps::parse(&synthetic_sps(Opts::default())).unwrap();
        assert_eq!((s.width, s.height, s.bit_depth, s.chroma_format_idc, s.log2_ctu_size), (1920, 1080, 10, 1, 7));
        assert_eq!((s.profile_idc, s.level_idc, s.tier_high, s.max_sublayers), (Some(1), Some(83), false, 1));
    }

    #[test]
    fn sps_with_gci_sublayers_crop_and_subpictures() {
        let o = Opts { gci: true, sublayers_minus1: 3, crop: Some([0, 4, 0, 4]), subpics: true, width: 256, height: 256 };
        let s = Sps::parse(&synthetic_sps(o)).unwrap();
        assert_eq!((s.coded_width, s.coded_height, s.width, s.height), (256, 256, 248, 248));
        assert_eq!((s.bit_depth, s.max_sublayers, s.subpictures), (10, 4, true));
    }

    #[test]
    fn hostile_sps_is_an_error() {
        let good = synthetic_sps(Opts::default());
        for n in 0..good.len() {
            let _ = Sps::parse(&good[..n]);
        }
        assert!(Sps::parse(&good[..6]).is_err());
        let mut pps = good.clone();
        pps[1] = 16 << 3 | 1;
        assert!(Sps::parse(&pps).is_err());
        let crop_all = Opts { crop: Some([480, 480, 0, 0]), ..Opts::default() };
        assert!(Sps::parse(&synthetic_sps(crop_all)).is_err());
    }
}
