//! CFA4 hardware control surface catalogue (layer L1; plan CFA4-HW-CONTROL-SURFACES-001 FINAL 1.1).
//!
//! - [`catalog`]: the device catalogue (§2), TOML files under `crates/surface-catalog/catalog/`,
//!   one family or device per file, so a new device needs no code change (§6.2).
//! - [`report`]: hardware test reports (§4), YAML files under `docs/audit/hw/`.
//! - [`status`]: the feature and model status computed from both (§3.2, §4.3) and the gate of the
//!   mapping editor (§3.4).
//!
//! The three support dimensions are kept apart (§3.1): the goal is written in the catalogue, the
//! proven state is computed (a feature is VERIFIED only through a valid report, AC-04), and the
//! connection state belongs to the running device layer.
//!
//! The event model (`SurfaceEvent`, `SurfaceInput`, `SurfaceFeedback`) is not here: it belongs to
//! `cfa4-surface-model` of CFA4-CONTROL-SURFACE-001 (integration annex E1,
//! `docs/plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md`). This crate does no
//! device I/O.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::fmt;

pub mod catalog;
pub mod model;
pub mod report;
pub mod status;

pub use catalog::{Catalog, Model};
pub use model::{ConnectionState, ControlType, FeatureStatus, ModelStatus, Phase, SupportGoal, SystemPath};
pub use report::Report;
pub use status::{Adapters, Gate, ModelState, evaluate, evaluate_all, gate};

/// One problem found while checking a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub origin: String,
    pub message: String,
}

impl Problem {
    pub(crate) fn new(origin: &str, message: String) -> Problem {
        Problem { origin: origin.to_owned(), message }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.origin, self.message)
    }
}

/// Errors of this crate. Never a panic: a bad file is an error the user can act on.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("{origin}: {message}")]
    Parse { origin: String, message: String },
    #[error("{} problem(s): {}", .0.len(), .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<Problem>),
}
