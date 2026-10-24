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
  (accel nodes: Intel `intel_vpu`, AMD `amdxdna`) Linux lists in `/sys`, with their PCI vendor.
  Nothing is opened or loaded; backends add free-memory figures when they measure them.
- **Choice** (`Router::choose`): the first route whose backend is registered and enabled, whose
  device is present and allowed by the family's preference, which the backend accepts, and which
  fits the memory budget. Every route passed over is kept with its `Reason`.
- **Fallback** (`Router::next_after`): the route after a failed one, for work that can restart.
- **Preferences** (`Preferences`): per family Automatic, one device (with the CPU as last resort)
  or CPU only. Hardware encoding stays opt-in: the default for `codec_encode` is CPU only.

## Guarantees

- No `unsafe`, no `unwrap` / `expect` / `panic!` outside tests.
- Every input is bounded: identifiers (64 bytes, `a-z 0-9 . _ - :`), the table file (256 KiB,
  32 backends per list, 1024 task rows), `/sys` attributes (4 KiB), devices (64), backend reasons
  (256 bytes).
- The shipped table ends every family on a CPU path and puts CFA4's own engine before the
  GStreamer paths in the codec families (tests in `table.rs`).

## Not yet

Registering the existing backends (NVDEC, VA-API, NVENC, wgpu, candle) through this crate is phase 1
of ADR 0004; until then nothing in CFA4 calls the router and behaviour is unchanged.
