//! NAL unit header (H.266 § 7.3.1.2, § 7.4.2.2) and NAL unit types (Table 5).

use crate::{Error, Result};

/// `nal_unit_type` values of H.266 Table 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NalType {
    Trail,
    Stsa,
    Radl,
    Rasl,
    /// `RSV_VCL_4..6`.
    ReservedVcl(u8),
    IdrWRadl,
    IdrNLp,
    Cra,
    Gdr,
    /// `RSV_IRAP_11`.
    ReservedIrap,
    Opi,
    Dci,
    Vps,
    Sps,
    Pps,
    PrefixAps,
    SuffixAps,
    Ph,
    Aud,
    Eos,
    Eob,
    PrefixSei,
    SuffixSei,
    Fd,
    /// `RSV_NVCL_26..27`.
    ReservedNonVcl(u8),
    /// `UNSPEC_28..31`.
    Unspecified(u8),
}

impl NalType {
    /// The type for a 5-bit `nal_unit_type`.
    pub fn from_u8(t: u8) -> NalType {
        match t & 0x1f {
            0 => NalType::Trail,
            1 => NalType::Stsa,
            2 => NalType::Radl,
            3 => NalType::Rasl,
            4..=6 => NalType::ReservedVcl(t & 0x1f),
            7 => NalType::IdrWRadl,
            8 => NalType::IdrNLp,
            9 => NalType::Cra,
            10 => NalType::Gdr,
            11 => NalType::ReservedIrap,
            12 => NalType::Opi,
            13 => NalType::Dci,
            14 => NalType::Vps,
            15 => NalType::Sps,
            16 => NalType::Pps,
            17 => NalType::PrefixAps,
            18 => NalType::SuffixAps,
            19 => NalType::Ph,
            20 => NalType::Aud,
            21 => NalType::Eos,
            22 => NalType::Eob,
            23 => NalType::PrefixSei,
            24 => NalType::SuffixSei,
            25 => NalType::Fd,
            26 | 27 => NalType::ReservedNonVcl(t & 0x1f),
            other => NalType::Unspecified(other),
        }
    }

    /// The 5-bit `nal_unit_type` of this type.
    pub fn to_u8(self) -> u8 {
        match self {
            NalType::Trail => 0,
            NalType::Stsa => 1,
            NalType::Radl => 2,
            NalType::Rasl => 3,
            NalType::ReservedVcl(t) | NalType::ReservedNonVcl(t) | NalType::Unspecified(t) => t,
            NalType::IdrWRadl => 7,
            NalType::IdrNLp => 8,
            NalType::Cra => 9,
            NalType::Gdr => 10,
            NalType::ReservedIrap => 11,
            NalType::Opi => 12,
            NalType::Dci => 13,
            NalType::Vps => 14,
            NalType::Sps => 15,
            NalType::Pps => 16,
            NalType::PrefixAps => 17,
            NalType::SuffixAps => 18,
            NalType::Ph => 19,
            NalType::Aud => 20,
            NalType::Eos => 21,
            NalType::Eob => 22,
            NalType::PrefixSei => 23,
            NalType::SuffixSei => 24,
            NalType::Fd => 25,
        }
    }

    /// A VCL NAL unit (types 0..=11): it carries a coded slice.
    pub fn is_vcl(self) -> bool {
        self.to_u8() <= 11
    }

    /// An IRAP picture (IDR_W_RADL, IDR_N_LP, CRA and the reserved IRAP type, § 3).
    pub fn is_irap(self) -> bool {
        matches!(self, NalType::IdrWRadl | NalType::IdrNLp | NalType::Cra | NalType::ReservedIrap)
    }

    /// An IDR picture: no picture before it in decoding order is referenced (closed GOP).
    pub fn is_idr(self) -> bool {
        matches!(self, NalType::IdrWRadl | NalType::IdrNLp)
    }

    /// A non-VCL type that starts a new access unit when it follows the last VCL NAL unit of a
    /// picture (§ 7.4.2.4.3): AUD, OPI, DCI, VPS, SPS, PPS, prefix APS, PH, prefix SEI and the
    /// types 26, 28 and 29.
    pub fn starts_access_unit(self) -> bool {
        matches!(self.to_u8(), 12 | 13 | 14 | 15 | 16 | 17 | 19 | 20 | 23 | 26 | 28 | 29)
    }
}

/// The two-byte NAL unit header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NalHeader {
    pub nal_type: NalType,
    /// `nuh_layer_id` (0..=55 in conforming streams).
    pub layer_id: u8,
    /// `TemporalId` = `nuh_temporal_id_plus1 - 1`.
    pub temporal_id: u8,
}

/// Length of the NAL unit header in bytes.
pub const HEADER_LEN: usize = 2;

impl NalHeader {
    /// Parses the header at the start of `nal` (the NAL unit without start code or length).
    pub fn parse(nal: &[u8]) -> Result<NalHeader> {
        let [b0, b1, ..] = *nal else {
            return Err(Error::Truncated("NAL unit header"));
        };
        if b0 & 0x80 != 0 {
            return Err(Error::Invalid("forbidden_zero_bit"));
        }
        let tid_plus1 = b1 & 0x07;
        if tid_plus1 == 0 {
            return Err(Error::Invalid("nuh_temporal_id_plus1"));
        }
        Ok(NalHeader { nal_type: NalType::from_u8(b1 >> 3), layer_id: b0 & 0x3f, temporal_id: tid_plus1 - 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_round_trip() {
        for t in 0..32u8 {
            assert_eq!(NalType::from_u8(t).to_u8(), t);
        }
        assert!(NalType::Gdr.is_vcl() && !NalType::Gdr.is_irap());
        assert!(NalType::Cra.is_irap() && !NalType::Cra.is_idr());
        assert!(!NalType::Opi.is_vcl());
        assert!(NalType::Ph.starts_access_unit() && !NalType::SuffixSei.starts_access_unit());
    }

    #[test]
    fn header_fields() {
        // SPS, layer 0, TemporalId 0: 0x00 0x79.
        let h = NalHeader::parse(&[0x00, 0x79]).unwrap();
        assert_eq!(h, NalHeader { nal_type: NalType::Sps, layer_id: 0, temporal_id: 0 });
        // IDR_W_RADL, layer 1, TemporalId 2: type 7 << 3 | 3.
        let h = NalHeader::parse(&[0x01, (7 << 3) | 3]).unwrap();
        assert_eq!((h.nal_type, h.layer_id, h.temporal_id), (NalType::IdrWRadl, 1, 2));
        assert!(NalHeader::parse(&[0x00]).is_err());
        assert!(NalHeader::parse(&[0x80, 0x79]).is_err());
        assert!(NalHeader::parse(&[0x00, 0x78]).is_err());
    }
}
