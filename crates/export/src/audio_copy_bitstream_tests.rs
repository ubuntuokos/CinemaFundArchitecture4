//! R2-02 of `CFA4-SMART-CUT-RENDER-R2`: AC-3, E-AC-3, DTS and TrueHD tracks of an MP4 are copied
//! with a Smart Cut, packet for packet, in sync, and decode from the cut point.
//!
//! The sources are made by FFmpeg (the encoders and the MP4 muxer of the oracle): the H.264 clip
//! of `smart_cut_tests` with a 2 s tone in the codec under test. The cut audio decoded by FFmpeg
//! must equal the source audio decoded from the cut: within rounding for the lossy codecs (after
//! the pre-roll packet the decoders are in the same state), bit for bit for TrueHD.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_isobmff::{Brand, Mp4File};
use cfa4_smart_edit::capabilities::Container;
use cfa4_smart_edit::copy_planner::CutMode;
use cfa4_testkit::oracle::{ffmpeg_has_encoder, ffmpeg_or_skip, skip};

use crate::audio_copy::{AudioPlan, CutInterval, mux_with_audio, plan_audio_copy, verify_audio_copy};
use crate::smart_cut::{Mp4CopySource, SmartCutError, execute_mp4_copy};
use crate::smart_cut_tests::{FPS, index_of, plan, source_mp4};

const RATE: u32 = 48_000;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/smart-cut-bitstream-audio").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Source {
    path: PathBuf,
    bytes: Vec<u8>,
    file: Mp4File,
}

/// The 40-picture, 25 fps H.264 clip with a 2 s tone in `encoder`, muxed by FFmpeg into MP4.
/// `offset` starts the audio that many seconds after the video, so that the cut falls between
/// two TrueHD restart points.
fn source(name: &str, encoder: &str, layout: &str, offset: &str) -> Option<Source> {
    let context = format!("Smart Cut {name} audio");
    let ff = ffmpeg_or_skip(&context)?;
    if !ffmpeg_has_encoder(&ff, encoder) {
        skip(&context, &format!("ffmpeg has no {encoder} encoder"));
        return None;
    }
    let d = dir(name);
    let video = d.join("video.mp4");
    std::fs::write(&video, source_mp4(50, 10, 2, Brand::Mp4)).unwrap();
    let path = d.join("source.mp4");
    let tone = format!("sine=frequency=440:sample_rate={RATE}:duration=2,aformat=channel_layouts={layout}");
    let o = Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&video)
        .args(["-itsoffset", offset, "-f", "lavfi", "-i", &tone, "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", encoder, "-strict", "-2", "-shortest"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(o.status.success(), "{context}: {}", String::from_utf8_lossy(&o.stderr));
    let bytes = std::fs::read(&path).unwrap();
    let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
    Some(Source { path, bytes, file })
}

struct Cut {
    video: Vec<u8>,
    output: Vec<u8>,
    audio: AudioPlan,
    interval: CutInterval,
}

/// Cut `[from, to)` pictures at GOP boundaries (keyframes every 10 pictures), with the audio.
fn cut(s: &Source, from: i64, to: i64) -> Result<Cut, SmartCutError> {
    let index = index_of(&s.file, &s.bytes);
    // FFmpeg's muxer picks its own video timescale: the plan counts pictures (25 fps)
    let vt = &s.file.tracks[0];
    let p0 = vt.samples.iter().map(|x| x.pts).min().unwrap() * i64::from(FPS) / i64::from(vt.timescale);
    let p = plan(&index, s.bytes.len(), p0 + from, p0 + to, CutMode::GopBoundary, Container::Mp4);
    let source = Mp4CopySource { file: &s.file, bytes: s.bytes.as_slice(), track: 0 };
    let (out, _) = execute_mp4_copy(&p.plan, &index, &source, Cursor::new(Vec::new())).unwrap();
    let video = out.into_inner();
    let first = &p.plan.segments[0];
    let interval = CutInterval::of_video(&s.file.tracks[0], first.source_in.value, p.plan.expected_duration.value as u64);
    let audio = plan_audio_copy(&s.file, s.bytes.as_slice(), interval);
    let output = mux_with_audio(&video, &s.file, s.bytes.as_slice(), &audio, interval, Brand::Mp4)?;
    Ok(Cut { video, output, audio, interval })
}

/// FFmpeg's decode of the first audio stream as interleaved 32-bit float samples.
fn decode(ff: &Path, file: &Path) -> Vec<f32> {
    let o = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-map", "0:a:0", "-f", "f32le", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// Cut 0.4–1.2 s and check the copy, its sync and its decode against the source's.
fn check(name: &str, encoder: &str, layout: &str, channels: usize, codec: &str, exact: bool) {
    // TrueHD restart points fall on the video's 40 ms grid unless the audio is offset
    let (offset, shift) = if exact { ("0.005", 0.005) } else { ("0", 0.0) };
    let Some(s) = source(name, encoder, layout, offset) else { return };
    let c = cut(&s, 10, 30).unwrap();
    assert!(c.audio.dropped.is_empty(), "{name}: {:?}", c.audio.dropped);
    let [t] = c.audio.tracks.as_slice() else { panic!("{name}: one audio track, got {:?}", c.audio.tracks) };
    assert_eq!(t.codec, codec);
    // 20 pictures at 25 fps: 0.8 s presented, starting at the cut; what is copied before the cut
    // (pre-roll, or back to a TrueHD restart point) is hidden by the edit list
    assert_eq!((t.presented, t.lead_in), (38_400 * u64::from(t.timescale) / u64::from(RATE), 0), "{name}: {t:?}");
    assert!(t.media_time > 0, "{name}: something is copied before the cut: {t:?}");
    let reports = verify_audio_copy(&c.output, &c.video, &s.file, s.bytes.as_slice(), &c.audio, c.interval).unwrap();
    assert!(reports[0].sync_error_ns.abs() <= 10_417, "{name}: {reports:?}");
    let out = cfa4_isobmff::open(c.output.as_slice()).unwrap();
    assert_eq!(out.tracks[1].entries, s.file.tracks[1].entries, "{name}: the sample entry is the source's");

    let Some(ff) = ffmpeg_or_skip(name) else { return };
    let cut_path = dir(name).join("cut.mp4");
    std::fs::write(&cut_path, &c.output).unwrap();
    let got = decode(&ff, &cut_path);
    let src = decode(&ff, &s.path);
    let at = |sec: f64| (sec * f64::from(RATE)) as usize * channels;
    // the source is decoded from its start; compare 0.4 s… against the cut's start, leaving the
    // last frame alone (FFmpeg trims the tail only to whole packets)
    let n = at(0.7);
    assert!(got.len() >= n, "{name}: {} samples decoded", got.len());
    // FFmpeg decodes the source track from its own start, `shift` seconds after the video's
    let want = &src[at(0.4 - shift)..at(0.4 - shift) + n];
    let worst = got[..n].iter().zip(want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    if exact {
        assert_eq!(worst, 0.0, "{name}: TrueHD decodes bit for bit");
    } else {
        assert!(worst < 1e-3, "{name}: the cut decodes to the source audio (max difference {worst})");
    }
}

#[test]
fn ac3_is_copied_and_decodes_from_the_cut() {
    check("ac3", "ac3", "5.1(side)", 6, "ac3", false);
}

#[test]
fn eac3_is_copied_and_decodes_from_the_cut() {
    check("eac3", "eac3", "5.1(side)", 6, "eac3", false);
}

#[test]
fn dts_is_copied_and_decodes_from_the_cut() {
    check("dts", "dca", "5.1(side)", 6, "dts", false);
}

#[test]
fn truehd_starts_at_a_restart_point_and_decodes_bit_for_bit() {
    check("truehd", "truehd", "5.1(side)", 6, "truehd", true);
}

#[test]
fn a_damaged_bitstream_packet_stops_the_export_before_writing() {
    let Some(mut s) = source("ac3-damaged", "ac3", "stereo", "0") else { return };
    // overwrite the sync word of an AC-3 packet inside the cut
    let track = &s.file.tracks[1];
    let k = track.samples.len() / 2;
    let off = track.samples[k].offset as usize;
    s.bytes[off] = 0;
    s.file = cfa4_isobmff::open(s.bytes.as_slice()).unwrap();
    let err = cut(&s, 10, 40).err().expect("the damaged packet is refused");
    assert!(matches!(&err, SmartCutError::Incompatible { detail } if detail.contains("ac3")), "{err:?}");
}
