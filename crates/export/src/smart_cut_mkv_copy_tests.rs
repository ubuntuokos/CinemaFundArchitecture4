//! R2-04 of `CFA4-SMART-CUT-RENDER-R2`: a GOP-boundary Smart Cut from Matroska into Matroska /
//! WebM, with the audio copied packet for packet.
//!
//! FFmpeg (the oracle) makes the sources: a 48-picture, 25 fps clip with a 2 s tone in the audio
//! codec under test. Each cut is checked four ways:
//! - [`verify_mkv_copy`] demuxes it again: every block keeps its source payload and time.
//! - FFmpeg's framemd5 of the cut equals that of the cut pictures of the source.
//! - The cut audio decoded by FFmpeg equals the source audio decoded from the same instant:
//!   within rounding for the lossy codecs once the pre-roll frame is past, bit for bit for TrueHD.
//! - The A/V offset is the source's.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_smart_edit::capabilities::{Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::copy_planner::{CutMode, PlanRefusal, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::media_index::{MediaIndex, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::CopyGuarantee;
use cfa4_smart_edit::timestamp::MediaTime;
use cfa4_testkit::oracle::{ffmpeg_has_encoder, ffmpeg_or_skip, skip};

use crate::mkv_source::{MkvVideo, index_mkv_video};
use crate::smart_cut::SmartCutError;
use crate::smart_cut_mkv::{MkvAudio, MkvCopyReport, MkvCopySource, execute_mkv_copy, mkv_cut_interval, plan_mkv_audio, verify_mkv_copy};

const STRICT: Policy = Policy { allow_experimental: false };
const RATE: u32 = 48_000;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/smart-cut-mkv-copy").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// One audio track of a source: FFmpeg encoder, channel layout and extra options.
struct Audio<'a> {
    encoder: &'a str,
    layout: &'a str,
}

/// The source `name/source.<ext>`: the clip in `video` (FFmpeg encoder and options) with a 2 s
/// tone in each of `audio`.
fn source(name: &str, video: &[&str], audio: &[Audio<'_>], ext: &str) -> Option<PathBuf> {
    let context = format!("Smart Cut into Matroska {name}");
    let ff = ffmpeg_or_skip(&context)?;
    for e in std::iter::once(video[0]).chain(audio.iter().map(|a| a.encoder)) {
        if !ffmpeg_has_encoder(&ff, e) {
            skip(&context, &format!("ffmpeg has no {e} encoder"));
            return None;
        }
    }
    let path = dir(name).join(format!("source.{ext}"));
    let mut c = Command::new(&ff);
    c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=128x96:rate=25"]);
    for a in audio {
        // TrueHD restart points fall on the 40 ms picture grid: start the tone 5 ms late so the
        // cut lies between two of them
        if a.encoder == "truehd" {
            c.args(["-itsoffset", "0.005"]);
        }
        c.args(["-f", "lavfi", "-i", &format!("sine=frequency=440:sample_rate={RATE}:duration=2,aformat=channel_layouts={}", a.layout)]);
    }
    c.args(["-map", "0:v", "-frames:v", "48", "-c:v"]).args(video);
    for (k, a) in audio.iter().enumerate() {
        c.args(["-map", &format!("{}:a", k + 1), &format!("-c:a:{k}"), a.encoder]);
    }
    let o = c.args(["-strict", "-2"]).arg(&path).output().unwrap();
    assert!(o.status.success(), "{context}: {}", String::from_utf8_lossy(&o.stderr));
    Some(path)
}

/// A GOP-boundary plan of the pictures from presentation position `from` to `to`, moved to the
/// nearest safe points.
fn plan(stream: &StreamIndex, source: Container, output: Container, from: u64, to: u64) -> SmartCutPlan {
    let pts = |p: u64| {
        let f = stream.frames.iter().find(|f| f.presentation_index == Some(p)).unwrap();
        MediaTime::new(f.pts.unwrap(), stream.timebase).to_tick().unwrap()
    };
    let media = MediaIndex::new("source", SourceFingerprint { size: 1, modified_ns: None, content_id: None }, vec![stream.clone()]);
    let mut req = SmartCutRequest {
        stream_id: stream.stream_id,
        in_point: pts(from),
        out_point: pts(to),
        mode: CutMode::GopBoundary,
        guarantee: CopyGuarantee::BitExact,
        source_container: source,
        output_container: output,
        format: Some(StreamFormat { profile: "main".into(), chroma: "4:2:0".into(), bit_depth: 8 }),
    };
    match plan_smart_cut(&Registry::native(), &media, &req, STRICT) {
        Err(PlanRefusal::NotAtSafePoints { safe, .. }) => {
            let (i, o) = (safe.in_after.or(safe.in_before).unwrap(), safe.out_after.or(safe.out_before).unwrap());
            req.in_point = i.time.to_tick().unwrap();
            req.out_point = o.time.to_tick().unwrap();
            plan_smart_cut(&Registry::native(), &media, &req, STRICT).unwrap()
        }
        p => p.unwrap(),
    }
}

struct Cut {
    bytes: Vec<u8>,
    file: cfa4_matroska::MkvFile,
    video: MkvVideo,
    plan: SmartCutPlan,
    audio: MkvAudio,
    report: MkvCopyReport,
    output: Vec<u8>,
}

fn cut_bytes(bytes: Vec<u8>, output: Container, from: u64, to: u64) -> Result<Cut, SmartCutError> {
    let file = cfa4_matroska::open(bytes.as_slice()).unwrap();
    let video = index_mkv_video(&file, bytes.as_slice()).unwrap();
    let source_container = crate::mkv_source::container(&file);
    let plan = plan(&video.index, source_container, output, from, to);
    assert!(plan.executable, "{:?}", plan.blockers);
    let src = MkvCopySource { file: &file, bytes: bytes.as_slice(), video: &video };
    let (start_ns, end_ns) = mkv_cut_interval(&plan.plan, &video.index, &src)?;
    let audio = plan_mkv_audio(&file, bytes.as_slice(), output, start_ns, end_ns)?;
    let (out, report) = execute_mkv_copy(&plan.plan, &video.index, &src, &audio, Cursor::new(Vec::new()))?;
    let output = out.into_inner();
    verify_mkv_copy(&plan.plan, &video.index, &src, &audio, &report, output.as_slice())?;
    Ok(Cut { bytes, file, video, plan, audio, report, output })
}

/// FFmpeg's framemd5 hashes of the video, in presentation order.
fn framemd5(ff: &Path, file: &Path) -> Vec<String> {
    let o = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-map", "0:v:0", "-f", "framemd5", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_string())).collect()
}

/// FFmpeg's decode of audio stream `k` as interleaved 32-bit float samples, from the stream's
/// first timestamp (no padding to the file start).
fn decode(ff: &Path, file: &Path, k: usize) -> Vec<f32> {
    let o = Command::new(ff).args(["-v", "error", "-copyts", "-i"]).arg(file).args(["-map", &format!("0:a:{k}"), "-f", "f32le", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// Cut 12–36 of `name` into `output` and check it; `expect` names the copied audio codecs, whether
/// they decode bit for bit (TrueHD) and how many samples one of their frames holds.
fn check(name: &str, video: &[&str], audio: &[Audio<'_>], ext: &str, output: Container, expect: &[(&str, bool, usize)]) -> Option<Cut> {
    let path = source(name, video, audio, ext)?;
    let c = cut_bytes(std::fs::read(&path).unwrap(), output, 12, 36).unwrap_or_else(|e| panic!("{name}: {e}"));
    let codecs: Vec<&str> = c.audio.tracks.iter().map(|t| t.codec.as_str()).collect();
    assert_eq!(codecs, expect.iter().map(|e| e.0).collect::<Vec<_>>(), "{name}: {:?}", c.audio.dropped);
    assert_eq!(c.report.frames, c.plan.plan.copied_frames(), "{name}");
    for t in &c.audio.tracks {
        // the audio starts at most one access unit before the picture (TrueHD: one restart interval)
        assert!(t.lead_ns <= 0 && t.lead_ns > -120_000_000, "{name}: {t:?}");
    }
    let ff = ffmpeg_or_skip(name)?;
    let cut_path = dir(name).join(format!("cut.{}", if output == Container::WebM { "webm" } else { "mkv" }));
    std::fs::write(&cut_path, &c.output).unwrap();
    let (want, got) = (framemd5(&ff, &path), framemd5(&ff, &cut_path));
    let first = c.plan.plan.segments[0].presentation.first as usize;
    assert_eq!(got, want[first..first + got.len()].to_vec(), "{name}: the cut pictures decode as the source's");
    assert_eq!(got.len() as u64, c.plan.plan.copied_frames(), "{name}");
    for (k, (t, &(_, exact, frame))) in c.audio.tracks.iter().zip(expect).enumerate() {
        let channels = c.file.tracks[t.track].audio.as_ref().unwrap().channels as usize;
        let (src, out) = (decode(&ff, &path, k), decode(&ff, &cut_path, k));
        // output sample 0 is the first copied frame; Matroska times are rounded to the millisecond,
        // so the source position is counted in frames (FFmpeg decodes the source from its first)
        let from = t.first * frame * channels;
        let skip = if exact { 0 } else { frame * channels };
        let n = (RATE as usize / 2) * channels;
        assert!(out.len() >= skip + n && src.len() >= from + skip + n, "{name}: {} / {} samples", out.len(), src.len());
        let worst = out[skip..skip + n].iter().zip(&src[from + skip..]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        if exact {
            assert_eq!(worst, 0.0, "{name}: {} decodes bit for bit", t.codec);
        } else {
            assert!(worst < 1e-3, "{name}: {} decodes to the source audio (max difference {worst})", t.codec);
        }
    }
    Some(c)
}

const H264: [&str; 7] = ["libx264", "-bf", "2", "-pix_fmt", "yuv420p", "-x264-params", "keyint=12:min-keyint=12:scenecut=0:log-level=error"];

#[test]
fn h264_with_ac3_and_aac_is_cut_into_matroska() {
    let audio = [Audio { encoder: "ac3", layout: "5.1(side)" }, Audio { encoder: "aac", layout: "stereo" }];
    check("h264-ac3-aac", &H264, &audio, "mkv", Container::Matroska, &[("ac3", false, 1536), ("aac", false, 1024)]);
}

#[test]
fn hevc_with_truehd_starts_at_a_major_sync_and_decodes_bit_for_bit() {
    let video = ["libx265", "-pix_fmt", "yuv420p", "-x265-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=0:bframes=3:log-level=error"];
    let audio = [Audio { encoder: "truehd", layout: "5.1(side)" }];
    let Some(c) = check("hevc-truehd", &video, &audio, "mkv", Container::Matroska, &[("truehd", true, 40)]) else { return };
    // the cut is not on a restart point: the copy starts at the one before it
    assert!(c.audio.tracks[0].lead_ns < 0, "{:?}", c.audio.tracks[0]);
}

#[test]
fn mpeg2_with_dts_and_eac3_is_cut_into_matroska() {
    let video = ["mpeg2video", "-bf", "2", "-g", "12", "-q:v", "4", "-flags", "+cgop", "-sc_threshold", "1000000000"];
    let audio = [Audio { encoder: "dca", layout: "5.1(side)" }, Audio { encoder: "eac3", layout: "5.1(side)" }];
    check("mpeg2-dts-eac3", &video, &audio, "mkv", Container::Matroska, &[("dts", false, 512), ("eac3", false, 1536)]);
}

#[test]
fn av1_webm_keeps_the_video_and_names_the_audio_it_drops() {
    let video = ["libaom-av1", "-cpu-used", "8", "-g", "12", "-keyint_min", "12", "-lag-in-frames", "16", "-pix_fmt", "yuv420p"];
    let audio = [Audio { encoder: "libopus", layout: "stereo" }];
    let Some(c) = check("av1-webm-opus", &video, &audio, "webm", Container::WebM, &[]) else { return };
    assert_eq!(c.audio.dropped.len(), 1);
    assert!(c.audio.dropped[0].reason.contains("A_OPUS"), "{:?}", c.audio.dropped);
    let out = cfa4_matroska::open(c.output.as_slice()).unwrap();
    assert!(out.is_webm());
}

#[test]
fn a_damaged_audio_frame_stops_the_export_before_writing() {
    let audio = [Audio { encoder: "ac3", layout: "stereo" }];
    let Some(path) = source("h264-ac3-damaged", &H264, &audio, "mkv") else { return };
    let mut bytes = std::fs::read(&path).unwrap();
    let file = cfa4_matroska::open(bytes.as_slice()).unwrap();
    let a = file.tracks.iter().position(|t| t.kind == cfa4_matroska::TrackKind::Audio).unwrap();
    let s = file.tracks[a].samples[file.tracks[a].samples.len() / 2];
    bytes[s.offset as usize] ^= 0xFF;
    let err = cut_bytes(bytes, Container::Matroska, 12, 36).err().expect("the damaged frame is refused");
    assert!(matches!(&err, SmartCutError::Incompatible { detail } if detail.contains("ac3")), "{err:?}");
}

#[test]
fn verification_catches_a_changed_payload_and_wrong_containers_are_refused() {
    let audio = [Audio { encoder: "ac3", layout: "stereo" }];
    let Some(path) = source("h264-ac3-verify", &H264, &audio, "mkv") else { return };
    let mut c = cut_bytes(std::fs::read(&path).unwrap(), Container::Matroska, 12, 36).unwrap();
    let src = MkvCopySource { file: &c.file, bytes: c.bytes.as_slice(), video: &c.video };
    // flip a byte inside the last video frame of the output
    let out = cfa4_matroska::open(c.output.as_slice()).unwrap();
    let last = *out.tracks[0].samples.last().unwrap();
    c.output[(last.offset + u64::from(last.size) - 1) as usize] ^= 1;
    let err = verify_mkv_copy(&c.plan.plan, &c.video.index, &src, &c.audio, &c.report, c.output.as_slice()).unwrap_err();
    assert!(matches!(&err, SmartCutError::Verification { detail } if detail.contains("payload")), "{err:?}");
    // H.264 has no WebM mapping, and an MP4 source is not this executor's
    for (source, output) in [(Container::Matroska, Container::WebM), (Container::Mp4, Container::Matroska)] {
        let mut p = c.plan.plan.clone();
        (p.source_container, p.output_container) = (source, output);
        let r = execute_mkv_copy(&p, &c.video.index, &src, &MkvAudio::default(), Cursor::new(Vec::new()));
        assert!(matches!(r, Err(SmartCutError::Unsupported { .. })), "{source:?} → {output:?}: {:?}", r.err());
    }
}

/// Physical test (R2-07 sample set): the second and third GOP of every `.mkv` in
/// `CFA4_SMART_CUT_SAMPLES` are cut into Matroska with all their audio, and the re-demux check
/// passes. Skips when the variable is unset; the samples and cuts are never committed.
#[test]
fn physical_matroska_samples_cut_into_matroska() {
    let Some(dir_os) = std::env::var_os("CFA4_SMART_CUT_SAMPLES") else {
        skip("Smart Cut into Matroska samples", "CFA4_SMART_CUT_SAMPLES is not set");
        return;
    };
    let mut seen = 0;
    for entry in std::fs::read_dir(dir_os).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "mkv") {
            continue;
        }
        let bytes = std::fs::File::open(&path).unwrap();
        let file = cfa4_matroska::open(&bytes).unwrap();
        let video = index_mkv_video(&file, &bytes).unwrap();
        let gops = &video.index.gops;
        let first = |g: usize| video.index.frames[gops[g].start as usize].presentation_index.unwrap();
        let plan = plan(&video.index, Container::Matroska, Container::Matroska, first(1), first(3));
        assert!(plan.executable, "{}: {:?}", path.display(), plan.blockers);
        let src = MkvCopySource { file: &file, bytes: &bytes, video: &video };
        let (start, end) = mkv_cut_interval(&plan.plan, &video.index, &src).unwrap();
        let audio = plan_mkv_audio(&file, &bytes, Container::Matroska, start, end).unwrap();
        let out = dir("physical").join(path.file_name().unwrap());
        let (w, report) = execute_mkv_copy(&plan.plan, &video.index, &src, &audio, std::fs::File::create(&out).unwrap()).unwrap();
        drop(w);
        let written = std::fs::File::open(&out).unwrap();
        verify_mkv_copy(&plan.plan, &video.index, &src, &audio, &report, &written).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        eprintln!("{}: {} frames, audio {:?}, dropped {:?}", path.display(), report.frames, audio.tracks, audio.dropped);
        assert!(audio.dropped.is_empty(), "{}: {:?}", path.display(), audio.dropped);
        seen += 1;
    }
    assert!(seen > 0, "no .mkv in CFA4_SMART_CUT_SAMPLES");
}
