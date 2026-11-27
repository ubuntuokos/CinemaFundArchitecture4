//! Verified delivery: interrupted copies, damaged assets, bit identity after delivery (the WP13
//! gate), and refusal of names a delivery drive cannot hold.

use std::fs;
use std::path::{Path, PathBuf};

use cfa4_cinema_pkg::Error;
use cfa4_cinema_pkg::delivery::{self, INCOMPLETE, PROGRESS};

/// A fresh directory under the target dir, removed again by the test.
fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("delivery-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// A small package: an ASSETMAP, a CPL and three "track files" of 3 MB each (several read steps).
fn package(root: &Path) -> PathBuf {
    let pkg = root.join("Demo_TST-1_F_EN-XX_INT-TL_51_2K_NULL_20261115_NUL_SMPTE_OV");
    fs::create_dir_all(pkg.join("sub")).unwrap();
    fs::write(pkg.join("ASSETMAP.xml"), b"<AssetMap/>").unwrap();
    fs::write(pkg.join("CPL_demo.xml"), b"<CompositionPlaylist/>").unwrap();
    for (i, name) in ["j2c_a.mxf", "wav_b.mxf", "sub/sub_c.mxf"].iter().enumerate() {
        let data: Vec<u8> = (0..3_000_000u32).map(|k| (k.wrapping_mul(2_654_435_761).wrapping_add(i as u32) >> 24) as u8).collect();
        fs::write(pkg.join(name), data).unwrap();
    }
    pkg
}

fn same_tree(a: &Path, b: &Path) {
    let fa = delivery::package_files(a).unwrap();
    assert_eq!(fa, delivery::package_files(b).unwrap());
    for f in fa {
        assert!(fs::read(a.join(&f)).unwrap() == fs::read(b.join(&f)).unwrap(), "{f} differs");
    }
}

#[test]
fn delivers_bit_identical_with_manifest() {
    let root = scratch("ok");
    let pkg = package(&root.join("src"));
    let target = root.join("drive");
    fs::create_dir_all(&target).unwrap();
    let r = delivery::deliver(&pkg, &target).unwrap();
    assert_eq!((r.files, r.resumed), (5, 0));
    same_tree(&pkg, &r.package);
    assert!(delivery::verify(&r.package, &r.manifest).unwrap().is_empty());
    assert!(!target.join(format!("{}{INCOMPLETE}", pkg.file_name().unwrap().to_str().unwrap())).exists());
    assert!(!r.package.join(PROGRESS).exists());
    // Same package, same manifest; an existing delivery is never overwritten.
    let first = fs::read_to_string(&r.manifest).unwrap();
    let t2 = root.join("drive2");
    fs::create_dir_all(&t2).unwrap();
    assert_eq!(fs::read_to_string(delivery::deliver(&pkg, &t2).unwrap().manifest).unwrap(), first);
    assert!(matches!(delivery::deliver(&pkg, &target), Err(Error::Delivery(_))));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn interrupted_delivery_stays_incomplete_and_resumes() {
    let root = scratch("resume");
    let pkg = package(&root.join("src"));
    let target = root.join("drive");
    fs::create_dir_all(&target).unwrap();
    let name = pkg.file_name().unwrap().to_str().unwrap().to_string();
    // Stop in the middle of the third file (after 2 complete files and one step of the third).
    let mut steps = 0;
    let mut stop = |rel: &str| {
        steps += 1;
        !(rel == "sub/sub_c.mxf" && steps > 2 + 2 + 3)
    };
    assert!(matches!(delivery::deliver_with(&pkg, &target, &mut stop), Err(Error::Cancelled)));
    let staging = target.join(format!("{name}{INCOMPLETE}"));
    assert!(staging.is_dir() && !target.join(&name).exists(), "an interrupted package must not look complete");
    // Damage one already verified file: the resume must notice and copy it again.
    let mut damaged = fs::read(staging.join("CPL_demo.xml")).unwrap();
    damaged[0] ^= 0xFF;
    fs::write(staging.join("CPL_demo.xml"), damaged).unwrap();
    let r = delivery::deliver(&pkg, &target).unwrap();
    assert!(r.resumed >= 1 && r.resumed < r.files, "resumed {}", r.resumed);
    same_tree(&pkg, &r.package);
    assert!(delivery::verify(&r.package, &r.manifest).unwrap().is_empty());
    assert!(!staging.exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn damaged_delivery_is_reported() {
    let root = scratch("damage");
    let pkg = package(&root.join("src"));
    let target = root.join("drive");
    fs::create_dir_all(&target).unwrap();
    let r = delivery::deliver(&pkg, &target).unwrap();
    let mut b = fs::read(r.package.join("wav_b.mxf")).unwrap();
    b[1_500_000] ^= 1;
    fs::write(r.package.join("wav_b.mxf"), b).unwrap();
    fs::remove_file(r.package.join("ASSETMAP.xml")).unwrap();
    fs::write(r.package.join("extra.txt"), b"x").unwrap();
    let mut got: Vec<(String, &str)> = delivery::verify(&r.package, &r.manifest).unwrap().into_iter().map(|m| (m.path, m.kind)).collect();
    got.sort();
    assert_eq!(got, [("ASSETMAP.xml".into(), "missing"), ("extra.txt".into(), "unexpected"), ("wav_b.mxf".into(), "changed")]);
    // A manifest that points outside the package is refused, not followed.
    for evil in ["../src", "/etc/passwd", "a/./b", "a//b", "..\\x"] {
        let m = root.join("evil.sha256");
        fs::write(&m, format!("{}  {evil}\n", "0".repeat(64))).unwrap();
        assert!(delivery::verify(&r.package, &m).is_err(), "{evil}");
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn undeliverable_names_are_refused_before_copying() {
    let root = scratch("names");
    let pkg = package(&root.join("src"));
    fs::write(pkg.join("bad name.xml"), b"x").unwrap();
    fs::write(pkg.join("ékezet.xml"), b"x").unwrap();
    fs::write(pkg.join(format!("{}.mxf", "a".repeat(300))), b"x").unwrap_or(());
    let issues = delivery::check_names(&pkg).unwrap();
    assert!(issues.iter().any(|i| i.path == "bad name.xml"));
    assert!(issues.iter().any(|i| i.path == "ékezet.xml"));
    let target = root.join("drive");
    fs::create_dir_all(&target).unwrap();
    assert!(matches!(delivery::deliver(&pkg, &target), Err(Error::Delivery(_))));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0, "nothing written");
    #[cfg(unix)]
    {
        fs::remove_file(pkg.join("bad name.xml")).unwrap();
        fs::remove_file(pkg.join("ékezet.xml")).unwrap();
        std::os::unix::fs::symlink("ASSETMAP.xml", pkg.join("link.xml")).unwrap();
        assert!(delivery::deliver(&pkg, &target).is_err(), "symbolic links are refused");
    }
    fs::remove_dir_all(&root).unwrap();
}
