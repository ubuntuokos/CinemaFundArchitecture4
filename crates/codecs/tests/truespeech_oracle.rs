//! TrueSpeech in WAV, AVI and Matroska (`A_MS/ACM`), read through the media sources, against
//! ffmpeg's decode of the same file. ffmpeg has no TrueSpeech encoder: the frames are random bytes
//! in a WAV that ffmpeg remuxes (`-c copy`). Every sample equal, read front to back and after a
//! jump (the decoder's memory must come back from a saved state).

mod common;

use common::*;

fn wav(frames: &[u8]) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0x22u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&8000u32.to_le_bytes());
    fmt.extend_from_slice(&1067u32.to_le_bytes());
    fmt.extend_from_slice(&32u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&32u16.to_le_bytes());
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

fn check(ff: &std::path::Path, name: &str, path: &std::path::Path) {
    let decoded: Vec<f32> = ffmpeg_out(ff, &["-i", path.to_str().unwrap(), "-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le", "-"])
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
        .collect();
    assert!(!decoded.is_empty(), "{name}: ffmpeg decoded nothing");
    let s = cfa4_codecs::open_bytes(&format!("ts_{name}"), bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    assert_eq!((info.sample_rate, info.channels), (8000, 1), "{name}");
    let read = |from: usize, n: usize| -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        let mut at = from;
        while at < from + n {
            let k = 1000.min(from + n - at);
            let b = s.audio(at as i64, k, 8000).unwrap_or_else(|e| panic!("{name}: {e}"));
            out.extend_from_slice(&b.channels[0][..k]);
            at += k;
        }
        out
    };
    let compare = |from: usize, ours: &[f32]| {
        let theirs = &decoded[from..from + ours.len()];
        if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
            panic!("{name}: sample {} differs: ours {}, ffmpeg {}", from + i, ours[i], theirs[i]);
        }
    };
    let n = decoded.len();
    compare(0, &read(0, n));
    // jumps into the middle (past a saved state), then on
    let mid = n / 2 + 777;
    compare(mid, &read(mid, n / 4));
    compare(1234, &read(1234, 5000));
}

#[test]
fn truespeech_in_containers() {
    let ff = cfa4_testkit::require_ffmpeg!();
    // enough frames for a state saved at an interval (250 packets) to be used
    let mut x = 11u32;
    let data: Vec<u8> = (0..700 * 32)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (x >> 16) as u8
        })
        .collect();
    let wav_path = dir().join("truespeech_src.wav");
    std::fs::create_dir_all(dir()).expect("dir");
    std::fs::write(&wav_path, wav(&data)).expect("write");
    check(&ff, "wav", &wav_path);
    for ext in ["avi", "mkv"] {
        let Some(path) = fixture(&ff, &format!("truespeech.{ext}"), &["-i", wav_path.to_str().unwrap(), "-c:a", "copy"]) else {
            cfa4_testkit::oracle::skip(ext, "ffmpeg can't mux TrueSpeech here");
            continue;
        };
        check(&ff, ext, &path);
    }
}
