"""Organizations and their roles in a production (G2-02): create, update, retire, list, link, unlink.

Organizations are shared between productions (no ``production_id``), so any OS user of the M1 machine may create and
edit them; linking one to a production needs a write role in that production. Audit ``organization.*`` and
``production.organization.linked/unlinked``.
"""

from __future__ import annotations

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
    InvalidInput,
    OrganizationAlreadyLinked,
    OrganizationInUse,
    OrganizationNotFound,
    OrganizationNotLinked,
    OrganizationRetired,
)
from .service import _WRITE_ROLES, _guard_db, _int, _jsonable, _require_member, _text, _validate_fields

ROLES = ("client", "producer", "coproducer", "subcontractor", "distributor", "other")
_TEXT_LIMITS = {"legal_name": 300, "registration_no": 64, "tax_no": 64, "notes": 20000}
_FIELDS = ("name", "legal_name", "country", "registration_no", "tax_no", "notes")
_OPEN = ("planned", "active", "on_hold")  # a production in these categories still needs its organizations


def _view(row: Mapping[Any, Any]) -> dict[str, Any]:
    out = {k: _jsonable(row[k]) for k in (*_FIELDS, "retired_at", "created_at", "updated_at", "created_by", "updated_by", "row_version")}
    out["id"], out["retired"] = str(row["id"]), row["retired_at"] is not None
    return out


def _load(conn: Connection, organization_id: uuid.UUID) -> dict[str, Any]:
    row = conn.execute(select(t.organization).where(t.organization.c.id == organization_id)).mappings().first()
    if row is None:
        raise OrganizationNotFound(str(organization_id))
    return _view(row)


def _clean(conn: Connection, fields: Mapping[str, Any], *, creating: bool) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for name, value in fields.items():
        if name not in _FIELDS:
            raise InvalidInput(f"unknown field {name!r}")
        if value is None:
            if name == "name":
                raise InvalidInput("name must not be empty")
            out[name] = None
        elif name == "name":
            out[name] = _text(name, value, max_len=200)
        elif name == "country":
            out[name] = _validate_fields(conn, {"country": value})["country"]
        else:
            out[name] = _text(name, value, max_len=_TEXT_LIMITS[name], allow_empty=False)
    if creating and "name" not in out:
        raise InvalidInput("missing: name")
    return out


def create_organization(engine: Engine, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.organization.create``."""
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            values = _clean(u.conn, params, creating=True)
            oid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.organization).values(
                    id=oid, created_at=now, updated_at=now, created_by=actor.ref, updated_by=actor.ref, row_version=1, **values
                )
            )
            view = _load(u.conn, oid)
            u.audit(
                "organization.created", "organization", oid,
                change={k: {"old": None, "new": view[k]} for k in _FIELDS if view[k] is not None},
                command_id="backlot.organization.create",
            )
            return view
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def update_organization(
    engine: Engine, organization_id: uuid.UUID, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None
) -> dict[str, Any]:
    """``backlot.organization.update``: fields, and ``retired`` (true/false) to retire or bring back an organization.
    Retiring one that is linked to an open production fails with ``ORGANIZATION_IN_USE``."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    if not changes:
        raise InvalidInput("nothing to change")
    if "retired" in changes and not isinstance(changes["retired"], bool):
        raise InvalidInput("retired must be true or false")
    try:
        with uow(engine, actor) as u:
            before = _load(u.conn, organization_id)
            if before["row_version"] != row_version:
                raise ConflictStaleVersion(f"current version is {before['row_version']}")
            values = _clean(u.conn, {k: v for k, v in changes.items() if k != "retired"}, creating=False)
            real: dict[str, Any] = {k: v for k, v in values.items() if _jsonable(v) != before[k]}
            if "retired" in changes and changes["retired"] != before["retired"]:
                if changes["retired"]:
                    _refuse_if_in_use(u, organization_id)
                    real["retired_at"] = func.now()
                else:
                    real["retired_at"] = None
            if not real:
                return before
            done = u.conn.execute(
                update(t.organization)
                .where(and_(t.organization.c.id == organization_id, t.organization.c.row_version == row_version))
                .values(**real, updated_at=func.now(), updated_by=actor.ref, row_version=row_version + 1)
            )
            if done.rowcount != 1:
                raise ConflictStaleVersion("changed by someone else")
            after = _load(u.conn, organization_id)
            keys = [("retired" if k == "retired_at" else k) for k in real]
            u.audit(
                "organization.updated", "organization", organization_id,
                change={k: {"old": before[k], "new": after[k]} for k in keys}, command_id="backlot.organization.update",
            )
            return after
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def _refuse_if_in_use(u: Uow, organization_id: uuid.UUID) -> None:
    busy = u.conn.execute(
        select(t.production.c.code)
        .join(t.production_organization, t.production_organization.c.production_id == t.production.c.id)
        .join(t.status, t.status.c.id == t.production.c.status_id)
        .where(and_(t.production_organization.c.organization_id == organization_id, t.status.c.category.in_(_OPEN)))
        .distinct()
        .order_by(t.production.c.code)
        .limit(5)
    ).scalars().all()
    if busy:
        raise OrganizationInUse(f"linked to open production(s): {', '.join(busy)}")


def list_organizations(
    engine: Engine, *, query: str | None = None, include_retired: bool = False, limit: int = 100, offset: int = 0
) -> list[dict[str, Any]]:
    """``backlot.organization.list``: by name; ``query`` matches part of the name or legal name (case-insensitive)."""
    _int("limit", limit, low=1)
    _int("offset", offset, low=0)
    if limit > 500:
        raise InvalidInput("limit must be at most 500")
    stmt = select(t.organization).order_by(func.lower(t.organization.c.name), t.organization.c.id).limit(limit).offset(offset)
    if not include_retired:
        stmt = stmt.where(t.organization.c.retired_at.is_(None))
    if query:
        like = "%" + query.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_") + "%"
        stmt = stmt.where(or_(t.organization.c.name.ilike(like, escape="\\"), t.organization.c.legal_name.ilike(like, escape="\\")))
    try:
        with engine.connect() as conn:
            return [_view(r) for r in conn.execute(stmt).mappings()]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def _check_role(role: Any) -> str:
    if role not in ROLES:
        raise InvalidInput(f"role must be one of: {', '.join(ROLES)}")
    return str(role)


def link_organization(
    engine: Engine, production_id: uuid.UUID, organization_id: uuid.UUID, role: str, actor: Actor | None = None
) -> dict[str, Any]:
    """``backlot.production.organization.link``: the organization with a role in the production."""
    actor = actor or os_actor()
    role = _check_role(role)
    try:
        with uow(engine, actor) as u:
            _require_member(u.conn, production_id, actor, _WRITE_ROLES)
            org = _load(u.conn, organization_id)
            if org["retired"]:
                raise OrganizationRetired(org["name"])
            taken = u.conn.execute(
                select(
                    exists().where(
                        and_(
                            t.production_organization.c.production_id == production_id,
                            t.production_organization.c.organization_id == organization_id,
                            t.production_organization.c.role == role,
                        )
                    )
                )
            ).scalar_one()
            if taken:
                raise OrganizationAlreadyLinked(f"{org['name']} as {role}")
            u.conn.execute(
                insert(t.production_organization).values(
                    production_id=production_id, organization_id=organization_id, role=role, created_at=func.now(), created_by=actor.ref
                )
            )
            u.audit(
                "production.organization.linked", "organization", organization_id, production_id=production_id,
                change={"role": {"old": None, "new": role}, "name": {"old": None, "new": org["name"]}},
                command_id="backlot.production.organization.link",
            )
            return {"production_id": str(production_id), "organization_id": str(organization_id), "role": role, "name": org["name"]}
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def unlink_organization(
    engine: Engine, production_id: uuid.UUID, organization_id: uuid.UUID, role: str, actor: Actor | None = None
) -> dict[str, Any]:
    """``backlot.production.organization.unlink``."""
    actor = actor or os_actor()
    role = _check_role(role)
    try:
        with uow(engine, actor) as u:
            _require_member(u.conn, production_id, actor, _WRITE_ROLES)
            org = _load(u.conn, organization_id)
            done = u.conn.execute(
                delete(t.production_organization).where(
                    and_(
                        t.production_organization.c.production_id == production_id,
                        t.production_organization.c.organization_id == organization_id,
                        t.production_organization.c.role == role,
                    )
                )
            )
            if done.rowcount != 1:
                raise OrganizationNotLinked(f"{org['name']} as {role}")
            u.audit(
                "production.organization.unlinked", "organization", organization_id, production_id=production_id,
                change={"role": {"old": role, "new": None}, "name": {"old": org["name"], "new": None}},
                command_id="backlot.production.organization.unlink",
            )
            return {"production_id": str(production_id), "organization_id": str(organization_id), "role": role, "name": org["name"]}
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def list_linked(engine: Engine, production_id: uuid.UUID, actor: Actor | None = None) -> list[dict[str, Any]]:
    """``backlot.production.organization.list``: the production's organizations with their roles (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            _require_member(conn, production_id, actor, None)
            rows = conn.execute(
                select(t.production_organization.c.role, t.organization.c.id, t.organization.c.name, t.organization.c.retired_at)
                .join(t.organization, t.organization.c.id == t.production_organization.c.organization_id)
                .where(t.production_organization.c.production_id == production_id)
                .order_by(t.production_organization.c.role, func.lower(t.organization.c.name))
            )
            return [{"organization_id": str(r[1]), "name": r[2], "role": r[0], "retired": r[3] is not None} for r in rows]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc
