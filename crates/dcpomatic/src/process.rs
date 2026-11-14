//! Running one DCP-o-matic tool: an argument array (no shell), its own process group, a cleared
//! environment with a private `HOME` / `TMPDIR` / working directory, output to files (no pipe can
//! fill up), a deadline, cancellation and the console progress.

use std::ffi::OsString;
use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustix::process::{Pid, Signal, kill_process_group};

use crate::{Control, DcpomaticError, Phase};

/// How often the child is polled.
const POLL: Duration = Duration::from_millis(50);
/// How often the progress in stdout is read.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
/// How long a tool gets to stop after SIGINT before the group is killed.
const GRACE: Duration = Duration::from_secs(5);
/// How much of the end of stdout is read for the progress.
const TAIL: u64 = 4096;

/// A private directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Scratch> {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cfa4-dcpomatic-{}-{nanos}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let s = Scratch(dir);
        for sub in ["home", "tmp"] {
            fs::DirBuilder::new().mode(0o700).create(s.0.join(sub))?;
        }
        Ok(s)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Only `PATH`, `LD_LIBRARY_PATH` (a DCP-o-matic built into a prefix) and the locale come from
/// the caller; `HOME` and `TMPDIR` are private.
fn command(program: &Path, scratch: &Scratch) -> Command {
    let mut c = Command::new(program);
    c.env_clear();
    c.env("PATH", std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/local/bin:/usr/bin:/bin")));
    for k in ["LANG", "LC_ALL", "LD_LIBRARY_PATH"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
        c.env("LANG", "C.UTF-8");
    }
    c.env("HOME", scratch.0.join("home")).env("TMPDIR", scratch.0.join("tmp")).env("TERM", "dumb");
    c.current_dir(&scratch.0).stdin(Stdio::null()).process_group(0);
    c
}

/// Starts `c`; `ETXTBSY` (the program was just written and another thread's fork still holds
/// the write descriptor until its exec) is retried briefly.
fn spawn(mut c: Command, program: &Path) -> Result<Child, DcpomaticError> {
    let mut tries = 0;
    loop {
        match c.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 50 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(DcpomaticError::Unavailable(format!("{} cannot be started: {e}", program.display()))),
        }
    }
}

fn signal_group(child: &Child, sig: Signal) {
    if let Some(pid) = Pid::from_raw(child.id().cast_signed()) {
        let _ = kill_process_group(pid, sig);
    }
}

enum Outcome {
    /// The exit code, `None` when a signal ended it.
    Exited(Option<i32>),
    Cancelled,
    TimedOut,
}

/// Waits for `child` until it exits, `control` is cancelled (SIGINT to the group, SIGKILL after
/// [`GRACE`]) or `deadline` passes (SIGKILL). `tick` runs every [`PROGRESS_EVERY`].
fn wait(child: &mut Child, deadline: Instant, control: Option<&Control>, mut tick: impl FnMut()) -> std::io::Result<Outcome> {
    let mut next_tick = Instant::now() + PROGRESS_EVERY;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Outcome::Exited(status.code()));
        }
        if control.is_some_and(Control::is_cancelled) {
            signal_group(child, Signal::INT);
            let grace = Instant::now() + GRACE;
            while Instant::now() < grace {
                if child.try_wait()?.is_some() {
                    signal_group(child, Signal::KILL);
                    return Ok(Outcome::Cancelled);
                }
                std::thread::sleep(POLL);
            }
            signal_group(child, Signal::KILL);
            let _ = child.wait();
            return Ok(Outcome::Cancelled);
        }
        if Instant::now() >= deadline {
            signal_group(child, Signal::KILL);
            let _ = child.wait();
            return Ok(Outcome::TimedOut);
        }
        if Instant::now() >= next_tick {
            tick();
            next_tick = Instant::now() + PROGRESS_EVERY;
        }
        std::thread::sleep(POLL);
    }
}

/// The end of a file, as text (escape sequences and all).
fn tail(path: &Path) -> String {
    let Ok(mut f) = fs::File::open(path) else { return String::new() };
    let len = f.metadata().map_or(0, |m| m.len());
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL)));
    let mut b = Vec::new();
    let _ = f.take(TAIL).read_to_end(&mut b);
    String::from_utf8_lossy(&b).into_owned()
}

/// The last `NN%` DCP-o-matic printed (its job status: `Transcoding: 45%; …`), as 0..=1.
pub(crate) fn last_percent(text: &str) -> Option<f32> {
    let b = text.as_bytes();
    let mut i = b.len();
    while i > 0 {
        i -= 1;
        if b[i] != b'%' {
            continue;
        }
        let end = i;
        let mut start = end;
        while start > 0 && b[start - 1].is_ascii_digit() && end - start < 3 {
            start -= 1;
        }
        if start < end
            && let Ok(n) = text[start..end].parse::<u32>()
            && n <= 100
        {
            return Some(n as f32 / 100.0);
        }
    }
    None
}

/// The last line that says something: without escape sequences, `Running` filler or blanks.
fn last_message(path: &Path) -> String {
    let text = tail(path);
    let clean: String = strip_ansi(&text);
    clean.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").chars().take(500).collect()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// `program --version`, its stdout.
pub(crate) fn run_version(program: &Path, timeout: Duration) -> Result<String, DcpomaticError> {
    let scratch = Scratch::new()?;
    let out = scratch.0.join("stdout");
    let mut c = command(program, &scratch);
    c.arg("--version").stdout(fs::File::create(&out)?).stderr(Stdio::null());
    let mut child = spawn(c, program)?;
    match wait(&mut child, Instant::now() + timeout, None, || {})? {
        Outcome::Exited(Some(0)) => Ok(fs::read_to_string(&out).unwrap_or_default()),
        Outcome::Exited(code) => Err(DcpomaticError::Unavailable(format!("{} --version exited with {code:?}", program.display()))),
        Outcome::Cancelled | Outcome::TimedOut => Err(DcpomaticError::Unavailable(format!("{} --version did not finish", program.display()))),
    }
}

/// One tool run to its end.
pub(crate) fn run_step(program: &Path, args: &[String], deadline: Instant, timeout: Duration, control: &Control, phase: Phase) -> Result<(), DcpomaticError> {
    let scratch = Scratch::new()?;
    let (out, err) = (scratch.0.join("stdout"), scratch.0.join("stderr"));
    let mut c = command(program, &scratch);
    c.args(args).stdout(fs::File::create(&out)?).stderr(fs::File::create(&err)?);
    let mut child = spawn(c, program)?;
    let mut last = None;
    let outcome = wait(&mut child, deadline, Some(control), || {
        let p = last_percent(&tail(&out));
        if p.is_some() && p != last {
            last = p;
            control.report(phase, p.unwrap_or(0.0));
        }
    })?;
    let name = program.file_name().map_or_else(|| program.display().to_string(), |n| n.to_string_lossy().into_owned());
    let why = || {
        let e = last_message(&err);
        if e.is_empty() { last_message(&out) } else { e }
    };
    match outcome {
        Outcome::Exited(Some(0)) => Ok(()),
        Outcome::Exited(Some(code)) => Err(DcpomaticError::Engine(format!("{name} exited with {code}: {}", why()))),
        Outcome::Exited(None) => Err(DcpomaticError::Engine(format!("{name} was killed by a signal: {}", why()))),
        Outcome::Cancelled => Err(DcpomaticError::Cancelled),
        Outcome::TimedOut => Err(DcpomaticError::TimedOut(timeout)),
    }
}
