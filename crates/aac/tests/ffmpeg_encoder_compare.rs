//! Our AAC-LC encoder next to the pinned FFmpeg's native `aac` encoder (CFA4-FFMPEG-9.1-001 F2n:
//! FFmpeg as a comparison baseline, used as a black box; no FFmpeg code was read). The same
//! signals at the same bitrates go through both encoders, ffmpeg decodes both streams, and the
//! test prints one table: actual bitrate, SNR, SNR below our encoder's bandwidth, and segmental
//! SNR. FFmpeg runs twice, with its defaults (NMR coder, PNS on) and with PNS off: noise
//! substitution trades waveform SNR for perceived quality, so its default rows are not a
//! like-for-like SNR comparison. By owner decision the table carries no thresholds; the test
//! only fails when a stream does not decode. Run with `--nocapture` to see it.

mod common;

use cfa4_aac::{EncoderConfig, split_adts};
use common::*;

const CONTEXT: &str = "aac encoder comparison";

/// ffmpeg at `CFA4_FFMPEG_MIN_VERSION` (`packaging/ffmpeg-version.env`) or newer, else a skip.
fn pinned_ffmpeg() -> Option<std::path::PathBuf> {
    let ff = cfa4_testkit::ffmpeg_or_skip(CONTEXT)?;
    let env = std::fs::read_to_string(cfa4_testkit::workspace_root().join("packaging/ffmpeg-version.env")).expect("read packaging/ffmpeg-version.env");
    let v = env
        .lines()
        .find_map(|l| l.trim().strip_prefix("CFA4_FFMPEG_MIN_VERSION="))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("CFA4_FFMPEG_MIN_VERSION in packaging/ffmpeg-version.env");
    let mut p = v.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
    if !cfa4_testkit::oracle::ffmpeg_at_least(&ff, p.next().unwrap_or(0), p.next().unwrap_or(0)) {
        let have = cfa4_testkit::oracle::ffmpeg_version(&ff).unwrap_or_default();
        cfa4_testkit::skip(CONTEXT, &format!("ffmpeg {have} predates the pinned AAC encoder (needs {v}+)"));
        return None;
    }
    Some(ff)
}

struct Signal {
    name: &'static str,
    make: fn(u32, usize) -> Vec<Vec<f32>>,
}

fn stereo(a: Vec<f32>, b: Vec<f32>) -> Vec<Vec<f32>> {
    vec![a, b]
}

const SIGNALS: &[Signal] = &[
    Signal { name: "sweep", make: |r, n| stereo(sweep(r, n, 0.5), sweep(r, n, 0.4)) },
    Signal { name: "noise", make: |_, n| stereo(noise(n, 0.3, 7), noise(n, 0.3, 8)) },
    Signal { name: "speech", make: |r, n| stereo(speech(r, n, 3), speech(r, n, 4)) },
    Signal { name: "chords", make: |r, n| stereo(chords(r, n, 0.0), chords(r, n, 0.002)) },
    Signal { name: "clicks", make: |r, n| stereo(clicks(r, n), clicks(r, n).iter().map(|v| v * 0.7).collect()) },
];

struct Measure {
    bitrate: f64,
    snr: f64,
    bw_snr: f64,
    seg: f64,
}

/// Bitrate of an ADTS stream without its headers, and ffmpeg's decode measured against `input`.
fn measure(ff: &std::path::Path, adts: &[u8], path: &std::path::Path, input: &[Vec<f32>], rate: u32, bandwidth: u32) -> Measure {
    std::fs::write(path, adts).unwrap();
    let frames = split_adts(adts).unwrap();
    let payload: usize = frames.iter().map(|(_, au)| au.len()).sum();
    let bitrate = payload as f64 * 8.0 * f64::from(rate) / (frames.len() as f64 * 1024.0);
    let (dec, err) = ffmpeg_decode(ff, path, input.len());
    assert!(err.trim().is_empty(), "ffmpeg reported errors for {}: {err}", path.display());
    let n = input[0].len();
    assert!(dec[0].len() >= n + 1024, "{}: decoded stream too short", path.display());
    let fc = bandwidth as f32 * 0.97;
    let (mut snr_sum, mut bw_sum, mut seg_sum) = (0.0, 0.0, 0.0);
    for (c, x) in input.iter().enumerate() {
        let d = &dec[c][1024..1024 + n];
        snr_sum += snr(x, d);
        bw_sum += snr(&lowpass(x, fc, rate as f32), &lowpass(d, fc, rate as f32));
        seg_sum += seg_snr(x, d);
    }
    let k = input.len() as f64;
    Measure { bitrate, snr: snr_sum / k, bw_snr: bw_sum / k, seg: seg_sum / k }
}

#[test]
fn our_encoder_next_to_the_pinned_ffmpeg_encoder() {
    let Some(ff) = pinned_ffmpeg() else { return };
    println!(
        "{:<8} {:>6} {:>7}  {:>22}  {:>22}  {:>22}",
        "signal", "rate", "target", "CFA4  kb/s SNR bw seg", "ffmpeg kb/s SNR bw seg", "ffmpeg-noPNS kb/s SNR bw seg"
    );
    for &(rate, bitrate) in &[(44_100u32, 64_000u32), (44_100, 128_000), (48_000, 96_000), (48_000, 192_000)] {
        let n = rate as usize * 2 + 333;
        for s in SIGNALS {
            let input = (s.make)(rate, n);
            let ours = encode(EncoderConfig::cbr(rate, 2, bitrate), &input);
            let tag = format!("{}_{rate}_{bitrate}", s.name);
            let dir = fixtures();
            let m_ours = measure(&ff, &ours.adts, &dir.join(format!("cmp_cfa4_{tag}.aac")), &input, rate, ours.bandwidth);
            let mut rows = vec![m_ours];
            for (label, extra) in [("ff", &[][..]), ("ffnopns", &["-aac_pns", "0"][..])] {
                let path = dir.join(format!("cmp_{label}_{tag}.aac"));
                let kbps = format!("{}k", bitrate / 1000);
                let mut args = vec!["-b:a", kbps.as_str()];
                args.extend_from_slice(extra);
                ffmpeg_encode(&ff, &input, rate, &args, &path);
                let adts = std::fs::read(&path).unwrap();
                rows.push(measure(&ff, &adts, &path, &input, rate, ours.bandwidth));
            }
            let cell = |m: &Measure| format!("{:>6.1} {:>5.1} {:>5.1} {:>5.1}", m.bitrate / 1000.0, m.snr, m.bw_snr, m.seg);
            println!("{:<8} {:>6} {:>7}  {}  {}  {}", s.name, rate, bitrate, cell(&rows[0]), cell(&rows[1]), cell(&rows[2]));
        }
    }
}
