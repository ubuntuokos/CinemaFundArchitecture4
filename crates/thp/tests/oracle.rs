//! `cfa4-thp` against ffmpeg's `thp` demuxer, `thp` video decoder and `adpcm_thp` decoder (the
//! oracle only). ffmpeg has no THP writer, so the files here come from a writer of ours that
//! picks every syntax element at random within the format's rules: pictures with random
//! coefficients (several samplings, sizes that are no multiple of 16, restart intervals) and
//! sound packets with random coefficient tables and nibbles.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::{fixtures, oracle};

mod support;
use support::*;

fn ffmpeg_out(ff: &Path, file: &Path, args: &[&str], ext: &str) -> Option<Vec<u8>> {
    let out = file.with_extension(ext);
    let ok = Command::new(ff).args(["-v", "error", "-y", "-i"]).arg(file).args(args).arg(&out).status().is_ok_and(|s| s.success());
    ok.then(|| std::fs::read(&out).ok()).flatten()
}

fn dir() -> PathBuf {
    fixtures::fixtures_dir("thp")
}

#[test]
fn pictures_match_ffmpeg() {
    let Some(ff) = oracle::ffmpeg_or_skip("thp oracle") else { return };
    let cases: [(usize, usize, Option<(usize, usize)>); 8] = [
        (64, 48, Some((2, 2))),
        (50, 37, Some((2, 2))),
        (96, 80, Some((2, 1))),
        (33, 70, Some((1, 2))),
        (48, 40, Some((1, 1))),
        (31, 17, Some((1, 1))),
        (40, 24, None),
        (130, 66, Some((2, 2))),
    ];
    for (i, &(w, h, samp)) in cases.iter().enumerate() {
        for (round, restart) in [0usize, 1, 3, 7].into_iter().enumerate() {
            let mut rng = Rng(0x7F4A_7C15 + (i * 10 + round) as u64 * 977);
            let pics: Vec<Vec<u8>> = (0..3).map(|_| picture(w, h, samp, restart, &mut rng)).collect();
            let data = thp_file(&pics, None, w, h);
            let file = dir().join(format!("pic-{i}-{round}.thp"));
            std::fs::write(&file, &data).unwrap();
            let Some(theirs) = ffmpeg_out(&ff, &file, &["-f", "rawvideo"], "raw") else { panic!("pic {i}-{round}: ffmpeg failed") };
            let m = cfa4_thp::parse(&data).unwrap();
            assert_eq!(m.video.len(), 3);
            let mut ours = Vec::new();
            for s in &m.video {
                let p = cfa4_thp::jpeg::decode(s.of(&data)).unwrap_or_else(|e| panic!("pic {i}-{round}: {e}"));
                for pl in &p.planes {
                    ours.extend_from_slice(pl);
                }
            }
            assert_eq!(ours.len(), theirs.len(), "pic {i}-{round}: size");
            if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
                panic!("pic {i}-{round} ({w}x{h} {samp:?} restart {restart}): first difference at byte {at} of {}", ours.len());
            }
        }
    }
}

#[test]
fn sound_matches_ffmpeg() {
    let Some(ff) = oracle::ffmpeg_or_skip("thp oracle") else { return };
    for (i, ch) in [1usize, 2, 1, 2].into_iter().enumerate() {
        let mut rng = Rng(0xA0D1_0000 + i as u64 * 31);
        let w = 32;
        let h = 32;
        let pics: Vec<Vec<u8>> = (0..5).map(|_| picture(w, h, Some((2, 2)), 0, &mut rng)).collect();
        let audio: Vec<Vec<u8>> = (0..5).map(|_| audio_packet(ch, 100 + rng.below(900) as usize, &mut rng)).collect();
        let data = thp_file(&pics, Some((ch as u32, audio)), w, h);
        let file = dir().join(format!("snd-{i}.thp"));
        std::fs::write(&file, &data).unwrap();
        let Some(theirs) = ffmpeg_out(&ff, &file, &["-map", "0:a", "-f", "s16le"], "pcm") else { panic!("snd {i}: ffmpeg failed") };
        let m = cfa4_thp::parse(&data).unwrap();
        let info = m.audio.unwrap();
        assert_eq!((info.channels as usize, info.sample_rate), (ch, 32000));
        let mut dec = cfa4_thp::AudioDecoder::new();
        let mut ours = Vec::new();
        for a in &m.audio_packets {
            ours.extend(dec.decode(a.of(&data), info.channels).unwrap());
        }
        let theirs: Vec<i16> = theirs.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(ours.len(), theirs.len(), "snd {i}: length");
        if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
            panic!("snd {i}: first difference at sample {at}");
        }
    }
}

#[test]
fn damaged_files_do_not_panic() {
    let mut rng = Rng(0xBAD_7819);
    let pics: Vec<Vec<u8>> = (0..4).map(|_| picture(48, 32, Some((2, 2)), 2, &mut rng)).collect();
    let audio: Vec<Vec<u8>> = (0..4).map(|_| audio_packet(2, 300, &mut rng)).collect();
    let good = thp_file(&pics, Some((2, audio)), 48, 32);
    for round in 0..300 {
        let mut d = good.clone();
        match round % 3 {
            0 => d.truncate(rng.below(d.len() as u64) as usize),
            1 => (0..1 + rng.below(8)).for_each(|_| {
                let at = rng.below(d.len() as u64) as usize;
                d[at] = rng.below(256) as u8;
            }),
            _ => {
                let at = rng.below(100.min(d.len() as u64)) as usize;
                d[at] ^= 0xFF;
            }
        }
        if let Ok(m) = cfa4_thp::parse(&d) {
            let mut dec = cfa4_thp::AudioDecoder::new();
            for s in &m.video {
                let _ = cfa4_thp::jpeg::decode(s.of(&d));
            }
            for a in &m.audio_packets {
                let _ = dec.decode(a.of(&d), m.audio.map_or(1, |a| a.channels));
            }
        }
    }
}

#[test]
fn probe_and_bad_headers() {
    assert!(!cfa4_thp::probe(b""));
    assert!(!cfa4_thp::probe(b"RIFF\0\0\0\0AVI "));
    assert_eq!(cfa4_thp::parse(b"THP\0"), Err(cfa4_thp::Error::NotThp));
}
