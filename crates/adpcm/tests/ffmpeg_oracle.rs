//! Bit-exactness against ffmpeg's decoders on ffmpeg-encoded fixtures (ffmpeg is only an
//! external oracle and fixture generator). Skipped without ffmpeg unless `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_adpcm::{Codec, Decoder};
use cfa4_testkit::{fixtures, oracle};

/// A test signal with tones, a sweep, noise, silence and full-scale clipping (to reach every
/// quantizer level and the saturation paths).
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,1),0.5*sin(2*PI*440*t),if(lt(t,2),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,3),(random(0)-0.5)*1.6,if(lt(t,3.5),0,1.2*sin(2*PI*1500*t)))))|0.3*sin(2*PI*220*t)+0.3*(random(1)-0.5)':s=RATE:d=4.3";

/// Encode `source` (`channels` channels at `rate`) with `codec` into `name` (the extension picks
/// the container), plus ffmpeg's own decode of it as raw s16le.
fn fixture(name: &str, codec: &str, rate: u32, channels: u32, extra: &[&str]) -> Option<(PathBuf, Vec<i16>)> {
    let Some(ffmpeg) = oracle::ffmpeg() else {
        oracle::skip("adpcm oracle", "ffmpeg not found");
        return None;
    };
    let dir = fixtures::fixtures_dir("adpcm");
    let out = dir.join(name);
    let src = SOURCE.replace("RATE", &rate.to_string());
    let layout = if channels == 1 { "pan=mono|c0=c0" } else { "anull" };
    let made = fixtures::generate(&out, |tmp| {
        Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &src, "-af", layout, "-c:a", codec])
            .args(extra)
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    });
    let Some(out) = made else {
        oracle::skip("adpcm oracle", &format!("ffmpeg could not encode {name}"));
        return None;
    };
    let pcm = dir.join(format!("{name}.s16"));
    let pcm = fixtures::generate(&pcm, |tmp| {
        Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(&out)
            .args(["-f", "s16le", "-c:a", "pcm_s16le"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })?;
    let raw = std::fs::read(pcm).ok()?;
    Some((out, raw.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()))
}

/// `fmt ` (tag, channels, block align, bits, extra) and `data` of a RIFF WAVE file.
fn wav(path: &Path) -> (u16, u16, u16, u16, Vec<u8>, Vec<u8>) {
    let b = std::fs::read(path).expect("read wav");
    let mut pos = 12;
    let (mut fmt, mut data) = (None, None);
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &b[pos + 8..(pos + 8 + len).min(b.len())];
        if id == b"fmt " {
            fmt = Some(body.to_vec());
        } else if id == b"data" {
            data = Some(body.to_vec());
        }
        pos += 8 + len + (len & 1);
    }
    let f = fmt.expect("fmt chunk");
    let u16_at = |o: usize| u16::from_le_bytes([f[o], f[o + 1]]);
    let extra = if f.len() > 18 { f[18..].to_vec() } else { Vec::new() };
    (u16_at(0), u16_at(2), u16_at(12), u16_at(14), extra, data.expect("data chunk"))
}

/// Our decode of a WAV fixture, interleaved.
fn decode_wav(path: &Path) -> Vec<i16> {
    let (tag, ch, align, bits, extra, data) = wav(path);
    let codec = Codec::from_wave_format(tag, align, bits, &extra).expect("our tag").expect("valid format");
    let mut d = Decoder::new(codec, usize::from(ch)).expect("decoder");
    // decode in packets of a few units, as a demuxer would hand them over
    let unit = d.codec().unit_bytes(usize::from(ch)) * 3;
    let mut planes: Vec<Vec<i16>> = vec![Vec::new(); usize::from(ch)];
    for p in data.chunks(unit) {
        for (o, c) in planes.iter_mut().zip(d.decode(p).expect("decode")) {
            o.extend(c);
        }
    }
    interleave(&planes)
}

fn interleave(planes: &[Vec<i16>]) -> Vec<i16> {
    let n = planes.iter().map(Vec::len).min().unwrap_or(0);
    (0..n).flat_map(|i| planes.iter().map(move |p| p[i])).collect()
}

/// Every sample ffmpeg gives matches ours (ffmpeg may drop the tail of a padded last block).
fn assert_same(name: &str, ours: &[i16], reference: &[i16], channels: usize) {
    assert!(!reference.is_empty(), "{name}: ffmpeg decoded nothing");
    assert!(ours.len() >= reference.len(), "{name}: we decoded {} samples, ffmpeg {}", ours.len(), reference.len());
    assert!(ours.len() - reference.len() < 4096 * channels, "{name}: {} extra samples", ours.len() - reference.len());
    if let Some(i) = ours.iter().zip(reference).position(|(a, b)| a != b) {
        let lo = i.saturating_sub(4);
        panic!(
            "{name}: sample {i} (frame {}) differs: ours {:?}, ffmpeg {:?}",
            i / channels,
            &ours[lo..(i + 4).min(ours.len())],
            &reference[lo..(i + 4).min(reference.len())]
        );
    }
}

fn check_wav(name: &str, codec: &str, rate: u32, channels: u32, extra: &[&str]) {
    let Some((path, reference)) = fixture(name, codec, rate, channels, extra) else { return };
    assert_same(name, &decode_wav(&path), &reference, channels as usize);
}

#[test]
fn mulaw_wav() {
    check_wav("mulaw.wav", "pcm_mulaw", 8000, 2, &[]);
}

#[test]
fn alaw_wav() {
    check_wav("alaw.wav", "pcm_alaw", 8000, 1, &[]);
}

#[test]
fn g722_wav() {
    check_wav("g722.wav", "g722", 16000, 1, &[]);
}

#[test]
fn ima_wav_mono_and_stereo() {
    check_wav("ima_mono.wav", "adpcm_ima_wav", 22050, 1, &[]);
    check_wav("ima_stereo.wav", "adpcm_ima_wav", 44100, 2, &["-block_size", "2048"]);
}

#[test]
fn ms_adpcm_mono_and_stereo() {
    check_wav("ms_mono.wav", "adpcm_ms", 22050, 1, &[]);
    check_wav("ms_stereo.wav", "adpcm_ms", 44100, 2, &["-block_size", "1024"]);
}

/// QuickTime `ima4`: the `mdat` of a single-track MOV is the packets back to back.
#[test]
fn ima_qt_mov() {
    for (name, ch) in [("ima4_mono.mov", 1u32), ("ima4_stereo.mov", 2)] {
        let Some((path, reference)) = fixture(name, "adpcm_ima_qt", 44100, ch, &[]) else { return };
        let b = std::fs::read(&path).unwrap();
        let mut pos = 0;
        let mut mdat = None;
        while pos + 8 <= b.len() {
            let len = u32::from_be_bytes(b[pos..pos + 4].try_into().unwrap()) as usize;
            if &b[pos + 4..pos + 8] == b"mdat" {
                mdat = Some(&b[pos + 8..pos + len]);
            }
            pos += len.max(8);
        }
        let mut d = Decoder::new(Codec::ImaQt, ch as usize).unwrap();
        let planes = d.decode(mdat.expect("mdat")).unwrap();
        assert_same(name, &interleave(&planes), &reference, ch as usize);
    }
}

/// A WAV file with `fmt ` fields and `data`, for streams written byte by byte.
fn write_wav(path: &Path, tag: u16, channels: u16, rate: u32, align: u16, bits: u16, extra: &[u8], data: &[u8]) {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&tag.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    fmt.extend_from_slice(&align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    fmt.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    fmt.extend_from_slice(extra);
    let mut w = b"RIFF".to_vec();
    w.extend_from_slice(&((4 + 8 + fmt.len() + 8 + data.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    w.extend_from_slice(&fmt);
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(data);
    std::fs::write(path, w).unwrap();
}

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8
        })
        .collect()
}

/// ffmpeg's decode of a hand-written WAV.
fn ffmpeg_decode(path: &Path) -> Option<Vec<i16>> {
    let Some(ffmpeg) = oracle::ffmpeg() else {
        oracle::skip("adpcm oracle", "ffmpeg not found");
        return None;
    };
    let out = Command::new(ffmpeg).args(["-v", "error", "-i"]).arg(path).args(["-f", "s16le", "-c:a", "pcm_s16le", "-"]).output().ok()?;
    assert!(out.status.success(), "ffmpeg failed on {}", path.display());
    Some(out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// Random octets reach every codeword pair, the transmission-error codes and the saturation paths
/// of both sub-band predictors.
#[test]
fn g722_random_codewords() {
    let path = fixtures::fixtures_dir("adpcm").join("g722_random.wav");
    write_wav(&path, 0x028F, 1, 16000, 1, 4, &[], &noise(64_000, 0x9E37_79B9));
    let Some(reference) = ffmpeg_decode(&path) else { return };
    assert_same("g722_random", &decode_wav(&path), &reference, 1);
}

/// IMA WAV and Microsoft ADPCM blocks with random codes after valid headers (step index ≤ 88,
/// predictor index ≤ 6), mono and stereo.
#[test]
fn block_formats_random_codes() {
    let dir = fixtures::fixtures_dir("adpcm");
    for ch in [1u16, 2] {
        let c = usize::from(ch);
        // IMA: 256-byte blocks
        let mut data = noise(256 * 64, 17 + u32::from(ch));
        for (k, b) in data.chunks_mut(256).enumerate() {
            for h in b[..4 * c].chunks_mut(4) {
                h[2] = (k * 7 % 89) as u8;
                h[3] = 0;
            }
        }
        let path = dir.join(format!("ima_random_{ch}.wav"));
        write_wav(&path, 0x11, ch, 22050, 256, 4, &((256 - 4 * c) * 2 / c + 1).to_le_bytes()[..2], &data);
        let Some(reference) = ffmpeg_decode(&path) else { return };
        assert_same(&format!("ima_random_{ch}"), &decode_wav(&path), &reference, c);
        // MS: 256-byte blocks with the standard coefficient extension
        let mut data = noise(256 * 64, 99 + u32::from(ch));
        for (k, b) in data.chunks_mut(256).enumerate() {
            for p in &mut b[..c] {
                *p = (k % 7) as u8;
            }
            // a small positive delta keeps the stream plausible, a few blocks start large
            for d in b[c..3 * c].chunks_mut(2) {
                d.copy_from_slice(&(16 + (k as i16 * 37) % 2000).to_le_bytes());
            }
        }
        let mut extra = (((256 - 7 * c) * 2 / c + 2) as u16).to_le_bytes().to_vec();
        extra.extend_from_slice(&7u16.to_le_bytes());
        for (a, b) in cfa4_adpcm::ms::STANDARD_COEFFS {
            extra.extend_from_slice(&(a as i16).to_le_bytes());
            extra.extend_from_slice(&(b as i16).to_le_bytes());
        }
        let path = dir.join(format!("ms_random_{ch}.wav"));
        write_wav(&path, 0x02, ch, 22050, 256, 4, &extra, &data);
        let Some(reference) = ffmpeg_decode(&path) else { return };
        assert_same(&format!("ms_random_{ch}"), &decode_wav(&path), &reference, c);
    }
}

/// Microsoft ADPCM's delta grows without a bound on the stream's side: a run of the largest code
/// takes it far past 32 bits, then a long run of the smallest brings it back into range.
#[test]
fn ms_adpcm_delta_runaway() {
    let mut block = vec![0u8; 7];
    block[1..3].copy_from_slice(&16i16.to_le_bytes());
    block.extend(std::iter::repeat_n(0x77, 40));
    block.extend(std::iter::repeat_n(0x1F, 400));
    let mut extra = (((block.len() - 7) * 2 + 2) as u16).to_le_bytes().to_vec();
    extra.extend_from_slice(&7u16.to_le_bytes());
    for (a, b) in cfa4_adpcm::ms::STANDARD_COEFFS {
        extra.extend_from_slice(&(a as i16).to_le_bytes());
        extra.extend_from_slice(&(b as i16).to_le_bytes());
    }
    let path = fixtures::fixtures_dir("adpcm").join("ms_runaway.wav");
    write_wav(&path, 0x02, 1, 8000, block.len() as u16, 4, &extra, &block);
    let Some(reference) = ffmpeg_decode(&path) else { return };
    assert_same("ms_runaway", &decode_wav(&path), &reference, 1);
}
