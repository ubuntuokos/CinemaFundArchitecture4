//! Run a QC: hash the file, gather the facts, judge them by the profile, assemble the report.

use std::sync::atomic::{AtomicBool, Ordering};

use cfa4_media::SharedReader;

use crate::expect::Expectation;
use crate::profile::QcProfile;
use crate::report::{FileRef, Links, Parts, QcReport, RenderOutcome, RunInfo, Tools};
use crate::{Error, checks, hash, inspect};

/// What the caller knows about the run besides the file.
#[derive(Debug, Clone)]
pub struct Options {
    /// Decode every sample (the integrity checks). Off: only what the file says about itself.
    pub decode: bool,
    pub links: Links,
    pub render: Option<RenderOutcome>,
    pub input: Option<FileRef>,
    /// Start time, duration, executable, CPU/GPU: filled in by the caller (this crate reads no
    /// clock, so it runs the same everywhere).
    pub run: RunInfo,
}

impl Default for Options {
    fn default() -> Self {
        Options { decode: true, links: Links::default(), render: None, input: None, run: RunInfo::default() }
    }
}

/// Check the media `reader` holds. `path` is what the report shows as the file's path.
/// A cancelled run still returns a report: its verdict is INCOMPLETE, never PASS.
pub fn check(path: &str, reader: SharedReader, profile: &QcProfile, expect: &Expectation, opts: Options, cancel: &AtomicBool) -> Result<QcReport, Error> {
    let bytes = reader.len();
    let sha256 = match hash::sha256_reader(&*reader, cancel) {
        Ok(h) => h,
        Err(Error::Cancelled) => String::new(),
        Err(e) => return Err(e),
    };
    let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let facts = if cancel.load(Ordering::Relaxed) {
        inspect::Facts { cancelled: true, ..inspect::Facts::default() }
    } else {
        inspect::inspect(&name, reader, opts.decode, cancel)
    };
    let mut results = checks::evaluate(profile, &facts, expect);
    let cancelled = facts.cancelled || cancel.load(Ordering::Relaxed);
    if cancelled {
        // results built from a partial decode would claim more than was checked
        for r in &mut results {
            if r.id.starts_with("integrity.") {
                *r = crate::CheckResult::not_run(
                    &r.id,
                    &profile.checks.get(&r.id).cloned().unwrap_or_default(),
                    "the run was cancelled before this check finished",
                );
            }
        }
    }
    let output = FileRef { path: path.to_string(), sha256, bytes };
    let mut run = opts.run;
    run.cancelled = cancelled;
    let parts = Parts { input: opts.input, links: opts.links, render: opts.render, tools: Tools::default(), run, checks: results, human: None };
    QcReport::assemble(profile, output, parts)
}

/// Check a file on disk (read in place, not loaded whole).
#[cfg(any(unix, windows))]
pub fn check_file(path: &std::path::Path, profile: &QcProfile, expect: &Expectation, opts: Options, cancel: &AtomicBool) -> Result<QcReport, Error> {
    let reader = cfa4_media::reader::FileReader::open(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    check(&path.to_string_lossy(), std::sync::Arc::new(reader), profile, expect, opts, cancel)
}
