"""F0V reference fixtures: the committed manifest matches the generator, and the media matches the manifest."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

from cfa4_devtools import render_fixtures as rf

PLAN = rf.ROOT / "docs" / "plans" / "CFA4-RENDER-INTEGRATION-PLAN-001.md"


def test_committed_manifest_is_current() -> None:
    committed = json.loads(rf.MANIFEST.read_text(encoding="utf-8"))
    assert committed == rf.manifest(), "run: python3 python/cfa4_devtools/render_fixtures.py write-manifest"


def test_names_and_outputs_are_unique() -> None:
    assert len({f.name for f in rf.FIXTURES}) == len(rf.FIXTURES)
    assert len({f.output for f in rf.FIXTURES}) == len(rf.FIXTURES)


def test_every_fixture_names_a_defined_acceptance_criterion() -> None:
    defined = set(re.findall(r"\| (AC-\d\d[abc]?) \|", PLAN.read_text(encoding="utf-8")))
    assert defined, "plan copy has no AC table"
    for fx in rf.FIXTURES:
        assert fx.acs, fx.name
        assert set(fx.acs) <= defined, (fx.name, set(fx.acs) - defined)


def test_no_media_is_committed() -> None:
    for path in (rf.ROOT / "docs" / "audit" / "render-f0v").iterdir():
        assert path.suffix in {".md", ".json", ".csv"}, path


@pytest.mark.skipif(rf.tool("ffmpeg") is None or rf.tool("ffprobe") is None, reason="ffmpeg/ffprobe not found")
@pytest.mark.skipif(
    os.environ.get("CFA4_RENDER_FIXTURES_SLOW") != "1", reason="set CFA4_RENDER_FIXTURES_SLOW=1 (generates ~130 MB)"
)
def test_generated_media_matches_manifest(tmp_path: Path) -> None:
    env = {**os.environ, "CFA4_FIXTURES_DIR": str(tmp_path)}
    gen = subprocess.run(
        [sys.executable, str(rf.ROOT / "python/cfa4_devtools/render_fixtures.py"), "generate"],
        env=env,
        capture_output=True,
        text=True,
    )
    assert "FAIL" not in gen.stdout, gen.stdout
    assert gen.returncode == 0, gen.stdout + gen.stderr
