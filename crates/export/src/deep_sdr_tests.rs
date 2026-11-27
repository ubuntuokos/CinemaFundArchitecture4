//! SDR exports to deep formats (10 bits and more) keep the precision of deep sources: the
//! encoders get R'G'B' floats, not the RGBA8 picture (PROCODEC-002 1b).

use super::*;
use cfa4_media::{FrameRequest, MediaSource};
use cfa4_time::TICKS_PER_SECOND;

/// A horizontal ramp from black to white, even in sRGB code values, as linear floats: finer than
/// 8 bits (320 pixels share a range a 10-bit luma spans with 876 codes).
struct Ramp(cfa4_media::SharedSource);

impl MediaSource for Ramp {
    fn info(&self) -> &cfa4_media::MediaInfo {
        self.0.info()
    }
    fn video_frame(&self, _: FrameRequest) -> cfa4_media::Result<Arc<cfa4_frame::VideoFrame>> {
        let (w, h) = (320usize, 180usize);
        let row: Vec<f32> = (0..w)
            .flat_map(|x| {
                let v = cfa4_color::srgb_to_linear(x as f32 / (w - 1) as f32);
                [v, v, v, 1.0]
            })
            .collect();
        Ok(Arc::new(cfa4_frame::VideoFrame::rgba_f32(w as u32, h as u32, row.repeat(h))))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> cfa4_media::Result<cfa4_frame::AudioBuffer> {
        self.0.audio(start, frames, sample_rate)
    }
}

fn ramp_project() -> (Arc<Project>, ItemId, cfa4_render::SourceMap) {
    let (p, seq, mut m) = crate::tests::project();
    let item = p.sequence(seq).unwrap().video_tracks[0].items[0].item;
    let src = m.0.get(&item).unwrap().clone();
    m.0.insert(item, Arc::new(Ramp(src)));
    (p, seq, m)
}

/// Distinct luma (or green) codes across the middle row of the frame at 1/3 s of `path`.
fn distinct_codes(path: &str) -> (usize, String) {
    let bytes: Arc<[u8]> = std::fs::read(path).unwrap().into();
    let src = cfa4_codecs::open_bytes(path, bytes).unwrap();
    let f = src.video_frame(FrameRequest::full(Tick(TICKS_PER_SECOND / 3))).unwrap();
    let w = f.width as usize;
    let row = (f.height as usize / 2) * w;
    let plane: Vec<u16> = match &f.data {
        cfa4_frame::PixelData::Yuv16 { planes, .. } => planes[0][row..row + w].to_vec(),
        cfa4_frame::PixelData::Rgb16 { planes, .. } => planes[1][row..row + w].to_vec(),
        other => panic!(
            "{path}: expected a deep frame, got {}",
            match other {
                cfa4_frame::PixelData::Rgba8(_) => "RGBA8",
                cfa4_frame::PixelData::RgbaF32(_) => "RGBA f32",
                _ => "8-bit Y'CbCr",
            }
        ),
    };
    let mut codes = plane;
    codes.sort_unstable();
    codes.dedup();
    (codes.len(), f.format_label())
}

#[test]
fn deep_sdr_exports_keep_more_than_8_bits() {
    let (p, seq, m) = ramp_project();
    let dir = std::env::temp_dir().join(format!("fc-deep-sdr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        ("prores.mov", ExportSettings { format: Format::ProRes, ..Default::default() }),
        ("dnx.mov", ExportSettings { format: Format::DnxHr, dnx_profile: "hqx".into(), ..Default::default() }),
        ("cfhd.mov", ExportSettings { format: Format::Cineform, cineform_profile: "rgb".into(), ..Default::default() }),
    ];
    for (name, mut s) in cases {
        s.path = dir.join(name).to_string_lossy().into_owned();
        s.range = Some(TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(12)));
        export(&p, seq, &s, &m, &Progress::default()).unwrap();
        let (n, label) = distinct_codes(&s.path);
        // RGBA8 input gives at most 256 codes; the ramp needs 320.
        assert!(n > 300, "{name} ({label}): {n} distinct codes across a 320-step ramp");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pipeline's SDR floats match its RGBA8 picture to within 8-bit rounding, unpremultiplied
/// when the export keeps straight alpha.
#[test]
fn sdr_floats_agree_with_rgba8() {
    let (p, seq, m) = ramp_project();
    let s = ExportSettings { format: Format::ProRes, ..Default::default() };
    let pipe = pipeline::Pipeline::new(p, seq, &s, false).unwrap().with_deep_sdr(true);
    assert!(pipe.float_out());
    let (rgba, float) = pipe.frame(0, &m);
    assert_eq!(float.len() * 4, rgba.len() * 3);
    for (px, f) in rgba.chunks_exact(4).zip(float.chunks_exact(3)) {
        for c in 0..3 {
            assert!((f32::from(px[c]) / 255.0 - f[c]).abs() <= 1.0 / 255.0, "{px:?} vs {f:?}");
        }
    }
    // never for HDR output, which has its own floats
    let (p, seq, _) = ramp_project();
    let hdr = pipeline::Pipeline::new(p, seq, &s, true).unwrap().with_deep_sdr(true);
    assert!(!hdr.deep_sdr);
}

/// DCP: X'Y'Z' from floats equals X'Y'Z' from RGBA8 at the 8-bit codes, and splits the codes
/// between them.
#[test]
fn dcp_xyz_from_floats() {
    let rgba: Vec<u8> = (0..=255u8).flat_map(|v| [v, v / 2, 255 - v, 255]).collect();
    let float: Vec<f32> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0)).collect();
    let a = dcp_out::rgba_to_xyz12(&rgba, 256, 1);
    let b = dcp_out::rgbf_to_xyz12(&float, 256, 1);
    for c in 0..3 {
        for (x, y) in a[c].iter().zip(&b[c]) {
            assert!((x - y).abs() <= 1, "component {c}: {x} vs {y}");
        }
    }
    let between = dcp_out::rgbf_to_xyz12(&[127.5 / 255.0; 3], 1, 1);
    let (lo, hi) = (dcp_out::rgba_to_xyz12(&[127, 127, 127, 255], 1, 1), dcp_out::rgba_to_xyz12(&[128, 128, 128, 255], 1, 1));
    assert!(lo[1][0] < between[1][0] && between[1][0] < hi[1][0], "{lo:?} {between:?} {hi:?}");
    // short or hostile input never panics
    let _ = dcp_out::rgbf_to_xyz12(&[f32::NAN, f32::INFINITY], 4, 4);
}

/// DPX (10-bit) and OpenEXR (half float) image sequences take the floats too: the ramp keeps its
/// 320 levels instead of 256.
#[test]
fn deep_image_sequences_keep_more_than_8_bits() {
    let (p, seq, m) = ramp_project();
    let dir = std::env::temp_dir().join(format!("fc-deep-seq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for format in [Format::DpxSequence, Format::ExrSequence] {
        let path = dir.join(format!("ramp.{}", format.extension())).to_string_lossy().into_owned();
        let s = ExportSettings { format, path, range: Some(TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(1))), ..Default::default() };
        export(&p, seq, &s, &m, &Progress::default()).unwrap();
        let file = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == format.extension())).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        let mut row: Vec<u32> = match format {
            Format::DpxSequence => {
                // one big-endian word per pixel after the 2048-byte header: R, G, B in 10 bits each
                let words: Vec<u32> = bytes[2048..].as_chunks::<4>().0.iter().map(|w| u32::from_be_bytes(*w)).collect();
                words[90 * 320..91 * 320].iter().map(|w| (w >> 12) & 0x3FF).collect()
            }
            _ => {
                let d = cfa4_media::still::exr::decode("ramp.exr", &bytes).unwrap();
                let cfa4_frame::PixelData::RgbaF32(px) = &d.frame.data else { panic!("EXR decodes to floats") };
                px.chunks_exact(4).skip(90 * 320).take(320).map(|p| p[1].to_bits()).collect()
            }
        };
        row.sort_unstable();
        row.dedup();
        assert!(row.len() > 300, "{format:?}: {} distinct values across a 320-step ramp", row.len());
        let _ = std::fs::remove_file(&file);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The 8-bit path of the DPX and EXR writers is unchanged, and float input agrees with it at the
/// 8-bit codes (alpha included).
#[test]
fn dpx_and_exr_floats_match_their_8_bit_codes() {
    let rgba: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 3, if v % 2 == 0 { 255 } else { 128 }]).collect();
    let float: Vec<f32> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0)).collect();
    let a = raster::dpx(&rgba, 256, 1).unwrap();
    let b = raster::dpx_deep(&rgba, Some(&float), 256, 1, None).unwrap();
    assert_eq!(a.len(), b.len());
    for (x, y) in a[2048..].as_chunks::<4>().0.iter().zip(b[2048..].as_chunks::<4>().0) {
        let (x, y) = (u32::from_be_bytes(*x), u32::from_be_bytes(*y));
        for s in [22, 12, 2] {
            assert!(((x >> s) & 0x3FF).abs_diff((y >> s) & 0x3FF) <= 1, "{x:08x} vs {y:08x}");
        }
    }
    for alpha in [false, true] {
        let a = cfa4_media::still::exr::decode("a.exr", &raster::exr(&rgba, 256, 1, alpha, ExrCompression::Zip).unwrap()).unwrap();
        let b = cfa4_media::still::exr::decode("b.exr", &raster::exr_with(&rgba, Some(&float), 256, 1, alpha, ExrCompression::Zip, None).unwrap()).unwrap();
        let (cfa4_frame::PixelData::RgbaF32(a), cfa4_frame::PixelData::RgbaF32(b)) = (&a.frame.data, &b.frame.data) else { panic!("floats") };
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() <= 1e-3 * x.abs().max(1e-2), "alpha {alpha}: {x} vs {y}");
        }
    }
    // short float input falls back to the 8-bit picture, NaN codes as black
    assert_eq!(raster::dpx_deep(&rgba, Some(&float[..3]), 256, 1, None).unwrap(), a);
    let nan = raster::dpx_deep(&rgba[..4], Some(&[f32::NAN; 3]), 1, 1, None).unwrap();
    assert_eq!(&nan[2048..], &[0, 0, 0, 0]);
}
