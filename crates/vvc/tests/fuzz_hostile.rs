//! Hostile input: truncation and seeded random mutation of a synthetic stream (and of the encoded
//! fixture, when present) must never panic or run away.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cfa4_bitstream::{BitWriter, annexb_nals, escape_rbsp};
use cfa4_vvc::{Sps, access_units, annexb_access_units, au_info};

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

fn nal(t: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0, 0, 0, 1, 0, (t << 3) | 1];
    v.extend_from_slice(payload);
    v
}

/// SPS (1920x1080, 10 bit, with GCI), PPS, an IDR slice and two trailing slices, Annex B.
fn synthetic_stream() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 8);
    w.write_bits(2, 3);
    w.write_bits(1, 2);
    w.write_bits(2, 2);
    w.write_bit(true);
    w.write_bits(1, 7);
    w.write_bit(false);
    w.write_bits(83, 8);
    w.write_bits(2, 2);
    w.write_bit(true); // gci
    for _ in 0..71 {
        w.write_bit(false);
    }
    w.write_bits(0, 8);
    w.align_zero();
    w.write_bits(0b10, 2);
    w.align_zero();
    w.write_bits(80, 8);
    w.write_bits(0, 8);
    w.write_bits(0, 2);
    w.write_ue(1920);
    w.write_ue(1080);
    w.write_bit(false);
    w.write_bit(false);
    w.write_ue(2);
    w.rbsp_trailing();
    let mut s = nal(15, &escape_rbsp(&w.finish()));
    s.extend(nal(16, &[0x00, 0x80]));
    s.extend(nal(7, &[0xc0, 0x12, 0x34]));
    s.extend(nal(0, &[0x80, 0x55]));
    s.extend(nal(0, &[0xa0, 0x66]));
    s
}

fn exercise(data: &[u8]) {
    let nals = annexb_nals(data);
    let units = access_units(&nals);
    for r in &units {
        assert!(r.start < r.end && r.end <= nals.len());
        let _ = au_info(&nals[r.clone()]);
    }
    let bytes = annexb_access_units(data);
    let mut end = 0;
    for &(off, len) in &bytes {
        assert_eq!(off, end);
        end = off + len;
    }
    assert!(bytes.is_empty() || end == data.len());
    for n in &nals {
        let _ = Sps::parse(n);
    }
}

#[test]
fn synthetic_stream_parses() {
    let data = synthetic_stream();
    let nals = annexb_nals(&data);
    assert_eq!(access_units(&nals), vec![0..3, 3..4, 4..5]);
    let sps = Sps::parse(nals[0]).unwrap();
    assert_eq!((sps.width, sps.height, sps.bit_depth, sps.max_sublayers), (1920, 1080, 10, 3));
    assert!(au_info(&nals[0..3]).idr);
    assert!(au_info(&nals[4..5]).non_ref);
}

#[test]
fn truncation_and_mutation_never_panic() {
    let mut inputs = vec![synthetic_stream()];
    let fixture = cfa4_testkit::fixtures::fixtures_root().join("vvc");
    if let Ok(dir) = std::fs::read_dir(&fixture) {
        for e in dir.flatten() {
            if e.path().extension().is_some_and(|x| x == "266")
                && let Ok(d) = std::fs::read(e.path())
            {
                inputs.push(d.into_iter().take(64 * 1024).collect());
            }
        }
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for base in &inputs {
        for n in (0..base.len()).step_by((base.len() / 200).max(1)) {
            let r = catch_unwind(AssertUnwindSafe(|| exercise(&base[..n])));
            assert!(r.is_ok(), "panic on truncation to {n} bytes");
        }
        for round in 0..400 {
            let mut d = base.clone();
            for _ in 0..1 + rng.below(8) {
                let i = rng.below(d.len());
                d[i] = rng.next() as u8;
            }
            let r = catch_unwind(AssertUnwindSafe(|| exercise(&d)));
            assert!(r.is_ok(), "panic on mutation round {round}");
        }
    }
}
