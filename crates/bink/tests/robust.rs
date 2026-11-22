//! Damaged and random input never panics, and the container rules of the demuxer.

mod common;

use cfa4_bink::audio;
use cfa4_bink::container::{Bink, probe};
use cfa4_bink::video::Decoder;
use common::{Rng, Track, bik};

fn random_file(rng: &mut Rng, ver: u8, with_audio: bool) -> Vec<u8> {
    let video: Vec<Vec<u8>> = (0..6)
        .map(|_| {
            let n = 4 + rng.below(300) as usize;
            (0..n).map(|_| rng.next() as u8).collect()
        })
        .collect();
    let tracks = if with_audio {
        let pk: Vec<Vec<u8>> = (0..6)
            .map(|_| {
                let n = 4 + rng.below(400) as usize;
                (0..n).map(|_| rng.next() as u8).collect()
            })
            .collect();
        vec![Track { rate: 44100, flags: 0x2000, packets: pk }]
    } else {
        vec![]
    };
    bik(&[b'B', b'I', b'K', ver], 24, 16, 0, &video, &tracks)
}

fn run_all(file: &[u8]) {
    let Ok(b) = Bink::parse(file) else { return };
    let Ok(mut v) = Decoder::new(b.width, b.height, b.tag, b.flags) else { return };
    let mut a: Vec<_> = b.tracks.iter().filter_map(|t| audio::Decoder::new(t.sample_rate, t.channels, t.dct, b.tag[3]).ok()).collect();
    for i in 0..b.frame_count() {
        let Ok(f) = b.frame(i) else { continue };
        let _ = v.decode(f.video);
        for (t, pkt) in f.audio {
            if let Some(d) = a.get_mut(t) {
                let _ = d.decode(pkt);
            }
        }
    }
}

#[test]
fn random_and_damaged_streams_never_panic() {
    let mut rng = Rng(0xB1_4B);
    for k in 0..400 {
        let ver = (*b"bfghik")[k % 6];
        let mut file = random_file(&mut rng, ver, k % 2 == 0);
        run_all(&file);
        // damage: flip bytes, then cut
        for _ in 0..8 {
            let at = rng.below(file.len() as u32) as usize;
            file[at] ^= 1 << rng.below(8);
        }
        run_all(&file);
        file.truncate(rng.below(file.len() as u32) as usize);
        run_all(&file);
        // header bytes at random
        let mut head: Vec<u8> = (0..120).map(|_| rng.next() as u8).collect();
        head[..4].copy_from_slice(&[b'B', b'I', b'K', ver]);
        run_all(&head);
    }
}

#[test]
fn container_rules() {
    let mut rng = Rng(5);
    let file = random_file(&mut rng, b'i', true);
    assert!(probe(&file));
    let b = Bink::parse(&file).unwrap();
    assert_eq!((b.width, b.height, b.frame_count()), (24, 16, 6));
    assert_eq!((b.tracks.len(), b.tracks[0].sample_rate, b.tracks[0].channels, b.tracks[0].dct), (1, 44100, 2, false));
    let f0 = b.frame(0).unwrap();
    assert!(f0.keyframe);
    assert!(!b.frame(1).unwrap().keyframe);
    assert!(b.frame(6).is_err());
    // Bink 2 and other signatures are refused, not mis-read
    let mut k2 = file.clone();
    k2[..4].copy_from_slice(b"KB2i");
    assert!(Bink::parse(&k2).is_err());
    assert!(!probe(&k2[..3]));
    // an SMUSH wrapper: 512-byte blocks before the BIK header
    let mut smush = Vec::new();
    smush.extend_from_slice(b"SMUS");
    smush.resize(512, 0);
    smush.extend_from_slice(&file);
    assert!(probe(&smush));
    let s = Bink::parse(&smush).unwrap();
    assert_eq!(s.frame(2).unwrap().video, b.frame(2).unwrap().video);
}
