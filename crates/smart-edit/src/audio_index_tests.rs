use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::oracle::{ffmpeg_has_encoder, ffmpeg_or_skip, ffprobe, skip};

use crate::audio_index::{AudioCodec, AudioIndexError, index_audio, split_elementary};

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/smart-edit-audio");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A raw elementary stream FFmpeg encodes from a 2 s tone, or `None` (skipped) without the encoder.
fn encode(context: &str, encoder: &str, format: &str, layout: &str, rate: u32, extra: &[&str]) -> Option<(PathBuf, Vec<u8>)> {
    let ff = ffmpeg_or_skip(context)?;
    if !ffmpeg_has_encoder(&ff, encoder) {
        skip(context, &format!("ffmpeg has no {encoder} encoder"));
        return None;
    }
    // one file per test case: tests run in parallel and some encode the same stream
    let name: String = context.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let out = fixtures().join(format!("{name}.{format}"));
    let src = format!("sine=frequency=440:sample_rate={rate}:duration=2,aformat=channel_layouts={layout}");
    let status = Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &src, "-strict", "-2", "-c:a", encoder])
        .args(extra)
        .args(["-f", format])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "{context}: ffmpeg failed");
    let data = std::fs::read(&out).unwrap();
    Some((out, data))
}

/// FFmpeg's packets of a raw stream: (key frame flag) for each.
fn ffprobe_packets(path: &Path) -> Vec<bool> {
    let probe = ffprobe().expect("ffprobe next to ffmpeg");
    let out =
        Command::new(probe).args(["-v", "error", "-select_streams", "a:0", "-show_entries", "packet=flags", "-of", "csv=p=0"]).arg(path).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(|l| l.starts_with('K')).collect()
}

fn check_against_ffmpeg(context: &str, codec: AudioCodec, encoder: &str, format: &str, layout: &str, rate: u32, extra: &[&str]) {
    let Some((path, data)) = encode(context, encoder, format, layout, rate, extra) else { return };
    let packets = split_elementary(codec, &data);
    assert_eq!(packets.iter().map(|p| p.len()).sum::<usize>(), data.len(), "{context}: the packets cover the stream");
    let index = index_audio(codec, packets.iter().copied()).unwrap_or_else(|e| panic!("{context}: {e}"));
    let flags = ffprobe_packets(&path);
    assert_eq!(index.units.len(), flags.len(), "{context}: packet count against ffprobe");
    let entries: Vec<bool> = index.units.iter().map(|u| u.entry).collect();
    assert_eq!(entries, flags, "{context}: entry points against ffprobe's key frames");
    assert_eq!(index.configs.len(), 1, "{context}: one configuration");
    assert_eq!(index.configs[0].sample_rate, rate, "{context}: sample rate");
    // the samples add up to the 2 s tone, up to the last frame's padding
    let samples: u64 = index.units.iter().map(|u| u64::from(u.samples)).sum();
    let tone = u64::from(rate) * 2;
    let frame = u64::from(index.units.iter().map(|u| u.samples).max().unwrap());
    assert!(samples >= tone && samples < tone + 2 * frame + 2048, "{context}: {samples} samples for {tone}");
    assert_eq!(index.copy_start(0), Some(0));
}

#[test]
fn ac3_frames_match_ffmpeg() {
    for (layout, rate) in [("stereo", 48_000), ("5.1", 48_000), ("mono", 44_100), ("5.1", 32_000)] {
        check_against_ffmpeg(&format!("ac3 {layout} {rate}"), AudioCodec::Ac3, "ac3", "ac3", layout, rate, &[]);
    }
}

#[test]
fn eac3_frames_match_ffmpeg() {
    for (layout, rate) in [("stereo", 48_000), ("5.1(side)", 48_000), ("5.1", 44_100)] {
        check_against_ffmpeg(&format!("eac3 {layout} {rate}"), AudioCodec::Eac3, "eac3", "eac3", layout, rate, &[]);
    }
}

#[test]
fn dts_frames_match_ffmpeg() {
    for (layout, rate) in [("stereo", 48_000), ("5.1(side)", 48_000), ("stereo", 44_100)] {
        check_against_ffmpeg(&format!("dts {layout} {rate}"), AudioCodec::Dts, "dca", "dts", layout, rate, &[]);
    }
}

#[test]
fn truehd_entry_points_are_the_major_syncs() {
    for (layout, rate) in [("stereo", 48_000), ("5.1(side)", 48_000), ("stereo", 96_000)] {
        check_against_ffmpeg(&format!("truehd {layout} {rate}"), AudioCodec::TrueHd, "truehd", "truehd", layout, rate, &[]);
    }
    let Some((_, data)) = encode("truehd restart points", "truehd", "truehd", "stereo", 48_000, &[]) else { return };
    let index = index_audio(AudioCodec::TrueHd, split_elementary(AudioCodec::TrueHd, &data)).unwrap();
    let entries: Vec<usize> = index.units.iter().enumerate().filter(|u| u.1.entry).map(|u| u.0).collect();
    assert!(entries.len() > 2, "several major syncs in 2 s");
    let between = entries[1] + 3;
    assert!(!index.units[between].entry);
    // a cut after a restart point starts its copy at that restart point; TrueHD needs no pre-roll
    assert_eq!(index.copy_start(between), Some(entries[1]));
    assert_eq!(index.entry_at_or_before(entries[1]), Some(entries[1]));
    // a stream cut before its first major sync cannot start
    assert_eq!(index_audio(AudioCodec::TrueHd, split_elementary(AudioCodec::TrueHd, &data).into_iter().skip(1)), Err(AudioIndexError::NoMajorSync));
}

#[test]
fn frame_codecs_copy_one_packet_before_the_cut() {
    let Some((_, data)) = encode("ac3 pre-roll", "ac3", "ac3", "stereo", 48_000, &[]) else { return };
    let index = index_audio(AudioCodec::Ac3, split_elementary(AudioCodec::Ac3, &data)).unwrap();
    assert_eq!(index.copy_start(10), Some(9));
    assert_eq!(index.copy_start(0), Some(0));
    assert!(index.one_config(0, index.units.len() - 1));
    assert!(!index.one_config(0, index.units.len()));
}

#[test]
fn a_frame_of_the_other_dolby_codec_or_a_torn_packet_is_refused() {
    let Some((_, ac3)) = encode("ac3 torn", "ac3", "ac3", "stereo", 48_000, &[]) else { return };
    let packets = split_elementary(AudioCodec::Ac3, &ac3);
    assert!(matches!(index_audio(AudioCodec::Eac3, packets.iter().copied()), Err(AudioIndexError::BadFrame { packet: 0, .. })));
    let torn = &packets[3][..packets[3].len() - 1];
    let r = index_audio(AudioCodec::Ac3, packets[..3].iter().copied().chain([torn]));
    assert!(matches!(r, Err(AudioIndexError::BadFrame { packet: 3, .. })), "{r:?}");
    let doubled = [packets[0], packets[1]].concat();
    assert_eq!(index_audio(AudioCodec::Ac3, [doubled.as_slice()]).unwrap().units[0].samples, 3072);
}

#[test]
fn hostile_packets_do_not_panic() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let prefixes: [&[u8]; 6] = [
        &[0x0B, 0x77],
        &[0x7F, 0xFE, 0x80, 0x01],
        &[0x64, 0x58, 0x20, 0x25],
        &[0x00, 0x08, 0, 0, 0xF8, 0x72, 0x6F, 0xBA],
        &[0x00, 0x08, 0, 0, 0xF8, 0x72, 0x6F, 0xBB],
        &[],
    ];
    for codec in [AudioCodec::Ac3, AudioCodec::Eac3, AudioCodec::Dts, AudioCodec::TrueHd] {
        for _ in 0..4000 {
            let len = (next() % 64) as usize;
            let mut p = prefixes[(next() % prefixes.len() as u64) as usize].to_vec();
            p.extend((0..len).map(|_| next() as u8));
            let _ = index_audio(codec, [p.as_slice(), p.as_slice()]);
            let _ = split_elementary(codec, &p);
        }
        assert_eq!(index_audio(codec, std::iter::empty()).map(|i| i.units.len()), Ok(0));
    }
}

/// An extension substream of `size` bytes (TS 102 114 §7.4.1), with the short or the long header
/// size fields; the rest of its bytes are not read.
fn exss(size: usize, long: bool) -> Vec<u8> {
    let mut v = vec![0x64, 0x58, 0x20, 0x25];
    let f = (size - 1) as u64;
    // user defined 8 bits, index 2, header size type 1, header size 8 / 12, frame size 16 / 20
    let bits: u64 = if long { (1 << 32) | (0x20 << 20) | f } else { (0x10 << 16) | f };
    let total = if long { 8 + 2 + 1 + 12 + 20 } else { 8 + 2 + 1 + 8 + 16 };
    let word = bits << (64 - total);
    v.extend_from_slice(&word.to_be_bytes()[..6]);
    v.resize(size, 0);
    v
}

#[test]
fn dts_extension_substreams_follow_the_core_or_stand_alone() {
    let Some((_, data)) = encode("dts core for extensions", "dca", "dts", "5.1(side)", 48_000, &[]) else { return };
    let cores = split_elementary(AudioCodec::Dts, &data);
    for long in [false, true] {
        let size = if long { 70_000 } else { 300 };
        let with: Vec<Vec<u8>> = cores.iter().map(|c| [*c, exss(size, long).as_slice()].concat()).collect();
        let joined = with.concat();
        let split = split_elementary(AudioCodec::Dts, &joined);
        assert_eq!(split.len(), cores.len(), "long header {long}");
        let index = index_audio(AudioCodec::Dts, split).unwrap();
        assert!(index.units.iter().all(|u| u.entry && u.samples == 512), "long header {long}");
        assert_eq!(index.configs.len(), 1);
        // a stream of extension substreams only (DTS-HD without a core)
        let alone: Vec<Vec<u8>> = (0..5).map(|_| exss(size, long)).collect();
        let joined = alone.concat();
        assert_eq!(split_elementary(AudioCodec::Dts, &joined).len(), 5);
        let index = index_audio(AudioCodec::Dts, alone.iter().map(Vec::as_slice)).unwrap();
        assert!(index.units.iter().all(|u| u.entry));
    }
    // a core followed by bytes that are not an extension substream is refused
    let bad = [cores[0], &[0u8; 8]].concat();
    assert!(index_audio(AudioCodec::Dts, [bad.as_slice()]).is_err());
}
