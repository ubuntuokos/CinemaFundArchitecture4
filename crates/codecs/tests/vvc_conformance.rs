//! JVET conformance of the external VVC decoder (ADR 0013, VVC-02, gate G2; plan section 6).
//!
//! Every bitstream of the JVET `FDIS_r1` corpus goes through
//! `cfa4_vvc::annexb_access_units` -> `ExternalVvcDecoder` -> `cfa4-vvc-decode` (VVdeC), and the
//! MD5 of the output YUV (every picture, in output order) is compared with the stream's
//! `.yuv.md5` made by the VTM. Three stream kinds are judged by the rule the corpus itself gives:
//!
//! * 4:0:0 streams: the VTM writes the luma plane only, so the luma plane is compared (CFA4's
//!   frames always carry three planes).
//! * `STILL_B_1`, `STILL444_B_1`: the `.yuv.md5` is of the first picture only (a Main 10 Still
//!   Picture decoder decodes only that one, see the stream's description), so the first output
//!   picture is compared.
//! * The streams in [`UNSUPPORTED`] use tools or layers the Main 10 VVdeC build does not decode
//!   (multi-layer, 4:2:2 / 4:4:4 screen-content tools). They must be refused with a decode
//!   error, never crash, hang or give wrong pictures; a stream that starts to decode is reported
//!   so the list shrinks.
//!
//! The test needs the unpacked corpus (`scripts/dev/vvc-conformance.py fetch`, or
//! `CFA4_VVC_CONFORMANCE_DIR`) and the helper (`scripts/dev/vvc-decoder.py build`, or
//! `CFA4_VVC_DECODER`); otherwise it reports SKIPPED. `CFA4_VVC_CONFORMANCE_FILTER` is a
//! substring of the stream names to run, `CFA4_VVC_CONFORMANCE_JOBS` the parallel decodes
//! (default 2). The corpus and VVdeC are restricted components (`cfa4-restricted-extras`), so it
//! skips even with `CFA4_REQUIRE_ORACLES=1`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use cfa4_codecs::video::{DecodedFrame, VideoDecoder};
use cfa4_codecs::vvc_external::{self, ExternalVvcDecoder};
use cfa4_frame::PixelData;

/// Streams the helper is expected to refuse: 10 multi-layer / VPS-OLS streams, and 21 4:2:2 /
/// 4:4:4 streams with palette, ACT and other screen-content tools (FDIS_r1, VVdeC v3.2.1).
const UNSUPPORTED: &[&str] = &[
    "10b422_G_5",
    "10b422_H_5",
    "10b422_I_5",
    "10b422_J_5",
    "10b422_K_5",
    "10b422_L_5",
    "8b422_G_5",
    "8b422_H_5",
    "8b422_I_5",
    "8b422_J_5",
    "8b422_K_5",
    "8b422_L_5",
    "8b444_A_2",
    "8b444_B_2",
    "ACT_A_3",
    "ACT_B_3",
    "PALETTE_A_2",
    "PALETTE_B_2",
    "PALETTE_C_2",
    "PALETTE_D_2",
    "PALETTE_E_2",
    "ILRPL_A_2",
    "OLS_A_5",
    "OLS_B_5",
    "OLS_C_5",
    "OPI_B_3",
    "SPATSCAL444_A_2",
    "SPATSCAL_A_3",
    "VPS_A_3",
    "VPS_B_1",
    "VPS_C_2",
];

/// The 4:0:0 streams: the VTM output has the luma plane only.
const MONOCHROME: &[&str] = &["10b400_A_2", "10b400_B_2", "8b400_A_2", "8b400_B_2", "SCALING_A_1"];

/// Streams whose reference MD5 covers the first picture only.
const FIRST_PICTURE_ONLY: &[&str] = &["STILL_B_1", "STILL444_B_1"];

fn helper() -> Option<PathBuf> {
    if let Some(p) = vvc_external::configured_helper() {
        return Some(p);
    }
    let root = cfa4_testkit::workspace_root();
    let pin = std::fs::read_to_string(root.join("packaging/vvc-decode/vvdec.json")).ok()?;
    let pin: serde_json::Value = serde_json::from_str(&pin).ok()?;
    let commit = pin.get("commit")?.as_str()?.to_string();
    let prefix = root.join("target/vvc-decoder");
    let stamp = std::fs::read_to_string(prefix.join("VVDEC_COMMIT")).ok()?;
    let exe = prefix.join("bin/cfa4-vvc-decode");
    (stamp.trim() == commit && exe.is_file()).then_some(exe)
}

fn corpus_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CFA4_VVC_CONFORMANCE_DIR").map_or_else(|| cfa4_testkit::fixtures_dir("vvc-conformance"), PathBuf::from);
    dir.is_dir().then_some(dir)
}

/// A streaming MD5 through `md5sum` (no MD5 crate in the workspace).
struct Md5(Child);

impl Md5 {
    fn new() -> Md5 {
        Md5(Command::new("md5sum").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("md5sum"))
    }

    fn update(&mut self, data: &[u8]) {
        self.0.stdin.as_mut().unwrap().write_all(data).unwrap();
    }

    fn finish(mut self) -> String {
        drop(self.0.stdin.take());
        let out = self.0.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string()
    }
}

/// The MD5s of what the decoder gave: all pictures, the luma planes, the first picture.
struct Hashes {
    all: Md5,
    luma: Md5,
    first: Md5,
    pictures: usize,
}

impl Hashes {
    fn add(&mut self, f: &DecodedFrame) {
        let mut all = Vec::new();
        let mut luma = Vec::new();
        match &f.frame.data {
            PixelData::Yuv8 { planes, .. } => {
                for (i, p) in planes.iter().enumerate() {
                    all.extend_from_slice(p);
                    if i == 0 {
                        luma.extend_from_slice(p);
                    }
                }
            }
            PixelData::Yuv16 { planes, .. } => {
                for (i, p) in planes.iter().enumerate() {
                    let b: Vec<u8> = p.iter().flat_map(|v| v.to_le_bytes()).collect();
                    all.extend_from_slice(&b);
                    if i == 0 {
                        luma.extend_from_slice(&b);
                    }
                }
            }
            other => panic!("unexpected pixel data {:?}", std::mem::discriminant(other)),
        }
        self.all.update(&all);
        self.luma.update(&luma);
        if self.pictures == 0 {
            self.first.update(&all);
        }
        self.pictures += 1;
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Pass(&'static str),
    Unsupported,
    Mismatch(String),
    Error(String),
}

fn run_stream(helper: &Path, dir: &Path, name: &str) -> Outcome {
    let files: Vec<PathBuf> = walk(dir);
    let bit: Vec<&PathBuf> = files.iter().filter(|f| f.extension().is_some_and(|e| e == "bit")).collect();
    let md5 = files.iter().find(|f| f.to_string_lossy().to_lowercase().ends_with(".yuv.md5"));
    let (&[bit], Some(md5)) = (bit.as_slice(), md5) else {
        return Outcome::Error(format!("{name}: need one .bit and a .yuv.md5"));
    };
    let want = std::fs::read_to_string(md5).unwrap().to_lowercase();
    let data = std::fs::read(bit).unwrap();
    let mut dec = match ExternalVvcDecoder::new(helper, 0) {
        Ok(d) => d,
        Err(e) => return Outcome::Error(format!("{name}: helper: {e}")),
    };
    dec.set_request_timeout(Duration::from_secs(300));
    let mut h = Hashes { all: Md5::new(), luma: Md5::new(), first: Md5::new(), pictures: 0 };
    let mut errors = 0;
    for (i, (off, len)) in cfa4_vvc::annexb_access_units(&data).into_iter().enumerate() {
        match dec.decode(&data[off..off + len], i as i64) {
            Ok(frames) => frames.iter().for_each(|f| h.add(f)),
            Err(_) => errors += 1,
        }
    }
    dec.flush().iter().for_each(|f| h.add(f));
    let (all, luma, first) = (h.all.finish(), h.luma.finish(), h.first.finish());
    if h.pictures == 0 && errors > 0 {
        return Outcome::Unsupported;
    }
    if errors > 0 {
        return Outcome::Error(format!("{name}: {errors} access units failed, {} pictures came out", h.pictures));
    }
    if FIRST_PICTURE_ONLY.contains(&name) {
        return if want.contains(&first) { Outcome::Pass("first picture") } else { Outcome::Mismatch(format!("{name}: first picture {first}")) };
    }
    if want.contains(&all) {
        Outcome::Pass("all pictures")
    } else if MONOCHROME.contains(&name) {
        if want.contains(&luma) { Outcome::Pass("luma plane") } else { Outcome::Mismatch(format!("{name}: luma {luma}")) }
    } else {
        Outcome::Mismatch(format!("{name}: all {all}, wanted {want}"))
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn jvet_fdis_r1_conformance() {
    let ctx = "vvc_conformance::jvet_fdis_r1_conformance";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some(corpus) = corpus_dir() else {
        return eprintln!("SKIPPED ({ctx}): no corpus (scripts/dev/vvc-conformance.py fetch, or CFA4_VVC_CONFORMANCE_DIR)");
    };
    let filter = std::env::var("CFA4_VVC_CONFORMANCE_FILTER").unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(&corpus)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(&filter))
        .collect();
    names.sort();
    if names.is_empty() {
        return eprintln!("SKIPPED ({ctx}): the corpus directory has no stream matching the filter");
    }
    let jobs: usize = std::env::var("CFA4_VVC_CONFORMANCE_JOBS").ok().and_then(|v| v.parse().ok()).unwrap_or(2).max(1);

    let next = Mutex::new(0usize);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    let Some(name) = names.get(i) else { break };
                    let outcome = run_stream(&helper, &corpus.join(name), name);
                    eprintln!("{name}: {outcome:?}");
                    results.lock().unwrap().push((name.clone(), outcome));
                }
            });
        }
    });

    let results = results.into_inner().unwrap();
    let mut bad = Vec::new();
    for (name, outcome) in &results {
        let refused = UNSUPPORTED.contains(&name.as_str());
        match (outcome, refused) {
            (Outcome::Pass(_), false) | (Outcome::Unsupported, true) => {}
            (Outcome::Pass(_), true) => bad.push(format!("{name} now decodes: remove it from UNSUPPORTED")),
            (Outcome::Unsupported, false) => bad.push(format!("{name} is refused but is not on the UNSUPPORTED list")),
            (Outcome::Mismatch(m) | Outcome::Error(m), _) => bad.push(m.clone()),
        }
    }
    let passed = results.iter().filter(|(_, o)| matches!(o, Outcome::Pass(_))).count();
    let unsupported = results.iter().filter(|(_, o)| *o == Outcome::Unsupported).count();
    eprintln!("{} streams: {passed} PASS, {unsupported} unsupported, {} problems", results.len(), bad.len());
    assert!(bad.is_empty(), "conformance problems:\n{}", bad.join("\n"));
}
