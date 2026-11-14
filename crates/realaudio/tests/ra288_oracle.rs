//! RealAudio 2.0 (28.8): ffmpeg's FATE sample `real/ra_288.rm` (from
//! <https://fate-suite.ffmpeg.org/real/>, in `target/fixtures/real` when present locally; ffmpeg
//! has no 28.8 encoder), deinterleaved by `cfa4-realmedia` and decoded by us, against ffmpeg's
//! `real_288` decoder: the same `f32` samples, bit for bit.

use std::process::Command;

use cfa4_realaudio::ra288::{Decoder, FRAME_SIZE};
use cfa4_realmedia::audio::{Deinterleaver, parse_audio_info};
use cfa4_realmedia::{Demuxer, StreamKind};

#[test]
fn fate_ra_288_matches() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("ra288 FATE") else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("real").join("ra_288.rm");
    let Ok(data) = std::fs::read(&f) else { return };
    let mut dm = Demuxer::new(std::io::Cursor::new(&data[..])).unwrap();
    let (si, info) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Audio { codec_data } => parse_audio_info(codec_data).ok().filter(|a| &a.fourcc == b"28_8").map(|a| (i, a)),
            _ => None,
        })
        .expect("no RealAudio 2.0 stream");
    assert_eq!(info.block_align as usize, FRAME_SIZE);
    let mut dec = Decoder::new();
    let mut di = Deinterleaver::new(&info);
    let mut ours = Vec::new();
    while let Ok(Some(p)) = dm.next_packet() {
        if p.stream != si {
            continue;
        }
        for fr in di.push(&p) {
            ours.extend_from_slice(&dec.decode(&fr.data).unwrap());
        }
    }
    let out = Command::new(&ff)
        .args(["-v", "error", "-flags", "+bitexact", "-i"])
        .arg(&f)
        .args(["-vn", "-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .output()
        .unwrap();
    let theirs: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert!(theirs.len() > 1000, "ffmpeg decoded {} samples", theirs.len());
    assert_eq!(ours.len(), theirs.len(), "sample count");
    if let Some(i) = ours.iter().zip(&theirs).position(|(a, b)| a.to_bits() != b.to_bits()) {
        panic!("sample {i} differs: ours {} ffmpeg {}", ours[i], theirs[i]);
    }
}
