"""AU-4.1: the archive package of a closed month (CFA4-AUDIT-001 section 8).

A package is one self-describing JSON file ``<root>/<cluster-id>/<YYYY-MM>.cfa4audit.json`` plus a ``.sha256`` sidecar
(``sha256sum`` format). It holds the manifest (first and last ``seq``, the boundary hashes, the event count and the SHA-256
of the events) and every event of the month with its hash, so it verifies without any database
(:func:`verify_archive`). Creating a package only reads: **nothing is removed from the database** (releasing a
partition is a separate, approved step, not part of this module).
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Any

import psycopg

from .canonical import canonical_bytes, event_hash
from .errors import AuditError, InvalidEvent
from .store import (
    _COLUMNS,
    _MAX_LIMIT,
    ChainBroken,
    _atomic_write,
    _map_db_error,
    _row_to_stored,
    cluster_id,
)

ARCHIVE_FORMAT = "cfa4-audit-archive/1"
SUFFIX = ".cfa4audit.json"


class ArchiveInvalid(AuditError):
    """An archive package does not verify (``AUDIT_ARCHIVE_INVALID``)."""

    code = "AUDIT_ARCHIVE_INVALID"


@dataclass(frozen=True)
class ArchiveResult:
    """``audit.archive.verify``: ``ok`` or the first fault."""

    ok: bool
    count: int = 0
    code: str | None = None
    detail: str = ""
    first_seq: int | None = None
    last_seq: int | None = None


def _month_bounds(month: date) -> tuple[datetime, datetime]:
    start = datetime(month.year, month.month, 1, tzinfo=UTC)
    end = datetime(month.year + month.month // 12, month.month % 12 + 1, 1, tzinfo=UTC)
    return start, end


def _events_sha256(events: list[dict[str, Any]]) -> str:
    return hashlib.sha256(canonical_bytes(events)).hexdigest()


def archive_month(conn: psycopg.Connection, month: date, root: Path, *, now: datetime | None = None) -> Path:
    """``audit.archive``: write the package of the closed month ``month`` (any day in it) and return its path.

    Refuses an open month (the current one and later), an empty month and an existing package. The chain is checked
    on the way (every hash, every link, and the link into the month): a broken chain is never archived.
    """
    start, end = _month_bounds(month)
    if end > (now or datetime.now(UTC)):
        raise InvalidEvent("ARCHIVE_MONTH_OPEN: only a closed month can be archived")
    events: list[dict[str, Any]] = []
    try:
        with conn.cursor() as cur:
            after = 0
            while True:
                cur.execute(
                    f"SELECT {_COLUMNS} FROM cfa4_audit.event WHERE recorded_at >= %s AND recorded_at < %s AND seq > %s "
                    "ORDER BY seq LIMIT %s",
                    (start, end, after, _MAX_LIMIT),
                )
                rows = [_row_to_stored(r) for r in cur.fetchall()]
                events.extend(r.as_export() for r in rows)
                if len(rows) < _MAX_LIMIT:
                    break
                after = rows[-1].seq
            if not events:
                raise InvalidEvent(f"ARCHIVE_EMPTY: no events were recorded in {start:%Y-%m}")
            cur.execute("SELECT hash FROM cfa4_audit.event WHERE seq = %s", (int(events[0]["seq"]) - 1,))
            before = cur.fetchone()
        cluster = cluster_id(conn)
    except psycopg.Error as exc:
        raise _map_db_error(exc) from exc
    problem = _chain_problem(events)
    if problem is None and before is not None and bytes(before[0]).hex() != events[0]["prev_hash"]:
        problem = "the month does not link to the event before it"
    if problem is not None:
        raise ChainBroken(f"not archived: {problem}")
    path = root / cluster / f"{start:%Y-%m}{SUFFIX}"
    if path.exists():
        raise InvalidEvent(f"ARCHIVE_EXISTS: {path}")
    manifest = {
        "cluster_id": cluster,
        "month": f"{start:%Y-%m}",
        "first_seq": events[0]["seq"],
        "last_seq": events[-1]["seq"],
        "count": len(events),
        "first_prev_hash": events[0]["prev_hash"],
        "last_hash": events[-1]["hash"],
        "events_sha256": _events_sha256(events),
        "created_at": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }
    text = json.dumps({"format": ARCHIVE_FORMAT, "manifest": manifest, "events": events}, ensure_ascii=False, indent=1)
    _atomic_write(path, text)
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
    _atomic_write(path.with_name(path.name + ".sha256"), f"{digest}  {path.name}\n")
    return path


def _chain_problem(events: list[dict[str, Any]]) -> str | None:
    """The first fault in a run of exported events, or None."""
    prev: tuple[Any, Any] | None = None  # (seq, hash) of the previous event
    for ev in events:
        seq = ev.get("seq")
        try:
            computed = event_hash(ev).hex()
        except AuditError as exc:
            return f"seq {seq}: {exc.code}"
        if computed != ev.get("hash"):
            return f"seq {seq}: content does not match its hash"
        if prev is not None and (ev.get("prev_hash") != prev[1] or seq != prev[0] + 1):
            return f"seq {seq}: not linked to the previous event"
        prev = (seq, ev["hash"])
    return None


def verify_archive(path: Path) -> ArchiveResult:
    """``audit.archive.verify``: check a package without any database. Never raises for a bad file."""
    try:
        raw = path.read_text(encoding="utf-8")
        doc = json.loads(raw)
        if not isinstance(doc, dict) or doc.get("format") != ARCHIVE_FORMAT:
            return ArchiveResult(False, code=ArchiveInvalid.code, detail="not an audit archive package")
        manifest, events = doc["manifest"], doc["events"]
        if not isinstance(events, list) or not events or not all(isinstance(e, dict) for e in events):
            return ArchiveResult(False, code=ArchiveInvalid.code, detail="no events")
        sidecar = path.with_name(path.name + ".sha256")
        if sidecar.exists():
            expected = sidecar.read_text(encoding="utf-8").split()[0]
            if hashlib.sha256(raw.encode("utf-8")).hexdigest() != expected:
                return ArchiveResult(False, code=ArchiveInvalid.code, detail="the file does not match its .sha256")
        if _events_sha256(events) != manifest["events_sha256"]:
            return ArchiveResult(False, code=ArchiveInvalid.code, detail="the events do not match the manifest digest")
        problem = _chain_problem(events)
        if problem is None:
            first, last = events[0], events[-1]
            if (
                manifest["count"] != len(events)
                or manifest["first_seq"] != first["seq"]
                or manifest["last_seq"] != last["seq"]
                or manifest["first_prev_hash"] != first["prev_hash"]
                or manifest["last_hash"] != last["hash"]
            ):
                problem = "the manifest does not describe the events"
        if problem is not None:
            return ArchiveResult(False, code=ArchiveInvalid.code, detail=problem)
        return ArchiveResult(True, len(events), first_seq=int(events[0]["seq"]), last_seq=int(events[-1]["seq"]))
    except (OSError, ValueError, KeyError, TypeError, IndexError) as exc:
        return ArchiveResult(False, code=ArchiveInvalid.code, detail=f"unreadable package: {type(exc).__name__}")
