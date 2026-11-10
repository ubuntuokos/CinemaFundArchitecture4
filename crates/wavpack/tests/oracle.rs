//! Bit-exactness against ffmpeg's WavPack decoder (on ffmpeg-encoded files) and against the
//! reference `wvunpack` (on files from the reference `wavpack` encoder, which also makes the hybrid,
//! float, extended-integer and high / extra modes ffmpeg cannot). Both programs are external
//! oracles only. The ffmpeg tests skip without ffmpeg unless `CFA4_REQUIRE_ORACLES=1`; the
//! `wavpack` CLI tests skip without the CLI.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::{fixtures, oracle};

/// Tones, a sweep, noise, silence, a DC step and clipping, decorrelated across channels.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,0.7),0.5*sin(2*PI*440*t),if(lt(t,1.4),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,2.1),(random(0)-0.5)*1.6,if(lt(t,2.4),0,if(lt(t,2.6),0.25,1.2*sin(2*PI*1500*t))))))|0.3*sin(2*PI*220*t)+0.3*(random(1)-0.5)|0.2*sin(2*PI*3*t)|0.7*sin(2*PI*50*t)*sin(2*PI*1000*t)|(random(2)-0.5)*0.01|0.5*sin(2*PI*60*t)':s=RATE:d=3.1";

fn ffmpeg() -> Option<PathBuf> {
    let f = oracle::ffmpeg();
    if f.is_none() {
        oracle::skip("wavpack oracle", "ffmpeg not found");
    }
    f
}

fn run(cmd: &mut Command) -> bool {
    cmd.status().is_ok_and(|s| s.success())
}

/// The `channels`-channel source at `rate` as a WAV file of ffmpeg PCM codec `pcm`.
fn source_wav(name: &str, rate: u32, channels: u32, pcm: &str) -> Option<PathBuf> {
    let ffmpeg = ffmpeg()?;
    let out = fixtures::fixtures_dir("wavpack").join(name);
    let src = SOURCE.replace("RATE", &rate.to_string());
    let pan = format!("pan={}c|{}", channels, (0..channels).map(|c| format!("c{c}=c{}", c % 6)).collect::<Vec<_>>().join("|"));
    fixtures::generate(&out, |tmp| {
        run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i", &src, "-af", &pan, "-c:a", pcm, "-f", "wav"]).arg(tmp))
    })
}

/// ffmpeg's decode of `file`: s32le for integer streams, f32le for float, as raw 32-bit words.
fn ffmpeg_decode(file: &Path, float: bool) -> Option<Vec<u32>> {
    let ffmpeg = ffmpeg()?;
    let out = file.with_extension(if float { "ff.f32" } else { "ff.s32" });
    let (fmt, codec) = if float { ("f32le", "pcm_f32le") } else { ("s32le", "pcm_s32le") };
    let out = fixtures::generate(&out, |tmp| run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(file).args(["-f", fmt, "-c:a", codec]).arg(tmp)))?;
    Some(words(&std::fs::read(out).ok()?))
}

fn words(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect()
}

/// Our decode, interleaved and left-justified to 32 bits (floats as bits).
fn ours(info: &cfa4_wavpack::Info, planes: &[Vec<i32>]) -> Vec<u32> {
    let n = planes.first().map_or(0, Vec::len);
    let shift = if info.float { 0 } else { 32 - info.bits };
    (0..n).flat_map(|i| planes.iter().map(move |p| (p[i] as u32) << shift)).collect()
}

fn first_mismatch(a: &[u32], b: &[u32]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

fn check(name: &str, ours: &[u32], want: &[u32]) {
    assert_eq!(ours.len(), want.len(), "{name}: length");
    if let Some(i) = first_mismatch(ours, want) {
        panic!("{name}: first mismatch at value {i}: ours {:#010x}, oracle {:#010x}", ours[i], want[i]);
    }
}

/// ffmpeg-encoded `.wv` files of every sample format and compression level, mono to 6 channels.
#[test]
fn ffmpeg_encoded_files_match_ffmpeg() {
    let cases: &[(&str, u32, u32, &str, &[&str])] = &[
        ("s16_stereo_l1.wv", 44_100, 2, "pcm_s16le", &["-compression_level", "1"]),
        ("s16_stereo_l3.wv", 48_000, 2, "pcm_s16le", &["-compression_level", "3"]),
        ("s16_stereo_l8.wv", 48_000, 2, "pcm_s16le", &["-compression_level", "8"]),
        ("s16_mono_l5.wv", 22_050, 1, "pcm_s16le", &["-compression_level", "5"]),
        ("s16_mono_l0.wv", 8_000, 1, "pcm_s16le", &["-compression_level", "0"]),
        ("u8_stereo.wv", 11_025, 2, "pcm_u8", &[]),
        ("s24_stereo_l6.wv", 96_000, 2, "pcm_s24le", &["-compression_level", "6"]),
        ("s32_stereo.wv", 48_000, 2, "pcm_s32le", &[]),
        ("f32_stereo.wv", 48_000, 2, "pcm_f32le", &[]),
        ("f32_mono_l7.wv", 44_100, 1, "pcm_f32le", &["-compression_level", "7"]),
        ("s16_51_l4.wv", 48_000, 6, "pcm_s16le", &["-compression_level", "4"]),
        ("s24_3ch.wv", 32_000, 3, "pcm_s24le", &[]),
        ("s16_odd_rate.wv", 37_800, 2, "pcm_s16le", &[]),
        ("s16_joint.wv", 44_100, 2, "pcm_s16le", &["-joint_stereo", "1"]),
        ("s16_nojoint.wv", 44_100, 2, "pcm_s16le", &["-joint_stereo", "0"]),
    ];
    let Some(ffmpeg) = ffmpeg() else { return };
    for (name, rate, ch, pcm, extra) in cases {
        let Some(wav) = source_wav(&format!("{name}.src.wav"), *rate, *ch, pcm) else { return };
        let out = fixtures::fixtures_dir("wavpack").join(name);
        let made = fixtures::generate(&out, |tmp| {
            run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(&wav).args(["-c:a", "wavpack"]).args(*extra).args(["-f", "wv"]).arg(tmp))
        });
        let Some(out) = made else {
            oracle::skip("wavpack oracle", &format!("ffmpeg could not encode {name}"));
            return;
        };
        let bytes = std::fs::read(&out).expect("read");
        let (info, planes) = cfa4_wavpack::decode_file(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((info.channels, info.sample_rate), (*ch as usize, *rate), "{name}");
        let Some(want) = ffmpeg_decode(&out, info.float) else { return };
        check(name, &ours(&info, &planes), &want);
    }
}

/// Files from the reference encoder: hybrid lossy (with and without noise shaping), extra modes,
/// float and > 24-bit integers, false stereo, many channels, compared with `wvunpack --raw`.
#[test]
fn reference_encoded_files_match_wvunpack() {
    let (Some(wavpack), Some(wvunpack)) = (oracle::find_tool("wavpack"), oracle::find_tool("wvunpack")) else {
        eprintln!("SKIPPED (wavpack oracle): the reference wavpack / wvunpack CLI is not installed");
        return;
    };
    let cases: &[(&str, u32, u32, &str, &[&str])] = &[
        ("ref_s16_default.wv", 44_100, 2, "pcm_s16le", &[]),
        ("ref_s16_fast.wv", 44_100, 2, "pcm_s16le", &["-f"]),
        ("ref_s16_high_x6.wv", 48_000, 2, "pcm_s16le", &["-hh", "-x6"]),
        ("ref_s16_mono_x3.wv", 48_000, 1, "pcm_s16le", &["-h", "-x3"]),
        ("ref_s24_hx4.wv", 96_000, 2, "pcm_s24le", &["-h", "-x4"]),
        ("ref_s32.wv", 48_000, 2, "pcm_s32le", &["-x1"]),
        ("ref_f32.wv", 48_000, 2, "pcm_f32le", &["-x2"]),
        ("ref_f32_mono_hh.wv", 44_100, 1, "pcm_f32le", &["-hh"]),
        ("ref_u8.wv", 22_050, 2, "pcm_u8", &[]),
        ("ref_6ch_h.wv", 48_000, 6, "pcm_s16le", &["-h"]),
        ("ref_8ch.wv", 48_000, 8, "pcm_s24le", &[]),
        ("ref_hybrid_256.wv", 44_100, 2, "pcm_s16le", &["-b256"]),
        ("ref_hybrid_mono_128.wv", 44_100, 1, "pcm_s16le", &["-b128", "-h"]),
        ("ref_hybrid_24bit.wv", 48_000, 2, "pcm_s24le", &["-b3.0", "-x3"]),
        ("ref_hybrid_shaped.wv", 44_100, 2, "pcm_s16le", &["-b2.5", "-s0.5"]),
        ("ref_hybrid_neg_shaped.wv", 44_100, 2, "pcm_s16le", &["-b2.2", "-s-0.5", "-hh"]),
        ("ref_hybrid_float.wv", 48_000, 2, "pcm_f32le", &["-b3.5"]),
        ("ref_hybrid_32bit.wv", 48_000, 2, "pcm_s32le", &["-b4"]),
        ("ref_hybrid_6ch.wv", 48_000, 6, "pcm_s16le", &["-b3", "-x2"]),
        ("ref_hybrid_8bit.wv", 22_050, 1, "pcm_u8", &["-b2.1"]),
    ];
    let dir = fixtures::fixtures_dir("wavpack");
    for (name, rate, ch, pcm, extra) in cases {
        let Some(wav) = source_wav(&format!("{name}.src.wav"), *rate, *ch, pcm) else { return };
        let out = dir.join(name);
        let Some(out) = fixtures::generate(&out, |tmp| run(Command::new(&wavpack).args(["-q", "-y"]).args(*extra).arg(&wav).arg("-o").arg(tmp))) else {
            panic!("wavpack could not encode {name}");
        };
        let raw = out.with_extension("ref.raw");
        let Some(raw) = fixtures::generate(&raw, |tmp| run(Command::new(&wvunpack).args(["-q", "-y", "--raw"]).arg(&out).arg("-o").arg(tmp))) else {
            panic!("wvunpack could not decode {name}");
        };
        let bytes = std::fs::read(&out).expect("read");
        let (info, planes) = cfa4_wavpack::decode_file(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((info.channels, info.sample_rate), (*ch as usize, *rate), "{name}");
        // wvunpack --raw writes the source's PCM layout (little-endian; 8-bit unsigned)
        let raw = std::fs::read(raw).expect("raw");
        let bytes_per = (info.bits / 8) as usize;
        let want: Vec<u32> = raw
            .chunks_exact(bytes_per)
            .map(|b| match (info.float, bytes_per) {
                (true, _) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (false, 1) => u32::from(b[0] ^ 0x80) << 24,
                (false, 2) => u32::from(u16::from_le_bytes([b[0], b[1]])) << 16,
                (false, 3) => (u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16) << 8,
                _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            })
            .collect();
        check(name, &ours(&info, &planes), &want);
        // ffmpeg agrees on the lossless ones as well
        if !info.hybrid
            && let Some(ff) = ffmpeg_decode(&out, info.float)
        {
            check(&format!("{name} (ffmpeg)"), &ours(&info, &planes), &ff);
        }
    }
}

/// Lossless means lossless: the decode of an ffmpeg-encoded file is the source PCM.
#[test]
fn lossless_round_trip_is_the_source() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let Some(wav) = source_wav("rt.src.wav", 48_000, 2, "pcm_s16le") else { return };
    let out = fixtures::fixtures_dir("wavpack").join("rt.wv");
    let Some(out) =
        fixtures::generate(&out, |tmp| run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(&wav).args(["-c:a", "wavpack", "-f", "wv"]).arg(tmp)))
    else {
        return;
    };
    let (_, planes) = cfa4_wavpack::decode_file(&std::fs::read(out).expect("read")).expect("decode");
    let src = std::fs::read(wav).expect("wav");
    let data = src.windows(4).position(|w| w == b"data").expect("data chunk") + 8;
    let pcm: Vec<i32> = src[data..].chunks_exact(2).map(|b| i32::from(i16::from_le_bytes([b[0], b[1]]))).collect();
    let ours: Vec<i32> = (0..planes[0].len()).flat_map(|i| [planes[0][i], planes[1][i]]).collect();
    assert_eq!(ours, pcm);
}

/// A damaged block decodes to silence of its length; the blocks around it are untouched.
#[test]
fn damaged_block_is_muted() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let Some(wav) = source_wav("dmg.src.wav", 44_100, 2, "pcm_s16le") else { return };
    let out = fixtures::fixtures_dir("wavpack").join("dmg.wv");
    let Some(out) = fixtures::generate(&out, |tmp| {
        run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(&wav).args(["-c:a", "wavpack", "-frame_size", "4096", "-f", "wv"]).arg(tmp))
    }) else {
        return;
    };
    let good = std::fs::read(out).expect("read");
    let (_, clean) = cfa4_wavpack::decode_file(&good).expect("decode");
    let blocks: Vec<(usize, usize)> = {
        let mut v = Vec::new();
        let mut p = 0;
        for (h, b) in cfa4_wavpack::blocks(&good) {
            let start = good[p..].windows(4).position(|w| w == b"wvpk").map_or(p, |i| p + i);
            v.push((start, b.len()));
            p = start + h.size;
        }
        v
    };
    assert!(blocks.len() > 4);
    let (start, len) = blocks[2];
    let mut bad = good.clone();
    for b in &mut bad[start + len - 40..start + len - 20] {
        *b ^= 0x5a;
    }
    let (_, planes) = cfa4_wavpack::decode_file(&bad).expect("decode damaged");
    for (p, c) in planes.iter().zip(&clean) {
        assert_eq!(p.len(), c.len());
        assert_eq!(p[..2 * 4096], c[..2 * 4096]);
        assert!(p[2 * 4096..3 * 4096].iter().all(|v| *v == 0));
        assert_eq!(p[3 * 4096..], c[3 * 4096..]);
    }
    // every truncation decodes or fails cleanly
    for n in (0..good.len()).step_by(997) {
        let _ = cfa4_wavpack::decode_file(&good[..n]);
    }
}
