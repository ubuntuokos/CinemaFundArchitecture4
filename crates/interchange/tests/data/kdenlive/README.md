# Kdenlive OTIO test data

OpenTimelineIO documents from Kdenlive's own OTIO export/import tests, used by
`crates/interchange/tests/kdenlive_otio.rs` (Kdenlive parity K1, first step of `.kdenlive` import).
They are JSON timelines only; the media they name is not included.

- **Source:** `tests/dataset/*.otio` in Kdenlive (https://invent.kde.org/multimedia/kdenlive), copied
  unchanged from the mirror `ubuntuokos/kdenlive` at commit `364e2d5078b1b4cf2c44839bf250780c6a14f98d`.
- **Authors:** Jean-Baptiste Mardelle, Eric Jiang (per Kdenlive's `REUSE.toml`).
- **Licence:** CC0-1.0 (`REUSE.toml`: `path = "tests/dataset/**"`).
- **Local modifications:** none.
