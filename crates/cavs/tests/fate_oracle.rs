//! FFmpeg FATE AVS1 samples (`fate-suite/cavs/`, downloaded locally into
//! `target/fixtures/cavs/fate/`, never committed) as raw elementary streams, decoded here and by
//! ffmpeg's `cavs` decoder: every output picture must be equal, in the same order. Skipped when
//! ffmpeg or the samples are missing.
//!
//! Prepare: `ffmpeg -i bunny.mp4 -map 0:v:0 -c:v copy -f cavsvideo bunny.avs` (and `cavs.mpg`).

use std::path::PathBuf;
use std::process::{Command, Stdio};

use cfa4_cavs::{Decoder, Frame, split_packets};

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn decode_all(data: &[u8]) -> Vec<Frame> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for (i, p) in split_packets(data).into_iter().enumerate() {
        if let Ok(Some(f)) = d.decode_tagged(p, i as i64) {
            out.push(f);
        }
    }
    out.extend(d.flush());
    out
}

fn ffmpeg_raw(path: &PathBuf) -> Vec<u8> {
    let out = Command::new("ffmpeg").args(["-v", "quiet", "-i"]).arg(path).args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]).output().unwrap();
    out.stdout
}

#[test]
fn fate_samples_match_ffmpeg() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/cavs/fate");
    if !have_ffmpeg() {
        eprintln!("SKIPPED (AVS1 FATE oracle): ffmpeg is not installed");
        return;
    }
    let mut ran = 0;
    for name in ["bunny.avs", "cavs.avs"] {
        let path = dir.join(name);
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("SKIPPED (AVS1 FATE oracle): {} is missing", path.display());
            continue;
        };
        ran += 1;
        let want = ffmpeg_raw(&path);
        let got = decode_all(&data);
        let first = got.first().unwrap_or_else(|| panic!("{name}: no pictures"));
        let size = (first.width as usize * first.height as usize) + 2 * (first.width.div_ceil(2) as usize * first.height.div_ceil(2) as usize);
        assert_eq!(want.len() % size, 0, "{name}: ffmpeg output size");
        let n = want.len() / size;
        for (i, (f, w)) in got.iter().zip(want.chunks(size)).enumerate() {
            let g: Vec<u8> = f.planes.concat();
            if g != w {
                let ny = f.width as usize * f.height as usize;
                let pos = g.iter().zip(w).position(|(a, b)| a != b).unwrap_or(0);
                let (plane, at, wd) = if pos < ny { ("Y", pos, f.width as usize) } else { ("C", pos - ny, f.width.div_ceil(2) as usize) };
                panic!("{name}: picture {i} (packet {}) differs, first at {plane} x={} y={}", f.tag, at % wd, at / wd);
            }
        }
        assert_eq!(got.len(), n, "{name}: picture count");
    }
    if ran == 0 {
        eprintln!("SKIPPED (AVS1 FATE oracle): no samples");
    }
}
