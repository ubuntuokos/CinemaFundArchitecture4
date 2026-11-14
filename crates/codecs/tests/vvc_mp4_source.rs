//! VVC in MP4 through the media source (ADR 0013, VVC-02): with the VVdeC helper registered as
//! the decoder, frames requested by time, in any order (seeks back and across random-access
//! units), are FFmpeg's pictures. A test binary of its own, because it registers a decoder
//! factory for the whole process.
//!
//! Needs `scripts/dev/vvc-decoder.py build` (or `CFA4_VVC_DECODER`) and an FFmpeg with
//! `libvvenc`; otherwise SKIPPED, also under `CFA4_REQUIRE_ORACLES` (restricted components).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use cfa4_codecs::video::VideoDecoder;
use cfa4_codecs::vvc_external::{self, Mp4VvcDecoder};
use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_testkit::fixtures::generate;
use cfa4_testkit::{ffmpeg_has_encoder, ffmpeg_or_skip, fixtures_dir};

const FRAMES: usize = 24;
const WIDTH: u32 = 330;
const HEIGHT: u32 = 202;

static HELPER: OnceLock<PathBuf> = OnceLock::new();

/// The helper: `CFA4_VVC_DECODER`, or the default build when its VVdeC stamp matches the pin.
fn helper() -> Option<PathBuf> {
    if let Some(p) = vvc_external::configured_helper() {
        return Some(p);
    }
    let root = cfa4_testkit::workspace_root();
    let pin = std::fs::read_to_string(root.join("packaging/vvc-decode/vvdec.json")).ok()?;
    let pin: serde_json::Value = serde_json::from_str(&pin).ok()?;
    let commit = pin.get("commit")?.as_str()?.to_string();
    let prefix = root.join("target/vvc-decoder");
    let stamp = std::fs::read_to_string(prefix.join("VVDEC_COMMIT")).ok()?;
    let exe = prefix.join("bin/cfa4-vvc-decode");
    (stamp.trim() == commit && exe.is_file()).then_some(exe)
}

/// The factory under test: [`vvc_external::vvc_factory`] with the helper found above.
fn factory(e: &cfa4_isobmff::SampleEntry) -> Option<cfa4_codecs::Result<Box<dyn VideoDecoder>>> {
    let cfa4_isobmff::CodecConfig::Vvc(cfg) = &e.codec else { return None };
    Some(Mp4VvcDecoder::new(HELPER.get()?, 0, cfg).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

fn mp4_fixture(ff: &Path) -> PathBuf {
    let out = fixtures_dir("vvc").join(format!("testsrc-{WIDTH}x{HEIGHT}-{FRAMES}f-ip8.mp4"));
    generate(&out, |tmp| {
        Command::new(ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=24"))
            .args(["-frames:v", &FRAMES.to_string(), "-pix_fmt", "yuv420p10le", "-c:v", "libvvenc", "-preset", "faster", "-qp", "32"])
            .args(["-vvenc-params", "IntraPeriod=8", "-tag:v", "vvc1", "-f", "mp4"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
    .expect("ffmpeg with libvvenc failed to encode the MP4 fixture")
}

/// FFmpeg's decode, frames in presentation order, planes packed two bytes little-endian.
fn ffmpeg_frames(ff: &Path, path: &Path) -> Vec<Vec<u8>> {
    let out = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-c:v", "vvc", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p10le", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg could not decode the fixture: {}", String::from_utf8_lossy(&out.stderr));
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let size = (w * h + 2 * w.div_ceil(2) * h.div_ceil(2)) * 2;
    out.stdout.chunks(size).map(<[u8]>::to_vec).collect()
}

#[test]
fn mp4_source_frames_by_time_match_ffmpeg() {
    let ctx = "vvc_mp4_source::mp4_source_frames_by_time_match_ffmpeg";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some(ff) = ffmpeg_or_skip(ctx) else { return };
    if !ffmpeg_has_encoder(&ff, "libvvenc") {
        return eprintln!("SKIPPED ({ctx}): this ffmpeg has no libvvenc (restricted component)");
    }
    let _ = HELPER.set(helper);
    cfa4_codecs::register_video_decoder(factory);

    let mp4 = mp4_fixture(&ff);
    let want = ffmpeg_frames(&ff, &mp4);
    assert_eq!(want.len(), FRAMES);
    let src = cfa4_codecs::open_bytes("vvc.mp4", std::fs::read(&mp4).unwrap().into()).unwrap();
    let rate = src.info().video.as_ref().unwrap().frame_rate;
    // forward, a seek back, a jump over a random-access unit, the last frame, the first
    for i in [0usize, 1, 2, 9, 3, 17, 16, 23, 8, 0] {
        let f = src.video_frame(FrameRequest::full(rate.tick_of(i as i64))).unwrap();
        let got: Vec<u8> = match &f.data {
            PixelData::Yuv16 { planes, bits: 10, .. } => planes.iter().flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes())).collect(),
            _ => panic!("frame {i}: expected 10-bit pixels"),
        };
        assert!(got == want[i], "frame {i} differs from FFmpeg's");
    }
}
