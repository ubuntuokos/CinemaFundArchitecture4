"""AU-3.1: tamper fuzzing, a crash while writing, restore from a backup, canonical-form fuzzing (CFA4-AUDIT-001 AU-3)."""

from __future__ import annotations

import json
import random
import subprocess
import uuid
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("psycopg")

import psycopg  # noqa: E402
import psycopg.sql  # noqa: E402

from cfa4_audit import AuditError, canonical_bytes, load_checkpoint, record, take_checkpoint, verify  # noqa: E402
from pg_throwaway import _bin_dir  # noqa: E402
from test_audit_store import _conn, _event, _write, cluster, db  # noqa: E402, F401

# ---------------------------------------------------------------- tamper fuzzing

# (column, SQL expression for a different value). Every stored column except the hash itself is part of the hash.
_MUTATIONS = [
    ("event_id", "gen_random_uuid()"),
    ("schema_ver", "schema_ver + 1"),
    ("occurred_at", "occurred_at + interval '1 microsecond'"),
    ("recorded_at", "recorded_at + interval '1 microsecond'"),
    ("app", "app || 'x'"),
    ("action", "'other.action'"),
    ("command_id", "coalesce(command_id, '') || 'x'"),
    ("actor_kind", "'system'"),
    ("actor_ref", "actor_ref || 'x'"),
    ("on_behalf_of", "coalesce(on_behalf_of, '') || 'x'"),
    ("subject_type", "subject_type || 'x'"),
    ("subject_id", "gen_random_uuid()"),
    ("scope", "scope || '{\"extra\": 1}'::jsonb"),
    ("change", "'{\"a\": {\"old\": 1, \"new\": 2}}'::jsonb"),
    ("outcome", "'rejected'"),
    ("reason_code", "'TAMPERED'"),
    ("correlation_id", "gen_random_uuid()"),
    ("causation_id", "gen_random_uuid()"),
    ("trace_id", "coalesce(trace_id, '') || 'x'"),
    ("prev_hash", "decode(repeat('cd', 32), 'hex')"),
    ("hash", "decode(repeat('ab', 32), 'hex')"),
]


def test_every_changed_column_of_any_event_is_found(db: str) -> None:  # noqa: F811
    _write(db, 12)
    rng = random.Random(20261123)
    with psycopg.connect(db) as conn:  # a superuser: the M1 limit (G2-01 section 5), changes are rolled back per case
        parts = [
            r[0]
            for r in conn.execute(
                "SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid "
                "WHERE i.inhparent = 'cfa4_audit.event'::regclass"
            ).fetchall()
        ]
        for column, expr in _MUTATIONS:
            for seq in rng.sample(range(2, 14), 3):
                for part in parts:
                    conn.execute(
                        psycopg.sql.SQL("ALTER TABLE cfa4_audit.{} DISABLE TRIGGER USER").format(psycopg.sql.Identifier(part))
                    )
                update = (
                    psycopg.sql.SQL("UPDATE cfa4_audit.event SET ")
                    + psycopg.sql.Identifier(column)
                    + psycopg.sql.SQL(" = ")
                    + psycopg.sql.SQL(expr)
                    + psycopg.sql.SQL(" WHERE seq = %s")
                )
                conn.execute(update, (seq,))
                result = verify(conn)
                assert not result.ok, f"{column} of seq {seq} was changed and not found"
                assert result.seq == seq, f"{column}: first fault at {result.seq}, expected {seq} ({result.detail})"
                conn.rollback()
        assert verify(conn).ok, "the rollbacks left the chain intact"


# ---------------------------------------------------------------- a crash while writing


def test_a_killed_connection_leaves_no_event_and_no_lock(db: str) -> None:  # noqa: F811
    _write(db, 3)
    victim = _conn(db)
    pid = victim.info.backend_pid
    record(victim, _event(scope={"half": "written"}))  # holds the chain-head lock, not committed
    with psycopg.connect(db, autocommit=True) as admin:
        assert admin.execute("SELECT pg_terminate_backend(%s)", (pid,)).fetchone() == (True,)
    with pytest.raises(psycopg.OperationalError):
        victim.execute("SELECT 1")
    victim.close()
    with _conn(db) as conn:
        conn.execute("SET lock_timeout = '3s'")  # the dead writer's lock must be gone
        assert record(conn, _event()).seq == 5, "seq 4 was never committed, so the next event takes it"
        conn.commit()
    with _conn(db, "cfa4_audit_reader") as reader:
        assert verify(reader).ok
        assert reader.execute("SELECT count(*) FROM cfa4_audit.event WHERE scope ? 'half'").fetchone() == (0,)


# ---------------------------------------------------------------- restore from a backup


def _pg(tool: str, *args: str) -> None:
    bindir = _bin_dir()
    assert bindir is not None
    subprocess.run([str(bindir / tool), *args], check=True, capture_output=True, text=True, timeout=120)


def _restore(cluster_info: str, source_db: str, dump: Path) -> str:
    """Restore ``dump`` into a new database of the same cluster and return its connection string."""
    name = "r" + uuid.uuid4().hex[:12]
    host = dict(p.split("=", 1) for p in cluster_info.split())["host"]
    _pg("createdb", "-h", host, "-U", "postgres", name)
    _pg("pg_restore", "-h", host, "-U", "postgres", "-d", name, "--no-owner", str(dump))
    return source_db.replace(source_db.split("dbname=")[1], name)


def _dump(info: str, out: Path) -> None:
    host = dict(p.split("=", 1) for p in info.split())["host"]
    _pg("pg_dump", "-h", host, "-U", "postgres", "-Fc", "-n", "cfa4_audit", "-f", str(out), info.split("dbname=")[1])


def test_a_restored_backup_verifies_and_the_chain_continues(cluster: str, db: str, tmp_path: Path) -> None:  # noqa: F811
    _write(db, 6)
    _dump(db, tmp_path / "full.dump")
    restored = _restore(cluster, db, tmp_path / "full.dump")
    with _conn(restored) as conn:
        assert verify(conn).ok
        assert record(conn, _event()).seq == 8
        conn.commit()
        assert verify(conn).ok


def test_a_backup_older_than_the_checkpoint_is_caught_after_a_restore(cluster: str, db: str, tmp_path: Path) -> None:  # noqa: F811
    _write(db, 4)
    _dump(db, tmp_path / "old.dump")
    _write(db, 3)  # events after the backup
    with _conn(db) as conn:
        take_checkpoint(conn, root=tmp_path / "cp")
        conn.commit()
    restored = _restore(cluster, db, tmp_path / "old.dump")
    with _conn(restored) as conn:
        assert verify(conn).ok, "the old backup is a consistent chain on its own"
        result = verify(conn, checkpoint=load_checkpoint(conn, root=tmp_path / "cp"))
        assert not result.ok, "but it is shorter than the checkpoint kept outside the database"


# ---------------------------------------------------------------- canonical form fuzzing


def _random_value(rng: random.Random, depth: int = 0) -> Any:
    kinds = ["int", "str", "bool", "none"] + (["list", "dict"] if depth < 3 else [])
    kind = rng.choice(kinds)
    if kind == "int":
        return rng.choice([0, 1, -1, rng.randint(-(2**63), 2**63 - 1), 2**63 - 1, -(2**63)])
    if kind == "str":
        pool = ["", "a", "ü", "€", "\u0000", "\n", '"', "\\", "\U0001f600", "퟿", "", "é" * 5, "z", "10", "B"]
        return "".join(rng.choice(pool) for _ in range(rng.randint(0, 6)))
    if kind == "bool":
        return rng.random() < 0.5
    if kind == "none":
        return None
    if kind == "list":
        return [_random_value(rng, depth + 1) for _ in range(rng.randint(0, 4))]
    return {_random_value_key(rng): _random_value(rng, depth + 1) for _ in range(rng.randint(0, 4))}


def _random_value_key(rng: random.Random) -> str:
    return rng.choice(["a", "b", "B", "ä", "\U0001f600", "", "10", "2", "", "key", "€"])


def _shuffled(value: Any, rng: random.Random) -> Any:
    if isinstance(value, dict):
        items = list(value.items())
        rng.shuffle(items)
        return {k: _shuffled(v, rng) for k, v in items}
    if isinstance(value, list):
        return [_shuffled(v, rng) for v in value]
    return value


def test_canonical_form_is_stable_round_trips_and_ignores_key_order() -> None:
    rng = random.Random(8785)
    for _ in range(2000):
        value = _random_value(rng)
        data = canonical_bytes(value)
        assert json.loads(data.decode("utf-8")) == value, "the canonical text is the same JSON value"
        assert canonical_bytes(_shuffled(value, rng)) == data, "key insertion order does not matter"
        assert canonical_bytes(json.loads(data.decode("utf-8"))) == data, "canonicalising twice changes nothing"


@pytest.mark.parametrize("bad", [1.5, float("nan"), float("inf"), [1.0], {"a": 2.5}, 2**63, -(2**63) - 1, "\ud800", {1: 2}, object()])
def test_canonical_form_refuses_what_it_cannot_represent_without_crashing(bad: Any) -> None:
    with pytest.raises((AuditError, TypeError)):
        canonical_bytes(bad)
