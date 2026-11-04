//! AC-3 and E-AC-3 against ffmpeg: ffmpeg's AC-3 and E-AC-3 encoders write raw syncframes from
//! generated tones and noise; ffmpeg's decode of the same file is the reference. AC-3 output is not bit-exact across
//! decoders (zero-bit mantissas are filled with decoder-specific dither, §7.3.4; float vs fixed
//! arithmetic), so the criterion is SNR against ffmpeg's decode.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Spec {
    name: &'static str,
    /// ffmpeg encoder and raw muxer: "ac3" or "eac3".
    codec: &'static str,
    input: &'static str,
    rate: u32,
    channels: u32,
    enc: &'static [&'static str],
    /// Minimum SNR (dB) against ffmpeg.
    min_snr: f64,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "ac3_stereo_192k",
        codec: "ac3",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000",
        rate: 48_000,
        channels: 2,
        enc: &["-b:a", "192k"],
        min_snr: 60.0,
    },
    // low bitrate stereo: coupling and rematrixing
    Spec {
        name: "ac3_stereo_96k_coupling",
        codec: "ac3",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.05*sin(2*PI*9100*t)|0.3*sin(2*PI*441*t)+0.05*sin(2*PI*11000*t):s=44100",
        rate: 44_100,
        channels: 2,
        enc: &["-b:a", "96k", "-channel_coupling", "1"],
        min_snr: 40.0,
    },
    Spec {
        name: "ac3_mono_32k",
        codec: "ac3",
        input: "sine=frequency=1000:sample_rate=32000",
        rate: 32_000,
        channels: 1,
        enc: &["-b:a", "96k"],
        min_snr: 60.0,
    },
    // 5.1 with LFE
    Spec {
        name: "ac3_51_448k",
        codec: "ac3",
        input: "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1",
        rate: 48_000,
        channels: 6,
        enc: &["-b:a", "448k"],
        min_snr: 55.0,
    },
    // noise (wideband, many zero-bit mantissas): the SNR is limited by dither
    Spec {
        name: "ac3_noise_stereo",
        codec: "ac3",
        input: "anoisesrc=color=pink:sample_rate=48000:amplitude=0.3:seed=3,pan=stereo|c0=c0|c1=-0.5*c0",
        rate: 48_000,
        channels: 2,
        enc: &["-b:a", "128k"],
        min_snr: 18.0,
    },
    // E-AC-3 (Annex E): ffmpeg writes six-block syncframes with frame-based exponent strategies
    Spec {
        name: "eac3_stereo_192k",
        codec: "eac3",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000",
        rate: 48_000,
        channels: 2,
        enc: &["-b:a", "192k"],
        min_snr: 60.0,
    },
    // low bitrate stereo: coupling (with the default band structure) and rematrixing
    Spec {
        name: "eac3_stereo_64k_coupling",
        codec: "eac3",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.05*sin(2*PI*9100*t)|0.3*sin(2*PI*441*t)+0.05*sin(2*PI*11000*t):s=44100",
        rate: 44_100,
        channels: 2,
        enc: &["-b:a", "64k", "-channel_coupling", "1"],
        min_snr: 35.0,
    },
    Spec {
        name: "eac3_mono_32k",
        codec: "eac3",
        input: "sine=frequency=1000:sample_rate=32000",
        rate: 32_000,
        channels: 1,
        enc: &["-b:a", "64k"],
        min_snr: 60.0,
    },
    // 5.1 with LFE and mixing / informational metadata in the bsi
    Spec {
        name: "eac3_51_384k_metadata",
        codec: "eac3",
        input: "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1",
        rate: 48_000,
        channels: 6,
        enc: &[
            "-b:a",
            "384k",
            "-dmix_mode",
            "loro",
            "-ltrt_cmixlev",
            "0.707",
            "-loro_surmixlev",
            "0.5",
            "-dsurex_mode",
            "on",
            "-mixing_level",
            "100",
            "-room_type",
            "small",
        ],
        min_snr: 55.0,
    },
    Spec {
        name: "eac3_noise_stereo",
        codec: "eac3",
        input: "anoisesrc=color=pink:sample_rate=48000:amplitude=0.3:seed=3,pan=stereo|c0=c0|c1=-0.5*c0",
        rate: 48_000,
        channels: 2,
        enc: &["-b:a", "96k", "-dsur_mode", "on", "-dheadphone_mode", "off"],
        min_snr: 18.0,
    },
];

fn make(ff: &Path, s: &Spec) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("ac3").join(format!("{}.{}", s.name, s.codec));
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", s.input, "-t", "1"])
            .args(["-c:a", s.codec])
            .args(s.enc)
            .args(["-f", s.codec])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(x) if x.success())
    })
}

fn frames(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    while let Ok(h) = cfa4_ac3::parse_header(&data[p..]) {
        if p + h.frame_bytes > data.len() {
            break;
        }
        out.push(&data[p..p + h.frame_bytes]);
        p += h.frame_bytes;
    }
    out
}

#[test]
fn ffmpeg_encoded_streams_decode() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        let mut dec = cfa4_ac3::Decoder::new();
        let mut ours: Vec<Vec<f32>> = vec![Vec::new(); s.channels as usize];
        for fr in frames(&data) {
            let d = dec.decode(fr).unwrap();
            assert_eq!((d.header.sample_rate, d.channels.len() as u32), (s.rate, s.channels), "{}", s.name);
            assert_eq!(d.header.is_eac3(), s.codec == "eac3", "{}", s.name);
            for (o, c) in ours.iter_mut().zip(d.channels) {
                o.extend(c);
            }
        }
        let o = Command::new(&ff).args(["-v", "error", "-i"]).arg(&f).args(["-f", "f32le", "-"]).output().unwrap();
        let want: Vec<f32> = o.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let ch = s.channels as usize;
        let n = want.len() / ch;
        assert_eq!(n, ours[0].len(), "{}: sample count", s.name);
        let mut worst = f64::INFINITY;
        for c in 0..ch {
            let (mut sig, mut err) = (0f64, 0f64);
            for i in 0..n {
                let (a, b) = (ours[c][i] as f64, want[i * ch + c] as f64);
                sig += b * b;
                err += (a - b) * (a - b);
            }
            let snr = 10.0 * (sig / err.max(1e-30)).log10();
            worst = worst.min(snr);
        }
        // the same stream decoded with another dither sequence: how far dither alone moves the
        // output (the difference to ffmpeg must be of that order)
        let mut dec2 = cfa4_ac3::Decoder::new();
        dec2.set_dither_seed(0xDEAD_BEEF);
        let other: Vec<f32> = frames(&data).iter().flat_map(|fr| dec2.decode(fr).unwrap().channels.swap_remove(0)).collect();
        let (mut sig, mut err) = (0f64, 0f64);
        for i in 0..n {
            sig += (ours[0][i] as f64).powi(2);
            err += (ours[0][i] as f64 - other[i] as f64).powi(2);
        }
        let dither_snr = 10.0 * (sig / err.max(1e-30)).log10();
        println!(
            "{}: {} Hz, {} ch, {} frames, worst channel SNR vs ffmpeg {worst:.1} dB (two dither seeds: {dither_snr:.1} dB)",
            s.name,
            s.rate,
            ch,
            frames(&data).len()
        );
        assert!(worst >= s.min_snr, "{}: SNR {worst:.1} dB < {}", s.name, s.min_snr);
    }
}

/// Damaged E-AC-3 syncframes (bit flips, truncation, corrupt sizes) give errors or noise, never
/// a panic.
#[test]
fn damaged_eac3_frames_never_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut x = 0x00C0_FFEEu32;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for s in SPECS.iter().filter(|s| s.codec == "eac3") {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        let mut dec = cfa4_ac3::Decoder::new();
        for fr in frames(&data).into_iter().take(8) {
            for k in 0..300 {
                let mut m = fr.to_vec();
                match k % 3 {
                    0 => {
                        for _ in 0..1 + rnd() % 8 {
                            let bit = rnd() as usize % (m.len() * 8);
                            m[bit / 8] ^= 0x80 >> (bit % 8);
                        }
                    }
                    1 => m.truncate(rnd() as usize % m.len()),
                    _ => {
                        // keep the sync word, scramble frmsiz / fscod / numblkscod / acmod
                        m[2] ^= rnd() as u8;
                        m[3] ^= rnd() as u8;
                        m[4] ^= rnd() as u8;
                    }
                }
                let _ = dec.decode(&m);
            }
        }
    }
}

#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("ac3 fixtures") else { return };
    for s in SPECS {
        let out = cfa4_testkit::fixtures_dir("ac3").join(format!("{}.{}", s.name, s.codec));
        cfa4_testkit::fixtures::generate_and_report(&format!("ac3/{}", s.name), &[out], || make(&ff, s));
    }
}

/// Interleaved f32 samples from ffmpeg's stdout, split per channel.
fn planar(bytes: &[u8], ch: usize) -> Vec<Vec<f32>> {
    let v: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    (0..ch).map(|c| v.iter().skip(c).step_by(ch).copied().collect()).collect()
}

/// SNR (dB) of `got` against `want` over the middle of the signal, at the delay in `delays`
/// that gives the best.
fn aligned_snr(want: &[f32], got: &[f32], delays: std::ops::Range<usize>) -> f64 {
    let n = want.len().saturating_sub(4096);
    let mut best = f64::NEG_INFINITY;
    for d in delays {
        let (mut s, mut e) = (0f64, 0f64);
        for i in 2048..n {
            let (a, b) = (f64::from(want[i]), f64::from(got.get(i + d).copied().unwrap_or(0.0)));
            s += a * a;
            e += (a - b) * (a - b);
        }
        let snr = 10.0 * (s / e.max(1e-30)).log10();
        best = best.max(snr);
    }
    best
}

/// Our encoder against ffmpeg's: the same input at the same bit rate, both decoded by ffmpeg
/// with CRC checking on. Ours must decode in ffmpeg (CRCs valid), agree with our decoder, and
/// come within a few dB of ffmpeg's own encoder in SNR.
#[test]
fn encoded_streams_decode_in_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let dir = cfa4_testkit::fixtures_dir("ac3").join("encoder");
    std::fs::create_dir_all(&dir).unwrap();
    // (name, lavfi input, rate, channels, kb/s, dB behind ffmpeg allowed, minimum SNR); names
    // starting with "eac3" are E-AC-3
    let cases: &[(&str, &str, u32, usize, u32, f64, f64)] = &[
        (
            "stereo_192k",
            "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000",
            48_000,
            2,
            192,
            6.0,
            30.0,
        ),
        ("mono_32k", "sine=frequency=1000:sample_rate=32000", 32_000, 1, 96, 6.0, 40.0),
        (
            "stereo_44k_96k",
            "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.05*sin(2*PI*2100*t)|0.3*sin(2*PI*445*t)+0.05*sin(2*PI*3000*t):s=44100",
            44_100,
            2,
            96,
            8.0,
            20.0,
        ),
        (
            "51_448k",
            "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1",
            48_000,
            6,
            448,
            6.0,
            30.0,
        ),
        (
            "eac3_stereo_192k",
            "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000",
            48_000,
            2,
            192,
            6.0,
            30.0,
        ),
        (
            "eac3_51_384k",
            "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1",
            48_000,
            6,
            384,
            6.0,
            30.0,
        ),
        ("eac3_mono_44k_100k", "sine=frequency=1000:sample_rate=44100", 44_100, 1, 100, 6.0, 40.0),
        ("pink_stereo_192k", "anoisesrc=color=pink:sample_rate=48000:amplitude=0.3:seed=3,pan=stereo|c0=c0|c1=-0.5*c0", 48_000, 2, 192, 6.0, 5.0),
    ];
    for &(name, input, rate, ch, kbps, behind, min) in cases {
        let o = Command::new(&ff).args(["-v", "error", "-f", "lavfi", "-i", input, "-t", "2", "-f", "f32le", "-"]).output().unwrap();
        assert!(o.status.success(), "{name}: input");
        let pcm = planar(&o.stdout, ch);
        // ours
        let eac3 = name.starts_with("eac3");
        let codec = if eac3 { "eac3" } else { "ac3" };
        let mut enc = if eac3 { cfa4_ac3::Encoder::new_eac3(rate, ch, kbps) } else { cfa4_ac3::Encoder::new(rate, ch, kbps) }.unwrap();
        let refs: Vec<&[f32]> = pcm.iter().map(Vec::as_slice).collect();
        let mut stream: Vec<u8> = enc.encode(&refs).unwrap().concat();
        stream.extend(enc.flush().unwrap().concat());
        let ours = dir.join(format!("{name}.{codec}"));
        std::fs::write(&ours, &stream).unwrap();
        // ffmpeg's encoder on the same samples
        let raw = dir.join(format!("{name}.f32"));
        std::fs::write(&raw, &o.stdout).unwrap();
        let theirs = dir.join(format!("{name}_ffmpeg.{codec}"));
        let st = Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "f32le", "-ar", &rate.to_string(), "-ac", &ch.to_string(), "-i"])
            .arg(&raw)
            .args(["-c:a", codec, "-b:a", &format!("{kbps}k")])
            .arg(&theirs)
            .status();
        assert!(st.is_ok_and(|s| s.success()), "{name}: ffmpeg encode");
        let decode = |f: &Path| {
            let o = Command::new(&ff).args(["-v", "error", "-err_detect", "crccheck+explode", "-i"]).arg(f).args(["-f", "f32le", "-"]).output().unwrap();
            assert!(o.status.success(), "{name}: ffmpeg decode of {}: {}", f.display(), String::from_utf8_lossy(&o.stderr));
            planar(&o.stdout, ch)
        };
        let (a, b) = (decode(&ours), decode(&theirs));
        // our decoder on our stream agrees with ffmpeg's (up to dither)
        let mut dec = cfa4_ac3::Decoder::new();
        let mine: Vec<f32> = frames(&stream).iter().flat_map(|f| dec.decode(f).unwrap().channels.swap_remove(0)).collect();
        let agree = aligned_snr(&a[0], &mine, 0..1);
        let mut worst = (f64::INFINITY, f64::INFINITY);
        for c in 0..ch {
            // ours lags by the transform overlap; ffmpeg's encoder by its own priming
            let s_ours = aligned_snr(&pcm[c], &a[c], 256..257);
            let s_ff = aligned_snr(&pcm[c], &b[c], 0..1700);
            worst = (worst.0.min(s_ours), worst.1.min(s_ff));
        }
        println!("{name}: SNR ours {:.1} dB, ffmpeg's encoder {:.1} dB; our decode vs ffmpeg's {agree:.1} dB", worst.0, worst.1);
        // the two decoders differ by their dither, which fills the zero-bit mantissas of noise
        let mut dec2 = cfa4_ac3::Decoder::new();
        dec2.set_dither_seed(0xDEAD_BEEF);
        let reseeded: Vec<f32> = frames(&stream).iter().flat_map(|f| dec2.decode(f).unwrap().channels.swap_remove(0)).collect();
        let dither = aligned_snr(&mine, &reseeded, 0..1);
        assert!(agree > dither.min(30.0) - 6.0, "{name}: decoders disagree ({agree:.1} dB, dither alone {dither:.1} dB)");
        assert!(worst.0 >= min && worst.0 >= worst.1 - behind, "{name}: {:.1} dB, ffmpeg {:.1} dB", worst.0, worst.1);
    }
}
