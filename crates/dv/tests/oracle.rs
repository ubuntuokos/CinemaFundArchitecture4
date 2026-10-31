//! DV against ffmpeg: ffmpeg encodes lavfi test patterns into raw DIF streams, ffmpeg's decode
//! of the same file is the reference. The DCT is not bit-exact by definition (our float IDCT
//! and ffmpeg's integer one differ by rounding), so frames are compared within a tolerance;
//! frame count, size and chroma format are exact.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_dv::{Aspect, ChromaFormat, Decoder, Format, Picture};

struct Spec {
    name: &'static str,
    lavfi: &'static str,
    frames: u32,
    enc: &'static [&'static str],
    pix_fmt: &'static str,
    format: Format,
    chroma: ChromaFormat,
    aspect: Aspect,
}

const SPECS: &[Spec] = &[
    // 525/60 consumer DV, 4:1:1
    Spec {
        name: "dv25_525_411",
        lavfi: "testsrc2=size=720x480:rate=30000/1001",
        frames: 6,
        enc: &["-pix_fmt", "yuv411p"],
        pix_fmt: "yuv411p",
        format: Format::Dvcpro25,
        chroma: ChromaFormat::Yuv411,
        aspect: Aspect::Ratio4x3,
    },
    // 625/50 consumer DV, 4:2:0, 16:9
    Spec {
        name: "dv25_625_420_wide",
        lavfi: "testsrc2=size=720x576:rate=25",
        frames: 6,
        enc: &["-pix_fmt", "yuv420p", "-aspect", "16:9"],
        pix_fmt: "yuv420p",
        format: Format::Dv25,
        chroma: ChromaFormat::Yuv420,
        aspect: Aspect::Ratio16x9,
    },
    // 625/50 DVCPRO 25, 4:1:1
    Spec {
        name: "dvcpro25_625_411",
        lavfi: "testsrc2=size=720x576:rate=25",
        frames: 6,
        enc: &["-pix_fmt", "yuv411p"],
        pix_fmt: "yuv411p",
        format: Format::Dvcpro25,
        chroma: ChromaFormat::Yuv411,
        aspect: Aspect::Ratio4x3,
    },
    // interlaced content with 2-4-8 DCT, detailed (bit spilling in passes 2 and 3)
    Spec {
        name: "dv25_625_248",
        lavfi: "mandelbrot=size=720x576:rate=50,tinterlace=mode=interleave_top",
        frames: 6,
        enc: &["-pix_fmt", "yuv420p", "-flags", "+ildct"],
        pix_fmt: "yuv420p",
        format: Format::Dv25,
        chroma: ChromaFormat::Yuv420,
        aspect: Aspect::Ratio4x3,
    },
    Spec {
        name: "dv25_525_248",
        lavfi: "testsrc2=size=720x480:rate=60000/1001,tinterlace=mode=interleave_bottom",
        frames: 6,
        enc: &["-pix_fmt", "yuv411p", "-flags", "+ildct"],
        pix_fmt: "yuv411p",
        format: Format::Dvcpro25,
        chroma: ChromaFormat::Yuv411,
        aspect: Aspect::Ratio4x3,
    },
    // noise: coarse quantisation, many escapes
    Spec {
        name: "dv25_625_noise",
        lavfi: "nullsrc=size=720x576:rate=25,geq=random(1)*255:128+40*sin(X/7):128+40*cos(Y/5)",
        frames: 3,
        enc: &["-pix_fmt", "yuv420p"],
        pix_fmt: "yuv420p",
        format: Format::Dv25,
        chroma: ChromaFormat::Yuv420,
        aspect: Aspect::Ratio4x3,
    },
];

/// Per frame: max |Δ| and PSNR against ffmpeg.
const TOL_MAX: u8 = 3;
const MIN_PSNR: f64 = 48.0;

fn fixture_dir() -> PathBuf {
    cfa4_testkit::fixtures_dir("dv")
}

fn path(spec: &Spec) -> PathBuf {
    fixture_dir().join(format!("{}.dv", spec.name))
}

fn make(ff: &Path, spec: &Spec) -> Option<PathBuf> {
    let out = path(spec);
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let frames = spec.frames.to_string();
        let st = Command::new(ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", spec.lavfi, "-frames:v", &frames, "-c:v", "dvvideo"])
            .args(spec.enc)
            .args(["-f", "dv"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(s) if s.success())
    })
}

fn ffmpeg_frames(ff: &Path, file: &Path, pix_fmt: &str) -> Vec<u8> {
    let o = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-f", "rawvideo", "-pix_fmt", pix_fmt, "-"])
        .stdin(Stdio::null())
        .output()
        .expect("run ffmpeg");
    assert!(o.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}

#[derive(Default, Clone, Copy)]
struct Stats {
    max: u8,
    sse: f64,
    n: u64,
}

impl Stats {
    fn add(&mut self, a: &[u8], b: &[u8]) {
        assert_eq!(a.len(), b.len(), "plane size");
        for (x, y) in a.iter().zip(b) {
            let d = x.abs_diff(*y);
            self.max = self.max.max(d);
            self.sse += f64::from(d) * f64::from(d);
        }
        self.n += a.len() as u64;
    }
    fn psnr(&self) -> f64 {
        if self.sse == 0.0 { f64::INFINITY } else { 10.0 * (255.0 * 255.0 * self.n as f64 / self.sse).log10() }
    }
}

fn compare(p: &Picture, raw: &[u8]) -> Stats {
    let (w, h) = (p.width as usize, p.height as usize);
    let c = (p.chroma_width() * p.chroma_height()) as usize;
    assert_eq!(raw.len(), w * h + 2 * c, "frame size");
    let mut s = Stats::default();
    s.add(&p.y, &raw[..w * h]);
    s.add(&p.cb, &raw[w * h..w * h + c]);
    s.add(&p.cr, &raw[w * h + c..]);
    s
}

fn check(spec: &Spec) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let f = make(&ff, spec).unwrap_or_else(|| panic!("could not generate {}", spec.name));
    let data = std::fs::read(&f).unwrap();
    let ranges = cfa4_dv::frames(&data).unwrap();
    assert_eq!(ranges.len(), spec.frames as usize, "{}: frame count", spec.name);
    let raw = ffmpeg_frames(&ff, &f, spec.pix_fmt);
    assert_eq!(raw.len() % ranges.len(), 0);
    let fsize = raw.len() / ranges.len();
    let mut dec = Decoder::new();
    let (mut worst_max, mut worst_psnr) = (0u8, f64::INFINITY);
    for (i, r) in ranges.into_iter().enumerate() {
        let pic = dec.decode(&data[r]).unwrap();
        assert_eq!(pic.profile.format, spec.format, "{}", spec.name);
        assert_eq!(pic.profile.chroma, spec.chroma, "{}", spec.name);
        assert_eq!(pic.profile.aspect, spec.aspect, "{}", spec.name);
        assert_eq!((pic.errors, pic.concealed), (0, 0), "{} frame {i}: coding errors", spec.name);
        let s = compare(&pic, &raw[i * fsize..(i + 1) * fsize]);
        worst_max = worst_max.max(s.max);
        worst_psnr = worst_psnr.min(s.psnr());
        assert!(s.max <= TOL_MAX && s.psnr() >= MIN_PSNR, "{} frame {i}: max {} psnr {:.2}", spec.name, s.max, s.psnr());
    }
    println!("{}: max diff {worst_max}, min PSNR {worst_psnr:.2} dB", spec.name);
}

#[test]
fn dv25_525_411() {
    check(&SPECS[0]);
}

#[test]
fn dv25_625_420_wide() {
    check(&SPECS[1]);
}

#[test]
fn dvcpro25_625_411() {
    check(&SPECS[2]);
}

#[test]
fn dv25_625_248() {
    check(&SPECS[3]);
}

#[test]
fn dv25_525_248() {
    check(&SPECS[4]);
}

#[test]
fn dv25_625_noise() {
    check(&SPECS[5]);
}

/// Damaged input: truncation, bit flips and corrupted DIF IDs never panic, and the frame
/// still comes out (with concealment) when the header survives.
#[test]
fn corrupt_frames_do_not_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let f = make(&ff, &SPECS[3]).unwrap_or_else(|| panic!("could not generate {}", SPECS[3].name));
    let data = std::fs::read(&f).unwrap();
    let r = cfa4_dv::frames(&data).unwrap();
    let frame = &data[r[0].clone()];
    let mut dec = Decoder::new();
    let mut x = 0x9e37_79b9u32;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for _ in 0..40 {
        let mut f = frame.to_vec();
        for _ in 0..200 {
            let i = (rnd() as usize) % f.len();
            // keep the header block so the frame is still recognised
            if i >= 80 {
                f[i] ^= 1 << (rnd() % 8);
            }
        }
        let pic = dec.decode(&f).unwrap();
        assert_eq!(pic.y.len(), 720 * 576);
    }
    // a whole DIF sequence overwritten: its macroblocks are concealed
    let mut f = frame.to_vec();
    f[12_000..24_000].fill(0);
    let pic = dec.decode(&f).unwrap();
    assert_eq!(pic.concealed, 135);
    // truncation
    for len in [0, 79, 80, 480, 12_000, frame.len() - 1] {
        assert!(dec.decode(&frame[..len]).is_err());
    }
}

/// Generate all fixtures (run by `cargo xtask fixtures`).
#[test]
#[ignore]
fn generate_fixtures() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let out = path(s);
        cfa4_testkit::fixtures::generate_and_report(s.name, std::slice::from_ref(&out), || make(&ff, s));
    }
}
