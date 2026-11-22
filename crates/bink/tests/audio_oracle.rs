//! The Bink audio decoders against ffmpeg's `binkaudio_rdft` and `binkaudio_dct`, on streams
//! written by a small encoder here (random coefficients through every code path: float or raw
//! header values, run-length groups of every width, all channel counts). The transforms are
//! float, so the comparison is a signal-to-noise bound, not bit equality.

mod common;

use std::path::Path;
use std::process::Command;

use cfa4_bink::audio::Decoder;
use cfa4_bink::container::Bink;
use common::{Params, Rng, Track, bik, packet};

fn ffmpeg_audio(ff: &Path, file: &[u8], name: &str) -> Vec<f32> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("bink/oracle");
    let path = dir.join(format!("{name}-{}.bik", std::process::id()));
    std::fs::write(&path, file).unwrap();
    let out = Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(&path)
        .args(["-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .output()
        .expect("run ffmpeg");
    let _ = std::fs::remove_file(&path);
    out.stdout.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn check(ff: &Path, p: &Params, seed: u64) {
    let mut rng = Rng(seed);
    let frames = 6;
    let flags: u16 = if p.channels > 1 { 0x2000 } else { 0 } | if p.dct { 0x1000 } else { 0 } | 0x4000;
    let packets: Vec<Vec<u8>> = (0..frames)
        .map(|_| {
            let blocks = 1 + rng.below(3) as usize;
            packet(&mut rng, p, blocks)
        })
        .collect();
    let video: Vec<Vec<u8>> = (0..frames).map(|_| vec![0u8; 8]).collect();
    let tag = [b'B', b'I', b'K', p.revision];
    let file = bik(&tag, 16, 16, 0, &video, &[Track { rate: p.rate as u16, flags, packets }]);
    let bink = Bink::parse(&file).expect("parse");
    let t = &bink.tracks[0];
    assert_eq!((t.sample_rate, t.channels, t.dct), (p.rate, p.channels.min(2), p.dct));
    let mut dec = Decoder::new(p.rate, p.channels.min(2), p.dct, p.revision).unwrap();
    let mut ours = Vec::new();
    for i in 0..bink.frame_count() {
        for (_, pkt) in bink.frame(i).unwrap().audio {
            ours.extend(dec.decode(pkt).expect("decode"));
        }
    }
    let theirs = ffmpeg_audio(ff, &file, &format!("a{seed}"));
    let label = format!("rate {} ch {} dct {} rev {}", p.rate, p.channels, p.dct, p.revision as char);
    assert_eq!(ours.len(), theirs.len(), "{label}: sample counts");
    let sig: f64 = theirs.iter().map(|&v| (v as f64).powi(2)).sum();
    let err: f64 = ours.iter().zip(&theirs).map(|(&a, &b)| ((a - b) as f64).powi(2)).sum();
    let snr = 10.0 * (sig / err.max(1e-30)).log10();
    eprintln!("{label}: {} samples, SNR {snr:.1} dB", ours.len());
    assert!(snr > 100.0, "{label}: SNR {snr:.1} dB");
}

#[test]
fn rdft_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("bink audio oracle") else { return };
    for (rate, channels, revision) in
        [(44100, 1, b'i'), (44100, 2, b'i'), (22050, 2, b'k'), (11025, 1, b'f'), (32000, 2, b'h'), (44100, 2, b'b'), (22050, 1, b'b')]
    {
        check(&ff, &Params { rate, channels, dct: false, revision }, 99 + rate as u64);
    }
}

#[test]
fn dct_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("bink audio oracle") else { return };
    for (rate, channels, revision) in [(44100, 1, b'i'), (44100, 2, b'i'), (22050, 2, b'k'), (11025, 1, b'i'), (48000, 2, b'h')] {
        check(&ff, &Params { rate, channels, dct: true, revision }, 7 + rate as u64);
    }
}
