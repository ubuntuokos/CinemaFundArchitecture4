//! DCP drive check on synthetic disk images, and on real ext2 / ext3 / ext4 images when
//! `mkfs.ext2` and friends are installed (skipped otherwise).

use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

use cfa4_cinema_pkg::drive::{self, FileSystem, Table};

/// A disk image: optional MBR with one partition of `kind` at sector 2048, and an ext superblock.
fn image(mbr: Option<u8>, journal: bool, incompat: u32, inode: u16) -> Vec<u8> {
    let start = if mbr.is_some() { 2048 * 512 } else { 0 };
    let mut d = vec![0u8; start + 4096];
    if let Some(kind) = mbr {
        d[446 + 4] = kind;
        d[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
        d[446 + 12..446 + 16].copy_from_slice(&100_000u32.to_le_bytes());
        d[510] = 0x55;
        d[511] = 0xAA;
    }
    let sb = start + 1024;
    d[sb + 24..sb + 28].copy_from_slice(&2u32.to_le_bytes()); // 4096-byte blocks
    d[sb + 56..sb + 58].copy_from_slice(&0xEF53u16.to_le_bytes());
    d[sb + 76..sb + 80].copy_from_slice(&1u32.to_le_bytes());
    d[sb + 88..sb + 90].copy_from_slice(&inode.to_le_bytes());
    d[sb + 92..sb + 96].copy_from_slice(&(if journal { 4u32 } else { 0 }).to_le_bytes());
    d[sb + 96..sb + 100].copy_from_slice(&incompat.to_le_bytes());
    d
}

#[test]
fn recommended_drive_has_no_findings() {
    let r = drive::inspect(Cursor::new(image(Some(0x83), true, 0, 128))).unwrap();
    assert_eq!((r.table, r.file_system, r.inode_size, r.block_size), (Table::Mbr, FileSystem::Ext3, Some(128), Some(4096)));
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    let r = drive::inspect(Cursor::new(image(Some(0x83), false, 0, 128))).unwrap();
    assert_eq!(r.file_system, FileSystem::Ext2);
    assert!(r.findings.is_empty());
}

#[test]
fn deviations_are_reported() {
    let has = |img: Vec<u8>, what: &str| drive::inspect(Cursor::new(img)).unwrap().findings.iter().any(|f| f.starts_with(what));
    assert!(has(image(Some(0x83), true, 0x40, 256), "DRIVE.FS"));
    assert!(has(image(Some(0x83), true, 0, 256), "DRIVE.INODE"));
    assert!(has(image(Some(0x07), true, 0, 128), "DRIVE.PARTITION"));
    assert!(has(image(Some(0xEE), true, 0, 128), "DRIVE.TABLE"));
    assert!(has(image(None, true, 0, 128), "DRIVE.TABLE"));
    assert!(has(vec![0u8; 8192], "DRIVE.FS"));
    assert!(drive::inspect(Cursor::new(vec![0u8; 10])).is_err());
    assert_eq!(drive::format_hint("/dev/sdx1"), "mkfs.ext3 -I 128 /dev/sdx1");
}

#[test]
fn hostile_images_never_panic() {
    let base = image(Some(0x83), true, 0, 128);
    let mut seed = 0x2545_F491u32;
    for i in 0..300usize {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let mut d = base.clone();
        let at = seed as usize % d.len();
        d[at] ^= 1 << (i % 8);
        // Partition start pointing far beyond the image, and huge block-size exponents.
        if i % 3 == 0 {
            d[446 + 8..446 + 12].copy_from_slice(&seed.to_le_bytes());
        }
        d.truncate((seed as usize / 5) % (d.len() + 1));
        assert!(std::panic::catch_unwind(|| drive::inspect(Cursor::new(d))).is_ok());
    }
}

#[test]
fn real_file_systems() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("drive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (tool, args, want, inode) in [
        ("mkfs.ext3", &["-q", "-F", "-I", "128"][..], FileSystem::Ext3, 128),
        ("mkfs.ext2", &["-q", "-F", "-I", "128"][..], FileSystem::Ext2, 128),
        ("mkfs.ext4", &["-q", "-F"][..], FileSystem::Ext4, 256),
    ] {
        let img = dir.join(format!("{tool}.img"));
        std::fs::write(&img, vec![0u8; 8 << 20]).unwrap();
        let Ok(st) = Command::new(tool).args(args).arg(&img).status() else {
            eprintln!("skipped: {tool} is not installed");
            continue;
        };
        assert!(st.success(), "{tool}");
        let r = drive::inspect(std::fs::File::open(&img).unwrap()).unwrap();
        assert_eq!((r.file_system, r.inode_size), (want, Some(inode)), "{tool}");
        assert_eq!(r.table, Table::None);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
