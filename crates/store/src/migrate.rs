//! Migrations: one way in every mode, one step back (PG-001 §9).
//!
//! Each component (`tasks`, later `ai`, `cache`, `mi`) has an ordered list of steps; a step is
//! an `up` and a `down` SQL text applied in one transaction together with its row in
//! `cfa4_store.migration`. The steps are embedded in the binary, never read from disk.

use deadpool_postgres::Object;

use crate::role;
use crate::{Store, StoreError};

pub struct Step {
    pub version: u32,
    pub up: &'static str,
    pub down: &'static str,
}

/// Every component and its steps, in the order they are applied.
pub fn components() -> &'static [(&'static str, &'static [Step])] {
    &[("tasks", TASKS), ("ai", AI), ("cache", CACHE)]
}

const TASKS: &[Step] = &[
    Step {
        version: 1,
        up: r#"
CREATE SCHEMA tasks;
-- one row per task: the indexed columns the scheduler selects on, and the whole task as the queue
-- keeps it (`doc`); times are UTC milliseconds like `cfa4_tasks::UtcMs`
CREATE TABLE tasks.task (
    id text PRIMARY KEY CHECK (length(id) BETWEEN 1 AND 256),
    state text NOT NULL,
    class_rank smallint NOT NULL,
    priority smallint NOT NULL,
    submitted_at bigint NOT NULL,
    start_at bigint,
    lease_owner text,
    lease_until bigint,
    version bigint NOT NULL,
    doc jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX task_open ON tasks.task (class_rank, priority DESC, submitted_at)
    WHERE state IN ('scheduled', 'queued', 'waiting', 'running');
-- append-only state log
CREATE TABLE tasks.transition (
    task_id text NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    seq bigint NOT NULL,
    from_state text NOT NULL,
    to_state text NOT NULL,
    at bigint NOT NULL,
    by_actor text NOT NULL,
    reason text NOT NULL,
    PRIMARY KEY (task_id, seq)
);
-- Event Fabric outbox (cfa4.execution.job.*), published by a separate relay
CREATE TABLE tasks.outbox (
    id bigserial PRIMARY KEY,
    task_id text NOT NULL,
    subject text NOT NULL,
    event jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    published_at timestamptz
);
CREATE INDEX outbox_unpublished ON tasks.outbox (id) WHERE published_at IS NULL;
"#,
        down: "DROP SCHEMA tasks CASCADE;",
    },
    Step {
        version: 2,
        // the scheduler also expires the lease of a task whose worker died while cancelling (ACS-7)
        up: "DROP INDEX tasks.task_open;
CREATE INDEX task_open ON tasks.task (class_rank, priority DESC, submitted_at)
    WHERE state IN ('scheduled', 'queued', 'waiting', 'running', 'cancelling');",
        down: "DROP INDEX tasks.task_open;
CREATE INDEX task_open ON tasks.task (class_rank, priority DESC, submitted_at)
    WHERE state IN ('scheduled', 'queued', 'waiting', 'running');",
    },
    Step {
        version: 3,
        // P5: wall-clock times are `timestamptz`. The task ids stay `text`: the queue's ids are chosen by
        // its callers (`export-1`, ...) and are not UUIDs, so the uuid half of D-5 is not applied here.
        up: "ALTER TABLE tasks.task ALTER COLUMN submitted_at TYPE timestamptz USING to_timestamp(submitted_at / 1000.0);
ALTER TABLE tasks.task ALTER COLUMN start_at TYPE timestamptz USING to_timestamp(start_at / 1000.0);
ALTER TABLE tasks.task ALTER COLUMN lease_until TYPE timestamptz USING to_timestamp(lease_until / 1000.0);
ALTER TABLE tasks.transition ALTER COLUMN at TYPE timestamptz USING to_timestamp(at / 1000.0);",
        down: "ALTER TABLE tasks.task ALTER COLUMN submitted_at TYPE bigint USING (extract(epoch from submitted_at) * 1000)::bigint;
ALTER TABLE tasks.task ALTER COLUMN start_at TYPE bigint USING (extract(epoch from start_at) * 1000)::bigint;
ALTER TABLE tasks.task ALTER COLUMN lease_until TYPE bigint USING (extract(epoch from lease_until) * 1000)::bigint;
ALTER TABLE tasks.transition ALTER COLUMN at TYPE bigint USING (extract(epoch from at) * 1000)::bigint;",
    },
    Step {
        version: 4,
        // PG-001 §4.3: the start rule, finished stages, estimates and finished runs of a task
        up: r#"
CREATE TABLE tasks.schedule (
    task_id text PRIMARY KEY REFERENCES tasks.task (id) ON DELETE CASCADE,
    mode text NOT NULL CHECK (mode IN ('now', 'resource', 'at', 'at_resource')),
    start_at timestamptz,
    time_zone text CHECK (length(time_zone) <= 64),
    if_missed text NOT NULL CHECK (if_missed IN ('delay', 'next_slot', 'ask'))
);
INSERT INTO tasks.schedule (task_id, mode, start_at, time_zone, if_missed)
    SELECT id,
           CASE doc #>> '{start,mode}' WHEN 'whenResources' THEN 'resource' WHEN 'at' THEN 'at' WHEN 'atWithResources' THEN 'at_resource' ELSE 'now' END,
           start_at, doc ->> 'timeZone',
           CASE doc ->> 'missed' WHEN 'nextSlot' THEN 'next_slot' WHEN 'askUser' THEN 'ask' ELSE 'delay' END
    FROM tasks.task;
CREATE TABLE tasks.checkpoint (
    task_id text NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    stage bigint NOT NULL CHECK (stage >= 0),
    cache_key text CHECK (length(cache_key) BETWEEN 1 AND 256),
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, stage)
);
CREATE TABLE tasks.estimate (
    id bigserial PRIMARY KEY,
    task_id text NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    min_s bigint NOT NULL CHECK (min_s >= 0),
    max_s bigint NOT NULL CHECK (max_s >= min_s),
    confidence text NOT NULL CHECK (confidence IN ('low', 'medium')),
    basis text NOT NULL CHECK (basis IN ('measurement', 'history', 'manifest')),
    at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX estimate_task ON tasks.estimate (task_id, id DESC);
-- what finished runs really cost; the estimator learns from it
CREATE TABLE tasks.run_history (
    id bigserial PRIMARY KEY,
    task_id text NOT NULL,
    implementation text NOT NULL CHECK (length(implementation) BETWEEN 1 AND 128),
    path text NOT NULL CHECK (length(path) <= 128),
    hardware text NOT NULL CHECK (length(hardware) <= 256),
    input_bytes bigint NOT NULL CHECK (input_bytes >= 0),
    duration_ms bigint NOT NULL CHECK (duration_ms >= 0),
    peak_ram_bytes bigint NOT NULL CHECK (peak_ram_bytes >= 0),
    peak_vram_bytes bigint NOT NULL CHECK (peak_vram_bytes >= 0),
    finished_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX run_history_impl ON tasks.run_history (implementation, hardware, id DESC);
"#,
        down: "DROP TABLE tasks.run_history, tasks.estimate, tasks.checkpoint, tasks.schedule;",
    },
    Step {
        version: 5,
        // D-3: the outbox row carries the whole Event Fabric Canonical Envelope with a UUIDv7
        // `event_id` that never changes on a repeat (EF §5.3); `tasks.uuid7()` makes one in SQL
        up: r#"
CREATE FUNCTION tasks.uuid7() RETURNS uuid LANGUAGE sql VOLATILE AS $f$
    SELECT encode(
        set_bit(set_bit(
            overlay(uuid_send(gen_random_uuid()) PLACING substring(int8send((extract(epoch FROM clock_timestamp()) * 1000)::bigint) FROM 3) FROM 1 FOR 6),
            52, 1), 53, 1),
        'hex')::uuid
$f$;
ALTER TABLE tasks.outbox ADD COLUMN event_id uuid NOT NULL DEFAULT tasks.uuid7();
ALTER TABLE tasks.outbox ADD CONSTRAINT outbox_event_id_key UNIQUE (event_id);
"#,
        down: "ALTER TABLE tasks.outbox DROP COLUMN event_id;
DROP FUNCTION tasks.uuid7();",
    },
    Step {
        version: 6,
        // the engine's in-memory queue, written behind by the application: one row per task of a
        // session, never read by the scheduler (`tasks.task` stays the queue of the database)
        up: r#"
CREATE TABLE tasks.session_task (
    session text NOT NULL CHECK (length(session) BETWEEN 1 AND 64),
    id text NOT NULL CHECK (length(id) BETWEEN 1 AND 256),
    state text NOT NULL CHECK (length(state) BETWEEN 1 AND 32),
    version bigint NOT NULL CHECK (version >= 0),
    doc jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (session, id)
);
CREATE INDEX session_task_updated ON tasks.session_task (updated_at);
"#,
        down: "DROP TABLE tasks.session_task;",
    },
];

const AI: &[Step] = &[Step {
    version: 1,
    up: r#"
CREATE SCHEMA ai;
-- the AI part of one queue task: what was asked, which implementation ran, how the route was
-- decided, and the project version Preview and Apply were made against (no foreign key into
-- `tasks`: every schema has one migration owner, PG-001 §6)
CREATE TABLE ai.job (
    id uuid PRIMARY KEY,
    task_id text NOT NULL UNIQUE CHECK (length(task_id) BETWEEN 1 AND 256),
    capability text NOT NULL CHECK (length(capability) BETWEEN 1 AND 128),
    implementation text NOT NULL CHECK (length(implementation) BETWEEN 1 AND 128),
    route_decision jsonb NOT NULL,
    preview_project_version text CHECK (length(preview_project_version) <= 256),
    apply_project_version text CHECK (length(apply_project_version) <= 256),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX job_capability ON ai.job (capability, created_at DESC);
-- the one model registry (D-7); the weights are files, never rows (CFA4 does not ship models)
CREATE TABLE ai.model (
    id uuid PRIMARY KEY,
    sha256 text NOT NULL UNIQUE CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    family text NOT NULL CHECK (length(family) BETWEEN 1 AND 128),
    revision text NOT NULL CHECK (length(revision) <= 128),
    variant text NOT NULL CHECK (length(variant) <= 128),
    format text NOT NULL CHECK (length(format) BETWEEN 1 AND 64),
    source text NOT NULL CHECK (length(source) <= 1024),
    license_code text NOT NULL CHECK (length(license_code) <= 256),
    license_weights text NOT NULL CHECK (length(license_weights) <= 256),
    license_training_data text NOT NULL CHECK (length(license_training_data) <= 256),
    license_output text NOT NULL CHECK (length(license_output) <= 256),
    path text CHECK (length(path) <= 4096),
    state text NOT NULL CHECK (state IN ('known', 'downloading', 'ready', 'broken', 'removed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE ai.model_use (
    model_id uuid NOT NULL REFERENCES ai.model (id) ON DELETE CASCADE,
    capability text NOT NULL CHECK (length(capability) BETWEEN 1 AND 128),
    PRIMARY KEY (model_id, capability)
);
-- the user's one-time acceptance of a model's terms; `project_ref` is empty for "everywhere"
CREATE TABLE ai.license_acceptance (
    id uuid PRIMARY KEY,
    implementation text NOT NULL CHECK (length(implementation) BETWEEN 1 AND 128),
    weight_sha256 text NOT NULL CHECK (weight_sha256 ~ '^[0-9a-f]{64}$'),
    project_ref text NOT NULL DEFAULT '' CHECK (length(project_ref) <= 4096),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    commercial_warning text NOT NULL DEFAULT '' CHECK (length(commercial_warning) <= 4096),
    UNIQUE (implementation, weight_sha256, project_ref)
);
"#,
    down: "DROP SCHEMA ai CASCADE;",
}];

const CACHE: &[Step] = &[Step {
    version: 1,
    up: r#"
CREATE SCHEMA cache;
-- the index of the content-addressed cache; the files stay on disk
CREATE TABLE cache.entry (
    key text PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 256),
    file text NOT NULL CHECK (length(file) BETWEEN 1 AND 4096),
    size_bytes bigint NOT NULL CHECK (size_bytes >= 0),
    tier text NOT NULL CHECK (tier IN ('preview', 'final')),
    last_used_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX entry_lru ON cache.entry (last_used_at, key);
"#,
    down: "DROP SCHEMA cache CASCADE;",
}];

const LEDGER: &str = "CREATE SCHEMA IF NOT EXISTS cfa4_store;
CREATE TABLE IF NOT EXISTS cfa4_store.migration (
    component text NOT NULL,
    version integer NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (component, version)
);";

/// Serialises migrations of concurrent CFA4 processes (a transaction-scoped advisory lock).
const LOCK: &str = "SELECT pg_advisory_xact_lock(4364100)";

async fn applied(client: &tokio_postgres::Transaction<'_>, component: &str) -> Result<u32, StoreError> {
    let row = client
        .query_one("SELECT COALESCE(MAX(version), 0) FROM cfa4_store.migration WHERE component = $1", &[&component])
        .await
        .map_err(StoreError::postgres)?;
    let v: i32 = row.try_get(0).map_err(StoreError::postgres)?;
    Ok(u32::try_from(v).unwrap_or(0))
}

const LEDGER_OWNER: &str = "cfa4_owner_cfa4_store";

pub(crate) async fn up(store: &Store) -> Result<Vec<(String, u32)>, StoreError> {
    let mut client = store.client().await?;
    let mut done = Vec::new();
    for (component, steps) in components() {
        for step in *steps {
            let tx = client.transaction().await.map_err(StoreError::postgres)?;
            role::assume(&tx, LEDGER_OWNER).await?;
            tx.batch_execute(LEDGER).await.map_err(StoreError::postgres)?;
            tx.execute(LOCK, &[]).await.map_err(StoreError::postgres)?;
            if applied(&tx, component).await? >= step.version {
                tx.rollback().await.map_err(StoreError::postgres)?;
                continue;
            }
            let owner = format!("cfa4_owner_{component}");
            role::assume(&tx, &owner).await?;
            let fail =
                |e: tokio_postgres::Error| StoreError::Migration { component: (*component).into(), version: step.version, detail: crate::postgres_detail(&e) };
            tx.batch_execute(step.up).await.map_err(fail)?;
            role::assume(&tx, LEDGER_OWNER).await?;
            let version = i32::try_from(step.version).unwrap_or(i32::MAX);
            tx.execute("INSERT INTO cfa4_store.migration (component, version) VALUES ($1, $2)", &[component, &version]).await.map_err(fail)?;
            tx.commit().await.map_err(fail)?;
            done.push(((*component).to_owned(), step.version));
        }
        ensure_app_privileges(&mut client, component).await?;
    }
    Ok(done)
}

/// Table rights for `cfa4_app_<component>` once that role and the schema exist. Idempotent, and
/// not a migration version: a cluster without the role model (the throw-away superuser) skips it.
async fn ensure_app_privileges(client: &mut Object, component: &str) -> Result<(), StoreError> {
    let app = format!("cfa4_app_{component}");
    let owner = format!("cfa4_owner_{component}");
    let tx = client.transaction().await.map_err(StoreError::postgres)?;
    let row = tx
        .query_one(
            "SELECT (SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)) AND (SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $2))",
            &[&app, &component],
        )
        .await
        .map_err(StoreError::postgres)?;
    let ready: bool = row.try_get(0).map_err(StoreError::postgres)?;
    if !ready {
        tx.rollback().await.map_err(StoreError::postgres)?;
        return Ok(());
    }
    role::assume(&tx, &owner).await?;
    let row = tx
        .query_one(
            "SELECT format('GRANT USAGE ON SCHEMA %I TO %I; GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA %I TO %I; GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA %I TO %I;', $1::text, $2::text, $1::text, $2::text, $1::text, $2::text)",
            &[&component, &app],
        )
        .await
        .map_err(StoreError::postgres)?;
    let sql: String = row.try_get(0).map_err(StoreError::postgres)?;
    if !sql.starts_with("GRANT ") || sql.matches(';').count() != 3 {
        return Err(StoreError::Query { detail: "refusing an unexpected GRANT statement".into() });
    }
    tx.batch_execute(&sql).await.map_err(StoreError::postgres)?;
    tx.commit().await.map_err(StoreError::postgres)?;
    Ok(())
}

pub(crate) async fn down(store: &Store, component: &str) -> Result<Option<u32>, StoreError> {
    let Some((_, steps)) = components().iter().find(|(c, _)| *c == component) else {
        return Err(StoreError::Query { detail: format!("unknown component {component}") });
    };
    let mut client = store.client().await?;
    let tx = client.transaction().await.map_err(StoreError::postgres)?;
    role::assume(&tx, LEDGER_OWNER).await?;
    tx.batch_execute(LEDGER).await.map_err(StoreError::postgres)?;
    tx.execute(LOCK, &[]).await.map_err(StoreError::postgres)?;
    let current = applied(&tx, component).await?;
    if current == 0 {
        tx.rollback().await.map_err(StoreError::postgres)?;
        return Ok(None);
    }
    let Some(step) = steps.iter().find(|s| s.version == current) else {
        return Err(StoreError::Migration { component: component.into(), version: current, detail: "applied by a newer CFA4: no down step here".into() });
    };
    let fail = |e: tokio_postgres::Error| StoreError::Migration { component: component.into(), version: current, detail: crate::postgres_detail(&e) };
    let owner = format!("cfa4_owner_{component}");
    role::assume(&tx, &owner).await?;
    tx.batch_execute(step.down).await.map_err(fail)?;
    role::assume(&tx, LEDGER_OWNER).await?;
    let version = i32::try_from(current).unwrap_or(i32::MAX);
    tx.execute("DELETE FROM cfa4_store.migration WHERE component = $1 AND version = $2", &[&component, &version]).await.map_err(fail)?;
    tx.commit().await.map_err(fail)?;
    Ok(Some(current))
}
