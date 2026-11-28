"""AU-2: the audit write and read paths on a real (throw-away) PostgreSQL."""

from __future__ import annotations

import csv
import hashlib
import json
import threading
import uuid
from collections.abc import Iterator
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("psycopg")

import psycopg  # noqa: E402
import psycopg.sql  # noqa: E402

from cfa4_audit import (  # noqa: E402
    ActorKind,
    AuditEvent,
    ChainBroken,
    CheckpointMismatch,
    Filters,
    InvalidEvent,
    Outcome,
    WriteDenied,
    canonical_bytes,
    ensure_partitions,
    erase_personal,
    event_hash,
    export_events,
    list_events,
    load_checkpoint,
    record,
    take_checkpoint,
    unit,
    verify,
)
from cfa4_audit.migrate import install  # noqa: E402
from pg_throwaway import throwaway_cluster  # noqa: E402


@pytest.fixture(scope="module")
def cluster() -> Iterator[str]:
    with throwaway_cluster() as info:
        with psycopg.connect(info, autocommit=True) as conn:
            # As in production (G2-01 section 5): the application's login role belongs to the writer AND the reader role.
            for role in ("cfa4_audit_owner", "cfa4_audit_writer", "cfa4_audit_reader"):
                conn.execute(psycopg.sql.SQL("CREATE ROLE {} NOLOGIN").format(psycopg.sql.Identifier(role)))
            conn.execute("CREATE ROLE cfa4_test_app LOGIN IN ROLE cfa4_audit_writer, cfa4_audit_reader")
        yield info


@pytest.fixture
def db(cluster: str) -> str:
    """A fresh database with the audit schema installed (one per test, so tests can tamper freely)."""
    name = "t" + uuid.uuid4().hex[:12]
    with psycopg.connect(cluster, autocommit=True) as conn:
        conn.execute(psycopg.sql.SQL("CREATE DATABASE {}").format(psycopg.sql.Identifier(name)))
    info = cluster.replace("dbname=postgres", f"dbname={name}")
    with psycopg.connect(info) as conn:
        install(conn)
    return info


def _conn(info: str, role: str | None = None, *, autocommit: bool = False) -> psycopg.Connection:
    """The application login role (writer + reader); ``role`` narrows it with SET ROLE to test one role alone."""
    conn = psycopg.connect(info.replace("user=postgres", "user=cfa4_test_app"), autocommit=autocommit)
    if role is not None:
        conn.execute(psycopg.sql.SQL("SET ROLE {}").format(psycopg.sql.Identifier(role)))
    return conn


def _event(**changes: Any) -> AuditEvent:
    base: dict[str, Any] = {
        "app": "backlot",
        "action": "production.created",
        "actor_kind": ActorKind.OS_USER,
        "actor_ref": "uid:1000",
        "subject_type": "production",
        "subject_id": uuid.uuid4(),
        "outcome": Outcome.SUCCESS,
        "occurred_at": datetime.now(UTC),
    }
    base.update(changes)
    return AuditEvent(**base)


def _tamper(info: str, sql: str, *params: Any) -> None:
    """Change the stored chain behind the trigger's back, as a superuser would (the M1 limit, G2-01 section 5)."""
    with psycopg.connect(info, autocommit=True) as conn:
        for part in conn.execute(
            "SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid "
            "WHERE i.inhparent = 'cfa4_audit.event'::regclass"
        ).fetchall():
            conn.execute(
                psycopg.sql.SQL("ALTER TABLE cfa4_audit.{} DISABLE TRIGGER USER").format(psycopg.sql.Identifier(part[0]))
            )
        conn.execute(sql, params)


def _write(info: str, n: int) -> list[int]:
    with _conn(info) as conn:
        seqs = []
        for i in range(n):
            seqs.append(record(conn, _event(scope={"i": i})).seq)
        conn.commit()
    return seqs


# ---------------------------------------------------------------- writing


def test_events_commit_and_the_chain_verifies(db: str) -> None:
    assert _write(db, 5) == [2, 3, 4, 5, 6], "seq 1 is the genesis event"
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert result.ok and result.checked == 6 and result.head_seq == 6 and result.code is None


def test_a_rollback_takes_the_audit_event_with_it(db: str) -> None:
    with _conn(db) as conn:
        record(conn, _event())
        conn.rollback()
    with _conn(db, "cfa4_audit_reader") as conn:
        assert [e.seq for e in list_events(conn)] == [1], "only the genesis event"


def test_unit_writes_all_events_at_the_end_and_nothing_on_error(db: str) -> None:
    with _conn(db) as conn:
        with unit(conn) as u:
            u.record(_event(scope={"k": 1}))
            u.record(_event(scope={"k": 2}))
            assert not u.recorded, "nothing is written until the block ends"
        assert [r.seq for r in u.recorded] == [2, 3]
        conn.commit()
        with pytest.raises(RuntimeError), unit(conn) as bad:
            bad.record(_event())
            raise RuntimeError("business error")
        assert not bad.recorded
        conn.rollback()
    with _conn(db, "cfa4_audit_reader") as conn:
        assert [e.seq for e in list_events(conn)] == [1, 2, 3]


def test_the_reader_cannot_write(db: str) -> None:
    with _conn(db, "cfa4_audit_reader") as conn, pytest.raises(WriteDenied) as caught:
        record(conn, _event())
    assert caught.value.code == "AUDIT_WRITE_DENIED"


def test_concurrent_writers_keep_one_unbroken_chain(db: str) -> None:
    errors: list[BaseException] = []

    def worker(label: str) -> None:
        try:
            with _conn(db) as conn:
                for i in range(15):
                    record(conn, _event(scope={"w": label, "i": i}))
                    conn.commit()
        except BaseException as exc:  # noqa: BLE001  # surfaced below
            errors.append(exc)

    threads = [threading.Thread(target=worker, args=(c,)) for c in "abc"]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert not errors
    with _conn(db, "cfa4_audit_reader") as conn:
        events = list_events(conn, limit=1000)
        assert [e.seq for e in events] == list(range(1, 47))
        times = [e.hashable["recorded_at"] for e in events]
        assert times == sorted(times), "recorded_at never goes back, even with concurrent writers"
        assert verify(conn).ok


# ---------------------------------------------------------------- reading


def test_list_filters_and_paging(db: str) -> None:
    prod = uuid.uuid4()
    with _conn(db) as conn:
        record(conn, _event(subject_id=prod, scope={"production": "p1"}))
        record(conn, _event(action="production.renamed", subject_id=prod, actor_ref="uid:2", scope={"production": "p1"}))
        record(conn, _event(action="task.created", subject_type="task", actor_kind=ActorKind.SYSTEM, actor_ref="sched"))
        record(conn, _event(action="task.created", subject_type="task", scope={"production": "p2"}))
        conn.commit()
    with _conn(db, "cfa4_audit_reader") as conn:
        assert [e.seq for e in list_events(conn, Filters(subject_id=prod))] == [2, 3]
        assert [e.seq for e in list_events(conn, Filters(action="production.*"))] == [2, 3]
        assert [e.seq for e in list_events(conn, Filters(action="production.renamed"))] == [3]
        assert [e.seq for e in list_events(conn, Filters(action="production.%"))] == [], "% is not a wildcard"
        assert [e.seq for e in list_events(conn, Filters(actor_ref="uid:2"))] == [3]
        assert [e.seq for e in list_events(conn, Filters(actor_kind="system"))] == [1, 4]
        assert [e.seq for e in list_events(conn, Filters(subject_type="task"))] == [4, 5]
        assert [e.seq for e in list_events(conn, Filters(scope_contains={"production": "p1"}))] == [2, 3]
        soon = datetime.now(UTC) + timedelta(minutes=1)
        assert len(list_events(conn, Filters(occurred_to=soon))) == 5
        assert list_events(conn, Filters(occurred_from=soon)) == []
        first = list_events(conn, limit=2)
        assert [e.seq for e in first] == [1, 2]
        assert [e.seq for e in list_events(conn, after_seq=first[-1].seq, limit=2)] == [3, 4]
        for bad in (0, 1001):
            with pytest.raises(InvalidEvent):
                list_events(conn, limit=bad)


def test_listed_events_hash_to_their_stored_hash(db: str) -> None:
    _write(db, 3)
    with _conn(db, "cfa4_audit_reader") as conn:
        for e in list_events(conn):
            assert event_hash(e.hashable) == e.hash


# ---------------------------------------------------------------- verify finds tampering


def test_verify_finds_a_changed_event(db: str) -> None:
    _write(db, 4)
    _tamper(db, "UPDATE cfa4_audit.event SET actor_ref = 'someone-else' WHERE seq = 3")
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert not result.ok and result.code == "AUDIT_CHAIN_BROKEN" and result.seq == 3
        assert "content" in result.detail
        assert verify(conn, to_seq=2).ok, "the part before the change still verifies"
        assert verify(conn, from_seq=4).ok, "the link into the range is the stored previous hash"


def test_verify_finds_a_removed_event_and_a_relinked_chain(db: str) -> None:
    _write(db, 4)
    _tamper(db, "DELETE FROM cfa4_audit.event WHERE seq = 3")
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert not result.ok and result.code == "AUDIT_CHAIN_BROKEN" and result.seq == 3
        assert "missing" in result.detail


def test_verify_finds_a_changed_prev_hash(db: str) -> None:
    _write(db, 3)
    _tamper(db, "UPDATE cfa4_audit.event SET prev_hash = %s WHERE seq = 3", bytes([7]) * 32)
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert not result.ok and result.seq == 3 and "prev_hash" in result.detail


def test_verify_finds_a_cut_back_chain_end(db: str) -> None:
    _write(db, 3)
    _tamper(db, "DELETE FROM cfa4_audit.event WHERE seq = 4")  # the last event, head still says 4
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert not result.ok and result.code == "AUDIT_CHAIN_BROKEN" and result.seq == 4


def test_verify_reports_an_unknown_schema_version(db: str) -> None:
    _write(db, 1)
    _tamper(db, "UPDATE cfa4_audit.event SET schema_ver = 2 WHERE seq = 2")
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert not result.ok and result.code == "AUDIT_SCHEMA_VERSION_UNKNOWN" and result.seq == 2


def test_verify_in_batches_covers_everything(db: str) -> None:
    _write(db, 12)
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn, batch=5)
        assert result.ok and result.checked == 13


# ---------------------------------------------------------------- checkpoints


def test_a_checkpoint_is_written_indexed_and_verified_against(db: str, tmp_path: Path) -> None:
    _write(db, 3)
    with _conn(db) as conn:
        path = take_checkpoint(conn, root=tmp_path)
        conn.commit()
        assert path.parent.parent == tmp_path and path.name == "4.json"
        assert not list(path.parent.glob("*.tmp")), "written atomically"
    body = json.loads(path.read_text(encoding="utf-8"))
    assert body["seq"] == 4 and len(body["hash"]) == 64
    _write(db, 2)
    with _conn(db, "cfa4_audit_reader") as conn:
        cp = load_checkpoint(conn, root=tmp_path)
        assert verify(conn, checkpoint=cp).ok, "the chain grew past the checkpoint: fine"
    with psycopg.connect(db) as conn:
        indexed = conn.execute("SELECT seq FROM cfa4_audit.checkpoint").fetchall()
        assert [r[0] for r in indexed] == [4]


def test_a_cut_back_chain_is_caught_by_the_checkpoint(db: str, tmp_path: Path) -> None:
    _write(db, 3)
    with _conn(db) as conn:
        take_checkpoint(conn, root=tmp_path)
        conn.commit()
    # An attacker cuts the last two events and rewinds the head so that the chain itself still verifies.
    _tamper(db, "DELETE FROM cfa4_audit.event WHERE seq > 2")
    _tamper(
        db,
        "UPDATE cfa4_audit.chain_head SET seq = 2, hash = (SELECT hash FROM cfa4_audit.event WHERE seq = 2)",
    )
    with _conn(db, "cfa4_audit_reader") as conn:
        assert verify(conn).ok, "the cut chain is internally consistent"
        result = verify(conn, checkpoint=load_checkpoint(conn, root=tmp_path))
        assert not result.ok and result.code == "AUDIT_CHECKPOINT_MISMATCH" and result.seq == 4
        assert "cut back" in result.detail


def test_a_checkpoint_with_a_wrong_hash_or_cluster_or_file_is_refused(db: str, tmp_path: Path) -> None:
    _write(db, 2)
    with _conn(db) as conn:
        path = take_checkpoint(conn, root=tmp_path)
        conn.commit()
    body = json.loads(path.read_text(encoding="utf-8"))
    with _conn(db, "cfa4_audit_reader") as conn:
        forged = tmp_path / "forged.json"
        forged.write_text(json.dumps({**body, "hash": "ab" * 32}), encoding="utf-8")
        result = verify(conn, checkpoint=load_checkpoint(conn, path=forged))
        assert not result.ok and result.code == "AUDIT_CHECKPOINT_MISMATCH" and "differs" in result.detail
        other = tmp_path / "other.json"
        other.write_text(json.dumps({**body, "cluster_id": "1"}), encoding="utf-8")
        garbage = tmp_path / "garbage.json"
        garbage.write_text("not json", encoding="utf-8")
        for bad in (other, garbage, tmp_path / "missing.json"):
            with pytest.raises(CheckpointMismatch):
                load_checkpoint(conn, path=bad)
        with pytest.raises(CheckpointMismatch):
            load_checkpoint(conn, root=tmp_path / "empty")


def test_the_database_refuses_a_checkpoint_that_is_not_in_the_chain(db: str) -> None:
    with _conn(db) as conn, pytest.raises(psycopg.errors.Error) as caught:
        conn.execute("SELECT cfa4_audit.record_checkpoint(1, %s, '/x')", (bytes([9]) * 32,))
    assert caught.value.sqlstate == "AU004"


# ---------------------------------------------------------------- personal data


def test_personal_values_stay_out_of_the_chain_and_can_be_erased(db: str) -> None:
    subject = uuid.uuid4()
    phone = "+36 30 123 4567"
    with _conn(db) as conn:
        rec = record(
            conn,
            _event(subject_type="person", subject_id=subject, action="person.updated", change={"role": {"old": "a", "new": "b"}}),
            personal={"phone": {"old": phone, "new": "+36 1 999 9999"}},
        )
        conn.commit()
    with _conn(db, "cfa4_audit_reader") as conn:
        stored = list_events(conn, Filters(subject_id=subject))[0]
        change = stored.hashable["change"]
        assert change["role"] == {"old": "a", "new": "b"}
        assert change["phone"]["old"]["redacted"] is True
        assert phone not in json.dumps(stored.hashable)
        assert rec.seq == stored.seq and verify(conn).ok
    with psycopg.connect(db) as conn:  # superuser: check the salted digest against the stored detail
        row = conn.execute("SELECT detail, salt FROM cfa4_audit.personal_detail WHERE subject_id = %s", (subject,)).fetchone()
        assert row is not None
        detail, salt = row[0], bytes(row[1])
        want = hashlib.sha256(salt + canonical_bytes(detail["phone"]["old"])).hexdigest()
        assert change["phone"]["old"]["digest"] == want
    with _conn(db) as conn:
        assert erase_personal(conn, "person", subject) == 1
        assert erase_personal(conn, "person", subject) == 0
        conn.commit()
    with psycopg.connect(db) as conn:
        row = conn.execute("SELECT detail, salt, erased_at FROM cfa4_audit.personal_detail WHERE subject_id = %s", (subject,)).fetchone()
        assert row is not None and row[0] is None and row[1] is None and row[2] is not None
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert result.ok and result.erased_personal == 1, "the chain verifies after the erasure; the count is reported"


def test_personal_input_is_validated(db: str) -> None:
    with _conn(db) as conn:
        with pytest.raises(InvalidEvent):
            record(conn, _event(change={"phone": {"old": 1, "new": 2}}), personal={"phone": {"old": "x"}})
        with pytest.raises(InvalidEvent):
            record(conn, _event(), personal={"phone": {"before": "x"}})
        with pytest.raises(InvalidEvent):
            record(conn, _event(), personal={"phone": {}})
        conn.rollback()


# ---------------------------------------------------------------- export, maintenance


def test_json_export_can_be_verified_outside_the_database(db: str, tmp_path: Path) -> None:
    _write(db, 3)
    out = tmp_path / "audit.json"
    with _conn(db, "cfa4_audit_reader") as conn:
        assert export_events(conn, out) == 4
        assert export_events(conn, tmp_path / "some.json", Filters(action="production.*")) == 3
    doc = json.loads(out.read_text(encoding="utf-8"))
    assert doc["format"] == "cfa4-audit-export/1"
    prev = "00" * 32
    for ev in doc["events"]:
        assert ev["prev_hash"] == prev
        assert event_hash(ev).hex() == ev["hash"], "the exported event hashes to its hash without any database"
        prev = ev["hash"]
    assert not list(tmp_path.glob("*.tmp"))


def test_csv_export_is_flat(db: str, tmp_path: Path) -> None:
    _write(db, 2)
    out = tmp_path / "audit.csv"
    with _conn(db, "cfa4_audit_reader") as conn:
        assert export_events(conn, out, fmt="csv") == 3
        with pytest.raises(InvalidEvent):
            export_events(conn, tmp_path / "x", fmt="xml")
    with out.open(encoding="utf-8", newline="") as fh:
        rows = list(csv.DictReader(fh))
    assert [r["seq"] for r in rows] == ["1", "2", "3"] and json.loads(rows[1]["scope"]) == {"i": 0}


def test_export_pages_through_many_events(db: str, tmp_path: Path) -> None:
    _write(db, 7)
    with _conn(db, "cfa4_audit_reader") as conn:
        assert export_events(conn, tmp_path / "p.json", page=3) == 8


def test_ensure_partitions_is_owner_only(db: str) -> None:
    with psycopg.connect(db, autocommit=True) as conn:  # the installer's superuser acts as the owner (SET ROLE)
        conn.execute("SET ROLE cfa4_audit_owner")
        assert ensure_partitions(conn, 2) == 0
        assert ensure_partitions(conn, 3) == 1
    with _conn(db, "cfa4_audit_writer", autocommit=True) as conn, pytest.raises(WriteDenied):
        ensure_partitions(conn, 2)


def test_a_missing_chain_head_is_reported_not_raised(db: str) -> None:
    _tamper(db, "DELETE FROM cfa4_audit.chain_head")
    with _conn(db, "cfa4_audit_reader") as conn:
        result = verify(conn)
        assert not result.ok and result.code == "AUDIT_CHAIN_BROKEN"
    with _conn(db) as conn, pytest.raises(ChainBroken):
        record(conn, _event())
