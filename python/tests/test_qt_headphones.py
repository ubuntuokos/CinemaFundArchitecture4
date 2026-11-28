"""CFA4 Qt 6 GUI headphone monitoring (``docs/plans/CFA4-BINAURAL-MONITOR-001.md`` B-04): the
transport's headphone button and the wrench menu entry turn Settings ▸ Audio Hardware ▸
Monitoring to Headphones, the monitors then ask ``audio.render`` for the binaural render with the
chosen HRTF set, room and LFE level, and say what the headphones play. Against the real Rust
engine when it is built."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import EngineClient, locate_engine  # noqa: E402

REAL = locate_engine()


@pytest.fixture(scope="module")
def app() -> Any:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch: pytest.MonkeyPatch) -> None:
    def refuse(self: Any, *_args: Any, **_kwargs: Any) -> None:
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", refuse)


def wait_until(app: Any, predicate: Any, timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


def test_both_monitors_have_a_headphone_button() -> None:
    from cfa4_qt.panels.transport import PROGRAM_BUTTONS, SOURCE_BUTTONS

    for buttons in (PROGRAM_BUTTONS, SOURCE_BUTTONS):
        assert [b.command for b in buttons].count("monitor.headphones") == 1


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_headphones_from_the_button_to_the_ears(app: Any, tmp_path: Path) -> None:
    """The Program's headphone button saves Monitoring = Headphones; playback then asks for the
    binaural render (built-in set, the settings' room and LFE level), the engine answers with the
    ears and the monitor shows the layout heard; a missing HRTF file is reported once in Events and
    the speakers play; the button again turns the headphones off."""
    from cfa4_qt.main_window import MainWindow

    assert REAL is not None
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1400, 900)
    win.show()
    engine.start()
    mon = win.monitor
    asked: list[dict[str, Any]] = []
    heard: list[dict[str, Any]] = []
    messages: list[str] = []
    mon.message.connect(messages.append)
    real_call = engine.call

    def spy(method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        if method != "audio.render":
            return real_call(method, params, on_reply, on_error)
        asked.append(dict(params or {}))

        def got(result: Any, data: bytes | None) -> None:
            if isinstance(result, dict) and isinstance(result.get("monitor"), dict):
                heard.append(result["monitor"])
            if on_reply is not None:
                on_reply(result, data)

        return real_call(method, params, got, on_error)

    engine.call = spy  # type: ignore[method-assign]

    def play_until(predicate: Any, timeout_ms: int = 30_000) -> bool:
        mon.seek(0)
        mon.play(1.0)
        ok = wait_until(app, predicate, timeout_ms)
        mon.stop()
        return ok

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert mon.headphones is None and win.source_monitor.headphones is None
        button = mon.transport.buttons["headphones"]
        assert button.objectName() == "program.transport.monitor.headphones"

        # the button turns the headphones on, for both monitors, with the saved settings
        button.click()
        assert wait_until(app, lambda: mon.headphones is not None and win.source_monitor.headphones is not None, 10_000)
        assert mon.headphones == {"mode": "binaural", "hrtf": "builtin", "room": 0.35, "lfe_db": -10.0}
        assert play_until(lambda: any(h.get("mode") == "binaural" for h in heard), 60_000), heard
        assert all(p.get("monitor") == mon.headphones for p in asked if "speed" not in p), asked
        binaural = next(h for h in heard if h.get("mode") == "binaural")
        assert binaural["set"] and isinstance(binaural["layout"], int)
        assert mon.heard_label.isVisible() and mon.heard_label.text().endswith("· binaural"), mon.heard_label.text()

        # a missing set file: reported once, the speakers play
        bad = tmp_path / "gone.maudhrtf"
        done: list[Any] = []
        engine.execute("prefs.set", {"values": {"audioHardware.hrtfSet": str(bad), "audioHardware.headphoneRoom": "off"}}, on_reply=lambda r, _d: done.append(r))
        assert wait_until(app, lambda: done, 10_000)
        win.load_audio_output()
        assert wait_until(app, lambda: mon.headphones is not None and mon.headphones.get("hrtf") == str(bad), 10_000)
        assert mon.headphones is not None and mon.headphones.get("room") == 0.0
        heard.clear()
        assert play_until(lambda: any("error" in h for h in heard), 60_000), heard
        wait_until(app, lambda: False, 300)
        assert len([m for m in messages if "gone.maudhrtf" in m]) == 1, messages
        assert not mon.heard_label.isVisible()

        # the wrench's entry shows the state; the button again turns them off
        button.click()
        assert wait_until(app, lambda: mon.headphones is None and win.source_monitor.headphones is None, 10_000)
        asked.clear()
        assert play_until(lambda: len(asked) >= 2, 30_000)
        assert all("monitor" not in p for p in asked), asked
    finally:
        engine.call = real_call  # type: ignore[method-assign]
        mon.stop()
        win._quit_confirmed = True
        win.close()
        engine.stop()


def test_export_window_binaural_option_sends_the_headphone_settings(app: Any) -> None:
    """B-05c: Export ▸ Audio ▸ Binaural stereo puts the Headphones settings (HRTF set, room, LFE
    level) into ``settings.audio.binaural``; unticked, or without audio, nothing is sent."""
    from cfa4_qt.panels.export import ExportDialog

    class Engine:
        running = False

        def execute(self, command: str, params: Any = None, on_reply: Any = None, on_error: Any = None) -> int:
            if command == "prefs.get" and on_reply is not None:
                on_reply({"value": {"hrtfSet": "/x/own.sofa", "headphoneRoom": "medium", "headphoneLfeDb": -6}}, None)
            return 0

    dlg = ExportDialog(Engine())  # type: ignore[arg-type]
    assert dlg.audio_binaural.objectName() == "export.audio.binaural"
    assert not dlg.audio_binaural.isChecked()
    assert "binaural" not in dlg.params().get("settings", {}).get("audio", {})
    dlg._load_audio_prefs()
    dlg.audio_binaural.setChecked(True)
    binaural = dlg.params()["settings"]["audio"]["binaural"]
    assert binaural == {"hrtf": "/x/own.sofa", "room": 0.75, "lfe_db": -6.0}
    dlg.audio.setChecked(False)  # Export Audio off: the option is greyed out and not sent
    assert not dlg.audio_binaural.isEnabled()
    assert "binaural" not in dlg.params().get("settings", {}).get("audio", {})
