//! Prints what the GStreamer backend finds on this machine, and optionally discovers files.
//!
//! ```sh
//! cargo run -p cfa4-gstreamer --features gstreamer --example gst_probe -- [FILE…]
//! ```

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let status = match cfa4_gstreamer::probe() {
        Ok(status) => status,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    println!("GStreamer {}", status.version);
    for e in &status.elements {
        let state = if e.available { "available" } else { "missing" };
        let origin = match (&e.plugin, &e.license) {
            (Some(p), Some(l)) => format!("plugin {p}, {l}"),
            _ => String::new(),
        };
        println!("  {:<14} {:<10} {:<28} {}", e.spec.factory, state, origin, e.spec.purpose);
    }

    let mut failed = false;
    for arg in std::env::args().skip(1) {
        match cfa4_gstreamer::discover(Path::new(&arg), cfa4_gstreamer::DEFAULT_DISCOVER_TIMEOUT) {
            Ok(info) => {
                let seconds = info.duration.map(|d| format!("{:.3} s", d.seconds())).unwrap_or_else(|| "unknown".into());
                println!("\n{arg}\n  container: {}\n  duration:  {seconds}\n  seekable:  {}", info.container.as_deref().unwrap_or("none"), info.seekable);
                for s in &info.streams {
                    println!("  {:?}  codec: {}", s.kind, s.codec.as_deref().unwrap_or("?"));
                }
            }
            Err(e) => {
                eprintln!("\n{arg}: {e}");
                failed = true;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
