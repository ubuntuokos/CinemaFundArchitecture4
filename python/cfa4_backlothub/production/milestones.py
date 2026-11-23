"""Production milestones (G2-02): add, update, remove, list. Audit ``production.milestone.added/updated/removed``.

A milestone that is marked done is a fact and cannot be removed (``MILESTONE_DONE``); remove it from done first or leave it.
"""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from typing import Any

from sqlalchemy import Connection, Engine, and_, delete, func, insert, select, update
from sqlalchemy.exc import DBAPIError, IntegrityError

from cfa4_audit import uuid7

from ..db import schema as t
from ..db.uow import Actor, os_actor, uow
from ..errors import ConflictStaleVersion, InvalidInput, MilestoneDone, MilestoneNotFound
from .service import _WRITE_ROLES, _date, _guard_db, _int, _jsonable, _require_member, _text

_UPDATABLE = ("name", "due_date", "done", "ordinal")


def _view(row: Mapping[Any, Any]) -> dict[str, Any]:
    out = {k: _jsonable(row[k]) for k in ("name", "due_date", "done_at", "ordinal", "created_at", "updated_at", "created_by", "updated_by", "row_version")}
    out["id"], out["production_id"], out["done"] = str(row["id"]), str(row["production_id"]), row["done_at"] is not None
    return out


def _load(conn: Connection, milestone_id: uuid.UUID) -> dict[str, Any]:
    row = conn.execute(select(t.milestone).where(t.milestone.c.id == milestone_id)).mappings().first()
    if row is None:
        raise MilestoneNotFound(str(milestone_id))
    return _view(row)


def add_milestone(engine: Engine, production_id: uuid.UUID, name: str, due_date: Any, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.milestone.add``: a milestone at the end of the production's list."""
    actor = actor or os_actor()
    name, due = _text("name", name, max_len=200), _date("due_date", due_date)
    try:
        with uow(engine, actor) as u:
            _require_member(u.conn, production_id, actor, _WRITE_ROLES)
            ordinal = u.conn.execute(
                select(func.coalesce(func.max(t.milestone.c.ordinal), 0) + 1).where(t.milestone.c.production_id == production_id)
            ).scalar_one()
            if ordinal > 32000:
                raise InvalidInput("too many milestones")
            mid, now = uuid7(), func.now()
            u.conn.execute(
                insert(t.milestone).values(
                    id=mid, production_id=production_id, name=name, due_date=due, ordinal=ordinal, created_at=now, updated_at=now,
                    created_by=actor.ref, updated_by=actor.ref, row_version=1,
                )
            )
            view = _load(u.conn, mid)
            u.audit(
                "production.milestone.added", "milestone", mid, production_id=production_id,
                change={"name": {"old": None, "new": view["name"]}, "due_date": {"old": None, "new": view["due_date"]}},
                command_id="backlot.production.milestone.add",
            )
            return view
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def update_milestone(engine: Engine, milestone_id: uuid.UUID, row_version: int, changes: Mapping[str, Any], actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.milestone.update``: name, due date, done (true/false) or position."""
    actor = actor or os_actor()
    _int("row_version", row_version, low=1)
    bad = sorted(set(changes) - set(_UPDATABLE))
    if bad or not changes:
        raise InvalidInput(f"cannot change: {', '.join(bad)}" if bad else "nothing to change")
    values: dict[str, Any] = {}
    if "name" in changes:
        values["name"] = _text("name", changes["name"], max_len=200)
    if "due_date" in changes:
        values["due_date"] = _date("due_date", changes["due_date"])
    if "ordinal" in changes:
        values["ordinal"] = _int("ordinal", changes["ordinal"], low=1)
    if "done" in changes and not isinstance(changes["done"], bool):
        raise InvalidInput("done must be true or false")
    try:
        with uow(engine, actor) as u:
            before = _load(u.conn, milestone_id)
            _require_member(u.conn, uuid.UUID(before["production_id"]), actor, _WRITE_ROLES)
            if before["row_version"] != row_version:
                raise ConflictStaleVersion(f"current version is {before['row_version']}")
            if "done" in changes and changes["done"] != before["done"]:
                values["done_at"] = func.now() if changes["done"] else None
            real = {k: v for k, v in values.items() if k == "done_at" or _jsonable(v) != before[k]}
            if not real:
                return before
            done = u.conn.execute(
                update(t.milestone)
                .where(and_(t.milestone.c.id == milestone_id, t.milestone.c.row_version == row_version))
                .values(**real, updated_at=func.now(), updated_by=actor.ref, row_version=row_version + 1)
            )
            if done.rowcount != 1:
                raise ConflictStaleVersion("changed by someone else")
            after = _load(u.conn, milestone_id)
            keys = [("done" if k == "done_at" else k) for k in real]
            u.audit(
                "production.milestone.updated", "milestone", milestone_id, production_id=uuid.UUID(before["production_id"]),
                change={k: {"old": before[k], "new": after[k]} for k in keys}, command_id="backlot.production.milestone.update",
            )
            return after
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def remove_milestone(engine: Engine, milestone_id: uuid.UUID, row_version: int | None = None, actor: Actor | None = None) -> dict[str, Any]:
    """``backlot.production.milestone.remove``: delete a milestone that is not done (audited with its last values)."""
    actor = actor or os_actor()
    try:
        with uow(engine, actor) as u:
            before = _load(u.conn, milestone_id)
            pid = uuid.UUID(before["production_id"])
            _require_member(u.conn, pid, actor, _WRITE_ROLES)
            if row_version is not None and before["row_version"] != _int("row_version", row_version, low=1):
                raise ConflictStaleVersion(f"current version is {before['row_version']}")
            if before["done"]:
                raise MilestoneDone(before["name"])
            u.conn.execute(delete(t.milestone).where(t.milestone.c.id == milestone_id))
            u.audit(
                "production.milestone.removed", "milestone", milestone_id, production_id=pid,
                change={"name": {"old": before["name"], "new": None}, "due_date": {"old": before["due_date"], "new": None}},
                command_id="backlot.production.milestone.remove",
            )
            return before
    except (IntegrityError, DBAPIError) as exc:
        raise _guard_db(exc) from exc


def list_milestones(engine: Engine, production_id: uuid.UUID, actor: Actor | None = None) -> list[dict[str, Any]]:
    """``backlot.production.milestone.list``: by due date, then position (members only)."""
    actor = actor or os_actor()
    try:
        with engine.connect() as conn:
            _require_member(conn, production_id, actor, None)
            rows = conn.execute(
                select(t.milestone).where(t.milestone.c.production_id == production_id).order_by(t.milestone.c.due_date, t.milestone.c.ordinal)
            ).mappings()
            return [_view(r) for r in rows]
    except DBAPIError as exc:
        raise _guard_db(exc) from exc
