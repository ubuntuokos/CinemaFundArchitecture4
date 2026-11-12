//! Clean-room AV2 bitstream parsing, implemented from the AV2 Bitstream & Decoding Process
//! Specification v1.0.0 (AOMediaCodec/av2-spec).
//!
//! This is the skeleton of the native track (plan AV2-04): OBU framing (Annex B), OBU headers,
//! the leading fields of the sequence header and a stream-scope check. It decodes no pictures yet.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod obu;
pub mod seq;

use obu::{GLOBAL_XLAYER_ID, ObuType, annexb_obus};
use seq::SequenceHeader;

/// Errors from parsing an AV2 bitstream.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("bitstream: {0}")]
    Bits(#[from] cfa4_bitstream::BitError),
    #[error("unsupported AV2 stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Upper bound on OBUs scanned by [`probe_annexb`] before giving up on finding a sequence header.
const PROBE_MAX_OBUS: usize = 4096;

/// What [`probe_annexb`] learned about an Annex B stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    /// The first sequence header of the stream.
    pub sequence_header: SequenceHeader,
    /// OBUs read up to and including the first sequence header.
    pub obus_scanned: usize,
}

/// Find the first sequence header of an Annex B stream and check that the stream is in the
/// first supported scope: one embedded layer and one extended layer (no multi-stream
/// decoder operation, no `obu_xlayer_id` other than 0 and the global one).
///
/// Streams outside that scope are [`Error::Unsupported`], never a panic.
pub fn probe_annexb(data: &[u8]) -> Result<StreamInfo> {
    for (i, obu) in annexb_obus(data).take(PROBE_MAX_OBUS).enumerate() {
        let obu = obu?;
        if obu.header.obu_type == ObuType::Msdo {
            return Err(Error::Unsupported("multi-stream decoder operation"));
        }
        if obu.header.xlayer_id != 0 && obu.header.xlayer_id != GLOBAL_XLAYER_ID {
            return Err(Error::Unsupported("extended layers"));
        }
        if obu.header.obu_type == ObuType::SequenceHeader {
            let sequence_header = SequenceHeader::parse(obu.payload)?;
            if sequence_header.max_mlayer_id > 0 {
                return Err(Error::Unsupported("embedded layers"));
            }
            return Ok(StreamInfo { sequence_header, obus_scanned: i + 1 });
        }
    }
    Err(Error::Invalid("no sequence header in the stream"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_needs_a_sequence_header() {
        // temporal delimiter only
        assert_eq!(probe_annexb(&[0x01, 0x08]), Err(Error::Invalid("no sequence header in the stream")));
        assert!(probe_annexb(&[]).is_err());
    }

    #[test]
    fn probe_rejects_out_of_scope_streams() {
        // MSDO (type 20) without extension
        assert_eq!(probe_annexb(&[0x01, 0x08, 0x01, 20 << 2]), Err(Error::Unsupported("multi-stream decoder operation")));
        // tile group in extended layer 1
        assert_eq!(probe_annexb(&[0x01, 0x08, 0x02, 0x80 | (7 << 2), 0x01]), Err(Error::Unsupported("extended layers")));
    }
}
