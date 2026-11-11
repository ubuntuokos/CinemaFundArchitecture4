//! Huffyuv streams made by ffmpeg (an external encoder, used only as a fixture generator and
//! oracle), in AVI, decoded by us and compared sample for sample with ffmpeg's own decode.
//! Skipped when ffmpeg is not installed.

use cfa4_huffyuv::{Decoder, Format, Picture};
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
    fx!("yuv_left", "yuv422p", "yuv422p", 160, 120, 3, ["-pred", "left"]),
    fx!("yuv_plane", "yuv422p", "yuv422p", 160, 120, 3, ["-pred", "plane"]),
    fx!("yuv_median", "yuv422p", "yuv422p", 160, 120, 3, ["-pred", "median"]),
    fx!("yuv_median_odd", "yuv422p", "yuv422p", 72, 31, 2, ["-pred", "median"]),
    fx!("yuv_plane_il", "yuv422p", "yuv422p", 128, 576, 2, ["-pred", "plane", "-flags", "+ilme"]),
    fx!("yuv_median_il", "yuv422p", "yuv422p", 96, 320, 2, ["-pred", "median", "-flags", "+ilme"]),
    fx!("yuv_median_tall", "yuv422p", "yuv422p", 64, 576, 2, ["-pred", "median"]),
    fx!("rgb_left", "rgb24", "gbrp", 160, 120, 3, ["-pred", "left"]),
    fx!("rgb_plane", "rgb24", "gbrp", 67, 45, 3, ["-pred", "plane"]),
    fx!("rgb_plane_il", "rgb24", "gbrp", 64, 320, 2, ["-pred", "plane", "-flags", "+ilme"]),
    fx!("rgba_left", "bgra", "bgra", 96, 64, 3, ["-pred", "left"]),
    fx!("rgba_plane", "bgra", "bgra", 51, 37, 3, ["-pred", "plane"]),
];

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/huffyuv");
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
    let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src, "-frames:v", &n, "-pix_fmt", f.pix_fmt, "-c:v", "huffyuv"];
    args.extend_from_slice(f.args);
    let avis = avi.to_string_lossy().to_string();
    args.push(&avis);
    ffmpeg(&args);
    ffmpeg(&["-i", &avis, "-f", "rawvideo", "-pix_fmt", f.raw_fmt, &raw.to_string_lossy()]);
    (std::fs::read(&avi).unwrap(), std::fs::read(&raw).unwrap())
}

/// Our picture in ffmpeg's layout: planar Y U V and G B R, interleaved B G R A.
fn to_raw(p: &Picture) -> Vec<u8> {
    match p.format {
        Format::Yuv422 => p.planes.concat(),
        Format::Rgb => [&p.planes[1], &p.planes[2], &p.planes[0]].iter().flat_map(|pl| pl.iter().copied()).collect(),
        // ffmpeg's bgra to gbrap conversion does not keep alpha exactly: compare interleaved
        Format::Rgba => (0..p.planes[0].len()).flat_map(|i| [p.planes[2][i], p.planes[1][i], p.planes[0][i], p.planes[3][i]]).collect(),
    }
}

fn open(avi: &[u8]) -> (Decoder, Vec<Vec<u8>>) {
    let file = cfa4_avi::open(avi).unwrap();
    let (si, s) = file.streams.iter().enumerate().find(|(_, s)| s.video.is_some()).unwrap();
    let bih = s.video.as_ref().unwrap();
    assert_eq!(&bih.compression, b"HFYU");
    let dec = Decoder::new(bih.width.unsigned_abs(), bih.height.unsigned_abs(), bih.bit_count, &bih.extra).unwrap();
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
            let Ok(mut dec) = Decoder::new(bih.width.unsigned_abs(), bih.height.unsigned_abs(), bih.bit_count, &extra) else { continue };
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
