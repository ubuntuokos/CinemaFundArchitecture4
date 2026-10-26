//! Choosing a route: the first backend-on-device in the table's order that can take the task.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::Serialize;

use crate::catalog::{self, Maturity};
use crate::{BackendId, Device, DeviceId, DeviceKind, Error, Family, Inventory, Preference, Preferences, Table, Task, TaskId, Vendor};

/// Longest reason text kept from a backend (a backend's message cannot grow the report unbounded).
const MAX_REASON_LEN: usize = 256;

/// A way of doing a family of tasks: NVDEC, VA-API, Burn on Vulkan, candle on the CPU...
///
/// Implemented by the crate that owns the backend. `accepts` must be cheap (no session is opened):
/// it answers from capabilities found at start-up (codec profiles, model size, driver version).
pub trait Backend: Send + Sync {
    fn id(&self) -> &BackendId;

    /// The devices of this machine the backend can run on, best first. A CPU backend returns the
    /// CPU; a vendor backend returns only that vendor's devices.
    fn devices<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Device>;

    /// Whether the backend takes this task on this device. `Err` carries the reason, in English,
    /// for the UI and `perf.stats` ("HEVC 4:2:2 is not decoded by this GPU").
    fn accepts(&self, task: &Task, device: &Device) -> Result<(), String>;

    /// Whether the backend does this family's work. One id can name backends of two families
    /// (`vaapi` decodes, and will encode); a backend registered for one is not taken for the
    /// other, and the router reports it there as not registered (or not implemented, when the
    /// catalogue lists it as planned). All families by default.
    fn serves(&self, family: Family) -> bool {
        let _ = family;
        true
    }
}

/// Why a route was passed over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// The catalogue lists the backend as planned: not written yet.
    NotImplemented,
    /// The table names a backend this build did not register (feature off, other crate absent).
    NotRegistered,
    /// The backend runs on one vendor's hardware only and vendor-locked backends are switched off
    /// ([`Preferences::allow_vendor_locked`]).
    VendorLocked { vendor: Vendor },
    /// Switched off in the settings (for example Settings ▸ Playback ▸ Hardware decoding Off).
    Disabled,
    /// None of the backend's devices are on this machine.
    NoDevice,
    /// The user's preference for the family excludes the device.
    NotPreferred,
    /// The backend does not take the task on the device.
    Declined { why: String },
    /// The device has less free memory than the task's row asks for.
    NotEnoughMemory { need_mb: u64, free_mb: u64 },
    /// The route was tried and failed while running; [`Router::next_after`] moved past it.
    Failed,
}

/// One route passed over, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Declined {
    pub backend: BackendId,
    /// `None` when the reason is about the backend as a whole.
    pub device: Option<DeviceId>,
    pub reason: Reason,
}

/// The route chosen for a task, with everything passed over before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub task: TaskId,
    pub backend: BackendId,
    pub device: DeviceId,
    pub declined: Vec<Declined>,
}

/// One backend on one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    pub backend: BackendId,
    pub device: DeviceId,
}

/// Every route that could take a task, in the order they would be tried: the first is what
/// [`Router::choose`] picks, the rest are what [`Router::next_after`] moves to. `perf.stats`
/// reports it, and its last route runs on the CPU wherever the table has a CPU path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chain {
    pub task: TaskId,
    pub routes: Vec<Route>,
    pub declined: Vec<Declined>,
}

impl Chain {
    /// Whether the chain ends on the CPU (CFA4's rule for every family).
    pub fn ends_on_cpu(&self, inventory: &Inventory) -> bool {
        self.routes.last().and_then(|r| inventory.get(&r.device)).is_some_and(|d| d.kind == DeviceKind::Cpu)
    }
}

impl Decision {
    /// The `perf.stats` / control-protocol key this decision is reported under.
    pub fn stats_key(&self) -> String {
        format!("route.{}", self.task)
    }
}

/// Chooses routes. Built once at start-up, after the device probe and backend registration;
/// a settings change builds a new one (it is cheap).
pub struct Router {
    table: Table,
    inventory: Inventory,
    preferences: Preferences,
    backends: Vec<Arc<dyn Backend>>,
    disabled: BTreeSet<BackendId>,
}

fn clip(mut s: String) -> String {
    if s.len() > MAX_REASON_LEN {
        let mut end = MAX_REASON_LEN;
        while !s.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        s.truncate(end);
    }
    s
}

impl Router {
    pub fn new(table: Table, inventory: Inventory, preferences: Preferences) -> Router {
        Router { table, inventory, preferences, backends: Vec::new(), disabled: BTreeSet::new() }
    }

    /// Registers a backend. A second backend with the same id replaces the first.
    pub fn register(&mut self, backend: Arc<dyn Backend>) {
        self.backends.retain(|b| b.id() != backend.id());
        self.backends.push(backend);
    }

    /// Switches a backend off (or back on) without touching the table.
    pub fn set_enabled(&mut self, id: &BackendId, enabled: bool) {
        if enabled {
            self.disabled.remove(id);
        } else {
            self.disabled.insert(id.clone());
        }
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    fn backend(&self, id: &BackendId) -> Option<&Arc<dyn Backend>> {
        self.backends.iter().find(|b| b.id() == id)
    }

    fn allowed(pref: &Preference, device: &Device) -> bool {
        match pref {
            Preference::Automatic => true,
            Preference::CpuOnly => device.kind == DeviceKind::Cpu,
            Preference::Device(id) => &device.id == id || device.kind == DeviceKind::Cpu,
        }
    }

    /// The route for a task: the first one in the table's order that can take it.
    pub fn choose(&self, task: &Task) -> Result<Decision, Error> {
        self.pick(task, None)
    }

    /// The route after `failed` for a task whose run on `failed` went wrong (a decode error,
    /// a lost device). Only for work that can be restarted: a half-written encoded stream cannot
    /// move to another encoder, and its failure is an error for the caller.
    pub fn next_after(&self, task: &Task, failed: &Decision) -> Result<Decision, Error> {
        self.pick(task, Some((&failed.backend, &failed.device)))
    }

    /// Every route that could take `task`, in order, with every route passed over.
    pub fn chain(&self, task: &Task) -> Chain {
        let mut routes = Vec::new();
        let mut declined = Vec::new();
        self.walk(task, None, &mut declined, |backend, device| {
            routes.push(Route { backend: backend.clone(), device: device.clone() });
            false
        });
        Chain { task: task.id.clone(), routes, declined }
    }

    fn pick(&self, task: &Task, failed: Option<(&BackendId, &DeviceId)>) -> Result<Decision, Error> {
        let mut declined = Vec::new();
        let mut chosen = None;
        self.walk(task, failed, &mut declined, |backend, device| {
            chosen = Some((backend.clone(), device.clone()));
            true
        });
        match chosen {
            Some((backend, device)) => Ok(Decision { task: task.id.clone(), backend, device, declined }),
            None => Err(Error::NoRoute { task: task.id.clone(), declined }),
        }
    }

    /// Why a backend in the order cannot be used at all, before its devices are looked at.
    fn backend_reason(&self, family: Family, id: &BackendId) -> Option<Reason> {
        let entry = catalog::lookup(family, id);
        if !self.backend(id).is_some_and(|b| b.serves(family)) {
            return Some(if entry.is_some_and(|e| e.maturity == Maturity::Planned) { Reason::NotImplemented } else { Reason::NotRegistered });
        }
        if self.disabled.contains(id) {
            return Some(Reason::Disabled);
        }
        match entry.and_then(|e| e.vendor_lock) {
            Some(vendor) if !self.preferences.allow_vendor_locked => Some(Reason::VendorLocked { vendor }),
            _ => None,
        }
    }

    /// Walks the order: every usable route goes to `found` (which returns true to stop), every
    /// route passed over to `declined`.
    fn walk(&self, task: &Task, failed: Option<(&BackendId, &DeviceId)>, declined: &mut Vec<Declined>, mut found: impl FnMut(&BackendId, &DeviceId) -> bool) {
        let order = self.table.order(&task.id, task.family);
        let need = self.table.rule(&task.id).and_then(|r| r.min_free_vram_mb);
        let pref = self.preferences.get(task.family);

        // Routes up to and including the failed one are skipped: they were passed over (or never
        // reached) on the first choice, and the caller already has that report. A failed route that
        // is no longer in the order (the table, the settings or the devices changed since) is simply
        // left out, and the choice starts from the top.
        let failed_in_order = failed.is_some_and(|(fb, fd)| {
            order.contains(fb)
                && self.backend_reason(task.family, fb).is_none()
                && self.backend(fb).is_some_and(|b| b.devices(&self.inventory).iter().any(|d| &d.id == fd))
        });
        if let Some((fb, fd)) = failed {
            declined.push(Declined { backend: fb.clone(), device: Some(fd.clone()), reason: Reason::Failed });
        }
        let mut past_failed = !failed_in_order;

        for backend_id in order {
            if let Some(reason) = self.backend_reason(task.family, backend_id) {
                if past_failed {
                    declined.push(Declined { backend: backend_id.clone(), device: None, reason });
                }
                continue;
            }
            let Some(backend) = self.backend(backend_id) else {
                continue;
            };
            let devices = backend.devices(&self.inventory);
            if devices.is_empty() && past_failed {
                declined.push(Declined { backend: backend_id.clone(), device: None, reason: Reason::NoDevice });
            }
            for device in devices {
                if failed == Some((backend_id, &device.id)) {
                    past_failed = true;
                    continue;
                }
                if !past_failed {
                    continue;
                }
                let mut decline = |reason| declined.push(Declined { backend: backend_id.clone(), device: Some(device.id.clone()), reason });
                if !Self::allowed(pref, device) {
                    decline(Reason::NotPreferred);
                    continue;
                }
                if let Err(why) = backend.accepts(task, device) {
                    decline(Reason::Declined { why: clip(why) });
                    continue;
                }
                if device.kind != DeviceKind::Cpu
                    && let (Some(need_mb), Some(free_mb)) = (need, device.free_memory_mb)
                    && free_mb < need_mb
                {
                    decline(Reason::NotEnoughMemory { need_mb, free_mb });
                    continue;
                }
                if found(backend_id, &device.id) {
                    return;
                }
            }
        }
    }

    /// The first CPU route in the task's order that takes it, without asking any GPU or NPU route
    /// (their `accepts` may be a slow driver probe): the last resort of every family.
    pub fn cpu_route(&self, task: &Task) -> Option<Route> {
        let pref = self.preferences.get(task.family);
        self.table.order(&task.id, task.family).iter().filter(|id| self.backend_reason(task.family, id).is_none()).find_map(|id| {
            let backend = self.backend(id)?;
            backend
                .devices(&self.inventory)
                .into_iter()
                .find(|d| d.kind == DeviceKind::Cpu && Self::allowed(pref, d) && backend.accepts(task, d).is_ok())
                .map(|d| Route { backend: id.clone(), device: d.id.clone() })
        })
    }

    /// What runs for a task once `decision` is made: the chosen route, then the CPU route (when the
    /// choice is not already on the CPU). The routes between them are not asked, so a slow or
    /// blocking driver probe is never run after a working one. This is the chain `perf.stats`
    /// reports for the hardware decoder and encoder.
    pub fn fallback_chain(&self, task: &Task, decision: &Decision) -> Chain {
        let mut routes = vec![Route { backend: decision.backend.clone(), device: decision.device.clone() }];
        if let Some(cpu) = self.cpu_route(task)
            && routes.first() != Some(&cpu)
        {
            routes.push(cpu);
        }
        Chain { task: task.id.clone(), routes, declined: decision.declined.clone() }
    }

    /// Families whose order has no registered, enabled backend that runs on the CPU. CFA4's rule
    /// is that this list is empty (every feature works without a GPU); the engine logs it at
    /// start-up and a test of the default registration checks it.
    pub fn families_without_cpu_route(&self) -> Vec<Family> {
        Family::ALL
            .into_iter()
            .filter(|&family| {
                !self.table.family_order(family).iter().any(|id| {
                    self.backend_reason(family, id).is_none()
                        && self.backend(id).is_some_and(|b| b.devices(&self.inventory).iter().any(|d| d.kind == DeviceKind::Cpu))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vendor;

    /// A stand-in backend: runs on the devices of the given kinds and vendors, refuses tasks whose
    /// id is in `refuse`.
    struct Fake {
        id: BackendId,
        kinds: Vec<DeviceKind>,
        vendors: Vec<Vendor>,
        refuse: Vec<&'static str>,
    }

    impl Fake {
        fn cpu(id: &str) -> Arc<dyn Backend> {
            Arc::new(Fake { id: BackendId::new(id).unwrap(), kinds: vec![DeviceKind::Cpu], vendors: vec![], refuse: vec![] })
        }
        fn gpu(id: &str, vendors: &[Vendor]) -> Fake {
            Fake { id: BackendId::new(id).unwrap(), kinds: vec![DeviceKind::Gpu], vendors: vendors.to_vec(), refuse: vec![] }
        }
        fn npu(id: &str, vendors: &[Vendor]) -> Arc<dyn Backend> {
            Arc::new(Fake { id: BackendId::new(id).unwrap(), kinds: vec![DeviceKind::Npu], vendors: vendors.to_vec(), refuse: vec![] })
        }
    }

    impl Backend for Fake {
        fn id(&self) -> &BackendId {
            &self.id
        }
        fn devices<'a>(&self, inv: &'a Inventory) -> Vec<&'a Device> {
            inv.devices().iter().filter(|d| self.kinds.contains(&d.kind) && (d.kind == DeviceKind::Cpu || self.vendors.contains(&d.vendor))).collect()
        }
        fn accepts(&self, task: &Task, _device: &Device) -> Result<(), String> {
            if self.refuse.contains(&task.id.as_str()) { Err(format!("{} not supported", task.id)) } else { Ok(()) }
        }
    }

    fn dev(id: &str, kind: DeviceKind, vendor: Vendor, free: Option<u64>) -> Device {
        Device { id: DeviceId::new(id).unwrap(), kind, vendor, name: id.into(), driver: None, pci_device: None, free_memory_mb: free }
    }

    /// The reference machine: an NVIDIA RTX A1000 with 8 GB.
    fn a1000(free: Option<u64>) -> Inventory {
        Inventory::new([dev("gpu:renderd128", DeviceKind::Gpu, Vendor::Nvidia, free)])
    }

    /// A laptop with an Intel iGPU and an Intel NPU.
    fn intel_laptop() -> Inventory {
        Inventory::new([dev("gpu:renderd128", DeviceKind::Gpu, Vendor::Intel, None), dev("npu:accel0", DeviceKind::Npu, Vendor::Intel, None)])
    }

    fn all_backends(r: &mut Router) {
        r.register(Arc::new(Fake::gpu("nvdec", &[Vendor::Nvidia])));
        r.register(Arc::new(Fake::gpu("vaapi", &[Vendor::Intel, Vendor::Amd])));
        r.register(Fake::cpu("software"));
        r.register(Arc::new(Fake::gpu("nvenc", &[Vendor::Nvidia])));
        r.register(Arc::new(Fake::gpu("wgpu", &[Vendor::Nvidia, Vendor::Amd, Vendor::Intel, Vendor::Other])));
        r.register(Fake::cpu("cpu-compositor"));
        r.register(Fake::npu("openvino-npu", &[Vendor::Intel]));
        r.register(Arc::new(Fake::gpu("burn-cuda", &[Vendor::Nvidia])));
        r.register(Arc::new(Fake::gpu("burn-vulkan", &[Vendor::Nvidia, Vendor::Amd, Vendor::Intel])));
        r.register(Fake::cpu("candle-cpu"));
        r.register(Arc::new(Fake::gpu("wgpu-compute", &[Vendor::Nvidia, Vendor::Amd, Vendor::Intel])));
        r.register(Fake::cpu("cpu"));
    }

    fn router(inv: Inventory, prefs: Preferences) -> Router {
        let mut r = Router::new(Table::builtin().unwrap(), inv, prefs);
        all_backends(&mut r);
        r
    }

    fn task(id: &str, f: Family) -> Task {
        Task::new(id, f).unwrap()
    }

    #[test]
    fn the_reference_machine_decodes_on_nvdec() {
        let r = router(a1000(None), Preferences::default());
        let d = r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap();
        assert_eq!((d.backend.as_str(), d.device.as_str()), ("nvdec", "gpu:renderd128"));
        assert!(d.declined.is_empty());
        assert_eq!(d.stats_key(), "route.video.decode.hevc");
    }

    #[test]
    fn an_intel_machine_takes_the_vendor_neutral_path_and_says_why() {
        let r = router(intel_laptop(), Preferences::default());
        let d = r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap();
        assert_eq!(d.backend.as_str(), "vaapi");
        assert_eq!(d.declined, vec![Declined { backend: BackendId::new("nvdec").unwrap(), device: None, reason: Reason::NoDevice }]);
    }

    #[test]
    fn ai_goes_to_the_npu_where_there_is_one() {
        let r = router(intel_laptop(), Preferences::default());
        let d = r.choose(&task("ai.upscale", Family::Ai)).unwrap();
        assert_eq!((d.backend.as_str(), d.device.as_str()), ("openvino-npu", "npu:accel0"));

        // The reference machine has no NPU: CUDA first, and the OpenVINO NPU route is reported.
        let d = router(a1000(None), Preferences::default()).choose(&task("ai.upscale", Family::Ai)).unwrap();
        assert_eq!(d.backend.as_str(), "burn-cuda");
        assert_eq!(d.declined[0].reason, Reason::NoDevice);
    }

    #[test]
    fn whisper_leaves_the_gpu_when_memory_is_short() {
        let whisper = task("ai.speech.whisper", Family::Ai);
        let r = router(a1000(Some(6000)), Preferences::default());
        assert_eq!(r.choose(&whisper).unwrap().backend.as_str(), "burn-cuda");

        let r = router(a1000(Some(1500)), Preferences::default());
        let d = r.choose(&whisper).unwrap();
        assert_eq!((d.backend.as_str(), d.device.as_str()), ("candle-cpu", "cpu"));
        let reasons: Vec<_> = d.declined.iter().map(|x| &x.reason).collect();
        assert_eq!(reasons, vec![&Reason::NotEnoughMemory { need_mb: 2048, free_mb: 1500 }, &Reason::NotEnoughMemory { need_mb: 2048, free_mb: 1500 }]);

        // Unknown free memory: the budget is not applied.
        let r = router(a1000(None), Preferences::default());
        assert_eq!(r.choose(&whisper).unwrap().backend.as_str(), "burn-cuda");
    }

    #[test]
    fn hardware_encoding_is_opt_in() {
        let enc = task("video.encode.h264", Family::CodecEncode);
        let r = router(a1000(None), Preferences::default());
        let d = r.choose(&enc).unwrap();
        assert_eq!(d.backend.as_str(), "software", "byte-identical output by default");
        assert_eq!(d.declined[0].reason, Reason::NotPreferred);

        let mut prefs = Preferences::default();
        prefs.set(Family::CodecEncode, Preference::Automatic);
        assert_eq!(router(a1000(None), prefs).choose(&enc).unwrap().backend.as_str(), "nvenc");
    }

    #[test]
    fn one_device_preference_still_falls_back_to_the_cpu() {
        let mut prefs = Preferences::default();
        prefs.set(Family::Ai, Preference::Device(DeviceId::new("gpu:renderd128").unwrap()));
        let r = router(intel_laptop(), prefs.clone());
        let d = r.choose(&task("ai.upscale", Family::Ai)).unwrap();
        assert_eq!((d.backend.as_str(), d.device.as_str()), ("burn-vulkan", "gpu:renderd128"), "the NPU is not the chosen device");

        prefs.set(Family::Ai, Preference::Device(DeviceId::new("gpu:gone").unwrap()));
        let d = router(intel_laptop(), prefs).choose(&task("ai.upscale", Family::Ai)).unwrap();
        assert_eq!(d.device.as_str(), "cpu", "a device that is gone leaves the CPU");
    }

    #[test]
    fn disabled_unregistered_and_declining_backends_are_reported() {
        let mut r = Router::new(Table::builtin().unwrap(), a1000(None), Preferences::default());
        let mut nvdec = Fake::gpu("nvdec", &[Vendor::Nvidia]);
        nvdec.refuse = vec!["video.decode.hevc422"];
        r.register(Arc::new(nvdec));
        r.register(Fake::cpu("software"));
        r.set_enabled(&BackendId::new("nvdec").unwrap(), false);

        let d = r.choose(&task("video.decode.h264", Family::CodecDecode)).unwrap();
        assert_eq!(d.backend.as_str(), "software");
        let reasons: Vec<_> = d.declined.iter().map(|x| (x.backend.as_str(), x.reason.clone())).collect();
        assert_eq!(
            reasons,
            vec![
                ("nvdec", Reason::Disabled),
                ("vaapi", Reason::NotRegistered),
                ("ffmpeg", Reason::NotImplemented),
                ("gstreamer-vulkan-video", Reason::NotImplemented)
            ]
        );

        r.set_enabled(&BackendId::new("nvdec").unwrap(), true);
        let d = r.choose(&task("video.decode.hevc422", Family::CodecDecode)).unwrap();
        assert_eq!(d.backend.as_str(), "software");
        assert_eq!(d.declined[0].reason, Reason::Declined { why: "video.decode.hevc422 not supported".into() });
    }

    #[test]
    fn a_failed_route_moves_to_the_next_one() {
        let r = router(a1000(None), Preferences::default());
        let t = task("video.decode.hevc", Family::CodecDecode);
        let first = r.choose(&t).unwrap();
        let next = r.next_after(&t, &first).unwrap();
        assert_eq!(next.backend.as_str(), "software");
        assert_eq!(next.declined[0].reason, Reason::Failed);
        // The software path failing too leaves nothing.
        assert!(matches!(r.next_after(&t, &next), Err(Error::NoRoute { .. })));
    }

    #[test]
    fn a_failed_route_no_longer_in_the_order_is_left_out() {
        let r = router(a1000(None), Preferences::default());
        let t = task("video.decode.hevc", Family::CodecDecode);
        let stale = Decision { task: t.id.clone(), backend: BackendId::new("nvdec").unwrap(), device: DeviceId::new("gpu:old").unwrap(), declined: vec![] };
        // Not in the order (the device is gone): a fresh choice.
        assert_eq!(r.next_after(&t, &stale).unwrap().backend.as_str(), "nvdec");
    }

    #[test]
    fn no_route_lists_every_reason() {
        let r = Router::new(Table::builtin().unwrap(), Inventory::default(), Preferences::default());
        match r.choose(&task("ai.upscale", Family::Ai)) {
            Err(Error::NoRoute { declined, .. }) => assert_eq!(declined.len(), 4),
            other => panic!("{other:?}"),
        }
        let empty = Router::new(Table::default(), Inventory::default(), Preferences::default());
        assert!(matches!(empty.choose(&task("x", Family::Display)), Err(Error::NoRoute { .. })));
    }

    #[test]
    fn every_family_has_a_cpu_route_with_the_default_registration() {
        assert!(router(Inventory::default(), Preferences::default()).families_without_cpu_route().is_empty());
        let r = Router::new(Table::builtin().unwrap(), a1000(None), Preferences::default());
        assert_eq!(r.families_without_cpu_route(), Family::ALL.to_vec());
    }

    #[test]
    fn a_backend_registered_twice_is_replaced_and_long_reasons_are_clipped() {
        let mut r = Router::new(Table::builtin().unwrap(), a1000(None), Preferences::default());
        struct Chatty(BackendId);
        impl Backend for Chatty {
            fn id(&self) -> &BackendId {
                &self.0
            }
            fn devices<'a>(&self, inv: &'a Inventory) -> Vec<&'a Device> {
                inv.devices().iter().filter(|d| d.kind == DeviceKind::Gpu).collect()
            }
            fn accepts(&self, _: &Task, _: &Device) -> Result<(), String> {
                Err("é".repeat(1000))
            }
        }
        r.register(Arc::new(Fake::gpu("nvdec", &[Vendor::Nvidia])));
        r.register(Arc::new(Chatty(BackendId::new("nvdec").unwrap())));
        r.register(Fake::cpu("software"));
        let d = r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap();
        assert_eq!(d.backend.as_str(), "software", "the second registration replaced the first");
        let Reason::Declined { why } = &d.declined[0].reason else { panic!("{:?}", d.declined) };
        assert!(why.len() <= MAX_REASON_LEN && !why.is_empty());
    }

    #[test]
    fn vendor_locked_backends_can_be_switched_off_and_everything_still_runs() {
        let prefs = Preferences { allow_vendor_locked: false, ..Preferences::default() };
        let r = router(a1000(None), prefs.clone());
        let d = r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap();
        assert_eq!((d.backend.as_str(), d.device.as_str()), ("software", "cpu"));
        assert_eq!(
            d.declined[0],
            Declined { backend: BackendId::new("nvdec").unwrap(), device: None, reason: Reason::VendorLocked { vendor: Vendor::Nvidia } }
        );
        assert!(r.families_without_cpu_route().is_empty(), "the CPU paths are never vendor-locked");

        // a hybrid laptop decodes on its Intel GPU instead
        let hybrid =
            Inventory::new([dev("gpu:renderd128", DeviceKind::Gpu, Vendor::Intel, None), dev("gpu:renderd129", DeviceKind::Gpu, Vendor::Nvidia, None)]);
        let r = router(hybrid.clone(), prefs);
        assert_eq!(r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap().backend.as_str(), "vaapi");
        let r = router(hybrid, Preferences::default());
        let chain = r.chain(&task("video.decode.hevc", Family::CodecDecode));
        let routes: Vec<_> = chain.routes.iter().map(|x| (x.backend.as_str(), x.device.as_str())).collect();
        assert_eq!(routes, vec![("nvdec", "gpu:renderd129"), ("vaapi", "gpu:renderd128"), ("software", "cpu")]);
    }

    #[test]
    fn chains_list_every_usable_route_and_end_on_the_cpu() {
        let machines = [Inventory::default(), a1000(None), a1000(Some(100)), intel_laptop()];
        for inv in machines {
            for allow_vendor_locked in [true, false] {
                for pref in [Preference::Automatic, Preference::CpuOnly, Preference::Device(DeviceId::new("gpu:renderd128").unwrap())] {
                    let mut prefs = Preferences { allow_vendor_locked, ..Preferences::default() };
                    for f in Family::ALL {
                        prefs.set(f, pref.clone());
                    }
                    let r = router(inv.clone(), prefs);
                    for f in Family::ALL {
                        let t = task(&format!("x.{}", f.key()), f);
                        let chain = r.chain(&t);
                        let ctx = format!("{inv:?} {pref:?} locked={allow_vendor_locked} {f:?}");
                        assert!(chain.ends_on_cpu(&inv), "{ctx}: {chain:?}");
                        let first = r.choose(&t).unwrap();
                        assert_eq!((&first.backend, &first.device), (&chain.routes[0].backend, &chain.routes[0].device), "{ctx}");
                        for route in &chain.routes {
                            let entry = crate::catalog::lookup(f, &route.backend).unwrap();
                            assert!(allow_vendor_locked || entry.vendor_lock.is_none(), "{ctx}: {route:?}");
                        }
                        if pref == Preference::CpuOnly {
                            assert!(chain.routes.iter().all(|x| x.device.as_str() == "cpu"), "{ctx}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_fallback_chain_is_the_choice_then_the_cpu_without_asking_the_routes_between() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(BackendId, Arc<AtomicUsize>);
        impl Backend for Counting {
            fn id(&self) -> &BackendId {
                &self.0
            }
            fn devices<'a>(&self, inv: &'a Inventory) -> Vec<&'a Device> {
                inv.devices().iter().filter(|d| d.kind == DeviceKind::Gpu).collect()
            }
            fn accepts(&self, _: &Task, _: &Device) -> Result<(), String> {
                self.1.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
        let vaapi_asked = Arc::new(AtomicUsize::new(0));
        let mut r = Router::new(Table::builtin().unwrap(), a1000(None), Preferences::default());
        r.register(Arc::new(Fake::gpu("nvdec", &[Vendor::Nvidia])));
        r.register(Arc::new(Counting(BackendId::new("vaapi").unwrap(), vaapi_asked.clone())));
        r.register(Fake::cpu("software"));
        let t = task("video.decode", Family::CodecDecode);
        let d = r.choose(&t).unwrap();
        let chain = r.fallback_chain(&t, &d);
        let routes: Vec<_> = chain.routes.iter().map(|x| (x.backend.as_str(), x.device.as_str())).collect();
        assert_eq!(routes, vec![("nvdec", "gpu:renderd128"), ("software", "cpu")]);
        assert_eq!(vaapi_asked.load(Ordering::Relaxed), 0, "VA-API is never probed after NVDEC works");
        assert!(chain.ends_on_cpu(r.inventory()));

        // a choice already on the CPU is not repeated
        let r = router(Inventory::default(), Preferences::default());
        let d = r.choose(&t).unwrap();
        assert_eq!(r.fallback_chain(&t, &d).routes.len(), 1);
        assert_eq!(r.cpu_route(&t).map(|x| x.backend.as_str().to_string()), Some("software".into()));
    }

    #[test]
    fn a_backend_serves_only_its_families() {
        struct DecodeOnly(BackendId);
        impl Backend for DecodeOnly {
            fn id(&self) -> &BackendId {
                &self.0
            }
            fn devices<'a>(&self, inv: &'a Inventory) -> Vec<&'a Device> {
                inv.devices().iter().filter(|d| d.kind == DeviceKind::Gpu).collect()
            }
            fn accepts(&self, _: &Task, _: &Device) -> Result<(), String> {
                Ok(())
            }
            fn serves(&self, family: Family) -> bool {
                family == Family::CodecDecode
            }
        }
        let mut prefs = Preferences::default();
        prefs.set(Family::CodecEncode, Preference::Automatic);
        let mut r = Router::new(Table::builtin().unwrap(), intel_laptop(), prefs);
        r.register(Arc::new(DecodeOnly(BackendId::new("vaapi").unwrap())));
        r.register(Fake::cpu("software"));
        assert_eq!(r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap().backend.as_str(), "vaapi");
        let enc = r.choose(&task("video.encode.h264", Family::CodecEncode)).unwrap();
        assert_eq!(enc.backend.as_str(), "software");
        let vaapi = enc.declined.iter().find(|x| x.backend.as_str() == "vaapi").unwrap();
        assert_eq!(vaapi.reason, Reason::NotImplemented, "VA-API encoding is planned, not written");
    }

    #[test]
    fn decisions_serialise_for_perf_stats() {
        let r = router(intel_laptop(), Preferences::default());
        let d = r.choose(&task("video.decode.hevc", Family::CodecDecode)).unwrap();
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["backend"], "vaapi");
        assert_eq!(v["device"], "gpu:renderd128");
        assert_eq!(v["declined"][0]["reason"]["kind"], "no_device");
        assert!(v["declined"][0]["device"].is_null());
    }
}
