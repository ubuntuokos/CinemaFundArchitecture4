//! CFA4 media quality control (layer L3).
//!
//! Checks media files before and after a render and records the result as a portable,
//! versioned report. This crate holds the data model and the rules; the checks that measure
//! files come in later stages (`docs/plans/MEDIA-QC-2026-10-27.md`).
//!
//! - [`catalog`]: every check CFA4 knows, with its stable id and unit.
//! - [`profile`]: QC profiles (`cfa4-qc-profile/1`, TOML): which checks run, which are
//!   mandatory, and the bounds for PASS / WARN / FAIL. Built-in: `profiles/*.toml`.
//! - [`report`]: the report (`cfa4-qc-report/1`, JSON) and its verdict, which never passes a
//!   missing, skipped or failed mandatory result.
//! - [`hash`]: SHA-256 of the checked files.
//!
//! QC only reads media and writes its own report: it never changes a project, a timeline or
//! render settings, and it has no cloud service. Measurements use CFA4's own decoders; an
//! FFmpeg cross-check stays NOT_RUN until the `cfa4-ffmpeg` backend exists (ADR 0005).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod catalog;
pub mod hash;
pub mod profile;
pub mod report;

pub use profile::{Bounds, CheckSpec, ProfileRef, QcProfile, Requirement};
pub use report::{CheckResult, FileRef, HumanReview, Parts, QcReport, Status, Verdict};

/// Errors of the QC crate. Every failure is reported, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("i/o: {0}")]
    Io(String),
    #[error("cancelled")]
    Cancelled,
    #[error("invalid QC profile: {0}")]
    InvalidProfile(String),
    #[error("unknown QC profile `{0}`")]
    UnknownProfile(String),
    #[error("invalid QC report: {0}")]
    InvalidReport(String),
}
