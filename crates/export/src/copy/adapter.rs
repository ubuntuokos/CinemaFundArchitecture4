//! Native bitstream adapters of Compatible Stream Copy (CFA4-SCE-001 v1.5 §2.2, §11): the
//! container-level representation changes a transport stream needs, none of which touches what a
//! decoder reconstructs.
//!
//! Every adapter is an entry of the versioned allowlist ([`ALLOWLIST`]): source codec and
//! representation, target representation, what it does and how the validator proves it lossless.
//! Anything not on the list is refused; nothing is converted silently.
//!
//! - **H.264 / HEVC, length-prefixed (`avcC` / `hvcC`) → Annex B:** every NAL unit gets a 4-byte
//!   start code; an access unit delimiter is put first unless the sample already starts with
//!   one; a sync sample that lacks any of the parameter set types a decoder needs to start there
//!   (H.264: SPS, PPS; HEVC: VPS, SPS, PPS) gets the parameter sets of the sample entry in band,
//!   after the delimiter (H.222.0 §2.14 / §2.17). A sample whose own parameter sets differ from
//!   the sample entry's is refused: re-inserting the entry's sets later could undo an in-band
//!   update.
//! - **AAC, raw (`esds`) → ADTS:** a 7-byte header from the AudioSpecificConfig before each frame;
//!   only configurations ADTS signals completely (no 960-sample frames, no core coder).
//!
//! The Compatible Validator does not use this code: it reads the written stream back and checks
//! it on its own terms (`BITSTREAM_EQUIVALENT`, §10).

use cfa4_isobmff::CodecConfig;

use super::{CopyError, CopyErrorCode as Code};
use crate::aac_out::AdtsHeader;

/// Version of the allowlist below; reports carry it.
pub const ALLOWLIST_VERSION: u32 = 1;

/// One allowlisted, lossless representation change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed {
    /// Stable id, in reports and the execution log.
    pub id: &'static str,
    pub codec: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    /// Reference name of the equivalent ffmpeg bitstream filter (CI oracle only).
    pub reference: &'static str,
    /// How the Compatible Validator proves it.
    pub validation: &'static str,
}

/// The allowlist (SCE-001 §2.2, compatibility rule).
pub const ALLOWLIST: &[Allowed] = &[
    Allowed {
        id: "h264_avcc_to_annexb",
        codec: "H.264",
        from: "length-prefixed NAL units (avcC)",
        to: "Annex B with access unit delimiter and in-band SPS / PPS at sync samples",
        reference: "h264_mp4toannexb",
        validation: "the NAL units read back equal the source's, after the inserted delimiter and parameter sets (which must equal the avcC ones)",
    },
    Allowed {
        id: "hevc_hvcc_to_annexb",
        codec: "HEVC",
        from: "length-prefixed NAL units (hvcC)",
        to: "Annex B with access unit delimiter and in-band VPS / SPS / PPS at sync samples",
        reference: "hevc_mp4toannexb",
        validation: "the NAL units read back equal the source's, after the inserted delimiter and parameter sets (which must equal the hvcC ones)",
    },
    Allowed {
        id: "aac_raw_to_adts",
        codec: "AAC",
        from: "raw access units (esds AudioSpecificConfig)",
        to: "ADTS frames",
        reference: "aac_adtstoasc (inverse)",
        validation: "each frame read back is the expected 7-byte header followed by the source access unit",
    },
];

/// H.264 access unit delimiter, primary_pic_type 7 (any slice type).
const H264_AUD: [u8; 2] = [0x09, 0xF0];
/// HEVC access unit delimiter: nal_unit_type 35 (layer 0; the TemporalId goes in the second
/// byte), pic_type 2 (I, P, B).
const HEVC_AUD_TYPE: u8 = 0x46;
const HEVC_AUD_PIC: u8 = 0x50;
const START: [u8; 4] = [0, 0, 0, 1];

/// The adapter of one copied track.
#[derive(Clone, Debug)]
pub enum Adapter {
    AnnexB(AnnexB),
    Adts(Adts),
}

impl Adapter {
    /// The allowlisted adapter for a sample entry's codec, or why there is none.
    pub fn for_codec(codec: &CodecConfig) -> Result<Adapter, CopyError> {
        match codec {
            CodecConfig::Avc(c) => Ok(Adapter::AnnexB(AnnexB {
                hevc: false,
                length_size: usize::from(c.length_size),
                parameter_sets: c.sps.iter().chain(&c.pps).cloned().collect(),
            })),
            CodecConfig::Hevc(c) => Ok(Adapter::AnnexB(AnnexB {
                hevc: true,
                length_size: usize::from(c.length_size),
                parameter_sets: c.arrays.iter().flat_map(|a| a.nalus.iter().cloned()).collect(),
            })),
            CodecConfig::Aac(c) => {
                let header = AdtsHeader::from_asc(&c.asc).map_err(|e| CopyError::new(Code::TransformationNotAllowed, format!("AAC → ADTS: {e}")))?;
                // GASpecificConfig of AAC Main / LC / SSR / LTP (the object types ADTS has):
                // frameLengthFlag, dependsOnCoreCoder
                let ga = c.asc.get(1).copied().unwrap_or(0);
                if ga & 0x04 != 0 {
                    return Err(CopyError::new(Code::TransformationNotAllowed, "AAC → ADTS: 960-sample frames cannot be signalled in ADTS"));
                }
                if ga & 0x02 != 0 {
                    return Err(CopyError::new(Code::TransformationNotAllowed, "AAC → ADTS: a core-coder dependency cannot be signalled in ADTS"));
                }
                Ok(Adapter::Adts(Adts { header }))
            }
            other => Err(CopyError::new(
                Code::CodecUnsupported,
                format!("no allowlisted adapter takes {} into a transport stream (allowlist v{ALLOWLIST_VERSION})", other.name()),
            )),
        }
    }

    /// The allowlist entry this adapter implements.
    pub fn allowed(&self) -> &'static Allowed {
        let id = match self {
            Adapter::AnnexB(a) if a.hevc => "hevc_hvcc_to_annexb",
            Adapter::AnnexB(_) => "h264_avcc_to_annexb",
            Adapter::Adts(_) => "aac_raw_to_adts",
        };
        ALLOWLIST.iter().find(|a| a.id == id).unwrap_or(&ALLOWLIST[0])
    }

    /// The transport stream representation of one sample.
    pub fn apply(&self, sample: &[u8], sync: bool) -> Result<Vec<u8>, CopyError> {
        match self {
            Adapter::AnnexB(a) => a.apply(sample, sync),
            Adapter::Adts(a) => a.apply(sample),
        }
    }
}

/// Length-prefixed → Annex B (H.264 or HEVC).
#[derive(Clone, Debug)]
pub struct AnnexB {
    pub hevc: bool,
    pub length_size: usize,
    /// The sample entry's parameter sets (and SEI arrays of `hvcC`), in record order.
    pub parameter_sets: Vec<Vec<u8>>,
}

impl AnnexB {
    fn nal_type(&self, nal: &[u8]) -> Option<u8> {
        let h = *nal.first()?;
        Some(if self.hevc { (h >> 1) & 0x3F } else { h & 0x1F })
    }
    fn is_aud(&self, nal: &[u8]) -> bool {
        self.nal_type(nal) == Some(if self.hevc { 35 } else { 9 })
    }
    /// The parameter set types a decoder needs to start at a sync sample.
    pub fn required_parameter_sets(&self) -> &'static [u8] {
        if self.hevc { &[32, 33, 34] } else { &[7, 8] }
    }
    fn is_parameter_set(&self, nal: &[u8]) -> bool {
        self.nal_type(nal).is_some_and(|t| self.required_parameter_sets().contains(&t))
    }
    /// The delimiter put before `first` (HEVC: with its TemporalId).
    fn aud(&self, first: Option<&[u8]>) -> Vec<u8> {
        if self.hevc {
            let tid = first.and_then(|n| n.get(1)).map_or(1, |b| (b & 0x07).max(1));
            vec![HEVC_AUD_TYPE, tid, HEVC_AUD_PIC]
        } else {
            H264_AUD.to_vec()
        }
    }

    /// The source sample's NAL units; refused when one could not survive Annex B framing intact
    /// (empty, ending in a zero byte that would read as trailing_zero_8bits, or holding a start
    /// code prefix), or when it is a parameter set other than the sample entry's.
    pub fn source_nals<'a>(&self, sample: &'a [u8]) -> Result<Vec<&'a [u8]>, CopyError> {
        let nals = cfa4_bitstream::length_prefixed_nals(sample, self.length_size)
            .map_err(|e| CopyError::new(Code::BitstreamUnsupported, format!("a length-prefixed sample cannot be split ({e:?})")))?;
        for n in &nals {
            if n.is_empty() || n.last() == Some(&0) || n.windows(3).any(|w| w[0] == 0 && w[1] == 0 && w[2] <= 1) {
                return Err(CopyError::new(
                    Code::BitstreamUnsupported,
                    "a NAL unit would not survive Annex B framing unchanged (empty, zero-terminated or holding a start code)",
                ));
            }
            if !self.parameter_sets.is_empty() && self.is_parameter_set(n) && !self.parameter_sets.iter().any(|p| p.as_slice() == *n) {
                return Err(CopyError::new(
                    Code::BitstreamUnsupported,
                    "a sample carries a parameter set other than the sample entry's; re-inserting the entry's sets at later sync samples could undo it",
                ));
            }
        }
        Ok(nals)
    }

    /// What goes before the source NAL units: (delimiter inserted, parameter sets inserted).
    pub fn insertions(&self, nals: &[&[u8]], sync: bool) -> (bool, bool) {
        let aud = !nals.first().is_some_and(|n| self.is_aud(n));
        let params = sync && self.required_parameter_sets().iter().any(|&t| !nals.iter().any(|n| self.nal_type(n) == Some(t)));
        (aud, params)
    }

    fn apply(&self, sample: &[u8], sync: bool) -> Result<Vec<u8>, CopyError> {
        let nals = self.source_nals(sample)?;
        let (aud, params) = self.insertions(&nals, sync);
        let mut out = Vec::with_capacity(sample.len() + 64);
        let mut rest = nals.as_slice();
        if aud {
            out.extend_from_slice(&START);
            out.extend_from_slice(&self.aud(nals.first().copied()));
        } else if let Some((first, tail)) = rest.split_first() {
            // the sample's own delimiter stays first
            out.extend_from_slice(&START);
            out.extend_from_slice(first);
            rest = tail;
        }
        if params {
            for p in &self.parameter_sets {
                out.extend_from_slice(&START);
                out.extend_from_slice(p);
            }
        }
        for n in rest {
            out.extend_from_slice(&START);
            out.extend_from_slice(n);
        }
        Ok(out)
    }
}

/// Raw AAC → ADTS.
#[derive(Clone, Debug)]
pub struct Adts {
    header: AdtsHeader,
}

impl Adts {
    fn apply(&self, frame: &[u8]) -> Result<Vec<u8>, CopyError> {
        let h = self.header.bytes(frame.len()).map_err(|e| CopyError::new(Code::BitstreamUnsupported, e.to_string()))?;
        let mut out = Vec::with_capacity(frame.len() + 7);
        out.extend_from_slice(&h);
        out.extend_from_slice(frame);
        Ok(out)
    }
}
