//! Host side of the OpenVDB worker (plan CFA4-DNEG-TECHNOLOGY-ADAPTATION-001, N1; owner approval 2026-11-21).
//!
//! OpenVDB is C++ (Apache-2.0). It runs in the separate process `cfa4-vdb-worker` (`workers/vdb/`), one command
//! per process, and this crate only starts it, bounds its time and output, and parses the JSON it prints.
//! A worker that is missing, crashes, hangs, prints garbage or reports an error becomes an [`Error`]; nothing
//! here can panic on worker output (AGENTS.md §0). No `unsafe`, no OpenVDB linkage in any Rust crate.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Environment variable naming the worker executable.
pub const WORKER_ENV: &str = "CFA4_VDB_WORKER";
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
    /// The worker printed more than [`MAX_OUTPUT`] bytes and was killed.
    #[error("the OpenVDB worker produced too much output")]
    OutputTooLarge,
    /// The worker's answer is not the expected JSON.
    #[error("the OpenVDB worker answered with something that is not valid: {0}")]
    BadAnswer(String),
    /// The worker reported an error (bad or truncated file, missing file, ...).
    #[error("OpenVDB: {0}")]
    Reported(String),
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
}

impl Worker {
    /// A worker at an explicit path.
    #[must_use]
    pub fn at(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into(), timeout: DEFAULT_TIMEOUT }
    }

    /// Finds the worker: `CFA4_VDB_WORKER`, else the installed path. [`Error::Unavailable`] if neither exists.
    ///
    /// # Errors
    /// [`Error::Unavailable`] when no worker executable is found.
    pub fn discover() -> Result<Self, Error> {
        if let Some(p) = std::env::var_os(WORKER_ENV) {
            let p = PathBuf::from(p);
            return if p.is_file() { Ok(Self::at(p)) } else { Err(Error::Unavailable) };
        }
        let p = PathBuf::from(INSTALLED_WORKER);
        if p.is_file() { Ok(Self::at(p)) } else { Err(Error::Unavailable) }
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
        let out = self.run(&["inspect".as_ref(), file.as_os_str()])?;
        serde_json::from_str(&out).map_err(|e| Error::BadAnswer(e.to_string()))
    }

    /// Reads `input` and writes it to `output` (round trip, N1-4).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn copy(&self, input: &Path, output: &Path) -> Result<(), Error> {
        let out = self.run(&["copy".as_ref(), input.as_os_str(), output.as_os_str()])?;
        match serde_json::from_str::<Ack>(&out) {
            Ok(a) if a.ok => Ok(()),
            Ok(_) => Err(Error::BadAnswer("ok is false".into())),
            Err(e) => Err(Error::BadAnswer(e.to_string())),
        }
    }

    /// Writes a small synthetic `.vdb` (tests; the repository holds no binary fixture).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn write_selftest(&self, output: &Path) -> Result<(), Error> {
        let out = self.run(&["selftest-write".as_ref(), output.as_os_str()])?;
        match serde_json::from_str::<Ack>(&out) {
            Ok(a) if a.ok => Ok(()),
            _ => Err(Error::BadAnswer(out)),
        }
    }

    fn run(&self, args: &[&std::ffi::OsStr]) -> Result<String, Error> {
        if !self.exe.is_file() {
            return Err(Error::Unavailable);
        }
        let mut child = Command::new(&self.exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::Spawn(e.to_string()))?;
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
        let text = String::from_utf8_lossy(&bytes).into_owned();
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
