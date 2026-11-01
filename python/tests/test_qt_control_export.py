"""CFA4 Qt 6 GUI: the Export window's settings, open sections and Preset Manager as control-channel
state (``docs/qt-parity.md`` 1.11 with 4.2), as egui's ``ExportUi`` ``settings``,
``open_sections`` and ``manager``."""

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


def test_section_keys_as_egui():
    from cfa4_qt.panels.export import deep_merge
    from cfa4_qt.panels.export_effects import OPEN_SECTIONS, SECTION_KEYS

    assert SECTION_KEYS == ("video", "audio", "multiplexer", "captions", "effects", "metadata")
    assert OPEN_SECTIONS == ("video", "audio")
    assert deep_merge({"a": {"b": 1, "c": 2}, "d": 1}, {"a": {"b": 3}, "d": {"e": 1}}) == {"a": {"b": 3, "c": 2}, "d": {"e": 1}}


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_export_settings_sections_and_manager_through_the_control_channel(app, tmp_path: Path):
    """``ui.set {export: {settings, openSections, manager}}`` as egui's: a settings patch is
    deep-merged and resolved by the engine (one it refuses is taken back), the sections open and
    close as their titles do, the Preset Manager opens with its search, filter, selection and save
    name and null closes it; a value the window would not take is refused without a change."""
    from cfa4_qt.control import ControlError, ControlServer
    from cfa4_qt.main_window import MainWindow

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
        win.open_export()
        d = win.export_dialog
        assert wait_until(app, lambda: bool(d.resolved_settings) and bool(d.presets), 20_000)
        e = export()
        assert e["settings"] == d.resolved_settings and e["manager"] is None

        # sections: Video and Audio open, the rest closed; their titles and ui.set open and close them
        assert e["openSections"] == ["video", "audio"]
        assert d.captions.body.isHidden() and not d.sections["video"].body.isHidden()
        d.sections["effects"].head.click()
        assert export()["openSections"] == ["video", "audio", "effects"] and not d.effects.body.isHidden()
        assert ok({"openSections": ["captions", "video"]})["openSections"] == ["video", "captions"]
        assert d.sections["audio"].body.isHidden() and not d.captions.body.isHidden() and d.effects.body.isHidden()
        refused({"openSections": ["video", "sideways"]}, "openSections takes")

        # settings: merged into the export settings, which the engine resolves
        ok({"format": "wav"})
        assert wait_until(app, lambda: d.resolved_settings.get("format") in ("wav", "Wav", "WAV") or "wav" in str(d.resolved_settings.get("format")).lower(), 10_000)
        ok({"settings": {"audio": {"sample_rate": 96000}}})
        assert wait_until(app, lambda: (export()["settings"].get("audio") or {}).get("sample_rate") == 96000, 10_000), export()["settings"].get("audio")
        assert d.params()["settings"]["audio"]["sample_rate"] == 96000
        # one the engine refuses is taken back
        ok({"settings": {"audio": {"sample_rate": 384_001}}})
        assert wait_until(app, lambda: d.agent_settings["audio"]["sample_rate"] == 96000, 10_000)
        assert (export()["settings"].get("audio") or {}).get("sample_rate") == 96000
        refused({"settings": {"nonsense": 1}}, "no setting 'nonsense'")
        refused({"settings": 3}, "export.settings takes")
        # a preset starts the settings again
        ok({"preset": str(d.presets[0]["name"])})
        assert d.agent_settings == {} and "audio" not in (d.params().get("settings") or {})

        # the Preset Manager
        some = str(d.presets[0]["name"])
        u = ok({"manager": {"query": some[:3], "selected": some, "saveName": "Mine"}})
        assert u["manager"] == {"query": some[:3], "favoritesOnly": False, "selected": some, "saveName": "Mine"}
        m = d.preset_manager
        assert m is not None and m.isVisible() and m.search.text() == some[:3] and m.save.isEnabled()
        assert ok({"manager": {"favoritesOnly": True}})["manager"]["favoritesOnly"] is True
        refused({"manager": {"selected": "No such preset"}}, "no preset named")
        refused({"manager": {"zoom": 2}}, "unknown field")
        assert ok({"manager": None})["manager"] is None and not m.isVisible()
        refused({"manager": {"selected": "No such preset"}}, "no preset named")
        assert not m.isVisible(), "a refused manager opens nothing"
    finally:
        # the windows this test opened (the Export window, the Preset Manager) must not stay up:
        # a visible top-level window keeps later tests' window from being the active one
        for w in QtWidgets.QApplication.topLevelWidgets():
            if w is not win and w.isVisible():
                w.close()
        win._quit_confirmed = True
        win.close()
        engine.stop()
