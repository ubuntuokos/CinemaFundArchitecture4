//! `cfa4-nstream` against ffmpeg's `brstm` / `bfstm` demuxers and `adpcm_thp` decoders (the
//! oracle only), on the real FATE samples `brstm/lozswd_partial.brstm`,
//! `bfstm/spl-forest-day.bfstm` and `bfstm/loz-mm-mikau.bcstm`. They sit in
//! `target/fixtures/nstream/fate/` (from <https://fate-suite.ffmpeg.org/>); the tests skip when a
//! sample is missing. A small writer of ours adds the PCM variants and damaged headers.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_nstream::{Codec, Format, decode, parse};
use cfa4_testkit::{fixtures, oracle};

fn dir() -> PathBuf {
    fixtures::fixtures_dir("nstream")
}

fn sample(name: &str) -> Option<PathBuf> {
    let p = dir().join("fate").join(name);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipping: FATE sample {name} is not in target/fixtures/nstream/fate");
        None
    }
}

fn ffmpeg_pcm(ff: &Path, file: &Path, tag: &str) -> Option<Vec<i16>> {
    let out = dir().join(format!("{tag}.raw"));
    let _ = std::fs::remove_file(&out);
    let ok = Command::new(ff).args(["-v", "error", "-y", "-i"]).arg(file).args(["-map", "0:a:0", "-f", "s16le"]).arg(&out).status().is_ok_and(|s| s.success());
    ok.then(|| std::fs::read(&out).ok()).flatten().map(|b| b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

fn interleave(chs: &[Vec<i16>]) -> Vec<i16> {
    let n = chs.iter().map(Vec::len).min().unwrap_or(0);
    (0..n).flat_map(|i| chs.iter().map(move |c| c[i])).collect()
}

fn check(file: &Path, tag: &str, format: Format, codec_le: bool) {
    let Some(ff) = oracle::ffmpeg_or_skip("nstream oracle") else { return };
    let data = std::fs::read(file).unwrap();
    let s = parse(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
    assert_eq!(s.format, format);
    assert_eq!(s.codec, Codec::Dsp { little_endian: codec_le }, "{tag}");
    let theirs = ffmpeg_pcm(&ff, file, tag).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
    let d = decode(&data, &s);
    // the BRSTM sample is a partial file: it ends inside a block, where ffmpeg stops as well
    assert_eq!(d.truncated, tag == "brstm", "{tag}: truncated");
    let ours = interleave(&d.channels);
    assert_eq!(ours.len(), theirs.len(), "{tag}: sample count ({} channels, {} Hz)", s.channels, s.sample_rate);
    if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
        panic!("{tag}: first difference at value {at} of {}", ours.len());
    }
    eprintln!("{tag}: {} channels, {} Hz, {} values equal", s.channels, s.sample_rate, ours.len());
}

#[test]
fn brstm_matches_ffmpeg() {
    if let Some(f) = sample("lozswd_partial.brstm") {
        check(&f, "brstm", Format::Brstm, false);
    }
}

#[test]
fn bfstm_matches_ffmpeg() {
    if let Some(f) = sample("spl-forest-day.bfstm") {
        check(&f, "bfstm", Format::Bfstm, false);
    }
}

#[test]
fn bcstm_matches_ffmpeg() {
    if let Some(f) = sample("loz-mm-mikau.bcstm") {
        check(&f, "bcstm", Format::Bcstm, true);
    }
}

#[test]
fn damaged_input_does_not_panic() {
    let mut seed = 0x1357_9BDFu32;
    for name in ["lozswd_partial.brstm", "spl-forest-day.bfstm", "loz-mm-mikau.bcstm"] {
        let Some(file) = sample(name) else { continue };
        let data = std::fs::read(file).unwrap();
        for round in 0..80 {
            let mut d = data[..data.len().min(40_000)].to_vec();
            for _ in 0..(1 + round % 7) {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // mostly the header, where the sizes and offsets are
                let at = if round % 3 == 0 { (seed >> 8) as usize % d.len() } else { (seed >> 8) as usize % 0x400.min(d.len()) };
                d[at] ^= (seed >> 3) as u8 | 1;
            }
            if round % 11 == 0 {
                d.truncate(d.len() / 2 + round);
            }
            if let Ok(s) = parse(&d) {
                let _ = decode(&d, &s);
            }
        }
    }
}

/// A BRSTM file with PCM blocks: `blocks` blocks of `per` bytes per channel, the last one holding
/// `last_used` bytes (padded to `last_size`).
fn pcm_brstm(codec: u8, little_endian: bool, channels: usize, per: usize, blocks: usize, last_used: usize, last_size: usize, seed: &mut u32) -> Vec<u8> {
    let p16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&if little_endian { x.to_le_bytes() } else { x.to_be_bytes() });
    let p32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&if little_endian { x.to_le_bytes() } else { x.to_be_bytes() });
    let bps = if codec == 0 { 1 } else { 2 };
    let mut f = Vec::new();
    f.extend_from_slice(b"RSTM");
    f.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
    f.extend_from_slice(&[1, 0]);
    p32(&mut f, 0);
    p16(&mut f, 0x40);
    f.resize(0x40, 0);
    f.extend_from_slice(b"HEAD");
    p32(&mut f, 0x60);
    f.resize(0x40 + 12, 0);
    p32(&mut f, 0x20); // stream info offset
    f.resize(0x40 + 28, 0);
    p32(&mut f, 0); // coefficient table offset
    f.resize(0x40 + 0x28, 0);
    f.extend_from_slice(&[codec, 0, channels as u8, 0]);
    p16(&mut f, 32000);
    p16(&mut f, 0);
    p32(&mut f, 0); // loop start
    let samples_per_block = (per / bps) as u32;
    let last_samples = (last_used / bps) as u32;
    p32(&mut f, samples_per_block * (blocks as u32 - 1) + last_samples);
    p32(&mut f, 0xC0); // data start
    p32(&mut f, blocks as u32);
    p32(&mut f, per as u32);
    p32(&mut f, samples_per_block);
    p32(&mut f, last_used as u32);
    p32(&mut f, last_samples);
    p32(&mut f, last_size as u32);
    f.resize(0x40 + 0x60, 0);
    f.extend_from_slice(b"DATA");
    p32(&mut f, 0x20);
    f.resize(0xC0, 0);
    for b in 0..blocks {
        let size = if b + 1 == blocks { last_size } else { per };
        for _ in 0..channels * size {
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f.push((*seed >> 16) as u8);
        }
    }
    f
}

#[test]
fn pcm_variants_match_ffmpeg() {
    let Some(ff) = oracle::ffmpeg_or_skip("nstream oracle") else { return };
    let mut seed = 0xC0FFEE11u32;
    // (codec, little endian, channels, block bytes per channel, blocks, last used, last size)
    let cases = [
        (0u8, false, 1usize, 64usize, 3usize, 40usize, 64usize),
        (0, false, 2, 64, 3, 64, 64),
        (1, false, 2, 128, 4, 96, 128),
        (1, true, 2, 128, 3, 128, 128),
        (1, true, 1, 100, 2, 50, 64),
        (0, true, 3, 32, 2, 20, 32),
    ];
    for (i, &(codec, le, ch, per, blocks, used, size)) in cases.iter().enumerate() {
        let data = pcm_brstm(codec, le, ch, per, blocks, used, size, &mut seed);
        let file = dir().join(format!("pcm-{i}.brstm"));
        std::fs::write(&file, &data).unwrap();
        let s = parse(&data).unwrap_or_else(|e| panic!("case {i}: {e}"));
        assert_eq!(s.channels as usize, ch);
        let theirs = ffmpeg_pcm(&ff, &file, &format!("pcm-{i}")).unwrap_or_else(|| panic!("case {i}: ffmpeg failed"));
        let d = decode(&data, &s);
        assert!(!d.truncated, "case {i}");
        let ours = interleave(&d.channels);
        assert_eq!(ours.len(), theirs.len(), "case {i}: sample count");
        if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
            panic!("case {i}: first difference at value {at} of {}", ours.len());
        }
    }
}
