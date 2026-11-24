# SPDX-License-Identifier: GPL-3.0-or-later
"""check_ffmpeg_version.py on a throw-away git repository (CFA4-FFMPEG-9.1-001, R1)."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import check_ffmpeg_version as check


class CheckFfmpegVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.write("packaging/ffmpeg-version.env", "# pinned\nCFA4_FFMPEG_VERSION=7.6.5\nCFA4_FFMPEG_MIN_VERSION=7.6.4\n")
        self.write("packaging/ffmpeg-version-allowlist.txt", "docs/audit/*   # history\n")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, path: str, text: str) -> None:
        full = self.root / path
        full.parent.mkdir(parents=True, exist_ok=True)
        full.write_text(text, encoding="utf-8")
        subprocess.run(["git", "-C", str(self.root), "add", path], check=True)

    def run_check(self) -> int:
        return check.main(["--root", str(self.root)])

    def test_clean_tree_passes(self) -> None:
        self.write("scripts/a.sh", '. packaging/ffmpeg-version.env\necho "$CFA4_FFMPEG_VERSION"\n')
        self.assertEqual(self.run_check(), 0)

    def test_version_in_a_script_fails(self) -> None:
        self.write("scripts/a.sh", "prefix=/opt/cfa4/ffmpeg-7.6.5\n")
        self.assertEqual(self.run_check(), 1)

    def test_minimum_version_in_code_fails(self) -> None:
        self.write("crates/x/src/lib.rs", 'const FFMPEG_MIN: &str = "7.6.4";\n')
        self.assertEqual(self.run_check(), 1)

    def test_longer_numbers_are_not_the_version(self) -> None:
        self.write("scripts/a.sh", "ffmpeg_a=17.6.5\nffmpeg_b=7.6.51\nffmpeg_c=7.6.5.1\n")
        self.assertEqual(self.run_check(), 0)

    def test_prose_counts_only_next_to_ffmpeg(self) -> None:
        self.write("docs/a.md", "See §7.6.5 of the spec.\n")
        self.assertEqual(self.run_check(), 0)
        self.write("docs/b.md", "Built with FFmpeg 7.6.5.\n")
        self.assertEqual(self.run_check(), 1)

    def test_other_meanings_and_plan_ids_pass(self) -> None:
        self.write("crates/x/src/mix.rs", '// Auro-3D 7.6.5 bed\nconst LAYOUT: &str = "7.6.5";\n')
        self.write("docs/c.md", "See CFA4-FFMPEG-7.6.5-001 (the FFmpeg plan).\n")
        self.assertEqual(self.run_check(), 0)

    def test_allowlisted_history_passes(self) -> None:
        self.write("docs/audit/old.md", "Measured with FFmpeg 7.6.5.\n")
        self.assertEqual(self.run_check(), 0)

    def test_versioned_script_names_fail(self) -> None:
        self.write("scripts/dev/build-ffmpeg-7.6.5.sh", "#!/bin/sh\n")
        self.assertEqual(self.run_check(), 1)


if __name__ == "__main__":
    unittest.main()
