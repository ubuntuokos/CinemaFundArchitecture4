//! The compute router's decisions for this crate (ADR 0004, phase 1): which hardware video decoder
//! [`crate::register`] puts in front of CFA4's own decoders, and whether the NVENC export encoder
//! may act.
//!
//! The order comes from the routing table (`cfa4_route::Table`, the shipped `routing.toml` with the
//! user's `~/.config/cfa4/routing.toml` on top) and the `/sys` device probe. The shipped order is
//! today's: NVDEC, then VA-API, then CFA4's software decoders; hardware encoding stays opt-in per
//! export (Export ▸ Hardware encoding, Off by default), so the encoder decision here is the route an
//! export with hardware encoding Auto takes.
//!
//! Each backend's `accepts` is its own driver probe, asked only when the router reaches it: a
//! working NVDEC means VA-API (whose `vaInitialize` can block on some NVIDIA nodes) is never
//! probed. The decisions are made once per process and published to `cfa4_route::report` for
//! `perf.stats`.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use cfa4_route::{Backend, BackendId, Chain, Decision, Device, DeviceKind, Error, Family, Inventory, Preference, Preferences, Router, Table, Task};

/// The task the hardware decoder registration routes (H.264 and HEVC streams).
pub const DECODE_TASK: &str = "video.decode";
/// The task the hardware H.264 export encoder routes.
pub const ENCODE_TASK: &str = "video.encode.h264";

/// A driver probe: `Ok(description)` when the backend works here, `Err(why)` otherwise.
pub type Probe = Box<dyn Fn() -> Result<String, String> + Send + Sync>;

/// A hardware backend: runs on the GPUs `select` picks, and accepts when its probe succeeds.
struct Hardware {
    id: BackendId,
    family: Family,
    select: fn(&Device) -> bool,
    probe: Probe,
}

impl Backend for Hardware {
    fn id(&self) -> &BackendId {
        &self.id
    }

    fn devices<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Device> {
        inventory.gpus().filter(|d| (self.select)(d)).collect()
    }

    fn accepts(&self, _task: &Task, _device: &Device) -> Result<(), String> {
        (self.probe)().map(|_| ())
    }

    fn serves(&self, family: Family) -> bool {
        family == self.family
    }
}

/// CFA4's own software codecs: the CPU, always.
struct Software(BackendId);

impl Backend for Software {
    fn id(&self) -> &BackendId {
        &self.0
    }

    fn devices<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Device> {
        inventory.devices().iter().filter(|d| d.kind == DeviceKind::Cpu).collect()
    }

    fn accepts(&self, _task: &Task, _device: &Device) -> Result<(), String> {
        Ok(())
    }
}

/// The driver probes the decoder and encoder routes use (stand-ins in tests).
pub struct Probes {
    pub nvdec: Probe,
    pub vaapi: Probe,
}

/// NVDEC needs NVIDIA's proprietary driver.
fn nvidia_proprietary(d: &Device) -> bool {
    d.is_nvidia_proprietary()
}

/// VA-API is tried on every GPU, as before the router: its own probe decides (Mesa also has
/// VA-API drivers for virtual GPUs).
fn any_gpu(_: &Device) -> bool {
    true
}

fn id(s: &str) -> Result<BackendId, Error> {
    BackendId::new(s)
}

/// The routes of this crate on one machine.
#[derive(Debug, Clone)]
pub struct Routes {
    /// The hardware decoder: `nvdec`, `vaapi`, or `software` (no hardware decoder).
    pub decode: Decision,
    pub decode_chain: Chain,
    /// The export encoder an export with hardware encoding Auto takes: `nvenc` or `software`.
    pub encode: Decision,
    pub encode_chain: Chain,
}

impl Routes {
    pub fn decoder(&self) -> &str {
        self.decode.backend.as_str()
    }

    pub fn encoder(&self) -> &str {
        self.encode.backend.as_str()
    }
}

/// Routes the hardware decoder and encoder on `inventory` with `table`.
pub fn route(table: Table, inventory: Inventory, probes: Probes) -> Result<Routes, Error> {
    // Registration routes with every family on Automatic: the per-stream Hardware decoding setting
    // and the per-export Hardware encoding setting still decide at run time, as before.
    let mut prefs = Preferences::default();
    prefs.set(Family::CodecEncode, Preference::Automatic);
    let mut router = Router::new(table, inventory, prefs);
    let decoder = |name: &str, select: fn(&Device) -> bool, probe: Probe| -> Result<Arc<dyn Backend>, Error> {
        Ok(Arc::new(Hardware { id: id(name)?, family: Family::CodecDecode, select, probe }))
    };
    router.register(decoder("nvdec", nvidia_proprietary, probes.nvdec)?);
    // VA-API decodes only: VA-API encoding is ADR 0004 phase 3, and the encode family reports
    // its `vaapi` route as not implemented
    router.register(decoder("vaapi", any_gpu, probes.vaapi)?);
    // NVENC is asked at export time (a session per export); whether an NVIDIA GPU on the
    // proprietary driver is here is all the route needs
    router.register(Arc::new(Hardware { id: id("nvenc")?, family: Family::CodecEncode, select: nvidia_proprietary, probe: Box::new(|| Ok("NVENC".into())) }));
    router.register(Arc::new(Software(id("software")?)));

    let decode_task = Task::new(DECODE_TASK, Family::CodecDecode)?;
    let decode = router.choose(&decode_task)?;
    let decode_chain = router.fallback_chain(&decode_task, &decode);

    let encode_task = Task::new(ENCODE_TASK, Family::CodecEncode)?;
    let encode = router.choose(&encode_task)?;
    let encode_chain = router.fallback_chain(&encode_task, &encode);
    Ok(Routes { decode, decode_chain, encode, encode_chain })
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

fn real_probes() -> Probes {
    static NVDEC: OnceLock<Result<String, String>> = OnceLock::new();
    Probes {
        nvdec: Box::new(|| {
            NVDEC
                .get_or_init(|| {
                    log::info!("startup stage: probing NVDEC CUDA device");
                    let result = crate::nvdec::cuvid::probe();
                    log::info!("startup stage: NVDEC CUDA device probe returned");
                    result
                })
                .clone()
        }),
        vaapi: Box::new(|| {
            log::info!("startup stage: probing VA-API display");
            let result = (*crate::vaapi::va::probe()).clone();
            log::info!("startup stage: VA-API display probe returned");
            result
        }),
    }
}

/// This process's routes, made on first use and published to `cfa4_route::report`. `None` only
/// when the shipped table itself cannot be read (a build error the router tests catch); callers
/// then keep the pre-router behaviour.
pub fn current() -> Option<&'static Routes> {
    static ROUTES: OnceLock<Option<Routes>> = OnceLock::new();
    ROUTES
        .get_or_init(|| {
            let inventory = cfa4_route::probe::probe(Path::new("/"));
            for d in inventory.devices() {
                log::info!("compute device: {} ({})", d.name, d.id);
            }
            for n in inventory.notes() {
                log::info!("compute device probe: {n}");
            }
            cfa4_route::report::publish_inventory(inventory.clone());
            let routes = table().and_then(|t| route(t, inventory, real_probes()));
            match routes {
                Ok(r) => {
                    for d in [&r.decode, &r.encode] {
                        log::info!("route {}: {} on {}", d.task, d.backend, d.device);
                        for x in &d.declined {
                            log::info!("route {}: passed over {} ({:?})", d.task, x.backend, x.reason);
                        }
                    }
                    cfa4_route::report::publish(r.decode_chain.clone());
                    cfa4_route::report::publish(r.encode_chain.clone());
                    Some(r)
                }
                Err(e) => {
                    log::warn!("compute router: {e}");
                    None
                }
            }
        })
        .as_ref()
}

/// Whether the router lets the NVENC export encoder act (`Err` says why not). Without routes the
/// encoder decides alone, as before the router.
pub fn nvenc_allowed() -> Result<(), String> {
    match current() {
        None => Ok(()),
        Some(r) if r.encoder() == "nvenc" => Ok(()),
        Some(r) => {
            let why = r
                .encode
                .declined
                .iter()
                .find(|x| x.backend.as_str() == "nvenc")
                .map_or_else(|| "not in the routing order".to_string(), |x| format!("{:?}", x.reason));
            Err(format!("the compute router chose {} for H.264 encoding (NVENC: {why})", r.encoder()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cfa4_route::{DeviceId, Reason, Vendor};

    use super::*;

    fn gpu(node: &str, vendor: Vendor, driver: &str) -> Device {
        Device {
            id: DeviceId::new(format!("gpu:{node}")).unwrap(),
            kind: DeviceKind::Gpu,
            vendor,
            name: node.into(),
            driver: Some(driver.into()),
            pci_device: None,
            free_memory_mb: None,
        }
    }

    fn probes(nvdec: Result<&'static str, &'static str>, vaapi: Result<&'static str, &'static str>, vaapi_calls: Arc<AtomicUsize>) -> Probes {
        Probes {
            nvdec: Box::new(move || nvdec.map(str::to_string).map_err(str::to_string)),
            vaapi: Box::new(move || {
                vaapi_calls.fetch_add(1, Ordering::Relaxed);
                vaapi.map(str::to_string).map_err(str::to_string)
            }),
        }
    }

    fn routes(inv: Inventory, nvdec: Result<&'static str, &'static str>, vaapi: Result<&'static str, &'static str>) -> (Routes, usize) {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = route(Table::builtin().unwrap(), inv, probes(nvdec, vaapi, calls.clone())).unwrap();
        (r, calls.load(Ordering::Relaxed))
    }

    fn chain(c: &Chain) -> Vec<&str> {
        c.routes.iter().map(|r| r.backend.as_str()).collect()
    }

    #[test]
    fn a_working_nvdec_skips_the_blocking_vaapi_probe() {
        let inv = Inventory::new([gpu("renderd128", Vendor::Nvidia, "nvidia")]);
        let (r, vaapi_calls) = routes(inv, Ok("NVDEC"), Ok("VA-API"));
        assert_eq!(r.decoder(), "nvdec");
        assert_eq!(vaapi_calls, 0, "VA-API must not be probed after NVDEC succeeds");
        assert_eq!(chain(&r.decode_chain), vec!["nvdec", "software"]);
        assert_eq!(r.encoder(), "nvenc");
        assert_eq!(chain(&r.encode_chain), vec!["nvenc", "software"]);
    }

    #[test]
    fn nvdec_failing_moves_to_vaapi_then_to_the_software_decoders() {
        let hybrid = Inventory::new([gpu("renderd128", Vendor::Intel, "i915"), gpu("renderd129", Vendor::Nvidia, "nvidia")]);
        let (r, _) = routes(hybrid.clone(), Err("no libnvcuvid"), Ok("Intel iHD"));
        assert_eq!((r.decoder(), r.decode.device.as_str()), ("vaapi", "gpu:renderd128"));
        assert_eq!(r.decode.declined[0].reason, Reason::Declined { why: "no libnvcuvid".into() });

        let (r, _) = routes(hybrid, Err("no libnvcuvid"), Err("no libva"));
        assert_eq!((r.decoder(), chain(&r.decode_chain)), ("software", vec!["software"]));
    }

    #[test]
    fn amd_and_intel_decode_on_vaapi_and_encode_on_the_cpu() {
        for (vendor, driver) in [(Vendor::Amd, "amdgpu"), (Vendor::Intel, "xe"), (Vendor::Nvidia, "nouveau")] {
            let (r, calls) = routes(Inventory::new([gpu("renderd128", vendor, driver)]), Ok("NVDEC"), Ok("VA-API"));
            assert_eq!(r.decoder(), "vaapi", "{driver}");
            assert_eq!(calls, 1);
            assert_eq!(r.decode.declined[0].reason, Reason::NoDevice, "{driver}: NVDEC needs the proprietary driver");
            assert_eq!(r.encoder(), "software", "{driver}: no vendor-neutral hardware encoder yet");
        }
    }

    #[test]
    fn a_machine_without_gpus_uses_the_software_codecs() {
        let (r, calls) = routes(Inventory::default(), Ok("NVDEC"), Ok("VA-API"));
        assert_eq!((r.decoder(), r.encoder(), calls), ("software", "software", 0));
    }

    #[test]
    fn the_user_table_reorders_the_decoders() {
        let user = Table::parse("[compute.default]\ncodec_decode = [\"vaapi\", \"nvdec\", \"software\"]\n").unwrap();
        let inv = Inventory::new([gpu("renderd128", Vendor::Intel, "i915"), gpu("renderd129", Vendor::Nvidia, "nvidia")]);
        let r = route(Table::builtin().unwrap().merged(user), inv, probes(Ok("NVDEC"), Ok("VA-API"), Arc::new(AtomicUsize::new(0)))).unwrap();
        assert_eq!(r.decoder(), "vaapi");
    }

    #[test]
    fn the_real_machine_routes_without_panicking() {
        // whatever this machine has, there is a decision and the software path closes both chains
        if let Some(r) = current() {
            assert_eq!(r.decode_chain.routes.last().map(|x| x.backend.as_str()), Some("software"));
            assert_eq!(r.encode_chain.routes.last().map(|x| x.backend.as_str()), Some("software"));
        }
        let _ = nvenc_allowed();
    }
}
