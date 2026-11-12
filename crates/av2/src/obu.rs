//! OBU framing and headers (AV2 v1.0.0 § 5.2, § 6.2, Annex B).

use crate::{Error, Result};

/// `obu_xlayer_id` of global scope (§ 3 constants: `GLOBAL_XLAYER_ID`).
pub const GLOBAL_XLAYER_ID: u8 = 31;

/// Longest `leb128()` (§ 4.10.5: at most 8 bytes).
const LEB128_MAX_BYTES: usize = 8;

/// `obu_type` (Table 6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObuType {
    SequenceHeader,
    TemporalDelimiter,
    MultiFrameHeader,
    ClosedLoopKey,
    OpenLoopKey,
    LeadingTileGroup,
    RegularTileGroup,
    MetadataShort,
    MetadataGroup,
    Switch,
    LeadingSef,
    RegularSef,
    LeadingTip,
    RegularTip,
    BufferRemovalTiming,
    LayerConfigurationRecord,
    AtlasSegment,
    OperatingPointSet,
    BridgeFrame,
    Msdo,
    RasFrame,
    QuantizationMatrix,
    FilmGrain,
    ContentInterpretation,
    Padding,
    /// 0 or 26–31: ignored by a conforming decoder (§ 6.2).
    Reserved(u8),
}

impl ObuType {
    /// The type for a 5-bit `obu_type` value.
    pub fn from_code(code: u8) -> Self {
        use ObuType::*;
        match code {
            1 => SequenceHeader,
            2 => TemporalDelimiter,
            3 => MultiFrameHeader,
            4 => ClosedLoopKey,
            5 => OpenLoopKey,
            6 => LeadingTileGroup,
            7 => RegularTileGroup,
            8 => MetadataShort,
            9 => MetadataGroup,
            10 => Switch,
            11 => LeadingSef,
            12 => RegularSef,
            13 => LeadingTip,
            14 => RegularTip,
            15 => BufferRemovalTiming,
            16 => LayerConfigurationRecord,
            17 => AtlasSegment,
            18 => OperatingPointSet,
            19 => BridgeFrame,
            20 => Msdo,
            21 => RasFrame,
            22 => QuantizationMatrix,
            23 => FilmGrain,
            24 => ContentInterpretation,
            25 => Padding,
            other => Reserved(other),
        }
    }

    /// Whether the OBU carries tile data (`is_tile_group()` in § 5.2.1).
    pub fn is_tile_group(self) -> bool {
        matches!(
            self,
            ObuType::LeadingTileGroup | ObuType::RegularTileGroup | ObuType::ClosedLoopKey | ObuType::OpenLoopKey | ObuType::Switch | ObuType::RasFrame
        )
    }
}

/// `obu_header()` (§ 5.2.2), with the inferred layer ids when there is no extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObuHeader {
    pub obu_type: ObuType,
    pub extension: bool,
    pub tlayer_id: u8,
    pub mlayer_id: u8,
    pub xlayer_id: u8,
}

impl ObuHeader {
    /// Parse the 1- or 2-byte header at the start of `obu`; returns it and its length.
    pub fn parse(obu: &[u8]) -> Result<(Self, usize)> {
        let &first = obu.first().ok_or(Error::Truncated("OBU header"))?;
        let extension = first & 0x80 != 0;
        let obu_type = ObuType::from_code((first >> 2) & 0x1f);
        let tlayer_id = first & 0x03;
        if extension {
            let &second = obu.get(1).ok_or(Error::Truncated("OBU extension header"))?;
            Ok((ObuHeader { obu_type, extension, tlayer_id, mlayer_id: second >> 5, xlayer_id: second & 0x1f }, 2))
        } else {
            let xlayer_id = if matches!(obu_type, ObuType::Msdo | ObuType::TemporalDelimiter) { GLOBAL_XLAYER_ID } else { 0 };
            Ok((ObuHeader { obu_type, extension, tlayer_id, mlayer_id: 0, xlayer_id }, 1))
        }
    }
}

/// One OBU of a bitstream: its header and payload (`obuPayloadSize` bytes after the header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obu<'a> {
    pub header: ObuHeader,
    pub payload: &'a [u8],
    /// Byte offset of the OBU's `leb128` size in the bitstream.
    pub offset: usize,
}

/// `leb128()` at the start of `b` (§ 4.10.5): (value, bytes read). The value must fit in 32 bits.
pub fn leb128(b: &[u8]) -> Result<(u32, usize)> {
    let mut value = 0u64;
    for i in 0..LEB128_MAX_BYTES {
        let &byte = b.get(i).ok_or(Error::Truncated("leb128"))?;
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return u32::try_from(value).map(|v| (v, i + 1)).map_err(|_| Error::Invalid("leb128 value over 32 bits"));
        }
    }
    Err(Error::Invalid("leb128 longer than 8 bytes"))
}

/// The OBUs of an Annex B length-delimited bitstream (Annex B.2), in order.
///
/// Yields an error (and then stops) at a truncated or malformed OBU.
pub fn annexb_obus(data: &[u8]) -> AnnexBObus<'_> {
    AnnexBObus { data, pos: 0, done: false }
}

/// Iterator returned by [`annexb_obus`].
pub struct AnnexBObus<'a> {
    data: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> AnnexBObus<'a> {
    fn read(&mut self) -> Result<Obu<'a>> {
        let offset = self.pos;
        let rest = self.data.get(offset..).ok_or(Error::Truncated("OBU"))?;
        let (size, n) = leb128(rest)?;
        let size = usize::try_from(size).map_err(|_| Error::Invalid("OBU size"))?;
        if size == 0 {
            return Err(Error::Invalid("empty OBU (no header)"));
        }
        let obu = rest.get(n..n.checked_add(size).ok_or(Error::Invalid("OBU size"))?).ok_or(Error::Truncated("OBU payload"))?;
        let (header, header_len) = ObuHeader::parse(obu)?;
        let payload = obu.get(header_len..).ok_or(Error::Invalid("OBU smaller than its header"))?;
        self.pos = offset + n + size;
        Ok(Obu { header, payload, offset })
    }
}

impl<'a> Iterator for AnnexBObus<'a> {
    type Item = Result<Obu<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.pos >= self.data.len() {
            return None;
        }
        let r = self.read();
        if r.is_err() {
            self.done = true;
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_follow_section_5_2_2() {
        // temporal delimiter: type 2, no extension -> global xlayer
        let (h, n) = ObuHeader::parse(&[0x08]).unwrap();
        assert_eq!((h.obu_type, h.xlayer_id, h.mlayer_id, n), (ObuType::TemporalDelimiter, GLOBAL_XLAYER_ID, 0, 1));
        // sequence header without extension -> xlayer 0
        let (h, _) = ObuHeader::parse(&[0x04]).unwrap();
        assert_eq!((h.obu_type, h.xlayer_id), (ObuType::SequenceHeader, 0));
        // regular tile group (7), tlayer 2, extension: mlayer 3, xlayer 5
        let (h, n) = ObuHeader::parse(&[0x80 | (7 << 2) | 2, (3 << 5) | 5]).unwrap();
        assert_eq!((h.obu_type, h.tlayer_id, h.mlayer_id, h.xlayer_id, n), (ObuType::RegularTileGroup, 2, 3, 5, 2));
        assert!(h.obu_type.is_tile_group());
        assert_eq!(ObuType::from_code(0), ObuType::Reserved(0));
        assert_eq!(ObuType::from_code(26), ObuType::Reserved(26));
        assert!(ObuHeader::parse(&[]).is_err());
        assert!(ObuHeader::parse(&[0x80]).is_err());
    }

    #[test]
    fn leb128_limits() {
        assert_eq!(leb128(&[0x05]).unwrap(), (5, 1));
        assert_eq!(leb128(&[0x80 | 0x2c, 0x02]).unwrap(), (300, 2));
        assert_eq!(leb128(&[0xff, 0xff, 0xff, 0xff, 0x0f]).unwrap(), (u32::MAX, 5));
        assert!(leb128(&[0xff, 0xff, 0xff, 0xff, 0x10]).is_err(), "over 32 bits");
        assert!(leb128(&[0x80; 8]).is_err(), "longer than 8 bytes");
        assert!(leb128(&[0x80]).is_err(), "truncated");
    }

    #[test]
    fn annex_b_iteration() {
        let s = [0x01, 0x08, 0x03, 0x04, 0xaa, 0xbb, 0x02, 0x1c, 0x01];
        let obus: Vec<_> = annexb_obus(&s).collect::<Result<_>>().unwrap();
        assert_eq!(obus.len(), 3);
        assert_eq!(obus[1].header.obu_type, ObuType::SequenceHeader);
        assert_eq!(obus[1].payload, &[0xaa, 0xbb]);
        assert_eq!(obus[2].offset, 6);
        assert_eq!(obus[2].header.obu_type, ObuType::RegularTileGroup);
        // truncated last OBU: two good ones, then one error, then nothing
        let r: Vec<_> = annexb_obus(&s[..8]).collect();
        assert_eq!(r.len(), 3);
        assert!(r[2].is_err());
        // zero-size OBU
        assert!(annexb_obus(&[0x00]).next().unwrap().is_err());
    }
}
