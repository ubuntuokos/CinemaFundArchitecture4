//! Digital Cinema Package documents (DCP-02 of `docs/plans/DCP-VALIDATION-2026-10-27.md`): the
//! asset map (SMPTE ST 429-9, Interop `PROTO-ASDCP-AM`), the packing list (ST 429-8,
//! `PROTO-ASDCP-PKL`), the composition playlist (ST 429-7, `PROTO-ASDCP-CPL`, with the ST 429-10 /
//! 437-Y stereoscopic picture, ST 429-16 composition metadata and other extension assets) and the
//! volume index, read into a serialisable model. Spec editions: `README.md`.
//!
//! The reader is meant for validation as much as for import, so it is lenient: only text that is
//! not well-formed XML, a document over the [`Limits`], an unknown root element or namespace are
//! errors. Everything else (a missing `Id`, a duration that is not a number, an element the reader
//! does not know) becomes an [`Issue`] with its line and column, and the field stays `None`. Unknown
//! elements are kept by name, never dropped silently, so a validator can tell "not supported" from
//! "absent".
//!
//! Only reading. No signature check (the presence of `Signer` / `ds:Signature` is recorded) and no
//! file discovery: those belong to `cfa4-dcp-validate` (DCP-03 / DCP-05). [`sha1`] is the streaming
//! digest of the packing list `Hash`; the validator runs it over the files.
//!
//! The [`kdm`] module decrypts the content keys of a Key Delivery Message (ST 430-1) with the
//! recipient's RSA private key (RustCrypto `rsa`, `sha1`; `base64`).
//!
//! Layer L0: depends on `roxmltree`, `serde`, `thiserror`, `chrono` (date checks) and the KDM's crypto crates only. No `unsafe`, no panics on
//! malformed input, DTDs are refused and the node count is capped.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod assetmap;
mod cpl;
pub mod kdm;
mod pkl;
pub mod sha1;
pub mod subtitle;
mod xml;

pub use assetmap::{AssetMap, AssetMapAsset, Chunk, VolumeIndex};
pub use cpl::{AssetKind, Composition, ContentVersion, Marker, Rating, Reel, ReelAsset, ScreenAspectRatio};
pub use pkl::{PackingList, PackingListAsset};

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Namespaces of the documents (the root element's namespace decides the standard).
pub mod ns {
    pub const SMPTE_AM: &str = "http://www.smpte-ra.org/schemas/429-9/2007/AM";
    pub const SMPTE_PKL: &str = "http://www.smpte-ra.org/schemas/429-8/2007/PKL";
    pub const SMPTE_CPL: &str = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";
    pub const SMPTE_STEREO_CPL: &str = "http://www.smpte-ra.org/schemas/429-10/2008/Main-Stereo-Picture-CPL";
    pub const SMPTE_CPL_METADATA: &str = "http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata";
    pub const INTEROP_AM: &str = "http://www.digicine.com/PROTO-ASDCP-AM-20040311#";
    pub const INTEROP_PKL: &str = "http://www.digicine.com/PROTO-ASDCP-PKL-20040311#";
    pub const INTEROP_CPL: &str = "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";
    pub const INTEROP_STEREO_CPL: &str = "http://www.digicine.com/schemas/437-Y/2007/Main-Stereo-Picture-CPL";
    pub const XMLDSIG: &str = "http://www.w3.org/2000/09/xmldsig#";
}

/// Errors that stop reading a document. Everything else is an [`Issue`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("document is {size} bytes, over the limit of {limit}")]
    TooLarge { size: u64, limit: u64 },
    #[error("document is not UTF-8 text")]
    NotUtf8,
    #[error("not well-formed XML: {0}")]
    Xml(String),
    #[error("not a DCP document: root element {0}")]
    UnknownDocument(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Resource limits for one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest document accepted, in bytes.
    pub max_bytes: u64,
    /// Largest number of XML nodes (elements, attributes, text) parsed.
    pub max_nodes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        // A feature CPL with hundreds of reels and composition metadata stays well under 4 MiB.
        Limits { max_bytes: 32 << 20, max_nodes: 2_000_000 }
    }
}

/// The two DCP families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Standard {
    Smpte,
    Interop,
}

/// Line and column (1-based) of an element in its document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

/// What is wrong with an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueKind {
    /// A required element is absent.
    Missing,
    /// The element is present but its value does not parse.
    Invalid,
    /// The reader does not know this element (an extension, or a misspelling).
    Unknown,
    /// The element appears more often than allowed.
    Repeated,
}

/// A problem found while reading; the reader carries on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub kind: IssueKind,
    /// Element path from the root, e.g. `ReelList/Reel[2]/AssetList/MainSound/Duration`.
    pub path: String,
    pub pos: Pos,
    pub message: String,
}

/// A UUID (RFC 4122), written `urn:uuid:xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` in DCP documents.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Uuid(pub [u8; 16]);

impl Uuid {
    /// Parses `urn:uuid:` + 36 characters (case-insensitive). A bare UUID without the prefix is
    /// accepted by [`Uuid::parse_lenient`] only.
    pub fn parse(s: &str) -> Option<Uuid> {
        let s = s.trim();
        let rest = s.get(..9).filter(|p| p.eq_ignore_ascii_case("urn:uuid:")).and_then(|_| s.get(9..))?;
        Uuid::parse_bare(rest)
    }

    /// Like [`Uuid::parse`], but also accepts the 36 characters without `urn:uuid:`.
    pub fn parse_lenient(s: &str) -> Option<Uuid> {
        Uuid::parse(s).or_else(|| Uuid::parse_bare(s.trim()))
    }

    fn parse_bare(s: &str) -> Option<Uuid> {
        let b = s.as_bytes();
        if b.len() != 36 {
            return None;
        }
        let mut out = [0u8; 16];
        let mut n = 0usize;
        let mut i = 0usize;
        while i < 36 {
            if matches!(i, 8 | 13 | 18 | 23) {
                if b.get(i) != Some(&b'-') {
                    return None;
                }
                i += 1;
                continue;
            }
            let hi = hex(*b.get(i)?)?;
            let lo = hex(*b.get(i + 1)?)?;
            *out.get_mut(n)? = hi << 4 | lo;
            n += 1;
            i += 2;
        }
        Some(Uuid(out))
    }
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("urn:uuid:")?;
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl From<Uuid> for String {
    fn from(u: Uuid) -> String {
        u.to_string()
    }
}

impl TryFrom<String> for Uuid {
    type Error = String;
    fn try_from(s: String) -> std::result::Result<Uuid, String> {
        Uuid::parse(&s).ok_or_else(|| format!("not a urn:uuid: {s}"))
    }
}

/// An exact rational number (edit rates, frame rates, sample rates), written `24 1` or
/// `24000 1001`. The denominator is always positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    /// Parses two whitespace-separated integers; `None` unless the denominator is positive.
    pub fn parse(s: &str) -> Option<Rational> {
        let mut it = s.split_whitespace();
        let num = it.next()?.parse::<i64>().ok()?;
        let den = it.next()?.parse::<i64>().ok()?;
        if it.next().is_some() || den <= 0 {
            return None;
        }
        Some(Rational { num, den })
    }

    /// Exact comparison of the values (`48 2` equals `24 1`).
    pub fn same_value(self, other: Rational) -> bool {
        i128::from(self.num) * i128::from(other.den) == i128::from(other.num) * i128::from(self.den)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.num, self.den)
    }
}

/// Decodes the base64 SHA-1 digest of a `Hash` element (ST 429-8 / 429-7: 28 characters).
///
/// `Hash` is an `xs:base64Binary`, whose whitespace is collapsed: XML whitespace anywhere in the
/// text (a line break, spaces between groups) is not part of the value.
pub fn decode_sha1(s: &str) -> Option<[u8; 20]> {
    let compact: String = s.chars().filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r')).collect();
    let v = decode_base64(&compact)?;
    v.try_into().ok()
}

/// Strict RFC 4648 base64 (standard alphabet, padding required, no whitespace inside).
pub fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    let quads = b.chunks_exact(4);
    let count = quads.len();
    for (qi, q) in quads.enumerate() {
        let last = qi + 1 == count;
        let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut acc = 0u32;
        for (k, &c) in q.iter().enumerate() {
            let v = if k >= 4 - pad { 0 } else { val(c)? };
            acc = acc << 6 | v;
        }
        let bytes = [(acc >> 16) as u8, (acc >> 8) as u8, acc as u8];
        out.extend_from_slice(bytes.get(..3 - pad)?);
    }
    Some(out)
}

/// Any DCP document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Document {
    AssetMap(AssetMap),
    VolumeIndex(VolumeIndex),
    PackingList(PackingList),
    Composition(Composition),
}

impl Document {
    pub fn standard(&self) -> Standard {
        match self {
            Document::AssetMap(d) => d.standard,
            Document::VolumeIndex(d) => d.standard,
            Document::PackingList(d) => d.standard,
            Document::Composition(d) => d.standard,
        }
    }

    pub fn issues(&self) -> &[Issue] {
        match self {
            Document::AssetMap(d) => &d.issues,
            Document::VolumeIndex(d) => &d.issues,
            Document::PackingList(d) => &d.issues,
            Document::Composition(d) => &d.issues,
        }
    }
}

/// Reads any DCP document from text, deciding its kind from the root element.
pub fn parse(text: &str, limits: Limits) -> Result<Document> {
    let doc = xml::parse(text, limits)?;
    let root = doc.root_element();
    let name = root.tag_name();
    match (name.namespace(), name.name()) {
        (Some(ns::SMPTE_AM), "AssetMap") => Ok(Document::AssetMap(assetmap::read(&doc, Standard::Smpte))),
        (Some(ns::INTEROP_AM), "AssetMap") => Ok(Document::AssetMap(assetmap::read(&doc, Standard::Interop))),
        (Some(ns::SMPTE_AM), "VolumeIndex") => Ok(Document::VolumeIndex(assetmap::read_volume_index(&doc, Standard::Smpte))),
        (Some(ns::INTEROP_AM), "VolumeIndex") => Ok(Document::VolumeIndex(assetmap::read_volume_index(&doc, Standard::Interop))),
        (Some(ns::SMPTE_PKL), "PackingList") => Ok(Document::PackingList(pkl::read(&doc, Standard::Smpte))),
        (Some(ns::INTEROP_PKL), "PackingList") => Ok(Document::PackingList(pkl::read(&doc, Standard::Interop))),
        (Some(ns::SMPTE_CPL), "CompositionPlaylist") => Ok(Document::Composition(cpl::read(&doc, Standard::Smpte))),
        (Some(ns::INTEROP_CPL), "CompositionPlaylist") => Ok(Document::Composition(cpl::read(&doc, Standard::Interop))),
        (ns, local) => Err(Error::UnknownDocument(match ns {
            Some(ns) => format!("{{{ns}}}{local}"),
            None => local.to_string(),
        })),
    }
}

/// Reads any DCP document from bytes (UTF-8, an optional byte order mark).
pub fn parse_bytes(bytes: &[u8], limits: Limits) -> Result<Document> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.max_bytes {
        return Err(Error::TooLarge { size: u64::try_from(bytes.len()).unwrap_or(u64::MAX), limit: limits.max_bytes });
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| Error::NotUtf8)?;
    parse(text, limits)
}

/// Reads a DCP document file, refusing it before reading when it is over the size limit.
pub fn read(path: &Path, limits: Limits) -> Result<Document> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size > limits.max_bytes {
        return Err(Error::TooLarge { size, limit: limits.max_bytes });
    }
    // The reported size is only a hint (a sparse or growing file); a reservation that cannot be made
    // is an error, never an abort.
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(usize::try_from(size).unwrap_or(usize::MAX)).map_err(|_| Error::TooLarge { size, limit: limits.max_bytes })?;
    // `take` guards against a file that grows while it is read.
    file.take(limits.max_bytes.saturating_add(1)).read_to_end(&mut bytes)?;
    parse_bytes(&bytes, limits)
}

#[cfg(test)]
mod tests;
