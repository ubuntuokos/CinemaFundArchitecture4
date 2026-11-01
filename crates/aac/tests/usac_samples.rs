//! USAC (xHE-AAC) decoding of the Fraunhofer IIS test streams against their reference WAV files.
//! The streams are not part of the repository: set `CFA4_SAMPLES` to the sample folder (default
//! `/mnt/project-files/samples`); tests are skipped when the files are absent.

use std::path::PathBuf;

use cfa4_aac::drc::{DrcProcessor, DrcSettings};
use cfa4_aac::usac::UsacDecoder;

fn samples() -> PathBuf {
    std::env::var_os("CFA4_SAMPLES").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/mnt/project-files/samples"))
}

/// Minimal MP4 reader: the decoder config (from `esds`) and the access units of the first audio
/// track.
struct Mp4Audio {
    asc: Vec<u8>,
    aus: Vec<Vec<u8>>,
}

fn be32(d: &[u8], o: usize) -> usize {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize
}

fn boxes(d: &[u8], mut o: usize, end: usize) -> Vec<(String, usize, usize)> {
    let mut v = Vec::new();
    while o + 8 <= end {
        let mut size = be32(d, o);
        let ty = String::from_utf8_lossy(&d[o + 4..o + 8]).to_string();
        let mut h = 8;
        if size == 1 {
            size = u64::from_be_bytes(d[o + 8..o + 16].try_into().unwrap()) as usize;
            h = 16;
        } else if size == 0 {
            size = end - o;
        }
        v.push((ty, o + h, o + size));
        o += size;
    }
    v
}

fn find<'a>(v: &'a [(String, usize, usize)], t: &str) -> Option<&'a (String, usize, usize)> {
    v.iter().find(|b| b.0 == t)
}

fn read_mp4(path: &std::path::Path) -> Option<Mp4Audio> {
    let d = std::fs::read(path).ok()?;
    let top = boxes(&d, 0, d.len());
    let moov = find(&top, "moov")?;
    for trak in boxes(&d, moov.1, moov.2).iter().filter(|b| b.0 == "trak") {
        let mdia = boxes(&d, trak.1, trak.2);
        let mdia = find(&mdia, "mdia")?;
        let mb = boxes(&d, mdia.1, mdia.2);
        let hdlr = find(&mb, "hdlr")?;
        if &d[hdlr.1 + 8..hdlr.1 + 12] != b"soun" {
            continue;
        }
        let minf = find(&mb, "minf")?;
        let mi = boxes(&d, minf.1, minf.2);
        let stbl = find(&mi, "stbl")?;
        let sb = boxes(&d, stbl.1, stbl.2);
        let stsd = find(&sb, "stsd")?;
        let entry = boxes(&d, stsd.1 + 8, stsd.2);
        let mp4a = entry.first()?;
        let inner = boxes(&d, mp4a.1 + 28, mp4a.2);
        let esds = find(&inner, "esds")?;
        let x = &d[esds.1 + 4..esds.2];
        let mut j = 0;
        let mut asc = Vec::new();
        while j < x.len() {
            let tag = x[j];
            j += 1;
            let mut l = 0usize;
            loop {
                let c = x[j];
                j += 1;
                l = (l << 7) | (c & 0x7f) as usize;
                if c & 0x80 == 0 {
                    break;
                }
            }
            match tag {
                3 => j += 3,
                4 => j += 13,
                5 => {
                    asc = x[j..j + l].to_vec();
                    break;
                }
                _ => j += l,
            }
        }
        let stsz = find(&sb, "stsz")?;
        let fixed = be32(&d, stsz.1 + 4);
        let count = be32(&d, stsz.1 + 8);
        let sizes: Vec<usize> = (0..count).map(|i| if fixed != 0 { fixed } else { be32(&d, stsz.1 + 12 + 4 * i) }).collect();
        let offsets: Vec<usize> = if let Some(stco) = find(&sb, "stco") {
            (0..be32(&d, stco.1 + 4)).map(|i| be32(&d, stco.1 + 8 + 4 * i)).collect()
        } else {
            let co = find(&sb, "co64")?;
            (0..be32(&d, co.1 + 4)).map(|i| u64::from_be_bytes(d[co.1 + 8 + 8 * i..co.1 + 16 + 8 * i].try_into().unwrap()) as usize).collect()
        };
        let stsc = find(&sb, "stsc")?;
        let ne = be32(&d, stsc.1 + 4);
        let runs: Vec<(usize, usize)> = (0..ne).map(|i| (be32(&d, stsc.1 + 8 + 12 * i), be32(&d, stsc.1 + 12 + 12 * i))).collect();
        let mut aus = Vec::with_capacity(count);
        let mut s = 0;
        for (ci, &off) in offsets.iter().enumerate() {
            let chunk = ci + 1;
            let per = runs.iter().rev().find(|r| r.0 <= chunk).map(|r| r.1).unwrap_or(1);
            let mut o = off;
            for _ in 0..per {
                if s >= count {
                    break;
                }
                aus.push(d[o..o + sizes[s]].to_vec());
                o += sizes[s];
                s += 1;
            }
        }
        return Some(Mp4Audio { asc, aus });
    }
    None
}

/// 16-bit PCM WAV → planar f32.
fn read_wav(path: &std::path::Path) -> Option<Vec<Vec<f32>>> {
    let d = std::fs::read(path).ok()?;
    let mut o = 12;
    let mut ch = 0;
    let mut bits = 16;
    while o + 8 <= d.len() {
        let id = &d[o..o + 4];
        let sz = u32::from_le_bytes(d[o + 4..o + 8].try_into().unwrap()) as usize;
        if id == b"fmt " {
            ch = u16::from_le_bytes([d[o + 10], d[o + 11]]) as usize;
            bits = u16::from_le_bytes([d[o + 22], d[o + 23]]);
        } else if id == b"data" {
            assert_eq!(bits, 16);
            let data = &d[o + 8..(o + 8 + sz).min(d.len())];
            let n = data.len() / 2 / ch;
            let mut out = vec![Vec::with_capacity(n); ch];
            for i in 0..n {
                for (c, v) in out.iter_mut().enumerate() {
                    let k = 2 * (i * ch + c);
                    v.push(i16::from_le_bytes([data[k], data[k + 1]]) as f32 / 32768.0);
                }
            }
            return Some(out);
        }
        o += 8 + sz + (sz & 1);
    }
    None
}

fn decode_all(m: &Mp4Audio) -> Vec<Vec<f32>> {
    let mut dec = UsacDecoder::new(&m.asc).unwrap();
    let mut out = vec![Vec::new(); dec.channels()];
    for (i, au) in m.aus.iter().enumerate() {
        let pcm = dec.decode(au).unwrap_or_else(|e| panic!("AU {i}: {e}"));
        for (o, p) in out.iter_mut().zip(pcm) {
            o.extend(p);
        }
    }
    out
}

/// Decode with MPEG-D DRC and loudness normalization to -16 LKFS (the reference target).
fn decode_drc(m: &Mp4Audio) -> Vec<Vec<f32>> {
    let mut dec = UsacDecoder::new(&m.asc).unwrap();
    let settings = DrcSettings::default();
    let cfg = dec.config().clone();
    let mut drc = DrcProcessor::new(cfg.uni_drc_config(), cfg.loudness_info(), dec.channels(), dec.sample_rate(), dec.frame_length(), &settings).unwrap();
    let mut out = vec![Vec::new(); dec.channels()];
    for (i, au) in m.aus.iter().enumerate() {
        let mut pcm = dec.decode(au).unwrap_or_else(|e| panic!("AU {i}: {e}"));
        for p in dec.preroll_drc_gain_payloads().to_vec() {
            drc.skip(p.as_deref()).unwrap();
        }
        let payload = dec.drc_gain_payloads().first().cloned();
        drc.process(payload.as_deref(), &mut pcm).unwrap();
        for (o, p) in out.iter_mut().zip(pcm) {
            o.extend(p);
        }
    }
    out
}

/// Best alignment of `ours` against `reference` within ±`max_lag` samples: (lag, SNR in dB).
fn align(ours: &[f32], reference: &[f32], max_lag: isize) -> (isize, f64) {
    let mut best = (0, f64::MIN);
    for lag in -max_lag..=max_lag {
        let (mut sig, mut err) = (0f64, 0f64);
        for (i, &r) in reference.iter().enumerate() {
            let j = i as isize + lag;
            let o = if j >= 0 { ours.get(j as usize).copied().unwrap_or(0.0) } else { 0.0 };
            sig += (r as f64).powi(2);
            err += (r as f64 - o as f64).powi(2);
        }
        let snr = 10.0 * (sig / err.max(1e-20)).log10();
        if snr > best.1 {
            best = (lag, snr);
        }
    }
    best
}

/// Decode `stream` (with `drc`: normalized to -16 LKFS like the references) and check it against
/// `wav`: the reference's length (up to the last frame), no time offset, at least `min_snr` dB.
fn check(stream: &str, wav: &str, drc: bool, min_snr: f64) {
    let root = samples();
    let (Some(m), Some(reference)) = (read_mp4(&root.join(stream)), read_wav(&root.join(wav))) else {
        eprintln!("skipped: {stream} not found");
        return;
    };
    let ours = if drc { decode_drc(&m) } else { decode_all(&m) };
    assert_eq!(ours.len(), reference.len());
    for (c, (o, r)) in ours.iter().zip(&reference).enumerate() {
        let (lag, snr) = align(o, r, 64);
        eprintln!("{stream} ch{c}: {} vs {} samples, lag {lag}, SNR {snr:.1} dB", o.len(), r.len());
        // whole frames: the references end where the edit list does
        assert!(o.len() >= r.len() && o.len() < r.len() + 2048, "{stream} ch{c} length");
        assert_eq!(lag, 0, "{stream} ch{c} offset");
        assert!(snr >= min_snr, "{stream} ch{c}: SNR {snr:.1} dB");
    }
}

// The xHE-AAC references are the encoder input (full band) rather than a decode, so the SNR
// measures the codec, not the decoder.
#[test]
fn xhe_chid_stereo() {
    check("xhe-aac/xHEchID.mp4", "xhe-aac/xHEchID.wav", false, 30.0);
}

#[test]
fn ref_level_stereo() {
    check("xhe-aac/xHErefLevel.mp4", "xhe-aac/xHErefLevel.wav", false, 30.0);
}

// MPEG-D DRC references: loudness normalization to -16 LKFS with the default DRC selection.

#[test]
fn drc_loudness_normalization() {
    check("mpegd-drc/Test_PRL-20.mp4", "mpegd-drc/Reference_Test_PRL_TL-16.wav", true, 70.0);
}

/// `Test_PRL-30` is the same programme 10 dB quieter, coded separately; its reference is the one
/// of `Test_PRL-20`, so the coding noise of both streams limits the match.
#[test]
fn drc_loudness_normalization_quiet() {
    check("mpegd-drc/Test_PRL-30.mp4", "mpegd-drc/Reference_Test_PRL_TL-16.wav", true, 28.0);
}

/// +11 dB to reach -16 LKFS, capped at +6 dB by the 6 dB peak headroom; the peak limiter (5 ms
/// look-ahead, which delays its output) takes the rest down to -1 dBFS.
#[test]
fn drc_peak_limiter() {
    check("mpegd-drc/Test_Limiter.mp4", "mpegd-drc/Reference_Test_Limiter_TL-16.wav", true, 50.0);
}

/// Time-varying DRC gains with node reservoir; the first frames after the pre-roll differ by up
/// to 0.4 dB from the reference (libxaac's start-up), the rest matches to 70 dB.
#[test]
fn drc_gains() {
    check("mpegd-drc/Test_DRC.mp4", "mpegd-drc/Reference_Test_DRC_TL-16.wav", true, 50.0);
}

/// DRC gains jumping between 0 and 19.875 dB every frame (linear interpolation).
#[test]
fn drc_gains_alternating() {
    check("mpegd-drc/Test_DRC_2.mp4", "mpegd-drc/Reference_Test_DRC_2_TL-16.wav", true, 55.0);
}

/// Real access units with random bit flips and truncations decode to errors or finite PCM, never a
/// panic (stereo with complex prediction, and mono with DRC payloads).
#[test]
fn corrupted_access_units_do_not_panic() {
    let mut seed = 0x853c_49e6_748f_ea9bu64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for stream in ["xhe-aac/xHEchID.mp4", "mpegd-drc/Test_DRC.mp4"] {
        let Some(m) = read_mp4(&samples().join(stream)) else { continue };
        for round in 0..6 {
            let mut dec = UsacDecoder::new(&m.asc).unwrap();
            for au in m.aus.iter().take(400) {
                let mut au = au.clone();
                for _ in 0..1 + round {
                    let k = rand() as usize % (au.len() * 8).max(1);
                    if let Some(b) = au.get_mut(k / 8) {
                        *b ^= 1 << (k % 8);
                    }
                }
                if rand() % 8 == 0 {
                    au.truncate(rand() as usize % (au.len() + 1));
                }
                if let Ok(pcm) = dec.decode(&au) {
                    assert!(pcm.iter().flatten().all(|s| s.is_finite()), "{stream}");
                }
            }
        }
    }
}

/// The A/V sync test stream (no reference WAV) decodes without errors.
#[test]
fn av_sync_stream_decodes() {
    let Some(m) = read_mp4(&samples().join("xhe-aac/xheAvSync_v2.mp4")) else { return };
    let pcm = decode_all(&m);
    assert_eq!(pcm.len(), 2);
    assert!(pcm.iter().all(|c| c.len() == m.aus.len() * 1024 && c.iter().all(|s| s.is_finite())));
}
