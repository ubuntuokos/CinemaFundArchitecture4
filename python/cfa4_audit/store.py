"""The audit trail's write and read paths (CFA4-AUDIT-001 AU-2, G2-01 sections 7, 9, 10).

Writing (:func:`record`, :func:`unit`) happens **in the caller's open transaction**: the audit event and the
business change commit together or not at all (fail-closed). The chain end is locked for the rest of that
transaction, so ``record`` is the LAST step before ``COMMIT``; :func:`unit` makes that natural.

Reading (:func:`list_events`, :func:`verify`, :func:`export_events`) needs only the ``cfa4_audit_reader`` role.
"""

from __future__ import annotations

import csv
import io
import json
import os
import tempfile
import uuid
from collections.abc import Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import psycopg
from psycopg.types.json import Jsonb

from .canonical import GENESIS_PREV_HASH, event_hash
from .errors import AuditError, InvalidEvent
from .event import AuditEvent, format_timestamp, new_salt, redact

EXPORT_FORMAT = "cfa4-audit-export/1"
_MAX_LIMIT = 1000


class ChainConflict(AuditError):
    """The chain moved under the writer or the new event does not extend it (``AUDIT_CHAIN_CONFLICT``)."""

    code = "AUDIT_CHAIN_CONFLICT"


class WriteDenied(AuditError):
    """The connection's role may not write audit events (``AUDIT_WRITE_DENIED``)."""

    code = "AUDIT_WRITE_DENIED"


class DatabaseUnavailable(AuditError):
    """The database cannot be reached or answered with an error that is not an audit rule (``DB_UNAVAILABLE``)."""

    code = "DB_UNAVAILABLE"


class ChainBroken(AuditError):
    """The stored chain does not verify (``AUDIT_CHAIN_BROKEN``)."""

    code = "AUDIT_CHAIN_BROKEN"


class CheckpointMismatch(AuditError):
    """The chain does not match a checkpoint (``AUDIT_CHECKPOINT_MISMATCH``)."""

    code = "AUDIT_CHECKPOINT_MISMATCH"


@dataclass(frozen=True)
class Recorded:
    """What a write returns: the position and hash the event got in the chain."""

    seq: int
    hash: bytes
    event_id: uuid.UUID


@dataclass(frozen=True)
class StoredEvent:
    """One event as stored (``hashable`` is the object that was hashed, ``hash`` its hash)."""

    seq: int
    hashable: dict[str, Any]
    hash: bytes

    def as_export(self) -> dict[str, Any]:
        return {**self.hashable, "hash": self.hash.hex()}


# ---------------------------------------------------------------- writing


def _map_db_error(exc: psycopg.Error) -> AuditError:
    state = getattr(exc, "sqlstate", None)
    text = str(exc).splitlines()[0] if str(exc) else ""
    if state == "AU003":
        return ChainConflict(text)
    if state == "42501":  # insufficient_privilege
        return WriteDenied(text)
    return DatabaseUnavailable(text)


def _personal_change(
    event: AuditEvent, personal: Mapping[str, Mapping[str, Any]], salt: bytes
) -> dict[str, Any]:
    """The event's ``change`` with every personal value replaced by its salted digest (G2-01 section 6)."""
    change: dict[str, Any] = dict(event.change or {})
    for name, sides in personal.items():
        if name in change:
            raise InvalidEvent(f"field {name!r} is both in change and in personal")
        if not set(sides) <= {"old", "new"} or not sides:
            raise InvalidEvent("a personal field has only 'old' and/or 'new'")
        change[name] = {side: redact(value, salt) for side, value in sides.items()}
    return change


def record(
    conn: psycopg.Connection,
    event: AuditEvent,
    *,
    personal: Mapping[str, Mapping[str, Any]] | None = None,
) -> Recorded:
    """Append ``event`` to the chain inside the caller's open transaction.

    ``personal`` maps a field name to ``{"old": …, "new": …}`` of values that must not enter the chain: the chain gets
    their salted digests, ``personal_detail`` the values and the salt. Must be the last step before ``COMMIT``.
    Raises :class:`ChainConflict`, :class:`WriteDenied` or :class:`DatabaseUnavailable`; never leaves a half-written
    event (the caller's rollback undoes it).
    """
    if personal:
        salt = new_salt()
        event = AuditEvent(**{**event.__dict__, "change": _personal_change(event, personal, salt)})
    else:
        salt = b""
    try:
        with conn.cursor() as cur:
            cur.execute("SELECT seq, hash, head_recorded_at, now_utc FROM cfa4_audit.lock_head()")
            head = cur.fetchone()
            if head is None:
                raise ChainBroken("the chain head is missing")
            seq, prev_hash, recorded_at = int(head[0]) + 1, bytes(head[1]), head[3]
            body = event.hashable(seq=seq, recorded_at=recorded_at, prev_hash=prev_hash)
            digest = event_hash(body)
            cur.execute("SELECT cfa4_audit.append(%s::jsonb, %s, %s, %s)", (Jsonb(body), digest, prev_hash, seq))
            if personal:
                detail = {name: dict(sides) for name, sides in personal.items()}
                cur.execute(
                    "SELECT cfa4_audit.write_personal(%s, %s, %s, %s::jsonb, %s)",
                    (event.event_id, event.subject_type, event.subject_id, Jsonb(detail), salt),
                )
    except psycopg.Error as exc:
        raise _map_db_error(exc) from exc
    return Recorded(seq=seq, hash=digest, event_id=event.event_id)


class Unit:
    """Events of one business transaction: queued with :meth:`record`, written together when the block ends."""

    def __init__(self, conn: psycopg.Connection) -> None:
        self._conn = conn
        self._queued: list[tuple[AuditEvent, Mapping[str, Mapping[str, Any]] | None]] = []
        self.recorded: list[Recorded] = []

    def record(self, event: AuditEvent, *, personal: Mapping[str, Mapping[str, Any]] | None = None) -> None:
        self._queued.append((event, personal))

    def _flush(self) -> None:
        for event, personal in self._queued:
            self.recorded.append(record(self._conn, event, personal=personal))
        self._queued.clear()


@contextmanager
def unit(conn: psycopg.Connection) -> Iterator[Unit]:
    """``with unit(conn) as u: …business changes…; u.record(ev)``: the queued events are written when the block ends
    without an error, as the last statements before the caller's ``COMMIT``. On an error nothing is written."""
    u = Unit(conn)
    yield u
    u._flush()


# ---------------------------------------------------------------- reading


_COLUMNS = (
    "seq, event_id, schema_ver, occurred_at, recorded_at, app, action, command_id, actor_kind, actor_ref, "
    "on_behalf_of, subject_type, subject_id, scope, change, outcome, reason_code, correlation_id, causation_id, "
    "trace_id, prev_hash, hash"
)


def _row_to_stored(row: Sequence[Any]) -> StoredEvent:
    def uid(value: Any) -> str | None:
        return None if value is None else str(value)

    hashable = {
        "seq": int(row[0]),
        "event_id": str(row[1]),
        "schema_ver": int(row[2]),
        "occurred_at": format_timestamp(row[3]),
        "recorded_at": format_timestamp(row[4]),
        "app": row[5],
        "action": row[6],
        "command_id": row[7],
        "actor_kind": str(row[8]),
        "actor_ref": row[9],
        "on_behalf_of": row[10],
        "subject_type": row[11],
        "subject_id": str(row[12]),
        "scope": row[13],
        "change": row[14],
        "outcome": str(row[15]),
        "reason_code": row[16],
        "correlation_id": uid(row[17]),
        "causation_id": uid(row[18]),
        "trace_id": row[19],
        "prev_hash": bytes(row[20]).hex(),
    }
    return StoredEvent(seq=hashable["seq"], hashable=hashable, hash=bytes(row[21]))


@dataclass(frozen=True)
class Filters:
    """``audit.list`` / ``audit.export`` filters; every field is optional and they combine with AND."""

    actor_kind: str | None = None
    actor_ref: str | None = None
    subject_type: str | None = None
    subject_id: uuid.UUID | None = None
    action: str | None = None  # exact, or a prefix ending in ".*" (e.g. "production.*")
    occurred_from: datetime | None = None
    occurred_to: datetime | None = None  # exclusive
    scope_contains: dict[str, Any] = field(default_factory=dict)


def _where(filters: Filters, after_seq: int) -> tuple[str, list[Any]]:
    clauses: list[str] = ["seq > %s"]
    params: list[Any] = [after_seq]
    if filters.actor_kind is not None:
        clauses.append("actor_kind = %s::cfa4_audit.actor_kind")
        params.append(filters.actor_kind)
    if filters.actor_ref is not None:
        clauses.append("actor_ref = %s")
        params.append(filters.actor_ref)
    if filters.subject_type is not None:
        clauses.append("subject_type = %s")
        params.append(filters.subject_type)
    if filters.subject_id is not None:
        clauses.append("subject_id = %s")
        params.append(filters.subject_id)
    if filters.action is not None:
        if filters.action.endswith(".*"):
            clauses.append("action LIKE %s ESCAPE '\\'")
            params.append(filters.action[:-1].replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_") + "%")
        else:
            clauses.append("action = %s")
            params.append(filters.action)
    if filters.occurred_from is not None:
        clauses.append("occurred_at >= %s")
        params.append(filters.occurred_from)
    if filters.occurred_to is not None:
        clauses.append("occurred_at < %s")
        params.append(filters.occurred_to)
    if filters.scope_contains:
        clauses.append("scope @> %s::jsonb")
        params.append(Jsonb(filters.scope_contains))
    return " AND ".join(clauses), params


def list_events(
    conn: psycopg.Connection, filters: Filters | None = None, *, after_seq: int = 0, limit: int = 100
) -> list[StoredEvent]:
    """``audit.list``: events in ``seq`` order, paged by ``after_seq`` (the last seq of the previous page)."""
    if not 1 <= limit <= _MAX_LIMIT:
        raise InvalidEvent(f"limit must be 1..{_MAX_LIMIT}")
    where, params = _where(filters or Filters(), after_seq)
    try:
        with conn.cursor() as cur:
            cur.execute(f"SELECT {_COLUMNS} FROM cfa4_audit.event WHERE {where} ORDER BY seq LIMIT %s", [*params, limit])
            return [_row_to_stored(r) for r in cur.fetchall()]
    except psycopg.Error as exc:
        raise _map_db_error(exc) from exc


@dataclass(frozen=True)
class VerifyResult:
    """``audit.verify``: ``ok`` or the first fault (``code`` and ``seq`` say what and where)."""

    ok: bool
    checked: int
    code: str | None = None
    seq: int | None = None
    detail: str = ""
    erased_personal: int = 0
    head_seq: int = 0


def verify(
    conn: psycopg.Connection,
    *,
    from_seq: int = 1,
    to_seq: int | None = None,
    checkpoint: Mapping[str, Any] | None = None,
    batch: int = 500,
) -> VerifyResult:
    """``audit.verify``: recompute every hash from the stored columns, check the links and the chain end.

    A fault is returned, never raised: a broken chain is a finding, not a crash. ``checkpoint`` (a loaded checkpoint
    file, :func:`load_checkpoint`) additionally proves the chain was not cut back below it.
    """
    try:
        return _verify(conn, from_seq, to_seq, checkpoint, batch)
    except psycopg.Error as exc:
        return VerifyResult(False, 0, code="DB_UNAVAILABLE", detail=str(exc).splitlines()[0] if str(exc) else "")


def _verify(
    conn: psycopg.Connection, from_seq: int, to_seq: int | None, checkpoint: Mapping[str, Any] | None, batch: int
) -> VerifyResult:
    with conn.cursor() as cur:
        cur.execute("SELECT seq, hash FROM cfa4_audit.chain_head WHERE id = 1")
        head = cur.fetchone()
        cur.execute("SELECT cfa4_audit.personal_erased_count()")
        erased_row = cur.fetchone()
    erased = int(erased_row[0]) if erased_row is not None else 0
    if head is None:
        return VerifyResult(False, 0, code="AUDIT_CHAIN_BROKEN", detail="the chain head is missing", erased_personal=erased)
    head_seq, head_hash = int(head[0]), bytes(head[1])
    last = head_seq if to_seq is None else min(to_seq, head_seq)

    def fault(code: str, seq: int | None, detail: str, checked: int) -> VerifyResult:
        return VerifyResult(False, checked, code=code, seq=seq, detail=detail, erased_personal=erased, head_seq=head_seq)

    prev_hash = GENESIS_PREV_HASH
    if from_seq > 1:  # the link into the range comes from the stored previous event
        with conn.cursor() as cur:
            cur.execute("SELECT hash FROM cfa4_audit.event WHERE seq = %s", (from_seq - 1,))
            row = cur.fetchone()
        if row is None:
            return fault("AUDIT_CHAIN_BROKEN", from_seq - 1, "the event before the range is missing", 0)
        prev_hash = bytes(row[0])
    expected_seq, checked = from_seq, 0
    while expected_seq <= last:
        with conn.cursor() as cur:
            cur.execute(
                f"SELECT {_COLUMNS} FROM cfa4_audit.event WHERE seq >= %s AND seq <= %s ORDER BY seq LIMIT %s",
                (expected_seq, last, batch),
            )
            rows = cur.fetchall()
        if not rows:
            return fault("AUDIT_CHAIN_BROKEN", expected_seq, "an event is missing from the chain", checked)
        for row in rows:
            stored = _row_to_stored(row)
            if stored.seq != expected_seq:
                return fault("AUDIT_CHAIN_BROKEN", expected_seq, "an event is missing from the chain", checked)
            if stored.hashable["schema_ver"] != 1:
                return fault("AUDIT_SCHEMA_VERSION_UNKNOWN", stored.seq, "unknown schema_ver", checked)
            if bytes.fromhex(stored.hashable["prev_hash"]) != prev_hash:
                return fault("AUDIT_CHAIN_BROKEN", stored.seq, "prev_hash does not match the previous event", checked)
            if event_hash(stored.hashable) != stored.hash:
                return fault("AUDIT_CHAIN_BROKEN", stored.seq, "the stored hash does not match the content", checked)
            prev_hash = stored.hash
            expected_seq += 1
            checked += 1
    if to_seq is None or to_seq >= head_seq:
        if prev_hash != head_hash and checked:
            return fault("AUDIT_CHAIN_BROKEN", head_seq, "the chain end does not match the last event", checked)
    if checkpoint is not None:
        problem = _check_checkpoint(conn, checkpoint, head_seq)
        if problem is not None:
            return fault("AUDIT_CHECKPOINT_MISMATCH", int(checkpoint["seq"]), problem, checked)
    return VerifyResult(True, checked, erased_personal=erased, head_seq=head_seq)


def _check_checkpoint(conn: psycopg.Connection, checkpoint: Mapping[str, Any], head_seq: int) -> str | None:
    seq, want = int(checkpoint["seq"]), str(checkpoint["hash"])
    if head_seq < seq:
        return f"the chain ends at seq {head_seq}, before the checkpoint at seq {seq} (cut back)"
    with conn.cursor() as cur:
        cur.execute("SELECT hash FROM cfa4_audit.event WHERE seq = %s", (seq,))
        row = cur.fetchone()
    if row is None:
        return f"the event at checkpoint seq {seq} is missing"
    if bytes(row[0]).hex() != want:
        return f"the hash at seq {seq} differs from the checkpoint"
    return None


# ---------------------------------------------------------------- checkpoint files


def default_checkpoint_root() -> Path:
    """``$XDG_STATE_HOME/cfa4/audit/checkpoints`` (outside the PostgreSQL data directory, owned by the user)."""
    state = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    return Path(state) / "cfa4" / "audit" / "checkpoints"


def cluster_id(conn: psycopg.Connection) -> str:
    """The cluster identifier recorded in the genesis event."""
    with conn.cursor() as cur:
        cur.execute("SELECT scope->>'cluster_id' FROM cfa4_audit.event WHERE seq = 1")
        row = cur.fetchone()
    if row is None or row[0] is None:
        raise ChainBroken("the genesis event is missing")
    return str(row[0])


def _atomic_write(path: Path, data: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=path.name + ".", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(data)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)
    except BaseException:
        Path(tmp).unlink(missing_ok=True)
        raise


def take_checkpoint(conn: psycopg.Connection, *, root: Path | None = None) -> Path:
    """``audit.checkpoint``: write ``<root>/<cluster-id>/<seq>.json`` for the current chain end and index it in the
    database. The file is written atomically and survives a restore of the database; verify against it with
    :func:`load_checkpoint`. Needs the writer role."""
    root = root or default_checkpoint_root()
    try:
        with conn.cursor() as cur:
            cur.execute("SELECT seq, hash FROM cfa4_audit.chain_head WHERE id = 1")
            head = cur.fetchone()
        if head is None or int(head[0]) < 1:
            raise ChainBroken("the chain has no events")
        cluster = cluster_id(conn)
        seq, digest = int(head[0]), bytes(head[1])
        path = root / cluster / f"{seq}.json"
        body = {
            "format": "cfa4-audit-checkpoint/1",
            "seq": seq,
            "hash": digest.hex(),
            "cluster_id": cluster,
            "taken_at": format_timestamp(datetime.now(UTC)),
        }
        _atomic_write(path, json.dumps(body, indent=2) + "\n")
        with conn.cursor() as cur:
            cur.execute("SELECT cfa4_audit.record_checkpoint(%s, %s, %s)", (seq, digest, str(path)))
    except psycopg.Error as exc:
        raise _map_db_error(exc) from exc
    return path


def load_checkpoint(conn: psycopg.Connection, *, root: Path | None = None, path: Path | None = None) -> dict[str, Any]:
    """The checkpoint to verify against: ``path`` itself, or the newest file of this cluster under ``root``."""
    if path is None:
        directory = (root or default_checkpoint_root()) / cluster_id(conn)
        files = sorted(directory.glob("*.json"), key=lambda p: int(p.stem)) if directory.is_dir() else []
        if not files:
            raise CheckpointMismatch(f"no checkpoint file in {directory}")
        path = files[-1]
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
        if loaded.get("format") != "cfa4-audit-checkpoint/1" or int(loaded["seq"]) < 1 or len(str(loaded["hash"])) != 64:
            raise ValueError("not a checkpoint file")
        if str(loaded.get("cluster_id")) != cluster_id(conn):
            raise CheckpointMismatch("the checkpoint belongs to a different cluster")
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise CheckpointMismatch(f"unreadable checkpoint {path}: {exc}") from exc
    return dict(loaded)


# ---------------------------------------------------------------- export, maintenance, erase


def export_events(
    conn: psycopg.Connection, path: Path, filters: Filters | None = None, *, fmt: str = "json", page: int = _MAX_LIMIT
) -> int:
    """``audit.export``: write the matching events to ``path`` (atomically); returns the event count.

    ``json`` carries every field and the hashes, so the file can be verified outside the database; ``csv`` is flat
    (``scope`` and ``change`` as JSON text) for spreadsheets and has no hash guarantee of its own beyond the columns.
    """
    if fmt not in ("json", "csv"):
        raise InvalidEvent("format must be json or csv")
    events: list[StoredEvent] = []
    after = 0
    while True:
        chunk = list_events(conn, filters, after_seq=after, limit=page)
        events.extend(chunk)
        if len(chunk) < page:
            break
        after = chunk[-1].seq
    if fmt == "json":
        data = json.dumps(
            {"format": EXPORT_FORMAT, "events": [e.as_export() for e in events]}, ensure_ascii=False, indent=1
        )
    else:
        buf = io.StringIO()
        writer = csv.writer(buf, lineterminator="\n")
        names = [*(events[0].as_export().keys() if events else ["seq", "hash"])]
        writer.writerow(names)
        for e in events:
            row = e.as_export()
            writer.writerow(
                [json.dumps(row[n], ensure_ascii=False, sort_keys=True) if isinstance(row[n], dict) else row[n] for n in names]
            )
        data = buf.getvalue()
    _atomic_write(path, data + ("\n" if not data.endswith("\n") else ""))
    return len(events)


def ensure_partitions(conn: psycopg.Connection, months_ahead: int = 2) -> int:
    """``audit.partition.ensure``: create the monthly partitions up to ``months_ahead`` ahead (maintenance, run as
    ``cfa4_audit_owner``); returns how many were created."""
    try:
        with conn.cursor() as cur:
            cur.execute("SELECT cfa4_audit.ensure_partitions(%s)", (months_ahead,))
            row = cur.fetchone()
    except psycopg.Error as exc:
        raise _map_db_error(exc) from exc
    return int(row[0]) if row is not None else 0


def erase_personal(conn: psycopg.Connection, subject_type: str, subject_id: uuid.UUID) -> int:
    """``audit.personal.erase`` (SEC-06): erase the personal values and salts of one subject; the chain is untouched.

    Returns the number of rows erased. The caller (the business command that performs the erasure) then records
    the ``audit.personal.erased`` event in the same transaction with :func:`record`.
    """
    try:
        with conn.cursor() as cur:
            cur.execute("SELECT cfa4_audit.erase_personal(%s, %s)", (subject_type, subject_id))
            row = cur.fetchone()
    except psycopg.Error as exc:
        raise _map_db_error(exc) from exc
    return int(row[0]) if row is not None else 0
