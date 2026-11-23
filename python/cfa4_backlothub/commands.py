"""The BacklotHub command registry (G2-02 section 3, #51).

Same descriptor fields as ``crates/engine`` commands (``id``, ``label`` as translation key, ``menu_path``, ``shortcut``,
``params`` as JSON Schema, ``enabled``, ``run``), so the Qt6 shell, the CLI and MCP treat them like engine commands.
Ids live in the ``backlot.`` namespace. :func:`run_command` never raises a BacklotHub or audit error: it returns
``{"ok": false, "code": ..., "detail": ...}`` with the stable English code (the UI translates it, #16).
"""

from __future__ import annotations

import uuid
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from typing import Any

from sqlalchemy import Engine

from cfa4_audit import AuditError

from .db.uow import Actor
from .errors import BacklotError, DatabaseUnavailable, InvalidInput
from .production import config as cfg
from .production import milestones as ms
from .production import organizations as orgs
from .production import service as prod

Result = dict[str, Any]


@dataclass(frozen=True)
class Param:
    name: str
    kind: str  # "str" | "int" | "bool" | "uuid" | "date"
    required: bool = False
    help: str = ""


_JSON_TYPES = {"str": {"type": "string"}, "int": {"type": "integer"}, "bool": {"type": "boolean"}, "uuid": {"type": "string", "format": "uuid"}, "date": {"type": "string", "format": "date"}}


@dataclass(frozen=True)
class Command:
    id: str
    label: str  # translation key
    menu_path: tuple[str, ...]
    params: tuple[Param, ...]
    run: Callable[[Engine, Actor | None, Mapping[str, Any]], Any]
    shortcut: str | None = None
    modifies: bool = True

    def enabled(self, engine: Engine) -> bool:
        return engine is not None

    def schema(self) -> dict[str, Any]:
        """The parameters as a JSON Schema (what ``commands.list`` publishes)."""
        return {
            "type": "object",
            "properties": {p.name: {**_JSON_TYPES[p.kind], **({"description": p.help} if p.help else {})} for p in self.params},
            "required": [p.name for p in self.params if p.required],
            "additionalProperties": True,
        }

    def describe(self) -> dict[str, Any]:
        return {
            "id": self.id, "label": self.label, "menu_path": list(self.menu_path), "shortcut": self.shortcut,
            "params": self.schema(), "modifies": self.modifies,
        }  # fmt: skip


_PRODUCTION_FIELDS = (
    Param("title", "str"), Param("country", "str", help="ISO 3166-1 alpha-2"), Param("currency", "str", help="ISO 4217"),
    Param("reference_currency", "str", help="EUR or USD"), Param("fps_num", "int"), Param("fps_den", "int"),
    Param("width", "int"), Param("height", "int"), Param("pixel_aspect_num", "int"), Param("pixel_aspect_den", "int"),
    Param("start_timecode", "str", help="01:00:00:00"), Param("episode_count", "int"),
    Param("planned_start", "date"), Param("planned_end", "date"), Param("description", "str"),
)  # fmt: skip


def _create(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return prod.create_production(engine, p, actor)


def _update(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    changes = {k: v for k, v in p.items() if k not in ("id", "row_version")}
    return prod.update_production(engine, p["id"], p["row_version"], changes, actor)


def _get(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return prod.get_production(engine, p["id"], actor)


def _list(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return prod.list_productions(
        engine, type=p.get("type"), status=p.get("status"), active=p.get("active"), limit=p.get("limit", 100),
        offset=p.get("offset", 0), actor=actor,
    )  # fmt: skip


def _status_set(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return prod.set_status(engine, p["id"], p["status"], p.get("row_version"), actor)


def _members(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return prod.list_members(engine, p["id"], actor)


def _ms_add(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return ms.add_milestone(engine, p["production_id"], p["name"], p["due_date"], actor)


def _ms_update(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return ms.update_milestone(engine, p["id"], p["row_version"], {k: v for k, v in p.items() if k not in ("id", "row_version")}, actor)


def _ms_remove(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return ms.remove_milestone(engine, p["id"], p.get("row_version"), actor)


def _ms_list(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return ms.list_milestones(engine, p["production_id"], actor)


def _org_create(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return orgs.create_organization(engine, p, actor)


def _org_update(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return orgs.update_organization(engine, p["id"], p["row_version"], {k: v for k, v in p.items() if k not in ("id", "row_version")}, actor)


def _org_list(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return orgs.list_organizations(
        engine, query=p.get("query"), include_retired=bool(p.get("include_retired", False)), limit=p.get("limit", 100), offset=p.get("offset", 0)
    )


def _link(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return orgs.link_organization(engine, p["production_id"], p["organization_id"], p["role"], actor)


def _unlink(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return orgs.unlink_organization(engine, p["production_id"], p["organization_id"], p["role"], actor)


def _linked(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return orgs.list_linked(engine, p["production_id"], actor)


def _type_create(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.create_type(engine, p, actor)


def _type_update(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.update_type(engine, p["code"], p["row_version"], {k: v for k, v in p.items() if k not in ("code", "row_version")}, actor)


def _type_list(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.list_types(engine, include_retired=bool(p.get("include_retired", False)))


def _sl_create(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.create_status_list(engine, p["code"], actor)


def _sl_list(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.list_status_lists(engine)


def _status_add(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.add_status(engine, p["status_list"], p, actor)


def _status_update(engine: Engine, actor: Actor | None, p: Mapping[str, Any]) -> Any:
    return cfg.update_status(engine, p["id"], p["row_version"], {k: v for k, v in p.items() if k not in ("id", "row_version")}, actor)


_ORG_FIELDS = (
    Param("legal_name", "str"), Param("country", "str", help="ISO 3166-1 alpha-2"), Param("registration_no", "str"),
    Param("tax_no", "str"), Param("notes", "str"),
)  # fmt: skip

COMMANDS: dict[str, Command] = {
    c.id: c
    for c in (
        Command(
            "backlot.production.create", "command.backlot.production.create", ("Production", "New"),
            (Param("code", "str", True, "2..32 of A-Z 0-9 _ -"), Param("type", "str", True, "film, series, commercial, animation, vfx"),
             Param("status", "str"), *_PRODUCTION_FIELDS),
            _create,
        ),
        Command(
            "backlot.production.update", "command.backlot.production.update", ("Production", "Edit"),
            (Param("id", "uuid", True), Param("row_version", "int", True, "the version that was read"), *_PRODUCTION_FIELDS),
            _update,
        ),
        Command("backlot.production.get", "command.backlot.production.get", ("Production", "Open"), (Param("id", "uuid", True),), _get, modifies=False),
        Command(
            "backlot.production.list", "command.backlot.production.list", ("Production", "List"),
            (Param("type", "str"), Param("status", "str"), Param("active", "bool"), Param("limit", "int"), Param("offset", "int")),
            _list, modifies=False,
        ),
        Command(
            "backlot.production.status.set", "command.backlot.production.status.set", ("Production", "Status"),
            (Param("id", "uuid", True), Param("status", "str", True), Param("row_version", "int")), _status_set,
        ),
        Command(
            "backlot.production.member.list", "command.backlot.production.member.list", ("Production", "Members"),
            (Param("id", "uuid", True),), _members, modifies=False,
        ),
        Command(
            "backlot.production.type.create", "command.backlot.production.type.create", ("Settings", "Production types", "New"),
            (Param("code", "str", True), Param("label", "str", True), Param("status_list", "str", True, "code of a status list")), _type_create,
        ),
        Command(
            "backlot.production.type.update", "command.backlot.production.type.update", ("Settings", "Production types", "Edit"),
            (Param("code", "str", True), Param("row_version", "int", True), Param("label", "str"), Param("status_list", "str"),
             Param("retired", "bool", help="no new productions of this type")),
            _type_update,
        ),
        Command(
            "backlot.production.type.list", "command.backlot.production.type.list", ("Settings", "Production types", "List"),
            (Param("include_retired", "bool"),), _type_list, modifies=False,
        ),
        Command(
            "backlot.status_list.create", "command.backlot.status_list.create", ("Settings", "Status lists", "New"),
            (Param("code", "str", True),), _sl_create,
        ),
        Command(
            "backlot.status_list.list", "command.backlot.status_list.list", ("Settings", "Status lists", "List"), (), _sl_list, modifies=False
        ),
        Command(
            "backlot.status.add", "command.backlot.status.add", ("Settings", "Status lists", "Add status"),
            (Param("status_list", "str", True), Param("code", "str", True), Param("label", "str", True),
             Param("category", "str", True, "planned, active, on_hold, closed, cancelled")),
            _status_add,
        ),
        Command(
            "backlot.status.update", "command.backlot.status.update", ("Settings", "Status lists", "Edit status"),
            (Param("id", "uuid", True), Param("row_version", "int", True), Param("label", "str"), Param("category", "str"), Param("ordinal", "int")),
            _status_update,
        ),
        Command(
            "backlot.production.milestone.add", "command.backlot.production.milestone.add", ("Production", "Milestones", "Add"),
            (Param("production_id", "uuid", True), Param("name", "str", True), Param("due_date", "date", True)), _ms_add,
        ),
        Command(
            "backlot.production.milestone.update", "command.backlot.production.milestone.update", ("Production", "Milestones", "Edit"),
            (Param("id", "uuid", True), Param("row_version", "int", True), Param("name", "str"), Param("due_date", "date"),
             Param("done", "bool", help="mark done or open again"), Param("ordinal", "int")),
            _ms_update,
        ),
        Command(
            "backlot.production.milestone.remove", "command.backlot.production.milestone.remove", ("Production", "Milestones", "Remove"),
            (Param("id", "uuid", True), Param("row_version", "int")), _ms_remove,
        ),
        Command(
            "backlot.production.milestone.list", "command.backlot.production.milestone.list", ("Production", "Milestones", "List"),
            (Param("production_id", "uuid", True),), _ms_list, modifies=False,
        ),
        Command(
            "backlot.organization.create", "command.backlot.organization.create", ("Organization", "New"),
            (Param("name", "str", True), *_ORG_FIELDS), _org_create,
        ),
        Command(
            "backlot.organization.update", "command.backlot.organization.update", ("Organization", "Edit"),
            (Param("id", "uuid", True), Param("row_version", "int", True), Param("name", "str"), *_ORG_FIELDS,
             Param("retired", "bool", help="retire or bring back")),
            _org_update,
        ),
        Command(
            "backlot.organization.list", "command.backlot.organization.list", ("Organization", "List"),
            (Param("query", "str"), Param("include_retired", "bool"), Param("limit", "int"), Param("offset", "int")), _org_list, modifies=False,
        ),
        Command(
            "backlot.production.organization.link", "command.backlot.production.organization.link", ("Production", "Organizations", "Link"),
            (Param("production_id", "uuid", True), Param("organization_id", "uuid", True), Param("role", "str", True, "client, producer, coproducer, subcontractor, distributor, other")),
            _link,
        ),
        Command(
            "backlot.production.organization.unlink", "command.backlot.production.organization.unlink", ("Production", "Organizations", "Unlink"),
            (Param("production_id", "uuid", True), Param("organization_id", "uuid", True), Param("role", "str", True)), _unlink,
        ),
        Command(
            "backlot.production.organization.list", "command.backlot.production.organization.list", ("Production", "Organizations", "List"),
            (Param("production_id", "uuid", True),), _linked, modifies=False,
        ),
    )
}


def _coerce(param: Param, raw: Any) -> Any:
    try:
        if param.kind == "int":
            if isinstance(raw, bool):
                raise ValueError("a number is expected")
            return int(raw)
        if param.kind == "bool":
            if isinstance(raw, bool):
                return raw
            if str(raw).lower() in ("true", "1"):
                return True
            if str(raw).lower() in ("false", "0"):
                return False
            raise ValueError(str(raw))
        if param.kind == "uuid":
            return raw if isinstance(raw, uuid.UUID) else uuid.UUID(str(raw))
        return raw if isinstance(raw, str) else str(raw)  # date strings are checked by the service
    except (ValueError, TypeError) as exc:
        raise InvalidInput(f"parameter {param.name!r}: {exc}") from exc


def list_commands() -> list[dict[str, Any]]:
    """The ``commands.list`` answer: every descriptor."""
    return [c.describe() for c in COMMANDS.values()]


def run_command(engine: Engine, command_id: str, params: Mapping[str, Any] | None = None, actor: Actor | None = None) -> Result:
    """Run one command by id. Never raises a BacklotHub or audit error; it returns ``{"ok": false, "code": …}``."""
    command = COMMANDS.get(command_id)
    if command is None:
        return {"ok": False, "code": "BACKLOT_UNKNOWN_COMMAND", "detail": command_id}
    try:
        known = {p.name: p for p in command.params}
        given = dict(params or {})
        extra = sorted(set(given) - set(known))
        if extra:
            raise InvalidInput(f"unknown parameter(s): {', '.join(extra)}")
        missing = [p.name for p in command.params if p.required and given.get(p.name) is None]
        if missing:
            raise InvalidInput(f"missing parameter(s): {', '.join(missing)}")
        coerced = {k: (None if v is None else _coerce(known[k], v)) for k, v in given.items()}
        return {"ok": True, "result": command.run(engine, actor, coerced)}
    except BacklotError as exc:
        return {"ok": False, "code": exc.code, "detail": exc.detail}
    except AuditError as exc:
        return {"ok": False, "code": exc.code, "detail": exc.detail}
    except Exception as exc:  # never crash the service on a bug; report it with a stable code (AGENTS.md section 0)
        error = DatabaseUnavailable(str(exc).splitlines()[0] if str(exc) else type(exc).__name__)
        return {"ok": False, "code": "INTERNAL_ERROR" if not _is_db(exc) else error.code, "detail": type(exc).__name__}


def _is_db(exc: Exception) -> bool:
    module = type(exc).__module__
    return module.startswith(("sqlalchemy", "psycopg"))
