//! VC-2 streams made by ffmpeg's `vc2` encoder (an external tool, used only as a fixture
//! generator and oracle), decoded here and by ffmpeg's own decoder: every sample must be equal.
//! Covers every wavelet the encoder offers, 1 to 5 transform levels, the default and custom
//! quantisation matrices, slice sizes, 4:2:0 / 4:2:2 / 4:4:4 at 8, 10 and 12 bits, odd frame
//! sizes, and field coding (ffmpeg's decoder rejects it, so each field is checked as a
//! progressive picture). Skipped when ffmpeg is not installed.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use cfa4_dirac::{ChromaFormat, Decoder};

struct Case {
    name: &'static str,
    pix_fmt: &'static str,
    size: &'static str,
    args: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case { name: "dd97_420", pix_fmt: "yuv420p", size: "352x288", args: &[] },
    Case { name: "legall_422", pix_fmt: "yuv422p", size: "320x240", args: &["-wavelet_type", "5_3"] },
    Case { name: "haar_444", pix_fmt: "yuv444p", size: "176x144", args: &["-wavelet_type", "haar"] },
    Case { name: "haar_noshift_420", pix_fmt: "yuv420p", size: "176x144", args: &["-wavelet_type", "haar_noshift", "-wavelet_depth", "2"] },
    Case { name: "depth1_qm_color", pix_fmt: "yuv420p", size: "160x96", args: &["-wavelet_depth", "1", "-qm", "color"] },
    Case { name: "depth5_qm_flat", pix_fmt: "yuv422p", size: "256x128", args: &["-wavelet_depth", "5", "-qm", "flat"] },
    Case { name: "slices_64x8", pix_fmt: "yuv420p", size: "352x288", args: &["-slice_width", "64", "-slice_height", "8"] },
    Case { name: "odd_size", pix_fmt: "yuv420p", size: "334x206", args: &[] },
    Case { name: "low_rate", pix_fmt: "yuv420p", size: "352x288", args: &["-b:v", "600k"] },
    Case { name: "p10_422", pix_fmt: "yuv422p10le", size: "320x180", args: &[] },
    Case { name: "p12_444", pix_fmt: "yuv444p12le", size: "160x90", args: &["-wavelet_type", "5_3"] },
    Case { name: "p10_420_haar", pix_fmt: "yuv420p10le", size: "192x108", args: &["-wavelet_type", "haar"] },
];

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/dirac");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ffmpeg(args: &[&str]) -> bool {
    Command::new("ffmpeg").args(["-y", "-v", "error"]).args(args).status().is_ok_and(|s| s.success())
}

#[test]
fn vc2_streams_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("SKIPPED (VC-2 oracle): ffmpeg is not installed");
        return;
    }
    let d = dir();
    let mut failed = Vec::new();
    for c in CASES {
        let vc2 = d.join(format!("{}.vc2", c.name));
        let src = format!("testsrc2=size={}:rate=25:duration=0.12", c.size);
        let mut args = vec!["-f", "lavfi", "-i", &src, "-pix_fmt", c.pix_fmt, "-c:v", "vc2"];
        args.extend_from_slice(c.args);
        let out = vc2.to_str().unwrap().to_string();
        args.extend_from_slice(&["-f", "dirac", &out]);
        assert!(ffmpeg(&args), "{}: ffmpeg could not encode", c.name);
        let raw = d.join(format!("{}.yuv", c.name));
        assert!(ffmpeg(&["-i", &out, "-f", "rawvideo", "-pix_fmt", c.pix_fmt, raw.to_str().unwrap()]), "{}: ffmpeg could not decode", c.name);
        let want = std::fs::read(&raw).unwrap();

        let data = std::fs::read(&vc2).unwrap();
        let pictures = Decoder::new().decode(&data).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let mut got = Vec::new();
        for p in &pictures {
            let (w, h) = c.size.split_once('x').map(|(w, h)| (w.parse::<u32>().unwrap(), h.parse::<u32>().unwrap())).unwrap();
            assert_eq!((p.width, p.height), (w, h), "{}", c.name);
            let chroma = if c.pix_fmt.starts_with("yuv420") {
                ChromaFormat::C420
            } else if c.pix_fmt.starts_with("yuv422") {
                ChromaFormat::C422
            } else {
                ChromaFormat::C444
            };
            assert_eq!(p.chroma, chroma, "{}", c.name);
            for plane in &p.planes {
                if p.luma_depth > 8 {
                    got.extend(plane.iter().flat_map(|v| v.to_le_bytes()));
                } else {
                    got.extend(plane.iter().map(|&v| v as u8));
                }
            }
        }
        assert_eq!(pictures.len(), 3, "{}: frames", c.name);
        if got.len() != want.len() {
            failed.push(format!("{}: {} bytes, ffmpeg {}", c.name, got.len(), want.len()));
        } else if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            failed.push(format!("{}: byte {i} differs (ours {}, ffmpeg {})", c.name, got[i], want[i]));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The data units of a stream: (parse code, bytes of the whole unit).
fn units(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 13 <= data.len() {
        let next = u32::from_be_bytes(data[at + 5..at + 9].try_into().unwrap()) as usize;
        let end = if next == 0 { data.len().min(at + 13) } else { at + next };
        out.push((data[at + 4], &data[at..end]));
        if next == 0 {
            break;
        }
        at += next;
    }
    out
}

/// ffmpeg's decoder doesn't do field coding (`picture_coding_mode` 1), so the oracle decodes
/// each field as a progressive picture: the sequence headers of the field-coded stream are
/// replaced with that of a progressive stream of field size (same coding parameters), ffmpeg
/// decodes the pictures, and the fields are woven together and compared with our frames.
///
/// ffmpeg's encoder codes the top field first but signals base video format 0, whose
/// `top_field_first` is false, without overriding it; we follow the stream (§11.4.5: the field
/// with the even picture number is the earlier one, and the bottom one unless `top_field_first`).
#[test]
fn field_coding_matches_ffmpeg_fields() {
    if !have_ffmpeg() {
        eprintln!("SKIPPED (VC-2 fields): ffmpeg is not installed");
        return;
    }
    let d = dir();
    {
        let order = "tt";
        let ild = d.join(format!("fields_{order}.vc2"));
        let field = d.join(format!("fields_{order}_ref.vc2"));
        let src = "testsrc2=size=720x576:rate=25:duration=0.12";
        assert!(ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            src,
            "-pix_fmt",
            "yuv422p",
            "-field_order",
            order,
            "-flags",
            "+ildct",
            "-c:v",
            "vc2",
            "-f",
            "dirac",
            ild.to_str().unwrap()
        ]));
        assert!(ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=720x288:rate=25:duration=0.04",
            "-pix_fmt",
            "yuv422p",
            "-c:v",
            "vc2",
            "-f",
            "dirac",
            field.to_str().unwrap()
        ]));
        let data = std::fs::read(&ild).unwrap();
        let reference = std::fs::read(&field).unwrap();
        let progressive_header = units(&reference).into_iter().find(|u| u.0 == 0).unwrap().1.to_vec();
        let mut rewritten = Vec::new();
        let mut previous = 0u32;
        for (code, bytes) in units(&data) {
            let start = rewritten.len();
            rewritten.extend_from_slice(if code == 0 { &progressive_header } else { bytes });
            // keep the previous parse offsets consistent with the new header size
            rewritten[start + 9..start + 13].copy_from_slice(&previous.to_be_bytes());
            previous = (rewritten.len() - start) as u32;
        }
        let fields_vc2 = d.join(format!("fields_{order}_progressive.vc2"));
        std::fs::write(&fields_vc2, &rewritten).unwrap();
        let raw = d.join(format!("fields_{order}.yuv"));
        assert!(ffmpeg(&["-i", fields_vc2.to_str().unwrap(), "-f", "rawvideo", "-pix_fmt", "yuv422p", raw.to_str().unwrap()]));
        let want = std::fs::read(&raw).unwrap();
        let field_bytes = 720 * 288 * 2;
        assert_eq!(want.len(), 6 * field_bytes, "{order}: ffmpeg fields");

        let mut dec = Decoder::new();
        let frames = dec.decode(&data).unwrap();
        let seq = dec.sequence().unwrap();
        assert!(seq.fields, "{order}: not field coded");
        let tff = seq.video.top_field_first;
        assert!(!tff, "ffmpeg now signals top field first: drop the note above");
        assert_eq!(frames.len(), 3, "{order}");
        for (i, f) in frames.iter().enumerate() {
            let (first, second) = (&want[2 * i * field_bytes..(2 * i + 1) * field_bytes], &want[(2 * i + 1) * field_bytes..(2 * i + 2) * field_bytes]);
            let (top, bottom) = if tff { (first, second) } else { (second, first) };
            let mut woven = Vec::new();
            let mut offset = 0;
            for w in [720usize, 360, 360] {
                for y in 0..288 {
                    woven.extend_from_slice(&top[offset + y * w..offset + (y + 1) * w]);
                    woven.extend_from_slice(&bottom[offset + y * w..offset + (y + 1) * w]);
                }
                offset += w * 288;
            }
            let ours: Vec<u8> = f.planes.iter().flatten().map(|&v| v as u8).collect();
            assert!(ours == woven, "{order}: frame {i} differs from ffmpeg's fields");
        }
    }
}

#[test]
fn damaged_streams_never_panic() {
    if !have_ffmpeg() {
        eprintln!("SKIPPED (VC-2 damage): ffmpeg is not installed");
        return;
    }
    let path = dir().join("damage.vc2");
    let out = path.to_str().unwrap();
    assert!(ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=25:duration=0.04", "-pix_fmt", "yuv420p", "-c:v", "vc2", "-f", "dirac", out]));
    let data = std::fs::read(&path).unwrap();
    for cut in (0..data.len()).step_by(7) {
        let _ = Decoder::new().decode(&data[..cut]);
    }
    let mut seed = 0x1234_5678u32;
    for _ in 0..400 {
        let mut d = data.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % d.len();
            d[i] ^= 1 << (seed >> 29);
        }
        let _ = Decoder::new().decode(&d);
    }
}
