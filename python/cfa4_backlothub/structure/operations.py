"""Restructuring the entity tree (G2-03 section 5): move, reorder, merge, retire, delete, impact.

A move is one ``UPDATE`` over the whole subtree (``path = new prefix || rest of the old path``); the subtree's elements get
no event of their own, the move is one ``structure.entity.moved`` event with the subtree size. The database triggers are
the last line of defence (rules, cycles, path consistency); this module checks the same first to give readable codes.
"""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from typing import Any

from sqlalchemy import Connection, Engine, and_, delete, exists, func, or_, select, text, update
from sqlalchemy.exc import DBAPIError, IntegrityError

from ..db import schema as t
from ..db.uow import Actor, Uow, os_actor, uow
from ..errors import (
    ConflictStaleVersion,
    EntityCodeTaken,
    EntityInUse,
    InvalidInput,
    StructureAssetHasParent,
    StructureCrossProduction,
    StructureCycle,
    StructureRuleViolation,
)
from ..production.service import _WRITE_ROLES, _guard_db, _int, _require_member
from .entities import _code_taken, _entity_row, _load, _rule_exists, _sibling_filter, label

_IMPACT_LIST_MAX = 500
_IMPACT_DEPTH = 5


def _locked(conn: Connection, entity_id: uuid.UUID, row_version: int | None) -> Mapping[Any, Any]:
    """The entity row, locked for the rest of the transaction; ``CONFLICT_STALE_VERSION`` if the caller read another version."""
    _entity_row(conn, entity_id)
    conn.execute(select(t.production_entity.c.id).where(t.production_entity.c.id == entity_id).with_for_update()).first()
    row = _entity_row(conn, entity_id)
    if row_version is not None and row["row_version"] != row_version:
        raise ConflictStaleVersion(f"current version is {row['row_version']}")
    return row


def _next_ordinal(conn: Connection, production_id: uuid.UUID, parent_id: uuid.UUID | None) -> int:
    return int(conn.execute(select(func.coalesce(func.max(t.production_entity.c.ordinal), 0) + 1).where(_sibling_filter(production_id, parent_id))).scalar_one())


def _subtree_size(conn: Connection, production_id: uuid.UUID, path: str) -> int:
    return int(
        conn.execute(
            text("SELECT count(*) FROM backlot.production_entity WHERE production_id = :p AND path <@ CAST(:path AS ltree)"),
            {"p": production_id, "path": path},
        ).scalar_one()
    )


# ---------------------------------------------------------------- move


def _move_subtree(u: Uow, row: Mapping[Any, Any], new_parent: Mapping[Any, Any] | None) -> tuple[str, int, int]:
    """Move ``row`` and everything below it under ``new_parent`` (None: the production root) with one ``UPDATE``;
    returns the new path, the new ordinal and the subtree size. The caller has checked the rules."""
    conn = u.conn
    old_path = str(row["path"])
    new_path = label(row["id"]) if new_parent is None else f"{new_parent['path']}.{label(row['id'])}"
    new_parent_id = None if new_parent is None else new_parent["id"]
    ordinal = _next_ordinal(conn, row["production_id"], new_parent_id)
    done = conn.execute(
        text(
            """
            UPDATE backlot.production_entity SET
                path = CAST(:new AS ltree) || subpath(path, nlevel(CAST(:old AS ltree)) - 1),
                parent_id = CASE WHEN id = :id THEN CAST(:np AS uuid) ELSE parent_id END,
                ordinal = CASE WHEN id = :id THEN :ord ELSE ordinal END,
                row_version = CASE WHEN id = :id THEN row_version + 1 ELSE row_version END,
                updated_at = CASE WHEN id = :id THEN now() ELSE updated_at END,
                updated_by = CASE WHEN id = :id THEN :actor ELSE updated_by END
            WHERE production_id = :pid AND path <@ CAST(:old AS ltree)"""
        ),
        {"new": "" if new_parent is None else str(new_parent["path"]), "old": old_path, "id": row["id"], "np": new_parent_id, "ord": ordinal, "actor": u.actor.ref, "pid": row["production_id"]},
    )
    return new_path, ordinal, int(done.rowcount)


def _check_target(conn: Connection, row: Mapping[Any, Any], new_parent_id: uuid.UUID | None) -> Mapping[Any, Any] | None:
    """The new parent row, after the rule checks of a move (or of a merge's child move)."""
    if row["kind"] == "asset":
        raise StructureAssetHasParent("an asset has no parent")
    if new_parent_id is None:
        if not _rule_exists(conn, row["production_id"], None, row["entity_type_id"]):
            raise StructureRuleViolation(f"{row['type_code']} is not allowed at the root of this production")
        return None
    parent = _entity_row(conn, new_parent_id)
    if parent["production_id"] != row["production_id"]:
        raise StructureCrossProduction("the new parent is in another production")
    if parent["kind"] == "asset":
        raise StructureAssetHasParent("an asset cannot be a parent")
    if parent["retired_at"] is not None:
        raise StructureRuleViolation("the new parent is retired")
    parent_path, own_path = str(parent["path"]), str(row["path"])
    if parent["id"] == row["id"] or parent_path == own_path or parent_path.startswith(own_path + "."):
        raise StructureCycle("an element cannot move under itself")
    if not _rule_exists(conn, row["production_id"], parent["entity_type_id"], row["entity_type_id"]):
        raise StructureRuleViolation(f"{row['type_code']} is not allowed under {parent['type_code']}")
    return parent


def move_entity(engine: Engine, entity_id: uuid.UUID, row_version: int, new_parent_id: uuid.UUID | None, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.move``: the element and its subtree under another parent (``None``: the production root),
    at the end of the new siblings."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            first = _entity_row(conn, entity_id)
            _require_member(conn, first["production_id"], actor, _WRITE_ROLES)
            row = _locked(conn, entity_id, row_version)
            if row["retired_at"] is not None:
                raise StructureRuleViolation("a retired element cannot be moved")
            if row["parent_id"] == new_parent_id:
                return _load(conn, entity_id)
            parent = _check_target(conn, row, new_parent_id)
            if _code_taken(conn, row["production_id"], new_parent_id, row["entity_type_id"], row["code"], except_id=entity_id):
                raise EntityCodeTaken(row["code"])
            new_path, ordinal, size = _move_subtree(u, row, parent)
            u.audit(
                "structure.entity.moved", "entity", entity_id, production_id=row["production_id"], entity_path=new_path,
                change={
                    "parent_id": {"old": None if row["parent_id"] is None else str(row["parent_id"]), "new": None if new_parent_id is None else str(new_parent_id)},
                    "path": {"old": str(row["path"]), "new": new_path},
                    "subtree_size": {"old": None, "new": size},
                },
                command_id="backlot.structure.entity.move",
            )  # fmt: skip
            return _load(conn, entity_id)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- reorder


def reorder_entity(engine: Engine, entity_id: uuid.UUID, row_version: int, position: int, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.reorder``: put the element at ``position`` (1 = first) among its siblings of the same kind;
    the siblings are renumbered in the same transaction."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    _int("position", position, low=1)
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            first = _entity_row(conn, entity_id)
            _require_member(conn, first["production_id"], actor, _WRITE_ROLES)
            row = _locked(conn, entity_id, row_version)
            e = t.production_entity
            siblings = [
                (r[0], r[1])
                for r in conn.execute(
                    select(e.c.id, e.c.ordinal)
                    .join(t.entity_type, t.entity_type.c.id == e.c.entity_type_id)
                    .where(and_(_sibling_filter(row["production_id"], row["parent_id"]), t.entity_type.c.kind == row["kind"]))
                    .order_by(e.c.ordinal, e.c.id)
                    .with_for_update()
                )
            ]
            if position > len(siblings):
                raise InvalidInput(f"position must be 1..{len(siblings)}")
            ids = [i for i, _ in siblings if i != entity_id]
            ids.insert(position - 1, entity_id)
            old = dict(siblings)
            changed = [(i, n) for n, i in enumerate(ids, start=1) if old[i] != n]
            if not changed:
                return _load(conn, entity_id)
            for i, n in changed:
                conn.execute(
                    update(e).where(e.c.id == i).values(ordinal=n, updated_at=func.now(), updated_by=actor.ref, row_version=e.c.row_version + 1)
                )
            new_ordinal = ids.index(entity_id) + 1
            u.audit(
                "structure.entity.reordered", "entity", entity_id, production_id=row["production_id"], entity_path=str(row["path"]),
                change={"ordinal": {"old": row["ordinal"], "new": new_ordinal}, "siblings_renumbered": {"old": None, "new": len(changed)}},
                command_id="backlot.structure.entity.reorder",
            )  # fmt: skip
            return _load(conn, entity_id)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- retire and restore


def retire_entity(engine: Engine, entity_id: uuid.UUID, row_version: int, retired: bool = True, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.retire``: ``retired_at`` on the element and its subtree (links and references stay);
    ``retired=false`` brings back the element and the part of the subtree that was retired with it."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    if not isinstance(retired, bool):
        raise InvalidInput("retired must be true or false")
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            first = _entity_row(conn, entity_id)
            _require_member(conn, first["production_id"], actor, _WRITE_ROLES)
            row = _locked(conn, entity_id, row_version)
            if (row["retired_at"] is not None) == retired:
                return _load(conn, entity_id)
            e = t.production_entity
            path = str(row["path"])
            if retired:
                where = and_(e.c.production_id == row["production_id"], text("path <@ CAST(:path AS ltree)").bindparams(path=path), e.c.retired_at.is_(None))
                values: dict[str, Any] = {"retired_at": func.now()}
            else:
                if row["parent_id"] is not None and _entity_row(conn, row["parent_id"])["retired_at"] is not None:
                    raise StructureRuleViolation("the parent is retired")
                where = and_(e.c.production_id == row["production_id"], text("path <@ CAST(:path AS ltree)").bindparams(path=path), e.c.retired_at == row["retired_at"])
                values = {"retired_at": None}
            done = conn.execute(update(e).where(where).values(**values))
            conn.execute(update(e).where(e.c.id == entity_id).values(updated_at=func.now(), updated_by=actor.ref, row_version=e.c.row_version + 1))
            u.audit(
                "structure.entity.retired" if retired else "structure.entity.restored", "entity", entity_id, production_id=row["production_id"],
                entity_path=path, change={"retired": {"old": not retired, "new": retired}, "affected": {"old": None, "new": int(done.rowcount)}},
                command_id="backlot.structure.entity.retire",
            )  # fmt: skip
            return _load(conn, entity_id)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- delete


def delete_entity(engine: Engine, entity_id: uuid.UUID, row_version: int, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.delete``: only an element with no children, no links and no reference from another module
    (the database refuses the last one with a foreign key); returns the last values."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            first = _entity_row(conn, entity_id)
            _require_member(conn, first["production_id"], actor, _WRITE_ROLES)
            row = _locked(conn, entity_id, row_version)
            last = _load(conn, entity_id)
            e = t.production_entity
            if conn.execute(select(exists().where(e.c.parent_id == entity_id))).scalar_one():
                raise EntityInUse("it has children")
            if conn.execute(select(exists().where(or_(t.entity_link.c.from_id == entity_id, t.entity_link.c.to_id == entity_id)))).scalar_one():
                raise EntityInUse("it has links")
            conn.execute(delete(t.entity_timing).where(t.entity_timing.c.entity_id == entity_id))
            conn.execute(delete(e).where(e.c.id == entity_id))
            change: dict[str, Any] = {k: {"old": last[k], "new": None} for k in ("type", "code", "name")}
            if row["parent_id"] is not None:
                change["parent_id"] = {"old": str(row["parent_id"]), "new": None}
            u.audit(
                "structure.entity.deleted", "entity", entity_id, production_id=row["production_id"], entity_path=str(row["path"]),
                change=change, command_id="backlot.structure.entity.delete",
            )  # fmt: skip
            return last
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- merge


def merge_entity(engine: Engine, source_id: uuid.UUID, target_id: uuid.UUID, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.merge``: the source's children and links go to the target (same production and type);
    the source is retired. A link that would duplicate an existing one, or point at itself, is dropped."""
    actor = actor or os_actor()
    if source_id == target_id:
        raise InvalidInput("source and target are the same element")
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            src0 = _entity_row(conn, source_id)
            _require_member(conn, src0["production_id"], actor, _WRITE_ROLES)
            for i in sorted((source_id, target_id), key=str):  # a fixed lock order, so two merges cannot deadlock
                _locked(conn, i, None)
            src, dst = _entity_row(conn, source_id), _entity_row(conn, target_id)
            if src["production_id"] != dst["production_id"]:
                raise StructureCrossProduction("the two elements are in different productions")
            if src["entity_type_id"] != dst["entity_type_id"]:
                raise StructureRuleViolation("only two elements of the same type can be merged")
            if src["retired_at"] is not None or dst["retired_at"] is not None:
                raise StructureRuleViolation("a retired element cannot be merged")
            if str(dst["path"]).startswith(str(src["path"]) + "."):
                raise StructureCycle("the target is below the source")
            e = t.production_entity
            children = list(conn.execute(select(e.c.id).where(e.c.parent_id == source_id).order_by(e.c.ordinal, e.c.id)).scalars())
            for cid in children:  # every code must be free under the target before anything moves
                child = _entity_row(conn, cid)
                if _code_taken(conn, src["production_id"], target_id, child["entity_type_id"], child["code"]):
                    raise EntityCodeTaken(child["code"])
            for cid in children:
                child = _entity_row(conn, cid)
                _move_subtree(u, child, _entity_row(conn, target_id))
            moved, dropped = 0, 0
            lk = t.entity_link
            for link in conn.execute(select(lk).where(or_(lk.c.from_id == source_id, lk.c.to_id == source_id))).mappings().all():
                a = target_id if link["from_id"] == source_id else link["from_id"]
                b = target_id if link["to_id"] == source_id else link["to_id"]
                duplicate = conn.execute(select(exists().where(and_(lk.c.from_id == a, lk.c.to_id == b, lk.c.link_type == link["link_type"], lk.c.id != link["id"])))).scalar_one()
                if a == b or duplicate:
                    conn.execute(delete(lk).where(lk.c.id == link["id"]))
                    dropped += 1
                else:
                    conn.execute(update(lk).where(lk.c.id == link["id"]).values(from_id=a, to_id=b, updated_at=func.now(), updated_by=actor.ref, row_version=lk.c.row_version + 1))
                    moved += 1
            conn.execute(update(e).where(e.c.id == source_id).values(retired_at=func.now(), updated_at=func.now(), updated_by=actor.ref, row_version=e.c.row_version + 1))
            u.audit(
                "structure.entity.merged", "entity", source_id, production_id=src["production_id"], entity_path=str(src["path"]),
                change={
                    "merged_into": {"old": None, "new": str(target_id)}, "children_moved": {"old": None, "new": len(children)},
                    "links_moved": {"old": None, "new": moved}, "links_dropped": {"old": None, "new": dropped},
                    "retired": {"old": False, "new": True},
                },
                command_id="backlot.structure.entity.merge",
            )  # fmt: skip
            return _load(conn, target_id)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- impact


def impact(engine: Engine, entity_id: uuid.UUID, *, link_type: str | None = None, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.impact``: what a change to the element touches: its subtree, and the elements reachable over
    links (either direction, at most 5 hops; optionally one ``link_type``) from anything in the subtree. Read only."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            row = _entity_row(conn, entity_id)
            _require_member(conn, row["production_id"], actor, None)
            bind = {"pid": row["production_id"], "path": str(row["path"]), "lt": link_type, "depth": _IMPACT_DEPTH}
            by_type = {
                r[0]: int(r[1])
                for r in conn.execute(
                    text(
                        "SELECT et.code, count(*) FROM backlot.production_entity e JOIN backlot.entity_type et ON et.id = e.entity_type_id "
                        "WHERE e.production_id = :pid AND e.path <@ CAST(:path AS ltree) GROUP BY et.code ORDER BY et.code"
                    ),
                    bind,
                )
            }
            linked = conn.execute(
                text(
                    """
                    WITH RECURSIVE reach(id, depth) AS (
                        SELECT id, 0 FROM backlot.production_entity WHERE production_id = :pid AND path <@ CAST(:path AS ltree)
                      UNION
                        SELECT CASE WHEN l.from_id = r.id THEN l.to_id ELSE l.from_id END, r.depth + 1
                        FROM reach r JOIN backlot.entity_link l ON (l.from_id = r.id OR l.to_id = r.id)
                        WHERE r.depth < :depth AND (CAST(:lt AS text) IS NULL OR l.link_type = CAST(:lt AS text))
                    )
                    SELECT e.id, e.code, et.code AS type, CAST(e.path AS text) AS path, min(r.depth) AS hops
                    FROM reach r JOIN backlot.production_entity e ON e.id = r.id JOIN backlot.entity_type et ON et.id = e.entity_type_id
                    WHERE NOT (e.path <@ CAST(:path AS ltree))
                    GROUP BY e.id, e.code, et.code, e.path ORDER BY min(r.depth), e.code, e.id"""
                ),
                bind,
            ).mappings().all()
            return {
                "entity_id": str(entity_id), "path": str(row["path"]),
                "subtree": {"count": sum(by_type.values()), "by_type": by_type},
                "linked": [{"id": str(r["id"]), "code": r["code"], "type": r["type"], "path": r["path"], "hops": int(r["hops"])} for r in linked[:_IMPACT_LIST_MAX]],
                "linked_count": len(linked), "truncated": len(linked) > _IMPACT_LIST_MAX,
            }  # fmt: skip
    except DBAPIError as exc:
        raise _guard_db(exc) from exc
