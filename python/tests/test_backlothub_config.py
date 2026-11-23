"""BacklotHub step 2 (G2-02), PM-2: production types and status lists."""

from __future__ import annotations

import uuid

import pytest

pytest.importorskip("psycopg")
pytest.importorskip("sqlalchemy")

from sqlalchemy import Engine  # noqa: E402

from backlot_helpers import ME  # noqa: E402
from backlot_helpers import backlot_events as _backlot_events  # noqa: E402
from backlot_helpers import params as _params  # noqa: E402
from cfa4_audit import verify  # noqa: E402
from cfa4_backlothub import commands  # noqa: E402
from cfa4_backlothub.errors import (  # noqa: E402
    ConfigCodeTaken,
    ConflictStaleVersion,
    InvalidInput,
    StatusListNotFound,
    StatusNotFound,
    StatusNotInList,
    TypeInUse,
    TypeUnknown,
)
from cfa4_backlothub.production import config as cfg  # noqa: E402
from cfa4_backlothub.production import service as svc  # noqa: E402


def _list_with_statuses(engine: Engine, code: str = "doc_flow") -> None:
    cfg.create_status_list(engine, code, ME)
    cfg.add_status(engine, code, {"code": "research", "label": "Research", "category": "planned"}, ME)
    cfg.add_status(engine, code, {"code": "shooting", "label": "Shooting", "category": "active"}, ME)
    cfg.add_status(engine, code, {"code": "released", "label": "Released", "category": "closed"}, ME)


def test_a_custom_status_list_and_type_drive_a_production(engine: Engine) -> None:
    _list_with_statuses(engine)
    t = cfg.create_type(engine, {"code": "documentary", "label": "Documentary", "status_list": "doc_flow"}, ME)
    assert t["label_key"] == "text:Documentary" and t["status_list"] == "doc_flow" and not t["retired"]
    lists = {x["code"]: x for x in cfg.list_status_lists(engine)}
    assert [s["code"] for s in lists["doc_flow"]["statuses"]] == ["research", "shooting", "released"]
    assert [s["ordinal"] for s in lists["doc_flow"]["statuses"]] == [1, 2, 3]
    p = svc.create_production(engine, _params(type="documentary"), ME)
    assert p["status"] == "research", "a new production starts at the first status of its list"
    assert svc.set_status(engine, uuid.UUID(p["id"]), "shooting", None, ME)["status_category"] == "active"
    with pytest.raises(StatusNotInList):
        svc.set_status(engine, uuid.UUID(p["id"]), "production", None, ME)  # a status of the default list
    actions = [e["action"] for e in _backlot_events(engine)]
    assert actions[:6] == [
        "config.status_list.created", "config.status.added", "config.status.added", "config.status.added",
        "config.production_type.created", "production.created",
    ]
    ev = _backlot_events(engine)[4]
    assert ev["scope"] == {} and ev["change"]["status_list"] == {"old": None, "new": "doc_flow"}
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]


def test_config_input_and_rules(engine: Engine) -> None:
    for bad in ("X", "1abc", "has space", "a" * 33, ""):
        with pytest.raises(InvalidInput):
            cfg.create_status_list(engine, bad, ME)
    cfg.create_status_list(engine, "flow", ME)
    with pytest.raises(ConfigCodeTaken):
        cfg.create_status_list(engine, "flow", ME)
    with pytest.raises(ConfigCodeTaken):
        cfg.create_status_list(engine, "production_default", ME)
    with pytest.raises(StatusListNotFound):
        cfg.add_status(engine, "nope", {"code": "a1", "label": "A", "category": "active"}, ME)
    for bad_status in ({"code": "a1", "label": "A", "category": "weird"}, {"code": "A", "label": "A", "category": "active"}, {"code": "a1", "label": "", "category": "active"}):
        with pytest.raises(InvalidInput):
            cfg.add_status(engine, "flow", bad_status, ME)
    cfg.add_status(engine, "flow", {"code": "a1", "label": "A", "category": "active"}, ME)
    with pytest.raises(ConfigCodeTaken):
        cfg.add_status(engine, "flow", {"code": "a1", "label": "A", "category": "active"}, ME)
    with pytest.raises(InvalidInput, match="no statuses"):
        cfg.create_type(engine, {"code": "t_empty", "label": "T", "status_list": _empty_list(engine)}, ME)
    with pytest.raises(StatusListNotFound):
        cfg.create_type(engine, {"code": "t1", "label": "T", "status_list": "nope"}, ME)
    with pytest.raises(ConfigCodeTaken):
        cfg.create_type(engine, {"code": "film", "label": "T", "status_list": "flow"}, ME)
    with pytest.raises(TypeUnknown):
        cfg.update_type(engine, "nope", 1, {"label": "x"}, ME)


def _empty_list(engine: Engine) -> str:
    cfg.create_status_list(engine, "empty_list", ME)
    return "empty_list"


def test_updates_conflict_noop_and_audit(engine: Engine) -> None:
    _list_with_statuses(engine)
    lists = {x["code"]: x for x in cfg.list_status_lists(engine)}
    shooting = next(s for s in lists["doc_flow"]["statuses"] if s["code"] == "shooting")
    sid = uuid.UUID(shooting["id"])
    after = cfg.update_status(engine, sid, 1, {"label": "On set", "category": "on_hold", "ordinal": 9}, ME)
    assert (after["label_key"], after["category"], after["ordinal"], after["row_version"]) == ("text:On set", "on_hold", 9, 2)
    assert _backlot_events(engine)[-1]["change"]["category"] == {"old": "active", "new": "on_hold"}
    with pytest.raises(ConflictStaleVersion):
        cfg.update_status(engine, sid, 1, {"label": "x"}, ME)
    n = len(_backlot_events(engine))
    assert cfg.update_status(engine, sid, 2, {"category": "on_hold"}, ME)["row_version"] == 2 and len(_backlot_events(engine)) == n
    for bad in ({}, {"code": "x"}, {"category": "weird"}, {"ordinal": 0}):
        with pytest.raises(InvalidInput):
            cfg.update_status(engine, sid, 2, bad, ME)
    with pytest.raises(StatusNotFound):
        cfg.update_status(engine, uuid.uuid4(), 1, {"label": "x"}, ME)


def test_type_retire_and_the_status_list_guard(engine: Engine) -> None:
    _list_with_statuses(engine)
    cfg.create_type(engine, {"code": "documentary", "label": "Documentary", "status_list": "doc_flow"}, ME)
    svc.create_production(engine, _params(type="documentary"), ME)
    with pytest.raises(TypeInUse):
        cfg.update_type(engine, "documentary", 1, {"status_list": "production_default"}, ME)
    retired = cfg.update_type(engine, "documentary", 1, {"retired": True, "label": "Docs"}, ME)
    assert retired["retired"] and retired["label_key"] == "text:Docs" and retired["row_version"] == 2
    assert "documentary" not in [x["code"] for x in cfg.list_types(engine)]
    assert "documentary" in [x["code"] for x in cfg.list_types(engine, include_retired=True)]
    with pytest.raises(TypeUnknown):  # a retired type takes no new production
        svc.create_production(engine, _params(code="DOC-2", type="documentary"), ME)
    assert svc.list_productions(engine, type="documentary", actor=ME), "the existing production is untouched"
    assert cfg.update_type(engine, "documentary", 2, {"retired": False}, ME)["retired"] is False
    cfg.create_type(engine, {"code": "short_film", "label": "Short film", "status_list": "production_default"}, ME)
    moved = cfg.update_type(engine, "short_film", 1, {"status_list": "doc_flow"}, ME)  # no production has it: allowed
    assert moved["status_list"] == "doc_flow"
    with pytest.raises(InvalidInput):
        cfg.update_type(engine, "short_film", 2, {"retired": "no"}, ME)
    with pytest.raises(InvalidInput):
        cfg.update_type(engine, "short_film", 2, {"code": "x"}, ME)


def test_the_config_commands_run_through_the_registry(engine: Engine) -> None:
    assert commands.run_command(engine, "backlot.status_list.create", {"code": "flow"}, ME)["ok"]
    added = commands.run_command(engine, "backlot.status.add", {"status_list": "flow", "code": "go", "label": "Go", "category": "active"}, ME)
    assert added["ok"] and added["result"]["ordinal"] == 1
    upd = commands.run_command(engine, "backlot.status.update", {"id": added["result"]["id"], "row_version": "1", "category": "closed"}, ME)
    assert upd["ok"] and upd["result"]["category"] == "closed"
    made = commands.run_command(engine, "backlot.production.type.create", {"code": "talk", "label": "Talk show", "status_list": "flow"}, ME)
    assert made["ok"]
    assert commands.run_command(engine, "backlot.production.type.update", {"code": "talk", "row_version": 1, "retired": "true"}, ME)["result"]["retired"]
    assert [t["code"] for t in commands.run_command(engine, "backlot.production.type.list", {}, ME)["result"]] == ["animation", "commercial", "film", "series", "vfx"]
    assert len(commands.run_command(engine, "backlot.status_list.list", {}, ME)["result"]) == 2
    assert commands.run_command(engine, "backlot.status_list.create", {"code": "flow"}, ME)["code"] == "CONFIG_CODE_TAKEN"
