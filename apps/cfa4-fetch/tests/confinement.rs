//! CFA4-SECURITY-001 B9 for the downloader: what a hostile request or a hostile target directory
//! cannot make the real `cfa4-fetch` program do. Every case runs the built binary, the way the
//! engine starts it, and looks at the files and the events it leaves behind.
//!
//! Needs no network: a refused connection is the expected end of a download here, and the
//! requests that must fail do so before the program connects anywhere.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use sha2::Digest;

/// What a run of the downloader left on its standard output and its exit code.
struct Run {
    code: i32,
    events: Vec<Value>,
}

impl Run {
    fn done(&self) -> bool {
        self.events.iter().any(|e| e["event"] == "done")
    }
    fn errors(&self) -> Vec<&str> {
        self.events.iter().filter(|e| e["event"] == "error").filter_map(|e| e["message"].as_str()).collect()
    }
}

/// Start `cfa4-fetch` with `args`, feed it `stdin` and wait for it. The proxy variables are cleared,
/// so a download that gets that far is refused at once instead of reaching a real server.
fn run(args: &[&str], stdin: &[u8]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cfa4-fetch"))
        .args(args)
        .env_remove("ALL_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("HTTP_PROXY")
        .env_remove("all_proxy")
        .env_remove("https_proxy")
        .env_remove("http_proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let body = stdin.to_vec();
    // a request larger than the pipe is written while the program already reads it
    let writer = std::thread::spawn(move || {
        let _ = input.write_all(&body);
    });
    let out = child.wait_with_output().unwrap();
    let _ = writer.join();
    let events = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    Run { code: out.status.code().unwrap_or(-1), events }
}

fn sha256(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn request(dir: &Path, name: &str, url: &str, sha: &str, size: u64) -> Value {
    json!({ "dir": dir, "files": [{ "name": name, "url": url, "sha256": sha, "size": size }] })
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn a_hostile_request_is_refused_before_anything_is_created() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("models");
    let url = "https://example.invalid/model.bin";
    let bad_names = ["../escape", "a/b", "/etc/passwd", ".hidden", "..", ".", "x.part", "", "a\u{0}b", "名前", "a b", "c:\\x"];
    let mut cases: Vec<(String, Value)> = bad_names.iter().map(|n| (format!("name {n:?}"), request(&target, n, url, SHA, 1))).collect();
    for u in ["http://example.invalid/a", "file:///etc/passwd", "ftp://example.invalid/a", "https://example.invalid/a b", "HTTPS://example.invalid/a", ""] {
        cases.push((format!("url {u:?}"), request(&target, "model.bin", u, SHA, 1)));
    }
    cases.push(("short SHA-256".into(), request(&target, "model.bin", url, "abc", 1)));
    cases.push(("non-hex SHA-256".into(), request(&target, "model.bin", url, &"g".repeat(64), 1)));
    cases.push(("relative directory".into(), request(Path::new("models"), "model.bin", url, SHA, 1)));
    cases.push(("directory with ..".into(), request(&root.path().join("models/../elsewhere"), "model.bin", url, SHA, 1)));
    cases.push((
        "sizes that overflow".into(),
        json!({ "dir": target, "files": [
        { "name": "a", "url": url, "sha256": SHA, "size": u64::MAX },
        { "name": "b", "url": url, "sha256": SHA, "size": 1 } ] }),
    ));
    cases.push((
        "a file named twice".into(),
        json!({ "dir": target, "files": [
        { "name": "a", "url": url, "sha256": SHA, "size": 1 },
        { "name": "a", "url": url, "sha256": SHA, "size": 1 } ] }),
    ));
    for (what, req) in &cases {
        let r = run(&[], req.to_string().as_bytes());
        assert_eq!(r.code, 2, "{what}: refused as a bad request ({:?})", r.errors());
        assert!(!r.done() && !r.errors().is_empty(), "{what}: one error event, no result");
    }
    // not JSON at all, the wrong shape, and a request over the size limit
    for (what, body) in [("not JSON", b"hello".to_vec()), ("empty", Vec::new()), ("wrong shape", b"[1,2,3]".to_vec()), ("huge", vec![b' '; (1 << 20) + 16])] {
        let r = run(&[], &body);
        assert_eq!(r.code, 2, "{what}: refused ({:?})", r.errors());
        assert!(!r.done(), "{what}: no result");
    }
    assert!(!target.exists(), "no refused request created its target directory");
    assert!(!root.path().join("elsewhere").exists());
    assert_eq!(names_in(root.path()), Vec::<String>::new(), "nothing was created anywhere");
    // bad arguments are refused too
    for args in [&["--no-such-flag"][..], &["--tag"], &["--tag", "../x"], &["--tag", ""], &["--tag", &"a".repeat(65)]] {
        let r = run(args, b"{}");
        assert_eq!(r.code, 2, "arguments {args:?}");
    }
}

/// A directory the downloader is told to fill may hold a link to a secret elsewhere. The file's
/// size and SHA-256 are those of the secret, so a downloader that can read the secret through the
/// link believes the file is there and reports `done`; one that is confined cannot read it, goes to
/// download, and fails. The control is the same link to a file inside the directory, which the
/// confined downloader reads and accepts: the secret is refused for where it is, not for being a link.
/// (`--unprotected` is no control: it still applies the protections the kernel has.)
#[test]
fn a_confined_downloader_cannot_read_a_secret_outside_its_directory() {
    let root = tempfile::tempdir().unwrap();
    let secret = root.path().join("home/.ssh/id_ed25519");
    fs::create_dir_all(secret.parent().unwrap()).unwrap();
    let key = b"-----BEGIN OPENSSH PRIVATE KEY----- not a real key";
    fs::write(&secret, key).unwrap();
    let target = root.path().join("models");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&secret, target.join("model.bin")).unwrap();
    let req = request(&target, "model.bin", "https://example.invalid/model.bin", &sha256(key), key.len() as u64).to_string();

    let inside = root.path().join("inside");
    fs::create_dir_all(&inside).unwrap();
    fs::write(inside.join("real.bin"), key).unwrap();
    std::os::unix::fs::symlink(inside.join("real.bin"), inside.join("model.bin")).unwrap();
    let control = run(&[], request(&inside, "model.bin", "https://example.invalid/model.bin", &sha256(key), key.len() as u64).to_string().as_bytes());
    if control.code == 3 {
        eprintln!("skipped: this kernel cannot confine the downloader ({:?})", control.errors());
        return;
    }
    assert!(control.done() && control.code == 0, "control: a link to a file inside the directory counts as present ({:?})", control.errors());

    let confined = run(&[], req.as_bytes());
    assert!(
        !confined.done() && confined.code == 1,
        "confined: the secret could not be read, so the file was downloaded and the download failed (code {}, {:?})",
        confined.code,
        confined.errors()
    );
    assert_eq!(fs::read(&secret).unwrap(), key, "the secret is untouched");
    assert!(fs::symlink_metadata(target.join("model.bin")).unwrap().file_type().is_symlink(), "the link was not replaced by a download");
    assert_eq!(names_in(&target), ["model.bin"], "no partial file is left behind");
    assert_eq!(names_in(&root.path().join("home/.ssh")), ["id_ed25519"], "nothing was written next to the secret");
}

/// The downloader may write only beneath its target directory: a stale file of another name stays
/// untouched, and a failed download leaves nothing half-written in place of a file.
#[test]
fn a_failed_download_leaves_the_directory_as_it_was() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("models");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("other.bin"), b"keep me").unwrap();
    fs::write(root.path().join("neighbour.txt"), b"keep me too").unwrap();
    let req = request(&target, "model.bin", "https://example.invalid/model.bin", SHA, 10).to_string();
    let r = run(&[], req.as_bytes());
    if r.code == 3 {
        eprintln!("skipped: this kernel cannot confine the downloader ({:?})", r.errors());
        return;
    }
    assert!(!r.done() && r.code == 1, "the download fails ({:?})", r.errors());
    assert_eq!(names_in(&target), ["other.bin"], "no model.bin and no partial file");
    assert_eq!(fs::read(target.join("other.bin")).unwrap(), b"keep me");
    assert_eq!(names_in(root.path()), ["models", "neighbour.txt"]);
    assert_eq!(fs::read(root.path().join("neighbour.txt")).unwrap(), b"keep me too");
}
