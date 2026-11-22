-- CFA4 common audit trail, migration 1 (CFA4-AUDIT-001 AU-1, G2-01 sections 5-7).
-- Runs as a role that may CREATE ROLE and CREATE SCHEMA (the installer, not the application).
-- The genesis event is appended by cfa4_audit.migrate.install() after this script, because its
-- hash is computed in Python (the canonical form is not reproduced in PL/pgSQL, G2-01 section 7).

-- Roles (cluster-wide, created once). The OS-user login role is added to the writer and reader roles by the
-- CFA4 database setup, not here.
DO $roles$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'cfa4_audit_owner') THEN
        CREATE ROLE cfa4_audit_owner NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'cfa4_audit_writer') THEN
        CREATE ROLE cfa4_audit_writer NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'cfa4_audit_reader') THEN
        CREATE ROLE cfa4_audit_reader NOLOGIN;
    END IF;
END
$roles$;

CREATE SCHEMA cfa4_audit AUTHORIZATION cfa4_audit_owner;
SET LOCAL ROLE cfa4_audit_owner;

CREATE TYPE cfa4_audit.actor_kind AS ENUM ('os_user', 'identity', 'system', 'agent');
CREATE TYPE cfa4_audit.outcome    AS ENUM ('success', 'rejected', 'failed');

-- One row: the end of the chain. recorded_at is added to the G2 sketch so that append() can refuse a
-- recorded_at that goes back in time (a monthly partition must be one contiguous stretch of the chain).
CREATE TABLE cfa4_audit.chain_head (
    id          smallint    PRIMARY KEY CHECK (id = 1),
    seq         bigint      NOT NULL,
    hash        bytea       NOT NULL CHECK (octet_length(hash) = 32),
    recorded_at timestamptz NOT NULL
);
INSERT INTO cfa4_audit.chain_head (id, seq, hash, recorded_at)
VALUES (1, 0, '\x0000000000000000000000000000000000000000000000000000000000000000', '-infinity');

CREATE TABLE cfa4_audit.event (
    seq            bigint      NOT NULL,
    event_id       uuid        NOT NULL,
    schema_ver     smallint    NOT NULL DEFAULT 1,
    occurred_at    timestamptz NOT NULL,
    recorded_at    timestamptz NOT NULL,
    app            text        NOT NULL,
    action         text        NOT NULL CHECK (action ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$'),
    command_id     text,
    actor_kind     cfa4_audit.actor_kind NOT NULL,
    actor_ref      text        NOT NULL,
    on_behalf_of   text,
    subject_type   text        NOT NULL,
    subject_id     uuid        NOT NULL,
    scope          jsonb       NOT NULL DEFAULT '{}',
    change         jsonb,
    outcome        cfa4_audit.outcome NOT NULL,
    reason_code    text        CHECK (reason_code ~ '^[A-Z][A-Z0-9_]*$'),
    correlation_id uuid,
    causation_id   uuid,
    trace_id       text,
    prev_hash      bytea       NOT NULL CHECK (octet_length(prev_hash) = 32),
    hash           bytea       NOT NULL CHECK (octet_length(hash) = 32),
    PRIMARY KEY (seq, recorded_at),
    UNIQUE (event_id, recorded_at)
) PARTITION BY RANGE (recorded_at);

CREATE INDEX event_subject_idx  ON cfa4_audit.event (subject_type, subject_id, seq);
CREATE INDEX event_actor_idx    ON cfa4_audit.event (actor_kind, actor_ref, seq);
CREATE INDEX event_action_idx   ON cfa4_audit.event (action, seq);
CREATE INDEX event_occurred_idx ON cfa4_audit.event (occurred_at);
CREATE INDEX event_scope_idx    ON cfa4_audit.event USING gin (scope jsonb_path_ops);

-- Safety net: if the monthly partition is missing the write does not fail (ensure_partitions moves the rows).
CREATE TABLE cfa4_audit.event_default PARTITION OF cfa4_audit.event DEFAULT;

-- Personal values live here, outside the chain (SEC-06). The salt of the chain-side digest is stored with the
-- detail and is erased with it, after which the digest cannot be reversed (G2-01 section 6).
CREATE TABLE cfa4_audit.personal_detail (
    event_id     uuid        PRIMARY KEY,
    subject_type text        NOT NULL,
    subject_id   uuid        NOT NULL,
    detail       jsonb,
    salt         bytea       CHECK (salt IS NULL OR octet_length(salt) = 16),
    erased_at    timestamptz,
    CHECK ((erased_at IS NULL) = (detail IS NOT NULL AND salt IS NOT NULL))
);
CREATE INDEX personal_detail_subject_idx ON cfa4_audit.personal_detail (subject_type, subject_id);

CREATE TABLE cfa4_audit.checkpoint (
    seq         bigint      PRIMARY KEY,
    hash        bytea       NOT NULL CHECK (octet_length(hash) = 32),
    taken_at    timestamptz NOT NULL,
    file_path   text        NOT NULL
);

CREATE TABLE cfa4_audit.schema_version (
    version     integer     PRIMARY KEY,
    applied_at  timestamptz NOT NULL DEFAULT now()
);

-- Append-only: UPDATE, DELETE and TRUNCATE are refused for everyone, the owner included (AUDIT_APPEND_ONLY).
CREATE FUNCTION cfa4_audit.reject_modify() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog AS $fn$
BEGIN
    RAISE EXCEPTION 'AUDIT_APPEND_ONLY: the audit trail cannot be changed (% on %)', TG_OP, TG_TABLE_NAME
        USING ERRCODE = 'AU001';
END
$fn$;

CREATE TRIGGER event_no_update_delete BEFORE UPDATE OR DELETE ON cfa4_audit.event
    FOR EACH ROW EXECUTE FUNCTION cfa4_audit.reject_modify();
CREATE TRIGGER event_no_truncate BEFORE TRUNCATE ON cfa4_audit.event
    FOR EACH STATEMENT EXECUTE FUNCTION cfa4_audit.reject_modify();
CREATE TRIGGER event_default_no_truncate BEFORE TRUNCATE ON cfa4_audit.event_default
    FOR EACH STATEMENT EXECUTE FUNCTION cfa4_audit.reject_modify();

-- Takes the chain lock for the rest of the caller's transaction and returns the end of the chain plus the
-- time to use as recorded_at (taken AFTER the lock, so recorded_at never goes back, even with concurrent
-- writers; G2-01 section 7 said now(), which is the transaction start and can be older than the lock).
CREATE FUNCTION cfa4_audit.lock_head()
RETURNS TABLE (seq bigint, hash bytea, head_recorded_at timestamptz, now_utc timestamptz)
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, cfa4_audit AS $fn$
    SELECT h.seq, h.hash, h.recorded_at, clock_timestamp()
    FROM cfa4_audit.chain_head h WHERE h.id = 1 FOR UPDATE
$fn$;

CREATE FUNCTION cfa4_audit.append(p_event jsonb, p_hash bytea, p_prev_hash bytea, p_seq bigint) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, cfa4_audit AS $fn$
DECLARE
    head record;
    v_recorded timestamptz := (p_event->>'recorded_at')::timestamptz;
BEGIN
    SELECT h.seq, h.hash, h.recorded_at INTO head FROM cfa4_audit.chain_head h WHERE h.id = 1 FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'AUDIT_CHAIN_BROKEN: the chain head is missing' USING ERRCODE = 'AU002';
    END IF;
    IF p_seq <> head.seq + 1
       OR p_prev_hash <> head.hash
       OR (p_event->>'seq')::bigint <> p_seq
       OR p_event->>'prev_hash' <> encode(p_prev_hash, 'hex')
       OR v_recorded < head.recorded_at THEN
        RAISE EXCEPTION 'AUDIT_CHAIN_CONFLICT: expected seq % after %, got seq %',
            head.seq + 1, encode(head.hash, 'hex'), p_seq USING ERRCODE = 'AU003';
    END IF;
    INSERT INTO cfa4_audit.event (
        seq, event_id, schema_ver, occurred_at, recorded_at, app, action, command_id, actor_kind, actor_ref,
        on_behalf_of, subject_type, subject_id, scope, change, outcome, reason_code, correlation_id,
        causation_id, trace_id, prev_hash, hash)
    VALUES (
        p_seq, (p_event->>'event_id')::uuid, (p_event->>'schema_ver')::smallint,
        (p_event->>'occurred_at')::timestamptz, v_recorded, p_event->>'app', p_event->>'action',
        p_event->>'command_id', (p_event->>'actor_kind')::cfa4_audit.actor_kind, p_event->>'actor_ref',
        p_event->>'on_behalf_of', p_event->>'subject_type', (p_event->>'subject_id')::uuid,
        COALESCE(p_event->'scope', '{}'::jsonb), NULLIF(p_event->'change', 'null'::jsonb),
        (p_event->>'outcome')::cfa4_audit.outcome, p_event->>'reason_code',
        (p_event->>'correlation_id')::uuid, (p_event->>'causation_id')::uuid, p_event->>'trace_id',
        p_prev_hash, p_hash);
    UPDATE cfa4_audit.chain_head SET seq = p_seq, hash = p_hash, recorded_at = v_recorded WHERE id = 1;
    -- A row that landed in the default partition means the monthly partition is missing.
    IF EXISTS (SELECT 1 FROM cfa4_audit.event_default d WHERE d.seq = p_seq AND d.recorded_at = v_recorded) THEN
        RAISE WARNING 'AUDIT_PARTITION_DEFAULT_USED: no monthly partition for %', v_recorded;
    END IF;
END
$fn$;

-- Personal values (outside the chain). The salt is stored here with the detail, never in the chain.
CREATE FUNCTION cfa4_audit.write_personal(p_event_id uuid, p_subject_type text, p_subject_id uuid,
                                         p_detail jsonb, p_salt bytea) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, cfa4_audit AS $fn$
    INSERT INTO cfa4_audit.personal_detail (event_id, subject_type, subject_id, detail, salt)
    VALUES (p_event_id, p_subject_type, p_subject_id, p_detail, p_salt)
$fn$;

-- SEC-06: erases the detail and the salt of every row of one subject; the chain is not touched. The caller (the
-- business command that performs the erasure) records the audit.personal.erased event.
CREATE FUNCTION cfa4_audit.erase_personal(p_subject_type text, p_subject_id uuid) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, cfa4_audit AS $fn$
DECLARE n bigint;
BEGIN
    UPDATE cfa4_audit.personal_detail
       SET detail = NULL, salt = NULL, erased_at = now()
     WHERE subject_type = p_subject_type AND subject_id = p_subject_id AND erased_at IS NULL;
    GET DIAGNOSTICS n = ROW_COUNT;
    RETURN n;
END
$fn$;

-- Monthly partitions by recorded_at, from the current month to p_months_ahead months ahead (maintenance, run
-- as cfa4_audit_owner). Rows that landed in the default partition meanwhile are moved into the new partition
-- (the one case where the same rows are copied: the chain content is unchanged).
CREATE FUNCTION cfa4_audit.ensure_partitions(p_months_ahead integer DEFAULT 2) RETURNS integer
LANGUAGE plpgsql SET search_path = pg_catalog, cfa4_audit AS $fn$
DECLARE
    first_month timestamptz := date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC';
    m timestamptz;
    part text;
    created integer := 0;
    moved bigint;
BEGIN
    IF p_months_ahead < 0 OR p_months_ahead > 24 THEN
        RAISE EXCEPTION 'AUDIT_PARTITION_RANGE: months ahead must be 0..24';
    END IF;
    FOR i IN 0..p_months_ahead LOOP
        m := first_month + make_interval(months => i);
        part := 'event_' || to_char(m AT TIME ZONE 'UTC', 'YYYY_MM');
        IF to_regclass('cfa4_audit.' || quote_ident(part)) IS NOT NULL THEN
            CONTINUE;
        END IF;
        IF EXISTS (SELECT 1 FROM cfa4_audit.event_default
                   WHERE recorded_at >= m AND recorded_at < m + interval '1 month') THEN
            -- Detach the default partition (its cloned append-only trigger goes with the detach), move its rows of
            -- this month, attach it again (the trigger is cloned back).
            ALTER TABLE cfa4_audit.event DETACH PARTITION cfa4_audit.event_default;
            EXECUTE format('CREATE TABLE cfa4_audit.%I PARTITION OF cfa4_audit.event FOR VALUES FROM (%L) TO (%L)',
                           part, m, m + interval '1 month');
            EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON cfa4_audit.%I FOR EACH STATEMENT '
                           'EXECUTE FUNCTION cfa4_audit.reject_modify()', part || '_no_truncate', part);
            INSERT INTO cfa4_audit.event
                SELECT * FROM cfa4_audit.event_default WHERE recorded_at >= m AND recorded_at < m + interval '1 month';
            GET DIAGNOSTICS moved = ROW_COUNT;
            DELETE FROM cfa4_audit.event_default WHERE recorded_at >= m AND recorded_at < m + interval '1 month';
            ALTER TABLE cfa4_audit.event ATTACH PARTITION cfa4_audit.event_default DEFAULT;
            RAISE NOTICE 'AUDIT_PARTITION_DEFAULT_USED: moved % rows into %', moved, part;
        ELSE
            EXECUTE format('CREATE TABLE cfa4_audit.%I PARTITION OF cfa4_audit.event FOR VALUES FROM (%L) TO (%L)',
                           part, m, m + interval '1 month');
            EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON cfa4_audit.%I FOR EACH STATEMENT '
                           'EXECUTE FUNCTION cfa4_audit.reject_modify()', part || '_no_truncate', part);
        END IF;
        created := created + 1;
    END LOOP;
    RETURN created;
END
$fn$;

SELECT cfa4_audit.ensure_partitions(2);

-- Rights: the writer may only call the functions, never touch a table; the reader reads the chain.
RESET ROLE;
REVOKE ALL ON SCHEMA cfa4_audit FROM PUBLIC;
GRANT USAGE ON SCHEMA cfa4_audit TO cfa4_audit_writer, cfa4_audit_reader;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA cfa4_audit FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cfa4_audit.lock_head() TO cfa4_audit_writer;
GRANT EXECUTE ON FUNCTION cfa4_audit.append(jsonb, bytea, bytea, bigint) TO cfa4_audit_writer;
GRANT EXECUTE ON FUNCTION cfa4_audit.write_personal(uuid, text, uuid, jsonb, bytea) TO cfa4_audit_writer;
GRANT EXECUTE ON FUNCTION cfa4_audit.erase_personal(text, uuid) TO cfa4_audit_writer;
GRANT SELECT ON cfa4_audit.event, cfa4_audit.chain_head, cfa4_audit.checkpoint TO cfa4_audit_reader;
