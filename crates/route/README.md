# cfa4-route

The compute router of CFA4 (layer L1): for every accelerated task it chooses a **route**, one
backend on one device (GPU, NPU or CPU), from the routing table, the device inventory and the
user's preferences. The decision and its rules are in
[ADR 0004](../../docs/adr/ADR-0004-compute-routing.md).

```rust
use std::path::Path;
use cfa4_route::{probe, Family, Preferences, Router, Table, Task};

let table = Table::builtin()?.merged(Table::parse(&user_file_text)?);
let mut router = Router::new(table, probe::probe(Path::new("/")), Preferences::default());
router.register(nvdec_backend); // each backend crate registers its own `Backend`
let decision = router.choose(&Task::new("video.decode.hevc", Family::CodecDecode)?)?;
// decision.backend, decision.device, decision.declined (every route passed over, with its reason)
```

## What it does

- **Table** (`Table`): an ordered backend list per family (`display`, `codec_decode`,
  `codec_encode`, `ai`, `compute`) and per-task rows on top (`order`, `min_free_vram_mb`). The
  shipped table is [`routing.toml`](routing.toml); a user file overrides it family by family and
  field by field. A bad file is refused whole.
- **Inventory** (`Inventory`, `probe::probe`): the CPU plus the GPUs (DRM render nodes) and NPUs
  (accel nodes: Intel `intel_vpu`, AMD `amdxdna`) Linux lists in `/sys`, with their PCI vendor and
  device ids and kernel driver, and NVIDIA GPUs on the proprietary driver without a render node
  (`/proc/driver/nvidia/gpus`). Unreadable entries become notes. Nothing is opened or loaded;
  backends add free-memory figures when they measure them.
- **Choice** (`Router::choose`): the first route whose backend is registered and enabled, whose
  device is present and allowed by the family's preference, which the backend accepts, and which
  fits the memory budget. Every route passed over is kept with its `Reason`.
- **Fallback** (`Router::next_after`): the route after a failed one, for work that can restart.
- **Chain** (`Router::chain`): every route that could take the task, in order, with every route
  passed over. The first is the choice, the last runs on the CPU; `perf.stats` reports it.
- **Preferences** (`Preferences`, serde, unknown fields refused): per family Automatic, one device
  (with the CPU as last resort) or CPU only. Hardware encoding stays opt-in: the default for
  `codec_encode` is CPU only. `allow_vendor_locked: false` leaves out the backends that run on one
  vendor's hardware only (NVDEC, NVENC, CUDA, OpenVINO NPU): a strictly vendor-neutral routing, to
  prove every family still runs without them.
- **Catalogue** (`catalog::CATALOG`): every backend the shipped table names, per family, with its
  maturity (implemented / planned), the devices it runs on, its home crate and its vendor lock. A
  planned backend in the order is reported as `not_implemented`, an implemented one this build did
  not register as `not_registered`.
- **Coverage** (`coverage()`): per family, the implemented accelerators that run on any vendor's
  hardware, the vendor-locked ones, and the planned ones that close the gap.

## Guarantees

- No `unsafe`, no `unwrap` / `expect` / `panic!` outside tests.
- Every input is bounded: identifiers (64 bytes, `a-z 0-9 . _ - :`), the table file (256 KiB,
  32 backends per list, 1024 task rows), `/sys` attributes (64 bytes), node names (digits only
  after `renderD` / `accel`), devices (64), notes (32 × 256 bytes), backend reasons (256 bytes).
  Hostile `/sys` trees (oversized files, garbage ids, look-alike names, directories in place of
  files, too many nodes) and hostile preference JSON are tested.
- The shipped table ends every family on a CPU path and puts CFA4's own engine before the
  supplementary paths in the codec families, FFmpeg 9 before GStreamer (tests in `table.rs`).

## Vendor-neutrality status (`coverage_matches_the_workspace`)

| family | vendor-neutral accelerator today | vendor-locked | planned vendor-neutral |
|---|---|---|---|
| display | wgpu (Vulkan) | — | — |
| codec_decode | VA-API (AMD, Intel, nouveau) | NVDEC | FFmpeg 9, GStreamer Vulkan Video |
| codec_encode | **none: AMD and Intel encode on the CPU** | NVENC | VA-API, FFmpeg 9, GStreamer VA |
| ai | **none: CPU (candle)** | — (planned: OpenVINO NPU, Burn CUDA) | Burn Vulkan |
| compute | **none: CPU** | — | wgpu compute |

The test fails when the catalogue changes, so this table and the code move together.

## History

`cfa4-accel` (#39) was merged into this crate (owner decision, 2026-10-25): its catalogue,
vendor-lock switch, coverage report and the hostile-input tests of its `/sys` probe live here now.

## Not yet

Registering the existing backends (NVDEC, VA-API, NVENC, wgpu, candle) through this crate is phase 1
of ADR 0004; until then nothing in CFA4 calls the router and behaviour is unchanged.
