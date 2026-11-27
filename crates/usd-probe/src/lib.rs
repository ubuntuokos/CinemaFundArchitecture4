//! Host side of the OpenUSD probe worker (docs/plans/CFA4-OPENUSD-SOURCEBUILD-001-0.1.md, S5, SB-G6).
//!
//! OpenUSD is C++ (TOST-1.0). The probe runs in the separate process `cfa4-usd-probe` (`workers/usd-probe/`),
//! one command per process, and this crate only starts it, bounds its time and output, and parses the JSON it
//! prints. A worker that is missing, crashes, hangs, prints garbage or reports an error becomes an [`Error`];
//! nothing here can panic on worker output (AGENTS.md §0). No `unsafe`, no OpenUSD linkage in any Rust crate.
//! This is the G-USD-1 probe, not the U2 worker: it answers only what the sandbox and fuzz gate need.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Environment variable naming the worker executable.
pub const WORKER_ENV: &str = "CFA4_USD_PROBE";
/// Environment variable naming the OpenUSD install (`build-usd.sh` prefix) the probe loads its libraries from.
pub const ROOT_ENV: &str = "CFA4_USD_ROOT";
/// Environment variable naming the confining launcher (`cfa4-usd-probe-launch`).
pub const LAUNCHER_ENV: &str = "CFA4_USD_PROBE_LAUNCHER";
/// Where an installed CFA4 keeps the launcher.
pub const INSTALLED_LAUNCHER: &str = "/opt/cfa4/bin/cfa4-usd-probe-launch";
/// Where an installed CFA4 keeps the worker.
pub const INSTALLED_WORKER: &str = "/opt/cfa4/bin/cfa4-usd-probe";
/// The most the host reads from a worker's stdout.
pub const MAX_OUTPUT: usize = 16 * 1024 * 1024;
/// Default time limit of one worker command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a worker command failed. Always a value, never a panic.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No worker executable (not built or not installed): the engine works as before.
    #[error("OpenUSD probe worker is not available")]
    Unavailable,
    /// The worker could not be started.
    #[error("cannot start the OpenUSD probe worker: {0}")]
    Spawn(String),
    /// The worker died (signal or crash) or exited abnormally without a message.
    #[error("the OpenUSD probe worker failed ({0})")]
    WorkerDied(String),
    /// The worker ran past its time limit and was killed.
    #[error("the OpenUSD probe worker timed out")]
    Timeout,
    /// The worker printed more than [`MAX_OUTPUT`] bytes and was killed.
    #[error("the OpenUSD probe worker produced too much output")]
    OutputTooLarge,
    /// The worker's answer is not the expected JSON.
    #[error("the OpenUSD probe worker answered with something that is not valid: {0}")]
    BadAnswer(String),
    /// The worker reported an error (bad or truncated file, missing file, ...).
    #[error("OpenUSD: {0}")]
    Reported(String),
}

/// What `inspect` returns: a stage opened without loading payloads.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    /// Stage up axis (`Y` or `Z`).
    pub up_axis: String,
    /// Metres per stage unit.
    pub meters_per_unit: f64,
    /// First time code.
    pub start_time_code: f64,
    /// Last time code.
    pub end_time_code: f64,
    /// Time codes per second.
    pub time_codes_per_second: f64,
    /// Number of prims visited (capped by the worker).
    pub prim_count: u64,
    /// Deepest prim path seen.
    pub max_depth: u64,
    /// Whether the worker stopped at its prim or depth limit.
    pub truncated: bool,
    /// Number of root prims.
    pub root_count: u64,
    /// Names of the first root prims.
    pub root_prims: Vec<String>,
    /// Number of layers in the stage.
    pub layer_count: u64,
    /// The worker's peak resident memory in KiB (0 if unknown).
    #[serde(default)]
    pub peak_rss_kb: u64,
}

#[derive(Deserialize)]
struct Ack {
    ok: bool,
}

/// A configured worker.
#[derive(Debug, Clone)]
pub struct Worker {
    exe: PathBuf,
    timeout: Duration,
    launcher: Option<PathBuf>,
    usd_root: Option<PathBuf>,
    mem_mb: Option<u32>,
}

impl Worker {
    /// A worker at an explicit path.
    #[must_use]
    pub fn at(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into(), timeout: DEFAULT_TIMEOUT, launcher: None, usd_root: None, mem_mb: None }
    }

    /// Finds the worker: `CFA4_USD_PROBE`, else the installed path. [`Error::Unavailable`] if neither exists.
    /// The confining launcher (`CFA4_USD_PROBE_LAUNCHER`, the installed path, or `cfa4-usd-probe-launch` next to the worker)
    /// is used when found: every command then runs under Landlock and seccomp (SB-G6).
    ///
    /// # Errors
    /// [`Error::Unavailable`] when no worker executable is found.
    pub fn discover() -> Result<Self, Error> {
        let exe = match std::env::var_os(WORKER_ENV) {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(INSTALLED_WORKER),
        };
        if !exe.is_file() {
            return Err(Error::Unavailable);
        }
        let beside = exe.parent().map(|d| d.join("cfa4-usd-probe-launch"));
        let launcher = std::env::var_os(LAUNCHER_ENV)
            .map(PathBuf::from)
            .into_iter()
            .chain(std::iter::once(PathBuf::from(INSTALLED_LAUNCHER)))
            .chain(beside)
            .find(|p| p.is_file());
        let usd_root = std::env::var_os(ROOT_ENV).map(PathBuf::from);
        Ok(Self { launcher, usd_root, ..Self::at(exe) })
    }

    /// Runs every command through this confining launcher (`cfa4-usd-probe-launch`).
    #[must_use]
    pub fn with_launcher(mut self, launcher: impl Into<PathBuf>) -> Self {
        self.launcher = Some(launcher.into());
        self
    }

    /// Whether commands run confined.
    #[must_use]
    pub fn is_confined(&self) -> bool {
        self.launcher.is_some()
    }

    /// Sets the time limit of one command.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Makes every command read the OpenUSD install (`root`: libraries and plugin resources) and runs it with an
    /// emptied environment, so no inherited `PXR_*` variable can add a plugin path (plan §9, point 3).
    #[must_use]
    pub fn with_usd_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.usd_root = Some(root.into());
        self
    }

    /// Caps the worker's address space (`RLIMIT_AS`, MiB) instead of its default of 4096. A file that asks for more
    /// makes the worker fail, which is an [`Error`].
    #[must_use]
    pub fn with_memory_limit_mb(mut self, mb: u32) -> Self {
        self.mem_mb = Some(mb);
        self
    }

    /// `inspect`: up axis, units, time range, prim count and depth, root prims, layer count.
    /// The worker may read the file and, for references, its directory: both are granted.
    ///
    /// # Errors
    /// Any [`Error`]; a worker failure never panics the caller.
    pub fn inspect(&self, file: &Path) -> Result<Inspection, Error> {
        let dir = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let out = self.run(&["inspect".as_ref(), file.as_os_str()], &[file, dir], &[])?;
        serde_json::from_str(&out).map_err(|e| Error::BadAnswer(e.to_string()))
    }

    /// Writes a small synthetic scene; the format follows the extension (`.usda`, `.usdc`, `.usdz`).
    /// Tests only: the repository holds no binary fixture.
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn write_selftest(&self, output: &Path) -> Result<(), Error> {
        self.ack(&["selftest-write".as_ref(), output.as_os_str()], output)
    }

    /// Writes a chain of `depth` nested prims (depth tests).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn write_deep(&self, output: &Path, depth: u32) -> Result<(), Error> {
        let n = depth.to_string();
        self.ack(&["selftest-write-deep".as_ref(), output.as_os_str(), n.as_ref()], output)
    }

    /// Writes a `.usda` whose sublayer is itself (cycle tests).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn write_cycle(&self, output: &Path) -> Result<(), Error> {
        self.ack(&["selftest-write-cycle".as_ref(), output.as_os_str()], output)
    }

    fn ack(&self, args: &[&std::ffi::OsStr], output: &Path) -> Result<(), Error> {
        let out = self.run(args, &[], &[output])?;
        match serde_json::from_str::<Ack>(&out) {
            Ok(a) if a.ok => Ok(()),
            _ => Err(Error::BadAnswer(out)),
        }
    }

    fn run(&self, args: &[&std::ffi::OsStr], reads: &[&Path], outs: &[&Path]) -> Result<String, Error> {
        if !self.exe.is_file() {
            return Err(Error::Unavailable);
        }
        let mut cmd = match &self.launcher {
            Some(l) => {
                let mut c = Command::new(l);
                for r in reads.iter().copied().chain(self.usd_root.as_deref()) {
                    c.arg("--read").arg(r);
                }
                for o in outs {
                    c.arg("--out").arg(o);
                }
                c.arg("--").arg(&self.exe);
                c
            }
            None => Command::new(&self.exe),
        };
        if self.usd_root.is_some() {
            cmd.env_clear();
        }
        if let Some(mb) = self.mem_mb {
            cmd.env("CFA4_USD_MEM_LIMIT_MB", mb.to_string());
        }
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        // `ETXTBSY` happens when another thread is still writing a freshly installed executable: retry briefly.
        let mut tries = 0;
        let mut child = loop {
            match cmd.spawn() {
                Ok(c) => break c,
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(Error::Spawn(e.to_string())),
            }
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            return Err(Error::Spawn("no stdout pipe".into()));
        };
        // Read on a thread so a silent worker cannot block the time limit.
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let r = stdout.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut buf);
            let _ = tx.send((r.map(|_| buf), ()));
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    return Err(Error::WorkerDied(e.to_string()));
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Timeout);
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let _ = reader.join();
        let bytes = match rx.recv_timeout(Duration::from_secs(5)) {
            Ok((Ok(b), ())) => b,
            _ => return Err(Error::WorkerDied("output unreadable".into())),
        };
        if bytes.len() > MAX_OUTPUT {
            return Err(Error::OutputTooLarge);
        }
        // The answer is the last line that starts with `{`; a library may print warnings before it.
        let all = String::from_utf8_lossy(&bytes).into_owned();
        let text = all.lines().rev().find(|l| l.starts_with('{')).map_or(all.clone(), str::to_owned);
        // A reported error is JSON `{"error": "..."}` (exit 2); anything else abnormal is a death.
        if let Some(msg) =
            serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("error").and_then(serde_json::Value::as_str).map(str::to_owned))
        {
            return Err(Error::Reported(msg.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        if !status.success() {
            return Err(Error::WorkerDied(format!("{status}")));
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests;
