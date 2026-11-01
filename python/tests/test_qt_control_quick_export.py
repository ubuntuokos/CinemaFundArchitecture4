"""CFA4 Qt 6 GUI: the header's Quick Export popup as control-channel state (``docs/qt-parity.md``
1.11 with 4.2), as egui's ``ExportUi`` ``quick_open``, ``quick_path`` and ``quick_preset``."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import EngineClient, locate_engine

REAL = locate_engine()


@pytest.fixture(scope="module")
def app():
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch):
    def refuse(self, *_args, **_kwargs):
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", refuse)


def wait_until(app, predicate, timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_quick_export_through_the_control_channel(app, tmp_path: Path):
    """``ui.set {export: {quickOpen, quickPreset, quickPath}}`` as egui's: the popup opens under the
    header button and closes, the preset is one of its list (the file takes its extension), the
    path is fitted as a typed one; ``ui.inspect`` reads them back with the Export window closed,
    and a value the popup would not take is refused without a change."""
    from cfa4_qt.control import ControlError, ControlServer
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.quick_export import quick_presets

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    server = ControlServer(win)

    def call(method: str, params: dict[str, Any]) -> tuple[bool, Any]:
        out: list = []
        try:
            server.dispatch(method, params, lambda ok, v: out.append((ok, v)))
        except ControlError as e:  # what the channel answers with
            out.append((False, str(e)))
        assert out, method
        return out[0]

    def export() -> dict[str, Any]:
        good, r = call("ui.inspect", {})
        assert good, r
        return r["ui"]["export"]

    def ok(params: dict[str, Any]) -> dict[str, Any]:
        good, r = call("ui.set", {"export": params})
        assert good, (params, r)
        return r["ui"]["export"]

    def refused(params: dict[str, Any], text: str) -> None:
        before = export()
        good, r = call("ui.set", {"export": params})
        assert not good and text in str(r), (params, r)
        assert export() == before, f"{params} changed something"

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert export() == {"quickOpen": False, "quickPath": "", "quickPreset": ""}, "the Export window is closed"
        refused({"quickPath": "/tmp/x.mp4"}, "not been opened")
        refused({"quickOpen": True, "range": "custom"}, "not open")

        # open: under the header button, on the default preset and the sequence's name
        assert ok({"quickOpen": True})["quickOpen"] is True
        q = win.quick_export
        assert q.isVisible()
        assert wait_until(app, lambda: bool(q.presets) and bool(export()["quickPath"]), 10_000)
        e = export()
        name = str(win.sequence.get("name"))
        assert e["quickPreset"] == q.preset and Path(e["quickPath"]).stem == name

        # another preset of the list: the file takes its extension
        other = next(n for n in quick_presets(q.presets) if q._preset_info(n).get("extension") not in (None, q._ext()))
        ext = q._preset_info(other)["extension"]
        assert ok({"quickPreset": other})["quickPreset"] == other
        assert wait_until(app, lambda: export()["quickPath"].endswith(f"{name}.{ext}"), 10_000), export()
        refused({"quickPreset": "No such preset"}, "not in the Quick Export list")
        refused({"quickOpen": "yes"}, "true or false")
        # a path: fitted as a typed one (the preset's extension added)
        u = ok({"quickPath": str(tmp_path / "My Cut")})
        assert u["quickPath"] == str(tmp_path / f"My Cut.{ext}") and q.file.text() == f"My Cut.{ext}"
        assert q.location.text() == str(tmp_path)

        # closed: the choices stay for the next time
        u = ok({"quickOpen": False})
        assert u == {"quickOpen": False, "quickPath": str(tmp_path / f"My Cut.{ext}"), "quickPreset": other} and not q.isVisible()
        # with the Export window open, both sets of fields read back together
        win.open_export()
        assert wait_until(app, lambda: export().get("preset") is not None, 10_000)
        assert export()["quickPreset"] == other and export()["quickOpen"] is False
    finally:
        # the windows this test opened (the Export window, the popup) must not stay up: a visible
        # top-level window keeps later tests' window from being the active one (no tooltips)
        for w in QtWidgets.QApplication.topLevelWidgets():
            if w is not win and w.isVisible():
                w.close()
        win._quit_confirmed = True
        win.close()
        engine.stop()
