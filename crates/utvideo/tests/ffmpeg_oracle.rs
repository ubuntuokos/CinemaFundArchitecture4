//! Ut Video streams made by ffmpeg (an external encoder, used only as a fixture generator and
//! oracle), in AVI, decoded by us and compared sample for sample with ffmpeg's own decode.
//! Skipped when ffmpeg is not installed.

use cfa4_utvideo::{Decoder, Format, Picture};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

struct Fixture {
    name: &'static str,
    /// Encoder input pixel format.
    pix_fmt: &'static str,
    /// The format ffmpeg decodes to for comparison.
    raw_fmt: &'static str,
    w: u32,
    h: u32,
    frames: u32,
    args: &'static [&'static str],
}

macro_rules! fx {
    ($name:expr, $pf:expr, $raw:expr, $w:expr, $h:expr, $n:expr, [$($a:expr),* $(,)?]) => {
        Fixture { name: $name, pix_fmt: $pf, raw_fmt: $raw, w: $w, h: $h, frames: $n, args: &[$($a),*] }
    };
}

const FIXTURES: &[Fixture] = &[
    fx!("y0_none", "yuv420p", "yuv420p", 160, 120, 3, ["-pred", "none"]),
    fx!("y0_left", "yuv420p", "yuv420p", 160, 120, 3, ["-pred", "left"]),
    fx!("y0_median", "yuv420p", "yuv420p", 160, 120, 3, ["-pred", "median"]),
    fx!("y0_median_slices", "yuv420p", "yuv420p", 352, 288, 2, ["-pred", "median", "-slices", "7"]),
    fx!("h0_left", "yuv420p", "yuv420p", 66, 38, 2, ["-pred", "left", "-colorspace", "bt709"]),
    fx!("y2_median", "yuv422p", "yuv422p", 98, 37, 3, ["-pred", "median"]),
    fx!("y2_left_slices", "yuv422p", "yuv422p", 160, 120, 2, ["-pred", "left", "-slices", "16"]),
    fx!("y4_none", "yuv444p", "yuv444p", 67, 45, 2, ["-pred", "none"]),
    fx!("y4_median", "yuv444p", "yuv444p", 67, 45, 3, ["-pred", "median", "-slices", "3"]),
    fx!("rg_left", "gbrp", "gbrp", 160, 120, 3, ["-pred", "left"]),
    fx!("rg_median", "gbrp", "gbrp", 51, 37, 3, ["-pred", "median", "-slices", "5"]),
    fx!("ra_median", "gbrap", "gbrap", 96, 64, 3, ["-pred", "median"]),
    fx!("ra_none", "gbrap", "gbrap", 33, 17, 2, ["-pred", "none"]),
    fx!("flat_left", "yuv420p", "yuv420p", 64, 32, 2, ["-pred", "left", "-vf", "drawbox=c=black:t=fill"]),
];

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/utvideo");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Encodes the fixture (noisy test pattern, random alpha); returns (AVI bytes, ffmpeg's decode).
fn make(f: &Fixture) -> (Vec<u8>, Vec<u8>) {
    // the tests run in parallel and share the fixture files
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let d = dir();
    let avi = d.join(format!("{}.avi", f.name));
    let raw = d.join(format!("{}.{}.raw", f.name, f.raw_fmt));
    let src = format!("testsrc2=size={}x{}:rate=10,noise=alls=40:allf=t+u,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='random(1)*255'", f.w, f.h);
    let n = f.frames.to_string();
    let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src, "-frames:v", &n, "-pix_fmt", f.pix_fmt, "-c:v", "utvideo"];
    args.extend_from_slice(f.args);
    let avis = avi.to_string_lossy().to_string();
    args.push(&avis);
    ffmpeg(&args);
    ffmpeg(&["-i", &avis, "-f", "rawvideo", "-pix_fmt", f.raw_fmt, &raw.to_string_lossy()]);
    (std::fs::read(&avi).unwrap(), std::fs::read(&raw).unwrap())
}

/// Our picture in ffmpeg's planar layout (G, B, R, A for RGB).
fn to_raw(p: &Picture) -> Vec<u8> {
    match p.format {
        Format::Yuv { .. } => p.planes.concat(),
        Format::Rgb | Format::Rgba => [1, 2, 0, 3].iter().filter_map(|&k| p.planes.get(k)).flat_map(|pl| pl.iter().copied()).collect(),
    }
}

fn open(avi: &[u8]) -> (Decoder, Vec<Vec<u8>>) {
    let file = cfa4_avi::open(avi).unwrap();
    let (si, s) = file.streams.iter().enumerate().find(|(_, s)| s.video.is_some()).unwrap();
    let bih = s.video.as_ref().unwrap();
    let dec = Decoder::new(bih.compression, bih.width.unsigned_abs(), bih.height.unsigned_abs(), &bih.extra).unwrap();
    let chunks = (0..s.chunks.len()).map(|i| file.read_chunk(avi, si, i).unwrap()).collect();
    (dec, chunks)
}

#[test]
fn matches_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let mut failures = Vec::new();
    for f in FIXTURES {
        let (avi, raw) = make(f);
        let (mut dec, chunks) = open(&avi);
        assert_eq!(chunks.len(), f.frames as usize, "{}", f.name);
        let mut ours = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            match dec.decode(c) {
                Ok(p) => ours.extend(to_raw(&p)),
                Err(e) => failures.push(format!("{} frame {i}: {e}", f.name)),
            }
        }
        if ours.len() != raw.len() {
            failures.push(format!("{}: {} bytes, ffmpeg {}", f.name, ours.len(), raw.len()));
        } else if let Some(i) = ours.iter().zip(&raw).position(|(a, b)| a != b) {
            failures.push(format!("{}: first difference at byte {i} of {}", f.name, raw.len()));
        } else {
            eprintln!("{}: bit-exact ({} frames)", f.name, f.frames);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Damaged input (truncated and bit-flipped frames and headers of real streams) gives errors or
/// pictures, never a panic.
#[test]
fn damaged_streams_never_panic() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let mut seed = 0x9E37_79B9u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for f in FIXTURES.iter().filter(|f| f.w * f.h <= 160 * 120) {
        let (avi, _) = make(f);
        let file = cfa4_avi::open(&avi).unwrap();
        let s = file.streams.iter().find(|s| s.video.is_some()).unwrap();
        let bih = s.video.as_ref().unwrap();
        let (_, chunks) = open(&avi);
        for _ in 0..30 {
            let mut extra = bih.extra.clone();
            if rnd() % 2 == 0 {
                let k = rnd() as usize % extra.len();
                extra[k] ^= 1 << (rnd() % 8);
            }
            let Ok(mut dec) = Decoder::new(bih.compression, bih.width.unsigned_abs(), bih.height.unsigned_abs(), &extra) else { continue };
            for c in &chunks {
                let mut d = c.clone();
                match rnd() % 3 {
                    0 => d.truncate(rnd() as usize % (d.len() + 1)),
                    1 => {
                        for _ in 0..1 + rnd() % 4 {
                            let k = rnd() as usize % d.len();
                            d[k] ^= 1 << (rnd() % 8);
                        }
                    }
                    _ => {}
                }
                let _ = dec.decode(&d);
            }
        }
    }
}
