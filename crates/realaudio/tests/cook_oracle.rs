//! Cook: ffmpeg's FATE sample `real/ra_cook.rm` (from <https://fate-suite.ffmpeg.org/real/>, in
//! `target/fixtures/real` when present locally; ffmpeg has no Cook encoder), deinterleaved by
//! `cfa4-realmedia` and decoded by us, against ffmpeg's `cook` decoder: the same number of
//! samples, each within rounding of ffmpeg's (its transform runs in `f32` with SIMD code, ours in
//! `f64`).

use std::path::Path;
use std::process::Command;

use cfa4_realaudio::cook::{Decoder, Params};
use cfa4_realmedia::audio::{Deinterleaver, parse_audio_info};
use cfa4_realmedia::{Demuxer, StreamKind};

/// Our samples, interleaved, and the channel count.
fn decode_ours(data: &[u8]) -> (Vec<f32>, usize) {
    let mut dm = Demuxer::new(std::io::Cursor::new(data)).unwrap();
    let (si, info) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Audio { codec_data } => parse_audio_info(codec_data).ok().filter(|a| &a.fourcc == b"cook").map(|a| (i, a)),
            _ => None,
        })
        .expect("no Cook stream");
    let mut dec = Decoder::new(&Params {
        channels: info.channels,
        sample_rate: info.sample_rate,
        block_align: info.block_align as usize,
        extradata: info.extradata.clone(),
    })
    .unwrap();
    let mut di = Deinterleaver::new(&info);
    let mut out = Vec::new();
    while let Ok(Some(p)) = dm.next_packet() {
        if p.stream != si {
            continue;
        }
        for f in di.push(&p) {
            if let Ok(Some(fr)) = dec.decode(&f.data) {
                let n = fr.channels.first().map_or(0, Vec::len);
                for i in 0..n {
                    for c in &fr.channels {
                        out.push(c[i]);
                    }
                }
            }
        }
    }
    (out, usize::from(info.channels))
}

fn ffmpeg_samples(ff: &Path, file: &Path) -> Vec<f32> {
    let out = Command::new(ff)
        .args(["-v", "error", "-flags", "+bitexact", "-i"])
        .arg(file)
        .args(["-vn", "-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .output()
        .unwrap();
    out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn compare(name: &str) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cook FATE") else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("real").join(name);
    let Ok(data) = std::fs::read(&f) else { return };
    let (ours, ch) = decode_ours(&data);
    let theirs = ffmpeg_samples(&ff, &f);
    assert!(theirs.len() > 1000, "ffmpeg decoded {} samples", theirs.len());
    assert_eq!(ours.len(), theirs.len(), "sample count ({ch} channels)");
    let mut worst = 0f32;
    let mut at = 0;
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        let d = (a - b).abs();
        if d > worst {
            worst = d;
            at = i;
        }
    }
    if let Some(i) = ours.iter().zip(&theirs).position(|(a, b)| (a - b).abs() > 1e-4) {
        eprintln!("first diff at {i} (frame {}): {:?} vs {:?}", i / (ch * 1024), &ours[i..i + 6], &theirs[i..i + 6]);
    }
    eprintln!("{name}: {} samples, worst difference {worst:e} at {at}", ours.len());
    assert!(worst < 1e-6, "worst difference {worst} at sample {at}: ours {}, ffmpeg {}", ours[at], theirs[at]);
}

#[test]
fn fate_cook_stereo_matches_ffmpeg() {
    compare("ra_cook.rm");
}

#[test]
fn fate_cook_mono_in_rv30_file_matches_ffmpeg() {
    compare("rv30.rm");
}

#[test]
fn fate_cook_in_rv40_file_matches_ffmpeg() {
    compare("spygames-2MB.rmvb");
}

fn rng(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// Random byte changes in the frames of the FATE samples: errors or audio, never a panic.
#[test]
fn damaged_frames_do_not_panic() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for name in ["ra_cook.rm", "rv30.rm"] {
        let Ok(data) = std::fs::read(cfa4_testkit::fixtures::fixtures_dir("real").join(name)) else { continue };
        let mut dm = Demuxer::new(std::io::Cursor::new(&data[..])).unwrap();
        let (si, info) = dm
            .header()
            .streams
            .iter()
            .enumerate()
            .find_map(|(i, s)| match &s.kind {
                StreamKind::Audio { codec_data } => parse_audio_info(codec_data).ok().map(|a| (i, a)),
                _ => None,
            })
            .unwrap();
        let params =
            Params { channels: info.channels, sample_rate: info.sample_rate, block_align: info.block_align as usize, extradata: info.extradata.clone() };
        let mut di = Deinterleaver::new(&info);
        let mut frames = Vec::new();
        while let Ok(Some(p)) = dm.next_packet() {
            if p.stream == si {
                frames.extend(di.push(&p).into_iter().map(|f| f.data));
            }
            if frames.len() > 400 {
                break;
            }
        }
        for _ in 0..20 {
            let mut dec = Decoder::new(&params).unwrap();
            for f in &frames {
                let mut q = f.clone();
                for _ in 0..1 + rng(&mut seed) % 6 {
                    let at = (rng(&mut seed) as usize) % q.len().max(1);
                    if let Some(b) = q.get_mut(at) {
                        *b = rng(&mut seed) as u8;
                    }
                }
                if rng(&mut seed).is_multiple_of(9) {
                    q.truncate((rng(&mut seed) as usize) % (q.len() + 1));
                }
                if let Ok(Some(fr)) = dec.decode(&q) {
                    assert!(fr.channels.iter().flatten().all(|s| s.is_finite() && s.abs() <= 1.0));
                }
            }
        }
    }
}

/// Random stream parameters and frames: errors, never a panic.
#[test]
fn random_streams_do_not_panic() {
    let mut seed = 7u64;
    for _ in 0..3000 {
        let n = (rng(&mut seed) % 48) as usize;
        let mut extradata: Vec<u8> = (0..n).map(|_| rng(&mut seed) as u8).collect();
        // often a plausible record, so decoding gets going
        if rng(&mut seed).is_multiple_of(2) && extradata.len() >= 16 {
            let v: u32 = [0x0100_0001, 0x0100_0002, 0x0100_0003, 0x0200_0000][(rng(&mut seed) % 4) as usize];
            extradata[..4].copy_from_slice(&v.to_be_bytes());
            extradata[4..6].copy_from_slice(&[256u16, 512, 1024, 2048][(rng(&mut seed) % 4) as usize].to_be_bytes());
            extradata[6..8].copy_from_slice(&((rng(&mut seed) % 60) as u16).to_be_bytes());
        }
        let params = Params { channels: (rng(&mut seed) % 4) as u16, sample_rate: 44_100, block_align: (rng(&mut seed) % 700) as usize, extradata };
        let Ok(mut dec) = Decoder::new(&params) else { continue };
        for _ in 0..6 {
            let len = (rng(&mut seed) % 800) as usize;
            let f: Vec<u8> = (0..len).map(|_| rng(&mut seed) as u8).collect();
            if let Ok(Some(fr)) = dec.decode(&f) {
                assert!(fr.channels.iter().flatten().all(|s| s.is_finite()));
            }
        }
    }
    for _ in 0..3000 {
        let n = (rng(&mut seed) % 120) as usize;
        let mut d: Vec<u8> = (0..n).map(|_| rng(&mut seed) as u8).collect();
        if d.len() >= 6 {
            d[..4].copy_from_slice(b".ra\xfd");
            d[4..6].copy_from_slice(&[3u16, 4, 5][(rng(&mut seed) % 3) as usize].to_be_bytes());
        }
        if let Ok(info) = parse_audio_info(&d) {
            let mut di = Deinterleaver::new(&info);
            for _ in 0..4 {
                let len = (rng(&mut seed) % 300) as usize;
                let p = cfa4_realmedia::Packet {
                    stream: 0,
                    timestamp_ms: 0,
                    keyframe: rng(&mut seed).is_multiple_of(2),
                    pos: 0,
                    partial: false,
                    data: (0..len).map(|_| rng(&mut seed) as u8).collect(),
                };
                let _ = di.push(&p);
            }
        }
    }
}
