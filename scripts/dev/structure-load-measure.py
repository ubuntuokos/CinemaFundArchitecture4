#!/usr/bin/env python3
"""ST-2.3: measure the production structure at the L profile (G2-03 section 9) on a throw-away PostgreSQL cluster.

52 productions, each with 10 000 shots, 2 000 assets and the sequences and scenes above them (about 630 000 elements
in all). The rows are bulk-loaded with the row triggers off and then checked with the same invariants the triggers
enforce; everything measured afterwards goes through the real service functions. The result is JSON on stdout. Run it
under the CFA4-MIN limits (``scripts/dev/run-minspec.sh``) for the numbers that count; elsewhere the output is only a
stand-in and says so (``cpus``).

    PYTHONPATH=python:python/tests .venv/bin/python scripts/dev/structure-load-measure.py --productions 52
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
import time
import uuid
from collections.abc import Callable
from typing import Any

import psycopg
import psycopg.sql
from sqlalchemy import text

from backlot_helpers import ME, new_db, params
from cfa4_backlothub import migrate
from cfa4_backlothub.db.engine import make_engine
from cfa4_backlothub.production import service as svc
from cfa4_backlothub.structure import config as sconfig
from cfa4_backlothub.structure import entities as ent
from cfa4_backlothub.structure import operations as ops
from pg_throwaway import throwaway_cluster

ACTS, SEQUENCES, SCENES, SHOTS = 5, 4, 5, 100  # per production: 5 acts x 4 sequences x 5 scenes x 100 shots = 10 000 shots; an act is 2 025 elements
ASSETS = {"character": 1000, "prop": 500, "set": 500}


def _timed(fn: Callable[[], Any], repeat: int = 20) -> dict[str, float]:
    times = []
    for _ in range(repeat):
        t0 = time.perf_counter()
        fn()
        times.append((time.perf_counter() - t0) * 1000)
    times.sort()
    return {"median_ms": round(statistics.median(times), 2), "p95_ms": round(times[max(0, int(len(times) * 0.95) - 1)], 2), "runs": repeat}


def _load(conn: psycopg.Connection[Any], pid: uuid.UUID) -> None:
    """One production's tree with plain SQL (ids from gen_random_uuid; the path is parent path || label)."""
    types = dict(conn.execute("SELECT code, id FROM backlot.entity_type").fetchall())
    seq, scene, shot = types["sequence"], types["scene"], types["shot"]
    common = "created_by, updated_by"
    conn.execute(
        f"""
        WITH acts AS (
            INSERT INTO backlot.production_entity (id, production_id, entity_type_id, parent_id, path, code, name, ordinal, {common})
            SELECT id, %(p)s, %(seq)s, NULL, backlot.entity_label(id), 'ACT' || n, 'Act ' || n, n, 'load', 'load'
            FROM (SELECT gen_random_uuid() AS id, n FROM generate_series(1, %(acts)s) n) g RETURNING id, path, ordinal),
        seqs AS (
            INSERT INTO backlot.production_entity (id, production_id, entity_type_id, parent_id, path, code, name, ordinal, {common})
            SELECT id, %(p)s, %(seq)s, pid, ppath || backlot.entity_label(id), 'SQ' || pord || n, 'Sequence', n, 'load', 'load'
            FROM (SELECT a.id AS pid, a.path AS ppath, a.ordinal AS pord, n, gen_random_uuid() AS id
                  FROM acts a CROSS JOIN generate_series(1, %(seqs)s) n) g RETURNING id, path),
        scenes AS (
            INSERT INTO backlot.production_entity (id, production_id, entity_type_id, parent_id, path, code, name, ordinal, {common})
            SELECT id, %(p)s, %(scene)s, pid, ppath || backlot.entity_label(id), 'SC' || n, 'Scene', n, 'load', 'load'
            FROM (SELECT s.id AS pid, s.path AS ppath, n, gen_random_uuid() AS id
                  FROM seqs s CROSS JOIN generate_series(1, %(scenes)s) n) g RETURNING id, path)
        INSERT INTO backlot.production_entity (id, production_id, entity_type_id, parent_id, path, code, name, ordinal, {common})
        SELECT id, %(p)s, %(shot)s, pid, ppath || backlot.entity_label(id), 'SH' || n, 'Shot', n, 'load', 'load'
        FROM (SELECT s.id AS pid, s.path AS ppath, n, gen_random_uuid() AS id
              FROM scenes s CROSS JOIN generate_series(1, %(shots)s) n) g""",
        {"p": pid, "seq": seq, "scene": scene, "shot": shot, "acts": ACTS, "seqs": SEQUENCES, "scenes": SCENES, "shots": SHOTS},
    )
    for code, count in ASSETS.items():
        conn.execute(
            f"""INSERT INTO backlot.production_entity (id, production_id, entity_type_id, parent_id, path, code, name, ordinal, {common})
                SELECT id, %(p)s, %(t)s, NULL, backlot.entity_label(id), upper(%(c)s) || n, %(c)s, n, 'load', 'load'
                FROM (SELECT gen_random_uuid() AS id, n FROM generate_series(1, %(n)s) n) g""",
            {"p": pid, "t": types[code], "c": code, "n": count},
        )
    # every fifth shot is cast with one character
    conn.execute(
        """INSERT INTO backlot.entity_link (id, production_id, from_id, to_id, link_type, created_by, updated_by)
           SELECT gen_random_uuid(), %(p)s, s.id, c.id, 'casting', 'load', 'load'
           FROM (SELECT e.id, row_number() OVER (ORDER BY e.path) AS rn FROM backlot.production_entity e
                 WHERE e.production_id = %(p)s AND e.entity_type_id = %(shot)s) s
           JOIN (SELECT e.id, row_number() OVER (ORDER BY e.code) AS rn FROM backlot.production_entity e
                 WHERE e.production_id = %(p)s AND e.entity_type_id = %(char)s) c ON c.rn = (s.rn %% %(chars)s) + 1
           WHERE s.rn %% 5 = 0""",
        {"p": pid, "shot": shot, "char": types["character"], "chars": ASSETS["character"]},
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--productions", type=int, default=52)
    args = ap.parse_args()
    out: dict[str, Any] = {"cpus": os.cpu_count(), "productions": args.productions, "note": "stand-in unless run under the CFA4-MIN limits"}
    with throwaway_cluster() as cluster:
        with psycopg.connect(cluster, autocommit=True) as admin:
            for role in ("cfa4_audit_owner", "cfa4_audit_writer", "cfa4_audit_reader"):
                admin.execute(psycopg.sql.SQL("CREATE ROLE {} NOLOGIN").format(psycopg.sql.Identifier(role)))
            admin.execute("CREATE ROLE cfa4_test_app LOGIN IN ROLE cfa4_audit_writer, cfa4_audit_reader")
        info = new_db(cluster)
        engine = make_engine(info.replace("user=postgres", "user=cfa4_test_app"))
        migrate.upgrade(engine)
        pids = [uuid.UUID(svc.create_production(engine, params(code=f"P{i:03d}"), ME)["id"]) for i in range(args.productions)]
        for pid in pids:  # an act may hold sequences (a production rule, so this also exercises backlot.structure.rule.add)
            sconfig.add_rule(engine, pid, {"parent": "sequence", "child": "sequence"}, ME)

        started = time.perf_counter()
        with psycopg.connect(info) as conn:  # the owner loads with the row triggers off, then the invariants are checked below
            conn.execute("ALTER TABLE backlot.production_entity DISABLE TRIGGER USER")
            conn.execute("ALTER TABLE backlot.entity_link DISABLE TRIGGER USER")
            for pid in pids:
                _load(conn, pid)
            conn.execute("ALTER TABLE backlot.production_entity ENABLE TRIGGER USER")
            conn.execute("ALTER TABLE backlot.entity_link ENABLE TRIGGER USER")
            conn.commit()
            conn.execute("ANALYZE backlot.production_entity")
            conn.execute("ANALYZE backlot.entity_link")
            conn.commit()
            total = conn.execute("SELECT count(*) FROM backlot.production_entity").fetchone()[0]  # type: ignore[index]
            bad = conn.execute(
                "SELECT count(*) FROM backlot.production_entity e LEFT JOIN backlot.production_entity p ON p.id = e.parent_id "
                "WHERE e.path <> CASE WHEN p.id IS NULL THEN backlot.entity_label(e.id) ELSE p.path || backlot.entity_label(e.id) END"
            ).fetchone()[0]  # type: ignore[index]
            size = conn.execute("SELECT pg_total_relation_size('backlot.production_entity')").fetchone()[0]  # type: ignore[index]
        out["load"] = {"entities": total, "path_mismatches": bad, "seconds": round(time.perf_counter() - started, 1), "bytes_per_entity_with_indexes": round(size / total)}

        pid = pids[len(pids) // 2]
        tree = ent.tree_level(engine, pid, actor=ME)
        out["tree_first_level"] = _timed(lambda: ent.tree_level(engine, pid, actor=ME))
        out["tree_first_level"]["rows"] = len(tree)
        act = tree[0]
        act_id = uuid.UUID(act["id"])
        out["tree_second_level"] = _timed(lambda: ent.tree_level(engine, pid, act_id, actor=ME))
        out["open_entity"] = _timed(lambda: ent.get_entity(engine, act_id, ME))

        def subtree() -> int:
            with engine.connect() as conn:
                return int(conn.execute(text("SELECT count(*) FROM backlot.production_entity WHERE path <@ CAST(:p AS ltree)"), {"p": act["path"]}).scalar_one())

        out["subtree_size"] = subtree()
        out["subtree_query"] = _timed(subtree)
        out["impact_of_an_act"] = _timed(lambda: ops.impact(engine, act_id, actor=ME), repeat=10)
        counter = [0]

        def create() -> None:
            counter[0] += 1
            ent.create_entity(engine, pid, {"type": "sequence", "code": f"NEW{counter[0]}", "name": "new"}, ME)

        out["create_entity"] = _timed(create, repeat=30)

        # move the two-thousand-element subtree under another act and back to the top level
        target = uuid.UUID(tree[1]["id"])
        moves = []
        row_version = ent.get_entity(engine, act_id, ME)["row_version"]
        for destination in (target, None, target, None):
            t0 = time.perf_counter()
            moved = ops.move_entity(engine, act_id, row_version, destination, ME)
            moves.append((time.perf_counter() - t0) * 1000)
            row_version = moved["row_version"]
        out["move_subtree"] = {"elements": out["subtree_size"], "ms_each": [round(m, 1) for m in moves], "median_ms": round(statistics.median(moves), 1)}
        with engine.connect() as conn:
            out["path_mismatches_after_moves"] = conn.execute(
                text(
                    "SELECT count(*) FROM backlot.production_entity e LEFT JOIN backlot.production_entity p ON p.id = e.parent_id WHERE e.production_id = :p "
                    "AND e.path <> CASE WHEN p.id IS NULL THEN backlot.entity_label(e.id) ELSE p.path || backlot.entity_label(e.id) END"
                ),
                {"p": pid},
            ).scalar_one()
        t0 = time.perf_counter()
        retired = ops.retire_entity(engine, act_id, row_version, True, ME)
        out["retire_subtree"] = {"elements": out["subtree_size"], "ms": round((time.perf_counter() - t0) * 1000, 1)}
        t0 = time.perf_counter()
        ops.retire_entity(engine, act_id, retired["row_version"], False, ME)
        out["restore_subtree"] = {"elements": out["subtree_size"], "ms": round((time.perf_counter() - t0) * 1000, 1)}
        engine.dispose()
    json.dump(out, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
