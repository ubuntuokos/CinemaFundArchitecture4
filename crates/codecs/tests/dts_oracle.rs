//! DTS audio in Matroska, MP4 and MPEG-TS against ffmpeg: ffmpeg encodes (its `dca` encoder writes
//! the DTS core) and muxes the files, and its fixed-point decode (`-flags +bitexact`) is the
//! oracle: ours must equal it to the sample, also on random access.

mod common;

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
