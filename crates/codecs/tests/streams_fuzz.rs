//! Mutation fuzzing of the stream list readers (`cfa4_codecs::streams`, Media Cutter K13;
//! AGENTS.md §0): truncated, bit-flipped and size-corrupted Ogg, WAV, FLAC, MXF and MPEG TS / PS
//! files must give a list or an error, never a panic. The seeds are built in memory, so the test
//! needs no ffmpeg.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cfa4_codecs::streams::stream_list;
use cfa4_media::reader::MemReader;

/// A small deterministic generator (xorshift64*), so a failure names a reproducible case.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn ogg_page(flags: u8, serial: u32, body: &[u8]) -> Vec<u8> {
    let mut p = b"OggS\0".to_vec();
    p.push(flags);
    p.extend([0; 8]);
    p.extend(serial.to_le_bytes());
    p.extend([0; 8]);
    // lacing: 255-byte segments and a final shorter one
    let mut lacing = vec![255u8; body.len() / 255];
    lacing.push((body.len() % 255) as u8);
    p.push(lacing.len() as u8);
    p.extend(lacing);
    p.extend(body);
    p
}

fn ogg_seed() -> Vec<u8> {
    let mut opus = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00".to_vec();
    opus.truncate(19);
    let mut vorbis = b"\x01vorbis\0\0\0\0\x02".to_vec();
    vorbis.extend(44_100u32.to_le_bytes());
    vorbis.extend([0; 13]);
    let mut flac = b"\x7fFLAC\x01\x00\x00\x01fLaC\x00\x00\x00\x22".to_vec();
    flac.extend([0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0x0A, 0xC4, 0x42, 0xF0, 0, 0, 0, 0]);
    flac.extend([0; 16]);
    let mut theora = b"\x80theora\x03\x02\x01\x00\x14\x00\x0f\x00\x01\x40\x00\x00\xf0".to_vec();
    theora.extend([0; 20]);
    let mut f = ogg_page(2, 1, &opus);
    f.extend(ogg_page(2, 2, &vorbis));
    f.extend(ogg_page(2, 3, &flac));
    f.extend(ogg_page(2, 4, &theora));
    f.extend(ogg_page(2, 5, &[0x55; 300])); // unknown codec, two segments
    f.extend(ogg_page(0, 1, b"data"));
    f.extend(ogg_page(4, 1, b"end"));
    f
}

fn wav_seed() -> Vec<u8> {
    let mut f = b"RIFF\0\0\0\0WAVEJUNK".to_vec();
    f.extend(5u32.to_le_bytes());
    f.extend([0; 6]); // 5 bytes + pad
    f.extend(b"fmt ");
    f.extend(40u32.to_le_bytes());
    f.extend([0xFE, 0xFF, 6, 0]); // extensible, 6 channels
    f.extend(48_000u32.to_le_bytes());
    f.extend((48_000u32 * 18).to_le_bytes());
    f.extend([18, 0, 24, 0, 22, 0, 24, 0, 0x3F, 0, 0, 0, 1, 0]);
    f.extend([0; 14]);
    f.extend(b"data");
    f.extend(8u32.to_le_bytes());
    f.extend([0; 8]);
    f
}

fn flac_seed() -> Vec<u8> {
    let mut f = b"fLaC\x80\x00\x00\x22".to_vec();
    f.extend([0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0x0B, 0xB8, 0x03, 0x70, 0, 0, 0, 0]);
    f.extend([0; 16]);
    f.extend([0xFF, 0xF8, 0x69, 0x08]);
    f
}

fn mxf_seed() -> Vec<u8> {
    let frames: Vec<Vec<u8>> = (0..3).map(|i| cfa4_mxf::testing::anc_element(&[vec![0x41, 0x05, 2, i, i + 1]], false)).collect();
    cfa4_mxf::testing::build_anc((25, 1), &frames)
}

fn mpeg_seed(format: cfa4_mpegts::MuxFormat) -> Vec<u8> {
    use cfa4_mpegts::{MuxCodec, Muxer};
    let mut m = Muxer::new(Vec::new(), format, &[MuxCodec::Mpeg2Video, MuxCodec::MpegAudio], 20_000_000).expect("muxer");
    // an MPEG-2 sequence header (320x240) and a picture start code, then MPEG-1 layer II frames
    let mut video = vec![0, 0, 1, 0xB3, 0x14, 0x00, 0xF0, 0x13, 0xFF, 0xFF, 0xE0, 0x18];
    video.extend([0, 0, 1, 0, 0, 0x0F, 0xFF, 0xF8]);
    video.extend([0x11; 64]);
    m.write(0, &video, 0, None, true).expect("video");
    for k in 0..4i64 {
        let mut f = vec![0xFF, 0xFD, 0xA4, 0x00];
        f.extend((0..572).map(|i| (i as i64 + k) as u8));
        m.write(1, &f, k * 2160, None, true).expect("audio");
    }
    m.finish().expect("finish")
}

/// List `bytes`; a panic fails the test with the case that caused it.
fn list(name: &str, case: &str, bytes: Vec<u8>) {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let reader = MemReader(bytes.into());
        let _ = stream_list(&reader);
    }));
    assert!(r.is_ok(), "{name}: stream_list panicked on {case}");
}

fn fuzz(name: &str, seed: Vec<u8>, rounds: usize) {
    // the seed itself is a valid file of its container
    let reader = MemReader(seed.clone().into());
    let parsed = stream_list(&reader).unwrap_or_else(|| panic!("{name}: seed not recognised"));
    let streams = parsed.unwrap_or_else(|e| panic!("{name}: seed failed: {e}")).streams;
    assert!(!streams.is_empty(), "{name}: seed lists no stream");

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ name.len() as u64);
    // every truncation point in the first bytes, then a spread over the rest
    for n in (0..seed.len().min(512)).chain((512..seed.len()).step_by(97)) {
        list(name, &format!("truncation at {n}"), seed[..n].to_vec());
    }
    for round in 0..rounds {
        let mut b = seed.clone();
        match round % 3 {
            // bit flips
            0 => {
                for _ in 0..1 + rng.below(8) {
                    let i = rng.below(b.len());
                    b[i] ^= 1 << rng.below(8);
                }
            }
            // a corrupt 32-bit size or count (little- and big-endian extremes)
            1 => {
                let i = rng.below(b.len().saturating_sub(4));
                let v: [u8; 4] = match rng.below(4) {
                    0 => [0xFF; 4],
                    1 => [0; 4],
                    2 => [0xFF, 0xFF, 0xFF, 0x7F],
                    _ => (rng.next() as u32).to_le_bytes(),
                };
                b[i..i + 4].copy_from_slice(&v);
            }
            // a corrupt byte run and a truncation
            _ => {
                let i = rng.below(b.len());
                let n = rng.below(32).min(b.len() - i);
                for x in &mut b[i..i + n] {
                    *x = rng.next() as u8;
                }
                b.truncate(rng.below(b.len()) + 1);
            }
        }
        list(name, &format!("mutation round {round}"), b);
    }
}

#[test]
fn ogg_stream_lists_survive_mutation() {
    fuzz("ogg", ogg_seed(), 3000);
}

#[test]
fn wav_stream_lists_survive_mutation() {
    fuzz("wav", wav_seed(), 3000);
}

#[test]
fn flac_stream_lists_survive_mutation() {
    fuzz("flac", flac_seed(), 3000);
}

#[test]
fn mxf_stream_lists_survive_mutation() {
    fuzz("mxf", mxf_seed(), 1500);
}

#[test]
fn mpeg_ts_and_ps_stream_lists_survive_mutation() {
    fuzz("mpeg-ts", mpeg_seed(cfa4_mpegts::MuxFormat::Ts), 1500);
    fuzz("mpeg-ps", mpeg_seed(cfa4_mpegts::MuxFormat::Ps), 1500);
}

/// A FLAC file cut inside STREAMINFO, or with a wrong first block, is an error, not a list.
#[test]
fn a_damaged_flac_streaminfo_is_an_error() {
    let seed = flac_seed();
    let err = |b: Vec<u8>| matches!(stream_list(&MemReader(b.into())), Some(Err(_)));
    assert!(err(seed[..21].to_vec()), "cut inside STREAMINFO");
    let mut wrong_type = seed.clone();
    wrong_type[4] = 0x84; // last block, type 4 (VORBIS_COMMENT)
    assert!(err(wrong_type), "first block not STREAMINFO");
    let mut wrong_size = seed.clone();
    wrong_size[7] = 0x21; // 33 bytes
    assert!(err(wrong_size), "STREAMINFO not 34 bytes");
}

/// An Ogg beginning-of-stream page whose lacing names a second segment the file lacks is an error,
/// though its first packet is all there.
#[test]
fn an_ogg_page_cut_after_its_first_packet_is_an_error() {
    let opus = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00";
    let mut p = b"OggS\0\x02".to_vec();
    p.extend([0; 8]);
    p.extend(1u32.to_le_bytes());
    p.extend([0; 8]);
    p.extend([2, opus.len() as u8, 255]); // two segments: the OpusHead, then 255 bytes
    p.extend(opus);
    assert!(matches!(stream_list(&MemReader(p.into())), Some(Err(_))));
}
