//! Downloads through the `cfa4-fetch` process (CFA4-SECURITY-001, decision D5).
//!
//! The engine never opens a connection to the internet itself: a download the user asked for (an
//! AI model, the natural voices) runs in `cfa4-fetch`, a small process that confines itself with
//! the `fetch` profile of `cfa4-sandbox` before it touches the network (it may write only into the
//! download's target directory, connect out only to TCP port 443 and start no programs). This crate
//! is the engine's side of it, with no network code:
//!
//! - [`Request`] and [`FileSpec`]: what to download, every file pinned to its HTTPS address, size
//!   and SHA-256; [`Request::validate`] is the check both sides make;
//! - [`Event`]: the JSON lines `cfa4-fetch` writes to its standard output (progress, done, error);
//! - [`locate`] finds the program and [`run`] starts it, sends the request on its standard input,
//!   reports progress and stops it when the caller cancels.
//!
//! Each file streams into `<dir>/<name>.part`, its size and SHA-256 are checked, and only then is it
//! renamed into place; a file already present with the right size and SHA-256 is not downloaded again.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The program's file name.
pub const PROGRAM: &str = "cfa4-fetch";

/// Environment variable naming the program explicitly (tests, development builds).
pub const PROGRAM_ENV: &str = "CFA4_FETCH";

/// The largest request `cfa4-fetch` reads (bytes of JSON).
pub const MAX_REQUEST: u64 = 1 << 20;

/// The largest event line the client reads (bytes); a longer one is an error.
const MAX_LINE: u64 = 64 << 10;

/// Files in one request.
const MAX_FILES: usize = 256;

/// One file to download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSpec {
    /// File name inside the target directory (no path separators, not `.`/`..`, no leading dot).
    pub name: String,
    /// Its `https://` address.
    pub url: String,
    /// Expected SHA-256, 64 hex digits.
    pub sha256: String,
    /// Expected size in bytes.
    pub size: u64,
}

/// What `cfa4-fetch` downloads: missing files of `files` into `dir`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The target directory (absolute). It is created if needed and is the only place the
    /// downloader may write.
    pub dir: PathBuf,
    pub files: Vec<FileSpec>,
}

/// One line `cfa4-fetch` writes to its standard output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    /// Bytes done of the bytes still missing when the download started, and the file in progress.
    Progress { done: u64, total: u64, file: String },
    /// Every file is in place.
    Done,
    /// The download failed; nothing half-written is left in place of a file.
    Error { message: String },
}

impl Event {
    /// The event as one line of JSON (without the newline).
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"event":"error","message":"unencodable event"}"#.to_string())
    }
}

/// Errors of [`run`].
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("invalid download request: {0}")]
    Invalid(String),
    #[error("the downloader ({PROGRAM}) was not found next to this program; set {PROGRAM_ENV} to its path")]
    NotFound,
    #[error("download failed: {0}")]
    Failed(String),
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A name that stays inside the target directory.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.ends_with(".part")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Request {
    /// The checks `cfa4-fetch` makes before it downloads anything (the client makes them too, so a
    /// bad request fails without starting a process).
    pub fn validate(&self) -> Result<(), String> {
        if !self.dir.is_absolute() {
            return Err(format!("the target directory is not an absolute path: {}", self.dir.display()));
        }
        if self.dir.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(format!("the target directory contains '..': {}", self.dir.display()));
        }
        if self.files.len() > MAX_FILES {
            return Err(format!("{} files in one request (at most {MAX_FILES})", self.files.len()));
        }
        let mut seen = std::collections::HashSet::new();
        for f in &self.files {
            if !valid_name(&f.name) {
                return Err(format!("invalid file name: {:?}", f.name));
            }
            if !seen.insert(f.name.as_str()) {
                return Err(format!("file named twice: {}", f.name));
            }
            if !f.url.starts_with("https://") || f.url.len() > 2048 || f.url.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(format!("{}: only https:// addresses are downloaded: {:?}", f.name, f.url));
            }
            if f.sha256.len() != 64 || !f.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("{}: the SHA-256 is not 64 hex digits", f.name));
            }
        }
        Ok(())
    }
}

/// Where `cfa4-fetch` is: [`PROGRAM_ENV`] when set, else next to the running program.
pub fn locate() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(PROGRAM_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join(PROGRAM);
    p.is_file().then_some(p)
}

/// Download progress: `(bytes done, bytes total, file)`; return false to cancel.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64, &str) -> bool;

/// Run `program` (normally [`locate`]'s) on `req`: send the request, report progress, and stop the
/// process when `progress` returns false. `extra_args` are passed to the program (the engine passes
/// `--unprotected` only after the user's explicit opt-in, plan D4).
pub fn run(program: &Path, extra_args: &[&str], req: &Request, progress: Progress<'_>) -> Result<(), FetchError> {
    req.validate().map_err(FetchError::Invalid)?;
    let body = serde_json::to_vec(req).map_err(|e| FetchError::Invalid(e.to_string()))?;
    let mut child = Command::new(program)
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { FetchError::NotFound } else { FetchError::Io(e) })?;
    if let Some(mut stdin) = child.stdin.take() {
        // the program reads the whole request before it answers; a closed pipe shows in its exit
        let _ = stdin.write_all(&body);
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(FetchError::Failed("no output from the downloader".into()));
    };
    // lines are read on their own thread so a stalled download can still be cancelled
    let (tx, rx) = mpsc::channel::<Result<Event, String>>();
    let reader = std::thread::Builder::new().name("cfa4-fetch-events".into()).spawn(move || {
        let mut r = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match (&mut r).take(MAX_LINE).read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) if !line.ends_with(b"\n") && line.len() as u64 >= MAX_LINE => {
                    let _ = tx.send(Err("an over-long line from the downloader".into()));
                    break;
                }
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    let text = text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    let ev = serde_json::from_str::<Event>(text).map_err(|e| format!("unreadable line from the downloader: {e}"));
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    })?;
    let mut last = (0u64, req.files.iter().map(|f| f.size).sum::<u64>(), String::new());
    let mut outcome: Option<Result<(), FetchError>> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(Event::Progress { done, total, file })) => last = (done, total, file),
            Ok(Ok(Event::Done)) => {
                outcome.get_or_insert(Ok(()));
                continue;
            }
            Ok(Ok(Event::Error { message })) => {
                outcome.get_or_insert(Err(FetchError::Failed(message)));
                continue;
            }
            Ok(Err(e)) => {
                outcome.get_or_insert(Err(FetchError::Failed(e)));
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if outcome.is_none() && !progress(last.0, last.1, &last.2) {
            let _ = child.kill();
            let _ = child.wait();
            // the reader thread ends when the pipe closes; it is not waited for here
            drop(reader);
            remove_parts(req);
            return Err(FetchError::Cancelled);
        }
    }
    // an error line or the end of the output: the process is done or must be
    if matches!(outcome, Some(Err(_))) {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let _ = reader.join();
    match outcome {
        Some(Ok(())) if status.success() => Ok(()),
        Some(Err(e)) => {
            remove_parts(req);
            Err(e)
        }
        _ => {
            remove_parts(req);
            Err(FetchError::Failed(format!("the downloader stopped without finishing ({status})")))
        }
    }
}

/// Remove the `.part` files a stopped download may have left.
fn remove_parts(req: &Request) {
    for f in &req.files {
        if valid_name(&f.name) {
            let _ = std::fs::remove_file(req.dir.join(format!("{}.part", f.name)));
        }
    }
}

#[cfg(test)]
mod tests;
