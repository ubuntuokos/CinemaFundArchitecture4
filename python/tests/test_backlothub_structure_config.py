"""BacklotHub step 3 (G2-03), ST-2: entity types, structure templates and a production's rules."""

from __future__ import annotations

import uuid
from typing import Any

import pytest

pytest.importorskip("psycopg")
pytest.importorskip("sqlalchemy")

from sqlalchemy import Engine  # noqa: E402

from cfa4_audit import verify  # noqa: E402
from cfa4_backlothub import commands  # noqa: E402
from cfa4_backlothub.errors import (  # noqa: E402
    ConfigCodeTaken,
    ConflictStaleVersion,
    EntityTypeInUse,
    EntityTypeUnknown,
    InvalidInput,
    PermissionDenied,
    StructureRuleInUse,
    StructureRuleViolation,
    StructureTemplateNotFound,
    TypeUnknown,
)
from cfa4_backlothub.production import service as svc  # noqa: E402
from cfa4_backlothub.structure import config as sc  # noqa: E402
from cfa4_backlothub.structure import entities as ent  # noqa: E402
from backlot_helpers import ME, OTHER  # noqa: E402
from backlot_helpers import backlot_events as _backlot_events  # noqa: E402
from backlot_helpers import params as _params  # noqa: E402


def _production(engine: Engine, code: str = "FILM-1", type: str = "film") -> uuid.UUID:
    return uuid.UUID(svc.create_production(engine, _params(code=code, type=type), ME)["id"])


def _mk(engine: Engine, pid: uuid.UUID, type: str, code: str, parent: str | None = None) -> dict[str, Any]:
    p: dict[str, Any] = {"type": type, "code": code, "name": code}
    if parent is not None:
        p["parent_id"] = uuid.UUID(parent)
    return ent.create_entity(engine, pid, p, ME)


# ------------------------------------------------------------ entity types


def test_entity_type_create_update_list(engine: Engine) -> None:
    made = sc.create_entity_type(engine, {"code": "cue", "label": "Cue", "kind": "structural", "has_timing": True, "icon": "music"}, ME)
    assert made["code"] == "cue" and made["label"] == "text:Cue" and made["has_timing"] and made["row_version"] == 1
    assert any(t["code"] == "cue" for t in sc.list_entity_types(engine))
    with pytest.raises(ConfigCodeTaken):
        sc.create_entity_type(engine, {"code": "cue", "label": "x", "kind": "asset"}, ME)
    with pytest.raises(InvalidInput):
        sc.create_entity_type(engine, {"code": "prop2", "label": "x", "kind": "asset", "has_timing": True}, ME)
    with pytest.raises(InvalidInput):
        sc.create_entity_type(engine, {"code": "Bad Code", "label": "x", "kind": "asset"}, ME)
    with pytest.raises(InvalidInput):
        sc.create_entity_type(engine, {"code": "okay", "label": "x", "kind": "weird"}, ME)
    out = sc.update_entity_type(engine, "cue", 1, {"label": "Music cue", "retired": True}, ME)
    assert out["row_version"] == 2 and out["retired"] and out["label"] == "text:Music cue"
    assert all(t["code"] != "cue" for t in sc.list_entity_types(engine))
    assert any(t["code"] == "cue" for t in sc.list_entity_types(engine, include_retired=True))
    with pytest.raises(ConflictStaleVersion):
        sc.update_entity_type(engine, "cue", 1, {"label": "again"}, ME)
    with pytest.raises(EntityTypeUnknown):
        sc.update_entity_type(engine, "nope", 1, {"label": "x"}, ME)
    with pytest.raises(InvalidInput):
        sc.update_entity_type(engine, "cue", 2, {"kind": "asset"}, ME)  # the kind never changes
    before = len(_backlot_events(engine))
    sc.update_entity_type(engine, "cue", 2, {"label": "Music cue"}, ME)  # no change, no event
    assert len(_backlot_events(engine)) == before
    pid = _production(engine)
    with pytest.raises(EntityTypeUnknown):  # a retired type takes no new elements
        _mk(engine, pid, "cue", "C1")


def test_has_timing_is_locked_once_the_type_is_used(engine: Engine) -> None:
    pid = _production(engine)
    _mk(engine, pid, "sequence", "SQ1")
    with pytest.raises(EntityTypeInUse):
        sc.update_entity_type(engine, "sequence", 1, {"has_timing": True}, ME)
    free = sc.create_entity_type(engine, {"code": "marker", "label": "Marker", "kind": "structural"}, ME)
    assert sc.update_entity_type(engine, "marker", free["row_version"], {"has_timing": True}, ME)["has_timing"]


# ------------------------------------------------------------ templates


def test_templates_list_and_rule_changes_do_not_touch_existing_productions(engine: Engine) -> None:
    templates = {t["code"]: t for t in sc.list_templates(engine)}
    assert templates["film"]["production_type"] == "film"
    assert {(r["parent"], r["child"]) for r in templates["film"]["rules"]} == {(None, "sequence"), ("sequence", "scene"), ("scene", "shot")}
    pid = _production(engine)
    sc.add_template_rule(engine, "film", {"parent": "sequence", "child": "shot"}, ME)
    assert ("sequence", "shot") in {(r["parent"], r["child"]) for t in sc.list_templates(engine) if t["code"] == "film" for r in t["rules"]}
    assert ("sequence", "shot") not in {(r["parent"], r["child"]) for r in sc.list_rules(engine, pid, ME)}  # the old production keeps its copy
    later = _production(engine, "FILM-2")
    assert ("sequence", "shot") in {(r["parent"], r["child"]) for r in sc.list_rules(engine, later, ME)}
    with pytest.raises(ConfigCodeTaken):
        sc.add_template_rule(engine, "film", {"parent": "sequence", "child": "shot"}, ME)
    sc.remove_template_rule(engine, "film", {"parent": "sequence", "child": "shot"}, ME)
    with pytest.raises(InvalidInput):
        sc.remove_template_rule(engine, "film", {"parent": "sequence", "child": "shot"}, ME)
    with pytest.raises(StructureTemplateNotFound):
        sc.add_template_rule(engine, "nope", {"child": "scene"}, ME)
    with pytest.raises(InvalidInput):  # assets are not part of a structure
        sc.add_template_rule(engine, "film", {"parent": "sequence", "child": "character"}, ME)


def test_template_create_one_per_production_type(engine: Engine) -> None:
    with pytest.raises(ConfigCodeTaken):
        sc.create_template(engine, {"code": "film2", "label": "Film 2", "production_type": "film"}, ME)  # film has one already
    with pytest.raises(TypeUnknown):
        sc.create_template(engine, {"code": "x1", "label": "X", "production_type": "nope"}, ME)
    from cfa4_backlothub.production import config as cfg

    cfg.create_status_list(engine, "doc_flow", ME)
    cfg.add_status(engine, "doc_flow", {"code": "research", "label": "Research", "category": "planned"}, ME)
    cfg.create_type(engine, {"code": "docu", "label": "Documentary", "status_list": "doc_flow"}, ME)
    made = sc.create_template(engine, {"code": "documentary", "label": "Documentary", "production_type": "docu"}, ME)
    assert made["rules"] == []
    sc.add_template_rule(engine, "documentary", {"child": "sequence"}, ME)
    sc.add_template_rule(engine, "documentary", {"parent": "sequence", "child": "shot"}, ME)
    docu = uuid.UUID(svc.create_production(engine, _params(code="DOC-1", type="docu"), ME)["id"])
    assert {(r["parent"], r["child"]) for r in sc.list_rules(engine, docu, ME)} == {(None, "sequence"), ("sequence", "shot")}
    seq = _mk(engine, docu, "sequence", "SQ1")
    assert _mk(engine, docu, "shot", "SH1", seq["id"])["type"] == "shot"


# ------------------------------------------------------------ a production's rules


def test_production_rules_add_and_remove(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    with pytest.raises(StructureRuleViolation):
        _mk(engine, pid, "shot", "SH1", seq["id"])
    sc.add_rule(engine, pid, {"parent": "sequence", "child": "shot"}, ME)
    assert _mk(engine, pid, "shot", "SH1", seq["id"])["type"] == "shot"  # now allowed (service and trigger agree)
    with pytest.raises(StructureRuleInUse):
        sc.remove_rule(engine, pid, {"parent": "sequence", "child": "shot"}, ME)
    with pytest.raises(StructureRuleInUse):
        sc.remove_rule(engine, pid, {"child": "sequence"}, ME)  # a top-level sequence exists
    sc.add_rule(engine, pid, {"child": "episode"}, ME)
    sc.remove_rule(engine, pid, {"child": "episode"}, ME)  # unused: fine
    with pytest.raises(ConfigCodeTaken):
        sc.add_rule(engine, pid, {"parent": "sequence", "child": "shot"}, ME)
    with pytest.raises(InvalidInput):
        sc.remove_rule(engine, pid, {"child": "episode"}, ME)
    actions = [e["action"] for e in _backlot_events(engine)]
    assert actions.count("structure.rule.added") == 2 and actions.count("structure.rule.removed") == 1


def test_production_rules_permissions(engine: Engine) -> None:
    pid = _production(engine)
    with pytest.raises(PermissionDenied):
        sc.add_rule(engine, pid, {"child": "episode"}, OTHER)
    with pytest.raises(PermissionDenied):
        sc.list_rules(engine, pid, OTHER)
    with pytest.raises(PermissionDenied):
        sc.remove_rule(engine, pid, {"child": "sequence"}, OTHER)


# ------------------------------------------------------------ commands and audit


def test_commands_and_chain(engine: Engine) -> None:
    run = commands.run_command
    pid = _production(engine)
    created = run(engine, "backlot.structure.entity_type.create", {"code": "cue", "label": "Cue", "kind": "structural"}, ME)
    assert created["ok"]
    assert run(engine, "backlot.structure.entity_type.update", {"code": "cue", "row_version": 1, "retired": True}, ME)["ok"]
    assert any(t["code"] == "film" for t in run(engine, "backlot.structure.template.list", {}, ME)["result"])
    assert run(engine, "backlot.structure.template.rule.add", {"template": "film", "child": "episode"}, ME)["ok"]
    assert run(engine, "backlot.structure.template.rule.remove", {"template": "film", "child": "episode"}, ME)["ok"]
    assert run(engine, "backlot.structure.rule.add", {"production_id": str(pid), "parent": "sequence", "child": "shot"}, ME)["ok"]
    assert len(run(engine, "backlot.structure.rule.list", {"production_id": str(pid)}, ME)["result"]) == 4
    assert run(engine, "backlot.structure.rule.remove", {"production_id": str(pid), "parent": "sequence", "child": "shot"}, ME)["ok"]
    assert run(engine, "backlot.structure.rule.remove", {"production_id": str(pid), "child": "shot"}, ME)["code"] == "INPUT_INVALID"
    assert run(engine, "backlot.structure.entity_type.list", {}, ME)["ok"]
    config_events = [e for e in _backlot_events(engine) if e["action"].startswith("config.")]
    assert [e["action"] for e in config_events][-4:] == [
        "config.entity_type.created", "config.entity_type.updated", "config.structure_template.rule_added", "config.structure_template.rule_removed",
    ]  # fmt: skip
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]
