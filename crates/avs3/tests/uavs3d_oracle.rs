// SPDX-License-Identifier: GPL-3.0-or-later
//! Bit-exactness against the AVS3 reference implementations, built from pinned commits by
//! `scripts/ci/install-avs3-oracle.sh`: test streams are encoded here by uavs3e 1.2 from a synthetic
//! moving picture (into `target/fixtures/avs3/`, never committed) and decoded by this crate and by
//! uavs3d 1.2 (the decoder it is ported from); every output picture must be equal, in the same
//! order. The tests skip when a tool is missing. `CFA4_AVS3_ORACLE_DIR` names the directory holding `uavs3e`,
//! `uavs3d`, `uavs3e-10bit` and `uavs3d-10bit`; unset, they are looked up on `PATH`.
//!
//! uavs3e cannot write streams with several patches (it always writes one, whatever the header
//! says), so the patch handling (per-patch context reset, the loop filters across patch borders)
//! is ported from uavs3d but cannot be checked against the oracle here.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tool(name: &str) -> PathBuf {
    match std::env::var_os("CFA4_AVS3_ORACLE_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    }
}

fn have(name: &str) -> bool {
    // Both tools print a banner and fail without arguments; spawning is what is checked.
    Command::new(tool(name)).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/avs3");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A moving test picture: diagonal gradients, a sliding checkerboard, a bouncing block and
/// deterministic noise, so that intra, inter and the loop filters all have something to do.
/// 8-bit samples are bytes, deeper ones `u16` little endian.
fn source(w: usize, h: usize, frames: usize, bits: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut noise = |range: i32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % (2 * range as u64 + 1)) as i32 - range
    };
    let max = (1i32 << bits) - 1;
    let scale = 1i32 << (bits - 8);
    let put = |out: &mut Vec<u8>, v: i32| {
        let v = v.clamp(0, max);
        if bits > 8 {
            out.extend_from_slice(&(v as u16).to_le_bytes());
        } else {
            out.push(v as u8);
        }
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
                put(&mut out, v * scale + noise(8) * scale / 2);
            }
        }
        for c in 0..2 {
            for y in 0..h / 2 {
                for x in 0..w / 2 {
                    let v = 128 + ((x * (c + 1) + y * (2 - c) + f * 4) % 96) as i32 - 48;
                    put(&mut out, (v + noise(4)) * scale);
                }
            }
        }
    }
    out
}

struct Case<'a> {
    name: &'a str,
    w: usize,
    h: usize,
    frames: usize,
    ten_bit: bool,
    args: &'a [&'a str],
}

fn decode(data: &[u8]) -> Vec<u8> {
    let mut dec = cfa4_avs3::Decoder::new();
    let mut out = Vec::new();
    let mut put = |f: &cfa4_avs3::Frame| {
        for p in &f.planes {
            if f.bit_depth > 8 {
                p.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
            } else {
                out.extend(p.iter().map(|&v| v as u8));
            }
        }
    };
    for (i, pkt) in cfa4_avs3::split_packets(data).into_iter().enumerate() {
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

/// Runs a tool in `dir` with bare file names (the encoder's option buffers are small).
fn run_in(dir: &Path, name: &str, args: &[&str]) -> bool {
    Command::new(tool(name)).current_dir(dir).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn encode(dir: &Path, enc: &str, case: &Case<'_>, bits: u32) -> Vec<u8> {
    let input = format!("{}.src.yuv", case.name);
    std::fs::write(dir.join(&input), source(case.w, case.h, case.frames, bits)).unwrap();
    let stream = format!("{}.avs3", case.name);
    let recon = format!("{}.rec.yuv", case.name);
    let (w, h, f, d) = (case.w.to_string(), case.h.to_string(), case.frames.to_string(), bits.to_string());
    let mut args = vec!["-i", &input, "-o", &stream, "-r", &recon, "-w", &w, "-h", &h, "-d", &d, "-f", &f, "--fps_num", "25", "--fps_den", "1", "-v", "0"];
    if bits > 8 {
        args.extend(["--internal_bit_depth", "10"]);
    }
    args.extend(case.args.iter().copied());
    assert!(run_in(dir, enc, &args), "{}: {enc} failed", case.name);
    std::fs::read(dir.join(&stream)).unwrap()
}

fn run(cases: &[Case<'_>]) {
    let ten = cases.iter().any(|c| c.ten_bit);
    let (enc, dec) = if ten { ("uavs3e-10bit", "uavs3d-10bit") } else { ("uavs3e", "uavs3d") };
    if !have(enc) || !have(dec) {
        eprintln!("SKIPPED (AVS3 oracle): {enc} or {dec} is not installed (scripts/ci/install-avs3-oracle.sh)");
        return;
    }
    let dir = fixtures();
    let bits = if ten { 10 } else { 8 };
    for case in cases {
        let data = encode(&dir, enc, case, bits);
        let ours = decode(&data);
        let size = case.w * case.h * 3 / 2 * (bits as usize).div_ceil(8);
        assert_eq!(ours.len(), size * case.frames, "{}: decoded size", case.name);
        let reference = format!("{}.uavs3d.yuv", case.name);
        // uavs3d opens its output for appending.
        let _ = std::fs::remove_file(dir.join(&reference));
        let stream = format!("{}.avs3", case.name);
        assert!(run_in(&dir, dec, &["-i", &stream, "-o", &reference, "-l", "0", "-t", "1"]), "{}: {dec} failed", case.name);
        let want = std::fs::read(dir.join(&reference)).unwrap();
        assert!(want == ours, "{}: differs from uavs3d, {}", case.name, first_difference(&want, &ours));
        // The encoder's own reconstruction is only reported: for odd sizes and with some tools off
        // it differs from uavs3d (and from this crate, which matches uavs3d).
        if let Ok(rec) = std::fs::read(dir.join(format!("{}.rec.yuv", case.name)))
            && rec != ours
        {
            eprintln!("{}: note: the encoder's reconstruction differs, {}", case.name, first_difference(&rec, &ours));
        }
        eprintln!("{}: {} pictures bit-exact", case.name, case.frames);
    }
}

fn case<'a>(name: &'a str, w: usize, h: usize, frames: usize, args: &'a [&'a str]) -> Case<'a> {
    Case { name, w, h, frames, ten_bit: false, args }
}

#[test]
fn random_access_and_low_delay_match_uavs3d() {
    run(&[
        case("ra_ctu64", 192, 128, 6, &["-q", "28", "-p", "0", "-g", "3", "--ctu_size", "64"]),
        case("ra_ctu32", 192, 128, 6, &["-q", "28", "-p", "0", "-g", "3", "--ctu_size", "32"]),
        case("ra_gop8", 192, 128, 6, &["-q", "20", "-p", "0", "-g", "7"]),
        case("ld", 192, 128, 6, &["-q", "30", "-p", "0", "-g", "0"]),
        case("intra_period", 192, 128, 12, &["-q", "32", "-p", "4", "-g", "3", "--close_gop", "1"]),
        case("fast", 192, 128, 6, &["-q", "30", "-p", "0", "-g", "3", "--speed_level", "3"]),
        case("rate_control", 192, 128, 6, &["-q", "32", "-p", "0", "-g", "3", "--rc_type", "1", "--rc_crf", "30"]),
    ]);
}

#[test]
fn sizes_and_parameters_match_uavs3d() {
    run(&[
        case("odd_size", 200, 136, 6, &["-q", "30", "-p", "0", "-g", "3"]),
        case("wide", 320, 192, 4, &["-q", "28", "-p", "0", "-g", "3"]),
        case("qp_low", 192, 128, 6, &["-q", "5", "-p", "0", "-g", "3"]),
        case("qp_high", 192, 128, 6, &["-q", "55", "-p", "0", "-g", "3"]),
        case("adaptive_qp", 192, 128, 6, &["-q", "40", "-p", "0", "-g", "3", "--adaptive_dqp", "1"]),
        case("chroma_qp", 192, 128, 6, &["-q", "30", "-p", "0", "-g", "3", "--qp_offset_cb", "3", "--qp_offset_cr", "-4"]),
        case(
            "split_limits",
            192,
            128,
            6,
            &["-q", "32", "-p", "0", "-g", "3", "--min_qt_size", "16", "--max_bt_size", "32", "--max_eqt_size", "16", "--max_dt_size", "16"],
        ),
    ]);
}

#[test]
fn tools_match_uavs3d() {
    run(&[
        case(
            "all_tools",
            192,
            128,
            6,
            &["-q", "30", "-p", "0", "-g", "3", "--tscpm", "1", "--ipf", "1", "--affine", "1", "--umve", "1", "--amvr", "1", "--emvr", "1", "--smvd", "1"],
        ),
        case("tools_off", 192, 128, 6, &["-q", "30", "-p", "0", "-g", "3", "--hmvp", "0", "--dt_intra", "0", "--sectrans", "0", "--pbt", "0"]),
        case("no_deblock", 192, 128, 6, &["-q", "30", "-p", "0", "-g", "3", "--deblock", "0"]),
        case("no_sao_alf", 192, 128, 6, &["-q", "30", "-p", "0", "-g", "3", "--sao", "0", "--alf", "0"]),
        case("wq", 192, 128, 6, &["-q", "20", "-p", "0", "-g", "7", "--wq_enable", "1"]),
        case(
            "wq_picture",
            192,
            128,
            6,
            &["-q", "32", "-p", "0", "-g", "3", "--wq_enable", "1", "--pic_wq_data_idx", "1", "--wq_param", "1", "--wq_model", "1"],
        ),
        case("wq_picture2", 192, 128, 6, &["-q", "32", "-p", "0", "-g", "3", "--wq_enable", "1", "--pic_wq_data_idx", "2"]),
        case("wq_sequence", 192, 128, 6, &["-q", "32", "-p", "0", "-g", "3", "--seq_wq_mode", "1", "--wq_enable", "1"]),
    ]);
}

#[test]
fn main10_matches_uavs3d() {
    run(&[
        Case { name: "main10_ra", w: 192, h: 128, frames: 6, ten_bit: true, args: &["-q", "30", "-p", "0", "-g", "3"] },
        Case { name: "main10_lowqp", w: 192, h: 128, frames: 6, ten_bit: true, args: &["-q", "10", "-p", "0", "-g", "3"] },
        Case { name: "main10_highqp", w: 192, h: 128, frames: 6, ten_bit: true, args: &["-q", "50", "-p", "0", "-g", "7"] },
    ]);
}

/// Decoding-order-index wrap: 300 pictures overflow the 8-bit DOI.
#[test]
fn doi_wrap_matches_uavs3d() {
    run(&[case("doi_ld", 64, 64, 300, &["-q", "40", "-p", "0", "-g", "0"]), case("doi_ra", 64, 64, 300, &["-q", "40", "-p", "0", "-g", "15"])]);
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut s = seed;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }
}

/// Never crash: streams from the encoder with random bytes flipped, dropped or cut off decode to
/// errors or damaged pictures, never a panic or a hang.
#[test]
fn damaged_streams_do_not_panic() {
    if !have("uavs3e") {
        eprintln!("SKIPPED (AVS3 damaged streams): uavs3e is not installed");
        return;
    }
    let dir = fixtures();
    let mut streams = Vec::new();
    for (name, args) in [
        ("damage_ra", vec!["-q", "30", "-p", "0", "-g", "3"]),
        ("damage_ld", vec!["-q", "30", "-p", "0", "-g", "0", "--wq_enable", "1"]),
        ("damage_tools", vec!["-q", "30", "-p", "0", "-g", "3", "--affine", "1", "--umve", "1", "--tscpm", "1", "--ipf", "1"]),
    ] {
        let c = Case { name, w: 128, h: 96, frames: 6, ten_bit: false, args: &args };
        streams.push(encode(&dir, "uavs3e", &c, 8));
    }
    let mut next = rng(0x9e37_79b9_7f4a_7c15);
    let mut rnd = |n: usize| (next() % n.max(1) as u64) as usize;
    let rounds: usize = std::env::var("CFA4_AVS3_DAMAGE_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
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
        let mut dec = cfa4_avs3::Decoder::new();
        for (i, pkt) in cfa4_avs3::split_packets(&data).into_iter().enumerate() {
            let _ = dec.decode(pkt, i as i64);
        }
        let _ = dec.flush();
    }
}

/// Never crash, without the oracle: random packets behind AVS3 start codes.
#[test]
fn random_packets_do_not_panic() {
    let mut rnd = rng(0x1234_5678_9abc_def1);
    for _ in 0..400 {
        let mut dec = cfa4_avs3::Decoder::new();
        for _ in 0..8 {
            let code = [0xB0u8, 0xB3, 0xB6, 0x00, 0x01, 0xB7, 0xB1][(rnd() % 7) as usize];
            let mut pkt = vec![0, 0, 1, code];
            if code == 0xB0 && rnd().is_multiple_of(2) {
                pkt.extend_from_slice(&[0x20, 0x42, 0x80, 0x16, 0x00, 0x48, 0x8a]);
            }
            let n = (rnd() % 300) as usize;
            pkt.extend((0..n).map(|_| rnd() as u8));
            let _ = dec.decode(&pkt, 0);
        }
        let _ = dec.flush();
    }
}
