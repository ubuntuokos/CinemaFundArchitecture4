//! DTS against ffmpeg: ffmpeg's DTS encoder (`dca`, core only) writes raw streams from generated
//! tones and noise, and the decode must be bit-exact against ffmpeg's fixed-point decode
//! (`-flags +bitexact`, 24-bit samples in `s32le`). DTS-HD Master Audio has no free encoder: a
//! real stream named by `CFA4_DTSHD_SAMPLE` is compared with ffmpeg's lossless decode.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_dts::cfa4::{Decoded, Decoder, Speaker};

struct Spec {
    name: &'static str,
    input: &'static str,
    rate: u32,
    speakers: &'static [Speaker],
}

use Speaker::*;

const SPECS: &[Spec] = &[
    Spec { name: "dts_mono_48k", input: "sine=frequency=1000:sample_rate=48000", rate: 48_000, speakers: &[C] },
    Spec {
        name: "dts_stereo_44k",
        input: "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t):s=44100",
        rate: 44_100,
        speakers: &[L, R],
    },
    Spec {
        name: "dts_50_32k",
        input: "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=32000:c=5.0",
        rate: 32_000,
        speakers: &[L, R, C, Ls, Rs],
    },
    Spec {
        name: "dts_51_48k",
        input: "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1",
        rate: 48_000,
        speakers: &[L, R, C, LFE, Ls, Rs],
    },
    Spec {
        name: "dts_noise_stereo",
        input: "anoisesrc=color=pink:sample_rate=48000:amplitude=0.3:seed=3,pan=stereo|c0=c0|c1=-0.5*c0",
        rate: 48_000,
        speakers: &[L, R],
    },
];

fn make(ff: &Path, s: &Spec) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("dts").join(format!("{}.dts", s.name));
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", s.input, "-t", "1"])
            .args(["-c:a", "dca", "-strict", "-2", "-f", "dts"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(x) if x.success())
    })
}

/// ffmpeg's bit-exact decode: interleaved 24-bit samples in the top bits of an i32.
fn reference(ff: &Path, f: &Path) -> Vec<i32> {
    let o = Command::new(ff).args(["-v", "error", "-flags", "+bitexact", "-i"]).arg(f).args(["-f", "s32le", "-"]).output().unwrap();
    o.stdout.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect()
}

fn join(all: &mut Decoded, d: Decoded) {
    if d.channels.is_empty() {
        return;
    }
    if all.channels.is_empty() {
        *all = d;
    } else {
        assert_eq!(all.speakers, d.speakers);
        for (a, c) in all.channels.iter_mut().zip(d.channels) {
            a.extend(c);
        }
    }
}

/// The whole stream through [`Decoder::push`] in `chunk`-byte pieces.
fn decode_raw(data: &[u8], chunk: usize) -> Decoded {
    let mut dec = Decoder::new();
    let mut all = Decoded::default();
    for piece in data.chunks(chunk) {
        join(&mut all, dec.push(piece).unwrap());
    }
    join(&mut all, dec.flush().unwrap());
    all
}

/// The audio of a DTS-HD file (`.dtshd`): its `STRMDATA` chunk. The chunks are an 8-byte id and
/// a 64-bit big-endian size; others (a `BLACKOUT` chunk holds a silent frame) are not the stream,
/// and ffmpeg's demuxer skips them. A raw `.dts` comes back whole.
fn stream_data(data: &[u8]) -> &[u8] {
    if !data.starts_with(b"DTSHDHDR") {
        return data;
    }
    let mut p = 0;
    while let Some(head) = data.get(p..p + 16) {
        let size = usize::try_from(u64::from_be_bytes(head[8..16].try_into().unwrap())).unwrap();
        let body = data.get(p + 16..).map_or(&[][..], |b| &b[..size.min(b.len())]);
        if &head[..8] == b"STRMDATA" {
            return body;
        }
        p += 16 + size;
    }
    &[]
}

/// The stream's frames (a core frame and its extension substream, or the substream alone of a
/// stream without a core), as a demuxer gives them.
fn frames(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    let exss_sync = cfa4_dts::SYNCWORD_SUBSTREAM.to_be_bytes();
    while let Some(rest) = data.get(p..).filter(|r| r.starts_with(&exss_sync)) {
        let Some(n) = cfa4_dts::exss_substream_size(rest).filter(|&n| n <= rest.len()) else { break };
        out.push(&rest[..n]);
        p += n;
    }
    while let Ok(h) = cfa4_dts::parse_header(&data[p..]) {
        let mut end = p + h.frame_size;
        if data.get(end..end + 4) == Some(&cfa4_dts::SYNCWORD_SUBSTREAM.to_be_bytes()[..]) {
            end += cfa4_dts::exss_substream_size(&data[end..]).unwrap();
        }
        if end > data.len() {
            break;
        }
        out.push(&data[p..end]);
        p = end;
    }
    out
}

fn assert_bit_exact(name: &str, d: &Decoded, want: &[i32]) {
    let ch = d.channels.len();
    let n = want.len() / ch;
    assert_eq!(d.channels[0].len(), n, "{name}: sample count");
    for (c, chan) in d.channels.iter().enumerate() {
        for (i, &v) in chan.iter().enumerate() {
            let w = want[i * ch + c];
            assert_eq!((v as f64 * 2_147_483_648.0) as i64, w as i64, "{name}: channel {c} sample {i}");
        }
    }
}

#[test]
fn ffmpeg_encoded_streams_decode_bit_exact() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        let want = reference(&ff, &f);
        let whole = decode_raw(&data, data.len());
        assert_eq!((whole.sample_rate, whole.speakers.as_slice(), whole.lossless), (s.rate, s.speakers, false), "{}", s.name);
        assert_bit_exact(s.name, &whole, &want);
        assert_eq!(decode_raw(&data, 1000), whole, "{}: in 1000-byte pieces", s.name);

        let mut dec = Decoder::new();
        let mut packets = Decoded::default();
        let fr = frames(&data);
        let samples: i64 = fr.iter().map(|p| cfa4_dts::cfa4::packet_samples(p).unwrap()).sum();
        for p in fr {
            join(&mut packets, dec.decode_packet(p).unwrap());
        }
        assert_eq!(packets, whole, "{}: frame by frame", s.name);
        assert_eq!(samples, whole.channels[0].len() as i64, "{}: packet_samples", s.name);
        assert_eq!(cfa4_dts::cfa4::stream_layout(&data).unwrap(), (s.rate, s.speakers.to_vec()));
    }
}

/// After a reset (a seek) the decode resumes at the next frame; the filter banks converge within
/// a few frames, after which the samples are those of a continuous decode.
#[test]
fn a_reset_resumes_at_the_next_frame() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let s = &SPECS[3];
    let data = std::fs::read(make(&ff, s).unwrap()).unwrap();
    let whole = decode_raw(&data, data.len());
    let fr = frames(&data);
    let per = whole.channels[0].len() / fr.len();
    let start: usize = fr[..10].iter().map(|f| f.len()).sum();
    let mut dec = Decoder::new();
    dec.push(&data[..start + 100]).unwrap();
    dec.reset();
    let mut after = Decoded::default();
    for p in &fr[12..] {
        join(&mut after, dec.decode_packet(p).unwrap());
    }
    for (a, w) in after.channels.iter().zip(&whole.channels) {
        assert_eq!(a.len(), w.len() - 12 * per);
        assert_eq!(a[4 * per..], w[16 * per..], "converged after four frames");
    }
}

#[test]
fn damaged_streams_never_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut seed = 0x1234_5678u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for s in SPECS {
        let data = std::fs::read(make(&ff, s).unwrap()).unwrap();
        let fr = frames(&data);
        for round in 0..300 {
            let mut d = data[..data.len().min(40_000)].to_vec();
            match round % 4 {
                0 => {
                    for _ in 0..1 + rnd() % 8 {
                        let at = rnd() as usize % d.len();
                        d[at] ^= 1 << (rnd() % 8);
                    }
                }
                1 => d.truncate(rnd() as usize % d.len()),
                2 => {
                    let at = rnd() as usize % d.len();
                    let n = (rnd() as usize % 64).min(d.len() - at);
                    for b in &mut d[at..at + n] {
                        *b = rnd() as u8;
                    }
                }
                _ => {
                    // a core frame followed by an extension substream of random bytes
                    d = fr[rnd() as usize % fr.len()].to_vec();
                    d.extend(cfa4_dts::SYNCWORD_SUBSTREAM.to_be_bytes());
                    let n = rnd() as usize % 4096;
                    d.extend((0..n).map(|_| rnd() as u8));
                }
            }
            let mut dec = Decoder::new();
            let _ = dec.decode_packet(&d);
            let _ = dec.push(&d);
            let _ = dec.flush();
            let _ = cfa4_dts::cfa4::packet_samples(&d);
        }
    }
}

/// A real DTS-HD Master Audio (or DTS:X) stream, when `CFA4_DTSHD_SAMPLE` names one (a raw
/// `.dtshd` / `.dts`; none is committed): lossless, so bit-exact against ffmpeg.
#[test]
fn a_real_dts_hd_stream_decodes_like_ffmpeg() {
    let Some(path) = std::env::var_os("CFA4_DTSHD_SAMPLE") else {
        eprintln!("skipped: set CFA4_DTSHD_SAMPLE to a DTS-HD Master Audio stream");
        return;
    };
    let ff = cfa4_testkit::require_ffmpeg!();
    let path = PathBuf::from(path);
    let data = std::fs::read(&path).unwrap();
    let d = decode_raw(stream_data(&data), 65_536);
    eprintln!("{} Hz {:?} lossless {}", d.sample_rate, d.speakers, d.lossless);
    assert!(d.lossless);
    let want = reference(&ff, &path);
    assert_bit_exact("CFA4_DTSHD_SAMPLE", &d, &want);
    // as container packets, one frame each
    let fr = frames(stream_data(&data));
    let mut dec = Decoder::new();
    let mut packets = Decoded::default();
    for f in &fr {
        let o = dec.decode_packet(f).unwrap();
        if packets.channels.is_empty() {
            packets = o;
        } else {
            for (a, c) in packets.channels.iter_mut().zip(o.channels) {
                a.extend(c);
            }
        }
    }
    assert_bit_exact("CFA4_DTSHD_SAMPLE packets", &packets, &want);
    // a stream without a core counts its lossless frames
    if stream_data(&data).starts_with(&cfa4_dts::SYNCWORD_SUBSTREAM.to_be_bytes()) {
        let n: i64 = fr.iter().map(|f| cfa4_dts::cfa4::packet_samples(f).unwrap()).sum();
        assert_eq!(n as usize, packets.channels[0].len());
    }
}

/// An MSB-first bit writer.
#[derive(Default)]
struct Bits(Vec<bool>);

impl Bits {
    fn put(&mut self, v: u64, n: usize) {
        for i in (0..n).rev() {
            self.0.push((v >> i) & 1 == 1);
        }
    }
    fn pad_to(&mut self, bits: usize) {
        self.0.resize(bits.max(self.0.len()), false);
    }
    fn bytes(&self) -> Vec<u8> {
        self.0.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (u8::from(b) << (7 - i)))).collect()
    }
}

/// Core frames with an extension substream that parses and points at a lossless (XLL) asset whose
/// common header is plausible and whose channel sets are random: the lossless decoder must
/// reject or decode them, never panic.
#[test]
fn random_lossless_assets_never_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let data = std::fs::read(make(&ff, &SPECS[3]).unwrap()).unwrap();
    let fr = frames(&data);
    let mut seed = 0x9E37_79B9u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for round in 0..3000 {
        // the XLL asset: syncword, a common header with fields in range, then random bytes
        let mut x = Bits::default();
        x.put(0x41A2_9547, 32);
        x.put(0, 4); // version 1
        let header_size = 8 + rnd() as usize % 8;
        x.put(header_size as u64 - 1, 8);
        let payload = 64 + rnd() as usize % 2000;
        x.put(15, 5); // frame size bits
        x.put(payload as u64 - 1, 16);
        x.put(u64::from(rnd() % 3), 4); // channel sets
        x.put(u64::from(rnd() % 4), 4); // segments log2
        x.put(u64::from(5 + rnd() % 5), 4); // samples per segment log2
        x.put(u64::from(rnd() % 32), 5);
        x.put(u64::from(rnd() % 4), 2);
        x.put(u64::from(rnd() % 2), 1);
        x.put(u64::from(rnd() % 32), 5);
        x.put(u64::from(rnd() % 16), 4);
        x.pad_to(header_size * 8);
        let mut xll = x.bytes();
        xll.extend((xll.len()..payload).map(|_| rnd() as u8));
        if round % 3 == 0 {
            let at = rnd() as usize % xll.len();
            xll[at] ^= 1 << (rnd() % 8);
        }

        let exss = xll_substream(&xll);
        assert_eq!(cfa4_dts::exss_kind(&exss), cfa4_dts::ExssKind::Lossless, "the substream must reach the lossless decoder");
        let mut d = fr[rnd() as usize % fr.len()].to_vec();
        d.extend(exss);

        let mut dec = Decoder::new();
        for _ in 0..2 {
            if let Ok(out) = dec.decode_packet(&d) {
                assert!(out.channels.iter().flatten().all(|v| v.is_finite()));
            }
        }
        let _ = cfa4_dts::cfa4::packet_samples(&d);
    }
}

#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("dts fixtures") else { return };
    for s in SPECS {
        let out = cfa4_testkit::fixtures_dir("dts").join(format!("{}.dts", s.name));
        cfa4_testkit::fixtures::generate_and_report(&format!("dts/{}", s.name), &[out], || make(&ff, s));
    }
}

/// An extension substream (no static fields) of one asset in XLL coding mode around `xll`.
fn xll_substream(xll: &[u8]) -> Vec<u8> {
    let header = 16usize;
    let mut e = Bits::default();
    e.put(u64::from(cfa4_dts::SYNCWORD_SUBSTREAM), 32);
    e.put(0, 8);
    e.put(0, 2); // substream index
    e.put(0, 1); // narrow header
    e.put(header as u64 - 1, 8);
    e.put((header + xll.len()) as u64 - 1, 16);
    e.put(0, 1); // no static fields
    e.put(xll.len() as u64 - 1, 16); // asset size
    let descr = e.0.len();
    e.put(4, 9); // descriptor of 5 bytes
    e.put(0, 3); // asset index
    e.put(0, 1); // no DRC
    e.put(0, 1); // no dialogue normalisation
    e.put(1, 2); // coding mode: XLL alone
    e.put(xll.len() as u64 - 1, 16);
    e.put(0, 1); // no XLL sync
    e.put(0, 3); // stream id
    e.pad_to(descr + 40);
    e.pad_to(header * 8);
    let mut exss = e.bytes();
    exss.extend(xll);
    exss
}

/// A raw stream fed one byte at a time, or cut inside an extension substream, decodes as a whole.
#[test]
fn chunk_boundaries_do_not_lose_frames() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let data = std::fs::read(make(&ff, &SPECS[1]).unwrap()).unwrap();
    let whole = decode_raw(&data, data.len());
    // a push ending in a partial core syncword used to drop it, so every frame was lost
    assert_eq!(decode_raw(&data, 1), whole);
    assert_eq!(decode_raw(&data, 3), whole);

    // a core whose substream is still arriving waits for it rather than playing alone
    let fr = frames(&data);
    let exss = xll_substream(&[0; 200]);
    let mut first = fr[0].to_vec();
    first.extend(fr[1]);
    first.extend(&exss[..20]);
    let mut dec = Decoder::new();
    let d = dec.push(&first).unwrap();
    let per = cfa4_dts::cfa4::packet_samples(fr[0]).unwrap() as usize;
    assert_eq!(d.channels.first().map_or(0, Vec::len), per, "the second core waits for its substream");
    assert_eq!(cfa4_dts::exss_declared_size(&exss[..9]), Some(exss.len()));
    assert_eq!(cfa4_dts::exss_declared_size(&exss[..8]), None);
    assert_eq!(cfa4_dts::exss_substream_size(&exss[..20]), None);
}
