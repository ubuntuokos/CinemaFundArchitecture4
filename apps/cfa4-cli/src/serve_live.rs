//! The live studio host, driven from the engine (CFA4-LIVE-STUDIO-001 S0–S1).
//!
//! `live.studio.host.start` starts `cfa4-studio-host` (libobs in its own process) under the
//! supervisor of `cfa4-obs`, which restarts it after a crash; `live.studio.frame` returns its
//! newest program frame like `frame.render` does (RGBA, as bytes or through the `shm` ring), so
//! the Qt6 GUI can show it with the monitor code it already has. A crash of the host is a status
//! change here (`live.studio.host.status`), never an engine error.
//!
//! Every host status carries `ptp`: what `ptp4l` reports through `pmc` (lock, grandmaster,
//! offset), the TAI time and the `ts-refclk` / `mediaclk` an ST 2110 sender signals now
//! (`cfa4_obs::ptp`, ST 2110 addendum T1). It is read at most every [`PTP_REFRESH`].
//!
//! S1: the project's current studio scene (`live.studio.scene.*`, `cfa4_engine::studio`) goes to
//! the host after every request that changed it ([`upkeep`], run by the serve loop): its sources
//! are rendered with CFA4's own code (`cfa4_engine::studio::render_source`), cached by what they
//! show, and handed over as raw RGBA files in a private directory (0700) next to the frame ring.
//! A source that cannot be drawn is left out and listed in the status (`sceneProblems`).
//!
//! S1b: media and live graphics sources are moving pictures: each gets a frame ring the
//! `studio_feeds` module keeps writing (a player thread per video file; the live graphics
//! program drawn here, in [`upkeep`], once per program frame), and the host shows the newest
//! frame of each before every program frame.
//!
//! | method | params | result |
//! |---|---|---|
//! | `live.studio.host.start` | `{width?, height?, fps?: "N/D", backend?: "auto" \| "libobs" \| "software"}` | host status |
//! | `live.studio.host.status` | – | host status (`state: "stopped"` when none runs), with `ptp` |
//! | `live.studio.host.stop` | – | `{state: "stopped"}` |
//! | `live.studio.frame` | `{shm?}` | `{w, h, format: "rgba8", seq, ptsNs, backend, shm? \| shmError?}` + `w·h·4` bytes |

use crate::studio_feeds::{LiveFeed, MediaFeed};
use cfa4_engine::Session;
use cfa4_engine::studio::{self, LayerPicture};
use cfa4_obs::host::{HostOptions, Supervisor};
use cfa4_obs::scene::{HostScene, Layer, PictureRef, Transform};
use cfa4_obs::{BackendChoice, VideoSettings};
use cfa4_project::{SourceKind, StudioScene, StudioSource};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How old the PTP part of the status may be (`pmc` is a process start).
pub const PTP_REFRESH: Duration = Duration::from_secs(1);

thread_local! {
    static HOST: RefCell<Option<Supervisor>> = const { RefCell::new(None) };
    static PTP: RefCell<Option<(Instant, Value)>> = const { RefCell::new(None) };
    static SYNC: RefCell<SceneSync> = RefCell::new(SceneSync::default());
}

fn ptp_json() -> Value {
    PTP.with(|p| {
        let mut p = p.borrow_mut();
        if let Some((at, v)) = p.as_ref()
            && at.elapsed() < PTP_REFRESH
        {
            return v.clone();
        }
        let v = serde_json::to_value(cfa4_obs::ptp::query()).unwrap_or_else(|e| json!({"error": e.to_string()}));
        *p = Some((Instant::now(), v.clone()));
        v
    })
}

fn with_ptp(mut status: Value) -> Value {
    if let Some(o) = status.as_object_mut() {
        o.insert("ptp".into(), ptp_json());
    }
    status
}

/// A moving source's frame ring and what writes it.
enum Feed {
    Media(MediaFeed),
    Live(LiveFeed),
}

impl Feed {
    fn picture(&self) -> LayerPicture {
        let (width, height, ring) = match self {
            Feed::Media(m) => (m.width, m.height, m.ring.clone()),
            Feed::Live(l) => (l.width, l.height, l.ring.clone()),
        };
        LayerPicture::Feed { width, height, ring }
    }
}

/// What the host was last given, the rendered pictures by [`studio::picture_key`], and the
/// feeds of the moving sources by source id and kind.
#[derive(Default)]
struct SceneSync {
    /// The scene model last sent (`Some(None)`: the empty scene); `None`: nothing sent to this host.
    sent: Option<Option<StudioScene>>,
    revision: u64,
    /// key → (picture, its raw file for the host)
    cache: HashMap<String, (LayerPicture, Option<PathBuf>)>,
    /// `id|kind` → feed
    feeds: HashMap<String, Feed>,
    dir: Option<PathBuf>,
    problems: Vec<(u64, String)>,
    /// The host's video (the live graphics are drawn at its width, once per its frame).
    video: VideoSettings,
}

impl SceneSync {
    /// The private directory of the layer pictures, created (0700) on first use.
    fn dir(&mut self) -> Result<PathBuf, String> {
        if let Some(d) = &self.dir {
            return Ok(d.clone());
        }
        use std::os::unix::fs::DirBuilderExt;
        // one directory per host session (a test process runs several at once)
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = cfa4_obs::ring::default_dir().join(format!("cfa4-studio-layers-{}-{n}", std::process::id()));
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        self.dir = Some(d.clone());
        Ok(d)
    }

    fn clear(&mut self) {
        // the feeds stop (and remove their rings) before the directory goes
        self.feeds.clear();
        if let Some(d) = self.dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
        *self = SceneSync { revision: self.revision, video: self.video, ..SceneSync::default() };
    }
}

fn picture_file(dir: &std::path::Path, key: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    dir.join(format!("{:016x}.rgba", h.finish()))
}

fn feed_key(src: &StudioSource) -> String {
    format!("{}|{}", src.id, serde_json::to_string(&src.kind).unwrap_or_default())
}

/// Sends the project's current studio scene to the host when it changed since the last call
/// (or the host is new), then draws the live graphics feeds that are due. Cheap when nothing
/// changed: one comparison of the scene model. Returns whether a live graphics feed runs (the
/// serve loop then comes back about every frame).
pub fn upkeep(s: &mut Session) -> bool {
    let running = HOST.with(|h| h.borrow().is_some());
    if !running {
        return false;
    }
    let want = s.project.studio.current_scene();
    let unchanged = SYNC.with(|y| y.borrow().sent.as_ref().is_some_and(|sent| sent.as_ref() == want));
    if !unchanged {
        let want = want.cloned();
        let scene = SYNC.with(|y| build_scene(&mut y.borrow_mut(), s));
        HOST.with(|h| {
            if let Some(sup) = h.borrow().as_ref() {
                sup.set_scene(scene);
            }
        });
        SYNC.with(|y| y.borrow_mut().sent = Some(want));
    }
    draw_live_feeds(s)
}

/// Draws the program into every live graphics feed that is due. A program of another size
/// (the sequence changed) makes the scene be built again with a feed of the new size.
fn draw_live_feeds(s: &mut Session) -> bool {
    let (due, width) = SYNC.with(|y| {
        let y = y.borrow();
        let live: Vec<&LiveFeed> = y.feeds.values().filter_map(|f| if let Feed::Live(l) = f { Some(l) } else { None }).collect();
        (if live.is_empty() { None } else { Some(live.iter().any(|l| l.due())) }, y.video.width)
    });
    let Some(due) = due else { return false };
    if !due {
        return true;
    }
    let picture = match cfa4_engine::live::program_picture(s, width) {
        Ok(p) => p,
        Err(e) => {
            log::debug!("live studio: the live graphics program cannot be drawn: {e}");
            None
        }
    };
    SYNC.with(|y| {
        let mut y = y.borrow_mut();
        let mut rebuild = false;
        for f in y.feeds.values_mut() {
            if let Feed::Live(l) = f
                && l.due()
                && l.write(picture.clone()).is_err()
            {
                rebuild = true;
            }
        }
        if rebuild {
            y.sent = None;
        }
    });
    true
}

/// A feed for a moving source: the one it has, else a new one.
fn feed_for(sync: &mut SceneSync, fresh: &mut HashMap<String, Feed>, s: &Session, src: &StudioSource) -> Result<LayerPicture, String> {
    let key = feed_key(src);
    if let Some(f) = sync.feeds.get(&key).or_else(|| fresh.get(&key)) {
        if let Feed::Live(l) = f {
            // the program's size follows the sequence and the host's width
            let (w, h) = cfa4_engine::live::program_size(s, sync.video.width);
            if (l.width, l.height) == (w, h) {
                return Ok(f.picture());
            }
        } else {
            return Ok(f.picture());
        }
    }
    let dir = sync.dir()?;
    let feed = match &src.kind {
        SourceKind::Media { path, looping } => {
            let source = s.media.open_file(path, &*s.services).map_err(|e| format!("{path}: {e}"))?;
            Feed::Media(MediaFeed::start(&dir, path, source, *looping)?)
        }
        SourceKind::LiveGraphics {} => {
            let (w, h) = cfa4_engine::live::program_size(s, sync.video.width);
            Feed::Live(LiveFeed::new(&dir, w, h, sync.video.frame_interval())?)
        }
        _ => return Err("not a moving source".into()),
    };
    let picture = feed.picture();
    fresh.insert(key, feed);
    Ok(picture)
}

fn build_scene(sync: &mut SceneSync, s: &Session) -> HostScene {
    let st = &s.project.studio;
    let dir = sync.dir();
    let mut used: Vec<String> = Vec::new();
    let mut fresh: HashMap<String, (LayerPicture, Option<PathBuf>)> = HashMap::new();
    let mut fresh_feeds: HashMap<String, Feed> = HashMap::new();
    let mut used_feeds: Vec<String> = Vec::new();
    let (layers, mut problems) = studio::program_layers(st, |src| {
        if src.kind.is_moving() {
            let pic = feed_for(sync, &mut fresh_feeds, s, src)?;
            used_feeds.push(feed_key(src));
            used.push(String::new());
            return Ok(pic);
        }
        let kind = &src.kind;
        let key = studio::picture_key(kind);
        let entry = match sync.cache.get(&key).or_else(|| fresh.get(&key)) {
            Some(e) => e.clone(),
            None => {
                let pic = studio::render_source(kind)?;
                let file = match &pic {
                    LayerPicture::Color { .. } | LayerPicture::Feed { .. } => None,
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
    // a feed replaced by one of another size goes now (its ring with it)
    for (k, f) in fresh_feeds {
        sync.feeds.insert(k, f);
    }
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
            (LayerPicture::Feed { width, height, ring }, _) => PictureRef::Feed { path: ring.to_string_lossy().into_owned(), width: *width, height: *height },
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
    // the same for the feeds: a player of a source no longer shown stops
    let keep: std::collections::HashSet<&String> = used_feeds.iter().collect();
    sync.feeds.retain(|k, _| keep.contains(k));
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
    let status = with_ptp(status_json(&sup));
    HOST.with(|h| *h.borrow_mut() = Some(sup));
    // the new host gets the current scene from the next upkeep
    SYNC.with(|y| {
        let mut y = y.borrow_mut();
        y.sent = None;
        y.video = video;
    });
    Ok(status)
}

/// `live.studio.host.status`.
pub fn status() -> Value {
    with_ptp(HOST.with(|h| h.borrow().as_ref().map(status_json)).unwrap_or_else(|| json!({"state": "stopped"})))
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
        upkeep(&mut s);
        wait_rgba([255, 0, 0, 255], 60, 30);
        let st = status();
        assert_eq!(st["sceneLayers"], 2, "{st}");
        assert_eq!(st["sceneProblems"][0]["error"].as_str().map(|e| e.contains("cfa4.png")), Some(true), "{st}");
        let dir = SYNC.with(|y| y.borrow().dir.clone()).expect("layer directory");
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1, "one picture file: the text");
        // nothing changed: nothing is sent
        let rev = SYNC.with(|y| y.borrow().revision);
        upkeep(&mut s);
        assert_eq!(SYNC.with(|y| y.borrow().revision), rev);
        // a change (and its undo) is shown
        s.execute("live.studio.source.set", json!({"id": text, "visible": false})).expect("hide");
        s.execute("live.studio.source.add", json!({"type": "color", "color": "#0000FF", "width": 8, "height": 8, "transform": {"x": 56, "y": 28}}))
            .expect("blue");
        upkeep(&mut s);
        wait_rgba([0, 0, 255, 255], 60, 30);
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 0, "the hidden text's file is gone");
        s.undo();
        upkeep(&mut s);
        wait_rgba([255, 0, 0, 255], 60, 30);
        // another scene with nothing in it: the empty scene
        let other = s.execute("live.studio.scene.add", json!({})).expect("scene")["id"].as_u64().expect("id");
        s.execute("live.studio.scene.select", json!({"id": other})).expect("select");
        upkeep(&mut s);
        wait_rgba([0, 0, 0, 255], 60, 30);
        assert_ne!(other, scene);
        assert_eq!(stop()["state"], "stopped");
        assert!(!dir.exists(), "stopping removes the layer pictures");
    }

    /// A 32x16 YUV4MPEG2 video at 4 fps: frame 0 black, frame 1 white (BT.601 limited range).
    fn y4m(path: &std::path::Path) {
        let mut b = b"YUV4MPEG2 W32 H16 F4:1 Ip A1:1 C420jpeg\n".to_vec();
        for luma in [16u8, 235] {
            b.extend_from_slice(b"FRAME\n");
            b.extend(std::iter::repeat_n(luma, 32 * 16));
            b.extend(std::iter::repeat_n(128u8, 2 * 16 * 8));
        }
        std::fs::write(path, b).expect("write y4m");
    }

    #[test]
    fn a_playing_video_and_the_live_graphics_reach_the_host() {
        if !cfa4_obs::host::default_exe().is_file() {
            eprintln!("skipped: cfa4-studio-host is not built");
            return;
        }
        let dir = std::env::temp_dir().join(format!("cfa4-serve-feeds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let video = dir.join("flash.y4m");
        y4m(&video);
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).expect("demo");
        s.execute("live.studio.scene.add", json!({"name": "Show"})).expect("scene");
        s.execute("live.studio.source.add", json!({"type": "media", "path": video.to_string_lossy()})).expect("media");
        s.execute("live.studio.source.add", json!({"type": "liveGraphics"})).expect("live graphics");
        s.execute("live.studio.source.add", json!({"type": "media", "path": dir.join("gone.mp4").to_string_lossy()})).expect("missing video");
        start(&json!({"width": 64, "height": 36, "fps": "60", "backend": "software"})).expect("start");
        assert!(upkeep(&mut s), "a live graphics feed runs: the loop comes back every frame");
        let st = status();
        assert_eq!(st["sceneProblems"].as_array().map(Vec::len), Some(1), "{st}");
        assert_eq!(st["sceneProblems"][0]["error"].as_str().map(|e| e.contains("gone.mp4")), Some(true), "{st}");
        let feeds: Vec<(PathBuf, bool)> = SYNC.with(|y| {
            y.borrow()
                .feeds
                .values()
                .map(|f| match f {
                    Feed::Media(m) => (m.ring.clone(), false),
                    Feed::Live(l) => (l.ring.clone(), true),
                })
                .collect()
        });
        assert_eq!(feeds.len(), 2);
        // the video plays: its pixel goes black and white on the program
        let mut seen = std::collections::HashSet::new();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while seen.len() < 2 {
            assert!(std::time::Instant::now() < until, "the video did not play on the program: saw {seen:?}");
            upkeep(&mut s);
            if let Ok((_, bytes)) = frame() {
                match bytes.get(..3) {
                    Some([0..=8, 0..=8, 0..=8]) => seen.insert("black"),
                    Some([247..=255, 247..=255, 247..=255]) => seen.insert("white"),
                    _ => false,
                };
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // the live graphics: transparent until TAKE, then the program's pictures
        let live = feeds.iter().find(|f| f.1).map(|f| f.0.clone()).expect("live feed");
        let reader = cfa4_obs::ring::RingReader::open(&live).expect("live ring");
        assert_eq!(reader.latest_seq().expect("seq"), 1, "one transparent frame while nothing is on air");
        s.execute("live.cue.add", json!({"csv": "Head,Body\nDirected by,Jordan Lee\n"})).expect("cue");
        s.execute("live.take", json!({})).expect("take");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while reader.latest_seq().expect("seq") < 5 {
            assert!(std::time::Instant::now() < until, "the live program was not drawn into its feed");
            upkeep(&mut s);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // removing the sources stops the feeds and removes their rings
        let ids: Vec<u64> = s.project.studio.scenes[0].sources.iter().map(|x| x.id).collect();
        for id in ids {
            s.execute("live.studio.source.remove", json!({"id": id})).expect("remove");
        }
        assert!(!upkeep(&mut s));
        assert!(feeds.iter().all(|f| !f.0.exists()), "the feed rings are gone");
        assert_eq!(stop()["state"], "stopped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_host_status_is_stopped_and_frames_are_an_error() {
        assert_eq!(stop()["state"], "stopped");
        assert_eq!(status()["state"], "stopped");
        // The PTP part is always there; without ptp4l the sender signals its own timebase.
        let ptp = &status()["ptp"];
        assert!(ptp["mediaclk"] == "direct=0" || ptp["mediaclk"] == "sender", "{ptp}");
        assert!(ptp["tsRefclk"].as_str().is_some_and(|r| r.starts_with("ptp=") || r.starts_with("localmac=")));
        assert!(frame().is_err());
    }
}
