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

use cfa4_smacker::{Decoder, Smk, Track, TrackKind, decode_audio};
use cfa4_testkit::smk::{Rng, random_spec, smk_file};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------------------------

fn run_ffmpeg(ff: &Path, input: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new(ff).args(["-v", "error", "-max_error_rate", "1.0", "-i"]).arg(input).args(args).output().ok()?;
    if !out.status.success() {
        eprintln!("ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    Some(out.stdout)
}

/// ffmpeg's video frames as (pts, sha256 of the rgb24 picture).
fn ffmpeg_video(ff: &Path, input: &Path) -> Option<Vec<(i64, String)>> {
    let out = run_ffmpeg(ff, input, &["-map", "0:v:0", "-an", "-fps_mode", "passthrough", "-pix_fmt", "rgb24", "-f", "framehash", "-hash", "sha256", "-"])?;
    let text = String::from_utf8_lossy(&out).into_owned();
    let mut v = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        v.push((f.get(2)?.parse().ok()?, f.get(5)?.to_string()));
    }
    Some(v)
}

/// ffmpeg's samples of the `k`-th audio stream, 8-bit as the code less 128.
fn ffmpeg_audio(ff: &Path, input: &Path, k: usize, b16: bool) -> Option<Vec<i32>> {
    let map = format!("0:a:{k}");
    let fmt: [&str; 4] = if b16 { ["pcm_s16le", "-f", "s16le", "-"] } else { ["pcm_u8", "-f", "u8", "-"] };
    let out = run_ffmpeg(ff, input, &["-map", &map, "-vn", "-c:a", fmt[0], fmt[1], fmt[2], fmt[3]])?;
    Some(if b16 {
        out.chunks_exact(2).map(|c| i32::from(i16::from_le_bytes([c[0], c[1]]))).collect()
    } else {
        out.iter().map(|&b| i32::from(b) - 128).collect()
    })
}

fn rgb24(d: &Decoder) -> Vec<u8> {
    d.rgba().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
}

/// What ffmpeg should output for `data`: video frames and each audio track's samples.
#[allow(clippy::type_complexity)]
fn expected(data: &[u8]) -> Option<(Vec<(i64, String)>, Vec<(Track, Vec<i32>)>)> {
    let smk = Smk::parse(data).ok()?;
    let mut dec = Decoder::new(&smk).ok()?;
    let mut video = Vec::new();
    let tracks: Vec<(usize, Track)> = smk.tracks.iter().enumerate().filter_map(|(i, t)| t.map(|t| (i, t))).collect();
    let mut audio: Vec<(Track, Vec<i32>)> = tracks.iter().map(|&(_, t)| (t, Vec::new())).collect();
    for i in 0..smk.frames {
        let parts = smk.split(data, i);
        for (n, &(slot, t)) in tracks.iter().enumerate() {
            if let Some(chunk) = parts.audio[slot]
                && let Ok(s) = decode_audio(&t, chunk)
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
    let Some((video, audio)) = expected(data) else {
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
        let want = ffmpeg_audio(ff, &input, k, t.bits == 16);
        let want = want.unwrap_or_default();
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
    for f in &files {
        // truncated at every length near the header, then at random places
        for len in (0..400.min(f.len())).chain((0..80).map(|_| rng.below(f.len() as u64) as usize)) {
            exercise(&f[..len]);
            n += 1;
        }
        // bit flips and corrupt bytes
        for _ in 0..300 {
            let mut m = f.clone();
            for _ in 0..1 + rng.below(8) {
                let at = if rng.chance(50) { rng.below(m.len().min(220) as u64) } else { rng.below(m.len() as u64) } as usize;
                m[at] = if rng.chance(50) { m[at] ^ (1 << rng.below(8)) } else { rng.byte() };
            }
            exercise(&m);
            n += 1;
        }
    }
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
