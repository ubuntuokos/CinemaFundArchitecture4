#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
//! DCP validation backend (DCP-04) that runs the dcp_inspect reference engine.
//!
//! The engine (wolfgangw/backports, GPL-3.0-or-later) is never part of CFA4: it is installed
//! outside the repository by `scripts/dev/setup-dcp-inspect.sh` at one pinned commit
//! ([`ENGINE_COMMIT`]) and runs here as a separate process, with no Ruby in Rust or in the GUI
//! (`docs/audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md` §6.1, DCP-01 §6).
//!
//! A run:
//! - starts the engine's wrapper with an argument array (no shell), in its own process group, with
//!   a private working directory, `HOME` and `TMPDIR` that are removed afterwards;
//! - reads the package only: everything the engine writes stays in that directory;
//! - asks for the structured result with `--dump-result` and maps it onto the catalogue rules
//!   ([`map`]); messages no rule covers are kept under [`REF_ENGINE`] and fail the package;
//! - stops the whole process group on cancel (`SIGINT`, the engine exits with 18) or timeout
//!   (`SIGKILL`).
//!
//! When the engine is not installed the backend is unavailable
//! ([`ValidationError::Unavailable`]); nothing else in CFA4 depends on it.

mod map;
mod process;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use cfa4_dcp_validate::report::{self, BackendInfo, ProfileInfo, SCHEMA};
use cfa4_dcp_validate::rules::{self, REF_ENGINE};
use cfa4_dcp_validate::{
    Counts, DcpValidator, Finding, Status, ValidationControl, ValidationError, ValidationReport, ValidationRequest, ValidatorCapabilities, Verdict,
};

pub use map::EngineResult;

/// The reviewed reference commit (DCP-01, L1). The adapter reads the result JSON of this commit,
/// which has no schema version of its own.
pub const ENGINE_COMMIT: &str = "99d0fd863fe05eb806f1625233f2e93676ead268";
/// The engine version at [`ENGINE_COMMIT`].
pub const ENGINE_VERSION: &str = "1.2026.10.03";
/// The backend id in reports.
pub const BACKEND_ID: &str = "dcp-inspect";
/// The variable that names the engine's wrapper (as `setup-dcp-inspect.sh --print-env` prints it).
pub const ENV_WRAPPER: &str = "CFA4_DCP_INSPECT";

/// Engine exit codes (`lib/dcp_inspect/inspection/runtime.rb` at [`ENGINE_COMMIT`]).
mod exit {
    pub const OK: i32 = 0;
    pub const DCP_ERROR: i32 = 1;
    pub const ARG_NOT_A_DIR: i32 = 4;
    pub const GEM_LOAD_ERROR: i32 = 11;
    pub const REQUIRED_COMMAND_NOT_FOUND: i32 = 12;
    pub const USER_INTERRUPT: i32 = 18;
    pub const RUBY_VERSION_NOT_SUPPORTED: i32 = 19;
    pub const GRACEFUL_SHUTDOWN: i32 = 22;
    pub const LIB_LOAD_ERROR: i32 = 23;
}

/// The dcp_inspect backend.
#[derive(Debug)]
pub struct DcpInspect {
    program: PathBuf,
    timeout: Duration,
    version: OnceLock<Option<String>>,
}

impl DcpInspect {
    /// A backend that runs `program` (the wrapper written by `setup-dcp-inspect.sh --install`).
    pub fn new(program: impl Into<PathBuf>) -> DcpInspect {
        DcpInspect { program: program.into(), timeout: Duration::from_secs(24 * 60 * 60), version: OnceLock::new() }
    }

    /// The installed engine: [`ENV_WRAPPER`] if set, otherwise the wrapper in the default cache
    /// (`${XDG_CACHE_HOME:-~/.cache}/cfa4/dcp-inspect/<commit>/bin/cfa4-dcp-inspect`, or under
    /// `CFA4_DCP_INSPECT_DIR`). `None` when there is no such file.
    pub fn locate() -> Option<DcpInspect> {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let program = match var(ENV_WRAPPER) {
            Some(p) => p,
            None => {
                let root = var("CFA4_DCP_INSPECT_DIR")
                    .or_else(|| var("XDG_CACHE_HOME").map(|c| c.join("cfa4/dcp-inspect")))
                    .or_else(|| var("HOME").map(|h| h.join(".cache/cfa4/dcp-inspect")))?;
                root.join(ENGINE_COMMIT).join("bin/cfa4-dcp-inspect")
            }
        };
        program.is_file().then(|| DcpInspect::new(program))
    }

    /// The longest a run may take; the process group is killed after it (default 24 hours, since
    /// hashing a feature takes long).
    pub fn with_timeout(mut self, timeout: Duration) -> DcpInspect {
        self.timeout = timeout;
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The engine's version (`dcp_inspect v<version>`), or why it cannot be run.
    pub fn version(&self) -> Result<String, ValidationError> {
        let out = process::run_version(&self.program, Duration::from_secs(60))?;
        out.lines()
            .find_map(|l| l.trim().strip_prefix("dcp_inspect v"))
            .map(|v| v.trim().to_string())
            .ok_or_else(|| ValidationError::Unavailable(format!("{} --version printed no dcp_inspect version", self.program.display())))
    }

    fn cached_version(&self) -> Option<String> {
        self.version.get_or_init(|| self.version().ok()).clone()
    }
}

impl DcpValidator for DcpInspect {
    fn capabilities(&self) -> ValidatorCapabilities {
        ValidatorCapabilities {
            id: BACKEND_ID.into(),
            version: self.cached_version().unwrap_or_else(|| "unavailable".into()),
            rules: map::REPORTED.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn validate(&self, request: &ValidationRequest, control: &ValidationControl) -> Result<ValidationReport, ValidationError> {
        let started_at = report::now();
        let root = request.root.canonicalize().map_err(|_| ValidationError::NotADirectory(request.root.clone()))?;
        if !root.is_dir() {
            return Err(ValidationError::NotADirectory(request.root.clone()));
        }
        let Some(version) = self.cached_version() else {
            return Err(ValidationError::Unavailable(format!("dcp_inspect cannot be run from {}", self.program.display())));
        };
        let profile = &request.profile;
        let mut args: Vec<String> = Vec::new();
        // Checks the profile does not ask for are not run: hashing and audio analysis are the slow parts.
        if !profile.contains(rules::PKL_HASH) {
            args.push("--no-hash".into());
        }
        if !profile.contains(rules::AUD_LOUDNESS) {
            args.push("--no-audio-analysis".into());
        }
        control.report("dcp-inspect", 0, 1);
        let run = process::run_engine(&self.program, &args, &root, self.timeout, control)?;
        let finished = |findings: Vec<Finding>, verdict: Verdict, result: Option<&EngineResult>| {
            let root_text = request.root.to_string_lossy().into_owned();
            ValidationReport {
                schema: SCHEMA.into(),
                verdict,
                profile: ProfileInfo { id: profile.id.clone(), version: profile.version, title: profile.title.clone() },
                backend: BackendInfo { id: BACKEND_ID.into(), version: version.clone() },
                root: root_text,
                standard: result.and_then(EngineResult::standard),
                documents: result.map(EngineResult::documents).unwrap_or_default(),
                assets_checked: result.map_or(0, EngineResult::assets_checked),
                counts: Counts::of(&findings),
                findings,
                started_at: started_at.clone(),
                finished_at: report::now(),
            }
        };
        match run.outcome {
            process::Outcome::Cancelled => {
                let findings = interrupted(request, "cancelled while dcp_inspect was running");
                Ok(finished(findings, Verdict::Interrupted, None))
            }
            process::Outcome::TimedOut => Err(ValidationError::Backend(format!("dcp_inspect did not finish within {} s", self.timeout.as_secs()))),
            process::Outcome::Exited(code) => match code {
                Some(exit::OK | exit::DCP_ERROR) => {
                    let Some(text) = run.result else {
                        return Err(ValidationError::Backend(format!("dcp_inspect exited with {} but wrote no result", code.unwrap_or(-1))));
                    };
                    let result = EngineResult::parse(&text)
                        .map_err(|e| ValidationError::Backend(format!("dcp_inspect result is not the {ENGINE_VERSION} form: {e}")))?;
                    let findings = map::findings(&result, profile, &root);
                    let mut verdict = Verdict::of(profile, &findings);
                    if matches!(verdict, Verdict::Pass | Verdict::Incomplete) && findings.iter().any(|f| f.rule == REF_ENGINE && f.status == Status::Fail) {
                        verdict = Verdict::Fail;
                    }
                    control.report("dcp-inspect", 1, 1);
                    Ok(finished(findings, verdict, Some(&result)))
                }
                Some(exit::USER_INTERRUPT | exit::GRACEFUL_SHUTDOWN) => {
                    let findings = interrupted(request, "dcp_inspect was interrupted");
                    Ok(finished(findings, Verdict::Interrupted, None))
                }
                Some(exit::ARG_NOT_A_DIR) => Err(ValidationError::NotADirectory(request.root.clone())),
                Some(c @ (exit::GEM_LOAD_ERROR | exit::REQUIRED_COMMAND_NOT_FOUND | exit::RUBY_VERSION_NOT_SUPPORTED | exit::LIB_LOAD_ERROR)) => {
                    Err(ValidationError::Unavailable(format!("dcp_inspect exited with {c}: {}", run.last_error)))
                }
                Some(c) => Err(ValidationError::Backend(format!("dcp_inspect exited with {c}: {}", run.last_error))),
                None => Err(ValidationError::Backend(format!("dcp_inspect was killed by a signal: {}", run.last_error))),
            },
        }
    }
}

/// Every profile rule as `Interrupted`.
fn interrupted(request: &ValidationRequest, why: &str) -> Vec<Finding> {
    request.profile.rules.iter().map(|pr| Finding::new(&pr.rule, Status::Interrupted, why)).collect()
}

#[cfg(test)]
mod tests;
