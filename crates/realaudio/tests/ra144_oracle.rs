//! RealAudio 1.0 (14.4): streams that ffmpeg's `real_144` encoder writes into RealMedia files
//! (so the test runs wherever ffmpeg does), and ffmpeg's FATE sample `real/ra3_in_rm_file.rm`
//! (from <https://fate-suite.ffmpeg.org/real/>, in `target/fixtures/real` when present locally),
//! deinterleaved by `cfa4-realmedia` and decoded by us, against ffmpeg's `ra_144` decoder: the
//! same 16-bit samples, bit for bit.

use std::path::Path;
use std::process::Command;

use cfa4_realaudio::ra144::{Decoder, FRAME_SIZE};
use cfa4_realmedia::audio::{Deinterleaver, parse_audio_info};
use cfa4_realmedia::{Demuxer, StreamKind};

fn decode_ours(data: &[u8]) -> Vec<i16> {
    let mut dm = Demuxer::new(std::io::Cursor::new(data)).unwrap();
    let (si, info) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Audio { codec_data } => parse_audio_info(codec_data).ok().filter(|a| &a.fourcc == b"lpcJ").map(|a| (i, a)),
            _ => None,
        })
        .expect("no RealAudio 1.0 stream");
    let mut dec = Decoder::new();
    let mut di = Deinterleaver::new(&info);
    let mut out = Vec::new();
    while let Ok(Some(p)) = dm.next_packet() {
        if p.stream != si {
            continue;
        }
        for f in di.push(&p) {
            for frame in f.data.chunks_exact(FRAME_SIZE) {
                out.extend_from_slice(&dec.decode(frame).unwrap());
            }
        }
    }
    out
}

fn ffmpeg_samples(ff: &Path, file: &Path) -> Vec<i16> {
    let out = Command::new(ff)
        .args(["-v", "error", "-flags", "+bitexact", "-i"])
        .arg(file)
        .args(["-vn", "-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le", "-"])
        .output()
        .unwrap();
    out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

fn compare(ff: &Path, file: &Path) {
    let data = std::fs::read(file).unwrap();
    let ours = decode_ours(&data);
    let theirs = ffmpeg_samples(ff, file);
    assert!(theirs.len() > 1000, "ffmpeg decoded {} samples", theirs.len());
    assert_eq!(ours.len(), theirs.len(), "sample count");
    if let Some(i) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
        panic!("{}: sample {i} differs: ours {} ffmpeg {}", file.display(), ours[i], theirs[i]);
    }
}

#[test]
fn ffmpeg_encoded_streams_match() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("ra144") else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("ra144");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in [
        ("sine.rm", "sine=frequency=440:sample_rate=8000:duration=3"),
        ("noise.rm", "anoisesrc=color=pink:sample_rate=8000:duration=3:amplitude=0.8:seed=7"),
        ("chirp.rm", "aevalsrc=0.9*sin(2*PI*(100+600*t)*t):s=8000:d=3"),
        ("quiet.rm", "aevalsrc=0.002*sin(2*PI*300*t):s=8000:d=2"),
    ] {
        let f = dir.join(name);
        let st = Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", src, "-ac", "1", "-ar", "8000", "-c:a", "real_144", "-f", "rm"])
            .arg(&f)
            .status()
            .unwrap();
        assert!(st.success(), "ffmpeg could not encode {name}");
        compare(&ff, &f);
    }
}

#[test]
fn fate_ra3_in_rm_file_matches() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("ra144 FATE") else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("real").join("ra3_in_rm_file.rm");
    if f.exists() {
        compare(&ff, &f);
    }
}

#[test]
fn random_frames_do_not_panic() {
    let mut dec = Decoder::new();
    let mut x = 0x1234_5678u32;
    for _ in 0..20_000 {
        let mut f = [0u8; FRAME_SIZE];
        for b in &mut f {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
        let _ = dec.decode(&f);
    }
    assert!(dec.decode(&[0; 3]).is_err());
}
