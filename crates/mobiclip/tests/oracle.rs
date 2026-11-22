//! `cfa4-mobiclip` against ffmpeg's `moflex` demuxer, `mobiclip` video decoder and `fastaudio`
//! decoder (the oracle only). ffmpeg has no MobiClip encoder, so the real FATE sample
//! `mobiclip/mobiclip-reference-rect-partitions.moflex` is the input; it sits in
//! `target/fixtures/mobiclip/fate/` (fetched from https://fate-suite.ffmpeg.org/mobiclip/) and the
//! tests skip when it is not there.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_mobiclip::{AudioCodec, Decoder, FastAudio};
use cfa4_testkit::{fixtures, oracle};

const SAMPLE: &str = "mobiclip-reference-rect-partitions.moflex";

fn sample() -> Option<PathBuf> {
    let p = fixtures::fixtures_dir("mobiclip").join("fate").join(SAMPLE);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipping: FATE sample {} is not in target/fixtures/mobiclip/fate", SAMPLE);
        None
    }
}

fn ffmpeg_out(ff: &Path, file: &Path, args: &[&str], name: &str) -> Vec<u8> {
    let out = fixtures::fixtures_dir("mobiclip").join(name);
    let ok = Command::new(ff).args(["-v", "error", "-y", "-i"]).arg(file).args(args).arg(&out).status().is_ok_and(|s| s.success());
    assert!(ok, "ffmpeg failed for {name}");
    std::fs::read(&out).unwrap()
}

#[test]
fn video_matches_ffmpeg() {
    let Some(file) = sample() else { return };
    let Some(ff) = oracle::ffmpeg_or_skip("mobiclip oracle") else { return };
    let theirs = ffmpeg_out(&ff, &file, &["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "yuv420p"], "video.raw");
    let data = std::fs::read(&file).unwrap();
    let m = cfa4_mobiclip::parse(&data).unwrap();
    assert!(!m.damaged);
    assert_eq!((m.width, m.height), (320, 240));
    assert_eq!((m.fps_num, m.fps_den), (25, 1));
    let mut dec = Decoder::new(m.width, m.height).unwrap();
    let mut ours = Vec::new();
    for (i, p) in m.video.iter().enumerate() {
        let f = dec.decode(&p.data).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        assert_eq!(f.key, p.key, "frame {i}: key flag");
        for pl in &f.planes {
            ours.extend_from_slice(pl);
        }
    }
    assert_eq!(ours.len(), theirs.len(), "size ({} frames)", m.video.len());
    if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
        let frame = 320 * 240 * 3 / 2;
        panic!("first difference at byte {at}: frame {}, offset {}", at / frame, at % frame);
    }
}

#[test]
fn sound_matches_ffmpeg() {
    let Some(file) = sample() else { return };
    let Some(ff) = oracle::ffmpeg_or_skip("mobiclip oracle") else { return };
    let theirs = ffmpeg_out(&ff, &file, &["-map", "0:a:0", "-f", "f32le"], "audio.raw");
    let theirs: Vec<f32> = theirs.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let data = std::fs::read(&file).unwrap();
    let m = cfa4_mobiclip::parse(&data).unwrap();
    let a = m.audio.expect("audio");
    assert_eq!((a.codec, a.channels, a.sample_rate), (AudioCodec::FastAudio, 2, 32000));
    let mut dec = FastAudio::new(2);
    let mut ours = Vec::new();
    for p in &a.packets {
        let out = dec.decode(p);
        for i in 0..out[0].len() {
            ours.push(out[0][i]);
            ours.push(out[1][i]);
        }
    }
    assert_eq!(ours.len(), theirs.len());
    let worst = ours.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    let peak = theirs.iter().fold(0f32, |m, v| m.max(v.abs()));
    eprintln!("sound: {} values, worst difference {worst:e}, peak {peak}", ours.len());
    assert!(worst <= 1e-4 * peak.max(1e-3), "worst difference {worst} (peak {peak})");
}

#[test]
fn damaged_input_does_not_panic() {
    let Some(file) = sample() else { return };
    let data = std::fs::read(&file).unwrap();
    let mut seed = 0x9E37_79B9u32;
    for round in 0..60 {
        let mut d = data.clone();
        for _ in 0..(1 + round % 9) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let at = (seed >> 8) as usize % d.len();
            d[at] ^= (seed >> 3) as u8 | 1;
        }
        if round % 7 == 0 {
            d.truncate(d.len() / 2 + round);
        }
        let Ok(m) = cfa4_mobiclip::parse(&d) else { continue };
        let mut dec = Decoder::new(m.width, m.height);
        if let Ok(dec) = dec.as_mut() {
            for p in m.video.iter().take(30) {
                let _ = dec.decode(&p.data);
            }
        }
        if let Some(a) = &m.audio {
            let mut fa = FastAudio::new(a.channels as usize);
            for p in &a.packets {
                match a.codec {
                    AudioCodec::FastAudio => drop(fa.decode(p)),
                    AudioCodec::ImaMoflex => drop(cfa4_mobiclip::decode_ima(p, a.channels as usize)),
                    AudioCodec::PcmS16le => drop(cfa4_mobiclip::decode_pcm(p, a.channels as usize)),
                }
            }
        }
    }
}

/// Random bit flips in the packets drive the decoder through syntax the one real sample does not
/// use. Whenever ffmpeg decodes a damaged file without a complaint and so do we, every picture
/// must be the same.
#[test]
fn mutated_pictures_match_ffmpeg() {
    let Some(file) = sample() else { return };
    let Some(ff) = oracle::ffmpeg_or_skip("mobiclip oracle") else { return };
    let data = std::fs::read(&file).unwrap();
    let m = cfa4_mobiclip::parse(&data).unwrap();
    let (mut compared, mut skipped, mut seed) = (0, 0, 0x1234_5678u32);
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed >> 8
    };
    // offsets of the video packets in the file: search the packet bytes in the file
    let mut spans = Vec::new();
    for p in &m.video {
        if let Some(at) = data.windows(p.data.len().min(24)).position(|w| w == &p.data[..p.data.len().min(24)]) {
            spans.push((at, p.data.len()));
        }
    }
    for round in 0..150 {
        let mut d = data.clone();
        let (at, len) = spans[(next() as usize) % spans.len()];
        for _ in 0..(1 + next() % 6) {
            let o = at + 2 + next() as usize % len.saturating_sub(2).max(1);
            d[o] ^= 1 << (next() % 8);
        }
        let path = fixtures::fixtures_dir("mobiclip").join(format!("mut-{round}.moflex"));
        std::fs::write(&path, &d).unwrap();
        let out = fixtures::fixtures_dir("mobiclip").join(format!("mut-{round}.raw"));
        let st = Command::new(&ff)
            .args(["-v", "error", "-y", "-i"])
            .arg(&path)
            .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "yuv420p"])
            .arg(&out)
            .output()
            .unwrap();
        let theirs = std::fs::read(&out).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&out);
        let mm = cfa4_mobiclip::parse(&d).unwrap();
        let mut dec = Decoder::new(mm.width, mm.height).unwrap();
        let mut ours = Vec::new();
        let mut ok = true;
        for p in &mm.video {
            match dec.decode(&p.data) {
                Ok(f) => f.planes.iter().for_each(|pl| ours.extend_from_slice(pl)),
                Err(_) => ok = false,
            }
        }
        if !st.status.success() || !st.stderr.is_empty() || !ok || theirs.is_empty() {
            skipped += 1;
            continue;
        }
        compared += 1;
        if ours != theirs {
            let at = ours.iter().zip(&theirs).position(|(a, b)| a != b);
            panic!("round {round}: pictures differ (sizes {} / {}, first difference {at:?})", ours.len(), theirs.len());
        }
    }
    eprintln!("mutated: {compared} compared, {skipped} skipped");
    assert!(compared > 10);
}
