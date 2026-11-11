//! LCEVC-1: an LCEVC enhancement layer carried as T.35 SEI in H.264 / HEVC is found at import
//! and named in the codec label; the base layer still plays. ffmpeg encodes the base stream
//! (libx264 / libx265); this test inserts an LCEVC-shaped T.35 SEI NAL unit before every
//! picture (UK country code 0xB4, register bytes 00 50 00, as V-Nova's carriage documentation
//! describes) and ffmpeg remuxes the result, unchanged, into MP4, Matroska and MPEG-TS.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_media::FrameRequest;
use cfa4_time::Tick;
use common::*;

const SUFFIX: &str = cfa4_codecs::lcevc::LABEL_SUFFIX;

/// The NAL units of an Annex B stream (without start codes).
fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &s)| {
            let mut end = starts.get(k + 1).map_or(data.len(), |&n| n - 3);
            while end > s && data[end - 1] == 0 {
                end -= 1;
            }
            &data[s..end]
        })
        .collect()
}

/// Copy of `annexb` with an LCEVC T.35 SEI NAL unit before every picture's first slice.
fn with_lcevc_sei(annexb: &[u8], hevc: bool) -> Vec<u8> {
    let (sei_header, is_vcl): (&[u8], fn(&[u8]) -> bool) = if hevc {
        (&[39 << 1, 0x01], |n| (n[0] >> 1) & 0x3f < 32 && n.get(2).is_some_and(|b| b & 0x80 != 0))
    } else {
        (&[0x06], |n| matches!(n[0] & 0x1f, 1 | 5))
    };
    // payload type 4, size, B4 00 50 00 and a few bytes standing in for the LCEVC access unit
    let body = [0xB4, 0x00, 0x50, 0x00, 0x79, 0xFF, 0x42, 0x17, 0x80];
    let mut sei = sei_header.to_vec();
    sei.extend([4, body.len() as u8]);
    sei.extend(body);
    sei.push(0x80);
    let mut out = Vec::new();
    for n in nal_units(annexb) {
        if is_vcl(n) {
            out.extend([0, 0, 0, 1]);
            out.extend(&sei);
        }
        out.extend([0, 0, 0, 1]);
        out.extend(n);
    }
    out
}

fn run(ff: &Path, args: &[&str]) {
    let st = Command::new(ff).args(["-y", "-v", "error"]).args(args).stdin(Stdio::null()).status().expect("run ffmpeg");
    assert!(st.success(), "ffmpeg {args:?}");
}

/// Base stream (`enc`) and, with LCEVC SEI inserted, remuxed into each container.
fn make(ff: &Path, tag: &str, enc: &[&str], hevc: bool) -> (PathBuf, Vec<PathBuf>) {
    let d = dir().join("lcevc");
    std::fs::create_dir_all(&d).unwrap();
    let raw = d.join(format!("{tag}.annexb"));
    let fmt = if hevc { "hevc" } else { "h264" };
    let mut args = vec!["-f", "lavfi", "-i", "testsrc2=size=128x96:rate=25", "-frames:v", "6"];
    args.extend_from_slice(enc);
    args.extend_from_slice(&["-f", fmt]);
    args.push(raw.to_str().unwrap());
    run(ff, &args);
    let base = std::fs::read(&raw).unwrap();
    let tagged = d.join(format!("{tag}_lcevc.annexb"));
    std::fs::write(&tagged, with_lcevc_sei(&base, hevc)).unwrap();
    let plain = d.join(format!("{tag}_plain.mp4"));
    run(ff, &["-f", fmt, "-r", "25", "-i", raw.to_str().unwrap(), "-c", "copy", plain.to_str().unwrap()]);
    let outs = ["mp4", "mkv", "ts"]
        .iter()
        .map(|ext| {
            let out = d.join(format!("{tag}_lcevc.{ext}"));
            run(ff, &["-f", fmt, "-r", "25", "-i", tagged.to_str().unwrap(), "-c", "copy", out.to_str().unwrap()]);
            out
        })
        .collect();
    (plain, outs)
}

fn check(plain: &Path, tagged: &[PathBuf], codec: &str) {
    let src = cfa4_codecs::open_bytes("plain.mp4", bytes(plain)).unwrap();
    let label = src.info().video.as_ref().unwrap().codec.clone();
    assert!(!label.ends_with(SUFFIX), "{}: {label}", plain.display());
    for f in tagged {
        let name = f.file_name().unwrap().to_str().unwrap();
        let src = cfa4_codecs::open_bytes(name, bytes(f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert!(v.codec.starts_with(codec) && v.codec.ends_with(SUFFIX), "{name}: {}", v.codec);
        // the base layer plays
        let fr = src.video_frame(FrameRequest::full(Tick::from_rational(1, 1, 10))).unwrap();
        assert_eq!((fr.width, fr.height), (128, 96), "{name}");
    }
}

#[test]
fn lcevc_sei_in_h264_is_reported_and_the_base_plays() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (plain, tagged) = make(&ff, "h264", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-bf", "0", "-x264-params", "slices=1"], false);
    check(&plain, &tagged, "H.264");
}

#[test]
fn lcevc_sei_in_hevc_is_reported_and_the_base_plays() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (plain, tagged) = make(&ff, "hevc", &["-c:v", "libx265", "-pix_fmt", "yuv420p", "-x265-params", "log-level=error:bframes=0"], true);
    check(&plain, &tagged, "HEVC");
}
