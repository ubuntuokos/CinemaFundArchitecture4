//! The compute router's order for the supplementary media backends (ADR 0005, step 2).
//!
//! CFA4's own engine opens every file first, hardware and software alike (owner decision
//! 2026-10-27): the supplementary backends (FFmpeg 9, GStreamer) are a fallback for what the
//! engine cannot open or decode. The router (`cfa4_route`, ADR 0004) decides only their order and
//! which of them may act, from the `codec_decode` order of the routing table (the shipped
//! `routing.toml` with the user's `~/.config/cfa4/routing.toml` on top, where a
//! `[compute.task."media.fallback"]` order can put GStreamer before FFmpeg or leave one out) and
//! Preferences ▸ Media Backends. The chain is published to `cfa4_route::report`, so `perf.stats`
//! and `media.backends` show it, with every backend passed over and why.

use std::sync::Arc;

use cfa4_route::{Backend, BackendId, Chain, Device, DeviceKind, Error, Family, Inventory, Preferences, Router, Table, Task};

/// The routing task of the media fallbacks.
pub const FALLBACK_TASK: &str = "media.fallback";

/// CFA4's own decoders: listed so the chain shows them first, never handed to the fallbacks.
const ENGINE: &str = "software";

/// A backend that runs on the CPU and takes every file (it decides per file when it is asked).
struct Cpu(BackendId);

impl Backend for Cpu {
    fn id(&self) -> &BackendId {
        &self.0
    }

    fn devices<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Device> {
        inventory.devices().iter().filter(|d| d.kind == DeviceKind::Cpu).collect()
    }

    fn accepts(&self, _task: &Task, _device: &Device) -> Result<(), String> {
        Ok(())
    }

    fn serves(&self, family: Family) -> bool {
        family == Family::CodecDecode
    }
}

/// The fallback chain: `backends` are the registered fallbacks with whether Preferences ▸ Media
/// Backends lets them act. The routes after CFA4's own decoders are the fallback order; the routes
/// passed over are only the supplementary backends' (the hardware decoders are routed, and
/// reported, by `cfa4-platform`).
pub fn fallback_chain(table: Table, backends: &[(&str, bool)]) -> Result<Chain, Error> {
    let mut router = Router::new(table, Inventory::new([Device::cpu()]), Preferences::default());
    router.register(Arc::new(Cpu(BackendId::new(ENGINE)?)));
    for (id, enabled) in backends {
        let id = BackendId::new(*id)?;
        router.register(Arc::new(Cpu(id.clone())));
        router.set_enabled(&id, *enabled);
    }
    let mut chain = router.chain(&Task::new(FALLBACK_TASK, Family::CodecDecode)?);
    let supplementary = |b: &BackendId| backends.iter().any(|(id, _)| *id == b.as_str());
    chain.declined.retain(|d| supplementary(&d.backend));
    Ok(chain)
}

/// The fallback ids in the chain's order, CFA4's own decoders left out.
pub fn fallback_order(chain: &Chain) -> Vec<String> {
    chain.routes.iter().map(|r| r.backend.as_str().to_string()).filter(|b| b != ENGINE).collect()
}

/// The table this process routes with: the shipped one, with the user's file on top.
fn table() -> Result<Table, Error> {
    let Some(path) = Table::user_file_path() else {
        return Table::builtin();
    };
    let (table, warning) = Table::builtin_with_user_file(&path)?;
    if let Some(w) = warning {
        log::warn!("routing table: {w}; using the shipped table");
    }
    Ok(table)
}

/// Routes the registered fallbacks and hands their order to `cfa4_media`. Without a table (a build
/// error the router tests catch) the registration order stays, with the switches still applied.
pub fn apply(backends: &[(&str, bool)]) {
    match table().and_then(|t| fallback_chain(t, backends)) {
        Ok(chain) => {
            let order = fallback_order(&chain);
            log::info!("route {FALLBACK_TASK}: {}", if order.is_empty() { "no fallback".to_string() } else { order.join(", ") });
            for x in &chain.declined {
                log::info!("route {FALLBACK_TASK}: passed over {} ({:?})", x.backend, x.reason);
            }
            cfa4_media::set_fallback_order(Some(order));
            cfa4_route::report::publish(chain);
        }
        Err(e) => {
            log::warn!("media fallback route: {e}");
            cfa4_media::set_fallback_order(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_route::Reason;

    fn reasons(chain: &Chain) -> Vec<(&str, Reason)> {
        chain.declined.iter().map(|d| (d.backend.as_str(), d.reason.clone())).collect()
    }

    #[test]
    fn the_engine_comes_first_then_ffmpeg_then_gstreamer() {
        let chain = fallback_chain(Table::builtin().unwrap(), &[("gstreamer", true), ("ffmpeg", true)]).unwrap();
        let all: Vec<&str> = chain.routes.iter().map(|r| r.backend.as_str()).collect();
        assert_eq!(all, ["software", "ffmpeg", "gstreamer"]);
        assert_eq!(fallback_order(&chain), ["ffmpeg", "gstreamer"]);
        assert!(chain.declined.is_empty(), "{:?}", chain.declined);
    }

    #[test]
    fn a_switched_off_backend_is_left_out_and_reported() {
        let chain = fallback_chain(Table::builtin().unwrap(), &[("ffmpeg", false), ("gstreamer", true)]).unwrap();
        assert_eq!(fallback_order(&chain), ["gstreamer"]);
        assert_eq!(reasons(&chain), [("ffmpeg", Reason::Disabled)]);
    }

    #[test]
    fn an_unregistered_backend_is_reported_but_not_tried() {
        // a build without the gstreamer feature registers only FFmpeg
        let chain = fallback_chain(Table::builtin().unwrap(), &[("ffmpeg", true)]).unwrap();
        assert_eq!(fallback_order(&chain), ["ffmpeg"]);
        assert!(chain.declined.is_empty(), "the hardware decoders and unregistered backends are not this chain's: {:?}", chain.declined);
    }

    #[test]
    fn a_user_table_can_swap_or_drop_the_fallbacks() {
        let user = Table::parse("[compute.task.\"media.fallback\"]\norder = [\"software\", \"gstreamer\", \"ffmpeg\"]\n").unwrap();
        let chain = fallback_chain(Table::builtin().unwrap().merged(user), &[("ffmpeg", true), ("gstreamer", true)]).unwrap();
        assert_eq!(fallback_order(&chain), ["gstreamer", "ffmpeg"]);

        let user = Table::parse("[compute.task.\"media.fallback\"]\norder = [\"software\", \"ffmpeg\"]\n").unwrap();
        let chain = fallback_chain(Table::builtin().unwrap().merged(user), &[("ffmpeg", true), ("gstreamer", true)]).unwrap();
        assert_eq!(fallback_order(&chain), ["ffmpeg"], "a backend the table leaves out is not used");
    }

    #[test]
    fn hostile_backend_ids_are_an_error_not_a_crash() {
        assert!(fallback_chain(Table::builtin().unwrap(), &[("", true)]).is_err());
    }
}
