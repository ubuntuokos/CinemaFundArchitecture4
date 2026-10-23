//! Linux/AMD64 HEIF/HEIC still-image bridge, using the locally installed libheif tools.
//!
//! The Rust media pipeline remains independent of LGPL/GPL bindings. The heif-dec
//! and heif-enc executables are isolated subprocesses; no codec library is linked
//! or bundled. A private temporary directory is removed even after an error.
//! This first integration imports the first decoded picture as 8-bit sRGB RGBA;
//! full multi-image, 10-bit/HDR metadata and animation require separate work.

use std::fs::{self, DirBuilder};
use std::io::{Cursor, ErrorKind};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::Raster;
use crate::{MediaError, Result};

const MAX_SOURCE_BYTES: usize = 256 * 1024 * 1024;
const MAX_RESULT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RGBA_BYTES: u64 = 512 * 1024 * 1024;
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// HEIF brands in the ISO-BMFF file-type box (including generic HEIF brands).
/// Do not route ordinary MP4/MOV media to the HEIF image decoder.
pub fn sniff(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || bytes.get(4..8) != Some(b"ftyp".as_slice()) {
        return false;
    }
    let Some(size) = bytes.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()) else {
        return false;
    };
    let size = u32::from_be_bytes(size) as usize;
    if size < 16 || size > bytes.len() {
        return false;
    }
    let heif_brand = |brand: &[u8]| [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1"].iter().any(|b| brand == b.as_slice());
    bytes.get(8..12).is_some_and(heif_brand) || bytes.get(16..size).is_some_and(|tail| tail.chunks_exact(4).any(heif_brand))
}

struct Workdir {
    path: PathBuf,
}

impl Workdir {
    fn create() -> Result<Self> {
        let base = std::env::temp_dir();
        let clock = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        for _ in 0..16 {
            let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("cfa4-heif-{}-{clock}-{n}", std::process::id()));
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(MediaError::Io(format!("HEIF private working directory: {e}"))),
            }
        }
        Err(MediaError::Io("cannot allocate a unique HEIF working directory".into()))
    }
    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn tool_output(program: &str, result: std::io::Result<Output>) -> Result<()> {
    let output = result.map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            MediaError::Unsupported(format!("{program} is missing; install libheif-examples and the libheif HEVC codec plugins on Ubuntu 26.04"))
        } else {
            MediaError::Io(format!("{program}: {e}"))
        }
    })?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let short = detail.chars().take(1024).collect::<String>();
    Err(MediaError::Decode(format!("{program} failed ({}): {short}", output.status)))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let size = fs::metadata(path).map_err(|e| MediaError::Io(format!("HEIF output: {e}")))?.len();
    if size == 0 || size > MAX_RESULT_BYTES {
        return Err(MediaError::Unsupported("HEIF output is empty or exceeds the 256 MiB safety limit".into()));
    }
    fs::read(path).map_err(|e| MediaError::Io(format!("HEIF output: {e}")))
}

fn first_decoded_png(dir: &Workdir) -> Result<PathBuf> {
    let mut files = Vec::new();
    for e in fs::read_dir(&dir.path).map_err(|e| MediaError::Io(format!("HEIF image outputs: {e}")))? {
        let e = e.map_err(|e| MediaError::Io(format!("HEIF image outputs: {e}")))?;
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) == Some("png") && path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("decoded")) {
            files.push(path);
        }
    }
    files.sort_unstable();
    files.into_iter().next().ok_or_else(|| MediaError::Decode("heif-dec returned no PNG picture".into()))
}

/// Decode the first HEIF/HEIC picture, including the standard libheif image transforms.
/// Multi-picture content and high-bit-depth metadata are not yet retained.
pub fn decode(name: &str, bytes: &[u8]) -> Result<Raster> {
    if !sniff(bytes) {
        return Err(MediaError::Unsupported(format!("{name}: not a HEIF/HEIC image")));
    }
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(MediaError::Unsupported("HEIF input exceeds the 256 MiB safety limit".into()));
    }
    let dir = Workdir::create()?;
    let input = dir.file("input.heic");
    let output = dir.file("decoded.png");
    fs::write(&input, bytes).map_err(|e| MediaError::Io(format!("HEIF input: {e}")))?;
    tool_output("heif-dec", Command::new("heif-dec").arg(&input).arg(&output).output())?;
    let png = read_bounded(&first_decoded_png(&dir)?)?;
    let mut reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
    reader.limits(image::Limits::default());
    let decoded = reader.decode().map_err(|e| MediaError::Decode(format!("{name}: decoded HEIF PNG: {e}")))?;
    let (width, height) = (decoded.width(), decoded.height());
    let size = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| MediaError::Unsupported("HEIF dimensions overflow".into()))?;
    if size == 0 || size > MAX_RGBA_BYTES {
        return Err(MediaError::Unsupported("HEIF decoded picture exceeds the 512 MiB safety limit".into()));
    }
    let has_alpha = decoded.color().has_alpha();
    Ok(Raster { width, height, rgba: decoded.to_rgba8().into_raw(), has_alpha })
}

/// Encode a PNG into a local HEIC/HEIF (HEVC) file and return the container bytes.
/// Existing Export Frame settings provide the source PNG and precision; libheif
/// handles the HEVC image encoding without copying any LGPL/GPL source code.
pub fn encode_png(png: &[u8]) -> Result<Vec<u8>> {
    if png.len() > MAX_SOURCE_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(MediaError::Unsupported("HEIC export requires a valid PNG within 256 MiB".into()));
    }
    let dir = Workdir::create()?;
    let input = dir.file("frame.png");
    let output = dir.file("frame.heic");
    fs::write(&input, png).map_err(|e| MediaError::Io(format!("HEIC source image: {e}")))?;
    tool_output("heif-enc", Command::new("heif-enc").arg("-q").arg("90").arg("-o").arg(&output).arg(&input).output())?;
    let heif = read_bounded(&output)?;
    if !sniff(&heif) {
        return Err(MediaError::Decode("heif-enc produced an unrecognized HEIF container".into()));
    }
    Ok(heif)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_heif_brands_without_claiming_mp4() {
        for brand in [b"heic", b"heix", b"mif1", b"msf1"] {
            let mut header = [0u8; 20];
            header[..4].copy_from_slice(&20u32.to_be_bytes());
            header[4..8].copy_from_slice(b"ftyp");
            header[8..12].copy_from_slice(brand);
            assert!(sniff(&header), "{brand:?}");
        }
        let mut video = [0u8; 20];
        video[..4].copy_from_slice(&20u32.to_be_bytes());
        video[4..8].copy_from_slice(b"ftyp");
        video[8..12].copy_from_slice(b"isom");
        assert!(!sniff(&video));
        assert!(!sniff(&video[..12]));
        assert!(!sniff(b"not an image"));
    }

    #[test]
    fn malformed_or_oversized_inputs_return_errors() {
        assert!(decode("a.heic", b"not a HEIF file").is_err());
        assert!(encode_png(b"not PNG").is_err());
    }
}
