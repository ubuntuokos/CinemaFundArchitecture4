"""Production structure: entity types, templates, rules, the entity tree (ltree), timing and links (G2-03 section 4).

Revision ID: 0003
Revises: 0002
"""

from __future__ import annotations

import uuid

import sqlalchemy as sa
from alembic import op

revision = "0003"
down_revision = "0002"
branch_labels = None
depends_on = None

_NS = uuid.UUID("5d6c1a9e-3b7e-5c0a-9f55-0c0a0f4a7b01")  # the namespace of the 0001 seed ids
_SEED_ACTOR = "system:migration"

_COMMON = """
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    created_by  text        NOT NULL,
    updated_by  text        NOT NULL,
    row_version integer     NOT NULL DEFAULT 1 CHECK (row_version >= 1)
"""

_STRUCTURAL = ["season", "unit", "episode", "spot", "sequence", "scene", "shot"]
_ASSET = ["character", "prop", "set", "vehicle", "fx", "environment"]
# template code -> (production type code, [(parent type or None, child type)]); G2-03 section 4
_FILM = [(None, "sequence"), ("sequence", "scene"), ("scene", "shot")]
_TEMPLATES = {
    "film": ("film", _FILM),
    "series": ("series", [(None, "season"), ("season", "episode"), ("episode", "sequence"), ("sequence", "shot")]),
    "commercial": ("commercial", [(None, "spot"), ("spot", "shot")]),
    "animation": ("animation", _FILM),
    "vfx": ("vfx", _FILM),
}


def _id(*parts: str) -> str:
    return str(uuid.uuid5(_NS, "/".join(parts)))


def upgrade() -> None:
    op.execute("CREATE EXTENSION IF NOT EXISTS ltree")
    op.execute(
        f"""
        CREATE TABLE backlot.entity_type (
            id uuid PRIMARY KEY,
            code text NOT NULL UNIQUE CHECK (code ~ '^[a-z][a-z0-9_]{{1,31}}$'),
            label_key text NOT NULL,
            kind text NOT NULL CHECK (kind IN ('structural','asset')),
            icon text,
            has_timing boolean NOT NULL DEFAULT false,
            status_list_id uuid REFERENCES backlot.status_list(id),
            retired_at timestamptz,
            {_COMMON}
        )"""
    )
    op.execute(
        f"""
        CREATE TABLE backlot.structure_template (
            id uuid PRIMARY KEY,
            code text NOT NULL UNIQUE,
            label_key text NOT NULL,
            production_type_id uuid REFERENCES backlot.production_type(id),
            {_COMMON}
        )"""
    )
    op.execute(
        """
        CREATE TABLE backlot.structure_template_rule (
            template_id uuid NOT NULL REFERENCES backlot.structure_template(id),
            parent_type_id uuid REFERENCES backlot.entity_type(id),
            child_type_id uuid NOT NULL REFERENCES backlot.entity_type(id),
            UNIQUE NULLS NOT DISTINCT (template_id, parent_type_id, child_type_id)
        )"""
    )
    op.execute(
        """
        CREATE TABLE backlot.production_structure_rule (
            production_id uuid NOT NULL REFERENCES backlot.production(id),
            parent_type_id uuid REFERENCES backlot.entity_type(id),
            child_type_id uuid NOT NULL REFERENCES backlot.entity_type(id),
            UNIQUE NULLS NOT DISTINCT (production_id, parent_type_id, child_type_id)
        )"""
    )
    op.execute(
        f"""
        CREATE TABLE backlot.production_entity (
            id uuid PRIMARY KEY,
            production_id uuid NOT NULL REFERENCES backlot.production(id),
            entity_type_id uuid NOT NULL REFERENCES backlot.entity_type(id),
            parent_id uuid REFERENCES backlot.production_entity(id),
            path ltree NOT NULL,
            code text NOT NULL CHECK (code ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{{0,63}}$'),
            name text NOT NULL CHECK (name <> '' AND length(name) <= 200),
            ordinal integer NOT NULL CHECK (ordinal >= 1),
            status_id uuid REFERENCES backlot.status(id),
            description text CHECK (length(description) <= 20000),
            metadata jsonb NOT NULL DEFAULT '{{}}' CHECK (jsonb_typeof(metadata) = 'object'),
            retired_at timestamptz,
            {_COMMON},
            UNIQUE NULLS NOT DISTINCT (production_id, parent_id, entity_type_id, code)
        )"""
    )
    op.execute("CREATE INDEX production_entity_path_idx ON backlot.production_entity USING gist (path)")
    op.execute("CREATE INDEX production_entity_type_idx ON backlot.production_entity (production_id, entity_type_id)")
    op.execute("CREATE INDEX production_entity_parent_idx ON backlot.production_entity (parent_id, ordinal)")
    op.execute(
        """
        CREATE TABLE backlot.entity_timing (
            entity_id uuid PRIMARY KEY REFERENCES backlot.production_entity(id),
            frame_in integer NOT NULL CHECK (frame_in >= 0),
            frame_out integer NOT NULL,
            handle_in integer NOT NULL DEFAULT 0 CHECK (handle_in >= 0),
            handle_out integer NOT NULL DEFAULT 0 CHECK (handle_out >= 0),
            otio_ref text,
            edit_project_ref uuid,
            CHECK (frame_out >= frame_in)
        )"""
    )
    op.execute(
        f"""
        CREATE TABLE backlot.entity_link (
            id uuid PRIMARY KEY,
            production_id uuid NOT NULL REFERENCES backlot.production(id),
            from_id uuid NOT NULL REFERENCES backlot.production_entity(id),
            to_id uuid NOT NULL REFERENCES backlot.production_entity(id),
            link_type text NOT NULL CHECK (link_type ~ '^[a-z][a-z0-9_]*$'),
            metadata jsonb NOT NULL DEFAULT '{{}}' CHECK (jsonb_typeof(metadata) = 'object'),
            {_COMMON},
            UNIQUE (from_id, to_id, link_type),
            CHECK (from_id <> to_id)
        )"""
    )
    op.execute("CREATE INDEX entity_link_to_idx ON backlot.entity_link (to_id)")

    # The rules the database enforces itself (G2-03 section 4); the service checks the same first for readable codes.
    op.execute(
        """
        CREATE FUNCTION backlot.entity_label(entity_id uuid) RETURNS ltree LANGUAGE sql IMMUTABLE AS $$
            SELECT text2ltree(replace(entity_id::text, '-', ''))
        $$"""
    )
    op.execute(
        """
        CREATE FUNCTION backlot.check_entity() RETURNS trigger LANGUAGE plpgsql AS $$
        DECLARE
            parent backlot.production_entity%ROWTYPE;
            child_kind text;
            parent_kind text;
        BEGIN
            IF TG_OP = 'UPDATE' AND NEW.production_id <> OLD.production_id THEN
                RAISE EXCEPTION 'an entity cannot change its production' USING ERRCODE = 'BL103';
            END IF;
            SELECT kind INTO child_kind FROM backlot.entity_type WHERE id = NEW.entity_type_id;
            IF NEW.parent_id IS NULL THEN
                IF child_kind = 'structural' AND NOT EXISTS (
                    SELECT 1 FROM backlot.production_structure_rule
                    WHERE production_id = NEW.production_id AND parent_type_id IS NULL AND child_type_id = NEW.entity_type_id
                ) THEN
                    RAISE EXCEPTION 'the production structure does not allow this type at the root' USING ERRCODE = 'BL101';
                END IF;
                RETURN NEW;
            END IF;
            IF child_kind = 'asset' THEN
                RAISE EXCEPTION 'an asset has no parent' USING ERRCODE = 'BL104';
            END IF;
            SELECT * INTO parent FROM backlot.production_entity WHERE id = NEW.parent_id;
            IF parent.production_id <> NEW.production_id THEN
                RAISE EXCEPTION 'the parent is in another production' USING ERRCODE = 'BL103';
            END IF;
            SELECT kind INTO parent_kind FROM backlot.entity_type WHERE id = parent.entity_type_id;
            IF parent_kind = 'asset' THEN
                RAISE EXCEPTION 'an asset cannot be a parent' USING ERRCODE = 'BL104';
            END IF;
            IF NOT EXISTS (
                SELECT 1 FROM backlot.production_structure_rule
                WHERE production_id = NEW.production_id AND parent_type_id = parent.entity_type_id
                  AND child_type_id = NEW.entity_type_id
            ) THEN
                RAISE EXCEPTION 'the production structure does not allow this type under that parent' USING ERRCODE = 'BL101';
            END IF;
            IF TG_OP = 'UPDATE' AND NEW.parent_id IS DISTINCT FROM OLD.parent_id
               AND (NEW.parent_id = NEW.id OR parent.path <@ OLD.path) THEN
                RAISE EXCEPTION 'an entity cannot move under itself' USING ERRCODE = 'BL102';
            END IF;
            RETURN NEW;
        END $$"""
    )
    op.execute(
        "CREATE TRIGGER production_entity_check BEFORE INSERT OR UPDATE OF production_id, entity_type_id, parent_id "
        "ON backlot.production_entity FOR EACH ROW EXECUTE FUNCTION backlot.check_entity()"
    )
    # After the statement (so a whole subtree can move in one UPDATE): path = parent path || own label.
    op.execute(
        """
        CREATE FUNCTION backlot.check_entity_path() RETURNS trigger LANGUAGE plpgsql AS $$
        DECLARE parent_path ltree;
        BEGIN
            IF NEW.parent_id IS NULL THEN
                IF NEW.path <> backlot.entity_label(NEW.id) THEN
                    RAISE EXCEPTION 'path does not match the parent' USING ERRCODE = 'BL105';
                END IF;
            ELSE
                SELECT path INTO parent_path FROM backlot.production_entity WHERE id = NEW.parent_id;
                IF NEW.path <> parent_path || backlot.entity_label(NEW.id) THEN
                    RAISE EXCEPTION 'path does not match the parent' USING ERRCODE = 'BL105';
                END IF;
            END IF;
            RETURN NULL;
        END $$"""
    )
    op.execute(
        "CREATE TRIGGER production_entity_path_check AFTER INSERT OR UPDATE OF path, parent_id ON backlot.production_entity "
        "FOR EACH ROW EXECUTE FUNCTION backlot.check_entity_path()"
    )
    op.execute(
        """
        CREATE FUNCTION backlot.check_entity_timing() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM backlot.production_entity e JOIN backlot.entity_type t ON t.id = e.entity_type_id
                WHERE e.id = NEW.entity_id AND t.has_timing
            ) THEN
                RAISE EXCEPTION 'this entity type has no timing' USING ERRCODE = 'BL106';
            END IF;
            RETURN NEW;
        END $$"""
    )
    op.execute(
        "CREATE TRIGGER entity_timing_check BEFORE INSERT OR UPDATE OF entity_id ON backlot.entity_timing "
        "FOR EACH ROW EXECUTE FUNCTION backlot.check_entity_timing()"
    )
    op.execute(
        """
        CREATE FUNCTION backlot.check_entity_link() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            IF (SELECT production_id FROM backlot.production_entity WHERE id = NEW.from_id) <> NEW.production_id
               OR (SELECT production_id FROM backlot.production_entity WHERE id = NEW.to_id) <> NEW.production_id THEN
                RAISE EXCEPTION 'both ends of a link are in the production of the link' USING ERRCODE = 'BL103';
            END IF;
            RETURN NEW;
        END $$"""
    )
    op.execute(
        "CREATE TRIGGER entity_link_check BEFORE INSERT OR UPDATE OF production_id, from_id, to_id ON backlot.entity_link "
        "FOR EACH ROW EXECUTE FUNCTION backlot.check_entity_link()"
    )

    # Seed data: the types, one template per base production type, and the rules of every production that exists already.
    insert_type = sa.text(
        "INSERT INTO backlot.entity_type (id, code, label_key, kind, has_timing, created_by, updated_by) "
        "VALUES (CAST(:id AS uuid), :code, :label, :kind, :timing, :a, :a)"
    )
    for code in _STRUCTURAL:
        op.execute(insert_type.bindparams(id=_id("entity_type", code), code=code, label=f"entity_type.{code}", kind="structural", timing=code == "shot", a=_SEED_ACTOR))
    for code in _ASSET:
        op.execute(insert_type.bindparams(id=_id("entity_type", code), code=code, label=f"entity_type.{code}", kind="asset", timing=False, a=_SEED_ACTOR))
    for tcode, (ptype, rules) in _TEMPLATES.items():
        template_id = _id("structure_template", tcode)
        op.execute(
            sa.text(
                "INSERT INTO backlot.structure_template (id, code, label_key, production_type_id, created_by, updated_by) "
                "SELECT CAST(:id AS uuid), :code, :label, pt.id, :a, :a FROM backlot.production_type pt WHERE pt.code = :ptype"
            ).bindparams(id=template_id, code=tcode, label=f"structure_template.{tcode}", ptype=ptype, a=_SEED_ACTOR)
        )
        for parent, child in rules:
            op.execute(
                sa.text(
                    "INSERT INTO backlot.structure_template_rule (template_id, parent_type_id, child_type_id) "
                    "VALUES (CAST(:t AS uuid), CAST(:p AS uuid), CAST(:c AS uuid))"
                ).bindparams(t=template_id, p=_id("entity_type", parent) if parent else None, c=_id("entity_type", child))
            )
    op.execute(
        """
        INSERT INTO backlot.production_structure_rule (production_id, parent_type_id, child_type_id)
        SELECT p.id, r.parent_type_id, r.child_type_id
        FROM backlot.production p
        JOIN backlot.structure_template st ON st.production_type_id = p.type_id
        JOIN backlot.structure_template_rule r ON r.template_id = st.id"""
    )


def downgrade() -> None:
    for statement in (
        "DROP TABLE backlot.entity_link",
        "DROP TABLE backlot.entity_timing",
        "DROP TABLE backlot.production_entity",
        "DROP TABLE backlot.production_structure_rule",
        "DROP TABLE backlot.structure_template_rule",
        "DROP TABLE backlot.structure_template",
        "DROP TABLE backlot.entity_type",
        "DROP FUNCTION backlot.check_entity_link()",
        "DROP FUNCTION backlot.check_entity_timing()",
        "DROP FUNCTION backlot.check_entity_path()",
        "DROP FUNCTION backlot.check_entity()",
        "DROP FUNCTION backlot.entity_label(uuid)",
    ):
        op.execute(statement)
    # The ltree extension stays: it is a database-wide object other schemas may use.
