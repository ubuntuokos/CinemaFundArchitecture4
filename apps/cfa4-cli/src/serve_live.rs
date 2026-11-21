//! The live studio host, driven from the engine (CFA4-LIVE-STUDIO-001 S0).
//!
//! `live.studio.host.start` starts `cfa4-studio-host` (libobs in its own process) under the
//! supervisor of `cfa4-obs`, which restarts it after a crash; `live.studio.frame` returns its
//! newest program frame like `frame.render` does (RGBA, as bytes or through the `shm` ring), so
//! the Qt6 GUI can show it with the monitor code it already has. A crash of the host is a status
//! change here (`live.studio.host.status`), never an engine error.
//!
//! | method | params | result |
//! |---|---|---|
//! | `live.studio.host.start` | `{width?, height?, fps?: "N/D", backend?: "auto" \| "libobs" \| "software"}` | host status |
//! | `live.studio.host.status` | – | host status (`state: "stopped"` when none runs) |
//! | `live.studio.host.stop` | – | `{state: "stopped"}` |
//! | `live.studio.frame` | `{shm?}` | `{w, h, format: "rgba8", seq, ptsNs, backend, shm? \| shmError?}` + `w·h·4` bytes |

use cfa4_obs::host::{HostOptions, Supervisor};
use cfa4_obs::{BackendChoice, VideoSettings};
use serde_json::{Value, json};
use std::cell::RefCell;

thread_local! {
    static HOST: RefCell<Option<Supervisor>> = const { RefCell::new(None) };
}

fn u32_param(p: &Value, key: &str, default: u32) -> Result<u32, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| format!("`{key}` must be a positive integer")),
    }
}

fn video_param(p: &Value) -> Result<VideoSettings, String> {
    let d = VideoSettings::default();
    let mut v = VideoSettings { width: u32_param(p, "width", d.width)?, height: u32_param(p, "height", d.height)?, ..d };
    match p.get("fps") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => {
            let (n, den) = s.split_once('/').unwrap_or((s.as_str(), "1"));
            v.fps_num = n.trim().parse().map_err(|_| "`fps` must be \"N\" or \"N/D\"")?;
            v.fps_den = den.trim().parse().map_err(|_| "`fps` must be \"N\" or \"N/D\"")?;
        }
        Some(Value::Number(n)) => v.fps_num = n.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("`fps` must be a positive integer")?,
        Some(_) => return Err("`fps` must be \"N\" or \"N/D\"".into()),
    }
    v.validate().map_err(|e| e.to_string())?;
    Ok(v)
}

fn status_json(sup: &Supervisor) -> Value {
    serde_json::to_value(sup.status()).unwrap_or_else(|e| json!({"error": e.to_string()}))
}

/// `live.studio.host.start`: (re)starts the host with the given video and backend.
pub fn start(p: &Value) -> Result<Value, String> {
    let video = video_param(p)?;
    let backend = match p.get("backend").and_then(Value::as_str) {
        Some(b) => BackendChoice::parse(b).map_err(|e| e.to_string())?,
        None if p.get("backend").is_some_and(|v| !v.is_null()) => return Err("`backend` must be auto, libobs or software".into()),
        None => BackendChoice::Auto,
    };
    let mut opts = HostOptions::new(video);
    opts.backend = backend;
    // one host per engine: a new start replaces the running one
    stop();
    let sup = Supervisor::start(opts).map_err(|e| e.to_string())?;
    let status = status_json(&sup);
    HOST.with(|h| *h.borrow_mut() = Some(sup));
    Ok(status)
}

/// `live.studio.host.status`.
pub fn status() -> Value {
    HOST.with(|h| h.borrow().as_ref().map(status_json)).unwrap_or_else(|| json!({"state": "stopped"}))
}

/// `live.studio.host.stop` (also run when the engine ends).
pub fn stop() -> Value {
    if let Some(mut sup) = HOST.with(|h| h.borrow_mut().take()) {
        sup.stop();
    }
    json!({"state": "stopped"})
}

/// `live.studio.frame`: the newest program frame as RGBA.
pub fn frame() -> Result<(Value, Vec<u8>), String> {
    HOST.with(|h| {
        let host = h.borrow();
        let sup = host.as_ref().ok_or("the live studio host is not running (live.studio.host.start)")?;
        let Some(rf) = sup.latest_frame().map_err(|e| e.to_string())? else {
            return Err("no live studio frame yet".into());
        };
        let rgba = cfa4_obs::ring::bgra_to_rgba(&rf.frame);
        let backend = sup.status().backend;
        Ok((json!({"w": rf.frame.width, "h": rf.frame.height, "format": "rgba8", "seq": rf.seq, "ptsNs": rf.frame.pts_ns, "backend": backend}), rgba))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_parameters_are_errors() {
        for p in [
            json!({"width": -1}),
            json!({"width": "big"}),
            json!({"width": 7}),
            json!({"height": 100000}),
            json!({"fps": "0"}),
            json!({"fps": "60/0"}),
            json!({"fps": "x/y"}),
            json!({"fps": [60, 1]}),
            json!({"fps": 100000}),
            json!({"backend": "ndi"}),
            json!({"backend": 3}),
        ] {
            assert!(start(&p).is_err(), "{p}");
        }
        assert_eq!(video_param(&json!({"fps": "30000/1001"})).map(|v| (v.fps_num, v.fps_den)), Ok((30000, 1001)));
    }

    /// Runs when `cfa4-studio-host` was built next to the test binary (`cargo test --workspace`).
    #[test]
    fn a_started_host_delivers_rgba_frames_until_stopped() {
        if !cfa4_obs::host::default_exe().is_file() {
            eprintln!("skipped: cfa4-studio-host is not built");
            return;
        }
        let st = start(&json!({"width": 64, "height": 36, "fps": "60", "backend": "software"})).expect("start");
        assert_eq!(st["video"]["width"], 64);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let (meta, bytes) = loop {
            if let Ok(f) = frame() {
                break f;
            }
            assert!(std::time::Instant::now() < until, "no frame");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!((meta["w"].as_u64(), meta["h"].as_u64(), meta["format"].as_str()), (Some(64), Some(36), Some("rgba8")));
        assert_eq!(bytes.len(), 64 * 36 * 4);
        assert!(bytes.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        assert_eq!(status()["state"], "running");
        assert_eq!(stop()["state"], "stopped");
        assert!(frame().is_err());
    }

    #[test]
    fn without_a_host_status_is_stopped_and_frames_are_an_error() {
        assert_eq!(stop()["state"], "stopped");
        assert_eq!(status()["state"], "stopped");
        assert!(frame().is_err());
    }
}
