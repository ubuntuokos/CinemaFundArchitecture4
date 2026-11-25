"""CFA4 common audit trail (CFA4-AUDIT-001, ADR 0008), Python side.

AU-1: canonical form, hash, the event value and the database schema; AU-2: write path (``record``, ``unit``), ``list_events``,
``verify``, ``export_events``, checkpoints.
"""

from .archive import ArchiveResult, archive_month, verify_archive
from .canonical import GENESIS_PREV_HASH, HASH_DOMAIN_V1, canonical_bytes, event_hash, hash_canonical
from .errors import AuditError, EventNotObject, FloatForbidden, InvalidEvent, InvalidValue
from .store import (
    ChainBroken,
    ChainConflict,
    CheckpointMismatch,
    DatabaseUnavailable,
    Filters,
    Recorded,
    StoredEvent,
    Unit,
    VerifyResult,
    WriteDenied,
    ensure_partitions,
    erase_personal,
    export_events,
    list_events,
    load_checkpoint,
    record,
    take_checkpoint,
    unit,
    verify,
)
from .event import SCHEMA_VER, ActorKind, AuditEvent, Outcome, format_timestamp, new_salt, redact, uuid7

__all__ = [
    "GENESIS_PREV_HASH",
    "HASH_DOMAIN_V1",
    "SCHEMA_VER",
    "ActorKind",
    "ArchiveResult",
    "ChainBroken",
    "ChainConflict",
    "CheckpointMismatch",
    "DatabaseUnavailable",
    "Filters",
    "Recorded",
    "StoredEvent",
    "Unit",
    "VerifyResult",
    "WriteDenied",
    "archive_month",
    "ensure_partitions",
    "erase_personal",
    "export_events",
    "list_events",
    "load_checkpoint",
    "record",
    "take_checkpoint",
    "unit",
    "verify",
    "verify_archive",
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
