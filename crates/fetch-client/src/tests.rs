use super::*;

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn file(name: &str, url: &str) -> FileSpec {
    FileSpec { name: name.into(), url: url.into(), sha256: SHA.into(), size: 10 }
}

fn request(dir: &Path) -> Request {
    Request { dir: dir.to_path_buf(), files: vec![file("model.safetensors", "https://example.org/model.safetensors")] }
}

#[test]
fn validation_keeps_downloads_inside_the_target_over_https() {
    let ok = request(Path::new("/data/models/whisper-base"));
    assert_eq!(ok.validate(), Ok(()));
    let bad = |f: &dyn Fn(&mut Request)| {
        let mut r = ok.clone();
        f(&mut r);
        r.validate().unwrap_err()
    };
    assert!(bad(&|r| r.dir = PathBuf::from("models")).contains("absolute"));
    assert!(bad(&|r| r.dir = PathBuf::from("/data/../etc")).contains(".."));
    for name in ["", "../x", "a/b", ".hidden", "x.part", "a\\b", "ü"] {
        assert!(bad(&|r| r.files[0].name = name.into()).contains("invalid file name"), "{name:?}");
    }
    for url in ["http://example.org/m", "file:///etc/passwd", "ftp://x", "https://a b", "https://x\n"] {
        assert!(bad(&|r| r.files[0].url = url.into()).contains("https://"), "{url:?}");
    }
    assert!(bad(&|r| r.files[0].sha256 = "abc".into()).contains("SHA-256"));
    assert!(bad(&|r| r.files.push(r.files[0].clone())).contains("twice"));
}

#[test]
fn events_are_one_json_line_each() {
    let e = Event::Progress { done: 5, total: 10, file: "a.bin".into() };
    assert_eq!(e.to_line(), r#"{"event":"progress","done":5,"total":10,"file":"a.bin"}"#);
    assert_eq!(serde_json::from_str::<Event>(&e.to_line()).unwrap(), e);
    assert_eq!(Event::Done.to_line(), r#"{"event":"done"}"#);
    assert_eq!(serde_json::from_str::<Event>(r#"{"event":"error","message":"x"}"#).unwrap(), Event::Error { message: "x".into() });
}

/// A stand-in downloader: a shell script printing `lines`, then sleeping `sleep` seconds.
#[cfg(unix)]
fn fake(dir: &Path, lines: &[&str], sleep: u32, exit: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("fake-fetch");
    let mut s = String::from("#!/bin/sh\ncat >/dev/null\n");
    for l in lines {
        s.push_str(&format!("printf '%s\\n' '{l}'\n"));
    }
    s.push_str(&format!("if [ {sleep} -gt 0 ]; then exec sleep {sleep}; fi\nexit {exit}\n"));
    std::fs::write(&p, s).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[cfg(unix)]
#[test]
fn run_reports_progress_and_success() {
    let t = tempfile::tempdir().unwrap();
    let prog = fake(t.path(), &[r#"{"event":"progress","done":4,"total":10,"file":"model.safetensors"}"#, r#"{"event":"done"}"#], 0, 0);
    let mut seen = Vec::new();
    run(&prog, &[], &request(t.path()), &mut |d, total, f| {
        seen.push((d, total, f.to_string()));
        true
    })
    .unwrap();
    assert!(seen.iter().all(|(d, total, _)| *d <= *total));
}

#[cfg(unix)]
#[test]
fn run_reports_the_downloaders_error_and_removes_part_files() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("model.safetensors.part"), b"half").unwrap();
    let prog = fake(t.path(), &[r#"{"event":"error","message":"model.safetensors: checksum mismatch"}"#], 0, 1);
    let e = run(&prog, &[], &request(t.path()), &mut |_, _, _| true).unwrap_err();
    assert!(e.to_string().contains("checksum mismatch"), "{e}");
    assert!(!t.path().join("model.safetensors.part").exists());
}

#[cfg(unix)]
#[test]
fn run_fails_when_the_downloader_stops_without_a_result() {
    let t = tempfile::tempdir().unwrap();
    for (lines, code) in [(&[][..], 0), (&["not json"][..], 0), (&[r#"{"event":"done"}"#][..], 3)] {
        let prog = fake(t.path(), lines, 0, code);
        assert!(matches!(run(&prog, &[], &request(t.path()), &mut |_, _, _| true), Err(FetchError::Failed(_))), "{lines:?} {code}");
    }
}

#[cfg(unix)]
#[test]
fn cancelling_stops_a_stalled_download() {
    let t = tempfile::tempdir().unwrap();
    let prog = fake(t.path(), &[r#"{"event":"progress","done":1,"total":10,"file":"model.safetensors"}"#], 60, 0);
    let start = std::time::Instant::now();
    let mut calls = 0;
    let e = run(&prog, &[], &request(t.path()), &mut |_, _, _| {
        calls += 1;
        calls < 3
    })
    .unwrap_err();
    assert!(matches!(e, FetchError::Cancelled), "{e}");
    assert!(start.elapsed() < Duration::from_secs(20), "{:?}", start.elapsed());
}

#[test]
fn a_missing_program_or_a_bad_request_fails_cleanly() {
    let t = tempfile::tempdir().unwrap();
    let e = run(&t.path().join("no-such-program"), &[], &request(t.path()), &mut |_, _, _| true).unwrap_err();
    assert!(matches!(e, FetchError::NotFound), "{e}");
    let mut r = request(t.path());
    r.files[0].url = "http://example.org/x".into();
    assert!(matches!(run(Path::new("/bin/true"), &[], &r, &mut |_, _, _| true), Err(FetchError::Invalid(_))));
}
