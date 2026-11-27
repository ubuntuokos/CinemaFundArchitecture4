//! Bit-exactness against ffmpeg's TTA decoder on files from ffmpeg's TTA encoder (an external
//! oracle only): 8-, 16- and 24-bit audio, mono to 8 channels, several rates and lengths
//! (including a last frame shorter than the others and a file shorter than one frame); damaged
//! files never panic. Skips without ffmpeg unless `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::{fixtures, oracle};

/// Tones, a sweep, noise, silence, a DC step and clipping, decorrelated across channels.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,0.7),0.5*sin(2*PI*440*t),if(lt(t,1.4),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,2.1),(random(0)-0.5)*1.6,if(lt(t,2.4),0,if(lt(t,2.6),0.25,1.2*sin(2*PI*1500*t))))))|0.3*sin(2*PI*220*t)+0.3*(random(1)-0.5)|0.2*sin(2*PI*3*t)|0.7*sin(2*PI*50*t)*sin(2*PI*1000*t)|(random(2)-0.5)*0.01|0.5*sin(2*PI*60*t)':s=RATE:d=DUR";

fn ffmpeg() -> Option<PathBuf> {
    let f = oracle::ffmpeg();
    if f.is_none() {
        oracle::skip("tta oracle", "ffmpeg not found");
    }
    f
}

fn run(cmd: &mut Command) -> bool {
    cmd.status().is_ok_and(|s| s.success())
}

/// The source encoded by ffmpeg's TTA encoder from PCM format `fmt`.
fn encode(name: &str, rate: u32, channels: u32, fmt: &str, dur: &str) -> Option<PathBuf> {
    let ffmpeg = ffmpeg()?;
    let out = fixtures::fixtures_dir("tta").join(name);
    let src = SOURCE.replace("RATE", &rate.to_string()).replace("DUR", dur);
    let pan = format!("pan={}c|{}", channels, (0..channels).map(|c| format!("c{c}=c{}", c % 6)).collect::<Vec<_>>().join("|"));
    let af = format!("{pan},aformat=sample_fmts={fmt}");
    fixtures::generate(&out, |tmp| {
        run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i", &src, "-af", &af, "-c:a", "tta", "-f", "tta"]).arg(tmp))
    })
}

/// ffmpeg's decode of `file` as s32le words.
fn ffmpeg_decode(file: &Path) -> Option<Vec<u32>> {
    let ffmpeg = ffmpeg()?;
    let out = file.with_extension("ff.s32");
    let out = fixtures::generate(&out, |tmp| {
        run(Command::new(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(file).args(["-f", "s32le", "-c:a", "pcm_s32le"]).arg(tmp))
    })?;
    let b = std::fs::read(out).ok()?;
    Some(b.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect())
}

/// Our decode, interleaved and left-justified to 32 bits.
fn ours(info: &cfa4_tta::Info, planes: &[Vec<i32>]) -> Vec<u32> {
    let n = planes.first().map_or(0, Vec::len);
    let shift = 32 - u32::from(info.bits);
    (0..n).flat_map(|i| planes.iter().map(move |p| (p[i] as u32) << shift)).collect()
}

#[test]
fn ffmpeg_encoded_files_match_ffmpeg() {
    let cases: &[(&str, u32, u32, &str, &str)] = &[
        ("s16_stereo.tta", 44_100, 2, "s16", "3.1"),
        ("s16_mono.tta", 22_050, 1, "s16", "3.1"),
        ("u8_stereo.tta", 11_025, 2, "u8", "3.1"),
        ("s24_stereo.tta", 96_000, 2, "s32", "3.1"),
        ("s16_51.tta", 48_000, 6, "s16", "2.3"),
        ("s24_3ch.tta", 32_000, 3, "s32", "2.3"),
        ("s16_8ch.tta", 48_000, 8, "s16", "1.5"),
        ("s16_short.tta", 8_000, 2, "s16", "0.4"),
        ("s16_odd_rate.tta", 37_800, 2, "s16", "2.0"),
        ("s16_whole_frames.tta", 245, 2, "s16", "4"),
    ];
    let mut tested = 0;
    for &(name, rate, ch, fmt, dur) in cases {
        let Some(file) = encode(name, rate, ch, fmt, dur) else { continue };
        let Some(want) = ffmpeg_decode(&file) else { continue };
        let data = std::fs::read(&file).expect("read fixture");
        let (info, planes) = cfa4_tta::decode_file(&data, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(u32::from(info.channels), ch, "{name}: channels");
        assert_eq!(info.sample_rate, rate, "{name}: rate");
        let got = ours(&info, &planes);
        assert_eq!(got.len(), want.len(), "{name}: length");
        if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            panic!("{name}: first mismatch at value {i}: ours {:#010x}, ffmpeg {:#010x}", got[i], want[i]);
        }
        tested += 1;
    }
    if tested == 0 {
        oracle::skip("tta oracle", "no fixtures");
    }
}

/// Flipped bits, cuts and noise: an error or garbage samples, never a panic.
#[test]
fn damaged_files_never_panic() {
    let Some(file) = encode("s16_stereo_damage.tta", 44_100, 2, "s16", "2.5") else { return };
    let src = std::fs::read(&file).expect("read fixture");
    let mut seed = 0x7A7A_0001u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for round in 0..400 {
        let mut d = src.clone();
        match round % 4 {
            0 => {
                let at = rnd() as usize % d.len();
                d[at] ^= 1 << (rnd() % 8);
            }
            1 => d.truncate(rnd() as usize % d.len()),
            2 => {
                let start = rnd() as usize % d.len();
                for b in d.iter_mut().skip(start).take(1 + rnd() as usize % 200) {
                    *b = rnd() as u8;
                }
            }
            _ => {
                // a damaged seek table: frames are then read one after the other
                let at = 22 + rnd() as usize % 8;
                if let Some(b) = d.get_mut(at) {
                    *b ^= 0x40;
                }
                let start = 40 + rnd() as usize % (d.len() - 40);
                if let Some(b) = d.get_mut(start) {
                    *b = rnd() as u8;
                }
            }
        }
        let _ = cfa4_tta::decode_file(&d, None);
        let _ = cfa4_tta::decode_file(&d, Some(b"key"));
    }
}
