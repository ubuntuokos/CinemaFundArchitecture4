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

# (version, file in cfa4_audit/migrations). A new schema change is a new file and a new row; installed migrations are
# never edited.
MIGRATIONS: tuple[tuple[int, str], ...] = (
    (1, "0001_audit_schema.sql"),
    (2, "0002_audit_checkpoint_functions.sql"),
)
SCHEMA_VERSION = MIGRATIONS[-1][0]
_GENESIS_SUBJECT = uuid.UUID("00000000-0000-7000-8000-000000000001")


class SchemaInstallFailed(AuditError):
    """The migration could not be applied (the database rolled it back)."""

    code = "AUDIT_SCHEMA_INSTALL_FAILED"


def migration_sql(version: int = 1) -> str:
    """The text of one migration (default: migration 1)."""
    name = dict(MIGRATIONS)[version]
    return resources.files("cfa4_audit").joinpath(f"migrations/{name}").read_text(encoding="utf-8")


def installed_version(conn: psycopg.Connection) -> int:
    """The highest applied migration version; 0 when the schema is not installed."""
    with conn.cursor() as cur:
        cur.execute("SELECT to_regclass('cfa4_audit.schema_version') IS NOT NULL")
        row = cur.fetchone()
        if row is None or not row[0]:
            return 0
        cur.execute("SELECT coalesce(max(version), 0) FROM cfa4_audit.schema_version")
        row = cur.fetchone()
        return int(row[0]) if row is not None else 0


def is_installed(conn: psycopg.Connection) -> bool:
    """Whether the schema is installed at the current version."""
    return installed_version(conn) >= SCHEMA_VERSION


def install(conn: psycopg.Connection) -> bool:
    """Apply every pending migration (and, with migration 1, the genesis event); True if it did something.

    Each migration runs in ONE transaction together with its version row, so a half-applied migration is never left
    behind. Idempotent: at the current version nothing changes.
    """
    done = installed_version(conn)
    applied = False
    for version, _name in MIGRATIONS:
        if version <= done:
            continue
        try:
            with conn.transaction():
                conn.execute(migration_sql(version))  # a multi-statement script without parameters
                if version == 1:
                    _append_genesis(conn)
                conn.execute("INSERT INTO cfa4_audit.schema_version (version) VALUES (%s)", (version,))
        except psycopg.Error as exc:
            raise SchemaInstallFailed(f"migration {version}: {str(exc).splitlines()[0]}") from exc
        applied = True
    return applied


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
