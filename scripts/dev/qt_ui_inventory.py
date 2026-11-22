#!/usr/bin/env python3
"""Print what the Qt 6 GUI shows, as JSON, for ``cargo xtask parity`` (docs/gaps.md G7).

Starts the real editor window without a screen (``QT_QPA_PLATFORM=offscreen``) on the demo
project, the way ``cfa4-qt --smoke`` does, waits until the engine has sent its commands and the
menus are built, then reads the live window:

- ``--menus``: every menu-bar item, ``[{"id", "label", "path"}]`` (the shape of the egui
  ``menu_tree`` example; ``path`` is the menu path, ``label`` the item).
- ``--panels``: every panel, ``[{"title", "status"}]``; ``status`` is ``central`` (the Program
  monitor), ``docked``, ``placeholder`` (a dock that only shows a disabled label, as Libraries)
  or ``listed`` (in the Window menu but with no dock).

It only reads the window; it changes no Qt GUI file. It fails (exit 1, a message on stderr)
when Qt or the engine is missing or the window does not come up in time, never with an empty
list. Run from the repository root:

    QT_QPA_PLATFORM=offscreen .venv/bin/python scripts/dev/qt_ui_inventory.py --menus --engine target/release/cfa4-cli
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
# Fewest top-level menus a built menu bar has; fewer means the commands have not arrived yet.
MIN_MENUS = 5


def menu_items(bar: Any) -> list[dict[str, Any]]:
    """The leaf items of a menu bar (or menu), depth first, in menu order."""
    out: list[dict[str, Any]] = []

    def walk(actions: list[Any], path: list[str], depth: int) -> None:
        if depth > 16:  # a menu that contains itself
            return
        for a in actions:
            if a.isSeparator() or not a.isVisible():
                continue
            text = a.text().replace("&&", "\0").replace("&", "").replace("\0", "&").strip()
            if not text:
                continue
            sub = a.menu()
            if sub is not None:
                walk(list(sub.actions()), [*path, text], depth + 1)
            elif path:
                out.append({"id": a.objectName(), "label": text, "path": path})

    walk(list(bar.actions()), [], 0)
    return out


def is_placeholder(widget: Any) -> bool:
    """A dock that only shows a disabled label (egui's ``placeholder``), as Libraries."""
    from PySide6.QtWidgets import QLabel

    return isinstance(widget, QLabel) and not widget.isEnabled()


def panels(window: Any) -> list[dict[str, str]]:
    """The panels of the Window menu and the docks, with their status."""
    from cfa4_qt.window_menu import PANELS

    out: list[dict[str, str]] = []
    seen: set[str] = set()
    central = window.centralWidget()
    for entry in PANELS:
        title = str(entry.title)
        seen.add(title)
        dock = window.docks.get(title)
        if dock is not None:
            status = "placeholder" if is_placeholder(dock.widget()) else "docked"
        elif title == "Program" and central is not None:
            status = "central"
        else:
            status = "listed"
        out.append({"title": title, "status": status})
    for title, dock in window.docks.items():  # docked panels the Window menu does not list
        if title not in seen:
            out.append({"title": title, "status": "placeholder" if is_placeholder(dock.widget()) else "docked"})
    return out


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    what = parser.add_mutually_exclusive_group(required=True)
    what.add_argument("--menus", action="store_true", help="the menu bar")
    what.add_argument("--panels", action="store_true", help="the panels")
    parser.add_argument("--engine", help="path to cfa4-cli (default: PATH, then target/)")
    parser.add_argument("--timeout", type=float, default=60.0, help="seconds to wait for the window")
    args = parser.parse_args(argv)
    if not 1.0 <= args.timeout <= 600.0:
        parser.error("--timeout must be 1 to 600 seconds")
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    sys.path.insert(0, str(ROOT / "python"))
    try:
        from PySide6.QtCore import QSettings, QTimer
        from PySide6.QtWidgets import QApplication
    except ImportError as exc:
        print(f"qt_ui_inventory: Qt 6 / PySide6 missing: {exc} (docs/development/linux.md)", file=sys.stderr)
        return 1
    from cfa4_qt.engine import EngineClient, EngineError, locate_engine
    from cfa4_qt.main_window import MainWindow

    rejected: list[str] = []
    engine_path = locate_engine(args.engine, rejected)
    if engine_path is None:
        print("qt_ui_inventory: cfa4-cli not found; build it: cargo build --release -p cfa4-cli", file=sys.stderr)
        return 1
    app = QApplication([sys.argv[0]])
    # a throwaway settings file and data directory: the user's layout and language stay out
    scratch = tempfile.TemporaryDirectory(prefix="cfa4-qt-inventory-")
    settings = QSettings(str(Path(scratch.name) / "settings.ini"), QSettings.Format.IniFormat)
    engine = EngineClient(engine_path, ["--demo", "--data-dir", scratch.name])
    window = MainWindow(engine, settings)
    result: dict[str, Any] = {"code": 1, "error": "the window did not come up in time"}

    def done(code: int, error: str = "") -> None:
        result.update(code=code, error=error)
        window._quit_confirmed = True
        window.close()
        app.quit()

    def poll() -> None:
        bar = window.menuBar()
        menus = [a for a in bar.actions() if a.menu() is not None]
        if window.registry.commands and len(menus) >= MIN_MENUS:
            # one more turn of the event loop: the Window and Tools menus are rebuilt with the rest
            QTimer.singleShot(500, read)
        else:
            QTimer.singleShot(200, poll)

    def read() -> None:
        try:
            data = menu_items(window.menuBar()) if args.menus else panels(window)
        except Exception as exc:  # a changed window: report it, never print a partial list
            done(1, f"reading the window failed: {exc}")
            return
        if not data:
            done(1, "the window has no " + ("menu items" if args.menus else "panels"))
            return
        result["data"] = data
        done(0)

    engine.started.connect(lambda _hello: QTimer.singleShot(0, poll))
    QTimer.singleShot(int(args.timeout * 1000), lambda: done(1, result["error"]))
    try:
        engine.start()
    except EngineError as exc:
        print(f"qt_ui_inventory: {exc}", file=sys.stderr)
        return 1
    window.show()
    app.exec()
    engine.stop()
    scratch.cleanup()
    if result["code"] != 0:
        print(f"qt_ui_inventory: {result['error']}", file=sys.stderr)
        return 1
    print(json.dumps(result["data"], ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
