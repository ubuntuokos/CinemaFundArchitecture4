//! Capability decision (rule A4): what a request does when a backend cannot do it.
//! Export fails closed; preview may fall back, but the fallback is always reported. Nothing
//! degrades quietly.

use cfa4_frame::convert::PixelFormat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Context {
    Export,
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Place {
    Cpu,
    Gpu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Support {
    Supported,
    /// Export: stop with a named error.
    FailClosed,
    /// Preview: continue on another path and say so (a `RenderEvent` of kind `fallback`).
    FallbackWarned,
}

/// What a backend can do for one format and place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderCapability {
    pub format: PixelFormat,
    pub bit_depth: u8,
    pub place: Place,
    pub supported: bool,
    /// Why not, when `supported` is false (shown to the user, logged).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

pub fn decide(cap: &RenderCapability, ctx: Context) -> Support {
    match (cap.supported, ctx) {
        (true, _) => Support::Supported,
        (false, Context::Export) => Support::FailClosed,
        (false, Context::Preview) => Support::FallbackWarned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(supported: bool) -> RenderCapability {
        RenderCapability { format: PixelFormat::RgbaF32, bit_depth: 32, place: Place::Gpu, supported, reason: String::new() }
    }

    #[test]
    fn unsupported_never_continues_quietly() {
        assert_eq!(decide(&cap(true), Context::Export), Support::Supported);
        assert_eq!(decide(&cap(false), Context::Export), Support::FailClosed);
        assert_eq!(decide(&cap(false), Context::Preview), Support::FallbackWarned);
    }
}
