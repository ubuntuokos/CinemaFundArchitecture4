//! MXF-DIGEST (CineCert / HPA draft memo, `README.md`): a message digest of an MXF file that can be
//! computed packet by packet.
//!
//! 1. Run-in bytes before the header partition pack (SMPTE ST 377-1 6.5) are skipped.
//! 2. Every KLV packet (SMPTE ST 336: key, BER length as written, value) gets its own SHA-512, the
//!    *packet digest*.
//! 3. The packet digests, in file order, are digested again with SHA-512: the *sequence digest*.
//! 4. The sequence digest is written as `urn:smpte:mxf-digest:` followed by its 88 C4 Base58 digits
//!    (SMPTE ST 2114 5.1).
//!
//! [`mxf_digest`] does all of it over a reader; [`SequenceDigest`] and [`packet_digest`] let a writer
//! digest packets as it produces them, in parallel, as long as the packet digests reach the
//! sequence digest in file order.

use std::fmt;
use std::io::Read;

use sha2::{Digest, Sha512};

use crate::{Error, Result, base58};

/// URN prefix of an MXF-DIGEST.
pub const URN_PREFIX: &str = "urn:smpte:mxf-digest:";

/// The 13 bytes every partition pack key starts with (SMPTE ST 377-1 Table 4); byte 14 is 0x02 for
/// a header partition.
const PARTITION_PREFIX: [u8; 13] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01];

/// Longest run-in ST 377-1 allows (6.5: less than 64 KiB), plus one key.
const RUN_IN_SEARCH: usize = 65_536 + 16;

/// Bytes read at a time from a packet value.
const CHUNK: usize = 1 << 16;

/// A sequence digest: the 64 bytes of a SHA-512.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MxfDigest(pub [u8; 64]);

impl MxfDigest {
    /// The URN form, `urn:smpte:mxf-digest:<88 Base58 digits>`.
    pub fn to_urn(&self) -> String {
        format!("{URN_PREFIX}{}", base58::encode(&self.0))
    }

    /// Read the URN form back. Matching is exact and case-sensitive, as the memo requires.
    pub fn from_urn(urn: &str) -> Result<Self> {
        let digits = urn.strip_prefix(URN_PREFIX).ok_or(Error::Urn("mxf-digest"))?;
        base58::decode(digits).map(Self).ok_or(Error::Urn("mxf-digest"))
    }
}

impl fmt::Display for MxfDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_urn())
    }
}

impl fmt::Debug for MxfDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MxfDigest({})", self.to_urn())
    }
}

/// The packet digest of one complete KLV packet (key, length field and value as written).
pub fn packet_digest(packet: &[u8]) -> [u8; 64] {
    Sha512::digest(packet).into()
}

/// Accumulates packet digests in file order into the sequence digest.
#[derive(Clone, Default)]
pub struct SequenceDigest {
    hasher: Sha512,
    packets: u64,
}

impl SequenceDigest {
    /// An empty sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the next packet digest in file order.
    pub fn push(&mut self, packet_digest: &[u8; 64]) {
        self.hasher.update(packet_digest);
        self.packets = self.packets.saturating_add(1);
    }

    /// Number of packets added so far.
    pub fn packets(&self) -> u64 {
        self.packets
    }

    /// The MXF-DIGEST of the packets added.
    pub fn finish(self) -> MxfDigest {
        MxfDigest(self.hasher.finalize().into())
    }
}

/// Read until `buf` is full or the input ends; the number of bytes read.
fn fill<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(n)
}

/// Offset of the header partition pack key in `head`, if there is one.
fn header_partition(head: &[u8]) -> Option<usize> {
    head.windows(16).position(|w| w[..13] == PARTITION_PREFIX && w[13] == 0x02)
}

/// The MXF-DIGEST of an MXF file read from `r`, and the number of KLV packets in it.
///
/// Reads the whole input once, in 64 KiB steps; memory use does not depend on the file size.
pub fn mxf_digest<R: Read>(mut r: R) -> Result<(MxfDigest, u64)> {
    let mut head = vec![0u8; RUN_IN_SEARCH];
    let got = fill(&mut r, &mut head)?;
    head.truncate(got);
    let start = header_partition(&head).ok_or(Error::NotMxf)?;
    let mut input = (&head[start..]).chain(r);
    let mut pos = start as u64;
    let mut seq = SequenceDigest::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let mut key = [0u8; 16];
        let n = fill(&mut input, &mut key)?;
        if n == 0 {
            break;
        }
        if n < key.len() {
            return Err(Error::Truncated(pos));
        }
        let mut packet = Sha512::new();
        packet.update(key);
        let len_at = pos + 16;
        let mut first = [0u8; 1];
        if fill(&mut input, &mut first)? == 0 {
            return Err(Error::Truncated(pos));
        }
        packet.update(first);
        let (len, len_bytes) = if first[0] < 0x80 {
            (u64::from(first[0]), 1u64)
        } else {
            let count = usize::from(first[0] & 0x7F);
            if count == 0 || count > 8 {
                return Err(Error::BadLength(len_at));
            }
            let mut rest = [0u8; 8];
            if fill(&mut input, &mut rest[..count])? < count {
                return Err(Error::Truncated(pos));
            }
            packet.update(&rest[..count]);
            (rest[..count].iter().fold(0u64, |a, &b| (a << 8) | u64::from(b)), 1 + count as u64)
        };
        let mut left = len;
        while left > 0 {
            let step = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
            let k = fill(&mut input, &mut chunk[..step])?;
            packet.update(&chunk[..k]);
            if k < step {
                return Err(Error::Truncated(pos));
            }
            left -= step as u64;
        }
        seq.push(&packet.finalize().into());
        pos = pos.saturating_add(16).saturating_add(len_bytes).saturating_add(len);
    }
    let packets = seq.packets();
    Ok((seq.finish(), packets))
}
