//! Host side of the OpenVDB worker (plan CFA4-DNEG-TECHNOLOGY-ADAPTATION-001, N1; owner approval 2026-11-21).
//!
//! OpenVDB is C++ (Apache-2.0). It runs in the separate process `cfa4-vdb-worker` (`workers/vdb/`), one command
//! per process, and this crate only starts it, bounds its time and output, and parses the JSON it prints.
//! A worker that is missing, crashes, hangs, prints garbage or reports an error becomes an [`Error`]; nothing
//! here can panic on worker output (AGENTS.md §0). No `unsafe`, no OpenVDB linkage in any Rust crate.

mod expr_run;
pub mod tiles;

pub use expr_run::{ExprReport, ExprRequest, PREVIEW_MAX_BYTES, PREVIEW_TIMEOUT};

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Environment variable naming the worker executable.
pub const WORKER_ENV: &str = "CFA4_VDB_WORKER";
/// Environment variable naming the confining launcher (`cfa4-vdb-launch`).
pub const LAUNCHER_ENV: &str = "CFA4_VDB_LAUNCHER";
/// Where an installed CFA4 keeps the launcher.
pub const INSTALLED_LAUNCHER: &str = "/opt/cfa4/bin/cfa4-vdb-launch";
/// Where an installed CFA4 keeps the worker.
pub const INSTALLED_WORKER: &str = "/opt/cfa4/bin/cfa4-vdb-worker";
/// The most the host reads from a worker's stdout.
pub const MAX_OUTPUT: usize = 16 * 1024 * 1024;
/// Default time limit of one worker command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a worker command failed. Always a value, never a panic.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No worker executable (not built or not installed): the engine works as before.
    #[error("OpenVDB worker is not available")]
    Unavailable,
    /// The worker could not be started.
    #[error("cannot start the OpenVDB worker: {0}")]
    Spawn(String),
    /// The worker died (signal or crash) or exited abnormally without a message.
    #[error("the OpenVDB worker failed ({0})")]
    WorkerDied(String),
    /// The worker ran past its time limit and was killed.
    #[error("the OpenVDB worker timed out")]
    Timeout,
    /// The caller cancelled; the worker was killed.
    #[error("the OpenVDB worker was cancelled")]
    Cancelled,
    /// The worker printed more than [`MAX_OUTPUT`] bytes and was killed.
    #[error("the OpenVDB worker produced too much output")]
    OutputTooLarge,
    /// The worker's answer is not the expected JSON.
    #[error("the OpenVDB worker answered with something that is not valid: {0}")]
    BadAnswer(String),
    /// The worker reported an error (bad or truncated file, missing file, ...).
    #[error("OpenVDB: {0}")]
    Reported(String),
    /// The expression does not compile or failed at run time (the diagnostic code and place are in the text).
    #[error("expression: {0}")]
    Expression(String),
    /// The run would exceed a size limit.
    #[error("limit: {0}")]
    Limit(String),
}

/// One grid of a `.vdb` file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GridInfo {
    /// Grid name.
    pub name: String,
    /// Value type as OpenVDB names it (`float`, `vec3s`, `bool`, `ptdataidx32`, ...).
    #[serde(rename = "type")]
    pub value_type: String,
    /// Grid class (`unknown`, `level set`, `fog volume`, `staggered`).
    pub grid_class: String,
    /// Voxel size in world units.
    pub voxel_size: [f64; 3],
    /// Number of active voxels.
    pub active_voxels: u64,
    /// Bounding box of the active voxels (index space).
    pub bbox: Bbox,
    /// Whether this is a points grid.
    pub is_points: bool,
    /// Grid metadata (`key -> text`).
    pub meta: std::collections::BTreeMap<String, String>,
}

/// An index-space bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Bbox {
    /// Lowest active voxel.
    pub min: [i64; 3],
    /// Highest active voxel.
    pub max: [i64; 3],
}

/// What `vdb.inspect` returns.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    /// The grids, in file order.
    pub grids: Vec<GridInfo>,
    /// Number of file-level metadata entries.
    pub file_meta_count: u64,
    /// The worker's peak resident memory in KiB while reading (0 if unknown).
    #[serde(default)]
    pub peak_rss_kb: u64,
}

/// Preview quality: the ray-march step in voxels. A lower level is only ever chosen by the user (N2-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    /// 2 voxels per step: fast.
    Preview,
    /// 1 voxel per step.
    Standard,
    /// Half a voxel per step.
    High,
}

impl Quality {
    /// The name the worker takes.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Quality::Preview => "preview",
            Quality::Standard => "standard",
            Quality::High => "high",
        }
    }

    /// Parses [`Quality::id`].
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [Quality::Preview, Quality::Standard, Quality::High].into_iter().find(|q| q.id() == id)
    }
}

/// What to render.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRequest {
    /// The grid (a float grid); `None` takes the first float grid.
    pub grid: Option<String>,
    /// Picture size in pixels (1..=8192).
    pub width: u32,
    /// Picture size in pixels (1..=8192).
    pub height: u32,
    /// Quality level; never lowered automatically.
    pub quality: Quality,
    /// Camera azimuth in degrees.
    pub azimuth_deg: f64,
    /// Camera elevation in degrees.
    pub elevation_deg: f64,
    /// Hard time limit.
    pub timeout: Duration,
}

impl RenderRequest {
    /// A request with the default view (azimuth 30, elevation 20) and a 10 minute limit.
    #[must_use]
    pub fn new(width: u32, height: u32, quality: Quality) -> Self {
        Self { grid: None, width, height, quality, azimuth_deg: 30.0, elevation_deg: 20.0, timeout: Duration::from_secs(600) }
    }
}

/// What a render reports.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderInfo {
    /// Picture size.
    pub width: u32,
    /// Picture size.
    pub height: u32,
    /// The grid that was rendered.
    pub grid: String,
    /// Ray-march steps per ray.
    pub steps: u32,
    /// The worker's peak resident memory in KiB.
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
}

impl Worker {
    /// A worker at an explicit path.
    #[must_use]
    pub fn at(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into(), timeout: DEFAULT_TIMEOUT, launcher: None }
    }

    /// Finds the worker: `CFA4_VDB_WORKER`, else the installed path. [`Error::Unavailable`] if neither exists.
    /// The confining launcher (`CFA4_VDB_LAUNCHER`, the installed path, or `cfa4-vdb-launch` next to the worker)
    /// is used when found: every command then runs under Landlock and seccomp (N1-8).
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
        let beside = exe.parent().map(|d| d.join("cfa4-vdb-launch"));
        let launcher = std::env::var_os(LAUNCHER_ENV)
            .map(PathBuf::from)
            .into_iter()
            .chain(std::iter::once(PathBuf::from(INSTALLED_LAUNCHER)))
            .chain(beside)
            .find(|p| p.is_file());
        Ok(Self { launcher, ..Self::at(exe) })
    }

    /// Runs every command through this confining launcher (`cfa4-vdb-launch`).
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

    /// `vdb.inspect`: grid list, types, voxel size, active voxel count, bounding box, metadata (N1-1).
    ///
    /// # Errors
    /// Any [`Error`]; a worker failure never panics the caller.
    pub fn inspect(&self, file: &Path) -> Result<Inspection, Error> {
        let out = self.run(&["inspect".as_ref(), file.as_os_str()], &[file], &[])?;
        serde_json::from_str(&out).map_err(|e| Error::BadAnswer(e.to_string()))
    }

    /// Reads `input` and writes it to `output` (round trip, N1-4).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn copy(&self, input: &Path, output: &Path) -> Result<(), Error> {
        let out = self.run(&["copy".as_ref(), input.as_os_str(), output.as_os_str()], &[input], &[output])?;
        match serde_json::from_str::<Ack>(&out) {
            Ok(a) if a.ok => Ok(()),
            Ok(_) => Err(Error::BadAnswer("ok is false".into())),
            Err(e) => Err(Error::BadAnswer(e.to_string())),
        }
    }

    /// Renders a density preview of one float grid to an RGBA PNG (N2): an orthographic ray march whose result does
    /// not depend on the thread count. `cancel` set to `true` kills the worker; the time limit is
    /// [`RenderRequest::timeout`].
    ///
    /// # Errors
    /// Any [`Error`], [`Error::Cancelled`] included.
    pub fn render(&self, input: &Path, output: &Path, req: &RenderRequest, cancel: Option<&AtomicBool>) -> Result<RenderInfo, Error> {
        let (w, h) = (req.width.to_string(), req.height.to_string());
        let (az, el) = (req.azimuth_deg.to_string(), req.elevation_deg.to_string());
        let grid = req.grid.as_deref().unwrap_or("-");
        let out = self.run_with(
            &[
                "render".as_ref(),
                input.as_os_str(),
                grid.as_ref(),
                output.as_os_str(),
                w.as_ref(),
                h.as_ref(),
                req.quality.id().as_ref(),
                az.as_ref(),
                el.as_ref(),
            ],
            &[input],
            &[output],
            req.timeout,
            cancel,
        )?;
        serde_json::from_str(&out).map_err(|e| Error::BadAnswer(e.to_string()))
    }

    /// Writes a small synthetic `.vdb` (tests; the repository holds no binary fixture).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn write_selftest(&self, output: &Path) -> Result<(), Error> {
        let out = self.run(&["selftest-write".as_ref(), output.as_os_str()], &[], &[output])?;
        match serde_json::from_str::<Ack>(&out) {
            Ok(a) if a.ok => Ok(()),
            _ => Err(Error::BadAnswer(out)),
        }
    }

    fn run(&self, args: &[&std::ffi::OsStr], reads: &[&Path], outs: &[&Path]) -> Result<String, Error> {
        self.run_with(args, reads, outs, self.timeout, None)
    }

    pub(crate) fn run_with(
        &self,
        args: &[&std::ffi::OsStr],
        reads: &[&Path],
        outs: &[&Path],
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<String, Error> {
        if !self.exe.is_file() {
            return Err(Error::Unavailable);
        }
        let mut cmd = match &self.launcher {
            Some(l) => {
                let mut c = Command::new(l);
                for r in reads {
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
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    return Err(Error::WorkerDied(e.to_string()));
                }
            }
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Cancelled);
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
