"""CycloneDX SBOM of the Rust code in a CFA4 release package (F1-01, issue #220).

Reads `cargo metadata` output and `Cargo.lock`, and writes a CycloneDX 1.6 JSON document of every
package compiled into `cfa4` and `cfa4-cli`: normal dependencies only (no dev- or build-only
crates), with version, package URL, licence expression, the crates.io SHA-256 from `Cargo.lock`,
and the dependency graph. The output is deterministic: the serial number is derived from the
content, and the timestamp comes from SOURCE_DATE_EPOCH when it is set.

    cargo metadata --format-version 1 --locked --features gstreamer,ffmpeg \
        --filter-platform x86_64-unknown-linux-gnu > target/metadata.json
    python -m cfa4_devtools.sbom target/metadata.json --lock Cargo.lock \
        --version 0.1.0 -o target/cfa4.cdx.json

This lists what CFA4 builds from source. Libraries a package loads from the system (its Depends
and Recommends) are not components; `cargo deny check licenses` remains the enforcing gate.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tomllib
import uuid
from collections import deque
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from urllib.parse import quote

SPEC_VERSION = "1.6"
ROOTS = ("cfa4", "cfa4-cli")
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
# A fixed namespace, so the same content always gets the same serial number.
SERIAL_NAMESPACE = uuid.UUID("6f6b0f5e-2b8e-4c0e-9a51-3c1f4d7e2a10")
MAX_PACKAGES = 100_000


class SbomError(Exception):
    """The input cannot be turned into an SBOM."""


def purl(package: dict[str, Any]) -> str:
    """The package URL of a Cargo package."""
    name, version = (quote(str(package.get(k, "")), safe="-._~") for k in ("name", "version"))
    base = f"pkg:cargo/{name}@{version}"
    source = package.get("source")
    if source and source != CRATES_IO:
        # Not from crates.io (git or another registry): say where it came from.
        return f"{base}?repository_url={source.split('+', 1)[-1]}"
    return base


def license_expression(package: dict[str, Any]) -> str | None:
    """The package's SPDX expression; Cargo's legacy "A/B" form means "A OR B"."""
    expression = (package.get("license") or "").strip()
    if not expression:
        return None
    return " OR ".join(part.strip() for part in expression.split("/")) if "/" in expression else expression


def lock_checksums(lock_text: str) -> dict[tuple[str, str], str]:
    """Map (name, version) to the crates.io SHA-256 recorded in Cargo.lock."""
    try:
        lock = tomllib.loads(lock_text)
    except tomllib.TOMLDecodeError as exc:
        raise SbomError(f"Cargo.lock is not valid TOML: {exc}") from exc
    sums: dict[tuple[str, str], str] = {}
    for entry in lock.get("package", []):
        if isinstance(entry, dict) and isinstance(entry.get("checksum"), str):
            sums[(str(entry.get("name")), str(entry.get("version")))] = entry["checksum"]
    return sums


def reachable(metadata: dict[str, Any], roots: tuple[str, ...]) -> tuple[list[str], dict[str, list[str]]]:
    """Package ids the roots compile in, and each one's normal dependencies (both sorted)."""
    packages = {p["id"]: p for p in metadata.get("packages", []) if isinstance(p, dict) and "id" in p}
    workspace = set(metadata.get("workspace_members", []))
    nodes = {n["id"]: n for n in (metadata.get("resolve") or {}).get("nodes", []) if isinstance(n, dict) and "id" in n}
    start = sorted(i for i in workspace if packages.get(i, {}).get("name") in roots)
    found = {packages[i]["name"] for i in start}
    missing = [r for r in roots if r not in found]
    if missing:
        raise SbomError(f"workspace has no package named {', '.join(missing)}")

    graph: dict[str, list[str]] = {}
    queue = deque(start)
    while queue:
        current = queue.popleft()
        if current in graph:
            continue
        if len(graph) >= MAX_PACKAGES:
            raise SbomError(f"more than {MAX_PACKAGES} packages; refusing to continue")
        deps = set()
        for dep in nodes.get(current, {}).get("deps", []):
            if not isinstance(dep, dict) or dep.get("pkg") not in packages:
                continue
            kinds = [k for k in dep.get("dep_kinds") or [] if isinstance(k, dict)]
            # kind None is a normal dependency; "dev" and "build" never reach the binaries.
            if any(k.get("kind") is None for k in kinds):
                deps.add(dep["pkg"])
        graph[current] = sorted(deps)
        queue.extend(d for d in graph[current] if d not in graph)
    return sorted(graph), graph


def build_sbom(
    metadata: dict[str, Any],
    checksums: dict[tuple[str, str], str],
    version: str,
    timestamp: datetime | None = None,
) -> dict[str, Any]:
    packages = {p["id"]: p for p in metadata.get("packages", []) if isinstance(p, dict) and "id" in p}
    ids, graph = reachable(metadata, ROOTS)
    workspace = set(metadata.get("workspace_members", []))
    refs = {i: purl(packages[i]) for i in ids}

    components = []
    for package_id in ids:
        package = packages[package_id]
        component: dict[str, Any] = {
            "type": "application" if package.get("name") in ROOTS else "library",
            "bom-ref": refs[package_id],
            "name": package.get("name", ""),
            "version": package.get("version", ""),
            "purl": refs[package_id],
            "scope": "required",
        }
        if expression := license_expression(package):
            component["licenses"] = [{"expression": expression}]
        checksum = checksums.get((package.get("name", ""), package.get("version", "")))
        if checksum and package.get("source") == CRATES_IO:
            component["hashes"] = [{"alg": "SHA-256", "content": checksum}]
        if package_id in workspace:
            component["supplier"] = {"name": "CFA4 contributors"}
        elif not package.get("source"):
            # A path dependency outside the workspace: vendored in this repository (vendor/).
            component["properties"] = [{"name": "cfa4:vendored", "value": "true"}]
        if package_id not in workspace and (repository := package.get("repository")):
            component["externalReferences"] = [{"type": "vcs", "url": repository}]
        components.append(component)
    components.sort(key=lambda c: c["bom-ref"])

    product_ref = f"pkg:generic/cfa4@{version}"
    dependencies = [{"ref": product_ref, "dependsOn": sorted(refs[i] for i in ids if packages[i].get("name") in ROOTS)}]
    dependencies += [{"ref": refs[i], "dependsOn": sorted(refs[d] for d in graph[i])} for i in ids]
    dependencies.sort(key=lambda d: d["ref"])

    body = {"components": components, "dependencies": dependencies}
    serial = uuid.uuid5(SERIAL_NAMESPACE, json.dumps([version, body], sort_keys=True))
    metadata_block: dict[str, Any] = {
        "component": {
            "type": "application",
            "bom-ref": product_ref,
            "name": "cfa4",
            "version": version,
            "licenses": [{"expression": "GPL-3.0-or-later"}],
            "supplier": {"name": "CFA4 contributors"},
        },
        "tools": {"components": [{"type": "application", "name": "cfa4_devtools.sbom"}]},
        "properties": [{"name": "cfa4:target", "value": "x86_64-unknown-linux-gnu"}],
    }
    if timestamp is not None:
        metadata_block = {"timestamp": timestamp.strftime("%Y-%m-%dT%H:%M:%SZ"), **metadata_block}
    return {
        "bomFormat": "CycloneDX",
        "specVersion": SPEC_VERSION,
        "serialNumber": f"urn:uuid:{serial}",
        "version": 1,
        "metadata": metadata_block,
        **body,
    }


def source_date_epoch() -> datetime | None:
    """SOURCE_DATE_EPOCH as a UTC time, or None when unset or not a sane number."""
    raw = os.environ.get("SOURCE_DATE_EPOCH", "").strip()
    if not raw.isdigit():
        return None
    try:
        return datetime.fromtimestamp(int(raw), tz=UTC)
    except (OverflowError, OSError, ValueError):
        return None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("metadata", type=Path, help="cargo metadata JSON (see module docstring)")
    parser.add_argument("--lock", type=Path, default=Path("Cargo.lock"), help="Cargo.lock for checksums")
    parser.add_argument("--version", required=True, help="the release version the SBOM describes")
    parser.add_argument("-o", "--output", type=Path, help="write the SBOM here instead of stdout")
    args = parser.parse_args(argv)
    try:
        metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
        if not isinstance(metadata, dict):
            raise SbomError("metadata is not a JSON object")
        sbom = build_sbom(
            metadata, lock_checksums(args.lock.read_text(encoding="utf-8")), args.version, source_date_epoch()
        )
    except (OSError, ValueError, SbomError) as exc:
        print(f"sbom: {exc}", file=sys.stderr)
        return 2
    text = json.dumps(sbom, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
