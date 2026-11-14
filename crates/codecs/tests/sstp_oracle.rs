//! MPEG-4 Simple Studio Profile (`cfa4-sstp`) through the media layer against ffmpeg: raw
//! elementary streams (opened by content, frames in a shuffled order) and the same pictures as
//! MPEG-4 Visual essence in MXF (ffmpeg cannot write that, so the file is our writer's with the
//! picture essence coding label set to MPEG-4 Visual).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cfa4_frame::PixelData;
use cfa4_media::{FrameRequest, MediaSource};
use cfa4_mxf::{FrameInfo, MxfWriter, PackageIds, Pattern, PictureCoding, PictureDesc, Rational, WriterConfig};

#[path = "../../sstp/tests/synth/mod.rs"]
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

/// A frame's samples in ffmpeg's plane order (G, B, R for RGB).
fn samples(data: &PixelData) -> Vec<u16> {
    match data {
        PixelData::Yuv16 { planes, bits: 10, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(),
        PixelData::Rgb16 { planes: [r, g, b], bits: 10, .. } => [g, b, r].iter().flat_map(|p| p.iter().copied()).collect(),
        _ => panic!("not a 10-bit frame"),
    }
}

/// `n` pictures of `c` as a raw stream.
fn stream(c: &Cfg, seed: u64, n: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    (0..n).flat_map(|_| picture(c, &mut rng)).collect()
}

/// Every frame of `src` (in a shuffled order) against ffmpeg's frames of `reference`.
fn check(src: &dyn MediaSource, reference: &Path, pix_fmt: &str, n: usize) {
    let v = src.info().video.clone().unwrap();
    let want = ffmpeg_raw(reference, pix_fmt);
    assert_eq!(want.len() % n, 0);
    let per = want.len() / n;
    let mut order: Vec<usize> = (0..n).collect();
    order.reverse();
    order.swap(0, n / 2);
    for i in order.into_iter().chain(0..n) {
        let f = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let ours = samples(&f.data);
        assert_eq!(ours.len(), per, "frame {i}: sample count");
        let diff = ours.iter().zip(&want[i * per..(i + 1) * per]).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "frame {i}: {diff} of {per} samples differ");
    }
}

/// `pictures` as MPEG-4 Visual essence in an OP1a file at 30000/1001.
fn mxf(pictures: &[&[u8]], w: u32, h: u32) -> Vec<u8> {
    let mut cfg = WriterConfig::new(Pattern::Op1a, Rational::new(30000, 1001), PackageIds::from_seed("sstp", "SStP"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::ProRes { profile: 3 }, w, h));
    let mut wr = MxfWriter::new(std::io::Cursor::new(Vec::new()), cfg).unwrap();
    for p in pictures {
        wr.push_picture(p, FrameInfo::intra()).unwrap();
    }
    let mut f = wr.finish().unwrap().into_inner();
    // the ProRes 422 picture essence coding label (RDD 44) becomes MPEG-4 Visual (14496-2)
    let prores = [0x04, 0x01, 0x02, 0x02, 0x03, 0x06, 0x03];
    let mut patched = 0;
    for i in 0..f.len().saturating_sub(prores.len()) {
        if f[i..i + prores.len()] == prores {
            f[i + 4..i + 7].copy_from_slice(&[0x01, 0x20, 0x10]);
            patched += 1;
        }
    }
    assert!(patched > 0);
    f
}

#[test]
fn elementary_streams_and_mxf_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let dir = fixtures();
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        ("422", Cfg { width: 64, height: 32, slice_every: 3, ..BASE }, "yuv422p10le", 6),
        ("rgb", Cfg { rgb: true, chroma_format: 3, width: 33, height: 20, mpeg_quant: true, ..BASE }, "gbrp10le", 4),
    ];
    for (k, (name, c, fmt, n)) in cases.into_iter().enumerate() {
        let es = stream(&c, 0x5EED_0000 + k as u64, n);
        let path = dir.join(format!("media_{name}.m4v"));
        std::fs::write(&path, &es).unwrap();
        // the elementary stream, opened by content
        let src = cfa4_media::open_path(&path, &cfa4_codecs::openers()).unwrap();
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (c.width, c.height), "{name}");
        assert_eq!(v.codec, cfa4_codecs::sstp::NAME);
        assert_eq!((v.frame_rate.num, v.frame_rate.den), (30000, 1001));
        assert_eq!(src.info().duration, v.frame_rate.tick_of(n as i64));
        check(&*src, &path, fmt, n);
        // through the reader (index now, pictures on demand)
        let reader: cfa4_media::SharedReader = Arc::new(cfa4_media::reader::MemReader(es.clone().into()));
        let src = cfa4_codecs::SstpSource::open_reader(name, reader).unwrap();
        check(&src, &path, fmt, n);
        // the same pictures in MXF
        let units = cfa4_media_units(&es);
        assert_eq!(units.len(), n);
        let file = mxf(&units, c.width, c.height);
        let src = cfa4_codecs::MxfSource::open(&format!("{name}.mxf"), file.into()).unwrap();
        let mv = src.info().video.clone().unwrap();
        assert_eq!(mv.codec, cfa4_codecs::sstp::NAME);
        assert_eq!((mv.width, mv.height), (c.width, c.height));
        check(&src, &path, fmt, n);
        eprintln!("{name}: {n} pictures bit for bit (elementary stream and MXF)");
    }
}

/// The access units of a raw stream.
fn cfa4_media_units(b: &[u8]) -> Vec<&[u8]> {
    let mut starts = vec![0u64];
    cfa4_sstp::AccessUnits::new().feed(0, b, &mut starts);
    starts.push(b.len() as u64);
    starts.windows(2).map(|w| &b[w[0] as usize..w[1] as usize]).filter(|s| !s.is_empty()).collect()
}

#[test]
fn damaged_streams_never_panic() {
    let es = stream(&Cfg { width: 48, height: 32, ..BASE }, 7, 3);
    let mut rng = Rng(99);
    for round in 0..200 {
        let mut b = es.clone();
        let cut = (rng.next() as usize) % b.len();
        if round % 2 == 0 {
            b.truncate(cut.max(64));
        }
        for _ in 0..1 + round % 8 {
            let i = 64 + (rng.next() as usize) % (b.len() - 64);
            b[i] ^= 1 << (rng.next() % 8);
        }
        let Ok(src) = cfa4_codecs::SstpSource::open(&format!("bad{round}.m4v"), b.into()) else { continue };
        let v = src.info().video.clone().unwrap();
        for i in 0..4 {
            let _ = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i)));
        }
    }
}
