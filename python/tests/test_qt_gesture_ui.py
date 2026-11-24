"""Qt6 mouse-gesture editor (CFA4-CONTROL-SURFACE-GESTURE-001, MG3).

The widgets are the native Control Surfaces section. A stub stands in for the engine, so the
test checks that every MG3 control is there and that it sends the gesture command. Recognition
itself is covered by the engine tests.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore", exc_type=ImportError)
QtGui = pytest.importorskip("PySide6.QtGui", exc_type=ImportError)
QtWidgets = pytest.importorskip("PySide6.QtWidgets", exc_type=ImportError)

from cfa4_qt.gesture_hud import GestureHud, GestureLayer  # noqa: E402
from cfa4_qt.panels.settings import PreferencesDialog  # noqa: E402
from cfa4_qt.window_menu import Workspaces, build_window_menu, window_commands  # noqa: E402

from test_qt_control_surfaces import DEVICES, StubEngine, find  # noqa: E402

SECTIONS = (
    "settings.controlSurfaces.gesture",
    "settings.controlSurfaces.gesture.profiles",
    "settings.controlSurfaces.gesture.editor",
    "settings.controlSurfaces.gesture.templates",
    "settings.controlSurfaces.gesture.learn",
    "settings.controlSurfaces.gesture.learnCanvas",
    "settings.controlSurfaces.gesture.buttons",
    "settings.controlSurfaces.gesture.test",
    "settings.controlSurfaces.gesture.testCanvas",
    "settings.controlSurfaces.gesture.combinations",
    "settings.controlSurfaces.gesture.mapping",
    "settings.controlSurfaces.gesture.hud",
    "settings.controlSurfaces.gesture.thresholds",
    "settings.controlSurfaces.gesture.access",
    "settings.controlSurfaces.gesture.input.qt",
    "settings.controlSurfaces.gesture.input.evdev",
    "settings.controlSurfaces.gesture.input.warning",
)


def _state(**over: Any) -> dict[str, Any]:
    state: dict[str, Any] = {
        "enabled": False,
        "armed": False,
        "hand": "follow",
        "mirror": False,
        "path": "qt",
        "evdev": "not-opened",
        "evdevRequested": False,
        "evdevDevice": "",
        "status": "idle",
        "warning": "",
        "thresholds": {"minLengthPx": 40, "maxLengthPx": 2000, "slopPx": 8, "minMs": 20, "maxMs": 1500, "angleMilliDeg": 20000, "holdMs": 300},
        "hud": {"show": True, "trail": True, "direction": True, "preview": True, "hint": True, "execution": True, "error": True, "cancel": True, "reduced": False, "contrast": False},
        "templates": [],
        "mappings": [],
        "buttons": [],
        "combinations": [],
        "hudFrame": {"show": False, "trail": [], "direction": "", "hint": "", "cancelled": False, "error": "", "reduced": False, "contrast": False},
        "state": {"enabled": False, "mirror": False, "gestures": [], "bindings": [], "ui": {}},
    }
    state.update(over)
    return state


class GestureEngine(StubEngine):
    """Records gesture commands and answers with the state the test sets."""

    def __init__(self) -> None:
        super().__init__(DEVICES)
        self.gesture = _state()

    def execute(self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        n = super().execute(command, params, on_reply, on_error)
        if command.startswith("surface.gesture."):
            self._apply(command, params or {})
            if on_reply:
                on_reply(self.gesture, None)
        return n

    def _apply(self, command: str, params: dict[str, Any]) -> None:
        if command == "surface.gesture.configure":
            for key in ("enabled", "armed", "hand", "mirror", "evdevRequested", "evdevDevice"):
                if key in params:
                    self.gesture[key] = params[key]
            if isinstance(params.get("thresholds"), dict):
                self.gesture["thresholds"].update(params["thresholds"])
            if isinstance(params.get("hud"), dict):
                self.gesture["hud"].update(params["hud"])
            if params.get("evdevRequested"):
                self.gesture["path"] = "qt"
                self.gesture["evdev"] = "not-opened"
                self.gesture["warning"] = "evdev is stored and is not opened"
        elif command == "surface.gesture.map" and params.get("op") == "set":
            self.gesture["mappings"].append({"id": params.get("id"), "command": params.get("command"), "context": params.get("context") or "global"})
        elif command == "surface.gesture.combo" and params.get("op") == "set":
            roles = params.get("roles") or []
            self.gesture["combinations"].append({"roles": roles, "key": ",".join(str(r) for r in roles) + "+0", "command": params.get("command")})
        elif command == "surface.gesture.learn":
            self.gesture["status"] = "learning" if params.get("phase") == "begin" else "idle"
        elif command == "surface.gesture.sample":
            self.gesture["status"] = params.get("op") or "idle"


@pytest.fixture(scope="module")
def app() -> Any:
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def _dialog(engine: GestureEngine) -> PreferencesDialog:
    return PreferencesDialog(None, engine, "controlSurfaces", commands=lambda: [("playback.toggle", "Play/Stop"), ("edit.undo", "Undo")])


def test_every_gesture_section_is_on_the_control_surfaces_page(app: Any) -> None:
    engine = GestureEngine()
    dialog = _dialog(engine)
    for name in SECTIONS:
        assert dialog.findChild(QtWidgets.QWidget, name) is not None, name
    commands = [item[0] for item in engine.sent if item[0] == "surface.gesture.inspect"]
    assert commands
    dialog.close()


def test_the_button_list_follows_the_engine_and_is_not_a_fixed_set(app: Any) -> None:
    engine = GestureEngine()
    engine.gesture = _state(buttons=[{"code": 8 + i, "label": f"learned button {i}", "path": "qt", "role": f"learned-{i}"} for i in range(12)])
    dialog = _dialog(engine)
    rows = find(dialog, QtWidgets.QListWidget, "settings.controlSurfaces.gesture.buttons")
    assert rows.count() == 12
    assert "learned button 11" in rows.item(11).text()
    assert find(dialog, QtWidgets.QLabel, "settings.controlSurfaces.gesture.buttons.empty").isHidden()
    dialog.close()


def test_enabling_thresholds_evdev_mapping_and_combinations_send_commands(app: Any) -> None:
    engine = GestureEngine()
    dialog = _dialog(engine)
    find(dialog, QtWidgets.QCheckBox, "settings.controlSurfaces.gesture.enabled").setChecked(True)
    find(dialog, QtWidgets.QCheckBox, "settings.controlSurfaces.gesture.armed").setChecked(True)
    spin = find(dialog, QtWidgets.QSpinBox, "settings.controlSurfaces.gesture.threshold.minLength")
    spin.setValue(80)
    spin.editingFinished.emit()
    find(dialog, QtWidgets.QCheckBox, "settings.controlSurfaces.gesture.input.evdev").setChecked(True)
    assert not find(dialog, QtWidgets.QLabel, "settings.controlSurfaces.gesture.input.warning").isHidden()
    assert find(dialog, QtWidgets.QLabel, "settings.controlSurfaces.gesture.input.qt").text()
    find(dialog, QtWidgets.QLineEdit, "settings.controlSurfaces.gesture.mapGesture").setText("direction.right")
    find(dialog, QtWidgets.QPushButton, "settings.controlSurfaces.gesture.mapAdd").click()
    find(dialog, QtWidgets.QLineEdit, "settings.controlSurfaces.gesture.comboRoles").setText("0,1")
    find(dialog, QtWidgets.QPushButton, "settings.controlSurfaces.gesture.comboAdd").click()
    find(dialog, QtWidgets.QPushButton, "settings.controlSurfaces.gesture.learnStart").click()
    find(dialog, QtWidgets.QPushButton, "settings.controlSurfaces.gesture.buttonLearn").click()
    find(dialog, QtWidgets.QPushButton, "settings.controlSurfaces.gesture.testStart").click()
    sent = engine.sent
    assert {"enabled": True} in [p for name, p in sent if name == "surface.gesture.configure"]
    assert {"armed": True} in [p for name, p in sent if name == "surface.gesture.configure"]
    thresholds = [p["thresholds"]["minLengthPx"] for name, p in sent if name == "surface.gesture.configure" and "thresholds" in p]
    assert 80 in thresholds
    assert any(p.get("evdevRequested") is True for name, p in sent if name == "surface.gesture.configure")
    assert {"op": "set", "id": "direction.right", "command": "playback.toggle", "context": "global"} in [p for name, p in sent if name == "surface.gesture.map"]
    assert {"op": "set", "roles": [0, 1], "modifiers": 0, "command": "playback.toggle", "context": "global"} in [p for name, p in sent if name == "surface.gesture.combo"]
    assert {"mode": "gesture", "phase": "begin"} in [p for name, p in sent if name == "surface.gesture.learn"]
    assert {"mode": "button", "phase": "begin"} in [p for name, p in sent if name == "surface.gesture.learn"]
    assert {"mode": "test", "phase": "begin"} in [p for name, p in sent if name == "surface.gesture.learn"]
    # The command list offers playback, not an unrelated edit command.
    combo = find(dialog, QtWidgets.QComboBox, "settings.controlSurfaces.gesture.mapCommand")
    assert combo.findData("playback.toggle") >= 0
    assert combo.findData("edit.undo") < 0
    dialog.close()


def test_the_learn_pad_sends_samples_and_cancel_restores(app: Any) -> None:
    engine = GestureEngine()
    dialog = _dialog(engine)
    pad = find(dialog, QtWidgets.QWidget, "settings.controlSurfaces.gesture.learnCanvas")
    press = QtGui.QMouseEvent(
        QtCore.QEvent.Type.MouseButtonPress,
        QtCore.QPointF(4, 6),
        QtCore.QPointF(4, 6),
        QtCore.Qt.MouseButton.RightButton,
        QtCore.Qt.MouseButton.RightButton,
        QtCore.Qt.KeyboardModifier.NoModifier,
    )
    pad.mousePressEvent(press)
    samples = [p for name, p in engine.sent if name == "surface.gesture.sample"]
    assert samples[-1]["op"] == "press"
    assert samples[-1]["code"] == QtCore.Qt.MouseButton.RightButton.value
    assert samples[-1]["from"] == "editor"
    dialog.reject()
    assert any(name == "surface.gesture.restore" for name, _p in engine.sent)
    dialog.close()


def test_the_window_menu_opens_mouse_gestures(app: Any, tmp_path: Path) -> None:
    ids = {row[0]: row[1] for row in window_commands()}
    assert ids["window.mouseGestures"] == "Mouse Gestures…"

    class Host(QtWidgets.QMainWindow):
        def __init__(self) -> None:
            super().__init__()
            self.opened: list[tuple[str, str]] = []

        def open_settings(self, page: str = "general", section: str = "") -> None:
            self.opened.append((page, section))

    host = Host()
    menu = QtWidgets.QMenu(host)
    settings = QtCore.QSettings(str(tmp_path / "ui.ini"), QtCore.QSettings.Format.IniFormat)
    spaces = Workspaces(host, settings, QtCore.QByteArray(), 1)
    build_window_menu(host, menu, {}, QtWidgets.QWidget(), spaces, lambda _b: None, QtGui.QAction("Reset", host))
    action = host.findChild(QtGui.QAction, "window.mouseGestures")
    assert action is not None
    action.trigger()
    assert host.opened[-1] == ("controlSurfaces", "settings.controlSurfaces.gesture")
    host.close()


def test_the_hud_does_not_take_the_pointer(app: Any) -> None:
    window = QtWidgets.QWidget()
    window.resize(320, 200)

    class Engine:
        def execute(self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
            if on_reply:
                on_reply(_state(armed=False, hudFrame={"show": True, "trail": [[1, 2], [8, 9]], "direction": "right", "hint": "playback.toggle", "cancelled": False, "reduced": False, "contrast": True}), None)
            return 0

    window.show()
    layer = GestureLayer(window, Engine())
    app.processEvents()
    hud = layer.hud
    assert hud.testAttribute(QtCore.Qt.WidgetAttribute.WA_TransparentForMouseEvents)
    assert isinstance(hud, GestureHud)
    assert hud.isVisible()
    hud.repaint()
    image = hud.grab()
    assert not image.isNull()
    window.close()
