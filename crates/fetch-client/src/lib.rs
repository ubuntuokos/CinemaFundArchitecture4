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
//! Each file streams into `<dir>/<name>.<tag>.part` (see [`part_name`]; the tag is unique to one
//! download, so a partial file belongs to one downloader run), its size and SHA-256 are checked, and only then is it renamed into place; a file already present with the right size and SHA-256 is not downloaded again.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

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

/// How long the downloader may stay silent (no progress line) before it is stopped: a server that
/// stops sending in the middle of a file would otherwise hold the download forever. A progress line
/// comes every 256 KiB, so this fails only a link slower than about 1 KB/s.
pub const STALL: Duration = Duration::from_secs(300);

/// How long the downloader may keep its output open after its last line (done or error).
const LINGER: Duration = Duration::from_secs(5);

/// Event lines buffered between the reader thread and [`run`]; when full, the reader (and so the
/// downloader writing to the pipe) waits.
const QUEUE: usize = 64;

/// The partial file of `name` written by the download tagged `tag` (`cfa4-fetch --tag`): each run
/// has its own, so two downloads into the same directory never write or remove each other's.
pub fn part_name(name: &str, tag: &str) -> String {
    format!("{name}.{tag}.part")
}

/// A tag [`part_name`] accepts: 1 to 64 ASCII letters, digits and `-`.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 64 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A tag no other download running now has: this process's id (unique among running processes),
/// the time and a counter (unique within this process). Unlike the downloader's process id, it is
/// not reused when the downloader exits, so cleaning up after it cannot touch a newer run's file.
fn unique_tag() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("{:x}-{t:x}-{n:x}", std::process::id())
}

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
    /// The first line naming a file comes right after the downloader created that file's partial
    /// file: a stopped downloader's partial files are removed only when announced this way.
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
        if self.total_size().is_none() {
            return Err("the file sizes add up to more than 2^64 bytes".into());
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

    /// The bytes of all files together, `None` if they overflow (a request that does is invalid).
    pub fn total_size(&self) -> Option<u64> {
        self.files.iter().try_fold(0u64, |t, f| t.checked_add(f.size))
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
    run_with(program, extra_args, req, progress, STALL)
}

fn run_with(program: &Path, extra_args: &[&str], req: &Request, progress: Progress<'_>, stall: Duration) -> Result<(), FetchError> {
    req.validate().map_err(FetchError::Invalid)?;
    let body = serde_json::to_vec(req).map_err(|e| FetchError::Invalid(e.to_string()))?;
    let tag = unique_tag();
    let mut child = Command::new(program)
        .args(extra_args)
        .args(["--tag", tag.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { FetchError::NotFound } else { FetchError::Io(e) })?;
    // the request is written on its own thread: a downloader that does not read it would otherwise
    // block this call before the stall watchdog runs. The thread ends when the request is written or
    // the pipe closes (a closed pipe shows in the downloader's exit); it is not waited for.
    if let Some(mut stdin) = child.stdin.take() {
        let writer = std::thread::Builder::new().name("cfa4-fetch-request".into()).spawn(move || {
            let _ = stdin.write_all(&body);
        });
        if let Err(e) = writer {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FetchError::Io(e));
        }
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(FetchError::Failed("no output from the downloader".into()));
    };
    // lines are read on their own thread so a stalled download can still be cancelled
    let (tx, rx) = mpsc::sync_channel::<Result<Event, String>>(QUEUE);
    let spawned = std::thread::Builder::new().name("cfa4-fetch-events".into()).spawn(move || {
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
    });
    let reader = match spawned {
        Ok(r) => r,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FetchError::Io(e));
        }
    };
    let mut last = (0u64, req.total_size().unwrap_or(u64::MAX), String::new());
    let mut outcome: Option<Result<(), FetchError>> = None;
    let mut heard = Instant::now();
    // when the last line (done or error) came; the output should close right after it
    let mut ended: Option<Instant> = None;
    // the downloader reported its own failure: it removed its partial files before saying so, and
    // a partial file left under this download's tag then is not its own (it refuses to create one
    // that exists), so the client removes nothing
    let mut reported = false;
    // the files whose partial file the downloader announced (see Event::Progress): the only ones
    // removed after it is stopped
    let mut owned = std::collections::HashSet::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(Event::Progress { done, total, file })) => {
                heard = Instant::now();
                owned.insert(file.clone());
                last = (done, total, file);
            }
            Ok(Ok(Event::Done)) => {
                outcome.get_or_insert(Ok(()));
                ended.get_or_insert_with(Instant::now);
            }
            Ok(Ok(Event::Error { message })) => {
                reported |= outcome.is_none();
                outcome.get_or_insert(Err(FetchError::Failed(message)));
                ended.get_or_insert_with(Instant::now);
                // a failed downloader is stopped now, not when it closes its output
                let _ = child.kill();
            }
            Ok(Err(e)) => {
                outcome.get_or_insert(Err(FetchError::Failed(e)));
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Some(t) = ended {
            if t.elapsed() > LINGER {
                break;
            }
            continue;
        }
        if heard.elapsed() > stall {
            outcome = Some(Err(FetchError::Failed(format!("the downloader received nothing for {} s", stall.as_secs()))));
            break;
        }
        if !progress(last.0, last.1, &last.2) {
            let _ = child.kill();
            let _ = child.wait();
            // announcements still in the pipe count too; the reader thread then ends when the pipe
            // closes (or, waiting on the full queue, when the queue is dropped): not waited for
            drain(&rx, &mut owned);
            drop(rx);
            drop(reader);
            remove_parts(req, &tag, &owned);
            return Err(FetchError::Cancelled);
        }
    }
    // the output ended, or the process is stopped now: after an error, a stall, an unreadable line,
    // or output still open after the last line (killing an exited process does nothing)
    if !matches!(outcome, Some(Ok(()))) || ended.is_some_and(|t| t.elapsed() > LINGER) {
        let _ = child.kill();
    }
    // a downloader that closed its output but does not exit is stopped too
    let status = finish(&mut child, LINGER)?;
    drain(&rx, &mut owned);
    // the reader thread is not waited for: a process the downloader left behind may still hold
    // the pipe open. It ends when the pipe closes, or at once when it waits on the full queue,
    // which is gone now.
    drop(rx);
    drop(reader);
    match outcome {
        Some(Ok(())) if status.success() => Ok(()),
        Some(Err(e)) => {
            if !reported {
                remove_parts(req, &tag, &owned);
            }
            Err(e)
        }
        _ => {
            remove_parts(req, &tag, &owned);
            Err(FetchError::Failed(format!("the downloader stopped without finishing ({status})")))
        }
    }
}

/// Wait at most `grace` for `child` to exit, then stop it.
fn finish(child: &mut std::process::Child, grace: Duration) -> std::io::Result<std::process::ExitStatus> {
    let until = Instant::now() + grace;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= until {
            let _ = child.kill();
            return child.wait();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Collect the files announced in lines still queued from a stopped downloader, waiting at most a
/// second for its output to end.
fn drain(rx: &mpsc::Receiver<Result<Event, String>>, owned: &mut std::collections::HashSet<String>) {
    let until = Instant::now() + Duration::from_secs(1);
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(Ok(Event::Progress { file, .. })) => {
                owned.insert(file);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

/// Remove the partial files the stopped download `tag` left: only those of the files it announced
/// (another run's file under the same tag is never touched).
fn remove_parts(req: &Request, tag: &str, owned: &std::collections::HashSet<String>) {
    for f in &req.files {
        if valid_name(&f.name) && owned.contains(&f.name) {
            let _ = std::fs::remove_file(req.dir.join(part_name(&f.name, tag)));
        }
    }
}

#[cfg(test)]
mod tests;
