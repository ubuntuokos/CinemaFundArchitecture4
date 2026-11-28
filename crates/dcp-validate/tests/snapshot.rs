//! DCP-24: exact copy, manifest and repair log, on a small made-up package (no media).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use cfa4_dcp_validate::snapshot::*;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-snapshot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn package(d: &Path) -> PathBuf {
    let p = d.join("Pkg");
    std::fs::create_dir_all(p.join("sub")).unwrap();
    std::fs::write(p.join("ASSETMAP.xml"), "<AssetMap/>").unwrap();
    std::fs::write(p.join("a.mxf"), vec![7u8; 3_000_000]).unwrap(); // more than one read step
    std::fs::write(p.join("sub/b.xml"), "<B/>").unwrap();
    p
}

fn snap(root: &Path, dest: &Path) -> Result<Manifest, SnapshotError> {
    snapshot(root, dest, &AtomicBool::new(false), &|_, _, _| {})
}

#[test]
fn the_copy_is_byte_identical_and_the_manifest_names_every_file() {
    let d = scratch("copy");
    let pkg = package(&d);
    let dest = d.join("backup");
    let m = snap(&pkg, &dest).unwrap();
    assert_eq!(m.schema, MANIFEST_SCHEMA);
    assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["ASSETMAP.xml", "a.mxf", "sub/b.xml"]);
    for f in &m.files {
        assert_eq!(std::fs::read(pkg.join(&f.path)).unwrap(), std::fs::read(dest.join(PACKAGE_DIR).join(&f.path)).unwrap(), "{}", f.path);
        assert_eq!((f.sha256.len(), f.sha1.len()), (64, 40));
    }
    // SHA-256 and SHA-1 of "<B/>", computed independently
    let b = m.files.iter().find(|f| f.path == "sub/b.xml").unwrap();
    assert_eq!(b.size, 4);
    assert_eq!(b.sha256, "81edeb4b6472f5cc4fef6f511f8949202e71d4b145a2469440005d998826f527");
    assert_eq!(b.sha1, "3fe304e67ed9f48f27405535818af0ded7cdf9de");
    // the manifest on disk is the one returned, and its id is stable
    let text = std::fs::read_to_string(dest.join(MANIFEST_FILE)).unwrap();
    assert_eq!(Manifest::from_json(&text).unwrap(), m);
    assert_eq!(m.id(), Manifest::from_json(&text).unwrap().id());
    // the package itself is untouched
    assert!(verify(&pkg, &m, &AtomicBool::new(false)).unwrap().identical);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn verify_names_changed_missing_and_extra_files() {
    let d = scratch("verify");
    let pkg = package(&d);
    let m = snap(&pkg, &d.join("backup")).unwrap();
    std::fs::write(pkg.join("sub/b.xml"), "<C/>").unwrap(); // same size, other bytes
    std::fs::remove_file(pkg.join("ASSETMAP.xml")).unwrap();
    std::fs::write(pkg.join("new.xml"), "x").unwrap();
    let v = verify(&pkg, &m, &AtomicBool::new(false)).unwrap();
    assert!(!v.identical);
    let states: Vec<(&str, FileState)> = v.differences.iter().map(|f| (f.path.as_str(), f.state)).collect();
    assert_eq!(states, [("ASSETMAP.xml", FileState::Missing), ("sub/b.xml", FileState::Changed), ("new.xml", FileState::Extra)]);
    assert_eq!(v.counts["same"], 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_snapshot_never_goes_inside_the_package_or_over_something() {
    let d = scratch("refuse");
    let pkg = package(&d);
    assert!(matches!(snap(&pkg, &pkg.join("backup")), Err(SnapshotError::InsidePackage(_))));
    assert!(!pkg.join("backup").exists());
    assert!(matches!(snap(&pkg, &d), Err(SnapshotError::Exists(_))));
    assert!(matches!(snap(&pkg, &d.join("Pkg2/../Pkg/x")), Err(SnapshotError::Io { .. })), "a path with .. is refused");
    assert!(!pkg.join("x").exists());
    assert!(matches!(snap(&d.join("nope"), &d.join("b")), Err(SnapshotError::NotAFolder(_))));
    // a link that points into the package does not hide the copy inside it
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&pkg, d.join("link")).unwrap();
        assert!(matches!(snap(&pkg, &d.join("link/inner")), Err(SnapshotError::InsidePackage(_))));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_cancelled_snapshot_leaves_nothing_behind() {
    let d = scratch("cancel");
    let pkg = package(&d);
    let dest = d.join("backup");
    let r = snapshot(&pkg, &dest, &AtomicBool::new(true), &|_, _, _| {});
    assert!(matches!(r, Err(SnapshotError::Cancelled)));
    assert!(!dest.exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn links_are_listed_not_copied() {
    #[cfg(unix)]
    {
        let d = scratch("links");
        let pkg = package(&d);
        std::os::unix::fs::symlink("a.mxf", pkg.join("alias.mxf")).unwrap();
        let m = snap(&pkg, &d.join("backup")).unwrap();
        assert_eq!(m.skipped, ["alias.mxf"]);
        assert!(!d.join("backup/package/alias.mxf").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn the_log_only_grows_and_ties_an_action_to_the_copy_and_the_reports() {
    let d = scratch("log");
    let pkg = package(&d);
    let m = snap(&pkg, &d.join("backup")).unwrap();
    let log = d.join("backup").join(LOG_FILE);
    let mut first = LogEntry::new("snapshot", "Pkg", &m);
    first.report_before = Some(report_id("{\"verdict\":\"FAIL\"}"));
    append_log(&log, &first).unwrap();
    let mut second = LogEntry::new("reexport", "Pkg", &m);
    second.rule = Some("DCP.NAME.001".into());
    second.result = Some("Pkg_r2".into());
    append_log(&log, &second).unwrap();
    let entries = read_log(&log).unwrap();
    assert_eq!(entries, [first, second]);
    assert!(entries.iter().all(|e| e.manifest == m.id() && e.schema == LOG_SCHEMA));
    // a damaged log is an error, not silently shorter
    std::fs::write(&log, "{\"schema\":\"x\"}\nnot json\n").unwrap();
    assert!(read_log(&log).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_manifest_of_another_schema_is_refused() {
    assert!(Manifest::from_json("{}").is_err());
    assert!(Manifest::from_json("{\"schema\":\"other/1\",\"created_at\":\"\",\"cfa4_version\":\"\",\"root\":\"\",\"files\":[]}").is_err());
}
