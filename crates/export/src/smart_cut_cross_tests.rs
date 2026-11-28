//! R2-05 of `CFA4-SMART-CUT-RENDER-R2`: GOP-boundary Smart Cuts across container families,
//! Matroska → MP4 / MOV and MP4 → Matroska / WebM.
//!
//! FFmpeg (the oracle) makes the sources. Each cut is checked by its own verifier (every frame
//! with its source payload and time), by FFmpeg's framemd5 of the cut pictures, and by decoding
//! the audio: the cut decodes to the source audio from the same instant. Matroska times are
//! rounded to its timestamp scale (1 ms from FFmpeg), so the audio is matched within that
//! rounding: the best offset is searched within ±30 samples and must lie within half a
//! timestamp tick.

use std::io::Cursor;
use std::path::PathBuf;

use cfa4_isobmff::Mp4File;
use cfa4_smart_edit::capabilities::Container;
use cfa4_smart_edit::copy_planner::SmartCutPlan;
use cfa4_smart_edit::media_index::StreamIndex;
use cfa4_testkit::oracle::ffmpeg_or_skip;

use crate::audio_copy::AudioPlan;
use crate::mkv_source::{MkvVideo, index_mkv_video};
use crate::smart_cut::{Mp4CopySource, SmartCutError};
use crate::smart_cut_cross::{CrossReport, execute_mkv_to_mp4, execute_mp4_to_mkv, plan_mp4_to_mkv_audio, verify_mkv_to_mp4, verify_mp4_to_mkv};
use crate::smart_cut_mkv::{MkvAudio, MkvCopySource, mkv_cut_interval, plan_mkv_audio};
use crate::smart_cut_mkv_copy_tests::{Audio, H264, RATE, decode, framemd5, plan, source};
use crate::smart_cut_mkv_tests::index_mp4;

fn out_path(name: &str, ext: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/smart-cut-cross").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d.join(format!("cut.{ext}"))
}

/// The offset (in samples, `out` against `src[from..]`) within ±30 that matches best, and the
/// largest difference there over `n` samples after `skip`.
fn best_match(out: &[f32], src: &[f32], from: i64, skip: usize, n: usize, channels: usize) -> (i64, f32) {
    let mut best = (0, f32::MAX);
    for shift in -30i64..=30 {
        let at = from + shift * channels as i64 + skip as i64;
        let Ok(at) = usize::try_from(at) else { continue };
        if out.len() < skip + n || src.len() < at + n {
            continue;
        }
        let worst = out[skip..skip + n].iter().zip(&src[at..at + n]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        if worst < best.1 {
            best = (shift, worst);
        }
    }
    best
}

struct MkvCut {
    file: cfa4_matroska::MkvFile,
    plan: SmartCutPlan,
    audio: MkvAudio,
    report: CrossReport,
    output: Vec<u8>,
}

fn mkv_to_mp4(bytes: &[u8], output: Container, from: u64, to: u64) -> Result<(MkvCut, MkvVideo), SmartCutError> {
    let file = cfa4_matroska::open(bytes).unwrap();
    let video = index_mkv_video(&file, bytes).unwrap();
    let plan = plan(&video.index, crate::mkv_source::container(&file), output, from, to);
    assert!(plan.executable, "{:?}", plan.blockers);
    let src = MkvCopySource { file: &file, bytes, video: &video };
    let (start, end) = mkv_cut_interval(&plan.plan, &video.index, &src)?;
    let audio = plan_mkv_audio(&file, bytes, output, start, end)?;
    let (out, report) = execute_mkv_to_mp4(&plan.plan, &video.index, &src, &audio)?;
    verify_mkv_to_mp4(&plan.plan, &video.index, &src, &audio, &out)?;
    Ok((MkvCut { file, plan, audio, report, output: out }, video))
}

/// Cut 12–36 of a Matroska `name` into `output`; `expect` names the copied audio codecs and
/// whether they are lossless (matched bit for bit at the best offset).
fn check_mkv_to_mp4(name: &str, video: &[&str], audio: &[Audio<'_>], ext: &str, output: Container, expect: &[(&str, bool)]) -> Option<MkvCut> {
    let path = source(&format!("cross-{name}"), video, audio, ext)?;
    let bytes = std::fs::read(&path).unwrap();
    let (c, _) = mkv_to_mp4(&bytes, output, 12, 36).unwrap_or_else(|e| panic!("{name}: {e}"));
    let codecs: Vec<&str> = c.audio.tracks.iter().map(|t| t.codec.as_str()).collect();
    assert_eq!(codecs, expect.iter().map(|e| e.0).collect::<Vec<_>>(), "{name}: dropped {:?}", c.audio.dropped);
    assert_eq!(c.report.frames, c.plan.plan.copied_frames(), "{name}");
    assert!(!c.report.log.is_empty(), "{name}: the representation changes are logged");
    let ff = ffmpeg_or_skip(name)?;
    let cut = out_path(name, if output == Container::Mov { "mov" } else { "mp4" });
    std::fs::write(&cut, &c.output).unwrap();
    let (want, got) = (framemd5(&ff, &path), framemd5(&ff, &cut));
    let first = c.plan.plan.segments[0].presentation.first as usize;
    assert_eq!(got.len() as u64, c.plan.plan.copied_frames(), "{name}");
    assert_eq!(got, want[first..first + got.len()].to_vec(), "{name}: the cut pictures decode as the source's");
    // the MP4 edit list starts the audio at the first picture: the source audio from that instant
    let vt = c.file.tracks.iter().find(|t| t.kind == cfa4_matroska::TrackKind::Video).unwrap();
    let start_ns = mkv_start_ns(&c, vt);
    for (k, (t, &(codec, exact))) in c.audio.tracks.iter().zip(expect).enumerate() {
        let channels = c.file.tracks[t.track].audio.as_ref().unwrap().channels as usize;
        let (src, out) = (decode(&ff, &path, k), decode(&ff, &cut, k));
        // FFmpeg decodes a Matroska track from its first frame. FFmpeg 6 keeps the CodecDelay (AAC
        // priming, a first frame before 0), FFmpeg 9 drops it; neither pads a track that starts
        // late. Both readings are tried: the placement must be right against one of them
        let st = &c.file.tracks[t.track];
        let first_ns = st.ticks_to_ns(st.samples[0].pts);
        let skip = if exact { 0 } else { 2048 * channels };
        let (shift, worst) = [first_ns, first_ns.max(0)]
            .into_iter()
            .map(|f| {
                let from = (i128::from(start_ns - f) * i128::from(RATE) / 1_000_000_000) as i64 * channels as i64;
                best_match(&out, &src, from, skip, (RATE as usize / 4) * channels, channels)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(shift.abs() <= 24, "{name}: {codec} is placed {shift} samples off (more than half a millisecond)");
        if exact {
            assert_eq!(worst, 0.0, "{name}: {codec} decodes bit for bit");
        } else {
            assert!(worst < 1e-3, "{name}: {codec} decodes to the source audio (max difference {worst})");
        }
    }
    Some(c)
}

fn mkv_start_ns(c: &MkvCut, vt: &cfa4_matroska::Track) -> i64 {
    let s = &c.plan.plan.segments;
    let (first, last) = (s[0].decode.first as usize, s[s.len() - 1].decode.last as usize);
    vt.samples[first..=last].iter().map(|s| vt.ticks_to_ns(s.pts)).min().unwrap()
}

#[test]
fn h264_matroska_with_aac_flac_and_pcm_is_cut_into_mp4() {
    let audio = [
        Audio { encoder: "aac", layout: "stereo" },
        Audio { encoder: "flac", layout: "stereo" },
        Audio { encoder: "pcm_s16le", layout: "stereo" },
        Audio { encoder: "dca", layout: "5.1(side)" },
    ];
    let Some(c) = check_mkv_to_mp4("h264-mkv-mp4", &H264, &audio, "mkv", Container::Mp4, &[("aac", false), ("flac", true), ("pcm", true)]) else { return };
    // DTS into MP4 needs its ddts sample entry, which is not built yet: named, not dropped silently
    assert_eq!(c.audio.dropped.len(), 1, "{:?}", c.audio.dropped);
    assert!(c.audio.dropped[0].reason.contains("A_DTS"), "{:?}", c.audio.dropped);
}

#[test]
fn hevc_matroska_with_truehd_ac3_and_eac3_is_cut_into_mp4() {
    let video = ["libx265", "-pix_fmt", "yuv420p", "-x265-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=0:bframes=3:log-level=error"];
    let audio =
        [Audio { encoder: "truehd", layout: "5.1(side)" }, Audio { encoder: "ac3", layout: "5.1(side)" }, Audio { encoder: "eac3", layout: "5.1(side)" }];
    let Some(c) = check_mkv_to_mp4("hevc-mkv-dolby-mp4", &video, &audio, "mkv", Container::Mp4, &[("truehd", true), ("ac3", false), ("eac3", false)]) else {
        return;
    };
    assert!(c.audio.dropped.is_empty(), "{:?}", c.audio.dropped);
    let out = cfa4_isobmff::open(c.output.as_slice()).unwrap();
    let formats: Vec<String> = out.tracks.iter().skip(1).map(|t| String::from_utf8_lossy(&t.entries[0].format.0).into_owned()).collect();
    assert_eq!(formats, ["mlpa", "ac-3", "ec-3"]);
}

/// The `dac3`, `dec3` and `dmlp` payloads built from the first frame equal the ones FFmpeg writes
/// for the same stream.
#[test]
fn dolby_sample_entries_match_ffmpeg() {
    use crate::smart_cut_cross::{dac3_from_frame, dec3_from_access_unit, dmlp_from_access_unit};
    let audio = [Audio { encoder: "ac3", layout: "5.1(side)" }, Audio { encoder: "eac3", layout: "stereo" }, Audio { encoder: "truehd", layout: "5.1(side)" }];
    let Some(path) = source("cross-dolby-entries", &H264, &audio, "mp4") else { return };
    let bytes = std::fs::read(&path).unwrap();
    let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
    let mut seen = 0;
    for (i, t) in file.tracks.iter().enumerate() {
        let Some(e) = t.entries.first() else { continue };
        let first = file.read_sample(bytes.as_slice(), i, 0).unwrap();
        match &e.codec {
            cfa4_isobmff::CodecConfig::Ac3 { dac3 } => assert_eq!(&dac3_from_frame(&first).unwrap(), dac3, "dac3"),
            cfa4_isobmff::CodecConfig::Eac3 { dec3 } => assert_eq!(&dec3_from_access_unit(&first).unwrap(), dec3, "dec3"),
            cfa4_isobmff::CodecConfig::TrueHd { dmlp } => assert_eq!(&dmlp_from_access_unit(&first).unwrap(), dmlp, "dmlp"),
            _ => continue,
        }
        seen += 1;
    }
    assert_eq!(seen, 3);
}

#[test]
fn hevc_matroska_is_cut_into_quicktime() {
    let video = ["libx265", "-pix_fmt", "yuv420p", "-x265-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=0:bframes=3:log-level=error"];
    check_mkv_to_mp4("hevc-mkv-mov", &video, &[], "mkv", Container::Mov, &[]);
}

#[test]
fn av1_webm_is_cut_into_mp4() {
    let video = ["libaom-av1", "-cpu-used", "8", "-g", "12", "-keyint_min", "12", "-lag-in-frames", "16", "-pix_fmt", "yuv420p"];
    check_mkv_to_mp4("av1-webm-mp4", &video, &[], "webm", Container::Mp4, &[]);
}

struct Mp4Cut {
    file: Mp4File,
    plan: SmartCutPlan,
    audio: AudioPlan,
    report: CrossReport,
    output: Vec<u8>,
}

fn mp4_to_mkv(bytes: &[u8], output: Container) -> Result<(Mp4Cut, StreamIndex), SmartCutError> {
    let file = cfa4_isobmff::open(bytes).unwrap();
    let index = index_mp4(&file, bytes);
    let track = file.tracks.iter().position(|t| t.kind == cfa4_isobmff::TrackKind::Video).unwrap();
    let plan = plan(&index, Container::Mp4, output, 12, 36);
    assert!(plan.executable, "{:?}", plan.blockers);
    let src = Mp4CopySource { file: &file, bytes, track };
    let audio = plan_mp4_to_mkv_audio(&plan.plan, &index, &src)?;
    let (out, report) = execute_mp4_to_mkv(&plan.plan, &index, &src, &audio, Cursor::new(Vec::new()))?;
    let output = out.into_inner();
    verify_mp4_to_mkv(&plan.plan, &index, &src, &audio, output.as_slice())?;
    Ok((Mp4Cut { file, plan, audio, report, output }, index))
}

/// Cut 12–36 of an MP4 `name` into `output`, as [`check_mkv_to_mp4`] does the other way.
fn check_mp4_to_mkv(name: &str, video: &[&str], audio: &[Audio<'_>], output: Container, expect: &[(&str, bool)]) -> Option<Mp4Cut> {
    let path = source(&format!("cross-{name}"), video, audio, "mp4")?;
    let bytes = std::fs::read(&path).unwrap();
    let (c, _) = mp4_to_mkv(&bytes, output).unwrap_or_else(|e| panic!("{name}: {e}"));
    let codecs: Vec<&str> = c.audio.tracks.iter().map(|t| t.codec.as_str()).collect();
    assert_eq!(codecs, expect.iter().map(|e| e.0).collect::<Vec<_>>(), "{name}: dropped {:?}", c.audio.dropped);
    assert_eq!(c.report.frames, c.plan.plan.copied_frames(), "{name}");
    let ff = ffmpeg_or_skip(name)?;
    let cut = out_path(name, if output == Container::WebM { "webm" } else { "mkv" });
    std::fs::write(&cut, &c.output).unwrap();
    let (want, got) = (framemd5(&ff, &path), framemd5(&ff, &cut));
    let first = c.plan.plan.segments[0].presentation.first as usize;
    assert_eq!(got.len() as u64, c.plan.plan.copied_frames(), "{name}");
    assert_eq!(got, want[first..first + got.len()].to_vec(), "{name}: the cut pictures decode as the source's");
    for (k, (t, &(codec, exact))) in c.audio.tracks.iter().zip(expect).enumerate() {
        let st = &c.file.tracks[t.track];
        let channels = st.entries[0].audio.as_ref().unwrap().channels as usize;
        let (src, out) = (decode(&ff, &path, k), decode(&ff, &cut, k));
        // Matroska output sample 0 is the first copied packet. FFmpeg decodes the source from its
        // first sample with the edit list's hidden start (priming) dropped, but without an empty
        // edit's silence: the packet's position in that decode
        let from = (st.samples[t.first].dts + st.edit_offset.min(0)) * channels as i64;
        let skip = if exact { 0 } else { 2048 * channels };
        let (shift, worst) = best_match(&out, &src, from, skip, (RATE as usize / 4) * channels, channels);
        let half_tick = (c.report.max_rounding_ns.max(1) * i64::from(RATE) + 999_999_999) / 1_000_000_000;
        assert!(shift.abs() <= half_tick.max(1), "{name}: {codec} is placed {shift} samples off");
        if exact {
            assert_eq!(worst, 0.0, "{name}: {codec} decodes bit for bit");
        } else {
            assert!(worst < 1e-3, "{name}: {codec} decodes to the source audio (max difference {worst})");
        }
    }
    Some(c)
}

#[test]
fn h264_mp4_with_aac_ac3_and_eac3_is_cut_into_matroska() {
    let audio = [Audio { encoder: "aac", layout: "stereo" }, Audio { encoder: "ac3", layout: "5.1(side)" }, Audio { encoder: "eac3", layout: "5.1(side)" }];
    let Some(c) = check_mp4_to_mkv("h264-mp4-mkv", &H264, &audio, Container::Matroska, &[("aac", false), ("ac3", false), ("eac3", false)]) else { return };
    // the 25 fps MP4 timescale (12 800) is held exactly by a 78 125 ns timestamp scale
    let out = cfa4_matroska::open(c.output.as_slice()).unwrap();
    assert_eq!(out.info.timestamp_scale, 78_125);
    assert!(c.audio.dropped.is_empty(), "{:?}", c.audio.dropped);
}

#[test]
fn hevc_mp4_with_truehd_is_cut_into_matroska_bit_for_bit() {
    let video = ["libx265", "-pix_fmt", "yuv420p", "-x265-params", "keyint=12:min-keyint=12:scenecut=0:open-gop=0:bframes=3:log-level=error"];
    let audio = [Audio { encoder: "truehd", layout: "5.1(side)" }];
    check_mp4_to_mkv("hevc-mp4-truehd-mkv", &video, &audio, Container::Matroska, &[("truehd", true)]);
}

#[test]
fn av1_mp4_is_cut_into_webm_and_names_the_audio_it_drops() {
    let video = ["libaom-av1", "-cpu-used", "8", "-g", "12", "-keyint_min", "12", "-lag-in-frames", "16", "-pix_fmt", "yuv420p"];
    let audio = [Audio { encoder: "aac", layout: "stereo" }];
    let Some(c) = check_mp4_to_mkv("av1-mp4-webm", &video, &audio, Container::WebM, &[]) else { return };
    assert_eq!(c.audio.dropped.len(), 1);
    assert!(c.audio.dropped[0].reason.contains("WebM"), "{:?}", c.audio.dropped);
    assert!(cfa4_matroska::open(c.output.as_slice()).unwrap().is_webm());
}

#[test]
fn verification_catches_a_changed_payload_and_wrong_pairs_are_refused() {
    let audio = [Audio { encoder: "aac", layout: "stereo" }];
    let Some(path) = source("cross-h264-verify", &H264, &audio, "mkv") else { return };
    let bytes = std::fs::read(&path).unwrap();
    let (c, video) = mkv_to_mp4(&bytes, Container::Mp4, 12, 36).unwrap();
    let src = MkvCopySource { file: &c.file, bytes: bytes.as_slice(), video: &video };
    // flip the last byte of the last video sample of the output
    let out = cfa4_isobmff::open(c.output.as_slice()).unwrap();
    let v = out.tracks.iter().position(|t| t.kind == cfa4_isobmff::TrackKind::Video).unwrap();
    let last = out.tracks[v].samples.last().unwrap();
    let mut damaged = c.output.clone();
    damaged[(last.offset + u64::from(last.size) - 1) as usize] ^= 1;
    let err = verify_mkv_to_mp4(&c.plan.plan, &video.index, &src, &c.audio, &damaged).unwrap_err();
    assert!(matches!(&err, SmartCutError::Verification { detail } if detail.contains("payload")), "{err:?}");
    // H.264 has no WebM mapping, and a Matroska output is not the MP4 executor's
    for output in [Container::WebM, Container::Matroska] {
        let mut p = c.plan.plan.clone();
        p.output_container = output;
        let r = execute_mkv_to_mp4(&p, &video.index, &src, &MkvAudio::default());
        assert!(matches!(r, Err(SmartCutError::Unsupported { .. })), "{output:?}: {:?}", r.err());
    }
}

#[test]
fn mpeg2_matroska_is_not_cut_into_mp4() {
    let video = ["mpeg2video", "-bf", "2", "-g", "12", "-q:v", "4", "-flags", "+cgop", "-sc_threshold", "1000000000"];
    let Some(path) = source("cross-mpeg2-mp4", &video, &[], "mkv") else { return };
    let bytes = std::fs::read(&path).unwrap();
    let file = cfa4_matroska::open(bytes.as_slice()).unwrap();
    let video = index_mkv_video(&file, bytes.as_slice()).unwrap();
    // the MPEG-2 plan into MP4 has no executor: refused, never written
    let p = plan(&video.index, Container::Matroska, Container::Matroska, 12, 36);
    let mut plan = p.plan.clone();
    plan.output_container = Container::Mp4;
    let src = MkvCopySource { file: &file, bytes: bytes.as_slice(), video: &video };
    let r = execute_mkv_to_mp4(&plan, &video.index, &src, &MkvAudio::default());
    assert!(matches!(r, Err(SmartCutError::Unsupported { .. })), "{:?}", r.err());
}

/// Physical test (R2-07 sample set): GOPs 1–3 of every `.mkv` in `CFA4_SMART_CUT_SAMPLES` are cut
/// into MP4, and of every `.mp4` into Matroska, each checked by its verifier. Skips when the
/// variable is unset; the samples and cuts are never committed.
#[test]
fn physical_samples_cut_across_containers() {
    let Some(dir_os) = std::env::var_os("CFA4_SMART_CUT_SAMPLES") else {
        // a physical test: not an oracle test, so CFA4_REQUIRE_ORACLES does not make it fail
        eprintln!("SKIPPED (cross-container Smart Cut samples): CFA4_SMART_CUT_SAMPLES is not set");
        return;
    };
    let mut seen = 0;
    for entry in std::fs::read_dir(dir_os).unwrap() {
        let path = entry.unwrap().path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        let first = |index: &StreamIndex, g: usize| index.frames[index.gops[g].start as usize].presentation_index.unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        match ext.as_str() {
            "mkv" => {
                let bytes = std::fs::File::open(&path).unwrap();
                let file = cfa4_matroska::open(&bytes).unwrap();
                let video = index_mkv_video(&file, &bytes).unwrap();
                let plan = plan(&video.index, Container::Matroska, Container::Mp4, first(&video.index, 1), first(&video.index, 3));
                assert!(plan.executable, "{name}: {:?}", plan.blockers);
                let src = MkvCopySource { file: &file, bytes: &bytes, video: &video };
                let (start, end) = mkv_cut_interval(&plan.plan, &video.index, &src).unwrap();
                let audio = plan_mkv_audio(&file, &bytes, Container::Mp4, start, end).unwrap();
                let (out, report) = execute_mkv_to_mp4(&plan.plan, &video.index, &src, &audio).unwrap_or_else(|e| panic!("{name}: {e}"));
                verify_mkv_to_mp4(&plan.plan, &video.index, &src, &audio, &out).unwrap_or_else(|e| panic!("{name}: {e}"));
                std::fs::write(out_path("physical", &format!("{name}.mp4")), &out).unwrap();
                eprintln!("{name} → MP4: {} frames, audio {:?}, dropped {:?}, log {:?}", report.frames, report.audio, audio.dropped, report.log);
            }
            "mp4" => {
                let bytes = std::fs::read(&path).unwrap();
                let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
                let index = index_mp4(&file, &bytes);
                let track = file.tracks.iter().position(|t| t.kind == cfa4_isobmff::TrackKind::Video).unwrap();
                let plan = plan(&index, Container::Mp4, Container::Matroska, first(&index, 1), first(&index, 3));
                assert!(plan.executable, "{name}: {:?}", plan.blockers);
                let src = Mp4CopySource { file: &file, bytes: bytes.as_slice(), track };
                let audio = plan_mp4_to_mkv_audio(&plan.plan, &index, &src).unwrap();
                let (out, report) = execute_mp4_to_mkv(&plan.plan, &index, &src, &audio, Cursor::new(Vec::new())).unwrap_or_else(|e| panic!("{name}: {e}"));
                let out = out.into_inner();
                verify_mp4_to_mkv(&plan.plan, &index, &src, &audio, out.as_slice()).unwrap_or_else(|e| panic!("{name}: {e}"));
                std::fs::write(out_path("physical", &format!("{name}.mkv")), &out).unwrap();
                eprintln!("{name} → Matroska: {} frames, audio {:?}, dropped {:?}, log {:?}", report.frames, report.audio, audio.dropped, report.log);
                assert!(audio.dropped.is_empty(), "{name}: {:?}", audio.dropped);
                // the Dolby-authored dec3 (Atmos joint object coding) is rebuilt from the first access unit
                for (i, t) in file.tracks.iter().enumerate() {
                    if let Some(cfa4_isobmff::CodecConfig::Eac3 { dec3 }) = t.entries.first().map(|e| &e.codec) {
                        let first = file.read_sample(bytes.as_slice(), i, 0).unwrap();
                        assert_eq!(&crate::smart_cut_cross::dec3_from_access_unit(&first).unwrap(), dec3, "{name}: dec3");
                    }
                }
            }
            _ => continue,
        }
        seen += 1;
    }
    assert!(seen > 0, "no .mkv or .mp4 in CFA4_SMART_CUT_SAMPLES");
}
