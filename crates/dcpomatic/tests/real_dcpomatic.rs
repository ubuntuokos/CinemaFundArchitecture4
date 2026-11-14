//! The pinned DCP-o-matic itself (plan CFA4-DCPOMATIC-001, phase 2 acceptance): a short master →
//! `Dcpomatic::make_dcp` → CFA4's native validator, `full-integrity` profile, SMPTE and Interop.
//!
//! Runs only when [`ENV_BIN_DIR`] names a DCP-o-matic build (the CI job `dcpomatic.yml`, or a
//! development build: `docs/development/dcpomatic.md`) and `ffmpeg` can make the master;
//! otherwise it says so and passes.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_dcp_validate::{DcpValidator as _, NativeValidator, Profile, Status, ValidationControl, ValidationRequest, Verdict};
use cfa4_dcpomatic::{Container, Content, ContentKind, Control, DcpRequest, Dcpomatic, ENV_BIN_DIR, MIN_VERSION, Resolution, Standard};

/// Two seconds of 2K Flat ProRes 4444 with 24-bit PCM, as CFA4's `file.exportMedia` would hand over.
fn master() -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures::fixtures_dir("dcpomatic").join("master-2s.mov");
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1998x1080:rate=24:duration=2", "-f", "lavfi", "-i"])
            .args(["sine=frequency=440:sample_rate=48000:duration=2", "-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuv444p10le"])
            .args(["-c:a", "pcm_s24le"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn make_and_validate(d: &Dcpomatic, master: &Path, work: &Path, standard: Standard) {
    let tag = format!("{standard:?}").to_lowercase();
    let mut r = DcpRequest::new(vec![Content::new(master)], work.join(&tag), format!("CFA4 DCPOM2 {tag}"));
    r.content_kind = ContentKind::Test;
    r.standard = standard;
    r.resolution = Some(Resolution::TwoK);
    r.container = Some(Container::Flat);
    r.frame_rate = Some(24);
    r.config_dir = Some(work.join("config"));
    let out = d.make_dcp(&r, &Control::new()).unwrap_or_else(|e| panic!("{tag}: {e}"));
    assert_eq!(out.version, MIN_VERSION, "the pinned build");
    let profile = Profile::builtin("full-integrity").expect("full-integrity profile");
    let report = NativeValidator.validate(&ValidationRequest::new(&out.dcp, profile), &ValidationControl::new()).expect("validation runs");
    // Only the rule ids and messages of what failed: the full report carries the package's
    // document ids, which need not end up in CI logs.
    let failed: Vec<String> =
        report.findings.iter().filter(|f| f.status != Status::Pass).map(|f| format!("{} {:?}: {}", f.rule, f.status, f.message)).collect();
    println!("{tag}: {:?}, {} findings, not passed: {failed:?}", report.verdict, report.findings.len());
    assert_eq!(report.verdict, Verdict::Pass, "{tag}: {failed:?}");
}

#[test]
fn a_real_dcp_passes_full_integrity() {
    if std::env::var_os(ENV_BIN_DIR).is_none_or(|v| v.is_empty()) {
        eprintln!("skipped: {ENV_BIN_DIR} is not set (no DCP-o-matic build)");
        return;
    }
    let d = Dcpomatic::locate().expect("the DCP-o-matic build in CFA4_DCPOMATIC_BIN_DIR");
    let Some(master) = master() else {
        eprintln!("skipped: ffmpeg could not make the test master");
        return;
    };
    let work = std::env::temp_dir().join(format!("cfa4-dcpom2-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");
    make_and_validate(&d, &master, &work, Standard::Smpte);
    make_and_validate(&d, &master, &work, Standard::Interop);
    let _ = std::fs::remove_dir_all(&work);
}
