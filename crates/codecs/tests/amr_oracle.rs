//! AMR-NB and AMR-WB in their storage files (`.amr`, `.awb`), 3GP / QuickTime (`samr`, `sawb`)
//! and Matroska, read through the media sources: every sample must equal opencore's decode of
//! the stream (the `cfa4-amr-oracle` helper, scripts/ci/install-amr-oracle.sh, which also
//! encodes it; ffmpeg only remuxes). Read front to back and after jumps back and forth.
//! Skipped without the helper or ffmpeg unless `CFA4_REQUIRE_ORACLES=1`.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;

const TOOL: &str = "cfa4-amr-oracle";

/// Speech-like tones, a sweep, silence (comfort noise with DTX) and noise, three times over: 600
/// frames, so jumps restart from decoder checkpoints (`CHECKPOINT_INTERVAL` = 250 packets).
const SOURCE: &str = "aevalsrc=exprs='if(lt(mod(t,4),1),0.4*sin(2*PI*(180+40*sin(7*t))*t)*(1+0.5*sin(2*PI*3*t)),if(lt(mod(t,4),2),0.6*sin(2*PI*(100+2500*mod(t,4))*mod(t,4)),if(lt(mod(t,4),3),0,(random(0)-0.5)*0.8)))':s=RATE:d=12";

fn tool() -> Option<PathBuf> {
    let t = cfa4_testkit::oracle::find_tool(TOOL);
    if t.is_none() {
        cfa4_testkit::oracle::skip("amr oracle", "cfa4-amr-oracle not found (scripts/ci/install-amr-oracle.sh)");
    }
    t
}

fn run(tool: &Path, args: &[&str]) -> bool {
    Command::new(tool).args(args).status().is_ok_and(|s| s.success())
}

/// The storage file for `wide` (encoded by the oracle, DTX on, mode changing every 3 frames)
/// and opencore's decode of it.
fn stream(ff: &Path, tool: &Path, wide: bool) -> Option<(PathBuf, Vec<i16>)> {
    let (tag, rate, ext) = if wide { ("wb", 16000, "awb") } else { ("nb", 8000, "amr") };
    let raw = fixture(ff, &format!("amr12_{tag}.s16"), &["-f", "lavfi", "-i", &SOURCE.replace("RATE", &rate.to_string()), "-f", "s16le", "-c:a", "pcm_s16le"])?;
    let file = dir().join(format!("amr12_{tag}.{ext}"));
    let enc = format!("enc-{tag}");
    let file = cfa4_testkit::fixtures::generate(&file, |tmp| run(tool, &[&enc, "cycle", "1", raw.to_str().unwrap_or(""), tmp.to_str().unwrap_or("")]))?;
    let out = dir().join(format!("amr12_{tag}.ref.s16"));
    let dec = format!("dec-{tag}");
    assert!(run(tool, &[&dec, file.to_str()?, out.to_str()?]), "oracle decode");
    let pcm = std::fs::read(&out).ok()?.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    Some((file, pcm))
}

fn check(name: &str, path: &Path, wide: bool, reference: &[i16]) {
    let rate = if wide { 16000 } else { 8000 };
    let open = || cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let s = open();
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    assert_eq!((info.sample_rate, info.channels), (rate, 1), "{name}");
    assert_eq!(info.codec, if wide { "AMR-WB" } else { "AMR-NB" }, "{name}");
    let read = |s: &cfa4_media::SharedSource, from: usize, n: usize| -> Vec<i16> {
        let mut out = Vec::with_capacity(n);
        let mut at = from;
        while at < from + n {
            let k = 1000.min(from + n - at);
            let b = s.audio(at as i64, k, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
            out.extend(b.channels[0].iter().take(k).map(|&v| (v * 32768.0).round() as i16));
            at += k;
        }
        out
    };
    let ours = read(&s, 0, reference.len());
    if let Some(i) = ours.iter().zip(reference).position(|(a, b)| a != b) {
        panic!("{name}: sample {i} differs: ours {}, opencore {}", ours[i], reference[i]);
    }
    // Jumps into a tone, the sweep, comfort noise and noise, out of order on one source: the
    // first decodes from the start and leaves checkpoints, the others restart from one. AMR's
    // predictors never forget a wrong start, so anything but the saved state would differ.
    let s = open();
    for at in [0.85, 0.3, 0.6, 0.45, 0.1] {
        let from = (reference.len() as f64 * at) as usize + 77;
        let n = reference.len() / 20;
        let after = read(&s, from, n);
        if let Some(i) = after.iter().zip(&reference[from..from + n]).position(|(a, b)| a != b) {
            panic!("{name}: after a jump to {from}: sample {} differs: ours {}, opencore {}", from + i, after[i], reference[from + i]);
        }
    }
}

#[test]
fn amr_in_containers() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let Some(tool) = tool() else { return };
    for wide in [false, true] {
        let Some((file, reference)) = stream(&ff, &tool, wide) else {
            cfa4_testkit::oracle::skip("amr oracle", "could not encode the test stream");
            return;
        };
        let tag = if wide { "wb" } else { "nb" };
        check(&format!("amr_{tag}.{}", if wide { "awb" } else { "amr" }), &file, wide, &reference);
        for ext in ["3gp", "mov", "mkv"] {
            let name = format!("amr12_{tag}.{ext}");
            let Some(path) = fixture(&ff, &name, &["-i", file.to_str().unwrap_or(""), "-c", "copy"]) else {
                cfa4_testkit::oracle::skip(&name, "ffmpeg can't write it");
                continue;
            };
            check(&name, &path, wide, &reference);
        }
    }
}
