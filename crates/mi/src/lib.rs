//! CFA4 Media Intelligence core (layer L2).
//!
//! Pure logic with no database and no I/O beyond the reader the caller hands in: the stable media
//! identity ([`hash`]), time anchors in exact [`cfa4_time::Tick`]s ([`anchor`]) and the provenance
//! classes of machine and human data ([`provenance`]). The `mi` PostgreSQL schema lives in
//! `cfa4-store` and implements the traits this crate will grow; the engine reaches the index only
//! through them. Plan: `CFA4-MEDIA-INTELLIGENCE-001` (FINAL 2.0 and the execution plan, F1).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod anchor;
pub mod error;
pub mod hash;
pub mod provenance;

pub use anchor::TimeAnchor;
pub use error::MiError;
pub use hash::{ContentHash, HashAlg};
pub use provenance::Provenance;
