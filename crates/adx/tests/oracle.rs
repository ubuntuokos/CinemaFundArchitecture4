//! ADX streams against ffmpeg's `adpcm_adx` decoder (the code this crate is ported from).
//!
//! Three sources of streams: files written by ffmpeg's own ADX encoder from a generated tone
//! (mono and stereo (the encoder takes no more), several rates), hand-built files with random blocks and
//! every cutoff / rate / channel count (so every predictor and every nibble value is hit, also
//! the saturating ones), and damaged variants (end marker, partial last block, cut headers).

use std::path::Path;
use std::process::Command;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

/// A header (version 3) of `data_offset` = 0x20 bytes plus the "(c)CRI" marker.
fn header(channels: u8, rate: u32, cutoff: u16, samples: u32) -> Vec<u8> {
    let mut h = vec![0u8; 0x20];
    h[0] = 0x80;
    h[2..4].copy_from_slice(&(0x20u16 - 4).to_be_bytes());
    h[4] = 3;
    h[5] = 18;
    h[6] = 4;
    h[7] = channels;
    h[8..12].copy_from_slice(&rate.to_be_bytes());
    h[12..16].copy_from_slice(&samples.to_be_bytes());
    h[16..18].copy_from_slice(&cutoff.to_be_bytes());
    h[18] = 3;
    h[19] = 0;
    h[0x1A..0x20].copy_from_slice(b"(c)CRI");
    h
}

/// `groups` block groups of random data; scales are limited so that some streams stay small
/// and some saturate.
fn random_file(rng: &mut Rng, channels: u8, rate: u32, cutoff: u16, groups: usize, max_scale: u32) -> Vec<u8> {
    let mut f = header(channels, rate, cutoff, (groups * 32) as u32);
    for _ in 0..groups * usize::from(channels) {
        f.extend_from_slice(&((rng.next() % max_scale) as u16).to_be_bytes());
        f.extend((0..16).map(|_| rng.next() as u8));
    }
    f
}

fn ffmpeg_decode(ff: &Path, data: &[u8], tag: &str) -> Option<Vec<i16>> {
    let dir = std::env::temp_dir().join(format!("cfa4-adx-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.adx");
    std::fs::write(&input, data).ok()?;
    let out = Command::new(ff).args(["-v", "error", "-f", "adx", "-i"]).arg(&input).args(["-f", "s16le", "-c:a", "pcm_s16le", "-"]).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return None;
    }
    Some(out.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

fn interleave(planes: &[Vec<i16>]) -> Vec<i16> {
    let n = planes.first().map_or(0, Vec::len);
    (0..n).flat_map(|i| planes.iter().map(move |p| p[i])).collect()
}

#[test]
fn random_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("adx oracle") else { return };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut n = 0;
    for channels in [1u8, 2, 3, 6] {
        for (rate, cutoff) in [(44100u32, 500u16), (48000, 0), (22050, 11025), (32000, 20000), (8000, 65535), (96000, 4000), (1, 1)] {
            for max_scale in [64u32, 4096, 0x8000] {
                let data = random_file(&mut rng, channels, rate, cutoff, 40, max_scale);
                let tag = format!("c{channels}-r{rate}-f{cutoff}-s{max_scale}");
                let (info, planes) = cfa4_adx::decode_file(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
                assert_eq!((info.channels, info.sample_rate, info.cutoff), (channels, rate, cutoff));
                let theirs = ffmpeg_decode(&ff, &data, &tag).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
                assert_eq!(interleave(&planes), theirs, "{tag}");
                n += 1;
            }
        }
    }
    assert_eq!(n, 84);
}

#[test]
fn end_marker_and_partial_blocks_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("adx end") else { return };
    let mut rng = Rng(12345);
    for channels in [1u8, 2, 4] {
        let base = random_file(&mut rng, channels, 44100, 3000, 300, 0x4000);
        let ch = usize::from(channels);
        let group = 18 * ch;
        // 1. the usual ending: an end block (0x8001 …) after the data, plus padding
        let mut with_end = base.clone();
        with_end.extend_from_slice(&[0x80, 0x01, 0x0E]);
        with_end.extend_from_slice(&vec![0; group - 3 + 100]);
        // 2. an end marker in the middle, in the second channel's block
        let mut mid = base.clone();
        let at = 0x20 + 100 * group + (ch - 1) * 18;
        mid[at] |= 0x80;
        // 3. a partial last group (every cut length), 4. nothing but one group
        let mut cases = vec![("end".to_string(), with_end), ("mid".to_string(), mid)];
        for cut in [1usize, 17, 18, group - 1, group + 1] {
            cases.push((format!("cut{cut}"), base[..base.len() - cut].to_vec()));
        }
        cases.push(("one".to_string(), base[..0x20 + group].to_vec()));
        for (name, data) in cases {
            let tag = format!("c{channels}-{name}");
            let ours = cfa4_adx::decode_file(&data).map(|(_, p)| interleave(&p));
            let theirs = ffmpeg_decode(&ff, &data, &tag);
            match (ours, theirs) {
                (Ok(a), Some(b)) => assert_eq!(a, b, "{tag}"),
                (Err(_), None) => {}
                (Err(cfa4_adx::Error::Empty), Some(b)) => assert!(b.is_empty(), "{tag}"),
                (a, b) => panic!("{tag}: ours {:?}, ffmpeg {:?}", a.map(|v| v.len()), b.map(|v| v.len())),
            }
        }
    }
}

#[test]
fn header_checks() {
    let ok = header(2, 44100, 500, 64);
    let mut f = ok.clone();
    f.extend_from_slice(&[0u8; 36]);
    assert!(cfa4_adx::decode_file(&f).is_ok());
    let mut bad = f.clone();
    bad[0] = 0x81;
    assert_eq!(cfa4_adx::decode_file(&bad).unwrap_err(), cfa4_adx::Error::NotAdx);
    let mut bad = f.clone();
    bad[0x1F] = b'X'; // the (c)CRI marker
    assert!(matches!(cfa4_adx::decode_file(&bad), Err(cfa4_adx::Error::Invalid(_))));
    for (at, v) in [(4usize, 2u8), (5, 16), (6, 8)] {
        let mut bad = f.clone();
        bad[at] = v;
        assert!(matches!(cfa4_adx::decode_file(&bad), Err(cfa4_adx::Error::Unsupported(_))));
    }
    for ch in [0u8, 7] {
        let mut bad = f.clone();
        bad[7] = ch;
        assert!(cfa4_adx::decode_file(&bad).is_err());
    }
    let mut bad = f.clone();
    bad[8..12].copy_from_slice(&0u32.to_be_bytes());
    assert!(cfa4_adx::decode_file(&bad).is_err());
    assert!(cfa4_adx::decode_file(&[]).is_err());
}

#[test]
fn ffmpeg_encoder_files_match_ffmpeg_and_stay_close_to_the_source() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("adx encoder") else { return };
    if !cfa4_testkit::oracle::ffmpeg_has_encoder(&ff, "adpcm_adx") {
        cfa4_testkit::oracle::skip("adx encoder", "ffmpeg has no adpcm_adx encoder");
        return;
    }
    let dir = cfa4_testkit::fixtures_dir("adx");
    for (channels, rate) in [(1u32, 44100u32), (2, 48000), (2, 22050)] {
        let adx = dir.join(format!("tone-{channels}-{rate}.adx"));
        let ok = cfa4_testkit::fixtures::generate(&adx, |out| {
            Command::new(&ff)
                .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
                .arg(format!("sine=frequency=440:sample_rate={rate}:duration=1.5"))
                .args(["-ac", &channels.to_string(), "-c:a", "adpcm_adx", "-f", "adx"])
                .arg(out)
                .status()
                .is_ok_and(|s| s.success())
        });
        let Some(adx) = ok else { panic!("ffmpeg could not write {channels}ch {rate}Hz") };
        let data = std::fs::read(&adx).unwrap();
        let (info, planes) = cfa4_adx::decode_file(&data).unwrap();
        assert_eq!((u32::from(info.channels), info.sample_rate), (channels, rate));
        let theirs = ffmpeg_decode(&ff, &data, &format!("enc{channels}")).unwrap();
        assert_eq!(interleave(&planes), theirs, "{channels}ch {rate}Hz");
        // the tone (amplitude 0.125 of full scale from the sine source) survives the ADPCM
        let peak = planes[0].iter().map(|&v| i32::from(v).abs()).max().unwrap();
        assert!(peak > 2000, "peak {peak}");
    }
}

/// `decode_file` on `data` must return (an `Ok` or an `Err`), never panic.
fn no_panic(data: &[u8], what: &str) {
    let r = std::panic::catch_unwind(|| {
        let _ = cfa4_adx::decode_file(data);
    });
    assert!(r.is_ok(), "decode_file panicked: {what}");
}

#[test]
fn damaged_streams_never_panic() {
    let mut rng = Rng(777);
    let base = random_file(&mut rng, 2, 44100, 700, 20, 0x8000);
    for cut in 0..base.len() {
        no_panic(&base[..cut], &format!("cut at {cut}"));
    }
    for n in 0..3000 {
        let mut d = base.clone();
        for _ in 0..1 + rng.next() % 6 {
            let i = (rng.next() as usize) % d.len();
            d[i] = rng.next() as u8;
        }
        no_panic(&d, &format!("mutation {n}"));
    }
    let noise: Vec<u8> = (0..4096).map(|_| rng.next() as u8).collect();
    no_panic(&noise, "noise");
}

#[test]
fn a_changed_header_is_checked_again() {
    let mut f = header(2, 44100, 500, 64);
    f.extend_from_slice(&[0u8; 72]);
    let mut h = cfa4_adx::parse_header(&f).unwrap();
    for ch in [0u8, 7, 255] {
        h.channels = ch;
        assert!(matches!(cfa4_adx::decode_blocks(&h, &f[0x20..]), Err(cfa4_adx::Error::Invalid(_))));
    }
}
