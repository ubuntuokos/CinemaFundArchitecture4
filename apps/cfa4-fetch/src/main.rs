//! `cfa4-fetch`: the downloads the user asked for, in their own confined process
//! (CFA4-SECURITY-001, decision D5).
//!
//! ```text
//! cfa4-fetch [--unprotected] [--tag TAG] < request.json      (cfa4_fetch_client::Request)
//! ```
//!
//! It reads one request from standard input, checks it, creates the target directory and then
//! confines itself with the `fetch` profile of `cfa4-sandbox`: it may write only beneath the target
//! directory, connect out only to TCP port 443 (Landlock), and start no programs (seccomp). Without
//! the kernel protections it refuses to download (plan D4); `--unprotected` is the user's explicit
//! opt-in to run without them. Each missing file then streams over HTTPS into its own `<name>.<tag>.part` (the tag the engine passes, unique to the download; by default the
//! process id), its
//! size and SHA-256 are checked, and it is renamed into place. Progress, the result and errors are
//! JSON lines on standard output (`cfa4_fetch_client::Event`); exit status 0 means every file is in
//! place.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

use cfa4_fetch_client::{Event, FileSpec, MAX_REQUEST, Request};
use sha2::Digest;

/// TCP ports the downloader may connect to.
const PORTS: [u16; 1] = [443];

fn emit(e: &Event) -> bool {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", e.to_line()).and_then(|()| out.flush()).is_ok()
}

fn fail(message: String, code: u8) -> ExitCode {
    emit(&Event::Error { message });
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let mut unprotected = false;
    let mut tag = std::process::id().to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--unprotected" => unprotected = true,
            "--tag" => match args.next() {
                Some(t) if cfa4_fetch_client::valid_tag(&t) => tag = t,
                _ => return fail("--tag needs 1 to 64 ASCII letters, digits or '-'".into(), 2),
            },
            "--version" => {
                println!("cfa4-fetch {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => return fail(format!("unknown argument: {a}"), 2),
        }
    }
    let mut body = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_REQUEST + 1).read_to_end(&mut body) {
        return fail(format!("reading the request: {e}"), 2);
    }
    if body.len() as u64 > MAX_REQUEST {
        return fail(format!("the request is larger than {MAX_REQUEST} bytes"), 2);
    }
    let req: Request = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return fail(format!("unreadable request: {e}"), 2),
    };
    if let Err(e) = req.validate() {
        return fail(e, 2);
    }
    if let Err(e) = std::fs::create_dir_all(&req.dir) {
        return fail(format!("{}: {e}", req.dir.display()), 1);
    }
    // the proxy is read from the environment once (ureq's own order: ALL_PROXY, HTTPS_PROXY,
    // HTTP_PROXY), so the port the sandbox allows is the one the agent connects to
    let proxy = ureq::Proxy::try_from_env();
    if let Err(e) = confine(&req.dir, proxy.as_ref(), unprotected) {
        return fail(e, 3);
    }
    match download(&req, proxy, &tag) {
        Ok(()) => {
            emit(&Event::Done);
            ExitCode::SUCCESS
        }
        Err(e) => fail(e, 1),
    }
}

/// The `fetch` profile: write beneath `dir`, TCP out to port 443 (and to the port of `proxy`, the
/// one the agent uses), no program start; a certificate bundle named by `SSL_CERT_FILE` /
/// `SSL_CERT_DIR` is readable, so a system or proxy root outside `/etc` still verifies.
fn confine(dir: &Path, proxy: Option<&ureq::Proxy>, unprotected: bool) -> Result<(), String> {
    let mut ports = PORTS.to_vec();
    ports.extend(proxy.map(ureq::Proxy::port).filter(|p| !ports.contains(p)));
    let tls_roots = ["SSL_CERT_FILE", "SSL_CERT_DIR"]
        .iter()
        .filter_map(std::env::var_os)
        .flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .filter(|p| p.is_absolute() && p.exists())
        .collect();
    let ctx = cfa4_sandbox::Context { download_target: Some(dir.to_path_buf()), fetch_ports: ports, tls_roots, ..cfa4_sandbox::Context::default() };
    let profile = cfa4_sandbox::Profile::for_process(cfa4_sandbox::ProcessKind::Fetch, &ctx);
    let mode = if unprotected { cfa4_sandbox::Mode::Unprotected } else { cfa4_sandbox::Mode::Strict };
    cfa4_sandbox::apply(&profile, mode).map(|_| ()).map_err(|e| format!("the downloader cannot protect itself, so nothing was downloaded: {e}"))
}

/// Present with the pinned size and SHA-256.
fn valid_file(path: &Path, f: &FileSpec) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() || meta.len() != f.size {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(buf.get(..n).unwrap_or_default()),
            Err(_) => return false,
        }
    }
    hex(&hasher.finalize()).eq_ignore_ascii_case(&f.sha256)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The longest a download may take as a whole (a multi-gigabyte model on a slow link); a server
/// that stops sending is caught sooner by the engine, which stops a downloader that has been
/// silent for [`cfa4_fetch_client::STALL`].
const BODY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

fn agent(proxy: Option<ureq::Proxy>) -> ureq::Agent {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(provider)
                .build(),
        )
        // redirects (model hosts send downloads to their CDN) stay on HTTPS
        .https_only(true)
        .proxy(proxy)
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        .timeout_recv_body(Some(BODY_TIMEOUT))
        .build()
        .into()
}

fn download(req: &Request, proxy: Option<ureq::Proxy>, tag: &str) -> Result<(), String> {
    let dir = &req.dir;
    let missing: Vec<&FileSpec> = req.files.iter().filter(|f| !valid_file(&dir.join(&f.name), f)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    // validate() checked that the sizes of all files add up without overflow
    let total: u64 = missing.iter().fold(0u64, |t, f| t.saturating_add(f.size));
    let agent = agent(proxy);
    let mut done = 0u64;
    for f in missing {
        let dest = dir.join(&f.name);
        // this run's own partial file: another download into the same directory has its own
        let part = dir.join(cfa4_fetch_client::part_name(&f.name, tag));
        // created anew, never opened through an existing entry: a symbolic link there is refused,
        // not followed, and so is another run's partial file with the same tag, which is then
        // left alone (nothing is removed when this fails)
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(|e| format!("{}: {e} (is another download with the same tag running?)", part.display()))?;
        let r = fetch_one(&agent, f, file, &part, done, total);
        if let Err(e) = r {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        // on every failure this run removes its own partial file before it reports the error: the
        // engine does not remove it then (it could not tell it from another run's)
        std::fs::rename(&part, &dest).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            format!("{}: {e}", dest.display())
        })?;
        done = done.saturating_add(f.size);
    }
    Ok(())
}

fn fetch_one(agent: &ureq::Agent, f: &FileSpec, file: std::fs::File, part: &Path, done: u64, total: u64) -> Result<(), String> {
    let resp = agent.get(&f.url).call().map_err(|e| format!("{}: {e}", f.url))?;
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut out = std::io::BufWriter::new(file);
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut got = 0u64;
    let mut reported = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("{}: {e}", f.name))?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).unwrap_or_default();
        got += n as u64;
        // never more than announced: a server sending an endless body stops here
        if got > f.size {
            return Err(format!("{}: larger than the expected {} bytes", f.name, f.size));
        }
        hasher.update(chunk);
        out.write_all(chunk).map_err(|e| format!("{}: {e}", part.display()))?;
        // a progress line per 256 KiB (and at the end of the file) is enough for the engine
        if (got - reported >= 256 << 10 || got == f.size) && !emit(&Event::Progress { done: done.saturating_add(got), total, file: f.name.clone() }) {
            return Err("the engine stopped listening".into());
        }
        if got - reported >= 256 << 10 {
            reported = got;
        }
    }
    out.flush().map_err(|e| format!("{}: {e}", part.display()))?;
    drop(out);
    let sha = hex(&hasher.finalize());
    if !sha.eq_ignore_ascii_case(&f.sha256) || got != f.size {
        return Err(format!("{}: checksum mismatch (got {sha}, {got} bytes; expected {}, {} bytes)", f.name, f.sha256, f.size));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_allows_the_port_of_the_proxy_the_agent_uses() {
        // confine() allows `Proxy::port`, the port ureq connects to, with the scheme's default
        for (url, port) in [
            ("http://127.0.0.1:37921", 37921),
            ("http://user:pw@proxy.lan:3128/", 3128),
            ("proxy.lan:8080", 8080),
            ("http://proxy.lan", 80),
            ("socks5://[::1]:9050", 9050),
        ] {
            assert_eq!(ureq::Proxy::new(url).unwrap().port(), port, "{url}");
        }
    }

    #[test]
    fn a_file_is_valid_only_with_its_size_and_sha256() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.bin");
        std::fs::write(&p, b"abc").unwrap();
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let f = FileSpec { name: "a.bin".into(), url: "https://example.org/a.bin".into(), sha256: sha.into(), size: 3 };
        assert!(valid_file(&p, &f));
        assert!(!valid_file(&p, &FileSpec { size: 4, ..f.clone() }));
        assert!(!valid_file(&p, &FileSpec { sha256: sha.replace('b', "c"), ..f.clone() }));
        assert!(!valid_file(&t.path().join("missing"), &f));
        // nothing missing: no network needed
        assert_eq!(download(&Request { dir: t.path().to_path_buf(), files: vec![f] }, None, "t"), Ok(()));
    }

    #[test]
    fn another_runs_partial_file_with_the_same_tag_is_refused_and_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let f = FileSpec { name: "m.bin".into(), url: "https://example.invalid/m.bin".into(), sha256: "0".repeat(64), size: 3 };
        let part = t.path().join(cfa4_fetch_client::part_name("m.bin", "t"));
        std::fs::write(&part, b"theirs").unwrap();
        // the partial file is created before any connection, so this needs no network
        let e = download(&Request { dir: t.path().to_path_buf(), files: vec![f] }, None, "t").unwrap_err();
        assert!(e.contains("same tag"), "{e}");
        assert_eq!(std::fs::read(&part).unwrap(), b"theirs");
        assert!(!t.path().join("m.bin").exists());
    }
}
