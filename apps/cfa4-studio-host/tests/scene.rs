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
    let _ = std::fs::remove_dir_all(&pics);
}
