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

Tests start a throw-away cluster (`initdb` / `pg_ctl` in a temporary directory, Unix socket only;
as root through `runuser -u postgres`). Without PostgreSQL binaries they print a note and skip
(`CFA4_PG_BIN` points at another `bin` directory). `CFA4_PG_REQUIRE=1` makes that skip a failure
and requires PostgreSQL 18. `CFA4_DB_SETUP=1` runs `cfa4-db-setup` twice against a real cluster.
