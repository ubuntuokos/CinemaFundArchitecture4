//! Panasonic RW2 camera raw stills (F2d of the FFmpeg migration plan, `docs/plans/CFA4-FFMPEG-9.1-001.md`).
//!
//! FFmpeg's `rw2` decoder gives the raw Bayer mosaic (16-bit) with the sensor's black and white
//! points and the as-shot white balance as `AV_FRAME_DATA_RAW_COLOR_PARAMS` side data, and the
//! EXIF block as `AV_FRAME_DATA_EXIF`. An RW2 file carries no camera colour matrix, so the
//! picture is developed by `cfa4-dng` (the DNG pipeline: black and white points, white balance,
//! Malvar–He–Cutler demosaic, colour matrix) with the user's DNG camera profile (`.dcp`) for the
//! camera model when one is installed, else white balanced only, as an approximation.
//!
//! Profiles are looked up in `$CFA4_CAMERA_PROFILES` (an absolute directory), then
//! `$XDG_DATA_HOME/cfa4/camera-profiles` (or `~/.local/share/cfa4/camera-profiles`), by their
//! UniqueCameraModel ("Panasonic DC-GH6"); a profile named "Adobe Standard" is preferred. CFA4
//! ships no profiles and downloads none.

#![cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]

use std::path::{Path, PathBuf};

use cfa4_dng::dcp::Profile;
use cfa4_dng::demosaic::Pattern;

/// Environment variable naming an extra camera profile directory (absolute path).
pub const PROFILES_ENV: &str = "CFA4_CAMERA_PROFILES";
/// Profiles larger than this are skipped (Adobe's are well under 1 MB).
const MAX_PROFILE_BYTES: u64 = 16 << 20;
/// Files looked at per directory tree, and how deep.
const MAX_PROFILE_FILES: usize = 4096;
const MAX_PROFILE_DEPTH: usize = 4;

/// The CFA pattern of an FFmpeg Bayer pixel format name (`bayer_rggb16le`…).
pub(crate) fn pattern_for(pix_fmt: &str) -> Option<Pattern> {
    let order = pix_fmt.strip_prefix("bayer_")?.get(..4)?;
    let colors = order.chars().map(|c| match c {
        'r' => Some(0u8),
        'g' => Some(1),
        'b' => Some(2),
        _ => None,
    });
    let colors: Option<Vec<u8>> = colors.collect();
    let p = Pattern { rows: 2, cols: 2, colors: colors? };
    p.is_bayer().then_some(p)
}

/// Apply TIFF orientation `o` (1–8) to `w` × `h` RGBA floats; returns the new size and pixels.
pub(crate) fn orient(o: u16, w: usize, h: usize, rgba: Vec<f32>) -> (usize, usize, Vec<f32>) {
    if !(2..=8).contains(&o) || rgba.len() != w * h * 4 {
        return (w, h, rgba);
    }
    let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
    let mut out = vec![0f32; rgba.len()];
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = match o {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            let s = (sy * w + sx) * 4;
            let d = (y * ow + x) * 4;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    (ow, oh, out)
}

/// The directories searched for camera profiles, in order.
pub(crate) fn profile_dirs() -> Vec<PathBuf> {
    let abs = |v: Option<std::ffi::OsString>| v.map(PathBuf::from).filter(|p| p.is_absolute());
    let mut dirs = Vec::new();
    if let Some(d) = abs(std::env::var_os(PROFILES_ENV)) {
        dirs.push(d);
    }
    match abs(std::env::var_os("XDG_DATA_HOME")) {
        Some(d) => dirs.push(d.join("cfa4/camera-profiles")),
        None => {
            if let Some(h) = abs(std::env::var_os("HOME")) {
                dirs.push(h.join(".local/share/cfa4/camera-profiles"));
            }
        }
    }
    dirs
}

fn dcp_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_PROFILE_DEPTH || out.len() >= MAX_PROFILE_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if out.len() >= MAX_PROFILE_FILES {
            return;
        }
        // symlinks are not followed (no loops, no escaping the directory)
        let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
        if meta.is_dir() {
            dcp_files(&p, depth + 1, out);
        } else if meta.is_file() && meta.len() <= MAX_PROFILE_BYTES && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dcp")) {
            out.push(p);
        }
    }
}

/// The camera profile for `model` in `dirs`: an "Adobe Standard" one when there is one, else the
/// first match (by path). Unreadable or malformed files are skipped.
pub(crate) fn find_profile(model: &str, dirs: &[PathBuf]) -> Option<(PathBuf, Profile)> {
    let mut found: Option<(PathBuf, Profile)> = None;
    for dir in dirs {
        let mut files = Vec::new();
        dcp_files(dir, 0, &mut files);
        for f in files {
            let Ok(bytes) = std::fs::read(&f) else { continue };
            let p = match cfa4_dng::dcp::parse(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    log::debug!("{}: not a usable camera profile: {e}", f.display());
                    continue;
                }
            };
            if !p.camera_model.as_deref().is_some_and(|m| m.trim().eq_ignore_ascii_case(model.trim())) {
                continue;
            }
            let standard = p.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case("Adobe Standard"));
            if standard {
                return Some((f, p));
            }
            if found.is_none() {
                found = Some((f, p));
            }
        }
    }
    found
}

/// What the RAW colour parameters side data says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorParams {
    /// Black and white points in 16-bit sample codes.
    pub black: f64,
    pub white: f64,
    /// Camera neutral (1 / white balance multiplier): red, green, blue.
    pub neutral: [f64; 3],
}

/// A level from the side data: FFmpeg gives it as a fraction of the 16-bit sample range
/// (2048/65535); a value above 1 is taken as a sample code.
fn level(num: i32, den: i32) -> Option<f64> {
    if den <= 0 || num < 0 {
        return None;
    }
    let v = f64::from(num) / f64::from(den);
    Some(if v <= 1.0 { v * 65_535.0 } else { v }).filter(|v| v.is_finite() && *v <= 65_535.0)
}

/// Check the values read from the side data.
pub(crate) fn color_params(black: (i32, i32), white: (i32, i32), wb_red: (i32, i32), wb_blue: (i32, i32)) -> Option<ColorParams> {
    let (black, white) = (level(black.0, black.1)?, level(white.0, white.1)?);
    if white <= black {
        return None;
    }
    let mult = |(n, d): (i32, i32)| (d > 0 && n > 0).then(|| f64::from(n) / f64::from(d)).filter(|m| (1.0 / 64.0..=64.0).contains(m));
    let (r, b) = (mult(wb_red)?, mult(wb_blue)?);
    Some(ColorParams { black, white, neutral: [1.0 / r, 1.0, 1.0 / b] })
}

#[cfg(feature = "ffmpeg")]
mod decode {
    use std::mem::{offset_of, size_of};
    use std::path::Path;
    use std::sync::Arc;

    use rsmedia::{DecoderBuilder, MediaType};
    use rsmpeg::ffi;

    use super::*;
    use crate::Error;
    use crate::backend;

    /// The container name FFmpeg gives RW2 files.
    pub(crate) const DEMUXER: &str = "rw2_pipe";

    fn pair(b: &[u8], at: usize) -> Option<(i32, i32)> {
        let n = i32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?);
        let d = i32::from_ne_bytes(b.get(at + 4..at + 8)?.try_into().ok()?);
        Some((n, d))
    }

    /// Parse `AVRawColorParams` from the side data's bytes, field by field at the offsets of the
    /// bound C struct (no unsafe code; the struct's size is not ABI, its field layout is).
    pub(crate) fn parse_color_params(b: &[u8]) -> Option<ColorParams> {
        let codec = offset_of!(ffi::AVRawColorParams, codec);
        if b.len() < codec + size_of::<ffi::AVRW2RawColorParams>() {
            return None;
        }
        let t = offset_of!(ffi::AVRawColorParams, type_);
        let ty = u32::from_ne_bytes(b.get(t..t + 4)?.try_into().ok()?);
        if ty != ffi::AV_RAW_COLOR_PARAMS_RW2 {
            return None;
        }
        color_params(
            pair(b, offset_of!(ffi::AVRawColorParams, black_level))?,
            pair(b, offset_of!(ffi::AVRawColorParams, white_level))?,
            pair(b, codec + offset_of!(ffi::AVRW2RawColorParams, wb_red))?,
            pair(b, codec + offset_of!(ffi::AVRW2RawColorParams, wb_blue))?,
        )
    }

    /// What FFmpeg gives for an RW2 file: the Bayer mosaic and what the side data says about it.
    pub(crate) struct Decoded {
        pub(crate) width: usize,
        pub(crate) height: usize,
        pub(crate) samples: Vec<u16>,
        pub(crate) pattern: Pattern,
        /// FFmpeg's pixel format name, `bayer_rggb16le`…
        pub(crate) native: String,
        /// `None` when FFmpeg gave no (usable) RAW colour parameters.
        pub(crate) params: Option<ColorParams>,
        pub(crate) camera: cfa4_dng::exif::Camera,
    }

    /// Decode the RW2 file at `path` (a checked path) to its Bayer mosaic.
    pub(crate) fn decode_mosaic(path: &Path, name: &str) -> Result<Decoded, Error> {
        let (reader, probed) = backend::probe_file(path)?;
        if probed.container != DEMUXER {
            return Err(Error::Media(format!("{name}: not an RW2 file")));
        }
        let s = probed.streams.iter().find(|s| s.media_type == MediaType::VIDEO).ok_or_else(|| Error::Media(format!("{name}: RW2 without a picture")))?;
        let native = backend::pixel_format_name(s);
        let (Some(pattern), Some(pix_fmt)) = (pattern_for(&native).filter(|_| native.ends_with("16le")), s.format.into_pixel()) else {
            return Err(Error::Media(format!(
                "{name}: FFmpeg {} cannot decode this RW2 file's raw format yet (older Panasonic cameras write a raw format FFmpeg does not decode)",
                crate::min_version_text()
            )));
        };
        let mut reader = reader;
        let mut decoder =
            DecoderBuilder::new(MediaType::VIDEO).with_pix_fmt(pix_fmt).build_from_reader(&reader).map_err(|e| Error::Media(format!("{name}: {e}")))?;
        // rsmedia hands 16-bit Bayer samples over as little-endian byte pairs
        let f = decoder
            .decode::<u8>(&mut reader)
            .map_err(|e| Error::Media(format!("{name}: {e}")))?
            .ok_or_else(|| Error::Media(format!("{name}: FFmpeg decoded no picture")))?;
        let (w, h) = (f.width, f.height);
        let packed = f.data.as_packed().ok_or_else(|| Error::Media(format!("{name}: Bayer frame is not packed")))?;
        let bytes: Vec<u8> = packed.iter().copied().collect();
        if w == 0 || h == 0 || w.checked_mul(h).and_then(|p| p.checked_mul(2)) != Some(bytes.len()) {
            return Err(Error::Media(format!("{name}: Bayer frame of {} bytes for {w}×{h}", bytes.len())));
        }
        let samples: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        drop(bytes);
        let side = |t: ffi::AVFrameSideDataType| f.side_data.iter().find(|d| d.type_ == t).map(|d| d.data.as_slice());
        let params = side(ffi::AV_FRAME_DATA_RAW_COLOR_PARAMS).and_then(parse_color_params);
        let camera = side(ffi::AV_FRAME_DATA_EXIF).and_then(|b| cfa4_dng::exif::camera(b).ok()).unwrap_or_default();
        Ok(Decoded { width: w, height: h, samples, pattern, native, params, camera })
    }

    /// Open the RW2 file at `path` (a checked path) as a developed still.
    pub(crate) fn open(path: &Path, name: &str) -> Result<cfa4_media::SharedSource, Error> {
        let Decoded { width: w, height: h, samples, pattern, native, params, camera } = decode_mosaic(path, name)?;
        let params = params.unwrap_or_else(|| {
            log::warn!("{name}: no RAW colour parameters from FFmpeg; full sample range, no white balance");
            ColorParams { black: 0.0, white: 65_535.0, neutral: [1.0; 3] }
        });
        let model = camera.unique_model();
        let profile = model.as_deref().and_then(|m| find_profile(m, &profile_dirs()));
        let colours = match &profile {
            Some((file, p)) => {
                log::info!("{name}: camera profile {}", file.display());
                format!("camera profile “{}”", p.name.as_deref().unwrap_or("unnamed"))
            }
            None => {
                log::warn!(
                    "{name}: no camera profile for {} in {:?}; colours are approximate",
                    model.as_deref().unwrap_or("an unknown camera"),
                    profile_dirs()
                );
                "no camera profile: approximate colours".into()
            }
        };
        let mosaic = cfa4_dng::Mosaic { width: w, height: h, data: &samples, bits: 16, pattern, black: params.black, white: params.white };
        let d = cfa4_dng::develop_mosaic(&mosaic, params.neutral, profile.as_ref().map(|(_, p)| &p.calibration))
            .map_err(|e| Error::Media(format!("{name}: {e}")))?;
        let (w, h, rgba) = orient(camera.orientation.unwrap_or(1), d.width, d.height, d.rgba);
        let (Ok(w), Ok(h)) = (u32::try_from(w), u32::try_from(h)) else { return Err(Error::Media(format!("{name}: picture too large"))) };
        let label = format!("Bayer 16-bit raw ({native}, FFmpeg), {colours}");
        let file_size = std::fs::metadata(path).ok().map(|m| m.len());
        let still = cfa4_media::still::rw2::camera_raw_still(name, "Panasonic RW2 (FFmpeg)", label, w, h, rgba, file_size);
        Ok(Arc::new(still))
    }
}

#[cfg(feature = "ffmpeg")]
pub(crate) use decode::open;

/// Whether the file at `path` starts like an RW2 file (`IIU\0`).
#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
pub(crate) fn is_rw2(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && cfa4_media::still::rw2::sniff(&head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(pattern_for("bayer_rggb16le").unwrap().colors, vec![0, 1, 1, 2]);
        assert_eq!(pattern_for("bayer_grbg16le").unwrap().colors, vec![1, 0, 2, 1]);
        assert_eq!(pattern_for("bayer_gbrg16le").unwrap().colors, vec![1, 2, 0, 1]);
        assert_eq!(pattern_for("bayer_bggr16le").unwrap().colors, vec![2, 1, 1, 0]);
        assert!(pattern_for("bayer_rgbg16le").is_none());
        assert!(pattern_for("yuv420p").is_none());
        assert!(pattern_for("bayer_").is_none());
    }

    #[test]
    fn orientations() {
        // 3 × 2: pixel value = index
        let src: Vec<f32> = (0..6).flat_map(|i| [i as f32, 0.0, 0.0, 1.0]).collect();
        let red = |v: &[f32]| v.chunks_exact(4).map(|p| p[0] as u8).collect::<Vec<_>>();
        // 0 1 2 / 3 4 5
        assert_eq!(orient(1, 3, 2, src.clone()).2, src);
        assert_eq!(red(&orient(2, 3, 2, src.clone()).2), [2, 1, 0, 5, 4, 3]);
        assert_eq!(red(&orient(3, 3, 2, src.clone()).2), [5, 4, 3, 2, 1, 0]);
        assert_eq!(red(&orient(4, 3, 2, src.clone()).2), [3, 4, 5, 0, 1, 2]);
        let (w, h, v) = orient(6, 3, 2, src.clone());
        // rotated 90° clockwise: 3 0 / 4 1 / 5 2
        assert_eq!((w, h, red(&v)), (2, 3, vec![3, 0, 4, 1, 5, 2]));
        // rotated 90° counter-clockwise: 2 5 / 1 4 / 0 3
        assert_eq!(red(&orient(8, 3, 2, src.clone()).2), [2, 5, 1, 4, 0, 3]);
        assert_eq!(red(&orient(5, 3, 2, src.clone()).2), [0, 3, 1, 4, 2, 5]);
        assert_eq!(red(&orient(7, 3, 2, src.clone()).2), [5, 2, 4, 1, 3, 0]);
        // nonsense orientations and sizes leave the picture alone
        assert_eq!(orient(9, 3, 2, src.clone()).2, src);
        assert_eq!(orient(6, 4, 2, src.clone()).0, 4);
    }

    #[test]
    fn colour_params_are_checked() {
        let p = color_params((2048, 65_535), (65_535, 65_535), (707, 256), (508, 256)).unwrap();
        assert_eq!((p.black, p.white), (2048.0, 65_535.0));
        assert!((p.neutral[0] - 256.0 / 707.0).abs() < 1e-12 && p.neutral[1] == 1.0 && (p.neutral[2] - 256.0 / 508.0).abs() < 1e-12);
        // levels given as codes
        assert_eq!(color_params((512, 1), (4095, 1), (1, 1), (1, 1)).unwrap().white, 4095.0);
        for bad in [
            ((2048, 0), (1, 1), (1, 1), (1, 1)),
            ((70_000, 1), (1, 1), (1, 1), (1, 1)),
            ((1, 1), (1, 2), (1, 1), (1, 1)),
            ((0, 1), (1, 1), (0, 1), (1, 1)),
            ((0, 1), (1, 1), (1, 1), (1000, 1)),
            ((-5, 1), (1, 1), (1, 1), (1, 1)),
        ] {
            assert!(color_params(bad.0, bad.1, bad.2, bad.3).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn profiles_are_found_by_camera_model() {
        use cfa4_dng::tiff::tags;
        use cfa4_dng::writer::{Value, write_profile};
        let dir = std::env::temp_dir().join(format!("cfa4-rw2-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Panasonic DC-GH6")).unwrap();
        let cm = Value::srationals(&[0.6, -0.2, 0.0, -0.4, 1.2, 0.2, -0.1, 0.2, 0.6]);
        let write = |path: &str, model: &str, name: &str| {
            let b = write_profile(
                &[(tags::UNIQUE_CAMERA_MODEL, Value::Ascii(model.into())), (tags::PROFILE_NAME, Value::Ascii(name.into())), (tags::COLOR_MATRIX_1, cm.clone())],
                false,
            );
            std::fs::write(dir.join(path), b).unwrap();
        };
        write("Panasonic DC-GH6/a vivid.dcp", "Panasonic DC-GH6", "Camera Vivid");
        write("Panasonic DC-GH6/b standard.DCP", "Panasonic DC-GH6", "Adobe Standard");
        write("other.dcp", "Panasonic DC-S5M2", "Adobe Standard");
        std::fs::write(dir.join("broken.dcp"), b"IIRC\xff\xff").unwrap();
        let dirs = [dir.clone(), dir.join("missing")];
        let (path, p) = find_profile("panasonic dc-gh6", &dirs).unwrap();
        assert_eq!(p.name.as_deref(), Some("Adobe Standard"));
        assert!(path.ends_with("b standard.DCP"));
        assert_eq!(find_profile("Panasonic DC-S5M2", &dirs).unwrap().1.camera_model.as_deref(), Some("Panasonic DC-S5M2"));
        assert!(find_profile("Panasonic DC-G9M2", &dirs).is_none());
        // only the vivid one: it is used
        std::fs::remove_file(dir.join("Panasonic DC-GH6/b standard.DCP")).unwrap();
        assert_eq!(find_profile("Panasonic DC-GH6", &dirs).unwrap().1.name.as_deref(), Some("Camera Vivid"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Against public RW2 samples (`scripts/dev/fetch-rw2-samples.sh`, CC0, SHA-256 pinned there) and
/// the ffmpeg CLI oracle. A missing sample or oracle skips (`CFA4_REQUIRE_ORACLES=1`: fails).
#[cfg(all(test, feature = "ffmpeg"))]
mod samples {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use cfa4_media::FrameRequest;
    use cfa4_testkit::oracle;

    use super::decode::{Decoded, decode_mosaic};
    use super::*;

    fn sample(name: &str) -> Option<PathBuf> {
        let dir =
            std::env::var_os("CFA4_RW2_SAMPLES").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/rw2"));
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
        oracle::skip("cfa4-ffmpeg rw2", &format!("{} missing (scripts/dev/fetch-rw2-samples.sh)", path.display()));
        None
    }

    fn usable_ffmpeg() -> bool {
        match crate::probe().map(|s| s.refusal()) {
            Ok(None) => true,
            Ok(Some(r)) => {
                oracle::skip("cfa4-ffmpeg rw2", &r);
                false
            }
            Err(e) => {
                oracle::skip("cfa4-ffmpeg rw2", &e.to_string());
                false
            }
        }
    }

    /// The coded (uncropped) picture size, as ffprobe reports it.
    fn probed_size(path: &Path) -> (u32, u32) {
        let fp = oracle::ffprobe().expect("ffprobe next to ffmpeg");
        let out = Command::new(fp)
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
            .arg(path)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let mut it = text.trim().split(',').map(|v| v.parse::<u32>().unwrap());
        (it.next().unwrap(), it.next().unwrap())
    }

    /// The little-endian TIFF IFD 0 entries of an RW2 file: tag → (type, count, value or offset).
    fn ifd0(b: &[u8]) -> std::collections::HashMap<u16, (u16, u32, u32)> {
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let off = u32_at(4) as usize;
        (0..usize::from(u16_at(off)))
            .map(|i| {
                let e = off + 2 + 12 * i;
                let (ty, count) = (u16_at(e + 2), u32_at(e + 4));
                let v = if ty == 3 && count == 1 { u32::from(u16_at(e + 8)) } else { u32_at(e + 8) };
                (u16_at(e), (ty, count, v))
            })
            .collect()
    }

    /// Linear Rec. 709 → sRGB 8-bit, box-averaged down to `ow` × `oh`.
    fn srgb8_downscaled(rgba: &[f32], w: usize, h: usize, ow: usize, oh: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(ow * oh * 3);
        for oy in 0..oh {
            let (y0, y1) = (oy * h / oh, ((oy + 1) * h / oh).max(oy * h / oh + 1));
            for ox in 0..ow {
                let (x0, x1) = (ox * w / ow, ((ox + 1) * w / ow).max(ox * w / ow + 1));
                let mut sum = [0f64; 3];
                for y in y0..y1 {
                    for p in rgba[(y * w + x0) * 4..(y * w + x1) * 4].chunks_exact(4) {
                        (0..3).for_each(|c| sum[c] += f64::from(p[c]));
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as f64;
                for s in sum {
                    let v = (s / n).clamp(0.0, 1.0);
                    let e = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                    out.push((e * 255.0).round() as u8);
                }
            }
        }
        out
    }

    #[test]
    fn modern_cameras_decode_like_the_oracle() {
        if !usable_ffmpeg() {
            return;
        }
        let Some(ff) = oracle::ffmpeg_or_skip("cfa4-ffmpeg rw2") else { return };
        println!("| camera | mosaic | black | white | neutral (R, G, B) | preview | mean ΔE00 | share > 1 |");
        println!("|---|---|---|---|---|---|---|---|");
        for (file, model) in [("DC-GH6.rw2", "Panasonic DC-GH6"), ("DC-S5M2.rw2", "Panasonic DC-S5M2")] {
            let Some(path) = sample(file) else { continue };
            let d: Decoded = decode_mosaic(&path, file).unwrap();
            assert_eq!(d.native, "bayer_rggb16le", "{file}");
            assert_eq!(d.samples.len(), d.width * d.height);

            // 1. the mosaic is bit-exact with the oracle's raw Bayer output. The CLI crops to an
            // aligned window, so compare with its uncropped picture: the mosaic must be a window of
            // it at an even offset (the CFA phase is kept)
            let out = Command::new(&ff)
                .args(["-hide_banner", "-loglevel", "error", "-apply_cropping", "none", "-i"])
                .arg(&path)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", &d.native, "-"])
                .output()
                .unwrap();
            assert!(out.status.success(), "{file}: {}", String::from_utf8_lossy(&out.stderr));
            let full: Vec<u16> = out.stdout.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let (fw, fh) = {
                let s = probed_size(&path);
                (s.0 as usize, s.1 as usize)
            };
            assert_eq!(full.len(), fw * fh, "{file}: oracle frame size");
            assert!(d.width <= fw && d.height <= fh);
            let window = |dx: usize, dy: usize| (0..d.height).all(|y| full[(y + dy) * fw + dx..][..d.width] == d.samples[y * d.width..][..d.width]);
            let at = (0..=fh - d.height).step_by(2).flat_map(|dy| (0..=fw - d.width).step_by(2).map(move |dx| (dx, dy))).find(|&(dx, dy)| window(dx, dy));
            assert!(at.is_some(), "{file}: the mosaic is no even-offset window of the ffmpeg CLI's {fw}×{fh} picture");

            // 2. levels and white balance agree with the RW2 file's own tags (Panasonic IFD 0:
            // BlackLevelRed 0x1C, LinearityLimitRed 0x0E, WBRedLevel 0x24, WBGreenLevel 0x25,
            // WBBlueLevel 0x26)
            let bytes = std::fs::read(&path).unwrap();
            let tags = ifd0(&bytes);
            let tag = |t: u16| f64::from(tags[&t].2);
            let p = d.params.unwrap_or_else(|| panic!("{file}: no RAW colour parameters"));
            let rel = |a: f64, b: f64| (a - b).abs() / b;
            assert!(rel(p.black / p.white, tag(0x1C) / tag(0x0E)) < 0.01, "{file}: black {p:?}");
            assert!(rel(p.neutral[0], tag(0x25) / tag(0x24)) < 1e-3 && rel(p.neutral[2], tag(0x25) / tag(0x26)) < 1e-3, "{file}: {p:?}");
            assert_eq!(p.neutral[1], 1.0);
            assert_eq!(d.camera.unique_model().as_deref(), Some(model));

            // 3. the developed picture against the embedded JPEG preview (JpgFromRaw, 0x2E): a
            // printed table, no threshold (the camera's tone curve and look are not reproduced)
            let m =
                cfa4_dng::Mosaic { width: d.width, height: d.height, data: &d.samples, bits: 16, pattern: d.pattern.clone(), black: p.black, white: p.white };
            let dev = cfa4_dng::develop_mosaic(&m, p.neutral, None).unwrap();
            let (w, h, rgba) = orient(d.camera.orientation.unwrap_or(1), dev.width, dev.height, dev.rgba);
            let (_, len, at) = tags[&0x2E];
            let jpeg = &bytes[at as usize..at as usize + len as usize];
            let preview = cfa4_media::still::decode_raster("preview.jpg", jpeg).unwrap();
            let (pw, ph) = (preview.width as usize, preview.height as usize);
            let aspect = |a: usize, b: usize| a as f64 / b as f64;
            let de = if rel(aspect(w, h), aspect(pw, ph)) < 0.02 {
                let ours = srgb8_downscaled(&rgba, w, h, pw, ph);
                let theirs: Vec<u8> = preview.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
                let s = cfa4_metrics::delta_e_srgb8(&theirs, &ours, pw, ph, 3).unwrap();
                format!("{:.2} | {:.0} %", s.mean, s.above_jnd * 100.0)
            } else {
                "aspect differs | –".into()
            };
            println!(
                "| {model} | {}×{} | {:.0} | {:.0} | {:.3}, 1, {:.3} | {pw}×{ph} | {de} |",
                d.width, d.height, p.black, p.white, p.neutral[0], p.neutral[2]
            );

            // 4. through the backend's entry point: a still of the developed size
            let src = crate::open(&path).unwrap();
            let info = src.info();
            let v = info.video.as_ref().unwrap();
            assert_eq!((v.width as usize, v.height as usize), (w, h), "{file}");
            let f = src.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)).unwrap();
            assert_eq!((f.width as usize, f.height as usize), (w, h));
        }
    }

    #[test]
    fn older_raw_formats_are_a_clear_error() {
        if !usable_ffmpeg() {
            return;
        }
        let Some(path) = sample("DMC-LX7.rw2") else { return };
        let e = crate::open(&path).err().expect("FFmpeg decoded a RawFormat 4 file: update the docs and this test").to_string();
        assert!(e.contains("cannot decode this RW2 file's raw format"), "{e}");
    }

    #[test]
    fn damaged_files_do_not_panic() {
        if !usable_ffmpeg() {
            return;
        }
        let Some(path) = sample("DC-S5M2.rw2") else { return };
        let bytes = std::fs::read(&path).unwrap();
        let dir = std::env::temp_dir().join(format!("cfa4-rw2-damaged-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cases: Vec<(&str, Vec<u8>)> = vec![
            ("header-only", bytes[..8].to_vec()),
            ("first-kilobyte", bytes[..1024].to_vec()),
            ("half", bytes[..bytes.len() / 2].to_vec()),
            ("no-tail", bytes[..bytes.len() - 4096].to_vec()),
        ];
        let mut noisy = bytes.clone();
        let mut s = 0x1234_5678u32;
        for _ in 0..2000 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let at = 8 + (s as usize % (noisy.len() - 8));
            noisy[at] ^= (s >> 24) as u8;
        }
        cases.push(("noisy", noisy));
        let mut ifd = bytes.clone();
        ifd[8..64].fill(0xFF);
        cases.push(("broken-ifd", ifd));
        for (name, b) in cases {
            let p = dir.join(format!("{name}.rw2"));
            std::fs::write(&p, &b).unwrap();
            // an error or a picture; never a panic (the call is guarded, so check both ways)
            let r = std::panic::catch_unwind(|| crate::open(&p).map(|s| s.info().video.as_ref().map(|v| (v.width, v.height))));
            let r = r.unwrap_or_else(|_| panic!("{name}: panicked"));
            assert!(!matches!(r, Err(crate::Error::Internal(_))), "{name}: {r:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
