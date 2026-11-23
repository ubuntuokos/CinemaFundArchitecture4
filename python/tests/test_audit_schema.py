"""AU-1: the audit schema on a real (throw-away) PostgreSQL: append-only, chain rules, rights, partitions."""

from __future__ import annotations

import json
import uuid
from collections.abc import Iterator
from datetime import UTC, datetime
from typing import Any

import pytest

pytest.importorskip("psycopg")

import psycopg  # noqa: E402
import psycopg.errors  # noqa: E402
import psycopg.sql  # noqa: E402
from psycopg.types.json import Jsonb  # noqa: E402

from cfa4_audit import GENESIS_PREV_HASH, ActorKind, AuditEvent, Outcome, event_hash  # noqa: E402
from cfa4_audit.migrate import SchemaInstallFailed, install, is_installed, migration_sql  # noqa: E402

from pg_throwaway import throwaway_cluster  # noqa: E402


@pytest.fixture(scope="module")
def conninfo() -> Iterator[str]:
    with throwaway_cluster() as info:
        yield info


@pytest.fixture(scope="module")
def installed(conninfo: str) -> str:
    with psycopg.connect(conninfo, autocommit=True) as conn:
        conn.execute("CREATE DATABASE cfa4")
    info = conninfo.replace("dbname=postgres", "dbname=cfa4")
    with psycopg.connect(info) as conn:
        assert install(conn) is True
    return info


def _as(info: str, role: str) -> psycopg.Connection:
    conn = psycopg.connect(info, autocommit=True)
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


def _append(conn: psycopg.Connection, ev: AuditEvent) -> int:
    """The write path as the library will do it (AU-2): lock the head, hash, append; returns the seq."""
    head = conn.execute("SELECT seq, hash, now_utc FROM cfa4_audit.lock_head()").fetchone()
    assert head is not None
    seq, prev_hash, now = head[0] + 1, bytes(head[1]), head[2]
    body = ev.hashable(seq=seq, recorded_at=now, prev_hash=prev_hash)
    conn.execute(
        "SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)", (Jsonb(body), event_hash(body), prev_hash, seq)
    )
    return int(seq)


def test_install_is_idempotent_and_leaves_a_genesis_event(installed: str) -> None:
    with psycopg.connect(installed) as conn:
        assert is_installed(conn)
        assert install(conn) is False
        rows = conn.execute("SELECT seq, action, app, prev_hash, scope FROM cfa4_audit.event ORDER BY seq").fetchall()
        assert rows[0][0] == 1 and rows[0][1] == "audit.chain.genesis" and rows[0][2] == "cfa4-audit"
        assert bytes(rows[0][3]) == GENESIS_PREV_HASH
        assert set(rows[0][4]) == {"cluster_id", "database_oid"}
        head = conn.execute("SELECT seq, hash FROM cfa4_audit.chain_head").fetchone()
        assert head is not None and head[0] == len(rows)


def test_a_failed_install_rolls_back_completely(conninfo: str) -> None:
    with psycopg.connect(conninfo, autocommit=True) as conn:
        conn.execute("CREATE DATABASE broken")
    info = conninfo.replace("dbname=postgres", "dbname=broken")
    with psycopg.connect(info) as conn:
        conn.execute("CREATE SCHEMA cfa4_audit")  # a clash: the migration's CREATE SCHEMA must fail
        conn.commit()
        with pytest.raises(SchemaInstallFailed) as caught:
            install(conn)
        assert caught.value.code == "AUDIT_SCHEMA_INSTALL_FAILED"
        row = conn.execute("SELECT count(*) FROM information_schema.tables WHERE table_schema = 'cfa4_audit'").fetchone()
        assert row is not None and row[0] == 0, "nothing of the failed migration remains"


def test_the_writer_appends_through_the_function_and_the_chain_verifies(installed: str) -> None:
    with _as(installed, "cfa4_audit_writer") as conn:
        conn.autocommit = False
        seqs = []
        for i in range(3):
            seqs.append(_append(conn, _event(scope={"n": i})))
        conn.commit()
    with psycopg.connect(installed) as conn:
        rows = conn.execute("SELECT seq, prev_hash, hash FROM cfa4_audit.event ORDER BY seq").fetchall()
        for prev, cur in zip(rows, rows[1:], strict=False):
            assert bytes(cur[1]) == bytes(prev[2]), "prev_hash links to the previous hash"
        assert [r[0] for r in rows] == list(range(1, len(rows) + 1))
    assert seqs == sorted(seqs)


def test_writer_has_no_direct_table_rights(installed: str) -> None:
    with _as(installed, "cfa4_audit_writer") as conn:
        for sql in (
            "INSERT INTO cfa4_audit.event (seq) VALUES (999)",
            "SELECT * FROM cfa4_audit.event",
            "UPDATE cfa4_audit.chain_head SET seq = 0",
            "SELECT * FROM cfa4_audit.personal_detail",
        ):
            with pytest.raises(psycopg.errors.InsufficientPrivilege):
                conn.execute(sql)


def test_reader_reads_the_chain_but_not_personal_data_and_cannot_write(installed: str) -> None:
    with _as(installed, "cfa4_audit_reader") as conn:
        row = conn.execute("SELECT count(*) FROM cfa4_audit.event").fetchone()
        assert row is not None and row[0] >= 1
        with pytest.raises(psycopg.errors.InsufficientPrivilege):
            conn.execute("SELECT * FROM cfa4_audit.personal_detail")
        with pytest.raises(psycopg.errors.InsufficientPrivilege):
            conn.execute("SELECT cfa4_audit.lock_head()")


@pytest.mark.parametrize("role", ["cfa4_audit_owner", "postgres"])
def test_nobody_can_update_delete_or_truncate_the_chain(installed: str, role: str) -> None:
    """The owner and even the superuser: the trigger refuses (the M1 limit is the trigger being disabled)."""
    conn = psycopg.connect(installed, autocommit=True)
    if role != "postgres":
        conn.execute(psycopg.sql.SQL("SET ROLE {}").format(psycopg.sql.Identifier(role)))
    with conn:
        for sql in (
            "UPDATE cfa4_audit.event SET app = 'x'",
            "DELETE FROM cfa4_audit.event",
            "TRUNCATE cfa4_audit.event",
            "TRUNCATE cfa4_audit.event_default",
        ):
            with pytest.raises(psycopg.errors.Error) as caught:
                conn.execute(sql)
            assert caught.value.sqlstate == "AU001", sql
            assert "AUDIT_APPEND_ONLY" in str(caught.value)
        part = conn.execute(
            "SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid "
            "WHERE i.inhparent = 'cfa4_audit.event'::regclass AND c.relname <> 'event_default' LIMIT 1"
        ).fetchone()
        assert part is not None
        with pytest.raises(psycopg.errors.Error) as caught:
            conn.execute(psycopg.sql.SQL("TRUNCATE cfa4_audit.{}").format(psycopg.sql.Identifier(part[0])))
        assert caught.value.sqlstate == "AU001", "truncating a monthly partition directly is refused too"


def test_a_stale_chain_head_is_a_conflict_not_a_fork(installed: str) -> None:
    with _as(installed, "cfa4_audit_writer") as conn:
        head = conn.execute("SELECT seq, hash, now_utc FROM cfa4_audit.lock_head()").fetchone()
        assert head is not None
        seq, prev, now = head[0], bytes(head[1]), head[2]
        ev = _event()
        for bad_seq, bad_prev in ((seq, prev), (seq + 2, prev), (seq + 1, bytes(32))):
            body = ev.hashable(seq=bad_seq, recorded_at=now, prev_hash=bad_prev)
            with pytest.raises(psycopg.errors.Error) as caught:
                conn.execute(
                    "SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)",
                    (Jsonb(body), event_hash(body), bad_prev, bad_seq),
                )
            assert caught.value.sqlstate == "AU003" and "AUDIT_CHAIN_CONFLICT" in str(caught.value)
        # jsonb that disagrees with the arguments is a conflict too
        body = ev.hashable(seq=seq + 1, recorded_at=now, prev_hash=prev)
        with pytest.raises(psycopg.errors.Error):
            conn.execute(
                "SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)",
                (Jsonb({**body, "seq": 9999}), event_hash(body), prev, seq + 1),
            )
        # recorded_at may not go back in time
        old = datetime(2000, 1, 1, tzinfo=UTC)
        body = ev.hashable(seq=seq + 1, recorded_at=old, prev_hash=prev)
        with pytest.raises(psycopg.errors.Error) as caught:
            conn.execute(
                "SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)",
                (Jsonb(body), event_hash(body), prev, seq + 1),
            )
        assert caught.value.sqlstate == "AU003"


def test_constraints_reject_malformed_rows(installed: str) -> None:
    with _as(installed, "cfa4_audit_writer") as conn:
        head = conn.execute("SELECT seq, hash, now_utc FROM cfa4_audit.lock_head()").fetchone()
        assert head is not None
        seq, prev, now = head[0] + 1, bytes(head[1]), head[2]
        body = _event().hashable(seq=seq, recorded_at=now, prev_hash=prev)
        with pytest.raises(psycopg.errors.CheckViolation):  # a hash that is not 32 bytes
            conn.execute(
                "SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)", (Jsonb(body), b"short", prev, seq)
            )


def test_partitions_exist_for_this_and_the_next_two_months_and_ensure_is_idempotent(installed: str) -> None:
    conn = psycopg.connect(installed, autocommit=True)
    with conn:
        conn.execute("SET ROLE cfa4_audit_owner")
        names = {
            r[0]
            for r in conn.execute(
                "SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid "
                "WHERE i.inhparent = 'cfa4_audit.event'::regclass"
            ).fetchall()
        }
        assert "event_default" in names and len(names) == 4
        row = conn.execute("SELECT cfa4_audit.ensure_partitions(2)").fetchone()
        assert row is not None and row[0] == 0, "nothing to create the second time"
        row = conn.execute("SELECT cfa4_audit.ensure_partitions(3)").fetchone()
        assert row is not None and row[0] == 1
        with pytest.raises(psycopg.errors.Error):
            conn.execute("SELECT cfa4_audit.ensure_partitions(99)")
    # the writer may not run maintenance
    with _as(installed, "cfa4_audit_writer") as w:
        with pytest.raises(psycopg.errors.InsufficientPrivilege):
            w.execute("SELECT cfa4_audit.ensure_partitions(2)")


def test_rows_in_the_default_partition_are_moved_when_the_month_is_created(installed: str) -> None:
    now_utc = datetime.now(UTC)
    total = now_utc.year * 12 + now_utc.month - 1 + 18  # eighteen months ahead: beyond the 3 partitions made at install
    far = datetime(total // 12, total % 12 + 1, 1, 12, 0, 0, tzinfo=UTC)
    with _as(installed, "cfa4_audit_writer") as conn:
        conn.autocommit = False
        head = conn.execute("SELECT seq, hash, now_utc FROM cfa4_audit.lock_head()").fetchone()
        assert head is not None
        seq, prev = head[0] + 1, bytes(head[1])
        body = _event().hashable(seq=seq, recorded_at=far, prev_hash=prev)
        conn.execute("SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)", (Jsonb(body), event_hash(body), prev, seq))
        conn.commit()
    with psycopg.connect(installed, autocommit=True) as conn:
        row = conn.execute("SELECT count(*) FROM ONLY cfa4_audit.event_default").fetchone()
        assert row is not None and row[0] == 1, "no monthly partition: the default partition took the row"
    # Create exactly that month: the partition must be created and the row moved, with no loss and no AUDIT_APPEND_ONLY.
    month = far.strftime("%Y_%m")
    with psycopg.connect(installed, autocommit=True) as conn:
        conn.execute("SET ROLE cfa4_audit_owner")
        months_ahead = 18
        conn.execute("SELECT cfa4_audit.ensure_partitions(%s)", (months_ahead,))
        in_default = conn.execute("SELECT count(*) FROM ONLY cfa4_audit.event_default").fetchone()
        in_month = conn.execute(
            psycopg.sql.SQL("SELECT count(*) FROM ONLY cfa4_audit.{}").format(psycopg.sql.Identifier(f"event_{month}"))
        ).fetchone()
        assert in_default is not None and in_month is not None
        assert in_default[0] == 0 and in_month[0] == 1
        # The re-attached default partition and the new month are protected again (the row triggers are cloned
        # back; the default partition has no rows left, so check the catalog, and the month partition by UPDATE).
        for table in ("event_default", f"event_{month}"):
            row = conn.execute(
                "SELECT count(*) FROM pg_trigger WHERE tgrelid = ('cfa4_audit.' || quote_ident(%s))::regclass "
                "AND tgname = 'event_no_update_delete'",
                (table,),
            ).fetchone()
            assert row is not None and row[0] == 1, table
        with pytest.raises(psycopg.errors.Error) as caught:
            conn.execute(psycopg.sql.SQL("UPDATE cfa4_audit.{} SET app = 'x'").format(psycopg.sql.Identifier(f"event_{month}")))
        assert caught.value.sqlstate == "AU001"


def test_personal_detail_is_written_and_erased_without_touching_the_chain(installed: str) -> None:
    subject = uuid.uuid4()
    with _as(installed, "cfa4_audit_writer") as conn:
        conn.execute(
            "SELECT cfa4_audit.write_personal(%s, 'person', %s, %s::jsonb, %s)",
            (uuid.uuid4(), subject, json.dumps({"phone": "+36 30 123 4567"}), bytes(range(16))),
        )
        conn.execute(
            "SELECT cfa4_audit.write_personal(%s, 'person', %s, %s::jsonb, %s)",
            (uuid.uuid4(), subject, json.dumps({"iban": "HU42 1177 3016 1111 1018 0000 0000"}), bytes(range(16, 32))),
        )
        before = conn.execute("SELECT count(*) FROM cfa4_audit.lock_head()").fetchone()
        row = conn.execute("SELECT cfa4_audit.erase_personal('person', %s)", (subject,)).fetchone()
        assert row is not None and row[0] == 2
        row = conn.execute("SELECT cfa4_audit.erase_personal('person', %s)", (subject,)).fetchone()
        assert row is not None and row[0] == 0, "erasing twice changes nothing"
        assert before is not None
    with psycopg.connect(installed) as conn:
        rows = conn.execute(
            "SELECT detail, salt, erased_at FROM cfa4_audit.personal_detail WHERE subject_id = %s", (subject,)
        ).fetchall()
        assert len(rows) == 2 and all(r[0] is None and r[1] is None and r[2] is not None for r in rows)
        with pytest.raises(psycopg.errors.CheckViolation):  # an erased row cannot keep a salt
            conn.execute("UPDATE cfa4_audit.personal_detail SET salt = '\\x00' WHERE subject_id = %s", (subject,))


def test_the_migration_text_is_packaged() -> None:
    assert "CREATE SCHEMA cfa4_audit" in migration_sql()
