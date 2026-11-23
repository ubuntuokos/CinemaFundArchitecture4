//! Samples against ffmpeg's `truespeech` decoder: random frames in a WAV, and the FATE sample.

use cfa4_truespeech::{Decoder, FRAME_BYTES};
use std::process::Command;

fn ffmpeg_pcm(wav: &std::path::Path) -> Option<Vec<i16>> {
    let out = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(wav).args(["-f", "s16le", "-"]).output().ok()?;
    out.status.success().then(|| out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

fn wav(frames: &[u8]) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0x22u16.to_le_bytes()); // TrueSpeech
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&8000u32.to_le_bytes());
    fmt.extend_from_slice(&1067u32.to_le_bytes());
    fmt.extend_from_slice(&32u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&32u16.to_le_bytes()); // cbSize
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&[0u8; 30]);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((4 + 8 + fmt.len() + 8 + frames.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    out.extend_from_slice(&fmt);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    out.extend_from_slice(frames);
    out
}

#[test]
fn random_frames_match_ffmpeg() {
    let dir = std::env::temp_dir().join(format!("cfa4-truespeech-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut x = 7u32;
    // several streams, so that different frame fields are exercised
    for (n, frames) in [(1usize, 40usize), (2, 300), (3, 300)] {
        let data: Vec<u8> = (0..frames * FRAME_BYTES)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 16) as u8
            })
            .collect();
        let path = dir.join(format!("r{n}.wav"));
        std::fs::write(&path, wav(&data)).expect("write");
        let Some(want) = ffmpeg_pcm(&path) else {
            eprintln!("ffmpeg not available or cannot decode the stream; skipped");
            return;
        };
        let got = Decoder::new().decode(&data).expect("decode");
        assert_eq!(got.len(), want.len(), "stream {n}: sample count");
        let bad = got.iter().zip(&want).position(|(a, b)| a != b);
        assert_eq!(bad, None, "stream {n}: first differing sample");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fate_sample_matches_ffmpeg() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/fate/a6.wav");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("FATE sample a6.wav not fetched; skipped");
        return;
    };
    let pos = bytes.windows(4).position(|w| w == b"data").expect("data chunk");
    let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
    let data = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
    let Some(want) = ffmpeg_pcm(&path) else { return };
    let got = Decoder::new().decode(data).expect("decode");
    assert_eq!(got.len(), want.len());
    assert_eq!(got.iter().zip(&want).position(|(a, b)| a != b), None);
}
