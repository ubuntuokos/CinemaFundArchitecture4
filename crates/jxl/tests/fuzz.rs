//! Corrupt JPEG XL files must give an error, never a panic: files from `cjxl` (an external
//! fixture generator) are truncated, bit-flipped and overwritten, and every result goes through
//! the decoder without its panic guard.

use std::path::PathBuf;
use std::process::Command;

use cfa4_jxl::cfa4::decode_unguarded;

fn cjxl() -> Option<PathBuf> {
    let found = cfa4_testkit::oracle::find_tool("cjxl");
    if found.is_none() {
        cfa4_testkit::skip("jxl fuzz", "cjxl not found (Ubuntu package libjxl-tools)");
    }
    found
}

/// A small picture with edges and noise, encoded with `args`.
fn encode(cjxl: &PathBuf, name: &str, args: &[&str]) -> Vec<u8> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("jxl/fuzz");
    let (w, h) = (45u32, 37u32);
    let mut s = 7u32;
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let n = (s >> 24) as u8;
        image::Rgba([(x * 5) as u8 ^ n, if (x / 4 + y / 4) % 2 == 0 { 30 } else { 220 }, (y * 6) as u8, 255 - n / 2])
    });
    let png = dir.join(format!("{name}.png"));
    let jxl = dir.join(format!("{name}.jxl"));
    img.save(&png).unwrap();
    let out = Command::new(cjxl).arg(&png).arg(&jxl).args(args).arg("--quiet").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::read(&jxl).unwrap()
}

fn check(name: &str, data: &[u8], failures: &mut Vec<String>) {
    let owned = data.to_vec();
    if std::panic::catch_unwind(move || {
        let _ = decode_unguarded(&owned);
    })
    .is_err()
    {
        failures.push(format!("{name} ({} bytes)", data.len()));
    }
}

#[test]
fn corrupt_files_do_not_panic() {
    let Some(cjxl) = cjxl() else { return };
    let files = [
        ("vardct", encode(&cjxl, "vardct", &["-d", "1"])),
        ("vardct_progressive", encode(&cjxl, "vardct_progressive", &["-d", "2", "-p", "--patches", "1"])),
        ("modular_lossy", encode(&cjxl, "modular_lossy", &["-m", "1", "-d", "1"])),
        ("lossless", encode(&cjxl, "lossless", &["-d", "0", "-e", "3"])),
        ("noise", encode(&cjxl, "noise", &["-d", "1", "--photon_noise", "ISO1600"])),
    ];
    let mut failures = Vec::new();
    let mut s = 0x9e37_79b9u32;
    let mut rng = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    for (name, data) in &files {
        assert!(decode_unguarded(data).is_ok(), "{name} does not decode");
        // Every truncation of the first 256 bytes, then every 7th length.
        for len in (0..data.len().min(256)).chain((256..data.len()).step_by(7)) {
            check(&format!("{name} cut at {len}"), &data[..len], &mut failures);
        }
        let iterations: usize = std::env::var("CFA4_JXL_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
        for i in 0..iterations {
            let mut bad = data.clone();
            let flips = 1 + rng() as usize % 4;
            for _ in 0..flips {
                let pos = rng() as usize % bad.len();
                if i % 3 == 0 {
                    bad[pos] = rng() as u8;
                } else {
                    bad[pos] ^= 1 << (rng() % 8);
                }
            }
            check(&format!("{name} mutation {i}"), &bad, &mut failures);
        }
    }
    assert!(failures.is_empty(), "{} panics, first: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
}

#[test]
fn garbage_does_not_panic() {
    let mut failures = Vec::new();
    for len in [0usize, 1, 2, 3, 12, 40, 300] {
        let mut data = vec![0u8; len];
        check("zeros", &data, &mut failures);
        if len >= 2 {
            data[0] = 0xff;
            data[1] = 0x0a;
        }
        for (i, b) in data.iter_mut().enumerate().skip(2) {
            *b = (i * 37 % 251) as u8;
        }
        check("codestream signature + junk", &data, &mut failures);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
