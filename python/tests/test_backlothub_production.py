"""BacklotHub step 2 (G2-02): migration, production commands, audit in the same transaction, permissions."""

from __future__ import annotations

import threading
import uuid
from typing import Any

import pytest

pytest.importorskip("psycopg")
pytest.importorskip("sqlalchemy")
pytest.importorskip("alembic")

import psycopg  # noqa: E402
import psycopg.sql  # noqa: E402
from sqlalchemy import Engine, text  # noqa: E402

from cfa4_audit import verify  # noqa: E402
from cfa4_backlothub import commands, migrate  # noqa: E402
from cfa4_backlothub.db import schema as t  # noqa: E402
from cfa4_backlothub.db.engine import make_engine  # noqa: E402
from cfa4_backlothub.errors import (  # noqa: E402
    BacklotError,
    ConflictStaleVersion,
    CountryUnknown,
    CurrencyUnknown,
    FpsInvalid,
    InvalidInput,
    PermissionDenied,
    ProductionCodeTaken,
    ProductionNotFound,
    StatusNotInList,
    TypeUnknown,
)
from cfa4_backlothub.production import service as svc  # noqa: E402
from backlot_helpers import ME, OTHER, new_db  # noqa: E402
from backlot_helpers import audit_events as _audit  # noqa: E402
from backlot_helpers import backlot_events as _backlot_events  # noqa: E402
from backlot_helpers import count as _count  # noqa: E402
from backlot_helpers import params as _params  # noqa: E402

# ---------------------------------------------------------------- migration


def test_migration_up_down_up_and_seed_data(info: str) -> None:
    eng = make_engine(info.replace("user=postgres", "user=cfa4_test_app"))
    try:
        assert migrate.current(eng) is None
        migrate.upgrade(eng)
        assert migrate.current(eng) == migrate.head() == "0002"
        assert (_count(eng, "production_type"), _count(eng, "status"), _count(eng, "status_list")) == (5, 8, 1)
        assert _count(eng, "country") == 249 and _count(eng, "currency") >= 150
        migrate.upgrade(eng)  # again: nothing is seeded twice
        assert _count(eng, "production_type") == 5
        migrate.downgrade(eng)
        assert migrate.current(eng) is None
        with eng.connect() as conn:
            assert conn.execute(text("SELECT to_regclass('backlot.production')")).scalar_one() is None
        migrate.upgrade(eng)
        assert (_count(eng, "production_type"), _count(eng, "status")) == (5, 8)
    finally:
        eng.dispose()


def test_the_core_tables_match_the_database(engine: Engine) -> None:
    with engine.connect() as conn:
        for table in t.metadata.sorted_tables:
            rows = conn.execute(
                text(
                    "SELECT column_name, is_nullable FROM information_schema.columns "
                    "WHERE table_schema = 'backlot' AND table_name = :t AND is_generated = 'NEVER'"
                ),
                {"t": table.name},
            ).all()
            database = {name: nullable == "YES" for name, nullable in rows}
            mapped = {c.name: bool(c.nullable) for c in table.columns}
            assert set(mapped) == set(database), table.name
            assert {n: v for n, v in mapped.items() if not v} == {n: v for n, v in database.items() if not v}, table.name


# ---------------------------------------------------------------- create


def test_create_makes_the_production_its_owner_and_one_audit_event(engine: Engine) -> None:
    view = svc.create_production(engine, _params(width=1920, height=1080, planned_start="2027-01-10"), ME)
    assert view["code"] == "FILM-1" and view["type"] == "film" and view["status"] == "development"
    assert view["row_version"] == 1 and view["fps_num"] == 24000 and view["planned_start"] == "2027-01-10"
    assert view["pixel_aspect_num"] == 1 and view["created_by"] == ME.ref
    members = svc.list_members(engine, uuid.UUID(view["id"]), ME)
    assert [(m["actor_ref"], m["role"]) for m in members] == [(ME.ref, "owner")]
    events = _backlot_events(engine)
    assert [e["action"] for e in events] == ["production.created"]
    ev = events[0]
    assert ev["subject_id"] == view["id"] and ev["scope"] == {"production_id": view["id"]}
    assert ev["actor_ref"] == ME.ref and ev["command_id"] == "backlot.production.create"
    assert ev["change"]["code"] == {"old": None, "new": "FILM-1"} and ev["change"]["type"]["new"] == "film"
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]


@pytest.mark.parametrize(
    ("changes", "error"),
    [
        ({"fps_num": 0}, FpsInvalid),
        ({"fps_den": -1}, FpsInvalid),
        ({"fps_num": 24.5}, FpsInvalid),
        ({"currency": "XXY"}, CurrencyUnknown),
        ({"currency": "euro"}, CurrencyUnknown),
        ({"reference_currency": "GBP"}, InvalidInput),
        ({"country": "ZZ"}, CountryUnknown),
        ({"code": "x"}, InvalidInput),
        ({"code": "A" * 33}, InvalidInput),
        ({"code": "-BAD"}, InvalidInput),
        ({"title": ""}, InvalidInput),
        ({"title": "t" * 201}, InvalidInput),
        ({"type": "opera"}, TypeUnknown),
        ({"status": "finished"}, StatusNotInList),
        ({"width": 1920}, InvalidInput),
        ({"planned_start": "2027-02-01", "planned_end": "2027-01-01"}, InvalidInput),
        ({"planned_start": "not a date"}, InvalidInput),
        ({"start_timecode": "1:00"}, InvalidInput),
        ({"episode_count": -1}, InvalidInput),
        ({"surprise": 1}, InvalidInput),
    ],
)
def test_bad_input_gets_an_english_code_and_changes_nothing(engine: Engine, changes: dict[str, Any], error: type[BacklotError]) -> None:
    before = len(_audit(engine))
    with pytest.raises(error):
        svc.create_production(engine, _params(**changes), ME)
    assert _count(engine, "production") == 0 and len(_audit(engine)) == before


def test_a_duplicate_code_is_refused(engine: Engine) -> None:
    svc.create_production(engine, _params(), ME)
    with pytest.raises(ProductionCodeTaken):
        svc.create_production(engine, _params(title="Other"), ME)
    assert _count(engine, "production") == 1 and len(_backlot_events(engine)) == 1


def test_a_missing_required_field_is_named(engine: Engine) -> None:
    params = _params()
    del params["currency"]
    with pytest.raises(InvalidInput, match="currency"):
        svc.create_production(engine, params, ME)


def test_if_the_audit_fails_the_business_change_is_gone(engine: Engine, monkeypatch: pytest.MonkeyPatch) -> None:
    from cfa4_backlothub.db import uow as uow_module

    def broken(*_args: Any, **_kwargs: Any) -> None:
        raise RuntimeError("audit down")

    monkeypatch.setattr(uow_module, "record", broken)
    with pytest.raises(RuntimeError):
        svc.create_production(engine, _params(), ME)
    assert _count(engine, "production") == 0 and _count(engine, "production_member") == 0


# ---------------------------------------------------------------- update, status, read


def _created(engine: Engine) -> dict[str, Any]:
    return svc.create_production(engine, _params(), ME)


def test_update_changes_the_fields_and_audits_old_and_new(engine: Engine) -> None:
    p = _created(engine)
    pid = uuid.UUID(p["id"])
    after = svc.update_production(engine, pid, 1, {"title": "A better film", "episode_count": 3}, ME)
    assert after["title"] == "A better film" and after["episode_count"] == 3 and after["row_version"] == 2
    assert after["updated_by"] == ME.ref
    ev = _backlot_events(engine)[-1]
    assert ev["action"] == "production.updated" and ev["change"] == {
        "title": {"old": "A film", "new": "A better film"},
        "episode_count": {"old": None, "new": 3},
    }


def test_a_stale_version_conflicts_and_a_no_op_writes_nothing(engine: Engine) -> None:
    p = _created(engine)
    pid = uuid.UUID(p["id"])
    svc.update_production(engine, pid, 1, {"title": "v2"}, ME)
    with pytest.raises(ConflictStaleVersion):
        svc.update_production(engine, pid, 1, {"title": "v3"}, ME)
    events = len(_backlot_events(engine))
    same = svc.update_production(engine, pid, 2, {"title": "v2"}, ME)
    assert same["row_version"] == 2 and len(_backlot_events(engine)) == events


def test_two_parallel_updates_one_wins_one_conflicts(engine: Engine) -> None:
    pid = uuid.UUID(_created(engine)["id"])
    results: list[str] = []

    def attempt(title: str) -> None:
        try:
            svc.update_production(engine, pid, 1, {"title": title}, ME)
            results.append("ok")
        except ConflictStaleVersion:
            results.append("conflict")

    threads = [threading.Thread(target=attempt, args=(f"t{i}",)) for i in range(2)]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    assert sorted(results) == ["conflict", "ok"]
    assert svc.get_production(engine, pid, ME)["row_version"] == 2
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]


def test_update_rejects_the_code_and_bad_values(engine: Engine) -> None:
    pid = uuid.UUID(_created(engine)["id"])
    with pytest.raises(InvalidInput):
        svc.update_production(engine, pid, 1, {"code": "NEW-1"}, ME)
    with pytest.raises(InvalidInput):
        svc.update_production(engine, pid, 1, {}, ME)
    with pytest.raises(FpsInvalid):
        svc.update_production(engine, pid, 1, {"fps_num": 0}, ME)
    with pytest.raises(InvalidInput):  # a width without a height
        svc.update_production(engine, pid, 1, {"width": 1920}, ME)
    assert svc.update_production(engine, pid, 1, {"width": 1920, "height": 1080}, ME)["height"] == 1080
    with pytest.raises(ProductionNotFound):
        svc.update_production(engine, uuid.uuid4(), 1, {"title": "x"}, ME)


def test_status_change_is_audited_and_checked_against_the_type_list(engine: Engine) -> None:
    pid = uuid.UUID(_created(engine)["id"])
    after = svc.set_status(engine, pid, "production", 1, ME)
    assert after["status"] == "production" and after["status_category"] == "active" and after["row_version"] == 2
    ev = _backlot_events(engine)[-1]
    assert ev["action"] == "production.status.changed" and ev["change"] == {"status": {"old": "development", "new": "production"}}
    with pytest.raises(StatusNotInList):
        svc.set_status(engine, pid, "finished", None, ME)
    with pytest.raises(ConflictStaleVersion):
        svc.set_status(engine, pid, "closed", 1, ME)
    n = len(_backlot_events(engine))
    assert svc.set_status(engine, pid, "production", None, ME)["row_version"] == 2 and len(_backlot_events(engine)) == n


def test_only_members_with_enough_role_may_act(engine: Engine, info: str) -> None:
    pid = uuid.UUID(_created(engine)["id"])
    for call in (
        lambda: svc.get_production(engine, pid, OTHER),
        lambda: svc.list_members(engine, pid, OTHER),
        lambda: svc.update_production(engine, pid, 1, {"title": "x"}, OTHER),
        lambda: svc.set_status(engine, pid, "closed", None, OTHER),
    ):
        with pytest.raises(PermissionDenied):
            call()
    assert svc.list_productions(engine, actor=OTHER) == [] and len(svc.list_productions(engine, actor=ME)) == 1
    with psycopg.connect(info) as conn:  # add anna as a viewer and as a coordinator (the M1 UI has no member.add)
        conn.execute(
            "INSERT INTO backlot.production_member (production_id, actor_kind, actor_ref, role, created_by) "
            "VALUES (%s, 'os_user', %s, 'viewer', 'test')",
            (pid, OTHER.ref),
        )
    assert svc.get_production(engine, pid, OTHER)["code"] == "FILM-1"
    with pytest.raises(PermissionDenied):
        svc.update_production(engine, pid, 1, {"title": "x"}, OTHER)
    with psycopg.connect(info) as conn:
        conn.execute("UPDATE backlot.production_member SET role = 'coordinator' WHERE actor_ref = %s", (OTHER.ref,))
    assert svc.update_production(engine, pid, 1, {"title": "by anna"}, OTHER)["updated_by"] == OTHER.ref
    with pytest.raises(PermissionDenied):  # a coordinator may not change the status
        svc.set_status(engine, pid, "closed", None, OTHER)
    with pytest.raises(ProductionNotFound):
        svc.get_production(engine, uuid.uuid4(), ME)


def test_list_filters_and_scoping(engine: Engine) -> None:
    for i, (ptype, status) in enumerate([("film", None), ("series", "production"), ("vfx", "closed"), ("film", "cancelled")]):
        svc.create_production(engine, _params(code=f"P{i}", type=ptype, **({"status": status} if status else {})), ME)
    assert len(svc.list_productions(engine, actor=ME)) == 4
    assert {p["code"] for p in svc.list_productions(engine, type="film", actor=ME)} == {"P0", "P3"}
    assert [p["code"] for p in svc.list_productions(engine, status="production", actor=ME)] == ["P1"]
    assert {p["code"] for p in svc.list_productions(engine, active=True, actor=ME)} == {"P0", "P1"}
    assert {p["code"] for p in svc.list_productions(engine, active=False, actor=ME)} == {"P2", "P3"}
    assert len(svc.list_productions(engine, limit=2, actor=ME)) == 2
    assert len(svc.list_productions(engine, limit=2, offset=3, actor=ME)) == 1
    with pytest.raises(InvalidInput):
        svc.list_productions(engine, limit=0, actor=ME)
    with pytest.raises(InvalidInput):
        svc.list_productions(engine, limit=501, actor=ME)


def test_the_database_itself_refuses_a_status_of_another_list(engine: Engine) -> None:
    pid = _created(engine)["id"]
    with engine.begin() as conn:
        conn.execute(text("INSERT INTO backlot.status_list (id, code, target, created_by, updated_by) VALUES (:i, 'other', 'production', 't', 't')"), {"i": uuid.uuid4()})
        sid = uuid.uuid4()
        conn.execute(
            text(
                "INSERT INTO backlot.status (id, status_list_id, code, label_key, ordinal, category, created_by, updated_by) "
                "SELECT :s, id, 'x', 'x', 1, 'active', 't', 't' FROM backlot.status_list WHERE code = 'other'"
            ),
            {"s": sid},
        )
    with pytest.raises(Exception, match="status list"):  # noqa: B017
        with engine.begin() as conn:
            conn.execute(text("UPDATE backlot.production SET status_id = :s WHERE id = :p"), {"s": sid, "p": uuid.UUID(pid)})


# ---------------------------------------------------------------- the command registry


def test_every_command_has_a_descriptor_and_a_json_schema() -> None:
    assert {
        "backlot.production.create", "backlot.production.update", "backlot.production.get", "backlot.production.list",
        "backlot.production.status.set", "backlot.production.member.list",
    } <= set(commands.COMMANDS)  # fmt: skip
    for cid, d in ((c["id"], c) for c in commands.list_commands()):
        assert cid.startswith("backlot.") and d["label"].startswith("command.") and d["menu_path"]
        assert d["params"]["type"] == "object" and isinstance(d["params"]["required"], list)
    create = next(c for c in commands.list_commands() if c["id"] == "backlot.production.create")
    assert {"code", "type"} <= set(create["params"]["required"])


def test_run_command_never_raises_and_returns_codes(engine: Engine) -> None:
    ok = commands.run_command(engine, "backlot.production.create", _params(), ME)
    assert ok["ok"] and ok["result"]["code"] == "FILM-1"
    pid = ok["result"]["id"]
    assert commands.run_command(engine, "backlot.production.get", {"id": pid}, ME)["result"]["id"] == pid
    updated = commands.run_command(engine, "backlot.production.update", {"id": pid, "row_version": "1", "title": "New"}, ME)
    assert updated["ok"] and updated["result"]["title"] == "New", "string numbers from the command line are accepted"
    assert commands.run_command(engine, "backlot.production.update", {"id": pid, "row_version": 1, "title": "x"}, ME)["code"] == "CONFLICT_STALE_VERSION"
    assert commands.run_command(engine, "backlot.production.create", _params(fps_num=0, code="B1"), ME)["code"] == "FPS_INVALID"
    assert commands.run_command(engine, "backlot.production.create", {"code": "B2"}, ME)["code"] == "INPUT_INVALID"
    assert commands.run_command(engine, "backlot.production.get", {"id": "not-a-uuid"}, ME)["code"] == "INPUT_INVALID"
    assert commands.run_command(engine, "backlot.production.get", {"id": pid, "x": 1}, ME)["code"] == "INPUT_INVALID"
    assert commands.run_command(engine, "backlot.production.get", {"id": str(uuid.uuid4())}, ME)["code"] == "PRODUCTION_NOT_FOUND"
    assert commands.run_command(engine, "backlot.nothing", {}, ME)["code"] == "BACKLOT_UNKNOWN_COMMAND"
    listed = commands.run_command(engine, "backlot.production.list", {"active": "true"}, ME)
    assert listed["ok"] and len(listed["result"]) == 1
    statused = commands.run_command(engine, "backlot.production.status.set", {"id": pid, "status": "closed"}, ME)
    assert statused["ok"] and statused["result"]["status"] == "closed"
    assert commands.run_command(engine, "backlot.production.member.list", {"id": pid}, ME)["result"][0]["role"] == "owner"


def test_a_database_that_is_gone_is_a_code_not_a_crash() -> None:
    eng = make_engine("host=/nonexistent user=nobody dbname=none connect_timeout=2")
    result = commands.run_command(eng, "backlot.production.list", {}, ME)
    assert result["ok"] is False and result["code"] == "DB_UNAVAILABLE"


# ---------------------------------------------------------------- the command line


def test_the_command_line_migrates_lists_and_runs(cluster: str, capsys: pytest.CaptureFixture[str]) -> None:
    import json

    from cfa4_backlothub.__main__ import main

    conninfo = new_db(cluster).replace("user=postgres", "user=cfa4_test_app")
    capsys.readouterr()
    assert main(["commands"]) == 0 and len(json.loads(capsys.readouterr().out)["commands"]) == len(commands.COMMANDS)
    assert main(["--conninfo", conninfo, "migrate"]) == 0
    assert json.loads(capsys.readouterr().out) == {"ok": True, "revision": migrate.head()}
    created = json.dumps(_params())
    assert main(["--conninfo", conninfo, "run", "backlot.production.create", "--params", created]) == 0
    assert json.loads(capsys.readouterr().out)["result"]["code"] == "FILM-1"
    assert main(["--conninfo", conninfo, "run", "backlot.production.create", "--params", created]) == 1
    assert json.loads(capsys.readouterr().out)["code"] == "PRODUCTION_CODE_TAKEN"
    assert main(["--conninfo", conninfo, "run", "backlot.production.list", "--params", "[1]"]) == 1
    assert main(["--conninfo", "host=/nonexistent connect_timeout=2", "migrate"]) == 2
    assert json.loads(capsys.readouterr().out.splitlines()[-1])["code"] == "DB_UNAVAILABLE"
