//! Media identity for the `mi` index: a content hash that does not depend on the file's name or
//! path (FINAL 2.0 §6; hézagrendezés L2/R1).
//!
//! The project file keeps only a quick 64-bit fingerprint (`MediaIdentity` in `cfa4-project`,
//! unchanged). The index needs a stronger key, and reading every byte of a multi-hour camera file
//! on import would stall the machine, so the default is the **sampled SHA-256** `sha256s1`:
//!
//! * a file of at most [`FULL_LIMIT`] bytes is hashed whole;
//! * a larger file is hashed from [`SAMPLES`] blocks of [`BLOCK`] bytes: the first, the last and
//!   [`SAMPLES`] `- 2` blocks spread evenly in between (`(size - BLOCK) * i / (SAMPLES - 1)`);
//! * the digest input is the domain tag, the size, then for every block its offset, its length and
//!   its bytes, all integers little-endian `u64`.
//!
//! A sampled hash does not prove two files are byte-identical: an edit that touches only the
//! unsampled gaps is not seen. That is why the full `sha256` ([`HashAlg::Sha256`]) is a separate
//! algorithm tag: a later, optional job can add it to the same asset. The sampling rule is frozen
//! under the name `s1`; a different rule gets a new tag, never a silent change.

use std::fmt;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::MiError;

/// Size of one sampled block.
pub const BLOCK: u64 = 1 << 20;
/// Number of blocks of a sampled hash of a large file.
pub const SAMPLES: u64 = 16;
/// Files up to this size are hashed whole by `sha256s1`.
pub const FULL_LIMIT: u64 = SAMPLES * BLOCK;

const DOMAIN: &[u8] = b"cfa4-mi/sha256s1\0";
const CHUNK: usize = 64 * 1024;

/// Which algorithm produced a [`ContentHash`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HashAlg {
    /// The sampled SHA-256 described in the module docs (rule `s1`).
    #[serde(rename = "sha256s1")]
    Sha256Sampled1,
    /// SHA-256 of the whole file.
    #[serde(rename = "sha256")]
    Sha256,
}

impl HashAlg {
    /// The tag stored in `mi.asset.hash_alg` and in the text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha256Sampled1 => "sha256s1",
            Self::Sha256 => "sha256",
        }
    }
}

/// A content hash: the algorithm and a 32-byte digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash {
    pub alg: HashAlg,
    pub digest: [u8; 32],
}

impl fmt::Display for ContentHash {
    /// `sha256s1:<64 lowercase hex digits>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.alg.as_str())?;
        for b in self.digest {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContentHash {
    type Err = MiError;

    fn from_str(s: &str) -> Result<Self, MiError> {
        let (alg, hex) = s.split_once(':').ok_or(MiError::InvalidHash("missing ':'"))?;
        let alg = match alg {
            "sha256s1" => HashAlg::Sha256Sampled1,
            "sha256" => HashAlg::Sha256,
            _ => return Err(MiError::InvalidHash("unknown algorithm")),
        };
        if hex.len() != 64 {
            return Err(MiError::InvalidHash("digest is not 64 hex digits"));
        }
        let mut digest = [0u8; 32];
        for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
            let hi = nibble(pair.first().copied())?;
            let lo = nibble(pair.get(1).copied())?;
            if let Some(slot) = digest.get_mut(i) {
                *slot = hi << 4 | lo;
            }
        }
        Ok(Self { alg, digest })
    }
}

fn nibble(c: Option<u8>) -> Result<u8, MiError> {
    match c {
        Some(c @ b'0'..=b'9') => Ok(c - b'0'),
        Some(c @ b'a'..=b'f') => Ok(c - b'a' + 10),
        _ => Err(MiError::InvalidHash("digest has a non-lowercase-hex character")),
    }
}

impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The `(offset, length)` blocks `sha256s1` reads for a file of `size` bytes, in order.
pub fn sample_plan(size: u64) -> Vec<(u64, u64)> {
    if size <= FULL_LIMIT {
        return vec![(0, size)];
    }
    let span = u128::from(size - BLOCK);
    (0..SAMPLES)
        .map(|i| {
            let off = span * u128::from(i) / u128::from(SAMPLES - 1);
            (u64::try_from(off).unwrap_or(0), BLOCK)
        })
        .collect()
}

/// The sampled SHA-256 (`sha256s1`) of `reader`, whose length the caller declares as `size`
/// (the file size it read from the file system). A reader of another length, or one that ends
/// early, is an error: the index must not key an asset by a hash of a file that changed under it.
pub fn sampled_sha256<R: Read + Seek>(reader: &mut R, size: u64) -> Result<ContentHash, MiError> {
    let actual = reader.seek(SeekFrom::End(0))?;
    if actual != size {
        return Err(MiError::SizeMismatch("the reader's length differs from the declared size"));
    }
    let mut h = Sha256::new();
    h.update(DOMAIN);
    h.update(size.to_le_bytes());
    for (offset, len) in sample_plan(size) {
        h.update(offset.to_le_bytes());
        h.update(len.to_le_bytes());
        feed(reader, offset, len, &mut h)?;
    }
    Ok(ContentHash { alg: HashAlg::Sha256Sampled1, digest: h.finalize().into() })
}

/// The SHA-256 of every byte of `reader` (declared length `size`, checked as above). Optional
/// and expensive: run it as a low-priority job, never on import.
pub fn full_sha256<R: Read + Seek>(reader: &mut R, size: u64) -> Result<ContentHash, MiError> {
    let actual = reader.seek(SeekFrom::End(0))?;
    if actual != size {
        return Err(MiError::SizeMismatch("the reader's length differs from the declared size"));
    }
    let mut h = Sha256::new();
    feed(reader, 0, size, &mut h)?;
    Ok(ContentHash { alg: HashAlg::Sha256, digest: h.finalize().into() })
}

fn feed<R: Read + Seek>(reader: &mut R, offset: u64, len: u64, h: &mut Sha256) -> Result<(), MiError> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; CHUNK];
    let mut left = len;
    while left > 0 {
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let Some(slot) = buf.get_mut(..want) else {
            return Err(MiError::SizeMismatch("internal buffer"));
        };
        match reader.read_exact(slot) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                return Err(MiError::SizeMismatch("the media ended before its declared size"));
            }
            Err(e) => return Err(e.into()),
        }
        h.update(&*slot);
        left -= want as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn data(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    #[test]
    fn text_form_round_trips() {
        let h = sampled_sha256(&mut Cursor::new(data(1000, 1)), 1000).unwrap();
        let text = h.to_string();
        assert!(text.starts_with("sha256s1:") && text.len() == 9 + 64);
        assert_eq!(text.parse::<ContentHash>().unwrap(), h);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<ContentHash>(&json).unwrap(), h);
    }

    #[test]
    fn hostile_text_is_an_error() {
        for bad in ["", "sha256s1", "md5:00", "sha256:xyz", "sha256s1:ABCDEF", &format!("sha256:{}", "0".repeat(63)), &format!("sha256:{}", "g".repeat(64))] {
            assert!(bad.parse::<ContentHash>().is_err(), "{bad}");
        }
    }

    #[test]
    fn small_file_is_hashed_whole_and_a_change_anywhere_shows() {
        let a = data(5000, 1);
        let mut b = a.clone();
        b[2500] ^= 1;
        let ha = sampled_sha256(&mut Cursor::new(a), 5000).unwrap();
        let hb = sampled_sha256(&mut Cursor::new(b), 5000).unwrap();
        assert_ne!(ha, hb);
    }

    #[test]
    fn same_bytes_same_hash_different_algorithms_differ() {
        let a = data(5000, 7);
        let s1 = sampled_sha256(&mut Cursor::new(a.clone()), 5000).unwrap();
        let s2 = sampled_sha256(&mut Cursor::new(a.clone()), 5000).unwrap();
        let full = full_sha256(&mut Cursor::new(a), 5000).unwrap();
        assert_eq!(s1, s2);
        assert_ne!(s1.digest, full.digest);
        assert_ne!(s1.alg, full.alg);
    }

    /// The `s1` rule is frozen: this digest was computed by an independent implementation of the
    /// rule in the module docs (Python `hashlib`), so a change to the sampling or the byte layout
    /// fails here instead of silently re-keying every indexed asset.
    #[test]
    fn s1_digest_is_pinned() {
        let h = sampled_sha256(&mut Cursor::new(data(3000, 9)), 3000).unwrap();
        assert_eq!(h.to_string(), "sha256s1:745d80eb8a07c66453b41680670a72e0894f00043b6116885dca346c9d40d866");
    }

    #[test]
    fn plan_for_a_large_file_has_sixteen_blocks_inside_the_file() {
        let size = 10 * FULL_LIMIT + 12345;
        let plan = sample_plan(size);
        assert_eq!(plan.len(), SAMPLES as usize);
        assert_eq!(plan[0], (0, BLOCK));
        assert_eq!(plan[15], (size - BLOCK, BLOCK));
        assert!(plan.windows(2).all(|w| w[0].0 + BLOCK <= w[1].0));
        assert!(plan.iter().all(|&(o, l)| o + l <= size));
    }

    #[test]
    fn plan_is_safe_at_the_extremes() {
        assert_eq!(sample_plan(0), vec![(0, 0)]);
        assert_eq!(sample_plan(FULL_LIMIT), vec![(0, FULL_LIMIT)]);
        let plan = sample_plan(u64::MAX);
        assert_eq!(plan.len(), SAMPLES as usize);
        assert!(plan.iter().all(|&(o, l)| o.checked_add(l).is_some()));
    }

    #[test]
    fn empty_file_hashes() {
        let h = sampled_sha256(&mut Cursor::new(Vec::new()), 0).unwrap();
        assert_eq!(h.alg, HashAlg::Sha256Sampled1);
    }

    #[test]
    fn declared_size_must_match() {
        let a = data(100, 3);
        assert!(matches!(sampled_sha256(&mut Cursor::new(a.clone()), 99), Err(MiError::SizeMismatch(_))));
        assert!(matches!(full_sha256(&mut Cursor::new(a), 101), Err(MiError::SizeMismatch(_))));
    }

    /// A reader that reports a length and then runs dry: the truncation case of a file that is
    /// being copied while it is indexed.
    struct Lying {
        inner: Cursor<Vec<u8>>,
        claimed: u64,
    }
    impl Read for Lying {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl Seek for Lying {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            match pos {
                SeekFrom::End(0) => Ok(self.claimed),
                other => self.inner.seek(other),
            }
        }
    }

    #[test]
    fn a_reader_that_ends_early_is_an_error_not_a_hash() {
        let mut r = Lying { inner: Cursor::new(data(10, 1)), claimed: 100 };
        assert!(matches!(sampled_sha256(&mut r, 100), Err(MiError::SizeMismatch(_))));
    }
}
