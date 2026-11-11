//! Stream Copy export (CFA4-SCE-001 v1.5 §2, §9, §18–20; phase P2): the compressed packets of a
//! source go to a new container with their payload bytes unchanged.
//!
//! - **Strict Bit-Exact Stream Copy** ([`stream_copy_strict`], internal id `stream_copy_strict`):
//!   packets are copied with unchanged payload; only the container (boxes, indexes, track ids,
//!   metadata) is new. A combination that would need a payload change is refused with
//!   [`CopyErrorCode::PayloadModificationRequired`], never silently switched to another mode.
//! - The output is written through [`crate::atomic::AtomicOutput`] and published only after the
//!   **Strict Validator** has read it back and found every copied payload byte-identical to the
//!   source (`PAYLOAD_BIT_EXACT: PASS`).
//!
//! - What the MP4 writer cannot carry yet is refused, not dropped: several sample descriptions
//!   per track, encrypted entries, a display matrix, disabled or alternate-group tracks, sample
//!   groups / `sdtp`, uncompressed PCM tracks and sources that do not start at decode time 0.
//!   The validator also compares the sample descriptions, edit lists and track headers.
//!
//! This phase copies whole tracks of an MP4 / QuickTime source into MP4 or QuickTime. Cutting at
//! GOP boundaries (the `ExportExecutionPlan` of `cfa4-smart-edit`), Matroska and transport stream
//! targets (P2a) and Compatible Stream Copy (P3) build on it.

mod strict;
mod validate;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use strict::stream_copy_strict;

use crate::atomic::Overwrite;

/// The error codes of SCE-001 §19 that Stream Copy reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CopyErrorCode {
    CodecUnsupported,
    ContainerUnsupported,
    BoundaryUnsafe,
    ParamsMismatch,
    TimestampInvalid,
    BitstreamUnsupported,
    PayloadModificationRequired,
    TransformationNotAllowed,
    StrictValidationFailed,
    CompatibleValidationFailed,
    SourceChanged,
    IoFailed,
    Cancelled,
    AtomicCommitFailed,
}

impl CopyErrorCode {
    /// The stable code (`COPY_…`, `EXPORT_ATOMIC_COMMIT_FAILED`).
    pub fn code(self) -> &'static str {
        match self {
            CopyErrorCode::CodecUnsupported => "COPY_CODEC_UNSUPPORTED",
            CopyErrorCode::ContainerUnsupported => "COPY_CONTAINER_UNSUPPORTED",
            CopyErrorCode::BoundaryUnsafe => "COPY_BOUNDARY_UNSAFE",
            CopyErrorCode::ParamsMismatch => "COPY_PARAMS_MISMATCH",
            CopyErrorCode::TimestampInvalid => "COPY_TIMESTAMP_INVALID",
            CopyErrorCode::BitstreamUnsupported => "COPY_BITSTREAM_UNSUPPORTED",
            CopyErrorCode::PayloadModificationRequired => "COPY_PAYLOAD_MODIFICATION_REQUIRED",
            CopyErrorCode::TransformationNotAllowed => "COPY_TRANSFORMATION_NOT_ALLOWED",
            CopyErrorCode::StrictValidationFailed => "COPY_STRICT_VALIDATION_FAILED",
            CopyErrorCode::CompatibleValidationFailed => "COPY_COMPATIBLE_VALIDATION_FAILED",
            CopyErrorCode::SourceChanged => "COPY_SOURCE_CHANGED",
            CopyErrorCode::IoFailed => "COPY_IO_FAILED",
            CopyErrorCode::Cancelled => "COPY_CANCELLED",
            CopyErrorCode::AtomicCommitFailed => "EXPORT_ATOMIC_COMMIT_FAILED",
        }
    }
}

/// A refused or failed Stream Copy: the code and a technical explanation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", code.code())]
pub struct CopyError {
    pub code: CopyErrorCode,
    pub message: String,
}

impl CopyError {
    pub(crate) fn new(code: CopyErrorCode, message: impl Into<String>) -> CopyError {
        CopyError { code, message: message.into() }
    }
}

impl From<CopyError> for crate::ExportError {
    fn from(e: CopyError) -> Self {
        match e.code {
            CopyErrorCode::Cancelled => crate::ExportError::Cancelled,
            CopyErrorCode::IoFailed | CopyErrorCode::AtomicCommitFailed | CopyErrorCode::SourceChanged => crate::ExportError::Io(e.to_string()),
            _ => crate::ExportError::Unsupported(e.to_string()),
        }
    }
}

/// The output container of a Stream Copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyContainer {
    Mp4,
    /// QuickTime.
    Mov,
    Matroska,
    /// MPEG-2 transport stream, 188-byte packets.
    MpegTs,
    /// BDAV transport stream, 192-byte packets.
    M2ts,
}

/// A Strict Stream Copy of a whole source.
#[derive(Clone, Debug)]
pub struct StrictCopyRequest {
    pub source: PathBuf,
    pub output: PathBuf,
    pub container: CopyContainer,
    /// Source track indices to copy, in output order; `None` copies every video and audio track.
    pub tracks: Option<Vec<usize>>,
    pub overwrite: Overwrite,
    /// MP4 / QuickTime: put `moov` before the media data.
    pub faststart: bool,
}

impl StrictCopyRequest {
    pub fn new(source: impl Into<PathBuf>, output: impl Into<PathBuf>, container: CopyContainer) -> StrictCopyRequest {
        StrictCopyRequest { source: source.into(), output: output.into(), container, tracks: None, overwrite: Overwrite::Refuse, faststart: false }
    }
}

/// One copied track.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopiedTrack {
    pub source_track: usize,
    pub kind: String,
    pub codec: String,
    pub samples: u64,
    pub payload_bytes: u64,
}

/// A source track that was not copied, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedTrack {
    pub source_track: usize,
    pub reason: String,
}

/// What the Strict Validator checked (SCE-001 §18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrictValidation {
    /// `PAYLOAD_BIT_EXACT: PASS`.
    pub result: String,
    pub packets: u64,
    pub payload_bytes: u64,
    /// Checks that did not run (`VALIDATION_DECODE_NOT_AVAILABLE`…): warnings, not failures.
    pub not_run: Vec<String>,
}

/// The result of a successful Stream Copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyReport {
    /// `stream_copy_strict`.
    pub mode: String,
    pub output: PathBuf,
    pub container: CopyContainer,
    pub bytes: u64,
    pub tracks: Vec<CopiedTrack>,
    pub skipped: Vec<SkippedTrack>,
    pub validation: StrictValidation,
}
