"""AU-2: the audit command descriptors, the dispatcher and the ``cfa4-audit`` command line."""

from __future__ import annotations

import json
import uuid
from collections.abc import Iterator
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("psycopg")

import psycopg  # noqa: E402
import psycopg.sql  # noqa: E402

from cfa4_audit import ActorKind, AuditEvent, Outcome, record  # noqa: E402
from cfa4_audit.__main__ import main  # noqa: E402
from cfa4_audit.commands import COMMANDS, run_command  # noqa: E402
from cfa4_audit.migrate import install  # noqa: E402
from pg_throwaway import throwaway_cluster  # noqa: E402


@pytest.fixture(scope="module")
def prepared() -> Iterator[tuple[str, str]]:
    """(application conninfo, superuser conninfo) of a database with the schema and three events."""
    with throwaway_cluster() as info:
        with psycopg.connect(info, autocommit=True) as conn:
            for role in ("cfa4_audit_owner", "cfa4_audit_writer", "cfa4_audit_reader"):
                conn.execute(psycopg.sql.SQL("CREATE ROLE {} NOLOGIN").format(psycopg.sql.Identifier(role)))
            conn.execute("CREATE ROLE cfa4_test_app LOGIN IN ROLE cfa4_audit_writer, cfa4_audit_reader")
            conn.execute("CREATE DATABASE cfa4")
        admin = info.replace("dbname=postgres", "dbname=cfa4")
        app = admin.replace("user=postgres", "user=cfa4_test_app")
        with psycopg.connect(admin) as conn:
            install(conn)
        with psycopg.connect(app) as conn:
            for i in range(3):
                record(
                    conn,
                    AuditEvent(
                        app="backlot",
                        action="production.created" if i < 2 else "task.created",
                        actor_kind=ActorKind.OS_USER,
                        actor_ref="uid:1000",
                        subject_type="production",
                        subject_id=uuid.uuid4(),
                        outcome=Outcome.SUCCESS,
                        occurred_at=datetime.now(UTC),
                        scope={"i": i},
                    ),
                )
            conn.commit()
        yield app, admin


def test_every_command_has_a_descriptor() -> None:
    assert set(COMMANDS) == {
        "audit.list",
        "audit.verify",
        "audit.export",
        "audit.checkpoint",
        "audit.partition.ensure",
        "audit.personal.erase",
    }
    for cid, command in COMMANDS.items():
        assert command.id == cid and command.label and command.menu_path and command.menu_path[0] == "Audit"
        assert len({p.name for p in command.params}) == len(command.params)


def test_list_verify_and_export_through_the_dispatcher(prepared: tuple[str, str], tmp_path: Path) -> None:
    app, _ = prepared
    with psycopg.connect(app, autocommit=True) as conn:
        assert COMMANDS["audit.list"].enabled(conn)
        out = run_command(conn, "audit.list", {"action": "production.*", "limit": "10"})
        assert out["ok"] and [e["seq"] for e in out["events"]] == [2, 3] and out["last_seq"] == 3
        assert run_command(conn, "audit.list", {"after_seq": 99})["events"] == []
        ver = run_command(conn, "audit.verify", {})
        assert ver["ok"] and ver["checked"] == 4 and ver["code"] is None
        exp = run_command(conn, "audit.export", {"path": str(tmp_path / "e.json"), "action": "task.*"})
        assert exp == {"ok": True, "path": str(tmp_path / "e.json"), "events": 1}
        cp = run_command(conn, "audit.checkpoint", {"root": str(tmp_path / "cp")})
        assert cp["ok"] and Path(cp["path"]).is_file()
        ver = run_command(conn, "audit.verify", {"from_checkpoint": True, "checkpoint_path": cp["path"]})
        assert ver["ok"]


def test_the_dispatcher_turns_every_problem_into_a_code(prepared: tuple[str, str]) -> None:
    app, _ = prepared
    with psycopg.connect(app, autocommit=True) as conn:
        assert run_command(conn, "audit.nope")["code"] == "AUDIT_UNKNOWN_COMMAND"
        assert run_command(conn, "audit.list", {"bogus": 1})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.export", {})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.list", {"limit": "many"})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.list", {"limit": 0})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.list", {"subject_id": "not-a-uuid"})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.list", {"occurred_from": "2026-01-01T00:00:00"})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.list", {"scope_contains": "[1]"})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.verify", {"from_checkpoint": "maybe"})["code"] == "AUDIT_EVENT_INVALID"
        assert run_command(conn, "audit.verify", {"from_checkpoint": True, "checkpoint_path": "/nonexistent.json"})[
            "code"
        ] == "AUDIT_CHECKPOINT_MISMATCH"
        assert run_command(conn, "audit.partition.ensure", {})["code"] == "AUDIT_WRITE_DENIED", "owner-only maintenance"
        assert run_command(conn, "audit.list", {"scope_contains": '{"i": 1}'})["events"][0]["seq"] == 3


def test_erase_through_the_dispatcher(prepared: tuple[str, str]) -> None:
    app, _ = prepared
    with psycopg.connect(app, autocommit=True) as conn:
        out = run_command(conn, "audit.personal.erase", {"subject_type": "person", "subject_id": str(uuid.uuid4())})
        assert out == {"ok": True, "erased": 0}


def _run(capsys: pytest.CaptureFixture[str], conninfo: str, *argv: str) -> tuple[int, dict[str, Any]]:
    code = main(["--conninfo", conninfo, *argv])
    return code, json.loads(capsys.readouterr().out)


def test_the_command_line(prepared: tuple[str, str], tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    app, admin = prepared
    code, out = _run(capsys, app, "list", "--action", "production.*")
    assert code == 0 and [e["seq"] for e in out["events"]] == [2, 3]
    code, out = _run(capsys, app, "verify")
    assert code == 0 and out["ok"] and out["checked"] == 4
    code, out = _run(capsys, app, "export", "--path", str(tmp_path / "x.csv"), "--format", "csv")
    assert code == 0 and out["events"] == 4
    code, out = _run(capsys, app, "checkpoint", "--root", str(tmp_path / "cps"))
    assert code == 0 and Path(out["path"]).name == "4.json"
    # A finding is exit status 1, an unusable database is 2 with a code, never a traceback.
    with psycopg.connect(admin, autocommit=True) as conn:
        for part in conn.execute(
            "SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid "
            "WHERE i.inhparent = 'cfa4_audit.event'::regclass"
        ).fetchall():
            conn.execute(psycopg.sql.SQL("ALTER TABLE cfa4_audit.{} DISABLE TRIGGER USER").format(psycopg.sql.Identifier(part[0])))
        conn.execute("UPDATE cfa4_audit.event SET actor_ref = 'x' WHERE seq = 3")
    code, out = _run(capsys, app, "verify")
    assert code == 1 and out["code"] == "AUDIT_CHAIN_BROKEN" and out["seq"] == 3
    code, out = _run(capsys, "host=/nonexistent-dir user=nobody dbname=none connect_timeout=2", "verify")
    assert code == 2 and out["code"] == "DB_UNAVAILABLE"
