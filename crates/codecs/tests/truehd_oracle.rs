//! Dolby TrueHD audio in Matroska and MP4 against ffmpeg: ffmpeg encodes and muxes the files and
//! decodes them as the oracle. TrueHD is lossless, so the samples must be equal (24-bit integers
//! in f32 are exact), and a window read on a fresh source (random access, which pre-rolls to a
//! major sync) must give the same samples as the continuous read.

mod common;

use cfa4_media::MediaSource;
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

/// TrueHD as Blu-ray carries it: a BDAV transport stream (192-byte packets) whose TrueHD PID
/// interleaves the AC-3 core frames between the access units. The units must be the TrueHD ones
/// only, decoding bit-exact to ffmpeg's decode of the raw stream, with random access exact.
/// ffmpeg's encoder stops at 5.1, so this is 5.1 (7.1 needs a disc sample).
#[test]
fn truehd_in_bdav_with_an_ac3_core_is_bit_exact() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let thd_args = ["-f", "lavfi", "-i", SURROUND, "-t", "2", "-af", "aformat=s32", "-c:a", "truehd", "-strict", "-2", "-f", "truehd"];
    let thd = fixture(&ff, "truehd_51.thd", &thd_args).expect("could not write truehd_51.thd");
    let ac3_args = ["-f", "lavfi", "-i", SURROUND, "-t", "2", "-c:a", "ac3", "-b:a", "640k", "-f", "ac3"];
    let ac3 = fixture(&ff, "truehd_51_core.ac3", &ac3_args).expect("could not write truehd_51_core.ac3");
    let (thd_bytes, ac3_bytes) = (std::fs::read(&thd).unwrap(), std::fs::read(&ac3).unwrap());
    let aus = split(&thd_bytes, |b| ((usize::from(b[0] & 0x0F) << 8) | usize::from(b[1])) * 2);
    let frames = split(&ac3_bytes, |b| cfa4_mpegts::frame_bytes(&cfa4_mpegts::Codec::Ac3, b).unwrap());
    // an AC-3 frame (1536 samples) is due before the TrueHD access unit (40 samples) at its time
    let mut pes = Vec::new();
    let (mut a, mut f) = (0, 0);
    while a < aus.len() {
        if f < frames.len() && f * 1536 <= a * 40 {
            pes.push((f as u64 * 2880, frames[f].to_vec()));
            f += 1;
        } else {
            let first = a;
            let mut data = Vec::new();
            while a < aus.len() && !(f < frames.len() && f * 1536 <= a * 40) && data.len() < 4000 {
                data.extend_from_slice(aus[a]);
                a += 1;
            }
            pes.push((first as u64 * 75, data));
        }
    }
    let m2ts = bdav(&pes);
    let name = "truehd_51_ac3_core.m2ts";
    let src = cfa4_codecs::MpegSource::open(name, m2ts.clone().into()).unwrap();
    let a = src.info().audio().cloned().unwrap();
    assert_eq!((a.codec.as_str(), a.sample_rate, a.channels), ("Dolby TrueHD", 48_000, 6));
    let want = ffmpeg_audio_f32(&ff, &thd, &[]);
    let n = want.len() / 6;
    assert_eq!(n, aus.len() * 40);
    let (start, len) = src.audio_extent().unwrap();
    assert_eq!(len as usize, n, "TrueHD samples only, the AC-3 core left out");
    let ours = src.audio(start, n, 48_000).unwrap().channels;
    let bad: usize = (0..6).map(|c| (0..n).filter(|&i| ours[c][i] != want[i * 6 + c]).count()).sum();
    assert_eq!(bad, 0, "{bad} of {} samples differ from ffmpeg", n * 6);
    for at in [n / 2 + 13, 5] {
        let fresh = cfa4_codecs::open_bytes(name, m2ts.clone().into()).unwrap();
        let win = fresh.audio(start + at as i64, 4800, 48_000).unwrap().channels;
        for c in 0..6 {
            assert!(win[c].iter().zip(&ours[c][at..]).all(|(x, y)| x == y), "random access at {at}, channel {c}");
        }
    }
    println!("{name}: {} TrueHD access units and {} AC-3 core frames, {n} samples bit-exact", aus.len(), frames.len());
}

/// `data` cut into the frames `len` gives from each one's header.
fn split(data: &[u8], len: impl Fn(&[u8]) -> usize) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 8 <= data.len() {
        let l = len(&data[p..]);
        assert!(l > 0 && p + l <= data.len());
        out.push(&data[p..p + l]);
        p += l;
    }
    out
}

/// A BDAV stream: PAT, a PMT with the `HDMV` registration and one stream_type 0x83 PID, and the
/// PES packets (90 kHz PTS, payload) on it.
fn bdav(pes: &[(u64, Vec<u8>)]) -> Vec<u8> {
    const PID: u16 = 0x1100;
    let mut out = Vec::new();
    let mut cc = 0u8;
    let packet = |out: &mut Vec<u8>, pid: u16, start: bool, payload: &[u8], cc: &mut u8| {
        out.extend_from_slice(&[0; 4]); // TP_extra_header
        out.extend_from_slice(&[0x47, (u8::from(start) << 6) | (pid >> 8) as u8, pid as u8]);
        let stuff = 184 - payload.len();
        if stuff == 0 {
            out.push(0x10 | *cc);
        } else {
            out.push(0x30 | *cc);
            out.push(stuff as u8 - 1);
            if stuff > 1 {
                out.push(0);
                out.extend(std::iter::repeat_n(0xFF, stuff - 2));
            }
        }
        out.extend_from_slice(payload);
        *cc = (*cc + 1) & 15;
    };
    let section = |table: u8, ext: u16, body: &[u8]| {
        let mut s = vec![table, 0xB0 | ((body.len() + 9) >> 8) as u8, (body.len() + 9) as u8, (ext >> 8) as u8, ext as u8, 0xC1, 0, 0];
        s.extend_from_slice(body);
        let crc = crc32(&s);
        s.extend_from_slice(&crc.to_be_bytes());
        let mut p = vec![0];
        p.extend_from_slice(&s);
        p.resize(184, 0xFF);
        p
    };
    let pat = section(0, 1, &[0, 1, 0xE1, 0x00]);
    let pmt = section(2, 1, &[0xE0 | (PID >> 8) as u8, PID as u8, 0xF0, 6, 5, 4, b'H', b'D', b'M', b'V', 0x83, 0xE0 | (PID >> 8) as u8, PID as u8, 0xF0, 0]);
    let mut tables = [0u8; 2];
    for (k, (pts, data)) in pes.iter().enumerate() {
        if k % 64 == 0 {
            packet(&mut out, 0, true, &pat, &mut tables[0]);
            packet(&mut out, 0x100, true, &pmt, &mut tables[1]);
        }
        let pts = pts + 90_000;
        let mut p = vec![0, 0, 1, 0xBD];
        p.extend_from_slice(&((data.len() + 8) as u16).to_be_bytes());
        p.extend_from_slice(&[0x80, 0x80, 5]);
        p.extend_from_slice(&[
            0x21 | ((pts >> 29) & 0x0E) as u8,
            (pts >> 22) as u8,
            0x01 | ((pts >> 14) & 0xFE) as u8,
            (pts >> 7) as u8,
            0x01 | ((pts << 1) & 0xFE) as u8,
        ]);
        p.extend_from_slice(data);
        for (i, chunk) in p.chunks(184).enumerate() {
            packet(&mut out, PID, i == 0, chunk, &mut cc);
        }
    }
    out
}

/// CRC-32/MPEG-2 of a PSI section.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}
