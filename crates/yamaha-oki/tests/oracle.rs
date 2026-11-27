//! Compare the decoders with ffmpeg's `adpcm_yamaha` and `adpcm_ima_oki` on random nibble streams
//! (every code sequence is valid) and on ffmpeg-encoded Yamaha ADPCM.

use cfa4_yamaha_oki::{Kind, State};
use std::path::Path;
use std::process::Command;

fn wav(tag: u16, channels: u16, rate: u32, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&tag.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * u32::from(channels) / 2).to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    v
}

fn noise(n: usize, mut s: u32) -> Vec<u8> {
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 24) as u8
        })
        .collect()
}

fn ffmpeg_pcm(ff: &Path, file: &Path, channels: u16) -> Option<Vec<Vec<i16>>> {
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-f", "s16le", "-"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let all: Vec<i16> = out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let ch = usize::from(channels);
    Some((0..ch).map(|c| all.iter().skip(c).step_by(ch).copied().collect()).collect())
}

fn mine(kind: Kind, channels: usize, data: &[u8]) -> Vec<Vec<i16>> {
    let mut st = State::new(kind, channels).unwrap();
    let mut out = vec![Vec::new(); channels];
    st.decode(data, &mut out);
    out
}

#[test]
fn random_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("yamaha-oki") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-yo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (kind, tag) in [(Kind::Yamaha, 0x20u16), (Kind::Oki, 0x10)] {
        for channels in [1u16, 2] {
            let data = noise(20_000, u32::from(tag) + u32::from(channels));
            let f = dir.join(format!("{tag}-{channels}.wav"));
            std::fs::write(&f, wav(tag, channels, 22_050, &data)).unwrap();
            let Some(want) = ffmpeg_pcm(&ff, &f, channels) else {
                eprintln!("skip: this ffmpeg does not decode WAV tag {tag:#x}");
                continue;
            };
            assert_eq!(mine(kind, usize::from(channels), &data), want, "{kind:?} {channels} channels");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffmpeg_encoded_yamaha_matches() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("yamaha-oki") else { return };
    if !cfa4_testkit::oracle::ffmpeg_has_encoder(&ff, "adpcm_yamaha") {
        eprintln!("skip: no adpcm_yamaha encoder");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cfa4-yo-enc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for channels in [1u16, 2] {
        let f = dir.join(format!("enc{channels}.wav"));
        let ok = Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1:sample_rate=22050", "-ac"])
            .arg(channels.to_string())
            .args(["-c:a", "adpcm_yamaha"])
            .arg(&f)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg encode");
        let bytes = std::fs::read(&f).unwrap();
        let pos = bytes.windows(4).position(|w| w == b"data").unwrap();
        let data = &bytes[pos + 8..];
        let want = ffmpeg_pcm(&ff, &f, channels).unwrap();
        assert_eq!(mine(Kind::Yamaha, usize::from(channels), data), want);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn damaged_input_never_panics() {
    for kind in [Kind::Yamaha, Kind::Oki] {
        for channels in 1..=2 {
            let out = mine(kind, channels, &noise(5000, 7));
            assert_eq!(out.iter().map(Vec::len).sum::<usize>(), 10_000);
        }
    }
    assert!(State::new(Kind::Oki, 3).is_none());
}
