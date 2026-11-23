"""Production management (G2-02): create, update, read, list, status change, members.

Every function is one transaction through :func:`cfa4_backlothub.db.uow.uow`: the change and its single audit event
commit together. Input is validated first and fails with an English error code (never an unhandled exception).
"""

from __future__ import annotations

import re
import uuid
from collections.abc import Mapping
from datetime import date, datetime
from typing import Any

from sqlalchemy import Connection, Engine, Uuid, and_, exists, func, insert, literal, select, update
from sqlalchemy.exc import DBAPIError, IntegrityError

from cfa4_audit import uuid7

from ..db import schema as t
from ..db.uow import Actor, Uow, os_actor, uow
from ..errors import (
    ConflictStaleVersion,
    CountryUnknown,
    CurrencyUnknown,
    DatabaseUnavailable,
    EntityInUse,
    FpsInvalid,
    InvalidInput,
    PermissionDenied,
    ProductionCodeTaken,
    ProductionNotFound,
    StatusNotInList,
    StructureAssetHasParent,
    StructureCrossProduction,
    StructureCycle,
    StructureRuleViolation,
    TimingInvalid,
    TypeUnknown,
)

CODE_RE = re.compile(r"^[A-Z0-9][A-Z0-9_-]{1,31}$")
TIMECODE_RE = re.compile(r"^[0-9]{2}:[0-5][0-9]:[0-5][0-9][:;][0-9]{2}$")
_INT_MAX = 2_000_000_000
_WRITE_ROLES = ("owner", "producer", "coordinator")  # who may change the basic data
_STATUS_ROLES = ("owner", "producer")  # who may change the status
REFERENCE_CURRENCIES = ("EUR", "USD")

# The columns a caller may set on create; on update every one except ``code`` (the code is the stable handle).
_FIELDS = (
    "code", "title", "country", "currency", "reference_currency", "fps_num", "fps_den", "width", "height",
    "pixel_aspect_num", "pixel_aspect_den", "start_timecode", "episode_count", "planned_start", "planned_end",
    "description",
)  # fmt: skip
_UPDATABLE = tuple(f for f in _FIELDS if f != "code")


# ---------------------------------------------------------------- validation


def _int(name: str, value: Any, *, low: int, error: type[Exception] = InvalidInput) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not low <= value <= _INT_MAX:
        raise error(f"{name} must be an integer >= {low}")
    return value


def _date(name: str, value: Any) -> date:
    if isinstance(value, datetime):
        raise InvalidInput(f"{name} must be a date")
    if isinstance(value, date):
        return value
    try:
        return date.fromisoformat(str(value))
    except ValueError as exc:
        raise InvalidInput(f"{name} must be a date (YYYY-MM-DD)") from exc


def _text(name: str, value: Any, *, max_len: int, allow_empty: bool = False) -> str:
    if not isinstance(value, str) or (not value.strip() and not allow_empty) or len(value) > max_len:
        raise InvalidInput(f"{name} must be a text of 1..{max_len} characters")
    return value


def _validate_fields(conn: Connection, fields: Mapping[str, Any]) -> dict[str, Any]:
    """Check the values that are given (partial input is fine) and return the normalised ones."""
    out: dict[str, Any] = {}
    for name, value in fields.items():
        if value is None:
            if name in ("width", "height", "start_timecode", "episode_count", "planned_start", "planned_end", "description"):
                out[name] = None
                continue
            raise InvalidInput(f"{name} must not be empty")
        if name == "code":
            if not isinstance(value, str) or not CODE_RE.match(value):
                raise InvalidInput("code must be 2..32 characters of A-Z, 0-9, _ and -, not starting with _ or -")
            out[name] = value
        elif name == "title":
            out[name] = _text(name, value, max_len=200)
        elif name == "description":
            out[name] = _text(name, value, max_len=20000, allow_empty=True)
        elif name == "country":
            code = str(value).upper()
            if not re.fullmatch(r"[A-Z]{2}", code) or conn.execute(select(t.country.c.code).where(t.country.c.code == code)).first() is None:
                raise CountryUnknown(str(value))
            out[name] = code
        elif name in ("currency", "reference_currency"):
            code = str(value).upper()
            row = None
            if re.fullmatch(r"[A-Z]{3}", code):
                row = conn.execute(select(t.currency.c.active).where(t.currency.c.code == code)).first()
            if row is None or not row[0]:
                raise CurrencyUnknown(str(value))
            if name == "reference_currency" and code not in REFERENCE_CURRENCIES:
                raise InvalidInput("reference_currency must be EUR or USD")
            out[name] = code
        elif name in ("fps_num", "fps_den"):
            out[name] = _int(name, value, low=1, error=FpsInvalid)
        elif name in ("width", "height", "pixel_aspect_num", "pixel_aspect_den"):
            out[name] = _int(name, value, low=1)
        elif name == "episode_count":
            out[name] = _int(name, value, low=0)
        elif name == "start_timecode":
            if not isinstance(value, str) or not TIMECODE_RE.match(value):
                raise InvalidInput("start_timecode must look like 01:00:00:00")
            out[name] = value
        elif name in ("planned_start", "planned_end"):
            out[name] = _date(name, value)
        else:
            raise InvalidInput(f"unknown field {name!r}")
    return out


def _check_consistency(values: Mapping[str, Any]) -> None:
    if (values.get("width") is None) != (values.get("height") is None):
        raise InvalidInput("width and height are given together")
    start, end = values.get("planned_start"), values.get("planned_end")
    if start is not None and end is not None and end < start:
        raise InvalidInput("planned_end is before planned_start")


# ---------------------------------------------------------------- helpers


def _jsonable(value: Any) -> Any:
    if isinstance(value, (date, datetime)):
        return value.isoformat()
    if isinstance(value, uuid.UUID):
        return str(value)
    return value


def _view(row: Mapping[Any, Any], type_code: str, status_code: str, category: str) -> dict[str, Any]:
    keys = (*_FIELDS, "settings", "created_at", "updated_at", "created_by", "updated_by", "row_version")
    out: dict[str, Any] = {"id": str(row["id"])}
    out.update({k: _jsonable(row[k]) for k in keys})
    out["type"], out["status"], out["status_category"] = type_code, status_code, category
    return out


def _load(conn: Connection, production_id: uuid.UUID) -> dict[str, Any]:
    row = (
        conn.execute(
            select(t.production, t.production_type.c.code.label("type_code"), t.status.c.code.label("status_code"), t.status.c.category)
            .join(t.production_type, t.production_type.c.id == t.production.c.type_id)
            .join(t.status, t.status.c.id == t.production.c.status_id)
            .where(t.production.c.id == production_id)
        )
        .mappings()
        .first()
    )
    if row is None:
        raise ProductionNotFound(str(production_id))
    return _view(row, row["type_code"], row["status_code"], row["category"])


def _require_member(conn: Connection, production_id: uuid.UUID, actor: Actor, roles: tuple[str, ...] | None) -> str:
    """The actor's role in the production; ``PRODUCTION_NOT_FOUND`` if there is none, ``PERMISSION_DENIED`` if the
    actor is not a member or the role is too weak."""
    if conn.execute(select(t.production.c.id).where(t.production.c.id == production_id)).first() is None:
        raise ProductionNotFound(str(production_id))
    row = conn.execute(
        select(t.production_member.c.role).where(
            and_(
                t.production_member.c.production_id == production_id,
                t.production_member.c.actor_kind == actor.kind,
                t.production_member.c.actor_ref == actor.ref,
            )
        )
    ).first()
    if row is None or (roles is not None and row[0] not in roles):
        raise PermissionDenied("not a member of the production" if row is None else f"role {row[0]!r} may not do this")
    return str(row[0])


def _status_id(conn: Connection, status_list_id: uuid.UUID, code: str) -> uuid.UUID:
    row = conn.execute(
        select(t.status.c.id).where(and_(t.status.c.status_list_id == status_list_id, t.status.c.code == code))
    ).first()
    if row is None:
        raise StatusNotInList(code)
    return uuid.UUID(str(row[0]))


_SQLSTATE_ERRORS = {  # the codes the structure triggers raise (migration 0003); the service checks first for a readable detail
    "BL101": StructureRuleViolation, "BL102": StructureCycle, "BL103": StructureCrossProduction,
    "BL104": StructureAssetHasParent, "BL105": InvalidInput, "BL106": TimingInvalid,
}  # fmt: skip


def _guard_db(error: Exception) -> Exception:
    state = getattr(getattr(error, "orig", None), "sqlstate", None)
    if state in _SQLSTATE_ERRORS:
        return _SQLSTATE_ERRORS[state](str(error.orig).splitlines()[0] if str(error.orig) else "")  # type: ignore[attr-defined]
    if isinstance(error, IntegrityError):
        text = str(error.orig)
        if "production_code_key" in text:
            return ProductionCodeTaken()
        if state == "23503" and "still referenced" in text:
            return EntityInUse("another record still refers to it")
        return InvalidInput("a value violates a database rule")
    if isinstance(error, DBAPIError):
        return DatabaseUnavailable(str(error.orig).splitlines()[0] if str(error.orig) else "")
    return error


# ---------------------------------------------------------------- commands


def create_production(engine: Engine, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.create``: a new production; the creator becomes its owner (audit ``production.created``)."""
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            return _create(u, params)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def _create(u: Uow, params: Mapping[str, Any]) -> dict[str, Any]:
    conn = u.conn
    missing = [f for f in ("code", "title", "type", "country", "currency", "reference_currency", "fps_num", "fps_den") if params.get(f) is None]
    if missing:
        raise InvalidInput(f"missing: {', '.join(missing)}")
    unknown = sorted(set(params) - {*_FIELDS, "type", "status"})
    if unknown:
        raise InvalidInput(f"unknown field(s): {', '.join(unknown)}")
    values = _validate_fields(conn, {k: v for k, v in params.items() if k in _FIELDS})
    values.setdefault("pixel_aspect_num", 1)
    values.setdefault("pixel_aspect_den", 1)
    _check_consistency(values)
    if conn.execute(select(t.production.c.id).where(t.production.c.code == values["code"])).first() is not None:
        raise ProductionCodeTaken(values["code"])
    ptype = conn.execute(
        select(t.production_type.c.id, t.production_type.c.status_list_id, t.production_type.c.retired_at).where(
            t.production_type.c.code == str(params["type"])
        )
    ).first()
    if ptype is None or ptype[2] is not None:
        raise TypeUnknown(str(params["type"]))
    if params.get("status") is not None:
        status_id = _status_id(conn, ptype[1], str(params["status"]))
    else:  # the first status of the list
        first = conn.execute(
            select(t.status.c.id).where(t.status.c.status_list_id == ptype[1]).order_by(t.status.c.ordinal).limit(1)
        ).first()
        if first is None:
            raise StatusNotInList("the status list is empty")
        status_id = first[0]
    pid, now = uuid7(), func.now()
    created_by = u.actor.ref
    conn.execute(
        insert(t.production).values(
            id=pid, type_id=ptype[0], status_id=status_id, settings={}, created_at=now, updated_at=now,
            created_by=created_by, updated_by=created_by, row_version=1, **values,
        )
    )
    conn.execute(
        insert(t.production_member).values(
            production_id=pid, actor_kind=u.actor.kind, actor_ref=u.actor.ref, role="owner", created_at=now, created_by=created_by
        )
    )
    conn.execute(  # the production starts with the structure rules of its type's template (G2-03 section 4)
        insert(t.production_structure_rule).from_select(
            ["production_id", "parent_type_id", "child_type_id"],
            select(literal(pid, Uuid), t.structure_template_rule.c.parent_type_id, t.structure_template_rule.c.child_type_id)
            .select_from(
                t.structure_template.join(
                    t.structure_template_rule, t.structure_template_rule.c.template_id == t.structure_template.c.id
                )
            )
            .where(t.structure_template.c.production_type_id == ptype[0]),
        )
    )
    view = _load(conn, pid)
    change = {k: {"old": None, "new": view[k]} for k in (*_FIELDS, "type", "status") if view.get(k) is not None}
    u.audit("production.created", "production", pid, production_id=pid, change=change, command_id="backlot.production.create")
    return view


def update_production(
    engine: Engine, production_id: uuid.UUID, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None
) -> dict[str, Any]:
    """``backlot.production.update``: change basic data; ``row_version`` must be the one that was read (audit
    ``production.updated`` with old and new value of each changed field)."""
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            return _update(u, production_id, row_version, changes)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def _update(u: Uow, production_id: uuid.UUID, row_version: int, changes: Mapping[str, Any]) -> dict[str, Any]:
    conn = u.conn
    _int("row_version", row_version, low=1)
    bad = sorted(set(changes) - set(_UPDATABLE))
    if bad:
        raise InvalidInput(f"cannot change: {', '.join(bad)}")
    if not changes:
        raise InvalidInput("nothing to change")
    _require_member(conn, production_id, u.actor, _WRITE_ROLES)
    values = _validate_fields(conn, changes)
    before = _load(conn, production_id)
    if before["row_version"] != row_version:
        raise ConflictStaleVersion(f"current version is {before['row_version']}")
    merged = {k: (values[k] if k in values else _from_view(before, k)) for k in _UPDATABLE}
    _check_consistency(merged)
    real = {k: v for k, v in values.items() if _jsonable(v) != before[k]}
    if not real:
        return before  # nothing differs: no write, no audit event
    result = conn.execute(
        update(t.production)
        .where(and_(t.production.c.id == production_id, t.production.c.row_version == row_version))
        .values(**real, updated_at=func.now(), updated_by=u.actor.ref, row_version=row_version + 1)
    )
    if result.rowcount != 1:  # a concurrent change between the read and the write
        raise ConflictStaleVersion("changed by someone else")
    after = _load(conn, production_id)
    u.audit(
        "production.updated", "production", production_id, production_id=production_id,
        change={k: {"old": before[k], "new": after[k]} for k in real}, command_id="backlot.production.update",
    )
    return after


def _from_view(view: Mapping[str, Any], key: str) -> Any:
    value = view[key]
    return date.fromisoformat(value) if key in ("planned_start", "planned_end") and value is not None else value


def set_status(
    engine: Engine, production_id: uuid.UUID, status: str, row_version: int | None = None, actor: Actor | None = None
) -> dict[str, Any]:
    """``backlot.production.status.set``: any status of the type's list (M1; the Workflow Engine checks transitions
    from the 4th step on). Audit ``production.status.changed``."""
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            conn = u.conn
            _require_member(conn, production_id, u.actor, _STATUS_ROLES)
            before = _load(conn, production_id)
            if row_version is not None and before["row_version"] != _int("row_version", row_version, low=1):
                raise ConflictStaleVersion(f"current version is {before['row_version']}")
            list_id = conn.execute(
                select(t.production_type.c.status_list_id).where(t.production_type.c.id == select(t.production.c.type_id).where(t.production.c.id == production_id).scalar_subquery())
            ).scalar_one()
            new_id = _status_id(conn, list_id, str(status))
            if before["status"] == status:
                return before
            result = conn.execute(
                update(t.production)
                .where(and_(t.production.c.id == production_id, t.production.c.row_version == before["row_version"]))
                .values(status_id=new_id, updated_at=func.now(), updated_by=u.actor.ref, row_version=before["row_version"] + 1)
            )
            if result.rowcount != 1:
                raise ConflictStaleVersion("changed by someone else")
            after = _load(conn, production_id)
            u.audit(
                "production.status.changed", "production", production_id, production_id=production_id,
                change={"status": {"old": before["status"], "new": after["status"]}}, command_id="backlot.production.status.set",
            )
            return after
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def get_production(engine: Engine, production_id: uuid.UUID, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.get``: one production (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            _require_member(conn, production_id, actor, None)
            return _load(conn, production_id)
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def list_productions(
    engine: Engine,
    *,
    type: str | None = None,
    status: str | None = None,
    active: bool | None = None,
    limit: int = 100,
    offset: int = 0,
    actor: Actor | None = None,
) -> list[dict[str, Any]]:
    """``backlot.production.list``: the productions the actor is a member of, newest first. ``active`` selects
    the planned/active/on-hold ones (True) or the closed/cancelled ones (False)."""
    actor = actor or os_actor()
    _int("limit", limit, low=1)
    _int("offset", offset, low=0)
    if limit > 500:
        raise InvalidInput("limit must be at most 500")
    member = exists().where(
        and_(
            t.production_member.c.production_id == t.production.c.id,
            t.production_member.c.actor_kind == actor.kind,
            t.production_member.c.actor_ref == actor.ref,
        )
    )
    query = (
        select(t.production, t.production_type.c.code.label("type_code"), t.status.c.code.label("status_code"), t.status.c.category)
        .join(t.production_type, t.production_type.c.id == t.production.c.type_id)
        .join(t.status, t.status.c.id == t.production.c.status_id)
        .where(member)
        .order_by(t.production.c.created_at.desc(), t.production.c.id.desc())
        .limit(limit)
        .offset(offset)
    )
    if type is not None:
        query = query.where(t.production_type.c.code == type)
    if status is not None:
        query = query.where(t.status.c.code == status)
    if active is not None:
        closed = t.status.c.category.in_(("closed", "cancelled"))
        query = query.where(~closed if active else closed)
    try:
        with engine.connect() as conn:
            return [_view(r, r["type_code"], r["status_code"], r["category"]) for r in conn.execute(query).mappings()]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def list_members(engine: Engine, production_id: uuid.UUID, actor: Actor | None = None) -> list[dict[str, Any]]:
    """``backlot.production.member.list``: members and roles (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            _require_member(conn, production_id, actor, None)
            rows = conn.execute(
                select(t.production_member).where(t.production_member.c.production_id == production_id).order_by(t.production_member.c.actor_ref)
            ).mappings()
            return [
                {"actor_kind": r["actor_kind"], "actor_ref": r["actor_ref"], "role": r["role"], "created_at": r["created_at"].isoformat()}
                for r in rows
            ]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc
