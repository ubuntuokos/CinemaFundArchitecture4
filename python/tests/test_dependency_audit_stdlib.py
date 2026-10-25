"""Standard-library regression checks for CFA4 Cargo source detachment."""

from __future__ import annotations

import json
import shutil
import tempfile
import unittest
from pathlib import Path

from cfa4_devtools.dependency_audit import ROOT, audit_repository, git_locations, verify_vendored_files


class SourceDetachmentTests(unittest.TestCase):
    def test_repository_has_no_cargo_git_sources(self) -> None:
        self.assertEqual(audit_repository(ROOT), [])

    def test_all_vendored_source_hashes_match(self) -> None:
        self.assertEqual(verify_vendored_files(ROOT), [])

    def test_unmanifested_vendor_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            vendor = root / "vendor" / "cfa4-libs" / "docpath"
            shutil.copytree(ROOT / "vendor" / "cfa4-libs" / "docpath", vendor)
            (vendor / "src" / "untracked.rs").write_text(
                "// not in SOURCE_MANIFEST.json\n", encoding="utf-8"
            )
            errors = verify_vendored_files(root)
            self.assertTrue(
                any("unmanifested source file src/untracked.rs" in error for error in errors),
                errors,
            )

    def test_upstream_revisions_are_full_shas(self) -> None:
        for crate in ("docpath", "ui"):
            manifest = ROOT / "vendor" / "cfa4-libs" / crate / "SOURCE_MANIFEST.json"
            sha = json.loads(manifest.read_text(encoding="utf-8"))["source_commit"]
            self.assertEqual(len(sha), 40)
            self.assertTrue(all(char in "0123456789abcdef" for char in sha))

    def test_git_sources_are_absent_from_all_workspace_manifests(self) -> None:
        import tomllib

        root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
        self.assertEqual(list(git_locations(root_manifest)), [])
        self.assertFalse(any(str(pkg.get("source", "")).startswith("git+") for pkg in lock["package"]))


if __name__ == "__main__":
    unittest.main()
