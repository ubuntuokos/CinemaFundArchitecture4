"""AU-4.1: the archive package of a closed month (create, verify offline, refuse what is wrong)."""

from __future__ import annotations

import json
from datetime import UTC, date, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("psycopg")

from cfa4_audit import InvalidEvent, archive_month, verify_archive  # noqa: E402
from cfa4_audit.__main__ import main  # noqa: E402
from cfa4_audit.commands import run_command  # noqa: E402
from cfa4_audit.store import ChainBroken  # noqa: E402
from test_audit_store import _conn, _tamper, _write, cluster, db  # noqa: E402, F401

_LATER = datetime.now(UTC) + timedelta(days=62)  # "now" after the test month has closed
_THIS_MONTH = date.today()


def _package(info: str, tmp_path: Path, n: int = 5) -> Path:
    _write(info, n)
    with _conn(info, "cfa4_audit_reader") as conn:
        return archive_month(conn, _THIS_MONTH, tmp_path, now=_LATER)


def _rewrite(path: Path, change: Any) -> None:
    doc = json.loads(path.read_text(encoding="utf-8"))
    change(doc)
    path.write_text(json.dumps(doc, indent=1), encoding="utf-8")
    path.with_name(path.name + ".sha256").unlink(missing_ok=True)  # so the digest of the events is what is tested, not the sidecar


def test_a_closed_month_is_archived_and_verifies_without_the_database(db: str, tmp_path: Path) -> None:  # noqa: F811
    path = _package(db, tmp_path)
    assert path.name == f"{_THIS_MONTH:%Y-%m}.cfa4audit.json" and path.with_name(path.name + ".sha256").exists()
    doc = json.loads(path.read_text(encoding="utf-8"))
    m = doc["manifest"]
    assert (m["first_seq"], m["last_seq"], m["count"]) == (1, 6, 6) and m["first_prev_hash"] == "00" * 32
    result = verify_archive(path)
    assert result.ok and (result.count, result.first_seq, result.last_seq) == (6, 1, 6)
    with _conn(db, "cfa4_audit_reader") as conn:  # the database is untouched
        assert conn.execute("SELECT count(*) FROM cfa4_audit.event").fetchone() == (6,)


def test_a_changed_package_is_found(db: str, tmp_path: Path) -> None:  # noqa: F811
    path = _package(db, tmp_path)
    original = path.read_text(encoding="utf-8")
    sidecar = path.with_name(path.name + ".sha256")
    saved = sidecar.read_text(encoding="utf-8")

    path.write_text(original.replace("uid:1000", "uid:1001", 1), encoding="utf-8")
    assert "sha256" in verify_archive(path).detail, "the file no longer matches its sidecar"
    sidecar.write_text(saved, encoding="utf-8")
    path.write_text(original, encoding="utf-8")
    assert verify_archive(path).ok

    def edit_event(doc: dict[str, Any]) -> None:
        doc["events"][2]["actor_ref"] = "someone-else"

    def drop_event(doc: dict[str, Any]) -> None:
        del doc["events"][2]

    def swap_events(doc: dict[str, Any]) -> None:
        doc["events"][2], doc["events"][3] = doc["events"][3], doc["events"][2]

    def edit_manifest(doc: dict[str, Any]) -> None:
        doc["manifest"]["last_seq"] = 99

    def recompute_digest_only(doc: dict[str, Any]) -> None:
        from cfa4_audit.archive import _events_sha256

        doc["events"][2]["actor_ref"] = "someone-else"
        doc["manifest"]["events_sha256"] = _events_sha256(doc["events"])

    for change in (edit_event, drop_event, swap_events, edit_manifest, recompute_digest_only):
        path.write_text(original, encoding="utf-8")
        _rewrite(path, change)
        result = verify_archive(path)
        assert not result.ok and result.code == "AUDIT_ARCHIVE_INVALID", change.__name__
    # the last case shows why each event carries its own hash: a consistent digest cannot hide a changed event
    assert "hash" in result.detail


def test_garbage_and_missing_files_are_a_finding_not_a_crash(tmp_path: Path) -> None:
    for name, content in (("a.json", "not json"), ("b.json", "[]"), ("c.json", '{"format": "other"}'), ("d.json", "")):
        (tmp_path / name).write_text(content, encoding="utf-8")
        assert not verify_archive(tmp_path / name).ok
    assert not verify_archive(tmp_path / "missing.json").ok


def test_an_open_empty_or_existing_month_is_refused(db: str, tmp_path: Path) -> None:  # noqa: F811
    _write(db, 2)
    with _conn(db, "cfa4_audit_reader") as conn:
        with pytest.raises(InvalidEvent, match="ARCHIVE_MONTH_OPEN"):
            archive_month(conn, _THIS_MONTH, tmp_path)
        with pytest.raises(InvalidEvent, match="ARCHIVE_EMPTY"):
            archive_month(conn, date(2020, 1, 15), tmp_path, now=_LATER)
        archive_month(conn, _THIS_MONTH, tmp_path, now=_LATER)
        with pytest.raises(InvalidEvent, match="ARCHIVE_EXISTS"):
            archive_month(conn, _THIS_MONTH, tmp_path, now=_LATER)


def test_a_broken_chain_is_never_archived(db: str, tmp_path: Path) -> None:  # noqa: F811
    _write(db, 4)
    _tamper(db, "UPDATE cfa4_audit.event SET actor_ref = 'someone-else' WHERE seq = 3")
    with _conn(db, "cfa4_audit_reader") as conn:
        with pytest.raises(ChainBroken, match="seq 3"):
            archive_month(conn, _THIS_MONTH, tmp_path, now=_LATER)
    assert not list(tmp_path.rglob("*.cfa4audit.json"))


def test_the_commands_and_the_command_line(db: str, tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:  # noqa: F811
    _write(db, 3)
    with _conn(db, "cfa4_audit_reader") as conn:
        bad = run_command(conn, "audit.archive", {"month": _THIS_MONTH.strftime("%Y-%m"), "root": str(tmp_path)})
        assert bad == {"ok": False, "code": "AUDIT_EVENT_INVALID", "detail": bad["detail"]} and "OPEN" in bad["detail"]
        assert run_command(conn, "audit.archive", {"month": "June"})["code"] == "AUDIT_EVENT_INVALID"
    path = _package(db, tmp_path / "p", 0)
    assert run_command(None, "audit.archive.verify", {"path": str(path)})["ok"] is True, "no database needed"
    assert run_command(None, "audit.list")["code"] == "AUDIT_EVENT_INVALID"
    capsys.readouterr()
    assert main(["archive-verify", "--path", str(path)]) == 0
    assert json.loads(capsys.readouterr().out)["count"] >= 4
    assert main(["archive-verify", "--path", str(tmp_path / "nope.json")]) == 1
