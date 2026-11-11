//! CFA4 fuzz harnesses (CFA4-SECURITY-001 B2).
//!
//! One function per parser that reads untrusted bytes: every L0 container and codec crate, and the
//! project, interchange, caption, credits and graphics-template readers above them. Each takes the
//! fuzzer's bytes and drives the parser as the engine would; it must return (with `Ok` or `Err`)
//! and never panic, overflow, hang or allocate without bound (AGENTS.md §0).
//!
//! The functions are plain Rust so they run in two ways:
//! - `cargo +nightly fuzz run <target>`: libFuzzer, coverage-guided (`fuzz_targets/<target>.rs`);
//! - `cargo test --lib` in this directory (stable): every target over its seed corpus and a few
//!   thousand deterministic mutations of it, under `catch_unwind` (`src/smoke.rs`).
//!
//! [`TARGETS`] is the list both use; `cargo xtask fuzz-targets` checks that every L0 parser crate has
//! one.

#![forbid(unsafe_code)]

use std::sync::Arc;

/// Largest picture side a harness asks a decoder to set up (keeps one input from allocating gigabytes).
const MAX_SIDE: u32 = 512;

/// The fuzzer's bytes, consumed from the front for parameters.
struct Input<'a> {
    data: &'a [u8],
}

impl<'a> Input<'a> {
    fn new(data: &'a [u8]) -> Self {
        Input { data }
    }
    fn byte(&mut self) -> u8 {
        let (b, rest) = self.data.split_first().map(|(b, r)| (*b, r)).unwrap_or((0, &[]));
        self.data = rest;
        b
    }
    fn u16(&mut self) -> u16 {
        u16::from(self.byte()) << 8 | u16::from(self.byte())
    }
    /// A picture side in 1..=MAX_SIDE.
    fn side(&mut self) -> u32 {
        1 + u32::from(self.u16()) % MAX_SIDE
    }
    fn fourcc(&mut self, choices: &[[u8; 4]]) -> [u8; 4] {
        let i = usize::from(self.byte()) % choices.len().max(1);
        choices.get(i).copied().unwrap_or(*b"    ")
    }
    fn rest(&self) -> &'a [u8] {
        self.data
    }
}

/// Split the input into packets, each with a 2-byte big-endian length before it (at most 64).
fn packets(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let n = match rest {
            [a, b, tail @ ..] => {
                let n = (usize::from(*a) << 8 | usize::from(*b)).min(tail.len());
                rest = tail;
                n
            }
            // one byte left (`rest` is not empty here): it is the last packet
            _ => return Some(std::mem::take(&mut rest)),
        };
        let (p, tail) = rest.split_at(n);
        rest = tail;
        Some(p)
    })
    .take(64)
}

fn utf8(data: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(data)
}

// ---------------------------------------------------------------- containers

pub fn isobmff(data: &[u8]) {
    if let Ok(f) = cfa4_isobmff::open(data) {
        std::hint::black_box(f);
    }
}

pub fn matroska(data: &[u8]) {
    if let Ok(f) = cfa4_matroska::open(data) {
        std::hint::black_box(f);
    }
}

pub fn mxf(data: &[u8]) {
    if let Ok(f) = cfa4_mxf::open(data) {
        std::hint::black_box(f);
    }
}

pub fn avi(data: &[u8]) {
    if let Ok(f) = cfa4_avi::open(data) {
        std::hint::black_box(f);
    }
}

pub fn mpegts(data: &[u8]) {
    if let Ok(f) = cfa4_mpegts::open(data) {
        std::hint::black_box(f);
    }
}

pub fn ogg(data: &[u8]) {
    if let Ok(f) = cfa4_ogg::open(data) {
        std::hint::black_box(f);
    }
}

pub fn cfb(data: &[u8]) {
    if let Ok(f) = cfa4_cfb::CompoundFile::open(data) {
        std::hint::black_box(f);
    }
}

pub fn heif(data: &[u8]) {
    let _ = cfa4_heif::parse(data);
}

pub fn dcp(data: &[u8]) {
    let _ = cfa4_dcp::parse_bytes(data, cfa4_dcp::Limits::default());
}

pub fn iab(data: &[u8]) {
    let _ = cfa4_iab::frame_size(data);
    for f in cfa4_iab::split(data).into_iter().take(16) {
        let _ = cfa4_iab::parse(f);
    }
}

pub fn dv(data: &[u8]) {
    let _ = cfa4_dv::probe(data);
    let _ = cfa4_dv::decode_audio(data);
    if let Ok(frames) = cfa4_dv::frames(data) {
        let mut dec = cfa4_dv::Decoder::new();
        for r in frames.into_iter().take(4) {
            if let Some(f) = data.get(r) {
                let _ = dec.decode(f);
            }
        }
    }
}

pub fn lcevc(data: &[u8]) {
    use cfa4_lcevc::{BaseCodec, Framing};
    let mut i = Input::new(data);
    let codec = if i.byte() & 1 == 0 { BaseCodec::H264 } else { BaseCodec::Hevc };
    let framing = match i.byte() % 5 {
        0 => Framing::AnnexB,
        n => Framing::LengthPrefixed(n),
    };
    let _ = cfa4_lcevc::sample_has_lcevc(i.rest(), codec, framing);
}

// ---------------------------------------------------------------- video

pub fn h264(data: &[u8]) {
    let mut dec = cfa4_h264::Decoder::new();
    let _ = dec.decode(data, 0);
}

pub fn hevc(data: &[u8]) {
    let mut dec = cfa4_hevc::Decoder::new();
    let _ = dec.decode(data, 0);
}

pub fn mpeg2v(data: &[u8]) {
    let _ = cfa4_mpeg2v::probe(data);
    let mut dec = cfa4_mpeg2v::Decoder::new();
    let _ = dec.decode(data, 0);
}

pub fn vp8(data: &[u8]) {
    let _ = cfa4_vp8::keyframe_info(data);
    let mut dec = cfa4_vp8::Decoder::new();
    for p in packets(data) {
        let _ = dec.decode(p, 0);
    }
}

pub fn vp9(data: &[u8]) {
    let _ = cfa4_vp9::keyframe_info(data);
    let mut dec = cfa4_vp9::Decoder::new();
    for p in packets(data) {
        let _ = dec.decode(p, 0);
    }
}

pub fn av1(data: &[u8]) {
    let _ = cfa4_av1::is_key_frame_unit(data);
    let mut dec = cfa4_av1::Decoder::new();
    let _ = dec.decode(data);
}

pub fn prores(data: &[u8]) {
    let _ = cfa4_prores::decode_frame(data);
}

pub fn dnx(data: &[u8]) {
    let _ = cfa4_dnx::decode_frame(data);
}

pub fn cineform(data: &[u8]) {
    let _ = cfa4_cineform::decode_frame(data);
}

pub fn apv(data: &[u8]) {
    let _ = cfa4_apv::decode_frame(data);
}

pub fn j2k(data: &[u8]) {
    let _ = cfa4_j2k::decode(data);
}

pub fn jxl(data: &[u8]) {
    let _ = cfa4_jxl::cfa4::decode(data);
}

pub fn dng(data: &[u8]) {
    let _ = cfa4_dng::probe(data);
    let _ = cfa4_dng::decode(data);
}

pub fn hap(data: &[u8]) {
    let mut i = Input::new(data);
    let fourcc = i.fourcc(&[*b"Hap1", *b"Hap5", *b"HapY", *b"HapM", *b"HapA", *b"Hap7", *b"HapH"]);
    let (w, h) = (i.side(), i.side());
    if let Ok(mut dec) = cfa4_hap::Decoder::new(fourcc, w, h) {
        let _ = dec.decode(i.rest());
    }
}

pub fn utvideo(data: &[u8]) {
    let mut i = Input::new(data);
    let fourcc = i.fourcc(&[*b"ULRG", *b"ULRA", *b"ULY0", *b"ULY2", *b"ULY4", *b"ULH0", *b"ULH2", *b"ULH4", *b"UQY2", *b"UQRG", *b"UQRA"]);
    let (w, h) = (i.side(), i.side());
    let extra: Vec<u8> = (0..16).map(|_| i.byte()).collect();
    if let Ok(mut dec) = cfa4_utvideo::Decoder::new(fourcc, w, h, &extra) {
        let _ = dec.decode(i.rest());
    }
}

pub fn huffyuv(data: &[u8]) {
    let mut i = Input::new(data);
    let (w, h) = (i.side(), i.side());
    let bit_count = [16u16, 24, 32][usize::from(i.byte()) % 3];
    let extra_len = usize::from(i.byte()) % 64;
    let extra: Vec<u8> = (0..extra_len).map(|_| i.byte()).collect();
    if let Ok(mut dec) = cfa4_huffyuv::Decoder::new(w, h, bit_count, &extra) {
        let _ = dec.decode(i.rest());
    }
}

pub fn ffv1(data: &[u8]) {
    let _ = cfa4_ffv1::is_keyframe(data);
    let mut i = Input::new(data);
    let (w, h) = (i.side(), i.side());
    let config_len = usize::from(i.byte()) % 128;
    let config: Vec<u8> = (0..config_len).map(|_| i.byte()).collect();
    if let Ok(mut dec) = cfa4_ffv1::Decoder::new(w, h, &config) {
        let _ = dec.decode(i.rest());
    }
}

// ---------------------------------------------------------------- audio

pub fn aac(data: &[u8]) {
    let _ = cfa4_aac::AdtsHeader::parse(data);
    let _ = cfa4_aac::split_adts(data);
    let mut i = Input::new(data);
    let asc_len = usize::from(i.byte() % 8);
    let asc: Vec<u8> = (0..asc_len).map(|_| i.byte()).collect();
    if let Ok(mut dec) = cfa4_aac::Decoder::new(&asc) {
        for p in packets(i.rest()) {
            let _ = dec.decode(p);
        }
    }
}

pub fn ac3(data: &[u8]) {
    let _ = cfa4_ac3::parse_header(data);
    let _ = cfa4_ac3::program_locations(data);
    let mut dec = cfa4_ac3::Decoder::new();
    let _ = dec.decode(data);
    let _ = dec.decode_program(data);
}

pub fn mpa(data: &[u8]) {
    let _ = cfa4_mpa::parse_header(data);
    let mut dec = cfa4_mpa::Decoder::new();
    for p in packets(data) {
        let _ = dec.decode(p);
    }
}

pub fn dts(data: &[u8]) {
    let _ = cfa4_dts::exss_kind(data);
    let mut dec = cfa4_dts::cfa4::Decoder::new();
    for p in packets(data) {
        let _ = dec.decode_packet(p);
    }
}

pub fn truehd(data: &[u8]) {
    let mut dec = cfa4_truehd::cfa4::Decoder::new();
    for p in packets(data) {
        let _ = dec.decode(p);
    }
}

pub fn opus(data: &[u8]) {
    let _ = cfa4_opus::OpusHead::parse(data);
    let _ = cfa4_opus::packet::packet_samples_48k(data);
    let mut i = Input::new(data);
    let channels = 1 + usize::from(i.byte() % 2);
    if let Ok(mut dec) = cfa4_opus::StreamDecoder::new(48_000, channels) {
        let mut out = Vec::new();
        for p in packets(i.rest()) {
            let _ = dec.decode_interleaved(Some(p), &mut out);
        }
    }
}

pub fn adpcm(data: &[u8]) {
    use cfa4_adpcm::Codec;
    let mut i = Input::new(data);
    let channels = 1 + usize::from(i.byte() % 2);
    let block_align = 1 + usize::from(i.u16() % 4096);
    let codec = match i.byte() % 6 {
        0 => Codec::MuLaw,
        1 => Codec::ALaw,
        2 => Codec::G722,
        3 => Codec::ImaWav { block_align },
        4 => Codec::ImaQt,
        _ => Codec::Ms { block_align, coeffs: vec![(256, 0), (512, -256), (0, 0), (192, 64), (240, 0), (460, -208), (392, -232)] },
    };
    if let Ok(mut dec) = cfa4_adpcm::Decoder::new(codec, channels) {
        let _ = dec.decode(i.rest());
    }
}

pub fn auro(data: &[u8]) {
    // a 24-bit PCM carrier (two or more channels) that may hide an Auro-3D stream
    let mut i = Input::new(data);
    let channels = 2 + usize::from(i.byte() % 7);
    let samples: Vec<i32> = i.rest().chunks_exact(3).map(|c| i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8).collect();
    let per = samples.len() / channels;
    let planes: Vec<Vec<i32>> = (0..channels).map(|c| samples.iter().skip(c).step_by(channels).take(per).copied().collect()).collect();
    let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
    let _ = cfa4_auro::cfa4::unfold(&refs);
}

// ---------------------------------------------------------------- projects and interchange (B5)

pub fn project(data: &[u8]) {
    if let Ok(p) = cfa4_project::Project::from_json(&utf8(data)) {
        std::hint::black_box(Arc::new(p));
    }
}

pub fn interchange(data: &[u8]) {
    let mut i = Input::new(data);
    let pick = usize::from(i.byte());
    let formats = cfa4_interchange::Format::ALL;
    let Some(&format) = formats.get(pick % formats.len()) else { return };
    let _ = cfa4_interchange::detect(i.rest(), None);
    let _ = cfa4_interchange::import(i.rest(), format, None);
}

pub fn captions(data: &[u8]) {
    let mut i = Input::new(data);
    let pick = usize::from(i.byte());
    let formats = cfa4_captions::Format::ALL;
    let Some(&format) = formats.get(pick % formats.len()) else { return };
    let _ = cfa4_captions::parse(i.rest(), format);
}

pub fn credits(data: &[u8]) {
    let _ = cfa4_credits::read(data, &cfa4_credits::Styling::default());
}

pub fn graphics_template(data: &[u8]) {
    let _ = cfa4_project::gtemplate::GraphicsTemplate::from_bytes(data);
}

/// A harness: takes the fuzzer's bytes, returns when the parser is done.
pub type Target = fn(&[u8]);

/// Every harness, by target name (`fuzz_targets/<name>.rs`) and the crate it covers.
pub const TARGETS: &[(&str, &str, Target)] = &[
    ("isobmff", "cfa4-isobmff", isobmff),
    ("matroska", "cfa4-matroska", matroska),
    ("mxf", "cfa4-mxf", mxf),
    ("avi", "cfa4-avi", avi),
    ("mpegts", "cfa4-mpegts", mpegts),
    ("ogg", "cfa4-ogg", ogg),
    ("cfb", "cfa4-cfb", cfb),
    ("heif", "cfa4-heif", heif),
    ("dcp", "cfa4-dcp", dcp),
    ("iab", "cfa4-iab", iab),
    ("dv", "cfa4-dv", dv),
    ("lcevc", "cfa4-lcevc", lcevc),
    ("h264", "cfa4-h264", h264),
    ("hevc", "cfa4-hevc", hevc),
    ("mpeg2v", "cfa4-mpeg2v", mpeg2v),
    ("vp8", "cfa4-vp8", vp8),
    ("vp9", "cfa4-vp9", vp9),
    ("av1", "cfa4-av1", av1),
    ("prores", "cfa4-prores", prores),
    ("dnx", "cfa4-dnx", dnx),
    ("cineform", "cfa4-cineform", cineform),
    ("apv", "cfa4-apv", apv),
    ("j2k", "cfa4-j2k", j2k),
    ("jxl", "cfa4-jxl", jxl),
    ("dng", "cfa4-dng", dng),
    ("hap", "cfa4-hap", hap),
    ("utvideo", "cfa4-utvideo", utvideo),
    ("huffyuv", "cfa4-huffyuv", huffyuv),
    ("ffv1", "cfa4-ffv1", ffv1),
    ("aac", "cfa4-aac", aac),
    ("ac3", "cfa4-ac3", ac3),
    ("mpa", "cfa4-mpa", mpa),
    ("dts", "cfa4-dts", dts),
    ("truehd", "cfa4-truehd", truehd),
    ("opus", "cfa4-opus", opus),
    ("adpcm", "cfa4-adpcm", adpcm),
    ("auro", "cfa4-auro", auro),
    ("project", "cfa4-project", project),
    ("interchange", "cfa4-interchange", interchange),
    ("captions", "cfa4-captions", captions),
    ("credits", "cfa4-credits", credits),
    ("graphics_template", "cfa4-project", graphics_template),
];

#[cfg(test)]
mod smoke;
