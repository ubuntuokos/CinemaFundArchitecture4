//! What the workspace can run where: every backend the shipped routing table names, per family,
//! with its maturity, the devices it runs on, its home crate and whether it is locked to one
//! vendor's hardware.
//!
//! Keep [`CATALOG`] true to the code. A backend becomes [`Maturity::Implemented`] in the same
//! commit that makes it work, and `coverage_matches_the_workspace` below is updated with it: that
//! test is the vendor-neutrality status of CFA4.

use serde::{Deserialize, Serialize};

use crate::{BackendId, DeviceKind, Family, Vendor};

/// Whether a backend works today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maturity {
    Implemented,
    Planned,
}

/// One backend in one family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub family: Family,
    /// The backend id the routing table uses.
    pub backend: &'static str,
    pub maturity: Maturity,
    /// The kinds of device it runs on.
    pub runs_on: &'static [DeviceKind],
    /// The one vendor whose hardware it needs (NVDEC, NVENC, CUDA, OpenVINO NPU), `None` when any
    /// vendor's device will do.
    pub vendor_lock: Option<Vendor>,
    /// Where it lives (or will live).
    pub home: &'static str,
    pub note: &'static str,
}

impl Entry {
    /// Whether it runs on a GPU or an NPU (not only on the CPU).
    pub fn is_accelerator(&self) -> bool {
        self.runs_on.iter().any(|k| *k != DeviceKind::Cpu)
    }

    /// Whether it runs only on the CPU.
    pub fn is_cpu_only(&self) -> bool {
        !self.runs_on.is_empty() && self.runs_on.iter().all(|k| *k == DeviceKind::Cpu)
    }
}

use DeviceKind::{Cpu as C, Gpu as G, Npu as N};
use Family as F;
use Maturity::{Implemented as Done, Planned};

const fn entry(
    family: Family,
    backend: &'static str,
    maturity: Maturity,
    runs_on: &'static [DeviceKind],
    vendor_lock: Option<Vendor>,
    home: &'static str,
    note: &'static str,
) -> Entry {
    Entry { family, backend, maturity, runs_on, vendor_lock, home, note }
}

const NVIDIA: Option<Vendor> = Some(Vendor::Nvidia);
const INTEL: Option<Vendor> = Some(Vendor::Intel);

/// Every backend of the shipped table (`routing.toml`), per family.
pub const CATALOG: &[Entry] = &[
    entry(F::Display, "wgpu", Done, &[G], None, "crates/gpu", "WGSL compositor, effects, LUTs, masks, transitions (Vulkan)"),
    entry(F::Display, "cpu-compositor", Done, &[C], None, "crates/render", "reference CPU renderer"),
    entry(F::CodecDecode, "nvdec", Done, &[G], NVIDIA, "crates/platform/src/nvdec", "H.264, HEVC Main / Main 10; proprietary driver only"),
    entry(F::CodecDecode, "vaapi", Done, &[G], None, "crates/platform/src/vaapi", "H.264, HEVC Main / Main 10 on AMD, Intel and nouveau; VP9 / AV1 not yet"),
    entry(F::CodecDecode, "ffmpeg", Planned, &[G, C], None, "ADR 0005", "FFmpeg 9 supplementary backend"),
    entry(F::CodecDecode, "gstreamer-vulkan-video", Planned, &[G], None, "crates/gstreamer", "Vulkan Video decode queues through GStreamer (ADR 0002)"),
    entry(F::CodecDecode, "software", Done, &[C], None, "crates/codecs", "pure-Rust decoders, bit-exact fallback (HybridDecoder)"),
    entry(F::CodecEncode, "nvenc", Done, &[G], NVIDIA, "crates/platform/src/nvenc", "H.264 and HEVC; proprietary driver only"),
    entry(F::CodecEncode, "vaapi", Planned, &[G], None, "crates/platform/src/vaapi", "H.264 / HEVC on AMD and Intel (docs/gaps.md G5)"),
    entry(F::CodecEncode, "ffmpeg", Planned, &[G, C], None, "ADR 0005", "FFmpeg 9 supplementary backend"),
    entry(F::CodecEncode, "gstreamer-va", Planned, &[G], None, "crates/gstreamer", "VA encoders through GStreamer (ADR 0002)"),
    entry(F::CodecEncode, "software", Done, &[C], None, "crates/h264enc, crates/prores, crates/dnx, crates/apv", "built-in encoders"),
    entry(F::Ai, "openvino-npu", Planned, &[N], INTEL, "crates/platform", "Intel NPU through OpenVINO (ADR 0004 phase 4)"),
    entry(F::Ai, "burn-cuda", Planned, &[G], NVIDIA, "crates/speech, crates/tts", "Burn on CUDA (ADR 0004 phase 2)"),
    entry(F::Ai, "burn-vulkan", Planned, &[G], None, "crates/speech, crates/tts", "Burn on Vulkan, any GPU vendor (ADR 0004 phase 2)"),
    entry(F::Ai, "candle-cpu", Done, &[C], None, "crates/speech, crates/tts", "Whisper and Kokoro on candle"),
    entry(F::Compute, "wgpu-compute", Planned, &[G], None, "crates/gpu", "denoise, stabilisation, optical flow in compute shaders"),
    entry(F::Compute, "cpu", Done, &[C], None, "crates/engine, crates/audio-dsp", "CPU implementations"),
];

/// The catalogue row of a backend in a family.
pub fn lookup(family: Family, backend: &BackendId) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.family == family && e.backend == backend.as_str())
}

/// The catalogue rows of one family, in table order.
pub fn entries(family: Family) -> impl Iterator<Item = &'static Entry> {
    CATALOG.iter().filter(move |e| e.family == family)
}

/// How vendor-neutral one family's acceleration is, from the catalogue alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub family: Family,
    /// Implemented accelerators (GPU / NPU) that run on any vendor's hardware.
    pub open: Vec<&'static str>,
    /// Implemented accelerators that run on one vendor's hardware only.
    pub locked: Vec<&'static str>,
    /// Planned vendor-neutral accelerators: the work that closes the gap.
    pub planned_open: Vec<&'static str>,
    /// Planned accelerators locked to one vendor.
    pub planned_locked: Vec<&'static str>,
    /// Whether every vendor's hardware can accelerate this family today.
    pub vendor_neutral: bool,
}

/// [`Coverage`] of every family: the vendor-neutrality gaps still to close (`perf.stats`).
pub fn coverage() -> Vec<Coverage> {
    Family::ALL
        .into_iter()
        .map(|family| {
            let pick = |maturity: Maturity, locked: bool| -> Vec<&'static str> {
                entries(family).filter(|e| e.is_accelerator() && e.maturity == maturity && e.vendor_lock.is_some() == locked).map(|e| e.backend).collect()
            };
            let open = pick(Done, false);
            Coverage {
                family,
                vendor_neutral: !open.is_empty(),
                open,
                locked: pick(Done, true),
                planned_open: pick(Planned, false),
                planned_locked: pick(Planned, true),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Table;

    #[test]
    fn every_backend_of_the_shipped_table_is_catalogued_and_vice_versa() {
        let table = Table::builtin().unwrap();
        for f in Family::ALL {
            let listed: Vec<&str> = table.family_order(f).iter().map(BackendId::as_str).collect();
            let catalogued: Vec<&str> = entries(f).map(|e| e.backend).collect();
            assert_eq!(listed, catalogued, "{f:?}: routing.toml and the catalogue list the same backends in the same order");
        }
    }

    #[test]
    fn every_family_ends_with_an_implemented_cpu_only_backend() {
        for f in Family::ALL {
            let last = entries(f).last().unwrap();
            assert!(last.is_cpu_only() && last.maturity == Maturity::Implemented, "{f:?}: {last:?}");
            assert!(last.vendor_lock.is_none());
        }
    }

    #[test]
    fn npus_serve_only_ai_and_cpu_backends_are_never_vendor_locked() {
        for e in CATALOG {
            if e.runs_on.contains(&DeviceKind::Npu) {
                assert_eq!(e.family, Family::Ai, "{e:?}");
            }
            if e.is_cpu_only() {
                assert!(e.vendor_lock.is_none(), "{e:?}");
            }
            assert!(BackendId::new(e.backend).is_ok(), "{e:?}");
        }
    }

    /// Update this test with the catalogue: it is the vendor-neutrality status of the workspace.
    #[test]
    fn coverage_matches_the_workspace() {
        let all = coverage();
        let get = |f: Family| all.iter().find(|c| c.family == f).unwrap();
        let neutral: Vec<Family> = all.iter().filter(|c| c.vendor_neutral).map(|c| c.family).collect();
        assert_eq!(neutral, vec![Family::Display, Family::CodecDecode]);

        let display = get(Family::Display);
        assert_eq!((display.open.clone(), display.locked.clone()), (vec!["wgpu"], vec![]));

        let decode = get(Family::CodecDecode);
        assert_eq!((decode.open.clone(), decode.locked.clone()), (vec!["vaapi"], vec!["nvdec"]));
        assert_eq!(decode.planned_open, vec!["ffmpeg", "gstreamer-vulkan-video"]);

        // hardware encoding is NVIDIA-only today: AMD and Intel encode on the CPU
        let encode = get(Family::CodecEncode);
        assert_eq!((encode.open.clone(), encode.locked.clone()), (vec![], vec!["nvenc"]));
        assert_eq!(encode.planned_open, vec!["vaapi", "ffmpeg", "gstreamer-va"]);

        // local AI runs on the CPU (candle) everywhere
        let ai = get(Family::Ai);
        assert_eq!((ai.open.clone(), ai.locked.clone()), (vec![], vec![]));
        assert_eq!((ai.planned_open.clone(), ai.planned_locked.clone()), (vec!["burn-vulkan"], vec!["openvino-npu", "burn-cuda"]));

        let compute = get(Family::Compute);
        assert_eq!((compute.open.clone(), compute.planned_open.clone()), (vec![], vec!["wgpu-compute"]));
    }

    #[test]
    fn coverage_serialises_for_perf_stats() {
        let v = serde_json::to_value(coverage()).unwrap();
        assert_eq!(v[1]["family"], "codec_decode");
        assert_eq!(v[1]["locked"], serde_json::json!(["nvdec"]));
        assert_eq!(v[2]["vendor_neutral"], false);
    }
}
