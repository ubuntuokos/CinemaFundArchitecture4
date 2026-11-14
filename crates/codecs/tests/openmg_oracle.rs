//! ATRAC3plus through the media layer (OpenMG `.oma` and WAV `.at3` with the extensible ATRAC3plus
//! sub-format) against ffmpeg's demuxers and `atrac3plus` decoder: random valid frames (CI) and
//! the FATE samples when present (`target/fixtures/atrac3p`, local only), read in order and with
//! jumps backwards across decoder checkpoints.

mod common;

use std::path::Path;

use common::*;

/// Random frames whose decoding is fully defined (the decoder's strict mode accepts them).
fn valid_frames(channels: u16, ba: usize, want: usize) -> Vec<Vec<u8>> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::new();
    while out.len() < want {
        let mut f: Vec<u8> = (0..ba).map(|_| rng.below(256) as u8).collect();
        f[0] = (f[0] & 0x1F) | if channels == 1 { 0 } else { 0x20 };
        let mut d = cfa4_realaudio::atrac3p::Decoder::new(channels, ba).unwrap();
        d.set_strict(true);
        if d.decode(&f).is_ok() {
            out.push(f);
        }
    }
    out
}

fn oma(chan_id: u32, ba: usize, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut h = vec![0u8; 96];
    h[..3].copy_from_slice(b"EA3");
    h[3] = 1;
    h[5] = 96;
    h[6] = 0xFF;
    h[7] = 0xFF;
    h[32] = 1;
    let p = 1 << 13 | chan_id << 10 | ((ba - 8) / 8) as u32;
    h[33..36].copy_from_slice(&p.to_be_bytes()[1..]);
    h.extend(frames.concat());
    h
}

/// A WAV with `WAVE_FORMAT_EXTENSIBLE` and the ATRAC3plus sub-format, as Sony's `.at3` files.
fn at3(channels: u16, ba: usize, frames: &[Vec<u8>]) -> Vec<u8> {
    let data = frames.concat();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0xFFFEu16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&44_100u32.to_le_bytes());
    fmt.extend_from_slice(&((ba * 44_100 / 2048) as u32).to_le_bytes());
    fmt.extend_from_slice(&(ba as u16).to_le_bytes());
    fmt.extend_from_slice(&0u16.to_le_bytes());
    fmt.extend_from_slice(&34u16.to_le_bytes());
    fmt.extend_from_slice(&2048u16.to_le_bytes());
    fmt.extend_from_slice(&3u32.to_le_bytes());
    fmt.extend_from_slice(&[0xBF, 0xAA, 0x23, 0xE9, 0x58, 0xCB, 0x71, 0x44, 0xA1, 0x19, 0xFF, 0xFA, 0x01, 0xE4, 0xCE, 0x62]);
    fmt.extend_from_slice(&[0x01, 0x00, 0x28, 0x5C, 0, 0, 0, 0, 0, 0, 0, 0]);
    let mut w = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    w.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    w.extend_from_slice(&fmt);
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    let riff = (w.len() - 8) as u32;
    w[4..8].copy_from_slice(&riff.to_le_bytes());
    w
}

/// Our samples through `cfa4_codecs::open_bytes` against ffmpeg's, planar, within `tol` of each
/// 2048-sample frame's peak (at least 1).
fn compare(ff: &Path, path: &Path, channels: usize, tol: f32) {
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string();
    let src = cfa4_codecs::open_bytes(&file, bytes(path)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let a = src.info().audio_streams.first().cloned().unwrap_or_else(|| panic!("{file}: no audio"));
    assert_eq!((a.codec.as_str(), a.channels, a.sample_rate), ("ATRAC3plus", channels as u32, 44_100), "{file}");
    let flat = ffmpeg_audio_f32(ff, path, &[]);
    let want: Vec<Vec<f32>> = (0..channels).map(|c| flat.iter().skip(c).step_by(channels).copied().collect()).collect();
    let n = want[0].len();
    assert_eq!(src.info().duration, cfa4_time::Tick::from_units(n as i64, 44_100), "{file}: length");
    let peaks: Vec<f32> =
        (0..n.div_ceil(2048)).map(|k| want.iter().flat_map(|w| &w[k * 2048..n.min(k * 2048 + 2048)]).fold(1f32, |m, v| m.max(v.abs()))).collect();
    let check = |start: usize, len: usize| {
        let got = src.audio(start as i64, len, 44_100).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (c, (g, w)) in got.channels.iter().zip(&want).enumerate() {
            for (i, (x, y)) in g.iter().zip(&w[start..start + len]).enumerate() {
                let at = start + i;
                assert!((x - y).abs() <= tol * peaks[at / 2048], "{file}: channel {c} sample {at}: ours {x} ffmpeg {y}");
            }
        }
    };
    for s in (0..n).step_by(30_000) {
        check(s, 30_000.min(n - s));
    }
    for s in (0..n).step_by(70_000).rev() {
        check(s, 5000.min(n - s));
    }
}

#[test]
fn random_atrac3plus_in_openmg_and_wav_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("ATRAC3plus") else { return };
    let dir = cfa4_testkit::fixtures_dir("atrac3p");
    for (chan_id, channels, ba) in [(1u32, 1u16, 376usize), (2, 2, 280)] {
        let frames = valid_frames(channels, ba, 150);
        let o = dir.join(format!("random_{channels}.oma"));
        std::fs::write(&o, oma(chan_id, ba, &frames)).unwrap();
        compare(&ff, &o, usize::from(channels), 1e-5);
        let w = dir.join(format!("random_{channels}.at3"));
        std::fs::write(&w, at3(channels, ba, &frames)).unwrap();
        compare(&ff, &w, usize::from(channels), 1e-5);
    }
}

#[test]
fn fate_atrac3plus_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("ATRAC3plus FATE") else { return };
    for name in ["at3p_sample1.oma", "sonateno14op27-2-cut.aa3"] {
        let p = cfa4_testkit::fixtures_dir("atrac3p").join(name);
        if !p.exists() {
            eprintln!("skipping: {} not present", p.display());
            continue;
        }
        compare(&ff, &p, 2, 1e-6);
    }
}
