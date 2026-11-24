//! Smacker files against ffmpeg's `smk` demuxer, `smackvid` and `smackaud` decoders (ffmpeg has
//! no Smacker encoder, so the files come from `cfa4_testkit::smk`'s writer, and from ffmpeg's FATE sample
//! `wetlogo.smk` when it is in `$SMACKER_SAMPLES` or `target/fixtures/smacker`).
//!
//! The writer draws every syntax element at random within the format's rules: the four header
//! trees (with skipped trees, escape values and most-recent-value slots), palette updates (kept,
//! copied and new runs), SMK2 and SMK4, ring frames, up to seven audio tracks (packed 8- and
//! 16-bit, mono and stereo, and raw PCM), and frame data that is random bytes (every byte string
//! is a valid walk through a prefix code, so the picture blocks come out random, with runs,
//! skips, fills and every full-block mode). Damaged files are decoded without a panic.

use cfa4_smacker::{Decoder, Smk, Track, TrackKind, decode_audio, decode_audio_lenient};
use cfa4_testkit::smk::{Rng, random_spec, smk_file};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------------------------

/// ffmpeg's output, or its error text when it exits with a failure (it cannot start: a panic).
fn run_ffmpeg(ff: &Path, input: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new(ff)
        .args(["-v", "error", "-max_error_rate", "1.0", "-i"])
        .arg(input)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("ffmpeg did not start: {e}"));
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(out.stdout)
}

/// ffmpeg's video frames as (pts, sha256 of the rgb24 picture).
fn ffmpeg_video(ff: &Path, input: &Path) -> Option<Vec<(i64, String)>> {
    let out =
        run_ffmpeg(ff, input, &["-map", "0:v:0", "-an", "-fps_mode", "passthrough", "-pix_fmt", "rgb24", "-f", "framehash", "-hash", "sha256", "-"]).ok()?;
    let text = String::from_utf8_lossy(&out).into_owned();
    let mut v = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        v.push((f.get(2)?.parse().ok()?, f.get(5)?.to_string()));
    }
    Some(v)
}

/// ffmpeg's samples of the `k`-th audio stream, 8-bit as the code less 128; the error text when
/// ffmpeg gives up on the stream.
fn ffmpeg_audio(ff: &Path, input: &Path, k: usize, b16: bool) -> Result<Vec<i32>, String> {
    let map = format!("0:a:{k}");
    let fmt: [&str; 4] = if b16 { ["pcm_s16le", "-f", "s16le", "-"] } else { ["pcm_u8", "-f", "u8", "-"] };
    let out = run_ffmpeg(ff, input, &["-map", &map, "-vn", "-c:a", fmt[0], fmt[1], fmt[2], fmt[3]])?;
    Ok(if b16 { out.chunks_exact(2).map(|c| i32::from(i16::from_le_bytes([c[0], c[1]]))).collect() } else { out.iter().map(|&b| i32::from(b) - 128).collect() })
}

/// Whether the ffmpeg at `ff` checks that a packed audio chunk holds its first samples (n8 and
/// later; the CI oracle is the version of packaging/ffmpeg-version.env, a local one may be older).
fn ffmpeg_checks_first_samples(ff: &Path) -> bool {
    static CHECKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHECKS.get_or_init(|| ffmpeg_checks_first_samples_now(ff))
}

fn ffmpeg_checks_first_samples_now(ff: &Path) -> bool {
    let out = Command::new(ff).arg("-version").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let major = out.split_whitespace().nth(2).and_then(|v| v.trim_start_matches(['n', 'N']).split('.').next()?.parse::<u32>().ok());
    major.is_none_or(|m| m >= 8)
}

fn rgb24(d: &Decoder) -> Vec<u8> {
    d.rgba().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
}

/// What ffmpeg should output for `data`: video frames and each audio track's samples.
#[allow(clippy::type_complexity)]
fn expected(data: &[u8], strict: bool) -> Option<(Vec<(i64, String)>, Vec<(Track, Vec<i32>)>)> {
    let smk = Smk::parse(data).ok()?;
    let mut dec = Decoder::new(&smk).ok()?;
    let mut video = Vec::new();
    let tracks: Vec<(usize, Track)> = smk.tracks.iter().enumerate().filter_map(|(i, t)| t.map(|t| (i, t))).collect();
    let mut audio: Vec<(Track, Vec<i32>)> = tracks.iter().map(|&(_, t)| (t, Vec::new())).collect();
    for i in 0..smk.frames {
        let parts = smk.split(data, i);
        for (n, &(slot, t)) in tracks.iter().enumerate() {
            if let Some(chunk) = parts.audio[slot]
                && let Ok(s) = if strict { decode_audio(&t, chunk) } else { decode_audio_lenient(&t, chunk) }
            {
                audio[n].1.extend(s);
            }
        }
        match (parts.video, parts.palette) {
            (Some(v), _) => {
                if dec.decode(parts.palette, v).is_ok() {
                    video.push((i as i64, cfa4_testkit::av2::sha256_hex(&rgb24(&dec))));
                }
            }
            (None, Some(p)) => {
                let _ = dec.update_palette(p);
            }
            (None, None) => {}
        }
    }
    Some((video, audio))
}

fn check(ff: &Path, data: &[u8], tag: &str) {
    let Some((video, audio)) = expected(data, ffmpeg_checks_first_samples(ff)) else {
        panic!("{tag}: our parser refused a file the writer made");
    };
    let dir = std::env::temp_dir().join(format!("cfa4-smk-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.smk");
    std::fs::write(&input, data).unwrap();
    let got = ffmpeg_video(ff, &input);
    if got.as_deref() != Some(&video[..]) {
        panic!(
            "{tag}: video frames differ from ffmpeg: {:?} against {:?}",
            got.as_ref().map(|g| g.iter().map(|f| f.0).collect::<Vec<_>>()),
            video.iter().map(|f| f.0).collect::<Vec<_>>()
        );
    }
    for (k, (t, samples)) in audio.iter().enumerate() {
        if t.kind == TrackKind::Bink {
            continue;
        }
        // ffmpeg may refuse a stream none of whose chunks decodes (no samples come out of it);
        // any other failure of the oracle is a failure of the test
        let want = match ffmpeg_audio(ff, &input, k, t.bits == 16) {
            Ok(w) => w,
            Err(e) if samples.is_empty() && !e.trim().is_empty() => Vec::new(),
            Err(e) => panic!("{tag}: ffmpeg failed on audio track {k} that we decode to {} samples: {e}", samples.len()),
        };
        if want != *samples {
            let at = want.iter().zip(samples).position(|(a, b)| a != b).unwrap_or(want.len().min(samples.len()));
            panic!("{tag}: audio track {k} {t:?} differs from ffmpeg: {} samples against {}, first difference at {at}", want.len(), samples.len());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn random_files_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("smacker oracle") else { return };
    let mut rng = Rng(0x5eed_5ac4);
    let count: u64 = std::env::var("SMACKER_RANDOM_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    for n in 0..count {
        let spec = random_spec(&mut rng);
        let data = smk_file(&mut rng, &spec);
        check(&ff, &data, &format!("r{n}"));
    }
}

fn sample_dir() -> PathBuf {
    std::env::var_os("SMACKER_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("smacker"))
}

#[test]
fn fate_wetlogo_matches_ffmpeg() {
    let path = sample_dir().join("wetlogo.smk");
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("skipped: {} is not there (ffmpeg's FATE sample)", path.display());
        return;
    };
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("smacker FATE") else { return };
    let smk = Smk::parse(&data).unwrap();
    assert_eq!((smk.width, smk.height, smk.v4), (320, 200, false));
    check(&ff, &data, "wetlogo");
}

// ---------------------------------------------------------------------------------------------
// Damaged input
// ---------------------------------------------------------------------------------------------

/// Parses and decodes everything in `data`; the result does not matter, only that nothing panics.
fn exercise(data: &[u8]) {
    let Ok(smk) = Smk::parse(data) else { return };
    let dec = Decoder::new(&smk);
    let mut dec = dec.ok();
    for i in 0..smk.frames {
        let parts = smk.split(data, i);
        for (slot, t) in smk.tracks.iter().enumerate() {
            if let (Some(t), Some(c)) = (t, parts.audio[slot]) {
                let _ = decode_audio(t, c);
            }
        }
        if let (Some(d), Some(v)) = (dec.as_mut(), parts.video) {
            let _ = d.decode(parts.palette, v);
            let _ = d.rgba();
        }
    }
}

#[test]
fn damaged_files_do_not_panic() {
    let mut rng = Rng(0xbad_f11e);
    let mut files = Vec::new();
    for _ in 0..6 {
        let spec = random_spec(&mut rng);
        files.push(smk_file(&mut rng, &spec));
    }
    if let Ok(real) = std::fs::read(sample_dir().join("wetlogo.smk")) {
        files.push(real[..60_000.min(real.len())].to_vec());
    }
    let mut n = 0;
    let mut panicked = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        let mut run = |name: String, m: &[u8]| {
            if std::panic::catch_unwind(|| exercise(m)).is_err() {
                panicked.push(name);
            }
        };
        // truncated at every length near the header, then at random places
        for len in (0..400.min(f.len())).chain((0..80).map(|_| rng.below(f.len() as u64) as usize)) {
            run(format!("file {fi} cut at {len}"), &f[..len]);
            n += 1;
        }
        // bit flips and corrupt bytes
        for k in 0..300 {
            let mut m = f.clone();
            for _ in 0..1 + rng.below(8) {
                let at = if rng.chance(50) { rng.below(m.len().min(220) as u64) } else { rng.below(m.len() as u64) } as usize;
                m[at] = if rng.chance(50) { m[at] ^ (1 << rng.below(8)) } else { rng.byte() };
            }
            run(format!("file {fi} mutation {k}"), &m);
            n += 1;
        }
        // corrupt size fields: the header's, the frame sizes
        for at in (4..104).step_by(4).chain((104..160.min(f.len())).step_by(4)) {
            for v in [0u32, 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x0100_0000] {
                let mut m = f.clone();
                if let Some(w) = m.get_mut(at..at + 4) {
                    w.copy_from_slice(&v.to_le_bytes());
                }
                run(format!("file {fi} size at {at} = {v:#x}"), &m);
                n += 1;
            }
        }
    }
    assert!(panicked.is_empty(), "panics on damaged input: {panicked:?}");
    assert!(n > 2000);
}

#[test]
fn garbage_and_tiny_inputs() {
    assert!(Smk::parse(b"").is_err());
    assert!(Smk::parse(b"SMK2").is_err());
    assert!(Smk::parse(b"RIFF\0\0\0\0AVI LIST").is_err());
    assert!(!cfa4_smacker::probe(b"SMK"));
    assert!(cfa4_smacker::probe(b"SMK4 anything"));
    let mut rng = Rng(7);
    for _ in 0..300 {
        let mut d = vec![0u8; 100 + rng.below(300) as usize];
        d.iter_mut().for_each(|b| *b = rng.byte());
        d[..4].copy_from_slice(b"SMK2");
        exercise(&d);
    }
}

// ---------------------------------------------------------------------------------------------
// Limits and damaged-frame details
// ---------------------------------------------------------------------------------------------

/// The index counts a chunk's samples as the decoder does: past a "no data" first byte, the
/// chunk that is decoded gives the length.
#[test]
fn packed_chunk_length_follows_the_decoded_chunk() {
    let t = Track { rate: 22050, channels: 1, bits: 8, kind: TrackKind::Packed };
    // declared 10240 bytes, but its first byte says "no data"; the rest is a chunk of 40 bytes
    let chunk = [0u8, 40, 0, 0, 0, 0x01, 0, 0, 0];
    assert_eq!(cfa4_smacker::chunk_samples(&t, &chunk), 40);
    // an ordinary chunk: its own length
    let chunk = [40u8, 0, 0, 0, 0x01, 0, 0];
    assert_eq!(cfa4_smacker::chunk_samples(&t, &chunk), 40);
    // too big to unpack: the declared length stays, nothing is decoded
    let chunk = [0u8, 0, 0, 0x7F, 0x01, 0, 0];
    assert!(decode_audio(&t, &chunk).is_err());
}

/// A palette update that fails halfway keeps the entries it changed before the failure; the
/// frame is damaged and has no video.
#[test]
fn a_failed_palette_update_keeps_what_it_changed() {
    let mut rng = Rng(11);
    let spec = random_spec(&mut rng);
    let data = smk_file(&mut rng, &spec);
    let smk = Smk::parse(&data).unwrap();
    let mut dec = Decoder::new(&smk).unwrap();
    // one new entry (index 0, colour 63 63 63), then a copy of 6 entries from offset 254
    let chunk = [63u8, 63, 63, 0x40 | 5, 254, 0, 0, 0];
    assert!(dec.update_palette(&chunk).is_err());
    assert_eq!(&dec.indexed().1[..3], &[0xFF, 0xFF, 0xFF]);
}

/// A header that declares a huge tree and holds a short one does not allocate for the declared size.
#[test]
fn a_short_header_tree_is_cheap() {
    let mut rng = Rng(12);
    let spec = random_spec(&mut rng);
    let mut data = smk_file(&mut rng, &spec);
    for at in [56usize, 60, 64, 68] {
        data[at..at + 4].copy_from_slice(&0x0FFF_FFF0u32.to_le_bytes());
    }
    let started = std::time::Instant::now();
    if let Ok(smk) = Smk::parse(&data) {
        let _ = Decoder::new(&smk);
    }
    assert!(started.elapsed().as_secs() < 5);
}

/// A file that ends inside the video data of its last frame still yields the audio of that frame
/// (and its palette); the picture is lost.
#[test]
fn a_file_cut_in_the_last_video_matches_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("smacker truncated oracle") else { return };
    let mut rng = Rng(0x7c07);
    let mut checked = 0;
    for n in 0..80 {
        let spec = random_spec(&mut rng);
        let data = smk_file(&mut rng, &spec);
        let Ok(smk) = Smk::parse(&data) else { continue };
        let Some(last) = smk.frames.checked_sub(1) else { continue };
        let Some(frame) = smk.frame_bytes(&data, last) else { continue };
        let Some(video) = smk.split_frame(last, frame).video else { continue };
        if video.len() < 3 {
            continue;
        }
        let start = video.as_ptr() as usize - data.as_ptr() as usize;
        let cut = start + 1 + rng.below(video.len() as u64 - 2) as usize;
        check(&ff, &data[..cut], &format!("t{n}"));
        checked += 1;
    }
    assert!(checked > 20, "{checked} cut files");
}
