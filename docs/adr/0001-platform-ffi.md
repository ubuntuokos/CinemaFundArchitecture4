# ADR 0001: `unsafe` OS media FFI in `crates/platform`, and nowhere else

- **Status:** accepted (2026-10-05, approved by the project owner; recorded in
  [AGENTS.md](../../AGENTS.md) §0 item 3). Rewritten for Linux on 2026-11-09 (#357; owner decision):
  CFA4 targets Ubuntu 26.04 AMD64 only, so this record now describes the Linux backends that exist.
  The decision itself is unchanged. The original text, written for the removed macOS (VideoToolbox)
  and Windows (Media Foundation / Direct3D 11) backends, is in the git history of this file
  (`git log -- docs/adr/0001-platform-ffi.md`).
- **Issue:** #30 (hardware acceleration)

## Context

CFA4 is pure Rust and the workspace sets `unsafe_code = "forbid"`. Every codec has a
clean-room software decoder, and those remain the reference. But decoding H.264 / HEVC in software
is expensive: on the Linux measurement machine 4K H.264 costs 230–420 ms of CPU per frame and 4K
HEVC 200–350 ms ([performance.md](../performance.md), VA-API results), so 4K playback drops frames
under load. Linux exposes the GPU's video engines through two driver interfaces:

- **VA-API** (`libva`) for Intel and AMD GPUs (Mesa, Intel iHD);
- **NVDEC / NVENC** (`libnvcuvid`, `libnvidia-encode`, `libcuda`) in NVIDIA's proprietary driver,
  which has no VA-API of its own.

Using them needs FFI into C APIs, which is `unsafe` Rust. The declarations are transcribed from the
MIT-licensed headers of libva and NVIDIA's Video Codec SDK and checked against a C compiler's
layout (`*/abi_tests.rs`). The libraries are loaded at run time through `libloading` (ISC); nothing
GPL/LGPL is linked, and building needs no SDK or driver headers.

## Decision

Allow `unsafe` in exactly one crate, `crates/platform` (`cfa4-platform`, layer L5), for one
job: OS media FFI (hardware decoding, and hardware encoding through NVENC). Audio output, dialogs,
menus and other OS integration do not go in it. Components in other languages follow the
2026-11-07 addendum below.

Containment rules:

1. The crate does not use `lints.workspace = true`. Its own `[lints]` table copies the workspace
   lints except `unsafe_code = "deny"` (not `forbid`), and adds
   `clippy::undocumented_unsafe_blocks = "deny"`. Only the FFI modules (`vaapi::va`, `nvdec::cuvid`,
   `nvenc::ffi` / `nvenc::session` / `nvenc::device`) carry `#[allow(unsafe_code)]`; the rest of the
   crate (the fallback logic in `hybrid`, the decoder logic in `vaapi` and `nvdec`, `annexb`,
   `biplanar`, the encoder logic in `nvenc`, H.264 and H.265 alike) has no `unsafe`.
2. Every `unsafe` block has a `// SAFETY:` comment saying why it is sound.
3. The public API is safe: no `pub unsafe fn`, no raw pointers or FFI types in public signatures;
   failures are `Result`s. The crate keeps the never-crash `deny(clippy::unwrap_used, …)`
   attribute of every clean crate.
4. No panic crosses the FFI boundary. libva's error-message callback and NVDEC's parser callbacks
   run under `catch_unwind`; the NVDEC callbacks never unwind into C (they record the first error
   and return 0, and the decoder then fails over to software). NVENC has no callbacks into Rust.
5. The driver libraries are optional at run time. A system without libva, a DRM render node, the
   NVIDIA driver or a working GPU starts as before and decodes and encodes in software:
   `register()` reports `Unavailable` and does nothing. The FFI modules are compiled only for
   64-bit Linux (`target_os = "linux"`, `target_pointer_width = "64"`; NVENC export on x86_64);
   elsewhere the crate still compiles and `register()` is a no-op. It is not part of the wasm check
   (L5).
6. Which backend runs is decided by the compute router (`cfa4-route`, ADR 0004 / 0005), not by the
   platform crate: the shipped decode order is NVDEC, then VA-API, then CFA4's software decoders.

## The fallback guarantee

Registering a hardware decoder never makes a file undecodable, and never changes a picture:

- The factory declines (the software decoder is used) when Settings ▸ Playback ▸ Hardware
  decoding is Off, for formats the hardware path does not take (field-coded H.264, bit depths
  other than 8 / 10, 4:4:4, mismatched luma / chroma depth), and when the driver cannot create a
  hardware session for the stream.
- A hardware decoder that fails mid-stream (decode error, session lost after a GPU reset or
  suspend, in-band parameter sets that differ from the sample entry) becomes the software decoder
  for that stream without the caller noticing: `HybridDecoder` replays the samples since the last
  restart point through it and drops pictures already returned. It is logged and counted
  (`perf.stats` `decode.hardware.fallbacks`).
- Pictures are interchangeable with the software decoder's: same planes (bit-exact on the H.264
  High, HEVC Main and Main 10 parity fixtures, including after seeks and flushes), colour, pixel
  aspect, pts and presentation order (`crates/platform/tests/vaapi.rs`). The fallback logic is
  tested on every run with a stand-in hardware decoder (`crates/platform/tests/fallback.rs`).

## Consequences

- 4K H.264 / HEVC decode costs ~10–18 ms of CPU per frame through VA-API instead of 200–420 ms
  ([performance.md](../performance.md)).
- Better patent posture for the most encumbered formats: H.264 / HEVC decoding and encoding run in
  the GPU vendor's driver wherever the hardware path is used.
- A new review burden: changes to `crates/platform` need the same scrutiny as any FFI code.
  Anything else that wants `unsafe` needs a new decision, not an extension of this one (see the
  2026-11-07 addendum for components in other languages).

## VA-API hardware decoding

H.264 and HEVC decoding through VA-API (`vaapi/`, 2026-10-08):

- **libva is loaded at run time** (`libva.so.2`, `libva-drm.so.2`). The declarations
  (`vaapi/ffi.rs`) are transcribed from libva's MIT-licensed `va.h`, and `vaapi/abi_tests.rs` checks
  sizes, offsets, constants and bit-field positions against a C compiler's view of it. Only
  `vaapi::va` carries `#[allow(unsafe_code)]`; the declarations, the H.264 / HEVC front ends and the
  decoder (`VaDecoder`) are safe code.
- **Decoding is stateless:** the host parses the stream and keeps the decoded picture buffer; the GPU
  only decodes slice data. CFA4 does that host side with the software decoders' own parsers and
  DPBs (`cfa4_h264::dpb` and `cfa4_hevc::dpb` are generic over what a picture is), so the two
  decoders decide alike by construction, and the front end is tested against a stand-in for the
  hardware. The one place they cannot agree is concealment: pictures predicted from frames that were
  never decoded (the leading pictures of an open H.264 GOP after a seek) are concealed by both but
  can differ; HEVC pictures with references that were never decoded go to the software decoder
  instead.
- **One callback into Rust:** libva's error messages go to our log through a callback that runs
  under `catch_unwind`; its info messages are turned off.

## NVDEC hardware decoding

NVIDIA's proprietary driver has no VA-API of its own (only through the separate
`libva-nvidia-driver`), so Linux has a second decoder backend, `nvdec/` (2026-10-09), which the
router puts in front of VA-API. `libnvcuvid.so.1` and `libcuda.so.1` are loaded at run time,
`nvdec/ffi.rs` is transcribed from NVIDIA's MIT-licensed headers and checked by
`nvdec/abi_tests.rs`, only `nvdec::cuvid` carries `#[allow(unsafe_code)]`, and the CUDA context is
the one `nvenc::device` already retains for encoding. Unlike VA-API, NVDEC parses the stream
itself; its callbacks run inside our parse call and never unwind into C.

## NVENC hardware encoding (H.264, H.265, H.265 Main 10 HDR)

The crate's second use of the FFI is hardware encoding behind `cfa4_export::VideoEncoder`
(`nvenc/`, the safe encoder logic in `nvenc::export`). It differs from decoding in two ways, both
deliberate:

- **It is opt-in per export** (`ExportSettings::hardware_encoding`, `Off` by default). Exports from
  the built-in encoder are byte-identical across machines; a hardware encoder's output depends on the
  machine, so it must be asked for.
- **There is no mid-stream fallback.** A decoder can replay samples through another decoder; an
  encoder cannot hand a half-written stream to another one. The factory declines up front (the
  built-in encoder is used) for everything the hardware path does not take, and for any machine
  where the driver cannot create a session; a hardware encoder that fails during an export stops it
  with an error.

Device and driver: `nvenc::device` retains the first CUDA device's primary context through
`libcuda.so.1` (`cuDevicePrimaryCtxRetain` does not push a context onto the calling thread's stack);
the retained reference is released after the NVENC session and its buffers, and other users'
references are not reset. `libnvidia-encode.so.1` stays loaded with its function table. Both
libraries come from the installed NVIDIA driver, not this project. Export registration does not
claim that a hardware decoder is available.

**H.265 (HEVC).** The same session, ring and NV12 path serve both codecs; H.265 added no `unsafe`
module (new data declarations and a second codec-configuration member of an existing union). There
is no software HEVC encoder to fall back to, and there will not be one in this crate (x265 is GPL),
so `Format::Hevc` is offered only where a probe the platform crate registers finds an NVENC HEVC
encoder, and only where the router sends H.265 encoding to NVENC. Choosing the format is the opt-in;
what the hardware path does not take (two-pass, odd sizes) is an error naming the reason instead of
a different encoder.

**H.265 Main 10 (HDR).** `nvenc::ffi` gained data declarations (the HEVC picture parameters,
`NV_ENC_SEI_PAYLOAD`, the 10-bit buffer format) checked by the generated layout tests, and
`nvenc::session` gained the P010 pitch check and the SEI pointers it hands to the driver, which are
heap blocks owned by the session for its whole life (`// SAFETY:` comments say so). The conversion
from float pictures to 10-bit planes is safe code in the export crate.

## Addendum (2026-11-07): non-Rust components (owner decision)

The owner decided that a component written in another language (C, C++ or other) conflicts with
CFA4's rules **only if it slows CFA4 down**, and that step-by-step implementation is allowed: large
components may stay in their own language for now and are rewritten in Rust over later releases
([AGENTS.md](../../AGENTS.md), CFA4 section). This widens the "nowhere else" of this decision for
such components. Their bindings keep this ADR's containment rules:

1. `unsafe` stays in the binding module or crate of the component, with
   `#[allow(unsafe_code)]` only on its FFI modules and a `// SAFETY:` comment on every block.
2. The public API is safe and returns `Result`s; no panic crosses the FFI boundary
   (`catch_unwind`, AGENTS.md §0).
3. Callers reach the component through the engine's interfaces and the router (`cfa4-route`,
   ADR 0004/0005), never directly; the Rust path, where one exists, stays the fallback.
4. Running the component as a separate process is an equally valid choice when it isolates
   failures without slowing CFA4 down.
5. "Slows CFA4 down" is decided by measurement on the reference machine
   ([reference-machine.md](../development/reference-machine.md)), not by the language.

The planned Rust rewrite of such a component is tracked as its own task in the plan or ADR that
introduced it.

## Addendum (2026-11-09): libobs binding (`cfa4-obs`, live studio S0)

The first binding under the 2026-11-07 addendum: `crates/obs` (`cfa4-obs`, L5) binds libobs 33.0.1,
the OBS Studio core (C, GPL-2.0-or-later), for the live studio
([CFA4-LIVE-STUDIO-001](../plans/CFA4-LIVE-STUDIO-001.md) S0). It follows the containment rules
above: `unsafe_code = "deny"` with `#[allow(unsafe_code)]` only on `libobs::sys`, a `// SAFETY:`
comment on every block, a safe `Result` API behind the `StudioBackend` trait, the declarations
(`libobs::ffi`) checked against the installed headers by `ffi::abi_tests`, and the raw-video
callback under `catch_unwind`. libobs (and `libX11` / `libwayland-client` for its display) is
loaded at run time from the pinned CFA4 build, never the system's. Two choices of point 4 and 3:

- **Separate process** (plan D2): libobs runs only in `cfa4-studio-host`; the engine supervises it
  and restarts it after a crash, so a libobs fault is a status change, not a CFA4 crash.
- **Rust fallback:** the pure-Rust software backend behind the same trait; `auto` uses it when
  libobs cannot start, and later phases replace libobs with Rust part by part (plan S12+).
