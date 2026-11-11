//! The live page server: HTTP/1.1 on 127.0.0.1 only, for OBS's Browser Source (or any browser on
//! the same machine). Routes: `/` the page, `/frame.png` the program picture, `/status.json` the
//! live status, `/audio` the cue's sound (with byte ranges). Requests from another host name
//! (DNS rebinding) and anything but GET/HEAD are refused; each connection has timeouts, there
//! are at most [`MAX_CONNECTIONS`] at once, and a panic in one only drops that connection.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::store::FrameStore;

/// The page OBS loads.
pub const PAGE: &str = include_str!("page.html");
/// Most connections served at once (OBS polls a few times a second; more are dropped).
pub const MAX_CONNECTIONS: usize = 32;
/// Longest request head read.
const MAX_HEAD: u64 = 16 * 1024;
/// Largest sound file served.
const MAX_AUDIO: u64 = 2 * 1024 * 1024 * 1024;

pub struct LiveServer {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LiveServer {
    /// Listen on 127.0.0.1:`port` (0: any free port).
    pub fn start(port: u16, store: Arc<FrameStore>) -> std::io::Result<LiveServer> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let open = Arc::new(AtomicUsize::new(0));
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("cfa4-live-server".into()).spawn(move || {
            for conn in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                if open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    drop(stream);
                    continue;
                }
                open.fetch_add(1, Ordering::SeqCst);
                let open_ref = open.clone();
                let (store, open) = (store.clone(), open.clone());
                let spawned = std::thread::Builder::new().name("cfa4-live-conn".into()).spawn(move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = serve(stream, &store, port);
                    }));
                    open.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    // the connection was dropped with its closure; count it back
                    open_ref.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })?;
        Ok(LiveServer { port, stop, thread: Some(thread) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The page's address for OBS's Browser Source.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    pub fn stop(&mut self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        // wake the accept loop
        let _ = TcpStream::connect_timeout(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port).into(), Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Request {
    method: String,
    path: String,
    host: Option<String>,
    range: Option<String>,
}

fn read_request(stream: &TcpStream) -> std::io::Result<Option<Request>> {
    let mut r = BufReader::new(stream.try_clone()?.take(MAX_HEAD));
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else { return Ok(None) };
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let (mut host, mut range) = (None, None);
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            return Ok(None);
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let k = k.trim();
            if k.eq_ignore_ascii_case("host") {
                host = Some(v.trim().to_string());
            } else if k.eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    Ok(Some(Request { method: method.to_string(), path, host, range }))
}

/// Whether `host` names this machine on `port` (no other name may reach the page).
pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(h) = host else { return false };
    let h = h.to_ascii_lowercase();
    ["127.0.0.1", "localhost", "[::1]"].iter().any(|n| h == *n || h == format!("{n}:{port}"))
}

fn respond(s: &mut TcpStream, status: &str, ctype: &str, extra: &str, body: &[u8], head_only: bool) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n{extra}\r\n",
        body.len()
    )?;
    if !head_only {
        s.write_all(body)?;
    }
    s.flush()
}

fn serve(mut stream: TcpStream, store: &FrameStore, port: u16) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let Some(req) = read_request(&stream)? else { return Ok(()) };
    if !host_allowed(req.host.as_deref(), port) {
        return respond(&mut stream, "403 Forbidden", "text/plain", "", b"CFA4 live: local requests only\n", false);
    }
    let head = req.method == "HEAD";
    if req.method != "GET" && !head {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain", "Allow: GET, HEAD\r\n", b"GET only\n", false);
    }
    match req.path.as_str() {
        "/" | "/index.html" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", "", PAGE.as_bytes(), head),
        "/frame.png" => {
            let png = store.png();
            respond(&mut stream, "200 OK", "image/png", &format!("X-CFA4-Seq: {}\r\n", store.seq()), &png, head)
        }
        "/status.json" => {
            let mut status = store.status();
            status["audio"] = serde_json::json!(store.audio().is_some());
            respond(&mut stream, "200 OK", "application/json", "", status.to_string().as_bytes(), head)
        }
        "/audio" => serve_audio(&mut stream, store, req.range.as_deref(), head),
        _ => respond(&mut stream, "404 Not Found", "text/plain", "", b"not found\n", false),
    }
}

fn audio_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        Some("flac") => "audio/flac",
        Some("m4a" | "aac" | "mp4") => "audio/mp4",
        _ => "application/octet-stream",
    }
}

/// `bytes=a-b` / `bytes=a-` / `bytes=-n` of a file of `len` bytes → (first, last).
pub fn parse_range(range: &str, len: u64) -> Option<(u64, u64)> {
    let spec = range.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (first, last) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let first: u64 = a.parse().ok()?;
        let last = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (first, last)
    };
    (first <= last && first < len).then_some((first, last))
}

fn serve_audio(stream: &mut TcpStream, store: &FrameStore, range: Option<&str>, head: bool) -> std::io::Result<()> {
    let Some(path) = store.audio() else {
        return respond(stream, "404 Not Found", "text/plain", "", b"no sound on air\n", false);
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return respond(stream, "404 Not Found", "text/plain", "", b"the sound file is gone\n", false);
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len == 0 || len > MAX_AUDIO {
        return respond(stream, "404 Not Found", "text/plain", "", b"no usable sound file\n", false);
    }
    let ctype = audio_type(&path);
    let (status, first, last) = match range.map(|r| parse_range(r, len)) {
        None => ("200 OK", 0, len - 1),
        Some(Some((a, b))) => ("206 Partial Content", a, b),
        Some(None) => {
            return respond(stream, "416 Range Not Satisfiable", "text/plain", &format!("Content-Range: bytes */{len}\r\n"), b"", false);
        }
    };
    let count = last - first + 1;
    let extra = if status.starts_with("206") { format!("Content-Range: bytes {first}-{last}/{len}\r\n") } else { String::new() };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {count}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n"
    )?;
    if head {
        return stream.flush();
    }
    file.seek(SeekFrom::Start(first))?;
    std::io::copy(&mut file.take(count), stream)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(port: u16, raw: &str) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&out[..split]).into_owned(), out[split + 4..].to_vec())
    }

    #[test]
    fn serves_page_picture_status_and_refuses_the_rest() {
        let store = FrameStore::new();
        let mut srv = LiveServer::start(0, store.clone()).unwrap();
        let p = srv.port();
        assert_eq!(srv.url(), format!("http://127.0.0.1:{p}/"));
        let (h, body) = get(p, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n"));
        assert!(h.starts_with("HTTP/1.1 200") && h.contains("text/html"), "{h}");
        assert!(String::from_utf8_lossy(&body).contains("frame.png"));
        store.publish(vec![9, 8, 7]);
        let (h, body) = get(p, &format!("GET /frame.png?3 HTTP/1.1\r\nHost: localhost:{p}\r\n\r\n"));
        assert!(h.contains("image/png") && h.contains("X-CFA4-Seq: 1"), "{h}");
        assert_eq!(body, vec![9, 8, 7]);
        store.set_status(serde_json::json!({"onAir": true}));
        let (_, body) = get(p, &format!("GET /status.json HTTP/1.1\r\nHost: localhost:{p}\r\n\r\n"));
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!((v["onAir"].as_bool(), v["seq"].as_u64(), v["audio"].as_bool()), (Some(true), Some(1), Some(false)));
        // another host name (DNS rebinding), no host, other methods, other paths
        assert!(get(p, &format!("GET / HTTP/1.1\r\nHost: evil.example:{p}\r\n\r\n")).0.starts_with("HTTP/1.1 403"));
        assert!(get(p, "GET / HTTP/1.1\r\n\r\n").0.starts_with("HTTP/1.1 403"));
        assert!(get(p, &format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n")).0.starts_with("HTTP/1.1 405"));
        assert!(get(p, &format!("GET /etc/passwd HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n")).0.starts_with("HTTP/1.1 404"));
        let (h, body) = get(p, &format!("HEAD /frame.png HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n"));
        assert!(h.contains("Content-Length: 3") && body.is_empty());
        // garbage and silence do not stop the server
        get_raw(p, b"\x00\x01\x02 garbage\r\n\r\n");
        drop(TcpStream::connect(("127.0.0.1", p)).unwrap());
        assert!(get(p, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n")).0.starts_with("HTTP/1.1 200"));
        srv.stop();
        srv.stop();
        assert!(TcpStream::connect_timeout(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, p).into(), Duration::from_millis(300)).is_err());
    }

    fn get_raw(port: u16, raw: &[u8]) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = s.write_all(raw);
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
    }

    #[test]
    fn serves_the_sound_with_byte_ranges() {
        let dir = std::env::temp_dir().join(format!("cfa4-live-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("music.wav");
        std::fs::write(&file, (0u8..100).collect::<Vec<_>>()).unwrap();
        let store = FrameStore::new();
        let srv = LiveServer::start(0, store.clone()).unwrap();
        let p = srv.port();
        let host = format!("Host: 127.0.0.1:{p}\r\n");
        assert!(get(p, &format!("GET /audio HTTP/1.1\r\n{host}\r\n")).0.starts_with("HTTP/1.1 404"));
        store.set_audio(Some(file.clone()));
        let (h, body) = get(p, &format!("GET /audio HTTP/1.1\r\n{host}\r\n"));
        assert!(h.starts_with("HTTP/1.1 200") && h.contains("audio/wav") && body.len() == 100, "{h}");
        let (h, body) = get(p, &format!("GET /audio HTTP/1.1\r\n{host}Range: bytes=10-19\r\n\r\n"));
        assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 10-19/100"), "{h}");
        assert_eq!(body, (10u8..20).collect::<Vec<_>>());
        assert!(get(p, &format!("GET /audio HTTP/1.1\r\n{host}Range: bytes=500-\r\n\r\n")).0.starts_with("HTTP/1.1 416"));
        std::fs::remove_file(&file).unwrap();
        assert!(get(p, &format!("GET /audio HTTP/1.1\r\n{host}\r\n")).0.starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn ranges_and_hosts() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=9-3", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
        assert!(host_allowed(Some("LOCALHOST:80"), 80) && host_allowed(Some("127.0.0.1"), 80));
        assert!(!host_allowed(Some("127.0.0.1:81"), 80) && !host_allowed(Some("127.0.0.1.evil"), 80) && !host_allowed(None, 80));
    }
}
