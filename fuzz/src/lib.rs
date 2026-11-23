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

/// ST 2110: an RTP datagram, SDP text (with its `fmtp` video description) and ST 2110-20
/// reassembly of the input split into datagrams, into a small fixed 4:2:2 10-bit format.
pub fn st2110(data: &[u8]) {
    use cfa4_st2110::video::{Colorimetry, Depth, FrameRate, Sampling};
    let _ = std::hint::black_box(cfa4_st2110::rtp::RtpPacket::parse(data));
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(session) = cfa4_st2110::sdp::Session::parse(text) {
            for m in &session.media {
                let _ = std::hint::black_box((m.ts_refclk(), m.mediaclk()));
                for pt in m.formats.iter().filter_map(|f| f.parse::<u8>().ok()) {
                    let _ = std::hint::black_box(m.rtpmap(pt));
                    if let Some(f) = m.fmtp(pt) {
                        let _ = std::hint::black_box(cfa4_st2110::VideoFormat::from_fmtp(&f).map(|v| v.validate()));
                    }
                }
            }
        }
        let _ = std::hint::black_box(cfa4_st2110::VideoFormat::from_fmtp(&cfa4_st2110::sdp::FmtpParams::parse(text)));
    }
    let Ok(rate) = FrameRate::new(50, 1) else { return };
    let format = cfa4_st2110::VideoFormat::progressive(Sampling::YCbCr422, Depth::D10, 64, 48, rate, Colorimetry::Bt709);
    if let Ok(mut d) = cfa4_st2110::Depacketizer::new(format) {
        for datagram in data.chunks(1400) {
            let _ = d.push(datagram);
        }
        d.flush();
        while let Some(image) = d.pop() {
            std::hint::black_box(image);
        }
    }
}

pub fn tta(data: &[u8]) {
    if let Ok(f) = cfa4_tta::decode_file(data, None) {
        std::hint::black_box(f);
    }
}

pub fn amr(data: &[u8]) {
    if let Ok(f) = cfa4_amr::decode_file(data) {
        std::hint::black_box(f);
    }
}

pub fn av2(data: &[u8]) {
    if let Ok(f) = cfa4_av2::probe_annexb(data) {
        std::hint::black_box(f);
    }
}

/// Canopus HQ/HQA and HQX: the first byte picks the codec, the rest is one frame.
pub fn canopus(data: &[u8]) {
    let Some((&pick, frame)) = data.split_first() else { return };
    let codec = if pick & 1 == 0 { cfa4_canopus::Codec::Hq } else { cfa4_canopus::Codec::Hqx };
    if let Ok(p) = cfa4_canopus::Decoder::new(codec).decode(frame) {
        std::hint::black_box(p);
    }
}

/// Theora: an even first byte takes synthetic valid headers and decodes the rest as packets (a
/// length byte, then that many bytes of packet); an odd one reads the rest as three header packets.
pub fn theora(data: &[u8]) {
    let Some((&pick, rest)) = data.split_first() else { return };
    if pick & 1 == 1 {
        let (a, b) = (rest.len() / 3, 2 * rest.len() / 3);
        if let (Some(i), Some(c), Some(s)) = (rest.get(..a), rest.get(a..b), rest.get(b..)) {
            if let Ok(mut d) = cfa4_theora::Decoder::new(i, c, s) {
                std::hint::black_box(d.decode(&[]).ok());
            }
        }
        return;
    }
    let seed = rest.iter().take(8).fold(0u64, |a, &b| a << 8 | u64::from(b));
    let mut rng = cfa4_theora::synth::Rng::new(seed);
    let (i, c, s) = (
        cfa4_theora::synth::ident(48, 32, u64::from(pick >> 1) % 4 / 2 * 2 + u64::from(pick >> 3 & 1)),
        cfa4_theora::synth::comment(),
        cfa4_theora::synth::setup(&mut rng),
    );
    let Ok(mut d) = cfa4_theora::Decoder::new(&i, &c, &s) else { return };
    let mut at = rest.get(8..).unwrap_or(&[]);
    while let Some((&n, tail)) = at.split_first() {
        let (packet, next) = tail.split_at(usize::from(n).min(tail.len()));
        std::hint::black_box(d.decode(packet).ok());
        at = next;
    }
}

/// Dirac / VC-2: the sequence header probe and the decoder over the whole input.
pub fn dirac(data: &[u8]) {
    std::hint::black_box((cfa4_dirac::sequence_header(data), cfa4_dirac::is_vc2(data)));
    if let Ok(p) = cfa4_dirac::Decoder::new().decode(data) {
        std::hint::black_box(p);
    }
}

/// Speex: an 80-byte stream header, then the rest in 64-byte packets.
pub fn speex(data: &[u8]) {
    let (head, packets) = data.split_at(data.len().min(80));
    let Ok(header) = cfa4_speex::Header::parse(head) else { return };
    let mut d = cfa4_speex::Decoder::new(&header);
    for p in packets.chunks(64).take(64) {
        std::hint::black_box(d.decode_packet(p));
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

// One decoding thread: a finding then reproduces the same way every time; `flush` also drains the
// pictures still held for reordering, so their output path is exercised too.
pub fn h264(data: &[u8]) {
    let mut dec = cfa4_h264::Decoder::with_threads(1);
    let _ = dec.decode(data, 0);
    let _ = dec.flush();
}

pub fn hevc(data: &[u8]) {
    let mut dec = cfa4_hevc::Decoder::with_threads(1);
    let _ = dec.decode(data, 0);
    let _ = dec.flush();
}

pub fn mpeg2v(data: &[u8]) {
    let _ = cfa4_mpeg2v::probe(data);
    let mut dec = cfa4_mpeg2v::Decoder::new();
    let _ = dec.decode(data, 0);
}

/// Only key frames set the VP8 picture size, so one larger than `MAX_SIDE` is skipped: a 1 KiB
/// packet may ask for a 16383 × 16383 picture, valid but slow to decode (a 10 s timeout, not a bug).
pub fn vp8(data: &[u8]) {
    let _ = cfa4_vp8::keyframe_info(data);
    let mut dec = cfa4_vp8::Decoder::new();
    for p in packets(data) {
        if cfa4_vp8::keyframe_info(p).is_some_and(|k| k.width > MAX_SIDE || k.height > MAX_SIDE) {
            continue;
        }
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
        // a packet sequence: a key frame's state carries into the frames after it
        for p in packets(i.rest()) {
            let _ = dec.decode(p);
        }
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

// ---------------------------------------------------------------- RealMedia, MPEG-4 SStP, EVC, AVS and lossless

/// RealMedia: the header and every data packet (at most 256), with each audio stream's
/// `.ra\xfd` header parsed and its packets deinterleaved, as the engine reads a `.rm` file.
pub fn realmedia(data: &[u8]) {
    let _ = cfa4_realmedia::probe(data);
    let Ok(mut demux) = cfa4_realmedia::Demuxer::new(std::io::Cursor::new(data)) else { return };
    let mut audio: Vec<Option<cfa4_realmedia::audio::Deinterleaver>> = demux
        .header()
        .streams
        .iter()
        .map(|s| match &s.kind {
            cfa4_realmedia::StreamKind::Audio { codec_data } => {
                cfa4_realmedia::audio::parse_audio_info(codec_data).ok().map(|i| cfa4_realmedia::audio::Deinterleaver::new(&i))
            }
            _ => None,
        })
        .collect();
    for _ in 0..256 {
        let Ok(Some(p)) = demux.next_packet() else { break };
        if let Some(Some(d)) = audio.get_mut(p.stream) {
            std::hint::black_box(d.push(&p));
        }
    }
    let _ = demux.seek(0);
}

/// RealAudio: the first byte picks the codec (Cook, ATRAC3, ATRAC3plus, ATRAC9, 14.4, 28.8, SIPR),
/// the next bytes the stream parameters, the rest is fed as frames of the block size.
pub fn realaudio(data: &[u8]) {
    use cfa4_realaudio::{atrac3, atrac3p, atrac9, cook, ra144, ra288, sipr};
    let mut i = Input::new(data);
    let pick = i.byte() % 7;
    let channels = 1 + u16::from(i.byte() % 8);
    let block_align = 1 + usize::from(i.u16() % 4096);
    let extra_len = usize::from(i.byte() % 32);
    let extradata: Vec<u8> = (0..extra_len).map(|_| i.byte()).collect();
    let frames = || i.rest().chunks(block_align).take(64);
    match pick {
        0 => {
            let p = cook::Params { channels, sample_rate: 44_100, block_align, extradata: extradata.clone() };
            if let Ok(mut d) = cook::Decoder::new(&p) {
                for f in frames() {
                    let _ = d.decode(f);
                }
            }
        }
        1 => {
            let p = atrac3::Params { channels, block_align, extradata: extradata.clone() };
            if let Ok(mut d) = atrac3::Decoder::new(&p) {
                for f in frames() {
                    let _ = d.decode(f);
                }
            }
        }
        2 => {
            if let Ok(mut d) = atrac3p::Decoder::new(channels, block_align) {
                for f in frames() {
                    let _ = d.decode(f);
                }
            }
        }
        3 => {
            if let Ok(mut d) = atrac9::Decoder::new(&extradata, block_align) {
                for f in frames() {
                    let _ = d.decode(f);
                }
            }
        }
        4 => {
            let mut d = ra144::Decoder::new();
            for f in frames() {
                let _ = d.decode(f);
            }
        }
        5 => {
            let mut d = ra288::Decoder::new();
            for f in frames() {
                let _ = d.decode(f);
            }
        }
        _ => {
            if let Ok(mut d) = sipr::Decoder::new(block_align) {
                for f in frames() {
                    let _ = d.decode(f);
                }
            }
        }
    }
}

/// RealVideo 1.0/2.0 and 3.0/4.0: the first byte picks the codec, then the coded size and
/// 8–23 bytes of codec data, then length-prefixed packets; `flush` drains the held picture.
pub fn realvideo(data: &[u8]) {
    use cfa4_realvideo::{Codec, rv34};
    let mut i = Input::new(data);
    let pick = i.byte() % 4;
    let (w, h) = (i.side(), i.side());
    let extra_len = 8 + usize::from(i.byte() % 16);
    let extradata: Vec<u8> = (0..extra_len).map(|_| i.byte()).collect();
    if pick < 2 {
        let codec = if pick == 0 { Codec::Rv10 } else { Codec::Rv20 };
        if let Ok(mut d) = cfa4_realvideo::Decoder::new(codec, w, h, &extradata) {
            for p in packets(i.rest()) {
                let _ = cfa4_realvideo::picture_type(codec, p);
                let _ = d.decode(p);
            }
            let _ = d.flush();
        }
    } else {
        let codec = if pick == 2 { rv34::Codec::Rv30 } else { rv34::Codec::Rv40 };
        if let Ok(mut d) = rv34::Decoder::new(codec, w, h, &extradata) {
            for p in packets(i.rest()) {
                let _ = rv34::picture_type(codec, p);
                let _ = d.decode(p);
            }
            let _ = d.flush();
        }
    }
}

/// MPEG-4 Studio Profile: the access-unit scanner and probe over the whole input, then the
/// first packet as configuration headers and the others as access units.
pub fn sstp(data: &[u8]) {
    let _ = cfa4_sstp::probe(data);
    let mut starts = Vec::new();
    cfa4_sstp::AccessUnits::new().feed(0, data, &mut starts);
    std::hint::black_box(starts);
    let mut d = cfa4_sstp::Decoder::new();
    let mut ps = packets(data);
    if let Some(head) = ps.next() {
        let _ = d.parse_headers(head);
    }
    for p in ps {
        let _ = d.decode(p);
    }
}

/// EVC: the probe, the picture scanner and the decoder over the input's NAL units (at most 64).
pub fn evc(data: &[u8]) {
    let _ = cfa4_evc::probe(data);
    let mut scanner = cfa4_evc::Scanner::new();
    let mut d = cfa4_evc::Decoder::new();
    for nal in cfa4_evc::split_nal_units(data).take(64) {
        let _ = cfa4_evc::nal_type(nal);
        let _ = scanner.push(nal);
        let _ = d.decode_nal(nal);
    }
    std::hint::black_box(d.flush());
}

/// AVS (Chinese AVS1-P2 / CAVS): the probe and stream info, then the decoder over the packets.
pub fn cavs(data: &[u8]) {
    let _ = (cfa4_cavs::probe(data), cfa4_cavs::stream_info(data));
    let mut d = cfa4_cavs::Decoder::new();
    for p in cfa4_cavs::split_packets(data).into_iter().take(64) {
        let _ = cfa4_cavs::picture_type(p);
        let _ = d.decode(p);
    }
    let _ = d.flush();
}

pub fn bink(data: &[u8]) {
    let _ = cfa4_bink::container::probe(data);
    let Ok(b) = cfa4_bink::container::Bink::parse(data) else { return };
    let Ok(mut v) = cfa4_bink::video::Decoder::new(b.width.min(256), b.height.min(256), b.tag, b.flags) else { return };
    let mut a: Vec<_> = b.tracks.iter().filter_map(|t| cfa4_bink::audio::Decoder::new(t.sample_rate, t.channels, t.dct, b.tag[3]).ok()).collect();
    for i in 0..b.frame_count().min(16) {
        let Ok(f) = b.frame(i) else { continue };
        let _ = v.decode(f.video);
        for (t, pkt) in f.audio {
            if let Some(d) = a.get_mut(t) {
                let _ = d.decode(pkt);
            }
        }
    }
}

pub fn lagarith(data: &[u8]) {
    let mut i = Input::new(data);
    let (w, h) = (i.side(), i.side());
    let bits = [24u16, 32][usize::from(i.byte() & 1)];
    if let Ok(mut d) = cfa4_lagarith::Decoder::new(w, h, bits) {
        let _ = d.decode(i.rest());
    }
}

pub fn svq3(data: &[u8]) {
    let mut i = Input::new(data);
    let n = usize::from(i.byte() % 64);
    let extra: Vec<u8> = (0..n).map(|_| i.byte()).collect();
    if let Ok(mut d) = cfa4_svq3::Decoder::new(0, 0, &extra) {
        let _ = d.decode(i.rest(), 0);
        let _ = d.decode(i.rest(), 1);
        let _ = d.flush();
    }
}

pub fn magicyuv(data: &[u8]) {
    let _ = cfa4_magicyuv::Decoder::new().decode(data);
}

pub fn g729(data: &[u8]) {
    let _ = cfa4_g729::decode_file(data);
}

pub fn ilbc(data: &[u8]) {
    let _ = cfa4_ilbc::parse_header(data);
    let _ = cfa4_ilbc::decode_file(data);
}

pub fn musepack(data: &[u8]) {
    let _ = cfa4_musepack::probe(data);
    let _ = cfa4_musepack::decode_file(data);
}

pub fn acm(data: &[u8]) {
    let _ = cfa4_acm::probe(data);
    let _ = cfa4_acm::decode_file(data);
}

pub fn roq(data: &[u8]) {
    let _ = cfa4_roq::probe(data);
    let Ok(s) = cfa4_roq::parse(data) else { return };
    if let Some(ch) = s.audio_channels {
        for a in s.audio.iter().take(64) {
            let _ = cfa4_roq::decode_audio(a.of(data), ch);
        }
    }
    if s.width.saturating_mul(s.height) <= 1 << 20 {
        if let Ok(mut d) = cfa4_roq::Decoder::new(s.width, s.height) {
            for v in s.video.iter().take(16) {
                let _ = d.decode(v.of(data));
            }
        }
    }
}

pub fn mve(data: &[u8]) {
    let _ = cfa4_mve::probe(data);
    let Ok(m) = cfa4_mve::parse(data) else { return };
    if let Some(a) = &m.audio {
        for p in m.audio_packets.iter().take(64) {
            let _ = cfa4_mve::decode_audio(a, p.of(data));
        }
    }
    if m.width.saturating_mul(m.height) <= 1 << 20 {
        if let Ok(mut d) = cfa4_mve::Decoder::new(m.width, m.height, m.bpp16) {
            for v in m.video.iter().take(16) {
                let input = cfa4_mve::Input {
                    format: v.format,
                    send_buffer: v.send_buffer,
                    video: v.video.of(data),
                    decode_map: v.decode_map.of(data),
                    skip_map: v.skip_map.of(data),
                    palette: v.palette.as_deref(),
                };
                let _ = d.decode(&input);
            }
        }
    }
}

pub fn thp(data: &[u8]) {
    let _ = cfa4_thp::probe(data);
    let Ok(m) = cfa4_thp::parse(data) else { return };
    for v in m.video.iter().take(4) {
        let _ = cfa4_thp::jpeg::decode(v.of(data));
    }
    if let Some(a) = m.audio {
        let mut d = cfa4_thp::AudioDecoder::new();
        for p in m.audio_packets.iter().take(64) {
            let _ = d.decode(p.of(data), a.channels);
        }
    }
}

pub fn mobiclip(data: &[u8]) {
    let _ = cfa4_mobiclip::probe(data);
    let Ok(m) = cfa4_mobiclip::parse(data) else { return };
    if m.width.saturating_mul(m.height) <= 1 << 20 {
        if let Ok(mut d) = cfa4_mobiclip::Decoder::new(m.width, m.height) {
            for p in m.video.iter().take(8) {
                let _ = d.decode(&p.data);
            }
        }
    }
    if let Some(a) = &m.audio {
        let ch = a.channels as usize;
        let mut fast = cfa4_mobiclip::FastAudio::new(ch.min(8));
        for p in a.packets.iter().take(64) {
            match a.codec {
                cfa4_mobiclip::AudioCodec::FastAudio => drop(fast.decode(p)),
                cfa4_mobiclip::AudioCodec::ImaMoflex => drop(cfa4_mobiclip::decode_ima(p, ch)),
                cfa4_mobiclip::AudioCodec::PcmS16le => drop(cfa4_mobiclip::decode_pcm(p, ch)),
            }
        }
    }
pub fn amv(data: &[u8]) {
    // a file: its chunks decode, whatever they hold
    if let Ok(a) = cfa4_amv::Amv::parse(data) {
        if let Ok(d) = cfa4_amv::Decoder::new(a.width.min(64), a.height.min(64)) {
            for c in a.video.iter().take(4) {
                let _ = d.decode(data.get(c.offset..c.offset + c.len).unwrap_or(&[]));
            }
        }
        for c in a.audio.iter().take(4) {
            let _ = cfa4_amv::decode_audio(data.get(c.offset..c.offset + c.len).unwrap_or(&[]));
        }
    }
    // a bare chunk
    if let Ok(d) = cfa4_amv::Decoder::new(32, 24) {
        let _ = d.decode(data);
    }
    let _ = cfa4_amv::decode_audio(data);
}

pub fn adx(data: &[u8]) {
    let _ = cfa4_adx::decode_file(data);
}

pub fn hca(data: &[u8]) {
    let _ = cfa4_hca::decode_file(data, None);
    let _ = cfa4_hca::decode_file(data, Some(cfa4_hca::Key { key: 0xCF22_2F1F_E074_8978, subkey: 0x3B8E }));
}

pub fn twinvq(data: &[u8]) {
    if let Ok(s) = cfa4_twinvq::parse(data) {
        let _ = cfa4_twinvq::decode(data, &s);
    }
    // MetaSound: the first 16 bytes stand for the format extradata, the next two for the block size
    if let (Some(extra), Some(a)) = (data.get(..16), data.get(16..18)) {
        let align = usize::from(u16::from_le_bytes([a[0], a[1]]));
        if let Ok(s) = cfa4_twinvq::parse_metasound(extra, align) {
            let _ = cfa4_twinvq::decode(data.get(18..).unwrap_or(&[]), &s);
        }
pub fn nstream(data: &[u8]) {
    if let Ok(s) = cfa4_nstream::parse(data) {
        let _ = cfa4_nstream::decode(data, &s);
    }
}

pub fn shorten(data: &[u8]) {
    let _ = cfa4_shorten::probe(data);
    let _ = cfa4_shorten::decode_file(data);
}

/// TAK: the whole-file decoder, and a STREAMINFO block followed by frames for the packet decoder.
pub fn tak(data: &[u8]) {
    let _ = cfa4_tak::probe(data);
    let _ = cfa4_tak::decode_file(data);
    let mut ps = packets(data);
    let info = ps.next().and_then(|b| cfa4_tak::parse_streaminfo_block(b).ok());
    let mut d = cfa4_tak::Decoder::new(info);
    for p in ps {
        let _ = d.decode(p);
    }
}

/// Smacker: the whole file through the frame table, each frame's parts, the video decoder and
/// every audio track.
pub fn smacker(data: &[u8]) {
    let _ = cfa4_smacker::probe(data);
    let Ok(smk) = cfa4_smacker::Smk::parse(data) else { return };
    let mut dec = cfa4_smacker::Decoder::new(&smk).ok();
    for i in 0..smk.frames.min(64) {
        let parts = smk.split(data, i);
        for (slot, track) in smk.tracks.iter().enumerate() {
            if let (Some(t), Some(chunk)) = (track, parts.audio.get(slot).copied().flatten()) {
                let _ = cfa4_smacker::decode_audio(t, chunk);
            }
        }
        match (dec.as_mut(), parts.video, parts.palette) {
            (Some(d), Some(v), _) => {
                let _ = d.decode(parts.palette, v);
                std::hint::black_box(d.rgba());
            }
            // a frame whose video is lost keeps its palette update
            (Some(d), None, Some(p)) => {
                let _ = d.update_palette(p);
            }
            _ => {}
        }
    }
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
    ("st2110", "cfa4-st2110", st2110),
    ("tta", "cfa4-tta", tta),
    ("twinvq", "cfa4-twinvq", twinvq),
    ("amr", "cfa4-amr", amr),
    ("av2", "cfa4-av2", av2),
    ("canopus", "cfa4-canopus", canopus),
    ("theora", "cfa4-theora", theora),
    ("dirac", "cfa4-dirac", dirac),
    ("speex", "cfa4-speex", speex),
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
    ("realmedia", "cfa4-realmedia", realmedia),
    ("realaudio", "cfa4-realaudio", realaudio),
    ("realvideo", "cfa4-realvideo", realvideo),
    ("sstp", "cfa4-sstp", sstp),
    ("evc", "cfa4-evc", evc),
    ("cavs", "cfa4-cavs", cavs),
    ("lagarith", "cfa4-lagarith", lagarith),
    ("svq3", "cfa4-svq3", svq3),
    ("bink", "cfa4-bink", bink),
    ("magicyuv", "cfa4-magicyuv", magicyuv),
    ("g729", "cfa4-g729", g729),
    ("mace", "cfa4-mace", mace),
    ("ilbc", "cfa4-ilbc", ilbc),
    ("musepack", "cfa4-musepack", musepack),
    ("shorten", "cfa4-shorten", shorten),
    ("adx", "cfa4-adx", adx),
    ("acm", "cfa4-acm", acm),
    ("roq", "cfa4-roq", roq),
    ("mve", "cfa4-mve", mve),
    ("thp", "cfa4-thp", thp),
    ("mobiclip", "cfa4-mobiclip", mobiclip),
    ("nstream", "cfa4-nstream", nstream),
    ("amv", "cfa4-amv", amv),
    ("hca", "cfa4-hca", hca),
    ("tak", "cfa4-tak", tak),
    ("smacker", "cfa4-smacker", smacker),
    ("project", "cfa4-project", project),
    ("interchange", "cfa4-interchange", interchange),
    ("captions", "cfa4-captions", captions),
    ("credits", "cfa4-credits", credits),
    ("graphics_template", "cfa4-project", graphics_template),
];

#[cfg(test)]
mod smoke;

/// MACE: random packets of random sizes through one decoder of each version and channel count.
pub fn mace(data: &[u8]) {
    let (head, body) = data.split_at(data.len().min(1));
    let sel = head.first().copied().unwrap_or(0);
    let version = if sel & 1 == 0 { cfa4_mace::Version::Mace3 } else { cfa4_mace::Version::Mace6 };
    let Ok(mut d) = cfa4_mace::Decoder::new(version, 1 + usize::from(sel >> 1 & 1)) else { return };
    for pkt in body.chunks(1 + usize::from(sel >> 2) % 64) {
        let _ = d.decode(pkt);
    }
}
