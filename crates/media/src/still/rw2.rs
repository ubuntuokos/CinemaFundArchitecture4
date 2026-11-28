//! Panasonic RW2 camera raw stills. CFA4 has no RW2 decoder of its own: the FFmpeg backend
//! (`cfa4-ffmpeg`, F2d of the FFmpeg migration plan) decodes the raw mosaic and develops it with the
//! DNG pipeline of `cfa4-dng`. Without that backend an RW2 file is refused with a message that
//! says so; the media fallback then hands it to FFmpeg when FFmpeg is registered.

use cfa4_color::{ColorInfo, Transfer};
use cfa4_frame::VideoFrame;
use cfa4_time::FrameRate;

use super::{DEFAULT_STILL_DURATION, StillSource};
use crate::{MediaError, MediaInfo, MediaKind, VideoStreamInfo};

/// Whether `b` starts like an RW2 file: a little-endian TIFF-style header with the magic 0x55.
pub fn sniff(b: &[u8]) -> bool {
    b.starts_with(b"IIU\0")
}

/// The error for an RW2 file the built-in openers cannot read.
pub fn needs_ffmpeg(name: &str) -> MediaError {
    MediaError::Unsupported(format!(
        "{name}: Panasonic RW2 camera raw is decoded by the FFmpeg backend (Settings ▸ Media & Playback ▸ Media Backends); it is not available in this build, it is switched off, or it cannot decode this file (FFmpeg does not decode the raw format of older Panasonic cameras such as the GH2, GX9 or LX7)"
    ))
}

/// The colour of developed camera raw frames: scene-linear light, Rec. 709 primaries.
pub const LINEAR_709: ColorInfo = ColorInfo { transfer: Transfer::Linear, ..ColorInfo::SRGB_FULL };

/// A developed camera raw picture as a still: `rgba` holds `w` × `h` opaque scene-linear
/// Rec. 709 floats. `codec` and `pixel_format` are the media info's labels.
pub fn camera_raw_still(name: &str, codec: &str, pixel_format: String, w: u32, h: u32, rgba: Vec<f32>, file_size: Option<u64>) -> StillSource {
    let mut frame = VideoFrame::rgba_f32(w, h, rgba);
    frame.color = LINEAR_709;
    let info = MediaInfo {
        name: name.to_string(),
        kind: MediaKind::Still,
        duration: DEFAULT_STILL_DURATION,
        video: Some(VideoStreamInfo {
            width: w,
            height: h,
            frame_rate: FrameRate::FPS_29_97,
            par: (1, 1),
            codec: codec.to_string(),
            pixel_format,
            color: LINEAR_709,
            has_alpha: false,
            bitrate: None,
            hdr: None,
        }),
        audio_streams: Vec::new(),
        container: "Image".into(),
        start_timecode: None,
        file_size,
    };
    StillSource::from_frame(info, frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaSource;

    #[test]
    fn sniff_and_refusal() {
        assert!(sniff(b"IIU\0\x08\0\0\0"));
        assert!(!sniff(b"II*\0\x08\0\0\0"));
        assert!(!sniff(b"IIU"));
        let e = crate::open_bytes("a.rw2", (*b"IIU\0\x18\0\0\0rest-of-file").into(), &[]).err().expect("refused");
        assert!(matches!(&e, MediaError::Unsupported(m) if m.contains("FFmpeg backend")), "{e}");
        assert!(crate::STILL_EXTENSIONS.contains(&"rw2"));
    }

    #[test]
    fn camera_raw_still_is_linear_float() {
        let s = camera_raw_still("a.rw2", "Panasonic RW2", "Bayer 16-bit raw".into(), 2, 1, vec![0.5, 0.25, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0], Some(9));
        let v = s.info().video.as_ref().unwrap();
        assert_eq!((v.width, v.height, v.codec.as_str(), v.color.transfer), (2, 1, "Panasonic RW2", Transfer::Linear));
        assert_eq!(s.info().kind, MediaKind::Still);
        let f = s.video_frame(crate::FrameRequest::full(cfa4_time::Tick::ZERO)).unwrap();
        assert_eq!(f.color, LINEAR_709);
    }
}
