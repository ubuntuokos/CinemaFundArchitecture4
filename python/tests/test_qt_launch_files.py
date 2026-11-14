"""Files handed to ``cfa4-qt`` on the command line, as the desktop entry's ``%F`` passes them.

The packaged desktop entry starts the Qt GUI (owner decision 2026-11-12), so it must take what the
egui editor took: a project to open and media files to import (``apps/cfa4/src/args.rs``
``project_and_media``), plus CFA4's credits files. Pure argument handling: no Qt, no engine.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from cfa4_qt.__main__ import is_project, launch_files


def test_projects_are_fcproj_in_any_case_as_egui() -> None:
    for p in ["cut.fcproj", "CUT.FCPROJ", "Cut.FcProj", "/edits/a b.fcproj"]:
        assert is_project(p), p
    for p in ["cut.mp4", "fcproj", ".fcproj", "a.fcproj.mov", "a.fcprojx", ""]:
        assert not is_project(p), p


def test_the_first_project_opens_and_every_other_file_is_imported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    project, credits, media = launch_files(["a.mp4", "Cut.FCPROJ", "b.wav", "other.fcproj", "c.png"])
    assert project == str((tmp_path / "Cut.FCPROJ").resolve())
    assert credits is None
    # relative paths made absolute against the working directory, in the order given; a second
    # project is neither opened nor imported (egui's `project_and_media` drops it the same way)
    assert media == [str(tmp_path / "a.mp4"), str(tmp_path / "b.wav"), str(tmp_path / "c.png")]


def test_media_only_launch_imports_without_a_project(tmp_path: Path) -> None:
    project, credits, media = launch_files([str(tmp_path / "x.mov"), str(tmp_path / "y.mxf")])
    assert project is None and credits is None
    assert media == [str(tmp_path / "x.mov"), str(tmp_path / "y.mxf")]


def test_credits_files_open_in_the_workspace_and_are_not_imported(tmp_path: Path) -> None:
    project, credits, media = launch_files(
        [str(tmp_path / "film.cfa4credits"), str(tmp_path / "a.mp4"), str(tmp_path / "Look.CFA4CREDTPL")]
    )
    assert project is None
    assert credits == str((tmp_path / "film.cfa4credits").resolve())
    assert media == [str(tmp_path / "a.mp4")]


def test_media_paths_keep_their_symlinks(tmp_path: Path) -> None:
    """As egui's `std::path::absolute`: the path saved in the project is the one the user gave."""
    real = tmp_path / "real.mp4"
    real.write_bytes(b"")
    link = tmp_path / "link.mp4"
    os.symlink(real, link)
    assert launch_files([str(link)])[2] == [str(link)]


def test_no_files_change_nothing() -> None:
    assert launch_files([]) == (None, None, [])
    assert launch_files([""]) == (None, None, [])
