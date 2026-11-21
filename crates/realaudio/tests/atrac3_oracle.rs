//! ATRAC3: ffmpeg's FATE samples `atrac3/mc_sich_at3_{066,105,132}_small.wav` (from
//! <https://fate-suite.ffmpeg.org/atrac3/>, in `target/fixtures/atrac3` when present locally;
//! ffmpeg has no ATRAC3 encoder), decoded by us and by ffmpeg's `atrac3` decoder.

use std::process::Command;

use cfa4_realaudio::atrac3::{Decoder, Params};

/// (channels, block_align, fmt extra data, data chunk) of a WAV file.
fn wav(d: &[u8]) -> (u16, usize, Vec<u8>, Vec<u8>) {
    let mut pos = 12;
    let (mut ch, mut ba, mut extra, mut data) = (0, 0, Vec::new(), Vec::new());
    while pos + 8 <= d.len() {
        let id = &d[pos..pos + 4];
        let len = u32::from_le_bytes([d[pos + 4], d[pos + 5], d[pos + 6], d[pos + 7]]) as usize;
        let body = &d[pos + 8..(pos + 8 + len).min(d.len())];
        if id == b"fmt " {
            ch = u16::from_le_bytes([body[2], body[3]]);
            ba = usize::from(u16::from_le_bytes([body[12], body[13]]));
            let cb = usize::from(u16::from_le_bytes([body[16], body[17]]));
            extra = body[18..18 + cb].to_vec();
        } else if id == b"data" {
            data = body.to_vec();
        }
        pos += 8 + len + (len & 1);
    }
    (ch, ba, extra, data)
}

fn compare(name: &str) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("atrac3 FATE") else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("atrac3").join(name);
    let Ok(bytes) = std::fs::read(&f) else { return };
    let (ch, ba, extra, data) = wav(&bytes);
    let mut dec = Decoder::new(&Params { channels: ch, block_align: ba, extradata: extra }).unwrap();
    let c = usize::from(ch);
    let mut ours = Vec::new();
    for fr in data.chunks_exact(ba) {
        let p = dec.decode(fr).unwrap();
        for i in 0..p[0].len() {
            for chan in &p {
                ours.push(chan[i]);
            }
        }
    }
    let out = Command::new(&ff).args(["-v", "error", "-flags", "+bitexact", "-i"]).arg(&f).args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]).output().unwrap();
    let theirs: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert!(theirs.len() > 10_000, "{name}: ffmpeg decoded {}", theirs.len());
    assert_eq!(ours.len(), theirs.len(), "{name}: sample count ({c} channels)");
    let (mut worst, mut at) = (0f32, 0);
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        if (a - b).abs() > worst {
            worst = (a - b).abs();
            at = i;
        }
    }
    eprintln!("{name}: {} samples, worst {worst:e} at {at}", ours.len());
    assert!(worst < 1e-6, "{name}: sample {at}: ours {} ffmpeg {}", ours[at], theirs[at]);
}

#[test]
fn fate_066() {
    compare("mc_sich_at3_066_small.wav");
}

#[test]
fn fate_105() {
    compare("mc_sich_at3_105_small.wav");
}

#[test]
fn fate_132() {
    compare("mc_sich_at3_132_small.wav");
}
