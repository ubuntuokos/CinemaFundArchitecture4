//! `vvcC`: the VVC decoder configuration record (ISO/IEC 14496-15 § 11.2.4.2), carried in the
//! `vvc1` / `vvi1` sample entries.

use crate::bytes::{BoxBuf, Cur};
use crate::error::{Error, Result};

/// VVC NAL unit types that the record lists without a NAL unit count (DCI, OPI).
const NAL_DCI: u8 = 13;
const NAL_OPI: u8 = 12;

/// One NAL unit array of a `vvcC` record.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VvcNalArray {
    pub completeness: bool,
    /// H.266 `nal_unit_type` (VPS 14, SPS 15, PPS 16, prefix APS 17, prefix SEI 23…).
    pub nal_type: u8,
    pub nalus: Vec<Vec<u8>>,
}

/// `VvcPTLRecord`: the profile, tier and level of the operating point.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VvcPtl {
    pub general_profile_idc: u8,
    pub general_tier_flag: bool,
    pub general_level_idc: u8,
    /// `ptl_frame_only_constraint_flag`, `ptl_multilayer_enabled_flag` and
    /// `general_constraint_info`, as stored (`num_bytes_constraint_info` bytes, at most 63).
    pub constraint_info: Vec<u8>,
    /// `sublayer_level_idc[i]` for `i` from `num_sublayers - 2` down to 0 (`None`: not present).
    pub sublayer_level_idc: Vec<Option<u8>>,
    pub sub_profile_idc: Vec<u32>,
}

/// The `ptl_present_flag` part of the record.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VvcOperatingPoint {
    pub ols_idx: u16,
    /// 1 to 7.
    pub num_sublayers: u8,
    pub constant_frame_rate: u8,
    pub chroma_format_idc: u8,
    pub bit_depth: u8,
    pub ptl: VvcPtl,
    pub max_picture_width: u16,
    pub max_picture_height: u16,
    pub avg_frame_rate: u16,
}

/// `vvcC`: VvcDecoderConfigurationRecord, after the FullBox header.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VvcConfig {
    /// NAL unit length field size of the samples: 1, 2 or 4.
    pub length_size: u8,
    pub operating_point: Option<VvcOperatingPoint>,
    pub arrays: Vec<VvcNalArray>,
}

impl VvcConfig {
    /// Parse a `vvcC` box payload (FullBox: version 0, then the record).
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::new(data, "vvcC");
        let (version, _flags) = c.full_header()?;
        if version != 0 {
            return Err(Error::Invalid(format!("unsupported vvcC version {version}")));
        }
        let b = c.u8()?;
        let length_size = ((b >> 1) & 3) + 1;
        if length_size == 3 {
            return Err(Error::Invalid("vvcC: NAL unit length size 3".into()));
        }
        let operating_point = if b & 1 != 0 { Some(parse_operating_point(&mut c)?) } else { None };
        let narrays = c.u8()? as usize;
        let mut arrays = Vec::new();
        for _ in 0..narrays {
            let t = c.u8()?;
            let nal_type = t & 0x1f;
            let n = if nal_type == NAL_DCI || nal_type == NAL_OPI { 1 } else { c.u16()? as usize };
            c.check_count(n, 2)?;
            let mut nalus = Vec::with_capacity(n);
            for _ in 0..n {
                let len = c.u16()? as usize;
                nalus.push(c.bytes(len)?.to_vec());
            }
            arrays.push(VvcNalArray { completeness: t & 0x80 != 0, nal_type, nalus });
        }
        Ok(VvcConfig { length_size, operating_point, arrays })
    }

    /// The `vvcC` box payload.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u32(0); // version 0, flags 0
        let ls = match self.length_size {
            1 => 0,
            2 => 1,
            _ => 3,
        };
        b.u8(0xf8 | (ls << 1) | u8::from(self.operating_point.is_some()));
        if let Some(op) = &self.operating_point {
            let n = op.num_sublayers.clamp(1, 7);
            b.u16(((op.ols_idx & 0x1ff) << 7) | (u16::from(n) << 4) | (u16::from(op.constant_frame_rate & 3) << 2) | u16::from(op.chroma_format_idc & 3));
            b.u8((op.bit_depth.saturating_sub(8).min(7) << 5) | 0x1f);
            let ptl = &op.ptl;
            let ci = &ptl.constraint_info[..ptl.constraint_info.len().min(63)];
            b.u8(ci.len() as u8);
            b.u8((ptl.general_profile_idc << 1) | u8::from(ptl.general_tier_flag));
            b.u8(ptl.general_level_idc);
            b.bytes(ci);
            if n > 1 {
                // present flags for i = n-2 down to 0, then reserved zero bits to the byte end
                let mut flags = 0u8;
                for k in 0..usize::from(n - 1) {
                    if ptl.sublayer_level_idc.get(k).is_some_and(Option::is_some) {
                        flags |= 0x80 >> k;
                    }
                }
                b.u8(flags);
                for k in 0..usize::from(n - 1) {
                    if let Some(Some(l)) = ptl.sublayer_level_idc.get(k) {
                        b.u8(*l);
                    }
                }
            }
            b.u8(ptl.sub_profile_idc.len().min(255) as u8);
            for p in ptl.sub_profile_idc.iter().take(255) {
                b.u32(*p);
            }
            b.u16(op.max_picture_width);
            b.u16(op.max_picture_height);
            b.u16(op.avg_frame_rate);
        }
        b.u8(self.arrays.len().min(255) as u8);
        for a in self.arrays.iter().take(255) {
            b.u8((u8::from(a.completeness) << 7) | (a.nal_type & 0x1f));
            if a.nal_type != NAL_DCI && a.nal_type != NAL_OPI {
                b.u16(a.nalus.len().min(0xffff) as u16);
            }
            for n in &a.nalus {
                b.u16(n.len().min(0xffff) as u16);
                b.bytes(&n[..n.len().min(0xffff)]);
            }
        }
        b.buf
    }

    /// The parameter set and SEI NAL units of the record, in record order.
    pub fn nalus(&self) -> impl Iterator<Item = &[u8]> {
        self.arrays.iter().flat_map(|a| a.nalus.iter().map(Vec::as_slice))
    }

    /// The sequence parameter sets.
    pub fn sps(&self) -> Vec<&[u8]> {
        self.arrays.iter().filter(|a| a.nal_type == 15).flat_map(|a| a.nalus.iter().map(Vec::as_slice)).collect()
    }
}

fn parse_operating_point(c: &mut Cur<'_>) -> Result<VvcOperatingPoint> {
    let w = c.u16()?;
    let ols_idx = w >> 7;
    let num_sublayers = ((w >> 4) & 7) as u8;
    if num_sublayers == 0 {
        return Err(Error::Invalid("vvcC: num_sublayers 0".into()));
    }
    let constant_frame_rate = ((w >> 2) & 3) as u8;
    let chroma_format_idc = (w & 3) as u8;
    let bit_depth = (c.u8()? >> 5) + 8;
    let nci = usize::from(c.u8()? & 0x3f);
    let b = c.u8()?;
    let general_level_idc = c.u8()?;
    let constraint_info = c.bytes(nci)?.to_vec();
    let mut sublayer_level_idc = Vec::new();
    if num_sublayers > 1 {
        let flags = c.u8()?;
        let present: Vec<bool> = (0..usize::from(num_sublayers - 1)).map(|k| flags & (0x80 >> k) != 0).collect();
        for p in present {
            sublayer_level_idc.push(if p { Some(c.u8()?) } else { None });
        }
    }
    let nsub = usize::from(c.u8()?);
    c.check_count(nsub, 4)?;
    let mut sub_profile_idc = Vec::with_capacity(nsub);
    for _ in 0..nsub {
        sub_profile_idc.push(c.u32()?);
    }
    Ok(VvcOperatingPoint {
        ols_idx,
        num_sublayers,
        constant_frame_rate,
        chroma_format_idc,
        bit_depth,
        ptl: VvcPtl { general_profile_idc: b >> 1, general_tier_flag: b & 1 != 0, general_level_idc, constraint_info, sublayer_level_idc, sub_profile_idc },
        max_picture_width: c.u16()?,
        max_picture_height: c.u16()?,
        avg_frame_rate: c.u16()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VvcConfig {
        VvcConfig {
            length_size: 4,
            operating_point: Some(VvcOperatingPoint {
                ols_idx: 0,
                num_sublayers: 3,
                constant_frame_rate: 1,
                chroma_format_idc: 1,
                bit_depth: 10,
                ptl: VvcPtl {
                    general_profile_idc: 1,
                    general_tier_flag: false,
                    general_level_idc: 35,
                    constraint_info: vec![0x00],
                    sublayer_level_idc: vec![None, Some(32)],
                    sub_profile_idc: vec![0xdead_beef],
                },
                max_picture_width: 1920,
                max_picture_height: 1080,
                avg_frame_rate: 0,
            }),
            arrays: vec![
                VvcNalArray { completeness: true, nal_type: 15, nalus: vec![vec![0x00, 0x79, 1, 2, 3]] },
                VvcNalArray { completeness: true, nal_type: 16, nalus: vec![vec![0x00, 0x81, 4]] },
                VvcNalArray { completeness: false, nal_type: NAL_OPI, nalus: vec![vec![0x00, 0x61, 9]] },
            ],
        }
    }

    #[test]
    fn round_trips() {
        let cfg = sample();
        let bytes = cfg.to_bytes();
        assert_eq!(VvcConfig::parse(&bytes).unwrap(), cfg);
        assert_eq!(cfg.sps(), vec![&[0x00, 0x79, 1, 2, 3][..]]);
        assert_eq!(cfg.nalus().count(), 3);
        let bare = VvcConfig { length_size: 2, operating_point: None, arrays: Vec::new() };
        assert_eq!(VvcConfig::parse(&bare.to_bytes()).unwrap(), bare);
    }

    #[test]
    fn truncated_or_hostile_records_are_errors() {
        let bytes = sample().to_bytes();
        for n in 0..bytes.len() {
            assert!(VvcConfig::parse(&bytes[..n]).is_err(), "prefix {n}");
        }
        let mut v1 = bytes.clone();
        v1[0] = 1;
        assert!(VvcConfig::parse(&v1).is_err());
        // a NAL unit count far beyond the payload
        let huge = [0, 0, 0, 0, 0xfe, 1, 0x8f, 0xff, 0xff];
        assert!(VvcConfig::parse(&huge).is_err());
    }
}
