//! DCP validation (DCP-03 of `docs/plans/DCP-VALIDATION-2026-10-27.md`): one API for every
//! validation backend, the seven result states, the package verdict, versioned profiles and the
//! report. The native backend ([`NativeValidator`]) reads the package with `cfa4-dcp`; the
//! `dcp_inspect` reference engine (DCP-04) will implement the same [`DcpValidator`] trait, so both
//! produce the same [`ValidationReport`].
//!
//! The rules ([`rules::RULES`]) have stable ids (`DCP.PKL.003`). A profile names the rules it needs
//! and which of them are required. A backend reports every rule of the profile: a rule it does not
//! implement is `Unchecked`, never silently left out, so an incomplete check can never add up to
//! `PASS` (plan §5–6).
//!
//! Validation is read-only on the package. Paths from the asset map are resolved inside the
//! package root only: absolute paths, `..` and symbolic links that lead outside are refused
//! (plan §9).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod integrity;
mod native;
pub mod package;
pub mod profile;
pub mod report;
pub mod rules;

pub use native::NativeValidator;
pub use profile::{Profile, ProfileRule};
pub use report::{Counts, DocumentIds, ValidationReport, Verdict};

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

/// The result of one check (plan §6). Only `Pass` and `NotApplicable` count towards a `PASS`
/// verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Pass,
    NotApplicable,
    Warning,
    /// Left out on purpose (a profile option, or the check needs something the request excluded).
    Skipped,
    /// Could not be checked: the backend does not implement the rule, or its input is missing.
    Unchecked,
    /// The validation was cancelled before the check finished.
    Interrupted,
    Fail,
}

/// The four states of the Media QC report (`cfa4-qc`, #255 / #302) a DCP status maps to (DCP-00 §6.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QcStatus {
    Pass,
    Warn,
    Fail,
    NotRun,
}

impl Status {
    /// The Media QC state; `None` for `NotApplicable`, which a QC report leaves out.
    pub fn qc(self) -> Option<QcStatus> {
        match self {
            Status::Pass => Some(QcStatus::Pass),
            Status::Warning => Some(QcStatus::Warn),
            Status::Fail => Some(QcStatus::Fail),
            Status::Skipped | Status::Unchecked | Status::Interrupted => Some(QcStatus::NotRun),
            Status::NotApplicable => None,
        }
    }
}

/// Where a finding points. Paths are relative to the package root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Element path inside the document, as `cfa4-dcp` writes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<cfa4_dcp::Uuid>,
}

impl Location {
    pub fn file(file: impl Into<String>) -> Location {
        Location { file: Some(file.into()), ..Location::default() }
    }
}

/// One result of one rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "is_default")]
    pub location: Location,
    pub message: String,
    /// What was compared or measured, e.g. `Size 1024, file 1000 bytes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// How to fix it, when there is a known fix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
}

fn is_default(l: &Location) -> bool {
    *l == Location::default()
}

impl Finding {
    pub fn new(rule: &str, status: Status, message: impl Into<String>) -> Finding {
        Finding { rule: rule.to_string(), status, location: Location::default(), message: message.into(), evidence: None, remedy: None }
    }

    pub fn at(mut self, location: Location) -> Finding {
        self.location = location;
        self
    }

    pub fn evidence(mut self, evidence: impl Into<String>) -> Finding {
        self.evidence = Some(evidence.into());
        self
    }
}

/// What a backend is and which rules it implements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorCapabilities {
    /// `native`, `dcp-inspect`.
    pub id: String,
    pub version: String,
    /// Rule ids the backend checks; every other rule of a profile is reported `Unchecked`.
    pub rules: Vec<String>,
}

/// Resource limits of one validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Per XML document.
    pub document: cfa4_dcp::Limits,
    /// Assets in the asset map; more is a `Fail` of `DCP.AM.002` and the rest is not read.
    pub max_assets: usize,
    /// XML documents (packing lists and playlists) read.
    pub max_documents: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { document: cfa4_dcp::Limits::default(), max_assets: 100_000, max_documents: 1_000 }
    }
}

/// What to validate.
#[derive(Clone, Debug)]
pub struct ValidationRequest {
    /// The package directory (the one holding `ASSETMAP` / `ASSETMAP.xml`).
    pub root: PathBuf,
    pub profile: Profile,
    pub limits: Limits,
}

impl ValidationRequest {
    pub fn new(root: impl Into<PathBuf>, profile: Profile) -> ValidationRequest {
        ValidationRequest { root: root.into(), profile, limits: Limits::default() }
    }
}

/// Progress of a running validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// `discover`, `documents`, `rules`.
    pub stage: String,
    pub done: u64,
    pub total: u64,
}

/// Cancellation and progress for a running validation. The engine runs `validate` as a background
/// job; the GUI thread only sets the flag and reads progress.
#[derive(Clone, Default)]
pub struct ValidationControl {
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(Progress) + Send + Sync>>,
}

impl std::fmt::Debug for ValidationControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidationControl").field("cancelled", &self.is_cancelled()).finish_non_exhaustive()
    }
}

impl ValidationControl {
    pub fn new() -> ValidationControl {
        ValidationControl::default()
    }

    pub fn with_progress(mut self, f: impl Fn(Progress) + Send + Sync + 'static) -> ValidationControl {
        self.progress = Some(Arc::new(f));
        self
    }

    /// The flag another thread sets to cancel.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn report(&self, stage: &str, done: u64, total: u64) {
        if let Some(f) = &self.progress {
            f(Progress { stage: stage.to_string(), done, total });
        }
    }
}

/// Errors that prevent a report. A broken package is not an error: it is a report with `Fail`s.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("package root {0} does not exist or is not a directory")]
    NotADirectory(PathBuf),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("backend failed: {0}")]
    Backend(String),
}

/// A validation backend.
pub trait DcpValidator: Send + Sync {
    fn capabilities(&self) -> ValidatorCapabilities;

    /// Validates synchronously; long checks poll `control` for cancellation.
    fn validate(&self, request: &ValidationRequest, control: &ValidationControl) -> Result<ValidationReport, ValidationError>;
}

#[cfg(test)]
mod tests;
