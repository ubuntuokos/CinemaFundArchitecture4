"""Experimental Linux Qt 6 shell. NOT the complete video editor.

The Rust cfa4-cli is queried for its version, and Edit ▸ Settings… opens the Settings window
(`qt_settings.py`), which reads and writes preferences through local `cfa4-cli settings` calls
on a worker thread. No shell, no remote calls.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path


def locate_engine(explicit: str | None = None) -> Path | None:
    """Find a local CLI without installing or downloading software."""
    if explicit:
        candidate = Path(explicit).expanduser()
        return candidate if candidate.is_file() else None
    # Prefer the installed CFA4 command.
    executable = shutil.which("cfa4-cli")
    if executable:
        return Path(executable)
    root = Path(__file__).resolve().parents[2]
    for relative in ("target/release/cfa4-cli", "target/debug/cfa4-cli"):
        candidate = root / relative
        if candidate.is_file():
            return candidate
    return None


def engine_version(executable: Path | None) -> str:
    if executable is None:
        return "Rust CLI: not built (cargo build --release -p cfa4-cli)"
    try:
        process = subprocess.run(
            [str(executable), "--version"], capture_output=True, text=True, timeout=8, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"Rust CLI unavailable: {exc}"
    if process.returncode != 0:
        return f"Rust CLI error ({process.returncode}): {(process.stderr or '').strip()}"
    return (process.stdout or "").strip() or "Rust CLI returned an empty version"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", help="Path to the local cfa4-cli executable")
    parser.add_argument("--smoke", action="store_true", help="Create Qt widgets and exit without interaction")
    args = parser.parse_args(argv)

    if not sys.platform.startswith("linux"):
        parser.error("CFA4 Qt development targets Linux only")

    # Optional dependency: the Rust workspace does not link Qt or depend on Python.
    try:
        from PySide6.QtCore import QTimer
        from PySide6.QtGui import QAction, QKeySequence
        from PySide6.QtWidgets import QApplication, QLabel, QMainWindow, QPushButton, QVBoxLayout, QWidget

        from cfa4_devtools.qt_settings import SettingsDialog
        from cfa4_devtools.settings_backend import MEDIA_BACKENDS_PAGE
    except ImportError as exc:
        parser.error(f"Qt 6/PySide6 missing: {exc}; install: python -m pip install -e '.[qt]'")

    app = QApplication([sys.argv[0]])
    window = QMainWindow()
    window.setWindowTitle("CFA4 — Qt 6 development shell")
    container = QWidget()
    layout = QVBoxLayout(container)
    layout.addWidget(QLabel("CFA4 · Linux · Rust / Python / Qt 6"))
    layout.addWidget(QLabel("Experimental Qt UI only. Existing Rust editor is not yet ported to Qt."))
    status = QLabel()
    status.setWordWrap(True)
    engine = locate_engine(args.engine)

    def refresh() -> None:
        status.setText(engine_version(engine))

    button = QPushButton("Check local Rust engine")
    button.clicked.connect(refresh)
    layout.addWidget(status)
    layout.addWidget(button)
    window.setCentralWidget(container)

    def open_settings(page: str | None = None) -> None:
        if engine is None:
            status.setText("Settings need the Rust CLI: cargo build --release -p cfa4-cli")
            return
        SettingsDialog(engine, parent=window, page=page).exec()

    settings_action = QAction("Settings…", window)
    settings_action.setObjectName("menu.edit.settings")
    settings_action.setShortcut(QKeySequence.StandardKey.Preferences)
    if settings_action.shortcut().isEmpty():
        settings_action.setShortcut(QKeySequence("Ctrl+,"))
    settings_action.triggered.connect(lambda: open_settings())
    edit_menu = window.menuBar().addMenu("&Edit")
    edit_menu.addAction(settings_action)
    # Edit ▸ Media Backends… and Help ▸ Restricted Codecs…: the Settings window opened at the
    # Media & Playback ▸ Media Backends page (ADR 0005; Ubuntu model, docs/licensing.md §7)
    backends_action = QAction("Media Backends…", window)
    backends_action.setObjectName("menu.edit.mediaBackends")
    backends_action.triggered.connect(lambda: open_settings(MEDIA_BACKENDS_PAGE))
    edit_menu.addAction(backends_action)
    restricted_action = QAction("Restricted Codecs…", window)
    restricted_action.setObjectName("menu.help.restrictedCodecs")
    restricted_action.triggered.connect(lambda: open_settings(MEDIA_BACKENDS_PAGE))
    window.menuBar().addMenu("&Help").addAction(restricted_action)
    window.resize(650, 200)
    if os.environ.get("CFA4_QT_LIST_MENUS"):
        # tests: every menu item's automation id, so menu reachability is checked without a display
        for menu_action in window.menuBar().actions():
            menu = menu_action.menu()
            for action in menu.actions() if menu is not None else []:
                print(action.objectName())
    refresh()
    window.show()
    if args.smoke:
        QTimer.singleShot(0, app.quit)
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
