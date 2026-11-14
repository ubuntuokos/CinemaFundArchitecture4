"""Settings ▸ Control Surfaces (CFA4-CONTROL-SURFACE-001, docs/adr/ADR-0012-control-surface.md):
the page lists the MIDI controllers and jog/shuttle devices `surface.devices` reports, each with
Use and Profile; the choices are a draft that OK applies as `controlSurfaces.devices`. Runs when
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


class StubEngine:
    """Answers at once, as the engine would; remembers every request."""

    def __init__(self, devices: dict[str, Any]) -> None:
        self.devices = devices
        self.sent: list[tuple[str, dict[str, Any]]] = []
        self.values: dict[str, Any] = {"controlSurfaces": {"enabled": True, "devices": []}}

    def call(self, method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        self.sent.append((method, params or {}))
        if method == "surface.devices" and on_reply:
            on_reply(self.devices, None)
        return len(self.sent)

    def execute(self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        self.sent.append((command, params or {}))
        replies = {"prefs.schema": SCHEMA, "prefs.get": {"values": self.values}, "controlSurfaces.profiles": PROFILES, "prefs.set": {}}
        if command in replies and on_reply:
            on_reply(replies[command], None)
        return len(self.sent)


@pytest.fixture(scope="module")
def app() -> Any:
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def find(d: Any, kind: Any, name: str) -> Any:
    w = d.findChild(kind, name)
    assert w is not None, name
    return w


def test_control_surfaces_page_lists_devices_and_applies_choices(app: Any) -> None:
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
    assert {"device": "midi:nanoKONTROL2:nanoKONTROL2 _ CTRL", "enabled": False, "profile": ""} in devices
    assert {"device": "hid:0b33:0030:usb-0000:00:14.0-3/input0", "enabled": True, "profile": "cfa4.shuttlepro.kdenlive"} in devices


def test_control_surfaces_page_without_devices_and_refresh(app: Any) -> None:
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


def _real_engine() -> Path | None:
    from cfa4_qt.engine import locate_engine

    return locate_engine()


@pytest.mark.skipif(_real_engine() is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_reports_control_surfaces(app: Any, tmp_path: Path) -> None:
    """The built engine answers surface.devices (a list, empty in CI) and announces the feature."""
    import time

    from cfa4_qt.engine import EngineClient

    path = _real_engine()
    assert path is not None
    engine = EngineClient(path, ["--data-dir", str(tmp_path / "data")])
    got: dict[str, Any] = {}
    engine.start()
    try:
        engine.call("hello", {}, on_reply=lambda r, _b: got.update(hello=r))
        engine.call("surface.devices", {}, on_reply=lambda r, _b: got.update(devices=r))
        engine.execute("controlSurfaces.profiles", {}, on_reply=lambda r, _b: got.update(profiles=r))
        deadline = time.monotonic() + 30
        while len(got) < 3 and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        assert "surface" in got["hello"]["features"]
        assert isinstance(got["devices"]["devices"], list)
        assert {p["id"] for p in got["profiles"]["profiles"]} >= {"cfa4.shuttlepro", "cfa4.generic-midi"}
    finally:
        engine.stop()
