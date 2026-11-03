//! JPEG XL decoder (ISO/IEC 18181), ported from jxl-oxide.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod api;
pub mod bitstream;
pub mod cfa4;
pub mod coding;
pub mod color;
pub mod common;
pub mod frame;
pub mod grid;
pub mod image;
pub mod modular;
pub mod render;
pub mod threadpool;
mod tracing;
pub mod vardct;

pub use api::*;
