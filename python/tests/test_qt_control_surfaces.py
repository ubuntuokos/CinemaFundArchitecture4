"""Settings ▸ Control Surfaces (CFA4-CONTROL-SURFACE-001, docs/adr/ADR-0012-control-surface.md):
the page lists the MIDI controllers and jog/shuttle devices `surface.devices` reports, each with
Use and Profile; the choices are a draft that OK applies as `controlSurfaces.devices`. The MIDI
Monitor below shows the `surfaceMonitor` events while it is on. Runs when
PySide6 is installed (offscreen), with a stub engine, and against a built `cfa4-cli` when there is one.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtWidgets = pytest.importorskip("PySide6.QtWidgets", exc_type=ImportError)

from cfa4_qt.panels.settings import PreferencesDialog  # noqa: E402

SCHEMA = {
    "groups": [{"id": "hardware", "title": "Hardware & Devices"}],
    "categories": [
        {
            "id": "controlSurfaces",
            "title": "Control Surfaces",
            "group": "hardware",
            "fields": [],
            "rows": [
                {"type": "field", "field": {"key": "controlSurfaces.enabled", "label": "Use control surfaces (MIDI controllers, jog/shuttle devices)",
                                            "kind": {"type": "bool"}, "wired": False, "value": True}},
                {"type": "custom", "id": "controlSurfaces"},
                {"type": "note", "text": "Mapping comes later."},
            ],
            "defaults": {"enabled": True, "devices": []},
        }
    ],
}
PROFILES = {
    "profiles": [
        {"id": "cfa4.shuttlepro", "name": "CFA4 ShuttlePRO", "driver": "hid-shuttle", "default": True},
        {"id": "cfa4.shuttlepro.kdenlive", "name": "Kdenlive-style ShuttlePRO", "driver": "hid-shuttle", "default": False},
        {"id": "cfa4.generic-midi", "name": "Generic MIDI", "driver": "generic-midi", "default": True},
    ]
}
DEVICES = {
    "enabled": True,
    "devices": [
        {"device": "midi:nanoKONTROL2:nanoKONTROL2 _ CTRL", "name": "nanoKONTROL2", "protocol": "midi", "driver": "generic-midi",
         "state": "online", "port": "24:0", "enabled": True, "profile": "cfa4.generic-midi"},
        {"device": "hid:0b33:0030:usb-0000:00:14.0-3/input0", "name": "Contour ShuttlePRO v2", "protocol": "hid",
         "driver": "hid-shuttle", "state": "error", "port": "/dev/hidraw3", "enabled": True, "profile": "cfa4.shuttlepro",
         "error": "no permission to read the device: install CFA4's udev rule (uaccess) and plug it in again"},
    ],
    "problems": [],
}


class Notifications:
    """Stands in for the engine client's ``event`` signal."""

    def __init__(self) -> None:
        self.slots: list[Any] = []

    def connect(self, slot) -> None:
        self.slots.append(slot)

    def emit(self, message: dict[str, Any]) -> None:
        for slot in self.slots:
            slot(message)


class StubEngine:
    """Answers at once, as the engine would; remembers every request."""

    def __init__(self, devices: dict[str, Any], monitor_error: str = "", learn_error: str = "") -> None:
        self.devices = devices
        self.monitor_error = monitor_error
        self.learn_error = learn_error
        self.event = Notifications()
        self.sent: list[tuple[str, dict[str, Any]]] = []
        self.values: dict[str, Any] = {"controlSurfaces": {"enabled": True, "devices": []}}

    def call(self, method: str, params: dict[str, Any] | None = None, on_reply=None, on_error=None) -> int:
        self.sent.append((method, params or {}))
        if method == "surface.devices" and on_reply:
            on_reply(self.devices, None)
        if method == "surface.monitor":
            if self.monitor_error and (params or {}).get("enabled"):
                if on_error:
                    on_error(self.monitor_error)
            elif on_reply:
                on_reply({"enabled": bool((params or {}).get("enabled"))}, None)
        if method == "surface.learn":
            if self.learn_error:
                if on_error:
                    on_error(self.learn_error)
            elif on_reply:
                on_reply({"state": "learning"}, None)
        return len(self.sent)

    def execute(self, command: str, params: dict[str, Any] | None = None, on_reply=None, on_error=None) -> int:
        self.sent.append((command, params or {}))
        replies = {"prefs.schema": SCHEMA, "prefs.get": {"values": self.values}, "controlSurfaces.profiles": PROFILES, "prefs.set": {}}
        if command in replies and on_reply:
            on_reply(replies[command], None)
        return len(self.sent)


@pytest.fixture(scope="module")
def app():
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def find(d, kind, name):
    w = d.findChild(kind, name)
    assert w is not None, name
    return w


def test_control_surfaces_page_lists_devices_and_applies_choices(app):
    engine = StubEngine(DEVICES)
    d = PreferencesDialog(None, engine, "controlSurfaces")
    assert d.page == "controlSurfaces"
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.1.name").text() == "nanoKONTROL2"
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.2.name").text() == "Contour ShuttlePRO v2"
    assert "24:0" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.1.name").toolTip()
    # the device that cannot be read says why
    assert "udev" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.2.error").text()
    # a device offers only its driver's profiles
    midi = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.1.profile")
    assert [midi.itemData(i) for i in range(midi.count())] == ["cfa4.generic-midi"]
    shuttle = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.2.profile")
    assert [shuttle.itemData(i) for i in range(shuttle.count())] == ["cfa4.shuttlepro", "cfa4.shuttlepro.kdenlive"]
    assert shuttle.currentData() == "cfa4.shuttlepro"
    # choices are a draft: turn the MIDI device off, pick the Kdenlive-style profile, then OK
    find(d, QtWidgets.QCheckBox, "settings.controlSurfaces.1.enabled").setChecked(False)
    shuttle.setCurrentIndex(1)
    shuttle.activated.emit(1)
    assert not [m for m, _p in engine.sent if m == "prefs.set"]
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    sets = [p for m, p in engine.sent if m == "prefs.set"]
    assert len(sets) == 1
    devices = sets[0]["values"]["controlSurfaces.devices"]
    assert {"device": "midi:nanoKONTROL2:nanoKONTROL2 _ CTRL", "enabled": False, "profile": "", "bindings": []} in devices
    assert {"device": "hid:0b33:0030:usb-0000:00:14.0-3/input0", "enabled": True, "profile": "cfa4.shuttlepro.kdenlive", "bindings": []} in devices


def test_control_surfaces_page_without_devices_and_refresh(app):
    engine = StubEngine({"enabled": True, "devices": [], "problems": ["MIDI: no ALSA sequencer"]})
    d = PreferencesDialog(None, engine, "controlSurfaces")
    assert "No MIDI controller" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.empty").text()
    assert any("ALSA sequencer" in w.text() for w in d.findChildren(QtWidgets.QLabel))
    before = len([m for m, _p in engine.sent if m == "surface.devices"])
    engine.devices = DEVICES
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.refresh").click()
    assert len([m for m, _p in engine.sent if m == "surface.devices"]) == before + 1
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.1.name").text() == "nanoKONTROL2"
    # nothing changed: OK sends no prefs.set
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    assert not [m for m, _p in engine.sent if m == "prefs.set"]


def monitor_calls(engine: StubEngine) -> list[bool]:
    return [bool(p.get("enabled")) for m, p in engine.sent if m == "surface.monitor"]


def test_midi_monitor_shows_messages_until_stopped(app):
    engine = StubEngine(DEVICES)
    d = PreferencesDialog(None, engine, "controlSurfaces")
    toggle = find(d, QtWidgets.QPushButton, "settings.controlSurfaces.monitor.toggle")
    log = find(d, QtWidgets.QPlainTextEdit, "settings.controlSurfaces.monitor.log")
    assert toggle.text() == "Start Monitor" and not monitor_calls(engine)
    # messages before Start are not shown
    engine.event.emit({"event": "surfaceMonitor", "messages": [{"device": "midi:x", "at": 0, "raw": "B0 01 02", "decoded": "Ch 1 CC 1 = 2"}]})
    assert log.toPlainText() == ""
    toggle.click()
    assert monitor_calls(engine) == [True]
    assert toggle.isChecked() and toggle.text() == "Stop Monitor"
    engine.event.emit({"event": "surfaceMonitor", "dropped": 0, "messages": [
        {"device": "midi:nanoKONTROL2:nanoKONTROL2 _ CTRL", "at": 1_500_000_000, "raw": "B0 10 40", "decoded": "Ch 1 CC 16 = 64"},
        {"device": "midi:Other:Port", "at": 1_600_000_000, "raw": "90 3C 64", "decoded": "Ch 1 Note On 60 vel 100"},
    ]})
    engine.event.emit({"event": "surfaceMonitor", "messages": [], "dropped": 7})
    engine.event.emit({"event": "projectChanged", "revision": 3})
    lines = log.toPlainText().splitlines()
    assert len(lines) == 3
    assert "nanoKONTROL2" in lines[0] and "B0 10 40" in lines[0] and "Ch 1 CC 16 = 64" in lines[0] and "1.500" in lines[0]
    assert "Other:Port" in lines[1]
    assert "7 messages not shown" in lines[2]
    # the lines survive a rebuilt page (Refresh)
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.refresh").click()
    log = find(d, QtWidgets.QPlainTextEdit, "settings.controlSurfaces.monitor.log")
    assert len(log.toPlainText().splitlines()) == 3
    assert find(d, QtWidgets.QPushButton, "settings.controlSurfaces.monitor.toggle").text() == "Stop Monitor"
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.monitor.clear").click()
    assert log.toPlainText() == "" and d.monitor_lines == []
    # closing the dialog stops reading; the monitor sends no prefs.set
    find(d, QtWidgets.QPushButton, "settings.cancel").click()
    assert monitor_calls(engine) == [True, False]
    assert not [m for m, _p in engine.sent if m == "prefs.set"]


def test_midi_monitor_says_why_it_cannot_start(app):
    engine = StubEngine(DEVICES, monitor_error="MIDI: no ALSA sequencer (No such file or directory)")
    d = PreferencesDialog(None, engine, "controlSurfaces")
    error = find(d, QtWidgets.QLabel, "settings.controlSurfaces.monitor.error")
    assert error.isHidden()
    toggle = find(d, QtWidgets.QPushButton, "settings.controlSurfaces.monitor.toggle")
    toggle.click()
    assert not toggle.isChecked() and toggle.text() == "Start Monitor"
    assert "ALSA sequencer" in error.text() and not error.isHidden()
    find(d, QtWidgets.QPushButton, "settings.cancel").click()
    assert monitor_calls(engine) == [True], "nothing to stop"


COMMANDS = [("playback.toggle", "Play/Stop"), ("edit.undo", "Undo"), ("markers.markIn", "Mark In")]
NANO = "midi:nanoKONTROL2:nanoKONTROL2 _ CTRL"
SHUTTLE = "hid:0b33:0030:usb-0000:00:14.0-3/input0"
CC41 = {"midi": {"channel": 0, "kind": "cc", "number": 41}}


def rows(d, what: str) -> list[str]:
    return [w.text() for w in sorted((w for w in d.findChildren(QtWidgets.QLabel) if w.objectName().startswith("settings.controlSurfaces.assign.")
                                      and w.objectName().endswith(f".{what}")), key=lambda w: w.objectName())]


def test_learn_assigns_a_button_and_ok_saves_it(app):
    engine = StubEngine(DEVICES)
    d = PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)
    # MIDI controllers and jog/shuttle devices are offered
    device = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.assign.device")
    assert [device.itemData(i) for i in range(device.count())] == [NANO, SHUTTLE]
    assert find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.none")
    command = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.assign.command")
    keys = [command.itemData(i) for i in range(command.count())]
    assert keys[:3] == ["markers.markIn", "playback.toggle", "edit.undo"], "by label"
    assert keys[3] is None, "a separator before the mixer strips"
    assert keys[4:6] == ["mixer.stripVolume|Mix", "mixer.stripVolume|A1"] and "mixer.stripPan|A16" in keys
    command.setCurrentIndex(command.findData("playback.toggle"))
    command.activated.emit(command.currentIndex())
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    assert ("surface.learn", {"device": NANO}) in engine.sent
    learn = find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn")
    assert learn.isChecked() and learn.text() == "Cancel Learn"
    # another device's press, or the monitor's events, assign nothing
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": "midi:Other:Port", "control": CC41})
    assert d.learning
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": NANO, "control": CC41, "label": "Ch 1 CC 41"})
    assert not d.learning
    assert rows(d, "control") == ["Ch 1 CC 41"] and rows(d, "command") == ["Play/Stop"]
    assert "Ch 1 CC 41 runs Play/Stop" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.status").text()
    # learning the same control again replaces its command
    command = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.assign.command")
    assert command.currentData() == "playback.toggle", "the picked command stays"
    command.setCurrentIndex(command.findData("edit.undo"))
    command.activated.emit(command.currentIndex())
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": NANO, "control": CC41})
    assert rows(d, "command") == ["Undo"]
    note = {"midi": {"channel": 9, "kind": "note", "number": 36}}
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": NANO, "control": note})
    assert rows(d, "control") == ["Ch 1 CC 41", "Ch 10 Note 36"]
    find(d, QtWidgets.QToolButton, "settings.controlSurfaces.assign.1.remove").click()
    assert rows(d, "control") == ["Ch 10 Note 36"]
    assert not [m for m, _p in engine.sent if m == "prefs.set"], "a draft until OK"
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    sets = [p for m, p in engine.sent if m == "prefs.set"]
    assert sets[0]["values"]["controlSurfaces.devices"] == [
        {"device": NANO, "enabled": True, "profile": "", "bindings": [{"control": note, "command": "edit.undo"}]}]
    assert "surface.learnCancel" not in [m for m, _p in engine.sent], "nothing was waiting"


def test_learn_cancel_errors_and_close(app):
    engine = StubEngine(DEVICES, learn_error="MIDI: no ALSA sequencer (No such file or directory)")
    d = PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    assert not d.learning
    assert "ALSA sequencer" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.status").text()
    engine.learn_error = ""
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()  # Cancel Learn
    assert [m for m, _p in engine.sent if m.startswith("surface.learn")] == ["surface.learn", "surface.learn", "surface.learnCancel"]
    # a press after cancelling assigns nothing
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": NANO, "control": CC41})
    assert rows(d, "control") == []
    # the sequencer going away ends Learn with its reason
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    engine.event.emit({"event": "surfaceError", "code": "alsa_unavailable", "error": "MIDI: the sequencer stopped"})
    assert not d.learning and "sequencer stopped" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.status").text()
    # closing while Learn waits cancels it
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    find(d, QtWidgets.QPushButton, "settings.cancel").click()
    assert [m for m, _p in engine.sent if m.startswith("surface.learn")][-1] == "surface.learnCancel"


def test_shuttlepro_buttons_are_learned_too(app):
    engine = StubEngine(DEVICES)
    d = PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)
    device = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.assign.device")
    device.setCurrentIndex(device.findData(SHUTTLE))
    device.activated.emit(device.currentIndex())
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    assert ("surface.learn", {"device": SHUTTLE}) in engine.sent
    button = {"hid": {"name": "button3", "usagePage": 0, "usage": 0}}
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": SHUTTLE, "control": button, "label": "button3"})
    assert rows(d, "control") == ["Button 3"]
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    saved = [p for m, p in engine.sent if m == "prefs.set"][0]["values"]["controlSurfaces.devices"]
    assert saved == [{"device": SHUTTLE, "enabled": True, "profile": "", "bindings": [{"control": button, "command": "markers.markIn"}]}]


def test_a_fader_is_learned_onto_a_mixer_strip(app):
    engine = StubEngine(DEVICES)
    d = PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)
    command = find(d, QtWidgets.QComboBox, "settings.controlSurfaces.assign.command")
    command.setCurrentIndex(command.findData("mixer.stripVolume|A2"))
    command.activated.emit(command.currentIndex())
    find(d, QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn").click()
    fader = {"midi": {"channel": 0, "kind": "cc", "number": 0}}
    engine.event.emit({"event": "surfaceLearn", "state": "captured", "device": NANO, "control": fader})
    assert rows(d, "control") == ["Ch 1 CC 0"] and rows(d, "command") == ["A2 Volume (fader)"]
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    saved = [p for m, p in engine.sent if m == "prefs.set"][0]["values"]["controlSurfaces.devices"]
    assert saved[0]["bindings"] == [{"control": fader, "command": "mixer.stripVolume", "params": {"strip": "A2"}}]
    # the saved assignment shows with its strip when the dialog opens again
    engine.values = {"controlSurfaces": {"enabled": True, "devices": saved}}
    again = PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)
    assert rows(again, "command") == ["A2 Volume (fader)"]


def test_mixer_strip_follows_a_fader(app):
    """``TrackMixerPanel.surface_value``: touch with the fader's taper (volume) or -100..100
    (pan), one release after the control rests; an unknown strip does nothing."""
    from PySide6 import QtTest

    from cfa4_qt.panels.mixer import TrackMixerPanel, pos_to_db

    m = TrackMixerPanel()
    m.strips_data = [{"id": 7, "ref": "A1", "kind": "audio"}, {"id": 0, "ref": "Mix", "kind": "mix"}]
    m.now = 1234
    sent: list[tuple[str, dict]] = []
    m.command.connect(lambda c, p: sent.append((c, p)))
    m.SURFACE_RELEASE_MS = 50
    assert m.surface_value("volume", "A1", 0.76)
    assert m.surface_value("volume", "A1", 1.0)
    assert m.surface_value("pan", "Mix", 0.25)
    assert not m.surface_value("volume", "A9", 0.5)
    assert sent[0] == ("mixer.touch", {"strip": 7, "lane": "volume", "value": pos_to_db(0.76), "time": 1234})
    assert abs(sent[0][1]["value"]) < 0.01, "0.76 of the way is 0 dB on the taper"
    assert abs(sent[1][1]["value"] - 15.0) < 1e-9, "the top is +15 dB"
    assert sent[2] == ("mixer.touch", {"strip": 0, "lane": "pan", "value": -50.0, "time": 1234})
    QtTest.QTest.qWait(200)
    releases = [p for c, p in sent if c == "mixer.release"]
    assert sorted((p["strip"], p["lane"]) for p in releases) == [(0, "pan"), (7, "volume")], "one release per moved lane"


def test_assignments_need_a_controller(app):
    engine = StubEngine({"enabled": True, "devices": [], "problems": []})
    d = PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: COMMANDS)
    assert "Connect a MIDI controller or a ShuttlePRO" in find(d, QtWidgets.QLabel, "settings.controlSurfaces.assign.empty").text()
    assert d.findChild(QtWidgets.QPushButton, "settings.controlSurfaces.assign.learn") is None


def _real_engine() -> Path | None:
    from cfa4_qt.engine import locate_engine

    return locate_engine()


@pytest.mark.skipif(_real_engine() is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_reports_control_surfaces(app, tmp_path: Path):
    """The built engine answers surface.devices (a list, empty in CI) and announces the feature."""
    import time

    from cfa4_qt.engine import EngineClient

    engine = EngineClient(_real_engine(), ["--data-dir", str(tmp_path / "data")])
    got: dict[str, Any] = {}
    engine.start()
    try:
        engine.call("hello", {}, on_reply=lambda r, _b: got.update(hello=r))
        engine.call("surface.devices", {}, on_reply=lambda r, _b: got.update(devices=r))
        engine.execute("controlSurfaces.profiles", {}, on_reply=lambda r, _b: got.update(profiles=r))
        # the MIDI Monitor starts on a desktop and says why not in a container; off always works
        engine.call("surface.monitor", {"enabled": True}, on_reply=lambda r, _b: got.update(monitor=r),
                    on_error=lambda m: got.update(monitor={"error": m}))
        engine.call("surface.monitor", {"enabled": False}, on_reply=lambda r, _b: got.update(monitorOff=r))
        engine.call("surface.learnCancel", {}, on_reply=lambda r, _b: got.update(learnCancel=r))
        deadline = time.monotonic() + 30
        while len(got) < 6 and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert {"surface", "surfaceMonitor", "surfaceLearn"} <= set(got["hello"]["features"])
        assert got["learnCancel"] == {"state": "cancelled"}
        assert got["monitor"].get("enabled") is True or "ALSA sequencer" in got["monitor"]["error"]
        assert got["monitorOff"] == {"enabled": False}
        assert isinstance(got["devices"]["devices"], list)
        assert {p["id"] for p in got["profiles"]["profiles"]} >= {"cfa4.shuttlepro", "cfa4.generic-midi"}
    finally:
        engine.stop()


@pytest.mark.skipif(_real_engine() is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_main_window_runs_what_a_control_surface_asks(app, tmp_path: Path):
    """``surfaceAction`` runs its command as the command's key would: a Qt-run command through its
    action, an engine command through the menu path; an unknown one says so in the status bar."""
    import time

    from PySide6 import QtCore

    from cfa4_qt.engine import EngineClient
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(_real_engine(), ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    engine.start()
    try:
        deadline = time.monotonic() + 60
        while len(win.registry.commands) < 100 and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        offered = dict(win.surface_commands())
        assert "playback.toggle" in offered and "edit.undo" in offered
        assert all(not win.registry.commands[c].needs_params for c in offered if c in win.registry.commands)
        fired: list[int] = []
        win.registry.local_actions["playback.toggle"].triggered.connect(lambda *_: fired.append(1))
        win._on_event({"event": "surfaceAction", "command": "playback.toggle", "params": {}, "device": NANO, "control": CC41})
        assert fired == [1]
        ran: list[tuple[str, dict]] = []
        win.run_menu_command = lambda cid, params: ran.append((cid, params))
        enabled = next(c for c, cmd in win.registry.commands.items() if cmd.enabled and not cmd.needs_params and c not in win.registry.local_actions)
        win._on_event({"event": "surfaceAction", "command": enabled, "params": {}})
        assert ran == [(enabled, {})]
        win._on_event({"event": "surfaceAction", "command": "no.such.command"})
        assert "no.such.command" in win.statusBar().currentMessage()
        # the shuttle ring plays the focused monitor at its speed; 0 stops
        speeds: list[float] = []
        win.focused_monitor().shuttle_to = speeds.append
        win._on_event({"event": "surfaceAction", "command": "playback.shuttleSpeed", "params": {"speed": -1.5}})
        win._on_event({"event": "surfaceAction", "command": "playback.shuttleSpeed", "params": {"speed": "fast"}})
        assert speeds == [-1.5]
        # a fader or knob moves its mixer strip
        moved: list[tuple] = []
        win.mixer.surface_value = lambda *a: moved.append(a)
        win._on_event({"event": "surfaceAction", "command": "mixer.stripVolume", "params": {"strip": "A1", "value": 0.5}})
        win._on_event({"event": "surfaceAction", "command": "mixer.stripPan", "params": {"strip": "Mix", "value": 1}})
        win._on_event({"event": "surfaceAction", "command": "mixer.stripPan", "params": {"strip": "Mix"}})
        assert moved == [("volume", "A1", 0.5), ("pan", "Mix", 1.0)]
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


def test_monitor_shuttle_to_plays_at_the_ring_speed(app):
    """``Monitor.shuttle_to``: the speed, capped at 8×; the same speed again does not restart;
    0 stops."""
    from cfa4_qt.panels.monitor import Monitor

    class Fake:
        playing = False
        speed = 0.0
        stop_at = rest_at = 1

        def __init__(self) -> None:
            self.calls: list[tuple[str, float]] = []

        def play(self, speed: float) -> None:
            self.calls.append(("play", speed))
            self.playing, self.speed = True, speed

        def stop(self) -> None:
            self.calls.append(("stop", 0.0))
            self.playing = False

    m = Fake()
    Monitor.shuttle_to(m, 2.0)
    Monitor.shuttle_to(m, 2.0)
    Monitor.shuttle_to(m, -60)
    Monitor.shuttle_to(m, 0)
    assert m.calls == [("play", 2.0), ("play", -8.0), ("stop", 0.0)]
    assert m.stop_at is None and m.rest_at is None
