//! Our Opus encoder against libopus (through ffmpeg, as an external oracle only; skipped when
//! ffmpeg with libopus is unavailable).
//!
//! Streams from [`cfa4_opus::Encoder`] are written as Ogg Opus, decoded by libopus and by our
//! decoder (which must agree to float precision, since the encoder output is a normal CELT
//! stream), and their fidelity to the source is compared with libopus's own encoder at the same
//! bit rate.

mod common;

use std::process::Command;

use cfa4_ogg::OggWriter;
use cfa4_opus::{Encoder, StreamDecoder};

fn has_libopus(ff: &std::path::Path) -> bool {
    Command::new(ff).args(["-hide_banner", "-encoders"]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains("libopus")).unwrap_or(false)
}

fn read_f32(path: &std::path::Path) -> Vec<f32> {
    std::fs::read(path).unwrap().as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

fn opus_tags() -> Vec<u8> {
    let vendor = b"cfa4-opus";
    let mut t = b"OpusTags".to_vec();
    t.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    t.extend_from_slice(vendor);
    t.extend_from_slice(&0u32.to_le_bytes());
    t
}

/// Encodes `pcm` (interleaved) as Ogg Opus; returns the file and our own decode (pre-skip and
/// end trimming applied).
fn encode_ogg(pcm: &[f32], channels: usize, bitrate: u32) -> (Vec<u8>, Vec<f32>) {
    let mut enc = Encoder::new(channels, bitrate).unwrap();
    let mut dec = StreamDecoder::new(48_000, channels).unwrap();
    let mut w = OggWriter::new(Vec::new(), 0x0CFA_0004);
    let head = enc.head(48_000);
    w.write_packet(&head.to_bytes(), 0).unwrap();
    w.flush_page().unwrap();
    w.write_packet(&opus_tags(), 0).unwrap();
    w.flush_page().unwrap();
    let n = pcm.len() / channels;
    let fs = Encoder::FRAME_SIZE;
    // Enough frames to flush the encoder delay.
    let frames = (n + Encoder::LOOKAHEAD).div_ceil(fs);
    let total = (Encoder::LOOKAHEAD + n) as u64;
    let mut ours = Vec::new();
    let mut frame = vec![0f32; fs * channels];
    for f in 0..frames {
        frame.fill(0.0);
        let a = (f * fs * channels).min(pcm.len());
        let b = ((f + 1) * fs * channels).min(pcm.len());
        frame[..b - a].copy_from_slice(&pcm[a..b]);
        let pkt = enc.encode(&frame);
        dec.decode_interleaved(Some(&pkt), &mut ours).unwrap();
        assert_eq!(dec.final_range(), enc.final_range());
        let granule = (((f + 1) * fs) as u64).min(total);
        w.write_packet(&pkt, granule).unwrap();
    }
    let file = w.finish().unwrap();
    let ours = ours[Encoder::LOOKAHEAD * channels..(Encoder::LOOKAHEAD + n) * channels].to_vec();
    (file, ours)
}

fn libopus_decode(ff: &std::path::Path, file: &[u8], name: &str) -> Vec<f32> {
    let dir = common::fixtures();
    let opus = dir.join(format!("{name}.opus"));
    let dec = dir.join(format!("{name}.dec.f32"));
    std::fs::write(&opus, file).unwrap();
    let st = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-c:a", "libopus", "-i"])
        .arg(&opus)
        .args(["-f", "f32le"])
        .arg(&dec)
        .status()
        .unwrap();
    assert!(st.success(), "libopus failed to decode {name}");
    let out = read_f32(&dec);
    let _ = std::fs::remove_file(&opus);
    let _ = std::fs::remove_file(&dec);
    out
}

/// Source → libopus encoder at `bitrate` → libopus decoder.
fn libopus_round_trip(ff: &std::path::Path, pcm: &[f32], channels: usize, bitrate: u32, name: &str) -> Vec<f32> {
    let dir = common::fixtures();
    let raw = dir.join(format!("{name}.src.f32"));
    let opus = dir.join(format!("{name}.ref.opus"));
    std::fs::write(&raw, pcm.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let layout = if channels == 1 { "mono" } else { "stereo" };
    let st = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "f32le", "-ar", "48000", "-ch_layout", layout, "-i"])
        .arg(&raw)
        .args(["-c:a", "libopus", "-b:a", &bitrate.to_string(), "-vbr", "off"])
        .arg(&opus)
        .status()
        .unwrap();
    assert!(st.success());
    let out = libopus_decode(ff, &std::fs::read(&opus).unwrap(), &format!("{name}.ref"));
    let _ = std::fs::remove_file(&raw);
    let _ = std::fs::remove_file(&opus);
    out
}

#[test]
fn libopus_decodes_our_streams() {
    let Some(ff) = common::ffmpeg() else {
        eprintln!("ffmpeg not found; skipping");
        return;
    };
    if !has_libopus(&ff) {
        eprintln!("ffmpeg without libopus; skipping");
        return;
    }
    let n = 48_000 * 3 + 123;
    let mut failures = Vec::new();
    for (channels, bitrate, signal) in [
        (1usize, 48_000u32, common::speech as fn(usize, u64) -> Vec<f32>),
        (1, 96_000, common::music),
        (2, 64_000, common::music),
        (2, 128_000, common::music),
        (2, 256_000, common::music),
        (2, 160_000, common::speech),
    ] {
        let mut pcm = vec![0f32; n * channels];
        for c in 0..channels {
            let s = signal(n, 11 + c as u64);
            for i in 0..n {
                let j = i.saturating_sub(c * 37);
                pcm[i * channels + c] = s[j] * (1.0 - 0.1 * c as f32);
            }
        }
        let name = format!("enc_{channels}ch_{}k", bitrate / 1000);
        let (file, ours) = encode_ogg(&pcm, channels, bitrate);
        let theirs = libopus_decode(&ff, &file, &name);
        assert_eq!(theirs.len(), ours.len(), "{name}: libopus decoded length (pre-skip + end trimming)");
        let agree = common::snr_db(&theirs, &ours);
        let fidelity = common::snr_db(&pcm, &theirs);
        let reference = libopus_round_trip(&ff, &pcm, channels, bitrate, &name);
        let ref_fidelity = common::snr_db(&pcm, &reference);
        let q = common::opus_quality(&pcm, &theirs, channels, 48_000);
        let q_ref = common::opus_quality(&pcm, &reference, channels, 48_000);
        let ok = agree >= 50.0 && fidelity >= ref_fidelity - 6.0;
        eprintln!(
            "{name:16} decoders agree {agree:6.1} dB | source SNR ours {fidelity:5.1} dB, libopus {ref_fidelity:5.1} dB | quality ours {q:7.1}, libopus {q_ref:7.1} {}",
            if ok { "ok" } else { "FAIL" }
        );
        if !ok {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
