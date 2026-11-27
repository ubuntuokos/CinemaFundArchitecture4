"""Entity types, structure templates and a production's structure rules (G2-03 sections 4 and 6).

Types and templates are shared by all productions, so in M1 (one user, #42) any OS user may change them and every change is
one ``config.*`` audit event without a production scope; from M2 a role check replaces that. A production's own rules belong
to the production: owner or producer may change them (``structure.rule.*`` events with the production scope). Changing a
template never touches productions that exist already: they got a copy of the rules when they were created.
"""

from __future__ import annotations

import re
import uuid
from collections.abc import Mapping
from typing import Any

from sqlalchemy import Connection, Engine, and_, delete, exists, func, insert, select, update
from sqlalchemy.exc import DBAPIError, IntegrityError

from cfa4_audit import uuid7

from ..db import schema as t
from ..db.uow import Actor, os_actor, uow
from ..errors import (
    ConfigCodeTaken,
    ConflictStaleVersion,
    EntityTypeInUse,
    EntityTypeUnknown,
    InvalidInput,
    StructureRuleInUse,
    StructureTemplateNotFound,
    TypeUnknown,
)
from ..production.config import _label, _stale
from ..production.service import _STATUS_ROLES, _guard_db, _int, _require_member

_CODE = re.compile(r"^[a-z][a-z0-9_]{1,31}$")
KINDS = ("structural", "asset")


def _code(value: Any) -> str:
    if not isinstance(value, str) or not _CODE.match(value):
        raise InvalidInput("code must be 2..32 characters of a-z, 0-9 and _, starting with a letter")
    return value


def _flag(name: str, value: Any) -> bool:
    if not isinstance(value, bool):
        raise InvalidInput(f"{name} must be true or false")
    return value


def _type_view(row: Mapping[Any, Any]) -> dict[str, Any]:
    return {
        "id": str(row["id"]), "code": row["code"], "label": row["label_key"], "kind": row["kind"], "icon": row["icon"],
        "has_timing": row["has_timing"], "retired": row["retired_at"] is not None, "row_version": row["row_version"],
    }  # fmt: skip


def _find_type(conn: Connection, code: Any) -> Mapping[Any, Any]:
    row = conn.execute(select(t.entity_type).where(t.entity_type.c.code == str(code))).mappings().first()
    if row is None:
        raise EntityTypeUnknown(str(code))
    return row


def _structural_type(conn: Connection, code: Any) -> Mapping[Any, Any]:
    row = _find_type(conn, code)
    if row["kind"] != "structural" or row["retired_at"] is not None:
        raise InvalidInput(f"{code} is not an active structural type")
    return row


# ---------------------------------------------------------------- entity types


def create_entity_type(engine: Engine, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity_type.create``: ``code``, ``label``, ``kind`` (structural or asset), optional ``has_timing``
    (structural only) and ``icon``."""
    actor = actor or os_actor()
    unknown = sorted(set(params) - {"code", "label", "kind", "has_timing", "icon"})
    if unknown:
        raise InvalidInput(f"unknown field(s): {', '.join(unknown)}")
    code, label = _code(params.get("code")), _label(params.get("label"))
    kind = params.get("kind")
    if kind not in KINDS:
        raise InvalidInput("kind must be structural or asset")
    timing = _flag("has_timing", params.get("has_timing", False))
    if timing and kind != "structural":
        raise InvalidInput("only a structural type can have timing")
    icon = params.get("icon")
    if icon is not None and (not isinstance(icon, str) or not 1 <= len(icon) <= 64):
        raise InvalidInput("icon must be a name of 1..64 characters")
    try:
        with uow(engine, actor) as u:
            if u.conn.execute(select(exists().where(t.entity_type.c.code == code))).scalar_one():
                raise ConfigCodeTaken(code)
            tid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.entity_type).values(
                    id=tid, code=code, label_key=label, kind=kind, icon=icon, has_timing=timing, created_at=now, updated_at=now,
                    created_by=actor.ref, updated_by=actor.ref, row_version=1,
                )
            )
            u.audit(
                "config.entity_type.created", "entity_type", tid,
                change={"code": {"old": None, "new": code}, "kind": {"old": None, "new": kind}, "has_timing": {"old": None, "new": timing}},
                command_id="backlot.structure.entity_type.create",
            )  # fmt: skip
            return _type_view(_find_type(u.conn, code))
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def update_entity_type(engine: Engine, code: str, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.entity_type.update``: ``label``, ``icon``, ``retired`` (no new elements of the type; existing ones stay)
    and ``has_timing`` (only while no element has the type, ``ENTITY_TYPE_IN_USE``). The kind never changes."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    bad = sorted(set(changes) - {"label", "icon", "retired", "has_timing"})
    if bad or not changes:
        raise InvalidInput(f"cannot change: {', '.join(bad)}" if bad else "nothing to change")
    try:
        with uow(engine, actor) as u:
            row = _find_type(u.conn, code)
            if row["row_version"] != row_version:
                raise ConflictStaleVersion(f"current version is {row['row_version']}")
            real: dict[str, Any] = {}
            audit: dict[str, Any] = {}
            if "label" in changes and _label(changes["label"]) != row["label_key"]:
                real["label_key"] = _label(changes["label"])
                audit["label"] = {"old": row["label_key"], "new": real["label_key"]}
            if "icon" in changes and changes["icon"] != row["icon"]:
                if changes["icon"] is not None and (not isinstance(changes["icon"], str) or not 1 <= len(changes["icon"]) <= 64):
                    raise InvalidInput("icon must be a name of 1..64 characters")
                real["icon"] = changes["icon"]
                audit["icon"] = {"old": row["icon"], "new": changes["icon"]}
            if "retired" in changes and _flag("retired", changes["retired"]) != (row["retired_at"] is not None):
                real["retired_at"] = func.now() if changes["retired"] else None
                audit["retired"] = {"old": row["retired_at"] is not None, "new": changes["retired"]}
            if "has_timing" in changes and _flag("has_timing", changes["has_timing"]) != row["has_timing"]:
                if changes["has_timing"] and row["kind"] != "structural":
                    raise InvalidInput("only a structural type can have timing")
                if u.conn.execute(select(exists().where(t.production_entity.c.entity_type_id == row["id"]))).scalar_one():
                    raise EntityTypeInUse(code)
                real["has_timing"] = changes["has_timing"]
                audit["has_timing"] = {"old": row["has_timing"], "new": changes["has_timing"]}
            if not real:
                return _type_view(row)
            _stale(
                u.conn.execute(
                    update(t.entity_type)
                    .where(and_(t.entity_type.c.id == row["id"], t.entity_type.c.row_version == row_version))
                    .values(**real, updated_at=func.now(), updated_by=actor.ref, row_version=row_version + 1)
                )
            )
            u.audit("config.entity_type.updated", "entity_type", row["id"], change=audit, command_id="backlot.structure.entity_type.update")
            return _type_view(_find_type(u.conn, code))
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def list_entity_types(engine: Engine, include_retired: bool = False) -> list[dict[str, Any]]:
    """``backlot.structure.entity_type.list``."""
    query = select(t.entity_type).order_by(t.entity_type.c.kind.desc(), t.entity_type.c.code)
    if not include_retired:
        query = query.where(t.entity_type.c.retired_at.is_(None))
    try:
        with engine.connect() as conn:
            return [_type_view(r) for r in conn.execute(query).mappings()]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- templates


def _rules_of(conn: Connection, table: Any, key: Any, value: Any) -> list[dict[str, Any]]:
    parent = t.entity_type.alias("p")
    child = t.entity_type.alias("c")
    rows = conn.execute(
        select(parent.c.code.label("parent"), child.c.code.label("child"))
        .select_from(table.outerjoin(parent, parent.c.id == table.c.parent_type_id).join(child, child.c.id == table.c.child_type_id))
        .where(key == value)
        .order_by(parent.c.code.nulls_first(), child.c.code)
    ).mappings()
    return [{"parent": r["parent"], "child": r["child"]} for r in rows]


def list_templates(engine: Engine) -> list[dict[str, Any]]:
    """``backlot.structure.template.list``: each template with its production type and rules (``parent`` null: top level)."""
    try:
        with engine.connect() as conn:
            out = []
            for r in conn.execute(
                select(t.structure_template, t.production_type.c.code.label("type_code"))
                .join(t.production_type, t.production_type.c.id == t.structure_template.c.production_type_id, isouter=True)
                .order_by(t.structure_template.c.code)
            ).mappings():
                rules = _rules_of(conn, t.structure_template_rule, t.structure_template_rule.c.template_id, r["id"])
                out.append({"id": str(r["id"]), "code": r["code"], "label": r["label_key"], "production_type": r["type_code"], "rules": rules, "row_version": r["row_version"]})
            return out
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def _find_template(conn: Connection, code: Any) -> Mapping[Any, Any]:
    row = conn.execute(select(t.structure_template).where(t.structure_template.c.code == str(code))).mappings().first()
    if row is None:
        raise StructureTemplateNotFound(str(code))
    return row


def create_template(engine: Engine, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.template.create``: an empty template for a production type (one template per type). New productions
    of the type get a copy of its rules."""
    actor = actor or os_actor()
    unknown = sorted(set(params) - {"code", "label", "production_type"})
    if unknown:
        raise InvalidInput(f"unknown field(s): {', '.join(unknown)}")
    code, label = _code(params.get("code")), _label(params.get("label"))
    try:
        with uow(engine, actor) as u:
            ptype = u.conn.execute(select(t.production_type).where(t.production_type.c.code == str(params.get("production_type")))).mappings().first()
            if ptype is None:
                raise TypeUnknown(str(params.get("production_type")))
            if u.conn.execute(select(exists().where(t.structure_template.c.code == code))).scalar_one():
                raise ConfigCodeTaken(code)
            if u.conn.execute(select(exists().where(t.structure_template.c.production_type_id == ptype["id"]))).scalar_one():
                raise ConfigCodeTaken(f"production type {ptype['code']} has a template already")
            tid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.structure_template).values(
                    id=tid, code=code, label_key=label, production_type_id=ptype["id"], created_at=now, updated_at=now,
                    created_by=actor.ref, updated_by=actor.ref, row_version=1,
                )
            )
            u.audit(
                "config.structure_template.created", "structure_template", tid,
                change={"code": {"old": None, "new": code}, "production_type": {"old": None, "new": ptype["code"]}},
                command_id="backlot.structure.template.create",
            )  # fmt: skip
            return {"id": str(tid), "code": code, "label": label, "production_type": ptype["code"], "rules": [], "row_version": 1}
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def _pair(conn: Connection, params: Mapping[str, Any]) -> tuple[Mapping[Any, Any] | None, Mapping[Any, Any]]:
    parent_code, child_code = params.get("parent"), params.get("child")
    if child_code is None:
        raise InvalidInput("missing: child")
    return (None if parent_code is None else _structural_type(conn, parent_code)), _structural_type(conn, child_code)


def _template_rule(engine: Engine, template: str, params: Mapping[str, Any], add: bool, actor: Actor | None) -> dict[str, Any]:
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            tpl = _find_template(u.conn, template)
            parent, child = _pair(u.conn, params)
            r = t.structure_template_rule
            same = and_(
                r.c.template_id == tpl["id"], r.c.child_type_id == child["id"],
                r.c.parent_type_id.is_(None) if parent is None else r.c.parent_type_id == parent["id"],
            )  # fmt: skip
            present = u.conn.execute(select(exists().where(same))).scalar_one()
            pair = {"parent": None if parent is None else parent["code"], "child": child["code"]}
            if add == bool(present):
                if add:
                    raise ConfigCodeTaken("the template has this rule already")
                raise InvalidInput("the template has no such rule")
            if add:
                u.conn.execute(insert(r).values(template_id=tpl["id"], parent_type_id=None if parent is None else parent["id"], child_type_id=child["id"]))
            else:
                u.conn.execute(delete(r).where(same))
            u.conn.execute(update(t.structure_template).where(t.structure_template.c.id == tpl["id"]).values(updated_at=func.now(), updated_by=actor.ref, row_version=t.structure_template.c.row_version + 1))
            u.audit(
                "config.structure_template.rule_added" if add else "config.structure_template.rule_removed", "structure_template", tpl["id"],
                change={"rule": {"old": None if add else pair, "new": pair if add else None}, "template": {"old": None, "new": template}},
                command_id="backlot.structure.template.rule.add" if add else "backlot.structure.template.rule.remove",
            )  # fmt: skip
            return {"template": template, **pair}
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def add_template_rule(engine: Engine, template: str, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.template.rule.add``: ``parent`` (a structural type code, or none for the top level) and ``child``."""
    return _template_rule(engine, template, params, True, actor)


def remove_template_rule(engine: Engine, template: str, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.template.rule.remove``: productions that exist keep their rules."""
    return _template_rule(engine, template, params, False, actor)


# ---------------------------------------------------------------- a production's rules


def list_rules(engine: Engine, production_id: uuid.UUID, actor: Actor | None = None) -> list[dict[str, Any]]:
    """``backlot.structure.rule.list``: the production's rules (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            _require_member(conn, production_id, actor, None)
            return _rules_of(conn, t.production_structure_rule, t.production_structure_rule.c.production_id, production_id)
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def _production_rule(engine: Engine, production_id: uuid.UUID, params: Mapping[str, Any], add: bool, actor: Actor | None) -> dict[str, Any]:
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            _require_member(u.conn, production_id, actor, _STATUS_ROLES)
            parent, child = _pair(u.conn, params)
            r = t.production_structure_rule
            same = and_(
                r.c.production_id == production_id, r.c.child_type_id == child["id"],
                r.c.parent_type_id.is_(None) if parent is None else r.c.parent_type_id == parent["id"],
            )  # fmt: skip
            present = u.conn.execute(select(exists().where(same))).scalar_one()
            pair = {"parent": None if parent is None else parent["code"], "child": child["code"]}
            if add == bool(present):
                if add:
                    raise ConfigCodeTaken("the production has this rule already")
                raise InvalidInput("the production has no such rule")
            if add:
                u.conn.execute(insert(r).values(production_id=production_id, parent_type_id=None if parent is None else parent["id"], child_type_id=child["id"]))
            else:
                e, p = t.production_entity, t.production_entity.alias("pe")
                used = u.conn.execute(
                    select(
                        exists()
                        .select_from(e.outerjoin(p, p.c.id == e.c.parent_id))
                        .where(and_(e.c.production_id == production_id, e.c.entity_type_id == child["id"], p.c.id.is_(None) if parent is None else p.c.entity_type_id == parent["id"]))
                    )
                ).scalar_one()
                if used:
                    raise StructureRuleInUse(f"{pair['child']} elements exist " + ("at the top level" if parent is None else f"under {pair['parent']}"))
                u.conn.execute(delete(r).where(same))
            u.audit(
                "structure.rule.added" if add else "structure.rule.removed", "production", production_id, production_id=production_id,
                change={"rule": {"old": None if add else pair, "new": pair if add else None}},
                command_id="backlot.structure.rule.add" if add else "backlot.structure.rule.remove",
            )  # fmt: skip
            return pair
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def add_rule(engine: Engine, production_id: uuid.UUID, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.rule.add``: allow ``child`` under ``parent`` (none: at the top level) in the production (owner, producer)."""
    return _production_rule(engine, production_id, params, True, actor)


def remove_rule(engine: Engine, production_id: uuid.UUID, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.structure.rule.remove``: refused (``STRUCTURE_RULE_IN_USE``) while an element uses the rule."""
    return _production_rule(engine, production_id, params, False, actor)
