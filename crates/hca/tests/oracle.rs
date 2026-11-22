//! HCA streams against ffmpeg's `hca` decoder (the code this crate is ported from).
//!
//! No HCA encoder ships with ffmpeg and ffmpeg's FATE suite has no HCA sample, so the streams are
//! built here: a header writer (the `fmt`, `comp` / `dec`, `ath`, `ciph`, `loop`, `rva`, `comm`
//! and `pad` chunks, with and without the masked tags) and blocks of random bits behind the sync
//! word, encrypted with the inverse of the key's cipher table. Random bits reach every branch of
//! the decoder: the scale-factor modes, every quantiser resolution, the noise level, intensity
//! stereo, high-frequency reconstruction and the stereo channel layouts. The block size is large
//! enough that no block is read past its end, which keeps ffmpeg's result deterministic.
//!
//! Both decoders compute the inverse MDCT in float (ffmpeg with an FFT, this crate with a double
//! precision DCT-IV), so the comparison allows a relative error of 1e-5 of the stream's peak.

use std::path::Path;
use std::process::Command;

use cfa4_hca::{Error, Key, decode_file};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
        }
    }
    crc
}

#[derive(Clone)]
struct Spec {
    version: u16,
    channels: u8,
    rate: u32,
    blocks: u32,
    dec_chunk: bool,
    tracks: u8,
    config: u8,
    total: u8,
    base: u8,
    stereo: u8,
    bands_per_hfr: u8,
    ath_chunk: Option<u16>,
    cipher: u16,
    key: Option<Key>,
    mask_tags: bool,
    extra_chunks: bool,
    sparse: bool,
}

impl Spec {
    fn new(channels: u8) -> Self {
        Self {
            version: 0x0200,
            channels,
            rate: 48000,
            blocks: 12,
            dec_chunk: false,
            tracks: 1,
            config: 0,
            total: 128,
            base: 128,
            stereo: 0,
            bands_per_hfr: 0,
            ath_chunk: None,
            cipher: 0,
            key: None,
            mask_tags: false,
            extra_chunks: false,
            sparse: false,
        }
    }

    fn block_size(&self) -> usize {
        (usize::from(self.channels) * 1700 + 64).min(65535)
    }
}

fn header(s: &Spec) -> Vec<u8> {
    let mut h = Vec::new();
    let tag = |h: &mut Vec<u8>, t: &[u8; 4]| h.extend(t.iter().map(|&b| if s.mask_tags && b != 0 { b | 0x80 } else { b }));
    tag(&mut h, b"HCA\0");
    h.extend_from_slice(&s.version.to_be_bytes());
    h.extend_from_slice(&[0, 0]);
    tag(&mut h, b"fmt\0");
    h.push(s.channels);
    h.extend_from_slice(&s.rate.to_be_bytes()[1..]);
    h.extend_from_slice(&s.blocks.to_be_bytes());
    h.extend_from_slice(&128u16.to_be_bytes());
    h.extend_from_slice(&64u16.to_be_bytes());
    if s.dec_chunk {
        tag(&mut h, b"dec\0");
        h.extend_from_slice(&(s.block_size() as u16).to_be_bytes());
        h.extend_from_slice(&[1, 15, s.total - 1, s.base - 1, (s.tracks << 4) | s.config, u8::from(s.base != s.total)]);
    } else {
        tag(&mut h, b"comp");
        h.extend_from_slice(&(s.block_size() as u16).to_be_bytes());
        h.extend_from_slice(&[1, 15, s.tracks, s.config, s.total, s.base, s.stereo, s.bands_per_hfr, 0, 0]);
    }
    if s.extra_chunks {
        tag(&mut h, b"vbr\0");
        h.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    }
    if let Some(t) = s.ath_chunk {
        tag(&mut h, b"ath\0");
        h.extend_from_slice(&t.to_be_bytes());
    }
    if s.extra_chunks {
        tag(&mut h, b"loop");
        h.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0, 0]);
    }
    if s.cipher != 0 || s.key.is_some() {
        tag(&mut h, b"ciph");
        h.extend_from_slice(&s.cipher.to_be_bytes());
    }
    if s.extra_chunks {
        tag(&mut h, b"rva\0");
        h.extend_from_slice(&1.0f32.to_be_bytes());
        tag(&mut h, b"comm");
        h.push(2);
        h.extend_from_slice(b"0123456789abcdef"); // 2 * 8 bytes of comment
    }
    // Real headers end with a "pad" chunk; ffmpeg reads the key from the bytes it adds behind
    // the header only when the chunk loop stopped there.
    tag(&mut h, b"pad\0");
    let len = h.len() + 2;
    h[6..8].copy_from_slice(&(len as u16).to_be_bytes());
    let crc = crc16(&h);
    h.extend_from_slice(&crc.to_be_bytes());
    h
}

fn blocks(s: &Spec, rng: &mut Rng) -> Vec<u8> {
    let table = s.key.filter(|_| s.cipher == 56).map(cfa4_hca::cipher_table);
    let mut inverse = [0u8; 256];
    if let Some(t) = &table {
        for (c, &p) in t.iter().enumerate() {
            inverse[usize::from(p)] = c as u8;
        }
    }
    let mut out = Vec::new();
    for _ in 0..s.blocks {
        let mut b: Vec<u8> = (0..s.block_size() - 2)
            .map(|_| {
                let v = rng.next() as u8;
                if s.sparse && rng.next() % 10 < 7 { 0 } else { v }
            })
            .collect();
        b.splice(0..0, [0xFF, 0xFF]);
        let n = b.len();
        let crc = crc16(&b[..n - 2]);
        b[n - 2..].copy_from_slice(&crc.to_be_bytes());
        if table.is_some() {
            for v in &mut b {
                *v = inverse[usize::from(*v)];
            }
        }
        out.extend(b);
    }
    out
}

fn file(s: &Spec, rng: &mut Rng) -> Vec<u8> {
    let mut f = header(s);
    f.extend(blocks(s, rng));
    f
}

fn ffmpeg_decode(ff: &Path, data: &[u8], key: Option<Key>, tag: &str) -> Option<Vec<f32>> {
    let dir = std::env::temp_dir().join(format!("cfa4-hca-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.hca");
    std::fs::write(&input, data).ok()?;
    let mut cmd = Command::new(ff);
    cmd.args(["-v", "error"]);
    if let Some(k) = key {
        cmd.args(["-hca_highkey", &(k.key >> 32).to_string(), "-hca_lowkey", &(k.key & 0xFFFF_FFFF).to_string(), "-hca_subkey", &k.subkey.to_string()]);
    }
    let out = cmd.args(["-f", "hca", "-i"]).arg(&input).args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return None;
    }
    Some(out.stdout.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn interleave(planes: &[Vec<f32>]) -> Vec<f32> {
    let n = planes.first().map_or(0, Vec::len);
    (0..n).flat_map(|i| planes.iter().map(move |p| p[i])).collect()
}

/// Largest error relative to the peak of the reference.
fn rel_error(ours: &[f32], theirs: &[f32]) -> f64 {
    assert_eq!(ours.len(), theirs.len());
    // `f64::max` ignores a NaN, so a non-finite sample would pass the comparison below.
    assert!(ours.iter().chain(theirs).all(|v| v.is_finite()), "non-finite sample");
    let peak = theirs.iter().fold(0f64, |a, &v| a.max(f64::from(v).abs())).max(1e-30);
    ours.iter().zip(theirs).fold(0f64, |a, (&x, &y)| a.max((f64::from(x) - f64::from(y)).abs())) / peak
}

/// ffmpeg before 8.0 skips the two reserved bytes of the `comp` chunk wrongly and so never sees
/// the `ath` and `ciph` chunks that follow (fixed in n8.0; CFA4's oracle is 9.0.2).
fn needs_fixed_ffmpeg(ff: &Path, context: &str) -> bool {
    let ok = cfa4_testkit::oracle::ffmpeg_at_least(ff, 8, 0);
    if !ok {
        cfa4_testkit::oracle::skip(context, "ffmpeg older than 8.0 misparses the HCA chunks after `comp`");
    }
    ok
}

fn check(ff: &Path, s: &Spec, rng: &mut Rng, tag: &str) -> f64 {
    let data = file(s, rng);
    let (info, planes, damaged) = decode_file(&data, s.key).unwrap_or_else(|e| panic!("{tag}: {e}"));
    assert_eq!(damaged, 0, "{tag}");
    assert_eq!((info.channels, info.sample_rate, info.block_count), (s.channels, s.rate, s.blocks));
    assert_eq!(planes[0].len(), s.blocks as usize * 1024);
    let theirs = ffmpeg_decode(ff, &data, s.key, tag).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
    let err = rel_error(&interleave(&planes), &theirs);
    assert!(err < 1e-5, "{tag}: relative error {err}");
    assert!(theirs.iter().any(|&v| v != 0.0), "{tag}: silent");
    err
}

#[test]
fn layouts_and_band_configurations_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("hca oracle") else { return };
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut worst = 0f64;
    let mut n = 0;
    // (channels, tracks, config): mono, stereo, 3, 4 (both configs), 5 (both), 6, 8 and two tracks
    let layouts = [(1u8, 1u8, 0u8), (2, 1, 0), (3, 1, 0), (4, 1, 0), (4, 1, 3), (4, 2, 0), (5, 1, 1), (5, 1, 3), (6, 1, 0), (8, 1, 0), (8, 2, 0), (16, 2, 0)];
    // (total, base, stereo, bands per HFR group)
    let bands = [(128u8, 128u8, 0u8, 0u8), (80, 64, 8, 4), (128, 60, 20, 8), (96, 96, 0, 8), (100, 40, 30, 0), (127, 30, 10, 7), (20, 10, 5, 3)];
    for (ch, tracks, config) in layouts {
        for (i, &(total, base, stereo, bpg)) in bands.iter().enumerate() {
            let mut s = Spec::new(ch);
            s.tracks = tracks;
            s.config = config;
            (s.total, s.base, s.stereo, s.bands_per_hfr) = (total, base, stereo, bpg);
            s.sparse = i % 2 == 1;
            s.rate = [48000, 44100, 32000, 22050, 96000, 24000, 88200][i];
            worst = worst.max(check(&ff, &s, &mut rng, &format!("c{ch}t{tracks}k{config}b{i}")));
            n += 1;
        }
    }
    println!("{n} streams, worst relative error {worst:.3e}");
}

#[test]
fn dec_chunk_versions_ath_and_extra_chunks_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("hca chunks") else { return };
    if !needs_fixed_ffmpeg(&ff, "chunks") {
        return;
    }
    let mut rng = Rng(99);
    for (i, version) in [0x0101u16, 0x0102, 0x0103, 0x0200, 0x0300].into_iter().enumerate() {
        for ath in [None, Some(0u16), Some(1)] {
            for (dec_chunk, extra, mask) in [(false, false, false), (true, false, false), (false, true, false), (true, true, true), (false, false, true)] {
                let mut s = Spec::new(2);
                s.version = version;
                s.ath_chunk = ath;
                s.dec_chunk = dec_chunk;
                s.extra_chunks = extra;
                s.mask_tags = mask;
                (s.total, s.base, s.stereo, s.bands_per_hfr) = if dec_chunk { (110, 90, 20, 0) } else { (110, 90, 20, 4) };
                s.rate = 44100 + 1000 * i as u32;
                check(&ff, &s, &mut rng, &format!("v{version:x}-a{ath:?}-d{dec_chunk}e{extra}m{mask}"));
            }
        }
    }
    // a "dec" chunk whose second band count is off: base = total
    let mut s = Spec::new(2);
    s.dec_chunk = true;
    (s.total, s.base) = (100, 100);
    check(&ff, &s, &mut rng, "dec-flat");
}

#[test]
fn encrypted_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("hca keys") else { return };
    if !needs_fixed_ffmpeg(&ff, "keys") {
        return;
    }
    let mut rng = Rng(4242);
    for key in [
        Key { key: 0xCF22_2F1F_E074_8978, subkey: 0 },
        Key { key: 1, subkey: 0 },
        Key { key: 0x0000_1234_5678_9ABC, subkey: 0 },
        Key { key: 0xFFFF_FFFF_FFFF_FFFF, subkey: 0 },
        Key { key: 0xCF22_2F1F_E074_8978, subkey: 0x3B8E },
        Key { key: 0x1234_5678, subkey: 1 },
        Key { key: 0xDEAD_BEEF_0000_0001, subkey: 0xFFFF },
    ] {
        let mut s = Spec::new(2);
        s.cipher = 56;
        s.key = Some(key);
        (s.total, s.base, s.stereo, s.bands_per_hfr) = (96, 64, 16, 4);
        check(&ff, &s, &mut rng, &format!("key{:x}-{:x}", key.key, key.subkey));
    }
    // cipher type 0 ignores a key; without the key an encrypted stream is refused
    let mut s = Spec::new(2);
    s.key = Some(Key { key: 5, subkey: 0 });
    check(&ff, &s, &mut rng, "plain-with-key");
    let mut s = Spec::new(2);
    s.cipher = 56;
    let data = file(&s, &mut rng);
    assert_eq!(decode_file(&data, None).unwrap_err(), Error::KeyRequired);
    assert_eq!(decode_file(&data, Some(Key::default())).unwrap_err(), Error::KeyRequired);
    let mut s = Spec::new(2);
    s.cipher = 1;
    let data = file(&s, &mut rng);
    assert!(matches!(decode_file(&data, None), Err(Error::Unsupported(_))));
}

#[test]
fn damaged_blocks_become_silence_and_headers_are_checked() {
    let mut rng = Rng(7);
    let mut s = Spec::new(2);
    s.blocks = 6;
    let mut data = file(&s, &mut rng);
    let off = header(&s).len();
    let size = s.block_size();
    let (_, clean, damaged) = decode_file(&data, None).unwrap();
    assert_eq!(damaged, 0);
    data[off + 2 * size] = 0x12; // no sync word in block 2
    let (_, planes, damaged) = decode_file(&data, None).unwrap();
    assert_eq!(damaged, 1);
    assert_eq!(planes[0].len(), 6 * 1024);
    assert!(planes[0][2048..3072].iter().all(|&v| v == 0.0));
    assert_eq!(planes[0][..2048], clean[0][..2048]); // the blocks before are unchanged
    // a partial last block is not decoded
    let cut = &data[..data.len() - size / 2];
    assert_eq!(decode_file(cut, None).unwrap().1[0].len(), 5 * 1024);
    // header checks
    assert_eq!(decode_file(b"RIFF", None).unwrap_err(), Error::NotHca);
    assert_eq!(decode_file(&[], None).unwrap_err(), Error::NotHca);
    let mut bad = data.clone();
    bad[0] = b'X';
    assert_eq!(decode_file(&bad, None).unwrap_err(), Error::NotHca);
    let mut bad = data.clone();
    bad[12] = 0; // fmt channel count
    assert!(matches!(decode_file(&bad, None), Err(Error::Invalid(_))));
    let mut bad = data.clone();
    bad[13..16].copy_from_slice(&[0, 0, 0]); // sample rate
    assert!(matches!(decode_file(&bad, None), Err(Error::Invalid(_))));
    let mut bad = data.clone();
    bad[6..8].copy_from_slice(&6u16.to_be_bytes()); // header size
    assert!(matches!(decode_file(&bad, None), Err(Error::Invalid(_))));
    assert!(matches!(decode_file(&data[..20], None), Err(Error::Invalid(_))));
    // ath type 7
    let mut s2 = Spec::new(2);
    s2.ath_chunk = Some(7);
    let d2 = file(&s2, &mut rng);
    assert!(matches!(decode_file(&d2, None), Err(Error::Invalid(_))));
    // base bands above total, and a band layout over 128
    let mut s3 = Spec::new(2);
    (s3.total, s3.base) = (50, 60);
    assert!(matches!(decode_file(&file(&s3, &mut rng), None), Err(Error::Invalid(_))));
    let mut s4 = Spec::new(2);
    (s4.total, s4.base, s4.stereo, s4.bands_per_hfr) = (100, 80, 30, 4); // stereo bands reach past the total
    assert!(matches!(decode_file(&file(&s4, &mut rng), None), Err(Error::Invalid(_))));
    // no blocks at all
    let mut s5 = Spec::new(2);
    s5.blocks = 0;
    assert_eq!(decode_file(&file(&s5, &mut rng), None).unwrap_err(), Error::Empty);
}

/// `decode_file` on `data` must return (an `Ok` or an `Err`), never panic.
fn no_panic(data: &[u8], what: &str) {
    let r = std::panic::catch_unwind(|| {
        let _ = decode_file(data, None);
    });
    assert!(r.is_ok(), "decode_file panicked: {what}");
}

#[test]
fn non_finite_samples_are_rejected_by_the_comparison() {
    let r = std::panic::catch_unwind(|| rel_error(&[f32::NAN], &[1.0]));
    assert!(r.is_err());
}

#[test]
fn a_changed_header_is_checked_again() {
    let mut rng = Rng(3);
    let mut s = Spec::new(2);
    s.blocks = 1;
    let data = file(&s, &mut rng);
    let h = cfa4_hca::parse_header(&data).unwrap();
    for mutate in [
        (|h: &mut cfa4_hca::Header| (h.total_band_count, h.base_band_count, h.stereo_band_count) = (128, 128, 1)) as fn(&mut cfa4_hca::Header),
        |h| (h.total_band_count, h.base_band_count, h.stereo_band_count, h.bands_per_hfr_group) = (128, 100, 40, 1),
        |h| h.channels = 0,
        |h| h.channels = 17,
        |h| h.sample_rate = 0,
        |h| h.total_band_count = 129,
        |h| (h.total_band_count, h.base_band_count, h.stereo_band_count, h.bands_per_hfr_group) = (64, 32, 40, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let (i, mutate) = mutate;
        let mut m = h.clone();
        mutate(&mut m);
        assert!(matches!(cfa4_hca::Decoder::new(m, None), Err(Error::Invalid(_))), "mutation {i}");
    }
}

#[test]
fn damaged_streams_never_panic() {
    let mut rng = Rng(31337);
    let mut s = Spec::new(4);
    s.blocks = 3;
    (s.total, s.base, s.stereo, s.bands_per_hfr) = (100, 50, 20, 6);
    let base = file(&s, &mut rng);
    for cut in (0..base.len()).step_by(7) {
        no_panic(&base[..cut], &format!("cut at {cut}"));
    }
    for n in 0..600 {
        let mut d = base.clone();
        for _ in 0..1 + rng.next() % 8 {
            let i = (rng.next() as usize) % d.len().min(96);
            d[i] = rng.next() as u8;
        }
        no_panic(&d, &format!("mutation {n}"));
    }
    // random blocks of every small size (reads past the end give zeros)
    let hdr = header(&Spec::new(2));
    for size in [9usize, 10, 16, 40, 200] {
        let mut h = hdr.clone();
        // patch the comp block size
        let at = h.windows(4).position(|w| w == b"comp").unwrap() + 4;
        h[at..at + 2].copy_from_slice(&(size as u16).to_be_bytes());
        for n in 0..50 {
            let mut f = h.clone();
            f.push(0xFF);
            f.push(0xFF);
            f.extend((0..size * 3).map(|_| rng.next() as u8));
            no_panic(&f, &format!("block size {size}, case {n}"));
        }
    }
    let noise: Vec<u8> = (0..4096).map(|_| rng.next() as u8).collect();
    no_panic(&noise, "noise");
}
