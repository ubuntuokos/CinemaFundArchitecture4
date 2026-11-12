//! Running the engine: an argument array (no shell), its own process group, a private working
//! directory, output to files (no pipe can fill up), a timeout and cancellation.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cfa4_dcp_validate::{ValidationControl, ValidationError};
use rustix::process::{Pid, Signal, kill_process_group};

/// The largest result file read (the result of a long feature is a few MB).
const MAX_RESULT: u64 = 256 * 1024 * 1024;
/// How often the child is polled.
const POLL: Duration = Duration::from_millis(50);
/// How long the engine gets to stop after SIGINT before the group is killed.
const GRACE: Duration = Duration::from_secs(5);

pub(crate) enum Outcome {
    /// The exit code, `None` when a signal ended it.
    Exited(Option<i32>),
    Cancelled,
    TimedOut,
}

pub(crate) struct Run {
    pub outcome: Outcome,
    /// The `--dump-result` file, when the engine wrote one.
    pub result: Option<String>,
    /// The last non-empty line of stderr (or stdout), for error messages.
    pub last_error: String,
}

/// A private directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Scratch> {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cfa4-dcp-inspect-{}-{nanos}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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

/// The engine's environment: only `PATH` and the locale come from the caller; `HOME` and `TMPDIR`
/// are private, and `TERM` is set because the engine's progress bar needs a terminal size.
fn command(program: &Path, scratch: &Scratch) -> Command {
    let mut c = Command::new(program);
    c.env_clear();
    c.env("PATH", std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/local/bin:/usr/bin:/bin")));
    for k in ["LANG", "LC_ALL"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
        c.env("LANG", "C.UTF-8");
    }
    c.env("HOME", scratch.0.join("home")).env("TMPDIR", scratch.0.join("tmp")).env("TERM", "dumb").env("COLUMNS", "200");
    c.current_dir(&scratch.0).stdin(Stdio::null()).process_group(0);
    c
}

fn spawn(mut c: Command, program: &Path) -> Result<Child, ValidationError> {
    c.spawn().map_err(|e| ValidationError::Unavailable(format!("{} cannot be started: {e}", program.display())))
}

fn signal_group(child: &Child, sig: Signal) {
    if let Some(pid) = Pid::from_raw(child.id().cast_signed()) {
        let _ = kill_process_group(pid, sig);
    }
}

/// Waits for `child` until it exits, `control` is cancelled (SIGINT to the group, SIGKILL after
/// [`GRACE`]) or `deadline` passes (SIGKILL to the group).
fn wait(child: &mut Child, deadline: Instant, control: Option<&ValidationControl>) -> std::io::Result<Outcome> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Outcome::Exited(status.code()));
        }
        if control.is_some_and(ValidationControl::is_cancelled) {
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
        std::thread::sleep(POLL);
    }
}

fn last_line(path: &Path) -> String {
    let text = fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").chars().take(500).collect()
}

/// `program --version`, its stdout.
pub(crate) fn run_version(program: &Path, timeout: Duration) -> Result<String, ValidationError> {
    let scratch = Scratch::new()?;
    let out = scratch.0.join("stdout");
    let mut c = command(program, &scratch);
    c.arg("--version").stdout(fs::File::create(&out)?).stderr(Stdio::null());
    let mut child = spawn(c, program)?;
    match wait(&mut child, Instant::now() + timeout, None)? {
        Outcome::Exited(Some(0)) => Ok(fs::read_to_string(&out).unwrap_or_default()),
        Outcome::Exited(code) => Err(ValidationError::Unavailable(format!("{} --version exited with {code:?}", program.display()))),
        Outcome::Cancelled | Outcome::TimedOut => Err(ValidationError::Unavailable(format!("{} --version did not finish", program.display()))),
    }
}

/// One inspection of `root`.
pub(crate) fn run_engine(program: &Path, args: &[String], root: &Path, timeout: Duration, control: &ValidationControl) -> Result<Run, ValidationError> {
    let scratch = Scratch::new()?;
    if scratch.0.starts_with(root) {
        return Err(ValidationError::Backend("the engine's working directory would be inside the package".into()));
    }
    let result = scratch.0.join("result.json");
    let (out, err) = (scratch.0.join("stdout"), scratch.0.join("stderr"));
    let mut c = command(program, &scratch);
    c.args(args).arg("--dump-result").arg(&result).arg(root);
    c.stdout(fs::File::create(&out)?).stderr(fs::File::create(&err)?);
    let mut child = spawn(c, program)?;
    let outcome = wait(&mut child, Instant::now() + timeout, Some(control))?;
    let text = match fs::metadata(&result) {
        Ok(m) if m.len() > MAX_RESULT => return Err(ValidationError::Backend(format!("dcp_inspect result is over {MAX_RESULT} bytes"))),
        Ok(_) => fs::read_to_string(&result).ok(),
        Err(_) => None,
    };
    let mut last_error = last_line(&err);
    if last_error.is_empty() {
        last_error = last_line(&out);
    }
    Ok(Run { outcome, result: text, last_error })
}
