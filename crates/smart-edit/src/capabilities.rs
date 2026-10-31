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
//! capability the unavailable one wins. The same entries and probes therefore always give the same
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

/// Who implements a capability. The order is the order [`Registry::decide`] tries them in: CFA4's
/// own Rust engine first, always the fallback the others supplement.
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
}

/// What a decision may accept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// Accept [`SupportLevel::Experimental`] entries (developer setting; off for exports).
    pub allow_experimental: bool,
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
    /// One line per backend considered, in [`Backend`] order.
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

/// The capability registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    /// Rows as inserted, before probes.
    base: BTreeMap<(Key, Backend), Entry>,
    /// Every probe applied so far, in canonical order.
    probes: Vec<Probe>,
    /// `base` with `probes` applied: what decisions read.
    entries: BTreeMap<(Key, Backend), Entry>,
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

    /// Add or replace a row.
    pub fn insert(&mut self, entry: Entry) -> Result<(), RegistryError> {
        if !entry.key.is_valid() {
            return Err(RegistryError::InvalidKey(entry.key));
        }
        self.base.insert((entry.key, entry.backend), entry);
        self.rebuild();
        Ok(())
    }

    pub fn get(&self, key: Key, backend: Backend) -> Option<&Entry> {
        self.entries.get(&(key, backend))
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// Apply run-time probes. All probes applied so far, from this call and earlier ones, take
    /// effect in a canonical order (by key, backend, then outcome with every unavailable outcome
    /// last), so an "unavailable" always overrides an "available" for the same capability whatever
    /// order or batches the probes came in. The probes are kept for the [`Report`].
    pub fn apply_probes(&mut self, probes: impl IntoIterator<Item = Probe>) -> Result<(), RegistryError> {
        let probes: Vec<Probe> = probes.into_iter().collect();
        if let Some(p) = probes.iter().find(|p| !p.key.is_valid()) {
            return Err(RegistryError::InvalidKey(p.key));
        }
        self.probes.extend(probes);
        self.probes.sort_by(|a, b| (a.key, a.backend, &a.outcome, &a.limits).cmp(&(b.key, b.backend, &b.outcome, &b.limits)));
        self.probes.dedup();
        self.rebuild();
        Ok(())
    }

    fn rebuild(&mut self) {
        let mut entries = self.base.clone();
        for p in &self.probes {
            let previous = entries.get(&(p.key, p.backend)).and_then(|e| e.limits.clone());
            let (level, evidence) = match &p.outcome {
                ProbeOutcome::Available { detail } => (SupportLevel::Supported, format!("run-time probe: {detail}")),
                ProbeOutcome::Unavailable { reason } => (SupportLevel::Unsupported, format!("run-time probe: {reason}")),
            };
            let limits = p.limits.clone().or(previous);
            entries.insert((p.key, p.backend), Entry { key: p.key, backend: p.backend, level, evidence, limits });
        }
        self.entries = entries;
    }

    /// Whether `key` can be done, and by which backend: the first backend (in [`Backend`] order)
    /// whose entry the policy accepts.
    pub fn decide(&self, key: Key, policy: Policy) -> Decision {
        let mut reasons = Vec::new();
        let mut best = SupportLevel::NotRegistered;
        let mut chosen = None;
        if !key.is_valid() {
            reasons.push("invalid capability key".to_string());
            return Decision { key, allowed: false, backend: None, level: SupportLevel::Unsupported, reasons };
        }
        for e in self.entries.range((key, Backend::Native)..=(key, Backend::Ffmpeg)).map(|(_, e)| e) {
            best = best.min(e.level);
            let accepted = e.level == SupportLevel::Supported || (policy.allow_experimental && e.level == SupportLevel::Experimental);
            reasons.push(format!("{:?}: {:?} ({})", e.backend, e.level, e.evidence));
            if accepted && chosen.is_none() {
                chosen = Some(e.backend);
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
        let backend = self.decide(key, policy).backend?;
        self.get(key, backend)?.limits.as_ref()
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

    /// The first allowed encoder backend (in [`Backend`] order) whose limits fit `stream`; when
    /// none does, why not, one line per allowed backend.
    fn encoder_for(&self, codec: Codec, stream: &StreamFormat, policy: Policy) -> (Option<Backend>, Vec<String>) {
        let key = Key::codec(Operation::Encode, codec);
        let mut why = Vec::new();
        for e in self.entries.range((key, Backend::Native)..=(key, Backend::Ffmpeg)).map(|(_, e)| e) {
            let accepted = e.level == SupportLevel::Supported || (policy.allow_experimental && e.level == SupportLevel::Experimental);
            if !accepted {
                continue;
            }
            match &e.limits {
                Some(limits) => match limits.check(stream) {
                    Ok(()) => return (Some(e.backend), Vec::new()),
                    Err(reasons) => why.extend(reasons.into_iter().map(|r| format!("{:?} encoder: {r}", e.backend))),
                },
                None => why.push(format!("{:?} encoder: no limits are known for {}, compatibility cannot be shown", e.backend, codec.label())),
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
    Entry { key, backend, level, evidence: evidence.to_string(), limits: None }
}

/// The built-in rows (baseline audit, 2026-10-27, `main` @ f49a5def).
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
        row(c(Encode, Mpeg2Video), Native, Unsupported, "CFA4 has no MPEG-2 video encoder"),
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
        row(k(Mux, Mpeg2Video, MpegPs), Native, Unsupported, "cfa4-mpegts has no muxer"),
        row(k(Mux, Mpeg2Video, MpegTs), Native, Unsupported, "cfa4-mpegts has no muxer"),
        row(k(Mux, Mpeg2Video, Mp4), Native, Unsupported, "cfa4-isobmff has no MPEG-2 video sample entry"),
        row(k(Mux, Aac, Mp4), Native, Supported, "cfa4-isobmff; crates/isobmff/tests/oracle_mux.rs (remux_aac.m4a)"),
        row(k(Mux, Aac, Mov), Native, Supported, "cfa4-isobmff; crates/isobmff/tests/oracle_mux.rs"),
        row(k(Mux, Pcm, Mov), Native, Supported, "cfa4-isobmff; crates/isobmff/tests/oracle_mux.rs (pcm remuxes)"),
        row(k(Mux, Pcm, Mxf), Native, Supported, "cfa4-mxf PCM; crates/export/src/mxf_tests.rs"),
    ];
    let mut h264 = row(c(Encode, H264), Native, Supported, "cfa4-h264enc; crates/export/src/tests.rs, ffmpeg oracle");
    h264.limits = Some(EncoderLimits { profiles: vec!["baseline".into(), "main".into(), "high".into()], chroma: vec!["4:2:0".into()], bit_depths: vec![8] });
    v.push(h264);
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
