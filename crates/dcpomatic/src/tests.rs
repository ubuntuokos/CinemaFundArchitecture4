//! The adapter against fake DCP-o-matic tools: shell scripts that record their arguments and
//! environment, write a film directory and a DCP folder, print progress the way
//! `dcpomatic2_cli` does, and fail, hang or stop on demand.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcpomatic-test-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("master.mov"), b"not really a movie").unwrap();
    d.canonicalize().unwrap()
}

fn script(p: &Path, body: &str) {
    std::fs::write(p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Fake tools in `d`. `create_tail` and `cli_tail` run after each tool's normal work.
fn tools(d: &Path, version: &str, create_tail: &str, cli_tail: &str) -> Dcpomatic {
    let dd = d.display();
    script(
        &d.join("dcpomatic2_create"),
        &format!(
            r#"printf '%s\n' "$@" > '{dd}/create.args'
printf 'HOME=%s\nTMPDIR=%s\nPWD=%s\n' "$HOME" "$TMPDIR" "$PWD" > '{dd}/env'
env | cut -d= -f1 | sort > '{dd}/envnames'
out=""; prev=""
for a in "$@"; do if [ "$prev" = "--output" ]; then out="$a"; fi; prev="$a"; done
mkdir -p "$out" && echo '<Metadata/>' > "$out/metadata.xml"
{create_tail}"#
        ),
    );
    script(
        &d.join("dcpomatic2_cli"),
        &format!(
            r#"if [ "$1" = "--version" ]; then echo "dcpomatic version {version} 0123abcd"; exit 0; fi
printf '%s\n' "$@" > '{dd}/cli.args'
for a in "$@"; do film="$a"; done
printf 'Making DCP for test\n'
printf 'Examining content: Running\n'
printf '\033[1A\033[2KTranscoding; 1 of 1: 42%%; 20.0 fps\n'
mkdir -p "$film/Test_TST-1_F_51_2K_20261113_SMPTE_OV" && touch "$film/Test_TST-1_F_51_2K_20261113_SMPTE_OV/ASSETMAP.xml"
{cli_tail}"#
        ),
    );
    Dcpomatic::new(d.join("dcpomatic2_create"), d.join("dcpomatic2_cli"))
}

fn request(d: &Path) -> DcpRequest {
    let mut r = DcpRequest::new(vec![Content::new(d.join("master.mov"))], d.join("film"), "Test");
    r.config_dir = Some(d.join("config"));
    r
}

fn args(d: &Path, which: &str) -> Vec<String> {
    std::fs::read_to_string(d.join(format!("{which}.args"))).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn version_is_read_and_checked() {
    assert_eq!(Version::parse("dcpomatic version 2.18.20 0123abcd\n"), Some(Version { major: 2, minor: 18, patch: 20 }));
    assert_eq!(Version::parse("dcpomatic version 2.19.2devel abc"), Some(Version { major: 2, minor: 19, patch: 2 }));
    assert_eq!(Version::parse("dcp_inspect v1"), None);
    let d = dir("old");
    let e = tools(&d, "2.16.99", "", "").version().unwrap_err();
    assert!(matches!(e, DcpomaticError::Unavailable(ref m) if m.contains("older than 2.18.20")), "{e}");
    assert_eq!(tools(&d, "2.18.40", "", "").version().unwrap(), Version { major: 2, minor: 18, patch: 40 });
}

#[test]
fn makes_a_dcp_with_the_requested_settings() {
    let d = dir("make");
    let mut r = request(&d);
    r.content_kind = ContentKind::Feature;
    r.standard = Standard::Interop;
    r.resolution = Some(Resolution::FourK);
    r.container = Some(Container::Scope);
    r.frame_rate = Some(24);
    r.audio_channels = Some(8);
    r.video_bit_rate_mbps = Some(150);
    r.threads = Some(6);
    r.content.push(Content { path: d.join("master.mov"), eye: None, channel: Some(Channel::Hi), gain_db: Some(-3.0) });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let control = Control::new().with_progress(move |ph, f| s.lock().unwrap().push((ph, f)));
    let out = tools(&d, "2.18.40", "", "sleep 1").make_dcp(&r, &control).unwrap();

    assert_eq!(out.film_dir, d.join("film"));
    assert_eq!(out.dcp, d.join("film/Test_TST-1_F_51_2K_20261113_SMPTE_OV"));
    assert_eq!(out.version.to_string(), "2.18.40");
    let m = d.join("master.mov").display().to_string();
    let cfg = d.join("config").display().to_string();
    let film = d.join("film").display().to_string();
    #[rustfmt::skip]
    let expect: Vec<String> = [
        "--config", &cfg, "--output", &film, "--name", "Test", "--no-encrypt", "--dcp-content-type", "FTR", "--standard", "interop", "--fourk",
        "--container-ratio", "239", "--dcp-frame-rate", "24", "--audio-channels", "8", "--video-bit-rate", "150", "--twod", &m, "--channel", "HI",
        "--gain", "-3", &m,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(args(&d, "create"), expect);
    assert_eq!(args(&d, "cli"), ["--config", &cfg, "--no-remote", "--threads", "6", "make-dcp", &film]);
    assert!(d.join("config").is_dir(), "the configuration directory is created");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.first(), Some(&(Phase::Create, 0.0)));
    assert!(seen.contains(&(Phase::Encode, 0.42)), "console progress is passed on: {seen:?}");
    assert_eq!(seen.last(), Some(&(Phase::Encode, 1.0)));
}

#[test]
fn runs_in_a_private_cleared_environment() {
    let d = dir("env");
    tools(&d, "2.18.40", "", "").make_dcp(&request(&d), &Control::new()).unwrap();
    let env = std::fs::read_to_string(d.join("env")).unwrap();
    let home = env.lines().find_map(|l| l.strip_prefix("HOME=")).unwrap();
    assert!(home.contains("cfa4-dcpomatic-") && !Path::new(home).exists(), "private HOME, removed afterwards: {home}");
    let names = std::fs::read_to_string(d.join("envnames")).unwrap();
    for n in names.lines() {
        assert!(["HOME", "TMPDIR", "PATH", "LANG", "LC_ALL", "LD_LIBRARY_PATH", "TERM", "PWD", "SHLVL", "OLDPWD", "_"].contains(&n), "unexpected variable {n}");
    }
}

#[test]
fn bad_requests_are_refused_before_anything_runs() {
    let d = dir("invalid");
    let t = tools(&d, "2.18.40", "", "");
    let check = |r: DcpRequest, want: &str| {
        let e = t.make_dcp(&r, &Control::new()).unwrap_err();
        assert!(matches!(e, DcpomaticError::Invalid(ref m) if m.contains(want)), "{want}: {e}");
    };
    let mut r = request(&d);
    r.content.clear();
    check(r, "no content");
    let mut r = request(&d);
    r.content[0].path = d.join("missing.mov");
    check(r, "does not exist");
    let mut r = request(&d);
    r.name = "a/b".into();
    check(r, "slash");
    let mut r = request(&d);
    r.frame_rate = Some(23);
    check(r, "not a DCP frame rate");
    let mut r = request(&d);
    r.audio_channels = Some(7);
    check(r, "even");
    let mut r = request(&d);
    r.audio_channels = Some(6);
    r.content[0].channel = Some(Channel::BsR);
    check(r, "needs 12");
    let mut r = request(&d);
    r.content[0].eye = Some(Eye::Left);
    check(r, "not 3D");
    let mut r = request(&d);
    r.video_bit_rate_mbps = Some(5);
    check(r, "at least 10");
    std::fs::create_dir(d.join("film")).unwrap();
    check(request(&d), "exists already");
    assert!(!d.join("create.args").exists(), "nothing ran");
}

#[test]
fn engine_errors_carry_the_last_message_and_keep_the_film() {
    let d = dir("fail");
    let e = tools(&d, "2.18.40", "", "echo 'Error during encoding: content file has changed' >&2; exit 1").make_dcp(&request(&d), &Control::new()).unwrap_err();
    assert!(matches!(e, DcpomaticError::Engine(ref m) if m == "dcpomatic2_cli exited with 1: Error during encoding: content file has changed"), "{e}");
    assert!(d.join("film/metadata.xml").is_file(), "the film directory (with its log) is kept");

    let d = dir("nodcp");
    let t = tools(&d, "2.18.40", "", "");
    script(&d.join("dcpomatic2_create"), "exit 0");
    let e = t.make_dcp(&request(&d), &Control::new()).unwrap_err();
    assert!(matches!(e, DcpomaticError::Engine(ref m) if m.contains("wrote no film directory")), "{e}");
}

#[test]
fn missing_tools_are_unavailable() {
    let d = dir("missing");
    let t = Dcpomatic::new(d.join("nope-create"), d.join("nope-cli"));
    assert!(matches!(t.make_dcp(&request(&d), &Control::new()), Err(DcpomaticError::Unavailable(_))));
}

#[test]
fn cancel_stops_the_group_and_removes_the_partial_film() {
    let d = dir("cancel");
    // a background child that would outlive a plain kill of the tool
    let t = tools(&d, "2.18.40", "", &format!("(sleep 30; touch '{}/survivor') & sleep 30", d.display()));
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    let started = Instant::now();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        f.store(true, Ordering::Relaxed);
    });
    let flag2 = flag.clone();
    let e = t.make_dcp(&request(&d), &Control::new().with_cancel(move || flag2.load(Ordering::Relaxed))).unwrap_err();
    canceller.join().unwrap();
    assert!(matches!(e, DcpomaticError::Cancelled), "{e}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!d.join("film").exists(), "the partial film directory is removed");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!d.join("survivor").exists());
}

#[test]
fn timeout_kills_the_run() {
    let d = dir("timeout");
    let t = tools(&d, "2.18.40", "sleep 30", "").with_timeout(Duration::from_millis(700));
    let started = Instant::now();
    let e = t.make_dcp(&request(&d), &Control::new()).unwrap_err();
    assert!(matches!(e, DcpomaticError::TimedOut(_)), "{e}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!d.join("film").exists());
}

#[test]
fn progress_parsing() {
    assert_eq!(process::last_percent("Transcoding: 5%\n\x1b[1A\x1b[2KTranscoding: 73%; 12 fps\n"), Some(0.73));
    assert_eq!(process::last_percent("Examining content: Running\n"), None);
    assert_eq!(process::last_percent("100%"), Some(1.0));
    assert_eq!(process::last_percent("1234%"), None, "more than 100 is not a percentage");
}

#[test]
fn request_round_trips_through_json() {
    let d = dir("json");
    let mut r = request(&d);
    r.resolution = Some(Resolution::TwoK);
    r.content[0].channel = Some(Channel::Vi);
    let text = serde_json::to_string(&r).unwrap();
    assert!(text.contains(r#""resolution":"2k""#) && text.contains(r#""channel":"VI""#), "{text}");
    let back: DcpRequest = serde_json::from_str(&text).unwrap();
    assert_eq!(back, r);
    let min: DcpRequest = serde_json::from_str(r#"{"content":[{"path":"/x.mov"}],"filmDir":"/tmp/f","name":"N"}"#).unwrap();
    assert!(min.isdcf_name && min.standard == Standard::Smpte && min.content_kind == ContentKind::Test);
}
