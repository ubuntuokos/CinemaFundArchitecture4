# cfa4-accel

Vendor-neutral acceleration planning for CFA4 (layer L1; no UI, no FFI, no `unsafe`).

CFA4 must be accelerated on AMD, Intel and NVIDIA GPUs, use NPUs where they help, and always work
on the CPU. This crate decides, in one place, **which backend is tried for which workload, in
which order**, and reports where vendor neutrality is still missing. It runs no media and links
no driver: the backends themselves live in `crates/gpu`, `crates/platform`, `crates/speech`,
`crates/tts` and the codec crates.

```rust
let inventory = cfa4_accel::probe();                 // GPUs and NPUs from sysfs
let settings = cfa4_accel::Settings::default();      // Auto everywhere
for plan in cfa4_accel::plan_all(&inventory, &settings) {
    log::info!("{:?}: {:?}", plan.workload, plan.chain); // e.g. VideoDecode: [VaApiDecode, Cpu]
}
```

## Pieces

| module | what |
|---|---|
| `probe` | Inventory from `/sys/class/drm/renderD*` (GPUs) and `/sys/class/accel/accel*` (NPUs): PCI vendor / device ids and the kernel driver. Bounded reads (64 bytes per attribute, 64 nodes per class); unreadable entries become notes, never errors or panics. Empty outside Linux. |
| `inventory` | `Device` (kind, vendor, ids, driver, node) and `Inventory`, plain data, so planning is testable without hardware. |
| `catalog` | `CATALOG`: every backend per `Workload` in `Auto` order, with `Maturity` (implemented / planned), its home crate and whether it is locked to one vendor (`Backend::vendor_lock`). Every workload ends with its CPU backend. |
| `plan` | `plan()` turns inventory + catalogue + `Settings` into a fallback chain that always ends with `Backend::Cpu`, and records why every other backend was skipped (`notImplemented`, `excludedByPreference`, `vendorLockedOff`, `noDevice`). `coverage()` says per workload whether an implemented accelerator exists that is not vendor-locked. |

## Settings

Serde (camelCase), so the UI, the control channel and MCP read and set them:

```json
{ "preferences": { "videoEncode": "gpu", "speechToText": "npu" },
  "allowVendorLocked": true,
  "includePlanned": false }
```

- `preferences`: `auto` (every available backend, catalogue order) · `gpu` · `npu` · `cpu`, per
  workload; missing workloads are `auto`.
- `allowVendorLocked: false` leaves out NVDEC / NVENC: a strictly vendor-neutral chain, to prove
  that every workload still runs without them.
- `includePlanned: true` lists planned backends too (roadmaps and reports, never for running work).

## Guarantees

- **The CPU is always last.** Every chain is non-empty and ends with `Backend::Cpu` (tested for
  every machine × preference × flag combination).
- **A chain is a list of candidates.** The backend's own factory still verifies the hardware and
  declines what it cannot do (`HybridDecoder`, the export encoder registry); the next entry runs.
- **Never crash:** no `unwrap` / `expect` / `panic!` outside tests; hostile sysfs contents
  (oversized files, garbage ids, missing links, look-alike node names, more than 64 nodes) and
  hostile settings JSON are tested.

## Vendor-neutrality status (`coverage_matches_the_workspace`)

| workload | vendor-neutral accelerator today | vendor-locked | planned vendor-neutral |
|---|---|---|---|
| Composite | wgpu (Vulkan) | — | — |
| Video decode | VA-API (AMD, Intel, nouveau) | NVDEC | Vulkan Video |
| Video encode | **none: AMD and Intel encode on the CPU** | NVENC | VA-API encode, Vulkan Video |
| Speech to text | **none: CPU (candle)** | — | wgpu compute, NPU |
| Text to speech | **none: CPU (candle)** | — | wgpu compute, NPU |

The test fails when the catalogue changes, so this table and the code move together.

## Not yet

- Wiring: `cfa4_platform::register()` and the export encoder registry should consult the
  plan (Settings ▸ Acceleration), and `perf.stats` should report `plan_all` and `coverage`.
- Merging wgpu's adapter list (`crates/gpu/src/adapter.rs` `Candidate`) into the inventory for
  discrete / integrated and software (llvmpipe) adapters.
- NPU backends need an owner decision: NPU runtimes (OpenVINO for Intel, the XDNA stack for AMD)
  are C / C++ libraries, so they would need FFI in `crates/platform` beyond OS media APIs
  (ADR 0001) and a licence review.
