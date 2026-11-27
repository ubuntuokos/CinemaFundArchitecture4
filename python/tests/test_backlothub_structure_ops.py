"""BacklotHub step 3 (G2-03), ST-2: move, reorder, merge, retire, delete, impact; the tree invariants under random edits."""

from __future__ import annotations

import random
import uuid
from typing import Any

import pytest

pytest.importorskip("psycopg")
pytest.importorskip("sqlalchemy")

from sqlalchemy import Engine, text  # noqa: E402

from cfa4_audit import verify  # noqa: E402
from cfa4_backlothub import commands  # noqa: E402
from cfa4_backlothub.errors import (  # noqa: E402
    BacklotError,
    ConflictStaleVersion,
    EntityCodeTaken,
    EntityInUse,
    InvalidInput,
    PermissionDenied,
    StructureAssetHasParent,
    StructureCrossProduction,
    StructureCycle,
    StructureRuleViolation,
)
from cfa4_backlothub.production import service as svc  # noqa: E402
from cfa4_backlothub.structure import entities as ent  # noqa: E402
from cfa4_backlothub.structure import operations as ops  # noqa: E402
from backlot_helpers import ME, OTHER  # noqa: E402
from backlot_helpers import backlot_events as _backlot_events  # noqa: E402
from backlot_helpers import params as _params  # noqa: E402


def _production(engine: Engine, code: str = "FILM-1") -> uuid.UUID:
    return uuid.UUID(svc.create_production(engine, _params(code=code), ME)["id"])


def _mk(engine: Engine, pid: uuid.UUID, type: str, code: str, parent: str | None = None, **more: Any) -> dict[str, Any]:
    p: dict[str, Any] = {"type": type, "code": code, "name": code, **more}
    if parent is not None:
        p["parent_id"] = uuid.UUID(parent)
    return ent.create_entity(engine, pid, p, ME)


def _u(value: str) -> uuid.UUID:
    return uuid.UUID(value)


def _get(engine: Engine, eid: str) -> dict[str, Any]:
    return ent.get_entity(engine, _u(eid), ME)


def _v(engine: Engine, eid: str) -> int:
    return int(_get(engine, eid)["row_version"])


def _check_invariants(engine: Engine, pid: uuid.UUID) -> None:
    """path matches the parent chain, no cycle, every parent/child pair has a rule, one production per tree."""
    with engine.connect() as conn:
        bad_path = conn.execute(
            text(
                "SELECT count(*) FROM backlot.production_entity e LEFT JOIN backlot.production_entity p ON p.id = e.parent_id "
                "WHERE e.production_id = :p AND e.path <> CASE WHEN p.id IS NULL THEN backlot.entity_label(e.id) ELSE p.path || backlot.entity_label(e.id) END"
            ),
            {"p": pid},
        ).scalar_one()
        assert bad_path == 0
        repeated = conn.execute(
            text(
                "SELECT count(*) FROM backlot.production_entity WHERE production_id = :p "
                "AND nlevel(path) <> (SELECT count(DISTINCT x) FROM unnest(string_to_array(path::text, '.')) x)"
            ),
            {"p": pid},
        ).scalar_one()
        assert repeated == 0
        no_rule = conn.execute(
            text(
                "SELECT count(*) FROM backlot.production_entity e JOIN backlot.entity_type et ON et.id = e.entity_type_id "
                "LEFT JOIN backlot.production_entity p ON p.id = e.parent_id "
                "WHERE e.production_id = :p AND et.kind = 'structural' AND NOT EXISTS ("
                "  SELECT 1 FROM backlot.production_structure_rule r WHERE r.production_id = e.production_id "
                "  AND r.child_type_id = e.entity_type_id AND r.parent_type_id IS NOT DISTINCT FROM p.entity_type_id)"
            ),
            {"p": pid},
        ).scalar_one()
        assert no_rule == 0
        assert conn.execute(text("SELECT count(*) FROM backlot.production_entity e JOIN backlot.production_entity p ON p.id = e.parent_id WHERE e.production_id <> p.production_id")).scalar_one() == 0


# ------------------------------------------------------------ move


def test_move_takes_the_subtree_in_one_event(engine: Engine) -> None:
    pid = _production(engine)
    a, b = _mk(engine, pid, "sequence", "SQ1"), _mk(engine, pid, "sequence", "SQ2")
    scene = _mk(engine, pid, "scene", "SC1", a["id"])
    shots = [_mk(engine, pid, "shot", f"SH{i}", scene["id"], frame_in=0, frame_out=10) for i in range(3)]
    before = len(_backlot_events(engine))
    moved = ops.move_entity(engine, _u(scene["id"]), 1, _u(b["id"]), ME)
    assert moved["parent_id"] == b["id"] and moved["path"].startswith(b["path"] + ".") and moved["row_version"] == 2
    for s in shots:
        got = _get(engine, s["id"])
        assert got["path"] == f"{moved['path']}.{ent.label(_u(s['id']))}" and got["row_version"] == 1  # descendants keep their version
    events = _backlot_events(engine)[before:]
    assert [e["action"] for e in events] == ["structure.entity.moved"]
    _check_invariants(engine, pid)
    assert [e["code"] for e in ent.tree_level(engine, pid, _u(a["id"]), actor=ME)] == []
    assert [e["code"] for e in ent.tree_level(engine, pid, _u(b["id"]), actor=ME)] == ["SC1"]


def test_move_refusals(engine: Engine) -> None:
    pid, other = _production(engine), _production(engine, "FILM-2")
    a, b = _mk(engine, pid, "sequence", "SQ1"), _mk(engine, pid, "sequence", "SQ2")
    scene = _mk(engine, pid, "scene", "SC1", a["id"])
    shot = _mk(engine, pid, "shot", "SH1", scene["id"])
    twin = _mk(engine, pid, "scene", "SC1", b["id"])
    hero = _mk(engine, pid, "character", "HERO")
    foreign = _mk(engine, other, "sequence", "SQ9")
    with pytest.raises(StructureCycle):
        ops.move_entity(engine, _u(a["id"]), 1, _u(shot["id"]), ME)
    with pytest.raises(StructureRuleViolation):  # a shot cannot sit under a sequence
        ops.move_entity(engine, _u(shot["id"]), 1, _u(a["id"]), ME)
    with pytest.raises(StructureRuleViolation):  # a scene cannot be a root of a film
        ops.move_entity(engine, _u(scene["id"]), 1, None, ME)
    with pytest.raises(EntityCodeTaken):  # SC1 exists under SQ2
        ops.move_entity(engine, _u(scene["id"]), 1, _u(b["id"]), ME)
    with pytest.raises(StructureCrossProduction):
        ops.move_entity(engine, _u(scene["id"]), 1, _u(foreign["id"]), ME)
    with pytest.raises(StructureAssetHasParent):
        ops.move_entity(engine, _u(hero["id"]), 1, _u(a["id"]), ME)
    with pytest.raises(StructureAssetHasParent):
        ops.move_entity(engine, _u(scene["id"]), 1, _u(hero["id"]), ME)
    with pytest.raises(ConflictStaleVersion):
        ops.move_entity(engine, _u(twin["id"]), 7, _u(a["id"]), ME)
    with pytest.raises(PermissionDenied):
        ops.move_entity(engine, _u(twin["id"]), 1, _u(a["id"]), OTHER)
    before = len(_backlot_events(engine))
    assert ops.move_entity(engine, _u(scene["id"]), 1, _u(a["id"]), ME)["row_version"] == 1  # same parent: no change, no event
    assert len(_backlot_events(engine)) == before
    _check_invariants(engine, pid)


# ------------------------------------------------------------ reorder


def test_reorder_renumbers_the_siblings(engine: Engine) -> None:
    pid = _production(engine)
    seqs = [_mk(engine, pid, "sequence", f"SQ{i}") for i in range(1, 5)]
    out = ops.reorder_entity(engine, _u(seqs[3]["id"]), 1, 1, ME)
    assert out["ordinal"] == 1
    assert [e["code"] for e in ent.tree_level(engine, pid, actor=ME)] == ["SQ4", "SQ1", "SQ2", "SQ3"]
    assert [e["ordinal"] for e in ent.tree_level(engine, pid, actor=ME)] == [1, 2, 3, 4]
    ops.reorder_entity(engine, _u(seqs[3]["id"]), 2, 4, ME)
    assert [e["code"] for e in ent.tree_level(engine, pid, actor=ME)] == ["SQ1", "SQ2", "SQ3", "SQ4"]
    assert [e["action"] for e in _backlot_events(engine)].count("structure.entity.reordered") == 2
    with pytest.raises(InvalidInput):
        ops.reorder_entity(engine, _u(seqs[0]["id"]), _get(engine, seqs[0]["id"])["row_version"], 9, ME)
    with pytest.raises(InvalidInput):
        ops.reorder_entity(engine, _u(seqs[0]["id"]), _get(engine, seqs[0]["id"])["row_version"], 0, ME)


def test_reorder_keeps_assets_and_structure_apart(engine: Engine) -> None:
    pid = _production(engine)
    s1 = _mk(engine, pid, "sequence", "SQ1")
    h1, h2 = _mk(engine, pid, "character", "H1"), _mk(engine, pid, "character", "H2")
    ops.reorder_entity(engine, _u(h2["id"]), 1, 1, ME)
    assert [e["code"] for e in ent.tree_level(engine, pid, kind="asset", actor=ME)] == ["H2", "H1"]
    assert _get(engine, s1["id"])["row_version"] == 1
    _ = h1


# ------------------------------------------------------------ retire and restore


def test_retire_covers_the_subtree_and_restore_only_what_went_with_it(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    s1, s2 = _mk(engine, pid, "scene", "SC1", seq["id"]), _mk(engine, pid, "scene", "SC2", seq["id"])
    ops.retire_entity(engine, _u(s2["id"]), 1, True, ME)  # retired on its own first
    out = ops.retire_entity(engine, _u(seq["id"]), 1, True, ME)
    assert out["retired"] and _get(engine, s1["id"])["retired"] and _get(engine, s2["id"])["retired"]
    assert ent.tree_level(engine, pid, actor=ME) == []
    assert len(ent.tree_level(engine, pid, include_retired=True, actor=ME)) == 1
    with pytest.raises(StructureRuleViolation):  # no new child under a retired parent
        _mk(engine, pid, "scene", "SC3", seq["id"])
    with pytest.raises(StructureRuleViolation):
        ops.retire_entity(engine, _u(s1["id"]), _v(engine, s1["id"]), False, ME)  # the parent is retired
    ops.retire_entity(engine, _u(seq["id"]), _v(engine, seq["id"]), False, ME)
    assert not _get(engine, seq["id"])["retired"] and not _get(engine, s1["id"])["retired"]
    assert _get(engine, s2["id"])["retired"]  # it was retired before, and stays so
    actions = [e["action"] for e in _backlot_events(engine)]
    assert actions.count("structure.entity.retired") == 2 and actions.count("structure.entity.restored") == 1
    assert ops.retire_entity(engine, _u(seq["id"]), _v(engine, seq["id"]), False, ME)["row_version"] == _v(engine, seq["id"])  # already active: no-op
    with pytest.raises(StructureRuleViolation):
        ops.move_entity(engine, _u(s2["id"]), _v(engine, s2["id"]), None, ME)


# ------------------------------------------------------------ delete


def test_delete_only_a_leaf_without_links(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    scene = _mk(engine, pid, "scene", "SC1", seq["id"])
    shot = _mk(engine, pid, "shot", "SH1", scene["id"], frame_in=0, frame_out=5)
    hero = _mk(engine, pid, "character", "HERO")
    ent.add_link(engine, _u(shot["id"]), _u(hero["id"]), "casting", None, ME)
    with pytest.raises(EntityInUse):
        ops.delete_entity(engine, _u(seq["id"]), 1, ME)  # children
    with pytest.raises(EntityInUse):
        ops.delete_entity(engine, _u(shot["id"]), 1, ME)  # link
    with pytest.raises(EntityInUse):
        ops.delete_entity(engine, _u(hero["id"]), 1, ME)
    link = ent.list_links(engine, _u(shot["id"]), actor=ME)[0]
    ent.remove_link(engine, _u(link["id"]), ME)
    last = ops.delete_entity(engine, _u(shot["id"]), 1, ME)
    assert last["code"] == "SH1"
    with engine.connect() as conn:
        assert conn.execute(text("SELECT count(*) FROM backlot.entity_timing")).scalar_one() == 0
    ops.delete_entity(engine, _u(scene["id"]), 1, ME)
    ops.delete_entity(engine, _u(seq["id"]), 1, ME)
    assert [e["action"] for e in _backlot_events(engine)].count("structure.entity.deleted") == 3
    _check_invariants(engine, pid)


def test_a_foreign_key_from_another_module_blocks_the_delete(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    with engine.begin() as conn:  # stand-in for a later module's table with the RESTRICT reference G2-03 section 5 describes
        conn.execute(text("CREATE TABLE backlot.zz_task (id serial PRIMARY KEY, entity_id uuid REFERENCES backlot.production_entity(id) ON DELETE RESTRICT)"))
        conn.execute(text("INSERT INTO backlot.zz_task (entity_id) VALUES (:e)"), {"e": _u(seq["id"])})
    with pytest.raises(EntityInUse):
        ops.delete_entity(engine, _u(seq["id"]), 1, ME)
    assert _get(engine, seq["id"])["code"] == "SQ1"
    with engine.begin() as conn:
        conn.execute(text("DROP TABLE backlot.zz_task"))


# ------------------------------------------------------------ merge


def test_merge_moves_children_and_links_and_retires_the_source(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    a, b = _mk(engine, pid, "scene", "SC1", seq["id"]), _mk(engine, pid, "scene", "SC1B", seq["id"])
    sa = _mk(engine, pid, "shot", "SH1", a["id"], frame_in=0, frame_out=9)
    _mk(engine, pid, "shot", "SH2", b["id"], frame_in=0, frame_out=9)
    hero, kitchen = _mk(engine, pid, "character", "HERO"), _mk(engine, pid, "set", "KITCHEN")
    ent.add_link(engine, _u(a["id"]), _u(hero["id"]), "casting", None, ME)
    ent.add_link(engine, _u(b["id"]), _u(hero["id"]), "casting", None, ME)  # would duplicate after the merge
    ent.add_link(engine, _u(a["id"]), _u(kitchen["id"]), "setting", None, ME)
    before = len(_backlot_events(engine))
    target = ops.merge_entity(engine, _u(a["id"]), _u(b["id"]), ME)
    assert [e["code"] for e in ent.tree_level(engine, pid, _u(b["id"]), actor=ME)] == ["SH2", "SH1"]
    assert _get(engine, sa["id"])["path"].startswith(target["path"] + ".")
    assert _get(engine, a["id"])["retired"]
    assert sorted(link["link_type"] for link in ent.list_links(engine, _u(b["id"]), actor=ME)) == ["casting", "setting"]
    assert ent.list_links(engine, _u(a["id"]), actor=ME) == []
    events = _backlot_events(engine)[before:]
    assert [e["action"] for e in events] == ["structure.entity.merged"]
    _check_invariants(engine, pid)


def test_merge_refusals(engine: Engine) -> None:
    pid, other = _production(engine), _production(engine, "FILM-2")
    seq1, seq2 = _mk(engine, pid, "sequence", "SQ1"), _mk(engine, pid, "sequence", "SQ2")
    a, b = _mk(engine, pid, "scene", "A", seq1["id"]), _mk(engine, pid, "scene", "B", seq2["id"])
    _mk(engine, pid, "shot", "SH1", a["id"])
    _mk(engine, pid, "shot", "SH1", b["id"])
    foreign = _mk(engine, other, "scene", "F", _mk(engine, other, "sequence", "SQ1")["id"])
    with pytest.raises(EntityCodeTaken):  # SH1 under both
        ops.merge_entity(engine, _u(a["id"]), _u(b["id"]), ME)
    with pytest.raises(StructureRuleViolation):  # different types
        ops.merge_entity(engine, _u(seq1["id"]), _u(a["id"]), ME)
    with pytest.raises(StructureCrossProduction):
        ops.merge_entity(engine, _u(a["id"]), _u(foreign["id"]), ME)
    with pytest.raises(InvalidInput):
        ops.merge_entity(engine, _u(a["id"]), _u(a["id"]), ME)
    with pytest.raises(PermissionDenied):
        ops.merge_entity(engine, _u(a["id"]), _u(b["id"]), OTHER)
    ops.retire_entity(engine, _u(b["id"]), 1, True, ME)
    with pytest.raises(StructureRuleViolation):
        ops.merge_entity(engine, _u(a["id"]), _u(b["id"]), ME)
    # nothing moved after the refusals
    assert len(ent.tree_level(engine, pid, _u(a["id"]), actor=ME)) == 1
    _check_invariants(engine, pid)


# ------------------------------------------------------------ impact


def test_impact_lists_subtree_and_linked_elements(engine: Engine) -> None:
    pid = _production(engine)
    seq = _mk(engine, pid, "sequence", "SQ1")
    scene = _mk(engine, pid, "scene", "SC1", seq["id"])
    shot = _mk(engine, pid, "shot", "SH1", scene["id"], frame_in=0, frame_out=5)
    other_scene = _mk(engine, pid, "scene", "SC2", seq["id"])
    hero = _mk(engine, pid, "character", "HERO")
    ent.add_link(engine, _u(shot["id"]), _u(hero["id"]), "casting", None, ME)
    ent.add_link(engine, _u(other_scene["id"]), _u(hero["id"]), "casting", None, ME)
    out = ops.impact(engine, _u(scene["id"]), actor=ME)
    assert out["subtree"] == {"count": 2, "by_type": {"scene": 1, "shot": 1}}
    assert [(e["code"], e["hops"]) for e in out["linked"]] == [("HERO", 1), ("SC2", 2)]
    assert out["linked_count"] == 2 and out["truncated"] is False
    assert ops.impact(engine, _u(scene["id"]), link_type="setting", actor=ME)["linked"] == []
    with pytest.raises(PermissionDenied):
        ops.impact(engine, _u(scene["id"]), actor=OTHER)
    before = len(_backlot_events(engine))
    ops.impact(engine, _u(scene["id"]), actor=ME)
    assert len(_backlot_events(engine)) == before  # read only


# ------------------------------------------------------------ commands


def test_commands_run_by_id(engine: Engine) -> None:
    pid = _production(engine)
    a, b = _mk(engine, pid, "sequence", "SQ1"), _mk(engine, pid, "sequence", "SQ2")
    scene = _mk(engine, pid, "scene", "SC1", a["id"])
    run = commands.run_command
    out = run(engine, "backlot.structure.entity.move", {"id": scene["id"], "row_version": 1, "parent_id": b["id"]}, ME)
    assert out["ok"] and out["result"]["parent_id"] == b["id"]
    assert run(engine, "backlot.structure.entity.reorder", {"id": b["id"], "row_version": 1, "position": 1}, ME)["ok"]
    assert run(engine, "backlot.structure.impact", {"id": b["id"]}, ME)["result"]["subtree"]["count"] == 2
    assert run(engine, "backlot.structure.entity.retire", {"id": b["id"], "row_version": _v(engine, b["id"])}, ME)["ok"]
    assert run(engine, "backlot.structure.entity.merge", {"source_id": a["id"], "target_id": a["id"]}, ME)["code"] == "INPUT_INVALID"
    assert run(engine, "backlot.structure.entity.delete", {"id": a["id"], "row_version": _v(engine, a["id"])}, ME)["ok"]
    assert run(engine, "backlot.structure.entity.delete", {"id": b["id"], "row_version": _v(engine, b["id"])}, ME)["code"] == "ENTITY_IN_USE"
    assert run(engine, "backlot.structure.entity.move", {"id": scene["id"], "row_version": 99}, ME)["code"] == "CONFLICT_STALE_VERSION"


def test_the_chain_still_verifies_after_restructuring(engine: Engine) -> None:
    pid = _production(engine)
    a, b = _mk(engine, pid, "sequence", "SQ1"), _mk(engine, pid, "sequence", "SQ2")
    scene = _mk(engine, pid, "scene", "SC1", a["id"])
    ops.move_entity(engine, _u(scene["id"]), 1, _u(b["id"]), ME)
    ops.merge_entity(engine, _u(a["id"]), _u(b["id"]), ME)
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]


# ------------------------------------------------------------ random edits keep the tree consistent


@pytest.mark.parametrize("seed", [1, 2, 3])
def test_random_edits_keep_the_tree_consistent(engine: Engine, seed: int) -> None:
    rng = random.Random(seed)
    pid = _production(engine)
    nodes: dict[str, dict[str, Any]] = {}  # id -> latest view
    counter = 0

    def pick(kind: str | None = None) -> dict[str, Any] | None:
        pool = [v for v in nodes.values() if kind is None or v["type"] == kind]
        return rng.choice(pool) if pool else None

    def refresh(eid: str) -> None:
        nodes[eid] = _get(engine, eid)

    for n in range(3):  # a starting tree, so the edits have something to work on
        seq = _mk(engine, pid, "sequence", f"SQ{n}")
        nodes[seq["id"]] = seq
        for m in range(2):
            scene = _mk(engine, pid, "scene", f"SC{n}{m}", seq["id"])
            nodes[scene["id"]] = scene
            shot = _mk(engine, pid, "shot", f"SH{n}{m}", scene["id"], frame_in=0, frame_out=5)
            nodes[shot["id"]] = shot
    for _ in range(80):
        counter += 1
        op = rng.choice(["create", "create", "move", "move", "move", "reorder", "reorder", "retire", "restore", "merge", "delete"])
        try:
            if op == "create":
                type = rng.choice(["sequence", "scene", "scene", "shot", "shot"])
                parent = None if type == "sequence" else pick({"scene": "sequence", "shot": "scene"}[type])
                if type != "sequence" and parent is None:
                    continue
                code = f"{type[:2].upper()}X{counter}" if rng.random() < 0.8 else f"{type[:2].upper()}{rng.randint(0, 2)}"  # some collisions happen
                extra = {"frame_in": 0, "frame_out": 5} if type == "shot" else {}
                v = _mk(engine, pid, type, code, parent["id"] if parent else None, **extra)
                nodes[v["id"]] = v
            elif op == "move":
                node = pick(rng.choice(["scene", "shot"]))
                if node is None:
                    continue
                parent = pick("sequence" if node["type"] == "scene" else "scene")
                ops.move_entity(engine, _u(node["id"]), node["row_version"], _u(parent["id"]) if parent else None, ME)
            elif op == "reorder":
                node = pick()
                if node is not None:
                    ops.reorder_entity(engine, _u(node["id"]), node["row_version"], rng.randint(1, 3), ME)
                    refresh(node["id"])
            elif op in ("retire", "restore"):
                node = pick()
                if node is not None:
                    ops.retire_entity(engine, _u(node["id"]), node["row_version"], op == "retire", ME)
                    for k in list(nodes):
                        refresh(k)
            elif op == "merge":
                a = pick(rng.choice(["scene", "shot", "sequence"]))
                if a is None:
                    continue
                b = pick(a["type"])
                if b is not None:
                    ops.merge_entity(engine, _u(a["id"]), _u(b["id"]), ME)
                    for k in list(nodes):
                        refresh(k)
            elif op == "delete":
                node = pick()
                if node is not None:
                    ops.delete_entity(engine, _u(node["id"]), node["row_version"], ME)
                    del nodes[node["id"]]
        except BacklotError:
            pass  # a refused edit leaves the tree as it was; the invariants below are what matters
        for k in list(nodes):  # versions may have moved on through subtree or renumbering side effects
            refresh(k)
        _check_invariants(engine, pid)
    actions = [e["action"] for e in _backlot_events(engine)]
    for needed in ("structure.entity.created", "structure.entity.moved", "structure.entity.reordered"):
        assert needed in actions, f"seed {seed} never reached {needed}"
