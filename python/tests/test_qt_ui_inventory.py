"""scripts/dev/qt_ui_inventory.py: the Qt 6 GUI's menus and panels for `cargo xtask parity` (G7)."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path
from types import ModuleType
from typing import Any

import pytest

pytest.importorskip("PySide6")
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtWidgets import QApplication, QDockWidget, QLabel, QMainWindow, QMenuBar, QWidget  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "dev" / "qt_ui_inventory.py"


def load() -> ModuleType:
    spec = importlib.util.spec_from_file_location("qt_ui_inventory", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def app() -> QApplication:
    instance = QApplication.instance()
    return instance if isinstance(instance, QApplication) else QApplication([])


def test_menu_items_walks_the_bar_in_order(app: QApplication) -> None:
    inv = load()
    bar = QMenuBar()
    file_menu = bar.addMenu("&File")
    new = file_menu.addMenu("&New")
    seq = new.addAction("Sequence…")
    seq.setObjectName("file.new.sequence")
    file_menu.addSeparator()
    file_menu.addAction("Save && Close").setObjectName("file.saveClose")
    file_menu.addAction("   ")  # no label: skipped
    hidden = file_menu.addAction("Hidden")
    hidden.setVisible(False)
    bar.addMenu("Empty")
    bar.addAction("Not a menu")  # a bare menu-bar action has no menu path
    assert inv.menu_items(bar) == [
        {"id": "file.new.sequence", "label": "Sequence…", "path": ["File", "New"]},
        {"id": "file.saveClose", "label": "Save & Close", "path": ["File"]},
    ]


class FakeWindow(QMainWindow):
    def __init__(self) -> None:
        super().__init__()
        self.docks: dict[str, QDockWidget] = {}
        self.setCentralWidget(QWidget(self))


def test_panels_status(app: QApplication) -> None:
    inv = load()
    from cfa4_qt.window_menu import PANELS

    window = FakeWindow()
    for entry in PANELS:
        title = str(entry.title)
        if title in ("Program", "Metadata"):
            continue
        dock = QDockWidget(title, window)
        if title == "Libraries":
            label = QLabel("coming later")
            label.setEnabled(False)
            dock.setWidget(label)
        else:
            dock.setWidget(QWidget())
        window.docks[title] = dock
    extra = QDockWidget("Extra", window)
    extra.setWidget(QWidget())
    window.docks["Extra"] = extra
    status = {p["title"]: p["status"] for p in inv.panels(window)}
    assert status["Program"] == "central"
    assert status["Libraries"] == "placeholder"
    assert status["Metadata"] == "listed"
    assert status["Extra"] == "docked"
    assert status["Timeline"] == "docked"
    assert len(status) == len(PANELS) + 1


def engine() -> Path | None:
    for candidate in (os.environ.get("CFA4_CLI"), os.environ.get("CARGO_TARGET_DIR", "") and str(Path(os.environ["CARGO_TARGET_DIR"]) / "release" / "cfa4-cli"), str(ROOT / "target" / "release" / "cfa4-cli")):
        if candidate and Path(candidate).is_file():
            return Path(candidate)
    return None


def run(*args: str) -> Any:
    cli = engine()
    if cli is None:
        pytest.skip("cfa4-cli not built (cargo build --release -p cfa4-cli)")
    env = {**os.environ, "QT_QPA_PLATFORM": "offscreen"}
    out = subprocess.run([sys.executable, str(SCRIPT), *args, "--engine", str(cli)], capture_output=True, text=True, timeout=180, env=env, check=False)
    assert out.returncode == 0, out.stderr
    return json.loads(out.stdout)


def test_live_window_panels() -> None:
    """The real window: Program is central, Libraries a placeholder, every other panel docked."""
    from cfa4_qt.window_menu import PANELS

    status = {p["title"]: p["status"] for p in run("--panels")}
    assert status.pop("Program") == "central"
    assert status.pop("Libraries") == "placeholder"
    assert set(status.values()) == {"docked"}
    assert {str(e.title) for e in PANELS} <= {*status, "Program", "Libraries"}


def test_live_window_menus() -> None:
    items = run("--menus")
    paths = {(" > ".join(i["path"]), i["label"]) for i in items}
    tops = {i["path"][0] for i in items}
    assert {"File", "Edit", "Window", "Help"} <= tops
    assert ("Window", "Timeline") in paths
    assert all(i["label"] and i["path"] for i in items)
