//! Identifiers of digital cinema and IMF packages (CINECERT WP08 of
//! `docs/plans/CFA4-CINECERT-001-v2.3.md`):
//!
//! - [`mxf_digest`]: the MXF-DIGEST of an MXF file, a SHA-512 over the SHA-512 of every KLV packet,
//!   written as `urn:smpte:mxf-digest:<Base58>`. Packets can be digested in parallel or out of
//!   order, so a track file can be identified while it is written.
//! - [`vtfp`]: the IMF virtual track fingerprint of one virtual track of an IMF composition
//!   playlist (SMPTE ST 2067-3), a SHA-1 over its canonical timeline, written as
//!   `urn:smpte:imf-vtfp:<hex>`. Two compositions that play the same edit units of the same track
//!   files on a track get the same fingerprint, however their segments and resources are laid out.
//!
//! - [`isdcf`]: an offline snapshot of the ISDCF metadata registries and a checker / builder for
//!   names that follow the ISDCF Digital Cinema Naming Convention (CINECERT WP09).
//!
//! - [`delivery`]: verified copy of a finished package to its delivery target, marked incomplete
//!   until every file is read back and matches, resumable, with a deterministic SHA-256 manifest;
//!   [`drive`]: a read-only check of a DCP delivery drive (MBR, ext2/ext3, 128-byte inodes)
//!   (CINECERT WP13); [`archive`]: the provenance report and BagIt (RFC 8493) archive bags.
//!
//! MXF-DIGEST and VTFP follow the draft memos published by CineCert for the HPA (`README.md` names the revisions).
//! They are implemented from the memos' text; the memos' own test material is the test oracle.
//!
//! Layer L1: depends on `sha1`, `sha2`, `roxmltree`, `serde_json`, `chrono` and `thiserror` only. No `unsafe`, no panics on
//! malformed input; DTDs are refused and the XML node count is capped.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod archive;
mod base58;
pub mod delivery;
pub mod drive;
pub mod isdcf;
pub mod mxf_digest;
pub mod vtfp;

#[cfg(test)]
mod tests;

pub use mxf_digest::{MxfDigest, mxf_digest};
pub use vtfp::{VirtualTrack, Vtfp, fingerprint, virtual_tracks};

/// Why an identifier could not be computed or read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading the input failed.
    #[error("read error: {0}")]
    Io(#[from] std::io::Error),
    /// The input is not an MXF file: no header partition pack in the first 65 536 bytes.
    #[error("not an MXF file: no header partition pack within the run-in")]
    NotMxf,
    /// The file ends inside a KLV packet.
    #[error("truncated KLV packet at byte {0}")]
    Truncated(u64),
    /// A BER length is malformed (longer than 8 bytes, or the reserved form 0x80).
    #[error("invalid BER length at byte {0}")]
    BadLength(u64),
    /// The composition playlist is not well-formed XML, has a DTD or is too large.
    #[error("composition playlist: {0}")]
    Xml(String),
    /// The composition playlist does not hold what the fingerprint needs.
    #[error("composition playlist: {0}")]
    Cpl(String),
    /// The text is not a URN of this kind.
    #[error("not a valid {0} URN")]
    Urn(&'static str),
    /// The ISDCF registry snapshot, or a registry checkout being reduced to one, is unusable.
    #[error("ISDCF registry: {0}")]
    Registry(String),
    /// A delivery cannot start or did not complete; the target stays marked incomplete.
    #[error("delivery: {0}")]
    Delivery(String),
    /// The caller stopped a delivery; the target stays marked incomplete and can be resumed.
    #[error("delivery cancelled")]
    Cancelled,
}

/// Result type of this crate.
pub type Result<T> = std::result::Result<T, Error>;
