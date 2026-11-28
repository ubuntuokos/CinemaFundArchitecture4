"""Milestones, organizations and their roles in a production (G2-02 section 4).

Revision ID: 0002
Revises: 0001
"""

from __future__ import annotations

from alembic import op

revision = "0002"
down_revision = "0001"
branch_labels = None
depends_on = None

_COMMON = """
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    created_by  text        NOT NULL,
    updated_by  text        NOT NULL,
    row_version integer     NOT NULL DEFAULT 1 CHECK (row_version >= 1)
"""


def upgrade() -> None:
    op.execute(
        f"""
        CREATE TABLE backlot.milestone (
            id uuid PRIMARY KEY,
            production_id uuid NOT NULL REFERENCES backlot.production(id),
            name text NOT NULL CHECK (name <> '' AND length(name) <= 200),
            due_date date NOT NULL,
            done_at timestamptz,
            ordinal smallint NOT NULL CHECK (ordinal >= 1),
            {_COMMON}
        )"""
    )
    op.execute("CREATE INDEX milestone_production_idx ON backlot.milestone (production_id, due_date, ordinal)")
    op.execute(
        f"""
        CREATE TABLE backlot.organization (
            id uuid PRIMARY KEY,
            name text NOT NULL CHECK (name <> '' AND length(name) <= 200),
            legal_name text CHECK (length(legal_name) <= 300),
            country char(2) REFERENCES backlot.country(code),
            registration_no text CHECK (length(registration_no) <= 64),
            tax_no text CHECK (length(tax_no) <= 64),
            notes text CHECK (length(notes) <= 20000),
            retired_at timestamptz,
            {_COMMON}
        )"""
    )
    op.execute("CREATE INDEX organization_name_idx ON backlot.organization (lower(name))")
    op.execute(
        """
        CREATE TABLE backlot.production_organization (
            production_id uuid NOT NULL REFERENCES backlot.production(id),
            organization_id uuid NOT NULL REFERENCES backlot.organization(id),
            role text NOT NULL CHECK (role IN ('client','producer','coproducer','subcontractor','distributor','other')),
            created_at timestamptz NOT NULL DEFAULT now(),
            created_by text NOT NULL,
            PRIMARY KEY (production_id, organization_id, role)
        )"""
    )
    op.execute("CREATE INDEX production_organization_org_idx ON backlot.production_organization (organization_id)")


def downgrade() -> None:
    for statement in (
        "DROP TABLE backlot.production_organization",
        "DROP TABLE backlot.organization",
        "DROP TABLE backlot.milestone",
    ):
        op.execute(statement)
