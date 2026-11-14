//! Sequence header OBU (AV2 v1.0.0 § 5.4.1, semantics § 6.4), parsed up to the cropping window.
//!
//! The fields that say what the picture is (profile, level, chroma format, bit depth, layers,
//! maximum size, cropping) are read. The tool configurations that follow (decoder model,
//! layer dependency maps, partition … tile config, film grain flag) belong to the decoder and
//! are parsed by a later step; [`SequenceHeader::parse`] stops before them.

use cfa4_bitstream::BitReader;

use crate::{Error, Result};

/// Number of sequence header ids (`MAX_SEQ_NUM`).
pub const MAX_SEQ_NUM: u32 = 16;

/// `chroma_format_idc` (Table 6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChromaFormat {
    Yuv420,
    Monochrome,
    Yuv444,
    Yuv422,
}

impl ChromaFormat {
    fn from_idc(idc: u32) -> Result<Self> {
        match idc {
            0 => Ok(ChromaFormat::Yuv420),
            1 => Ok(ChromaFormat::Monochrome),
            2 => Ok(ChromaFormat::Yuv444),
            3 => Ok(ChromaFormat::Yuv422),
            _ => Err(Error::Invalid("chroma_format_idc above 3")),
        }
    }

    /// (`SubsamplingX`, `SubsamplingY`).
    pub fn subsampling(self) -> (u8, u8) {
        match self {
            ChromaFormat::Yuv420 | ChromaFormat::Monochrome => (1, 1),
            ChromaFormat::Yuv422 => (1, 0),
            ChromaFormat::Yuv444 => (0, 0),
        }
    }

    /// `NumPlanes`.
    pub fn num_planes(self) -> u8 {
        if self == ChromaFormat::Monochrome { 1 } else { 3 }
    }
}

/// `seq_profile_idc` (Table A.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    Main420_10Ip0,
    Main420_10Ip1,
    Main420_10Ip2,
    Main422_10Ip1,
    Main444_10Ip1,
    Configurable,
    /// 5–30.
    Reserved(u8),
}

impl Profile {
    pub fn from_idc(idc: u8) -> Self {
        match idc {
            0 => Profile::Main420_10Ip0,
            1 => Profile::Main420_10Ip1,
            2 => Profile::Main420_10Ip2,
            3 => Profile::Main422_10Ip1,
            4 => Profile::Main444_10Ip1,
            31 => Profile::Configurable,
            other => Profile::Reserved(other),
        }
    }

    /// The profile label of Table A.1.
    pub fn label(self) -> &'static str {
        match self {
            Profile::Main420_10Ip0 => "Main_420_10_IP0",
            Profile::Main420_10Ip1 => "Main_420_10_IP1",
            Profile::Main420_10Ip2 => "Main_420_10_IP2",
            Profile::Main422_10Ip1 => "Main_422_10_IP1",
            Profile::Main444_10Ip1 => "Main_444_10_IP1",
            Profile::Configurable => "Configurable",
            Profile::Reserved(_) => "reserved",
        }
    }

    /// Whether Table A.1 allows `chroma` in this profile (`None` for a reserved profile).
    pub fn allows(self, chroma: ChromaFormat) -> Option<bool> {
        use ChromaFormat::*;
        Some(match self {
            Profile::Main420_10Ip0 | Profile::Main420_10Ip1 | Profile::Main420_10Ip2 => matches!(chroma, Monochrome | Yuv420),
            Profile::Main422_10Ip1 => matches!(chroma, Monochrome | Yuv420 | Yuv422),
            Profile::Main444_10Ip1 => matches!(chroma, Monochrome | Yuv420 | Yuv444),
            Profile::Configurable => true,
            Profile::Reserved(_) => return None,
        })
    }
}

/// `seq_cropping_win_*_offset`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CroppingWindow {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

/// The leading part of `sequence_header_obu()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceHeader {
    pub seq_header_id: u32,
    pub profile: Profile,
    pub single_picture_header: bool,
    pub level_idx: u8,
    pub tier: u8,
    pub chroma_format: ChromaFormat,
    /// 8 or 10 (`lookup_bitdepth( bit_depth_idc )`, Table 6.3).
    pub bit_depth: u8,
    pub lcr_id: u8,
    pub still_picture: bool,
    pub max_tlayer_id: u8,
    pub max_mlayer_id: u8,
    /// `SeqMaxMlayerCnt`.
    pub max_mlayer_count: u8,
    pub monotonic_output_order: bool,
    /// `max_frame_width_minus_1 + 1`.
    pub max_frame_width: u32,
    /// `max_frame_height_minus_1 + 1`.
    pub max_frame_height: u32,
    pub cropping: CroppingWindow,
}

/// `uvlc()` (§ 4.11.3): at most `(1 << 32) - 1`.
fn uvlc(r: &mut BitReader<'_>) -> Result<u32> {
    let mut leading_zeros = 0u32;
    while !r.read_bit()? {
        leading_zeros += 1;
        if leading_zeros >= 32 {
            // a conformant stream never gets here (§ 4.11.3 requires leadingZeros < 32)
            return Err(Error::Invalid("uvlc with 32 or more leading zeros"));
        }
    }
    let value = r.read_bits(leading_zeros)?;
    Ok(value.saturating_add((1u32 << leading_zeros) - 1))
}

fn bits_u8(r: &mut BitReader<'_>, n: u32) -> Result<u8> {
    u8::try_from(r.read_bits(n)?).map_err(|_| Error::Invalid("field wider than 8 bits"))
}

impl SequenceHeader {
    /// Parse a sequence header OBU payload (the bytes after the OBU header).
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(payload);
        let seq_header_id = uvlc(&mut r)?;
        if seq_header_id >= MAX_SEQ_NUM {
            return Err(Error::Invalid("seq_header_id not below MAX_SEQ_NUM"));
        }
        let profile = Profile::from_idc(bits_u8(&mut r, 5)?);
        let single_picture_header = r.read_flag()?;
        let level_idx = bits_u8(&mut r, 5)?;
        let tier = if level_idx > 3 && !single_picture_header { bits_u8(&mut r, 1)? } else { 0 };
        let chroma_format = ChromaFormat::from_idc(uvlc(&mut r)?)?;
        let bit_depth = match uvlc(&mut r)? {
            0 => 10,
            1 => 8,
            _ => return Err(Error::Invalid("bit_depth_idc above 1 (reserved)")),
        };
        let (lcr_id, still_picture, max_tlayer_id, max_mlayer_id, max_mlayer_count, monotonic_output_order) = if single_picture_header {
            (0, true, 0, 0, 1, true)
        } else {
            let lcr_id = bits_u8(&mut r, 3)?;
            let still_picture = r.read_flag()?;
            let max_tlayer_id = bits_u8(&mut r, 2)?;
            let max_mlayer_id = bits_u8(&mut r, 3)?;
            let max_mlayer_count = if max_mlayer_id > 0 {
                // n = CeilLog2( max_mlayer_id + 1 )
                let n = u32::from(max_mlayer_id).ilog2() + 1;
                bits_u8(&mut r, n)?.saturating_add(1)
            } else {
                1
            };
            (lcr_id, still_picture, max_tlayer_id, max_mlayer_id, max_mlayer_count, r.read_flag()?)
        };
        let width_bits = r.read_bits(4)? + 1;
        let height_bits = r.read_bits(4)? + 1;
        let max_frame_width = r.read_bits(width_bits)?.saturating_add(1);
        let max_frame_height = r.read_bits(height_bits)?.saturating_add(1);
        let cropping = if r.read_flag()? {
            CroppingWindow { left: uvlc(&mut r)?, right: uvlc(&mut r)?, top: uvlc(&mut r)?, bottom: uvlc(&mut r)? }
        } else {
            CroppingWindow::default()
        };
        Ok(SequenceHeader {
            seq_header_id,
            profile,
            single_picture_header,
            level_idx,
            tier,
            chroma_format,
            bit_depth,
            lcr_id,
            still_picture,
            max_tlayer_id,
            max_mlayer_id,
            max_mlayer_count,
            monotonic_output_order,
            max_frame_width,
            max_frame_height,
            cropping,
        })
    }

    /// The output size after the cropping window (`None` when the window is larger than the frame).
    pub fn cropped_size(&self) -> Option<(u32, u32)> {
        let c = &self.cropping;
        let w = self.max_frame_width.checked_sub(c.left.checked_add(c.right)?)?;
        let h = self.max_frame_height.checked_sub(c.top.checked_add(c.bottom)?)?;
        (w > 0 && h > 0).then_some((w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_bitstream::BitWriter;

    fn write_uvlc(w: &mut BitWriter, v: u32) {
        w.write_ue(v);
    }

    /// A sequence header with the given fields, written per § 5.4.1.
    fn header(profile: u8, level: u8, chroma: u32, bit_depth_idc: u32, max_mlayer_id: u32, (wd, ht): (u32, u32), crop: Option<[u32; 4]>) -> Vec<u8> {
        let mut w = BitWriter::new();
        write_uvlc(&mut w, 3); // seq_header_id
        w.write_bits(u32::from(profile), 5);
        w.write_bit(false); // single_picture_header_flag
        w.write_bits(u32::from(level), 5);
        if level > 3 {
            w.write_bit(true); // seq_tier
        }
        write_uvlc(&mut w, chroma);
        write_uvlc(&mut w, bit_depth_idc);
        w.write_bits(2, 3); // seq_lcr_id
        w.write_bit(false); // still_picture
        w.write_bits(1, 2); // max_tlayer_id
        w.write_bits(max_mlayer_id, 3);
        if max_mlayer_id > 0 {
            let n = (max_mlayer_id).ilog2() + 1;
            w.write_bits(max_mlayer_id, n); // seq_max_mlayer_cnt_minus_1
        }
        w.write_bit(true); // monotonic_output_order_flag
        w.write_bits(15, 4); // frame_width_bits_minus_1
        w.write_bits(15, 4);
        w.write_bits(wd - 1, 16);
        w.write_bits(ht - 1, 16);
        match crop {
            Some(c) => {
                w.write_bit(true);
                for v in c {
                    write_uvlc(&mut w, v);
                }
            }
            None => w.write_bit(false),
        }
        w.align_zero();
        w.finish()
    }

    #[test]
    fn fields_follow_section_5_4_1() {
        let h = SequenceHeader::parse(&header(4, 8, 2, 1, 0, (1920, 1080), Some([0, 0, 0, 8]))).unwrap();
        assert_eq!(h.seq_header_id, 3);
        assert_eq!(h.profile, Profile::Main444_10Ip1);
        assert_eq!((h.level_idx, h.tier), (8, 1));
        assert_eq!((h.chroma_format, h.bit_depth), (ChromaFormat::Yuv444, 8));
        assert_eq!((h.lcr_id, h.max_tlayer_id, h.max_mlayer_id, h.max_mlayer_count), (2, 1, 0, 1));
        assert!(h.monotonic_output_order && !h.still_picture);
        assert_eq!((h.max_frame_width, h.max_frame_height), (1920, 1080));
        assert_eq!(h.cropped_size(), Some((1920, 1072)));
        assert_eq!(h.profile.allows(h.chroma_format), Some(true));
    }

    #[test]
    fn layers_tiers_and_reserved_values() {
        let h = SequenceHeader::parse(&header(0, 2, 0, 0, 3, (64, 48), None)).unwrap();
        assert_eq!((h.tier, h.bit_depth, h.max_mlayer_id, h.max_mlayer_count), (0, 10, 3, 4));
        assert!(SequenceHeader::parse(&header(0, 2, 4, 0, 0, (64, 48), None)).is_err(), "chroma_format_idc 4");
        assert!(SequenceHeader::parse(&header(0, 2, 0, 2, 0, (64, 48), None)).is_err(), "bit_depth_idc 2");
        assert_eq!(Profile::from_idc(9), Profile::Reserved(9));
        assert_eq!(Profile::Reserved(9).allows(ChromaFormat::Yuv420), None);
        assert_eq!(Profile::Main420_10Ip0.allows(ChromaFormat::Yuv444), Some(false));
    }

    #[test]
    fn single_picture_header_infers_the_layer_fields() {
        let mut w = BitWriter::new();
        w.write_ue(0);
        w.write_bits(0, 5);
        w.write_bit(true); // single_picture_header_flag
        w.write_bits(5, 5); // level 5: no tier with a single picture header
        w.write_ue(1); // monochrome
        w.write_ue(1); // 8 bit
        w.write_bits(7, 4);
        w.write_bits(7, 4);
        w.write_bits(99, 8);
        w.write_bits(49, 8);
        w.write_bit(false);
        w.align_zero();
        let h = SequenceHeader::parse(&w.finish()).unwrap();
        assert!(h.still_picture && h.single_picture_header && h.monotonic_output_order);
        assert_eq!((h.tier, h.max_mlayer_count, h.max_frame_width, h.max_frame_height), (0, 1, 100, 50));
        assert_eq!(h.chroma_format.num_planes(), 1);
    }

    #[test]
    fn hostile_payloads_are_errors() {
        assert!(SequenceHeader::parse(&[]).is_err());
        assert!(SequenceHeader::parse(&[0x00; 6]).is_err(), "uvlc with 32+ leading zeros");
        let full = header(0, 2, 0, 0, 0, (64, 48), None);
        for n in 0..full.len().saturating_sub(1) {
            assert!(SequenceHeader::parse(&full[..n]).is_err(), "truncated at {n}");
        }
        // seq_header_id 16 (uvlc '000010001')
        assert!(SequenceHeader::parse(&[0x08, 0x80, 0, 0, 0, 0]).is_err());
        let huge = SequenceHeader { cropping: CroppingWindow { left: u32::MAX, right: 1, top: 0, bottom: 0 }, ..SequenceHeader::parse(&full).unwrap() };
        assert_eq!(huge.cropped_size(), None);
    }
}
