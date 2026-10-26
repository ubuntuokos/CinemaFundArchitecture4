"""Rust dependency licence inventory for the CFA4 Linux AMD64 build (F1-01, issue #6).

Reads `cargo metadata` output and writes a deterministic JSON inventory of every
third-party package resolved for `x86_64-unknown-linux-gnu` with all features on.
Packages whose licence needs a human decision are flagged with a reason.

This is an inventory, not a legal review or a distribution licence: `cargo deny
check licenses` (deny.toml) remains the enforcing gate.

    cargo metadata --format-version 1 --locked --all-features \
        --filter-platform x86_64-unknown-linux-gnu > target/metadata.json
    python -m cfa4_devtools.license_inventory target/metadata.json \
        -o docs/audit/f1-01-rust-inventory.json
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any

SCHEMA = "cfa4-license-inventory/1"
TARGET = "x86_64-unknown-linux-gnu"

# Licence identifiers that are not plain permissive code licences. Each needs a
# documented reason in docs/audit, even when deny.toml allows it.
REVIEW_TERMS: dict[str, str] = {
    "MPL-2.0": "weak copyleft (file-level); keep source offer for modified files",
    "LGPL": "weak copyleft",
    "GPL-2.0-only": "GPL-2.0-only is incompatible with GPL-3.0; must be an alternative, never the chosen licence",
    "AGPL": "not allowed (docs/licensing.md)",
    "GPL": "copyleft",
    "OFL-1.1": "font licence (bundled data)",
    "Ubuntu-font-1.0": "font licence (bundled data)",
    "Unicode-3.0": "Unicode data licence",
    "CC-BY": "Creative Commons data licence",
}


def review_reasons(expression: str) -> list[str]:
    """Return the review reasons that apply to an SPDX licence expression."""
    reasons: list[str] = []
    matched: set[str] = set()
    for term, reason in REVIEW_TERMS.items():
        if term in expression and not any(term in longer for longer in matched):
            reasons.append(f"{term}: {reason}")
            matched.add(term)
    return reasons


def build_inventory(metadata: dict[str, Any]) -> dict[str, Any]:
    packages = {p["id"]: p for p in metadata.get("packages", []) if isinstance(p, dict) and "id" in p}
    workspace = set(metadata.get("workspace_members", []))
    resolve = metadata.get("resolve") or {}
    nodes = [n for n in resolve.get("nodes", []) if isinstance(n, dict)]

    entries: list[dict[str, Any]] = []
    for node in nodes:
        package = packages.get(node.get("id"))
        if package is None or node.get("id") in workspace:
            continue
        expression = package.get("license") or ""
        source = package.get("source") or "path"
        entry: dict[str, Any] = {
            "name": package.get("name", ""),
            "version": package.get("version", ""),
            "source": source.split("+", 1)[0],
            "license": expression or None,
        }
        if not expression:
            entry["license_file"] = package.get("license_file")
        if package.get("links"):
            entry["links"] = package["links"]
        if package.get("links") or str(entry["name"]).endswith("-sys"):
            # Binds a system C library: its licence is not covered by this entry.
            entry["native"] = True
        reasons = review_reasons(expression) if expression else ["no SPDX licence field"]
        if reasons:
            entry["review"] = reasons
        entries.append(entry)
    entries.sort(key=lambda e: (e["name"], e["version"]))

    workspace_licenses = Counter(
        packages[w].get("license") or "none" for w in workspace if w in packages
    )
    return {
        "schema": SCHEMA,
        "target": TARGET,
        "features": "all",
        "summary": {
            "workspace_packages": len(workspace),
            "workspace_licenses": dict(sorted(workspace_licenses.items())),
            "third_party_packages": len(entries),
            "sources": dict(sorted(Counter(e["source"] for e in entries).items())),
            "licenses": dict(sorted(Counter(e["license"] or "none" for e in entries).items())),
            "needs_review": sum(1 for e in entries if "review" in e),
            "native": sorted(e["name"] for e in entries if e.get("native")),
        },
        "packages": entries,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("metadata", type=Path, help="cargo metadata JSON (see module docstring)")
    parser.add_argument("-o", "--output", type=Path, help="write the inventory here instead of stdout")
    args = parser.parse_args(argv)
    try:
        metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        print(f"license inventory: cannot read {args.metadata}: {exc}", file=sys.stderr)
        return 2
    if not isinstance(metadata, dict):
        print("license inventory: metadata is not a JSON object", file=sys.stderr)
        return 2
    text = json.dumps(build_inventory(metadata), indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
