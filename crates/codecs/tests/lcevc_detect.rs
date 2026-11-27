//! An LCEVC enhancement layer carried as T.35 SEI in H.264 / HEVC is found at import and decoded
//! (CB-17.2), or, without a configuration, named in the codec label while the base plays.
//! ffmpeg encodes the base stream (libx264 / libx265); this test inserts a T.35 SEI NAL unit
//! before every picture (UK country code 0xB4, register bytes 00 50 00, as V-Nova's carriage
//! documentation describes) holding an LCEVC access unit that doubles the picture in both
//! directions (no residuals: the bit-exact residual decoding is `cfa4-lcevc`'s oracle test), and
//! ffmpeg remuxes the result, unchanged, into MP4, Matroska and MPEG-TS.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_media::FrameRequest;
use cfa4_time::Tick;
use common::*;

const SUFFIX: &str = cfa4_codecs::lcevc::LABEL_SUFFIX;
const DECODED: &str = cfa4_codecs::lcevc::DECODED_SUFFIX;
const ALL: &[&str] = &["mp4", "mkv", "ts"];

/// Emulation prevention: `00 00` followed by a byte up to 3 gains a `03`.
fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    let mut zeros = 0;
    for &b in data {
        if zeros == 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// An LCEVC block: type, size code and payload.
fn block(out: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    assert!(payload.len() < 128);
    if payload.len() <= 5 {
        out.push(kind | ((payload.len() as u8) << 5));
    } else {
        out.extend([kind | (7 << 5), payload.len() as u8]);
    }
    out.extend_from_slice(payload);
}

/// An LCEVC access unit (NAL unit without start code) over a 128×96 4:2:0 8-bit base: 2D
/// scaling at the full-resolution level, no scaling below it, no residuals.
fn lcevc_au(idr: bool) -> Vec<u8> {
    let mut au = Vec::new();
    if idr {
        // global configuration: one enhanced plane, DD, 4:2:0, 8 bits, cubic kernel, 256×192
        block(&mut au, 1, &[0x7E, 0x40, 0x08, 0x80, 0x01, 0x00, 0x00, 0xC0]);
    }
    // picture configuration: default quantisation matrix, step width 1000
    block(&mut au, 2, &[0x10, 0x07, 0xD0]);
    // encoded data: every chunk of both levels off
    block(&mut au, 3, &[0x00, 0x00]);
    let mut nal = vec![0x41 | ((if idr { 29 } else { 28 }) << 1), 0xFF];
    nal.extend(escape(&au));
    nal.push(0x80);
    nal
}

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

/// Copy of `annexb` with an LCEVC T.35 SEI NAL unit before every picture's first slice: a real
/// access unit (an IDR one first), or a few bytes that are none.
fn with_lcevc_sei(annexb: &[u8], hevc: bool, real: bool) -> Vec<u8> {
    let (sei_header, is_vcl): (&[u8], fn(&[u8]) -> bool) = if hevc {
        (&[39 << 1, 0x01], |n| (n[0] >> 1) & 0x3f < 32 && n.get(2).is_some_and(|b| b & 0x80 != 0))
    } else {
        (&[0x06], |n| matches!(n[0] & 0x1f, 1 | 5))
    };
    let sei = |idr: bool| {
        // payload type 4, size, B4 00 50 00 and the LCEVC access unit (or bytes standing in)
        let mut body = vec![0xB4, 0x00, 0x50, 0x00];
        body.extend(if real { lcevc_au(idr) } else { vec![0x79, 0xFF, 0x42, 0x17, 0x80] });
        let mut rbsp = vec![4, body.len() as u8];
        rbsp.extend(body);
        rbsp.push(0x80);
        let mut sei = sei_header.to_vec();
        sei.extend(escape(&rbsp));
        sei
    };
    let mut out = Vec::new();
    let mut first = true;
    for n in nal_units(annexb) {
        if is_vcl(n) {
            out.extend([0, 0, 0, 1]);
            out.extend(sei(first));
            first = false;
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
fn make(ff: &Path, tag: &str, enc: &[&str], hevc: bool, real: bool, exts: &[&str]) -> (PathBuf, Vec<PathBuf>) {
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
    std::fs::write(&tagged, with_lcevc_sei(&base, hevc, real)).unwrap();
    let plain = d.join(format!("{tag}_plain.mp4"));
    run(ff, &["-f", fmt, "-r", "25", "-i", raw.to_str().unwrap(), "-c", "copy", plain.to_str().unwrap()]);
    let outs = exts
        .iter()
        .map(|ext| {
            let out = d.join(format!("{tag}_lcevc.{ext}"));
            run(ff, &["-fflags", "+genpts", "-f", fmt, "-r", "25", "-i", tagged.to_str().unwrap(), "-c", "copy", out.to_str().unwrap()]);
            out
        })
        .collect();
    (plain, outs)
}

fn luma(f: &cfa4_frame::VideoFrame) -> Vec<u8> {
    match &f.data {
        cfa4_frame::PixelData::Yuv8 { planes, .. } => planes[0].to_vec(),
        _ => panic!("not 8-bit Y'CbCr"),
    }
}

fn check(plain: &Path, tagged: &[PathBuf], codec: &str, real: bool) {
    let at = FrameRequest::full(Tick::from_rational(1, 1, 10));
    let src = cfa4_codecs::open_bytes("plain.mp4", bytes(plain)).unwrap();
    let label = src.info().video.as_ref().unwrap().codec.clone();
    assert!(!label.contains(DECODED), "{}: {label}", plain.display());
    let base = src.video_frame(at).unwrap();
    for f in tagged {
        let name = f.file_name().unwrap().to_str().unwrap();
        let src = cfa4_codecs::open_bytes(name, bytes(f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        let fr = src.video_frame(at).unwrap();
        if real {
            // decoded: twice the size, and the base picture upscaled
            assert!(v.codec.starts_with(codec) && v.codec.ends_with(DECODED), "{name}: {}", v.codec);
            assert_eq!((v.width, v.height), (256, 192), "{name}");
            assert_eq!((fr.width, fr.height), (256, 192), "{name}");
            let (b, e) = (luma(&base), luma(&fr));
            let diff: u64 = (0..96)
                .flat_map(|y| (0..128).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let up = (0..4).map(|k| u64::from(e[(2 * y + k / 2) * 256 + 2 * x + k % 2])).sum::<u64>() / 4;
                    up.abs_diff(u64::from(b[y * 128 + x]))
                })
                .sum();
            assert!(diff / (128 * 96) <= 2, "{name}: mean difference {}", diff / (128 * 96));
        } else {
            // the base layer plays
            assert!(v.codec.starts_with(codec) && v.codec.ends_with(SUFFIX), "{name}: {}", v.codec);
            assert_eq!((fr.width, fr.height), (128, 96), "{name}");
        }
    }
}

#[test]
fn lcevc_sei_in_h264_is_reported_and_the_base_plays() {
    let ff = cfa4_testkit::require_ffmpeg!();
    if !cfa4_testkit::encoders_or_skip("lcevc h264", &ff, &["libx264"]) {
        return;
    }
    let enc = ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-bf", "0", "-x264-params", "slices=1"];
    let (plain, tagged) = make(&ff, "h264_fake", &enc, false, false, ALL);
    check(&plain, &tagged, "H.264", false);
}

#[test]
fn lcevc_sei_in_h264_is_decoded() {
    let ff = cfa4_testkit::require_ffmpeg!();
    if !cfa4_testkit::encoders_or_skip("lcevc h264 decode", &ff, &["libx264"]) {
        return;
    }
    let enc = ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-bf", "0", "-x264-params", "slices=1"];
    let (plain, tagged) = make(&ff, "h264", &enc, false, true, ALL);
    check(&plain, &tagged, "H.264", true);
    // B-frames: the enhancement follows the pictures into presentation order (ffmpeg cannot put into Matroska
    // or MPEG-TS a raw stream with B-frames: no timestamps)
    let enc = ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-bf", "2", "-x264-params", "slices=1"];
    let (plain, tagged) = make(&ff, "h264_bf", &enc, false, true, &["mp4"]);
    check(&plain, &tagged, "H.264", true);
}

#[test]
fn lcevc_sei_in_hevc_is_decoded() {
    let ff = cfa4_testkit::require_ffmpeg!();
    if !cfa4_testkit::encoders_or_skip("lcevc hevc decode", &ff, &["libx265"]) {
        return;
    }
    let (plain, tagged) = make(&ff, "hevc", &["-c:v", "libx265", "-pix_fmt", "yuv420p", "-x265-params", "log-level=error:bframes=0"], true, true, ALL);
    check(&plain, &tagged, "HEVC", true);
}
