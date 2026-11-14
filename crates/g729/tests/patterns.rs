//! G.729 against bcg729's decoder test patterns (the ITU-T G.729 and Annex B test vectors with
//! bcg729's expected output), when they are present locally: download
//! `bcg729-patterns-v1.1.2.zip` from https://github.com/jeannotlapin/bcg729-test-patterns and
//! unpack it into `target/fixtures/g729/p` (or point `$G729_PATTERNS` at its `patterns` directory).

use std::path::PathBuf;

use cfa4_g729::{Decoder, Frame};

fn patterns() -> Option<PathBuf> {
    let dir = std::env::var_os("G729_PATTERNS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/g729/p/patterns"));
    dir.is_dir().then_some(dir)
}

/// `parametersArray2BitStream`.
fn pack(p: &[u16]) -> [u8; 10] {
    let g = |i: usize| p.get(i).copied().unwrap_or(0);
    [
        (((g(0) & 1) << 7) | (g(1) & 0x7f)) as u8,
        (((g(2) & 0x1f) << 3) | ((g(3) >> 2) & 7)) as u8,
        (((g(3) & 3) << 6) | ((g(4) >> 2) & 0x3f)) as u8,
        (((g(4) & 3) << 6) | ((g(5) & 1) << 5) | ((g(6) >> 8) & 0x1f)) as u8,
        (g(6) & 0xff) as u8,
        (((g(7) & 0xf) << 4) | ((g(8) & 7) << 1) | ((g(9) >> 3) & 1)) as u8,
        (((g(9) & 7) << 5) | (g(10) & 0x1f)) as u8,
        ((g(11) >> 5) & 0xff) as u8,
        (((g(11) & 0x1f) << 3) | ((g(12) >> 1) & 7)) as u8,
        (((g(12) & 1) << 7) | ((g(13) & 7) << 4) | (g(14) & 0xf)) as u8,
    ]
}

fn rows(text: &str) -> Vec<Vec<u16>> {
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.split(',').map(|v| v.trim().parse::<i32>().unwrap_or(0) as u16).collect()).collect()
}

fn expected_pcm(path: &std::path::Path) -> Vec<i16> {
    std::fs::read(path).map(|b| b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()).unwrap_or_default()
}

fn expected_csv(path: &std::path::Path) -> Vec<i16> {
    std::fs::read_to_string(path)
        .map(|t| t.split([',', '\n']).filter(|v| !v.trim().is_empty()).map(|v| v.trim().parse::<i16>().unwrap_or(0)).collect())
        .unwrap_or_default()
}

fn compare(name: &str, ours: &[i16], want: &[i16]) {
    assert_eq!(ours.len(), want.len(), "{name}: length");
    let first = ours.iter().zip(want).position(|(a, b)| a != b);
    assert_eq!(first, None, "{name}: first differing sample");
}

#[test]
fn decoder_patterns_are_bit_exact() {
    let Some(dir) = patterns() else {
        eprintln!("bcg729 test patterns not present");
        return;
    };
    for name in ["algthm", "erasure", "fixed", "lsp", "overflow", "parity", "pitch", "speech", "tame", "test"] {
        let input = dir.join(format!("decoder/{name}.Decoder.in"));
        let Ok(text) = std::fs::read_to_string(&input) else { continue };
        let mut d = Decoder::new();
        let mut ours = Vec::new();
        for r in rows(&text) {
            let bits = pack(&r);
            let frame = if r.get(15).copied().unwrap_or(0) != 0 { Frame::Erased } else { Frame::Speech(&bits) };
            ours.extend_from_slice(&d.decode(frame));
        }
        compare(name, &ours, &expected_pcm(&dir.join(format!("decoder/{name}.Decoder.pcm"))));
        eprintln!("{name}: {} samples bit-exact", ours.len());
    }
}

#[test]
fn annex_b_patterns_are_bit_exact() {
    let Some(dir) = patterns() else {
        eprintln!("bcg729 test patterns not present");
        return;
    };
    for k in 1..=10 {
        let input = dir.join(format!("CNGdecoder/annexBseq{k}.in"));
        let Ok(text) = std::fs::read_to_string(&input) else { continue };
        let mut d = Decoder::new();
        let mut ours = Vec::new();
        for r in rows(&text) {
            let g = |i: usize| r.get(i).copied().unwrap_or(0);
            let bits = pack(&r);
            let sid = [(((g(0) & 1) << 7) | ((g(1) & 0x1f) << 2) | ((g(2) >> 2) & 3)) as u8, (((g(2) & 3) << 6) | ((g(3) & 0x1f) << 1)) as u8];
            let frame = match (g(16), g(15)) {
                (1, 0) => Frame::Speech(&bits),
                (2, 0) => Frame::Sid(&sid),
                _ => Frame::Erased,
            };
            ours.extend_from_slice(&d.decode(frame));
        }
        compare(&format!("annexBseq{k}"), &ours, &expected_csv(&dir.join(format!("CNGdecoder/annexBseq{k}.pattern"))));
        eprintln!("annexBseq{k}: {} samples bit-exact", ours.len());
    }
}
