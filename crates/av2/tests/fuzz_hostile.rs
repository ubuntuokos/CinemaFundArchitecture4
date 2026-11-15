//! Hostile-input robustness: truncation and seeded random mutation of a synthetic AV2 stream and
//! of the AVM conformance corpus (when present) must never panic or run away.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cfa4_av2::obu::{ObuHeader, annexb_obus};
use cfa4_av2::probe_annexb;
use cfa4_av2::seq::SequenceHeader;
use cfa4_bitstream::BitWriter;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Temporal delimiter, a 64x64 Main_420_10_IP0 sequence header and a small tile group, Annex B.
fn synthetic_stream() -> Vec<u8> {
    // all-zero tool syntax: try lengths until the header ends exactly at its trailing bits
    let seq = (0..400).map(sequence_payload).find(|p| SequenceHeader::parse(p).is_ok()).unwrap();
    let mut s = vec![0x01, 0x08];
    s.push(u8::try_from(seq.len() + 1).unwrap());
    s.push(0x04);
    s.extend_from_slice(&seq);
    s.extend_from_slice(&[0x05, 7 << 2, 0x12, 0x34, 0x56, 0x78]);
    s
}

/// A 64x64 4:2:0 10-bit sequence header with `tool_bits` zero bits after the cropping window,
/// `obu_extension_flag` 0 and the trailing bits.
fn sequence_payload(tool_bits: usize) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // seq_header_id
    w.write_bits(0, 5); // seq_profile_idc
    w.write_bit(false); // single_picture_header_flag
    w.write_bits(0, 5); // seq_level_idx
    w.write_ue(0); // chroma_format_idc: 4:2:0
    w.write_ue(0); // bit_depth_idc: 10 bit
    w.write_bits(0, 3); // seq_lcr_id
    w.write_bit(false); // still_picture
    w.write_bits(0, 2); // max_tlayer_id
    w.write_bits(0, 3); // max_mlayer_id
    w.write_bit(true); // monotonic_output_order_flag
    w.write_bits(7, 4);
    w.write_bits(7, 4);
    w.write_bits(63, 8);
    w.write_bits(63, 8);
    w.write_bit(false); // seq_cropping_window_present_flag
    for _ in 0..tool_bits {
        w.write_bit(false);
    }
    w.write_bit(false); // obu_extension_flag
    w.write_bit(true); // trailing one bit
    w.align_zero();
    w.finish()
}

/// Everything a hostile stream can reach in this crate.
fn exercise(data: &[u8]) {
    for obu in annexb_obus(data).take(10_000) {
        let Ok(obu) = obu else { break };
        let _ = ObuHeader::parse(obu.payload);
        let _ = SequenceHeader::parse(obu.payload).map(|h| h.cropped_size());
    }
    let _ = probe_annexb(data);
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = cfa4_testkit::fixtures_dir("av2/vectors");
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "obu"))
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), std::fs::read(&p).ok()?)))
        .collect();
    out.sort();
    out
}

fn mutate_and_truncate(name: &str, seed: &[u8], rounds: usize) {
    // every prefix of the head of the stream (the headers live there)
    for n in 0..seed.len().min(512) {
        let r = catch_unwind(AssertUnwindSafe(|| exercise(&seed[..n])));
        assert!(r.is_ok(), "{name}: panic on the first {n} bytes");
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.len() as u64);
    let head = seed.len().min(4096);
    for round in 0..rounds {
        let mut m = seed[..head].to_vec();
        for _ in 0..1 + rng.below(8) {
            let i = rng.below(m.len());
            m[i] = match rng.below(3) {
                0 => m[i] ^ (1 << rng.below(8)),
                1 => rng.next() as u8,
                _ => [0x00, 0x80, 0xff][rng.below(3)],
            };
        }
        let r = catch_unwind(AssertUnwindSafe(|| exercise(&m)));
        assert!(r.is_ok(), "{name}: panic in mutation round {round}");
    }
}

#[test]
fn synthetic_stream_survives_mutation() {
    let s = synthetic_stream();
    let info = probe_annexb(&s).unwrap();
    assert_eq!((info.sequence_header.max_frame_width, info.sequence_header.max_frame_height, info.sequence_header.bit_depth), (64, 64, 10));
    mutate_and_truncate("synthetic", &s, 20_000);
}

#[test]
fn corpus_streams_survive_mutation() {
    let corpus = corpus();
    if corpus.is_empty() {
        eprintln!("skip: no AV2 corpus in fixtures av2/vectors (scripts/dev/av2-oracle.py corpus)");
        return;
    }
    for (name, data) in &corpus {
        mutate_and_truncate(name, data, 2_000);
    }
}

/// The sequence headers of the AVM conformance corpus parse completely, to the geometry and tools
/// they were encoded with.
#[test]
fn corpus_sequence_headers() {
    let corpus = corpus();
    if corpus.is_empty() {
        eprintln!("skip: no AV2 corpus in fixtures av2/vectors");
        return;
    }
    for (name, data) in &corpus {
        let info = probe_annexb(data).unwrap_or_else(|e| panic!("{name}: {e}"));
        let h = &info.sequence_header;
        let (w, ht) = h.cropped_size().unwrap();
        eprintln!("{name}: {:?} {:?} {}-bit {}x{} (max {}x{})", h.profile, h.chroma_format, h.bit_depth, w, ht, h.max_frame_width, h.max_frame_height);
        // the whole tool syntax was read: parse() checks that the trailing bits follow it
        let t = &h.tools;
        assert_eq!(t.film_grain_params_present, name.contains("film-grain"), "{name}");
        if name.contains("tiles-2x2") {
            let tiles = t.seq_tile_info.as_ref().map(|i| (i.params.tile_cols, i.params.tile_rows));
            assert_eq!(tiles, Some((2, 2)), "{name}");
        }
        let bits = if name.contains("10bit") { 10 } else { 8 };
        assert_eq!(h.bit_depth, bits, "{name}");
        let chroma = format!("{:?}", h.chroma_format);
        let want = if name.starts_with("444") {
            "Yuv444"
        } else if name.starts_with("400") {
            "Monochrome"
        } else if name.starts_with("422") {
            "Yuv422"
        } else {
            "Yuv420"
        };
        assert_eq!(chroma, want, "{name}");
        if name.contains("odd-size") {
            assert_eq!((w, ht), (66, 50), "{name}");
        }
        if name.contains("resize") {
            // the sequence header keeps the full size; the frames are coded at half of it
            assert_eq!((h.max_frame_width, h.max_frame_height), (352, 288), "{name}");
        }
    }
}
