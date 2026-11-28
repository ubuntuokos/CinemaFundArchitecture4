#!/usr/bin/env python3
"""AU-3.2: measure the audit chain under load on a throw-away PostgreSQL cluster (CFA4-AUDIT-001 AU-3).

Writes N events through the real ``cfa4_audit.record`` path (no shortcuts), then measures verify and the common
queries. The result is JSON on stdout. Run it under the CFA4-MIN limits (``scripts/dev/run-minspec.sh``) for the
numbers that count; on any other machine the output is only a stand-in and says so (``cpus``).

    PYTHONPATH=python:python/tests .venv/bin/python scripts/dev/audit-load-measure.py --events 1000000 --batch 100
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
import time
import uuid
from datetime import UTC, datetime

import psycopg
import psycopg.sql

from cfa4_audit import ActorKind, AuditEvent, Filters, Outcome, list_events, record, verify
from cfa4_audit.migrate import install
from pg_throwaway import throwaway_cluster


def _event(i: int, subjects: list[uuid.UUID]) -> AuditEvent:
    return AuditEvent(
        app="backlot",
        action="task.updated",
        actor_kind=ActorKind.OS_USER,
        actor_ref=f"uid:{1000 + i % 20}",
        subject_type="task",
        subject_id=subjects[i % len(subjects)],
        outcome=Outcome.SUCCESS,
        occurred_at=datetime.now(UTC),
        scope={"production": i % 52, "i": i},
        change={"status": {"old": "open", "new": "done"}},
    )


def _timed(fn):  # type: ignore[no-untyped-def]
    start = time.perf_counter()
    result = fn()
    return result, time.perf_counter() - start


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--events", type=int, default=100_000)
    ap.add_argument("--batch", type=int, default=100, help="events per transaction (1 = one transaction per event)")
    ap.add_argument("--queries", type=int, default=30)
    args = ap.parse_args()

    out: dict[str, object] = {
        "cpus": os.cpu_count(),
        "events": args.events,
        "batch": args.batch,
        "note": "stand-in unless run under the CFA4-MIN limits",
    }
    subjects = [uuid.uuid4() for _ in range(2000)]
    with throwaway_cluster() as info:
        with psycopg.connect(info, autocommit=True) as admin:
            for role in ("cfa4_audit_owner", "cfa4_audit_writer", "cfa4_audit_reader"):
                admin.execute(psycopg.sql.SQL("CREATE ROLE {} NOLOGIN").format(psycopg.sql.Identifier(role)))
            admin.execute("CREATE ROLE cfa4_app LOGIN IN ROLE cfa4_audit_writer, cfa4_audit_reader")
            admin.execute("CREATE DATABASE load")
        db = info.replace("dbname=postgres", "dbname=load")
        with psycopg.connect(db) as conn:
            install(conn)
        app = db.replace("user=postgres", "user=cfa4_app")

        latencies: list[float] = []
        started = time.perf_counter()
        with psycopg.connect(app) as conn:
            done = 0
            while done < args.events:
                n = min(args.batch, args.events - done)
                t0 = time.perf_counter()
                for k in range(n):
                    record(conn, _event(done + k, subjects))
                conn.commit()
                latencies.append((time.perf_counter() - t0) / n)
                done += n
        elapsed = time.perf_counter() - started
        latencies.sort()
        out["write"] = {
            "seconds": round(elapsed, 1),
            "events_per_second": round(args.events / elapsed),
            "per_event_ms_median": round(statistics.median(latencies) * 1000, 3),
            "per_event_ms_p99": round(latencies[int(len(latencies) * 0.99) - 1] * 1000, 3),
        }
        with psycopg.connect(db) as conn:
            conn.execute("ANALYZE cfa4_audit.event")
            size = conn.execute("SELECT sum(pg_total_relation_size(inhrelid)) FROM pg_inherits WHERE inhparent = 'cfa4_audit.event'::regclass").fetchone()
            out["bytes_per_event_with_indexes"] = round(int(size[0]) / (args.events + 1)) if size else None

        with psycopg.connect(app) as conn:
            result, secs = _timed(lambda: verify(conn))
            out["verify"] = {
                "ok": result.ok,
                "seconds": round(secs, 1),
                "events_per_second": round(result.checked / secs) if secs else None,
            }

            def timed_query(make):  # type: ignore[no-untyped-def]
                times = []
                for q in range(args.queries):
                    _, secs = _timed(lambda q=q: make(q))
                    times.append(secs * 1000)
                times.sort()
                return {"median_ms": round(statistics.median(times), 2), "p95_ms": round(times[int(len(times) * 0.95) - 1], 2)}

            out["query_by_subject_ms"] = timed_query(
                lambda q: list_events(conn, Filters(subject_type="task", subject_id=subjects[q % len(subjects)]), limit=50)
            )
            out["query_by_actor_ms"] = timed_query(
                lambda q: list_events(conn, Filters(actor_kind=ActorKind.OS_USER, actor_ref=f"uid:{1000 + q % 20}"), limit=50)
            )
    json.dump(out, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
