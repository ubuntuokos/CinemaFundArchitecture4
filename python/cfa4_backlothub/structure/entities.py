"""The production entity tree and its links (G2-03): create, update, read, tree levels, links.

Structural elements (season, sequence, scene, shot, ...) form a tree by ``ltree`` path under the production's structure
rules; assets (character, prop, set, ...) belong to the production and are linked to structural elements. The database
triggers enforce the rules (migration 0003); this module checks the same first to give readable codes.
"""

from __future__ import annotations

import re
import uuid
from collections.abc import Mapping
from typing import Any

from sqlalchemy import Connection, Engine, and_, delete, exists, func, insert, or_, select, update
from sqlalchemy.exc import DBAPIError, IntegrityError

from cfa4_audit import uuid7

from ..db import schema as t
from ..db.uow import Actor, Uow, os_actor, uow
from ..errors import (
    ConflictStaleVersion,
    EntityCodeTaken,
    EntityNotFound,
    EntityTypeUnknown,
    InvalidInput,
    LinkDuplicate,
    LinkInvalid,
    LinkNotFound,
    StructureAssetHasParent,
    StructureCrossProduction,
    StructureRuleViolation,
    TimingInvalid,
)
from ..production.service import _WRITE_ROLES, _guard_db, _int, _jsonable, _require_member, _text

CODE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$")
LINK_TYPE_RE = re.compile(r"^[a-z][a-z0-9_]*$")
_UPDATABLE = ("name", "code", "description", "metadata", "frame_in", "frame_out", "handle_in", "handle_out")
_TIMING = ("frame_in", "frame_out", "handle_in", "handle_out")
_METADATA_MAX = 65536
_SCENE_LIKE = ("scene", "shot", "sequence")  # where a casting or setting link may start (G2-19, G2-20)


def label(entity_id: uuid.UUID) -> str:
    """The ltree label of an entity: its UUID without dashes, so a rename never changes a path."""
    return entity_id.hex


def _no_floats(value: Any, depth: int = 0) -> None:
    if depth > 12:
        raise InvalidInput("metadata is nested too deeply")
    if isinstance(value, float):
        raise InvalidInput("metadata must not contain floating-point numbers (money and ratios are decimal strings)")
    if isinstance(value, Mapping):
        for k, v in value.items():
            if not isinstance(k, str):
                raise InvalidInput("metadata keys must be text")
            _no_floats(v, depth + 1)
    elif isinstance(value, list):
        for v in value:
            _no_floats(v, depth + 1)
    elif value is not None and not isinstance(value, (str, int, bool)):
        raise InvalidInput("metadata must be JSON")


def _metadata(value: Any) -> dict[str, Any]:
    if not isinstance(value, Mapping):
        raise InvalidInput("metadata must be an object")
    _no_floats(value)
    if len(repr(value)) > _METADATA_MAX:
        raise InvalidInput("metadata is too large")
    return dict(value)


def _timing_values(fields: Mapping[str, Any]) -> dict[str, int]:
    out: dict[str, int] = {}
    for name in _TIMING:
        if name in fields and fields[name] is not None:
            value = fields[name]
            if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= 2_000_000_000:
                raise TimingInvalid(f"{name} must be a whole number of frames >= 0")
            out[name] = value
    return out


def _view(row: Mapping[Any, Any], type_code: str, kind: str, timing: Mapping[Any, Any] | None = None, child_count: int | None = None) -> dict[str, Any]:
    keys = ("path", "code", "name", "ordinal", "description", "metadata", "created_at", "updated_at", "created_by", "updated_by", "row_version")
    out: dict[str, Any] = {k: _jsonable(row[k]) for k in keys}
    out.update(
        id=str(row["id"]), production_id=str(row["production_id"]), parent_id=str(row["parent_id"]) if row["parent_id"] else None,
        type=type_code, kind=kind, retired=row["retired_at"] is not None,
    )  # fmt: skip
    out["timing"] = None if timing is None else {k: timing[k] for k in (*_TIMING, "otio_ref", "edit_project_ref") if k in timing} | {"edit_project_ref": str(timing["edit_project_ref"]) if timing["edit_project_ref"] else None}
    if child_count is not None:
        out["child_count"] = child_count
    return out


def _entity_row(conn: Connection, entity_id: uuid.UUID) -> Mapping[Any, Any]:
    row = (
        conn.execute(
            select(t.production_entity, t.entity_type.c.code.label("type_code"), t.entity_type.c.kind, t.entity_type.c.has_timing)
            .join(t.entity_type, t.entity_type.c.id == t.production_entity.c.entity_type_id)
            .where(t.production_entity.c.id == entity_id)
        )
        .mappings()
        .first()
    )
    if row is None:
        raise EntityNotFound(str(entity_id))
    return row


def _load(conn: Connection, entity_id: uuid.UUID) -> dict[str, Any]:
    row = _entity_row(conn, entity_id)
    timing = conn.execute(select(t.entity_timing).where(t.entity_timing.c.entity_id == entity_id)).mappings().first()
    return _view(row, row["type_code"], row["kind"], timing)


def _type(conn: Connection, code: Any) -> Mapping[Any, Any]:
    row = conn.execute(select(t.entity_type).where(t.entity_type.c.code == str(code))).mappings().first()
    if row is None or row["retired_at"] is not None:
        raise EntityTypeUnknown(str(code))
    return row


def _rule_exists(conn: Connection, production_id: uuid.UUID, parent_type_id: uuid.UUID | None, child_type_id: uuid.UUID) -> bool:
    parent = t.production_structure_rule.c.parent_type_id.is_(None) if parent_type_id is None else t.production_structure_rule.c.parent_type_id == parent_type_id
    return bool(
        conn.execute(
            select(exists().where(and_(t.production_structure_rule.c.production_id == production_id, parent, t.production_structure_rule.c.child_type_id == child_type_id)))
        ).scalar_one()
    )


def _sibling_filter(production_id: uuid.UUID, parent_id: uuid.UUID | None) -> Any:
    return and_(
        t.production_entity.c.production_id == production_id,
        t.production_entity.c.parent_id.is_(None) if parent_id is None else t.production_entity.c.parent_id == parent_id,
    )


def _code_taken(conn: Connection, production_id: uuid.UUID, parent_id: uuid.UUID | None, type_id: uuid.UUID, code: str, *, except_id: uuid.UUID | None = None) -> bool:
    cond = and_(
        _sibling_filter(production_id, parent_id), t.production_entity.c.entity_type_id == type_id, t.production_entity.c.code == code,
        *( [t.production_entity.c.id != except_id] if except_id else [] ),
    )  # fmt: skip
    return bool(conn.execute(select(exists().where(cond))).scalar_one())


# ---------------------------------------------------------------- create, update, read


def create_entity(engine: Engine, production_id: uuid.UUID, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.create``: ``type`` (code), ``parent_id`` (none: under the production root, or an asset), ``code``,
    ``name``, optional ``description``, ``metadata`` and, for a type with timing, ``frame_in``/``frame_out``/handles."""
    actor = actor or os_actor()
    known = {"type", "parent_id", "code", "name", "description", "metadata", *_TIMING}
    unknown = sorted(set(params) - known)
    if unknown:
        raise InvalidInput(f"unknown field(s): {', '.join(unknown)}")
    missing = [f for f in ("type", "code", "name") if params.get(f) is None]
    if missing:
        raise InvalidInput(f"missing: {', '.join(missing)}")
    code = str(params["code"])
    if not CODE_RE.match(code):
        raise InvalidInput("code must be 1..64 characters of A-Z, a-z, 0-9, _, . and -, starting with a letter or digit")
    name = _text("name", params["name"], max_len=200)
    description = None if params.get("description") is None else _text("description", params["description"], max_len=20000, allow_empty=True)
    metadata = _metadata(params.get("metadata", {}))
    timing = _timing_values(params)
    try:
        with uow(engine, actor) as u:
            _require_member(u.conn, production_id, actor, _WRITE_ROLES)
            return _create(u, production_id, params, code, name, description, metadata, timing)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def _create(u: Uow, production_id: uuid.UUID, params: Mapping[str, Any], code: str, name: str, description: str | None, metadata: dict[str, Any], timing: dict[str, int]) -> dict[str, Any]:
    conn = u.conn
    etype = _type(conn, params["type"])
    parent_id: uuid.UUID | None = params.get("parent_id")
    parent = None
    if parent_id is not None:
        parent = _entity_row(conn, parent_id)
        if parent["production_id"] != production_id:
            raise StructureCrossProduction("the parent is in another production")
        if parent["retired_at"] is not None:
            raise StructureRuleViolation("the parent is retired")
        if etype["kind"] == "asset" or parent["kind"] == "asset":
            raise StructureAssetHasParent("an asset has no parent and cannot be a parent")
    elif etype["kind"] == "structural" and not _rule_exists(conn, production_id, None, etype["id"]):
        raise StructureRuleViolation(f"{etype['code']} is not allowed at the root of this production")
    if parent is not None and not _rule_exists(conn, production_id, parent["entity_type_id"], etype["id"]):
        raise StructureRuleViolation(f"{etype['code']} is not allowed under {parent['type_code']}")
    if _code_taken(conn, production_id, parent_id, etype["id"], code):
        raise EntityCodeTaken(code)
    if timing and not etype["has_timing"]:
        raise TimingInvalid(f"{etype['code']} has no timing")
    if timing:
        _check_timing({"frame_in": timing.get("frame_in"), "frame_out": timing.get("frame_out")})
    ordinal = conn.execute(
        select(func.coalesce(func.max(t.production_entity.c.ordinal), 0) + 1).where(_sibling_filter(production_id, parent_id))
    ).scalar_one()
    eid, now = uuid7(), func.now()
    path = label(eid) if parent is None else f"{parent['path']}.{label(eid)}"
    conn.execute(
        insert(t.production_entity).values(
            id=eid, production_id=production_id, entity_type_id=etype["id"], parent_id=parent_id, path=path, code=code, name=name,
            ordinal=ordinal, description=description, metadata=metadata, created_at=now, updated_at=now, created_by=u.actor.ref,
            updated_by=u.actor.ref, row_version=1,
        )
    )
    if timing:
        conn.execute(insert(t.entity_timing).values(entity_id=eid, **{"handle_in": 0, "handle_out": 0, **timing}))
    view = _load(conn, eid)
    change: dict[str, Any] = {"type": {"old": None, "new": etype["code"]}, "code": {"old": None, "new": code}, "name": {"old": None, "new": name}}
    if parent_id is not None:
        change["parent_id"] = {"old": None, "new": str(parent_id)}
    for k, v in timing.items():
        change[k] = {"old": None, "new": v}
    u.audit(
        "structure.entity.created", "entity", eid, production_id=production_id, entity_path=view["path"], change=change,
        command_id="backlot.structure.entity.create",
    )
    return view


def _check_timing(values: Mapping[str, Any]) -> None:
    fin, fout = values.get("frame_in"), values.get("frame_out")
    if fin is None or fout is None:
        raise TimingInvalid("frame_in and frame_out are given together")
    if fout < fin:
        raise TimingInvalid("frame_out is before frame_in")


def update_entity(engine: Engine, entity_id: uuid.UUID, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.update``: name, code, description, metadata and timing; each changed field is audited
    with its old and new value."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    bad = sorted(set(changes) - set(_UPDATABLE))
    if bad or not changes:
        raise InvalidInput(f"cannot change: {', '.join(bad)}" if bad else "nothing to change")
    values: dict[str, Any] = {}
    if "name" in changes:
        values["name"] = _text("name", changes["name"], max_len=200)
    if "code" in changes:
        if not isinstance(changes["code"], str) or not CODE_RE.match(changes["code"]):
            raise InvalidInput("code must be 1..64 characters of A-Z, a-z, 0-9, _, . and -, starting with a letter or digit")
        values["code"] = changes["code"]
    if "description" in changes:
        values["description"] = None if changes["description"] is None else _text("description", changes["description"], max_len=20000, allow_empty=True)
    if "metadata" in changes:
        values["metadata"] = _metadata(changes["metadata"])
    timing = _timing_values(changes)
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            row = _entity_row(conn, entity_id)
            _require_member(conn, row["production_id"], actor, _WRITE_ROLES)
            before = _load(conn, entity_id)
            if before["row_version"] != row_version:
                raise ConflictStaleVersion(f"current version is {before['row_version']}")
            if timing and not row["has_timing"]:
                raise TimingInvalid(f"{row['type_code']} has no timing")
            if "code" in values and values["code"] != row["code"] and _code_taken(conn, row["production_id"], row["parent_id"], row["entity_type_id"], values["code"], except_id=entity_id):
                raise EntityCodeTaken(values["code"])
            change: dict[str, Any] = {k: {"old": before[k], "new": v} for k, v in values.items() if before[k] != _jsonable(v)}
            old_timing = before["timing"] or {}
            new_timing = {**{k: old_timing.get(k) for k in _TIMING}, **timing}
            timing_changed = {k: v for k, v in timing.items() if old_timing.get(k) != v}
            if timing_changed:
                if before["timing"] is None:
                    new_timing.setdefault("handle_in", 0)
                    new_timing.setdefault("handle_out", 0)
                    new_timing["handle_in"] = new_timing["handle_in"] or 0
                    new_timing["handle_out"] = new_timing["handle_out"] or 0
                _check_timing(new_timing)
                for k in timing_changed:
                    change[k] = {"old": old_timing.get(k), "new": timing_changed[k]}
            if not change:
                return before
            real = {k: v for k, v in values.items() if k in change}
            done = conn.execute(
                update(t.production_entity)
                .where(and_(t.production_entity.c.id == entity_id, t.production_entity.c.row_version == row_version))
                .values(**real, updated_at=func.now(), updated_by=actor.ref, row_version=row_version + 1)
            )
            if done.rowcount != 1:
                raise ConflictStaleVersion("changed by someone else")
            if timing_changed:
                if before["timing"] is None:
                    conn.execute(insert(t.entity_timing).values(entity_id=entity_id, **new_timing))
                else:
                    conn.execute(update(t.entity_timing).where(t.entity_timing.c.entity_id == entity_id).values(**timing_changed))
            after = _load(conn, entity_id)
            u.audit(
                "structure.entity.updated", "entity", entity_id, production_id=row["production_id"], entity_path=after["path"],
                change=change, command_id="backlot.structure.entity.update",
            )
            return after
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def get_entity(engine: Engine, entity_id: uuid.UUID, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity.get``: one element with its timing (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            row = _entity_row(conn, entity_id)
            _require_member(conn, row["production_id"], actor, None)
            return _load(conn, entity_id)
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def tree_level(
    engine: Engine,
    production_id: uuid.UUID,
    parent_id: uuid.UUID | None = None,
    *,
    kind: str = "structural",
    type: str | None = None,
    include_retired: bool = False,
    limit: int = 500,
    offset: int = 0,
    actor: Actor | None = None,
) -> list[dict[str, Any]]:
    """``backlot.structure.tree``: one level of the tree (the children of ``parent_id`` or of the production root), in order,
    each with its ``child_count`` (lazy loading). ``kind="asset"`` lists the production's assets (flat, optionally by ``type``)."""
    actor = actor or os_actor()
    _int("limit", limit, low=1)
    _int("offset", offset, low=0)
    if limit > 2000:
        raise InvalidInput("limit must be at most 2000")
    if kind not in ("structural", "asset"):
        raise InvalidInput("kind must be structural or asset")
    child = t.production_entity.alias("child")
    count = (
        select(func.count())
        .select_from(child)
        .where(and_(child.c.parent_id == t.production_entity.c.id, *( [] if include_retired else [child.c.retired_at.is_(None)] )))
        .scalar_subquery()
    )
    query = (
        select(t.production_entity, t.entity_type.c.code.label("type_code"), t.entity_type.c.kind, count.label("child_count"))
        .join(t.entity_type, t.entity_type.c.id == t.production_entity.c.entity_type_id)
        .where(and_(_sibling_filter(production_id, parent_id), t.entity_type.c.kind == kind))
        .order_by(t.production_entity.c.ordinal, t.production_entity.c.id)
        .limit(limit)
        .offset(offset)
    )
    if not include_retired:
        query = query.where(t.production_entity.c.retired_at.is_(None))
    if type is not None:
        query = query.where(t.entity_type.c.code == type)
    try:
        with engine.connect() as conn:
            _require_member(conn, production_id, actor, None)
            if parent_id is not None:
                parent = _entity_row(conn, parent_id)
                if parent["production_id"] != production_id:
                    raise StructureCrossProduction("the parent is in another production")
            return [_view(r, r["type_code"], r["kind"], None, int(r["child_count"])) for r in conn.execute(query).mappings()]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- links


def _link_view(row: Mapping[Any, Any]) -> dict[str, Any]:
    return {
        "id": str(row["id"]), "production_id": str(row["production_id"]), "from_id": str(row["from_id"]), "to_id": str(row["to_id"]),
        "link_type": row["link_type"], "metadata": row["metadata"], "row_version": row["row_version"],
    }  # fmt: skip


def add_link(engine: Engine, from_id: uuid.UUID, to_id: uuid.UUID, link_type: str, metadata: Mapping[str, Any] | None = None, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.link.add``: a non-hierarchical link; ``casting`` runs from a scene, shot or sequence to a character,
    ``setting`` to a set (G2-19, G2-20)."""
    actor = actor or os_actor()
    if not isinstance(link_type, str) or not LINK_TYPE_RE.match(link_type) or len(link_type) > 40:
        raise InvalidInput("link_type must be lower-case letters, digits and _, starting with a letter")
    meta = _metadata(metadata or {})
    if from_id == to_id:
        raise LinkInvalid("an element cannot be linked to itself")
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            a, b = _entity_row(conn, from_id), _entity_row(conn, to_id)
            if a["production_id"] != b["production_id"]:
                raise StructureCrossProduction("the two ends are in different productions")
            pid = a["production_id"]
            _require_member(conn, pid, actor, _WRITE_ROLES)
            if link_type in ("casting", "setting"):
                wanted = "character" if link_type == "casting" else "set"
                if a["type_code"] not in _SCENE_LIKE or b["type_code"] != wanted:
                    raise LinkInvalid(f"{link_type} runs from a scene, shot or sequence to a {wanted}")
            if conn.execute(select(exists().where(and_(t.entity_link.c.from_id == from_id, t.entity_link.c.to_id == to_id, t.entity_link.c.link_type == link_type)))).scalar_one():
                raise LinkDuplicate(link_type)
            lid, now = uuid7(), func.now()
            conn.execute(
                insert(t.entity_link).values(
                    id=lid, production_id=pid, from_id=from_id, to_id=to_id, link_type=link_type, metadata=meta, created_at=now,
                    updated_at=now, created_by=actor.ref, updated_by=actor.ref, row_version=1,
                )
            )
            u.audit(
                "structure.link.added", "entity_link", lid, production_id=pid, entity_path=str(a["path"]),
                change={"link_type": {"old": None, "new": link_type}, "from_id": {"old": None, "new": str(from_id)}, "to_id": {"old": None, "new": str(to_id)}},
                command_id="backlot.structure.link.add",
            )
            return _link_view(conn.execute(select(t.entity_link).where(t.entity_link.c.id == lid)).mappings().one())
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def remove_link(engine: Engine, link_id: uuid.UUID, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.link.remove``."""
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            row = u.conn.execute(select(t.entity_link).where(t.entity_link.c.id == link_id)).mappings().first()
            if row is None:
                raise LinkNotFound(str(link_id))
            _require_member(u.conn, row["production_id"], actor, _WRITE_ROLES)
            path = u.conn.execute(select(t.production_entity.c.path).where(t.production_entity.c.id == row["from_id"])).scalar_one()
            u.conn.execute(delete(t.entity_link).where(t.entity_link.c.id == link_id))
            u.audit(
                "structure.link.removed", "entity_link", link_id, production_id=row["production_id"], entity_path=str(path),
                change={"link_type": {"old": row["link_type"], "new": None}, "from_id": {"old": str(row["from_id"]), "new": None}, "to_id": {"old": str(row["to_id"]), "new": None}},
                command_id="backlot.structure.link.remove",
            )
            return _link_view(row)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def list_links(engine: Engine, entity_id: uuid.UUID, *, link_type: str | None = None, actor: Actor | None = None) -> list[dict[str, Any]]:
    """``backlot.structure.link.list``: the links that start or end at the element (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            row = _entity_row(conn, entity_id)
            _require_member(conn, row["production_id"], actor, None)
            query = select(t.entity_link).where(or_(t.entity_link.c.from_id == entity_id, t.entity_link.c.to_id == entity_id)).order_by(t.entity_link.c.link_type, t.entity_link.c.id)
            if link_type is not None:
                query = query.where(t.entity_link.c.link_type == link_type)
            return [_link_view(r) for r in conn.execute(query).mappings()]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc
