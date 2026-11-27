//! JCT-VC range-extension conformance streams (`draft_conformance/RExt`, one folder per stream
//! holding `<name>.bit` and the md5 of the reference decoder's output yuv).
//!
//! The streams are not in the repository: point `CFA4_HEVC_REXT_DIR` at the folder with the
//! unpacked zips (default `/mnt/project-files/samples/hevc-rext`). Without them the test skips.
//! `SUPPORTED` lists the streams that must decode bit-exactly; every other stream must be refused
//! cleanly (an error, never a wrong picture).

mod common;

use cfa4_hevc::Picture;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Streams that decode bit-exactly (grows with each tool).
const SUPPORTED: &[&str] = &[
    "ADJUST_IPRED_ANGLE_A_RExt_Mitsubishi_2",
    "GENERAL_10b_420_RExt_Sony_1",
    "GENERAL_12b_400_RExt_Sony_1",
    "GENERAL_12b_420_RExt_Sony_1",
    "GENERAL_8b_400_RExt_Sony_1",
    "GENERAL_8b_420_RExt_Sony_1",
    "IPCM_A_RExt_NEC_2",
    "EXTPREC_HIGHTHROUGHPUT_444_16_INTRA_16BIT_RExt_Sony_1",
    "EXTPREC_MAIN_444_16_INTRA_16BIT_RExt_Sony_1",
    "GENERAL_16b_400_RExt_Sony_1",
    "GENERAL_16b_444_RExt_Sony_2",
    "GENERAL_16b_444_highThroughput_RExt_Sony_2",
    "Bitdepth_B_RExt_Sony_1",
    "Bitdepth_A_RExt_Sony_1",
    "CCP_10bit_RExt_QCOM_1",
    "CCP_8bit_RExt_QCOM_1",
    "ExplicitRdpcm_A_BBC_1",
    "PERSIST_RPARAM_A_RExt_Sony_3",
    "QMATRIX_A_RExt_Sony_1",
    "WPP_AND_TILE_10Bit422Test_HIGH_TP_444_10BIT_RExt_Apple_2",
    "WPP_AND_TILE_AND_CABAC_BYPASS_ALIGN_0_HIGH_TP_444_14BIT_RExt_Apple_2",
    "WPP_AND_TILE_AND_CABAC_BYPASS_ALIGN_1_HIGH_TP_444_14BIT_RExt_Apple_2",
    "WPP_AND_TILE_AND_CABAC_EXT_PREC_1_HIGH_TP_444_14BIT_RExt_Apple_2",
    "WPP_AND_TILE_HIGH_TP_444_8BIT_RExt_Apple_2",
    "WPP_HIGH_TP_444_8BIT_RExt_Apple_2",
    "EXTPREC_HIGHTHROUGHPUT_444_16_INTRA_10BIT_RExt_Sony_1",
    "EXTPREC_HIGHTHROUGHPUT_444_16_INTRA_12BIT_RExt_Sony_1",
    "EXTPREC_HIGHTHROUGHPUT_444_16_INTRA_8BIT_RExt_Sony_1",
    "EXTPREC_MAIN_444_16_INTRA_10BIT_RExt_Sony_1",
    "EXTPREC_MAIN_444_16_INTRA_12BIT_RExt_Sony_1",
    "EXTPREC_MAIN_444_16_INTRA_8BIT_RExt_Sony_1",
    "GENERAL_10b_422_RExt_Sony_1",
    "GENERAL_10b_444_RExt_Sony_2",
    "GENERAL_12b_422_RExt_Sony_1",
    "GENERAL_12b_444_RExt_Sony_2",
    "GENERAL_8b_444_RExt_Sony_2",
    "IPCM_B_RExt_NEC",
    "Main_422_10_A_RExt_Sony_2",
    "Main_422_10_B_RExt_Sony_2",
    "TSCTX_10bit_I_RExt_SHARP_1",
    "TSCTX_10bit_RExt_SHARP_1",
    "TSCTX_8bit_I_RExt_SHARP_1",
    "TSCTX_8bit_RExt_SHARP_1",
    "WAVETILES_RExt_Sony_2",
];

fn dir() -> Option<PathBuf> {
    let d = std::env::var_os("CFA4_HEVC_REXT_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/mnt/project-files/samples/hevc-rext"));
    d.is_dir().then_some(d)
}

/// `<root>/<name>/**/<name>.bit` and the md5 of the yuv from the md5 file next to it.
fn streams(root: &Path) -> Vec<(String, PathBuf, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let mut files = Vec::new();
        collect(&e.path(), &mut files);
        let Some(bit) = files.iter().find(|f| matches!(f.extension().and_then(|x| x.to_str()), Some("bit" | "bin"))) else { continue };
        let md5 = files.iter().filter(|f| matches!(f.extension().and_then(|x| x.to_str()), Some("md5" | "txt"))).find_map(|f| {
            let text = std::fs::read_to_string(f).ok()?;
            text.lines().find(|l| l.contains(".yuv")).and_then(|l| l.split_whitespace().next()).filter(|h| h.len() == 32).map(str::to_string)
        });
        if let Some(md5) = md5 {
            out.push((name, bit.clone(), md5));
        }
    }
    out.sort();
    out
}

fn collect(d: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else {
                out.push(p);
            }
        }
    }
}

/// The reference decoder's yuv layout: every picture's planes in output order, 8-bit samples as
/// bytes, deeper ones (of luma or chroma) as 16-bit little endian; 4:0:0 has the luma plane only.
fn yuv(pics: &[Picture], mono: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for p in pics {
        // HM writes every plane as 16-bit words as soon as one of the two depths exceeds 8
        let wide = matches!(p.y, cfa4_hevc::Plane::U16(_));
        let planes: &[(&cfa4_hevc::Plane, usize, usize, usize)] = &[
            (&p.y, p.width as usize, p.height as usize, p.y_stride),
            (&p.u, p.chroma_width as usize, p.chroma_height as usize, p.uv_stride),
            (&p.v, p.chroma_width as usize, p.chroma_height as usize, p.uv_stride),
        ];
        for (k, (pl, w, h, stride)) in planes.iter().enumerate() {
            if mono && k > 0 {
                break;
            }
            for y in 0..*h {
                for x in 0..*w {
                    let s = pl.get(y * stride + x);
                    if wide {
                        out.extend_from_slice(&s.to_le_bytes());
                    } else {
                        out.push(s as u8);
                    }
                }
            }
        }
    }
    out
}

fn md5(data: &[u8]) -> Option<String> {
    let mut c = Command::new("md5sum").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
    let mut stdin = c.stdin.take()?;
    let data = data.to_vec();
    let t = std::thread::spawn(move || stdin.write_all(&data));
    let out = c.wait_with_output().ok()?;
    let _ = t.join();
    String::from_utf8(out.stdout).ok()?.split_whitespace().next().map(str::to_string)
}

#[test]
fn range_extension_conformance_streams() {
    let Some(root) = dir() else {
        eprintln!("skipped: no RExt conformance streams");
        return;
    };
    if Command::new("md5sum").arg("--version").output().is_err() {
        eprintln!("skipped: no md5sum");
        return;
    }
    let mut failures = Vec::new();
    let mut report = Vec::new();
    for (name, bit, want) in streams(&root) {
        let mono = name.contains("_400_");
        let supported = SUPPORTED.contains(&name.as_str());
        let got = match common::decode_file_threads(&bit, 0) {
            Ok(pics) => md5(&yuv(&pics, mono)).unwrap_or_default(),
            Err((_, e, _)) => {
                report.push(format!("{name}: refused ({e})"));
                if supported {
                    failures.push(format!("{name}: {e}"));
                }
                continue;
            }
        };
        let ok = got == want;
        report.push(format!("{name}: {}", if ok { "bit-exact" } else { "MISMATCH" }));
        if supported && !ok {
            failures.push(format!("{name}: md5 {got} != {want}"));
        }
        if !supported && ok {
            failures.push(format!("{name} decodes bit-exactly: add it to SUPPORTED"));
        }
        if !supported && !ok {
            failures.push(format!("{name} decodes to a wrong picture instead of being refused"));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
