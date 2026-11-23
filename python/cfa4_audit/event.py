"""The audit event as a value: validation, the hashable form and the personal-data digest.

Dataclass plus hand-written validation (no pydantic: fewer dependencies, G2-01 section 11). Fields that the
library assigns on write (``seq``, ``recorded_at``, ``prev_hash``) are not part of :class:`AuditEvent`;
:meth:`AuditEvent.hashable` adds them. Writing to the database is AU-2.
"""

from __future__ import annotations

import hashlib
import os
import re
import secrets
import time
import uuid
from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum
from typing import Any

from .canonical import canonical_bytes
from .errors import InvalidEvent

SCHEMA_VER = 1

_ACTION = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$")
_REASON = re.compile(r"^[A-Z][A-Z0-9_]*$")
_HEX32 = re.compile(r"^[0-9a-f]{64}$")


class ActorKind(StrEnum):
    OS_USER = "os_user"
    IDENTITY = "identity"
    SYSTEM = "system"
    AGENT = "agent"


class Outcome(StrEnum):
    SUCCESS = "success"
    REJECTED = "rejected"
    FAILED = "failed"


def uuid7() -> uuid.UUID:
    """A UUIDv7 (RFC 9562): 48-bit Unix millisecond timestamp, version 7, 74 random bits."""
    ms = time.time_ns() // 1_000_000
    rand = int.from_bytes(os.urandom(10), "big")
    value = (ms & (2**48 - 1)) << 80 | 0x7 << 76 | (rand >> 68 & 0xFFF) << 64 | 0b10 << 62 | rand & (2**62 - 1)
    return uuid.UUID(int=value)


def format_timestamp(moment: datetime) -> str:
    """RFC 3339, UTC, ``Z``, microsecond precision (the resolution of PostgreSQL ``timestamptz``)."""
    if moment.tzinfo is None:
        raise InvalidEvent("timestamp has no time zone")
    return moment.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def new_salt() -> bytes:
    """16 random bytes: the salt of one personal-data digest (kept next to the detail, never in the chain)."""
    return secrets.token_bytes(16)


def redact(value: Any, salt: bytes) -> dict[str, Any]:
    """The chain-side stand-in of a personal value: ``{"redacted": true, "digest": sha256(salt || canonical)}``.

    The salted digest cannot be reversed by a dictionary attack on low-entropy values (phone number, date of
    birth, IBAN, name) and, once the salt is erased with the detail (SEC-06), not at all (G2-01 section 6).
    """
    if len(salt) != 16:
        raise InvalidEvent("salt must be 16 bytes")
    return {"redacted": True, "digest": hashlib.sha256(salt + canonical_bytes(value)).hexdigest()}


@dataclass(frozen=True)
class AuditEvent:
    """What happened, to what, by whom; validated on construction."""

    app: str
    action: str
    actor_kind: ActorKind
    actor_ref: str
    subject_type: str
    subject_id: uuid.UUID
    outcome: Outcome
    occurred_at: datetime
    event_id: uuid.UUID = field(default_factory=uuid7)
    command_id: str | None = None
    on_behalf_of: str | None = None
    scope: dict[str, Any] = field(default_factory=dict)
    change: dict[str, Any] | None = None
    reason_code: str | None = None
    correlation_id: uuid.UUID | None = None
    causation_id: uuid.UUID | None = None
    trace_id: str | None = None
    schema_ver: int = SCHEMA_VER

    def __post_init__(self) -> None:
        if not _ACTION.match(self.action):
            raise InvalidEvent("action must be dotted lower-case, e.g. production.created")
        if self.reason_code is not None and not _REASON.match(self.reason_code):
            raise InvalidEvent("reason_code must be UPPER_SNAKE_CASE")
        for name in ("app", "actor_ref", "subject_type"):
            if not getattr(self, name):
                raise InvalidEvent(f"{name} must not be empty")
        if self.event_id.version != 7:
            raise InvalidEvent("event_id must be a UUIDv7")
        if self.occurred_at.tzinfo is None:
            raise InvalidEvent("occurred_at has no time zone")
        if self.schema_ver != SCHEMA_VER:
            raise InvalidEvent("unknown schema_ver")
        if self.outcome is not Outcome.SUCCESS and self.reason_code is None:
            raise InvalidEvent("a rejected or failed outcome needs a reason_code")
        # Fail early on floats and non-JSON content instead of at write time.
        canonical_bytes(self.scope)
        if self.change is not None:
            canonical_bytes(self.change)

    def hashable(self, *, seq: int, recorded_at: datetime, prev_hash: bytes) -> dict[str, Any]:
        """The event object that is hashed and stored (without ``hash``)."""
        if seq < 1:
            raise InvalidEvent("seq starts at 1")
        if len(prev_hash) != 32:
            raise InvalidEvent("prev_hash must be 32 bytes")
        return {
            "seq": seq,
            "event_id": str(self.event_id),
            "schema_ver": self.schema_ver,
            "occurred_at": format_timestamp(self.occurred_at),
            "recorded_at": format_timestamp(recorded_at),
            "app": self.app,
            "action": self.action,
            "command_id": self.command_id,
            "actor_kind": self.actor_kind.value,
            "actor_ref": self.actor_ref,
            "on_behalf_of": self.on_behalf_of,
            "subject_type": self.subject_type,
            "subject_id": str(self.subject_id),
            "scope": self.scope,
            "change": self.change,
            "outcome": self.outcome.value,
            "reason_code": self.reason_code,
            "correlation_id": None if self.correlation_id is None else str(self.correlation_id),
            "causation_id": None if self.causation_id is None else str(self.causation_id),
            "trace_id": self.trace_id,
            "prev_hash": prev_hash.hex(),
        }
