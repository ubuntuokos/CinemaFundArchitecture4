//! Decoder accuracy against ffmpeg's decoder on streams produced by ffmpeg's AAC encoder (oracle
//! only; skipped when ffmpeg is absent).

mod common;

use common::*;

struct Case {
    name: &'static str,
    rate: u32,
    channels: usize,
    args: &'static [&'static str],
}

fn input(rate: u32, channels: usize) -> Vec<Vec<f32>> {
    let n = rate as usize * 3 / 2;
    (0..channels)
        .map(|c| {
            let a = chords(rate, n, 0.003 * c as f32);
            let b = clicks(rate, n);
            let s = speech(rate, n, 5 + c as u64);
            a.iter().zip(&b).zip(&s).map(|((x, y), z)| 0.6 * x + 0.3 * y + 0.3 * z * if c % 2 == 0 { 1.0 } else { -0.5 }).collect()
        })
        .collect()
}

/// Returns (max abs error, samples compared).
fn compare(case: &Case) -> Option<(f32, usize)> {
    let ff = ffmpeg()?;
    let path = fixtures().join(format!("ffenc_{}.aac", case.name));
    ffmpeg_encode(&ff, &input(case.rate, case.channels), case.rate, case.args, &path);
    let data = std::fs::read(&path).unwrap();
    let (ref_pcm, err) = ffmpeg_decode(&ff, &path, case.channels);
    assert!(err.trim().is_empty(), "{err}");
    let ours = our_decode(&data);
    assert_eq!(ours.len(), case.channels);
    // ffmpeg reorders channels to its native layouts; match each of our (AAC element order)
    // channels to the closest ffmpeg channel and require a permutation.
    let mut worst = 0f32;
    let mut taken = vec![false; case.channels];
    for c in 0..case.channels {
        let (best, err) =
            (0..case.channels).filter(|&j| !taken[j]).map(|j| (j, max_abs_diff(&ours[c], &ref_pcm[j]))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        taken[best] = true;
        assert_eq!(ours[c].len(), ref_pcm[best].len(), "{}: length", case.name);
        worst = worst.max(err);
    }
    Some((worst, ours[0].len()))
}

#[test]
fn matches_ffmpeg_decoder() {
    if ffmpeg().is_none() {
        eprintln!("ffmpeg not found; skipping");
        return;
    }
    let cases = [
        Case { name: "stereo_44k_128k", rate: 44100, channels: 2, args: &["-b:a", "128k", "-aac_pns", "0"] },
        Case { name: "stereo_48k_256k", rate: 48000, channels: 2, args: &["-b:a", "256k", "-aac_pns", "0"] },
        Case { name: "mono_48k_64k", rate: 48000, channels: 1, args: &["-b:a", "64k", "-aac_pns", "0"] },
        Case {
            name: "stereo_44k_96k_is_ms_tns",
            rate: 44100,
            channels: 2,
            args: &["-b:a", "96k", "-aac_pns", "0", "-aac_is", "1", "-aac_ms", "1", "-aac_tns", "1"],
        },
        Case { name: "stereo_32k_48k_is", rate: 32000, channels: 2, args: &["-b:a", "48k", "-aac_pns", "0", "-aac_is", "1"] },
        Case { name: "mono_22k_32k", rate: 22050, channels: 1, args: &["-b:a", "32k", "-aac_pns", "0"] },
        Case { name: "mono_16k_24k", rate: 16000, channels: 1, args: &["-b:a", "24k", "-aac_pns", "0"] },
        Case { name: "mono_8k_16k", rate: 8000, channels: 1, args: &["-b:a", "16k", "-aac_pns", "0"] },
        Case { name: "stereo_96k_256k", rate: 96000, channels: 2, args: &["-b:a", "256k", "-aac_pns", "0"] },
        Case { name: "c3_48k", rate: 48000, channels: 3, args: &["-b:a", "192k", "-aac_pns", "0"] },
        Case { name: "c51_48k_320k", rate: 48000, channels: 6, args: &["-b:a", "320k", "-aac_pns", "0"] },
        Case { name: "c71_48k_448k", rate: 48000, channels: 8, args: &["-b:a", "448k", "-aac_pns", "0"] },
        Case { name: "stereo_44k_fast_coder", rate: 44100, channels: 2, args: &["-b:a", "160k", "-aac_coder", "fast", "-aac_pns", "0"] },
    ];
    println!("{:<28} {:>10} {:>12}", "stream", "samples/ch", "max |err|");
    let mut fails = Vec::new();
    for c in &cases {
        let (err, n) = compare(c).unwrap();
        println!("{:<28} {:>10} {:>12.3e}", c.name, n, err);
        if err >= 1e-3 {
            fails.push((c.name, err));
        }
    }
    assert!(fails.is_empty(), "decoder mismatches: {fails:?}");
}

/// PNS output is random by design (older ffmpeg encoders do not emit it; the pinned one does, see
/// `matches_ffmpeg_decoder_with_pns_in_band_power`): build noise-substituted
/// access units by hand (mono, and a common-window pair with M/S = correlated noise) and compare
/// per-frame energies and the L/R correlation with ffmpeg's decode.
#[test]
fn pns_matches_ffmpeg() {
    use cfa4_aac::AdtsHeader;
    use cfa4_bitstream::BitWriter;
    let Some(ff) = ffmpeg() else { return };
    fn ics(bw: &mut BitWriter, with_info: bool, nrg: u32) {
        bw.write_bits(100, 8); // global_gain
        if with_info {
            bw.write_bits(0, 4); // reserved, ONLY_LONG, sine
            bw.write_bits(40, 6);
            bw.write_bits(0, 1);
        }
        bw.write_bits(13, 4); // NOISE_HCB for all 40 bands
        bw.write_bits(31, 5);
        bw.write_bits(9, 5);
        bw.write_bits(nrg, 9); // first noise energy (PCM)
        for _ in 1..40 {
            bw.write_bits(0, 1); // difference 0
        }
        bw.write_bits(0, 3); // pulse, tns, gain control
    }
    for channels in [1usize, 2] {
        let mut stream = Vec::new();
        for f in 0..12u32 {
            let mut bw = BitWriter::new();
            let nrg = 256 + 20 + (f % 6) * 7;
            if channels == 1 {
                bw.write_bits(0, 3);
                bw.write_bits(0, 4);
                ics(&mut bw, true, nrg);
            } else {
                bw.write_bits(1, 3);
                bw.write_bits(0, 4);
                bw.write_bits(1, 1); // common_window
                bw.write_bits(0, 4);
                bw.write_bits(40, 6);
                bw.write_bits(0, 1);
                bw.write_bits(2, 2); // ms_mask_present = all
                ics(&mut bw, false, nrg);
                ics(&mut bw, false, nrg - 8);
            }
            bw.write_bits(7, 3);
            let au = bw.finish();
            stream.extend_from_slice(&AdtsHeader::write(2, 4, channels as u8, au.len(), 0x7FF));
            stream.extend_from_slice(&au);
        }
        let path = fixtures().join(format!("handmade_pns_{channels}.aac"));
        std::fs::write(&path, &stream).unwrap();
        let (ref_pcm, err) = ffmpeg_decode(&ff, &path, channels);
        assert!(err.trim().is_empty(), "{err}");
        let ours = our_decode(&stream);
        let mut worst_db = 0f64;
        for c in 0..channels {
            let e = |v: &[f32]| 10.0 * v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().log10();
            let (eo, ef) = (e(&ours[c]), e(&ref_pcm[c]));
            println!("PNS {channels}ch ch{c}: total energy ours {eo:.2} dB, ffmpeg {ef:.2} dB");
            assert!((eo - ef).abs() < 0.5);
            assert_eq!(ours[c].len(), ref_pcm[c].len());
            for s in (1024..ours[c].len() - 1024).step_by(1024) {
                let ea: f64 = ours[c][s..s + 1024].iter().map(|v| (*v as f64).powi(2)).sum();
                let eb: f64 = ref_pcm[c][s..s + 1024].iter().map(|v| (*v as f64).powi(2)).sum();
                worst_db = worst_db.max((10.0 * (ea / eb).log10()).abs());
            }
        }
        let corr = |a: &[f32], b: &[f32]| {
            let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
            for (x, y) in a.iter().zip(b) {
                ab += *x as f64 * *y as f64;
                aa += (*x as f64).powi(2);
                bb += (*y as f64).powi(2);
            }
            ab / (aa * bb).sqrt()
        };
        let (c_ours, c_ff) = if channels == 2 { (corr(&ours[0], &ours[1]), corr(&ref_pcm[0], &ref_pcm[1])) } else { (0.0, 0.0) };
        println!("PNS {channels}ch: per-frame energy max deviation {worst_db:.2} dB; L/R correlation ours {c_ours:.3} ffmpeg {c_ff:.3}");
        if channels == 1 {
            assert!(worst_db < 1.0);
        } else {
            // ISO/IEC 14496-3 §4.6.13: noise bands with ms_used share one random vector (correlated
            // noise). ffmpeg does not implement this and generates independent noise.
            assert!(c_ours > 0.99);
        }
    }
}

/// `CFA4_FFMPEG_MIN_VERSION` from `packaging/ffmpeg-version.env` as (major, minor).
fn min_ffmpeg_version() -> (String, u32, u32) {
    let env = std::fs::read_to_string(cfa4_testkit::workspace_root().join("packaging/ffmpeg-version.env")).expect("read packaging/ffmpeg-version.env");
    let v = env
        .lines()
        .find_map(|l| l.trim().strip_prefix("CFA4_FFMPEG_MIN_VERSION="))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("CFA4_FFMPEG_MIN_VERSION in packaging/ffmpeg-version.env");
    let mut p = v.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
    let (major, minor) = (p.next().unwrap_or(0), p.next().unwrap_or(0));
    (v, major, minor)
}

/// ffmpeg at the pinned minimum or newer (its AAC encoder has the NMR coder, `-aac_rc` and
/// `-aac_nmr_speed`), else `None` after a skip.
fn pinned_ffmpeg(context: &str) -> Option<std::path::PathBuf> {
    let ff = cfa4_testkit::ffmpeg_or_skip(context)?;
    let (v, major, minor) = min_ffmpeg_version();
    if !cfa4_testkit::oracle::ffmpeg_at_least(&ff, major, minor) {
        let have = cfa4_testkit::oracle::ffmpeg_version(&ff).unwrap_or_default();
        cfa4_testkit::skip(context, &format!("ffmpeg {have} predates the pinned AAC encoder (needs {v}+)"));
        return None;
    }
    Some(ff)
}

/// The pinned FFmpeg's AAC encoder modes, without PNS: our decoder must match ffmpeg's sample
/// by sample, like the cases above.
#[test]
fn matches_ffmpeg_decoder_on_the_pinned_encoder_modes() {
    if pinned_ffmpeg("aac decoder oracle (pinned encoder modes)").is_none() {
        return;
    }
    let cases = [
        Case { name: "pin_stereo_44k_128k_nmr", rate: 44100, channels: 2, args: &["-b:a", "128k", "-aac_coder", "nmr", "-aac_pns", "0"] },
        Case { name: "pin_stereo_44k_128k_twoloop", rate: 44100, channels: 2, args: &["-b:a", "128k", "-aac_coder", "twoloop", "-aac_pns", "0"] },
        Case { name: "pin_stereo_48k_96k_abr", rate: 48000, channels: 2, args: &["-b:a", "96k", "-aac_rc", "abr", "-aac_pns", "0"] },
        Case { name: "pin_stereo_48k_96k_speed4", rate: 48000, channels: 2, args: &["-b:a", "96k", "-aac_nmr_speed", "4", "-aac_pns", "0"] },
        Case { name: "pin_stereo_32k_40k_is_ms", rate: 32000, channels: 2, args: &["-b:a", "40k", "-aac_is", "1", "-aac_ms", "1", "-aac_pns", "0"] },
        Case { name: "pin_mono_16k_20k_abr", rate: 16000, channels: 1, args: &["-b:a", "20k", "-aac_rc", "abr", "-aac_pns", "0"] },
        Case { name: "pin_c51_48k_256k", rate: 48000, channels: 6, args: &["-b:a", "256k", "-aac_pns", "0"] },
        Case { name: "pin_c71_48k_384k_abr", rate: 48000, channels: 8, args: &["-b:a", "384k", "-aac_rc", "abr", "-aac_pns", "0"] },
    ];
    println!("{:<30} {:>10} {:>12}", "stream", "samples/ch", "max |err|");
    let mut fails = Vec::new();
    for c in &cases {
        let (err, n) = compare(c).unwrap();
        println!("{:<30} {:>10} {:>12.3e}", c.name, n, err);
        if err >= 1e-3 {
            fails.push((c.name, err));
        }
    }
    assert!(fails.is_empty(), "decoder mismatches: {fails:?}");
}

/// Band edges (Hz) for [`band_db`]: third-octave-like, 100 Hz up to Nyquist.
fn band_edges(rate: u32) -> Vec<f64> {
    let nyq = f64::from(rate) / 2.0;
    let mut e = vec![100.0];
    while *e.last().unwrap() * 1.26 < nyq {
        let next = e.last().unwrap() * 1.26;
        e.push(next);
    }
    e.push(nyq);
    e
}

/// Mean power per band in dB over Hann-windowed 2048-sample frames (hop 1024), from a direct DFT.
fn band_db(x: &[f32], rate: u32) -> Vec<f64> {
    const N: usize = 2048;
    let edges = band_edges(rate);
    let win: Vec<f64> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N as f64).cos()).collect();
    let (cos, sin): (Vec<f64>, Vec<f64>) = (0..N).map(|i| (2.0 * std::f64::consts::PI * i as f64 / N as f64).sin_cos()).map(|(s, c)| (c, s)).unzip();
    let mut power = vec![0f64; N / 2];
    let mut frames = 0;
    for start in (0..x.len().saturating_sub(N)).step_by(N / 2) {
        let f: Vec<f64> = x[start..start + N].iter().zip(&win).map(|(s, w)| f64::from(*s) * w).collect();
        for (k, p) in power.iter_mut().enumerate().skip(1) {
            let (mut re, mut im) = (0f64, 0f64);
            for (n, v) in f.iter().enumerate() {
                let i = (k * n) % N;
                re += v * cos[i];
                im -= v * sin[i];
            }
            *p += re * re + im * im;
        }
        frames += 1;
    }
    let bin_hz = f64::from(rate) / N as f64;
    edges
        .windows(2)
        .map(|w| {
            let s: f64 = power.iter().enumerate().filter(|(k, _)| (*k as f64 * bin_hz) >= w[0] && (*k as f64 * bin_hz) < w[1]).map(|(_, p)| p).sum();
            10.0 * (s / frames.max(1) as f64 + 1e-30).log10()
        })
        .collect()
}

/// The pinned FFmpeg's AAC encoder with its defaults, PNS included. Noise-substituted bands are
/// random, so the decoders are compared by power per band (within 1 dB wherever a band is within
/// 60 dB of the loudest); where the stream carries no PNS the samples still match.
#[test]
fn matches_ffmpeg_decoder_with_pns_in_band_power() {
    let Some(ff) = pinned_ffmpeg("aac decoder oracle (PNS)") else { return };
    let cases = [
        Case { name: "pin_pns_stereo_44k_48k", rate: 44100, channels: 2, args: &["-b:a", "48k"] },
        Case { name: "pin_pns_stereo_48k_64k_abr", rate: 48000, channels: 2, args: &["-b:a", "64k", "-aac_rc", "abr"] },
        Case { name: "pin_pns_mono_32k_24k", rate: 32000, channels: 1, args: &["-b:a", "24k"] },
        Case { name: "pin_pns_mono_44k_32k_twoloop", rate: 44100, channels: 1, args: &["-b:a", "32k", "-aac_coder", "twoloop"] },
        Case { name: "pin_pns_stereo_44k_64k_fast", rate: 44100, channels: 2, args: &["-b:a", "64k", "-aac_coder", "fast"] },
    ];
    println!("{:<30} {:>12} {:>14}", "stream", "max |err|", "max band dB");
    let mut fails = Vec::new();
    for case in &cases {
        let path = fixtures().join(format!("ffenc_{}.aac", case.name));
        ffmpeg_encode(&ff, &input(case.rate, case.channels), case.rate, case.args, &path);
        let data = std::fs::read(&path).unwrap();
        let (ref_pcm, err) = ffmpeg_decode(&ff, &path, case.channels);
        assert!(err.trim().is_empty(), "{err}");
        let ours = our_decode(&data);
        assert_eq!(ours.len(), case.channels);
        let (mut worst_err, mut worst_db) = (0f32, 0f64);
        for c in 0..case.channels {
            assert_eq!(ours[c].len(), ref_pcm[c].len(), "{}: length", case.name);
            worst_err = worst_err.max(max_abs_diff(&ours[c], &ref_pcm[c]));
            let (a, b) = (band_db(&ours[c], case.rate), band_db(&ref_pcm[c], case.rate));
            let loudest = b.iter().copied().fold(f64::MIN, f64::max);
            for (x, y) in a.iter().zip(&b) {
                if *y > loudest - 60.0 {
                    worst_db = worst_db.max((x - y).abs());
                }
            }
        }
        println!("{:<30} {:>12.3e} {:>14.2}", case.name, worst_err, worst_db);
        if worst_db > 1.0 {
            fails.push((case.name, worst_db));
        }
    }
    assert!(fails.is_empty(), "band power mismatches (dB): {fails:?}");
}
