"""CFA4 common audit trail (CFA4-AUDIT-001, ADR 0008), Python side.

AU-1: canonical form, hash and the event value (this package); the database write path is AU-2.
"""

from .canonical import GENESIS_PREV_HASH, HASH_DOMAIN_V1, canonical_bytes, event_hash, hash_canonical
from .errors import AuditError, EventNotObject, FloatForbidden, InvalidEvent, InvalidValue
from .event import SCHEMA_VER, ActorKind, AuditEvent, Outcome, format_timestamp, new_salt, redact, uuid7

__all__ = [
    "GENESIS_PREV_HASH",
    "HASH_DOMAIN_V1",
    "SCHEMA_VER",
    "ActorKind",
    "AuditError",
    "AuditEvent",
    "EventNotObject",
    "FloatForbidden",
    "InvalidEvent",
    "InvalidValue",
    "Outcome",
    "canonical_bytes",
    "event_hash",
    "format_timestamp",
    "hash_canonical",
    "new_salt",
    "redact",
    "uuid7",
]
