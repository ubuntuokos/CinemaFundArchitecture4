//! Render contracts v0 (CFA4-RENDER-INTEGRATION-PLAN-001, phase C0).
//!
//! A thin crate. It does not own the frame, the queue or the router: those stay in
//! `cfa4-frame`, `cfa4-tasks` and `cfa4-route` and are only *named* here, so that the renderer, the
//! adapters, the Qt GUI and the CLI import the contract from one place. What is new here is small
//! and value-typed: the [`matte`] plane (a key or an AI matte as one value), the [`capability`]
//! decision (fail closed for export, a warned fallback for preview, never silent), the [`audio`]
//! channel layout, the [`worker`] protocol messages and the [`capture`] trait (no implementation).
//!
//! The contract changes only with [`ContractVersion`]: a field or a state mapping that moves
//! breaks a test in `contract_tests` until the spec and the version are updated together.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod audio;
pub mod capability;
pub mod capture;
pub mod matte;
pub mod version;
pub mod worker;

#[cfg(test)]
mod contract_tests;

pub use audio::{AudioFormat, AudioFormatError, ChannelLayout};
pub use capability::{Context, Place, RenderCapability, Support, decide};
pub use capture::{CaptureError, CaptureSource};
pub use matte::{MAX_MATTE_DIM, MAX_MATTE_PIXELS, MatteData, MatteDepth, MatteError, MatteFrame, MatteMethod, MatteOrigin, MatteRef, ModelRef};
pub use version::{CONTRACT_VERSION, ContractVersion};
pub use worker::{HandshakeError, PathPolicy, ResourceLimits, WorkerKind, WorkerMessage, negotiate};

// The existing types the contract is made of (not redefined).
pub use cfa4_frame::convert::{Conversion, Cost, PixelFormat, Unsupported, convert, plan};
pub use cfa4_frame::side::{FrameSideData, Residency, Rotation};
pub use cfa4_frame::{AudioBuffer, VideoFrame};
pub use cfa4_route::{Declined, Family, Reason};
pub use cfa4_tasks::{Class, RenderEvent, RenderEventKind, RenderFallback, RenderProgress, RenderState, State};
