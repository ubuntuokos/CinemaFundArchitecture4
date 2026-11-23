"""SQLAlchemy Core tables of the ``backlot`` schema (they mirror the Alembic migrations; a test compares them)."""

from __future__ import annotations

from typing import Any

from sqlalchemy import (
    CHAR,
    TIMESTAMP,
    Boolean,
    Column,
    Date,
    Integer,
    MetaData,
    SmallInteger,
    Table,
    Text,
    Uuid,
)
from sqlalchemy.dialects.postgresql import JSONB

metadata = MetaData(schema="backlot")


def _common() -> list[Column[Any]]:
    return [
        Column("created_at", TIMESTAMP(timezone=True), nullable=False),
        Column("updated_at", TIMESTAMP(timezone=True), nullable=False),
        Column("created_by", Text, nullable=False),
        Column("updated_by", Text, nullable=False),
        Column("row_version", Integer, nullable=False),
    ]


country = Table("country", metadata, Column("code", CHAR(2), primary_key=True), Column("name", Text, nullable=False))

currency = Table(
    "currency",
    metadata,
    Column("code", CHAR(3), primary_key=True),
    Column("exponent", SmallInteger, nullable=False),
    Column("name_key", Text, nullable=False),
    Column("active", Boolean, nullable=False),
)

status_list = Table(
    "status_list",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("code", Text, nullable=False),
    Column("target", Text, nullable=False),
    *_common(),
)

status = Table(
    "status",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("status_list_id", Uuid, nullable=False),
    Column("code", Text, nullable=False),
    Column("label_key", Text, nullable=False),
    Column("ordinal", SmallInteger, nullable=False),
    Column("category", Text, nullable=False),
    *_common(),
)

production_type = Table(
    "production_type",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("code", Text, nullable=False),
    Column("label_key", Text, nullable=False),
    Column("status_list_id", Uuid, nullable=False),
    Column("retired_at", TIMESTAMP(timezone=True)),
    *_common(),
)

# ``production.production_id`` is a generated column (= id) the database keeps; it is never written, so it is not mapped.
production = Table(
    "production",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("code", Text, nullable=False),
    Column("title", Text, nullable=False),
    Column("type_id", Uuid, nullable=False),
    Column("status_id", Uuid, nullable=False),
    Column("country", CHAR(2), nullable=False),
    Column("currency", CHAR(3), nullable=False),
    Column("reference_currency", CHAR(3), nullable=False),
    Column("fps_num", Integer, nullable=False),
    Column("fps_den", Integer, nullable=False),
    Column("width", Integer),
    Column("height", Integer),
    Column("pixel_aspect_num", Integer, nullable=False),
    Column("pixel_aspect_den", Integer, nullable=False),
    Column("start_timecode", Text),
    Column("episode_count", Integer),
    Column("planned_start", Date),
    Column("planned_end", Date),
    Column("description", Text),
    Column("settings", JSONB, nullable=False),
    *_common(),
)

production_member = Table(
    "production_member",
    metadata,
    Column("production_id", Uuid, primary_key=True),
    Column("actor_kind", Text, primary_key=True),
    Column("actor_ref", Text, primary_key=True),
    Column("role", Text, nullable=False),
    Column("created_at", TIMESTAMP(timezone=True), nullable=False),
    Column("created_by", Text, nullable=False),
)

milestone = Table(
    "milestone",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("production_id", Uuid, nullable=False),
    Column("name", Text, nullable=False),
    Column("due_date", Date, nullable=False),
    Column("done_at", TIMESTAMP(timezone=True)),
    Column("ordinal", SmallInteger, nullable=False),
    *_common(),
)

organization = Table(
    "organization",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("name", Text, nullable=False),
    Column("legal_name", Text),
    Column("country", CHAR(2)),
    Column("registration_no", Text),
    Column("tax_no", Text),
    Column("notes", Text),
    Column("retired_at", TIMESTAMP(timezone=True)),
    *_common(),
)

production_organization = Table(
    "production_organization",
    metadata,
    Column("production_id", Uuid, primary_key=True),
    Column("organization_id", Uuid, primary_key=True),
    Column("role", Text, primary_key=True),
    Column("created_at", TIMESTAMP(timezone=True), nullable=False),
    Column("created_by", Text, nullable=False),
)
