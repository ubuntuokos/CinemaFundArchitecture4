"""Production core: country and currency lists, status lists, production types, production, members (G2-02 section 4).

Revision ID: 0001
Revises:
"""

from __future__ import annotations

import json
import uuid
from pathlib import Path

import sqlalchemy as sa
from alembic import op

revision = "0001"
down_revision = None
branch_labels = None
depends_on = None

_DATA = Path(__file__).resolve().parents[2] / "data"
_NS = uuid.UUID("5d6c1a9e-3b7e-5c0a-9f55-0c0a0f4a7b01")  # fixed namespace: the seed ids are the same on every install
_SEED_ACTOR = "system:migration"

_COMMON = """
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    created_by  text        NOT NULL,
    updated_by  text        NOT NULL,
    row_version integer     NOT NULL DEFAULT 1 CHECK (row_version >= 1)
"""

# (code, label key, ordinal, category): the default production status list (G2-02 section 4)
_STATUSES = [
    ("development", "status.production.development", 1, "planned"),
    ("pre_production", "status.production.pre_production", 2, "active"),
    ("production", "status.production.production", 3, "active"),
    ("post_production", "status.production.post_production", 4, "active"),
    ("delivered", "status.production.delivered", 5, "closed"),
    ("closed", "status.production.closed", 6, "closed"),
    ("on_hold", "status.production.on_hold", 7, "on_hold"),
    ("cancelled", "status.production.cancelled", 8, "cancelled"),
]
_TYPES = ["film", "series", "commercial", "animation", "vfx"]


def _id(*parts: str) -> str:
    return str(uuid.uuid5(_NS, "/".join(parts)))


def upgrade() -> None:
    op.execute(
        """
        CREATE TABLE backlot.country (
            code char(2) PRIMARY KEY CHECK (code ~ '^[A-Z]{2}$'),
            name text NOT NULL
        )"""
    )
    op.execute(
        """
        CREATE TABLE backlot.currency (
            code     char(3) PRIMARY KEY CHECK (code ~ '^[A-Z]{3}$'),
            exponent smallint NOT NULL CHECK (exponent BETWEEN 0 AND 4),
            name_key text NOT NULL,
            active   boolean NOT NULL DEFAULT true
        )"""
    )
    countries = json.loads((_DATA / "iso3166.json").read_text(encoding="utf-8"))["countries"]
    currencies = json.loads((_DATA / "iso4217.json").read_text(encoding="utf-8"))["currencies"]
    op.bulk_insert(
        sa.table("country", sa.column("code"), sa.column("name"), schema="backlot"),
        [{"code": c["code"], "name": c["name"]} for c in countries],
    )
    op.bulk_insert(
        sa.table("currency", sa.column("code"), sa.column("exponent"), sa.column("name_key"), schema="backlot"),
        [{"code": c["code"], "exponent": c["exponent"], "name_key": f"currency.{c['code']}"} for c in currencies],
    )

    op.execute(
        f"""
        CREATE TABLE backlot.status_list (
            id uuid PRIMARY KEY,
            code text NOT NULL UNIQUE,
            target text NOT NULL,
            {_COMMON}
        )"""
    )
    op.execute(
        f"""
        CREATE TABLE backlot.status (
            id uuid PRIMARY KEY,
            status_list_id uuid NOT NULL REFERENCES backlot.status_list(id),
            code text NOT NULL,
            label_key text NOT NULL,
            ordinal smallint NOT NULL,
            category text NOT NULL CHECK (category IN ('planned','active','on_hold','closed','cancelled')),
            {_COMMON},
            UNIQUE (status_list_id, code)
        )"""
    )
    op.execute(
        f"""
        CREATE TABLE backlot.production_type (
            id uuid PRIMARY KEY,
            code text NOT NULL UNIQUE CHECK (code ~ '^[a-z][a-z0-9_]{{1,31}}$'),
            label_key text NOT NULL,
            status_list_id uuid NOT NULL REFERENCES backlot.status_list(id),
            retired_at timestamptz,
            {_COMMON}
        )"""
    )
    op.execute(
        f"""
        CREATE TABLE backlot.production (
            id uuid PRIMARY KEY,
            production_id uuid GENERATED ALWAYS AS (id) STORED,
            code text NOT NULL UNIQUE CHECK (code ~ '^[A-Z0-9][A-Z0-9_-]{{1,31}}$'),
            title text NOT NULL CHECK (title <> '' AND length(title) <= 200),
            type_id uuid NOT NULL REFERENCES backlot.production_type(id),
            status_id uuid NOT NULL REFERENCES backlot.status(id),
            country char(2) NOT NULL REFERENCES backlot.country(code),
            currency char(3) NOT NULL REFERENCES backlot.currency(code),
            reference_currency char(3) NOT NULL REFERENCES backlot.currency(code)
                CHECK (reference_currency IN ('EUR','USD')),
            fps_num integer NOT NULL CHECK (fps_num > 0),
            fps_den integer NOT NULL CHECK (fps_den > 0),
            width integer CHECK (width > 0),
            height integer CHECK (height > 0),
            pixel_aspect_num integer NOT NULL DEFAULT 1 CHECK (pixel_aspect_num > 0),
            pixel_aspect_den integer NOT NULL DEFAULT 1 CHECK (pixel_aspect_den > 0),
            start_timecode text CHECK (start_timecode ~ '^[0-9]{{2}}:[0-5][0-9]:[0-5][0-9][:;][0-9]{{2}}$'),
            episode_count integer CHECK (episode_count >= 0),
            planned_start date,
            planned_end date,
            description text,
            settings jsonb NOT NULL DEFAULT '{{}}' CHECK (jsonb_typeof(settings) = 'object'),
            {_COMMON},
            CHECK ((width IS NULL) = (height IS NULL)),
            CHECK (planned_end IS NULL OR planned_start IS NULL OR planned_end >= planned_start)
        )"""
    )
    op.execute("CREATE INDEX production_status_idx ON backlot.production (status_id)")
    op.execute("CREATE INDEX production_type_idx ON backlot.production (type_id)")
    # The status must belong to the status list of the production's type (STATUS_NOT_IN_LIST).
    op.execute(
        """
        CREATE FUNCTION backlot.check_production_status() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM backlot.status s JOIN backlot.production_type t ON t.status_list_id = s.status_list_id
                WHERE s.id = NEW.status_id AND t.id = NEW.type_id
            ) THEN
                RAISE EXCEPTION 'status is not in the status list of the production type' USING ERRCODE = 'BL001';
            END IF;
            RETURN NEW;
        END $$"""
    )
    op.execute(
        "CREATE TRIGGER production_status_in_list BEFORE INSERT OR UPDATE OF status_id, type_id ON backlot.production "
        "FOR EACH ROW EXECUTE FUNCTION backlot.check_production_status()"
    )
    op.execute(
        """
        CREATE TABLE backlot.production_member (
            production_id uuid NOT NULL REFERENCES backlot.production(id),
            actor_kind text NOT NULL CHECK (actor_kind IN ('os_user','identity')),
            actor_ref text NOT NULL CHECK (actor_ref <> ''),
            role text NOT NULL CHECK (role IN ('owner','producer','coordinator','artist','viewer')),
            created_at timestamptz NOT NULL DEFAULT now(),
            created_by text NOT NULL,
            PRIMARY KEY (production_id, actor_kind, actor_ref)
        )"""
    )
    op.execute("CREATE INDEX production_member_actor_idx ON backlot.production_member (actor_kind, actor_ref)")

    # Seed data (once: an Alembic revision runs once): one status list, its statuses, the five base types.
    status_list_id = _id("status_list", "production_default")
    op.execute(
        sa.text(
            "INSERT INTO backlot.status_list (id, code, target, created_by, updated_by) "
            "VALUES (CAST(:id AS uuid), 'production_default', 'production', :a, :a)"
        ).bindparams(id=status_list_id, a=_SEED_ACTOR)
    )
    for code, label, ordinal, category in _STATUSES:
        op.execute(
            sa.text(
                "INSERT INTO backlot.status (id, status_list_id, code, label_key, ordinal, category, created_by, updated_by) "
                "VALUES (CAST(:id AS uuid), CAST(:sl AS uuid), :code, :label, :ord, :cat, :a, :a)"
            ).bindparams(
                id=_id("status", "production_default", code), sl=status_list_id, code=code, label=label, ord=ordinal,
                cat=category, a=_SEED_ACTOR,
            )
        )
    for code in _TYPES:
        op.execute(
            sa.text(
                "INSERT INTO backlot.production_type (id, code, label_key, status_list_id, created_by, updated_by) "
                "VALUES (CAST(:id AS uuid), :code, :label, CAST(:sl AS uuid), :a, :a)"
            ).bindparams(
                id=_id("production_type", code), code=code, label=f"production_type.{code}", sl=status_list_id,
                a=_SEED_ACTOR,
            )
        )


def downgrade() -> None:
    # The schema (and Alembic's version table inside it) stays; everything this revision created goes.
    for statement in (
        "DROP TABLE backlot.production_member",
        "DROP TABLE backlot.production",
        "DROP FUNCTION backlot.check_production_status()",
        "DROP TABLE backlot.production_type",
        "DROP TABLE backlot.status",
        "DROP TABLE backlot.status_list",
        "DROP TABLE backlot.currency",
        "DROP TABLE backlot.country",
    ):
        op.execute(statement)
