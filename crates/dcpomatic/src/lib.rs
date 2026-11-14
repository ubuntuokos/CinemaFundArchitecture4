#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
//! DCP mastering with DCP-o-matic (ADR 0011).
//!
//! DCP-o-matic (Carl Hetherington, GPL-2.0-or-later, C++) is never linked into CFA4 and never
//! copied into the repository. CFA4 ships its own pinned, command-line-only build of it in a
//! private folder ([`PRIVATE_DIR`], off `PATH`; plan CFA4-DCPOMATIC-001 D2/D9) and runs its tools
//! here as separate processes, behind this Rust interface, as `AGENTS.md` ("Port or keep the
//! original language", "Non-Rust components") requires. A copy the user installed is never used.
//! A crash of DCP-o-matic is an error here, never a CFA4 crash.
//!
//! A run ([`Dcpomatic::make_dcp`]) has two steps:
//! 1. `dcpomatic2_create` writes a DCP-o-matic film directory from the content files (a master
//!    CFA4 exported, image sequences, WAV files, …) and the request's settings;
//! 2. `dcpomatic2_cli make-dcp` encodes the DCP into that directory. Its console progress
//!    (`NN%`) is passed on to [`Control`].
//!
//! Each step starts with an argument array (no shell), in its own process group, with a cleared
//! environment, a private `HOME` / `TMPDIR` / working directory (removed afterwards) and output
//! to files. Cancel sends `SIGINT` to the group, then `SIGKILL`; a timeout sends `SIGKILL`.
//!
//! What this backend adds next to CFA4's own DCP export (`cfa4-export`, `dcp_out.rs`): Interop
//! packages, several content files in one composition, 3D, subtitle files as DCP subtitle tracks,
//! encrypted packages (ST 429-6) and their KDMs (`dcpomatic2_kdm_cli`, [`Dcpomatic::make_kdm`]),
//! and DCP-o-matic's own resizing, colour conversion and audio mapping (ADR 0011).

mod kdm;
mod process;
mod request;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use kdm::{Formulation, KdmOutcome, KdmRequest, Validity, ValidityUnit};
pub use request::{Channel, Container, Content, ContentKind, DcpRequest, Eye, Resolution, Standard};

/// The program that writes the film directory.
pub const CREATE: &str = "dcpomatic2_create";
/// The program that encodes the DCP.
pub const CLI: &str = "dcpomatic2_cli";
/// The program that makes KDMs for encrypted DCPs.
pub const KDM_CLI: &str = "dcpomatic2_kdm_cli";
/// DCP-o-matic's patched OpenSSL, which makes the signing certificate chain on the first run. The
/// other tools look for it in their own folder.
pub const OPENSSL: &str = "dcpomatic2_openssl";
/// The folder of the DCP-o-matic build CFA4 ships (plan CFA4-DCPOMATIC-001 D2/D9). A package
/// built for another prefix sets `CFA4_DCPOMATIC_DIR` at compile time.
pub const PRIVATE_DIR: &str = match option_env!("CFA4_DCPOMATIC_DIR") {
    Some(d) => d,
    None => "/usr/lib/cfa4/dcpomatic",
};
/// For development only: a DCP-o-matic build folder used instead of [`PRIVATE_DIR`]. `PATH` is
/// never searched.
pub const ENV_BIN_DIR: &str = "CFA4_DCPOMATIC_BIN_DIR";
/// The pinned DCP-o-matic release CFA4 ships (D8, DCPOM-01 audit); an older one is refused.
pub const MIN_VERSION: Version = Version { major: 2, minor: 18, patch: 51 };

/// A DCP-o-matic release number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Reads `dcpomatic version 2.18.20 0123abcd` (the first line of `--version`); a development
    /// build prints `2.19.2devel` or `2.19.2-12-gabc`, whose leading digits count.
    pub fn parse(text: &str) -> Option<Version> {
        let line = text.lines().find_map(|l| l.trim().strip_prefix("dcpomatic version "))?;
        let word = line.split_whitespace().next()?;
        let mut parts = word.splitn(3, '.');
        let num = |s: Option<&str>| -> Option<u32> {
            let s = s?;
            let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        };
        Some(Version { major: num(parts.next())?, minor: num(parts.next())?, patch: num(parts.next()).unwrap_or(0) })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Why a run did not produce a DCP.
#[derive(Debug, thiserror::Error)]
pub enum DcpomaticError {
    /// DCP-o-matic is not installed, cannot start or is too old.
    #[error("DCP-o-matic is not available: {0}")]
    Unavailable(String),
    /// The request cannot be run (checked before anything starts).
    #[error("invalid DCP-o-matic request: {0}")]
    Invalid(String),
    /// A DCP-o-matic tool failed; the message is its last line of output.
    #[error("{0}")]
    Engine(String),
    #[error("cancelled")]
    Cancelled,
    #[error("DCP-o-matic did not finish within {} s", .0.as_secs())]
    TimedOut(Duration),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The phase a run is in, for progress reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// `dcpomatic2_create` is writing the film directory.
    Create,
    /// `dcpomatic2_cli` is encoding the DCP.
    Encode,
    /// `dcpomatic2_kdm_cli` is making KDMs.
    Kdm,
}

/// Cancellation and progress for one run. Both hooks are called from the running thread.
#[derive(Default)]
pub struct Control {
    cancelled: Option<Box<dyn Fn() -> bool + Send + Sync>>,
    progress: Option<Box<dyn Fn(Phase, f32) + Send + Sync>>,
}

impl Control {
    pub fn new() -> Control {
        Control::default()
    }

    /// `f` is asked regularly; once it returns `true` the run stops.
    pub fn with_cancel(mut self, f: impl Fn() -> bool + Send + Sync + 'static) -> Control {
        self.cancelled = Some(Box::new(f));
        self
    }

    /// `f` gets the phase and DCP-o-matic's progress in it (0..=1).
    pub fn with_progress(mut self, f: impl Fn(Phase, f32) + Send + Sync + 'static) -> Control {
        self.progress = Some(Box::new(f));
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.as_ref().is_some_and(|f| f())
    }

    pub(crate) fn report(&self, phase: Phase, fraction: f32) {
        if let Some(f) = &self.progress {
            f(phase, fraction.clamp(0.0, 1.0));
        }
    }
}

/// What a successful run made.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DcpOutcome {
    /// The DCP folder (the one holding the asset map), inside [`DcpOutcome::film_dir`].
    pub dcp: PathBuf,
    /// The DCP-o-matic film directory (`metadata.xml`, the log and the DCP).
    pub film_dir: PathBuf,
    /// The DCP-o-matic that made it.
    pub version: Version,
}

/// The DCP-o-matic backend: where its two programs are, and how long a run may take.
#[derive(Clone, Debug)]
pub struct Dcpomatic {
    create: PathBuf,
    cli: PathBuf,
    kdm: Option<PathBuf>,
    timeout: Duration,
}

impl Dcpomatic {
    /// A backend that runs the given programs.
    pub fn new(create: impl Into<PathBuf>, cli: impl Into<PathBuf>) -> Dcpomatic {
        Dcpomatic { create: create.into(), cli: cli.into(), kdm: None, timeout: Duration::from_secs(48 * 60 * 60) }
    }

    /// The KDM program ([`KDM_CLI`]); without it [`Dcpomatic::make_kdm`] is unavailable.
    pub fn with_kdm_program(mut self, kdm: impl Into<PathBuf>) -> Dcpomatic {
        self.kdm = Some(kdm.into());
        self
    }

    /// The folder searched for the tools: [`ENV_BIN_DIR`] when it is set (development),
    /// otherwise [`PRIVATE_DIR`]. Never `PATH`.
    pub fn search_dir() -> PathBuf {
        Dcpomatic::search_dir_for(std::env::var_os(ENV_BIN_DIR))
    }

    /// [`Dcpomatic::search_dir`] for a given value of [`ENV_BIN_DIR`].
    pub(crate) fn search_dir_for(var: Option<std::ffi::OsString>) -> PathBuf {
        var.filter(|v| !v.is_empty()).map_or_else(|| PathBuf::from(PRIVATE_DIR), PathBuf::from)
    }

    /// The tools CFA4 ships, in [`Dcpomatic::search_dir`]. `Unavailable` names what is missing
    /// when [`CREATE`], [`CLI`] or [`OPENSSL`] is not there. [`KDM_CLI`] is taken when present;
    /// without it only KDMs are unavailable.
    pub fn locate() -> Result<Dcpomatic, DcpomaticError> {
        Dcpomatic::locate_in(&Dcpomatic::search_dir())
    }

    /// The tools in `dir` (see [`Dcpomatic::locate`]).
    pub fn locate_in(dir: &Path) -> Result<Dcpomatic, DcpomaticError> {
        let missing: Vec<&str> = [CREATE, CLI, OPENSSL].into_iter().filter(|n| !dir.join(n).is_file()).collect();
        if !missing.is_empty() {
            return Err(DcpomaticError::Unavailable(format!("{} is missing from {}", missing.join(", "), dir.display())));
        }
        let d = Dcpomatic::new(dir.join(CREATE), dir.join(CLI));
        Ok(if dir.join(KDM_CLI).is_file() { d.with_kdm_program(dir.join(KDM_CLI)) } else { d })
    }

    /// The longest a whole run may take (default 48 hours: a 4K feature on a small machine is
    /// slow); the process group is killed after it.
    pub fn with_timeout(mut self, timeout: Duration) -> Dcpomatic {
        self.timeout = timeout;
        self
    }

    pub fn create_program(&self) -> &Path {
        &self.create
    }

    pub fn cli_program(&self) -> &Path {
        &self.cli
    }

    pub fn kdm_program(&self) -> Option<&Path> {
        self.kdm.as_deref()
    }

    /// The installed version (from `dcpomatic2_cli --version`); `Unavailable` when it cannot be
    /// run, prints no version or is older than [`MIN_VERSION`].
    pub fn version(&self) -> Result<Version, DcpomaticError> {
        let out = process::run_version(&self.cli, Duration::from_secs(60))?;
        let v = Version::parse(&out).ok_or_else(|| DcpomaticError::Unavailable(format!("{} --version printed no DCP-o-matic version", self.cli.display())))?;
        if v < MIN_VERSION {
            return Err(DcpomaticError::Unavailable(format!("DCP-o-matic {v} is older than {MIN_VERSION}, the oldest this adapter supports")));
        }
        Ok(v)
    }

    /// Makes a DCP. The film directory must not exist yet; it is created, and the DCP is written
    /// inside it. On cancel or timeout the partly written film directory is removed; after an
    /// engine error it is kept, since DCP-o-matic's log in it says what went wrong.
    pub fn make_dcp(&self, request: &DcpRequest, control: &Control) -> Result<DcpOutcome, DcpomaticError> {
        let plan = request.plan()?;
        let version = self.version()?;
        if control.is_cancelled() {
            return Err(DcpomaticError::Cancelled);
        }
        let deadline = std::time::Instant::now() + self.timeout;
        let film = plan.film_dir.clone();
        let cleanup = |e: DcpomaticError| {
            if matches!(e, DcpomaticError::Cancelled | DcpomaticError::TimedOut(_)) {
                let _ = std::fs::remove_dir_all(&film);
            }
            e
        };
        if let Some(cfg) = &plan.config_dir {
            create_private_dir(cfg)?;
        }

        control.report(Phase::Create, 0.0);
        process::run_step(&self.create, &plan.create_args, deadline, self.timeout, control, Phase::Create).map_err(cleanup)?;
        if !film.join("metadata.xml").is_file() {
            return Err(DcpomaticError::Engine(format!("{CREATE} finished but wrote no film directory at {}", film.display())));
        }
        control.report(Phase::Create, 1.0);

        control.report(Phase::Encode, 0.0);
        process::run_step(&self.cli, &plan.cli_args, deadline, self.timeout, control, Phase::Encode).map_err(cleanup)?;
        let dcp = find_dcp(&film).ok_or_else(|| DcpomaticError::Engine(format!("{CLI} finished but no DCP (ASSETMAP) is in {}", film.display())))?;
        control.report(Phase::Encode, 1.0);
        Ok(DcpOutcome { dcp, film_dir: film, version })
    }
}

/// Creates `dir` (and its parents) with mode 0700 when it does not exist yet.
pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// The DCP folder DCP-o-matic wrote into the film directory: the subdirectory with an asset map
/// (`ASSETMAP.xml` for SMPTE, `ASSETMAP` for Interop). With several, the newest.
fn find_dcp(film: &Path) -> Option<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(film)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && (p.join("ASSETMAP.xml").is_file() || p.join("ASSETMAP").is_file()))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

#[cfg(test)]
mod tests;
