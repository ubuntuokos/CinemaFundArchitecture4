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
- Ids of the `ai` schema are UUIDs the caller chooses (UUIDv7 by convention), times are `timestamptz` in the database and UTC milliseconds in the API (P5). The `tasks` schema (migration v7, D-5) has `uuid` task ids: the store accepts only lowercase canonical UUIDs (UUIDv7 by convention) and refuses anything else with a queue error; the step refuses to convert a table that still holds ids that are not UUIDs. `tasks.session_task` and `ai.job.task_id` keep text ids.
- **`tasks` schedule, stages, estimates and runs** (migration v4): `tasks.schedule` (start mode, time, IANA zone, missed-start rule; kept equal to the task on submit, reschedule and next-slot moves), `tasks.checkpoint` (one row per finished stage with the cache key of its result: `TaskStore::checkpoint_keyed`), `tasks.estimate` (`record_estimate`, `current_estimate`) and `tasks.run_history` (`record_run`, `runs_for`: what the estimator learns from).
- **Outbox and relay** (`outbox`, D-3): every state change writes, in its own transaction, one Event Fabric Canonical Envelope (UUIDv7 `event_id` from `tasks.uuid7()`, the six `cfa4.execution.job.*` subjects via `subject_for`, the detailed state and the task's own sequence `entity_seq` in the payload; creation is sequence 0). `Store::outbox().relay_once(limit, publish)` hands unpublished events to a publisher at least once (`FOR UPDATE SKIP LOCKED`, only confirmed rows are marked, same `event_id` on every repeat); `pending` and `prune` as well. The Event Fabric client itself is not in this crate.
- **Session record** (`mirror`, PG-001 §6): the engine keeps its queue in memory and never waits for the database. `SessionMirror::start(config, startup)` connects, migrates and starts a writer thread; it is the engine's `TaskJournal` (`TaskHost::set_journal`), so every task change goes through a bounded channel (a full channel drops and counts) and is written in batches to `tasks.session_task` (one row per session and task, never read by the database scheduler). A later start closes the open rows of dead sessions (`/proc/<pid>`) as `interrupted` and prunes rows older than 30 days. `cfa4-cli serve` starts it; with no server it says so once and the tasks run as before (`--no-db` skips it).
- **Wake-ups** (`listen`): `Store::listen()` opens a connection of its own that `LISTEN`s on `cfa4_tasks`; `Listener::wait(timeout)` returns `Wake::Changed(ids)`, `Wake::Timeout` or `Wake::Lost`. Submit and every state change notify in their own transaction, so the wake-up comes after the commit.

Tests start a throw-away cluster (`initdb` / `pg_ctl` in a temporary directory, Unix socket only;
as root through `runuser -u postgres`). Without PostgreSQL binaries they print a note and skip
(`CFA4_PG_BIN` points at another `bin` directory). `CFA4_PG_REQUIRE=1` makes that skip a failure
and requires PostgreSQL 18. `CFA4_DB_SETUP=1` runs `cfa4-db-setup` twice against a real cluster.
