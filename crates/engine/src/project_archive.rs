//! Pack a collected project folder into one `.zip` or `.tar` file (Kdenlive plan KDN-45), written
//! from the formats' specifications with `std` only.
//!
//! - **tar**: POSIX ustar headers (a name over 100 bytes is split into `prefix` + `name`); a file of
//!   8 GiB or more uses the GNU base-256 size field.
//! - **zip**: entries are *stored*, not deflated: project media (video, audio, images) is already
//!   compressed, so deflate would cost time for nothing. Names are UTF-8 (flag bit 11). ZIP64 is used
//!   exactly where a size, an offset or the entry count needs it, so a project of any size packs.
//!
//! Only regular files are packed, in a sorted order so the same folder always gives the same
//! archive. Symbolic links are skipped (never followed: an archive never reaches outside the folder).
//! The archive is written to `<out>.part` and renamed when complete, so a stopped or failed pack
//! never leaves a half-written archive under the real name.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// The archive formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
}

impl Format {
    /// `"zip"` or `"tar"` (case-insensitive).
    pub fn parse(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "zip" => Some(Format::Zip),
            "tar" => Some(Format::Tar),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::Tar => "tar",
        }
    }
}

/// One file to pack: its path on disk and its name inside the archive (`/`-separated, relative).
struct Entry {
    disk: PathBuf,
    name: String,
    size: u64,
    mtime: u64,
}

const MAX_ENTRIES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;

fn collect(dir: &Path, rel: &str, depth: usize, out: &mut Vec<Entry>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: folders nested deeper than {MAX_DEPTH} levels", dir.display()));
    }
    let mut names: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(|e| e.ok()).collect();
    names.sort_by_key(|e| e.file_name());
    for e in names {
        let ty = e.file_type().map_err(|e2| format!("{}: {e2}", e.path().display()))?;
        let file_name = e.file_name().to_string_lossy().into_owned();
        let name = if rel.is_empty() { file_name } else { format!("{rel}/{file_name}") };
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            collect(&e.path(), &name, depth + 1, out)?;
        } else if ty.is_file() {
            if out.len() >= MAX_ENTRIES {
                return Err(format!("more than {MAX_ENTRIES} files: not packed"));
            }
            let meta = e.metadata().map_err(|e2| format!("{}: {e2}", e.path().display()))?;
            let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            out.push(Entry { disk: e.path(), name, size: meta.len(), mtime });
        }
    }
    Ok(())
}

/// Pack every regular file under `dir` into `out` (replacing it). Returns the archive's size.
/// `progress` is called with the number of files packed so far; `cancelled` is polled between
/// chunks and ends the pack with `Err("cancelled")`.
pub fn pack(dir: &Path, out: &Path, format: Format, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(usize)) -> Result<u64, String> {
    let mut entries = Vec::new();
    collect(dir, "", 0, &mut entries)?;
    let part = PathBuf::from(format!("{}.part", out.display()));
    let result = (|| {
        let file = File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut w = Counting { inner: BufWriter::new(file), written: 0 };
        match format {
            Format::Tar => write_tar(&mut w, &entries, cancelled, progress)?,
            Format::Zip => write_zip(&mut w, &entries, cancelled, progress)?,
        }
        w.inner.flush().map_err(|e| format!("{}: {e}", part.display()))?;
        Ok(w.written)
    })();
    match result {
        Ok(n) => {
            std::fs::rename(&part, out).map_err(|e| format!("{}: {e}", out.display()))?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

struct Counting<W: Write> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

const CHUNK: usize = 1 << 20;

/// Copy a file's bytes to `w`, checking for a cancel between chunks.
fn copy_file(entry: &Entry, w: &mut dyn Write, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let mut f = File::open(&entry.disk).map_err(|e| format!("{}: {e}", entry.disk.display()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut left = entry.size;
    while left > 0 {
        if cancelled() {
            return Err("cancelled".into());
        }
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let n = f.read(&mut buf[..want]).map_err(|e| format!("{}: {e}", entry.disk.display()))?;
        if n == 0 {
            return Err(format!("{} changed while it was packed (it got shorter)", entry.disk.display()));
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        left -= n as u64;
    }
    Ok(())
}

// ---------------------------------------------------------------- tar

fn octal(field: &mut [u8], v: u64) {
    let digits = field.len() - 1;
    let s = format!("{v:0digits$o}");
    let b = s.as_bytes();
    let b = &b[b.len().saturating_sub(digits)..];
    field[..digits].copy_from_slice(b);
    field[digits] = 0;
}

/// A ustar header for a regular file, or an error when the name cannot be stored.
pub(crate) fn tar_header(name: &str, size: u64, mtime: u64) -> Result<[u8; 512], String> {
    let n = name.as_bytes();
    let (prefix, base): (&[u8], &[u8]) = if n.len() <= 100 {
        (&[], n)
    } else {
        // the split must fall on a `/`, leave a name of at most 100 bytes and a prefix of at most 155
        let cut = n.iter().enumerate().filter(|(i, c)| **c == b'/' && *i <= 155 && n.len() - i - 1 <= 100 && n.len() - i - 1 > 0).map(|(i, _)| i).next();
        match cut {
            Some(i) => (&n[..i], &n[i + 1..]),
            None => return Err(format!("{name}: the path is too long for a tar file")),
        }
    };
    let mut h = [0u8; 512];
    h[..base.len()].copy_from_slice(base);
    octal(&mut h[100..108], 0o644);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    if size < 0o77_777_777_777 {
        octal(&mut h[124..136], size);
    } else {
        // GNU base-256: the high bit of the first byte, then the size big-endian
        h[124] = 0x80;
        h[128..136].copy_from_slice(&size.to_be_bytes());
    }
    octal(&mut h[136..148], mtime);
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix);
    // the checksum counts the checksum field as spaces
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    let s = format!("{sum:06o}");
    h[148..154].copy_from_slice(s.as_bytes());
    h[154] = 0;
    h[155] = b' ';
    Ok(h)
}

fn write_tar(w: &mut dyn Write, entries: &[Entry], cancelled: &dyn Fn() -> bool, progress: &dyn Fn(usize)) -> Result<(), String> {
    for (i, e) in entries.iter().enumerate() {
        w.write_all(&tar_header(&e.name, e.size, e.mtime)?).map_err(|e| e.to_string())?;
        copy_file(e, w, cancelled)?;
        let pad = (512 - (e.size % 512) as usize) % 512;
        w.write_all(&[0u8; 512][..pad]).map_err(|e| e.to_string())?;
        progress(i + 1);
    }
    w.write_all(&[0u8; 1024]).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- zip

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

/// CRC-32 (IEEE) of `data`, continuing from `crc` (start with 0).
pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = !crc;
    for b in data {
        c = t[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

fn file_crc(entry: &Entry, cancelled: &dyn Fn() -> bool) -> Result<u32, String> {
    let mut f = File::open(&entry.disk).map_err(|e| format!("{}: {e}", entry.disk.display()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut crc = 0u32;
    let mut left = entry.size;
    while left > 0 {
        if cancelled() {
            return Err("cancelled".into());
        }
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let n = f.read(&mut buf[..want]).map_err(|e| format!("{}: {e}", entry.disk.display()))?;
        if n == 0 {
            return Err(format!("{} changed while it was packed (it got shorter)", entry.disk.display()));
        }
        crc = crc32(crc, &buf[..n]);
        left -= n as u64;
    }
    Ok(crc)
}

const U32_MAX: u64 = 0xFFFF_FFFF;

fn le16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn le32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn le64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// MS-DOS time and date of 1980-01-01 00:00 (the entries carry no clock: the archive is reproducible).
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

/// A local file header for a stored entry; a size of 4 GiB or more goes in a ZIP64 extra field.
pub(crate) fn zip_local_header(name: &str, crc: u32, size: u64) -> Result<Vec<u8>, String> {
    let nb = name.as_bytes();
    let name_len = u16::try_from(nb.len()).map_err(|_| format!("{name}: the path is too long for a zip file"))?;
    let big = size >= U32_MAX;
    let mut h = Vec::with_capacity(30 + nb.len() + 20);
    le32(&mut h, 0x0403_4b50);
    le16(&mut h, if big { 45 } else { 20 });
    le16(&mut h, 0x0800);
    le16(&mut h, 0);
    le16(&mut h, DOS_TIME);
    le16(&mut h, DOS_DATE);
    le32(&mut h, crc);
    let s32 = if big { U32_MAX as u32 } else { size as u32 };
    le32(&mut h, s32);
    le32(&mut h, s32);
    le16(&mut h, name_len);
    le16(&mut h, if big { 20 } else { 0 });
    h.extend_from_slice(nb);
    if big {
        le16(&mut h, 0x0001);
        le16(&mut h, 16);
        le64(&mut h, size);
        le64(&mut h, size);
    }
    Ok(h)
}

/// A central directory header; sizes and the local header offset that need 64 bits go in the ZIP64
/// extra field, in the order the specification gives (uncompressed size, compressed size, offset).
pub(crate) fn zip_central_header(name: &str, crc: u32, size: u64, offset: u64) -> Result<Vec<u8>, String> {
    let nb = name.as_bytes();
    let name_len = u16::try_from(nb.len()).map_err(|_| format!("{name}: the path is too long for a zip file"))?;
    let big_size = size >= U32_MAX;
    let big_off = offset >= U32_MAX;
    let mut extra = Vec::new();
    if big_size || big_off {
        let mut body = Vec::new();
        if big_size {
            le64(&mut body, size);
            le64(&mut body, size);
        }
        if big_off {
            le64(&mut body, offset);
        }
        le16(&mut extra, 0x0001);
        le16(&mut extra, body.len() as u16);
        extra.extend_from_slice(&body);
    }
    let mut h = Vec::with_capacity(46 + nb.len() + extra.len());
    le32(&mut h, 0x0201_4b50);
    le16(&mut h, 0x031E); // made by: Unix, spec 3.0
    le16(&mut h, if extra.is_empty() { 20 } else { 45 });
    le16(&mut h, 0x0800);
    le16(&mut h, 0);
    le16(&mut h, DOS_TIME);
    le16(&mut h, DOS_DATE);
    le32(&mut h, crc);
    let s32 = if big_size { U32_MAX as u32 } else { size as u32 };
    le32(&mut h, s32);
    le32(&mut h, s32);
    le16(&mut h, name_len);
    le16(&mut h, extra.len() as u16);
    le16(&mut h, 0); // comment
    le16(&mut h, 0); // disk number
    le16(&mut h, 0); // internal attributes
    le32(&mut h, 0o100_644 << 16); // external attributes: a regular file, 0644
    le32(&mut h, if big_off { U32_MAX as u32 } else { offset as u32 });
    h.extend_from_slice(nb);
    h.extend_from_slice(&extra);
    Ok(h)
}

/// The end of the central directory, preceded by the ZIP64 record and locator when a count, size or
/// offset does not fit the classic fields.
pub(crate) fn zip_end(count: usize, cd_size: u64, cd_offset: u64) -> Vec<u8> {
    let mut v = Vec::new();
    let need64 = count >= 0xFFFF || cd_size >= U32_MAX || cd_offset >= U32_MAX;
    if need64 {
        let rec_offset = cd_offset + cd_size;
        le32(&mut v, 0x0606_4b50);
        le64(&mut v, 44);
        le16(&mut v, 0x031E);
        le16(&mut v, 45);
        le32(&mut v, 0);
        le32(&mut v, 0);
        le64(&mut v, count as u64);
        le64(&mut v, count as u64);
        le64(&mut v, cd_size);
        le64(&mut v, cd_offset);
        le32(&mut v, 0x0706_4b50);
        le32(&mut v, 0);
        le64(&mut v, rec_offset);
        le32(&mut v, 1);
    }
    le32(&mut v, 0x0605_4b50);
    le16(&mut v, 0);
    le16(&mut v, 0);
    let c16 = if count >= 0xFFFF { 0xFFFF } else { count as u16 };
    le16(&mut v, c16);
    le16(&mut v, c16);
    le32(&mut v, if cd_size >= U32_MAX { U32_MAX as u32 } else { cd_size as u32 });
    le32(&mut v, if cd_offset >= U32_MAX { U32_MAX as u32 } else { cd_offset as u32 });
    le16(&mut v, 0);
    v
}

fn write_zip(w: &mut Counting<impl Write>, entries: &[Entry], cancelled: &dyn Fn() -> bool, progress: &dyn Fn(usize)) -> Result<(), String> {
    let mut central: Vec<u8> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let crc = file_crc(e, cancelled)?;
        let offset = w.written;
        w.write_all(&zip_local_header(&e.name, crc, e.size)?).map_err(|e| e.to_string())?;
        copy_file(e, w, cancelled)?;
        central.extend_from_slice(&zip_central_header(&e.name, crc, e.size, offset)?);
        progress(i + 1);
    }
    let cd_offset = w.written;
    w.write_all(&central).map_err(|e| e.to_string())?;
    w.write_all(&zip_end(entries.len(), central.len() as u64, cd_offset)).map_err(|e| e.to_string())
}
