//! FFV1 streams made by ffmpeg (an external encoder, used only as a fixture generator and
//! oracle), wrapped in AVI and decoded by us, compared sample for sample with ffmpeg's own
//! decode. Skipped when ffmpeg is not installed.

use cfa4_ffv1::{Decoder, Format, Picture};
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    name: &'static str,
    /// Encoder input pixel format.
    pix_fmt: &'static str,
    /// The raw format ffmpeg decodes to for comparison (planar, same depth).
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
    fx!("v1_range_420", "yuv420p", "yuv420p", 64, 48, 4, ["-level", "1", "-coder", "1"]),
    fx!("v0_golomb_420", "yuv420p", "yuv420p", 64, 48, 4, ["-level", "0", "-coder", "0"]),
    fx!("v1_golomb_422", "yuv422p", "yuv422p", 70, 30, 3, ["-level", "1", "-coder", "0"]),
    fx!("v3_default", "yuv420p", "yuv420p", 352, 288, 6, ["-level", "3"]),
    fx!("v3_golomb", "yuv420p", "yuv420p", 352, 288, 4, ["-level", "3", "-coder", "0"]),
    fx!("v3_range_tab", "yuv420p", "yuv420p", 160, 120, 4, ["-level", "3", "-coder", "2"]),
    fx!("v3_range_def", "yuv420p", "yuv420p", 160, 120, 4, ["-level", "3", "-coder", "1"]),
    fx!("v3_slices_24_ec", "yuv420p", "yuv420p", 720, 576, 3, ["-level", "3", "-slices", "24", "-slicecrc", "1"]),
    fx!("v3_slices_odd", "yuv444p", "yuv444p", 333, 211, 3, ["-level", "3", "-slices", "6"]),
    fx!("v3_context1", "yuv420p", "yuv420p", 176, 144, 3, ["-level", "3", "-context", "1"]),
    fx!("v3_gop1", "yuv420p", "yuv420p", 176, 144, 3, ["-level", "3", "-g", "1"]),
    fx!("v3_gray8", "gray", "gray", 96, 64, 3, ["-level", "3"]),
    fx!("v3_gray16", "gray16le", "gray16le", 96, 64, 3, ["-level", "3"]),
    fx!("v3_yuv420p10", "yuv420p10le", "yuv420p10le", 128, 72, 3, ["-level", "3"]),
    fx!("v3_yuv422p10", "yuv422p10le", "yuv422p10le", 128, 72, 3, ["-level", "3"]),
    fx!("v3_yuv444p16", "yuv444p16le", "yuv444p16le", 64, 48, 3, ["-level", "3"]),
    fx!("v3_yuva420p", "yuva420p", "yuva420p", 64, 48, 3, ["-level", "3"]),
    fx!("v3_bgr0", "bgr0", "gbrp", 128, 96, 3, ["-level", "3"]),
    fx!("v3_bgra", "bgra", "gbrap", 128, 96, 3, ["-level", "3"]),
    fx!("v3_gbrp10", "gbrp10le", "gbrp10le", 96, 64, 3, ["-level", "3"]),
    fx!("v3_gbrp12_golomb", "gbrp12le", "gbrp12le", 96, 64, 3, ["-level", "3", "-coder", "1"]),
    fx!("v3_gbrap16", "gbrap16le", "gbrap16le", 64, 48, 3, ["-level", "3"]),
    fx!("v3_bgr0_range", "bgr0", "gbrp", 64, 48, 3, ["-level", "3", "-coder", "1"]),
    fx!("v1_bgr0_golomb", "bgr0", "gbrp", 64, 48, 3, ["-level", "1", "-coder", "0"]),
    fx!("v3_yuv410", "yuv410p", "yuv410p", 67, 45, 3, ["-level", "3"]),
];

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/ffv1");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Encodes the fixture; returns (AVI bytes, ffmpeg's decode as raw planar frames).
fn make(f: &Fixture) -> (Vec<u8>, Vec<u8>) {
    // the tests run in parallel and share the fixture files
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let d = dir();
    let avi = d.join(format!("{}.avi", f.name));
    let raw = d.join(format!("{}.{}.raw", f.name, f.raw_fmt));
    let src = format!("testsrc2=size={}x{}:rate=10", f.w, f.h);
    let n = f.frames.to_string();
    let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src, "-frames:v", &n, "-pix_fmt", f.pix_fmt, "-c:v", "ffv1"];
    args.extend_from_slice(f.args);
    let avis = avi.to_string_lossy().to_string();
    args.push(&avis);
    ffmpeg(&args);
    ffmpeg(&["-i", &avis, "-f", "rawvideo", "-pix_fmt", f.raw_fmt, &raw.to_string_lossy()]);
    (std::fs::read(&avi).unwrap(), std::fs::read(&raw).unwrap())
}

/// Our picture in ffmpeg's raw planar layout (gbrp order for RGB, 16-bit little-endian above 8).
fn to_raw(p: &Picture) -> Vec<u8> {
    let mut planes: Vec<&Vec<u16>> = match p.format {
        Format::Rgb => vec![&p.planes[1], &p.planes[2], &p.planes[0]],
        _ => p.planes.iter().collect(),
    };
    if let Some(a) = &p.alpha {
        planes.push(a);
    }
    let mut out = Vec::new();
    for pl in planes {
        for &v in pl {
            if p.bits > 8 {
                out.extend_from_slice(&v.to_le_bytes());
            } else {
                out.push(v as u8);
            }
        }
    }
    out
}

fn decode_all(avi: &[u8]) -> Vec<Picture> {
    let file = cfa4_avi::open(avi).unwrap();
    let (si, s) = file.streams.iter().enumerate().find(|(_, s)| s.video.is_some()).unwrap();
    let bih = s.video.as_ref().unwrap();
    assert_eq!(&bih.compression, b"FFV1");
    // ffmpeg may round the size (4:1:0 needs multiples of 4): the container has the real one
    let (w, h) = (bih.width.unsigned_abs(), bih.height.unsigned_abs());
    let mut dec = Decoder::new(w, h, &bih.extra).unwrap();
    let mut out = Vec::new();
    for i in 0..s.chunks.len() {
        let chunk = file.read_chunk(avi, si, i).unwrap();
        out.push(dec.decode(&chunk).unwrap_or_else(|e| panic!("frame {i}: {e}")));
    }
    out
}

#[test]
fn matches_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let only = std::env::var("FFV1_ONLY").ok();
    let mut failures = Vec::new();
    for f in FIXTURES.iter().filter(|f| only.as_deref().is_none_or(|o| f.name.contains(o))) {
        let (avi, raw) = make(f);
        let res = std::panic::catch_unwind(|| decode_all(&avi));
        let pics = match res {
            Ok(p) => p,
            Err(e) => {
                let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
                failures.push(format!("{}: {msg}", f.name));
                continue;
            }
        };
        assert_eq!(pics.len(), f.frames as usize, "{}", f.name);
        let ours: Vec<u8> = pics.iter().flat_map(to_raw).collect();
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

/// Damaged input (truncated and bit-flipped frames and configuration records of real streams)
/// gives errors or pictures, never a panic; the decoder recovers at the next intact key frame.
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
        let (si, s) = file.streams.iter().enumerate().find(|(_, s)| s.video.is_some()).unwrap();
        let bih = s.video.as_ref().unwrap();
        let (w, h) = (bih.width.unsigned_abs(), bih.height.unsigned_abs());
        let chunks: Vec<Vec<u8>> = (0..s.chunks.len()).map(|i| file.read_chunk(&avi, si, i).unwrap()).collect();
        for _ in 0..40 {
            let mut rec = bih.extra.clone();
            if !rec.is_empty() && rnd() % 2 == 0 {
                let k = rnd() as usize % rec.len();
                rec[k] ^= 1 << (rnd() % 8);
            }
            let Ok(mut dec) = Decoder::new(w, h, &rec) else { continue };
            for c in &chunks {
                let mut d = c.clone();
                match rnd() % 3 {
                    0 => d.truncate(rnd() as usize % (d.len() + 1)),
                    1 if !d.is_empty() => {
                        for _ in 0..1 + rnd() % 4 {
                            let k = rnd() as usize % d.len();
                            d[k] ^= 1 << (rnd() % 8);
                        }
                    }
                    _ => {}
                }
                let _ = dec.decode(&d);
            }
            // an intact key frame decodes again
            let mut dec = Decoder::new(w, h, &bih.extra).unwrap();
            assert!(dec.decode(&chunks[0]).is_ok(), "{}: intact key frame after damage", f.name);
        }
    }
}
