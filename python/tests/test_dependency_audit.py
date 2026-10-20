"""Regression tests for the F1-01 locked-Git-source supply-chain gate."""

from __future__ import annotations

import copy
import tomllib

from cfa4_devtools.dependency_audit import (
    APPROVED_GIT,
    ROOT,
    UPSTREAM,
    audit_repository,
    git_locations,
    validate_snapshot,
    verify_vendored_files,
)


def snapshot() -> tuple[dict, dict]:
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    return manifest, lock


def test_repository_git_source_inventory() -> None:
    assert audit_repository(ROOT) == []


def test_moving_git_ref_is_rejected() -> None:
    manifest, lock = snapshot()
    manifest = copy.deepcopy(manifest)
    manifest["workspace"]["dependencies"]["craft-ui"]["rev"] = "main"
    assert any("craft-ui revision" in error for error in validate_snapshot(manifest, lock))


def test_unknown_workspace_git_source_is_rejected() -> None:
    manifest, lock = snapshot()
    manifest = copy.deepcopy(manifest)
    manifest["workspace"]["dependencies"]["new-upstream"] = {"git": "https://example.org/new"}
    assert any("unapproved Git source" in error for error in validate_snapshot(manifest, lock))


def test_lockfile_pin_must_match_manifest() -> None:
    manifest, lock = snapshot()
    lock = copy.deepcopy(lock)
    for package in lock["package"]:
        if package["name"] == "craft-ui":
            package["source"] = "git+" + UPSTREAM + "?rev=main#deadbeef"
    assert any("craft-ui source/revision" in error for error in validate_snapshot(manifest, lock))


def test_missing_locked_package_is_rejected() -> None:
    manifest, lock = snapshot()
    lock = copy.deepcopy(lock)
    lock["package"] = [package for package in lock["package"] if package["name"] != "craft-ui"]
    assert any("missing approved Git package craft-ui" in error for error in validate_snapshot(manifest, lock))


def test_unexpected_transitive_git_package_is_rejected() -> None:
    manifest, lock = snapshot()
    lock = copy.deepcopy(lock)
    lock["package"].append(
        {"name": "unknown-git", "version": "0.1.0", "source": "git+https://example.org/rogue#123"}
    )
    assert any("unapproved Git package" in error for error in validate_snapshot(manifest, lock))


def test_finds_nested_target_specific_git_sources() -> None:
    locations = list(git_locations({"target": {"cfg(unix)": {"dependencies": {"rogue": {"git": "https://x"}}}}}))
    assert locations == [(("target", "cfg(unix)", "dependencies", "rogue"), "https://x")]


def test_pins_are_full_commit_shas() -> None:
    assert all(len(rev) == 40 and all(c in "0123456789abcdef" for c in rev) for rev in APPROVED_GIT.values())

def test_docpath_is_local_without_locked_git_source() -> None:
    manifest, lock = snapshot()
    assert manifest["workspace"]["dependencies"]["craft-docpath"]["path"] == "vendor/craft-libs/docpath"
    assert all("source" not in package for package in lock["package"] if package["name"] == "craft-docpath")


def test_changed_vendor_source_is_detected(tmp_path) -> None:
    from shutil import copytree

    vendor = tmp_path / "vendor" / "craft-libs" / "docpath"
    copytree(ROOT / "vendor" / "craft-libs" / "docpath", vendor)
    source = vendor / "src" / "lib.rs"
    source.write_bytes(source.read_bytes() + b"\n// unauthorized mutation\n")
    assert any("changed vendor source" in error for error in verify_vendored_files(tmp_path))


def test_vendored_source_integrity() -> None:
    assert verify_vendored_files(ROOT) == []
