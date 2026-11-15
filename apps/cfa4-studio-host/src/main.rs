//! `cfa4-studio-host`: the live studio's own process (CFA4-LIVE-STUDIO-001 D2, V.1).
//!
//! It runs the studio backend (libobs, or the pure-Rust software backend), shows the empty scene
//! until the engine sends one (S1: `{"cmd":"scene",…}`, layers of still pictures) and writes
//! every program frame into the frame ring the engine created. The engine's
//! supervisor (`cfa4_obs::host::Supervisor`) starts it, watches it and restarts it after a
//! crash; libobs is C code, and its faults end this process, never the editor.
//!
//! ```text
//! cfa4-studio-host --frames RING [--width W] [--height H] [--fps N/D]
//!                  [--backend auto|libobs|software] [--libobs-dir DIR] [--generation N]
//! ```
//!
//! Protocol: JSON lines. Events go to stdout (`{"event":"ready",…}`, `{"event":"error",…}`,
//! `{"event":"sceneShown",…}`, `{"event":"stopped"}`); commands come on stdin (`{"cmd":"quit"}`,
//! `{"cmd":"scene","scene":{…}}`; EOF also quits). Log lines
//! (ours and libobs's) go to stderr.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use cfa4_obs::host::{HostCommand, HostEvent};
use cfa4_obs::ring::RingWriter;
use cfa4_obs::scene::LoadedScene;
use cfa4_obs::{BackendChoice, StudioBackend, VideoSettings};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

struct Args {
    frames: PathBuf,
    video: VideoSettings,
    backend: BackendChoice,
    libobs_dir: PathBuf,
    generation: u32,
    /// Test hook: abort (as a C crash would) after this many frames.
    abort_after: Option<u64>,
}

const USAGE: &str =
    "usage: cfa4-studio-host --frames RING [--width W] [--height H] [--fps N/D] [--backend auto|libobs|software] [--libobs-dir DIR] [--generation N]";

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut frames = None;
    let mut video = VideoSettings::default();
    let mut backend = BackendChoice::Auto;
    let mut libobs_dir = cfa4_obs::libobs::default_dir();
    let mut generation = 1;
    let mut abort_after = None;
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{a} needs a value\n{USAGE}"));
        let num = |v: String| v.parse::<u32>().map_err(|_| format!("{a}: `{v}` is not a number"));
        match a.as_str() {
            "--frames" => frames = Some(PathBuf::from(value()?)),
            "--width" => video.width = num(value()?)?,
            "--height" => video.height = num(value()?)?,
            "--fps" => {
                let v = value()?;
                let (n, d) = v.split_once('/').unwrap_or((v.as_str(), "1"));
                video.fps_num = n.parse().map_err(|_| format!("--fps: `{v}` is not N or N/D"))?;
                video.fps_den = d.parse().map_err(|_| format!("--fps: `{v}` is not N or N/D"))?;
            }
            "--backend" => backend = BackendChoice::parse(&value()?).map_err(|e| e.to_string())?,
            "--libobs-dir" => libobs_dir = PathBuf::from(value()?),
            "--generation" => generation = num(value()?)?,
            "--test-abort-after-frames" => abort_after = Some(u64::from(num(value()?)?)),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    let frames = frames.ok_or_else(|| format!("--frames is required\n{USAGE}"))?;
    video.validate().map_err(|e| e.to_string())?;
    Ok(Args { frames, video, backend, libobs_dir, generation, abort_after })
}

/// The event channel: the process's original stdout.
struct Events(std::fs::File);

impl Events {
    fn send(&mut self, e: &HostEvent) {
        if let Ok(line) = serde_json::to_string(e) {
            let _ = writeln!(self.0, "{line}");
            let _ = self.0.flush();
        }
    }
}

/// Keeps the original stdout for events and points fd 1 at stderr, so anything a C library
/// prints to stdout (libobs logs there) ends up in the log, not in the event stream.
fn take_stdout() -> Result<Events, String> {
    let original = rustix::io::dup(std::io::stdout()).map_err(|e| format!("dup stdout: {e}"))?;
    rustix::stdio::dup2_stdout(std::io::stderr()).map_err(|e| format!("redirect stdout: {e}"))?;
    Ok(Events(std::fs::File::from(original)))
}

fn run(args: Args, events: &mut Events) -> Result<(), String> {
    let mut ring = RingWriter::open(&args.frames, args.generation).map_err(|e| e.to_string())?;
    let opened = cfa4_obs::open_backend(args.backend, args.video, &args.libobs_dir).map_err(|e| e.to_string())?;
    let mut backend: Box<dyn StudioBackend> = opened.backend;
    backend.show_empty_scene().map_err(|e| e.to_string())?;
    events.send(&HostEvent::Ready { backend: backend.name().into(), version: backend.version(), fallback_reason: opened.fallback_reason });

    // stdin: a quit command or EOF ends the loop; scenes are loaded (files read, everything
    // checked) on this thread and handed to the frame loop
    let quit = Arc::new(AtomicBool::new(false));
    let q2 = quit.clone();
    let (scenes, incoming) = std::sync::mpsc::channel::<Result<LoadedScene, String>>();
    let _stdin = std::thread::Builder::new().name("cfa4-studio-host-stdin".into()).spawn(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<HostCommand>(&line) {
                    Ok(HostCommand::Quit) => break,
                    Ok(HostCommand::Scene { scene }) => {
                        let loaded = scene.load().map_err(|e| format!("scene {}: {e}", scene.revision));
                        if scenes.send(loaded).is_err() {
                            break;
                        }
                    }
                    Err(e) => eprintln!("cfa4-studio-host: ignoring a command that cannot be read: {e}"),
                }
            }
        }));
        q2.store(true, Ordering::SeqCst);
    });

    let mut written: u64 = 0;
    let mut silent = 0u32;
    let result = loop {
        if quit.load(Ordering::SeqCst) {
            break Ok(());
        }
        // only the newest scene matters when several arrived between two frames
        if let Some(next) = incoming.try_iter().last() {
            let shown = next.and_then(|scene| match backend.set_scene(&scene) {
                Ok(()) => Ok((scene.revision, scene.layers.len())),
                Err(e) => Err(format!("scene {}: {e}", scene.revision)),
            });
            match shown {
                Ok((revision, layers)) => events.send(&HostEvent::SceneShown { revision, layers }),
                Err(message) => {
                    eprintln!("cfa4-studio-host: {message}");
                    events.send(&HostEvent::Error { message });
                }
            }
        }
        match backend.next_frame(Duration::from_millis(500)) {
            Ok(Some(frame)) => {
                silent = 0;
                if let Err(e) = ring.write(&frame) {
                    break Err(e.to_string());
                }
                written = written.saturating_add(1);
                if args.abort_after.is_some_and(|n| written >= n) {
                    eprintln!("cfa4-studio-host: test hook: aborting after {written} frames");
                    std::process::abort();
                }
            }
            Ok(None) => {
                silent += 1;
                // libobs stopped producing frames for 10 s: end, so the supervisor restarts us
                if silent >= 20 {
                    break Err("the studio backend produced no frames for 10 s".into());
                }
            }
            Err(e) => break Err(e.to_string()),
        }
    };
    backend.shutdown();
    result
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut events = match take_stdout() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cfa4-studio-host: {e}");
            return ExitCode::from(1);
        }
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(args, &mut events)));
    match outcome {
        Ok(Ok(())) => {
            events.send(&HostEvent::Stopped);
            ExitCode::SUCCESS
        }
        Ok(Err(message)) => {
            eprintln!("cfa4-studio-host: {message}");
            events.send(&HostEvent::Error { message });
            ExitCode::from(1)
        }
        Err(_) => {
            events.send(&HostEvent::Error { message: "the studio host failed unexpectedly".into() });
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn arguments_are_parsed_and_checked() {
        let a = parse_args(&args("--frames /tmp/r --width 640 --height 360 --fps 30000/1001 --backend software --generation 3")).expect("ok");
        assert_eq!(a.video, VideoSettings { width: 640, height: 360, fps_num: 30000, fps_den: 1001 });
        assert_eq!((a.backend, a.generation), (BackendChoice::Software, 3));
        assert_eq!(parse_args(&args("--frames /r --fps 25")).expect("ok").video.fps_den, 1);
        for bad in [
            "",
            "--width 640",
            "--frames",
            "--frames /r --width x",
            "--frames /r --width 7",
            "--frames /r --fps 0",
            "--frames /r --backend ndi",
            "--frames /r --bogus",
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
