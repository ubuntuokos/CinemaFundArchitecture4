//! SHA-256 of files and bytes, streamed and cancellable.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::Error;

/// Bytes read per step (the cancel flag is checked between steps).
const CHUNK: usize = 1 << 20;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Lowercase hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// A file's SHA-256 (lowercase hex) and size, read in 1 MiB steps. Stops with
/// [`Error::Cancelled`] as soon as `cancel` is set.
pub fn sha256_file(path: &Path, cancel: &AtomicBool) -> Result<(String, u64), Error> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let mut f = File::open(path).map_err(io)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io(e)),
        };
        h.update(buf.get(..n).unwrap_or_default());
        total = total.saturating_add(n as u64);
    }
    Ok((hex(&h.finalize()), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn file_matches_bytes_and_cancels() {
        let dir = std::env::temp_dir().join(format!("cfa4-qc-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // a Unicode name with spaces (plan §6: such paths must work)
        let path = dir.join("árvíztűrő tükör.bin");
        let data: Vec<u8> = (0..(CHUNK + 4321)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let (h, n) = sha256_file(&path, &AtomicBool::new(false)).unwrap();
        assert_eq!((h, n), (sha256_hex(&data), data.len() as u64));
        assert!(matches!(sha256_file(&path, &AtomicBool::new(true)), Err(Error::Cancelled)));
        assert!(matches!(sha256_file(&dir.join("missing"), &AtomicBool::new(false)), Err(Error::Io(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
