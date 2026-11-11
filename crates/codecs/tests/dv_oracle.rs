//! DV in every container CFA4 imports it from (raw `.dv`, AVI `dvsd`, QuickTime `dvc `/`dvcp`/
//! `dvpp`/`dv5p`/`dvhp`, MXF), DVCPRO 50 and DVCPRO HD included, against ffmpeg: ffmpeg's `dvvideo` encoder writes the files and its decoder is the
//! oracle. The DCT is not bit-exact by definition (our float IDCT against ffmpeg's integer one), so
//! samples match within a small tolerance; frame count, size, sampling and aspect are exact.
//! 4:1:1 enters CFA4 as 4:2:2 (owner decision, 2026-11-01): every other 4:2:2 chroma sample is a
//! 4:1:1 sample and is compared with ffmpeg's 4:1:1 output.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData};
use cfa4_media::{FrameRequest, MediaKind, SharedSource};
use cfa4_time::{FrameRate, Tick};
use common::*;

struct Spec {
    name: &'static str,
    lavfi: &'static str,
    pix_fmt: &'static str,
    extra: &'static [&'static str],
    size: (u32, u32),
    /// frame rate (numerator, denominator)
    rate: (i64, i64),
    container: &'static str,
    par: (u32, u32),
}

const FRAMES: i64 = 4;

const NTSC: &str = "testsrc2=size=720x480:rate=30000/1001";
const PAL: &str = "testsrc2=size=720x576:rate=25";

const SPECS: &[Spec] = &[
    Spec { name: "dv_ntsc_411.dv", lavfi: NTSC, pix_fmt: "yuv411p", extra: &[], size: (720, 480), rate: (30000, 1001), container: "DV", par: (8, 9) },
    Spec {
        name: "dv_pal_420_wide.dif",
        lavfi: PAL,
        pix_fmt: "yuv420p",
        extra: &["-aspect", "16:9", "-f", "dv"],
        size: (720, 576),
        rate: (25, 1),
        container: "DV",
        par: (64, 45),
    },
    Spec { name: "dv_ntsc_411.avi", lavfi: NTSC, pix_fmt: "yuv411p", extra: &[], size: (720, 480), rate: (30000, 1001), container: "AVI", par: (8, 9) },
    Spec {
        name: "dv_pal_420_wide.avi",
        lavfi: PAL,
        pix_fmt: "yuv420p",
        extra: &["-aspect", "16:9"],
        size: (720, 576),
        rate: (25, 1),
        container: "AVI",
        par: (64, 45),
    },
    Spec { name: "dv_ntsc_411.mov", lavfi: NTSC, pix_fmt: "yuv411p", extra: &[], size: (720, 480), rate: (30000, 1001), container: "", par: (8, 9) },
    Spec { name: "dv_pal_420.mov", lavfi: PAL, pix_fmt: "yuv420p", extra: &[], size: (720, 576), rate: (25, 1), container: "", par: (16, 15) },
    Spec { name: "dvcpro_pal_411.mov", lavfi: PAL, pix_fmt: "yuv411p", extra: &[], size: (720, 576), rate: (25, 1), container: "", par: (16, 15) },
    Spec { name: "dv_pal_420.mxf", lavfi: PAL, pix_fmt: "yuv420p", extra: &[], size: (720, 576), rate: (25, 1), container: "", par: (16, 15) },
    // DVCPRO 50 and DVCPRO HD (4:2:2; HD is BT.709 and anamorphic 16:9)
    Spec { name: "dvcpro50_pal.mov", lavfi: PAL, pix_fmt: "yuv422p", extra: &[], size: (720, 576), rate: (25, 1), container: "", par: (16, 15) },
    Spec {
        name: "dvcpro50_ntsc_wide.dv",
        lavfi: NTSC,
        pix_fmt: "yuv422p",
        extra: &["-aspect", "16:9"],
        size: (720, 480),
        rate: (30000, 1001),
        container: "DV",
        par: (32, 27),
    },
    Spec {
        name: "dvcprohd_1080i50.dv",
        lavfi: "testsrc2=size=1440x1080:rate=25",
        pix_fmt: "yuv422p",
        extra: &[],
        size: (1440, 1080),
        rate: (25, 1),
        container: "DV",
        par: (4, 3),
    },
    Spec {
        name: "dvcprohd_1080i50.avi",
        lavfi: "testsrc2=size=1440x1080:rate=25",
        pix_fmt: "yuv422p",
        extra: &[],
        size: (1440, 1080),
        rate: (25, 1),
        container: "AVI",
        par: (4, 3),
    },
    Spec {
        name: "dvcprohd_1080i60.mxf",
        lavfi: "testsrc2=size=1280x1080:rate=30000/1001",
        pix_fmt: "yuv422p",
        extra: &[],
        size: (1280, 1080),
        rate: (30000, 1001),
        container: "",
        par: (3, 2),
    },
    Spec {
        name: "dvcprohd_720p60.mov",
        lavfi: "testsrc2=size=960x720:rate=60000/1001",
        pix_fmt: "yuv422p",
        extra: &[],
        size: (960, 720),
        rate: (60000, 1001),
        container: "",
        par: (4, 3),
    },
];

fn args(s: &Spec) -> Vec<&'static str> {
    [&["-f", "lavfi", "-i", s.lavfi, "-frames:v", "4", "-an", "-c:v", "dvvideo", "-pix_fmt", s.pix_fmt][..], s.extra].concat()
}

fn make(ff: &Path, s: &Spec) -> PathBuf {
    fixture(ff, s.name, &args(s)).unwrap_or_else(|| panic!("could not generate {}", s.name))
}

fn at(k: i64, rate: FrameRate) -> FrameRequest {
    // the middle of frame k
    FrameRequest::full(Tick::from_rational(2 * k + 1, rate.den, 2 * rate.num))
}

fn check(ff: &Path, s: &Spec, src: &SharedSource) {
    let rate = FrameRate::new(s.rate.0, s.rate.1);
    let info = src.info();
    assert_eq!(info.kind, MediaKind::Movie, "{}", s.name);
    if !s.container.is_empty() {
        assert_eq!(info.container, s.container, "{}", s.name);
    }
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), s.size, "{}", s.name);
    assert_eq!(v.frame_rate, rate, "{}", s.name);
    assert_eq!(v.par, s.par, "{}", s.name);
    assert!(v.codec.starts_with("DV"), "{}: codec {}", s.name, v.codec);
    let want_fmt = if s.pix_fmt == "yuv420p" { "YUV 4:2:0 8-bit" } else { "YUV 4:2:2 8-bit" };
    assert_eq!(v.pixel_format, want_fmt, "{}", s.name);
    let hd = s.size.1 >= 720;
    let matrix = if hd { cfa4_color::Matrix::Bt709 } else { cfa4_color::Matrix::Bt601 };
    assert_eq!(v.color.matrix, matrix, "{}", s.name);
    // QuickTime's time scale can round the NTSC frame duration: within half a frame
    let want = Tick::from_rational(FRAMES, s.rate.1, s.rate.0);
    let half = Tick::from_rational(1, s.rate.1, 2 * s.rate.0);
    assert!(info.duration > want - half && info.duration < want + half, "{}: duration {:?}", s.name, info.duration);

    let file = dir().join(s.name);
    let raw = ffmpeg_frames(ff, &file, s.pix_fmt);
    let (w, h) = (s.size.0 as usize, s.size.1 as usize);
    let (cw, ch) = match s.pix_fmt {
        "yuv420p" => (w / 2, h / 2),
        "yuv422p" => (w / 2, h),
        _ => (w / 4, h),
    };
    let per = w * h + 2 * cw * ch;
    assert_eq!(raw.len(), per * FRAMES as usize, "{}: ffmpeg frame count", s.name);
    // in reverse order: every frame is intra, random access needs no warm-up
    for k in (0..FRAMES).rev() {
        let f = src.video_frame(at(k, rate)).unwrap();
        assert_eq!((f.width, f.height, f.par), (s.size.0, s.size.1, s.par), "{} frame {k}", s.name);
        let PixelData::Yuv8 { planes: p, chroma, .. } = &f.data else { panic!("{}: {:?}", s.name, f.data) };
        let ours: Vec<Vec<u16>> = if s.pix_fmt == "yuv411p" {
            assert_eq!(*chroma, Chroma::C422, "{}", s.name);
            assert_eq!(p[1].len(), w / 2 * h, "{}", s.name);
            let even = |pl: &[u8]| pl.iter().step_by(2).map(|&v| u16::from(v)).collect::<Vec<_>>();
            vec![p[0].iter().map(|&v| u16::from(v)).collect(), even(&p[1]), even(&p[2])]
        } else {
            assert_eq!(*chroma, if s.pix_fmt == "yuv422p" { Chroma::C422 } else { Chroma::C420 }, "{}", s.name);
            planes(&f)
        };
        let k = k as usize;
        let want = raw_planes(&raw[k * per..(k + 1) * per], w, h, cw, ch, 1);
        let d = max_diff(&ours, &want);
        assert!(d <= 3, "{} frame {k}: max difference {d} against ffmpeg", s.name);
    }
}

#[test]
fn dv_in_every_container_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let file = make(&ff, s);
        let src = cfa4_codecs::open_bytes(s.name, bytes(&file)).unwrap_or_else(|e| panic!("{}: {e}", s.name));
        check(&ff, s, &src);
    }
}

#[test]
fn raw_dv_opens_through_a_reader_and_drops_a_partial_frame() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let s = &SPECS[0];
    let file = make(&ff, s);
    let mut data = std::fs::read(&file).unwrap();
    // read on demand, like a file opened from disk
    let reader: cfa4_media::SharedReader = Arc::new(cfa4_media::reader::MemReader(Arc::from(data.clone())));
    let src = cfa4_media::reader::open_reader(s.name, reader, &cfa4_codecs::reader_openers(), &cfa4_codecs::openers()).unwrap();
    check(&ff, s, &src);
    // a trailing partial frame (an interrupted capture) is left out
    data.extend_from_within(..50_000);
    let src = cfa4_codecs::open_bytes(s.name, data.into()).unwrap();
    assert_eq!(src.info().duration, Tick::from_rational(FRAMES, s.rate.1, s.rate.0));
}

#[test]
fn damaged_raw_dv_never_panics() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let file = make(&ff, &SPECS[1]);
    let full = std::fs::read(&file).unwrap();
    let mut rng = Rng(0x05ee_dd1f);
    for round in 0..20 {
        let mut g = full.clone();
        // keep the first frame's head so the file still opens; damage everything else
        for _ in 0..200 {
            let at = 6 * 80 + rng.below((g.len() - 6 * 80) as u64) as usize;
            g[at] = rng.below(256) as u8;
        }
        if round % 5 == 0 {
            g.truncate(g.len() / 2 + round);
        }
        if let Ok(src) = cfa4_codecs::open_bytes("bad.dv", g.into()) {
            for k in 0..FRAMES {
                let _ = src.video_frame(at(k, FrameRate::new(25, 1)));
            }
        }
    }
}

/// `cargo xtask fixtures codecs`: build every DV fixture.
#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg() else {
        for s in SPECS {
            cfa4_testkit::fixtures::report(s.name, cfa4_testkit::fixtures::Status::Skipped);
        }
        return;
    };
    for s in SPECS {
        let out = dir().join(s.name);
        cfa4_testkit::fixtures::generate_and_report(s.name, std::slice::from_ref(&out), || fixture(&ff, s.name, &args(s)));
    }
}

fn riff_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn riff_list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    riff_chunk(b"LIST", &[&kind[..], body].concat())
}

/// A type-1 DV AVI: one `iavs` stream (handler `dvsd`, a DVINFO `strf`) of whole DIF frames,
/// with an `idx1` (ffmpeg only writes type 2, so the file is built here from a raw DV file).
fn type1_avi(frames: &[&[u8]], rate: (u32, u32)) -> Vec<u8> {
    let mut avih = vec![0u8; 56];
    avih[16..20].copy_from_slice(&(frames.len() as u32).to_le_bytes());
    avih[24..28].copy_from_slice(&1u32.to_le_bytes());
    let mut strh = vec![0u8; 56];
    strh[..4].copy_from_slice(b"iavs");
    strh[4..8].copy_from_slice(b"dvsd");
    strh[20..24].copy_from_slice(&rate.1.to_le_bytes());
    strh[24..28].copy_from_slice(&rate.0.to_le_bytes());
    strh[32..36].copy_from_slice(&(frames.len() as u32).to_le_bytes());
    let strl = riff_list(b"strl", &[riff_chunk(b"strh", &strh), riff_chunk(b"strf", &[0u8; 32])].concat());
    let hdrl = riff_list(b"hdrl", &[riff_chunk(b"avih", &avih), strl].concat());
    let mut body = Vec::new();
    let mut idx = Vec::new();
    for f in frames {
        idx.extend_from_slice(b"00__");
        idx.extend_from_slice(&0x10u32.to_le_bytes());
        idx.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        idx.extend_from_slice(&(f.len() as u32).to_le_bytes());
        body.extend(riff_chunk(b"00__", f));
    }
    let movi = riff_list(b"movi", &body);
    riff_chunk(b"RIFF", &[&b"AVI "[..], &hdrl, &movi, &riff_chunk(b"idx1", &idx)].concat())
}

/// Type-1 DV AVI: the `iavs` stream's video decodes like the raw DV file it was built from.
#[test]
fn type1_dv_avi_video_matches_raw_dv() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for (k, rate) in [(0usize, (30000u32, 1001u32)), (1, (25, 1))] {
        let s = &SPECS[k];
        let raw = std::fs::read(make(&ff, s)).unwrap();
        let ranges = cfa4_dv::frames(&raw).unwrap();
        let frames: Vec<&[u8]> = ranges.iter().map(|r| &raw[r.clone()]).collect();
        let avi = cfa4_codecs::open_bytes("type1.avi", type1_avi(&frames, rate).into()).unwrap();
        let dv = cfa4_codecs::open_bytes(s.name, raw.clone().into()).unwrap();
        let (a, b) = (avi.info().video.clone().unwrap(), dv.info().video.clone().unwrap());
        assert_eq!((a.width, a.height, a.par, &a.codec, &a.pixel_format), (b.width, b.height, b.par, &b.codec, &b.pixel_format), "{}", s.name);
        assert_eq!(a.frame_rate, b.frame_rate, "{}", s.name);
        assert_eq!(avi.info().container, "AVI");
        let fr = FrameRate::new(i64::from(rate.0), i64::from(rate.1));
        for i in 0..FRAMES {
            let (x, y) = (avi.video_frame(at(i, fr)).unwrap(), dv.video_frame(at(i, fr)).unwrap());
            assert_eq!((planes(&x), x.par), (planes(&y), y.par), "{} frame {i}", s.name);
        }
    }
}

/// Raw DV with 16-bit audio: the source's audio (at the file's rate) equals ffmpeg's decode, and
/// a request at another rate is resampled.
#[test]
fn raw_dv_audio_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for (name, lavfi, pix, rate) in [("dv_ntsc_audio48k.dv", NTSC, "yuv411p", 48_000u32), ("dv_pal_audio32k.dv", PAL, "yuv420p", 32_000)] {
        let r = rate.to_string();
        let audio = format!("aevalsrc=0.5*sin(2*PI*440*t)|0.3*sin(2*PI*997*t):s={r}");
        let args = [
            "-f",
            "lavfi",
            "-i",
            lavfi,
            "-f",
            "lavfi",
            "-i",
            &audio,
            "-frames:v",
            "10",
            "-c:v",
            "dvvideo",
            "-pix_fmt",
            pix,
            "-c:a",
            "pcm_s16le",
            "-ar",
            &r,
            "-ac",
            "2",
            "-f",
            "dv",
        ];
        let file = fixture(&ff, name, &args).unwrap_or_else(|| panic!("could not generate {name}"));
        let src = cfa4_codecs::open_bytes(name, bytes(&file)).unwrap();
        let a = src.info().audio_streams.first().cloned().unwrap_or_else(|| panic!("{name}: no audio stream"));
        assert_eq!((a.sample_rate, a.channels, a.bits_per_sample), (rate, 2, Some(16)), "{name}");
        let want = ffmpeg_audio_f32(&ff, &file, &[]);
        let n = want.len() / 2;
        let got = src.audio(0, n, rate).unwrap();
        let worst = (0..n).flat_map(|k| (0..2).map(move |c| (k, c))).map(|(k, c)| (got.channels[c][k] - want[2 * k + c]).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-4, "{name}: worst difference {worst}");
        // a window in the middle, resampled to 48 kHz, has the same tone power
        let mid = src.audio(i64::from(rate) / 10, 4800, 48_000).unwrap();
        let rms = (mid.channels[0].iter().map(|v| v * v).sum::<f32>() / 4800.0).sqrt();
        assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.02, "{name}: rms {rms}");
    }
}

/// Type-1 DV AVI: the sound inside the `iavs` frames is audio stream 0 and equals the raw DV
/// file's (which matches ffmpeg, `raw_dv_audio_matches_ffmpeg`); a type-1 file whose frames carry
/// no audio has no audio stream.
#[test]
fn type1_dv_avi_audio_matches_raw_dv() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let audio = "aevalsrc=0.5*sin(2*PI*440*t)|0.3*sin(2*PI*997*t):s=48000";
    let args = [
        "-f",
        "lavfi",
        "-i",
        NTSC,
        "-f",
        "lavfi",
        "-i",
        audio,
        "-frames:v",
        "10",
        "-c:v",
        "dvvideo",
        "-pix_fmt",
        "yuv411p",
        "-c:a",
        "pcm_s16le",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-f",
        "dv",
    ];
    let file = fixture(&ff, "dv_ntsc_audio48k.dv", &args).unwrap_or_else(|| panic!("could not generate dv_ntsc_audio48k.dv"));
    let raw = std::fs::read(&file).unwrap();
    let ranges = cfa4_dv::frames(&raw).unwrap();
    let frames: Vec<&[u8]> = ranges.iter().map(|r| &raw[r.clone()]).collect();
    let avi = cfa4_codecs::open_bytes("type1_audio.avi", type1_avi(&frames, (30000, 1001)).into()).unwrap();
    let dv = cfa4_codecs::open_bytes("dv_ntsc_audio48k.dv", raw.clone().into()).unwrap();
    let (a, b) = (avi.info().audio_streams.clone(), dv.info().audio_streams.clone());
    assert_eq!(a.len(), 1, "type-1 AVI: one audio stream");
    assert_eq!((a[0].sample_rate, a[0].channels, &a[0].codec, a[0].bits_per_sample), (b[0].sample_rate, b[0].channels, &b[0].codec, b[0].bits_per_sample));
    // the whole sound, and a window in the middle resampled to 44.1 kHz
    let n = 10 * 1601;
    assert_eq!(avi.audio(0, n, 48_000).unwrap().channels, dv.audio(0, n, 48_000).unwrap().channels);
    assert_eq!(avi.audio(5000, 2000, 44_100).unwrap().channels, dv.audio(5000, 2000, 44_100).unwrap().channels);
    // frames without audio blocks' content: no audio stream
    let silent = std::fs::read(make(&ff, &SPECS[0])).unwrap();
    let ranges = cfa4_dv::frames(&silent).unwrap();
    let frames: Vec<&[u8]> = ranges.iter().map(|r| &silent[r.clone()]).collect();
    let avi = cfa4_codecs::open_bytes("type1.avi", type1_avi(&frames, (30000, 1001)).into()).unwrap();
    let dv = cfa4_codecs::open_bytes(SPECS[0].name, silent.clone().into()).unwrap();
    assert_eq!(avi.info().audio_streams.len(), dv.info().audio_streams.len());
}
