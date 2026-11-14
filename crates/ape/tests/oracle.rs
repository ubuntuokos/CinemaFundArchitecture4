//! Bit-exactness against the reference Monkey's Audio decoder (`mac … -d`) on files from the
//! reference encoder (`mac … -cNNNN`), and against ffmpeg's APE decoder. Both programs are
//! external oracles only. ffmpeg makes the source WAV files. The tests skip when `mac` (or
//! `CFA4_MAC`) or ffmpeg is missing; CI builds `mac` from the pinned SDK
//! (`scripts/ci/install-ffmpeg-oracle-deps.sh`).

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::{fixtures, oracle};

/// Tones, a sweep, noise, silence, a DC step and clipping, decorrelated across channels.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,0.7),0.5*sin(2*PI*440*t),if(lt(t,1.4),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,2.1),(random(0)-0.5)*1.6,if(lt(t,2.4),0,if(lt(t,2.6),0.25,1.2*sin(2*PI*1500*t))))))|0.3*sin(2*PI*220*t)+0.3*(random(1)-0.5)|0.2*sin(2*PI*3*t)|0.7*sin(2*PI*50*t)*sin(2*PI*1000*t)|(random(2)-0.5)*0.01|0.5*sin(2*PI*60*t)':s=RATE:d=DUR";

fn run(cmd: &mut Command) -> bool {
    cmd.stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn tools() -> Option<(PathBuf, PathBuf)> {
    let ffmpeg = oracle::ffmpeg();
    let mac = oracle::find_tool("mac");
    match (ffmpeg, mac) {
        (Some(f), Some(m)) => Some((f, m)),
        (None, _) => {
            oracle::skip("ape oracle", "ffmpeg not found");
            None
        }
        (_, None) => {
            eprintln!("SKIPPED (ape oracle): the reference Monkey's Audio `mac` CLI is not installed");
            None
        }
    }
}

/// The source as a WAV file of ffmpeg PCM codec `pcm`.
fn source_wav(ffmpeg: &Path, name: &str, rate: u32, channels: u32, pcm: &str, seconds: f32) -> Option<PathBuf> {
    let out = fixtures::fixtures_dir("ape").join(name);
    let src = SOURCE.replace("RATE", &rate.to_string()).replace("DUR", &seconds.to_string());
    let pan = format!("pan={}c|{}", channels, (0..channels).map(|c| format!("c{c}=c{}", c % 6)).collect::<Vec<_>>().join("|"));
    fixtures::generate(&out, |tmp| {
        run(Command::new(ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i", &src, "-af", &pan, "-c:a", pcm, "-f", "wav"]).arg(tmp))
    })
}

/// The payload of a WAV file's `data` chunk.
fn wav_data(bytes: &[u8]) -> Option<&[u8]> {
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        if id == b"data" {
            return bytes.get(at + 8..(at + 8 + len).min(bytes.len()));
        }
        at += 8 + len + (len & 1);
    }
    None
}

/// Our decode as the PCM bytes the reference writes.
fn ours_pcm(info: &cfa4_ape::Info, planes: &[Vec<i32>]) -> Vec<u8> {
    let n = planes.first().map_or(0, Vec::len);
    let mut out = Vec::new();
    for i in 0..n {
        for p in planes {
            let v = p[i];
            match info.bits {
                8 => out.push((v + 128) as u8),
                16 => out.extend_from_slice(&(v as i16).to_le_bytes()),
                24 => out.extend_from_slice(&v.to_le_bytes()[..3]),
                _ => out.extend_from_slice(&v.to_le_bytes()),
            }
        }
    }
    out
}

fn check(name: &str, ours: &[u8], want: &[u8]) {
    assert_eq!(ours.len(), want.len(), "{name}: length");
    if let Some(i) = ours.iter().zip(want).position(|(a, b)| a != b) {
        panic!("{name}: first mismatch at byte {i}: ours {:#04x}, reference {:#04x}", ours[i], want[i]);
    }
}

/// Encode `wav` with `mac` at `level`, decode it with `mac -d` and with us.
fn round(mac: &Path, wav: &Path, name: &str, level: u32) -> Option<(cfa4_ape::Info, Vec<Vec<i32>>, Vec<u8>, PathBuf)> {
    let dir = fixtures::fixtures_dir("ape");
    let ape = fixtures::generate(&dir.join(name), |tmp| {
        let tmp_ape = tmp.with_extension("ape");
        let ok = run(Command::new(mac).arg(wav).arg(&tmp_ape).arg(format!("-c{level}")));
        ok && std::fs::rename(&tmp_ape, tmp).is_ok()
    })?;
    let refwav = fixtures::generate(&ape.with_extension("ref.wav"), |tmp| {
        let tmp_wav = tmp.with_extension("wav");
        let ok = run(Command::new(mac).arg(&ape).arg(&tmp_wav).arg("-d"));
        ok && std::fs::rename(&tmp_wav, tmp).is_ok()
    })?;
    let bytes = std::fs::read(&ape).expect("read ape");
    let (info, planes) = cfa4_ape::decode_file(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let reference = std::fs::read(&refwav).expect("read reference");
    let want = wav_data(&reference).expect("reference data chunk").to_vec();
    Some((info, planes, want, ape))
}

/// Every compression level and sample format, mono to 8 channels, odd rates and frame sizes.
#[test]
fn reference_encoded_files_match_mac() {
    let Some((ffmpeg, mac)) = tools() else { return };
    let cases: &[(&str, u32, u32, &str, u32, f32)] = &[
        ("s16_stereo_c1000.ape", 44_100, 2, "pcm_s16le", 1000, 3.1),
        ("s16_stereo_c2000.ape", 44_100, 2, "pcm_s16le", 2000, 3.1),
        ("s16_stereo_c3000.ape", 48_000, 2, "pcm_s16le", 3000, 3.1),
        ("s16_stereo_c4000.ape", 48_000, 2, "pcm_s16le", 4000, 3.1),
        ("s16_stereo_c5000.ape", 48_000, 2, "pcm_s16le", 5000, 3.1),
        ("s16_mono_c2000.ape", 22_050, 1, "pcm_s16le", 2000, 3.1),
        ("s16_mono_c5000.ape", 8_000, 1, "pcm_s16le", 5000, 3.1),
        ("u8_stereo_c3000.ape", 11_025, 2, "pcm_u8", 3000, 3.1),
        ("u8_mono_c1000.ape", 11_025, 1, "pcm_u8", 1000, 3.1),
        ("s24_stereo_c2000.ape", 96_000, 2, "pcm_s24le", 2000, 3.1),
        ("s24_stereo_c4000.ape", 48_000, 2, "pcm_s24le", 4000, 3.1),
        ("s24_mono_c5000.ape", 44_100, 1, "pcm_s24le", 5000, 3.1),
        ("s32_stereo_c2000.ape", 48_000, 2, "pcm_s32le", 2000, 3.1),
        ("s32_stereo_c5000.ape", 48_000, 2, "pcm_s32le", 5000, 3.1),
        ("s32_mono_c3000.ape", 44_100, 1, "pcm_s32le", 3000, 3.1),
        ("f32_stereo_c2000.ape", 48_000, 2, "pcm_f32le", 2000, 3.1),
        ("f32_stereo_c4000.ape", 44_100, 2, "pcm_f32le", 4000, 3.1),
        ("s16_51_c2000.ape", 48_000, 6, "pcm_s16le", 2000, 3.1),
        ("s16_71_c3000.ape", 48_000, 8, "pcm_s16le", 3000, 3.1),
        ("s16_3ch_c1000.ape", 32_000, 3, "pcm_s16le", 1000, 3.1),
        ("s16_7ch_c2000.ape", 48_000, 7, "pcm_s16le", 2000, 3.1),
        ("s16_4ch_c4000.ape", 44_100, 4, "pcm_s16le", 4000, 3.1),
        ("s24_51_c3000.ape", 48_000, 6, "pcm_s24le", 3000, 3.1),
        ("s24_4ch_c2000.ape", 48_000, 4, "pcm_s24le", 2000, 3.1),
        ("s32_4ch_c2000.ape", 48_000, 4, "pcm_s32le", 2000, 3.1),
        ("u8_51_c2000.ape", 22_050, 6, "pcm_u8", 2000, 3.1),
        ("s16_odd_rate.ape", 37_800, 2, "pcm_s16le", 2000, 3.1),
        // Several frames: 73 728 × 4 blocks per frame below level 5000 (and more at 5000).
        ("s16_long_c3000.ape", 48_000, 2, "pcm_s16le", 3000, 14.0),
        ("s16_long_c5000.ape", 48_000, 2, "pcm_s16le", 5000, 31.0),
        ("s16_tiny.ape", 44_100, 2, "pcm_s16le", 2000, 0.01),
    ];
    for (name, rate, ch, pcm, level, secs) in cases {
        let Some(wav) = source_wav(&ffmpeg, &format!("{name}.src.wav"), *rate, *ch, pcm, *secs) else { return };
        let Some((info, planes, want, _)) = round(&mac, &wav, name, *level) else {
            oracle::skip("ape oracle", &format!("mac could not encode {name}"));
            return;
        };
        assert_eq!((info.channels, info.sample_rate), (*ch as u16, *rate), "{name}");
        check(name, &ours_pcm(&info, &planes), &want);
        // A lossless codec: the source PCM comes back.
        let src = std::fs::read(&wav).expect("read source");
        check(&format!("{name} (source)"), &ours_pcm(&info, &planes), wav_data(&src).expect("source data"));
    }
}

/// ffmpeg's APE decoder agrees on the formats it supports.
#[test]
fn reference_encoded_files_match_ffmpeg() {
    let Some((ffmpeg, mac)) = tools() else { return };
    let cases: &[(&str, u32, u32, &str, u32)] = &[
        ("ff_s16_c1000.ape", 44_100, 2, "pcm_s16le", 1000),
        ("ff_s16_c5000.ape", 44_100, 2, "pcm_s16le", 5000),
        ("ff_s24_c3000.ape", 48_000, 2, "pcm_s24le", 3000),
        ("ff_u8_c2000.ape", 22_050, 1, "pcm_u8", 2000),
    ];
    for (name, rate, ch, pcm, level) in cases {
        let Some(wav) = source_wav(&ffmpeg, &format!("{name}.src.wav"), *rate, *ch, pcm, 3.1) else { return };
        let Some((info, planes, _, ape)) = round(&mac, &wav, name, *level) else { return };
        let out = ape.with_extension("ff.s32");
        let Some(out) = fixtures::generate(&out, |tmp| {
            run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(&ape).args(["-f", "s32le", "-c:a", "pcm_s32le"]).arg(tmp))
        }) else {
            oracle::skip("ape oracle", &format!("ffmpeg could not decode {name}"));
            return;
        };
        let want: Vec<i32> = std::fs::read(out).expect("read").chunks_exact(4).map(|w| i32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let shift = 32 - u32::from(info.bits);
        let n = planes.first().map_or(0, Vec::len);
        let ours: Vec<i32> = (0..n).flat_map(|i| planes.iter().map(move |p| p[i] << shift)).collect();
        assert_eq!(ours.len(), want.len(), "{name}: length");
        if let Some(i) = ours.iter().zip(&want).position(|(a, b)| a != b) {
            panic!("{name}: first mismatch at value {i}: ours {}, ffmpeg {}", ours[i], want[i]);
        }
    }
}

/// A damaged frame decodes to silence; the frames around it stay exact.
#[test]
fn a_damaged_frame_is_muted() {
    let Some((ffmpeg, mac)) = tools() else { return };
    let Some(wav) = source_wav(&ffmpeg, "damaged.src.wav", 48_000, 2, "pcm_s16le", 20.0) else { return };
    let Some((info, planes, _, ape)) = round(&mac, &wav, "damaged_src.ape", 2000) else { return };
    let mut bytes = std::fs::read(&ape).expect("read");
    let file = cfa4_ape::File::parse(&bytes).expect("parse");
    assert!(file.frames() >= 3, "frames: {}", file.frames());
    let per_frame = file.blocks_per_frame as usize;
    let mid = bytes.len() / 2;
    let frame_of_mid = (0..file.frames()).rev().find(|&i| file.frame_offset(i) <= mid as u64).expect("frame");
    drop(file);
    // Flip bytes in the middle of the file.
    for b in &mut bytes[mid..mid + 64] {
        *b ^= 0x5A;
    }
    let (_, damaged) = cfa4_ape::decode_file(&bytes).expect("decode");
    assert_eq!(damaged[0].len(), planes[0].len());
    for ch in 0..usize::from(info.channels) {
        for (i, (&a, &b)) in damaged[ch].iter().zip(&planes[ch]).enumerate() {
            if i / per_frame == frame_of_mid {
                assert_eq!(a, 0, "sample {i} of the damaged frame");
            } else {
                assert_eq!(a, b, "sample {i} outside the damaged frame");
            }
        }
    }
}

/// Truncated and corrupted files never panic.
#[test]
fn hostile_input_does_not_panic() {
    let Some((ffmpeg, mac)) = tools() else { return };
    let Some(wav) = source_wav(&ffmpeg, "hostile.src.wav", 44_100, 2, "pcm_s16le", 1.0) else { return };
    let Some((_, _, _, ape)) = round(&mac, &wav, "hostile.ape", 4000) else { return };
    let bytes = std::fs::read(&ape).expect("read");
    for cut in (0..bytes.len()).step_by(97) {
        let _ = cfa4_ape::decode_file(&bytes[..cut]);
    }
    let mut seed = 0x1234_5678u32;
    for _ in 0..200 {
        let mut b = bytes.clone();
        for _ in 0..8 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let at = seed as usize % b.len();
            b[at] = (seed >> 24) as u8;
        }
        let _ = cfa4_ape::decode_file(&b);
    }
}
