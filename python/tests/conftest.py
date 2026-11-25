"""PostgreSQL fixtures of the BacklotHub tests: one throw-away cluster per module, a migrated database per test."""

from __future__ import annotations

from collections.abc import Iterator

import pytest

from pg_throwaway import throwaway_cluster


@pytest.fixture(scope="module")
def cluster() -> Iterator[str]:
    pytest.importorskip("psycopg")
    import psycopg
    import psycopg.sql

    with throwaway_cluster() as info:
        with psycopg.connect(info, autocommit=True) as conn:
            for role in ("cfa4_audit_owner", "cfa4_audit_writer", "cfa4_audit_reader"):
                conn.execute(psycopg.sql.SQL("CREATE ROLE {} NOLOGIN").format(psycopg.sql.Identifier(role)))
            conn.execute("CREATE ROLE cfa4_test_app LOGIN IN ROLE cfa4_audit_writer, cfa4_audit_reader")
        yield info


@pytest.fixture
def info(cluster: str) -> str:
    pytest.importorskip("psycopg")
    from backlot_helpers import new_db  # imported late: psycopg and SQLAlchemy are optional extras

    return new_db(cluster)


@pytest.fixture
def engine(info: str) -> Iterator[object]:
    pytest.importorskip("sqlalchemy")
    pytest.importorskip("alembic")
    from cfa4_backlothub import migrate
    from cfa4_backlothub.db.engine import make_engine

    eng = make_engine(info.replace("user=postgres", "user=cfa4_test_app"))
    migrate.upgrade(eng)
    yield eng
    eng.dispose()
