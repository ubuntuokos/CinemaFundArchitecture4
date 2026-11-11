//! The Layer II encoder against ffmpeg as the reference decoder (CRC checked), and its quality
//! against ffmpeg's MP2 encoder at the same bit rate.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_mpa::Decoder;
use cfa4_mpa::encode::{Config, DELAY, Encoder, StereoMode};

/// Two tones per channel (different per channel), at `rate`.
fn tones(rate: u32, channels: usize, n: usize) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|ch| {
            let (f1, f2) = if ch == 0 { (441.0, 3100.0) } else { (660.0, 5000.0) };
            (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    0.3 * (2.0 * std::f32::consts::PI * f1 * t).sin() + 0.1 * (2.0 * std::f32::consts::PI * f2 * t).sin()
                })
                .collect()
        })
        .collect()
}

fn encode(cfg: Config, input: &[Vec<f32>]) -> Vec<u8> {
    let mut enc = Encoder::new(cfg).unwrap();
    let refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
    let mut out: Vec<u8> = enc.encode(&refs).unwrap().concat();
    out.extend(enc.flush().concat());
    out
}

fn out_path(name: &str) -> PathBuf {
    let dir = cfa4_testkit::fixtures_dir("mpa");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

/// ffmpeg's decode, interleaved f32; with `crc`, a CRC mismatch fails the decode.
fn ffmpeg_decode(ff: &Path, file: &Path) -> Vec<f32> {
    let o = Command::new(ff)
        .args(["-v", "error", "-err_detect", "crccheck+explode", "-f", "mp3", "-i"])
        .arg(file)
        .args(["-f", "f32le", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{}: {}", file.display(), String::from_utf8_lossy(&o.stderr));
    o.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

fn ours(data: &[u8], channels: usize) -> Vec<Vec<f32>> {
    let mut dec = Decoder::new();
    let mut out = vec![Vec::new(); channels];
    let mut p = 0;
    while p < data.len() {
        let f = dec.decode(&data[p..]).unwrap();
        p += f.header.frame_bytes;
        for (o, c) in out.iter_mut().zip(f.channels) {
            o.extend(c);
        }
    }
    out
}

fn snr(reference: &[f32], test: &[f32]) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (&a, &b) in reference.iter().zip(test) {
        s += f64::from(a) * f64::from(a);
        e += f64::from(a - b) * f64::from(a - b);
    }
    10.0 * (s / e.max(1e-30)).log10()
}

/// Streams of every kind decode in ffmpeg (CRCs checked) exactly as in our decoder.
#[test]
fn ffmpeg_decodes_our_streams() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let cases: &[(&str, u32, usize, u32, StereoMode, bool)] = &[
        ("enc_stereo_48k_192k", 48_000, 2, 192, StereoMode::Stereo, false),
        ("enc_stereo_44k_128k_crc", 44_100, 2, 128, StereoMode::Stereo, true),
        ("enc_dual_48k_256k_crc", 48_000, 2, 256, StereoMode::DualChannel, true),
        ("enc_stereo_44k_384k", 44_100, 2, 384, StereoMode::Stereo, false),
        ("enc_mono_32k_64k_crc", 32_000, 1, 64, StereoMode::Stereo, true),
        ("enc_mono_48k_32k", 48_000, 1, 32, StereoMode::Stereo, false),
        ("enc_lsf_stereo_24k_96k_crc", 24_000, 2, 96, StereoMode::Stereo, true),
        ("enc_lsf_mono_22k_8k", 22_050, 1, 8, StereoMode::Stereo, false),
    ];
    for &(name, rate, channels, kbps, stereo, crc) in cases {
        let mut cfg = Config::new(rate, channels, kbps);
        cfg.stereo = stereo;
        cfg.crc = crc;
        let input = tones(rate, channels, rate as usize);
        let data = encode(cfg, &input);
        let file = out_path(&format!("{name}.mp2"));
        std::fs::write(&file, &data).unwrap();
        let theirs = ffmpeg_decode(&ff, &file);
        let mine = ours(&data, channels);
        assert_eq!(theirs.len(), mine[0].len() * channels, "{name}: sample count");
        let worst = (0..channels)
            .map(|c| {
                let t: Vec<f32> = theirs.iter().skip(c).step_by(channels).copied().collect();
                snr(&t, &mine[c])
            })
            .fold(f64::INFINITY, f64::min);
        println!("{name}: ffmpeg vs our decoder {worst:.1} dB");
        assert!(worst >= 70.0, "{name}: {worst:.1} dB");
    }
}

/// The best SNR of `decoded` against `input` over delays `0..max`, and that delay.
fn aligned_snr(input: &[f32], decoded: &[f32], max: usize) -> (f64, usize) {
    let n = input.len().saturating_sub(max + 4000);
    (0..max)
        .map(|d| (snr(&input[2000..2000 + n], decoded.get(2000 + d..2000 + d + n).unwrap_or(&[])), d))
        .fold((f64::NEG_INFINITY, 0), |a, b| if b.0 > a.0 { b } else { a })
}

/// Tone SNR of our encoder next to ffmpeg's `mp2` encoder at the same bit rate (both decoded by
/// ffmpeg). Ours must not fall far behind.
#[test]
fn quality_next_to_ffmpegs_encoder() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for (rate, channels, kbps) in [(48_000u32, 2usize, 192u32), (48_000, 2, 128), (44_100, 2, 256), (48_000, 1, 96)] {
        let input = tones(rate, channels, rate as usize * 2);
        let raw = out_path(&format!("src_{rate}_{channels}.f32"));
        let interleaved: Vec<u8> = (0..input[0].len()).flat_map(|i| input.iter().map(move |c| c[i])).flat_map(f32::to_le_bytes).collect();
        std::fs::write(&raw, interleaved).unwrap();
        let theirs_file = out_path(&format!("ffmpeg_{rate}_{channels}_{kbps}.mp2"));
        let ok = Command::new(&ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "f32le", "-ar", &rate.to_string(), "-ac", &channels.to_string(), "-i"])
            .arg(&raw)
            .args(["-c:a", "mp2", "-b:a", &format!("{kbps}k"), "-f", "mp2"])
            .arg(&theirs_file)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg mp2 encode");
        let ours_file = out_path(&format!("ours_{rate}_{channels}_{kbps}.mp2"));
        std::fs::write(&ours_file, encode(Config::new(rate, channels, kbps), &input)).unwrap();
        let mut results = Vec::new();
        for f in [&ours_file, &theirs_file] {
            let d = ffmpeg_decode(&ff, f);
            let (s, delay) = (0..channels)
                .map(|c| {
                    let ch: Vec<f32> = d.iter().skip(c).step_by(channels).copied().collect();
                    aligned_snr(&input[c], &ch, 2400)
                })
                .fold((f64::INFINITY, 0), |a, b| if b.0 < a.0 { b } else { a });
            results.push((s, delay));
        }
        let ((o, od), (t, td)) = (results[0], results[1]);
        println!("{rate} Hz {channels} ch {kbps} kbit/s: ours {o:.1} dB (delay {od}), ffmpeg {t:.1} dB (delay {td})");
        assert_eq!(od, DELAY);
        assert!(o >= t - 3.0, "ours {o:.1} dB, ffmpeg {t:.1} dB");
    }
}
