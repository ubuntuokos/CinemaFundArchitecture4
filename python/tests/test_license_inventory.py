from __future__ import annotations

import json
from pathlib import Path

from cfa4_devtools.license_inventory import build_inventory, main, review_reasons


def _metadata() -> dict:
    def pkg(name: str, license: str | None, source: str | None = "registry+https://github.com/rust-lang/crates.io-index", links: str | None = None) -> dict:
        return {"id": f"{name}-id", "name": name, "version": "1.0.0", "license": license, "source": source, "links": links, "license_file": None if license else "LICENSE"}

    packages = [
        pkg("cfa4-engine", "GPL-3.0-or-later", source=None),
        pkg("serde", "MIT OR Apache-2.0"),
        pkg("symphonia", "MPL-2.0"),
        pkg("self_cell", "Apache-2.0 OR GPL-2.0-only"),
        pkg("alsa-sys", "MIT", links="alsa"),
        pkg("mystery", None),
        pkg("glib-sys", "MIT"),
        pkg("not-resolved", "MIT"),
    ]
    resolved = [p["id"] for p in packages if p["name"] != "not-resolved"]
    return {
        "packages": packages,
        "workspace_members": ["cfa4-engine-id"],
        "resolve": {"nodes": [{"id": i, "deps": []} for i in resolved]},
    }


def test_review_reasons_pick_the_most_specific_term() -> None:
    assert review_reasons("MIT OR Apache-2.0") == []
    assert [r.split(":")[0] for r in review_reasons("LGPL-2.1-or-later")] == ["LGPL"]
    assert [r.split(":")[0] for r in review_reasons("Apache-2.0 OR GPL-2.0-only")] == ["GPL-2.0-only"]
    assert [r.split(":")[0] for r in review_reasons("AGPL-3.0-only")] == ["AGPL"]


def test_inventory_lists_resolved_third_party_packages_only() -> None:
    inventory = build_inventory(_metadata())
    names = [p["name"] for p in inventory["packages"]]
    assert names == ["alsa-sys", "glib-sys", "mystery", "self_cell", "serde", "symphonia"]
    summary = inventory["summary"]
    assert summary["workspace_licenses"] == {"GPL-3.0-or-later": 1}
    assert summary["third_party_packages"] == 6
    assert summary["needs_review"] == 3
    assert summary["native"] == ["alsa-sys", "glib-sys"]
    mystery = next(p for p in inventory["packages"] if p["name"] == "mystery")
    assert mystery["review"] == ["no SPDX licence field"]
    assert mystery["license_file"] == "LICENSE"


def test_hostile_metadata_does_not_crash() -> None:
    assert build_inventory({})["summary"]["third_party_packages"] == 0
    junk = {"packages": [1, {"no": "id"}], "resolve": {"nodes": [None, {"id": "missing"}]}, "workspace_members": []}
    assert build_inventory(junk)["packages"] == []


def test_cli_writes_deterministic_output(tmp_path: Path) -> None:
    source = tmp_path / "metadata.json"
    source.write_text(json.dumps(_metadata()), encoding="utf-8")
    first, second = tmp_path / "a.json", tmp_path / "b.json"
    assert main([str(source), "-o", str(first)]) == 0
    assert main([str(source), "-o", str(second)]) == 0
    assert first.read_bytes() == second.read_bytes()
    bad = tmp_path / "bad.json"
    bad.write_text("[1, 2]", encoding="utf-8")
    assert main([str(bad)]) == 2
    assert main([str(tmp_path / "absent.json")]) == 2
