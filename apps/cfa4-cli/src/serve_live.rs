//! The live studio host, driven from the engine (CFA4-LIVE-STUDIO-001 S0–S1).
//!
//! `live.studio.host.start` starts `cfa4-studio-host` (libobs in its own process) under the
//! supervisor of `cfa4-obs`, which restarts it after a crash; `live.studio.frame` returns its
//! newest program frame like `frame.render` does (RGBA, as bytes or through the `shm` ring), so
//! the Qt6 GUI can show it with the monitor code it already has. A crash of the host is a status
//! change here (`live.studio.host.status`), never an engine error.
//!
//! S1: the project's current studio scene (`live.studio.scene.*`, `cfa4_engine::studio`) goes to
//! the host after every request that changed it ([`upkeep`], run by the serve loop): its sources
//! are rendered with CFA4's own code (`cfa4_engine::studio::render_source`), cached by what they
//! show, and handed over as raw RGBA files in a private directory (0700) next to the frame ring.
//! A source that cannot be drawn is left out and listed in the status (`sceneProblems`).
//!
//! | method | params | result |
//! |---|---|---|
//! | `live.studio.host.start` | `{width?, height?, fps?: "N/D", backend?: "auto" \| "libobs" \| "software"}` | host status |
//! | `live.studio.host.status` | – | host status (`state: "stopped"` when none runs) |
//! | `live.studio.host.stop` | – | `{state: "stopped"}` |
//! | `live.studio.frame` | `{shm?}` | `{w, h, format: "rgba8", seq, ptsNs, backend, shm? \| shmError?}` + `w·h·4` bytes |

use cfa4_engine::Session;
use cfa4_engine::studio::{self, LayerPicture};
use cfa4_obs::host::{HostOptions, Supervisor};
use cfa4_obs::scene::{HostScene, Layer, PictureRef, Transform};
use cfa4_obs::{BackendChoice, VideoSettings};
use cfa4_project::StudioScene;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

thread_local! {
    static HOST: RefCell<Option<Supervisor>> = const { RefCell::new(None) };
    static SYNC: RefCell<SceneSync> = RefCell::new(SceneSync::default());
}

/// What the host was last given, and the rendered pictures by [`studio::picture_key`].
#[derive(Default)]
struct SceneSync {
    /// The scene model last sent (`Some(None)`: the empty scene); `None`: nothing sent to this host.
    sent: Option<Option<StudioScene>>,
    revision: u64,
    /// key → (picture, its raw file for the host)
    cache: HashMap<String, (LayerPicture, Option<PathBuf>)>,
    dir: Option<PathBuf>,
    problems: Vec<(u64, String)>,
}

impl SceneSync {
    /// The private directory of the layer pictures, created (0700) on first use.
    fn dir(&mut self) -> Result<PathBuf, String> {
        if let Some(d) = &self.dir {
            return Ok(d.clone());
        }
        use std::os::unix::fs::DirBuilderExt;
        let d = cfa4_obs::ring::default_dir().join(format!("cfa4-studio-layers-{}", std::process::id()));
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        self.dir = Some(d.clone());
        Ok(d)
    }

    fn clear(&mut self) {
        if let Some(d) = self.dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
        *self = SceneSync { revision: self.revision, ..SceneSync::default() };
    }
}

fn picture_file(dir: &std::path::Path, key: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    dir.join(format!("{:016x}.rgba", h.finish()))
}

/// Sends the project's current studio scene to the host when it changed since the last call
/// (or the host is new). Cheap when nothing changed: one comparison of the scene model.
pub fn upkeep(s: &Session) {
    let running = HOST.with(|h| h.borrow().is_some());
    if !running {
        return;
    }
    let want = s.project.studio.current_scene();
    let unchanged = SYNC.with(|y| y.borrow().sent.as_ref().is_some_and(|sent| sent.as_ref() == want));
    if unchanged {
        return;
    }
    let scene = SYNC.with(|y| build_scene(&mut y.borrow_mut(), &s.project.studio));
    HOST.with(|h| {
        if let Some(sup) = h.borrow().as_ref() {
            sup.set_scene(scene);
        }
    });
    SYNC.with(|y| y.borrow_mut().sent = Some(want.cloned()));
}

fn build_scene(sync: &mut SceneSync, st: &cfa4_project::LiveStudio) -> HostScene {
    let dir = sync.dir();
    let mut used: Vec<String> = Vec::new();
    let mut fresh: HashMap<String, (LayerPicture, Option<PathBuf>)> = HashMap::new();
    let (layers, mut problems) = studio::program_layers(st, |kind| {
        let key = studio::picture_key(kind);
        let entry = match sync.cache.get(&key).or_else(|| fresh.get(&key)) {
            Some(e) => e.clone(),
            None => {
                let pic = studio::render_source(kind)?;
                let file = match &pic {
                    LayerPicture::Color { .. } => None,
                    LayerPicture::Pixels { width, height, rgba } => {
                        let dir = dir.clone()?;
                        let f = picture_file(&dir, &key);
                        let raw = cfa4_obs::scene::Picture { width: *width, height: *height, rgba: rgba.clone() };
                        cfa4_obs::scene::write_raw(&f, &raw).map_err(|e| e.to_string())?;
                        Some(f)
                    }
                };
                let e = (pic, file);
                fresh.insert(key.clone(), e.clone());
                e
            }
        };
        used.push(key);
        Ok(entry.0)
    });
    sync.cache.extend(fresh);
    // the files of the layers, by source (program_layers keeps the order of `used`)
    let mut keys = used.iter();
    let mut host_layers = Vec::with_capacity(layers.len());
    for l in &layers {
        let Some(key) = keys.next() else { break };
        let file = sync.cache.get(key).and_then(|e| e.1.clone());
        let t = l.transform;
        let transform = Transform { x: t.x, y: t.y, scale_x: t.scale_x, scale_y: t.scale_y, rotation: t.rotation, crop: t.crop };
        let picture = match (&l.picture, file) {
            (LayerPicture::Color { rgba, width, height }, _) => PictureRef::Color { rgba: *rgba, width: *width, height: *height },
            (LayerPicture::Pixels { width, height, .. }, Some(f)) => PictureRef::Raw { path: f.to_string_lossy().into_owned(), width: *width, height: *height },
            (LayerPicture::Pixels { .. }, None) => {
                problems.push((l.source, "the picture could not be handed to the host".into()));
                continue;
            }
        };
        host_layers.push(Layer { id: l.source, picture, transform });
    }
    // forget (and delete) what this scene does not use, so the cache follows the scene
    let keep: std::collections::HashSet<&String> = used.iter().collect();
    let stale: Vec<String> = sync.cache.keys().filter(|k| !keep.contains(k)).cloned().collect();
    for k in stale {
        if let Some((_, Some(f))) = sync.cache.remove(&k) {
            let _ = std::fs::remove_file(f);
        }
    }
    sync.revision = sync.revision.saturating_add(1);
    sync.problems = problems;
    HostScene { revision: sync.revision, layers: host_layers }
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
    let mut v = serde_json::to_value(sup.status()).unwrap_or_else(|e| json!({"error": e.to_string()}));
    if let Some(o) = v.as_object_mut() {
        let problems: Vec<Value> = SYNC.with(|y| y.borrow().problems.iter().map(|(id, e)| json!({"source": id, "error": e})).collect());
        o.insert("sceneProblems".into(), Value::Array(problems));
    }
    v
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
    // the new host gets the current scene from the next upkeep
    SYNC.with(|y| y.borrow_mut().sent = None);
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
    SYNC.with(|y| y.borrow_mut().clear());
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

    /// S1: the project's current scene reaches the host after each change, and the host's
    /// frames show it. Runs when `cfa4-studio-host` was built next to the test binary.
    #[test]
    fn the_current_scene_reaches_the_host_after_each_change() {
        if !cfa4_obs::host::default_exe().is_file() {
            eprintln!("skipped: cfa4-studio-host is not built");
            return;
        }
        let mut s = Session::default();
        let scene = s.execute("live.studio.scene.add", json!({"name": "Main"})).expect("scene");
        let scene = scene["id"].as_u64().expect("id");
        s.execute("live.studio.source.add", json!({"type": "color", "color": "#FF0000", "width": 64, "height": 36})).expect("red");
        let text = s.execute("live.studio.source.add", json!({"type": "text", "text": "CFA4", "size": 12, "transform": {"x": 4, "y": 4}})).expect("text");
        let text = text["source"]["id"].as_u64().expect("id");
        s.execute("live.studio.source.add", json!({"type": "image", "path": "/nonexistent/cfa4.png"})).expect("missing picture");
        start(&json!({"width": 64, "height": 36, "fps": "60", "backend": "software"})).expect("start");
        let wait_rgba = |want: [u8; 4], x: usize, y: usize| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Ok((_, bytes)) = frame() {
                    let i = (y * 64 + x) * 4;
                    if bytes.get(i..i + 4) == Some(&want[..]) {
                        return;
                    }
                }
                assert!(std::time::Instant::now() < until, "no frame with {want:?} at ({x}, {y})");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        upkeep(&s);
        wait_rgba([255, 0, 0, 255], 60, 30);
        let st = status();
        assert_eq!(st["sceneLayers"], 2, "{st}");
        assert_eq!(st["sceneProblems"][0]["error"].as_str().map(|e| e.contains("cfa4.png")), Some(true), "{st}");
        let dir = SYNC.with(|y| y.borrow().dir.clone()).expect("layer directory");
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1, "one picture file: the text");
        // nothing changed: nothing is sent
        let rev = SYNC.with(|y| y.borrow().revision);
        upkeep(&s);
        assert_eq!(SYNC.with(|y| y.borrow().revision), rev);
        // a change (and its undo) is shown
        s.execute("live.studio.source.set", json!({"id": text, "visible": false})).expect("hide");
        s.execute("live.studio.source.add", json!({"type": "color", "color": "#0000FF", "width": 8, "height": 8, "transform": {"x": 56, "y": 28}}))
            .expect("blue");
        upkeep(&s);
        wait_rgba([0, 0, 255, 255], 60, 30);
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 0, "the hidden text's file is gone");
        s.undo();
        upkeep(&s);
        wait_rgba([255, 0, 0, 255], 60, 30);
        // another scene with nothing in it: the empty scene
        let other = s.execute("live.studio.scene.add", json!({})).expect("scene")["id"].as_u64().expect("id");
        s.execute("live.studio.scene.select", json!({"id": other})).expect("select");
        upkeep(&s);
        wait_rgba([0, 0, 0, 255], 60, 30);
        assert_ne!(other, scene);
        assert_eq!(stop()["state"], "stopped");
        assert!(!dir.exists(), "stopping removes the layer pictures");
    }

    #[test]
    fn without_a_host_status_is_stopped_and_frames_are_an_error() {
        assert_eq!(stop()["state"], "stopped");
        assert_eq!(status()["state"], "stopped");
        assert!(frame().is_err());
    }
}
