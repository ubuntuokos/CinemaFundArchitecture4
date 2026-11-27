//! Real Huffyuv files from ffmpeg's sample archive (<https://samples.ffmpeg.org/V-codecs/HuffYUV/>),
//! decoded by us and by ffmpeg: Huffyuv 1.x files (classic tables, no tables in the header) and
//! 2.x files of every predictor. The samples are never committed: fetch them into
//! `target/fixtures/fate/huffyuv/` (or `$CFA4_FIXTURES_DIR/fate/huffyuv/`); every missing file is
//! skipped, as is the whole test without ffmpeg.

use cfa4_huffyuv::{Decoder, Format};
use std::path::PathBuf;
use std::process::Command;

const SAMPLES: &[&str] = &[
    "yuv_v1.avi",
    "rgb_v1.avi",
    "huffyuv.avi",
    "huffyuv_decoding_problem.avi",
    "cons-huffyuv.avi",
    "yuv_predleft.avi",
    "yuv_predgrad.avi",
    "yuv_predmed.avi",
    "rgb_predleft.avi",
    "rgb_predgrad.avi",
    "rgb_predleftnodecorr.avi",
    "rgb24_interlaced.avi",
];

fn dir() -> PathBuf {
    std::env::var_os("CFA4_FIXTURES_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures"), PathBuf::from)
        .join("fate/huffyuv")
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

#[test]
fn fate_samples_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("cfa4-hfyu-fate-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("tmp");
    let mut failures = Vec::new();
    let mut tested = 0;
    for name in SAMPLES {
        let path = dir().join(name);
        let Ok(avi) = std::fs::read(&path) else {
            eprintln!("SKIP: sample {name} not fetched");
            continue;
        };
        let file = cfa4_avi::open(&avi).expect("avi");
        let (si, s) = file.streams.iter().enumerate().find(|(_, s)| s.video.is_some()).expect("video");
        let bih = s.video.as_ref().expect("bih");
        let mut dec = Decoder::new(bih.width.unsigned_abs(), bih.height.unsigned_abs(), bih.bit_count, &bih.extra).unwrap_or_else(|e| panic!("{name}: {e}"));
        let pix_fmt = if dec.format() == Format::Yuv422 { "yuv422p" } else { "gbrp" };
        let raw = tmp.join(format!("{name}.raw"));
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&path)
            .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", pix_fmt])
            .arg(&raw)
            .output()
            .expect("run ffmpeg");
        assert!(out.status.success(), "{name}: ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
        let want = std::fs::read(&raw).expect("raw");
        let mut ours = Vec::new();
        for i in 0..s.chunks.len() {
            let chunk = file.read_chunk(&avi, si, i).expect("chunk");
            match dec.decode(&chunk) {
                Ok(p) => match p.format {
                    Format::Yuv422 | Format::Yuv420 => ours.extend(p.planes.concat()),
                    _ => ours.extend([&p.planes[1], &p.planes[2], &p.planes[0]].iter().flat_map(|pl| pl.iter().copied())),
                },
                Err(e) => failures.push(format!("{name} frame {i}: {e}")),
            }
        }
        if ours.len() != want.len() {
            failures.push(format!("{name}: {} bytes, ffmpeg {}", ours.len(), want.len()));
        } else if let Some(i) = ours.iter().zip(&want).position(|(a, b)| a != b) {
            failures.push(format!("{name}: first difference at byte {i} of {}", want.len()));
        } else {
            eprintln!("{name}: bit-exact ({} frames)", s.chunks.len());
            tested += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("{tested} fate samples bit-exact");
}
