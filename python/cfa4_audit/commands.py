"""The audit commands (G2-01 section 10) as descriptors plus one dispatcher.

Same descriptor shape as the CFA4 command registry (id, label, menu path, shortcut, params, enabled, run), so a
BacklotHub service registry (BacklotHub #51, finalised at step 2) can adopt them unchanged. ``run_command`` never
raises: every audit error becomes ``{"ok": false, "code": ..., "detail": ...}`` with the stable English code (the
UI translates it, BacklotHub #16).
"""

from __future__ import annotations

import json
import uuid
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import date, datetime
from pathlib import Path
from typing import Any

import psycopg

from .archive import archive_month, verify_archive
from .errors import AuditError, InvalidEvent
from .store import (
    Filters,
    VerifyResult,
    ensure_partitions,
    erase_personal,
    export_events,
    list_events,
    load_checkpoint,
    take_checkpoint,
    verify,
)

Result = dict[str, Any]


@dataclass(frozen=True)
class Param:
    name: str
    kind: str  # "int" | "str" | "bool" | "uuid" | "datetime" | "json"
    required: bool = False
    help: str = ""


@dataclass(frozen=True)
class Command:
    id: str
    label: str
    menu_path: tuple[str, ...]
    params: tuple[Param, ...]
    run: Callable[[Any, Mapping[str, Any]], Result]  # the connection is None for an offline command
    shortcut: str | None = None
    offline: bool = False  # needs no database (verifies a file)

    def enabled(self, conn: psycopg.Connection) -> bool:
        """Commands need only a connection; the database enforces the roles."""
        return self.offline or not conn.closed


_FILTER_PARAMS = (
    Param("actor_kind", "str"),
    Param("actor_ref", "str"),
    Param("subject_type", "str"),
    Param("subject_id", "uuid"),
    Param("action", "str", help="exact, or a prefix ending in .* (production.*)"),
    Param("occurred_from", "datetime"),
    Param("occurred_to", "datetime", help="exclusive"),
    Param("scope_contains", "json", help="JSON object the scope must contain"),
)


def _coerce(param: Param, raw: Any) -> Any:
    try:
        if param.kind == "int":
            return int(raw)
        if param.kind == "bool":
            if isinstance(raw, str):
                if raw.lower() not in ("true", "false", "1", "0"):
                    raise ValueError(raw)
                return raw.lower() in ("true", "1")
            return bool(raw)
        if param.kind == "uuid":
            return uuid.UUID(str(raw))
        if param.kind == "datetime":
            moment = raw if isinstance(raw, datetime) else datetime.fromisoformat(str(raw))
            if moment.tzinfo is None:
                raise ValueError("no time zone")
            return moment
        if param.kind == "json":
            value = json.loads(raw) if isinstance(raw, str) else raw
            if not isinstance(value, dict):
                raise ValueError("not an object")
            return value
        return str(raw)
    except (ValueError, TypeError) as exc:
        raise InvalidEvent(f"parameter {param.name!r}: {exc}") from exc


def _filters(p: Mapping[str, Any]) -> Filters:
    return Filters(
        actor_kind=p.get("actor_kind"),
        actor_ref=p.get("actor_ref"),
        subject_type=p.get("subject_type"),
        subject_id=p.get("subject_id"),
        action=p.get("action"),
        occurred_from=p.get("occurred_from"),
        occurred_to=p.get("occurred_to"),
        scope_contains=p.get("scope_contains") or {},
    )


def _verify_result(r: VerifyResult) -> Result:
    return {
        "ok": r.ok,
        "checked": r.checked,
        "head_seq": r.head_seq,
        "erased_personal": r.erased_personal,
        "code": r.code,
        "seq": r.seq,
        "detail": r.detail,
    }


def _list(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    events = list_events(conn, _filters(p), after_seq=p.get("after_seq", 0), limit=p.get("limit", 100))
    return {"ok": True, "events": [e.as_export() for e in events], "last_seq": events[-1].seq if events else None}


def _verify(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    checkpoint = None
    if p.get("from_checkpoint"):
        checkpoint = load_checkpoint(conn, path=Path(p["checkpoint_path"]) if p.get("checkpoint_path") else None)
    return _verify_result(verify(conn, from_seq=p.get("from_seq", 1), to_seq=p.get("to_seq"), checkpoint=checkpoint))


def _export(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    count = export_events(conn, Path(p["path"]), _filters(p), fmt=p.get("format", "json"))
    return {"ok": True, "path": str(p["path"]), "events": count}


def _checkpoint(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    root = Path(p["root"]) if p.get("root") else None
    return {"ok": True, "path": str(take_checkpoint(conn, root=root))}


def _archive(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    try:
        month = date.fromisoformat(str(p["month"]) + "-01")
    except ValueError as exc:
        raise InvalidEvent("parameter 'month': use YYYY-MM") from exc
    from .store import default_checkpoint_root  # same state tree as the checkpoints

    root = Path(p["root"]) if p.get("root") else default_checkpoint_root().parent / "archives"
    return {"ok": True, "path": str(archive_month(conn, month, root))}


def _archive_verify(_conn: object, p: Mapping[str, Any]) -> Result:
    r = verify_archive(Path(p["path"]))
    return {"ok": r.ok, "count": r.count, "first_seq": r.first_seq, "last_seq": r.last_seq, "code": r.code, "detail": r.detail}


def _ensure(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    return {"ok": True, "created": ensure_partitions(conn, p.get("months_ahead", 2))}


def _erase(conn: psycopg.Connection, p: Mapping[str, Any]) -> Result:
    return {"ok": True, "erased": erase_personal(conn, p["subject_type"], p["subject_id"])}


COMMANDS: dict[str, Command] = {
    c.id: c
    for c in (
        Command(
            "audit.list",
            "List audit events",
            ("Audit", "List"),
            (*_FILTER_PARAMS, Param("after_seq", "int"), Param("limit", "int")),
            _list,
        ),
        Command(
            "audit.verify",
            "Verify the audit chain",
            ("Audit", "Verify"),
            (
                Param("from_seq", "int"),
                Param("to_seq", "int"),
                Param("from_checkpoint", "bool", help="also check against the newest checkpoint file"),
                Param("checkpoint_path", "str"),
            ),
            _verify,
        ),
        Command(
            "audit.export",
            "Export audit events",
            ("Audit", "Export"),
            (Param("path", "str", required=True), Param("format", "str", help="json (default) or csv"), *_FILTER_PARAMS),
            _export,
        ),
        Command("audit.checkpoint", "Write an audit checkpoint", ("Audit", "Checkpoint"), (Param("root", "str"),), _checkpoint),
        Command(
            "audit.archive",
            "Archive a closed month",
            ("Audit", "Archive", "Create"),
            (Param("month", "str", required=True, help="YYYY-MM, a closed month"), Param("root", "str")),
            _archive,
        ),
        Command(
            "audit.archive.verify",
            "Verify an archive package",
            ("Audit", "Archive", "Verify"),
            (Param("path", "str", required=True),),
            _archive_verify,
            offline=True,
        ),
        Command(
            "audit.partition.ensure",
            "Create the audit partitions",
            ("Audit", "Maintenance", "Partitions"),
            (Param("months_ahead", "int"),),
            _ensure,
        ),
        Command(
            "audit.personal.erase",
            "Erase a subject's personal audit data",
            ("Audit", "Personal data", "Erase"),
            (Param("subject_type", "str", required=True), Param("subject_id", "uuid", required=True)),
            _erase,
        ),
    )
}


def run_command(conn: psycopg.Connection | None, command_id: str, params: Mapping[str, Any] | None = None) -> Result:
    """Run one command by id. Never raises an audit error: it returns ``{"ok": false, "code": …}`` instead."""
    command = COMMANDS.get(command_id)
    if command is None:
        return {"ok": False, "code": "AUDIT_UNKNOWN_COMMAND", "detail": command_id}
    try:
        known = {p.name: p for p in command.params}
        given = dict(params or {})
        unknown = sorted(set(given) - set(known))
        if unknown:
            raise InvalidEvent(f"unknown parameter(s): {', '.join(unknown)}")
        missing = [p.name for p in command.params if p.required and given.get(p.name) is None]
        if missing:
            raise InvalidEvent(f"missing parameter(s): {', '.join(missing)}")
        coerced = {name: _coerce(known[name], value) for name, value in given.items() if value is not None}
        if conn is None and not command.offline:
            raise InvalidEvent("this command needs a database connection")
        return command.run(conn, coerced)
    except AuditError as exc:
        return {"ok": False, "code": exc.code, "detail": exc.detail}
    except psycopg.Error as exc:
        return {"ok": False, "code": "DB_UNAVAILABLE", "detail": str(exc).splitlines()[0] if str(exc) else ""}
