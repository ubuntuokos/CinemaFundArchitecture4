"""Installing the audit schema (AU-1): the SQL migration plus the genesis event.

Run once by the CFA4 database setup, as a role that may create roles and schemas (never by the application).
The application connects as an ordinary login role that belongs to ``cfa4_audit_writer`` and
``cfa4_audit_reader`` and can call only the functions of the schema (G2-01 section 5).
"""

from __future__ import annotations

import uuid
from datetime import datetime
from importlib import resources

import psycopg
from psycopg.types.json import Jsonb

from .canonical import GENESIS_PREV_HASH, event_hash
from .errors import AuditError
from .event import ActorKind, AuditEvent, Outcome, uuid7

SCHEMA_VERSION = 1
_GENESIS_SUBJECT = uuid.UUID("00000000-0000-7000-8000-000000000001")


class SchemaInstallFailed(AuditError):
    """The migration could not be applied (the database rolled it back)."""

    code = "AUDIT_SCHEMA_INSTALL_FAILED"


def migration_sql() -> str:
    """The text of migration 1."""
    return resources.files("cfa4_audit").joinpath("migrations/0001_audit_schema.sql").read_text(encoding="utf-8")


def is_installed(conn: psycopg.Connection) -> bool:
    """Whether the schema and its version row exist."""
    with conn.cursor() as cur:
        cur.execute("SELECT to_regclass('cfa4_audit.schema_version') IS NOT NULL")
        row = cur.fetchone()
        if row is None or not row[0]:
            return False
        cur.execute("SELECT 1 FROM cfa4_audit.schema_version WHERE version = %s", (SCHEMA_VERSION,))
        return cur.fetchone() is not None


def install(conn: psycopg.Connection) -> bool:
    """Apply migration 1 and append the genesis event in ONE transaction; True if it did something.

    Idempotent: when the version row exists nothing changes. On any error the transaction is rolled back, so a
    half-installed schema is never left behind.
    """
    if is_installed(conn):
        return False
    try:
        with conn.transaction():
            conn.execute(migration_sql())  # a multi-statement script without parameters
            _append_genesis(conn)
            conn.execute("INSERT INTO cfa4_audit.schema_version (version) VALUES (%s)", (SCHEMA_VERSION,))
    except psycopg.Error as exc:
        raise SchemaInstallFailed(str(exc).splitlines()[0]) from exc
    return True


def _append_genesis(conn: psycopg.Connection) -> None:
    with conn.cursor() as cur:
        cur.execute("SELECT clock_timestamp()")
        row = cur.fetchone()
        assert row is not None
        now: datetime = row[0]
        cur.execute("SELECT system_identifier::text FROM pg_control_system()")
        cluster = cur.fetchone()
        cur.execute("SELECT oid::bigint FROM pg_database WHERE datname = current_database()")
        database = cur.fetchone()
        assert cluster is not None and database is not None
        genesis = AuditEvent(
            app="cfa4-audit",
            action="audit.chain.genesis",
            actor_kind=ActorKind.SYSTEM,
            actor_ref="cfa4-audit",
            subject_type="audit_chain",
            subject_id=_GENESIS_SUBJECT,
            outcome=Outcome.SUCCESS,
            occurred_at=now,
            event_id=uuid7(),
            scope={"cluster_id": cluster[0], "database_oid": database[0]},
        )
        body = genesis.hashable(seq=1, recorded_at=now, prev_hash=GENESIS_PREV_HASH)
        digest = event_hash(body)
        cur.execute(
            "SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)",
            (Jsonb(body), digest, GENESIS_PREV_HASH, 1),
        )
