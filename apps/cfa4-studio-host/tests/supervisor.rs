//! The S0 acceptance tests of the live studio host (CFA4-LIVE-STUDIO-001 S0): the host starts,
//! the empty scene's frames reach the frame ring, and a crashed host is restarted while the
//! supervising process (in the product: the engine, and the editor above it) carries on.

use cfa4_obs::host::{HostOptions, HostState, Supervisor};
use cfa4_obs::{BackendChoice, VideoSettings};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn options(backend: BackendChoice) -> HostOptions {
    let mut o = HostOptions::new(VideoSettings { width: 64, height: 36, fps_num: 60, fps_den: 1 });
    o.exe = PathBuf::from(env!("CARGO_BIN_EXE_cfa4-studio-host"));
    o.backend = backend;
    o
}

fn kill(pid: u32) {
    let status = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status().expect("kill");
    assert!(status.success());
}

fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists() && std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

fn wait_for(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !f() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_host_delivers_empty_scene_frames_and_is_restarted_after_a_crash() {
    let mut sup = Supervisor::start(options(BackendChoice::Software)).expect("start");
    let st = sup.wait_ready(Duration::from_secs(10)).expect("ready");
    assert_eq!(st.backend.as_deref(), Some("software"));
    let first = sup.latest_frame().expect("read").expect("a frame");
    assert_eq!((first.frame.width, first.frame.height), (64, 36));
    assert!(first.frame.data.chunks_exact(4).all(|p| p == [0, 0, 0, 255]), "the empty scene is opaque black");

    // crash the host the hard way
    let pid = st.pid.expect("pid");
    kill(pid);
    wait_for("the restart", Duration::from_secs(10), || {
        let s = sup.status();
        s.starts == 2 && s.state == HostState::Running && s.pid.is_some_and(|p| p != pid)
    });
    let s = sup.status();
    assert_eq!(s.crashes, 1);
    assert_eq!(s.last_exit.as_deref(), Some("signal: 9 (SIGKILL)"));
    // frames keep coming, and their numbers keep increasing across the restart
    let before = sup.latest_frame().expect("read").map_or(0, |f| f.seq);
    wait_for("new frames", Duration::from_secs(5), || sup.latest_frame().ok().flatten().is_some_and(|f| f.seq > before));
    assert!(before >= first.seq);

    // a clean stop ends the host and removes the ring
    let ring = sup.ring_path().to_path_buf();
    let pid = sup.status().pid.expect("pid");
    sup.stop();
    assert_eq!(sup.status().state, HostState::Stopped);
    wait_for("the host to end", Duration::from_secs(5), || !alive(pid));
    assert!(!ring.exists());
}

#[test]
fn a_host_that_keeps_crashing_is_given_up_and_the_supervisor_carries_on() {
    let mut o = options(BackendChoice::Software);
    o.max_restarts = 2;
    o.extra_args = vec!["--test-abort-after-frames".into(), "3".into()];
    let sup = Supervisor::start(o).expect("start");
    wait_for("giving up", Duration::from_secs(30), || sup.status().state == HostState::Failed);
    let s = sup.status();
    assert_eq!(s.crashes, 3, "two restarts, then the third crash stops them");
    assert_eq!(s.starts, 3);
    assert_eq!(s.last_exit.as_deref(), Some("signal: 6 (SIGABRT)"));
    assert!(s.error.as_deref().is_some_and(|e| e.contains("not restarting")));
    // the frames written before the crashes are still there, and readable
    assert!(sup.latest_frame().expect("read").is_some_and(|f| f.seq >= 9));
    assert!(sup.wait_ready(Duration::from_millis(100)).is_err());
}

#[test]
fn a_host_that_cannot_open_its_backend_reports_why() {
    let mut o = options(BackendChoice::Libobs);
    o.libobs_dir = PathBuf::from("/nonexistent/cfa4-libobs");
    o.max_restarts = 0;
    let sup = Supervisor::start(o).expect("start");
    wait_for("giving up", Duration::from_secs(10), || sup.status().state == HostState::Failed);
    let s = sup.status();
    assert_eq!(s.last_exit.as_deref(), Some("exit status: 1"));
    assert!(s.error.is_some());
}

#[test]
fn auto_falls_back_to_the_software_backend_and_says_why() {
    let mut o = options(BackendChoice::Auto);
    o.libobs_dir = PathBuf::from("/nonexistent/cfa4-libobs");
    let sup = Supervisor::start(o).expect("start");
    let st = sup.wait_ready(Duration::from_secs(10)).expect("ready");
    assert_eq!(st.backend.as_deref(), Some("software"));
    assert!(st.fallback_reason.as_deref().is_some_and(|r| r.contains("not found")));
}

/// With the CFA4 libobs build and a display (X11 or Wayland; CI uses Xvfb), libobs itself runs
/// in the host: its empty scene arrives in the ring, and a killed libobs host comes back.
#[test]
fn libobs_shows_an_empty_scene_and_comes_back_after_a_crash() {
    let dir = cfa4_obs::libobs::default_dir();
    let has_display = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !cfa4_obs::libobs::library_path(&dir).is_file() || !has_display {
        eprintln!("skipped: needs the libobs build in {} and a display", dir.display());
        return;
    }
    let mut o = options(BackendChoice::Libobs);
    o.video = VideoSettings { width: 320, height: 180, fps_num: 30, fps_den: 1 };
    o.libobs_dir = dir;
    let mut sup = Supervisor::start(o).expect("start");
    let st = sup.wait_ready(Duration::from_secs(30)).expect("libobs ready");
    assert_eq!(st.backend.as_deref(), Some("libobs"));
    assert_eq!(st.version.as_deref(), Some(cfa4_obs::LIBOBS_VERSION));
    let f = sup.latest_frame().expect("read").expect("a frame");
    assert_eq!((f.frame.width, f.frame.height), (320, 180));
    assert!(f.frame.data.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]), "the empty scene is black");

    let pid = st.pid.expect("pid");
    kill(pid);
    wait_for("the libobs host to come back", Duration::from_secs(30), || {
        let s = sup.status();
        s.starts == 2 && s.state == HostState::Running
    });
    let before = sup.latest_frame().expect("read").map_or(0, |f| f.seq);
    wait_for("new libobs frames", Duration::from_secs(10), || sup.latest_frame().ok().flatten().is_some_and(|f| f.seq > before));
    sup.stop();
}
