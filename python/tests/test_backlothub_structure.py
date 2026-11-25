"""BacklotHub step 3 (G2-03), ST-1: entity types, templates, the entity tree (ltree), timing and links."""

from __future__ import annotations

import uuid
from typing import Any

import pytest

pytest.importorskip("psycopg")
pytest.importorskip("sqlalchemy")

from sqlalchemy import Engine, inspect, text  # noqa: E402
from sqlalchemy.exc import DBAPIError  # noqa: E402

from cfa4_audit import verify  # noqa: E402
from cfa4_backlothub import commands, migrate  # noqa: E402
from cfa4_backlothub.db import schema as t  # noqa: E402
from cfa4_backlothub.errors import (  # noqa: E402
    ConflictStaleVersion,
    EntityCodeTaken,
    EntityNotFound,
    EntityTypeUnknown,
    InvalidInput,
    LinkDuplicate,
    LinkInvalid,
    LinkNotFound,
    PermissionDenied,
    StructureAssetHasParent,
    StructureCrossProduction,
    StructureRuleViolation,
    TimingInvalid,
)
from cfa4_backlothub.production import service as svc  # noqa: E402
from cfa4_backlothub.structure import entities as ent  # noqa: E402
from backlot_helpers import ME, OTHER  # noqa: E402
from backlot_helpers import backlot_events as _backlot_events  # noqa: E402
from backlot_helpers import params as _params  # noqa: E402


def _production(engine: Engine, code: str = "FILM-1", type: str = "film") -> uuid.UUID:
    return uuid.UUID(svc.create_production(engine, _params(code=code, type=type), ME)["id"])


def _mk(engine: Engine, pid: uuid.UUID, type: str, code: str, parent: str | None = None, **more: Any) -> dict[str, Any]:
    p: dict[str, Any] = {"type": type, "code": code, "name": code.title(), **more}
    if parent is not None:
        p["parent_id"] = uuid.UUID(parent)
    return ent.create_entity(engine, pid, p, ME)


def _tree(engine: Engine, pid: uuid.UUID) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    seq = _mk(engine, pid, "sequence", "SQ010")
    scene = _mk(engine, pid, "scene", "SC010", seq["id"])
    shot = _mk(engine, pid, "shot", "SH0010", scene["id"], frame_in=0, frame_out=47)
    return seq, scene, shot


def _sqlstate(engine: Engine, sql: str, **bind: Any) -> str:
    with pytest.raises(DBAPIError) as err, engine.begin() as conn:
        conn.execute(text(sql), bind)
    return str(err.value.orig.sqlstate)  # type: ignore[union-attr]


# ------------------------------------------------------------ migration


def test_migration_up_down_up_and_seed(info: str) -> None:
    from cfa4_backlothub.db.engine import make_engine

    eng = make_engine(info.replace("user=postgres", "user=cfa4_test_app"))
    try:
        migrate.upgrade(eng)
        with eng.connect() as conn:
            kinds: dict[str, str] = {c: k for c, k in conn.execute(text("SELECT code, kind FROM backlot.entity_type"))}
            assert {k for k, v in kinds.items() if v == "structural"} == {"season", "unit", "episode", "spot", "sequence", "scene", "shot"}
            assert {k for k, v in kinds.items() if v == "asset"} == {"character", "prop", "set", "vehicle", "fx", "environment"}
            assert conn.execute(text("SELECT code FROM backlot.structure_template ORDER BY code")).scalars().all() == [
                "animation", "commercial", "film", "series", "vfx",
            ]  # fmt: skip
            assert conn.execute(text("SELECT has_timing FROM backlot.entity_type WHERE code='shot'")).scalar_one() is True
        migrate.downgrade(eng, "0002")
        with eng.connect() as conn:
            assert conn.execute(text("SELECT to_regclass('backlot.production_entity')")).scalar_one() is None
        migrate.upgrade(eng)
        with eng.connect() as conn:
            assert conn.execute(text("SELECT count(*) FROM backlot.entity_type")).scalar_one() == 13
    finally:
        eng.dispose()


@pytest.mark.filterwarnings("ignore:Did not recognize type .ltree:sqlalchemy.exc.SAWarning")
def test_core_tables_match_the_database(engine: Engine) -> None:
    insp = inspect(engine)
    for name in ("entity_type", "structure_template", "structure_template_rule", "production_structure_rule", "production_entity", "entity_timing", "entity_link"):
        db_cols = {c["name"] for c in insp.get_columns(name, schema="backlot")}
        assert db_cols == {c.name for c in getattr(t, name).columns}, name


# ------------------------------------------------------------ templates and rules


def test_production_create_copies_the_template_rules(engine: Engine) -> None:
    film, series = _production(engine), _production(engine, "SER-1", "series")
    with engine.connect() as conn:
        def rules(pid: uuid.UUID) -> set[tuple[str | None, str]]:
            rows = conn.execute(
                text(
                    "SELECT p.code, c.code FROM backlot.production_structure_rule r "
                    "LEFT JOIN backlot.entity_type p ON p.id = r.parent_type_id JOIN backlot.entity_type c ON c.id = r.child_type_id "
                    "WHERE r.production_id = :p"
                ),
                {"p": pid},
            )
            return {(a, b) for a, b in rows}

        assert rules(film) == {(None, "sequence"), ("sequence", "scene"), ("scene", "shot")}
        assert rules(series) == {(None, "season"), ("season", "episode"), ("episode", "sequence"), ("sequence", "shot")}


def test_rules_are_enforced_by_the_service(engine: Engine) -> None:
    pid = _production(engine)
    with pytest.raises(StructureRuleViolation):
        _mk(engine, pid, "scene", "SC1")  # a scene is not allowed at the root of a film
    seq = _mk(engine, pid, "sequence", "SQ1")
    with pytest.raises(StructureRuleViolation):
        _mk(engine, pid, "shot", "SH1", seq["id"])  # shots go under scenes
    with pytest.raises(EntityTypeUnknown):
        _mk(engine, pid, "nonsense", "X1")
    with pytest.raises(InvalidInput):
        _mk(engine, pid, "sequence", "bad code!")
    assert _backlot_events(engine)[-1]["action"] == "structure.entity.created"  # the failures wrote nothing


def test_the_database_trigger_enforces_the_rules_too(engine: Engine) -> None:
    pid = _production(engine)
    seq, scene, shot = _tree(engine, pid)
    # a shot moved straight under the sequence breaks the rule
    assert _sqlstate(engine, "UPDATE backlot.production_entity SET parent_id = :p WHERE id = :i", p=uuid.UUID(seq["id"]), i=uuid.UUID(shot["id"])) in ("BL101", "BL105")
    # a cycle: the sequence under its own scene
    assert _sqlstate(engine, "UPDATE backlot.production_entity SET parent_id = :p WHERE id = :i", p=uuid.UUID(scene["id"]), i=uuid.UUID(seq["id"])) in ("BL102", "BL101", "BL105")
    # a path that does not match the parent chain
    assert _sqlstate(engine, "UPDATE backlot.production_entity SET path = 'a.b' WHERE id = :i", i=uuid.UUID(scene["id"])) == "BL105"


def test_cross_production_parent_is_refused(engine: Engine) -> None:
    a, b = _production(engine), _production(engine, "FILM-2")
    seq_a = _mk(engine, a, "sequence", "SQ1")
    with pytest.raises(StructureCrossProduction):
        ent.create_entity(engine, b, {"type": "scene", "code": "SC1", "name": "x", "parent_id": uuid.UUID(seq_a["id"])}, ME)


def test_assets_are_flat_and_listed_by_kind(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    hero = _mk(engine, pid, "character", "HERO")
    assert hero["kind"] == "asset" and hero["parent_id"] is None
    with pytest.raises(StructureAssetHasParent):
        _mk(engine, pid, "prop", "KEY", seq["id"])
    with pytest.raises(StructureAssetHasParent):
        _mk(engine, pid, "sequence", "SQ2", hero["id"])
    assets = ent.tree_level(engine, pid, kind="asset", actor=ME)
    assert [a["code"] for a in assets] == ["HERO"]
    assert [a["code"] for a in ent.tree_level(engine, pid, actor=ME)] == ["SQ1"]


# ------------------------------------------------------------ the tree


def test_paths_codes_ordinals_and_lazy_levels(engine: Engine) -> None:
    pid = _production(engine)
    seq, scene, shot = _tree(engine, pid)
    assert seq["path"] == ent.label(uuid.UUID(seq["id"]))
    assert scene["path"] == f"{seq['path']}.{ent.label(uuid.UUID(scene['id']))}"
    assert shot["path"].count(".") == 2 and shot["parent_id"] == scene["id"]
    second = _mk(engine, pid, "sequence", "SQ020")
    assert (seq["ordinal"], second["ordinal"]) == (1, 2)
    top = ent.tree_level(engine, pid, actor=ME)
    assert [(e["code"], e["child_count"]) for e in top] == [("SQ010", 1), ("SQ020", 0)]
    assert [(e["code"], e["child_count"]) for e in ent.tree_level(engine, pid, uuid.UUID(seq["id"]), actor=ME)] == [("SC010", 1)]
    with pytest.raises(EntityCodeTaken):
        _mk(engine, pid, "sequence", "SQ010")
    # the same code under another parent is fine
    other_scene = _mk(engine, pid, "scene", "SC010", second["id"])
    assert other_scene["code"] == "SC010"


def test_update_is_versioned_and_audited_per_field(engine: Engine) -> None:
    pid = _production(engine)
    seq, _scene, _shot = _tree(engine, pid)
    eid = uuid.UUID(seq["id"])
    out = ent.update_entity(engine, eid, 1, {"name": "Opening", "metadata": {"mood": "dark"}}, ME)
    assert out["row_version"] == 2 and out["name"] == "Opening" and out["metadata"] == {"mood": "dark"}
    with pytest.raises(ConflictStaleVersion):
        ent.update_entity(engine, eid, 1, {"name": "Again"}, ME)
    events = [e for e in _backlot_events(engine) if e["action"] == "structure.entity.updated"]
    assert len(events) == 1
    scope = events[0]["scope"] if "scope" in events[0] else events[0]["hashable"]["scope"]
    assert scope["entity_path"] == seq["path"]
    # a no-op writes nothing
    before = len(_backlot_events(engine))
    ent.update_entity(engine, eid, 2, {"name": "Opening"}, ME)
    assert len(_backlot_events(engine)) == before


def test_metadata_must_be_float_free_json(engine: Engine) -> None:
    pid = _production(engine)
    with pytest.raises(InvalidInput):
        _mk(engine, pid, "sequence", "SQ1", metadata={"x": 1.5})
    with pytest.raises(InvalidInput):
        _mk(engine, pid, "sequence", "SQ2", metadata={"x": "a" * 70000})
    assert _mk(engine, pid, "sequence", "SQ3", metadata={"x": [1, {"y": None}]})["metadata"] == {"x": [1, {"y": None}]}


def test_get_entity_and_unknown_id(engine: Engine) -> None:
    pid = _production(engine)
    seq, *_ = _tree(engine, pid)
    assert ent.get_entity(engine, uuid.UUID(seq["id"]), ME)["code"] == "SQ010"
    with pytest.raises(EntityNotFound):
        ent.get_entity(engine, uuid.uuid4(), ME)


# ------------------------------------------------------------ timing


def test_timing_only_on_types_with_timing_and_in_order(engine: Engine) -> None:
    pid = _production(engine)
    seq, scene, shot = _tree(engine, pid)
    assert shot["timing"]["frame_in"] == 0 and shot["timing"]["frame_out"] == 47 and shot["timing"]["handle_in"] == 0
    with pytest.raises(TimingInvalid):
        _mk(engine, pid, "sequence", "SQ2", frame_in=1)
    with pytest.raises(TimingInvalid):
        _mk(engine, pid, "shot", "SH2", scene["id"], frame_in=10, frame_out=5)
    with pytest.raises(TimingInvalid):
        _mk(engine, pid, "shot", "SH3", scene["id"], frame_in=-1)
    updated = ent.update_entity(engine, uuid.UUID(shot["id"]), 1, {"frame_out": 95}, ME)
    assert updated["timing"]["frame_out"] == 95
    with pytest.raises(TimingInvalid):
        ent.update_entity(engine, uuid.UUID(shot["id"]), 2, {"frame_in": 200}, ME)
    assert _sqlstate(engine, "UPDATE backlot.entity_timing SET frame_out = -5 WHERE entity_id = :i", i=uuid.UUID(shot["id"])) in ("BL106", "23514")
    _ = seq


# ------------------------------------------------------------ links


def test_casting_and_setting_links(engine: Engine) -> None:
    pid = _production(engine)
    _seq, scene, shot = _tree(engine, pid)
    hero, kitchen = _mk(engine, pid, "character", "HERO"), _mk(engine, pid, "set", "KITCHEN")
    link = ent.add_link(engine, uuid.UUID(shot["id"]), uuid.UUID(hero["id"]), "casting", {"role": "lead"}, ME)
    assert link["link_type"] == "casting" and link["metadata"] == {"role": "lead"}
    ent.add_link(engine, uuid.UUID(scene["id"]), uuid.UUID(kitchen["id"]), "setting", None, ME)
    with pytest.raises(LinkDuplicate):
        ent.add_link(engine, uuid.UUID(shot["id"]), uuid.UUID(hero["id"]), "casting", None, ME)
    with pytest.raises(LinkInvalid):  # wrong direction
        ent.add_link(engine, uuid.UUID(hero["id"]), uuid.UUID(shot["id"]), "casting", None, ME)
    with pytest.raises(LinkInvalid):  # a character is not a set
        ent.add_link(engine, uuid.UUID(shot["id"]), uuid.UUID(hero["id"]), "setting", None, ME)
    with pytest.raises(LinkInvalid):
        ent.add_link(engine, uuid.UUID(shot["id"]), uuid.UUID(shot["id"]), "ref", None, ME)
    assert len(ent.list_links(engine, uuid.UUID(hero["id"]), actor=ME)) == 1
    assert len(ent.list_links(engine, uuid.UUID(scene["id"]), link_type="casting", actor=ME)) == 0
    ent.remove_link(engine, uuid.UUID(link["id"]), ME)
    with pytest.raises(LinkNotFound):
        ent.remove_link(engine, uuid.UUID(link["id"]), ME)
    actions = [e["action"] for e in _backlot_events(engine)]
    assert actions.count("structure.link.added") == 2 and actions.count("structure.link.removed") == 1


def test_link_across_productions_is_refused(engine: Engine) -> None:
    a, b = _production(engine), _production(engine, "FILM-2")
    _s, _sc, shot = _tree(engine, a)
    hero_b = _mk(engine, b, "character", "HERO")
    with pytest.raises(StructureCrossProduction):
        ent.add_link(engine, uuid.UUID(shot["id"]), uuid.UUID(hero_b["id"]), "casting", None, ME)


# ------------------------------------------------------------ permissions, audit, commands


def test_permissions(engine: Engine) -> None:
    pid = _production(engine)
    seq, *_ = _tree(engine, pid)
    with pytest.raises(PermissionDenied):
        ent.create_entity(engine, pid, {"type": "sequence", "code": "X", "name": "x"}, OTHER)
    with pytest.raises(PermissionDenied):
        ent.update_entity(engine, uuid.UUID(seq["id"]), 1, {"name": "x"}, OTHER)
    with pytest.raises(PermissionDenied):
        ent.get_entity(engine, uuid.UUID(seq["id"]), OTHER)
    with pytest.raises(PermissionDenied):
        ent.tree_level(engine, pid, actor=OTHER)


def test_every_change_is_one_audit_event_and_the_chain_verifies(engine: Engine) -> None:
    pid = _production(engine)
    before = len(_backlot_events(engine))
    seq, scene, shot = _tree(engine, pid)
    hero = _mk(engine, pid, "character", "HERO")
    ent.add_link(engine, uuid.UUID(shot["id"]), uuid.UUID(hero["id"]), "casting", None, ME)
    ent.update_entity(engine, uuid.UUID(scene["id"]), 1, {"description": "Kitchen"}, ME)
    events = _backlot_events(engine)[before:]
    assert [e["action"] for e in events] == [
        "structure.entity.created", "structure.entity.created", "structure.entity.created", "structure.entity.created",
        "structure.link.added", "structure.entity.updated",
    ]  # fmt: skip
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]


def test_commands_are_registered_and_run_by_id(engine: Engine) -> None:
    ids = set(commands.COMMANDS)
    assert {
        "backlot.structure.tree", "backlot.structure.entity.create", "backlot.structure.entity.update", "backlot.structure.entity.get",
        "backlot.structure.link.add", "backlot.structure.link.remove", "backlot.structure.link.list",
    } <= ids  # fmt: skip
    pid = _production(engine)
    out = commands.run_command(engine, "backlot.structure.entity.create", {"production_id": str(pid), "type": "sequence", "code": "SQ1", "name": "One", "metadata": {"a": 1}}, ME)
    assert out["ok"] and out["result"]["metadata"] == {"a": 1}
    listed = commands.run_command(engine, "backlot.structure.tree", {"production_id": str(pid)}, ME)
    assert listed["ok"] and [e["code"] for e in listed["result"]] == ["SQ1"]
    bad = commands.run_command(engine, "backlot.structure.entity.create", {"production_id": str(pid), "type": "scene", "code": "S", "name": "s"}, ME)
    assert bad == {"ok": False, "code": "STRUCTURE_RULE_VIOLATION", "detail": bad["detail"]}
    bad_meta = commands.run_command(engine, "backlot.structure.entity.create", {"production_id": str(pid), "type": "sequence", "code": "S2", "name": "s", "metadata": "x"}, ME)
    assert bad_meta["code"] == "INPUT_INVALID"

