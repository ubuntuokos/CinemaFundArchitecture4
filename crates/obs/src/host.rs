//! The supervisor of the `cfa4-studio-host` process (plan D2, V.1).
//!
//! libobs is C code; a fault in it must not take the editor with it (AGENTS.md §0). So it runs
//! in its own process, `cfa4-studio-host`, which the engine starts through [`Supervisor`]:
//!
//! - the supervisor creates the frame ring ([`crate::ring`]) and passes its path to the host, so
//!   the ring (and the newest frame in it) outlives a host crash;
//! - a monitor thread waits for the host; when it ends without being asked to, the crash is
//!   recorded and the host restarted after a short back-off (100 ms, doubling, at most 5 s);
//! - more than [`HostOptions::max_restarts`] crashes within a minute stop the restarts
//!   (`HostState::Failed`, with the reason), so a host that cannot run does not spin;
//! - [`Supervisor::stop`] (and drop) ask the host to quit over its stdin, wait briefly, then kill
//!   it, and remove the ring.
//!
//! The host talks JSON lines: the supervisor writes commands to its stdin (`{"cmd":"quit"}`,
//! `{"cmd":"scene",…}`), the host writes events to its stdout ([`HostEvent`]). The supervisor
//! keeps the last scene it was given ([`Supervisor::set_scene`]) and sends it again to every
//! restarted host, so a crash costs the program a few frames, not its scene. The monitor and reader threads run under
//! `catch_unwind`; nothing here panics.

use crate::ring::{self, RingReader};
use crate::scene::HostScene;
use crate::{BackendChoice, Result, StudioError, VideoSettings};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Name of the host executable.
pub const HOST_EXE: &str = "cfa4-studio-host";

/// An event the host writes to its stdout, one JSON object per line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum HostEvent {
    /// The backend runs and the empty scene is on the program output.
    #[serde(rename_all = "camelCase")]
    Ready {
        /// `libobs` or `software`.
        backend: String,
        /// Backend version.
        version: String,
        /// Why `auto` did not use libobs.
        fallback_reason: Option<String>,
    },
    /// The host could not start or run its backend.
    Error {
        /// What went wrong.
        message: String,
    },
    /// The host shows the scene `revision` (S1).
    #[serde(rename_all = "camelCase")]
    SceneShown {
        /// The revision of the [`HostScene`].
        revision: u64,
        /// Its layers.
        layers: usize,
    },
    /// The host is ending at the supervisor's request.
    Stopped,
}

/// A command the supervisor writes to the host's stdin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
pub enum HostCommand {
    /// End cleanly.
    Quit,
    /// Show this scene on the program (S1). A scene that cannot be loaded is an `error` event
    /// and the shown scene stays.
    Scene {
        /// The scene.
        scene: HostScene,
    },
}

/// State of the supervised host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostState {
    /// Started, waiting for its `ready` event.
    Starting,
    /// Running; frames arrive in the ring.
    Running,
    /// Ended unexpectedly; restarting after the back-off.
    Restarting,
    /// Too many crashes (or the executable cannot start); no more restarts.
    Failed,
    /// Stopped at the caller's request.
    Stopped,
}

/// What the supervisor knows about the host (serialised for `live.studio.host.status`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    /// Current state.
    pub state: HostState,
    /// Process id of the running host.
    pub pid: Option<u32>,
    /// Host starts so far (1 after the first start).
    pub starts: u32,
    /// Unexpected ends so far.
    pub crashes: u32,
    /// How the last host ended (`exit status: 1`, `signal: 9 (SIGKILL)`…).
    pub last_exit: Option<String>,
    /// Backend the running host uses.
    pub backend: Option<String>,
    /// Its version.
    pub version: Option<String>,
    /// Why `auto` fell back to the software backend.
    pub fallback_reason: Option<String>,
    /// The last error the host or the supervisor reported.
    pub error: Option<String>,
    /// The frame ring the host writes.
    pub ring: String,
    /// The video the host produces.
    pub video: VideoSettings,
    /// The revision of the scene the host shows (none: the empty scene).
    #[serde(default)]
    pub scene_revision: Option<u64>,
    /// The layers of that scene.
    #[serde(default)]
    pub scene_layers: usize,
}

/// How to start and supervise the host.
#[derive(Clone, Debug)]
pub struct HostOptions {
    /// The `cfa4-studio-host` executable.
    pub exe: PathBuf,
    /// Output video.
    pub video: VideoSettings,
    /// Backend to ask for.
    pub backend: BackendChoice,
    /// CFA4 libobs build (`CFA4_LIBOBS_DIR` of the host).
    pub libobs_dir: PathBuf,
    /// Directory of the frame ring.
    pub ring_dir: PathBuf,
    /// Crashes tolerated within one minute before giving up.
    pub max_restarts: u32,
    /// Extra host arguments (tests use `--test-abort-after-frames`).
    pub extra_args: Vec<String>,
}

impl HostOptions {
    /// Options with the defaults: host next to the current executable (or `CFA4_STUDIO_HOST`),
    /// `auto` backend, the default libobs directory and ring directory, 5 restarts a minute.
    pub fn new(video: VideoSettings) -> Self {
        Self {
            exe: default_exe(),
            video,
            backend: BackendChoice::Auto,
            libobs_dir: crate::libobs::default_dir(),
            ring_dir: ring::default_dir(),
            max_restarts: 5,
            extra_args: Vec::new(),
        }
    }
}

/// `CFA4_STUDIO_HOST`, else `cfa4-studio-host` next to the current executable (or in its parent
/// directory, for test binaries in `target/*/deps`).
pub fn default_exe() -> PathBuf {
    if let Some(p) = std::env::var_os("CFA4_STUDIO_HOST") {
        return PathBuf::from(p);
    }
    let here = std::env::current_exe().ok();
    let dir = here.as_deref().and_then(Path::parent);
    let candidates = [dir.map(|d| d.join(HOST_EXE)), dir.and_then(Path::parent).map(|d| d.join(HOST_EXE))];
    candidates.iter().flatten().find(|p| p.is_file()).cloned().unwrap_or_else(|| PathBuf::from(HOST_EXE))
}

struct Shared {
    status: Mutex<HostStatus>,
    /// The scene every (re)started host is given.
    scene: Mutex<Option<HostScene>>,
    /// The running host's stdin (commands go there).
    stdin: Mutex<Option<ChildStdin>>,
    stop: AtomicBool,
    wake: Condvar,
    wake_lock: Mutex<()>,
}

impl Shared {
    fn update(&self, f: impl FnOnce(&mut HostStatus)) {
        let mut s = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut s);
    }

    /// Sleeps up to `d`, waking early when a stop is requested. Returns whether to stop.
    fn sleep(&self, d: Duration) -> bool {
        let guard = self.wake_lock.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = self.wake.wait_timeout_while(guard, d, |_| !self.stop.load(Ordering::SeqCst)).unwrap_or_else(PoisonError::into_inner);
        self.stop.load(Ordering::SeqCst)
    }
}

/// The supervised host process and its frame ring.
pub struct Supervisor {
    shared: Arc<Shared>,
    monitor: Option<JoinHandle<()>>,
    ring_path: PathBuf,
    reader: RingReader,
}

impl Supervisor {
    /// Creates the frame ring and starts the host under supervision.
    pub fn start(opts: HostOptions) -> Result<Self> {
        opts.video.validate()?;
        if !opts.exe.is_file() {
            return Err(StudioError::Host(format!(
                "{} not found (build it with `cargo build -p cfa4-studio-host`, or set CFA4_STUDIO_HOST)",
                opts.exe.display()
            )));
        }
        let ring_path = ring::create(&opts.ring_dir, &opts.video)?;
        let reader = match RingReader::open(&ring_path) {
            Ok(r) => r,
            Err(e) => {
                ring::remove(&ring_path);
                return Err(e);
            }
        };
        let shared = Arc::new(Shared {
            status: Mutex::new(HostStatus {
                state: HostState::Starting,
                pid: None,
                starts: 0,
                crashes: 0,
                last_exit: None,
                backend: None,
                version: None,
                fallback_reason: None,
                error: None,
                ring: ring_path.to_string_lossy().into_owned(),
                video: opts.video,
                scene_revision: None,
                scene_layers: 0,
            }),
            scene: Mutex::new(None),
            stdin: Mutex::new(None),
            stop: AtomicBool::new(false),
            wake: Condvar::new(),
            wake_lock: Mutex::new(()),
        });
        let s2 = shared.clone();
        let rp = ring_path.clone();
        let monitor = std::thread::Builder::new()
            .name("cfa4-studio-supervisor".into())
            .spawn(move || {
                let s3 = s2.clone();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| monitor(&s2, &opts, &rp))).is_err() {
                    s3.update(|st| {
                        st.state = HostState::Failed;
                        st.error = Some("the studio host supervisor failed unexpectedly".into());
                    });
                }
            })
            .map_err(|e| StudioError::Io("starting the studio host supervisor".into(), e))?;
        Ok(Self { shared, monitor: Some(monitor), ring_path, reader })
    }

    /// The current status.
    pub fn status(&self) -> HostStatus {
        self.shared.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The newest frame in the ring (`None`: none yet, or torn and dropped).
    pub fn latest_frame(&self) -> Result<Option<ring::RingFrame>> {
        self.reader.latest()
    }

    /// Shows `scene` on the program: sends it to the running host and keeps it for every host
    /// started after a crash. The host reports the revision it shows in the status
    /// (`sceneRevision`), or an error when the scene cannot be loaded.
    pub fn set_scene(&self, scene: HostScene) {
        *self.shared.scene.lock().unwrap_or_else(PoisonError::into_inner) = Some(scene.clone());
        let mut stdin = self.shared.stdin.lock().unwrap_or_else(PoisonError::into_inner);
        send(&mut stdin, &HostCommand::Scene { scene });
    }

    /// Path of the frame ring.
    pub fn ring_path(&self) -> &Path {
        &self.ring_path
    }

    /// Waits up to `timeout` for the host to be running with at least one frame in the ring.
    pub fn wait_ready(&self, timeout: Duration) -> Result<HostStatus> {
        let until = Instant::now() + timeout;
        loop {
            let st = self.status();
            if st.state == HostState::Running && self.reader.latest_seq()? > 0 {
                return Ok(st);
            }
            if st.state == HostState::Failed {
                return Err(StudioError::Host(st.error.unwrap_or_else(|| "the studio host failed".into())));
            }
            if Instant::now() >= until {
                return Err(StudioError::Host(format!("the studio host did not become ready in {} ms", timeout.as_millis())));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Stops the host (quit request, then kill after 2 s) and removes the ring.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        if let Some(m) = self.monitor.take() {
            let _ = m.join();
        }
        ring::remove(&self.ring_path);
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn describe_exit(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(code) = status.code() {
        return format!("exit status: {code}");
    }
    match status.signal() {
        Some(sig) => {
            let name = match sig {
                6 => " (SIGABRT)",
                9 => " (SIGKILL)",
                11 => " (SIGSEGV)",
                15 => " (SIGTERM)",
                _ => "",
            };
            format!("signal: {sig}{name}")
        }
        None => "ended".into(),
    }
}

fn spawn(opts: &HostOptions, ring: &Path, generation: u32) -> std::io::Result<Child> {
    let v = opts.video;
    let mut cmd = Command::new(&opts.exe);
    cmd.arg("--frames")
        .arg(ring)
        .args(["--width", &v.width.to_string(), "--height", &v.height.to_string()])
        .args(["--fps", &format!("{}/{}", v.fps_num, v.fps_den)])
        .args(["--backend", opts.backend.as_str()])
        .args(["--generation", &generation.to_string()])
        .arg("--libobs-dir")
        .arg(&opts.libobs_dir)
        .args(&opts.extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    cmd.spawn()
}

/// Reads the host's events and records them; ends with the host's stdout.
fn read_events(shared: Arc<Shared>, out: std::process::ChildStdout) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<HostEvent>(&line) {
                Ok(HostEvent::Ready { backend, version, fallback_reason }) => shared.update(|s| {
                    s.state = HostState::Running;
                    s.backend = Some(backend);
                    s.version = Some(version);
                    s.fallback_reason = fallback_reason;
                }),
                Ok(HostEvent::Error { message }) => {
                    log::warn!("cfa4-studio-host: {message}");
                    shared.update(|s| s.error = Some(message));
                }
                Ok(HostEvent::SceneShown { revision, layers }) => shared.update(|s| {
                    s.scene_revision = Some(revision);
                    s.scene_layers = layers;
                    s.error = None;
                }),
                Ok(HostEvent::Stopped) => {}
                Err(_) => log::debug!("cfa4-studio-host (not an event): {line}"),
            }
        }
    }));
}

/// Writes `cmd` to the host's stdin; a host that is gone (broken pipe) is left to the monitor.
fn send(stdin: &mut Option<ChildStdin>, cmd: &HostCommand) {
    if let Some(s) = stdin.as_mut()
        && let Ok(line) = serde_json::to_string(cmd)
        && (writeln!(s, "{line}").is_err() || s.flush().is_err())
    {
        *stdin = None;
    }
}

fn ask_quit(shared: &Shared) {
    // dropping stdin afterwards closes it: EOF is a quit request too
    let mut stdin = shared.stdin.lock().unwrap_or_else(PoisonError::into_inner);
    send(&mut stdin, &HostCommand::Quit);
    *stdin = None;
}

fn monitor(shared: &Arc<Shared>, opts: &HostOptions, ring: &Path) {
    let mut crash_times: Vec<Instant> = Vec::new();
    let mut backoff = Duration::from_millis(100);
    let mut generation: u32 = 0;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            shared.update(|s| s.state = HostState::Stopped);
            return;
        }
        generation = generation.saturating_add(1);
        let mut child = match spawn(opts, ring, generation) {
            Ok(c) => c,
            Err(e) => {
                shared.update(|s| {
                    s.state = HostState::Failed;
                    s.error = Some(format!("cannot start {}: {e}", opts.exe.display()));
                });
                return;
            }
        };
        let started = Instant::now();
        shared.update(|s| {
            s.state = HostState::Starting;
            s.pid = Some(child.id());
            s.starts = generation;
            s.backend = None;
            s.version = None;
            s.scene_revision = None;
            s.scene_layers = 0;
        });
        {
            // the new host gets the scene at once (it reads it after starting its backend)
            let mut stdin = shared.stdin.lock().unwrap_or_else(PoisonError::into_inner);
            *stdin = child.stdin.take();
            if let Some(scene) = shared.scene.lock().unwrap_or_else(PoisonError::into_inner).clone() {
                send(&mut stdin, &HostCommand::Scene { scene });
            }
        }
        let reader = child.stdout.take().and_then(|out| {
            let s2 = shared.clone();
            std::thread::Builder::new().name("cfa4-studio-host-events".into()).spawn(move || read_events(s2, out)).ok()
        });
        // wait for the host to end or for a stop request
        let exit = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(e) => {
                    shared.update(|s| s.error = Some(format!("cannot watch the studio host: {e}")));
                    break None;
                }
            }
            if shared.sleep(Duration::from_millis(50)) {
                break None;
            }
        };
        let Some(status) = exit else {
            // stop requested (or the host cannot be watched): quit, then kill
            ask_quit(shared);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut ended = false;
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    ended = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if !ended {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(r) = reader {
                let _ = r.join();
            }
            shared.update(|s| {
                s.state = HostState::Stopped;
                s.pid = None;
            });
            return;
        };
        *shared.stdin.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if let Some(r) = reader {
            let _ = r.join();
        }
        // ended without being asked: a crash
        let how = describe_exit(status);
        log::warn!("cfa4-studio-host ended unexpectedly ({how}); restarting");
        let now = Instant::now();
        crash_times.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        crash_times.push(now);
        if started.elapsed() > Duration::from_secs(10) {
            backoff = Duration::from_millis(100);
        }
        let give_up = crash_times.len() > opts.max_restarts as usize;
        shared.update(|s| {
            s.crashes = s.crashes.saturating_add(1);
            s.last_exit = Some(how.clone());
            s.pid = None;
            if give_up {
                s.state = HostState::Failed;
                s.error = Some(format!("the studio host ended {} times within a minute (last: {how}); not restarting", crash_times.len()));
            } else {
                s.state = HostState::Restarting;
            }
        });
        if give_up {
            return;
        }
        if shared.sleep(backoff) {
            shared.update(|s| s.state = HostState::Stopped);
            return;
        }
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_and_commands_have_a_stable_wire_form() {
        let e = HostEvent::Ready { backend: "libobs".into(), version: "33.0.1".into(), fallback_reason: None };
        let s = serde_json::to_string(&e).expect("json");
        assert_eq!(s, r#"{"event":"ready","backend":"libobs","version":"33.0.1","fallbackReason":null}"#);
        assert_eq!(serde_json::from_str::<HostEvent>(&s).ok(), Some(e));
        assert_eq!(serde_json::to_string(&HostCommand::Quit).ok().as_deref(), Some(r#"{"cmd":"quit"}"#));
        let shown = HostEvent::SceneShown { revision: 3, layers: 2 };
        assert_eq!(serde_json::to_string(&shown).ok().as_deref(), Some(r#"{"event":"sceneShown","revision":3,"layers":2}"#));
        let cmd = HostCommand::Scene { scene: HostScene { revision: 4, layers: vec![] } };
        let line = serde_json::to_string(&cmd).expect("json");
        assert_eq!(line, r#"{"cmd":"scene","scene":{"revision":4,"layers":[]}}"#);
        assert_eq!(serde_json::from_str::<HostCommand>(&line).ok(), Some(cmd));
        assert!(serde_json::from_str::<HostEvent>(r#"{"event":"boom"}"#).is_err());
    }

    #[test]
    fn a_missing_executable_is_an_error() {
        let mut o = HostOptions::new(VideoSettings { width: 64, height: 36, ..Default::default() });
        o.exe = PathBuf::from("/nonexistent/cfa4-studio-host");
        assert!(matches!(Supervisor::start(o), Err(StudioError::Host(_))));
    }

    #[test]
    fn exits_are_described() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(describe_exit(std::process::ExitStatus::from_raw(9)), "signal: 9 (SIGKILL)");
        assert_eq!(describe_exit(std::process::ExitStatus::from_raw(1 << 8)), "exit status: 1");
    }
}
