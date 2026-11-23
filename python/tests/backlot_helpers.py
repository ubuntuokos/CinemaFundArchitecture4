"""Shared helpers of the BacklotHub tests (the PostgreSQL fixtures are in ``conftest.py``)."""

from __future__ import annotations

import uuid
from typing import Any

import psycopg
import psycopg.sql
from sqlalchemy import Engine, text

from cfa4_audit import list_events
from cfa4_audit.migrate import install
from cfa4_backlothub.db.uow import Actor

ME = Actor("os_user", "uid:1000/istvan")
OTHER = Actor("os_user", "uid:1001/anna")


def new_db(cluster: str) -> str:
    """A fresh database of the throw-away cluster with the audit schema installed; returns its connection string."""
    name = "t" + uuid.uuid4().hex[:12]
    with psycopg.connect(cluster, autocommit=True) as conn:
        conn.execute(psycopg.sql.SQL("CREATE DATABASE {}").format(psycopg.sql.Identifier(name)))
        conn.execute(psycopg.sql.SQL("GRANT CREATE ON DATABASE {} TO cfa4_test_app").format(psycopg.sql.Identifier(name)))
    info = cluster.replace("dbname=postgres", f"dbname={name}")
    with psycopg.connect(info) as conn:
        install(conn)
    return info


def params(**changes: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "code": "FILM-1", "title": "A film", "type": "film", "country": "HU", "currency": "HUF",
        "reference_currency": "EUR", "fps_num": 24000, "fps_den": 1001,
    }  # fmt: skip
    base.update(changes)
    return base


def audit_events(engine: Engine) -> list[Any]:
    with engine.connect() as conn:
        return list_events(conn.connection.driver_connection, None, limit=1000)  # type: ignore[arg-type]


def backlot_events(engine: Engine) -> list[dict[str, Any]]:
    return [e.as_export() for e in audit_events(engine) if e.hashable["app"] == "backlot"]


def count(engine: Engine, table: str) -> int:
    with engine.connect() as conn:
        return int(conn.execute(text(f"SELECT count(*) FROM backlot.{table}")).scalar_one())  # noqa: S608
