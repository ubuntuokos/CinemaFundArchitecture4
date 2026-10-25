//! What the workspace can run where: every backend per workload, in the order `Auto` tries them.
//!
//! Keep [`CATALOG`] true to the code. A backend becomes [`Maturity::Implemented`] in the same
//! commit that makes it work, and `coverage_matches_the_workspace` in `plan.rs` is updated with it.

use serde::{Deserialize, Serialize};

use crate::vendor::Vendor;

/// A kind of job CFA4 accelerates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Workload {
    /// Compositing, effects, transitions, colour: preview and export rendering.
    Composite,
    VideoDecode,
    VideoEncode,
    /// Transcription and captions (Whisper).
    SpeechToText,
    /// Narration and dubbing voices (Kokoro).
    TextToSpeech,
}

impl Workload {
    pub const ALL: [Workload; 5] = [Workload::Composite, Workload::VideoDecode, Workload::VideoEncode, Workload::SpeechToText, Workload::TextToSpeech];

    pub const fn id(self) -> &'static str {
        match self {
            Workload::Composite => "composite",
            Workload::VideoDecode => "videoDecode",
            Workload::VideoEncode => "videoEncode",
            Workload::SpeechToText => "speechToText",
            Workload::TextToSpeech => "textToSpeech",
        }
    }
}

/// The kind of processor a backend runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Gpu,
    Npu,
    Cpu,
}

/// One way to run a workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
    /// wgpu render pipelines (Vulkan on Linux): any vendor's GPU.
    Wgpu,
    /// VA-API decoding (Mesa radeonsi / nouveau, Intel iHD / i965).
    VaApiDecode,
    /// VA-API encoding (Mesa radeonsi, Intel iHD).
    VaApiEncode,
    /// Vulkan Video decode / encode queues: any vendor whose driver exposes them.
    VulkanVideo,
    /// NVIDIA NVDEC, proprietary driver only.
    Nvdec,
    /// NVIDIA NVENC, proprietary driver only.
    Nvenc,
    /// Neural-network inference in wgpu compute shaders (Vulkan): any vendor's GPU.
    GpuCompute,
    /// Neural-network inference on an NPU (Linux accel subsystem).
    Npu,
    /// Pure-Rust CPU implementation: always present.
    Cpu,
}

impl Backend {
    pub const fn id(self) -> &'static str {
        match self {
            Backend::Wgpu => "wgpu",
            Backend::VaApiDecode => "vaApiDecode",
            Backend::VaApiEncode => "vaApiEncode",
            Backend::VulkanVideo => "vulkanVideo",
            Backend::Nvdec => "nvdec",
            Backend::Nvenc => "nvenc",
            Backend::GpuCompute => "gpuCompute",
            Backend::Npu => "npu",
            Backend::Cpu => "cpu",
        }
    }

    pub const fn unit(self) -> Unit {
        match self {
            Backend::Wgpu | Backend::VaApiDecode | Backend::VaApiEncode | Backend::VulkanVideo | Backend::Nvdec | Backend::Nvenc | Backend::GpuCompute => {
                Unit::Gpu
            }
            Backend::Npu => Unit::Npu,
            Backend::Cpu => Unit::Cpu,
        }
    }

    /// The one vendor whose hardware this backend runs on, or `None` for vendor-neutral ones.
    pub const fn vendor_lock(self) -> Option<Vendor> {
        match self {
            Backend::Nvdec | Backend::Nvenc => Some(Vendor::Nvidia),
            _ => None,
        }
    }
}

/// Whether a backend works today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Maturity {
    Implemented,
    Planned,
}

/// One row of the catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub workload: Workload,
    pub backend: Backend,
    pub maturity: Maturity,
    /// Where it lives (or will live).
    pub home: &'static str,
    pub note: &'static str,
}

const fn entry(workload: Workload, backend: Backend, maturity: Maturity, home: &'static str, note: &'static str) -> Entry {
    Entry { workload, backend, maturity, home, note }
}

use Backend as B;
use Maturity::{Implemented as Done, Planned};
use Workload as W;

/// Every backend per workload, in `Auto` order. Each workload ends with its CPU backend.
pub const CATALOG: &[Entry] = &[
    entry(W::Composite, B::Wgpu, Done, "crates/gpu", "WGSL compositor, effects, LUTs, masks, transitions"),
    entry(W::Composite, B::Cpu, Done, "crates/render", "reference CPU renderer"),
    // the order of `cfa4_platform::register`: NVDEC in front of VA-API
    entry(W::VideoDecode, B::Nvdec, Done, "crates/platform/src/nvdec", "H.264, HEVC Main / Main 10"),
    entry(W::VideoDecode, B::VaApiDecode, Done, "crates/platform/src/vaapi", "H.264, HEVC Main / Main 10; VP9 / AV1 not yet"),
    entry(W::VideoDecode, B::VulkanVideo, Planned, "crates/platform", "vendor-neutral H.264 / HEVC / AV1 decode queues"),
    entry(W::VideoDecode, B::Cpu, Done, "crates/codecs", "pure-Rust decoders, bit-exact fallback (HybridDecoder)"),
    entry(W::VideoEncode, B::Nvenc, Done, "crates/platform/src/nvenc", "H.264 on Linux; HEVC Windows-only so far"),
    entry(W::VideoEncode, B::VaApiEncode, Planned, "crates/platform/src/vaapi", "H.264 / HEVC on AMD and Intel (docs/gaps.md G5)"),
    entry(W::VideoEncode, B::VulkanVideo, Planned, "crates/platform", "vendor-neutral H.264 / HEVC encode queues"),
    entry(W::VideoEncode, B::Cpu, Done, "crates/h264enc, crates/prores, crates/dnx, crates/apv", "built-in encoders"),
    entry(W::SpeechToText, B::GpuCompute, Planned, "crates/speech", "Whisper on wgpu compute (Vulkan), any GPU vendor"),
    entry(W::SpeechToText, B::Npu, Planned, "crates/platform", "needs an owner decision: NPU runtimes are C/C++ FFI"),
    entry(W::SpeechToText, B::Cpu, Done, "crates/speech", "Whisper on candle (feature `whisper`)"),
    entry(W::TextToSpeech, B::GpuCompute, Planned, "crates/tts", "Kokoro on wgpu compute (Vulkan), any GPU vendor"),
    entry(W::TextToSpeech, B::Npu, Planned, "crates/platform", "needs an owner decision: NPU runtimes are C/C++ FFI"),
    entry(W::TextToSpeech, B::Cpu, Done, "crates/tts", "Kokoro on candle (feature `kokoro`), formant voices"),
];

/// The catalogue rows of one workload, in `Auto` order.
pub fn entries(workload: Workload) -> impl Iterator<Item = &'static Entry> {
    CATALOG.iter().filter(move |e| e.workload == workload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workload_has_an_implemented_cpu_backend_last() {
        for w in Workload::ALL {
            let rows: Vec<&Entry> = entries(w).collect();
            let last = rows.last().unwrap();
            assert_eq!((last.backend, last.maturity), (Backend::Cpu, Maturity::Implemented), "{w:?}");
            assert_eq!(rows.iter().filter(|e| e.backend == Backend::Cpu).count(), 1, "{w:?}");
        }
    }

    #[test]
    fn no_backend_is_listed_twice_for_one_workload() {
        for w in Workload::ALL {
            let mut backends: Vec<Backend> = entries(w).map(|e| e.backend).collect();
            let n = backends.len();
            backends.sort();
            backends.dedup();
            assert_eq!(backends.len(), n, "{w:?}");
        }
    }

    #[test]
    fn ids_match_the_serde_names() {
        for w in Workload::ALL {
            assert_eq!(serde_json::to_string(&w).unwrap(), format!("\"{}\"", w.id()));
        }
        for e in CATALOG {
            assert_eq!(serde_json::to_string(&e.backend).unwrap(), format!("\"{}\"", e.backend.id()));
        }
    }
}
