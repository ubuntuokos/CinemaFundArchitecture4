"""CycloneDX SBOM of the release binaries (#220, F1-01)."""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path

import pytest
from cfa4_devtools import sbom

CRATES = sbom.CRATES_IO


def package(name: str, version: str = "1.0.0", source: str | None = CRATES, license: str | None = "MIT") -> dict:
    return {
        "id": f"{name} {version}",
        "name": name,
        "version": version,
        "source": source,
        "license": license,
        "repository": f"https://example.org/{name}" if source else None,
    }


def dep(name: str, *kinds: str | None, version: str = "1.0.0") -> dict:
    return {"pkg": f"{name} {version}", "dep_kinds": [{"kind": k, "target": None} for k in kinds]}


def metadata() -> dict:
    packages = [
        package("cfa4", "0.1.0", None, "GPL-3.0-or-later"),
        package("cfa4-cli", "0.1.0", None, "GPL-3.0-or-later"),
        package("cfa4-engine", "0.1.0", None, "GPL-3.0-or-later"),
        package("cfa4-xtask", "0.1.0", None, "GPL-3.0-or-later"),
        package("serde", license="MIT/Apache-2.0"),
        package("rsmpeg", "0.18.1+ffmpeg.9.0", None, "MIT"),
        package("proptest"),
        package("cc"),
        package("unused"),
    ]
    nodes = [
        {"id": "cfa4 0.1.0", "deps": [dep("cfa4-engine", None, version="0.1.0"), dep("proptest", "dev")]},
        {"id": "cfa4-cli 0.1.0", "deps": [dep("cfa4-engine", None, version="0.1.0")]},
        {
            "id": "cfa4-engine 0.1.0",
            "deps": [dep("serde", None, "dev"), dep("cc", "build"), dep("rsmpeg", None, version="0.18.1+ffmpeg.9.0")],
        },
        {"id": "cfa4-xtask 0.1.0", "deps": [dep("unused", None)]},
    ]
    members = ["cfa4 0.1.0", "cfa4-cli 0.1.0", "cfa4-engine 0.1.0", "cfa4-xtask 0.1.0"]
    return {"packages": packages, "workspace_members": members, "resolve": {"nodes": nodes}}


CHECKSUMS = {("serde", "1.0.0"): "ab" * 32, ("rsmpeg", "0.18.1+ffmpeg.9.0"): "cd" * 32}


def by_name(doc: dict) -> dict[str, dict]:
    return {c["name"]: c for c in doc["components"]}


def test_only_crates_compiled_into_the_binaries() -> None:
    names = set(by_name(sbom.build_sbom(metadata(), CHECKSUMS, "0.1.0")))
    # dev (proptest), build-only (cc) and crates of other workspace binaries (unused) are out
    assert names == {"cfa4", "cfa4-cli", "cfa4-engine", "serde", "rsmpeg"}


def test_component_fields() -> None:
    components = by_name(sbom.build_sbom(metadata(), CHECKSUMS, "0.1.0"))
    serde = components["serde"]
    assert serde["purl"] == "pkg:cargo/serde@1.0.0"
    assert serde["licenses"] == [{"expression": "MIT OR Apache-2.0"}]
    assert serde["hashes"] == [{"alg": "SHA-256", "content": "ab" * 32}]
    assert serde["externalReferences"] == [{"type": "vcs", "url": "https://example.org/serde"}]
    # a path crate gets no registry hash, even when the lock has one for that name
    rsmpeg = components["rsmpeg"]
    assert rsmpeg["purl"] == "pkg:cargo/rsmpeg@0.18.1%2Bffmpeg.9.0"
    assert "hashes" not in rsmpeg
    assert rsmpeg["properties"] == [{"name": "cfa4:vendored", "value": "true"}]
    assert components["cfa4"]["type"] == "application"
    assert components["cfa4-engine"]["supplier"] == {"name": "CFA4 contributors"}


def test_dependency_graph() -> None:
    doc = sbom.build_sbom(metadata(), CHECKSUMS, "0.1.0")
    graph = {d["ref"]: d["dependsOn"] for d in doc["dependencies"]}
    assert graph["pkg:generic/cfa4@0.1.0"] == ["pkg:cargo/cfa4-cli@0.1.0", "pkg:cargo/cfa4@0.1.0"]
    assert graph["pkg:cargo/cfa4-engine@0.1.0"] == ["pkg:cargo/rsmpeg@0.18.1%2Bffmpeg.9.0", "pkg:cargo/serde@1.0.0"]
    refs = {c["bom-ref"] for c in doc["components"]}
    assert all(set(deps) <= refs for deps in graph.values())


def test_deterministic_and_timestamped_only_on_request() -> None:
    meta = metadata()
    first = sbom.build_sbom(meta, CHECKSUMS, "0.1.0")
    meta["packages"].reverse()
    meta["resolve"]["nodes"].reverse()
    assert sbom.build_sbom(meta, CHECKSUMS, "0.1.0") == first
    assert "timestamp" not in first["metadata"]
    assert sbom.build_sbom(meta, CHECKSUMS, "0.2.0")["serialNumber"] != first["serialNumber"]
    stamped = sbom.build_sbom(meta, CHECKSUMS, "0.1.0", datetime(2026, 11, 8, tzinfo=UTC))
    assert stamped["metadata"]["timestamp"] == "2026-11-08T00:00:00Z"


def test_missing_root_and_junk_input() -> None:
    meta = metadata()
    meta["workspace_members"].remove("cfa4-cli 0.1.0")
    with pytest.raises(sbom.SbomError, match="cfa4-cli"):
        sbom.build_sbom(meta, CHECKSUMS, "0.1.0")
    junk = metadata()
    junk["resolve"]["nodes"][0]["deps"] += ["x", {"pkg": "nope 1"}, {"pkg": "cc 1.0.0", "dep_kinds": [1, None]}]
    junk["packages"].append("not a package")
    assert "cc" not in by_name(sbom.build_sbom(junk, CHECKSUMS, "0.1.0"))
    with pytest.raises(sbom.SbomError):
        sbom.lock_checksums("[[package]\n")


def test_lock_checksums() -> None:
    lock = '[[package]]\nname = "serde"\nversion = "1.0.0"\nchecksum = "ff"\n\n[[package]]\nname = "cfa4"\nversion = "0.1.0"\n'
    assert sbom.lock_checksums(lock) == {("serde", "1.0.0"): "ff"}


def test_cli_writes_a_cyclonedx_document(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    (tmp_path / "metadata.json").write_text(json.dumps(metadata()), encoding="utf-8")
    (tmp_path / "Cargo.lock").write_text('[[package]]\nname = "serde"\nversion = "1.0.0"\nchecksum = "ff"\n')
    monkeypatch.setenv("SOURCE_DATE_EPOCH", "0")
    out = tmp_path / "cfa4.cdx.json"
    args = [str(tmp_path / "metadata.json"), "--lock", str(tmp_path / "Cargo.lock"), "--version", "0.1.0"]
    assert sbom.main([*args, "-o", str(out)]) == 0
    doc = json.loads(out.read_text(encoding="utf-8"))
    assert (doc["bomFormat"], doc["specVersion"]) == ("CycloneDX", "1.6")
    assert doc["metadata"]["timestamp"] == "1970-01-01T00:00:00Z"
    assert sbom.main([str(tmp_path / "missing.json"), "--version", "0.1.0"]) == 2
