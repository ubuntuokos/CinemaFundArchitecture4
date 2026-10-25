//! A minimal read-only ZIP reader (APPNOTE.TXT: end of central directory, central directory,
//! local file headers; stored and DEFLATE entries) for office spreadsheet packages.
//!
//! Every input is hostile: offsets and sizes are bounds-checked, entry counts and uncompressed
//! sizes are capped, encrypted and ZIP64 archives are refused. Decompression uses CFA4's own
//! RFC 1951 inflater (`cfa4_media::inflate`).

use crate::CreditsError;

const EOCD: u32 = 0x0605_4b50;
const CEN: u32 = 0x0201_4b50;
const LOC: u32 = 0x0403_4b50;
/// Most entries an archive may list.
const MAX_ENTRIES: usize = 10_000;
/// Largest single uncompressed entry.
pub const MAX_ENTRY: usize = 64 << 20;

/// One entry of the central directory.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    method: u16,
    flags: u16,
    compressed: usize,
    size: usize,
    local: usize,
}

/// An opened archive.
pub struct Zip<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
}

fn bad(m: &str) -> CreditsError {
    CreditsError::Format(format!("damaged spreadsheet file (ZIP): {m}"))
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    let b = d.get(i..i.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    let b = d.get(i..i.checked_add(4)?)?;
    Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
}

/// Does `data` look like a ZIP archive?
pub fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04")
}

impl<'a> Zip<'a> {
    pub fn open(data: &'a [u8]) -> Result<Zip<'a>, CreditsError> {
        // the end record is in the last 22 + 65535 bytes
        let lo = data.len().saturating_sub(22 + 0xFFFF);
        let eocd = (lo..data.len().saturating_sub(21)).rev().find(|&i| u32_at(data, i) == Some(EOCD)).ok_or_else(|| bad("no end of central directory"))?;
        let count = usize::from(u16_at(data, eocd + 10).ok_or_else(|| bad("truncated end record"))?);
        let cd_size = u32_at(data, eocd + 12).ok_or_else(|| bad("truncated end record"))? as usize;
        let cd_off = u32_at(data, eocd + 16).ok_or_else(|| bad("truncated end record"))? as usize;
        if count == 0xFFFF || cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF {
            return Err(bad("ZIP64 archives are not supported"));
        }
        if count > MAX_ENTRIES {
            return Err(CreditsError::TooLarge(format!("the archive lists more than {MAX_ENTRIES} files")));
        }
        let mut entries = Vec::with_capacity(count);
        let mut p = cd_off;
        for _ in 0..count {
            if u32_at(data, p) != Some(CEN) {
                return Err(bad("bad central directory"));
            }
            let field = |o: usize| u16_at(data, p + o).ok_or_else(|| bad("truncated central directory"));
            let field32 = |o: usize| u32_at(data, p + o).ok_or_else(|| bad("truncated central directory"));
            let flags = field(8)?;
            let method = field(10)?;
            let compressed = field32(20)? as usize;
            let size = field32(24)? as usize;
            let (nlen, xlen, clen) = (usize::from(field(28)?), usize::from(field(30)?), usize::from(field(32)?));
            let local = field32(42)? as usize;
            let name_at = p + 46;
            let name = data.get(name_at..name_at + nlen).ok_or_else(|| bad("truncated file name"))?;
            entries.push(Entry { name: String::from_utf8_lossy(name).into_owned(), method, flags, compressed, size, local });
            p = name_at.saturating_add(nlen).saturating_add(xlen).saturating_add(clen);
        }
        Ok(Zip { data, entries })
    }

    /// The entry called `name` (exact, `/`-separated; a leading `/` is ignored).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        let n = name.trim_start_matches('/');
        self.entries.iter().find(|e| e.name == n)
    }

    /// Read an entry's uncompressed bytes.
    pub fn read(&self, e: &Entry) -> Result<Vec<u8>, CreditsError> {
        if e.flags & 1 != 0 {
            return Err(CreditsError::Format("the spreadsheet is password-protected".into()));
        }
        if e.size > MAX_ENTRY {
            return Err(CreditsError::TooLarge(format!("`{}` unpacks to more than {} MiB", e.name, MAX_ENTRY >> 20)));
        }
        if u32_at(self.data, e.local) != Some(LOC) {
            return Err(bad("bad local header"));
        }
        let nlen = usize::from(u16_at(self.data, e.local + 26).ok_or_else(|| bad("truncated local header"))?);
        let xlen = usize::from(u16_at(self.data, e.local + 28).ok_or_else(|| bad("truncated local header"))?);
        let start = e.local.saturating_add(30).saturating_add(nlen).saturating_add(xlen);
        let raw = self.data.get(start..start.saturating_add(e.compressed)).ok_or_else(|| bad("entry runs past the end of the file"))?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                let out = cfa4_media::inflate::inflate(raw, e.size).map_err(bad)?;
                if out.len() != e.size {
                    return Err(bad("entry size does not match"));
                }
                Ok(out)
            }
            m => Err(bad(&format!("compression method {m} is not supported"))),
        }
    }

    /// Read the entry called `name`.
    pub fn read_named(&self, name: &str) -> Result<Vec<u8>, CreditsError> {
        let e = self.find(name).ok_or_else(|| bad(&format!("`{name}` is missing")))?;
        self.read(e)
    }
}
