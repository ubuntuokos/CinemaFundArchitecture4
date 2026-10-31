//! Media capability registry (WP-01).
//!
//! One table answers every "can CFA4 do X with codec C in container K here?" question the Smart
//! Cut and Smart Render planners ask. Each [`Entry`] names its [`Backend`], a [`SupportLevel`] and
//! the evidence or reason behind it. The registry starts from [`Registry::native`], the state the
//! baseline audit documented, and is then refined by [`Probe`]s: run-time checks of things only the
//! running machine knows (a hardware encoder, an installed library).
//!
//! Decisions are reproducible: the registry is ordered data ([`BTreeMap`]), probes are applied in
//! a canonical order whatever order they arrive in, and when two probes disagree about the same
//! capability (and route) the unavailable one wins; separate concrete routes of one backend (`nvenc`,
//! `vaapi`) keep separate rows. The same entries and probes therefore always give the same
//! [`Decision`]s and the same [`Report`].
//!
//! A capability with no entry is unsupported. Only [`SupportLevel::Supported`] allows an
//! operation; [`SupportLevel::Experimental`] does so only when the [`Policy`] says so. Nothing is
//! allowed by default that has not been shown to work, so an unproven cut never counts as lossless.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Version of the registry data model ([`Report::registry_version`]).
pub const REGISTRY_VERSION: u32 = 1;

/// Coding formats the Smart Edit planners know about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codec {
    /// MPEG-2 video (ITU-T H.262 | ISO/IEC 13818-2).
    Mpeg2Video,
    /// H.264 / AVC (ITU-T H.264).
    H264,
    /// H.265 / HEVC (ITU-T H.265).
    Hevc,
    /// AV1 (AV1 Bitstream & Decoding Process Specification).
    Av1,
    /// AAC (ISO/IEC 14496-3).
    Aac,
    /// Uncompressed PCM audio.
    Pcm,
}

impl Codec {
    pub const ALL: [Codec; 6] = [Codec::Mpeg2Video, Codec::H264, Codec::Hevc, Codec::Av1, Codec::Aac, Codec::Pcm];
    pub const VIDEO: [Codec; 4] = [Codec::Mpeg2Video, Codec::H264, Codec::Hevc, Codec::Av1];

    pub fn label(self) -> &'static str {
        match self {
            Codec::Mpeg2Video => "MPEG-2 video",
            Codec::H264 => "H.264/AVC",
            Codec::Hevc => "HEVC/H.265",
            Codec::Av1 => "AV1",
            Codec::Aac => "AAC",
            Codec::Pcm => "PCM",
        }
    }

    pub fn is_video(self) -> bool {
        Codec::VIDEO.contains(&self)
    }

    /// The codec's part of a `cfa4-route` task id (`video.encode.h264`).
    pub fn task_name(self) -> &'static str {
        match self {
            Codec::Mpeg2Video => "mpeg2",
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
            Codec::Av1 => "av1",
            Codec::Aac => "aac",
            Codec::Pcm => "pcm",
        }
    }
}

/// Container formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Container {
    /// ISO BMFF / MPEG-4 Part 14.
    Mp4,
    /// QuickTime.
    Mov,
    Matroska,
    WebM,
    /// MPEG-2 Transport Stream (ISO/IEC 13818-1).
    MpegTs,
    /// MPEG-2 Program Stream (ISO/IEC 13818-1).
    MpegPs,
    /// SMPTE ST 377-1.
    Mxf,
}

impl Container {
    pub const ALL: [Container; 7] =
        [Container::Mp4, Container::Mov, Container::Matroska, Container::WebM, Container::MpegTs, Container::MpegPs, Container::Mxf];

    pub fn label(self) -> &'static str {
        match self {
            Container::Mp4 => "MP4",
            Container::Mov => "QuickTime",
            Container::Matroska => "Matroska",
            Container::WebM => "WebM",
            Container::MpegTs => "MPEG-TS",
            Container::MpegPs => "MPEG-PS",
            Container::Mxf => "MXF",
        }
    }
}

/// What is done with a codec (and, for container operations, in which container).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    /// Read the codec's packets from the container.
    Demux,
    /// Decode the codec to pictures or samples.
    Decode,
    /// Parse the codec's parameter sets and picture headers without decoding.
    ParseHeaders,
    /// Build a frame-level index: access points, picture types, reference dependencies (WP-02).
    GopIndex,
    /// Copy compressed packets unchanged into the output container (WP-04).
    PacketCopy,
    /// Encode pictures or samples to the codec.
    Encode,
    /// Write the codec's packets into the container.
    Mux,
    /// Re-encode boundary segments and join them with copied packets (WP-06).
    BoundaryReencode,
}

impl Operation {
    /// Whether the operation is about a codec inside a container (the key needs one).
    pub fn needs_container(self) -> bool {
        matches!(self, Operation::Demux | Operation::PacketCopy | Operation::Mux | Operation::BoundaryReencode)
    }
}

/// Who implements a capability. Which one [`Registry::decide`] tries first is not fixed here: it
/// comes from the `cfa4-route` routing table ([`BackendOrder`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    /// CFA4's own clean-room Rust crates.
    Native,
    /// Hardware through `cfa4-platform` (NVENC, VA-API…).
    Platform,
    /// The optional GStreamer backend (ADR 0002).
    Gstreamer,
    /// The FFmpeg ≥ 9.0.2 backend of ADR 0005 (`cfa4-ffmpeg`, not written yet). Not registered:
    /// Smart Cut / Smart Render runs on CFA4's own code, ffmpeg is its validation oracle (owner
    /// decision 2026-10-27, smart-edit-baseline §3).
    Ffmpeg,
}

/// One capability: an operation on a codec, in a container when the operation needs one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Key {
    pub operation: Operation,
    pub codec: Codec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Container>,
}

impl Key {
    /// A codec-only capability (decode, encode, header parsing, GOP index).
    pub fn codec(operation: Operation, codec: Codec) -> Key {
        Key { operation, codec, container: None }
    }

    /// A capability of a codec in a container (demux, mux, packet copy, boundary re-encode).
    pub fn in_container(operation: Operation, codec: Codec, container: Container) -> Key {
        Key { operation, codec, container: Some(container) }
    }

    /// The key is well formed: a container exactly when the operation needs one.
    pub fn is_valid(&self) -> bool {
        self.operation.needs_container() == self.container.is_some()
    }
}

/// How well a capability is supported. Ordered from best to worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupportLevel {
    /// Implemented and verified by automated tests or a recorded measurement.
    Supported,
    /// Implemented, not yet validated for Smart Edit use.
    Experimental,
    /// Depends on the machine: unknown until a [`Probe`] answers.
    NeedsProbe,
    /// Not available (not implemented, or a probe found it missing).
    Unsupported,
    /// The backend is not part of this build.
    NotRegistered,
}

/// Pixel layout limits of an encoder: a boundary re-encode may only join copied packets when the
/// source stream fits them (plan §8).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EncoderLimits {
    /// Profiles the encoder writes, by codec profile name (`"high"`, `"main"`…).
    pub profiles: Vec<String>,
    /// Chroma subsamplings, as `"4:2:0"`, `"4:2:2"`, `"4:4:4"`.
    pub chroma: Vec<String>,
    /// Bit depths per sample.
    pub bit_depths: Vec<u8>,
}

impl EncoderLimits {
    /// Whether a stream with these properties fits; the reasons it does not, otherwise.
    pub fn check(&self, stream: &StreamFormat) -> Result<(), Vec<String>> {
        let mut why = Vec::new();
        if !self.profiles.iter().any(|p| p == &stream.profile) {
            why.push(format!("profile {} is not one the encoder writes ({})", stream.profile, self.profiles.join(", ")));
        }
        if !self.chroma.iter().any(|c| c == &stream.chroma) {
            why.push(format!("chroma {} is not one the encoder writes ({})", stream.chroma, self.chroma.join(", ")));
        }
        if !self.bit_depths.contains(&stream.bit_depth) {
            let depths: Vec<String> = self.bit_depths.iter().map(|d| d.to_string()).collect();
            why.push(format!("{}-bit samples are not supported by the encoder ({})", stream.bit_depth, depths.join(", ")));
        }
        if why.is_empty() { Ok(()) } else { Err(why) }
    }
}

/// The properties of a source stream that decide whether an encoder can continue it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFormat {
    pub profile: String,
    pub chroma: String,
    pub bit_depth: u8,
}

/// One registry row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub key: Key,
    pub backend: Backend,
    pub level: SupportLevel,
    /// Why: the tests or measurements behind a supported entry, the missing piece behind an
    /// unsupported one.
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<EncoderLimits>,
    /// The concrete `cfa4-route` routes behind the entry (`nvenc`, `vaapi`…). Empty: any route of
    /// its backend. A routed operation only accepts the entry through a route its order lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,
}

/// The result of a run-time check.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum ProbeOutcome {
    Available { detail: String },
    Unavailable { reason: String },
}

/// A run-time check of one capability on one backend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    pub key: Key,
    pub backend: Backend,
    #[serde(flatten)]
    pub outcome: ProbeOutcome,
    /// Encoder limits the probe found (replaces the entry's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<EncoderLimits>,
    /// The concrete routes the probe checked (replaces the entry's). A hardware probe names its
    /// route (`nvenc`, `vaapi`…), so a routing order without it does not admit it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,
}

/// The order backends are tried in, taken from the `cfa4-route` routing table so the planner never
/// keeps a routing policy of its own (AGENTS.md, ADR 0004 / 0005).
///
/// Encoding and the boundary re-encode of a codec follow the table's order for the task
/// `video.encode.<codec>` (`h264`, `hevc`, `av1`, `mpeg2`…), decoding the one for `video.decode`,
/// looked up as the router does (the task's own row, else its family). These are the task ids the
/// export and decode routes use (`cfa4-platform`), so a user's override in `routing.toml` steers
/// the planner the same way. Route backends map onto [`Backend`]s:
/// `software` is [`Backend::Native`], `ffmpeg` is [`Backend::Ffmpeg`], `gstreamer-*` is
/// [`Backend::Gstreamer`] and every other (hardware) path is [`Backend::Platform`]; an entry or
/// probe that names its concrete routes ([`Entry::routes`]) is only admitted through one of them.
/// A route the order does not name is not a candidate at all, as in the router: an override such as
/// `order = ["software"]` must not let the registry promise a hardware encoder the export route
/// will never use. The other operations (demux, header parsing, GOP index, packet copy, mux) have
/// no routing task: CFA4's own code comes first there (VISION.md 2.1), then every other backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendOrder {
    /// `None` only when the built-in table failed to load: then CFA4's own code is the one
    /// candidate. A loaded table is followed as it is, an empty order included (no candidates).
    table: Option<cfa4_route::Table>,
}

const ALL_BACKENDS: [Backend; 4] = [Backend::Native, Backend::Platform, Backend::Gstreamer, Backend::Ffmpeg];

impl BackendOrder {
    /// The order a routing table gives (the shipped one, or with the user's file on top).
    pub fn from_route(table: &cfa4_route::Table) -> BackendOrder {
        BackendOrder { table: Some(table.clone()) }
    }

    /// The table's order for one task, with the router's own lookup: the task's row, else its
    /// family (`Table::order`).
    fn routed(&self, task: &str, family: cfa4_route::Family) -> Candidates {
        let Some(table) = &self.table else {
            return Candidates::Unrouted(vec![Backend::Native]);
        };
        let Ok(task) = cfa4_route::TaskId::new(task) else {
            return Candidates::Routed(Vec::new());
        };
        Candidates::Routed(table.order(&task, family).iter().map(|id| id.as_str().to_string()).collect())
    }

    /// The candidates for one capability.
    pub fn for_key(&self, key: Key) -> Candidates {
        match key.operation {
            Operation::Decode => self.routed(DECODE_TASK, cfa4_route::Family::CodecDecode),
            Operation::Encode | Operation::BoundaryReencode => self.routed(&format!("video.encode.{}", key.codec.task_name()), cfa4_route::Family::CodecEncode),
            _ => Candidates::Unrouted(ALL_BACKENDS.to_vec()),
        }
    }
}

/// Who may do one capability, in preference order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Candidates {
    /// Route ids from the routing table (`nvenc`, `vaapi`, `ffmpeg`, `software`…).
    Routed(Vec<String>),
    /// No routing task: these backends, in this order.
    Unrouted(Vec<Backend>),
}

/// The backend a `cfa4-route` route id belongs to.
pub fn backend_of_route(id: &str) -> Backend {
    match id {
        "software" => Backend::Native,
        "ffmpeg" => Backend::Ffmpeg,
        s if s.starts_with("gstreamer") => Backend::Gstreamer,
        _ => Backend::Platform,
    }
}

/// The task the hardware decoder registration routes for every codec (`cfa4-platform`).
const DECODE_TASK: &str = "video.decode";

impl Default for BackendOrder {
    /// The shipped routing table's order. The table is built in and tested in `cfa4-route`; if it
    /// ever failed to parse, CFA4's own code is the only candidate.
    fn default() -> Self {
        BackendOrder { table: cfa4_route::Table::builtin().ok() }
    }
}

/// What a decision may accept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// Accept [`SupportLevel::Experimental`] entries (developer setting; off for exports).
    pub allow_experimental: bool,
}

impl Policy {
    /// Whether an entry of this level allows the operation.
    pub fn accepts(&self, level: SupportLevel) -> bool {
        level == SupportLevel::Supported || (self.allow_experimental && level == SupportLevel::Experimental)
    }
}

/// One answer of the registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub key: Key,
    pub allowed: bool,
    /// The backend that would do it, when allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<Backend>,
    /// The best level any backend reached.
    pub level: SupportLevel,
    /// One line per backend considered, in [`BackendOrder`].
    pub reasons: Vec<String>,
}

/// A decision made of several capabilities (a Smart Cut needs demux, index, copy and mux).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositeDecision {
    pub allowed: bool,
    pub steps: Vec<Decision>,
    /// Extra conditions that failed (encoder limits…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,
}

impl CompositeDecision {
    fn new(steps: Vec<Decision>, conditions: Vec<String>) -> Self {
        let allowed = steps.iter().all(|d| d.allowed) && conditions.is_empty();
        CompositeDecision { allowed, steps, conditions }
    }

    /// The reasons of every step that was not allowed, then the failed conditions.
    pub fn blockers(&self) -> Vec<String> {
        let mut out: Vec<String> = self.steps.iter().filter(|d| !d.allowed).flat_map(|d| d.reasons.iter().cloned()).collect();
        out.extend(self.conditions.iter().cloned());
        out
    }
}

/// A rejected registry change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// The key has a container where the operation takes none, or lacks one it needs.
    InvalidKey(Key),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::InvalidKey(k) => write!(f, "invalid capability key: {:?} {} {:?}", k.operation, k.codec.label(), k.container),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Machine-readable dump of the registry (CI artefact, `smart.capabilities` later on).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub registry_version: u32,
    pub entries: Vec<Entry>,
    pub probes: Vec<Probe>,
}

/// A row's identity: one capability on one backend, for all of its routes (no route named) or
/// for the concrete routes named. Separate route sets of one backend (`nvenc`, `vaapi`) keep
/// separate outcomes and limits.
type EntryId = (Key, Backend, Vec<String>);

/// Routes as a row stores them: sorted, without repeats.
fn normalized(routes: &[String]) -> Vec<String> {
    let mut r = routes.to_vec();
    r.sort();
    r.dedup();
    r
}

/// The capability registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    /// Rows as inserted, before probes, by key, backend and their concrete routes (sorted; empty:
    /// the whole backend).
    base: BTreeMap<EntryId, Entry>,
    /// Every probe applied so far, in canonical order.
    probes: Vec<Probe>,
    /// `base` with `probes` applied: what decisions read.
    entries: BTreeMap<EntryId, Entry>,
    /// Which backend is tried first.
    order: BackendOrder,
}

impl Registry {
    /// An empty registry: everything unsupported.
    pub fn empty() -> Self {
        Self::default()
    }

    /// CFA4's built-in state, as documented by the WP-00 baseline audit
    /// (`docs/audit/smart-edit-baseline.md` §9). Hardware HEVC / AV1 encoding needs a probe.
    pub fn native() -> Self {
        let mut r = Registry::empty();
        for e in native_entries() {
            // the built-in table only holds valid keys (checked by a test)
            let _ = r.insert(e);
        }
        r
    }

    /// Use the backend order of another routing table (for example the shipped table with the
    /// user's `routing.toml` on top).
    pub fn set_backend_order(&mut self, order: BackendOrder) {
        self.order = order;
    }

    pub fn backend_order(&self) -> &BackendOrder {
        &self.order
    }

    /// The rows of `key` on `backend`: the whole-backend row first, then those of concrete routes.
    fn rows(&self, key: Key, backend: Backend) -> impl Iterator<Item = &Entry> {
        self.entries.range((key, backend, Vec::new())..).take_while(move |((k, b, _), _)| *k == key && *b == backend).map(|(_, e)| e)
    }

    /// The rows for `key`, in the backend order of its operation. Through a route id, the rows
    /// that name that route come before the whole-backend row, which is only taken when no row
    /// names the route.
    fn candidates(&self, key: Key) -> Vec<&Entry> {
        let mut out: Vec<&Entry> = Vec::new();
        match self.order.for_key(key) {
            Candidates::Routed(ids) => {
                for id in &ids {
                    let backend = backend_of_route(id);
                    let named: Vec<&Entry> = self.rows(key, backend).filter(|e| e.routes.iter().any(|r| r == id)).collect();
                    if named.is_empty() {
                        if let Some(e) = self.rows(key, backend).find(|e| e.routes.is_empty()) {
                            push_once(&mut out, e);
                        }
                    } else {
                        for e in named {
                            push_once(&mut out, e);
                        }
                    }
                }
            }
            Candidates::Unrouted(backends) => {
                for b in backends {
                    for e in self.rows(key, b) {
                        push_once(&mut out, e);
                    }
                }
            }
        }
        out
    }

    /// The row [`Registry::decide`] picks: the first candidate the policy accepts.
    fn chosen(&self, key: Key, policy: Policy) -> Option<&Entry> {
        self.candidates(key).into_iter().find(|e| policy.accepts(e.level))
    }

    /// Add or replace a row.
    pub fn insert(&mut self, entry: Entry) -> Result<(), RegistryError> {
        if !entry.key.is_valid() {
            return Err(RegistryError::InvalidKey(entry.key));
        }
        let mut entry = entry;
        entry.routes = normalized(&entry.routes);
        self.base.insert((entry.key, entry.backend, entry.routes.clone()), entry);
        self.rebuild();
        Ok(())
    }

    /// The row of `key` on `backend` for all of its routes, or else its first row for concrete
    /// routes.
    pub fn get(&self, key: Key, backend: Backend) -> Option<&Entry> {
        self.rows(key, backend).next()
    }

    /// The row of `key` that names `route`, or else the whole-backend row of the route's backend.
    pub fn get_route(&self, key: Key, route: &str) -> Option<&Entry> {
        let backend = backend_of_route(route);
        self.rows(key, backend).find(|e| e.routes.iter().any(|r| r == route)).or_else(|| self.rows(key, backend).find(|e| e.routes.is_empty()))
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// Apply run-time probes. All probes applied so far, from this call and earlier ones, take
    /// effect in a canonical order (by key, backend, then outcome with every unavailable outcome
    /// last), so an "unavailable" always overrides an "available" for the same capability and
    /// route whatever order or batches the probes came in; one for the whole backend covers all of
    /// its routes. The probes are kept for the [`Report`].
    pub fn apply_probes(&mut self, probes: impl IntoIterator<Item = Probe>) -> Result<(), RegistryError> {
        let probes: Vec<Probe> = probes.into_iter().collect();
        if let Some(p) = probes.iter().find(|p| !p.key.is_valid()) {
            return Err(RegistryError::InvalidKey(p.key));
        }
        self.probes.extend(probes);
        self.probes.sort_by(|a, b| probe_order(a).cmp(&probe_order(b)));
        self.probes.dedup();
        self.rebuild();
        Ok(())
    }

    /// Apply the probes (in canonical order) to the inserted rows. A probe that names routes sets
    /// the row of exactly those routes; one that names none sets every row of its backend (or the
    /// whole-backend row, when there is none yet).
    fn rebuild(&mut self) {
        let mut entries = self.base.clone();
        for p in &self.probes {
            let (level, evidence) = match &p.outcome {
                ProbeOutcome::Available { detail } => (SupportLevel::Supported, format!("run-time probe: {detail}")),
                ProbeOutcome::Unavailable { reason } => (SupportLevel::Unsupported, format!("run-time probe: {reason}")),
            };
            let generic = (p.key, p.backend, Vec::new());
            let targets: Vec<EntryId> = if p.routes.is_empty() {
                let existing: Vec<EntryId> =
                    entries.range(generic.clone()..).take_while(|((k, b, _), _)| *k == p.key && *b == p.backend).map(|(id, _)| id.clone()).collect();
                if existing.is_empty() { vec![generic.clone()] } else { existing }
            } else {
                vec![(p.key, p.backend, normalized(&p.routes))]
            };
            for id in targets {
                // limits the probe does not state stay those of the row, or of the whole backend
                let previous = entries.get(&id).or_else(|| entries.get(&generic)).and_then(|e| e.limits.clone());
                let limits = p.limits.clone().or(previous);
                let routes = id.2.clone();
                entries.insert(id, Entry { key: p.key, backend: p.backend, level, evidence: evidence.clone(), limits, routes });
            }
        }
        self.entries = entries;
    }

    /// Whether `key` can be done, and by which backend: the first backend (in the
    /// [`BackendOrder`] of the routing table) whose entry the policy accepts.
    pub fn decide(&self, key: Key, policy: Policy) -> Decision {
        let mut reasons = Vec::new();
        let mut best = SupportLevel::NotRegistered;
        let mut chosen = None;
        if !key.is_valid() {
            reasons.push("invalid capability key".to_string());
            return Decision { key, allowed: false, backend: None, level: SupportLevel::Unsupported, reasons };
        }
        let candidates = self.candidates(key);
        for e in &candidates {
            best = best.min(e.level);
            let accepted = policy.accepts(e.level);
            reasons.push(format!("{}: {:?} ({})", label(e), e.level, e.evidence));
            if accepted && chosen.is_none() {
                chosen = Some(e.backend);
            }
        }
        for e in self.entries.values().filter(|e| e.key == key) {
            if !candidates.iter().any(|c| std::ptr::eq(*c, e)) {
                reasons.push(format!("{}: not in the routing order", label(e)));
            }
        }
        if reasons.is_empty() {
            best = SupportLevel::Unsupported;
            reasons.push(format!("{} {}: no backend provides it", describe_op(key.operation), describe_target(key)));
        }
        Decision { key, allowed: chosen.is_some(), backend: chosen, level: best, reasons }
    }

    /// The encoder limits of the backend [`Registry::decide`] picks for encoding `codec`.
    pub fn encoder_limits(&self, codec: Codec, policy: Policy) -> Option<&EncoderLimits> {
        let key = Key::codec(Operation::Encode, codec);
        self.chosen(key, policy)?.limits.as_ref()
    }

    /// A lossless Smart Cut of `codec` from `source` into `output`: demux, GOP index, packet copy
    /// and mux must all be allowed.
    pub fn smart_cut(&self, codec: Codec, source: Container, output: Container, policy: Policy) -> CompositeDecision {
        let steps = vec![
            self.decide(Key::in_container(Operation::Demux, codec, source), policy),
            self.decide(Key::codec(Operation::GopIndex, codec), policy),
            self.decide(Key::in_container(Operation::PacketCopy, codec, output), policy),
            self.decide(Key::in_container(Operation::Mux, codec, output), policy),
        ];
        CompositeDecision::new(steps, Vec::new())
    }

    /// A Smart Render (boundary re-encode joined with copied packets) of a `stream` of `codec`:
    /// everything a Smart Cut needs, plus decode, encode, the boundary re-encode itself, and an
    /// encoder whose limits fit the stream.
    pub fn smart_render(&self, codec: Codec, source: Container, output: Container, stream: &StreamFormat, policy: Policy) -> CompositeDecision {
        let mut steps = self.smart_cut(codec, source, output, policy).steps;
        steps.push(self.decide(Key::codec(Operation::Decode, codec), policy));
        let mut encode = self.decide(Key::codec(Operation::Encode, codec), policy);
        let (backend, conditions) = self.encoder_for(codec, stream, policy);
        if encode.allowed {
            // the encoder that fits the stream, not merely the first one allowed
            encode.backend = backend;
            encode.allowed = backend.is_some();
        }
        steps.push(encode);
        steps.push(self.decide(Key::in_container(Operation::BoundaryReencode, codec, output), policy));
        CompositeDecision::new(steps, conditions)
    }

    /// The first allowed encoder backend (in [`BackendOrder`]) whose limits fit `stream`; when
    /// none does, why not, one line per allowed backend.
    fn encoder_for(&self, codec: Codec, stream: &StreamFormat, policy: Policy) -> (Option<Backend>, Vec<String>) {
        let key = Key::codec(Operation::Encode, codec);
        let mut why = Vec::new();
        for e in self.candidates(key) {
            if !policy.accepts(e.level) {
                continue;
            }
            match &e.limits {
                Some(limits) => match limits.check(stream) {
                    Ok(()) => return (Some(e.backend), Vec::new()),
                    Err(reasons) => why.extend(reasons.into_iter().map(|r| format!("{} encoder: {r}", label(e)))),
                },
                None => why.push(format!("{} encoder: no limits are known for {}, compatibility cannot be shown", label(e), codec.label())),
            }
        }
        if why.is_empty() {
            why.push(format!("no encoder limits are known for {}: compatibility cannot be shown", codec.label()));
        }
        (None, why)
    }

    /// The registry as data: every row in key order, and the probes applied.
    pub fn report(&self) -> Report {
        Report { registry_version: REGISTRY_VERSION, entries: self.entries.values().cloned().collect(), probes: self.probes.clone() }
    }
}

fn push_once<'a>(out: &mut Vec<&'a Entry>, e: &'a Entry) {
    if !out.iter().any(|o| std::ptr::eq(*o, e)) {
        out.push(e);
    }
}

/// How decisions name a row: its backend, and its routes when it names any (`Platform [nvenc]`).
fn label(e: &Entry) -> String {
    if e.routes.is_empty() { format!("{:?}", e.backend) } else { format!("{:?} [{}]", e.backend, e.routes.join(", ")) }
}

/// The canonical probe order: per capability and backend, available outcomes before unavailable
/// ones (so an unavailable one is applied last and wins), and within each, whole-backend probes
/// before those of concrete routes.
fn probe_order(p: &Probe) -> (Key, Backend, bool, bool, &ProbeOutcome, &Option<EncoderLimits>, &Vec<String>) {
    (p.key, p.backend, matches!(p.outcome, ProbeOutcome::Unavailable { .. }), !p.routes.is_empty(), &p.outcome, &p.limits, &p.routes)
}

fn describe_op(op: Operation) -> &'static str {
    match op {
        Operation::Demux => "demuxing",
        Operation::Decode => "decoding",
        Operation::ParseHeaders => "header parsing",
        Operation::GopIndex => "GOP indexing",
        Operation::PacketCopy => "packet copy",
        Operation::Encode => "encoding",
        Operation::Mux => "muxing",
        Operation::BoundaryReencode => "boundary re-encode",
    }
}

fn describe_target(key: Key) -> String {
    match key.container {
        Some(c) => format!("{} in {}", key.codec.label(), c.label()),
        None => key.codec.label().to_string(),
    }
}

fn row(key: Key, backend: Backend, level: SupportLevel, evidence: &str) -> Entry {
    Entry { key, backend, level, evidence: evidence.to_string(), limits: None, routes: Vec::new() }
}

/// The built-in rows (baseline audit, 2026-10-27, `main` @ f49a5def; updated 2026-10-31 with the
/// MPEG-2 encoder and the program / transport stream muxer that `main` has gained since).
pub(crate) fn native_entries() -> Vec<Entry> {
    use Backend::{Native, Platform};
    use Codec::*;
    use Container::*;
    use Operation::*;
    use SupportLevel::{Experimental, NeedsProbe, Supported, Unsupported};
    let c = Key::codec;
    let k = Key::in_container;
    let mut v = vec![
        // demux
        row(k(Demux, Mpeg2Video, MpegPs), Native, Supported, "cfa4-mpegts program streams; crates/codecs/tests/mpeg_oracle.rs"),
        row(k(Demux, Mpeg2Video, MpegTs), Native, Supported, "cfa4-mpegts transport streams; crates/codecs/tests/mpeg_oracle.rs"),
        row(k(Demux, Mpeg2Video, Mp4), Native, Supported, "cfa4-isobmff; crates/codecs/tests/mpeg_oracle.rs (mpeg2_in_quicktime_mp4_and_matroska)"),
        row(k(Demux, Mpeg2Video, Mov), Native, Supported, "cfa4-isobmff; crates/codecs/tests/mpeg_oracle.rs (mpeg2_in_quicktime_mp4_and_matroska)"),
        row(k(Demux, Mpeg2Video, Matroska), Native, Supported, "cfa4-matroska; crates/codecs/tests/mpeg_oracle.rs (mpeg2_in_quicktime_mp4_and_matroska)"),
        row(k(Demux, Mpeg2Video, Mxf), Native, Supported, "cfa4-mxf MPEG-2 essence; crates/codecs/tests/mxf_oracle.rs"),
        row(k(Demux, H264, Mp4), Native, Supported, "cfa4-isobmff; crates/codecs/tests/mp4_oracle.rs"),
        row(k(Demux, H264, Mov), Native, Supported, "cfa4-isobmff; crates/codecs/tests/mp4_oracle.rs"),
        row(k(Demux, H264, Matroska), Native, Experimental, "cfa4-matroska; synthetic tests in crates/codecs/src/mkv.rs, no oracle test yet"),
        row(k(Demux, H264, MpegTs), Native, Supported, "cfa4-mpegts NAL units; crates/codecs/tests/mpeg_oracle.rs"),
        row(k(Demux, H264, Mxf), Native, Supported, "cfa4-mxf AVC essence; crates/codecs/tests/mxf_oracle.rs"),
        row(k(Demux, Hevc, Mp4), Native, Supported, "cfa4-isobmff hvcC; crates/codecs/tests/mp4_oracle.rs (x265 conformance window)"),
        row(k(Demux, Hevc, Mov), Native, Experimental, "cfa4-isobmff hvcC (same demuxer as MP4); no QuickTime HEVC oracle test yet"),
        row(k(Demux, Hevc, Matroska), Native, Experimental, "cfa4-matroska; no oracle test yet"),
        row(k(Demux, Hevc, MpegTs), Native, Supported, "cfa4-mpegts NAL units; crates/codecs/tests/mpeg_oracle.rs (ts_hevc_latm.ts)"),
        row(k(Demux, Av1, Mp4), Native, Supported, "cfa4-isobmff av1C; crates/isobmff/tests/oracle_mux.rs (av1.mp4 remux)"),
        row(k(Demux, Av1, Matroska), Native, Experimental, "cfa4-matroska V_AV1; no oracle test yet"),
        row(k(Demux, Av1, WebM), Native, Experimental, "cfa4-matroska V_AV1; no oracle test yet"),
        row(k(Demux, Aac, Mp4), Native, Supported, "cfa4-isobmff esds; crates/codecs/tests/aac_channels_oracle.rs"),
        row(k(Demux, Aac, Mov), Native, Supported, "cfa4-isobmff esds"),
        row(k(Demux, Aac, Matroska), Native, Supported, "cfa4-matroska; crates/codecs/tests/matroska_audio_oracle.rs"),
        row(k(Demux, Aac, MpegTs), Native, Supported, "cfa4-mpegts ADTS; crates/codecs/tests/mpeg_oracle.rs (ts_h264_aac.ts)"),
        row(k(Demux, Pcm, Mov), Native, Supported, "cfa4-isobmff lpcm; crates/isobmff/tests/oracle_mux.rs"),
        row(k(Demux, Pcm, Mxf), Native, Supported, "cfa4-mxf PCM / AES3; crates/codecs/tests/mxf_oracle.rs"),
        // decode and header parsing
        row(c(Decode, Mpeg2Video), Native, Supported, "cfa4-mpeg2v; crates/mpeg2v/tests (ffmpeg oracle)"),
        row(c(Decode, H264), Native, Supported, "cfa4-h264; crates/h264/tests (ffmpeg oracle)"),
        row(c(Decode, Hevc), Native, Supported, "cfa4-hevc Main / Main 10; crates/hevc/tests (ffmpeg oracle)"),
        row(c(Decode, Av1), Native, Supported, "cfa4-av1 Main, bit-exact with libdav1d; crates/av1/tests"),
        row(c(Decode, Aac), Native, Supported, "cfa4-aac AAC-LC; crates/codecs/tests/aac_channels_oracle.rs"),
        row(c(Decode, Pcm), Native, Supported, "integer and float PCM in cfa4-codecs audio"),
        row(c(ParseHeaders, Mpeg2Video), Native, Supported, "cfa4-mpeg2v headers: sequence, GOP (closed_gop, broken_link), picture"),
        row(c(ParseHeaders, H264), Native, Supported, "cfa4-h264 params (SPS/PPS) and slice headers (IDR, POC)"),
        row(c(ParseHeaders, Hevc), Native, Supported, "cfa4-hevc params (VPS/SPS/PPS) and slice headers"),
        row(c(ParseHeaders, Av1), Native, Supported, "cfa4-av1 sequence and frame headers"),
        // GOP index, packet copy and boundary re-encode are what this work adds
        row(c(GopIndex, Mpeg2Video), Native, Unsupported, "not implemented yet (WP-02)"),
        row(c(GopIndex, H264), Native, Unsupported, "not implemented yet (WP-02)"),
        row(c(GopIndex, Hevc), Native, Unsupported, "not implemented yet (WP-07)"),
        row(c(GopIndex, Av1), Native, Unsupported, "not implemented yet (WP-07)"),
        // encode
        row(c(Encode, Hevc), Native, Unsupported, "CFA4 has no software HEVC encoder"),
        row(c(Encode, Hevc), Platform, NeedsProbe, "hardware encoder registered by cfa4-platform (NVENC); export::available(Format::Hevc)"),
        row(c(Encode, Av1), Native, Unsupported, "CFA4 has no software AV1 encoder"),
        row(
            c(Encode, Av1),
            Platform,
            Unsupported,
            "cfa4-platform registers no AV1 encoder; crates/export/src/settings_tests.rs asserts Format::Av1 is unavailable",
        ),
        row(c(Encode, Aac), Native, Supported, "cfa4-aac encoder; crates/export/src/surround_tests.rs"),
        row(c(Encode, Pcm), Native, Supported, "crates/export/src/pcm.rs"),
        // mux
        row(k(Mux, H264, Mp4), Native, Supported, "cfa4-isobmff Mp4Writer; crates/isobmff/tests/oracle_mux.rs (ffprobe)"),
        row(k(Mux, H264, Mov), Native, Supported, "cfa4-isobmff Mp4Writer; crates/isobmff/tests/oracle_mux.rs (ffprobe)"),
        row(k(Mux, H264, Mxf), Native, Supported, "cfa4-mxf OP1a AVC; crates/export/src/mxf_tests.rs"),
        row(k(Mux, H264, Matroska), Native, Experimental, "cfa4-matroska MkvWriter exists; not reachable from export"),
        row(k(Mux, Hevc, Mp4), Native, Supported, "cfa4-isobmff Mp4Writer; crates/isobmff/tests/oracle_mux.rs (remux_hevc.mp4)"),
        row(k(Mux, Hevc, Matroska), Native, Experimental, "cfa4-matroska MkvWriter exists; not reachable from export"),
        row(k(Mux, Av1, Mp4), Native, Supported, "cfa4-isobmff Mp4Writer; crates/isobmff/tests/oracle_mux.rs (remux_av1.mp4)"),
        row(k(Mux, Av1, Matroska), Native, Experimental, "cfa4-matroska MkvWriter exists; not reachable from export"),
        row(k(Mux, Mpeg2Video, MpegPs), Native, Supported, "cfa4-mpegts muxer; crates/export/src/mpeg2_tests.rs (ffmpeg_oracle_reads_every_container)"),
        row(k(Mux, Mpeg2Video, MpegTs), Native, Supported, "cfa4-mpegts muxer; crates/export/src/mpeg2_tests.rs (ffmpeg_oracle_reads_every_container)"),
        row(k(Mux, Mpeg2Video, Mp4), Native, Unsupported, "cfa4-isobmff has no MPEG-2 video sample entry"),
        row(k(Mux, Aac, Mp4), Native, Supported, "cfa4-isobmff; crates/isobmff/tests/oracle_mux.rs (remux_aac.m4a)"),
        row(k(Mux, Aac, Mov), Native, Supported, "cfa4-isobmff; crates/isobmff/tests/oracle_mux.rs"),
        row(k(Mux, Pcm, Mov), Native, Supported, "cfa4-isobmff; crates/isobmff/tests/oracle_mux.rs (pcm remuxes)"),
        row(k(Mux, Pcm, Mxf), Native, Supported, "cfa4-mxf PCM; crates/export/src/mxf_tests.rs"),
    ];
    let mut h264 = row(c(Encode, H264), Native, Supported, "cfa4-h264enc; crates/export/src/tests.rs, ffmpeg oracle");
    h264.limits = Some(EncoderLimits { profiles: vec!["baseline".into(), "main".into(), "high".into()], chroma: vec!["4:2:0".into()], bit_depths: vec![8] });
    v.push(h264);
    // Main profile (4:2:0) and the 4:2:2 profile, 8 bits (cfa4_mpeg2v::encode, export `mpeg2_422`)
    let mut mpeg2 = row(c(Encode, Mpeg2Video), Native, Supported, "cfa4-mpeg2v encoder; crates/export/src/mpeg2_tests.rs, crates/mpeg2v/src/encode/tests.rs");
    mpeg2.limits = Some(EncoderLimits { profiles: vec!["main".into(), "422".into()], chroma: vec!["4:2:0".into(), "4:2:2".into()], bit_depths: vec![8] });
    v.push(mpeg2);
    // packet copy and boundary re-encode: not implemented for any codec or container yet
    for codec in Codec::ALL {
        for container in Container::ALL {
            v.push(row(k(PacketCopy, codec, container), Native, Unsupported, "packet-copy export is not implemented yet (WP-04)"));
            if codec.is_video() {
                v.push(row(k(BoundaryReencode, codec, container), Native, Unsupported, "boundary re-encode is not implemented yet (WP-06)"));
            }
        }
    }
    v
}
