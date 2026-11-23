//! A QuickTime file with QDesign Music 2 sound read through the media source: the samples equal
//! the FFmpeg FATE reference PCM, also when read from the middle. Skips without the sample.

use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn qdm2_in_a_mov_matches_the_fate_reference() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/qdm2/fate");
    let (Ok(mov), Ok(pcm)) = (std::fs::read(dir.join("subpacket12.mov")), std::fs::read(dir.join("subpacket12.pcm"))) else {
        eprintln!("skipped: qdm2 FATE sample not fetched");
        return;
    };
    let want: Vec<f32> = pcm.chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0).collect();
    let src = cfa4_codecs::open_bytes("subpacket12.mov", Arc::from(mov)).unwrap();
    let a = src.info().audio().unwrap().clone();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (44100, 1, "QDesign Music 2"));

    let all = src.audio(0, want.len(), 44100).unwrap();
    let bad = all.channels[0].iter().zip(&want).position(|(a, b)| a != b);
    let worst = all.channels[0].iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(bad.is_none(), "first difference at {bad:?}, worst {worst}; got {} samples", all.channels[0].len());
    // from the middle: a packet is 2048 samples; the decoder state of the packets before the
    // first one read is rebuilt from one packet of pre-roll
    let mid = src.audio(6000, 3000, 44100).unwrap();
    let diff = mid.channels[0].iter().zip(&want[6000..9000]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(diff < 0.05, "read from the middle differs by {diff}");
}
