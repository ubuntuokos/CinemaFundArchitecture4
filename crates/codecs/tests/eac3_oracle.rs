//! E-AC-3 (Dolby Digital Plus) audio in MP4, Matroska and MPEG-TS against ffmpeg: ffmpeg encodes
//! and muxes the files and decodes them as the oracle. Zero-bit mantissas carry decoder-specific
//! dither (A/52 §7.3.4), so the criterion is SNR against ffmpeg's decode, sample-aligned.

mod common;

use common::*;

const STEREO: &str = "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000";
const SURROUND: &str =
    "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1";

/// Worst channel SNR (dB) of `ours` (planar) against ffmpeg's interleaved `want`, with ours
/// shifted by `lag` samples, over the samples both cover past the first 1000.
fn snr(ours: &[Vec<f32>], want: &[f32], ch: usize, lag: i64) -> f64 {
    let n = want.len() / ch;
    (0..ch)
        .map(|c| {
            let (mut sig, mut err) = (0f64, 0f64);
            for i in 1000..n - 1000 {
                let Some(&a) = ours[c].get((i as i64 + lag) as usize) else { continue };
                let (a, b) = (a as f64, want[i * ch + c] as f64);
                sig += b * b;
                err += (a - b) * (a - b);
            }
            10.0 * (sig / err.max(1e-30)).log10()
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn eac3_in_mp4_matroska_and_transport_streams() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let cases: [(&str, &str, u32, &[&str]); 4] = [
        ("eac3_51.mp4", SURROUND, 6, &["-b:a", "384k"]),
        ("eac3_51.mkv", SURROUND, 6, &["-b:a", "384k"]),
        ("eac3_stereo.mov", STEREO, 2, &["-b:a", "192k"]),
        ("eac3_stereo.ts", STEREO, 2, &["-b:a", "192k"]),
    ];
    for (name, input, channels, enc) in cases {
        let args: Vec<&str> = ["-f", "lavfi", "-i", input, "-t", "2", "-c:a", "eac3"].into_iter().chain(enc.iter().copied()).collect();
        let f = fixture(&ff, name, &args).unwrap_or_else(|| panic!("could not write {name}"));
        let src = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
        let a = src.info().audio().cloned().unwrap();
        assert_eq!((a.sample_rate, a.channels), (48_000, channels), "{name}: {}", a.codec);
        let want = ffmpeg_audio_f32(&ff, &f, &[]);
        let ch = channels as usize;
        let n = want.len() / ch;
        // MPEG-TS audio starts at its first PTS relative to the presentation start
        let start = match name.ends_with(".ts") {
            true => cfa4_codecs::MpegSource::open(name, bytes(&f)).unwrap().audio_extent().map_or(0, |e| e.0),
            false => 0,
        };
        let ours = src.audio(start, n, 48_000).unwrap().channels;
        assert_eq!(ours.len(), ch, "{name}");
        // Matroska stores the encoder's 256-sample priming as CodecDelay, which we trim exactly
        // (checked to the sample against the encoded signal in `matroska_audio_oracle`). Whether
        // ffmpeg's CLI trims it too depends on its version (9.x does; 2023-24 builds keep the
        // priming samples), so for .mkv its decode may start 256 samples earlier than ours.
        // Elsewhere the samples line up exactly, whatever the version.
        let lags: Vec<i64> = match name.ends_with(".mkv") {
            true => (-32..=32).chain(-256 - 32..=-256 + 32).collect(),
            false => (-32..=32).collect(),
        };
        let (worst, lag) = lags.into_iter().map(|lag| (snr(&ours, &want, ch, lag), lag)).fold((f64::MIN, 0), |a, b| if b.0 > a.0 { b } else { a });
        let ok = if name.ends_with(".mkv") { lag == 0 || lag == -256 } else { lag == 0 };
        assert!(ok, "{name}: decode is {lag} samples off ffmpeg's");
        // a window read on a fresh source (random access) gives the same samples
        let fresh = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
        let at = n / 3;
        let win = fresh.audio(start + at as i64, 4800, 48_000).unwrap().channels;
        let mut seek_err = 0f32;
        for c in 0..ch {
            for (k, v) in win[c].iter().enumerate() {
                seek_err = seek_err.max((v - ours[c][at + k]).abs());
            }
        }
        println!(
            "{name}: {} ({} Hz, {ch} ch), worst channel SNR vs ffmpeg {worst:.1} dB (lag {lag}), random access max diff {seek_err:.1e}",
            a.codec, a.sample_rate
        );
        assert!(worst >= 50.0, "{name}: SNR {worst:.1} dB");
        assert!(seek_err <= 1e-6, "{name}: random-access audio differs by {seek_err}");
    }
}

/// E-AC-3 7.1 (a 5.1 independent substream and a dependent substream carrying Ls, Rs, Lrs and Rrs)
/// in MP4 and Matroska: decoded as eight channels, L R C LFE Ls Rs Lrs Rrs. No free encoder writes
/// dependent substreams, so ffmpeg's 2/2 syncframes are rewritten into one
/// (`cfa4_ac3::make_dependent_substream`) and ffmpeg muxes the result.
#[test]
fn eac3_71_dependent_substreams_in_mp4_and_matroska() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let quad = "aevalsrc=exprs=0.25*sin(2*PI*900*t)|0.25*sin(2*PI*1000*t)|0.25*sin(2*PI*1100*t)|0.25*sin(2*PI*1200*t):s=48000:c=quad";
    let enc = |name: &str, input: &str, rate: &str| {
        let f = fixture(&ff, name, &["-f", "lavfi", "-i", input, "-t", "2", "-c:a", "eac3", "-b:a", rate, "-f", "eac3"]).unwrap();
        std::fs::read(f).unwrap()
    };
    let (core, extra) = (enc("eac3_71_core.eac3", SURROUND, "384k"), enc("eac3_71_extra.eac3", quad, "256k"));
    let syncframes = |d: &[u8]| {
        let mut out = Vec::new();
        let mut p = 0;
        while let Ok(h) = cfa4_ac3::parse_header(&d[p..]) {
            out.push(d[p..p + h.frame_bytes].to_vec());
            p += h.frame_bytes;
        }
        out
    };
    let raw: Vec<u8> = syncframes(&core)
        .into_iter()
        .zip(syncframes(&extra))
        .flat_map(|(c, e)| c.into_iter().chain(cfa4_ac3::make_dependent_substream(&e, 0x1A00).unwrap()))
        .collect();
    let raw_path = dir().join("eac3_71.eac3");
    std::fs::write(&raw_path, raw).unwrap();
    let raw_arg = raw_path.to_str().unwrap();
    // ffmpeg orders 7.1 as FL FR FC LFE BL BR SL SR: E-AC-3's Lrs / Rrs are its back channels
    const FFMPEG: [usize; 8] = [0, 1, 2, 3, 6, 7, 4, 5];
    for name in ["eac3_71.mp4", "eac3_71.mkv"] {
        let f = dir().join(name);
        let _ = std::fs::remove_file(&f);
        assert!(fixture(&ff, name, &["-i", raw_arg, "-c", "copy"]).is_some(), "{name}");
        let src = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
        let a = src.info().audio().cloned().unwrap();
        let want = ffmpeg_audio_f32(&ff, &f, &[]);
        let n = want.len() / 8;
        let ours = src.audio(0, n, 48_000).unwrap().channels;
        assert_eq!(ours.len(), 8, "{name}: decoded channels (container says {})", a.channels);
        let reordered: Vec<f32> = (0..n).flat_map(|i| FFMPEG.iter().map(move |&c| (i, c))).map(|(i, c)| want[i * 8 + c]).collect();
        let worst = snr(&ours, &reordered, 8, 0);
        println!("{name}: {} ({} Hz, container {} ch), worst channel SNR vs ffmpeg {worst:.1} dB", a.codec, a.sample_rate, a.channels);
        assert!(worst >= 50.0, "{name}: SNR {worst:.1} dB");
    }
}
