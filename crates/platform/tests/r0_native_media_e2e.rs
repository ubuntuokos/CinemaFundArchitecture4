//! CFA4 R0 #16 – strict Linux AMD64 app-level media oracles.
//!
//! Ignored by generic hosted CI: a *physical* NVIDIA GPU and the locally
//! recorded FFmpeg #14 H.264/HEVC MP4 samples are mandatory. The audit runner
//! uses --ignored and insists on exactly two passed tests, no SKIPPED results.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfa4_codecs::hw::{hw_stats, set_hardware_decoding};
use cfa4_export::{ExportSettings, Format, HardwareEncoding, Progress, export, hw_encode_stats};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{DemoScene, FrameRequest, Generator, MediaSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::{FrameRate, Tick, TimeRange};

fn samples_dir() -> PathBuf {
    let dir =
        std::env::var_os("CFA4_MEDIA_FIXTURE_DIR").map(PathBuf::from).expect("CFA4_MEDIA_FIXTURE_DIR must point to the measured FFmpeg #14 evidence folder");
    assert!(dir.is_dir(), "missing source evidence folder {}", dir.display());
    dir
}

fn first_eight_frames(path: &Path) -> Vec<Arc<cfa4_frame::VideoFrame>> {
    let bytes: Arc<[u8]> = std::fs::read(path).expect("read encoded export").into();
    let source = cfa4_codecs::open_bytes(&path.to_string_lossy(), bytes).expect("CFA4 imports its own MP4 export");
    let rate = source.info().frame_rate();
    (0..8).map(|i| source.video_frame(FrameRequest::full(rate.tick_of(i))).expect("CFA4 decodes exported frame")).collect()
}

#[test]
#[ignore = "requires physical NVIDIA GPU and CFA4_MEDIA_FIXTURE_DIR from FFmpeg #14"]
fn physical_nvdec_h264_hevc_reimport_matches_cfa4_software() {
    cfa4_platform::register();
    set_hardware_decoding(true);
    let dir = samples_dir();
    for name in ["h264.mp4", "hevc.mp4"] {
        let path = dir.join(name);
        assert!(path.is_file(), "missing {} – rerun FFmpeg #14 only if evidence is gone", path.display());
        let s = common::read_stream(&path);
        let before = hw_stats();
        let mut hw = cfa4_platform::nvdec_factory(&s.entry)
            .unwrap_or_else(|| panic!("{name}: native CFA4 NVDEC was declined"))
            .unwrap_or_else(|e| panic!("{name}: cannot open native CFA4 NVDEC: {e}"));
        let decoded_hw = common::decode_all(hw.as_mut(), &s.samples);
        let after = hw_stats();
        assert!(after.sessions > before.sessions, "{name}: hardware decoder session did not start");
        assert!(after.frames > before.frames, "{name}: no actual NVDEC frames were produced");
        assert_eq!(decoded_hw.len(), 8, "{name}: expected all eight reference frames");
        let mut software = cfa4_codecs::software_video_decoder(&s.entry).expect("software reference decoder");
        let decoded_sw = common::decode_all(software.as_mut(), &s.samples);
        common::assert_same(name, &decoded_hw, &decoded_sw);
    }
    set_hardware_decoding(true);
}

fn project_8_frames() -> (Arc<Project>, cfa4_project::ItemId, SourceMap) {
    let (w, h) = (640, 360);
    let rate = FrameRate::FPS_24;
    let g = GeneratorSource::new(Generator::Demo(DemoScene::OceanSunset), w, h, rate, rate.tick_of(8));
    let info = g.info().clone();
    let mut p = Project::new("R0 Native Export");
    let clip = p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(g.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    let seq = p.new_sequence("R0 Export", SequenceSettings { width: w, height: h, frame_rate: rate, ..Default::default() }, 1, 1, None);
    let timeline = p.make_track_item(clip, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, rate.tick_of(8)), rate).expect("place generator");
    p.sequence_mut(seq).expect("active sequence").video_tracks[0].items.push(timeline);
    let mut sources = SourceMap::default();
    sources.0.insert(clip, Arc::new(g));
    (Arc::new(p), seq, sources)
}

#[test]
#[ignore = "requires physical NVIDIA GPU; never accepts software fallback as hardware PASS"]
fn physical_nvenc_h264_and_hevc_export_roundtrip() {
    cfa4_platform::register();
    assert!(cfa4_export::available(Format::Hevc), "CFA4 Linux HEVC NVENC format is not available");
    let (project, seq, sources) = project_8_frames();
    let outdir =
        std::env::var_os("CFA4_NATIVE_E2E_OUTPUT_DIR").map(PathBuf::from).expect("CFA4_NATIVE_E2E_OUTPUT_DIR must identify an audit evidence directory");
    std::fs::create_dir_all(&outdir).expect("create audit output directory");
    for (codec, format) in [("h264", Format::H264), ("hevc", Format::Hevc)] {
        let path = outdir.join(format!("cfa4-native-{codec}.mp4"));
        let settings = ExportSettings {
            format,
            path: path.to_string_lossy().into_owned(),
            include_audio: false,
            hardware_encoding: HardwareEncoding::Auto,
            bitrate_kbps: 4_000,
            ..Default::default()
        };
        let before = hw_encode_stats();
        let report = export(&project, seq, &settings, &sources, &Progress::default()).unwrap_or_else(|e| panic!("{codec}: real CFA4 export failed: {e}"));
        let after = hw_encode_stats();
        assert_eq!(report.frames, 8, "{codec}: exported frame count");
        assert!(after.sessions > before.sessions, "{codec}: NVENC session did not start");
        assert_eq!(after.frames - before.frames, 8, "{codec}: the export fell back to software or lost frames");
        assert_eq!(first_eight_frames(&path).len(), 8, "{codec}: CFA4 cannot reimport its export");
        let ffprobe = std::env::var_os("CFA4_FFPROBE_BIN").unwrap_or_else(|| "ffprobe".into());
        let output = std::process::Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height,nb_read_frames",
                "-of",
                "default=nw=1",
                &path.to_string_lossy(),
            ])
            .output()
            .expect("ffprobe is required on the local audit machine");
        assert!(output.status.success(), "{codec}: ffprobe: {}", String::from_utf8_lossy(&output.stderr));
        let detail = String::from_utf8_lossy(&output.stdout);
        for marker in [format!("codec_name={codec}"), "width=640".into(), "height=360".into(), "nb_read_frames=8".into()] {
            assert!(detail.contains(&marker), "{codec}: missing {marker}; ffprobe output: {detail}");
        }
    }
}
