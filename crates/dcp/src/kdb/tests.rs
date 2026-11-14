//! Key Delivery Bundle tests: archives written by `crate::testing::ustar` and by GNU tar
//! (`--format=ustar`, oracle; skipped when missing), and hostile archives.

use super::*;
use crate::testing::{catalog, ustar};

const CPL: &str = "11111111-1111-1111-1111-111111111111";

fn bundle_files() -> Vec<(String, Vec<u8>)> {
    vec![
        ("CATALOG".into(), catalog(&[("a.xml", CPL), ("feature-1/b.xml", CPL), ("missing.xml", CPL)]).into_bytes()),
        ("CONTENT/a.xml".into(), b"<A/>".to_vec()),
        ("CONTENT/feature-1/b.xml".into(), b"<B/>".to_vec()),
        ("CONTENT/unlisted.xml".into(), b"<C/>".to_vec()),
        ("README".into(), b"other top-level files are ignored".to_vec()),
    ]
}

fn check(b: &Bundle) {
    assert_eq!(b.id.as_deref(), Some("urn:uuid:00000000-0000-0000-0000-0000000000b1"));
    assert_eq!(b.annotation.as_deref(), Some("Test bundle"));
    assert_eq!(b.creator.as_deref(), Some("cfa4 tests"));
    let got: Vec<(&str, &str)> = b.kdms.iter().map(|k| (k.path.as_str(), k.xml.as_str())).collect();
    assert_eq!(got, [("a.xml", "<A/>"), ("feature-1/b.xml", "<B/>"), ("unlisted.xml", "<C/>")]);
    assert_eq!(b.kdms[0].cpl_id.as_deref(), Some(&*format!("urn:uuid:{CPL}")));
    assert_eq!(b.kdms[0].recipient.as_deref(), Some("CN=SM.server,O=test"));
    assert_eq!(b.kdms[1].not_after.as_deref(), Some("2099-01-01T00:00:00Z"));
    assert_eq!(b.kdms[2].cpl_id, None);
    assert!(b.issues.iter().any(|i| i.contains("missing.xml") && i.contains("not in the bundle")), "{:?}", b.issues);
}

#[test]
fn bundles_read_with_their_catalog() {
    let files = bundle_files();
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, d)| (p.as_str(), d.as_slice())).collect();
    let tar = ustar(&refs);
    assert!(is_ustar(&tar));
    check(&read_bundle(&tar).unwrap());
    // a long path goes into the prefix field
    let long = format!("CONTENT/{}/k.xml", "d".repeat(120));
    let cat = catalog(&[(&long["CONTENT/".len()..], CPL)]);
    let b = read_bundle(&ustar(&[("CATALOG", cat.as_bytes()), (&long, b"<K/>")])).unwrap();
    assert_eq!(b.kdms[0].xml, "<K/>");
}

#[test]
fn gnu_tar_bundles_read() {
    if std::process::Command::new("tar").arg("--version").output().is_err() {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "tar not found");
        eprintln!("SKIPPED: tar not found");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cfa4-kdb-{}", std::process::id()));
    for (p, d) in bundle_files() {
        let path = dir.join(&p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, d).unwrap();
    }
    let out = std::process::Command::new("tar")
        .current_dir(&dir)
        .args(["--format=ustar", "--owner=root", "--group=root", "--mode=a=rX", "-cf", "bundle.tar", "CATALOG", "CONTENT", "README"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tar = std::fs::read(dir.join("bundle.tar")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    check(&read_bundle(&tar).unwrap());
}

#[test]
fn hostile_bundles_are_errors_not_panics() {
    let files = bundle_files();
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, d)| (p.as_str(), d.as_slice())).collect();
    let tar = ustar(&refs);
    assert!(matches!(read_bundle(b"not a tar"), Err(KdmError::NotKdm(_))));
    assert!(read_bundle(&ustar(&[("CONTENT/a.xml", b"<A/>")])).is_err(), "no CATALOG");
    assert!(read_bundle(&ustar(&[("CATALOG", catalog(&[]).as_bytes())])).is_err(), "no KDM");
    assert!(read_bundle(&ustar(&[("CATALOG", b"<Other/>"), ("CONTENT/a.xml", b"<A/>")])).is_err());
    // a damaged header
    let mut bad = tar.clone();
    bad[10] ^= 1;
    assert!(read_bundle(&bad).unwrap_err().to_string().contains("checksum"));
    // a size larger than the archive
    let mut big = ustar(&[("CATALOG", b"<Catalog/>")]);
    big[124..136].copy_from_slice(b"77777777777\0");
    big[148..156].copy_from_slice(b"        ");
    let sum: u32 = big[..512].iter().map(|&b| u32::from(b)).sum();
    big[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    assert!(read_bundle(&big).is_err());
    // links and paths leaving the archive are ignored
    let mut link = ustar(&[("CONTENT/evil.xml", b""), ("CATALOG", catalog(&[("a.xml", CPL)]).as_bytes()), ("CONTENT/a.xml", b"<A/>"), ("../x.xml", b"<X/>")]);
    link[156] = b'2';
    link[148..156].copy_from_slice(b"        ");
    let sum: u32 = link[..512].iter().map(|&b| u32::from(b)).sum();
    link[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let b = read_bundle(&link).unwrap();
    assert_eq!(b.kdms.len(), 1);
    assert!(b.issues.iter().any(|i| i.contains("evil.xml") && i.contains("not a regular file")), "{:?}", b.issues);
    assert!(b.issues.iter().any(|i| i.contains("../x.xml") && i.contains("not a relative path")), "{:?}", b.issues);
    // every truncation is an error or a smaller bundle, never a panic
    for cut in (0..tar.len()).step_by(37) {
        let _ = read_bundle(&tar[..cut]);
    }
}
