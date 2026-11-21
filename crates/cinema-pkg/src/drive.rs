//! Read-only check of a DCP delivery drive (CINECERT WP13).
//!
//! ISDCF's drive recommendation, as the plan summarises it: an MBR partition table, one Linux
//! partition, ext2 or ext3 with 128-byte inodes (older cinema servers cannot mount ext4 or larger
//! inodes). [`inspect`] reads the partition table and the ext superblock from a disk image or a
//! block device opened read-only, and says what does not match. It never writes: formatting a
//! drive is left to the operator, with the command the report suggests.

use std::io::{Read, Seek, SeekFrom};

use crate::{Error, Result};

const SECTOR: u64 = 512;
const EXT_MAGIC: u16 = 0xEF53;
/// ext4-only incompatible features: extents, 64-bit, flex_bg.
const EXT4_INCOMPAT: u32 = 0x40 | 0x80 | 0x200;
const HAS_JOURNAL: u32 = 0x4;

/// Partition table found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    /// A classic MBR.
    Mbr,
    /// A GUID partition table behind a protective MBR.
    Gpt,
    /// No partition table: a file system on the whole device, or nothing recognisable.
    None,
}

/// One MBR partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    /// Partition type byte (0x83 Linux).
    pub kind: u8,
    /// First sector.
    pub start: u64,
    /// Length in sectors.
    pub sectors: u64,
}

/// File system of the first partition (or of the whole device without a table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSystem {
    /// ext2: no journal.
    Ext2,
    /// ext3: journal, no ext4-only features.
    Ext3,
    /// ext4 features present.
    Ext4,
    /// No ext superblock.
    Other,
}

/// What [`inspect`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveReport {
    /// Partition table.
    pub table: Table,
    /// Used MBR partitions.
    pub partitions: Vec<Partition>,
    /// File system checked.
    pub file_system: FileSystem,
    /// Inode size in bytes, for an ext file system.
    pub inode_size: Option<u16>,
    /// Block size in bytes, for an ext file system.
    pub block_size: Option<u32>,
    /// Deviations from the recommendation, `DRIVE.<WHAT>: …`. Empty means the drive matches.
    pub findings: Vec<String>,
}

fn read_at<R: Read + Seek>(r: &mut R, at: u64, buf: &mut [u8]) -> Result<bool> {
    r.seek(SeekFrom::Start(at))?;
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => return Ok(false),
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Inspect a drive or disk image.
pub fn inspect<R: Read + Seek>(mut r: R) -> Result<DriveReport> {
    let mut mbr = [0u8; 512];
    if !read_at(&mut r, 0, &mut mbr)? {
        return Err(Error::Delivery("drive: shorter than one sector".into()));
    }
    let mut partitions = Vec::new();
    let table = if mbr[510] == 0x55 && mbr[511] == 0xAA {
        for i in 0..4 {
            let e = &mbr[446 + 16 * i..446 + 16 * (i + 1)];
            let p = Partition { kind: e[4], start: u64::from(u32le(e, 8)), sectors: u64::from(u32le(e, 12)) };
            if p.kind != 0 && p.sectors != 0 {
                partitions.push(p);
            }
        }
        if partitions.iter().any(|p| p.kind == 0xEE) { Table::Gpt } else { Table::Mbr }
    } else {
        Table::None
    };
    let fs_start = match table {
        Table::Mbr => partitions.first().map(|p| p.start.saturating_mul(SECTOR)),
        Table::None => Some(0),
        Table::Gpt => None,
    };
    let mut findings = Vec::new();
    let (mut file_system, mut inode_size, mut block_size) = (FileSystem::Other, None, None);
    if let Some(start) = fs_start {
        let mut sb = [0u8; 1024];
        if read_at(&mut r, start.saturating_add(1024), &mut sb)? && u16le(&sb, 56) == EXT_MAGIC {
            let rev = u32le(&sb, 76);
            let compat = u32le(&sb, 92);
            let incompat = u32le(&sb, 96);
            inode_size = Some(if rev == 0 { 128 } else { u16le(&sb, 88) });
            block_size = 1024u32.checked_shl(u32le(&sb, 24));
            file_system = if incompat & EXT4_INCOMPAT != 0 {
                FileSystem::Ext4
            } else if compat & HAS_JOURNAL != 0 {
                FileSystem::Ext3
            } else {
                FileSystem::Ext2
            };
        }
    }
    match table {
        Table::Mbr => {}
        Table::Gpt => findings.push("DRIVE.TABLE: GUID partition table; the recommendation is an MBR".to_string()),
        Table::None => findings.push("DRIVE.TABLE: no MBR partition table".to_string()),
    }
    if table == Table::Mbr {
        if partitions.len() != 1 {
            findings.push(format!("DRIVE.PARTITION: {} partitions; the recommendation is one", partitions.len()));
        }
        if let Some(p) = partitions.first().filter(|p| p.kind != 0x83) {
            findings.push(format!("DRIVE.PARTITION: first partition type {:#04x}; Linux is 0x83", p.kind));
        }
    }
    match file_system {
        FileSystem::Ext2 | FileSystem::Ext3 => {}
        FileSystem::Ext4 => findings.push("DRIVE.FS: ext4 features (extents, 64bit or flex_bg); use ext2 or ext3".to_string()),
        FileSystem::Other => findings.push("DRIVE.FS: no ext2 / ext3 file system".to_string()),
    }
    if let Some(n) = inode_size.filter(|&n| n != 128) {
        findings.push(format!("DRIVE.INODE: inode size {n}; the recommendation is 128"));
    }
    Ok(DriveReport { table, partitions, file_system, inode_size, block_size, findings })
}

/// The command that formats a partition as the recommendation asks. Shown, never run.
pub fn format_hint(partition: &str) -> String {
    format!("mkfs.ext3 -I 128 {partition}")
}
