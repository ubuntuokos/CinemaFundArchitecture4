"""The SQLAlchemy engine on the psycopg 3 driver (G2-02 section 2)."""

from __future__ import annotations

import psycopg
from sqlalchemy import Engine, create_engine


def make_engine(conninfo: str = "") -> Engine:
    """An engine for a libpq connection string (empty: the libpq defaults, ``PGHOST`` and friends)."""
    return create_engine("postgresql+psycopg://", creator=lambda: psycopg.connect(conninfo), pool_pre_ping=True)
