//! KDN-45: the zip and tar writers, checked against independent readers written in this file from
//! the format specifications (not against the writers' own code).

use std::path::{Path, PathBuf};

use crate::project_archive::{Format, crc32, pack, tar_header, zip_central_header, zip_end, zip_local_header};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let d = cfa4_testkit::workspace_root().join("target").join("archive-tests").join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let files: [(&str, Vec<u8>); 4] = [
        ("project.fcproj", b"project".to_vec()),
        ("clip.mov", (0..3000u32).map(|i| (i * 7 % 251) as u8).collect()),
        ("Proxies/clip.mp4", vec![9u8; 512]),
        ("Proxies/deep/a b é.wav", Vec::new()),
    ];
    for (n, d) in &files {
        let p = root.join(n);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, d).unwrap();
    }
    let mut v: Vec<(String, Vec<u8>)> = files.iter().map(|(n, d)| (n.to_string(), d.clone())).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn never() -> bool {
    false
}

fn no_progress(_: usize) {}

/// Entries of a ustar archive: (name, bytes), verifying every header checksum.
fn read_tar(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 512 <= b.len() && b[at..at + 512].iter().any(|x| *x != 0) {
        let h = &b[at..at + 512];
        let mut sum = 0u32;
        for (i, x) in h.iter().enumerate() {
            sum += if (148..156).contains(&i) { 32 } else { u32::from(*x) };
        }
        let stored = u32::from_str_radix(std::str::from_utf8(&h[148..154]).unwrap(), 8).unwrap();
        assert_eq!(sum, stored, "header checksum");
        assert_eq!(&h[257..263], b"ustar\0");
        let field = |r: std::ops::Range<usize>| String::from_utf8(h[r].iter().copied().take_while(|c| *c != 0).collect()).unwrap();
        let size = usize::from_str_radix(field(124..135).trim(), 8).unwrap();
        let (prefix, name) = (field(345..500), field(0..100));
        let full = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        at += 512;
        out.push((full, b[at..at + size].to_vec()));
        at += size.div_ceil(512) * 512;
    }
    // two zero blocks end the archive
    assert!(b.len() >= at + 1024 && b[at..at + 1024].iter().all(|x| *x == 0));
    out
}

#[test]
fn crc32_matches_the_standard_check_value() {
    assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926, "continuing from a partial crc");
    assert_eq!(crc32(0, b""), 0);
}

#[test]
fn tar_round_trips_a_folder_in_sorted_order() {
    let s = Scratch::new("tar");
    let src = s.0.join("src");
    let want = tree(&src);
    let out = s.0.join("a.tar");
    let n = pack(&src, &out, Format::Tar, &never, &no_progress).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(n as usize, bytes.len());
    assert_eq!(bytes.len() % 512, 0);
    assert_eq!(read_tar(&bytes), want);
    assert!(!PathBuf::from(format!("{}.part", out.display())).exists(), "no partial file is left");
}

#[test]
fn tar_splits_a_long_path_into_prefix_and_name() {
    let s = Scratch::new("tar-long");
    let src = s.0.join("src");
    let dir = format!("{}/{}", "d".repeat(90), "e".repeat(40));
    let rel = format!("{dir}/file.bin");
    std::fs::create_dir_all(src.join(&dir)).unwrap();
    std::fs::write(src.join(&rel), b"x").unwrap();
    let out = s.0.join("l.tar");
    pack(&src, &out, Format::Tar, &never, &no_progress).unwrap();
    assert_eq!(read_tar(&std::fs::read(&out).unwrap()), vec![(rel, b"x".to_vec())]);
}

#[test]
fn tar_refuses_a_path_it_cannot_store_and_leaves_nothing() {
    let s = Scratch::new("tar-toolong");
    let src = s.0.join("src");
    let name = "n".repeat(120); // one component longer than the 100-byte name field, no `/` to split at
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join(&name), b"x").unwrap();
    let out = s.0.join("t.tar");
    let e = pack(&src, &out, Format::Tar, &never, &no_progress).unwrap_err();
    assert!(e.contains("too long"), "{e}");
    assert!(!out.exists());
    assert!(!PathBuf::from(format!("{}.part", out.display())).exists());
}

/// A zip reader that trusts only the central directory: (name, crc, bytes), with the local headers
/// cross-checked.
fn read_zip(b: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
    let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
    let u32at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
    let eocd = (0..=b.len() - 22).rev().find(|&o| u32at(o) == 0x0605_4b50).expect("end of central directory");
    let (count, cd_size, cd_off) = (u16at(eocd + 10), u32at(eocd + 12), u32at(eocd + 16));
    assert_eq!(cd_off + cd_size, eocd, "no ZIP64 record is needed for a small archive");
    let mut at = cd_off;
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(u32at(at), 0x0201_4b50);
        assert_eq!(u16at(at + 8) & 0x0800, 0x0800, "UTF-8 names");
        assert_eq!(u16at(at + 10), 0, "stored");
        let (crc, size) = (u32at(at + 16), u32at(at + 24));
        let (nl, el, cl) = (u16at(at + 28), u16at(at + 30), u16at(at + 32));
        let off = u32at(at + 42);
        let name = String::from_utf8(b[at + 46..at + 46 + nl].to_vec()).unwrap();
        assert_eq!(u32at(off), 0x0403_4b50);
        let (lnl, lel) = (u16at(off + 26), u16at(off + 28));
        assert_eq!(&b[off + 30..off + 30 + lnl], name.as_bytes(), "local and central names agree");
        let data = b[off + 30 + lnl + lel..off + 30 + lnl + lel + size].to_vec();
        assert_eq!(crc32(0, &data) as usize, crc);
        out.push((name, crc as u32, data));
        at += 46 + nl + el + cl;
    }
    out
}

#[test]
fn zip_round_trips_a_folder_with_correct_crcs() {
    let s = Scratch::new("zip");
    let src = s.0.join("src");
    let want = tree(&src);
    let out = s.0.join("a.zip");
    let n = pack(&src, &out, Format::Zip, &never, &no_progress).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(n as usize, bytes.len());
    let got: Vec<(String, Vec<u8>)> = read_zip(&bytes).into_iter().map(|(n, _, d)| (n, d)).collect();
    assert_eq!(got, want);
}

#[test]
fn an_empty_folder_packs_to_a_valid_empty_archive() {
    let s = Scratch::new("empty");
    let src = s.0.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let z = s.0.join("e.zip");
    pack(&src, &z, Format::Zip, &never, &no_progress).unwrap();
    assert_eq!(std::fs::read(&z).unwrap().len(), 22, "only the end record");
    let t = s.0.join("e.tar");
    pack(&src, &t, Format::Tar, &never, &no_progress).unwrap();
    assert_eq!(std::fs::read(&t).unwrap(), vec![0u8; 1024]);
}

#[test]
fn a_cancel_stops_the_pack_and_leaves_no_archive() {
    let s = Scratch::new("cancel");
    let src = s.0.join("src");
    tree(&src);
    for (fmt, name) in [(Format::Zip, "c.zip"), (Format::Tar, "c.tar")] {
        let out = s.0.join(name);
        let e = pack(&src, &out, fmt, &|| true, &no_progress).unwrap_err();
        assert_eq!(e, "cancelled");
        assert!(!out.exists());
        assert!(!PathBuf::from(format!("{}.part", out.display())).exists());
    }
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_never_followed() {
    let s = Scratch::new("symlink");
    let src = s.0.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("real.txt"), b"real").unwrap();
    let outside = s.0.join("secret.txt");
    std::fs::write(&outside, b"secret").unwrap();
    std::os::unix::fs::symlink(&outside, src.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(&s.0, src.join("dirlink")).unwrap();
    let out = s.0.join("l.zip");
    pack(&src, &out, Format::Zip, &never, &no_progress).unwrap();
    let names: Vec<String> = read_zip(&std::fs::read(&out).unwrap()).into_iter().map(|e| e.0).collect();
    assert_eq!(names, vec!["real.txt".to_string()]);
}

#[test]
fn progress_counts_every_packed_file() {
    let s = Scratch::new("progress");
    let src = s.0.join("src");
    let want = tree(&src);
    let seen = std::cell::Cell::new(0usize);
    pack(&src, &s.0.join("p.tar"), Format::Tar, &never, &|n| seen.set(n)).unwrap();
    assert_eq!(seen.get(), want.len());
}

fn u16at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

const BIG: u64 = 5 * 1024 * 1024 * 1024; // 5 GiB: past what a classic zip field holds

#[test]
fn zip64_is_used_exactly_where_a_size_needs_it() {
    // small: the classic header, no extra field
    let small = zip_local_header("a.bin", 7, 100).unwrap();
    assert_eq!((u16at(&small, 4), u32at(&small, 18), u16at(&small, 28)), (20, 100, 0));
    // big: version 45, 0xFFFFFFFF sizes, a ZIP64 extra field holding both 64-bit sizes
    let big = zip_local_header("a.bin", 7, BIG).unwrap();
    assert_eq!((u16at(&big, 4), u32at(&big, 18), u32at(&big, 22), u16at(&big, 28)), (45, u32::MAX, u32::MAX, 20));
    let extra = 30 + "a.bin".len();
    assert_eq!((u16at(&big, extra), u16at(&big, extra + 2)), (1, 16));
    assert_eq!((u64at(&big, extra + 4), u64at(&big, extra + 12)), (BIG, BIG));
}

#[test]
fn zip64_central_header_carries_only_the_fields_that_overflow() {
    let name = "m.mov";
    // only the offset is large: one 8-byte value
    let c = zip_central_header(name, 1, 10, BIG).unwrap();
    assert_eq!((u32at(&c, 20), u32at(&c, 24), u32at(&c, 42)), (10, 10, u32::MAX));
    let e = 46 + name.len();
    assert_eq!((u16at(&c, e), u16at(&c, e + 2), u64at(&c, e + 4)), (1, 8, BIG));
    // both size and offset: uncompressed size, compressed size, then the offset, in that order
    let c = zip_central_header(name, 1, BIG, BIG + 1).unwrap();
    assert_eq!((u32at(&c, 20), u32at(&c, 24), u32at(&c, 42)), (u32::MAX, u32::MAX, u32::MAX));
    assert_eq!((u16at(&c, e + 2), u64at(&c, e + 4), u64at(&c, e + 12), u64at(&c, e + 20)), (24, BIG, BIG, BIG + 1));
    // neither: no extra field
    let c = zip_central_header(name, 1, 10, 10).unwrap();
    assert_eq!(u16at(&c, 30), 0);
}

#[test]
fn zip_end_adds_the_zip64_record_only_for_a_big_archive() {
    let small = zip_end(3, 300, 4000);
    assert_eq!((small.len(), u32at(&small, 0), u16at(&small, 10)), (22, 0x0605_4b50, 3));
    let many = zip_end(70_000, 300, 4000);
    // ZIP64 end record (56) + locator (20) + classic end record (22)
    assert_eq!(many.len(), 56 + 20 + 22);
    assert_eq!((u32at(&many, 0), u64at(&many, 24), u64at(&many, 32)), (0x0606_4b50, 70_000, 70_000));
    assert_eq!(u32at(&many, 56), 0x0706_4b50);
    assert_eq!(u64at(&many, 56 + 8), 4000 + 300, "the locator points at the ZIP64 record");
    assert_eq!((u32at(&many, 76), u16at(&many, 76 + 10)), (0x0605_4b50, 0xFFFF));
    let far = zip_end(2, 300, BIG);
    assert_eq!((far.len(), u32at(&far, 56 + 20 + 16)), (98, u32::MAX));
}

#[test]
fn tar_uses_base_256_for_a_file_of_8_gib_or_more() {
    let small = tar_header("a", 1234, 0).unwrap();
    assert_eq!(&small[124..135], b"00000002322");
    let big = tar_header("a", 9 * 1024 * 1024 * 1024, 0).unwrap();
    assert_eq!(big[124], 0x80);
    assert_eq!(u64::from_be_bytes(big[128..136].try_into().unwrap()), 9 * 1024 * 1024 * 1024);
    let sum: u32 = big.iter().enumerate().map(|(i, x)| if (148..156).contains(&i) { 32 } else { u32::from(*x) }).sum();
    assert_eq!(u32::from_str_radix(std::str::from_utf8(&big[148..154]).unwrap(), 8).unwrap(), sum);
}
