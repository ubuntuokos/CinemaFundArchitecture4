"""Alembic environment: one migration chain for the whole ``backlot`` schema (G2-02 section 2).

Run through :mod:`cfa4_backlothub.migrate`, which hands in an open connection (``config.attributes["connection"]``);
the version table lives in the ``backlot`` schema itself.
"""

from __future__ import annotations

from alembic import context

SCHEMA = "backlot"


def run_migrations() -> None:
    connection = context.config.attributes["connection"]
    connection.exec_driver_sql(f"CREATE SCHEMA IF NOT EXISTS {SCHEMA}")
    context.configure(
        connection=connection,
        target_metadata=None,
        version_table_schema=SCHEMA,
        transaction_per_migration=False,
    )
    with context.begin_transaction():
        context.run_migrations()


run_migrations()
