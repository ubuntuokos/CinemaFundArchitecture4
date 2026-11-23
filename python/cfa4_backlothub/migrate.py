"""Apply and revert the BacklotHub schema migrations (Alembic, one chain)."""

from __future__ import annotations

from pathlib import Path

from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import Engine

_SCRIPTS = Path(__file__).parent / "migrations"


def _config(connection: object) -> Config:
    cfg = Config()
    cfg.set_main_option("script_location", str(_SCRIPTS))
    cfg.attributes["connection"] = connection
    return cfg


def upgrade(engine: Engine, revision: str = "head") -> None:
    """Bring the schema to ``revision`` in one transaction (all or nothing)."""
    with engine.begin() as conn:
        command.upgrade(_config(conn), revision)


def downgrade(engine: Engine, revision: str = "base") -> None:
    with engine.begin() as conn:
        command.downgrade(_config(conn), revision)


def current(engine: Engine) -> str | None:
    """The applied revision, or None on an empty database."""
    with engine.connect() as conn:
        exists = conn.exec_driver_sql("SELECT to_regclass('backlot.alembic_version')").scalar()
        if exists is None:
            return None
        return MigrationContext.configure(conn, opts={"version_table_schema": "backlot"}).get_current_revision()


def head() -> str | None:
    return ScriptDirectory(str(_SCRIPTS)).get_current_head()
