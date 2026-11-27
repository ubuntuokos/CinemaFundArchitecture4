"""The unit of work: one transaction = the business change + its audit events (G2-02 section 2).

``with uow(engine, actor) as u: ...; u.audit(...)``. The audit events are written as the very last statements before the
``COMMIT`` (G2-01 section 7); if the audit fails or the block raises, nothing is committed.
"""

from __future__ import annotations

import getpass
import os
import uuid
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from sqlalchemy import Connection, Engine

from cfa4_audit import ActorKind, AuditEvent, Outcome, record


@dataclass(frozen=True)
class Actor:
    """Who acts. M1: the operating-system user (``uid:1000/istvan``), the same text as in the audit trail."""

    kind: str
    ref: str


def os_actor() -> Actor:
    try:
        name = getpass.getuser()
    except (KeyError, OSError):  # no passwd entry (a container)
        name = "unknown"
    return Actor("os_user", f"uid:{os.getuid()}/{name}")


class Uow:
    def __init__(self, conn: Connection, actor: Actor) -> None:
        self.conn = conn
        self.actor = actor
        self._events: list[AuditEvent] = []

    def audit(
        self,
        action: str,
        subject_type: str,
        subject_id: uuid.UUID,
        *,
        production_id: uuid.UUID | None = None,
        entity_path: str | None = None,
        change: Mapping[str, Any] | None = None,
        command_id: str | None = None,
    ) -> None:
        """Queue one audit event (``scope = {"production_id": ...}``); it is written when the block ends."""
        scope: dict[str, Any] = {"production_id": str(production_id)} if production_id is not None else {}
        if entity_path is not None:
            scope["entity_path"] = entity_path  # a whole sequence's history can be filtered by path
        self._events.append(
            AuditEvent(
                app="backlot",
                action=action,
                actor_kind=ActorKind(self.actor.kind),
                actor_ref=self.actor.ref,
                subject_type=subject_type,
                subject_id=subject_id,
                outcome=Outcome.SUCCESS,
                occurred_at=datetime.now(UTC),
                scope=scope,
                change=dict(change) if change is not None else None,
                command_id=command_id,
            )
        )

    def _flush_audit(self) -> None:
        driver = self.conn.connection.driver_connection  # the psycopg connection of this very transaction
        for event in self._events:
            record(driver, event)  # type: ignore[arg-type]
        self._events.clear()


@contextmanager
def uow(engine: Engine, actor: Actor | None = None) -> Iterator[Uow]:
    """Open a transaction; commit with the queued audit events, or roll everything back."""
    with engine.connect() as conn:
        trans = conn.begin()
        u = Uow(conn, actor or os_actor())
        try:
            yield u
            u._flush_audit()
            trans.commit()
        except BaseException:
            trans.rollback()
            raise
