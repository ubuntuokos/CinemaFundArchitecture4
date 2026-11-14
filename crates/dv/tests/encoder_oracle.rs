//! The DV encoder against ffmpeg: lavfi test patterns (raw, in the encoder's chroma format) are
//! encoded by `cfa4_dv::Encoder` into a raw DIF stream, and ffmpeg must read it as the same
//! format, decode the pictures within ±3 of `cfa4_dv`'s own decode, read the time code and decode
//! the sound bit-exactly. The picture quality against the source is compared with ffmpeg's own
//! `dvvideo` encoder on the same frames: ours may not be more than 1 dB worse.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_dv::{Aspect, Decoder, Encoder, EncoderConfig, EncoderInput, Format, System, Timecode};

struct Spec {
    name: &'static str,
    lavfi: &'static str,
    pix_fmt: &'static str,
    cfg: EncoderConfig,
    /// ffmpeg's `dvvideo` options for the same format (reference quality).
    ff_enc: &'static [&'static str],
}

const FRAMES: usize = 5;

const SPECS: &[Spec] = &[
    Spec {
        name: "enc_dv25_525_411",
        lavfi: "testsrc2=size=720x480:rate=30000/1001",
        pix_fmt: "yuv411p",
        cfg: EncoderConfig { system: System::Lines525, format: Format::Dv25, aspect: Aspect::Ratio4x3 },
        ff_enc: &[],
    },
    Spec {
        name: "enc_dv25_625_420_wide_interlaced",
        lavfi: "mandelbrot=size=720x576:rate=50,tinterlace=mode=interleave_top",
        pix_fmt: "yuv420p",
        cfg: EncoderConfig { system: System::Lines625, format: Format::Dv25, aspect: Aspect::Ratio16x9 },
        ff_enc: &["-flags", "+ildct", "-aspect", "16:9"],
    },
    Spec {
        name: "enc_dvcpro25_625_411",
        lavfi: "testsrc2=size=720x576:rate=25",
        pix_fmt: "yuv411p",
        cfg: EncoderConfig { system: System::Lines625, format: Format::Dvcpro25, aspect: Aspect::Ratio4x3 },
        ff_enc: &[],
    },
];

fn dir() -> PathBuf {
    cfa4_testkit::fixtures_dir("dv")
}

fn ffmpeg(ff: &Path, args: &[&str], input: Option<&[u8]>) -> Vec<u8> {
    let mut c = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run ffmpeg");
    if let (Some(data), Some(mut stdin)) = (input, c.stdin.take()) {
        stdin.write_all(data).unwrap();
    }
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let sse: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    10.0 * (255.0f64 * 255.0 * a.len() as f64 / sse.max(1e-9)).log10()
}

fn check(spec: &Spec) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = spec.cfg.size();
    let (sx, sy) = spec.cfg.chroma().shifts();
    let (ys, cs) = ((w * h) as usize, ((w >> sx) * (h >> sy)) as usize);
    let fsize = ys + 2 * cs;
    let n = FRAMES.to_string();
    let src = ffmpeg(&ff, &["-f", "lavfi", "-i", spec.lavfi, "-frames:v", &n, "-f", "rawvideo", "-pix_fmt", spec.pix_fmt, "-"], None);
    assert_eq!(src.len(), fsize * FRAMES, "{}: source size", spec.name);
    // encode with a time code and a tone
    let ntsc = spec.cfg.system == System::Lines525;
    let mut enc = Encoder::new(spec.cfg).unwrap();
    let (mut dv, mut pcm, mut t) = (Vec::new(), Vec::new(), 0usize);
    for i in 0..FRAMES {
        let f = &src[i * fsize..(i + 1) * fsize];
        let samples = spec.cfg.audio_samples(i as u64);
        let left: Vec<i16> = (t..t + samples).map(|k| ((k as f32 * 0.0573).sin() * 10000.0) as i16).collect();
        let right: Vec<i16> = (t..t + samples).map(|k| ((k as f32 * 0.0211).cos() * 7000.0) as i16).collect();
        t += samples;
        for (l, r) in left.iter().zip(&right) {
            pcm.extend_from_slice(&[*l, *r]);
        }
        let tc = Timecode { hours: 10, minutes: 0, seconds: 0, frames: i as u8, drop_frame: ntsc };
        let input = EncoderInput { y: &f[..ys], cb: &f[ys..ys + cs], cr: &f[ys + cs..], audio: Some([&left, &right]), timecode: Some(tc) };
        dv.extend_from_slice(&enc.encode(&input).unwrap());
    }
    let file = dir().join(format!("{}.dv", spec.name));
    std::fs::create_dir_all(dir()).unwrap();
    std::fs::write(&file, &dv).unwrap();
    let file_s = file.to_str().unwrap();
    // ffmpeg's decode of our stream, against ours
    let theirs = ffmpeg(&ff, &["-i", file_s, "-f", "rawvideo", "-pix_fmt", spec.pix_fmt, "-"], None);
    assert_eq!(theirs.len(), fsize * FRAMES, "{}: ffmpeg frame count / format", spec.name);
    let mut ours = Vec::new();
    let mut dec = Decoder::new();
    for r in cfa4_dv::frames(&dv).unwrap() {
        let p = dec.decode(&dv[r]).unwrap();
        assert_eq!((p.errors, p.concealed), (0, 0), "{}", spec.name);
        assert_eq!((p.profile.format, p.profile.aspect), (spec.cfg.format, spec.cfg.aspect), "{}", spec.name);
        ours.extend_from_slice(&p.y);
        ours.extend_from_slice(&p.cb);
        ours.extend_from_slice(&p.cr);
    }
    let max = ours.iter().zip(&theirs).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
    assert!(max <= 3, "{}: ffmpeg's decode differs from ours by {max}", spec.name);
    // quality against the source, next to ffmpeg's own encoder
    let ref_file = dir().join(format!("{}_ffmpeg.dv", spec.name));
    let ref_s = ref_file.to_str().unwrap();
    let size = format!("{w}x{h}");
    let rate = if ntsc { "30000/1001" } else { "25" };
    let mut args = vec!["-f", "rawvideo", "-pix_fmt", spec.pix_fmt, "-s", &size, "-r", rate, "-i", "-", "-c:v", "dvvideo"];
    args.extend_from_slice(spec.ff_enc);
    args.extend_from_slice(&["-f", "dv", ref_s]);
    ffmpeg(&ff, &args, Some(&src));
    let ref_dec = ffmpeg(&ff, &["-i", ref_s, "-f", "rawvideo", "-pix_fmt", spec.pix_fmt, "-"], None);
    let (q_ours, q_ref) = (psnr(&src, &theirs), psnr(&src, &ref_dec));
    println!("{}: PSNR ours {q_ours:.2} dB, ffmpeg dvvideo {q_ref:.2} dB", spec.name);
    assert!(q_ours >= q_ref - 1.0, "{}: PSNR {q_ours:.2} dB, ffmpeg's encoder {q_ref:.2} dB", spec.name);
    // time code and sound as ffmpeg reads them
    let probe = Command::new(ff.with_file_name("ffprobe"))
        .args(["-v", "error", "-show_entries", "format_tags=timecode:stream=sample_rate,channels", "-of", "compact", file_s])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if !probe.is_empty() {
        let want = if ntsc { "timecode=10:00:00;00" } else { "timecode=10:00:00:00" };
        assert!(probe.contains(want), "{}: ffprobe says {probe}", spec.name);
        assert!(probe.contains("sample_rate=48000") && probe.contains("channels=2"), "{}: ffprobe says {probe}", spec.name);
    }
    let audio = ffmpeg(&ff, &["-i", file_s, "-f", "s16le", "-ac", "2", "-"], None);
    let got: Vec<i16> = audio.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    assert_eq!(got.len(), pcm.len(), "{}: ffmpeg audio sample count", spec.name);
    assert!(got == pcm, "{}: ffmpeg's audio decode differs", spec.name);
}

#[test]
fn enc_dv25_525_411() {
    check(&SPECS[0]);
}

#[test]
fn enc_dv25_625_420_wide_interlaced() {
    check(&SPECS[1]);
}

#[test]
fn enc_dvcpro25_625_411() {
    check(&SPECS[2]);
}
