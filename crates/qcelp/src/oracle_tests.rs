//! The QCELP decoder and the QCP reader against ffmpeg's `qcelp` decoder and `qcp` demuxer, which
//! the port follows operation by operation, so the float output must match bit for bit.
//!
//! ffmpeg does not encode QCELP, so the streams are synthetic: random packets of every rate,
//! mostly re-rolled until the decoder takes them as normal frames (so every decoding path runs),
//! mixed with raw random packets (erasures, which both decoders conceal), packets of the wrong
//! size and packets that claim another rate.

use std::process::Command;

use crate::{Decoder, decode_file};

fn rng(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x
}

const RATE_BYTES: [usize; 5] = [1, 4, 8, 17, 35];

/// A QCP file: header (`fixed` = the packet size field, 0 for variable rate), the rate map
/// (data size after the mode byte, per mode) and the packets, each as `(mode byte, data)`.
fn qcp_file(rate_map: [u8; 5], packet_size: u16, packets: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(b"QLCMfmt ");
    f.extend_from_slice(&150u32.to_le_bytes());
    f.extend_from_slice(&[1, 0]); // major, minor
    f.push(0x41);
    f.extend_from_slice(&[0x6d, 0x7f, 0x5e, 0x15, 0xb1, 0xd0, 0x11, 0xba, 0x91, 0x00, 0x80, 0x5f, 0xb4, 0xb9, 0x7e]);
    f.extend_from_slice(&[0, 0]); // codec version
    f.extend_from_slice(&[b'Q'; 80]);
    f.extend_from_slice(&13000u16.to_le_bytes());
    f.extend_from_slice(&packet_size.to_le_bytes());
    f.extend_from_slice(&160u16.to_le_bytes());
    f.extend_from_slice(&8000u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(&5u32.to_le_bytes());
    for (mode, size) in rate_map.iter().enumerate() {
        f.extend_from_slice(&[*size, mode as u8]);
    }
    f.extend_from_slice(&[0; 6]); // empty rate-map entries
    f.extend_from_slice(&[0; 20]); // reserved
    assert_eq!(f.len(), 170, "header size");
    f.extend_from_slice(b"vrat");
    f.extend_from_slice(&8u32.to_le_bytes());
    f.extend_from_slice(&u32::from(packet_size == 0).to_le_bytes());
    f.extend_from_slice(&(packets.len() as u32).to_le_bytes());
    let data: Vec<u8> = packets.iter().flat_map(|(m, d)| std::iter::once(*m).chain(d.iter().copied())).collect();
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    if data.len() & 1 == 1 {
        f.push(0);
    }
    f
}

fn random_bytes(x: &mut u64, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng(x) as u8).collect()
}

/// A packet of `rate` (0..=4) with its rate byte, re-rolled until `d` decodes it as a normal frame.
fn valid_packet(x: &mut u64, d: &Decoder, rate: usize) -> Vec<u8> {
    for _ in 0..100_000 {
        let mut p = random_bytes(x, RATE_BYTES[rate]);
        p[0] = rate as u8;
        if rate == 0 || d.clone().decode(&p, &mut Vec::new()) {
            return p;
        }
    }
    panic!("no valid packet of rate {rate}");
}

/// Random packets with the rate byte inside; our decoding of exactly these.
fn stream(seed: u64, packets: usize, garbage_per_cent: u64, odd_sizes: bool) -> (Vec<(u8, Vec<u8>)>, Vec<f32>) {
    let mut x = seed;
    let mut d = Decoder::new();
    let mut pcm = Vec::new();
    let mut list = Vec::new();
    for _ in 0..packets {
        let roll = rng(&mut x) % 100;
        let rate = match rng(&mut x) % 16 {
            0 => 0,
            1..=3 => 1,
            4..=6 => 2,
            7..=10 => 3,
            _ => 4,
        };
        let p = if odd_sizes && rate == 1 {
            // the rate map of this file lists mode 1 with 5 bytes, a size no rate has
            let mut p = random_bytes(&mut x, 5);
            p[0] = 1;
            p
        } else if roll < garbage_per_cent {
            let mut p = random_bytes(&mut x, RATE_BYTES[rate]);
            // sometimes the rate byte is right, sometimes it claims another rate
            if !rng(&mut x).is_multiple_of(4) {
                p[0] = rate as u8;
            } else {
                p[0] %= 6;
            }
            if rate == 1 && rng(&mut x).is_multiple_of(3) {
                p[1] = 0xFF;
                p[2] = 0xFF;
            }
            p
        } else {
            valid_packet(&mut x, &d, rate)
        };
        list.push((rate as u8, p.clone()));
        d.decode(&p, &mut pcm);
    }
    (list, pcm)
}

fn ffmpeg_decode(ff: &std::path::Path, data: &[u8], tag: &str) -> Option<Vec<f32>> {
    let dir = std::env::temp_dir().join(format!("cfa4-qcelp-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.qcp");
    let output = dir.join("out.raw");
    std::fs::write(&input, data).ok()?;
    let ok =
        Command::new(ff).args(["-v", "error", "-y", "-f", "qcp", "-i"]).arg(&input).args(["-f", "f32le", "-ac", "1"]).arg(&output).status().ok()?.success();
    let raw = std::fs::read(&output).ok();
    let _ = std::fs::remove_dir_all(&dir);
    if !ok {
        return None;
    }
    Some(raw?.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn assert_same(tag: &str, ours: &[f32], theirs: &[f32]) {
    assert_eq!(ours.len(), theirs.len(), "{tag}: sample count");
    let diff = ours.iter().zip(theirs).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
    let first = ours.iter().zip(theirs).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(diff, 0, "{tag}: {diff}/{} samples differ, first at {first:?}", ours.len());
}

#[test]
fn streams_with_rate_bytes_match_ffmpeg_bit_exactly() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("QCELP oracle") else { return };
    for seed in 1..=8u64 {
        let seed = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let garbage = [0, 0, 10, 10, 30, 30, 60, 100][(seed >> 60) as usize % 8];
        let (packets, ours) = stream(seed, 400, garbage, false);
        let file = qcp_file([1, 4, 8, 17, 35], 0, &packets);
        let tag = format!("rate-bytes-{seed}");
        let Some(theirs) = ffmpeg_decode(&ff, &file, &tag) else { panic!("{tag}: ffmpeg failed") };
        assert_eq!(ours.len(), packets.len() * 160, "{tag}");
        let peak = ours.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.01 && ours.iter().all(|v| v.is_finite()), "{tag}: peak {peak}");
        assert_same(&tag, &ours, &theirs);
        assert_same(&format!("{tag} decode_file"), &decode_file(&file).expect("decode"), &theirs);
    }
}

#[test]
fn streams_without_rate_bytes_and_odd_sizes_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("QCELP oracle") else { return };
    for seed in 11..=14u64 {
        let seed = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        // the rate byte dropped from every packet (ffmpeg guesses the rate from the size); mode 0
        // keeps a size of 1 because a packet of 0 bytes is not delivered
        let (packets, _) = stream(seed, 300, 20, false);
        let dropped: Vec<(u8, Vec<u8>)> = packets.iter().map(|(m, d)| (*m, if d.len() > 1 { d[1..].to_vec() } else { d.clone() })).collect();
        let file = qcp_file([1, 3, 7, 16, 34], 0, &dropped);
        let tag = format!("no-rate-byte-{seed}");
        let Some(theirs) = ffmpeg_decode(&ff, &file, &tag) else { panic!("{tag}: ffmpeg failed") };
        let mut d = Decoder::new();
        let mut ours = Vec::new();
        for (m, p) in &dropped {
            // the guess works from the size alone; mode 0 with a 1-byte packet stays a blank frame
            let _ = m;
            d.decode(p, &mut ours);
        }
        assert_same(&tag, &ours, &theirs);
        assert_same(&format!("{tag} decode_file"), &decode_file(&file).expect("decode"), &theirs);

        // packets of a size no rate has (mode 1 listed with 5 bytes) are erasures
        let (odd, ours) = stream(seed ^ 0x55, 300, 20, true);
        let file = qcp_file([1, 5, 8, 17, 35], 0, &odd);
        let tag = format!("odd-sizes-{seed}");
        let Some(theirs) = ffmpeg_decode(&ff, &file, &tag) else { panic!("{tag}: ffmpeg failed") };
        assert_same(&tag, &ours, &theirs);
    }
}

#[test]
fn fixed_size_files_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("QCELP oracle") else { return };
    // a file with a fixed packet size (36 = mode byte + 35 bytes), every packet full rate
    let mut x = 0x1234_5678_9abc_def1u64;
    let d = Decoder::new();
    let packets: Vec<(u8, Vec<u8>)> = (0..200).map(|_| (4u8, valid_packet(&mut x, &d, 4))).collect();
    let file = qcp_file([1, 4, 8, 17, 35], 36, &packets);
    let Some(theirs) = ffmpeg_decode(&ff, &file, "fixed") else { panic!("ffmpeg failed") };
    let mut d = Decoder::new();
    let mut ours = Vec::new();
    for (_, p) in &packets {
        d.decode(p, &mut ours);
    }
    assert_same("fixed", &ours, &theirs);
    assert_same("fixed decode_file", &decode_file(&file).expect("decode"), &theirs);
}

/// FFmpeg's FATE sample `qcp/0036580847.QCP` (a real QCELP-13K recording; local only, fetched from
/// <https://fate-suite.ffmpeg.org/qcp/> into `target/fixtures/qcelp/`; skipped when absent).
#[test]
fn fate_sample_matches_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("QCELP oracle") else { return };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/qcelp/0036580847.QCP");
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("SKIPPED: {} not present", path.display());
        return;
    };
    let ours = decode_file(&data).expect("decode");
    let Some(theirs) = ffmpeg_decode(&ff, &data, "fate") else { panic!("ffmpeg failed") };
    assert_same("fate 0036580847.QCP", &ours, &theirs);
    let q = crate::parse(&data).unwrap();
    let mut d = Decoder::new();
    let normal = q.packets.iter().filter(|p| d.decode(p, &mut Vec::new())).count();
    eprintln!("fate sample: {} packets, {normal} decoded as normal frames", q.packets.len());
    assert!(normal * 10 > q.packets.len() * 8, "a real recording should be mostly normal frames");
}
