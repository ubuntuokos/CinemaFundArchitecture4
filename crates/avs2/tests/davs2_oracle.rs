// SPDX-License-Identifier: GPL-3.0-or-later
//! Bit-exactness against the AVS2 reference implementations, all built from pinned commits by
//! `scripts/ci/install-avs2-oracle.sh`: test streams are encoded here by xavs2 1.4 from a synthetic
//! moving picture (into `target/fixtures/avs2/`, never committed) and decoded by this crate and by
//! davs2 1.7 (the decoder it is ported from); every output picture must be equal, in the same order.
//! Where the encoder writes its reconstruction, the output must equal that too. The tests skip when
//! a tool is missing. `CFA4_AVS2_ORACLE_DIR` names the directory holding `xavs2`, `davs2`,
//! `xavs2-10bit` and `davs2-10bit`; unset, they are looked up on `PATH`.
//!
//! Two known disagreements between the reference tools decide which comparison a case makes:
//! - davs2 decides whether the LCU row below belongs to another slice (SAO, without cross-slice
//!   filtering) from the slice numbers of the *previous* picture, because that row is not decoded
//!   yet when it filters. This crate uses the current picture, as the encoder does, so multi-slice
//!   streams with SAO are checked against the encoder's reconstruction only.
//! - With several slices, ALF and B pictures, the encoder's reconstruction differs from davs2 (and
//!   from this crate, which matches davs2), so those streams are checked against davs2 only.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tool(name: &str) -> PathBuf {
    match std::env::var_os("CFA4_AVS2_ORACLE_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    }
}

fn have(name: &str) -> bool {
    // Both tools print their banner and fail without arguments; spawning is what is checked.
    Command::new(tool(name)).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/avs2");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A moving test picture: diagonal gradients, a sliding checkerboard, a bouncing block and
/// deterministic noise, so that intra, inter and the loop filters all have something to do.
fn source(w: usize, h: usize, frames: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3 / 2 * frames);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 17) as i32 - 8
    };
    for f in 0..frames {
        let (bx, by) = ((f * 7) % w.max(1), (f * 3) % h.max(1));
        for y in 0..h {
            for x in 0..w {
                let mut v = ((x + y + f * 2) * 255 / (w + h)) as i32;
                if ((x + f * 3) / 16 + y / 16) % 2 == 0 && y > h / 2 {
                    v = 255 - v;
                }
                if x.abs_diff(bx) < 24 && y.abs_diff(by) < 20 {
                    v = (v + 96) % 256;
                }
                out.push((v + noise()).clamp(0, 255) as u8);
            }
        }
        for c in 0..2 {
            for y in 0..h / 2 {
                for x in 0..w / 2 {
                    let v = 128 + ((x * (c + 1) + y * (2 - c) + f * 4) % 96) as i32 - 48;
                    out.push((v + noise() / 2).clamp(0, 255) as u8);
                }
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Check {
    /// davs2 and the encoder's reconstruction.
    Both,
    /// davs2 only.
    Davs2,
    /// The encoder's reconstruction only.
    Recon,
}

struct Case<'a> {
    name: &'a str,
    w: usize,
    h: usize,
    frames: usize,
    ten_bit: bool,
    args: &'a [&'a str],
    check: Check,
}

fn decode(data: &[u8]) -> Vec<u8> {
    let mut dec = cfa4_avs2::Decoder::new();
    let mut out = Vec::new();
    let mut put = |f: &cfa4_avs2::Frame| {
        for p in &f.planes {
            if f.bit_depth > 8 {
                p.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
            } else {
                out.extend(p.iter().map(|&v| v as u8));
            }
        }
    };
    for (i, pkt) in cfa4_avs2::split_packets(data).into_iter().enumerate() {
        for f in dec.decode(pkt, i as i64).unwrap_or_else(|e| panic!("packet {i}: {e}")) {
            put(&f);
        }
    }
    for f in dec.flush() {
        put(&f);
    }
    out
}

fn first_difference(a: &[u8], b: &[u8]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!("first difference at byte {i}"),
        None => format!("sizes {} and {}", a.len(), b.len()),
    }
}

// xavs2 copies its option strings into fixed-size buffers: a long absolute path (CI checkouts have
// one) overflows them for the longer case names. It therefore runs inside the fixtures directory and
// gets bare file names.
fn run(cases: &[Case<'_>]) {
    let (enc, dec) = if cases.iter().any(|c| c.ten_bit) { ("xavs2-10bit", "davs2-10bit") } else { ("xavs2", "davs2") };
    if !have(enc) || !have(dec) {
        eprintln!("SKIPPED (AVS2 oracle): {enc} or {dec} is not installed (scripts/ci/install-avs2-oracle.sh)");
        return;
    }
    let dir = fixtures();
    for case in cases {
        let input = dir.join(format!("{}.src.yuv", case.name));
        std::fs::write(&input, source(case.w, case.h, case.frames)).unwrap();
        let stream = dir.join(format!("{}.avs2", case.name));
        let recon = dir.join(format!("{}.rec.yuv", case.name));
        let reference = dir.join(format!("{}.davs2.yuv", case.name));
        let depth = if case.ten_bit { "10" } else { "8" };
        let out = Command::new(tool(enc))
            .arg("-p")
            .current_dir(&dir)
            .arg(format!("InputFile={}.src.yuv", case.name))
            .arg(format!("--SourceWidth={}", case.w))
            .arg(format!("--SourceHeight={}", case.h))
            .arg(format!("--FramesToBeEncoded={}", case.frames))
            .args(["--fps=25", "--InputSampleBitDepth=8", "--thread_frames=1", "--thread_rows=1"])
            .arg(format!("--SampleBitDepth={depth}"))
            .arg(format!("--OutputFile={}.avs2", case.name))
            .arg(format!("--ReconFile={}.rec.yuv", case.name))
            .args(case.args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}: {enc} failed ({}): {}", case.name, out.status, tail(&out));
        let data = std::fs::read(&stream).unwrap();
        let ours = decode(&data);
        let size = case.w * case.h * 3 / 2;
        assert_eq!(ours.len(), size * case.frames, "{}: decoded size", case.name);
        if case.check != Check::Recon {
            let status = Command::new(tool(dec)).arg("-i").arg(&stream).arg("-o").arg(&reference).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
            assert!(status.success(), "{}: {dec} failed", case.name);
            let want = std::fs::read(&reference).unwrap();
            assert!(want == ours, "{}: differs from davs2, {}", case.name, first_difference(&want, &ours));
        }
        if case.check != Check::Davs2 {
            let want = std::fs::read(&recon).unwrap();
            assert!(want == ours, "{}: differs from the encoder's reconstruction, {}", case.name, first_difference(&want, &ours));
        }
        eprintln!("{}: {} pictures bit-exact", case.name, case.frames);
    }
}

/// The last lines an oracle tool printed, for a failure message.
fn tail(out: &std::process::Output) -> String {
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(8)..].join(" | ")
}

const TOOLS: &[&str] = &["--SAOEnable=1", "--ALFEnable=1", "--NSQT=1", "--SDIP=1", "--SECTEnable=1", "--InterAMP=1", "--PMVREnable=1"];

fn with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    base.iter().chain(extra).copied().collect()
}

#[test]
fn random_access_matches_davs2() {
    let tools = with(&["--preset=6"], TOOLS);
    run(&[
        Case { name: "ra_fast", w: 352, h: 288, frames: 9, ten_bit: false, args: &["--preset=1"], check: Check::Both },
        Case { name: "ra_slow", w: 352, h: 288, frames: 9, ten_bit: false, args: &["--preset=9"], check: Check::Both },
        Case { name: "ra_tools", w: 352, h: 288, frames: 9, ten_bit: false, args: &tools, check: Check::Both },
        Case { name: "ra_bframes3", w: 352, h: 288, frames: 9, ten_bit: false, args: &["--preset=4", "--bframes=3"], check: Check::Both },
        Case {
            name: "ra_intra_period",
            w: 176,
            h: 144,
            frames: 17,
            ten_bit: false,
            args: &["--preset=4", "--IntraPeriodMax=8", "--IntraPeriodMin=8", "--OpenGOP=0"],
            check: Check::Both,
        },
    ]);
}

#[test]
fn low_delay_and_intra_match_davs2() {
    let f = with(&["--preset=4", "--bframes=0", "--FFRAMEEnable=1", "--DHPEnable=1", "--WSMEnable=1", "--MHPSKIPEnable=1"], TOOLS);
    let p = with(&["--preset=4", "--bframes=0", "--FFRAMEEnable=0"], TOOLS);
    run(&[
        Case { name: "ld_f", w: 352, h: 288, frames: 8, ten_bit: false, args: &f, check: Check::Both },
        Case { name: "ld_p", w: 352, h: 288, frames: 8, ten_bit: false, args: &p, check: Check::Both },
        Case {
            name: "all_intra",
            w: 352,
            h: 288,
            frames: 3,
            ten_bit: false,
            args: &["--preset=4", "--IntraPeriodMax=1", "--IntraPeriodMin=1"],
            check: Check::Both,
        },
    ]);
}

#[test]
fn sizes_and_parameters_match_davs2() {
    let odd = with(&["--preset=6"], TOOLS);
    let lcu32 = with(&["--preset=6", "--MaxSizeInBit=5"], TOOLS);
    let lcu16 = with(&["--preset=6", "--MaxSizeInBit=4"], TOOLS);
    let qp = with(&["--preset=4", "--QP=20", "--ChromaDeltaQPDisable=0", "--ChromaDeltaU=3", "--ChromaDeltaV=-2"], TOOLS);
    let lf = with(&["--preset=4", "--LoopFilterParameter=1", "--LoopFilterAlphaOffset=3", "--LoopFilterBetaOffset=-2", "--QP=45"], TOOLS);
    run(&[
        Case { name: "odd_size", w: 178, h: 98, frames: 8, ten_bit: false, args: &odd, check: Check::Both },
        Case { name: "lcu32", w: 200, h: 120, frames: 8, ten_bit: false, args: &lcu32, check: Check::Both },
        Case { name: "lcu16", w: 176, h: 144, frames: 6, ten_bit: false, args: &lcu16, check: Check::Both },
        Case { name: "qp_chroma_delta", w: 352, h: 288, frames: 6, ten_bit: false, args: &qp, check: Check::Both },
        Case { name: "loop_filter_offsets", w: 352, h: 288, frames: 6, ten_bit: false, args: &lf, check: Check::Both },
        Case {
            name: "rate_control",
            w: 352,
            h: 288,
            frames: 6,
            ten_bit: false,
            args: &["--preset=4", "--RateControl=1", "--TargetBitRate=300000"],
            check: Check::Both,
        },
    ]);
}

#[test]
fn slices_match_the_references() {
    let base = ["--preset=6", "--MaxSizeInBit=5", "--SliceNum=3", "--NSQT=1", "--SDIP=1", "--SECTEnable=1"];
    let sao = with(&base, &["--SAOEnable=1", "--ALFEnable=0"]);
    let sao_x = with(&base, &["--SAOEnable=1", "--ALFEnable=0", "--CrossSliceLoopFilter=1"]);
    let alf = with(&base, &["--SAOEnable=0", "--ALFEnable=1"]);
    let none = with(&base, &["--SAOEnable=0", "--ALFEnable=0"]);
    run(&[
        Case { name: "slices", w: 352, h: 288, frames: 6, ten_bit: false, args: &none, check: Check::Both },
        Case { name: "slices_sao", w: 352, h: 288, frames: 6, ten_bit: false, args: &sao, check: Check::Recon },
        Case { name: "slices_sao_cross", w: 352, h: 288, frames: 6, ten_bit: false, args: &sao_x, check: Check::Recon },
        Case { name: "slices_alf", w: 352, h: 288, frames: 6, ten_bit: false, args: &alf, check: Check::Davs2 },
    ]);
}

/// Main 10 with 10-bit coding precision; the encoder reads 8-bit input, so the stream signals 8-bit
/// output samples and both decoders round the 10-bit pictures to 8 bits (the encoder writes its
/// reconstruction truncated, so it is not compared).
#[test]
fn main10_matches_davs2() {
    let tools = with(&["--preset=4", "--ProfileID=34"], TOOLS);
    let ld = with(&["--preset=4", "--ProfileID=34", "--bframes=0", "--FFRAMEEnable=1", "--DHPEnable=1", "--WSMEnable=1"], TOOLS);
    run(&[
        Case { name: "main10_ra", w: 352, h: 288, frames: 9, ten_bit: true, args: &tools, check: Check::Davs2 },
        Case { name: "main10_ld", w: 176, h: 144, frames: 8, ten_bit: true, args: &ld, check: Check::Davs2 },
    ]);
}

/// Never crash: streams from the encoder with random bytes flipped, dropped or cut off decode to
/// errors or damaged pictures, never a panic or a hang.
#[test]
fn damaged_streams_do_not_panic() {
    if !have("xavs2") {
        eprintln!("SKIPPED (AVS2 damaged streams): xavs2 is not installed");
        return;
    }
    let dir = fixtures();
    let mut streams = Vec::new();
    for (name, args) in [
        ("damage_ra", with(&["--preset=6"], TOOLS)),
        ("damage_ld", with(&["--preset=4", "--bframes=0", "--FFRAMEEnable=1", "--DHPEnable=1", "--WSMEnable=1", "--MHPSKIPEnable=1"], TOOLS)),
        ("damage_slices", with(&["--preset=4", "--MaxSizeInBit=5", "--SliceNum=2"], TOOLS)),
    ] {
        let input = dir.join(format!("{name}.src.yuv"));
        std::fs::write(&input, source(176, 144, 6)).unwrap();
        let stream = dir.join(format!("{name}.avs2"));
        let status = Command::new(tool("xavs2"))
            .arg("-p")
            .current_dir(&dir)
            .arg(format!("InputFile={name}.src.yuv"))
            .args(["--SourceWidth=176", "--SourceHeight=144", "--FramesToBeEncoded=6", "--fps=25", "--thread_frames=1", "--thread_rows=1"])
            .arg(format!("--OutputFile={name}.avs2"))
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{name}: xavs2 failed");
        streams.push(std::fs::read(&stream).unwrap());
    }
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };
    let rounds: usize = std::env::var("CFA4_AVS2_DAMAGE_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    for round in 0..rounds {
        let mut data = streams[round % streams.len()].clone();
        match rnd(4) {
            0 => {
                for _ in 0..1 + rnd(8) {
                    let i = rnd(data.len());
                    data[i] ^= 1 << rnd(8);
                }
            }
            1 => {
                for _ in 0..1 + rnd(4) {
                    let i = rnd(data.len());
                    data[i] = rnd(256) as u8;
                }
            }
            2 => {
                let at = rnd(data.len());
                let len = rnd(64).min(data.len() - at);
                data.drain(at..at + len);
            }
            _ => data.truncate(rnd(data.len())),
        }
        let mut dec = cfa4_avs2::Decoder::new();
        for (i, pkt) in cfa4_avs2::split_packets(&data).into_iter().enumerate() {
            let _ = dec.decode(pkt, i as i64);
        }
        let _ = dec.flush();
    }
}

/// Never crash, without the oracle: random packets behind AVS2 start codes.
#[test]
fn random_packets_do_not_panic() {
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..400 {
        let mut dec = cfa4_avs2::Decoder::new();
        for _ in 0..8 {
            let code = [0xB0u8, 0xB3, 0xB6, 0x00, 0x01, 0xB5, 0xB1][(rnd() % 7) as usize];
            let mut pkt = vec![0, 0, 1, code];
            // Plausible sequence headers: Main profile, a small size, 4:2:0, 8 bits.
            if code == 0xB0 && rnd() % 2 == 0 {
                pkt.extend_from_slice(&[0x20, 0x42, 0x80, 0x16, 0x00, 0x48, 0x8a]);
            }
            let n = (rnd() % 300) as usize;
            pkt.extend((0..n).map(|_| rnd() as u8));
            let _ = dec.decode(&pkt, 0);
        }
        let _ = dec.flush();
    }
}
