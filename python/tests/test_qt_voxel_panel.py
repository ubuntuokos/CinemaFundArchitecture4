"""N1-10: the Info panel's "Voxel attributes" section (OpenVDB volumes), without an engine."""

from __future__ import annotations

import os
from collections.abc import Iterator
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.panels.info import InfoPanel

INSPECTION = {
    "grids": [
        {
            "name": "density",
            "type": "float",
            "gridClass": "unknown",
            "voxelSize": [0.5, 0.5, 0.5],
            "activeVoxels": 8000,
            "bbox": {"min": [0, 0, 0], "max": [19, 19, 19]},
            "isPoints": False,
            "meta": {"file_compression": "blosc"},
        },
        {
            "name": "pts",
            "type": "ptdataidx32",
            "gridClass": "unknown",
            "voxelSize": [1, 1, 1],
            "activeVoxels": 3,
            "bbox": {"min": [0, 0, 0], "max": [1, 1, 1]},
            "isPoints": True,
            "meta": {},
        },
    ],
    "fileMetaCount": 0,
}


@pytest.fixture(scope="module")
def app() -> Iterator[Any]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def test_voxel_section_is_readable_and_has_automation_ids(app: Any) -> None:
    panel = InfoPanel()
    assert panel.findChild(QtWidgets.QLabel, "info.voxel") is not None
    assert panel.findChild(QtWidgets.QLabel, "info.voxel.grids") is not None
    panel.update_voxels(INSPECTION)
    assert not panel.voxel_header.isHidden()
    text = panel.voxel_lines.text()
    for needle in ("density", "float", "0.5 × 0.5 × 0.5", "8,000", "pts", "points", "file_compression"):
        assert needle in text, needle


def test_error_and_other_items(app: Any) -> None:
    panel = InfoPanel()
    panel.update_voxels(None, "OpenVDB worker is not available")
    assert "not available" in panel.voxel_lines.text()
    panel.update_voxels(None)
    assert panel.voxel_header.isHidden() and panel.voxel_lines.isHidden()
