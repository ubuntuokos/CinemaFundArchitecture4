"""Offline audit of Cargo Git dependency pins for the CFA4 F1-01 migration.

This validates provenance references, NOT all dependency licenses or code safety.
The two approved Git references are temporary until fully vendored or replaced.
"""

from __future__ import annotations

import sys
import tomllib
from collections.abc import Iterator
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = "https://github.com/storytold/craft-libs"
APPROVED_GIT = {
    "craft-ui": "5c874d51eeb3de70e735c8ce87a59a7dedd47675",
    "craft-docpath": "82a24b006eefa036b43c09a67fb9a3fbd17ffe63",
}


def git_locations(data: Any, parents: tuple[str, ...] = ()) -> Iterator[tuple[tuple[str, ...], str]]:
    """Find every TOML table containing a git dependency, including target-specific ones."""
    if isinstance(data, dict):
        if isinstance(data.get("git"), str):
            yield parents, data["git"]
        for name, value in data.items():
            if name != "git":
                yield from git_locations(value, (*parents, str(name)))
    elif isinstance(data, list):
        for index, item in enumerate(data):
            yield from git_locations(item, (*parents, str(index)))


def validate_snapshot(manifest: dict[str, Any], lock: dict[str, Any]) -> list[str]:
    """Compare the approved immutable source revisions to Cargo.toml and Cargo.lock."""
    errors: list[str] = []
    dependencies = manifest.get("workspace", {}).get("dependencies", {})
    approved_names = set(APPROVED_GIT)

    for name, expected_rev in APPROVED_GIT.items():
        dependency = dependencies.get(name)
        if not isinstance(dependency, dict):
            errors.append(f"Cargo.toml: missing structured dependency {name}")
            continue
        if dependency.get("git") != UPSTREAM:
            errors.append(f"Cargo.toml: {name} Git repository differs from approved source")
        if dependency.get("rev") != expected_rev:
            errors.append(f"Cargo.toml: {name} revision differs from approved immutable SHA")
        if any(key in dependency for key in ("branch", "tag")):
            errors.append(f"Cargo.toml: {name} must not depend on a moving branch or tag")

    approved_locations = {("workspace", "dependencies", name) for name in approved_names}
    for location, url in git_locations(manifest):
        if location not in approved_locations or url != UPSTREAM:
            errors.append(f"Cargo.toml: unapproved Git source at {'.'.join(location)}")

    seen: set[str] = set()
    for package in lock.get("package", []):
        source = str(package.get("source", ""))
        if not source.startswith("git+"):
            continue
        name = str(package.get("name", ""))
        if name not in approved_names:
            errors.append(f"Cargo.lock: unapproved Git package {name}")
            continue
        if name in seen:
            errors.append(f"Cargo.lock: duplicated Git package {name}")
        seen.add(name)
        rev = APPROVED_GIT[name]
        expected_source = f"git+{UPSTREAM}?rev={rev}#{rev}"
        if source != expected_source:
            errors.append(f"Cargo.lock: {name} source/revision does not match approved pin")

    for missing in sorted(approved_names - seen):
        errors.append(f"Cargo.lock: missing approved Git package {missing}")
    return errors


def audit_repository(root: Path) -> list[str]:
    """Check the lockfile plus all top-level Rust workspace member manifests."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    errors = validate_snapshot(manifest, lock)

    for pattern in ("apps/*/Cargo.toml", "crates/*/Cargo.toml", "xtask/Cargo.toml"):
        for path in sorted(root.glob(pattern)):
            data = tomllib.loads(path.read_text(encoding="utf-8"))
            for location, _ in git_locations(data):
                relative = path.relative_to(root).as_posix()
                errors.append(f"{relative}: unexpected direct Git dependency at {'.'.join(location)}")
    return errors


def main() -> int:
    errors = audit_repository(ROOT)
    if errors:
        for error in errors:
            print(f"F1-01 dependency audit: {error}", file=sys.stderr)
        return 1
    print("F1-01 Git dependency audit passed: 2 approved immutable pins match manifests and Cargo.lock.")
    print("This does not certify transitive crate licenses, PySide6 or redistribution rights.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
