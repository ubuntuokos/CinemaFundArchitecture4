//! QDM2 decoded against the FFmpeg FATE sample `qdm2/subpacket12.mov` and its reference PCM
//! (mono, 44.1 kHz, 8 packets): every sample is identical. The test skips when the sample is not
//! fetched.

use cfa4_qdm2::Decoder;
use std::path::PathBuf;

fn be32(d: &[u8], p: usize) -> usize {
    u32::from_be_bytes(d[p..p + 4].try_into().unwrap()) as usize
}

fn find(d: &[u8], tag: &[u8; 4]) -> usize {
    d.windows(4).position(|w| w == tag).expect("atom present")
}

/// The sound description's `wave` atom, and the packets via stsz / stco (one sample per chunk).
fn demux(mov: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let wave = find(mov, b"wave") - 4;
    let extra = mov[wave + 8..wave + be32(mov, wave)].to_vec();
    let stsz = find(mov, b"stsz") + 4;
    let n = be32(mov, stsz + 8);
    let fixed = be32(mov, stsz + 4);
    let sizes: Vec<usize> = (0..n).map(|i| if fixed != 0 { fixed } else { be32(mov, stsz + 12 + 4 * i) }).collect();
    let stco = find(mov, b"stco") + 4;
    let offs: Vec<usize> = (0..n).map(|i| be32(mov, stco + 8 + 4 * i)).collect();
    let packets = offs.iter().zip(&sizes).map(|(&o, &s)| mov[o..o + s].to_vec()).collect();
    (extra, packets)
}

#[test]
fn matches_the_fate_reference() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/qdm2/fate");
    let (Ok(mov), Ok(pcm)) = (std::fs::read(dir.join("subpacket12.mov")), std::fs::read(dir.join("subpacket12.pcm"))) else {
        eprintln!("skipped: qdm2 FATE sample not fetched");
        return;
    };
    let want: Vec<i16> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let (extra, packets) = demux(&mov);
    let mut dec = Decoder::new(&extra).unwrap();
    assert_eq!((dec.channels, dec.sample_rate), (1, 44100));
    let mut got = Vec::new();
    for p in &packets {
        got.extend(dec.decode(p).unwrap());
    }
    assert_eq!(got.len(), want.len());
    let first_bad = got.iter().zip(&want).position(|(a, b)| a != b);
    assert!(first_bad.is_none(), "first mismatch at {first_bad:?}");
}
