"""Production types and status lists (G2-02 section 3: ``backlot.production.type.*``, ``backlot.status_list.*``).

Configuration is shared by all productions. M1 has one user (#42), so any OS user of the machine may change it; from M2 on a
role check replaces that. Every change is one ``config.*`` audit event without a production scope. A type's own
label is stored as ``label_key``: a translation key, or ``text:<label>`` for a type the user made up (shown literally).
"""

from __future__ import annotations

import re
import uuid
from collections.abc import Mapping
from typing import Any

from sqlalchemy import Connection, Engine, and_, exists, func, insert, select, update
from sqlalchemy.exc import DBAPIError, IntegrityError

from cfa4_audit import uuid7

from ..db import schema as t
from ..db.uow import Actor, os_actor, uow
from ..errors import (
    ConfigCodeTaken,
    ConflictStaleVersion,
    InvalidInput,
    StatusListNotFound,
    StatusNotFound,
    TypeInUse,
    TypeUnknown,
)
from .service import _guard_db, _int, _jsonable, _text

_CODE = re.compile(r"^[a-z][a-z0-9_]{1,31}$")
CATEGORIES = ("planned", "active", "on_hold", "closed", "cancelled")


def _code(value: Any) -> str:
    if not isinstance(value, str) or not _CODE.match(value):
        raise InvalidInput("code must be 2..32 characters of a-z, 0-9 and _, starting with a letter")
    return value


def _label(value: Any) -> str:
    return "text:" + _text("label", value, max_len=100)


def _stale(done: Any) -> None:
    if done.rowcount != 1:
        raise ConflictStaleVersion("changed by someone else")


# ---------------------------------------------------------------- status lists and statuses


def _list_view(row: Mapping[Any, Any]) -> dict[str, Any]:
    return {"id": str(row["id"]), "code": row["code"], "target": row["target"], "row_version": row["row_version"]}


def _status_view(row: Mapping[Any, Any]) -> dict[str, Any]:
    return {
        "id": str(row["id"]), "status_list_id": str(row["status_list_id"]), "code": row["code"], "label_key": row["label_key"],
        "ordinal": row["ordinal"], "category": row["category"], "row_version": row["row_version"],
    }  # fmt: skip


def _find_list(conn: Connection, code: Any) -> Mapping[Any, Any]:
    row = conn.execute(select(t.status_list).where(t.status_list.c.code == str(code))).mappings().first()
    if row is None:
        raise StatusListNotFound(str(code))
    return row


def create_status_list(engine: Engine, code: str, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.status_list.create``: an empty list for productions (statuses are added with ``status.add``)."""
    actor = actor or os_actor()
    code = _code(code)
    try:
        with uow(engine, actor) as u:
            if u.conn.execute(select(exists().where(t.status_list.c.code == code))).scalar_one():
                raise ConfigCodeTaken(code)
            lid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.status_list).values(
                    id=lid, code=code, target="production", created_at=now, updated_at=now, created_by=actor.ref, updated_by=actor.ref, row_version=1
                )
            )
            u.audit("config.status_list.created", "status_list", lid, change={"code": {"old": None, "new": code}}, command_id="backlot.status_list.create")
            return _list_view(_find_list(u.conn, code))
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def list_status_lists(engine: Engine) -> list[dict[str, Any]]:
    """``backlot.status_list.list``: every list with its statuses in order."""
    try:
        with engine.connect() as conn:
            lists = conn.execute(select(t.status_list).order_by(t.status_list.c.code)).mappings().all()
            out = []
            for lst in lists:
                statuses = conn.execute(
                    select(t.status).where(t.status.c.status_list_id == lst["id"]).order_by(t.status.c.ordinal, t.status.c.code)
                ).mappings()
                out.append({**_list_view(lst), "statuses": [_status_view(s) for s in statuses]})
            return out
    except DBAPIError as exc:
        raise _guard_db(exc) from exc


def add_status(engine: Engine, status_list: str, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.status.add``: a status at the end of a list (``category`` says what it means for filters)."""
    actor = actor or os_actor()
    code, category = _code(params.get("code")), params.get("category")
    if category not in CATEGORIES:
        raise InvalidInput(f"category must be one of: {', '.join(CATEGORIES)}")
    label = _label(params.get("label"))
    try:
        with uow(engine, actor) as u:
            lst = _find_list(u.conn, status_list)
            if u.conn.execute(select(exists().where(and_(t.status.c.status_list_id == lst["id"], t.status.c.code == code)))).scalar_one():
                raise ConfigCodeTaken(code)
            ordinal = u.conn.execute(
                select(func.coalesce(func.max(t.status.c.ordinal), 0) + 1).where(t.status.c.status_list_id == lst["id"])
            ).scalar_one()
            sid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.status).values(
                    id=sid, status_list_id=lst["id"], code=code, label_key=label, ordinal=ordinal, category=category,
                    created_at=now, updated_at=now, created_by=actor.ref, updated_by=actor.ref, row_version=1,
                )
            )
            u.audit(
                "config.status.added", "status", sid,
                change={"status_list": {"old": None, "new": lst["code"]}, "code": {"old": None, "new": code}, "category": {"old": None, "new": category}},
                command_id="backlot.status.add",
            )
            return _status_view(u.conn.execute(select(t.status).where(t.status.c.id == sid)).mappings().one())
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def update_status(engine: Engine, status_id: uuid.UUID, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.status.update``: label, category or position of a status (the code is its stable handle)."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    bad = sorted(set(changes) - {"label", "category", "ordinal"})
    if bad or not changes:
        raise InvalidInput(f"cannot change: {', '.join(bad)}" if bad else "nothing to change")
    values: dict[str, Any] = {}
    if "label" in changes:
        values["label_key"] = _label(changes["label"])
    if "category" in changes:
        if changes["category"] not in CATEGORIES:
            raise InvalidInput(f"category must be one of: {', '.join(CATEGORIES)}")
        values["category"] = changes["category"]
    if "ordinal" in changes:
        values["ordinal"] = _int("ordinal", changes["ordinal"], low=1)
    try:
        with uow(engine, actor) as u:
            row = u.conn.execute(select(t.status).where(t.status.c.id == status_id)).mappings().first()
            if row is None:
                raise StatusNotFound(str(status_id))
            if row["row_version"] != row_version:
                raise ConflictStaleVersion(f"current version is {row['row_version']}")
            real = {k: v for k, v in values.items() if _jsonable(v) != _jsonable(row[k])}
            if not real:
                return _status_view(row)
            _stale(
                u.conn.execute(
                    update(t.status)
                    .where(and_(t.status.c.id == status_id, t.status.c.row_version == row_version))
                    .values(**real, updated_at=func.now(), updated_by=actor.ref, row_version=row_version + 1)
                )
            )
            u.audit(
                "config.status.updated", "status", status_id,
                change={k: {"old": row[k], "new": v} for k, v in real.items()}, command_id="backlot.status.update",
            )
            return _status_view(u.conn.execute(select(t.status).where(t.status.c.id == status_id)).mappings().one())
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


# ---------------------------------------------------------------- production types


def _type_view(row: Mapping[Any, Any], status_list_code: str) -> dict[str, Any]:
    return {
        "id": str(row["id"]), "code": row["code"], "label_key": row["label_key"], "status_list": status_list_code,
        "retired": row["retired_at"] is not None, "row_version": row["row_version"],
    }  # fmt: skip


def _find_type(conn: Connection, code: Any) -> tuple[Mapping[Any, Any], str]:
    row = (
        conn.execute(
            select(t.production_type, t.status_list.c.code.label("list_code"))
            .join(t.status_list, t.status_list.c.id == t.production_type.c.status_list_id)
            .where(t.production_type.c.code == str(code))
        )
        .mappings()
        .first()
    )
    if row is None:
        raise TypeUnknown(str(code))
    return row, str(row["list_code"])


def create_type(engine: Engine, params: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.type.create``: a production type with the status list it uses."""
    actor = actor or os_actor()
    code, label = _code(params.get("code")), _label(params.get("label"))
    try:
        with uow(engine, actor) as u:
            lst = _find_list(u.conn, params.get("status_list"))
            if u.conn.execute(select(exists().where(t.production_type.c.code == code))).scalar_one():
                raise ConfigCodeTaken(code)
            if not u.conn.execute(select(exists().where(t.status.c.status_list_id == lst["id"]))).scalar_one():
                raise InvalidInput("the status list has no statuses yet")
            tid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.production_type).values(
                    id=tid, code=code, label_key=label, status_list_id=lst["id"], created_at=now, updated_at=now,
                    created_by=actor.ref, updated_by=actor.ref, row_version=1,
                )
            )
            u.audit(
                "config.production_type.created", "production_type", tid,
                change={"code": {"old": None, "new": code}, "status_list": {"old": None, "new": lst["code"]}}, command_id="backlot.production.type.create",
            )
            row, list_code = _find_type(u.conn, code)
            return _type_view(row, list_code)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def update_type(engine: Engine, code: str, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.type.update``: the label, the status list (only while no production has the type), or
    ``retired`` (true: no new productions of this type; existing ones are not touched)."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    bad = sorted(set(changes) - {"label", "status_list", "retired"})
    if bad or not changes:
        raise InvalidInput(f"cannot change: {', '.join(bad)}" if bad else "nothing to change")
    if "retired" in changes and not isinstance(changes["retired"], bool):
        raise InvalidInput("retired must be true or false")
    try:
        with uow(engine, actor) as u:
            row, list_code = _find_type(u.conn, code)
            if row["row_version"] != row_version:
                raise ConflictStaleVersion(f"current version is {row['row_version']}")
            real: dict[str, Any] = {}
            audit: dict[str, Any] = {}
            if "label" in changes and _label(changes["label"]) != row["label_key"]:
                real["label_key"] = _label(changes["label"])
                audit["label"] = {"old": row["label_key"], "new": real["label_key"]}
            if "status_list" in changes and changes["status_list"] != list_code:
                lst = _find_list(u.conn, changes["status_list"])
                if u.conn.execute(select(exists().where(t.production.c.type_id == row["id"]))).scalar_one():
                    raise TypeInUse("productions of this type exist; their statuses belong to the current list")
                if not u.conn.execute(select(exists().where(t.status.c.status_list_id == lst["id"]))).scalar_one():
                    raise InvalidInput("the status list has no statuses yet")
                real["status_list_id"] = lst["id"]
                audit["status_list"] = {"old": list_code, "new": lst["code"]}
            if "retired" in changes and changes["retired"] != (row["retired_at"] is not None):
                real["retired_at"] = func.now() if changes["retired"] else None
                audit["retired"] = {"old": row["retired_at"] is not None, "new": changes["retired"]}
            if not real:
                return _type_view(row, list_code)
            _stale(
                u.conn.execute(
                    update(t.production_type)
                    .where(and_(t.production_type.c.id == row["id"], t.production_type.c.row_version == row_version))
                    .values(**real, updated_at=func.now(), updated_by=actor.ref, row_version=row_version + 1)
                )
            )
            u.audit("config.production_type.updated", "production_type", row["id"], change=audit, command_id="backlot.production.type.update")
            fresh, fresh_list = _find_type(u.conn, code)
            return _type_view(fresh, fresh_list)
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def list_types(engine: Engine, *, include_retired: bool = False) -> list[dict[str, Any]]:
    """``backlot.production.type.list``."""
    stmt = (
        select(t.production_type, t.status_list.c.code.label("list_code"))
        .join(t.status_list, t.status_list.c.id == t.production_type.c.status_list_id)
        .order_by(t.production_type.c.code)
    )
    if not include_retired:
        stmt = stmt.where(t.production_type.c.retired_at.is_(None))
    try:
        with engine.connect() as conn:
            return [_type_view(r, str(r["list_code"])) for r in conn.execute(stmt).mappings()]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc
