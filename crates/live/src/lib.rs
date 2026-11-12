//! Live graphics plumbing (credits plan C10): what goes between the engine's live runner and a
//! broadcast program such as OBS.
//!
//! - [`server`]: a small HTTP server on **127.0.0.1 only** that serves the program picture as a
//!   transparent PNG and a page for OBS's Browser Source (`/`), plus the status and, in the
//!   separate-audio mode, the cue's sound.
//! - [`obs`]: an obs-websocket 5 client (over [`ws`], a minimal RFC 6455 client) that creates or
//!   points the Browser Source at that page. OBS never drives CFA4.
//!
//! No rendering happens here: the engine renders frames and puts them into a [`FrameStore`].
//! Nothing here may bring the host down: every failure is an `Err` or a refused request.

pub mod obs;
pub mod server;
mod sha1;
pub mod store;
pub mod ws;

pub use obs::{ObsClient, ObsError};
pub use server::LiveServer;
pub use store::{AudioMode, FrameStore};
