//! LCEVC carried as its own stream next to the base video (CFA4-FFMPEG-9.1-001 F2c): an ISOBMFF
//! `lvc1` track referring to the base track with `tref/sbas` (ISO/IEC 14496-15), and an MPEG-TS
//! elementary stream of `stream_type` 0x36 (ISO/IEC 13818-1). FFmpeg cannot encode LCEVC, so the
//! first dual-track MP4 is written here: the base by our own H.264 encoder (`cfa4-h264enc`, so
//! the test needs no libx264), the enhancement as LCEVC access units like those `lcevc_detect.rs`
//! puts in SEI (2× upscale, no enhancement). FFmpeg (the pinned minimum, `CFA4_FFMPEG_MIN_VERSION`
//! in `packaging/ffmpeg-version.env`, with its `lcevc_merge` bitstream filter) remuxes it into
//! an MP4 of its own and into MPEG-TS, the TS writer being new in that release. The test checks
//! that FFmpeg pairs the two streams in all three files and carries the LCEVC bytes unchanged,
//! and that CFA4 finds the enhancement and plays the base picture bit-exact. Decoding this
//! carriage is a codec-backlog item (`docs/codec-parity.md`); its check is written and ignored.

mod common;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_isobmff::{AvcConfig, Brand, CodecConfig, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_media::FrameRequest;
use cfa4_time::Tick;
use common::*;

const CONTEXT: &str = "LCEVC dual-track oracle";
const SUFFIX: &str = cfa4_codecs::lcevc::LABEL_SUFFIX;
const DECODED: &str = cfa4_codecs::lcevc::DECODED_SUFFIX;
const W: u32 = 128;
const H: u32 = 96;
const FPS: u32 = 25;
const FRAMES: u32 = 6;

/// `CFA4_FFMPEG_MIN_VERSION` from `packaging/ffmpeg-version.env`, the single version source.
fn min_ffmpeg_version() -> String {
    let env = std::fs::read_to_string(cfa4_testkit::workspace_root().join("packaging/ffmpeg-version.env")).expect("read packaging/ffmpeg-version.env");
    env.lines()
        .find_map(|l| l.trim().strip_prefix("CFA4_FFMPEG_MIN_VERSION="))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("CFA4_FFMPEG_MIN_VERSION in packaging/ffmpeg-version.env")
}

/// ffmpeg and ffprobe at the pinned minimum or newer, with `lcevc_merge`, else `None` (skip).
fn tools() -> Option<(PathBuf, PathBuf)> {
    let ff = cfa4_testkit::ffmpeg_or_skip(CONTEXT)?;
    let probe = cfa4_testkit::ffprobe_or_skip(CONTEXT)?;
    let min = min_ffmpeg_version();
    let mut parts = min.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    if !cfa4_testkit::oracle::ffmpeg_at_least(&ff, major, minor) {
        let v = cfa4_testkit::oracle::ffmpeg_version(&ff).unwrap_or_default();
        cfa4_testkit::skip(CONTEXT, &format!("ffmpeg {v} predates the LCEVC MPEG-TS writer (needs {min}+)"));
        return None;
    }
    let bsfs = Command::new(&ff).args(["-hide_banner", "-bsfs"]).stdin(Stdio::null()).output().expect("run ffmpeg -bsfs");
    if !String::from_utf8_lossy(&bsfs.stdout).lines().any(|l| l.trim() == "lcevc_merge") {
        cfa4_testkit::skip(CONTEXT, "ffmpeg has no lcevc_merge bitstream filter");
        return None;
    }
    Some((ff, probe))
}

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

/// One LCEVC NAL unit over the 128×96 4:2:0 8-bit base: 2D scaling to 256×192 and no
/// enhancement (the layout of `lcevc_detect.rs`, adjusted). Three choices keep the pinned FFmpeg's
/// LCEVC readers on it: a sequence configuration in the IDR (its `lvcC` writer needs one); nearest
/// upsampling, because its parser and `lvcC` writer skip the adaptive-cubic coefficients for
/// every non-zero `upsample_type` (the specification and its own CBS reader: only for 4) and then
/// read a wrong custom size; and no encoded-data block, which its CBS reader rejects when no
/// surface data follows the flags.
fn lcevc_nal(idr: bool) -> Vec<u8> {
    let mut au = Vec::new();
    if idr {
        // sequence configuration: main profile, level 1, sublevel 1, no conformance window
        block(&mut au, 0, &[0x01, 0x40]);
        // global configuration: one enhanced plane, DD, 4:2:0, 8 bits, nearest upsampling, 2D
        // scaling at the full-resolution level, custom size 256×192
        block(&mut au, 1, &[0x7E, 0x40, 0x00, 0x80, 0x01, 0x00, 0x00, 0xC0]);
    }
    // picture configuration: no enhancement (the base is only upscaled), no temporal signalling
    block(&mut au, 2, &[0x80]);
    let mut nal = vec![0x41 | ((if idr { 29 } else { 28 }) << 1), 0xFF];
    nal.extend(escape(&au));
    nal.push(0x80);
    nal
}

/// `lvcC` (LCEVCDecoderConfigurationRecord): version 1, main profile, level 1, 4:2:0 8-bit,
/// 4-byte NAL lengths, the enhanced size, and the IDR NAL unit with the global configuration.
fn lvcc() -> Vec<u8> {
    let idr = lcevc_nal(true);
    let mut r = vec![1, 0, 1, 1 << 6, 0xFF];
    r.extend((2 * W).to_be_bytes());
    r.extend((2 * H).to_be_bytes());
    r.extend([0xFF, 1, 29]);
    r.extend(1u16.to_be_bytes());
    r.extend((idr.len() as u16).to_be_bytes());
    r.extend(idr);
    r
}

/// The `lvc1` sample entry body (a VisualSampleEntry of the enhanced size holding `lvcC`).
fn lvc1_entry() -> SampleEntry {
    let mut b = vec![0; 6];
    b.extend(1u16.to_be_bytes()); // data_reference_index
    b.extend([0; 16]);
    b.extend(((2 * W) as u16).to_be_bytes());
    b.extend(((2 * H) as u16).to_be_bytes());
    b.extend(0x0048_0000u32.to_be_bytes());
    b.extend(0x0048_0000u32.to_be_bytes());
    b.extend([0; 4]);
    b.extend(1u16.to_be_bytes()); // frame_count
    b.extend([0; 32]); // compressorname
    b.extend(0x0018u16.to_be_bytes());
    b.extend((-1i16).to_be_bytes());
    let cfg = lvcc();
    b.extend((8 + cfg.len() as u32).to_be_bytes());
    b.extend(b"lvcC");
    b.extend(cfg);
    let lvc1 = FourCc(cfa4_lcevc::SAMPLE_ENTRY);
    SampleEntry::video(lvc1, CodecConfig::Unknown { fourcc: lvc1, raw: b }, (2 * W) as u16, (2 * H) as u16)
}

/// A moving gradient with a block pattern, so the base pictures differ.
fn picture(i: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (W as usize, H as usize);
    let y = (0..w * h).map(|k| ((k % w) * 2 + (k / w) + i as usize * 7 + if (k % w / 8 + k / w / 8) % 2 == 0 { 40 } else { 0 }) as u8).collect();
    let u = (0..w * h / 4).map(|k| (100 + (k % (w / 2)) + i as usize) as u8).collect();
    let v = (0..w * h / 4).map(|k| (150 - (k / (w / 2)) as i32 + i as i32) as u8).collect();
    (y, u, v)
}

/// The base pictures from `cfa4-h264enc` (length-prefixed samples) and its `avcC`.
fn base() -> (Vec<cfa4_h264enc::Packet>, AvcConfig) {
    let mut cfg = cfa4_h264enc::EncoderConfig::new(W, H, FPS, 1);
    cfg.format = cfa4_h264enc::PacketFormat::LengthPrefixed;
    cfg.aud = false;
    cfg.bframes = 0;
    let mut enc = cfa4_h264enc::Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..FRAMES {
        let (y, u, v) = picture(i);
        let f = cfa4_h264enc::YuvFrame { y: &y, u: &u, v: &v, y_stride: W as usize, uv_stride: W as usize / 2 };
        packets.extend(enc.try_encode(&f, i64::from(i)).unwrap());
    }
    packets.extend(enc.flush());
    assert_eq!(packets.len(), FRAMES as usize);
    (packets, AvcConfig::parse(&enc.avcc()).unwrap())
}

/// Length-prefix one NAL unit (4 bytes).
fn prefixed(nal: &[u8]) -> Vec<u8> {
    let mut s = (nal.len() as u32).to_be_bytes().to_vec();
    s.extend_from_slice(nal);
    s
}

/// Insert `tref/sbas` → track 1 into the second `trak` of a file whose `moov` is at the end.
fn with_sbas(mut file: Vec<u8>) -> Vec<u8> {
    let be32 = |d: &[u8], at: usize| u32::from_be_bytes(d[at..at + 4].try_into().unwrap()) as usize;
    let mut moov = 0;
    while &file[moov + 4..moov + 8] != b"moov" {
        moov += be32(&file, moov);
    }
    let (mut at, mut traks) = (moov + 8, 0);
    let trak = loop {
        if &file[at + 4..at + 8] == b"trak" {
            traks += 1;
            if traks == 2 {
                break at;
            }
        }
        at += be32(&file, at);
    };
    let tkhd_end = trak + 8 + be32(&file, trak + 8);
    assert_eq!(&file[trak + 12..trak + 16], b"tkhd");
    let mut tref = vec![0, 0, 0, 20];
    tref.extend(b"tref");
    tref.extend([0, 0, 0, 12]);
    tref.extend(b"sbas");
    tref.extend(1u32.to_be_bytes());
    file.splice(tkhd_end..tkhd_end, tref.iter().copied());
    for b in [moov, trak] {
        let n = (be32(&file, b) + tref.len()) as u32;
        file[b..b + 4].copy_from_slice(&n.to_be_bytes());
    }
    file
}

/// MP4 with the base track only, or with the `lvc1` track (track 2, `tref/sbas` → track 1).
fn mp4(packets: &[cfa4_h264enc::Packet], avcc: &AvcConfig, lcevc: bool) -> Vec<u8> {
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let vt = mux.add_track(TrackConfig::new(SampleEntry::avc(avcc.clone(), W as u16, H as u16), FPS)).unwrap();
    let lt = lcevc.then(|| mux.add_track(TrackConfig::new(lvc1_entry(), FPS)).unwrap());
    for (k, p) in packets.iter().enumerate() {
        mux.write_sample(vt, WriteSample { data: &p.data, duration: 1, composition_offset: 0, is_sync: p.keyframe }).unwrap();
        if let Some(lt) = lt {
            let s = prefixed(&lcevc_nal(k == 0));
            mux.write_sample(lt, WriteSample { data: &s, duration: 1, composition_offset: 0, is_sync: k == 0 }).unwrap();
        }
    }
    let file = mux.finish().unwrap().into_inner();
    if lcevc { with_sbas(file) } else { file }
}

fn run(ff: &Path, args: &[&str]) {
    let out = Command::new(ff).args(["-hide_banner", "-y", "-v", "error", "-xerror"]).args(args).stdin(Stdio::null()).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn probe(probe: &Path, file: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(probe).args(["-v", "error", "-of", "json"]).args(args).arg(file).stdin(Stdio::null()).output().expect("run ffprobe");
    assert!(out.status.success(), "ffprobe {}: {}", file.display(), String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("ffprobe JSON")
}

/// The bytes of an `ffprobe -show_data` hex dump (`00000000: 0000 0001 7bff ...  ....`).
fn hexdump_bytes(dump: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in dump.lines() {
        let Some((_, rest)) = line.split_once(": ") else { continue };
        let hex = rest.get(..40).unwrap_or(rest);
        for group in hex.split_whitespace() {
            for k in (0..group.len()).step_by(2) {
                out.push(u8::from_str_radix(&group[k..k + 2], 16).expect("hex dump"));
            }
        }
    }
    out
}

/// The NAL units of one packet: length-prefixed (4 bytes) or Annex B.
fn nal_units(data: &[u8]) -> Vec<Vec<u8>> {
    if data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]) {
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
        return starts
            .iter()
            .enumerate()
            .map(|(k, &s)| {
                let mut end = starts.get(k + 1).map_or(data.len(), |&n| n - 3);
                while end > s && data[end - 1] == 0 {
                    end -= 1;
                }
                data[s..end].to_vec()
            })
            .collect();
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= data.len() {
        let n = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        out.push(data[at + 4..at + 4 + n].to_vec());
        at += 4 + n;
    }
    out
}

/// FFmpeg pairs the base and the enhancement in one LCEVC stream group of the enhanced size,
/// and its LCEVC packets carry exactly the NAL units we wrote, in order.
fn check_ffmpeg_view(ffprobe: &Path, file: &Path) {
    let name = file.display();
    let info = probe(ffprobe, file, &["-show_streams", "-show_stream_groups"]);
    let streams = info["streams"].as_array().expect("streams");
    let lcevc: Vec<_> = streams.iter().filter(|s| s["codec_name"] == "lcevc").collect();
    assert_eq!(lcevc.len(), 1, "{name}: one LCEVC stream, got {streams:?}");
    let groups = info["stream_groups"].as_array().cloned().unwrap_or_default();
    let group = groups
        .iter()
        .find(|g| g["type"].as_str().is_some_and(|t| t.to_ascii_lowercase().contains("lcevc")))
        .unwrap_or_else(|| panic!("{name}: no LCEVC stream group in {groups:?}"));
    assert_eq!(group["streams"].as_array().map_or(0, Vec::len), 2, "{name}: group {group}");
    let c = &group["components"][0];
    assert_eq!(
        (c["el_index"].as_u64(), c["width"].as_u64(), c["height"].as_u64()),
        (Some(1), Some(u64::from(2 * W)), Some(u64::from(2 * H))),
        "{name}: group {group}"
    );

    let idx = lcevc[0]["index"].as_u64().unwrap();
    let pk = probe(ffprobe, file, &["-select_streams", &idx.to_string(), "-show_packets", "-show_data"]);
    let got: Vec<Vec<u8>> =
        pk["packets"].as_array().expect("packets").iter().flat_map(|p| nal_units(&hexdump_bytes(p["data"].as_str().unwrap_or("")))).collect();
    let want: Vec<Vec<u8>> = (0..FRAMES).map(|k| lcevc_nal(k == 0)).collect();
    assert_eq!(got, want, "{name}: LCEVC NAL units");
}

fn luma(f: &cfa4_frame::VideoFrame) -> Vec<u8> {
    match &f.data {
        cfa4_frame::PixelData::Yuv8 { planes, .. } => planes[0].to_vec(),
        _ => panic!("not 8-bit Y'CbCr"),
    }
}

fn at(i: u32) -> FrameRequest {
    FrameRequest::full(Tick::from_rational(i64::from(i), 1, i64::from(FPS)))
}

/// The three dual-track files (ours, FFmpeg's MP4, FFmpeg's TS) and the base-only MP4.
fn files(ff: &Path) -> (PathBuf, Vec<PathBuf>) {
    let d = dir().join("lcevc_dual");
    std::fs::create_dir_all(&d).unwrap();
    let (packets, avcc) = base();
    let plain = d.join("base_only.mp4");
    std::fs::write(&plain, mp4(&packets, &avcc, false)).unwrap();
    let ours = d.join("cfa4_lvc1.mp4");
    std::fs::write(&ours, mp4(&packets, &avcc, true)).unwrap();
    let mut out = vec![ours.clone()];
    for ext in ["mp4", "ts"] {
        let f = d.join(format!("ffmpeg_lvc1.{ext}"));
        let (src, dst) = (ours.to_str().unwrap(), f.to_str().unwrap());
        // `-stream_group map=0=0` copies the group's options but not its streams, so name both
        let group = format!("type=lcevc:st=0:st=1:el_index=1:video_size={}x{}", 2 * W, 2 * H);
        run(ff, &["-i", src, "-map", "0:0", "-map", "0:1", "-stream_group", &group, "-c", "copy", dst]);
        out.push(f);
    }
    (plain, out)
}

#[test]
fn ffmpeg_pairs_the_dual_track_lcevc_and_cfa4_plays_the_base() {
    let Some((ff, ffprobe)) = tools() else { return };
    let (plain, dual) = files(&ff);
    let src = cfa4_codecs::open_bytes("base_only.mp4", bytes(&plain)).unwrap();
    let label = src.info().video.as_ref().unwrap().codec.clone();
    assert!(!label.contains("LCEVC"), "base only: {label}");
    let base: Vec<Vec<u8>> = (0..FRAMES).map(|i| luma(&src.video_frame(at(i)).unwrap())).collect();

    for f in &dual {
        check_ffmpeg_view(&ffprobe, f);
        let name = f.file_name().unwrap().to_str().unwrap();
        let src = cfa4_codecs::open_bytes(name, bytes(f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert!(v.codec.ends_with(SUFFIX), "{name}: the enhancement stream is found and named: {}", v.codec);
        assert_eq!((v.width, v.height), (W, H), "{name}: the base plays");
        for (i, want) in base.iter().enumerate() {
            let fr = src.video_frame(at(i as u32)).unwrap();
            assert_eq!((fr.width, fr.height), (W, H), "{name} frame {i}");
            assert!(luma(&fr) == *want, "{name} frame {i}: base picture differs from the base-only file");
        }
    }
}

/// Copy of the base samples with the LCEVC NAL unit in a T.35 SEI before each picture: the
/// carriage CFA4 decodes today (bit-exact against LCEVCdec in `cfa4-lcevc`'s oracle).
fn with_sei(packets: &[cfa4_h264enc::Packet]) -> Vec<cfa4_h264enc::Packet> {
    packets
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let mut body = vec![0xB4, 0x00, 0x50, 0x00];
            body.extend(lcevc_nal(k == 0));
            let mut rbsp = vec![4, body.len() as u8];
            rbsp.extend(body);
            rbsp.push(0x80);
            let mut sei = vec![0x06];
            sei.extend(escape(&rbsp));
            let mut q = p.clone();
            q.data = prefixed(&sei);
            q.data.extend_from_slice(&p.data);
            q
        })
        .collect()
}

#[test]
#[ignore = "CFA4 does not decode the lvc1 / TS 0x36 LCEVC carriage yet (codec backlog, docs/codec-parity.md)"]
fn dual_track_lcevc_decodes_like_the_sei_carriage() {
    let Some((ff, _)) = tools() else { return };
    let (_, dual) = files(&ff);
    let (packets, avcc) = base();
    let sei = cfa4_codecs::open_bytes("sei.mp4", mp4(&with_sei(&packets), &avcc, false).into()).unwrap();
    let label = sei.info().video.as_ref().unwrap().codec.clone();
    assert!(label.ends_with(DECODED), "SEI carriage: {label}");
    let want: Vec<Vec<u8>> = (0..FRAMES).map(|i| luma(&sei.video_frame(at(i)).unwrap())).collect();
    for f in &dual {
        let name = f.file_name().unwrap().to_str().unwrap();
        let src = cfa4_codecs::open_bytes(name, bytes(f)).unwrap();
        let v = src.info().video.clone().unwrap();
        assert!(v.codec.ends_with(DECODED), "{name}: {}", v.codec);
        assert_eq!((v.width, v.height), (2 * W, 2 * H), "{name}");
        for (i, w) in want.iter().enumerate() {
            assert!(luma(&src.video_frame(at(i as u32)).unwrap()) == *w, "{name} frame {i}");
        }
    }
}
