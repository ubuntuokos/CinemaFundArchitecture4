//! CFA4 Smart Cut / Smart Render planning (layer L3).
//!
//! The specification is `CFA4-SMART-CUT-RENDER-R1` (`docs/plans/CFA4-SMART-CUT-RENDER-R1.md`);
//! what CFA4 could already do when this work started is in `docs/audit/smart-edit-baseline.md`.
//!
//! This crate decides, never executes: it says which parts of an export may be copied packet for
//! packet, which need their boundaries re-encoded and which must be rendered in full. Execution
//! stays in the existing export pipeline (`cfa4-export`, driven by `cfa4-engine`).
//!
//! - [`capabilities`] (WP-01): the media capability registry. Every support decision (demux,
//!   decode, GOP index, packet copy, encode, mux…) comes from it, with its reason and evidence,
//!   and is reproducible from the registry contents alone.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod capabilities;

#[cfg(test)]
mod capabilities_tests;
