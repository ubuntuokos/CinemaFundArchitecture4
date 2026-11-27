//! MPEG-4 Simple Studio Profile against FFmpeg's `mpeg4` decoder (the external oracle): the FATE
//! sample `mpeg4/mpeg4_sstp_dpcm.m4v` when present (`target/fixtures/sstp`, local only).

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_sstp::{Chroma, Decoder};

mod synth;

use synth::{BASE, Cfg, Rng, picture};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/sstp")
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// ffmpeg's frames as raw little-endian 16-bit planar samples.
fn ffmpeg_raw(path: &Path, pix_fmt: &str) -> Vec<u16> {
    let o = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args(["-f", "rawvideo", "-pix_fmt", pix_fmt, "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect()
}

/// Access units of a raw stream: split before every VOP start code's headers (VOS, VO, VOL,
/// GOP), as ffmpeg's parser does.
pub fn access_units(b: &[u8]) -> Vec<&[u8]> {
    let mut starts = vec![0u64];
    let mut split = cfa4_sstp::AccessUnits::new();
    // in odd-sized chunks, so start codes straddle them
    for (k, chunk) in b.chunks(1001).enumerate() {
        split.feed(k as u64 * 1001, chunk, &mut starts);
    }
    starts.push(b.len() as u64);
    starts.windows(2).map(|w| &b[w[0] as usize..w[1] as usize]).filter(|s| !s.is_empty()).collect()
}

#[test]
fn fate_dpcm_sample_matches_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let path = fixtures().join("mpeg4_sstp_dpcm.m4v");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipped: no FATE sample");
        return;
    };
    let info = cfa4_sstp::probe(&bytes).expect("a studio stream");
    assert_eq!(info.chroma, Chroma::Yuv422);
    let mut d = Decoder::new();
    let mut ours = Vec::new();
    for au in access_units(&bytes) {
        let p = d.decode(au).unwrap();
        assert_eq!((p.chroma, p.width, p.height), (info.chroma, info.width, info.height));
        for pl in &p.planes {
            ours.extend_from_slice(pl);
        }
    }
    let want = ffmpeg_raw(&path, "yuv422p10le");
    assert_eq!(ours.len(), want.len());
    let diff = ours.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(diff, 0, "{diff} samples differ");
}

#[test]
fn synthetic_pictures_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let dir = fixtures();
    std::fs::create_dir_all(&dir).unwrap();
    let base = BASE;
    let cases = [
        ("422", base),
        ("422_alt", Cfg { alternate_scan: true, dct_precision: 1, intra_dc_precision: 2, q_scale_type: true, slice_every: 2, ..base }),
        ("444", Cfg { chroma_format: 3, width: 33, height: 17, dct_precision: 3, intra_dc_precision: 3, matrices: true, ..base }),
        ("rgb", Cfg { rgb: true, chroma_format: 3, width: 48, height: 16, dct_precision: 2, intra_dc_precision: 1, mpeg_quant: true, slice_every: 1, ..base }),
    ];
    for (n, (name, c)) in cases.iter().enumerate() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((n as u64 + 1) * 0x1234_5678_9ABC));
        let mut stream = Vec::new();
        let mut ours = Vec::new();
        for _ in 0..3 {
            let au = picture(c, &mut rng);
            let p = Decoder::new().decode(&au).unwrap();
            for pl in &p.planes {
                ours.extend_from_slice(pl);
            }
            stream.extend(au);
        }
        let path = dir.join(format!("random_{name}.m4v"));
        std::fs::write(&path, &stream).unwrap();
        let fmt = match (c.rgb, c.chroma_format) {
            (true, _) => "gbrp10le",
            (false, 2) => "yuv422p10le",
            _ => "yuv444p10le",
        };
        let want = ffmpeg_raw(&path, fmt);
        assert_eq!(ours.len(), want.len(), "{name}: sample count");
        let diff = ours.iter().zip(&want).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "{name}: {diff} of {} samples differ", ours.len());
        eprintln!("{name}: {} samples bit for bit", ours.len());
    }
}
