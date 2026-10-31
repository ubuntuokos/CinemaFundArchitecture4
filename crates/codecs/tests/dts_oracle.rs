//! DTS audio in Matroska, MP4 and MPEG-TS against ffmpeg: ffmpeg encodes (its `dca` encoder writes
//! the DTS core) and muxes the files, and its fixed-point decode (`-flags +bitexact`) is the
//! oracle: ours must equal it to the sample, also on random access.

mod common;

use cfa4_media::MediaSource;
use common::*;

const STEREO: &str = "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000";
const SURROUND: &str =
    "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1";

#[test]
fn dts_in_matroska_mp4_and_transport_streams() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let cases: [(&str, &str, u32); 4] = [("dts_51.mkv", SURROUND, 6), ("dts_stereo.mkv", STEREO, 2), ("dts_51.mp4", SURROUND, 6), ("dts_51.ts", SURROUND, 6)];
    for (name, input, channels) in cases {
        let args = ["-f", "lavfi", "-i", input, "-t", "2", "-c:a", "dca", "-strict", "-2"];
        let f = fixture(&ff, name, &args).unwrap_or_else(|| panic!("could not write {name}"));
        let src = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
        let a = src.info().audio().cloned().unwrap();
        assert_eq!((a.sample_rate, a.channels), (48_000, channels), "{name}: {}", a.codec);
        let path = f.to_str().unwrap();
        let want: Vec<i32> = ffmpeg_out(&ff, &["-flags", "+bitexact", "-i", path, "-map", "0:a:0", "-f", "s32le", "-"])
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| i32::from_le_bytes(*b))
            .collect();
        let ch = channels as usize;
        let n = want.len() / ch;
        let start = match name.ends_with(".ts") {
            true => cfa4_codecs::MpegSource::open(name, bytes(&f)).unwrap().audio_extent().map_or(0, |e| e.0),
            false => 0,
        };
        let ours = src.audio(start, n, 48_000).unwrap().channels;
        assert_eq!(ours.len(), ch, "{name}");
        for c in 0..ch {
            for i in 0..n {
                let v = (f64::from(ours[c][i]) * 2_147_483_648.0) as i64;
                assert_eq!(v, i64::from(want[i * ch + c]), "{name}: channel {c} sample {i}");
            }
        }
        // a window read on a fresh source (random access) gives the same samples once the
        // filter banks have converged: the source primes the decoder with the frame before
        let fresh = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
        let at = n / 3;
        let win = fresh.audio(start + at as i64, 4800, 48_000).unwrap().channels;
        let mut seek_err = 0f32;
        for c in 0..ch {
            for (k, v) in win[c].iter().enumerate().skip(2048) {
                seek_err = seek_err.max((v - ours[c][at + k]).abs());
            }
        }
        println!("{name}: {} ({} Hz, {ch} ch) bit-exact against ffmpeg, random access max diff {seek_err:.1e}", a.codec, a.sample_rate);
        assert!(seek_err <= 1e-6, "{name}: random-access audio differs by {seek_err}");
    }
}

/// #321: a raw DTS elementary stream (`.dts`) opens as audio and equals ffmpeg's decode to the
/// sample; so does a real DTS-HD Master Audio `.dts` when `CFA4_DTSHD_SAMPLE` names one.
#[test]
fn raw_dts_streams_open_as_audio() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let f = fixture(&ff, "dts_51_raw.dts", &["-f", "lavfi", "-i", SURROUND, "-t", "2", "-c:a", "dca", "-strict", "-2", "-f", "dts"])
        .expect("could not write dts_51_raw.dts");
    let mut paths = vec![f];
    if let Some(p) = std::env::var_os("CFA4_DTSHD_SAMPLE").map(std::path::PathBuf::from).filter(|p| p.extension().is_some_and(|e| e == "dts" || e == "dtshd")) {
        paths.push(p);
    }
    for f in paths {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        // the plain stream: an Auro-3D carrier opens as its restored layout (auro_oracle)
        let src = cfa4_codecs::audio::AudioFileSource::decode(&name, bytes(&f)).unwrap();
        let a = src.info().audio().cloned().unwrap();
        assert_eq!(src.info().container, "DTS", "{name}");
        assert!(cfa4_codecs::open_bytes(&name, bytes(&f)).unwrap().info().audio().unwrap().codec.starts_with(&a.codec), "{name}");
        let path = f.to_str().unwrap();
        let want: Vec<i32> = ffmpeg_out(&ff, &["-flags", "+bitexact", "-i", path, "-map", "0:a:0", "-t", "10", "-f", "s32le", "-"])
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| i32::from_le_bytes(*b))
            .collect();
        let ch = a.channels as usize;
        let n = want.len() / ch;
        let ours = src.audio(0, n, a.sample_rate).unwrap().channels;
        assert_eq!(ours.len(), ch, "{name}");
        for c in 0..ch {
            let bad = (0..n).find(|&i| (f64::from(ours[c][i]) * 2_147_483_648.0) as i64 != i64::from(want[i * ch + c]));
            assert!(bad.is_none(), "{name}: channel {c} differs from sample {bad:?}");
        }
        println!("{name}: {} ({} Hz, {ch} ch) bit-exact against ffmpeg", a.codec, a.sample_rate);
    }
    // damage: no frames is an error, never a panic
    assert!(cfa4_codecs::open_bytes("junk.dts", vec![0x7Fu8; 4096].into()).is_err());
    assert!(cfa4_codecs::open_bytes("empty.dts", Vec::new().into()).is_err());
}

/// #321 review: a raw stream whose layout changes part way (5.1 then stereo) keeps the first
/// layout bit-exact and the stereo part's duration as silence, rather than replacing the audio
/// before the change; and a stream whose frames all fail to decode reports the decode error.
#[test]
fn raw_dts_layout_change_keeps_timing() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let surround = fixture(&ff, "dts_51_raw.dts", &["-f", "lavfi", "-i", SURROUND, "-t", "2", "-c:a", "dca", "-strict", "-2", "-f", "dts"])
        .expect("could not write dts_51_raw.dts");
    let stereo = fixture(
        &ff,
        "dts_20_raw.dts",
        &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "1", "-ac", "2", "-c:a", "dca", "-strict", "-2", "-f", "dts"],
    )
    .expect("could not write dts_20_raw.dts");
    let decode = |p: &std::path::Path, ch: usize| -> Vec<Vec<i32>> {
        let raw = ffmpeg_out(&ff, &["-flags", "+bitexact", "-i", p.to_str().unwrap(), "-f", "s32le", "-"]);
        let words: Vec<i32> = raw.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect();
        (0..ch).map(|c| words.iter().skip(c).step_by(ch).copied().collect()).collect()
    };
    let want = decode(&surround, 6);
    let stereo_len = decode(&stereo, 2)[0].len();
    let mut joined = std::fs::read(&surround).unwrap();
    joined.extend(std::fs::read(&stereo).unwrap());
    let src = cfa4_codecs::audio::AudioFileSource::decode("joined.dts", joined.into()).unwrap();
    let a = src.info().audio().cloned().unwrap();
    assert_eq!(a.channels, 6);
    let n = want[0].len();
    let ours = src.audio(0, n + stereo_len, a.sample_rate).unwrap().channels;
    for c in 0..6 {
        let bad = (0..n).find(|&i| (f64::from(ours[c][i]) * 2_147_483_648.0) as i64 != i64::from(want[c][i]));
        assert!(bad.is_none(), "channel {c} differs from sample {bad:?}");
        assert!(ours[c][n..].iter().all(|&v| v == 0.0), "channel {c}: the stereo part is not silence");
    }
    let expected = (n + stereo_len) as i64;
    let got = (src.info().duration.seconds() * f64::from(a.sample_rate)).round() as i64;
    assert!((got - expected).abs() <= 1, "duration {got} samples, want {expected}");

    // a DTS-HD file wrapper: only its STRMDATA chunk is the stream, not the frame in BLACKOUT
    let plain = std::fs::read(&surround).unwrap();
    let chunk = |id: &[u8; 8], body: &[u8]| [&id[..], &(body.len() as u64).to_be_bytes(), body].concat();
    let wrapped = [chunk(b"DTSHDHDR", &[0; 8]), chunk(b"BLACKOUT", &plain[..2048]), chunk(b"STRMDATA", &plain), chunk(b"TIMECODE", &[0; 4])].concat();
    let w = cfa4_codecs::audio::AudioFileSource::decode("wrapped.dtshd", wrapped.into()).unwrap();
    assert_eq!(w.info().duration, cfa4_codecs::audio::AudioFileSource::decode("plain.dts", plain.into()).unwrap().info().duration);

    // core syncwords with headers that do not parse: an error, not an empty success
    let mut broken = Vec::new();
    for _ in 0..64 {
        broken.extend([0x7F, 0xFE, 0x80, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }
    let Err(e) = cfa4_codecs::audio::AudioFileSource::decode("broken.dts", broken.into()) else { panic!("broken.dts opened") };
    println!("broken.dts: {e}");
}

/// A real DTS-HD Master Audio track in a container (`.m2ts` / `.mkv`), when
/// `CFA4_DTSHD_CONTAINER_SAMPLE` names one (none is committed, #131): bit-exact against ffmpeg
/// from the start, and a window read on a fresh source equals the straight decode.
#[test]
fn a_real_dts_hd_track_in_a_container() {
    let Some(path) = std::env::var_os("CFA4_DTSHD_CONTAINER_SAMPLE") else {
        eprintln!("skipped: set CFA4_DTSHD_CONTAINER_SAMPLE to a file with a DTS-HD Master Audio track");
        return;
    };
    let ff = cfa4_testkit::require_ffmpeg!();
    let path = std::path::PathBuf::from(path);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sample").to_string();
    let data: std::sync::Arc<[u8]> = std::fs::read(&path).unwrap().into();
    let src = cfa4_codecs::open_bytes(&name, data.clone()).unwrap();
    let a = src.info().audio().cloned().unwrap();
    let want: Vec<i32> = ffmpeg_out(&ff, &["-flags", "+bitexact", "-i", path.to_str().unwrap(), "-map", "0:a:0", "-f", "s32le", "-"])
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| i32::from_le_bytes(*b))
        .collect();
    let ch = a.channels as usize;
    let rate = a.sample_rate;
    // a cut file ends inside a frame ffmpeg may still decode in part: compare what both have
    // (a quarter of a short sample, such as FATE's 64 ms streams, is held back instead)
    let total = want.len() / ch;
    let n = total - 4096.min(total / 4);
    let start = match name.ends_with(".m2ts") || name.ends_with(".ts") {
        true => cfa4_codecs::MpegSource::open(&name, data.clone()).unwrap().audio_extent().map_or(0, |e| e.0),
        false => 0,
    };
    let ours = src.audio(start, n, rate).unwrap().channels;
    assert_eq!(ours.len(), ch);
    for c in 0..ch {
        for i in 0..n {
            let v = (f64::from(ours[c][i]) * 2_147_483_648.0) as i64;
            assert_eq!(v, i64::from(want[i * ch + c]), "{name}: channel {c} sample {i}");
        }
    }
    let fresh = cfa4_codecs::open_bytes(&name, data).unwrap();
    let at = n / 3;
    let len = 4800.min(n - at);
    let win = fresh.audio(start + at as i64, len, rate).unwrap().channels;
    let mut seek_err = 0f32;
    for c in 0..ch {
        for (k, v) in win[c].iter().enumerate().skip(2048.min(len / 2)) {
            seek_err = seek_err.max((v - ours[c][at + k]).abs());
        }
    }
    println!("{name}: {} ({rate} Hz, {ch} ch) bit-exact against ffmpeg over {n} samples, random access max diff {seek_err:.1e}", a.codec);
    assert!(seek_err == 0.0, "{name}: random-access audio differs by {seek_err}");
}
