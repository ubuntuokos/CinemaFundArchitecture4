//! DV in every container CFA4 imports it from (raw `.dv`, AVI `dvsd`, QuickTime `dvc `/`dvcp`/
//! `dvpp`, MXF), against ffmpeg: ffmpeg's `dvvideo` encoder writes the files and its decoder is the
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
    assert_eq!(v.color.matrix, cfa4_color::Matrix::Bt601, "{}", s.name);
    // QuickTime's time scale can round the NTSC frame duration: within half a frame
    let want = Tick::from_rational(FRAMES, s.rate.1, s.rate.0);
    let half = Tick::from_rational(1, s.rate.1, 2 * s.rate.0);
    assert!(info.duration > want - half && info.duration < want + half, "{}: duration {:?}", s.name, info.duration);

    let file = dir().join(s.name);
    let raw = ffmpeg_frames(ff, &file, s.pix_fmt);
    let (w, h) = (s.size.0 as usize, s.size.1 as usize);
    let (cw, ch) = if s.pix_fmt == "yuv420p" { (w / 2, h / 2) } else { (w / 4, h) };
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
            assert_eq!(*chroma, Chroma::C420, "{}", s.name);
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
