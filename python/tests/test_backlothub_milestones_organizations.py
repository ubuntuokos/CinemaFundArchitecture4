"""BacklotHub step 2 (G2-02), PM-2: milestones, organizations and their roles in a production."""

from __future__ import annotations

import uuid
from typing import Any

import pytest

pytest.importorskip("psycopg")
pytest.importorskip("sqlalchemy")

import psycopg  # noqa: E402
from sqlalchemy import Engine  # noqa: E402

from cfa4_audit import verify  # noqa: E402
from cfa4_backlothub import commands  # noqa: E402
from cfa4_backlothub.errors import (  # noqa: E402
    ConflictStaleVersion,
    CountryUnknown,
    InvalidInput,
    MilestoneDone,
    MilestoneNotFound,
    OrganizationAlreadyLinked,
    OrganizationInUse,
    OrganizationNotFound,
    OrganizationNotLinked,
    OrganizationRetired,
    PermissionDenied,
)
from cfa4_backlothub.production import milestones as ms  # noqa: E402
from cfa4_backlothub.production import organizations as orgs  # noqa: E402
from cfa4_backlothub.production import service as svc  # noqa: E402
from backlot_helpers import ME, OTHER  # noqa: E402
from backlot_helpers import backlot_events as _backlot_events  # noqa: E402
from backlot_helpers import count as _count  # noqa: E402
from backlot_helpers import params as _params  # noqa: E402


def _production(engine: Engine) -> uuid.UUID:
    return uuid.UUID(svc.create_production(engine, _params(), ME)["id"])


def _actions(engine: Engine) -> list[str]:
    return [e["action"] for e in _backlot_events(engine)]


# ---------------------------------------------------------------- milestones


def test_milestones_are_added_in_order_updated_and_audited(engine: Engine) -> None:
    pid = _production(engine)
    a = ms.add_milestone(engine, pid, "Script lock", "2027-02-01", ME)
    b = ms.add_milestone(engine, pid, "Shooting wrap", "2027-01-15", ME)
    assert (a["ordinal"], b["ordinal"], a["done"]) == (1, 2, False) and a["production_id"] == str(pid)
    assert [m["name"] for m in ms.list_milestones(engine, pid, ME)] == ["Shooting wrap", "Script lock"], "by due date"
    done = ms.update_milestone(engine, uuid.UUID(a["id"]), 1, {"done": True, "name": "Script locked"}, ME)
    assert done["done"] and done["done_at"] is not None and done["row_version"] == 2
    ev = _backlot_events(engine)[-1]
    assert ev["action"] == "production.milestone.updated" and ev["scope"] == {"production_id": str(pid)}
    assert ev["change"]["name"] == {"old": "Script lock", "new": "Script locked"}
    assert ev["change"]["done"] == {"old": False, "new": True}
    reopened = ms.update_milestone(engine, uuid.UUID(a["id"]), 2, {"done": False}, ME)
    assert reopened["done"] is False and reopened["done_at"] is None


def test_milestone_rules(engine: Engine) -> None:
    pid = _production(engine)
    a = ms.add_milestone(engine, pid, "Rough cut", "2027-03-01", ME)
    mid = uuid.UUID(a["id"])
    with pytest.raises(ConflictStaleVersion):
        ms.update_milestone(engine, mid, 5, {"name": "x"}, ME)
    n = len(_backlot_events(engine))
    assert ms.update_milestone(engine, mid, 1, {"name": "Rough cut"}, ME)["row_version"] == 1 and len(_backlot_events(engine)) == n
    for bad in ({}, {"id": 1}, {"done": "yes"}, {"name": ""}, {"due_date": "soon"}, {"ordinal": 0}):
        with pytest.raises(InvalidInput):
            ms.update_milestone(engine, mid, 1, bad, ME)
    with pytest.raises(InvalidInput):
        ms.add_milestone(engine, pid, "", "2027-03-01", ME)
    with pytest.raises(InvalidInput):
        ms.add_milestone(engine, pid, "x", "31/12/2027", ME)
    with pytest.raises(MilestoneNotFound):
        ms.update_milestone(engine, uuid.uuid4(), 1, {"name": "x"}, ME)
    ms.update_milestone(engine, mid, 1, {"done": True}, ME)
    with pytest.raises(MilestoneDone):
        ms.remove_milestone(engine, mid, None, ME)
    ms.update_milestone(engine, mid, 2, {"done": False}, ME)
    removed = ms.remove_milestone(engine, mid, 3, ME)
    assert removed["name"] == "Rough cut" and ms.list_milestones(engine, pid, ME) == []
    ev = _backlot_events(engine)[-1]
    assert ev["action"] == "production.milestone.removed" and ev["change"]["name"] == {"old": "Rough cut", "new": None}


def test_milestones_need_a_write_role(engine: Engine, info: str) -> None:
    pid = _production(engine)
    mid = uuid.UUID(ms.add_milestone(engine, pid, "Grade", "2027-05-01", ME)["id"])
    for call in (
        lambda: ms.add_milestone(engine, pid, "x", "2027-01-01", OTHER),
        lambda: ms.update_milestone(engine, mid, 1, {"name": "x"}, OTHER),
        lambda: ms.remove_milestone(engine, mid, None, OTHER),
        lambda: ms.list_milestones(engine, pid, OTHER),
    ):
        with pytest.raises(PermissionDenied):
            call()
    with psycopg.connect(info) as conn:
        conn.execute(
            "INSERT INTO backlot.production_member (production_id, actor_kind, actor_ref, role, created_by) "
            "VALUES (%s, 'os_user', %s, 'viewer', 'test')",
            (pid, OTHER.ref),
        )
    assert len(ms.list_milestones(engine, pid, OTHER)) == 1, "a viewer reads"
    with pytest.raises(PermissionDenied):
        ms.add_milestone(engine, pid, "x", "2027-01-01", OTHER)


# ---------------------------------------------------------------- organizations


def _org(engine: Engine, **changes: Any) -> dict[str, Any]:
    return orgs.create_organization(engine, {"name": "Acme Films", "legal_name": "Acme Films Kft.", "country": "hu", **changes}, ME)


def test_an_organization_is_created_updated_and_listed(engine: Engine) -> None:
    o = _org(engine, tax_no="12345678-1-42")
    assert o["country"] == "HU" and o["retired"] is False and o["row_version"] == 1
    ev = _backlot_events(engine)[-1]
    assert ev["action"] == "organization.created" and ev["scope"] == {} and ev["subject_id"] == o["id"]
    oid = uuid.UUID(o["id"])
    after = orgs.update_organization(engine, oid, 1, {"name": "Acme Pictures", "notes": "client"}, ME)
    assert after["name"] == "Acme Pictures" and after["row_version"] == 2
    assert _backlot_events(engine)[-1]["change"]["name"] == {"old": "Acme Films", "new": "Acme Pictures"}
    with pytest.raises(ConflictStaleVersion):
        orgs.update_organization(engine, oid, 1, {"name": "x"}, ME)
    _org(engine, name="Zeta 100%_Studio")
    assert [x["name"] for x in orgs.list_organizations(engine)] == ["Acme Pictures", "Zeta 100%_Studio"]
    assert [x["name"] for x in orgs.list_organizations(engine, query="pict")] == ["Acme Pictures"]
    assert [x["name"] for x in orgs.list_organizations(engine, query="100%_")] == ["Zeta 100%_Studio"], "wildcards are literal"
    assert len(orgs.list_organizations(engine, limit=1)) == 1


def test_organization_input_errors(engine: Engine) -> None:
    for bad in ({}, {"name": ""}, {"name": "x", "country": "ZZ"}, {"name": "x", "tax_no": "1" * 65}, {"name": "x", "oops": 1}):
        with pytest.raises((InvalidInput, CountryUnknown)):
            orgs.create_organization(engine, bad, ME)
    assert _count(engine, "organization") == 0
    o = _org(engine)
    with pytest.raises(OrganizationNotFound):
        orgs.update_organization(engine, uuid.uuid4(), 1, {"name": "x"}, ME)
    with pytest.raises(InvalidInput):
        orgs.update_organization(engine, uuid.UUID(o["id"]), 1, {"retired": "yes"}, ME)
    with pytest.raises(InvalidInput):
        orgs.list_organizations(engine, limit=501)


def test_link_unlink_and_the_retire_rules(engine: Engine) -> None:
    pid = _production(engine)
    o = _org(engine)
    oid = uuid.UUID(o["id"])
    link = orgs.link_organization(engine, pid, oid, "client", ME)
    assert link["role"] == "client"
    orgs.link_organization(engine, pid, oid, "producer", ME)
    with pytest.raises(OrganizationAlreadyLinked):
        orgs.link_organization(engine, pid, oid, "client", ME)
    with pytest.raises(InvalidInput):
        orgs.link_organization(engine, pid, oid, "boss", ME)
    assert [(x["role"], x["name"]) for x in orgs.list_linked(engine, pid, ME)] == [("client", "Acme Films"), ("producer", "Acme Films")]
    ev = _backlot_events(engine)[-2]
    assert ev["action"] == "production.organization.linked" and ev["scope"] == {"production_id": str(pid)}
    with pytest.raises(OrganizationInUse, match="FILM-1"):
        orgs.update_organization(engine, oid, 1, {"retired": True}, ME)
    svc.set_status(engine, pid, "closed", None, ME)  # a closed production no longer needs it
    retired = orgs.update_organization(engine, oid, 1, {"retired": True}, ME)
    assert retired["retired"] and retired["retired_at"] is not None
    assert orgs.list_organizations(engine) == [] and len(orgs.list_organizations(engine, include_retired=True)) == 1
    other = uuid.UUID(svc.create_production(engine, _params(code="FILM-2"), ME)["id"])
    with pytest.raises(OrganizationRetired):
        orgs.link_organization(engine, other, oid, "client", ME)
    assert orgs.update_organization(engine, oid, 2, {"retired": False}, ME)["retired"] is False
    orgs.unlink_organization(engine, pid, oid, "client", ME)
    with pytest.raises(OrganizationNotLinked):
        orgs.unlink_organization(engine, pid, oid, "client", ME)
    assert _actions(engine)[-1] == "production.organization.unlinked"
    with pytest.raises(OrganizationNotFound):
        orgs.link_organization(engine, pid, uuid.uuid4(), "client", ME)


def test_linking_needs_a_write_role_and_everything_is_on_the_chain(engine: Engine) -> None:
    pid = _production(engine)
    oid = uuid.UUID(_org(engine)["id"])
    with pytest.raises(PermissionDenied):
        orgs.link_organization(engine, pid, oid, "client", OTHER)
    with pytest.raises(PermissionDenied):
        orgs.list_linked(engine, pid, OTHER)
    with engine.connect() as conn:
        assert verify(conn.connection.driver_connection).ok  # type: ignore[arg-type]


# ---------------------------------------------------------------- commands


def test_the_new_commands_run_through_the_registry(engine: Engine) -> None:
    pid = commands.run_command(engine, "backlot.production.create", _params(), ME)["result"]["id"]
    m = commands.run_command(engine, "backlot.production.milestone.add", {"production_id": pid, "name": "Wrap", "due_date": "2027-06-01"}, ME)
    assert m["ok"]
    upd = commands.run_command(engine, "backlot.production.milestone.update", {"id": m["result"]["id"], "row_version": 1, "done": "true"}, ME)
    assert upd["ok"] and upd["result"]["done"]
    assert commands.run_command(engine, "backlot.production.milestone.list", {"production_id": pid}, ME)["result"][0]["name"] == "Wrap"
    assert commands.run_command(engine, "backlot.production.milestone.remove", {"id": m["result"]["id"]}, ME)["code"] == "MILESTONE_DONE"
    o = commands.run_command(engine, "backlot.organization.create", {"name": "Acme"}, ME)
    assert o["ok"]
    link = commands.run_command(engine, "backlot.production.organization.link", {"production_id": pid, "organization_id": o["result"]["id"], "role": "client"}, ME)
    assert link["ok"]
    assert commands.run_command(engine, "backlot.organization.update", {"id": o["result"]["id"], "row_version": 1, "retired": True}, ME)["code"] == "ORGANIZATION_IN_USE"
    assert commands.run_command(engine, "backlot.organization.list", {"query": "acm"}, ME)["result"][0]["name"] == "Acme"
    assert len(commands.run_command(engine, "backlot.production.organization.list", {"production_id": pid}, ME)["result"]) == 1
    unlink = commands.run_command(engine, "backlot.production.organization.unlink", {"production_id": pid, "organization_id": o["result"]["id"], "role": "client"}, ME)
    assert unlink["ok"]
