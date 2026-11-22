# cfa4-store

CFA4's PostgreSQL client (CFA4-TASKSTORE-PG-001, phases PG-1 and PG-2; plan:
[`docs/plans/CFA4-TASKSTORE-PG-001.md`](../../docs/plans/CFA4-TASKSTORE-PG-001.md)). Layer L5 because it does
socket operating-system I/O: a Unix-domain socket to PostgreSQL.

- **Driver:** `tokio-postgres` with the `deadpool-postgres` pool (owner decision 2026-11-11, shared
  with the BacklotHub). PostgreSQL runs as its own process; CFA4 is only a client, in pure Rust.
- **`Store::connect`:** opens the pool and checks one connection. A missing server is
  `StoreError::Unavailable` with code `DB_SERVER_MISSING`; a refused or timed-out connection is
  `DB_SERVER_STOPPED`. The hint points at `cfa4-db-setup`. Editing, playback and saving never need it.
- **`cfa4-db-setup`:** idempotent root command. It creates the PostgreSQL 18 cluster `cfa4` (never
  the default cluster), listens on a Unix socket only, writes the `cfa4map` peer map (OS user →
  `cfa4_login`) and records the assigned port in `<config dir>/cfa4/db.toml`. A second run changes
  nothing. `StoreConfig::load` reads that file; the login role defaults to `cfa4_login`.
- **Migrations** (`migrate`): embedded steps per component, each with an `up` and a `down`, applied
  in one transaction under an advisory lock and recorded in `cfa4_store.migration`;
  `Store::rollback` takes one step back.
- **`TaskBackend`** (`backend`): the queue's storage trait, implemented by `TaskStore` (PostgreSQL) and by `MemoryTaskStore` (a `cfa4_tasks::Queue` in memory, same rules, all-or-nothing steps). The memory store needs no server, so a machine without the cluster keeps a working queue and the tests of everything above run without PostgreSQL; one contract test runs against both.
- **`tasks` schema** (`TaskStore`): the `cfa4-tasks` queue in PostgreSQL. Each operation locks the
  rows it needs (`FOR UPDATE`; the scheduler step with `SKIP LOCKED`), runs the queue's own rule
  (`cfa4_tasks::Queue`), writes back the changed tasks (version-checked), their transitions
  (`tasks.transition`) and one Event Fabric outbox row per transition (`tasks.outbox`,
  `cfa4.execution.job.<state>`), and commits. The resource manager changes on a copy that replaces
  the caller's only after the commit. New tasks `NOTIFY cfa4_tasks`.
- **`ai` schema** (`AiStore`, PG-3): `ai.job` (the AI part of one task: capability, implementation, `route_decision`, the project versions Preview and Apply used), `ai.model` and `ai.model_use` (the one model registry, D-7: SHA-256, family, revision, variant, format, source, the four licence parts, path, state) and `ai.license_acceptance` (the user's one-time acceptance of a model's terms, for everywhere or one project). CFA4 ships no models: weights stay files, the user downloads them and accepts the terms first; `license_accepted` is the gate and there is no silent fallback.
- **`cache` schema** (`CacheIndex`, PG-3): the index of the content-addressed cache (key, file, size, tier, last use). `evict_to` removes the least recently used entries until a byte budget fits and returns them; the caller deletes the files.
- Ids of the `ai` schema are UUIDs the caller chooses (UUIDv7 by convention), times are `timestamptz` in the database and UTC milliseconds in the API (P5). The `tasks` schema still has `text` ids and millisecond columns until the D-5 migration step.

Tests start a throw-away cluster (`initdb` / `pg_ctl` in a temporary directory, Unix socket only;
as root through `runuser -u postgres`). Without PostgreSQL binaries they print a note and skip
(`CFA4_PG_BIN` points at another `bin` directory). `CFA4_PG_REQUIRE=1` makes that skip a failure
and requires PostgreSQL 18. `CFA4_DB_SETUP=1` runs `cfa4-db-setup` twice against a real cluster.
