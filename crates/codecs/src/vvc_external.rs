//! H.266/VVC decoding through an external helper process (ADR 0013, VVC-02).
//!
//! Behind the `vvc-external` cargo feature and the `CFA4_VVC_DECODER` environment variable (the
//! path of `cfa4-vvc-decode`, built by `scripts/dev/vvc-decoder.py`). The router registration
//! waits for the G2 gate (ADR 0013), so nothing opens VVC through this module on its own yet.
//!
//! The helper (`packaging/vvc-decode/cfa4-vvc-decode.c`) wraps Fraunhofer HHI's VVdeC. It runs
//! in its own process, so a crash, hang or garbage output of the decoder becomes a
//! [`CodecError`] here, never a crash of CFA4 (AGENTS.md § 0). The protocol and the process
//! handling are shared with the AV2 helper: see [`crate::external_decoder`].
//!
//! Samples are access units in Annex B form (start codes). MP4 samples (length prefixed) are
//! turned into Annex B with [`cfa4_vvc::to_annexb`] before they reach the decoder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cfa4_bitstream::annexb_nals;

use crate::external_decoder::{ExternalDecoder, HelperKind};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The environment variable naming the helper executable.
pub const DECODER_ENV: &str = "CFA4_VVC_DECODER";
/// The VVC helper (`cfa4-vvc-decode`).
pub static HELPER: HelperKind = HelperKind { magic: *b"CFA4VVCD", label: "VVC", decoder: "VVdeC", thread_name: "cfa4-vvc-helper" };

/// The helper configured in [`DECODER_ENV`], if any.
pub fn configured_helper() -> Option<PathBuf> {
    std::env::var_os(DECODER_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// VVC decoding through `cfa4-vvc-decode`. Samples are access units (Annex B).
pub struct ExternalVvcDecoder(ExternalDecoder);

impl ExternalVvcDecoder {
    /// Start the helper at `path` (`threads` 0 = the decoder's choice).
    pub fn new(path: &Path, threads: usize) -> Result<ExternalVvcDecoder> {
        Ok(ExternalVvcDecoder(ExternalDecoder::new(path, threads, &HELPER)?))
    }

    /// Time the helper gets to answer one request (default one minute).
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.0.set_request_timeout(timeout);
    }

    /// The helper configured in [`DECODER_ENV`].
    pub fn from_env() -> Result<ExternalVvcDecoder> {
        let path = configured_helper().ok_or_else(|| CodecError::Unsupported(format!("VVC: {DECODER_ENV} is not set")))?;
        Self::new(&path, 0)
    }
}

impl VideoDecoder for ExternalVvcDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        self.0.decode(sample, pts)
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.0.flush()
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(is_random_access_unit(sample))
    }
    fn is_disposable(&self, sample: &[u8]) -> bool {
        cfa4_vvc::au_info(&annexb_nals(sample)).non_ref
    }
}

/// Whether decoding can start at an access unit (Annex B): every picture in it is IRAP. The
/// leading pictures of a CRA are left out by the decoder, as for HEVC ([`crate::gop`]).
pub fn is_random_access_unit(au: &[u8]) -> bool {
    cfa4_vvc::au_info(&annexb_nals(au)).irap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nal(t: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1, 0, (t << 3) | 1];
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn random_access_and_disposable_units() {
        // IDR_N_LP slice with its picture header (gdr_or_irap = 1, non_ref = 0).
        let idr = nal(8, &[0xc0, 0x80]);
        // TRAIL slice with its picture header (non_ref = 1).
        let trail = nal(0, &[0xa0, 0x80]);
        assert!(is_random_access_unit(&idr));
        assert!(!is_random_access_unit(&trail));
        assert!(!is_random_access_unit(&[]));
        assert!(cfa4_vvc::au_info(&annexb_nals(&trail)).non_ref);
        assert!(!cfa4_vvc::au_info(&annexb_nals(&idr)).non_ref);
    }

    #[test]
    fn a_missing_or_wrong_helper_is_an_error() {
        assert!(ExternalVvcDecoder::new(Path::new("/nonexistent/cfa4-vvc-decode"), 0).is_err());
        // a program that is not the helper: no hello
        assert!(ExternalVvcDecoder::new(Path::new("/bin/true"), 0).is_err());
    }
}
