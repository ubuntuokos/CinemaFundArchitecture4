//! CFA4-wide task and resource infrastructure, the Adaptive Capability System (ACS; layer L2,
//! no UI, no FFI, no `unsafe`).
//!
//! One crate for every resource-hungry CFA4 module (AI, render, proxies, codec conversion, Media
//! Intelligence), so none of them builds its own competing resource manager (ADR-0008, decision
//! 2026-11-07; `cfa4-ai-jobs` will be a client of it). It grows phase by phase
//! (`docs/ai/acs-adaptiv-eroforras.md`):
//!
//! - **ACS-1 (done): hardware discovery and runtime resource monitor.** [`sample`] reads the
//!   machine's **nominal** resources (RAM, CPUs, cgroup limits, every GPU's VRAM) and its
//!   **current** ones (free memory, swap, PSI pressure, VRAM in use) as separate types;
//!   [`class`] places the machine against the owner's floors (16 / 32 GB RAM, 12 GB VRAM for
//!   AI); [`Monitor`] turns samples into events and tells a spike from a sustained limit.
//! - **ACS-2 (done): resource availability manager.** [`Budget`] turns the nominal
//!   resources and the user's [`BudgetSettings`] into what heavy work may take at most (reserves for
//!   the desktop, display GPU left out); [`Manager::try_reserve`] grants a [`Request`] all at once or
//!   not at all, against the budget, the other reservations, the current load and the monitor's
//!   pressure levels, and tells [`Denied::NeverFits`] (pick another path) from [`Denied::Wait`].
//! - **ACS-3 (done): adaptive execution planner.** [`plan()`] takes a task's validated
//!   [`ExecutionPath`]s (SDK manifest v5 `paths`, resolved for the job) and decides: run the
//!   reference path, switch on its own only to a result-preserving (`automatic`) path, wait, or
//!   offer a result-changing path for the user's approval; "currently not executable" when none
//!   fits. Every path gets a time [`Estimate`] as a range with its confidence.
//! - **ACS-4 (here now): task queue and scheduler.** [`Queue`] holds the rules every store
//!   follows: states (SDK host report v2), order (class, user priority, age) with starvation
//!   protection, four [`StartMode`]s, leases that never restart a task from the beginning, version
//!   checks and missed starts. The PostgreSQL store (`tasks` schema, PG-2) applies the same
//!   operations in transactions and must pass the same tests.
//!
//! GPU discovery is `cfa4-route`'s probe; the VRAM and display reading from sysfs is [`gpumem`]; this crate adds the system
//! side (`/proc`, cgroup v2, PSI) and the policy. Unknown numbers stay `None` everywhere: unknown
//! is never treated as free or as enough.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod budget;
pub mod class;
pub mod gpumem;
pub mod manager;
pub mod monitor;
pub mod plan;
pub mod queue;
pub mod resources;
pub mod schedule;
mod system;

pub use budget::{Budget, BudgetSettings, GpuBudget, GpuExcluded};
pub use class::{AiFloor, FloorReason, RamProfile, ai_floor, ram_profile};
pub use manager::{Class, Denied, GpuRequest, Grant, Manager, Request, ReservationId, Resource, WaitReason};
pub use monitor::{Event, Level, Monitor, MonitorConfig, NominalChange, Signal};
pub use plan::{Assessment, Confidence, Decision, Estimate, ExecutionPath, Fit, Job, Metric, Plan, Provider, Tolerance, plan};
pub use queue::{Actor, Awaiting, MissedStart, NewTask, Priority, Queue, QueueConfig, QueueError, QueueEvent, StartMode, State, Task, Transition, UtcMs};
pub use resources::{Current, GpuCurrent, GpuNominal, Nominal, Snapshot, sample, sample_root};
pub use system::Pressure;
