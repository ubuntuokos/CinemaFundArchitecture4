# Kdenlive project test data (`.kdenlive`)

Kdenlive documents from Kdenlive's own test dataset, used by
`crates/interchange/tests/mlt_kdenlive.rs` (Kdenlive plan K1, shared MLT reader core with Shotcut SC-5.1).
The media they name is not included; missing media is part of what the tests check.

- **Source:** `tests/dataset/{av,test-mix,test-missing,test-keyframes,test-nesting}.kdenlive` in Kdenlive
  (https://invent.kde.org/multimedia/kdenlive), copied unchanged from the mirror `ubuntuokos/kdenlive`
  (`root=` attributes keep the upstream developer's directory; the importer does not need it to exist).
- **Authors:** Jean-Baptiste Mardelle, Eric Jiang (per Kdenlive's `REUSE.toml`).
- **Licence:** CC0-1.0 (`REUSE.toml`: `path = "tests/dataset/**"`).
- **Local modifications:** none.
