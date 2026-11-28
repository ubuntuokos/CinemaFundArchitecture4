"""Parity test for the agent rule register (CFA4-DEV-AI-ORCHESTRATION-001, F2 / WP2, TC-01).

`docs/development/ai-orchestration/rules.toml` lists every agent rule by reference (source file and
anchor, never the rule text). This test fails when the register and the repository drift apart:
a missing source or anchor, a duplicate id, a mechanism that does not exist, a hook that is not wired,
a CLAUDE.md non-negotiable that no rule covers, or a clash that is still open.

It proves the references are real, not that a "text" rule is obeyed. Only "ci" and "hook" rules are
enforced, and "partial" ones only in part; the register says so in its own words.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
REGISTER = ROOT / "docs/development/ai-orchestration/rules.toml"

LEVELS = {"ci", "hook", "text"}
COVERAGE = {"full", "partial"}
STATUSES = {"active", "conflict"}
ID_FORMAT = re.compile(r"^CFA4-RULE-\d{3}$")
REQUIRED = ("id", "title", "source", "anchor", "level", "scope", "pi", "status")


def _load() -> dict[str, Any]:
    with REGISTER.open("rb") as handle:
        return tomllib.load(handle)


def _rules() -> list[dict[str, Any]]:
    rules = _load().get("rule", [])
    assert isinstance(rules, list) and rules, "the register holds no [[rule]]"
    return rules


def _norm(text: str) -> str:
    """Collapse whitespace so an anchor still matches where the source wraps the line."""
    return " ".join(text.split())


def _claude_md_points() -> list[str]:
    """The bold lead-ins of the bullets in CLAUDE.md "Non-negotiables", without markdown."""
    lines = (ROOT / "CLAUDE.md").read_text(encoding="utf-8").splitlines()
    start = lines.index("## Non-negotiables")
    points: list[str] = []
    for line in lines[start + 1 :]:
        if line.startswith("## "):
            break
        found = re.match(r"^- \*\*(.+?)\*\*", line)
        if found:
            lead = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", found.group(1)).replace("`", "")
            points.append(lead.rstrip(": ").strip())
    return points


def _covers(entry: str, point: str) -> bool:
    return point == entry or point.startswith(entry)


def test_register_loads_and_has_a_schema_version() -> None:
    assert _load().get("schema_version") == 1


def test_every_rule_has_the_required_fields_and_valid_values() -> None:
    for rule in _rules():
        rid = rule.get("id", "<no id>")
        for field in REQUIRED:
            assert field in rule, f"{rid}: missing field {field}"
        assert ID_FORMAT.match(rule["id"]), f"{rid}: id must look like CFA4-RULE-001"
        assert rule["level"] in LEVELS, f"{rid}: level {rule['level']!r}"
        assert rule["status"] in STATUSES, f"{rid}: status {rule['status']!r}"
        assert rule["scope"] == ["claude-code"], f"{rid}: scope is claude-code only (Pi is paused)"
        assert rule["pi"] == [], f"{rid}: the pi field is reserved and stays empty"
        assert rule["title"].strip(), f"{rid}: empty title"


def test_ids_are_unique() -> None:
    ids = [rule["id"] for rule in _rules()]
    assert len(ids) == len(set(ids)), sorted({i for i in ids if ids.count(i) > 1})


def test_every_source_exists_and_contains_its_anchor() -> None:
    for rule in _rules():
        source = ROOT / rule["source"]
        assert source.is_file(), f"{rule['id']}: source {rule['source']} does not exist"
        text = _norm(source.read_text(encoding="utf-8"))
        assert _norm(rule["anchor"]) in text, f"{rule['id']}: anchor not found in {rule['source']}"


def test_enforcement_mechanisms_exist() -> None:
    for rule in _rules():
        rid, level = rule["id"], rule["level"]
        mechanisms = rule.get("enforced_by", [])
        if level in {"ci", "hook"}:
            assert mechanisms, f"{rid}: level {level} needs enforced_by"
            assert rule.get("coverage") in COVERAGE, f"{rid}: level {level} needs coverage full|partial"
        for mechanism in mechanisms:
            assert (ROOT / mechanism).exists(), f"{rid}: enforced_by {mechanism} does not exist"


def test_hook_rules_are_wired_in_claude_settings() -> None:
    settings = (ROOT / ".claude/settings.json").read_text(encoding="utf-8")
    for rule in _rules():
        if rule["level"] != "hook":
            continue
        wired = [m for m in rule["enforced_by"] if m.startswith("scripts/") and Path(m).name in settings]
        assert wired, f"{rule['id']}: no enforced_by script is wired as a hook in .claude/settings.json"


def test_no_conflict_stays_open() -> None:
    ids = {rule["id"] for rule in _rules()}
    for rule in _rules():
        rid = rule["id"]
        assert rule["status"] != "conflict", f"{rid}: an open conflict must be settled before merge"
        for other in rule.get("conflicts_with", []):
            assert other in ids, f"{rid}: conflicts_with {other} is not a rule"
            assert rule.get("resolution", "").strip(), f"{rid}: a conflict needs a resolution"


def test_claude_md_non_negotiables_are_all_covered() -> None:
    data = _load()
    points = _claude_md_points()
    assert len(points) >= 10, f"expected the CLAUDE.md bullets, found {points}"
    covered = [entry for rule in _rules() for entry in rule.get("claude_md", [])]
    exempt = data.get("claude_md_exempt", [])
    for item in exempt:
        assert item.get("reason", "").strip(), f"exempt point {item.get('point')!r} needs a reason"
    exempt_points = [item["point"] for item in exempt]
    for point in points:
        ok = any(_covers(entry, point) for entry in covered + exempt_points)
        assert ok, f"CLAUDE.md non-negotiable {point!r} is covered by no rule and not exempt"


def test_claude_md_references_in_the_register_are_real() -> None:
    data = _load()
    points = _claude_md_points()
    entries = [entry for rule in _rules() for entry in rule.get("claude_md", [])]
    entries += [item["point"] for item in data.get("claude_md_exempt", [])]
    for entry in entries:
        assert any(_covers(entry, point) for point in points), f"{entry!r} names no CLAUDE.md bullet"
