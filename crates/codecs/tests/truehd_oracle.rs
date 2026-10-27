//! Dolby TrueHD audio in Matroska and MP4 against ffmpeg: ffmpeg encodes and muxes the files and
//! decodes them as the oracle. TrueHD is lossless, so the samples must be equal (24-bit integers
//! in f32 are exact), and a window read on a fresh source (random access, which pre-rolls to a
//! major sync) must give the same samples as the continuous read.

mod common;

use common::*;

const SURROUND: &str =
    "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1(side)";
const STEREO: &str = "anoisesrc=color=pink:sample_rate=96000:amplitude=0.3:seed=5,pan=stereo|c0=c0|c1=-0.5*c0";

#[test]
fn truehd_in_matroska_and_mp4_is_bit_exact() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let cases: [(&str, &str, u32, u32); 3] =
        [("truehd_51.mkv", SURROUND, 48_000, 6), ("truehd_51.mp4", SURROUND, 48_000, 6), ("truehd_stereo_96k.mkv", STEREO, 96_000, 2)];
    for (name, input, rate, channels) in cases {
        let args = ["-f", "lavfi", "-i", input, "-t", "2", "-af", "aformat=s32", "-c:a", "truehd", "-strict", "-2"];
        let f = fixture(&ff, name, &args).unwrap_or_else(|| panic!("could not write {name}"));
        let src = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
        let a = src.info().audio().cloned().unwrap();
        assert_eq!((a.sample_rate, a.channels), (rate, channels), "{name}: {}", a.codec);
        let want = ffmpeg_audio_f32(&ff, &f, &[]);
        let ch = channels as usize;
        let n = want.len() / ch;
        let ours = src.audio(0, n, rate).unwrap().channels;
        assert_eq!(ours.len(), ch, "{name}");
        let mut bad = 0;
        for c in 0..ch {
            bad += (0..n).filter(|&i| ours[c][i] != want[i * ch + c]).count();
        }
        assert_eq!(bad, 0, "{name}: {bad} of {} samples differ from ffmpeg", n * ch);
        // random access well past the first major sync, and right at the start
        for at in [n / 3, 7] {
            let fresh = cfa4_codecs::open_bytes(name, bytes(&f)).unwrap();
            let win = fresh.audio(at as i64, 4800, rate).unwrap().channels;
            for c in 0..ch {
                for (k, v) in win[c].iter().enumerate() {
                    assert_eq!(*v, ours[c][at + k], "{name}: random access at {at}, channel {c}, sample {k}");
                }
            }
        }
        println!("{name}: {} ({rate} Hz, {ch} ch), {n} samples bit-exact, random access exact", a.codec);
    }
}
