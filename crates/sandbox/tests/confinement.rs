//! Phase B1's acceptance test (CFA4-SECURITY-001 V.): a process confined with its profile cannot read
//! `~/.ssh`, cannot start a program and (where the profile says so) cannot open a network socket,
//! while it still reads the files its profile grants.
//!
//! Confinement is irreversible, so every case runs in a child: the parent test re-runs this test
//! binary with `CFA4_SANDBOX_CHILD` set, selecting one `child_*` test, which applies the profile and
//! checks what it can and cannot do. A kernel without Landlock or seccomp makes the strict
//! `apply` fail; the child then reports `SKIP` (the parent prints why) instead of a false pass.

#![cfg(target_os = "linux")]

use std::fs;
use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_sandbox::{Context, Mode, ProcessKind, Profile, SandboxError, apply};

const CHILD: &str = "CFA4_SANDBOX_CHILD";

/// A fake home with a secret key, a media directory and an output directory.
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("cfa4-sandbox-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.ssh")).unwrap();
        fs::create_dir_all(root.join("media")).unwrap();
        fs::create_dir_all(root.join("out")).unwrap();
        fs::write(root.join("home/.ssh/id_ed25519"), b"-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
        fs::write(root.join("home/notes.txt"), b"private").unwrap();
        fs::write(root.join("media/clip.mov"), b"not really a movie").unwrap();
        fs::write(root.join("media/other.mov"), b"another clip").unwrap();
        fs::write(root.join("out/sibling.txt"), b"next to an output").unwrap();
        Tree { root }
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

fn readable(p: &Path) -> bool {
    fs::File::open(p).and_then(|mut f| f.read_to_end(&mut Vec::new())).is_ok()
}

fn can_exec() -> bool {
    Command::new("/bin/true").status().is_ok() || Command::new("/usr/bin/true").status().is_ok()
}

fn can_inet_socket() -> bool {
    UdpSocket::bind("127.0.0.1:0").is_ok()
}

/// A port the parent listens on while a child runs: a refused connection there is the sandbox's
/// doing, not a missing server.
const LISTENER: &str = "CFA4_SANDBOX_LISTENER";

/// Run `child` in a fresh process and return its stdout; panics when the child failed.
fn run_child(child: &str, dir: &Path) -> Option<String> {
    run_child_with(child, dir, &[])
}

fn run_child_with(child: &str, dir: &Path, env: &[(&str, String)]) -> Option<String> {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new(exe);
    cmd.args([child, "--exact", "--nocapture", "--test-threads=1"]).env(CHILD, dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{child} failed:\n{stdout}\n{stderr}");
    // the test runner prints `test child_x ... ` without a line break before the child's output,
    // so the marker can follow it on the same line
    if let Some(why) = stdout.lines().find_map(|l| l.split_once("SKIP: ").map(|(_, why)| why)) {
        eprintln!("{child}: skipped, {why}");
        return None;
    }
    assert!(stdout.contains("CHILD-OK"), "{child} did not run its checks:\n{stdout}\n{stderr}");
    Some(stdout)
}

/// In a child: the tree the parent created, or `None` in a normal test run.
fn child_tree() -> Option<Tree> {
    std::env::var_os(CHILD).map(|d| Tree { root: PathBuf::from(d) })
}

/// Apply strictly; on a kernel without the needed protections print `SKIP` and return false.
fn confine(p: &Profile) -> bool {
    match apply(p, Mode::Strict) {
        Ok(en) => {
            println!("enforcement: {en:?}");
            true
        }
        Err(SandboxError::Missing(m)) => {
            println!("\nSKIP: kernel lacks {}", m.join(", "));
            false
        }
        Err(e) => panic!("apply: {e}"),
    }
}

#[test]
fn media_host_cannot_reach_ssh_exec_or_network() {
    let t = Tree::new("media");
    run_child("child_media_host", &t.root);
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn child_media_host() {
    let Some(t) = child_tree() else { return };
    assert!(readable(&t.p("home/.ssh/id_ed25519")) && can_exec() && can_inet_socket(), "unconfined baseline");
    let ctx = Context { media_file: Some(t.p("media/clip.mov")), ..Context::default() };
    if !confine(&Profile::for_process(ProcessKind::MediaHost, &ctx)) {
        return;
    }
    assert!(readable(&t.p("media/clip.mov")), "the opened file stays readable");
    assert!(!readable(&t.p("media/other.mov")), "only the opened file");
    assert!(!readable(&t.p("home/.ssh/id_ed25519")), "~/.ssh is out of reach");
    assert!(!readable(&t.p("home/notes.txt")), "the home directory is out of reach");
    assert!(fs::write(t.p("media/new.bin"), b"x").is_err(), "the media host writes nothing");
    assert!(!can_exec(), "no program start");
    assert!(!can_inet_socket(), "no network");
    assert!(readable(Path::new("/etc/hostname")) || !Path::new("/etc/hostname").exists(), "system files stay readable");
    println!("CHILD-OK");
}

#[test]
fn engine_reaches_project_but_not_ssh() {
    let t = Tree::new("engine");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    run_child_with("child_engine", &t.root, &[(LISTENER, port)]);
    drop(listener);
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn child_engine() {
    let Some(t) = child_tree() else { return };
    let exe = ["/usr/bin/true", "/bin/true"].into_iter().map(PathBuf::from).find(|p| p.exists());
    let ctx = Context {
        media: vec![t.p("media")],
        project_dirs: vec![t.p("out")],
        export_targets: vec![t.p("out/film.mp4")],
        helper_exes: exe.iter().cloned().collect(),
        loopback_ports: vec![47_311],
        ..Context::default()
    };
    if !confine(&Profile::for_process(ProcessKind::Engine, &ctx)) {
        return;
    }
    assert!(readable(&t.p("media/clip.mov")) && readable(&t.p("media/other.mov")), "added media is readable");
    assert!(fs::write(t.p("out/film.mp4"), b"export").is_ok(), "export target is writable");
    assert!(fs::write(t.p("media/new.bin"), b"x").is_err(), "media is read-only");
    assert!(!readable(&t.p("home/.ssh/id_ed25519")), "~/.ssh is out of reach");
    if let Some(exe) = exe {
        assert!(Command::new(&exe).status().is_ok(), "its own helper starts");
        assert!(Command::new("/bin/sh").arg("-c").arg("true").status().is_err(), "other programs do not");
    }
    // loopback port rules need Landlock ABI 4, which strict mode required for this profile
    assert!(TcpListener::bind("127.0.0.1:47311").is_ok(), "its control port");
    assert!(TcpListener::bind("127.0.0.1:47312").is_err(), "no other port");
    // a port that is listening (the parent's), so only the sandbox can refuse it
    let open: u16 = std::env::var(LISTENER).unwrap().parse().unwrap();
    let e = TcpStream::connect(("127.0.0.1", open)).expect_err("connect to a port outside the profile");
    assert_eq!(e.kind(), ErrorKind::PermissionDenied, "refused by Landlock, not by the peer: {e}");
    // Landlock restricts TCP only: UDP is refused by the seccomp socket rules of a loopback profile
    let udp = UdpSocket::bind("0.0.0.0:0").and_then(|s| s.send_to(b"x", "192.0.2.1:9"));
    assert_eq!(udp.expect_err("UDP out of a loopback profile").kind(), ErrorKind::PermissionDenied);
    println!("CHILD-OK");
}

/// READY-001 V7.4: the I/O host writes inside its recording directory and nowhere else, opens its
/// device (`/dev/null` stands in for a capture card), and gets no socket, no `execve`, no `$HOME`.
#[test]
fn io_host_writes_only_its_recording_dir() {
    let t = Tree::new("iohost");
    run_child("child_io_host", &t.root);
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn child_io_host() {
    let Some(t) = child_tree() else { return };
    assert!(readable(&t.p("home/.ssh/id_ed25519")) && can_exec() && can_inet_socket(), "unconfined baseline");
    let ctx = Context { capture_devices: vec![PathBuf::from("/dev/null")], recording_dir: Some(t.p("out")), ..Context::default() };
    if !confine(&Profile::for_process(ProcessKind::IoHost, &ctx)) {
        return;
    }
    assert!(fs::write(t.p("out/take1.mxf"), b"frames").is_ok(), "writes inside the recording directory");
    assert!(fs::create_dir(t.p("out/sub")).is_ok() && fs::write(t.p("out/sub/x"), b"x").is_ok(), "also beneath it");
    let denied = fs::write(t.p("media/new.bin"), b"x").expect_err("writing outside the recording directory");
    assert_eq!(denied.kind(), ErrorKind::PermissionDenied, "EPERM/EACCES: {denied}");
    assert!(fs::write(t.p("home/notes.txt"), b"x").is_err(), "no $HOME");
    assert!(!readable(&t.p("home/.ssh/id_ed25519")), "~/.ssh is out of reach");
    assert!(!readable(&t.p("media/clip.mov")), "no media either: frames come over the ring");
    assert!(fs::OpenOptions::new().read(true).write(true).open("/dev/null").is_ok(), "its device opens read-write");
    let udp = UdpSocket::bind("127.0.0.1:0").expect_err("a socket");
    assert_eq!(udp.kind(), ErrorKind::PermissionDenied, "no sockets: {udp}");
    assert!(TcpListener::bind("127.0.0.1:0").is_err(), "no TCP either");
    assert!(!can_exec(), "no program start");
    println!("CHILD-OK");
}

#[test]
fn model_converter_writes_only_its_output() {
    let t = Tree::new("model");
    run_child("child_model_converter", &t.root);
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn child_model_converter() {
    let Some(t) = child_tree() else { return };
    let ctx = Context { model_source: Some(t.p("media/clip.mov")), model_output: Some(t.p("out/model.safetensors")), ..Context::default() };
    if !confine(&Profile::for_process(ProcessKind::ModelConverter, &ctx)) {
        return;
    }
    assert!(readable(&t.p("media/clip.mov")));
    assert!(fs::write(t.p("out/model.safetensors"), b"safe").is_ok(), "writes its output (created before confinement)");
    assert!(!readable(&t.p("out/sibling.txt")), "not the output's directory");
    assert!(fs::write(t.p("out/other.bin"), b"x").is_err(), "no other file next to the output");
    assert!(!readable(&t.p("home/.ssh/id_ed25519")));
    assert!(!can_exec() && !can_inet_socket());
    println!("CHILD-OK");
}

#[test]
fn second_apply_is_refused() {
    let t = Tree::new("twice");
    run_child("child_twice", &t.root);
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn child_twice() {
    let Some(_) = child_tree() else { return };
    let p = Profile::for_process(ProcessKind::PluginHost, &Context::default());
    if !confine(&p) {
        return;
    }
    assert!(matches!(apply(&p, Mode::Strict), Err(SandboxError::AlreadyApplied("pluginHost"))));
    let st = cfa4_sandbox::status(&Context::default());
    assert_eq!(st["self"]["kind"], "pluginHost");
    assert_eq!(st["self"]["seccompMode"], 2, "{st}");
    assert_eq!(st["self"]["noNewPrivs"], true);
    println!("CHILD-OK");
}
