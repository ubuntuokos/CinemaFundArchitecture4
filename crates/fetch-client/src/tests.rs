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
    // sizes that overflow together (each alone is fine)
    let huge = |r: &mut Request| {
        r.files[0].size = u64::MAX;
        r.files.push(FileSpec { name: "b.bin".into(), size: 1, ..r.files[0].clone() });
    };
    assert!(bad(&huge).contains("2^64"));
    assert_eq!(ok.total_size(), Some(10));
}

#[test]
fn events_are_one_json_line_each() {
    let e = Event::Progress { done: 5, total: 10, file: "a.bin".into() };
    assert_eq!(e.to_line(), r#"{"event":"progress","done":5,"total":10,"file":"a.bin"}"#);
    assert_eq!(serde_json::from_str::<Event>(&e.to_line()).unwrap(), e);
    assert_eq!(Event::Done.to_line(), r#"{"event":"done"}"#);
    assert_eq!(serde_json::from_str::<Event>(r#"{"event":"error","message":"x"}"#).unwrap(), Event::Error { message: "x".into() });
}

/// The stand-in downloaders run as `/bin/sh <script>`: executing a script just written can fail with
/// "Text file busy" while another test thread's child still holds it open for writing.
#[cfg(unix)]
const SH_PATH: &str = "/bin/sh";

/// A stand-in downloader: a shell script printing `lines`, then sleeping `sleep` seconds.
#[cfg(unix)]
fn fake(dir: &Path, lines: &[&str], sleep: u32, exit: i32) -> PathBuf {
    fake_with(dir, "", lines, sleep, exit)
}

/// [`fake`] running the shell commands `pre` first (`$$` is the downloader's process id).
#[cfg(unix)]
fn fake_with(dir: &Path, pre: &str, lines: &[&str], sleep: u32, exit: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("fake-fetch");
    let mut s = format!("#!/bin/sh\ncat >/dev/null\n{pre}\n");
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
    run(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |d, total, f| {
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
    // another downloader's partial file is left alone
    let other = t.path().join(part_name("model.safetensors", 1));
    std::fs::write(&other, b"theirs").unwrap();
    let pre = format!("printf half > '{}/model.safetensors.'$$'.part'", t.path().display());
    let prog = fake_with(t.path(), &pre, &[r#"{"event":"error","message":"model.safetensors: checksum mismatch"}"#], 0, 1);
    let e = run(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |_, _, _| true).unwrap_err();
    assert!(e.to_string().contains("checksum mismatch"), "{e}");
    let left: Vec<_> = std::fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.ends_with(".part")).collect();
    assert_eq!(left, vec![part_name("model.safetensors", 1)]);
}

#[cfg(unix)]
#[test]
fn run_fails_when_the_downloader_stops_without_a_result() {
    let t = tempfile::tempdir().unwrap();
    for (lines, code) in [(&[][..], 0), (&["not json"][..], 0), (&[r#"{"event":"done"}"#][..], 3)] {
        let prog = fake(t.path(), lines, 0, code);
        assert!(
            matches!(run(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |_, _, _| true), Err(FetchError::Failed(_))),
            "{lines:?} {code}"
        );
    }
}

#[cfg(unix)]
#[test]
fn cancelling_stops_a_stalled_download() {
    let t = tempfile::tempdir().unwrap();
    let prog = fake(t.path(), &[r#"{"event":"progress","done":1,"total":10,"file":"model.safetensors"}"#], 60, 0);
    let start = std::time::Instant::now();
    let mut calls = 0;
    let e = run(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |_, _, _| {
        calls += 1;
        calls < 3
    })
    .unwrap_err();
    assert!(matches!(e, FetchError::Cancelled), "{e}");
    assert!(start.elapsed() < Duration::from_secs(20), "{:?}", start.elapsed());
}

#[cfg(unix)]
#[test]
fn a_silent_downloader_is_stopped() {
    let t = tempfile::tempdir().unwrap();
    let prog = fake(t.path(), &[r#"{"event":"progress","done":1,"total":10,"file":"model.safetensors"}"#], 60, 0);
    let start = std::time::Instant::now();
    let e = run_with(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |_, _, _| true, Duration::from_secs(1)).unwrap_err();
    assert!(e.to_string().contains("received nothing"), "{e}");
    assert!(start.elapsed() < Duration::from_secs(20), "{:?}", start.elapsed());
}

#[cfg(unix)]
#[test]
fn a_downloader_that_stays_after_its_last_line_is_stopped() {
    let t = tempfile::tempdir().unwrap();
    // an error stops it at once; after "done" it gets a few seconds to exit
    for (line, want) in [(r#"{"event":"error","message":"boom"}"#, "boom"), (r#"{"event":"done"}"#, "without finishing")] {
        let prog = fake(t.path(), &[line], 60, 0);
        let start = std::time::Instant::now();
        let e = run_with(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |_, _, _| true, Duration::from_secs(60)).unwrap_err();
        assert!(e.to_string().contains(want), "{e}");
        assert!(start.elapsed() < Duration::from_secs(20), "{:?}", start.elapsed());
    }
}

#[cfg(unix)]
#[test]
fn a_downloader_that_closes_its_output_but_does_not_exit_is_stopped() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let prog = t.path().join("fake-fetch");
    std::fs::write(&prog, "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"event\":\"done\"}'\nexec >&-\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
    let start = std::time::Instant::now();
    let e = run_with(Path::new(SH_PATH), &[prog.to_str().unwrap()], &request(t.path()), &mut |_, _, _| true, Duration::from_secs(60)).unwrap_err();
    assert!(e.to_string().contains("without finishing"), "{e}");
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
