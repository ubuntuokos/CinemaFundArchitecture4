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
    cast,
    func,
)
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.types import UserDefinedType

metadata = MetaData(schema="backlot")


class Ltree(UserDefinedType[str]):
    """PostgreSQL ``ltree`` as text on the Python side (bound with ``text2ltree``, read back with a cast to text)."""

    cache_ok = True

    def get_col_spec(self, **kw: Any) -> str:
        return "ltree"

    def bind_expression(self, bindvalue: Any) -> Any:
        return func.text2ltree(bindvalue)

    def column_expression(self, colexpr: Any) -> Any:
        return cast(colexpr, Text)


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

entity_type = Table(
    "entity_type",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("code", Text, nullable=False),
    Column("label_key", Text, nullable=False),
    Column("kind", Text, nullable=False),
    Column("icon", Text),
    Column("has_timing", Boolean, nullable=False),
    Column("status_list_id", Uuid),
    Column("retired_at", TIMESTAMP(timezone=True)),
    *_common(),
)

structure_template = Table(
    "structure_template",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("code", Text, nullable=False),
    Column("label_key", Text, nullable=False),
    Column("production_type_id", Uuid),
    *_common(),
)

# The two rule tables have no primary key in the database (a unique constraint with NULLS NOT DISTINCT); the whole row is the key here.
structure_template_rule = Table(
    "structure_template_rule",
    metadata,
    Column("template_id", Uuid, primary_key=True),
    Column("parent_type_id", Uuid, primary_key=True, nullable=True),
    Column("child_type_id", Uuid, primary_key=True),
)

production_structure_rule = Table(
    "production_structure_rule",
    metadata,
    Column("production_id", Uuid, primary_key=True),
    Column("parent_type_id", Uuid, primary_key=True, nullable=True),
    Column("child_type_id", Uuid, primary_key=True),
)

production_entity = Table(
    "production_entity",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("production_id", Uuid, nullable=False),
    Column("entity_type_id", Uuid, nullable=False),
    Column("parent_id", Uuid),
    Column("path", Ltree, nullable=False),
    Column("code", Text, nullable=False),
    Column("name", Text, nullable=False),
    Column("ordinal", Integer, nullable=False),
    Column("status_id", Uuid),
    Column("description", Text),
    Column("metadata", JSONB, nullable=False),
    Column("retired_at", TIMESTAMP(timezone=True)),
    *_common(),
)

entity_timing = Table(
    "entity_timing",
    metadata,
    Column("entity_id", Uuid, primary_key=True),
    Column("frame_in", Integer, nullable=False),
    Column("frame_out", Integer, nullable=False),
    Column("handle_in", Integer, nullable=False),
    Column("handle_out", Integer, nullable=False),
    Column("otio_ref", Text),
    Column("edit_project_ref", Uuid),
)

entity_link = Table(
    "entity_link",
    metadata,
    Column("id", Uuid, primary_key=True),
    Column("production_id", Uuid, nullable=False),
    Column("from_id", Uuid, nullable=False),
    Column("to_id", Uuid, nullable=False),
    Column("link_type", Text, nullable=False),
    Column("metadata", JSONB, nullable=False),
    *_common(),
)
