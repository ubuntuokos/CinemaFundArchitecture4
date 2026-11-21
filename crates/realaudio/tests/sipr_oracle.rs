//! RealAudio SIPR: ffmpeg's FATE samples `sipr/sipr_{5k0,6k5,8k5,16k}.rm` (from
//! <https://fate-suite.ffmpeg.org/sipr/>, in `target/fixtures/sipr` when present locally; ffmpeg
//! has no SIPR encoder), deinterleaved by `cfa4-realmedia` and decoded by us, against ffmpeg's
//! `sipr` decoder: the same `f32` samples, bit for bit.

use std::process::Command;

use cfa4_realaudio::sipr::Decoder;
use cfa4_realmedia::audio::{Deinterleaver, parse_audio_info};
use cfa4_realmedia::{Demuxer, StreamKind};

fn compare(name: &str) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("sipr FATE") else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("sipr").join(name);
    let Ok(data) = std::fs::read(&f) else { return };
    let mut dm = Demuxer::new(std::io::Cursor::new(&data[..])).unwrap();
    let (si, info) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Audio { codec_data } => parse_audio_info(codec_data).ok().filter(|a| &a.fourcc == b"sipr").map(|a| (i, a)),
            _ => None,
        })
        .expect("no SIPR stream");
    let mut dec = Decoder::new(info.block_align as usize).unwrap();
    let mut di = Deinterleaver::new(&info);
    let mut ours = Vec::new();
    while let Ok(Some(p)) = dm.next_packet() {
        if p.stream != si {
            continue;
        }
        for fr in di.push(&p) {
            ours.extend(dec.decode(&fr.data).unwrap());
        }
    }
    let out = Command::new(&ff)
        .args(["-v", "error", "-flags", "+bitexact", "-i"])
        .arg(&f)
        .args(["-vn", "-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .output()
        .unwrap();
    let theirs: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert!(theirs.len() > 1000, "{name}: ffmpeg decoded {} samples", theirs.len());
    assert_eq!(ours.len(), theirs.len(), "{name}: sample count");
    eprintln!("{name}: {} samples compared", ours.len());
    if let Some(i) = ours.iter().zip(&theirs).position(|(a, b)| a.to_bits() != b.to_bits()) {
        panic!("{name}: sample {i} of {} differs: ours {} ffmpeg {}", ours.len(), ours[i], theirs[i]);
    }
}

#[test]
fn fate_sipr_5k0() {
    compare("sipr_5k0.rm");
}

#[test]
fn fate_sipr_6k5() {
    compare("sipr_6k5.rm");
}

#[test]
fn fate_sipr_8k5() {
    compare("sipr_8k5.rm");
}

#[test]
fn fate_sipr_16k() {
    compare("sipr_16k.rm");
}
