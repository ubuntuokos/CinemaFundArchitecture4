"""CFA4-PY-TYPING-001 K1: a clip edge without its time is 0, not a TypeError (T0b report)."""

from __future__ import annotations

import os

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.panels.timeline import edge_time


def test_the_edges_of_a_whole_clip() -> None:
    clip = {"start": 100, "end": 250}
    assert edge_time(clip, out=False) == 100
    assert edge_time(clip, out=True) == 250


@pytest.mark.parametrize("clip", [{}, {"end": 250}, {"start": None, "end": 250}])
def test_a_missing_in_time_is_zero(clip: dict[str, object]) -> None:
    # the old `int(a if in else b or 0)` raised TypeError here: `or 0` guarded only the Out time
    assert edge_time(clip, out=False) == 0


@pytest.mark.parametrize("clip", [{}, {"start": 100}, {"start": 100, "end": None}])
def test_a_missing_out_time_is_zero(clip: dict[str, object]) -> None:
    assert edge_time(clip, out=True) == 0
