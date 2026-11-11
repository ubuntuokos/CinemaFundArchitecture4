//! WP-09b: the audio of a Smart Cut is copied with the video, unchanged and in sync (SCE-001 §9,
//! edge cases 12–13: AAC encoder delay, several audio tracks).

use std::io::Cursor;
use std::process::Command;

use cfa4_isobmff::{Brand, Mp4File, Mp4Writer, PcmConfig, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_smart_edit::capabilities::Container;
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan};

use crate::audio_copy::{AudioPlan, CutInterval, mux_with_audio, plan_audio_copy, verify_audio_copy};
use crate::smart_cut::{Mp4CopySource, SmartCutError, execute_mp4_copy, verify_mp4_copy};
use crate::smart_cut_tests::{index_of, plan, source_mp4};

const RATE: u32 = 48_000;

/// A test tone: `n` samples of channel `ch`, starting at sample `from`.
fn tone(from: usize, n: usize, ch: usize) -> Vec<f32> {
    (from..from + n).map(|i| (i as f32 * (0.03 + ch as f32 * 0.01)).sin() * 0.4).collect()
}

struct Av {
    bytes: Vec<u8>,
    file: Mp4File,
}

/// The H.264 clip of `smart_cut_tests` (`frames` pictures at 25 fps) with an AAC track (stereo,
/// its encoder delay hidden by an edit list) and a 16-bit PCM track, both as long as the video
/// (`audio_secs` overrides that).
fn av_source(frames: usize, bframes: u8, audio_secs: Option<f64>) -> Av {
    let video = source_mp4(frames, 10, bframes, Brand::Mp4);
    let v = cfa4_isobmff::open(video.as_slice()).unwrap();
    let vt = &v.tracks[0];
    let samples = (audio_secs.unwrap_or(frames as f64 / 25.0) * f64::from(RATE)) as usize;
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let mut cfg = TrackConfig::new(vt.entries[0].clone(), vt.timescale);
    cfg.edits = vt.edits.clone();
    let tv = w.add_track(cfg).unwrap();
    for (i, s) in vt.samples.iter().enumerate() {
        let data = v.read_sample(video.as_slice(), 0, i).unwrap();
        w.write_sample(tv, WriteSample { data: &data, duration: s.duration, composition_offset: (s.pts - s.dts) as i32, is_sync: s.is_sync }).unwrap();
    }
    // AAC: priming hidden by the edit list, as every AAC export writes it
    let mut enc = cfa4_aac::Encoder::new(cfa4_aac::EncoderConfig::cbr(RATE, 2, 128_000)).unwrap();
    let mut cfg = TrackConfig::new(SampleEntry::aac(enc.audio_specific_config(), 2, RATE), RATE);
    cfg.media_start = Some(i64::from(enc.priming_samples()));
    let ta = w.add_track(cfg).unwrap();
    let (l, r) = (tone(0, samples, 0), tone(0, samples, 1));
    let mut packets = enc.encode(&[&l, &r]);
    packets.extend(enc.flush());
    for p in &packets {
        w.write_sample(ta, WriteSample { data: p, duration: 1024, composition_offset: 0, is_sync: true }).unwrap();
    }
    // PCM: 16-bit mono in chunks of 1000 frames
    let pcm = PcmConfig { bits: 16, float: false, big_endian: false, signed: true, channels: 1, sample_rate: f64::from(RATE) };
    let tp = w.add_track(TrackConfig::new(SampleEntry::pcm(pcm), RATE)).unwrap();
    let mono: Vec<u8> = tone(0, samples, 2).iter().flat_map(|x| ((x * 32767.0) as i16).to_le_bytes()).collect();
    for chunk in mono.chunks(2000) {
        w.write_sample(tp, WriteSample { data: chunk, duration: 0, composition_offset: 0, is_sync: true }).unwrap();
    }
    let bytes = w.finish().unwrap().into_inner();
    let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
    Av { bytes, file }
}

struct Cut {
    video: Vec<u8>,
    output: Vec<u8>,
    audio: AudioPlan,
    interval: CutInterval,
}

fn interval(av: &Av, p: &SmartCutPlan) -> CutInterval {
    let first = &p.plan.segments[0];
    CutInterval::of_video(&av.file.tracks[0], first.source_in.value, p.plan.expected_duration.value as u64)
}

/// Cut `[from, to)` pictures (source PTS) at GOP boundaries, with the audio.
fn cut(av: &Av, from: i64, to: i64) -> Cut {
    let index = index_of(&av.file, &av.bytes);
    let p0 = av.file.tracks[0].samples.iter().map(|s| s.pts).min().unwrap();
    let p = plan(&index, av.bytes.len(), p0 + from, p0 + to, CutMode::GopBoundary, Container::Mp4);
    let source = Mp4CopySource { file: &av.file, bytes: av.bytes.as_slice(), track: 0 };
    let (out, report) = execute_mp4_copy(&p.plan, &index, &source, Cursor::new(Vec::new())).unwrap();
    let video = out.into_inner();
    assert_eq!(verify_mp4_copy(&p.plan, &index, &source, video.as_slice()).unwrap(), report);
    let interval = interval(av, &p);
    let audio = plan_audio_copy(&av.file, interval);
    let output = mux_with_audio(&video, &av.file, av.bytes.as_slice(), &audio, interval, Brand::Mp4).unwrap();
    Cut { video, output, audio, interval }
}

#[test]
fn aac_and_pcm_tracks_are_copied_unchanged_and_in_sync() {
    for bframes in [0u8, 2] {
        let av = av_source(40, bframes, None);
        let c = cut(&av, 10, 30);
        assert!(c.audio.dropped.is_empty(), "{:?}", c.audio.dropped);
        assert_eq!(c.audio.tracks.iter().map(|t| t.codec.as_str()).collect::<Vec<_>>(), ["aac", "pcm"]);
        // 20 pictures at 25 fps = 0.8 s of audio presented
        assert!(c.audio.tracks.iter().all(|t| t.presented == 38_400 && t.lead_in == 0), "{:?}", c.audio.tracks);
        // AAC: one packet of pre-roll before the cut, which the edit list hides with the rest
        let aac = &c.audio.tracks[0];
        assert!(aac.media_time >= 1024, "{aac:?}");
        // PCM: cut at the exact sample (0.4 s in)
        assert_eq!(c.audio.tracks[1].pcm.map(|p| p.1), Some(38_400));
        let reports = verify_audio_copy(&c.output, &c.video, &av.file, av.bytes.as_slice(), &c.audio, c.interval).unwrap();
        assert_eq!(reports.len(), 2);
        for r in &reports {
            // half a tick of 48 kHz is 10 417 ns; these cuts land on whole ticks
            assert_eq!(r.sync_error_ns, 0, "{r:?}");
        }
        assert_eq!(reports[1].payload_bytes, 38_400 * 2);
        let out = cfa4_isobmff::open(c.output.as_slice()).unwrap();
        assert_eq!(out.tracks.len(), 3);
        assert_eq!(out.tracks[1].entries, av.file.tracks[1].entries, "the AAC sample entry is the source's");
    }
}

#[test]
fn audio_shorter_or_later_than_the_cut_is_trimmed_or_dropped() {
    // the audio stops 1.0 s in: a cut 0.4–1.2 s keeps 0.6 s of it, a cut 1.2–1.6 s none
    let av = av_source(40, 0, Some(1.0));
    let c = cut(&av, 10, 30);
    // AAC ends with the encoder's last packet, which the source presents too (its edit list runs to
    // the end of the media); PCM ends exactly at 1.0 s
    assert!((28_800..28_800 + 1024).contains(&c.audio.tracks[0].presented), "{:?}", c.audio.tracks);
    assert_eq!(c.audio.tracks[1].presented, 28_800);
    verify_audio_copy(&c.output, &c.video, &av.file, av.bytes.as_slice(), &c.audio, c.interval).unwrap();
    let c = cut(&av, 30, 40);
    assert!(c.audio.tracks.is_empty());
    assert_eq!(c.audio.dropped.len(), 2);
    assert!(c.audio.dropped.iter().all(|d| d.reason.contains("overlap")), "{:?}", c.audio.dropped);
    let out = cfa4_isobmff::open(c.output.as_slice()).unwrap();
    assert_eq!(out.tracks.len(), 1);
}

#[test]
fn verification_catches_changed_or_shifted_audio() {
    let av = av_source(40, 0, None);
    let c = cut(&av, 10, 30);
    let out = cfa4_isobmff::open(c.output.as_slice()).unwrap();
    // one byte of the PCM track
    let mut bad = c.output.clone();
    let off = out.tracks[2].samples[0].offset as usize + 100;
    bad[off] ^= 0x40;
    let err = verify_audio_copy(&bad, &c.video, &av.file, av.bytes.as_slice(), &c.audio, c.interval).unwrap_err();
    assert!(matches!(&err, SmartCutError::Verification { detail } if detail.contains("differs")), "{err:?}");
    // an AAC track written 1 ms early is out of sync
    let mut early = c.audio.clone();
    early.tracks[0].media_time += 48;
    let moved = mux_with_audio(&c.video, &av.file, av.bytes.as_slice(), &early, c.interval, Brand::Mp4).unwrap();
    let err = verify_audio_copy(&moved, &c.video, &av.file, av.bytes.as_slice(), &c.audio, c.interval).unwrap_err();
    assert!(matches!(&err, SmartCutError::Verification { detail } if detail.contains("sync")), "{err:?}");
    // a file without the audio
    let err = verify_audio_copy(&c.video, &c.video, &av.file, av.bytes.as_slice(), &c.audio, c.interval).unwrap_err();
    assert!(matches!(&err, SmartCutError::Verification { detail } if detail.contains("tracks")), "{err:?}");
}

#[test]
fn ffmpeg_plays_the_cut_audio_from_the_cut_point() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("Smart Cut audio") else { return };
    let av = av_source(40, 2, None);
    let c = cut(&av, 10, 30);
    let dir = std::env::temp_dir().join(format!("cfa4-audio-copy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, out) = (dir.join("src.mp4"), dir.join("cut.mp4"));
    std::fs::write(&src, &av.bytes).unwrap();
    std::fs::write(&out, &c.output).unwrap();
    let pcm = |file: &std::path::Path, stream: &str, extra: &[&str]| -> Vec<u8> {
        let mut args: Vec<&str> = vec!["-v", "error"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["-i", file.to_str().unwrap(), "-map", stream, "-f", "s16le", "-ac", "1", "-"]);
        let o = Command::new(&ffmpeg).args(&args).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o.stdout
    };
    // PCM: exactly the source's samples from 0.4 s to 1.2 s
    let cut_pcm = pcm(&out, "0:a:1", &[]);
    let src_pcm = pcm(&src, "0:a:1", &[]);
    assert_eq!(cut_pcm.len(), 38_400 * 2);
    assert_eq!(cut_pcm, src_pcm[19_200 * 2..57_600 * 2]);
    // AAC: 0.8 s decoded, matching the source decode from 0.4 s (decoders agree after pre-roll)
    let cut_aac = pcm(&out, "0:a:0", &[]);
    let src_aac = pcm(&src, "0:a:0", &[]);
    // ffmpeg trims the pre-roll by the edit list exactly, and the tail only to whole packets
    assert!((38_400 * 2..(38_400 + 1024) * 2).contains(&cut_aac.len()), "{}", cut_aac.len());
    let samples = |b: &[u8]| b.chunks(2).map(|x| i16::from_le_bytes([x[0], x[1]])).collect::<Vec<_>>();
    let (a, b) = (samples(&cut_aac[..38_400 * 2]), samples(&src_aac[19_200 * 2..57_600 * 2]));
    let worst = a.iter().zip(&b).skip(1024).map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs()).max().unwrap();
    assert!(worst <= 2, "the cut AAC decodes to the source audio (max difference {worst})");
    let _ = std::fs::remove_dir_all(&dir);
}
