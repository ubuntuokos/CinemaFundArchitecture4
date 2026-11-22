//! Compare the decoder with ffmpeg's `vqf` demuxer and `twinvq` decoder on FFmpeg's FATE sample
//! (`vqf/achterba.vqf`, fetched into `target/fixtures/vqf/fate/`; the test skips without it).

use std::process::Command;

fn sample() -> Option<Vec<u8>> {
    let p = cfa4_testkit::fixtures::fixtures_dir("vqf/fate").join("achterba.vqf");
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("skip: {} not fetched (https://fate-suite.ffmpeg.org/vqf/achterba.vqf)", p.display());
            None
        }
    }
}

fn ffmpeg_f32(file: &std::path::Path) -> Option<Vec<f32>> {
    let ff = cfa4_testkit::oracle::ffmpeg_or_skip("twinvq")?;
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-f", "f32le", "-"]).output().ok()?;
    Some(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

#[test]
fn fate_sample_matches_ffmpeg() {
    let Some(data) = sample() else { return };
    let s = cfa4_twinvq::parse(&data).unwrap();
    eprintln!("{s:?}");
    let d = cfa4_twinvq::decode(&data, &s);
    let path = cfa4_testkit::fixtures::fixtures_dir("vqf/fate").join("achterba.vqf");
    let Some(want) = ffmpeg_f32(&path) else { return };
    let ch = s.channels;
    assert_eq!(d.channels.len(), ch);
    assert_eq!(d.channels[0].len() * ch, want.len(), "sample count");
    let mut worst = 0.0f32;
    for (i, w) in want.iter().enumerate() {
        let m = d.channels[i % ch][i / ch];
        worst = worst.max((m - w).abs());
    }
    eprintln!("worst difference {worst}");
    assert!(worst < 1e-4, "worst difference {worst}");
}

#[test]
fn damaged_input_never_panics() {
    let Some(data) = sample() else { return };
    for cut in [20usize, 100, 1000, 40_000] {
        if let Ok(s) = cfa4_twinvq::parse(&data[..cut.min(data.len())]) {
            let _ = cfa4_twinvq::decode(&data[..cut.min(data.len())], &s);
        }
    }
    let s = cfa4_twinvq::parse(&data).unwrap();
    let mut bad = data.clone();
    for (i, b) in bad.iter_mut().enumerate().skip(s.data_start) {
        *b = b.wrapping_mul(31).wrapping_add(i as u8);
    }
    let _ = cfa4_twinvq::decode(&bad[..s.data_start + 200_000], &s);
}

/// A VQF file of `frames` frames of random bits (every frame's window type valid) for one mode.
fn synthetic(rate_flag: u32, kbps: u32, channels: u32, frames: usize, seed: u32) -> Vec<u8> {
    let rate = match rate_flag {
        44 => 44_100,
        22 => 22_050,
        11 => 11_025,
        r => r * 1000,
    };
    let size = match (rate_flag, kbps) {
        (8, 8) | (11, 8) | (11, 10) | (22, 32) => 512,
        (16, 16) | (22, 20) | (22, 24) => 1024,
        _ => 2048,
    };
    let frame_bits = (u64::from(kbps) * 1000 * u64::from(channels) * size / u64::from(rate)) as usize;
    let mut x = seed;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut bits: Vec<bool> = Vec::new();
    for _ in 0..frames {
        let wt = next() % 9;
        for b in (0..4).rev() {
            bits.push((wt >> b) & 1 == 1);
        }
        for _ in 4..frame_bits {
            bits.push(next() >> 16 & 1 == 1);
        }
    }
    let mut data = vec![0u8; bits.len().div_ceil(8) + 4];
    for (i, b) in bits.iter().enumerate() {
        if *b {
            data[i >> 3] |= 0x80 >> (i & 7);
        }
    }
    let mut f = b"TWIN97012000".to_vec();
    f.extend_from_slice(&20u32.to_be_bytes());
    f.extend_from_slice(b"COMM");
    f.extend_from_slice(&12u32.to_be_bytes());
    f.extend_from_slice(&(channels - 1).to_be_bytes());
    f.extend_from_slice(&(kbps * channels).to_be_bytes());
    f.extend_from_slice(&rate_flag.to_be_bytes());
    f.extend_from_slice(b"DATA");
    f.extend_from_slice(&data);
    f
}

#[test]
fn synthetic_streams_in_every_mode_match_ffmpeg() {
    let Some(_) = cfa4_testkit::oracle::ffmpeg_or_skip("twinvq") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-twinvq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let modes = [(8, 8), (11, 8), (11, 10), (16, 16), (22, 20), (22, 24), (22, 32), (44, 40), (44, 48)];
    for (rate_flag, kbps) in modes {
        for channels in [1u32, 2] {
            let file = synthetic(rate_flag, kbps, channels, 40, rate_flag * 100 + kbps + channels);
            let path = dir.join(format!("{rate_flag}-{kbps}-{channels}.vqf"));
            std::fs::write(&path, &file).unwrap();
            let s = cfa4_twinvq::parse(&file).unwrap();
            let d = cfa4_twinvq::decode(&file, &s);
            let Some(want) = ffmpeg_f32(&path) else { continue };
            let ch = channels as usize;
            assert_eq!(d.channels[0].len() * ch, want.len(), "{rate_flag} kHz {kbps} kbit/s {ch} ch: sample count");
            let peak = want.iter().fold(0.0f32, |a, v| a.max(v.abs())).max(1e-6);
            let worst = want.iter().enumerate().fold(0.0f32, |a, (i, w)| a.max((d.channels[i % ch][i / ch] - w).abs()));
            eprintln!("{rate_flag} kHz {kbps} kbit/s {ch} ch: worst {worst:e} of peak {peak:e}");
            assert!(worst <= peak * 1e-5, "{rate_flag} kHz {kbps} kbit/s {ch} ch: worst {worst} of peak {peak}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
