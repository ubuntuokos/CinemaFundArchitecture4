"""Chapter 8 of the control-surface plan: the Qt panels and the menu items that open them.

The device names and counters in the offscreen shots are fixtures the stub returns. They are not
measurements of a connected controller.
"""

from __future__ import annotations

import os
import time
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore", exc_type=ImportError)
QtGui = pytest.importorskip("PySide6.QtGui", exc_type=ImportError)
QtWidgets = pytest.importorskip("PySide6.QtWidgets", exc_type=ImportError)

from cfa4_qt.panels.settings import MENU, PreferencesDialog  # noqa: E402
from cfa4_qt.window_menu import Workspaces, build_window_menu, window_commands  # noqa: E402

from test_qt_control_surfaces import (  # noqa: E402
    COMMANDS,
    DEVICES,
    NANO,
    StubEngine,
    find,
)

SHOTS = Path("/opt/cursor/artifacts")
CC41 = {"midi": {"channel": 0, "kind": "cc", "number": 41}}


class GuiEngine(StubEngine):
    """The chapter-8 replies on top of the device and Learn stub."""

    def __init__(self, devices: dict[str, Any]) -> None:
        super().__init__(devices)
        self.state = {
            "learn": {"state": "idle"},
            "kernelDropped": 3,
            "devices": [
                {"device": NANO, "state": "online", "received": 4, "dispatched": 2, "coalesced": 1, "stalls": 0, "errors": 0},
            ],
        }
        self.warnings = {"devices": [{"device": NANO, "warnings": ["assignment of a \"tsl\" control is disabled: unknown"]}]}
        self.audit = [{"device": NANO, "control": CC41, "problem": "conflict", "detail": "more than one binding"}]
        self.reload = {
            "warnings": ["active-midi.json: broken; keeping preferences and the built-in profile"],
            "devices": [{"device": NANO, "enabled": True, "profile": "cfa4.generic-midi", "bindings": [],
                         "pickupThreshold7": 12, "pickupThreshold14": 200, "shuttleSound": "mute"}],
        }

    def call(self, method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        n = super().call(method, params, on_reply, on_error)
        if method == "surface.state" and on_reply:
            on_reply(self.state, None)
        if method == "surface.enable" and on_reply:
            on_reply({"device": (params or {}).get("device"), "enabled": True}, None)
        return n

    def execute(self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        n = super().execute(command, params, on_reply, on_error)
        replies = {
            "surface.profile.warnings": self.warnings,
            "surface.audit": self.audit,
            "controlSurfaces.reloadProfiles": self.reload,
        }
        if command in replies and on_reply:
            on_reply(replies[command], None)
        return n


@pytest.fixture(scope="module")
def app() -> Any:
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def _dialog(engine: GuiEngine) -> PreferencesDialog:
    return PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)


def _save(widget: Any, name: str) -> None:
    SHOTS.mkdir(parents=True, exist_ok=True)
    widget.grab().save(str(SHOTS / name))


def test_devices_show_state_enable_and_counters(app: Any) -> None:
    engine = GuiEngine(DEVICES)
    d = _dialog(engine)
    d.show()
    app.processEvents()
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.1.state").text() == "Online"
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.2.state").text() == "Error"
    counters = find(d, QtWidgets.QLabel, "settings.controlSurfaces.1.counters").text()
    assert "4 received" in counters and "2 dispatched" in counters and "1 coalesced" in counters
    assert "0 stalls" in counters and "0 errors" in counters
    assert "3" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.kernelDropped").text()
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.1.enable").click()
    enabled = [p for m, p in engine.sent if m == "surface.enable"]
    assert enabled == [{"device": NANO, "enabled": True}]
    _save(find(d, QtWidgets.QGroupBox, "settings.controlSurfaces.devices"), "control-surfaces-devices.png")
    d.close()


def test_learn_panel_shows_state_hint_and_timers(app: Any) -> None:
    engine = GuiEngine(DEVICES)
    d = _dialog(engine)
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.state").text() == "Idle"
    assert find(d, QtWidgets.QSpinBox, "settings.controlSurfaces.assign.firstMs").value() == 7000
    assert find(d, QtWidgets.QSpinBox, "settings.controlSurfaces.assign.silenceMs").value() == 1500
    command = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.assign.command")
    command.setCurrentIndex(command.findData("playback.toggle"))
    command.activated.emit(command.currentIndex())
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.state").text() == "Learning"
    engine.event.emit({"event": "surfaceLearn", "state": "pending", "device": NANO,
                       "captured": {"device": NANO, "control": CC41, "mode": "cc"}})
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.state").text() == "Pending"
    assert "cc" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.hint").text()
    assert not find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.confirm").isHidden()
    assert not find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.reject").isHidden()
    find(d, QtWidgets.QSpinBox, "settings.controlSurfaces.assign.firstMs").setValue(8000)
    find(d, QtWidgets.QSpinBox, "settings.controlSurfaces.assign.silenceMs").setValue(1200)
    _save(find(d, QtWidgets.QGroupBox, "settings.controlSurfaces.assign"), "control-surfaces-assignments.png")
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    values = [p for m, p in engine.sent if m == "prefs.set"][-1]["values"]
    assert values["controlSurfaces.learnFirstMs"] == 8000
    assert values["controlSurfaces.learnSilenceMs"] == 1200


def test_profile_editor_pickup_mute_load_and_warnings(app: Any) -> None:
    engine = GuiEngine(DEVICES)
    d = _dialog(engine)
    assert "Generic MIDI" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.builtin").text()
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.user").text()
    assert "project profile" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.project").text().lower()
    assert "profile file" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.save").text().lower()
    assert "more than one binding" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.conflicts").text()
    assert "tsl" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.warnings.text").text()
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.corrupt").isHidden()
    find(d, QtWidgets.QSpinBox, "settings.controlSurfaces.profile.threshold7").setValue(12)
    find(d, QtWidgets.QSpinBox, "settings.controlSurfaces.profile.threshold14").setValue(200)
    find(d, QtWidgets.QCheckBox, "settings.controlSurfaces.profile.shuttleMute").setChecked(True)
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.profile.load").click()
    corrupt = find(d, QtWidgets.QLabel, "settings.controlSurfaces.profile.corrupt")
    assert not corrupt.isHidden()
    assert "damaged" in corrupt.text() and "keeping preferences" in corrupt.text()
    assert find(d, QtWidgets.QCheckBox, "settings.controlSurfaces.profile.shuttleMute").isChecked()
    _save(find(d, QtWidgets.QGroupBox, "settings.controlSurfaces.profile"), "control-surfaces-profile.png")
    _save(find(d, QtWidgets.QGroupBox, "settings.controlSurfaces.warnings"), "control-surfaces-warnings.png")
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    devices = [p for m, p in engine.sent if m == "prefs.set"][-1]["values"]["controlSurfaces.devices"]
    nano = next(row for row in devices if row["device"] == NANO)
    assert nano["pickupThreshold7"] == 12
    assert nano["pickupThreshold14"] == 200
    assert nano["shuttleSound"] == "mute"


def test_midi_monitor_filters(app: Any) -> None:
    engine = GuiEngine(DEVICES)
    d = _dialog(engine)
    d.show()
    app.processEvents()
    find(d, QtWidgets.QCheckBox, "settings.controlSurfaces.monitor.clock").setChecked(True)
    find(d, QtWidgets.QCheckBox, "settings.controlSurfaces.monitor.sense").setChecked(True)
    find(d, QtWidgets.QCheckBox, "settings.controlSurfaces.monitor.mtc").setChecked(True)
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.monitor.toggle").click()
    params = [p for m, p in engine.sent if m == "surface.monitor"][-1]
    assert params["enabled"] is True
    assert params["filter"] == {"clock": True, "sense": True, "mtc": True}
    device = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.monitor.device")
    device.setCurrentIndex(device.findData(NANO))
    device.activated.emit(device.currentIndex())
    assert [p for m, p in engine.sent if m == "surface.monitor"][-1]["device"] == NANO
    engine.event.emit({"event": "surfaceMonitor", "messages": [
        {"device": NANO, "at": 1_000_000_000, "raw": "B0 10 40", "decoded": "Ch 1 CC 16 = 64"},
        {"device": NANO, "at": 1_100_000_000, "raw": "90 3C 64", "decoded": "Ch 1 Note On 60"},
    ]})
    log = find(d, QtWidgets.QPlainTextEdit, "settings.controlSurfaces.monitor.log")
    assert len(log.toPlainText().splitlines()) == 2
    find(d, QtWidgets.QLineEdit, "settings.controlSurfaces.monitor.filter").setText("CC")
    assert log.toPlainText().splitlines() == [line for line in d.monitor_lines if "CC" in line]
    _save(find(d, QtWidgets.QGroupBox, "settings.controlSurfaces.monitor"), "control-surfaces-monitor.png")
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.monitor.clear").click()
    assert log.toPlainText() == ""
    d.close()


def test_menus_open_the_control_surface_page(app: Any, tmp_path: Path) -> None:
    assert ("controlSurfaces", "Control Surfaces…") in MENU
    ids = {row[0]: row[2] for row in window_commands()}
    assert ids["window.controlSurfaces"] == ["Window"]
    assert ids["window.midiMonitor"] == ["Window"]
    assert ids["window.surfaceAssignments"] == ["Window"]

    class Host(QtWidgets.QMainWindow):
        def __init__(self) -> None:
            super().__init__()
            self.opened: list[tuple[str, str]] = []

        def open_settings(self, page: str = "general", section: str = "") -> None:
            self.opened.append((page, section))

    host = Host()
    menu = QtWidgets.QMenu(host)
    menu.setObjectName("menu.Window")
    settings = QtCore.QSettings(str(tmp_path / "ui.ini"), QtCore.QSettings.Format.IniFormat)
    spaces = Workspaces(host, settings, QtCore.QByteArray(), 1)
    reset = QtGui.QAction("Reset", host)
    build_window_menu(host, menu, {}, QtWidgets.QWidget(), spaces, lambda _b: None, reset)
    host.show()
    menu.show()
    app.processEvents()
    _save(menu, "menu-window-control-surfaces.png")
    for name, section in (
        ("window.controlSurfaces", "settings.controlSurfaces.devices"),
        ("window.midiMonitor", "settings.controlSurfaces.monitor"),
        ("window.surfaceAssignments", "settings.controlSurfaces.assign"),
    ):
        host.findChild(QtGui.QAction, name).trigger()
        assert host.opened[-1] == ("controlSurfaces", section)

    edit = QtWidgets.QMenu(host)
    edit.setObjectName("menu.Edit.Preferences")
    edit.setTitle("Preferences")
    for page, label in MENU:
        action = edit.addAction(label)
        action.setObjectName(f"app.settings.{page}")
        action.triggered.connect(lambda _c=False, pg=page: host.open_settings(pg))
    edit.show()
    app.processEvents()
    _save(edit, "menu-edit-preferences-control-surfaces.png")
    host.findChild(QtGui.QAction, "app.settings.controlSurfaces").trigger()
    assert host.opened[-1] == ("controlSurfaces", "")
    host.close()


def _engine_binary() -> Path | None:
    from cfa4_qt.engine import locate_engine

    return locate_engine()


@pytest.mark.skipif(_engine_binary() is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_built_engine_serves_the_panels(app: Any, tmp_path: Path) -> None:
    """The built cfa4-cli answers the panel's calls, including an empty machine (no invented counts)."""
    from cfa4_qt.engine import EngineClient
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(_engine_binary(), ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    engine.start()
    got: dict[str, Any] = {}
    try:
        deadline = time.monotonic() + 60
        while len(win.registry.commands) < 100 and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert win.findChild(QtGui.QAction, "app.settings.controlSurfaces") is not None
        assert win.findChild(QtGui.QAction, "window.midiMonitor") is not None
        assert win.findChild(QtGui.QAction, "window.surfaceAssignments") is not None
        win.findChild(QtGui.QAction, "window.controlSurfaces").trigger()
        app.processEvents()
        dialog = win.settings_dialog
        assert dialog.focus_section == "settings.controlSurfaces.devices"
        deadline = time.monotonic() + 30
        while dialog.findChild(QtWidgets.QPushButton, "settings.controlSurfaces.refresh") is None and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert dialog.findChild(QtWidgets.QGroupBox, "settings.controlSurfaces.monitor") is not None
        assert dialog.findChild(QtWidgets.QGroupBox, "settings.controlSurfaces.profile") is not None
        assert dialog.findChild(QtWidgets.QGroupBox, "settings.controlSurfaces.assign") is not None
        assert dialog.findChild(QtWidgets.QGroupBox, "settings.controlSurfaces.warnings") is not None
        engine.call("surface.state", {}, on_reply=lambda r, _b: got.update(state=r))
        engine.execute("controlSurfaces.reloadProfiles", {}, on_reply=lambda r, _b: got.update(reload=r))
        engine.execute("surface.profile.warnings", {}, on_reply=lambda r, _b: got.update(warnings=r))
        engine.execute("surface.audit", {}, on_reply=lambda r, _b: got.update(audit=r))
        deadline = time.monotonic() + 30
        while len(got) < 4 and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert set(got) == {"state", "reload", "warnings", "audit"}
        assert isinstance(got["state"]["kernelDropped"], int)
        assert isinstance(got["state"]["devices"], list)
        assert isinstance(got["reload"]["warnings"], list)
        assert isinstance(got["reload"]["devices"], list)
        assert isinstance(got["warnings"]["devices"], list)
        assert isinstance(got["audit"], list)
        dialog.show()
        app.processEvents()
        _save(dialog, "control-surfaces-editor.png")
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()
