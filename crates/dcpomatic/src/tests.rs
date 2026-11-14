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
    let e = tools(&d, "2.18.40", "", "").version().unwrap_err();
    assert!(matches!(e, DcpomaticError::Unavailable(ref m) if m.contains("older than 2.18.51")), "{e}");
    assert_eq!(tools(&d, "2.18.51", "", "").version().unwrap(), Version { major: 2, minor: 18, patch: 51 });
    // the audit's shallow-clone build printed this; the leading number still counts
    assert_eq!(Version::parse("dcpomatic version 2.18.51 fatal: amb"), Some(MIN_VERSION));
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
    let out = tools(&d, "2.18.51", "", "sleep 1").make_dcp(&r, &control).unwrap();

    assert_eq!(out.film_dir, d.join("film"));
    assert_eq!(out.dcp, d.join("film/Test_TST-1_F_51_2K_20261113_SMPTE_OV"));
    assert_eq!(out.version.to_string(), "2.18.51");
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
    tools(&d, "2.18.51", "", "").make_dcp(&request(&d), &Control::new()).unwrap();
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
    let t = tools(&d, "2.18.51", "", "");
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
    let e = tools(&d, "2.18.51", "", "echo 'Error during encoding: content file has changed' >&2; exit 1").make_dcp(&request(&d), &Control::new()).unwrap_err();
    assert!(matches!(e, DcpomaticError::Engine(ref m) if m == "dcpomatic2_cli exited with 1: Error during encoding: content file has changed"), "{e}");
    assert!(d.join("film/metadata.xml").is_file(), "the film directory (with its log) is kept");

    let d = dir("nodcp");
    let t = tools(&d, "2.18.51", "", "");
    script(&d.join("dcpomatic2_create"), "exit 0");
    let e = t.make_dcp(&request(&d), &Control::new()).unwrap_err();
    assert!(matches!(e, DcpomaticError::Engine(ref m) if m.contains("wrote no film directory")), "{e}");
}

#[test]
fn only_the_private_folder_or_the_development_override_is_searched() {
    assert_eq!(Dcpomatic::search_dir_for(None), PathBuf::from(PRIVATE_DIR));
    assert_eq!(Dcpomatic::search_dir_for(Some("".into())), PathBuf::from(PRIVATE_DIR), "an empty override is no override");
    assert_eq!(Dcpomatic::search_dir_for(Some("/opt/build/bin".into())), PathBuf::from("/opt/build/bin"));

    let d = dir("locate");
    tools(&d, "2.18.51", "", "");
    let e = Dcpomatic::locate_in(&d).unwrap_err();
    assert!(matches!(e, DcpomaticError::Unavailable(ref m) if m.contains("dcpomatic2_openssl is missing")), "the patched OpenSSL is needed too: {e}");
    script(&d.join(OPENSSL), "exit 0");
    let t = Dcpomatic::locate_in(&d).unwrap();
    assert_eq!((t.create_program(), t.cli_program()), (d.join(CREATE).as_path(), d.join(CLI).as_path()));
    std::fs::remove_file(d.join(CLI)).unwrap();
    let e = Dcpomatic::locate_in(&d).unwrap_err();
    assert!(matches!(e, DcpomaticError::Unavailable(ref m) if m.contains("dcpomatic2_cli is missing")), "{e}");
}

#[test]
fn containers_are_the_ratios_dcpomatic_accepts() {
    use Container::*;
    let ids: Vec<&str> = [Flat, Scope, R119, R133, R137, R138, R166, R178].map(Container::ratio_id).to_vec();
    // `dcpomatic2_create --help`, 2.18.51 (DCPOM-01 audit): anything else is silently ignored
    assert_eq!(ids, ["185", "239", "119", "133", "137", "138", "166", "178"]);
    assert_eq!(serde_json::to_string(&R178).unwrap(), r#""1.78""#);
    assert_eq!(serde_json::from_str::<Container>(r#""flat""#).unwrap(), Flat);
    assert!(serde_json::from_str::<Container>(r#""full""#).is_err(), "there is no 1.90 container");
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
    let t = tools(&d, "2.18.51", "", &format!("(sleep 30; touch '{}/survivor') & sleep 30", d.display()));
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
    let t = tools(&d, "2.18.51", "sleep 30", "").with_timeout(Duration::from_millis(700));
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

/// A fake `dcpomatic2_kdm_cli` in `d` next to the other tools: records its arguments and where
/// `$XDG_CONFIG_HOME/dcpomatic2` points, writes one KDM into `--output` and runs `tail`.
fn kdm_tool(d: &Path, tail: &str) -> Dcpomatic {
    let dd = d.display();
    script(
        &d.join("dcpomatic2_kdm_cli"),
        &format!(
            r#"printf '%s\n' "$@" > '{dd}/kdm.args'
readlink "$XDG_CONFIG_HOME/dcpomatic2" > '{dd}/kdm.config'
printf '%s' "$TZ" > '{dd}/kdm.tz'
out=""; prev=""
for a in "$@"; do if [ "$prev" = "--output" ]; then out="$a"; fi; prev="$a"; done
echo '<DCinemaSecurityMessage/>' > "$out/Test_KDM.xml"
{tail}"#
        ),
    );
    tools(d, "2.18.51", "", "").with_kdm_program(d.join("dcpomatic2_kdm_cli"))
}

fn encrypted_film(d: &Path) -> KdmRequest {
    std::fs::create_dir_all(d.join("film")).unwrap();
    std::fs::write(d.join("film/metadata.xml"), "<Metadata><Encrypted>1</Encrypted></Metadata>").unwrap();
    std::fs::write(d.join("projector.pem"), "-----BEGIN CERTIFICATE-----\n").unwrap();
    KdmRequest {
        film_dir: d.join("film"),
        projector_certificate: d.join("projector.pem"),
        output: d.join("kdms"),
        cinema: Some("Uránia".into()),
        screen: Some("Nagyterem".into()),
        valid_from: Some("2026-11-20 10:00:00".into()),
        validity: Validity::For { count: 2, unit: ValidityUnit::Weeks },
        formulation: Formulation::default(),
        trusted_devices: Vec::new(),
        disable_forensic_marking_picture: true,
        disable_forensic_marking_audio: true,
        config_dir: Some(d.join("config")),
    }
}

#[test]
fn encrypt_asks_for_an_encrypted_dcp() {
    let d = dir("encrypt");
    let mut r = request(&d);
    r.encrypt = true;
    let out = tools(&d, "2.18.51", "", "").make_dcp(&r, &Control::new()).unwrap();
    let a = args(&d, "create");
    assert!(a.contains(&"--encrypt".to_string()) && !a.contains(&"--no-encrypt".to_string()), "{a:?}");
    // the content keys in metadata.xml: written inside a private staging folder, moved into place
    let staged = &a[a.iter().position(|x| x == "--output").unwrap() + 1];
    assert!(staged.starts_with(&format!("{}/.cfa4-dcpomatic-stage-", d.display())) && staged.ends_with("/film"), "{staged}");
    assert_eq!(out.film_dir, d.join("film"));
    assert!(d.join("film/metadata.xml").is_file());
    assert_eq!(std::fs::metadata(d.join("film")).unwrap().permissions().mode() & 0o777, 0o700);
    let left: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with(".cfa4")).collect();
    assert!(left.is_empty(), "the staging folder is removed: {left:?}");
}

#[test]
fn makes_a_kdm_from_the_film() {
    let d = dir("kdm");
    let t = kdm_tool(&d, "");
    std::fs::create_dir_all(d.join("kdms")).unwrap();
    std::fs::write(d.join("kdms/Earlier_KDM.xml"), "<earlier/>").unwrap();
    let out = t.make_kdm(&encrypted_film(&d)).unwrap();
    assert_eq!(out.files, [d.join("kdms/Test_KDM.xml")], "only this run's KDMs");
    assert!(d.join("kdms/Earlier_KDM.xml").is_file());
    let left: Vec<_> =
        std::fs::read_dir(d.join("kdms")).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with(".cfa4")).collect();
    assert!(left.is_empty(), "the run folder is removed: {left:?}");
    assert_eq!(std::fs::read_to_string(d.join("kdm.tz")).unwrap(), "UTC", "the times are UTC");
    let s = |x: &str| x.to_string();
    let mut a = args(&d, "kdm");
    assert!(a[1].starts_with(&format!("{}/.cfa4-kdm-", d.join("kdms").display())), "the run writes into its own folder: {}", a[1]);
    a[1] = d.join("kdms").display().to_string();
    assert_eq!(
        a,
        [
            s("--output"),
            d.join("kdms").display().to_string(),
            s("--verbose"),
            s("--valid-from"),
            s("2026-11-20 10:00:00"),
            s("--valid-duration"),
            s("2 weeks"),
            s("--formulation"),
            s("modified-transitional-1"),
            s("--projector-certificate"),
            d.join("projector.pem").display().to_string(),
            s("--cinema"),
            s("Uránia"),
            s("--screen"),
            s("Nagyterem"),
            s("--disable-forensic-marking-picture"),
            s("--disable-forensic-marking-audio=0"),
            s("create"),
            d.join("film").display().to_string(),
        ]
    );
    let cfg = std::fs::read_to_string(d.join("kdm.config")).unwrap();
    assert_eq!(cfg.trim(), d.join("config").display().to_string(), "the KDM tool reads CFA4's DCP-o-matic configuration");
}

#[test]
fn bad_kdm_requests_are_refused() {
    let d = dir("kdm-invalid");
    let t = kdm_tool(&d, "");
    let check = |r: KdmRequest, want: &str| {
        let e = t.make_kdm(&r).unwrap_err();
        assert!(matches!(e, DcpomaticError::Invalid(ref m) if m.contains(want)), "{want}: {e}");
    };
    let mut r = encrypted_film(&d);
    r.valid_from = Some("tomorrow".into());
    check(r, "YYYY-MM-DD");
    let mut r = encrypted_film(&d);
    r.validity = Validity::Until("2026-11-19 10:00:00".into());
    check(r, "not after");
    let mut r = encrypted_film(&d);
    r.validity = Validity::For { count: 0, unit: ValidityUnit::Days };
    check(r, "at least 1");
    let mut r = encrypted_film(&d);
    r.cinema = Some("-x".into());
    check(r, "starts with '-'");
    let mut r = encrypted_film(&d);
    r.projector_certificate = d.join("none.pem");
    check(r, "does not exist");
    let r = encrypted_film(&d);
    std::fs::write(d.join("film/metadata.xml"), "<Metadata><Encrypted>0</Encrypted></Metadata>").unwrap();
    check(r, "not encrypted");
    let r = encrypted_film(&d);
    std::fs::remove_file(d.join("film/metadata.xml")).unwrap();
    assert!(std::process::Command::new("mkfifo").arg(d.join("film/metadata.xml")).status().unwrap().success());
    check(r, "not a DCP-o-matic film directory");
    std::fs::remove_file(d.join("film/metadata.xml")).unwrap();
    let r = encrypted_film(&d);
    std::fs::File::create(d.join("film/metadata.xml")).unwrap().set_len(17 << 20).unwrap();
    check(r, "larger than");
    assert!(!d.join("kdm.args").exists(), "nothing ran");
}

#[test]
fn kdm_errors_and_missing_tool() {
    let d = dir("kdm-fail");
    let e = kdm_tool(&d, "echo 'could not read certificate' >&2; exit 1").make_kdm(&encrypted_film(&d)).unwrap_err();
    assert!(matches!(e, DcpomaticError::Engine(ref m) if m.ends_with("could not read certificate")), "{e}");
    let e = tools(&d, "2.18.51", "", "").make_kdm(&encrypted_film(&d)).unwrap_err();
    assert!(matches!(e, DcpomaticError::Unavailable(_)), "{e}");
}

#[test]
fn subtitle_files_become_content_after_the_picture() {
    let d = dir("subs");
    std::fs::write(d.join("en.srt"), "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();
    std::fs::write(d.join("notes.txt"), "x").unwrap();
    let t = tools(&d, "2.18.51", "", "");
    let mut r = request(&d);
    r.subtitles = vec![d.join("en.srt")];
    t.make_dcp(&r, &Control::new()).unwrap();
    let a = args(&d, "create");
    assert_eq!(a[a.len() - 2..], [d.join("master.mov").display().to_string(), d.join("en.srt").display().to_string()]);

    let check = |subs: Vec<PathBuf>, want: &str| {
        let mut r = request(&d);
        r.film_dir = d.join("film2");
        r.subtitles = subs;
        let e = t.make_dcp(&r, &Control::new()).unwrap_err();
        assert!(matches!(e, DcpomaticError::Invalid(ref m) if m.contains(want)), "{want}: {e}");
    };
    check(vec![d.join("notes.txt")], "not .srt");
    check(vec![d.join("missing.srt")], "does not exist");
    std::fs::write(d.join("hu.srt"), "1\n00:00:01,000 --> 00:00:02,000\nSzia\n").unwrap();
    check(vec![d.join("en.srt"), d.join("hu.srt")], "only one subtitle file");
}

#[test]
fn locate_takes_the_kdm_tool_when_it_is_there() {
    let d = dir("locate-kdm");
    tools(&d, "2.18.51", "", "");
    script(&d.join(OPENSSL), "exit 0");
    assert_eq!(Dcpomatic::locate_in(&d).unwrap().kdm_program(), None);
    script(&d.join(KDM_CLI), "exit 0");
    assert_eq!(Dcpomatic::locate_in(&d).unwrap().kdm_program(), Some(d.join(KDM_CLI).as_path()));
}
