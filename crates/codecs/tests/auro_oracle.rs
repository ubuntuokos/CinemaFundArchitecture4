//! Auro-3D (#299) on real carriers: a file whose 24-bit audio carries an Auro-Codec side channel
//! opens with the layout the side channel restores, and every sample of it equals what
//! `cfa4_auro` restores from ffmpeg's bit-exact decode of the carrier, also on random access.
//!
//! The PCM channel tests are Matroska files with `A_MS/ACM` audio, which CFA4's Matroska reader
//! does not take yet (#320); ffmpeg copies their PCM into a WAV first, so they test the WAV
//! path. A raw `.dts` file does not open either (#321); the Amplitude16 track is tested in
//! its Matroska file.
//!
//! The carriers are not in the repository. Point `CFA4_AURO_DIR` at the project's
//! `samples/auro/` folder (Amplitude16 DTS-HD Master Audio, and in `yandex/` the FLAC, PCM and
//! MP4 carriers); without it the test only says it was skipped.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfa4_auro::StreamId;
use cfa4_auro::cfa4::unfold;
use cfa4_media::SharedSource;

/// (file under `CFA4_AURO_DIR`, restored layout, restored channels, carrier channels)
const CASES: &[(&str, &str, u32, usize)] = &[
    ("Amplitude16_Auro-3D_2.35.mkv", "7.1_5H_1T", 14, 8),
    ("yandex/[Auro-3D] Channel Test - Noise {FHD SDR & Auro-3D PCM 9.1}.mkv", "5.1_4H", 10, 6),
    ("yandex/[Auro-3D] Channel Test - Noise {FHD SDR & Auro-3D PCM 11.1}.mkv", "5.1_5H_1T", 12, 6),
    ("yandex/[Auro-3D] Channel Test - Noise {FHD SDR & Auro-3D PCM 13.1}.mkv", "7.1_5H_1T", 14, 8),
    ("yandex/48kHz_4ch_auro_8.0(4.0+4H).flac", "4.0_4H", 8, 4),
    ("yandex/48hHz_6ch_auro_9.1(5.1+4H).flac", "5.1_4H", 10, 6),
    ("yandex/96hHz_8ch_auro_11.1(7.1+4H).flac", "7.1_4H", 12, 8),
    ("yandex/96hHz_8ch_auro_13.1(7.1+5H+T).flac", "7.1_5H_1T", 14, 8),
    ("yandex/Fast_Channel_Identification-Auro13_1-96k-ACEnc-FHD-25fps-H264.mp4", "7.1_5H_1T", 14, 8),
    ("yandex/Day_6011 (Auro 11.1, 48 kHz).mp4", "5.1_5H_1T", 12, 6),
    ("yandex/Deus Ex Machina (Auro-3D 13.1).flac", "7.1_5H_1T", 14, 8),
    ("yandex/[Anna Thorvaldsdottir] Luminance {FHD SDR & Auro-3D 9.1}.mkv", "5.1_4H", 10, 6),
];

/// Seconds of each carrier compared.
const SECONDS: &str = "12";

fn samples_dir() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("CFA4_AURO_DIR").map(PathBuf::from) else {
        cfa4_testkit::skip("auro_oracle", "CFA4_AURO_DIR is not set (the project's samples/auro/ folder)");
        return None;
    };
    Some(dir)
}

/// The file to open for `path`: a WAV copy of the PCM of an `A_MS/ACM` Matroska file.
fn openable(ff: &Path, path: &Path) -> PathBuf {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    if !name.contains("Channel Test - Noise") {
        return path.to_path_buf();
    }
    let wav = format!("auro_{}.wav", name.chars().filter(char::is_ascii_alphanumeric).collect::<String>());
    common::fixture(ff, &wav, &["-i", path.to_str().unwrap(), "-map", "0:a:0", "-c:a", "copy", "-f", "wav"]).unwrap()
}

fn open(path: &Path) -> SharedSource {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let reader = Arc::new(cfa4_media::reader::FileReader::open(path).unwrap());
    cfa4_media::reader::open_reader(&name, reader, &cfa4_codecs::reader_openers(), &cfa4_codecs::openers()).unwrap()
}

/// ffmpeg's bit-exact decode of the first audio stream as planar 24-bit samples.
fn carrier(ff: &Path, path: &Path, channels: usize) -> Vec<Vec<i32>> {
    let raw = common::ffmpeg_out(ff, &["-flags", "+bitexact", "-i", path.to_str().unwrap(), "-map", "0:a:0", "-t", SECONDS, "-f", "s32le", "-"]);
    let words: Vec<i32> = raw.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b) >> 8).collect();
    let mut planes = vec![Vec::with_capacity(words.len() / channels); channels];
    for (i, v) in words.into_iter().enumerate() {
        planes[i % channels].push(v);
    }
    planes
}

fn to_i24(v: f32) -> i32 {
    (v * 8_388_608.0).round() as i32
}

#[test]
fn auro_carriers_open_as_their_restored_layout() {
    let Some(dir) = samples_dir() else { return };
    let ff = cfa4_testkit::require_ffmpeg!();
    for &(file, layout, channels, carrier_channels) in CASES {
        let path = dir.join(file);
        if !path.is_file() {
            cfa4_testkit::skip("auro_oracle", &format!("{} is missing", path.display()));
            continue;
        }
        let path = openable(&ff, &path);
        let src = open(&path);
        let a = src.info().audio().cloned().unwrap();
        assert_eq!(a.channels, channels, "{file}: {}", a.codec);
        assert!(a.codec.contains("Auro-3D"), "{file}: {}", a.codec);

        let planes = carrier(&ff, &path, carrier_channels);
        let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
        let want = unfold(&refs).unwrap_or_else(|| panic!("{file}: no Auro carrier in ffmpeg's decode"));
        assert_eq!(want.detection.original.name(), Some(layout), "{file}");
        assert_eq!(want.decode_errors, 0, "{file}");
        let n = want.channels[0].len();
        // Leave out the last block: the whole-file decode ends there, the source does not.
        let n = n - 4096;

        let ours = src.audio(0, n, a.sample_rate).unwrap().channels;
        assert_eq!(ours.len(), channels as usize, "{file}");
        for (c, (o, w)) in ours.iter().zip(&want.channels).enumerate() {
            let bad = (0..n).find(|&i| to_i24(o[i]) != w[i]);
            assert!(bad.is_none(), "{file}: channel {c} differs from sample {bad:?}");
        }
        // random access on a fresh source: a window in the middle
        let fresh = open(&path);
        let at = n / 3;
        let win = fresh.audio(at as i64, 3000, a.sample_rate).unwrap().channels;
        for (c, (o, w)) in win.iter().zip(&want.channels).enumerate() {
            let bad = (0..3000).find(|&i| to_i24(o[i]) != w[at + i]);
            assert!(bad.is_none(), "{file}: random access, channel {c} differs from sample {bad:?}");
        }
        let names: Vec<&str> = want.speakers.iter().map(|s| StreamId::name(*s)).collect();
        println!("{file}: {} ({} Hz) -> {} bit-exact, random access too", a.codec, a.sample_rate, names.join(" "));
    }
}

/// The PCM channel tests play noise on one loudspeaker at a time. Each restored channel must
/// have a second in which it is loud and every other channel is (nearly) silent: the heights
/// come out on their own channels, not mixed into the bed.
#[test]
fn channel_test_noise_reaches_each_loudspeaker_alone() {
    let Some(dir) = samples_dir() else { return };
    let ff = cfa4_testkit::require_ffmpeg!();
    for n in ["9.1", "11.1", "13.1"] {
        let path = dir.join(format!("yandex/[Auro-3D] Channel Test - Noise {{FHD SDR & Auro-3D PCM {n}}}.mkv"));
        if !path.is_file() {
            cfa4_testkit::skip("auro_oracle", &format!("{} is missing", path.display()));
            continue;
        }
        let src = open(&openable(&ff, &path));
        let a = src.info().audio().cloned().unwrap();
        let secs = (src.info().duration.seconds() as usize).min(75);
        let rate = a.sample_rate as usize;
        let db = |c: &[f32]| 10.0 * (c.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / c.len() as f64 + 1e-20).log10();
        let mut alone = vec![false; a.channels as usize];
        for s in 0..secs {
            let buf = src.audio((s * rate) as i64, rate, a.sample_rate).unwrap();
            let levels: Vec<f64> = buf.channels.iter().map(|c| db(c)).collect();
            for (c, &l) in levels.iter().enumerate() {
                if l > -30.0 && levels.iter().enumerate().all(|(k, &o)| k == c || o < -90.0) {
                    alone[c] = true;
                }
            }
        }
        assert!(alone.iter().all(|&x| x), "Auro {n}: channels heard alone: {alone:?}");
        println!("Auro {n}: each of the {} channels heard alone", alone.len());
    }
}
