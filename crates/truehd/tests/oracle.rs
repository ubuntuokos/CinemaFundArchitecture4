//! TrueHD and MLP against ffmpeg: ffmpeg's TrueHD and MLP encoders write raw streams from
//! generated tones and noise. Both formats are lossless, so the decode must be bit-exact: equal to
//! ffmpeg's decode of the same file and to the encoder's 24-bit (or 16-bit) input.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_truehd::cfa4::Decoder;

struct Spec {
    name: &'static str,
    /// ffmpeg encoder and raw muxer: "truehd" or "mlp".
    codec: &'static str,
    input: &'static str,
    /// ffmpeg sample format given to the encoder: "s32" (24-bit) or "s16".
    format: &'static str,
    rate: u32,
    channels: usize,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "truehd_stereo_48k",
        codec: "truehd",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000",
        format: "s32",
        rate: 48_000,
        channels: 2,
    },
    Spec { name: "truehd_mono_44k_s16", codec: "truehd", input: "sine=frequency=1000:sample_rate=44100", format: "s16", rate: 44_100, channels: 1 },
    // 5.1 with LFE: L R C LFE Ls Rs
    Spec {
        name: "truehd_51_48k",
        codec: "truehd",
        input: "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1(side)",
        format: "s32",
        rate: 48_000,
        channels: 6,
    },
    Spec {
        name: "truehd_51_96k_noise",
        codec: "truehd",
        input: "anoisesrc=color=pink:sample_rate=96000:amplitude=0.3:seed=3,pan=5.1(side)|c0=c0|c1=-0.5*c0|c2=0.3*c0|c3=0.1*c0|c4=0.7*c0|c5=-0.2*c0",
        format: "s32",
        rate: 96_000,
        channels: 6,
    },
    Spec {
        name: "truehd_40_48k",
        codec: "truehd",
        input: "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.2*sin(2*PI*700*t):s=48000:c=4.0",
        format: "s32",
        rate: 48_000,
        channels: 4,
    },
    // MLP (DVD-Audio's Meridian Lossless Packing)
    Spec {
        name: "mlp_stereo_48k",
        codec: "mlp",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)|0.25*sin(2*PI*660*t):s=48000",
        format: "s32",
        rate: 48_000,
        channels: 2,
    },
];

fn make(ff: &Path, s: &Spec) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("truehd").join(format!("{}.{}", s.name, s.codec));
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", s.input, "-t", "1"])
            .args(["-af", &format!("aformat={}", s.format)])
            .args(["-c:a", s.codec, "-strict", "-2", "-f", s.codec])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(x) if x.success())
    })
}

/// Interleaved samples as signed 24-bit integers, from ffmpeg's s32 output.
fn s24(ff: &Path, args: &[&str]) -> Vec<i32> {
    let o = Command::new(ff).args(["-v", "error"]).args(args).args(["-f", "s32le", "-"]).output().unwrap();
    o.stdout.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b) >> 8).collect()
}

fn decode_all(data: &[u8], chunk: usize) -> cfa4_truehd::cfa4::Decoded {
    let mut dec = Decoder::new();
    let mut all = cfa4_truehd::cfa4::Decoded::default();
    for piece in data.chunks(chunk) {
        let d = dec.decode(piece).unwrap();
        if d.channels.is_empty() {
            continue;
        }
        if all.channels.is_empty() {
            all = d;
        } else {
            assert_eq!(all.labels, d.labels);
            for (a, c) in all.channels.iter_mut().zip(d.channels) {
                a.extend(c);
            }
        }
    }
    all
}

#[test]
fn ffmpeg_encoded_streams_decode_bit_exact() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        // whole file at once, and in odd pieces (access units split across calls)
        for chunk in [data.len(), 1000] {
            let d = decode_all(&data, chunk);
            assert_eq!((d.sample_rate, d.channels.len()), (s.rate, s.channels), "{}", s.name);
            let ours: Vec<Vec<i32>> = d.channels.iter().map(|c| c.iter().map(|&x| (x * 8_388_608.0) as i32).collect()).collect();
            let want = s24(&ff, &["-i", f.to_str().unwrap()]);
            let n = want.len() / s.channels;
            // ffmpeg pads the last access unit (40 samples at 44.1 kHz) with silence and, without
            // a container to give the duration, its decoder drops the padding
            let extra = ours[0].len().checked_sub(n).unwrap_or_else(|| panic!("{}: {} samples < {n}", s.name, ours[0].len()));
            assert!(extra < 160 && ours.iter().all(|c| c[n..].iter().all(|&x| x == 0)), "{}: {extra} extra samples", s.name);
            for (c, o) in ours.iter().enumerate() {
                let bad = (0..n).filter(|&i| o[i] != want[i * s.channels + c]).count();
                assert_eq!(bad, 0, "{}: channel {c} differs from ffmpeg in {bad} of {n} samples", s.name);
            }
            // lossless: the encoder's input, at the stream's precision
            let src = s24(&ff, &["-f", "lavfi", "-i", s.input, "-t", "1", "-af", &format!("aformat={}", s.format)]);
            let m = n.min(src.len() / s.channels);
            assert!(m + 2048 >= n, "{}: source has {m} samples, decode {n}", s.name);
            for (c, o) in ours.iter().enumerate() {
                let bad = (0..m).filter(|&i| o[i] != src[i * s.channels + c]).count();
                assert_eq!(bad, 0, "{}: channel {c} is not the encoder input in {bad} of {m} samples", s.name);
            }
            println!("{}: {} Hz, {} ch, {n} samples bit-exact (pieces of {chunk} bytes)", s.name, s.rate, s.channels);
        }
    }
}

/// The access units of a raw stream, by the length in each one's first two bytes.
fn access_units(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 2 <= data.len() {
        let len = ((usize::from(data[p] & 0x0F) << 8) | usize::from(data[p + 1])) * 2;
        if len < 4 || p + len > data.len() {
            break;
        }
        out.push(&data[p..p + len]);
        p += len;
    }
    out
}

/// A packet that fails keeps none of its bytes: they used to come out with the next packet's
/// samples, shifting the audio after damage.
#[test]
fn a_failed_packet_leaves_nothing_for_the_next() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let s = &SPECS[0];
    let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
    let data = std::fs::read(&f).unwrap();
    let aus = access_units(&data);
    let major: Vec<usize> = (0..aus.len()).filter(|&i| aus[i].get(4..8) == Some(&[0xF8, 0x72, 0x6F, 0xBA][..])).collect();
    assert!(major.len() >= 2, "need two major syncs");
    let (m1, m2) = (major[0], major[1]);
    let mut dec = Decoder::new();
    for au in &aus[..=m1] {
        dec.decode(au).unwrap();
    }
    // a damaged access unit, then a good one with a major sync, in one packet
    let mut bad = aus[m1 + 1].to_vec();
    for b in bad.iter_mut().skip(4) {
        *b ^= 0x5A;
    }
    let packet = [bad.as_slice(), aus[m2]].concat();
    assert!(dec.decode(&packet).is_err());
    let next = dec.decode(aus[m2 + 1]).unwrap();
    let n = next.channels.first().map_or(0, Vec::len);
    assert!(n <= 40, "the next 40-sample packet gave {n} samples");
}

/// A later major sync may reassign the speakers with the same substream count: the labels
/// follow it (they used to stay those of the first layout). The second half of a 5.1 stream has
/// its six-channel assignment changed from Ls / Rs to Tfl / Tfr, with the major sync CRC redone.
#[test]
fn a_new_speaker_layout_at_a_major_sync_updates_the_labels() {
    use cfa4_truehd::utils::bitstream_io::BsIoSliceReader;
    use cfa4_truehd::utils::crc::{CRC_MAJOR_SYNC_INFO_ALG, Crc16};
    let ff = cfa4_testkit::require_ffmpeg!();
    let s = &SPECS[2];
    let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
    let data = std::fs::read(&f).unwrap();
    let crc = Crc16::new(&CRC_MAJOR_SYNC_INFO_ALG);
    let mut aus: Vec<Vec<u8>> = access_units(&data).into_iter().map(<[u8]>::to_vec).collect();
    let half = aus.len() / 2;
    let mut changed = 0;
    for au in aus.iter_mut().skip(half) {
        if au.get(4..8) != Some(&[0xF8, 0x72, 0x6F, 0xBA][..]) {
            continue;
        }
        // the major sync CRC follows a variable-length channel meaning: find where it matches
        let k = (20..au.len().min(64) - 6)
            .find(|&k| {
                let got = BsIoSliceReader::from_slice(au).crc16_check(&crc, 32, (k as u64) * 8).ok();
                got == Some(u16::from_be_bytes([au[4 + k], au[5 + k]]))
            })
            .expect("major sync CRC");
        // format info bits 12..17 hold the six-channel assignment: swap bit 3 (Ls/Rs) for bit 4
        let mut fi = u32::from_be_bytes([au[8], au[9], au[10], au[11]]);
        let assign = (fi >> 15) & 0x1F;
        assert_eq!(assign & 0x18, 0x08, "expected Ls/Rs in assignment {assign:#x}");
        fi ^= 0x18 << 15;
        au[8..12].copy_from_slice(&fi.to_be_bytes());
        let c = BsIoSliceReader::from_slice(au).crc16_check(&crc, 32, (k as u64) * 8).unwrap();
        au[4 + k..6 + k].copy_from_slice(&c.to_be_bytes());
        changed += 1;
    }
    assert!(changed > 0);
    let mut dec = Decoder::new();
    let mut layouts = Vec::new();
    for au in &aus {
        let d = dec.decode(au).unwrap();
        if !d.channels.is_empty() && layouts.last() != Some(&d.labels) {
            layouts.push(d.labels);
        }
    }
    assert_eq!(layouts.len(), 2, "{layouts:?}");
    assert_ne!(layouts[0], layouts[1]);
}

/// Damaged variants per stream (a local run with 6000 in release and 1500 in debug found no panic).
const FUZZ: u32 = 150;

/// Damaged streams (bit flips, truncation, scrambled bytes) give errors or noise, never a panic.
#[test]
fn damaged_streams_never_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut x = 0x00C0_FFEEu32;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for s in SPECS {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        let head = &data[..data.len().min(20_000)];
        for k in 0..FUZZ {
            let mut m = head.to_vec();
            match k % 3 {
                0 => {
                    for _ in 0..1 + rnd() % 16 {
                        let bit = rnd() as usize % (m.len() * 8);
                        m[bit / 8] ^= 0x80 >> (bit % 8);
                    }
                }
                1 => m.truncate(rnd() as usize % m.len()),
                _ => {
                    let at = rnd() as usize % m.len();
                    for b in m.iter_mut().skip(at).take(1 + rnd() as usize % 64) {
                        *b = rnd() as u8;
                    }
                }
            }
            let mut dec = Decoder::new();
            for piece in m.chunks(1 + rnd() as usize % 4000) {
                let _ = dec.decode(piece);
            }
        }
    }
}

#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("truehd fixtures") else { return };
    for s in SPECS {
        let out = cfa4_testkit::fixtures_dir("truehd").join(format!("{}.{}", s.name, s.codec));
        cfa4_testkit::fixtures::generate_and_report(&format!("truehd/{}", s.name), &[out], || make(&ff, s));
    }
}
