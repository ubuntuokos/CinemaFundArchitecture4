"""F2 EXR compression fixtures: the codec list is the pre-registered one; generation needs OpenImageIO."""

from __future__ import annotations

from pathlib import Path

import pytest

from cfa4_devtools import render_f2_exr_fixtures as f2

PREREG = f2.ROOT / "docs" / "audit" / "render-f2" / "F2-preregistration.md"


def test_codec_list_covers_the_preregistered_compressions() -> None:
    text = PREREG.read_text(encoding="utf-8")
    assert "PIZ/DWA/B44" in text
    assert {"piz", "dwaa", "dwab", "b44", "pxr24", "zip", "rle", "none"} <= set(f2.CODECS)


def test_fixtures_go_under_target_and_not_into_the_repo() -> None:
    assert f2.fixtures_dir().is_relative_to(f2.ROOT / "target") or "CFA4_FIXTURES_DIR" in __import__("os").environ


def test_generate_and_selfcheck(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    pytest.importorskip("OpenImageIO")
    pytest.importorskip("numpy")
    monkeypatch.setenv("CFA4_FIXTURES_DIR", str(tmp_path))
    assert f2.generate() == 0
    assert f2.selfcheck() == 0
    assert len(list((tmp_path / "render-f2").glob("*.exr"))) == len(f2.CODECS)
