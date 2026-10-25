"""Offline provenance checks for approved Cargo Git sources and vendored snapshots.

The audit is not a complete SBOM, copyright review or distribution license.
"""

from __future__ import annotations

import hashlib
import json
import sys
import tomllib
from collections.abc import Iterator
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = "https://github.com/storytold/craft-libs"
APPROVED_GIT: dict[str, str] = {}
VENDORED = {
    "cfa4-docpath": "vendor/cfa4-libs/docpath",
    "cfa4-ui": "vendor/cfa4-libs/ui",
}


def git_locations(data: Any, parents: tuple[str, ...] = ()) -> Iterator[tuple[tuple[str, ...], str]]:
    """Locate all TOML Git dependencies, including target-specific dependencies."""
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
    errors: list[str] = []
    dependencies = manifest.get("workspace", {}).get("dependencies", {})
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

    for name, path in VENDORED.items():
        dependency = dependencies.get(name)
        if not isinstance(dependency, dict) or dependency.get("path") != path:
            errors.append(f"Cargo.toml: {name} must reference the approved local vendor path")
        elif any(key in dependency for key in ("git", "rev", "branch", "tag")):
            errors.append(f"Cargo.toml: {name} must not have a Git source or revision")

    approved_locations = {("workspace", "dependencies", name) for name in APPROVED_GIT}
    for location, url in git_locations(manifest):
        if location not in approved_locations or url != UPSTREAM:
            errors.append(f"Cargo.toml: unapproved Git source at {'.'.join(location)}")

    names = set(APPROVED_GIT) | set(VENDORED)
    seen: set[str] = set()
    for package in lock.get("package", []):
        name = str(package.get("name", ""))
        source = str(package.get("source", ""))
        if name in names:
            if name in seen:
                errors.append(f"Cargo.lock: duplicated package {name}")
            seen.add(name)
        if name in VENDORED and source:
            errors.append(f"Cargo.lock: vendored {name} must have no remote source")
        elif source.startswith("git+"):
            if name not in APPROVED_GIT:
                errors.append(f"Cargo.lock: unapproved Git package {name}")
                continue
            rev = APPROVED_GIT[name]
            if source != f"git+{UPSTREAM}?rev={rev}#{rev}":
                errors.append(f"Cargo.lock: {name} source/revision does not match approved pin")

    for missing in sorted(names - seen):
        errors.append(f"Cargo.lock: missing approved package {missing}")
    return errors


def git_blob_hash(data: bytes) -> str:
    """Reproduce Git SHA-1 object IDs for immutable upstream source snapshots."""
    return hashlib.sha1(b"blob " + str(len(data)).encode("ascii") + b"\0" + data).hexdigest()


def verify_vendored_files(root: Path) -> list[str]:
    errors: list[str] = []
    for package, rel in VENDORED.items():
        directory = root / rel
        try:
            ledger = json.loads((directory / "SOURCE_MANIFEST.json").read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            errors.append(f"{package}: missing or invalid SOURCE_MANIFEST.json: {exc}")
            continue
        if ledger.get("source_repository") != UPSTREAM:
            errors.append(f"{package}: upstream source URL does not match")
        sha = ledger.get("source_commit", "")
        if not isinstance(sha, str) or len(sha) != 40 or not all(c in "0123456789abcdef" for c in sha):
            errors.append(f"{package}: source commit must be a full Git SHA")
        files = ledger.get("files")
        if not isinstance(files, dict) or not files:
            errors.append(f"{package}: source provenance file list missing")
            continue
        expected_paths = {path for path in files if isinstance(path, str)}
        actual_paths = {
            path.relative_to(directory).as_posix()
            for path in directory.rglob("*")
            if path.is_file()
            and path.relative_to(directory).as_posix() not in {"Cargo.toml", "SOURCE_MANIFEST.json"}
        }
        for untracked in sorted(actual_paths - expected_paths):
            errors.append(f"{package}: unmanifested source file {untracked}")
        for path, expected in files.items():
            if not isinstance(path, str) or not isinstance(expected, str):
                errors.append(f"{package}: invalid provenance path/hash entry")
                continue
            candidate = (directory / path).resolve()
            if not candidate.is_relative_to(directory.resolve()):
                errors.append(f"{package}: unsafe provenance path {path}")
                continue
            try:
                actual = git_blob_hash(candidate.read_bytes())
            except OSError as exc:
                errors.append(f"{package}: missing source {path}: {exc}")
                continue
            if actual != expected:
                errors.append(f"{package}: changed vendor source {path}: Git blob hash differs")
    return errors


def audit_repository(root: Path) -> list[str]:
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    errors = validate_snapshot(manifest, lock)
    errors.extend(verify_vendored_files(root))
    for pattern in ("apps/*/Cargo.toml", "crates/*/Cargo.toml", "xtask/Cargo.toml", "vendor/cfa4-libs/*/Cargo.toml"):
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
            print(f"F1-02 dependency audit: {error}", file=sys.stderr)
        return 1
    print("F1-02 dependency audit passed: both vendored source trees validated; no Cargo Git sources remain.")
    print("This does not certify registry licenses, Qt/PySide6 or redistribution rights.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
