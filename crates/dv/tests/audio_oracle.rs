//! DV audio against ffmpeg: ffmpeg writes raw DIF streams with 16-bit PCM (48 kHz in both systems,
//! 44.1 and 32 kHz at 625/50), and its decode of the same file is the reference. PCM is exact: every
//! sample and the per-frame sample counts must match.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Spec {
    name: &'static str,
    lavfi: &'static str,
    pix_fmt: &'static str,
    rate: u32,
}

const SPECS: &[Spec] = &[
    Spec { name: "audio_525_48k", lavfi: "testsrc2=size=720x480:rate=30000/1001", pix_fmt: "yuv411p", rate: 48_000 },
    Spec { name: "audio_625_48k", lavfi: "testsrc2=size=720x576:rate=25", pix_fmt: "yuv420p", rate: 48_000 },
    Spec { name: "audio_625_44k", lavfi: "testsrc2=size=720x576:rate=25", pix_fmt: "yuv420p", rate: 44_100 },
    Spec { name: "audio_625_32k", lavfi: "testsrc2=size=720x576:rate=25", pix_fmt: "yuv420p", rate: 32_000 },
];

const FRAMES: usize = 12;

fn path(s: &Spec) -> PathBuf {
    cfa4_testkit::fixtures_dir("dv").join(format!("{}.dv", s.name))
}

fn make(ff: &Path, s: &Spec) -> Option<PathBuf> {
    let out = path(s);
    let rate = s.rate.to_string();
    // a different tone per channel, and noise, so a swapped or shuffled sample shows
    let audio = format!("aevalsrc=0.5*sin(2*PI*440*t)+0.1*random(1)|0.4*sin(2*PI*1013*t)+0.1*random(2):s={rate}");
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let frames = FRAMES.to_string();
        let st = Command::new(ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", s.lavfi, "-f", "lavfi", "-i", &audio])
            .args(["-frames:v", &frames, "-c:v", "dvvideo", "-pix_fmt", s.pix_fmt, "-c:a", "pcm_s16le", "-ar", &rate, "-ac", "2", "-f", "dv"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(st) if st.success())
    })
}

fn ffmpeg_pcm(ff: &Path, file: &Path) -> Vec<i16> {
    let o = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-f", "s16le", "-acodec", "pcm_s16le", "-"])
        .stdin(Stdio::null())
        .output()
        .expect("run ffmpeg");
    assert!(o.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&o.stderr));
    o.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

#[test]
fn pcm_matches_ffmpeg_exactly() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let file = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&file).unwrap();
        let mut ours = Vec::new();
        for r in cfa4_dv::frames(&data).unwrap() {
            let a = cfa4_dv::decode_audio(&data[r]).unwrap_or_else(|| panic!("{}: no audio", s.name));
            assert_eq!((a.format.sample_rate, a.format.bits, a.format.channels, a.errors), (s.rate, 16, 2, 0), "{}", s.name);
            for k in 0..a.format.samples {
                ours.push(a.channels[0][k]);
                ours.push(a.channels[1][k]);
            }
        }
        let want = ffmpeg_pcm(&ff, &file);
        assert_eq!(ours.len(), want.len(), "{}: sample count", s.name);
        let bad = ours.iter().zip(&want).filter(|(a, b)| a != b).count();
        assert_eq!(bad, 0, "{}: {bad} samples differ", s.name);
        assert!(want.iter().any(|&v| v.unsigned_abs() > 8000), "{}: silent fixture", s.name);
    }
}

#[test]
fn damaged_audio_blocks_are_concealed_without_panics() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let file = make(&ff, &SPECS[1]).unwrap();
    let data = std::fs::read(&file).unwrap();
    let r = cfa4_dv::frames(&data).unwrap()[0].clone();
    let mut f = data[r].to_vec();
    // mark every sample of the first audio block of sequence 2 as damaged (0x8000)
    let at = 2 * 12_000 + 6 * 80;
    for b in f[at + 8..at + 80].chunks_exact_mut(2) {
        b.copy_from_slice(&[0x80, 0x00]);
    }
    let a = cfa4_dv::decode_audio(&f).unwrap();
    assert_eq!(a.errors, 36);
    assert!(a.channels[0].iter().all(|&v| v != i16::MIN));
    // truncated frames and garbage never panic
    for n in [0, 100, 480, 2000, 30_000, 143_999] {
        let _ = cfa4_dv::decode_audio(&f[..n]);
    }
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..200 {
        let mut g = f.clone();
        for _ in 0..64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let i = (x as usize) % g.len();
            g[i] = (x >> 40) as u8;
        }
        let _ = cfa4_dv::decode_audio(&g);
    }
}

/// `cargo xtask fixtures dv`: build the audio fixtures.
#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg() else {
        for s in SPECS {
            cfa4_testkit::fixtures::report(s.name, cfa4_testkit::fixtures::Status::Skipped);
        }
        return;
    };
    for s in SPECS {
        let out = path(s);
        cfa4_testkit::fixtures::generate_and_report(s.name, std::slice::from_ref(&out), || make(&ff, s));
    }
}
