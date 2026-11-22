// SPDX-License-Identifier: GPL-3.0-or-later
//! Bit-exact tests against Samsung's xevd v0.8.0 Baseline decoder (`xevdb_app`), the code this
//! crate ports: streams encoded by xeve v0.7.1 (`xeveb_app`, from ffmpeg's `testsrc2`) with
//! hierarchical B GOPs, low delay, intra periods and odd sizes, plus the synthetic
//! streams of `synth` (random coding decisions, mono, cropping, QP deltas). Every output picture
//! must equal xevd's, in the same order. The tools are external test oracles built by
//! `scripts/ci/install-evc-oracle.sh`; the tests skip when they (or ffmpeg) are missing.

mod synth;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_evc::{Chroma, Decoder, Frame, split_nal_units};
use synth::{Content, PicCfg, Rng, SLICE_B, SLICE_I, SLICE_P, SeqCfg};

/// A tool on `PATH` (or in `CFA4_EVC_ORACLE_DIR`).
fn tool(name: &str) -> Option<PathBuf> {
    let dirs = std::env::var_os("CFA4_EVC_ORACLE_DIR").into_iter().chain(std::env::var_os("PATH")).flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>());
    dirs.map(|d| d.join(name)).find(|p| p.is_file())
}

fn dir() -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/evc/xevd");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn decode(data: &[u8]) -> Vec<Frame> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for nal in split_nal_units(data) {
        if let Some(f) = d.decode_nal(nal).expect("decodes") {
            out.push(f);
        }
    }
    out.extend(d.flush());
    out
}

/// A picture as xevd writes it with `--output-bit-depth 10`: planar, every sample as 16-bit little
/// endian and unscaled; 4:0:0 as luma only. (Its 8-bit output shifts the samples right by two as
/// if they had 10 bits, whatever the stream's depth, so the tests always take 16-bit output.)
fn as_xevd_bytes(f: &Frame) -> Vec<u8> {
    let planes = if f.chroma == Chroma::Mono { 1 } else { 3 };
    let mut out = Vec::new();
    for p in f.planes.iter().take(planes) {
        for &s in p {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

/// Decodes `evc` with xevd: its pictures, as `as_xevd_bytes` writes them.
fn run_xevd(xevd: &Path, name: &str, evc: &Path) -> Vec<u8> {
    let yuv = evc.with_extension("xevd.yuv");
    let o = Command::new(xevd).args(["-v", "0", "-i"]).arg(evc).arg("-o").arg(&yuv).args(["--output-bit-depth", "10"]).output().unwrap();
    assert!(o.status.success(), "{name}: xevd failed: {}", String::from_utf8_lossy(&o.stderr));
    std::fs::read(&yuv).unwrap()
}

/// Crops xevd's uncropped 4:2:0 pictures (`w`×`h`) by the SPS offsets `[left, right, top, bottom]`
/// (in chroma samples). xevd's application writes a cropped picture's top-left corner instead of
/// the window the offsets select, so cropped streams are checked against the uncropped twin.
fn crop_xevd(full: &[u8], w: usize, h: usize, c: [u32; 4]) -> Vec<u8> {
    let [l, r, t, b] = c.map(|v| v as usize);
    let planes = [(w, h, 2 * l, 2 * t, w - 2 * (l + r), h - 2 * (t + b)), (w / 2, h / 2, l, t, w / 2 - l - r, h / 2 - t - b)];
    let pic = w * h * 3; // bytes: 1.5 samples per pixel, 2 bytes each
    let mut out = Vec::new();
    for p in full.chunks_exact(pic) {
        let mut base = 0;
        for &(pw, ph, x0, y0, cw, ch) in &[planes[0], planes[1], planes[1]] {
            for y in 0..ch {
                let s = base + ((y0 + y) * pw + x0) * 2;
                out.extend_from_slice(&p[s..s + cw * 2]);
            }
            base += pw * ph * 2;
        }
    }
    out
}

/// Checks this crate's pictures of `evc` against `want`; panics on the first difference.
fn check(name: &str, evc: &Path, want: &[u8]) {
    let got = decode(&std::fs::read(evc).unwrap());
    let mut pos = 0usize;
    for (i, f) in got.iter().enumerate() {
        let g = as_xevd_bytes(f);
        let w = want.get(pos..pos + g.len()).unwrap_or_else(|| panic!("{name}: xevd gave fewer pictures ({i})"));
        if g != w {
            let at = g.iter().zip(w).position(|(a, b)| a != b).unwrap_or(0);
            let _ = std::fs::write(evc.with_extension("ours.yuv"), got.iter().flat_map(as_xevd_bytes).collect::<Vec<u8>>());
            panic!("{name}: picture {i} (poc {}) differs at byte {at} of {}", f.poc, g.len());
        }
        pos += g.len();
    }
    assert_eq!(pos, want.len(), "{name}: xevd gave more pictures than {}", got.len());
}

/// Decodes `evc` with xevd and with this crate; panics on the first difference.
fn compare(xevd: &Path, name: &str, evc: &Path) {
    check(name, evc, &run_xevd(xevd, name, evc));
}

struct Enc {
    name: &'static str,
    size: (u32, u32),
    frames: u32,
    args: &'static [&'static str],
}

/// xeve v0.7.1 crashes when asked for 8-bit coding (`--codec-bit-depth 8`), so its streams are
/// 10-bit (its default); the synthetic streams cover 8 bits. `-I` must be a multiple of the GOP
/// (`-b` + 1).
const ENCODED: &[Enc] = &[
    Enc { name: "ra_b15", size: (352, 288), frames: 20, args: &["-b", "15"] },
    Enc { name: "ra_b7_q27", size: (320, 240), frames: 17, args: &["-b", "7", "-q", "27"] },
    Enc { name: "ld_p_q36", size: (176, 144), frames: 12, args: &["-b", "0", "-q", "36"] },
    Enc { name: "keyint_odd", size: (198, 110), frames: 18, args: &["-b", "3", "-I", "8", "-q", "30"] },
    Enc { name: "intra_q22", size: (128, 96), frames: 4, args: &["-I", "1", "-b", "0", "-q", "22"] },
    Enc { name: "preset_fast_q45", size: (256, 144), frames: 10, args: &["--preset", "fast", "-q", "45"] },
];

#[test]
fn xeve_streams_match_xevd() {
    let (Some(xevd), Some(xeve)) = (tool("xevdb_app"), tool("xeveb_app")) else {
        eprintln!("SKIPPED (EVC xevd oracle): xevdb_app / xeveb_app not installed (scripts/ci/install-evc-oracle.sh)");
        return;
    };
    let have_ffmpeg = Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if !have_ffmpeg {
        eprintln!("SKIPPED (EVC xevd oracle): ffmpeg is not installed");
        return;
    }
    let d = dir();
    for c in ENCODED {
        let (w, h) = c.size;
        let src = d.join(format!("{}.yuv", c.name));
        let lavfi = format!("testsrc2=size={w}x{h}:rate=25,noise=alls=12:allf=t");
        let o = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &lavfi, "-frames:v", &c.frames.to_string(), "-pix_fmt", "yuv420p", "-f", "rawvideo"])
            .arg(&src)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}: ffmpeg: {}", c.name, String::from_utf8_lossy(&o.stderr));
        let evc = d.join(format!("{}.evc", c.name));
        let o = Command::new(&xeve)
            .args(["-v", "0", "-i"])
            .arg(&src)
            .args(["-w", &w.to_string(), "-h", &h.to_string(), "-z", "25", "--frames", &c.frames.to_string(), "--profile", "baseline", "-o"])
            .arg(&evc)
            .args(c.args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}: xeve failed: {}", c.name, String::from_utf8_lossy(&o.stderr));
        compare(&xevd, c.name, &evc);
    }
}

fn pic(slice_type: u32, idr: bool, tid: u32, num_refp: [u32; 2], qp: u32) -> PicCfg {
    PicCfg { slice_type, idr, tid, num_refp, qp, qp_u_offset: 1, qp_v_offset: -1, deblock: true, content: Content::Random }
}

#[test]
fn synthetic_streams_match_xevd() {
    let Some(xevd) = tool("xevdb_app") else {
        eprintln!("SKIPPED (EVC xevd oracle): xevdb_app not installed (scripts/ci/install-evc-oracle.sh)");
        return;
    };
    let d = dir();
    let ra = |n: usize| -> Vec<PicCfg> {
        let mut v = vec![pic(SLICE_I, true, 0, [0, 0], 30)];
        for k in 0..n {
            v.push(pic(SLICE_B, false, 0, [1, 1], 28 + (k as u32 % 5)));
            v.push(pic(SLICE_B, false, 1, [1, 1], 32));
        }
        v
    };
    let ld = |n: u32| -> Vec<PicCfg> {
        let mut v = vec![pic(SLICE_I, true, 0, [0, 0], 26)];
        for k in 1..n {
            v.push(pic(SLICE_P, false, 0, [k.min(2), 0], 30));
        }
        v
    };
    let cases: Vec<(&str, SeqCfg, Vec<PicCfg>)> = vec![
        ("synth_ra", SeqCfg { width: 120, height: 72, log2_sub_gop_length: 1, max_num_ref_pics: 1, ..SeqCfg::default() }, ra(4)),
        ("synth_ld_10bit_dqp", SeqCfg { width: 80, height: 64, bit_depth: 10, max_num_ref_pics: 2, cu_qp_delta: true, ..SeqCfg::default() }, ld(8)),
        // xevd v0.8.0 runs chroma motion compensation on 4:0:0 inter pictures and crashes, so the
        // monochrome stream is intra only.
        (
            "synth_mono",
            SeqCfg { width: 64, height: 40, chroma: false, ..SeqCfg::default() },
            (0..4).map(|k| pic(SLICE_I, k == 0, 0, [0, 0], 24 + 4 * k)).collect(),
        ),
        ("synth_crop_cip", SeqCfg { width: 96, height: 80, crop: Some([2, 6, 4, 2]), cip: true, max_num_ref_pics: 2, ..SeqCfg::default() }, ld(6)),
    ];
    let make = |seq: &SeqCfg, pics: &[PicCfg], seed: u64, path: &Path| {
        let mut rng = Rng::new(seed);
        let mut nals = vec![synth::sps(seq), synth::pps(seq)];
        for p in pics {
            nals.push(synth::slice(seq, p, &mut rng));
        }
        std::fs::write(path, synth::stream(&nals)).unwrap();
    };
    // `CFA4_EVC_SYNTH_ROUNDS` repeats the cases with other seeds (local stress runs).
    let rounds: u64 = std::env::var("CFA4_EVC_SYNTH_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for (k, (name, seq, pics)) in (0..rounds).flat_map(|r| cases.iter().cloned().enumerate().map(move |(k, c)| (k as u64 + 100 * r, c))) {
        let seed = 900 + k;
        let name = &format!("{name}#{seed}");
        let evc = d.join(format!("{name}.evc"));
        make(&seq, &pics, seed, &evc);
        match seq.crop {
            None => compare(&xevd, name, &evc),
            Some(c) => {
                // The same coding decisions without cropping, for xevd.
                let twin = d.join(format!("{name}.full.evc"));
                make(&SeqCfg { crop: None, ..seq }, &pics, seed, &twin);
                let full = run_xevd(&xevd, name, &twin);
                check(name, &evc, &crop_xevd(&full, seq.width as usize, seq.height as usize, c));
            }
        }
    }
}
