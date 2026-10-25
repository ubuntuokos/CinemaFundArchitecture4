//! Native Linux AMD64 libheif integration: HEIC/HEIF import, decode and file recognition.
//! Run with libheif-examples, libheif-plugin-libde265 and libheif-plugin-x265 installed.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::sync::Arc;

use cfa4_media::{MediaKind, still::heif};

fn make_png() -> Vec<u8> {
    let (w, h) = (64u32, 64u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[(x * 4) as u8, (y * 4) as u8, 83, 255]);
        }
    }
    let mut png = Vec::new();
    image::ImageEncoder::write_image(image::codecs::png::PngEncoder::new(&mut png), &rgba, w, h, image::ExtendedColorType::Rgba8).unwrap();
    png
}

#[test]
fn heif_and_heic_import_round_trip_through_local_libheif() {
    // The HEIF handler must run before generic ISO-BMFF video openers.
    fn rejecting_mp4_opener(_name: &str, _bytes: Arc<[u8]>) -> Option<cfa4_media::Result<cfa4_media::SharedSource>> {
        Some(Err(cfa4_media::MediaError::Unsupported("wrong MP4 opener".into())))
    }
    let heif_bytes = heif::encode_png(&make_png()).expect("libheif HEVC encoder must be installed in CI");
    assert!(heif::sniff(&heif_bytes));
    for ext in ["heic", "heif", "hif"] {
        let name = format!("phone_photo.{ext}");
        assert!(cfa4_media::is_importable(std::path::Path::new(&name)));
        let src = cfa4_media::open_bytes(&name, Arc::from(heif_bytes.clone()), &[rejecting_mp4_opener])
            .expect("HEIF still must be handled before video container openers");
        assert_eq!(src.info().kind, MediaKind::Still);
        let meta = src.info().video.as_ref().expect("still video info");
        assert_eq!((meta.width, meta.height), (64, 64));
        assert_eq!(meta.codec, ext.to_ascii_uppercase());
        assert_eq!(src.info().file_size, Some(heif_bytes.len() as u64));
    }
}

#[test]
fn truncated_and_false_heif_files_fail_cleanly() {
    let valid = heif::encode_png(&make_png()).expect("libheif encoder");
    let broken = &valid[..valid.len().min(64)];
    assert!(heif::decode("broken.heic", broken).is_err());
    assert!(!heif::sniff(b"not a HEIF"));
}
