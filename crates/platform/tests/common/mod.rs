//! Shared helpers for the platform tests: ffmpeg-made fixtures (external generator only, never
//! linked), MP4 sample reading, and decoding a run of samples to compare decoders.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_codecs::{DecodedFrame, VideoDecoder};
use cfa4_frame::PixelData;
use cfa4_isobmff::SampleEntry;

/// The parity fixtures: (file, ffmpeg arguments). B-frames everywhere (reordering), a 360-line
/// picture (coded 368: cropping), two GOPs or more (seeks), and an open-GOP HEVC stream (CRA with
/// RASL pictures).
pub const FIXTURES: &[(&str, &[&str])] = &[
    (
        "h264_high.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=24:d=3,noise=alls=12:allf=t",
            "-c:v",
            "libx264",
            "-profile:v",
            "high",
            "-preset",
            "fast",
            "-x264-params",
            "bframes=3:b-pyramid=normal:keyint=24:min-keyint=24:scenecut=0",
            "-pix_fmt",
            "yuv420p",
        ],
    ),
    (
        "hevc_main.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=24:d=3,noise=alls=12:allf=t",
            "-c:v",
            "libx265",
            "-preset",
            "fast",
            "-tag:v",
            "hvc1",
            "-x265-params",
            "log-level=error:bframes=4:keyint=24:min-keyint=24:scenecut=0:open-gop=1",
            "-pix_fmt",
            "yuv420p",
        ],
    ),
    (
        "hevc_main10.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=24:d=3,noise=alls=12:allf=t",
            "-c:v",
            "libx265",
            "-preset",
            "fast",
            "-profile:v",
            "main10",
            "-tag:v",
            "hvc1",
            "-x265-params",
            "log-level=error:bframes=4:keyint=24:min-keyint=24:scenecut=0",
            "-pix_fmt",
            "yuv420p10le",
        ],
    ),
];

pub fn dir() -> PathBuf {
    cfa4_testkit::fixtures_dir("platform")
}

/// Generate `name` with `ffmpeg -y -v error <args> <out>` (cached).
pub fn fixture(ff: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
    let out = dir().join(name);
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff).args(["-y", "-v", "error"]).args(args).arg(tmp).stdin(Stdio::null()).status();
        match st {
            Ok(s) if s.success() => true,
            other => {
                eprintln!("fixture {name}: ffmpeg failed: {other:?}");
                false
            }
        }
    })
}

/// The fixture by name. A fixture whose encoder is patent-restricted (x264, x265: never in CFA4's
/// FFmpeg) is `None` with a `SKIPPED` line when this ffmpeg lacks it.
pub fn named(ff: &Path, name: &str) -> Option<PathBuf> {
    let (_, args) = FIXTURES.iter().find(|f| f.0 == name)?;
    if !cfa4_testkit::fixture_encoders_or_skip(name, ff, args) {
        return None;
    }
    fixture(ff, name, args)
}

/// Fixtures coded by CFA4's own encoders (`cfa4-h264enc`, the `cfa4-hevc` encoder), so the
/// fallback tests run with every ffmpeg, which supplies only the source pictures (`rawvideo`) and
/// no encoder: H.264 High with three B-frames and HEVC Main (I and P pictures), the
/// [`FIXTURES`] source, keyframes every second.
pub const OWN_FIXTURES: &[&str] = &["h264_cfa4.mp4", "hevc_cfa4.mp4"];

const OWN_SIZE: (usize, usize) = (640, 360);
const OWN_FPS: u32 = 24;

/// An [`OWN_FIXTURES`] fixture (cached). Panics when it cannot be made: ffmpeg (the caller has it)
/// is all it needs.
pub fn own(ff: &Path, name: &str) -> PathBuf {
    assert!(OWN_FIXTURES.contains(&name), "{name}: not an own-encoder fixture");
    let out = dir().join(name);
    cfa4_testkit::fixtures::generate(&out, |tmp| match own_mp4(ff, name) {
        Ok(bytes) => std::fs::write(tmp, bytes).is_ok(),
        Err(e) => {
            eprintln!("fixture {name}: {e}");
            false
        }
    })
    .unwrap_or_else(|| panic!("fixture {name} could not be made"))
}

/// The source pictures (planar 4:2:0, 8-bit), decoded by ffmpeg from the [`FIXTURES`] test source.
fn own_source(ff: &Path) -> Result<Vec<Vec<u8>>, String> {
    let (w, h) = OWN_SIZE;
    let out = Command::new(ff)
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=24:d=3,noise=alls=12:allf=t", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let size = w * h * 3 / 2;
    if !out.status.success() || out.stdout.is_empty() || out.stdout.len() % size != 0 {
        return Err(format!("ffmpeg rawvideo: {:?}, {} bytes", out.status, out.stdout.len()));
    }
    Ok(out.stdout.chunks(size).map(<[u8]>::to_vec).collect())
}

fn own_mp4(ff: &Path, name: &str) -> Result<Vec<u8>, String> {
    use cfa4_isobmff::{AvcConfig, Brand, HevcConfig, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
    let (w, h) = OWN_SIZE;
    let pictures = own_source(ff)?;
    let planes = |p: &[u8]| {
        let (y, uv) = p.split_at(w * h);
        let (u, v) = uv.split_at(w * h / 4);
        (y.to_vec(), u.to_vec(), v.to_vec())
    };
    // (data, pts, dts, keyframe) per coded picture, decoding order
    let (entry, packets): (SampleEntry, Vec<(Vec<u8>, i64, i64, bool)>) = if name.starts_with("h264") {
        use cfa4_h264enc::{Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame};
        let mut cfg = EncoderConfig::new(w as u32, h as u32, OWN_FPS, 1);
        cfg.profile = Profile::High;
        cfg.preset = Preset::Speed;
        cfg.rate = RateControl::Qp(26);
        cfg.bframes = 3;
        cfg.keyint = OWN_FPS;
        cfg.scenecut = false;
        cfg.threads = 1;
        cfg.format = PacketFormat::LengthPrefixed;
        let mut enc = Encoder::new(cfg).map_err(|e| e.to_string())?;
        let mut packets = Vec::new();
        for (i, p) in pictures.iter().enumerate() {
            let (y, u, v) = planes(p);
            let frame = YuvFrame { y: &y, u: &u, v: &v, y_stride: w, uv_stride: w / 2 };
            packets.extend(enc.encode(&frame, i as i64).map_err(|e| e.to_string())?);
        }
        packets.extend(enc.flush());
        let entry = SampleEntry::avc(AvcConfig::parse(&enc.avcc()).map_err(|e| e.to_string())?, w as u16, h as u16);
        (entry, packets.into_iter().map(|p| (p.data, p.pts, p.dts, p.keyframe)).collect())
    } else {
        use cfa4_hevc::encode::{Encoder, EncoderConfig, PacketFormat, Preset, RateControl, YuvFrame};
        let mut cfg = EncoderConfig::new(w as u32, h as u32, OWN_FPS, 1);
        cfg.preset = Preset::Speed;
        cfg.rate = RateControl::Qp(28);
        cfg.keyint = OWN_FPS;
        cfg.slices = 1;
        cfg.format = PacketFormat::LengthPrefixed;
        let mut enc = Encoder::new(cfg).map_err(|e| e.to_string())?;
        let mut packets = Vec::new();
        for (i, p) in pictures.iter().enumerate() {
            let (y, u, v) = planes(p);
            let frame = YuvFrame::U8 { y: &y, u: &u, v: &v, y_stride: w, uv_stride: w / 2 };
            packets.extend(enc.encode(&frame, i as i64).map_err(|e| e.to_string())?);
        }
        packets.extend(enc.flush().map_err(|e| e.to_string())?);
        let entry = SampleEntry::hevc(HevcConfig::parse(&enc.hvcc()).map_err(|e| e.to_string())?, w as u16, h as u16);
        (entry, packets.into_iter().map(|p| (p.data, p.pts, p.dts, p.keyframe)).collect())
    };
    if packets.len() != pictures.len() {
        return Err(format!("{} pictures coded as {} samples", pictures.len(), packets.len()));
    }
    let mut mux = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(|e| e.to_string())?;
    let t = mux.add_track(TrackConfig::new(entry, OWN_FPS)).map_err(|e| e.to_string())?;
    for (data, pts, dts, keyframe) in &packets {
        let sample = WriteSample { data, duration: 1, composition_offset: (pts - dts) as i32, is_sync: *keyframe };
        mux.write_sample(t, sample).map_err(|e| e.to_string())?;
    }
    Ok(mux.finish().map_err(|e| e.to_string())?.into_inner())
}

/// The fixtures the fallback tests run on: the [`OWN_FIXTURES`] always, the x264/x265 [`FIXTURES`]
/// when this ffmpeg has those encoders (`SKIPPED` lines otherwise).
pub fn fallback_fixtures(ff: &Path) -> Vec<(String, PathBuf)> {
    let own = OWN_FIXTURES.iter().map(|name| (name.to_string(), own(ff, name)));
    let restricted = FIXTURES.iter().filter_map(|(name, _)| named(ff, name).map(|p| (name.to_string(), p)));
    own.chain(restricted).collect()
}

/// A video track's sample entry, samples (decode order) with pts, and sync flags.
pub struct Stream {
    pub entry: SampleEntry,
    pub samples: Vec<(Vec<u8>, i64)>,
    pub sync: Vec<bool>,
}

pub fn read_stream(path: &Path) -> Stream {
    let bytes = std::fs::read(path).unwrap();
    let file = cfa4_isobmff::open(bytes.clone()).unwrap();
    let t = file.track_of_kind(cfa4_isobmff::TrackKind::Video).unwrap();
    let track = &file.tracks[t];
    let samples = (0..track.samples.len()).map(|i| (file.read_sample(&bytes, t, i).unwrap(), track.samples[i].pts)).collect();
    Stream { entry: track.entries[0].clone(), samples, sync: track.samples.iter().map(|s| s.is_sync).collect() }
}

/// Feed `samples` then flush.
pub fn decode_all(d: &mut dyn VideoDecoder, samples: &[(Vec<u8>, i64)]) -> Vec<DecodedFrame> {
    let mut out = Vec::new();
    for (s, p) in samples {
        out.extend(d.decode(s, *p).unwrap_or_else(|e| panic!("{}: {e}", d.name())));
    }
    out.extend(d.flush());
    out
}

/// Assert two decoders' outputs are the same pictures (pts order, size, format, colour, aspect
/// and every sample).
pub fn assert_same(what: &str, a: &[DecodedFrame], b: &[DecodedFrame]) {
    let pa: Vec<i64> = a.iter().map(|f| f.pts).collect();
    let pb: Vec<i64> = b.iter().map(|f| f.pts).collect();
    assert_eq!(pa, pb, "{what}: pts sequence");
    for (x, y) in a.iter().zip(b) {
        let (fx, fy) = (&x.frame, &y.frame);
        assert_eq!((fx.width, fx.height, fx.par, fx.color), (fy.width, fy.height, fy.par, fy.color), "{what}: pts {} geometry / colour", x.pts);
        match (&fx.data, &fy.data) {
            (PixelData::Yuv8 { planes: p, chroma: c, .. }, PixelData::Yuv8 { planes: q, chroma: d, .. }) => {
                assert_eq!(c, d, "{what}: chroma");
                for i in 0..3 {
                    assert!(p[i] == q[i], "{what}: pts {} plane {i} differs (max diff {})", x.pts, max_diff8(&p[i], &q[i]));
                }
            }
            (PixelData::Yuv16 { planes: p, chroma: c, bits: bx, .. }, PixelData::Yuv16 { planes: q, chroma: d, bits: by, .. }) => {
                assert_eq!((c, bx), (d, by), "{what}: chroma / bits");
                for i in 0..3 {
                    assert!(p[i] == q[i], "{what}: pts {} plane {i} differs (max diff {})", x.pts, max_diff16(&p[i], &q[i]));
                }
            }
            _ => panic!("{what}: pts {}: pixel formats differ", x.pts),
        }
    }
}

fn max_diff8(a: &[u8], b: &[u8]) -> String {
    if a.len() != b.len() {
        return format!("sizes {} vs {}", a.len(), b.len());
    }
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0).to_string()
}

fn max_diff16(a: &[u16], b: &[u16]) -> String {
    if a.len() != b.len() {
        return format!("sizes {} vs {}", a.len(), b.len());
    }
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0).to_string()
}

pub fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}
