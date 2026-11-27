//! The S1 tests of the live studio host (CFA4-LIVE-STUDIO-001 S1): a scene of layers sent to the
//! host is shown on the program, kept across a host crash, and a scene that cannot be loaded
//! leaves the shown one in place. With the CFA4 libobs build and a display, libobs shows the same
//! scene as the software compositor.

use cfa4_obs::host::{HostOptions, HostState, Supervisor};
use cfa4_obs::ring::RingFrame;
use cfa4_obs::scene::{self, HostScene, Layer, PictureRef, Transform};
use cfa4_obs::{BackendChoice, VideoSettings};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const VIDEO: VideoSettings = VideoSettings { width: 160, height: 90, fps_num: 30, fps_den: 1 };

fn options(backend: BackendChoice) -> HostOptions {
    let mut o = HostOptions::new(VIDEO);
    o.exe = PathBuf::from(env!("CARGO_BIN_EXE_cfa4-studio-host"));
    o.backend = backend;
    o
}

fn wait_for(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !f() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A private directory with a 20×10 picture: left half red, right half green (RGBA).
fn picture_dir(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("cfa4-host-scene-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut rgba = Vec::new();
    for _y in 0..10 {
        for x in 0..20 {
            rgba.extend_from_slice(if x < 10 { &[255, 0, 0, 255] } else { &[0, 255, 0, 255] });
        }
    }
    let file = dir.join("pic.rgba");
    let pic = scene::Picture { width: 20, height: 10, rgba: std::sync::Arc::new(rgba) };
    scene::write_raw(&file, &pic).expect("write");
    (dir, file)
}

/// A blue backdrop, the picture scaled 2× at (40, 20), and a white bar rotated 90° at (150, 10).
fn test_scene(file: &std::path::Path, revision: u64) -> HostScene {
    HostScene {
        revision,
        layers: vec![
            Layer { id: 1, picture: PictureRef::Color { rgba: [0, 0, 255, 255], width: 160, height: 90 }, transform: Transform::default() },
            Layer {
                id: 2,
                picture: PictureRef::Raw { path: file.to_string_lossy().into_owned(), width: 20, height: 10 },
                transform: Transform { x: 40.0, y: 20.0, scale_x: 2.0, scale_y: 2.0, ..Default::default() },
            },
            Layer {
                id: 3,
                picture: PictureRef::Color { rgba: [255, 255, 255, 255], width: 40, height: 4 },
                transform: Transform { x: 150.0, y: 10.0, rotation: 90.0, ..Default::default() },
            },
        ],
    }
}

/// RGB of the frame's pixel (x, y) (the ring holds BGRA).
fn rgb(f: &RingFrame, x: u32, y: u32) -> [u8; 3] {
    let i = (y * f.frame.stride + x * 4) as usize;
    [f.frame.data[i + 2], f.frame.data[i + 1], f.frame.data[i]]
}

/// Waits for a frame written after the host reported `revision`.
fn frame_of(sup: &Supervisor, revision: u64) -> RingFrame {
    wait_for(&format!("scene {revision}"), Duration::from_secs(30), || sup.status().scene_revision == Some(revision));
    let seq = sup.latest_frame().ok().flatten().map_or(0, |f| f.seq);
    let mut out = None;
    wait_for("a frame of the scene", Duration::from_secs(10), || {
        out = sup.latest_frame().ok().flatten().filter(|f| f.seq > seq + 1);
        out.is_some()
    });
    out.expect("frame")
}

fn check_layout(f: &RingFrame, tolerance: i32) {
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| (i32::from(*x) - i32::from(y)).abs() <= tolerance);
    for (x, y, want, what) in [
        (5, 5, [0, 0, 255], "backdrop"),
        (45, 25, [255, 0, 0], "picture, red half"),
        (75, 35, [0, 255, 0], "picture, green half"),
        (85, 25, [0, 0, 255], "right of the picture"),
        (147, 30, [255, 255, 255], "the bar runs down, left of its corner"),
        (152, 30, [0, 0, 255], "right of the bar's corner"),
    ] {
        let got = rgb(f, x, y);
        assert!(near(got, want), "{what} at ({x}, {y}): {got:?}, expected {want:?}");
    }
}

#[test]
fn a_scene_is_shown_kept_across_a_crash_and_a_bad_one_is_refused() {
    let (dir, file) = picture_dir("sw");
    let sup = Supervisor::start(options(BackendChoice::Software)).expect("start");
    sup.wait_ready(Duration::from_secs(10)).expect("ready");
    sup.set_scene(test_scene(&file, 1));
    let f = frame_of(&sup, 1);
    check_layout(&f, 0);
    assert_eq!(sup.status().scene_layers, 3);
    // the software backend is the reference compositor: the frame is exactly its composite
    let expect = scene::compose(&test_scene(&file, 1).load().expect("load"), &VIDEO, 0);
    assert_eq!(f.frame.data, expect.data);

    // a scene that cannot be loaded is an error and the shown scene stays
    let mut bad = test_scene(&file, 2);
    bad.layers[1].picture = PictureRef::Raw { path: dir.join("missing.rgba").to_string_lossy().into_owned(), width: 20, height: 10 };
    sup.set_scene(bad);
    wait_for("the error", Duration::from_secs(10), || sup.status().error.is_some_and(|e| e.contains("scene 2")));
    assert_eq!(sup.status().scene_revision, Some(1));

    // a killed host comes back with the scene it was last given (the bad one: still refused) …
    sup.set_scene(test_scene(&file, 3));
    let f = frame_of(&sup, 3);
    let pid = sup.status().pid.expect("pid");
    let killed = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status().expect("kill");
    assert!(killed.success());
    wait_for("the restart", Duration::from_secs(10), || {
        let s = sup.status();
        s.starts == 2 && s.state == HostState::Running
    });
    // … and shows scene 3 again without being asked
    let g = frame_of(&sup, 3);
    assert!(g.seq > f.seq);
    check_layout(&g, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// With the CFA4 libobs build and a display (CI: Xvfb), libobs shows the scene through the CFA4
/// picture source and OBS's scene-item transform, matching the software compositor.
#[test]
fn libobs_shows_the_same_scene_as_the_software_compositor() {
    let dir = cfa4_obs::libobs::default_dir();
    let has_display = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !cfa4_obs::libobs::library_path(&dir).is_file() || !has_display {
        eprintln!("skipped: needs the libobs build in {} and a display", dir.display());
        return;
    }
    let (pics, file) = picture_dir("obs");
    let mut o = options(BackendChoice::Libobs);
    o.libobs_dir = dir;
    let sup = Supervisor::start(o).expect("start");
    let st = sup.wait_ready(Duration::from_secs(30)).expect("libobs ready");
    assert_eq!(st.backend.as_deref(), Some("libobs"));
    sup.set_scene(test_scene(&file, 1));
    let f = frame_of(&sup, 1);
    check_layout(&f, 2);
    // a second scene replaces the first
    let mut two = test_scene(&file, 2);
    two.layers.truncate(1);
    two.layers[0].picture = PictureRef::Color { rgba: [255, 255, 0, 255], width: 160, height: 90 };
    sup.set_scene(two);
    let g = frame_of(&sup, 2);
    assert_eq!(rgb(&g, 45, 25), [255, 255, 0]);
    // a feed (S1b): each new frame of the ring reaches the program through the layer's source
    let ring = cfa4_obs::ring::create_sized(&pics, 160, 90).expect("feed ring");
    let mut writer = cfa4_obs::ring::RingWriter::open(&ring, 1).expect("writer");
    let fill =
        |rgba: [u8; 4]| cfa4_obs::Frame { width: 160, height: 90, stride: 640, format: cfa4_obs::PixelFormat::Rgba8, pts_ns: 0, data: rgba.repeat(160 * 90) };
    writer.write(&fill([0, 0, 255, 255])).expect("blue");
    sup.set_scene(HostScene {
        revision: 3,
        layers: vec![Layer {
            id: 9,
            picture: PictureRef::Feed { path: ring.to_string_lossy().into_owned(), width: 160, height: 90 },
            transform: Transform::default(),
        }],
    });
    let h = frame_of(&sup, 3);
    assert_eq!(rgb(&h, 80, 45), [0, 0, 255]);
    writer.write(&fill([0, 255, 0, 255])).expect("green");
    wait_for("the feed's new frame", Duration::from_secs(10), || sup.latest_frame().ok().flatten().is_some_and(|f| rgb(&f, 80, 45) == [0, 255, 0]));
    cfa4_obs::ring::remove(&ring);
    let _ = std::fs::remove_dir_all(&pics);
}

/// A scene with a CFA4 title over a playing video: the video is a feed the test writes at 60 fps,
/// as the engine's player does (a moving bar); the title a still picture. Over two seconds the
/// title stays on top and the video moves; the program and shown-video rates are printed.
#[test]
fn a_title_over_a_playing_video_keeps_moving() {
    // shared CI runners: only that the program and the video move (the timing is the perf test's)
    title_over_video(10.0, 5.0);
}

/// The S1 acceptance (plan VI): the same scene runs at 60 fps: the host writes program frames at
/// the full rate and shows (nearly) every feed frame. A measurement, so ignored by default (as
/// the other perf tests: shared runners are not idle); run with
/// `cargo test --release -p cfa4-studio-host --test scene -- --ignored --nocapture`.
#[test]
#[ignore]
fn perf_a_title_over_a_playing_video_runs_at_60_fps() {
    title_over_video(57.0, 54.0);
}

fn title_over_video(min_program_fps: f64, min_shown_fps: f64) {
    use cfa4_obs::ring::{RingWriter, create_sized};
    use cfa4_obs::{Frame, PixelFormat};
    let (w, h) = (1920u32, 1080u32);
    let (dir, title) = picture_dir("fps");
    let video = VideoSettings { width: w, height: h, fps_num: 60, fps_den: 1 };
    let mut o = HostOptions::new(video);
    o.exe = PathBuf::from(env!("CARGO_BIN_EXE_cfa4-studio-host"));
    o.backend = BackendChoice::Software;
    let sup = Supervisor::start(o).expect("start");
    sup.wait_ready(Duration::from_secs(10)).expect("ready");
    let ring = create_sized(&dir, w, h).expect("feed ring");
    let mut writer = RingWriter::open(&ring, 1).expect("writer");
    // frame n: green, with a white bar at x = n mod 100 (tells the frames apart)
    let feed_frame = move |n: u64| {
        let mut data = [0u8, 160, 0, 255].repeat((w * h) as usize);
        let x = (n % 100) as usize;
        for y in 0..h as usize {
            let i = (y * w as usize + x) * 4;
            data[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        Frame { width: w, height: h, stride: w * 4, format: PixelFormat::Rgba8, pts_ns: n, data }
    };
    writer.write(&feed_frame(0)).expect("first feed frame");
    sup.set_scene(HostScene {
        revision: 1,
        layers: vec![
            Layer { id: 1, picture: PictureRef::Feed { path: ring.to_string_lossy().into_owned(), width: w, height: h }, transform: Transform::default() },
            Layer {
                id: 2,
                picture: PictureRef::Raw { path: title.to_string_lossy().into_owned(), width: 20, height: 10 },
                transform: Transform { x: 200.0, y: 800.0, scale_x: 10.0, scale_y: 10.0, ..Default::default() },
            },
        ],
    });
    let _ = frame_of(&sup, 1);
    // the player: a new feed frame every 1/60 s for 2 s
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop.clone();
    let player = std::thread::spawn(move || {
        let start = Instant::now();
        let mut n = 1u64;
        while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
            let due = Duration::from_nanos(n * 1_000_000_000 / 60);
            if let Some(wait) = due.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
            writer.write(&feed_frame(n)).expect("feed frame");
            n += 1;
        }
        n
    });
    let first = sup.latest_frame().ok().flatten().expect("frame");
    let start = Instant::now();
    let mut bars = std::collections::HashSet::new();
    let mut last = first.seq;
    while start.elapsed() < Duration::from_secs(2) {
        if let Some(f) = sup.latest_frame().ok().flatten().filter(|f| f.seq != last) {
            last = f.seq;
            // the bar's column in this program frame (row 100, above the title)
            if let Some(x) = (0..100u32).find(|&x| rgb(&f, x, 100) == [255, 255, 255]) {
                bars.insert((f.seq, x));
            }
            assert_eq!(rgb(&f, 250, 850), [255, 0, 0], "the title stays on top");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let secs = start.elapsed().as_secs_f64();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let fed = player.join().expect("player");
    let program_fps = (last - first.seq) as f64 / secs;
    let changes = {
        let mut v: Vec<_> = bars.into_iter().collect();
        v.sort();
        v.windows(2).filter(|p| p[0].1 != p[1].1).count()
    };
    let shown_fps = changes as f64 / secs;
    eprintln!("program {program_fps:.1} fps, video shown {shown_fps:.1} fps ({fed} frames fed)");
    if cfg!(debug_assertions) {
        eprintln!("timing not checked in a debug build");
        assert!(changes > 0, "the video moves");
    } else {
        assert!(program_fps >= min_program_fps, "program at {program_fps:.1} fps");
        assert!(shown_fps >= min_shown_fps, "the video shown at {shown_fps:.1} fps");
    }
    cfa4_obs::ring::remove(&ring);
    let _ = std::fs::remove_dir_all(&dir);
}
