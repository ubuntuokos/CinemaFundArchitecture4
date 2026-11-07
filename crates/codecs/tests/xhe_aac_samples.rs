//! xHE-AAC (USAC) in MP4 through the import path: the Fraunhofer IIS MPEG-D DRC test stream
//! `Test_PRL-20.mp4` (-20 LKFS programme) against its reference decode, which is normalized to
//! -16 LKFS (+4 dB). The stream is not part of the repository: set `CFA4_SAMPLES` to the sample
//! folder (default `/mnt/project-files/samples`); the test is skipped when it is absent.

use std::path::PathBuf;

fn samples() -> PathBuf {
    std::env::var_os("CFA4_SAMPLES").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/mnt/project-files/samples"))
}

/// 16-bit mono PCM WAV samples.
fn wav_mono(data: &[u8]) -> Vec<f32> {
    let mut o = 12;
    while o + 8 <= data.len() {
        let size = u32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()) as usize;
        if &data[o..o + 4] == b"data" {
            let end = (o + 8 + size).min(data.len());
            return data[o + 8..end].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
        }
        o += 8 + size + (size & 1);
    }
    Vec::new()
}

#[test]
fn xhe_aac_mp4_imports_at_its_own_level() {
    let root = samples().join("mpegd-drc");
    let (Ok(mp4), Ok(wav)) = (std::fs::read(root.join("Test_PRL-20.mp4")), std::fs::read(root.join("Reference_Test_PRL_TL-16.wav"))) else {
        eprintln!("skipped: xHE-AAC samples not found");
        return;
    };
    let src = cfa4_codecs::open_bytes("Test_PRL-20.mp4", mp4.into()).unwrap();
    let info = src.info().audio().unwrap().clone();
    assert_eq!((info.codec.as_str(), info.sample_rate, info.channels), ("xHE-AAC", 48_000, 1));
    let reference = wav_mono(&wav);
    let n = reference.len().min(48_000 * 5);
    let ours = src.audio(0, n, 48_000).unwrap().channels;
    let ours = ours.first().unwrap();
    assert_eq!(ours.len(), n);
    // no loudness normalization on import: the reference is 4 dB louder
    let k = 10f64.powf(4.0 / 20.0);
    let (mut sig, mut err) = (0f64, 0f64);
    for (&r, &o) in reference.iter().zip(ours) {
        sig += (r as f64).powi(2);
        err += (r as f64 - k * o as f64).powi(2);
    }
    let snr = 10.0 * (sig / err.max(1e-20)).log10();
    assert!(snr > 60.0, "SNR {snr:.1} dB");
}
