//! The ISDCF registry snapshot (`data/isdcf-registry.json`) against an upstream checkout.
//!
//! With `CFA4_ISDCF_REGISTRIES` naming a checkout of <https://github.com/ISDCF/registries>, the
//! checkout is reduced again and must give the committed snapshot byte for byte (offline / online
//! parity). `CFA4_ISDCF_WRITE=1` writes the reduction instead: that is how
//! `scripts/dev/update-isdcf-snapshot.sh` refreshes it. Without a checkout the test checks only
//! that the snapshot loads.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_cinema_pkg::isdcf::{self, Registry};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn snapshot_matches_upstream_checkout() {
    let committed = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/isdcf-registry.json");
    let Some(dir) = std::env::var_os("CFA4_ISDCF_REGISTRIES") else {
        let reg = Registry::builtin().unwrap();
        assert!(reg.content_types.iter().any(|e| e.code == "FTR"));
        assert_eq!(reg.source.files.len(), isdcf::UPSTREAM_FILES.len());
        eprintln!("skipped: CFA4_ISDCF_REGISTRIES is not set (scripts/dev/update-isdcf-snapshot.sh --check)");
        return;
    };
    let dir = PathBuf::from(dir);
    let commit = git(&dir, &["rev-parse", "HEAD"]);
    let date = git(&dir, &["log", "-1", "--format=%cs"]);
    let text = isdcf::reduce(&dir, &commit, &date).unwrap();
    if std::env::var("CFA4_ISDCF_WRITE").as_deref() == Ok("1") {
        std::fs::write(&committed, text).unwrap();
        return;
    }
    assert_eq!(std::fs::read_to_string(&committed).unwrap(), text, "snapshot differs from {commit}");
    let reg = Registry::builtin().unwrap();
    assert_eq!(reg.source.commit, commit);
}
