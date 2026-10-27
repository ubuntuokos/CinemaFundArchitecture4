use std::path::PathBuf;
use std::process::Command;

use super::*;
use cfa4_frame::PixelData;

fn cjxl() -> Option<PathBuf> {
    let found = cfa4_testkit::oracle::find_tool("cjxl");
    if found.is_none() {
        cfa4_testkit::oracle::skip("media jxl", "cjxl not found (Ubuntu package libjxl-tools)");
    }
    found
}

/// Encode `png` (already written) losslessly with `cjxl`.
fn encode(cjxl: &PathBuf, png: &std::path::Path) -> Vec<u8> {
    let out = png.with_extension("jxl");
    let r = Command::new(cjxl).arg(png).arg(&out).args(["-d", "0", "-e", "3", "--quiet"]).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    std::fs::read(out).unwrap()
}

fn rgba8(img: &image::RgbaImage, name: &str, cjxl: &PathBuf) -> Vec<u8> {
    let png = cfa4_testkit::fixtures::fixtures_dir("media/jxl").join(format!("{name}.png"));
    img.save(&png).unwrap();
    encode(cjxl, &png)
}

fn pattern(alpha: bool) -> image::RgbaImage {
    image::RgbaImage::from_fn(33, 21, |x, y| image::Rgba([(x * 7) as u8, (y * 11) as u8, ((x ^ y) * 5) as u8, if alpha { (x * 8) as u8 } else { 255 }]))
}

#[test]
fn lossless_8bit_rgb_and_rgba_are_exact() {
    let Some(cjxl) = cjxl() else { return };
    for alpha in [false, true] {
        let img = pattern(alpha);
        let bytes = rgba8(&img, if alpha { "rgba8" } else { "rgb8" }, &cjxl);
        assert!(sniff(&bytes));
        let d = decode("t.jxl", &bytes).unwrap();
        assert_eq!(d.has_alpha, alpha);
        assert_eq!((d.frame.width, d.frame.height), (33, 21));
        let PixelData::Rgba8(px) = &d.frame.data else { panic!("8-bit sRGB should stay RGBA8") };
        assert_eq!(px.as_slice(), img.as_raw().as_slice());
        assert_eq!(d.pixel_format, if alpha { "RGBA 8-bit" } else { "RGB 8-bit" });
    }
}

#[test]
fn grey_expands_to_rgb() {
    let Some(cjxl) = cjxl() else { return };
    let img = image::GrayImage::from_fn(16, 9, |x, y| image::Luma([(x * 13 + y * 3) as u8]));
    let png = cfa4_testkit::fixtures::fixtures_dir("media/jxl").join("grey8.png");
    img.save(&png).unwrap();
    let d = decode("g.jxl", &encode(&cjxl, &png)).unwrap();
    let PixelData::Rgba8(px) = &d.frame.data else { panic!("8-bit grey should stay RGBA8") };
    for (p, &g) in px.chunks_exact(4).zip(img.as_raw()) {
        assert_eq!(p, [g, g, g, 255]);
    }
    assert!(d.pixel_format.starts_with("Grey"));
}

#[test]
fn sixteen_bit_becomes_linear_float() {
    let Some(cjxl) = cjxl() else { return };
    let img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(17, 5, |x, y| image::Rgb([(x * 3855) as u16, (y * 16383) as u16, 65535]));
    let png = cfa4_testkit::fixtures::fixtures_dir("media/jxl").join("rgb16.png");
    img.save(&png).unwrap();
    let d = decode("h.jxl", &encode(&cjxl, &png)).unwrap();
    assert_eq!(d.frame.color.transfer, Transfer::Linear);
    let PixelData::RgbaF32(px) = &d.frame.data else { panic!("16-bit should be float") };
    for (p, s) in px.chunks_exact(4).zip(img.as_raw().chunks_exact(3)) {
        for c in 0..3 {
            let want = cfa4_color::to_linear(f32::from(s[c]) / 65535.0, Transfer::Srgb);
            assert!((p[c] - want).abs() < 1e-4, "{} vs {want}", p[c]);
        }
        assert_eq!(p[3], 1.0);
    }
}

#[test]
fn opens_as_a_still() {
    let Some(cjxl) = cjxl() else { return };
    let bytes = rgba8(&pattern(false), "open", &cjxl);
    let src = crate::open_bytes("x.jxl", bytes.into(), &[]).unwrap();
    let v = src.info().video.as_ref().unwrap();
    assert_eq!(v.codec, "JPEG XL");
    assert_eq!((v.width, v.height), (33, 21));
    assert_eq!(src.info().kind, MediaKind::Still);
}

#[test]
fn corrupt_files_are_errors() {
    assert!(decode("x.jxl", &[0xFF, 0x0A]).is_err());
    assert!(decode("x.jxl", &[0xFF, 0x0A, 0xFF, 0xFF, 0, 1, 2, 3, 4]).is_err());
    assert!(!sniff(b"\x89PNG"));
}
