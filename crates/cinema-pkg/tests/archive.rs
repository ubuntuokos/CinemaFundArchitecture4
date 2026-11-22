//! Provenance report and BagIt bags: a valid bag, a damaged one, an interrupted one.

use std::fs;
use std::path::{Path, PathBuf};

use cfa4_cinema_pkg::archive::{self, Check, FileHash, PROVENANCE_FILE, Provenance, Tool};
use cfa4_cinema_pkg::delivery::INCOMPLETE;
use chrono::NaiveDate;

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("archive-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn package(root: &Path) -> PathBuf {
    let pkg = root.join("Demo_TST-1_F_EN-XX_INT-TL_51_2K_NULL_20261115_NUL_SMPTE_OV");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("ASSETMAP.xml"), b"<AssetMap/>").unwrap();
    fs::write(pkg.join("j2c.mxf"), vec![7u8; 2_500_000]).unwrap();
    pkg
}

fn provenance() -> Provenance {
    Provenance {
        package: "Demo_TST-1_F_EN-XX_INT-TL_51_2K_NULL_20261115_NUL_SMPTE_OV".into(),
        created: "2026-11-15T10:00:00Z".into(),
        producer: Tool { name: "cfa4".into(), version: "0.10.0".into() },
        profile: "DCP SMPTE 2K 24 fps".into(),
        transcoders: vec![Tool { name: "cfa4-j2k".into(), version: "0.10.0".into() }],
        sources: vec![FileHash { path: "master.mov".into(), sha256: "ab".repeat(32) }],
        checks: vec![Check {
            tool: Tool { name: "asdcplib".into(), version: "2.13.2".into() },
            profile: "asdcp-info".into(),
            result: "pass".into(),
            at: "2026-11-15T10:05:00+01:00".into(),
            summary: String::new(),
        }],
    }
}

#[test]
fn bag_is_valid_and_damage_is_found() {
    let root = scratch("bag");
    let pkg = package(&root.join("src"));
    let bag = root.join("archive").join("demo-bag");
    fs::create_dir_all(bag.parent().unwrap()).unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
    let b = archive::make_bag(&pkg, &bag, date, &provenance(), "CFA4 test package").unwrap();
    assert_eq!((b.files, b.bytes), (2, 2_500_000 + 11));
    assert!(archive::validate_bag(&bag).unwrap().is_empty(), "{:?}", archive::validate_bag(&bag));
    let info = fs::read_to_string(bag.join("bag-info.txt")).unwrap();
    assert!(info.contains("Payload-Oxum: 2500011.2\n") && info.contains("Bagging-Date: 2026-11-15\n"), "{info}");
    let prov: Provenance = serde_json::from_str(&fs::read_to_string(bag.join(PROVENANCE_FILE)).unwrap()).unwrap();
    assert_eq!(prov, provenance());
    assert!(archive::make_bag(&pkg, &bag, date, &provenance(), "x").is_err(), "never overwritten");

    let payload = bag.join("data").join(pkg.file_name().unwrap()).join("j2c.mxf");
    let mut d = fs::read(&payload).unwrap();
    d[1_000_000] ^= 1;
    fs::write(&payload, d).unwrap();
    fs::write(bag.join("data/stray.txt"), b"x").unwrap();
    let mut tag = fs::read_to_string(bag.join(PROVENANCE_FILE)).unwrap();
    tag.push(' ');
    fs::write(bag.join(PROVENANCE_FILE), tag).unwrap();
    let problems = archive::validate_bag(&bag).unwrap();
    for want in ["j2c.mxf: changed", "data/stray.txt: not in the manifest", "Payload-Oxum", "cfa4-provenance.json: changed"] {
        assert!(problems.iter().any(|p| p.contains(want)), "{want}: {problems:?}");
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn interrupted_bag_is_incomplete_and_resumes() {
    let root = scratch("resume");
    let pkg = package(&root.join("src"));
    let bag = root.join("bag");
    let date = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
    // A payload delivered, then a crash before the tag files: only BAG.INCOMPLETE exists.
    let staging = root.join(format!("bag{INCOMPLETE}"));
    fs::create_dir_all(staging.join("data")).unwrap();
    cfa4_cinema_pkg::delivery::deliver(&pkg, &staging.join("data")).unwrap();
    assert!(!bag.exists());
    archive::make_bag(&pkg, &bag, date, &provenance(), "resumed").unwrap();
    assert!(!staging.exists());
    assert!(archive::validate_bag(&bag).unwrap().is_empty());
    assert!(!bag.join("data").join(format!("{}.sha256", pkg.file_name().unwrap().to_str().unwrap())).exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn provenance_is_checked() {
    let mut p = provenance();
    assert!(p.to_json().is_ok());
    p.created = "15/11/2026".into();
    assert!(p.to_json().is_err());
    let mut p = provenance();
    p.sources[0].sha256 = "AB".repeat(32);
    assert!(p.to_json().is_err());
    let root = scratch("desc");
    let pkg = package(&root.join("src"));
    assert!(archive::make_bag(&pkg, &root.join("b"), NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), &provenance(), "two\nlines").is_err());
    assert!(!root.join(format!("b{INCOMPLETE}")).exists(), "refused before anything is written");
    fs::remove_dir_all(&root).unwrap();
}
