//! The `ffmpeg` backend of the compute router (`cfa4-route`, ADR 0004 / ADR 0005).
//!
//! FFmpeg decodes on the CPU in this first step; its hardware paths (VA-API, Vulkan) come with
//! measurements later. It takes codec decoding when it is compiled in, registered (a GPL-3
//! FFmpeg >= 9.0.2 was found) and switched on; encoding through FFmpeg is not wired yet.

use std::sync::Arc;

use cfa4_route::{Backend, BackendId, Device, DeviceKind, Family, Inventory, Task};

/// The router id of this backend, as `routing.toml` names it.
pub const ROUTE_ID: &str = "ffmpeg";

struct FfmpegRoute {
    id: BackendId,
}

impl Backend for FfmpegRoute {
    fn id(&self) -> &BackendId {
        &self.id
    }

    fn devices<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Device> {
        inventory.devices().iter().filter(|d| d.kind == DeviceKind::Cpu).collect()
    }

    fn accepts(&self, task: &Task, _device: &Device) -> Result<(), String> {
        if task.family != Family::CodecDecode {
            return Err("FFmpeg only decodes in this version of CFA4".into());
        }
        if !crate::compiled_in() {
            return Err("this build of CFA4 was made without the FFmpeg backend".into());
        }
        if !crate::registered() {
            return Err("no usable FFmpeg (GPL-3 build, 9.0.2 or later) was found".into());
        }
        if !crate::enabled() {
            return Err("the FFmpeg backend is switched off in Settings ▸ Media Backends".into());
        }
        Ok(())
    }
}

/// The backend to register with the router (`Router::register`). `None` only if the shipped id
/// were invalid, which a test rules out.
pub fn route_backend() -> Option<Arc<dyn Backend>> {
    let id = BackendId::new(ROUTE_ID).ok()?;
    Some(Arc::new(FfmpegRoute { id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_route::{Preferences, Router, Table};

    #[test]
    fn the_route_id_is_valid_and_in_the_shipped_table() {
        let Some(backend) = route_backend() else { panic!("route id must be valid") };
        assert_eq!(backend.id().as_str(), ROUTE_ID);
        let table = Table::builtin().unwrap_or_else(|e| panic!("{e}"));
        let text = format!("{table:?}");
        assert!(text.contains("\"ffmpeg\""), "routing.toml must name the ffmpeg backend");
    }

    #[test]
    fn declines_encoding_and_explains_why_it_declines_decoding() {
        let Some(backend) = route_backend() else { panic!("route id must be valid") };
        let cpu = Device::cpu();
        let encode = Task::new("video.encode.h264", Family::CodecEncode).unwrap_or_else(|e| panic!("{e}"));
        assert!(backend.accepts(&encode, &cpu).is_err());
        let decode = Task::new("video.decode.h264.422.10bit", Family::CodecDecode).unwrap_or_else(|e| panic!("{e}"));
        // nothing registered FFmpeg in this test process: declined with a reason, never a panic
        let reason = backend.accepts(&decode, &cpu).err().unwrap_or_default();
        assert!(!reason.is_empty());
    }

    #[test]
    fn the_router_passes_over_an_unregistered_ffmpeg_with_a_reason() {
        let Some(backend) = route_backend() else { panic!("route id must be valid") };
        let table = Table::builtin().unwrap_or_else(|e| panic!("{e}"));
        let mut router = Router::new(table, Inventory::new([Device::cpu()]), Preferences::default());
        router.register(backend);
        let task = Task::new("video.decode.hevc.422.10bit", Family::CodecDecode).unwrap_or_else(|e| panic!("{e}"));
        let chain = router.chain(&task);
        let text = format!("{chain:?}");
        assert!(text.contains("ffmpeg"), "{text}");
    }
}
