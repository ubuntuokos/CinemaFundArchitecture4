//! JPEG 2000 against ffmpeg: ffmpeg's `jpeg2000` encoder (and `libopenjpeg` for the DCI cinema
//! profile) write codestreams from generated test patterns. Reversible 5/3 streams must decode
//! bit-exact to the encoder's input; irreversible 9/7 streams to within ±1 of ffmpeg's decode
//! (float rounding), with the same PSNR against the input.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Spec {
    name: &'static str,
    /// Frame size before the odd crop (3 columns, 5 rows off).
    size: &'static str,
    pix_fmt: &'static str,
    /// Encoder and its options.
    enc: &'static [&'static str],
    /// Bit-exact to the input (reversible, all layers).
    lossless: bool,
    /// File extension (j2k codestream or jp2 file).
    ext: &'static str,
}

const SPECS: &[Spec] = &[
    Spec { name: "gray_53", size: "96x64", pix_fmt: "gray", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec { name: "rgb_53_rct", size: "320x240", pix_fmt: "rgb24", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec { name: "yuv420_53", size: "320x240", pix_fmt: "yuv420p", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec { name: "gray16_53", size: "200x150", pix_fmt: "gray16le", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec {
        name: "rgb_53_tiles",
        size: "640x480",
        pix_fmt: "rgb24",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-tile_width", "64", "-tile_height", "48"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "lrcp",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "lrcp", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "rlcp",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "rlcp", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "rpcl",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "rpcl", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "pcrl",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "pcrl", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "cprl",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "cprl", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "sop_eph",
        size: "320x240",
        pix_fmt: "rgb24",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-sop", "1", "-eph", "1"],
        lossless: true,
        ext: "j2k",
    },
    Spec { name: "jp2", size: "200x100", pix_fmt: "rgb24", enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-format", "jp2"], lossless: true, ext: "jp2" },
    Spec { name: "yuv420_97", size: "320x240", pix_fmt: "yuv420p", enc: &["-c:v", "jpeg2000", "-pred", "dwt97int"], lossless: false, ext: "j2k" },
    Spec { name: "rgb_97_ict", size: "320x240", pix_fmt: "rgb24", enc: &["-c:v", "jpeg2000", "-pred", "dwt97int"], lossless: false, ext: "j2k" },
    Spec {
        name: "yuv420_97_q10",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt97int", "-q:v", "10"],
        lossless: false,
        ext: "j2k",
    },
    // quality layers, lossy rates: the full stream against ffmpeg
    Spec {
        name: "layers",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-layer_rates", "100,10,1"],
        lossless: false,
        ext: "j2k",
    },
    // DCI 2K (SMPTE 429-4): 12-bit X'Y'Z', 9/7, CPRL, a tile-part per component
    Spec {
        name: "dci_2k",
        size: "2051x1085",
        pix_fmt: "xyz12le",
        enc: &["-c:v", "libopenjpeg", "-profile:v", "cinema2k", "-cinema_mode", "2k_24"],
        lossless: false,
        ext: "j2k",
    },
];

fn input(s: &Spec) -> String {
    format!("testsrc2=s={},crop=iw-3:ih-5:1:2", s.size)
}

fn make(ff: &Path, s: &Spec) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("j2k").join(format!("{}.{}", s.name, s.ext));
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", &input(s), "-frames:v", "1", "-pix_fmt", s.pix_fmt])
            .args(s.enc)
            .args(["-f", "image2"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(x) if x.success())
    })
}

/// Samples of each component from ffmpeg raw output in `pix_fmt` (planar or packed).
fn raw(ff: &Path, args: &[&str], pix_fmt: &str, comps: &[(usize, u8)]) -> Vec<Vec<i32>> {
    let o = Command::new(ff).args(["-v", "error"]).args(args).args(["-pix_fmt", pix_fmt, "-f", "rawvideo", "-"]).output().unwrap();
    let d = o.stdout;
    let n = comps.len();
    match pix_fmt {
        "rgb24" => (0..n).map(|c| (0..comps[c].0).map(|i| i32::from(d[i * n + c])).collect()).collect(),
        "xyz12le" => {
            (0..n).map(|c| (0..comps[c].0).map(|i| i32::from(u16::from_le_bytes([d[2 * (i * n + c)], d[2 * (i * n + c) + 1]]) >> 4)).collect()).collect()
        }
        "gray16le" => vec![(0..comps[0].0).map(|i| i32::from(u16::from_le_bytes([d[2 * i], d[2 * i + 1]]))).collect()],
        _ => {
            let mut off = 0;
            comps
                .iter()
                .map(|&(len, _)| {
                    let v = d[off..off + len].iter().map(|&b| i32::from(b)).collect();
                    off += len;
                    v
                })
                .collect()
        }
    }
}

#[test]
fn ffmpeg_encoded_codestreams_decode() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        let img = cfa4_j2k::decode(&data).unwrap_or_else(|e| panic!("{}: {e}", s.name));
        let comps: Vec<(usize, u8)> = img.components.iter().map(|p| (p.data.len(), p.precision)).collect();
        let src = raw(&ff, &["-f", "lavfi", "-i", &input(s), "-frames:v", "1"], s.pix_fmt, &comps);
        let theirs = raw(&ff, &["-i", f.to_str().unwrap()], s.pix_fmt, &comps);
        let mut report = String::new();
        for (c, p) in img.components.iter().enumerate() {
            let peak = f64::from((1u32 << p.precision) - 1);
            let psnr = |want: &[i32]| {
                let se: f64 = p.data.iter().zip(want).map(|(&a, &b)| f64::from(a - b).powi(2)).sum();
                10.0 * (peak * peak / (se / p.data.len() as f64).max(1e-12)).log10()
            };
            let worst = p.data.iter().zip(&theirs[c]).map(|(&a, &b)| (a - b).abs()).max().unwrap_or(0);
            let (ps, pf) = (psnr(&src[c]), psnr(&theirs[c]));
            report += &format!(" c{c} {}x{}: {ps:.1} dB vs input, ±{worst} vs ffmpeg;", p.width, p.height);
            if s.lossless {
                assert!(p.data == src[c], "{}: component {c} is not lossless", s.name);
            } else {
                assert!(worst <= 1, "{}: component {c} differs from ffmpeg by {worst}", s.name);
                assert!(pf > 80.0, "{}: component {c} only {pf:.1} dB against ffmpeg", s.name);
            }
        }
        println!("{}:{report}", s.name);
    }
}

/// Damaged variants per stream.
const FUZZ: u32 = 60;

/// Damaged codestreams (bit flips, truncation, scrambled bytes) give errors or a picture, never a
/// panic.
#[test]
fn damaged_codestreams_never_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut x = 0x00C0_FFEEu32;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for s in SPECS.iter().filter(|s| s.size != "2051x1085") {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        for k in 0..FUZZ {
            let mut m = data.clone();
            match k % 3 {
                0 => {
                    for _ in 0..1 + rnd() % 16 {
                        let bit = rnd() as usize % (m.len() * 8);
                        m[bit / 8] ^= 0x80 >> (bit % 8);
                    }
                }
                1 => m.truncate(rnd() as usize % m.len()),
                _ => {
                    let at = rnd() as usize % m.len();
                    for b in m.iter_mut().skip(at).take(1 + rnd() as usize % 32) {
                        *b = rnd() as u8;
                    }
                }
            }
            let _ = cfa4_j2k::decode(&m);
        }
    }
}

#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("j2k fixtures") else { return };
    for s in SPECS {
        let out = cfa4_testkit::fixtures_dir("j2k").join(format!("{}.{}", s.name, s.ext));
        cfa4_testkit::fixtures::generate_and_report(&format!("j2k/{}", s.name), &[out], || make(&ff, s));
    }
}
