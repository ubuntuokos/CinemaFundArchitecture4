// SPDX-License-Identifier: GPL-3.0-or-later
//! Bit-exactness against the AVS3 reference implementations, built from pinned commits by
//! `scripts/ci/install-avs3-oracle.sh`: test streams are encoded here by uavs3e from a synthetic
//! moving picture (into `target/fixtures/avs3/`, never committed) and decoded by this crate and by
//! uavs3d (the decoder it is ported from, built C-only); every output picture must be equal, in the
//! same order. Where the encoder's reconstruction agrees with uavs3d, the output must equal that too.
//! The tests skip when a tool is missing. `CFA4_AVS3_ORACLE_DIR` names the directory holding
//! `uavs3enc`, `uavs3dec`, `uavs3enc-10bit` and `uavs3dec-10bit`; unset, they are looked up on `PATH`.
//!
//! uavs3e is used as a black-box test encoder only. Its multi-patch, `hmvp 0` and 8x8 minimum CU
//! settings write streams that its own reconstruction and uavs3d disagree on, so they are not covered.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tool(name: &str) -> PathBuf {
    match std::env::var_os("CFA4_AVS3_ORACLE_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    }
}

fn have(name: &str) -> bool {
    // the tools print a banner or usage and fail without arguments; spawning is what is checked
    Command::new(tool(name)).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/avs3");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A moving test picture: diagonal gradients, a sliding checkerboard, a bouncing block and
/// deterministic noise, so that intra, inter and the loop filters all have something to do.
fn source(w: usize, h: usize, frames: usize, bits: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3 / 2 * frames * if bits > 8 { 2 } else { 1 });
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 17) as i32 - 8
    };
    let put = |v: i32, out: &mut Vec<u8>| {
        let v = v.clamp(0, 255);
        if bits > 8 {
            out.extend_from_slice(&((v as u16) << (bits - 8)).to_le_bytes());
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
                put(v + noise(), &mut out);
            }
        }
        for c in 0..2 {
            for y in 0..h / 2 {
                for x in 0..w / 2 {
                    let v = 128 + ((x * (c + 1) + y * (2 - c) + f * 4) % 96) as i32 - 48;
                    put(v + noise() / 2, &mut out);
                }
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Check {
    /// uavs3d and the encoder's reconstruction.
    Both,
    /// uavs3d only (the encoder's reconstruction differs from uavs3d for these settings).
    Dec,
}

struct Case<'a> {
    name: &'a str,
    w: usize,
    h: usize,
    frames: usize,
    bits: u32,
    args: &'a [&'a str],
    check: Check,
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
        Some(i) => format!("first difference at byte {i} of {} / {}", a.len(), b.len()),
        None => format!("lengths {} / {}", a.len(), b.len()),
    }
}

/// Encodes `c`; returns the stream and the encoder's reconstruction, `None` without the tools.
fn encode(c: &Case<'_>) -> Option<(PathBuf, PathBuf)> {
    let ten = c.bits > 8;
    let (enc, dec) = if ten { ("uavs3enc-10bit", "uavs3dec-10bit") } else { ("uavs3enc", "uavs3dec") };
    if !have(enc) || !have(dec) {
        return None;
    }
    let dir = fixtures();
    let input = dir.join(format!("{}.src.yuv", c.name));
    std::fs::write(&input, source(c.w, c.h, c.frames, c.bits)).unwrap();
    let (stream, recon) = (dir.join(format!("{}.avs3", c.name)), dir.join(format!("{}.rec.yuv", c.name)));
    let mut cmd = Command::new(tool(enc));
    cmd.current_dir(&dir); // the encoder writes a psnr log into its working directory
    cmd.args(["-i"]).arg(&input);
    cmd.args(["-w", &c.w.to_string(), "-h", &c.h.to_string(), "-f", &c.frames.to_string()]);
    cmd.args(["-d", &c.bits.to_string(), "--fps_num", "25", "--fps_den", "1", "-v", "0"]);
    if ten {
        cmd.args(["--internal_bit_depth", "10"]);
    }
    cmd.arg("-o").arg(&stream).arg("-r").arg(&recon).args(c.args);
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?;
    assert!(status.success(), "{}: uavs3enc failed", c.name);
    Some((stream, recon))
}

fn run(c: &Case<'_>) -> bool {
    let Some((stream, recon)) = encode(c) else {
        eprintln!("SKIPPED (AVS3 {}): uavs3e / uavs3d are not installed (scripts/ci/install-avs3-oracle.sh)", c.name);
        return false;
    };
    let dec = if c.bits > 8 { "uavs3dec-10bit" } else { "uavs3dec" };
    let reference = stream.with_extension("ref.yuv");
    let status = Command::new(tool(dec)).arg("-i").arg(&stream).arg("-o").arg(&reference).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
    assert!(status.success(), "{}: uavs3dec failed", c.name);
    let want = std::fs::read(&reference).unwrap();
    let data = std::fs::read(&stream).unwrap();
    let got = decode(&data);
    assert!(got == want, "{}: differs from uavs3d: {}", c.name, first_difference(&got, &want));
    if c.check == Check::Both {
        let rec = std::fs::read(&recon).unwrap();
        assert!(got == rec, "{}: differs from the encoder's reconstruction: {}", c.name, first_difference(&got, &rec));
    }
    let info = cfa4_avs3::stream_info(&data).unwrap();
    assert_eq!((info.width as usize, info.height as usize, info.bit_depth), (c.w, c.h, c.bits), "{}", c.name);
    assert!(cfa4_avs3::probe(&data), "{}: probe", c.name);
    true
}

#[test]
fn encoded_streams_match_uavs3d() {
    let cases = [
        Case { name: "ra", w: 176, h: 144, frames: 12, bits: 8, args: &["-q", "30"], check: Check::Both },
        Case { name: "ra_odd", w: 200, h: 136, frames: 8, bits: 8, args: &["-q", "28"], check: Check::Dec },
        Case { name: "ra_tiny", w: 40, h: 24, frames: 4, bits: 8, args: &["-q", "30"], check: Check::Both },
        Case { name: "ra_small", w: 72, h: 56, frames: 5, bits: 8, args: &["-q", "30"], check: Check::Both },
        Case { name: "intra", w: 176, h: 144, frames: 10, bits: 8, args: &["-q", "38", "-g", "1"], check: Check::Both },
        Case { name: "low_delay", w: 176, h: 144, frames: 10, bits: 8, args: &["-q", "30", "-p", "0"], check: Check::Both },
        Case { name: "closed_gop", w: 176, h: 144, frames: 12, bits: 8, args: &["-q", "30", "-g", "7", "--close_gop", "1"], check: Check::Both },
        Case { name: "ctu32", w: 176, h: 144, frames: 10, bits: 8, args: &["-q", "30", "--ctu_size", "32"], check: Check::Both },
        Case {
            name: "ctu64_nofilter",
            w: 176,
            h: 144,
            frames: 10,
            bits: 8,
            args: &["-q", "30", "--ctu_size", "64", "--sao", "0", "--alf", "0"],
            check: Check::Both,
        },
        Case { name: "ctu128", w: 176, h: 144, frames: 10, bits: 8, args: &["-q", "34", "--ctu_size", "128"], check: Check::Both },
        Case { name: "hmvp4", w: 176, h: 144, frames: 8, bits: 8, args: &["-q", "30", "--hmvp", "4"], check: Check::Both },
        Case {
            name: "inter_tools_off",
            w: 176,
            h: 144,
            frames: 10,
            bits: 8,
            args: &["-q", "26", "--umve", "0", "--affine", "0", "--smvd", "0", "--amvr", "0"],
            check: Check::Dec,
        },
        Case {
            name: "intra_tools_off",
            w: 176,
            h: 144,
            frames: 8,
            bits: 8,
            args: &["-q", "30", "--dt_intra", "0", "--sectrans", "0", "--pbt", "0", "--tscpm", "0", "--ipf", "0"],
            check: Check::Both,
        },
        Case {
            name: "no_loop_filter",
            w: 176,
            h: 144,
            frames: 8,
            bits: 8,
            args: &["-q", "30", "--deblock", "0", "--sao", "0", "--alf", "0"],
            check: Check::Both,
        },
        Case {
            name: "weighted_quant",
            w: 176,
            h: 144,
            frames: 8,
            bits: 8,
            args: &["-q", "30", "--wq_enable", "1", "--seq_wq_mode", "0", "--wq_param", "1"],
            check: Check::Both,
        },
        Case {
            name: "delta_qp",
            w: 176,
            h: 144,
            frames: 8,
            bits: 8,
            args: &["-q", "30", "--adaptive_dqp", "1", "--chroma_dqp", "1", "--qp_offset_cb", "3", "--qp_offset_cr", "-2"],
            check: Check::Both,
        },
        Case { name: "ten_bit", w: 176, h: 144, frames: 10, bits: 10, args: &["-q", "30"], check: Check::Both },
        Case { name: "ten_bit_nofilter", w: 176, h: 144, frames: 10, bits: 10, args: &["-q", "30", "--sao", "0", "--alf", "0"], check: Check::Both },
        Case { name: "ten_bit_odd", w: 200, h: 136, frames: 8, bits: 10, args: &["-q", "28"], check: Check::Dec },
    ];
    let mut ran = 0;
    for c in &cases {
        if run(c) {
            ran += 1;
        }
    }
    eprintln!("AVS3 oracle: {ran} of {} streams checked", cases.len());
}

/// Damaged and truncated streams give errors or pictures, never a panic or a hang.
#[test]
fn damaged_streams_do_not_panic() {
    let c = Case { name: "damage", w: 96, h: 64, frames: 6, bits: 8, args: &["-q", "30"], check: Check::Dec };
    let Some((stream, _)) = encode(&c) else {
        eprintln!("SKIPPED (AVS3 damage): uavs3e is not installed (scripts/ci/install-avs3-oracle.sh)");
        return;
    };
    let data = std::fs::read(stream).unwrap();
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for round in 0..120 {
        let mut d = data.clone();
        match round % 3 {
            0 => d.truncate(next(d.len())),
            1 => {
                for _ in 0..1 + next(20) {
                    let i = next(d.len());
                    d[i] = next(256) as u8;
                }
            }
            _ => {
                let at = next(d.len());
                d.drain(at..(at + 1 + next(200)).min(d.len()));
            }
        }
        let mut dec = cfa4_avs3::Decoder::new();
        for pkt in cfa4_avs3::split_packets(&d) {
            let _ = dec.decode(pkt, 0);
        }
        let _ = dec.flush();
    }
}

#[test]
fn garbage_is_not_avs3() {
    assert!(!cfa4_avs3::probe(b""));
    assert!(!cfa4_avs3::probe(&[0, 0, 1, 0xb0]));
    assert!(!cfa4_avs3::probe(&[0, 0, 1, 0xb0, 0x20, 0, 0, 1, 0xb3, 0, 0, 0, 0]));
    assert!(cfa4_avs3::stream_info(&[0u8; 64]).is_err());
    let mut dec = cfa4_avs3::Decoder::new();
    for junk in [&[][..], &[0, 0, 1, 0xb3], &[0, 0, 1, 0xb0, 0x22, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[0xff; 100]] {
        let _ = dec.decode(junk, 0);
    }
    assert!(dec.flush().is_empty());
}
