"""Settings ▸ Control Surfaces ▸ Mouse Gestures (CFA4-CONTROL-SURFACE-GESTURE-001, MG3).

The widgets only send ``surface.gesture.*`` commands. The engine recognises the stroke
and, when a gesture is bound, emits ``surfaceAction``. MIDI Learn is not used.
"""

from __future__ import annotations

import copy
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QMouseEvent
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QFormLayout,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QPushButton,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)

from ..i18n import tr


def qt_int(value: object) -> int:
    number = getattr(value, "value", None)
    return number if isinstance(number, int) else 0

POINTER = "pointer:seat"
_COMMANDS = (
    "playback.toggle",
    "playback.stop",
    "playback.play",
    "playhead.start",
    "playhead.end",
    "playhead.stepForward",
    "playhead.stepBack",
    "window.controlSurfaces",
)


class StrokePad(QWidget):
    """The editor pad. It accepts the stroke so Button Learn does not fall through to a panel."""

    def __init__(self, name: str, page: GesturePage) -> None:
        super().__init__(page)
        self.setObjectName(name)
        self.setMinimumHeight(72)
        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self._page = page
        self._down = False
        self._code = 0

    def mousePressEvent(self, event: QMouseEvent) -> None:
        self._down = True
        self._code = qt_int(event.button())
        self._page.sample("press", self._code, event)
        event.accept()

    def mouseMoveEvent(self, event: QMouseEvent) -> None:
        if self._down:
            self._page.sample("move", self._code, event)
        event.accept()

    def mouseReleaseEvent(self, event: QMouseEvent) -> None:
        self._down = False
        self._page.sample("release", qt_int(event.button()) or self._code, event)
        event.accept()

    def keyPressEvent(self, event: Any) -> None:
        if event.key() == Qt.Key.Key_Escape:
            self._page.sample_body({"op": "cancel", "from": "editor"})
            self._down = False
        event.accept()


class GesturePage(QGroupBox):
    """The Mouse Gestures section of the Control Surfaces page."""

    def __init__(self, dialog: Any, parent: QWidget) -> None:
        super().__init__(tr("Mouse Gestures"), parent)
        self.setObjectName("settings.controlSurfaces.gesture")
        self._dialog = dialog
        self._snapshot: dict[str, Any] | None = None
        self._state: dict[str, Any] = {}
        self._filling = False
        root = QVBoxLayout(self)
        root.addWidget(self._switch_row())
        root.addWidget(self._profiles())
        root.addWidget(self._editor())
        root.addWidget(self._learn_box())
        root.addWidget(self._buttons())
        root.addWidget(self._combo_box())
        root.addWidget(self._mapping())
        root.addWidget(self._hud())
        root.addWidget(self._thresholds())
        root.addWidget(self._access())
        self._status = QLabel("", self)
        self._status.setObjectName("settings.controlSurfaces.gesture.status")
        self._status.setWordWrap(True)
        root.addWidget(self._status)
        rejected = getattr(dialog, "rejected", None)
        if rejected is not None:
            rejected.connect(self._revert)
        self._ask("surface.gesture.inspect", {}, self._filled)

    def sample(self, op: str, code: int, event: QMouseEvent) -> None:
        pos = event.position()
        body: dict[str, Any] = {
            "op": op,
            "code": code,
            "x": int(pos.x()),
            "y": int(pos.y()),
            "from": "editor",
            "modifiers": qt_int(event.modifiers()),
        }
        self.sample_body(body)

    def sample_body(self, body: dict[str, Any]) -> None:
        self._ask("surface.gesture.sample", body, self._filled)

    def _switch_row(self) -> QWidget:
        row = QWidget(self)
        layout = QHBoxLayout(row)
        layout.setContentsMargins(0, 0, 0, 0)
        self._enabled = self._check(row, "settings.controlSurfaces.gesture.enabled", tr("Mouse Gestures"), self._toggle_enabled)
        self._armed = self._check(row, "settings.controlSurfaces.gesture.armed", tr("Use gestures in the window"), self._toggle_armed)
        layout.addWidget(self._enabled)
        layout.addWidget(self._armed)
        layout.addStretch(1)
        return row

    def _profiles(self) -> QGroupBox:
        box = QGroupBox(tr("Gesture profiles"), self)
        box.setObjectName("settings.controlSurfaces.gesture.profiles")
        form = QFormLayout(box)
        self._hand = QComboBox(box)
        self._hand.setObjectName("settings.controlSurfaces.gesture.hand")
        for value, label in (("follow", tr("Follow the system")), ("right", tr("Right-handed")), ("left", tr("Left-handed"))):
            self._hand.addItem(label, value)
        self._hand.activated.connect(lambda _i: self._configure({"hand": str(self._hand.currentData() or "follow")}))
        form.addRow(tr("Hand"), self._hand)
        self._mirror = self._check(box, "settings.controlSurfaces.gesture.mirror", tr("Mirror ergonomic gestures"), self._toggle_mirror)
        form.addRow(self._mirror)
        buttons = QHBoxLayout()
        buttons.addWidget(self._button(box, "settings.controlSurfaces.gesture.save", tr("Save profile"), lambda: self._profile("save")))
        buttons.addWidget(self._button(box, "settings.controlSurfaces.gesture.load", tr("Load profile"), self._load))
        buttons.addWidget(self._button(box, "settings.controlSurfaces.gesture.import", tr("Import…"), self._import))
        buttons.addWidget(self._button(box, "settings.controlSurfaces.gesture.export", tr("Export…"), self._export))
        buttons.addWidget(self._button(box, "settings.controlSurfaces.gesture.delete", tr("Delete profile"), lambda: self._profile("delete")))
        form.addRow(buttons)
        return box

    def _editor(self) -> QGroupBox:
        box = QGroupBox(tr("Gesture editor"), self)
        box.setObjectName("settings.controlSurfaces.gesture.editor")
        layout = QVBoxLayout(box)
        self._templates = QListWidget(box)
        self._templates.setObjectName("settings.controlSurfaces.gesture.templates")
        layout.addWidget(self._templates)
        row = QHBoxLayout()
        self._spatial = self._check(box, "settings.controlSurfaces.gesture.spatial", tr("Spatial gesture"), None)
        row.addWidget(self._spatial)
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.templateRemove", tr("Remove"), self._remove_template))
        layout.addLayout(row)
        return box

    def _learn_box(self) -> QGroupBox:
        box = QGroupBox(tr("Gesture Learn"), self)
        box.setObjectName("settings.controlSurfaces.gesture.learn")
        layout = QVBoxLayout(box)
        layout.addWidget(self._note(box, tr("Draw the gesture with a button held.")))
        self._pad = StrokePad("settings.controlSurfaces.gesture.learnCanvas", self)
        layout.addWidget(self._pad)
        row = QHBoxLayout()
        self._learn_name = QLineEdit(box)
        self._learn_name.setObjectName("settings.controlSurfaces.gesture.learnName")
        self._learn_name.setPlaceholderText(tr("Name"))
        row.addWidget(self._learn_name, 1)
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.learnStart", tr("Start"), lambda: self._learn("gesture", "begin")))
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.learnSave", tr("Save gesture"), self._save_gesture))
        layout.addLayout(row)
        return box

    def _buttons(self) -> QGroupBox:
        box = QGroupBox(tr("Detected buttons"), self)
        box.setObjectName("settings.controlSurfaces.gesture.buttonsBox")
        layout = QVBoxLayout(box)
        self._button_list = QListWidget(box)
        self._button_list.setObjectName("settings.controlSurfaces.gesture.buttons")
        layout.addWidget(self._button_list)
        self._empty = self._note(box, tr("No buttons yet. Press one in Button Learn or Button Test."))
        self._empty.setObjectName("settings.controlSurfaces.gesture.buttons.empty")
        layout.addWidget(self._empty)
        row = QHBoxLayout()
        self._button_name = QLineEdit(box)
        self._button_name.setObjectName("settings.controlSurfaces.gesture.buttonName")
        self._button_name.setPlaceholderText(tr("Button name"))
        row.addWidget(self._button_name, 1)
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.buttonLearn", tr("Button Learn"), lambda: self._learn("button", "begin")))
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.buttonLearnStop", tr("Stop"), lambda: self._learn("off", "end")))
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.buttonNameSave", tr("Name"), self._name_button))
        layout.addLayout(row)
        test = QGroupBox(tr("Button Test"), box)
        test.setObjectName("settings.controlSurfaces.gesture.test")
        test_layout = QVBoxLayout(test)
        self._test_status = QLabel(tr("Idle"), test)
        self._test_status.setObjectName("settings.controlSurfaces.gesture.testStatus")
        test_layout.addWidget(self._test_status)
        self._test_pad = StrokePad("settings.controlSurfaces.gesture.testCanvas", self)
        test_layout.addWidget(self._test_pad)
        test_layout.addWidget(self._button(test, "settings.controlSurfaces.gesture.testStart", tr("Test"), lambda: self._learn("test", "begin")))
        layout.addWidget(test)
        return box

    def _combo_box(self) -> QGroupBox:
        box = QGroupBox(tr("Combination editor"), self)
        box.setObjectName("settings.controlSurfaces.gesture.combinations")
        layout = QVBoxLayout(box)
        self._combo_list = QListWidget(box)
        self._combo_list.setObjectName("settings.controlSurfaces.gesture.combos")
        layout.addWidget(self._combo_list)
        row = QHBoxLayout()
        self._combo_roles = QLineEdit(box)
        self._combo_roles.setObjectName("settings.controlSurfaces.gesture.comboRoles")
        self._combo_roles.setPlaceholderText(tr("Roles"))
        self._combo_hold = QCheckBox(tr("Hold"), box)
        self._combo_hold.setObjectName("settings.controlSurfaces.gesture.comboHold")
        self._combo_command = self._commands(box, "settings.controlSurfaces.gesture.comboCommand")
        row.addWidget(self._combo_roles)
        row.addWidget(self._combo_hold)
        row.addWidget(self._combo_command, 1)
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.comboAdd", tr("Add combination"), self._add_combo))
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.comboRemove", tr("Remove"), self._remove_combo))
        layout.addLayout(row)
        return box

    def _mapping(self) -> QGroupBox:
        box = QGroupBox(tr("Mapping"), self)
        box.setObjectName("settings.controlSurfaces.gesture.mapping")
        layout = QVBoxLayout(box)
        self._maps = QListWidget(box)
        self._maps.setObjectName("settings.controlSurfaces.gesture.mappings")
        layout.addWidget(self._maps)
        row = QHBoxLayout()
        self._map_id = QLineEdit(box)
        self._map_id.setObjectName("settings.controlSurfaces.gesture.mapGesture")
        self._map_id.setPlaceholderText(tr("Gesture"))
        self._map_context = QLineEdit(box)
        self._map_context.setObjectName("settings.controlSurfaces.gesture.mapContext")
        self._map_context.setPlaceholderText(tr("Context"))
        self._map_context.setText("global")
        self._map_command = self._commands(box, "settings.controlSurfaces.gesture.mapCommand")
        row.addWidget(self._map_id)
        row.addWidget(self._map_context)
        row.addWidget(self._map_command, 1)
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.mapAdd", tr("Add mapping"), self._add_map))
        row.addWidget(self._button(box, "settings.controlSurfaces.gesture.mapRemove", tr("Remove"), self._remove_map))
        layout.addLayout(row)
        return box

    def _hud(self) -> QGroupBox:
        box = QGroupBox(tr("HUD"), self)
        box.setObjectName("settings.controlSurfaces.gesture.hud")
        grid = QGridLayout(box)
        flags = (
            ("show", "settings.controlSurfaces.gesture.hud.show", tr("Show the gesture HUD")),
            ("trail", "settings.controlSurfaces.gesture.hud.trail", tr("Trail")),
            ("direction", "settings.controlSurfaces.gesture.hud.direction", tr("Direction")),
            ("preview", "settings.controlSurfaces.gesture.hud.preview", tr("Recognition preview")),
            ("hint", "settings.controlSurfaces.gesture.hud.hint", tr("Action hint")),
            ("execution", "settings.controlSurfaces.gesture.hud.execution", tr("Execution feedback")),
            ("error", "settings.controlSurfaces.gesture.hud.error", tr("Error feedback")),
            ("cancel", "settings.controlSurfaces.gesture.hud.cancel", tr("Cancel feedback")),
            ("reduced", "settings.controlSurfaces.gesture.hud.reduced", tr("Reduced animation")),
            ("contrast", "settings.controlSurfaces.gesture.hud.contrast", tr("High contrast")),
        )
        self._hud_boxes: dict[str, QCheckBox] = {}
        for index, (key, name, label) in enumerate(flags):
            box_widget = self._check(box, name, label, lambda _on, key=key: self._hud_changed(key))
            self._hud_boxes[key] = box_widget
            grid.addWidget(box_widget, index // 2, index % 2)
        return box

    def _thresholds(self) -> QGroupBox:
        box = QGroupBox(tr("Thresholds"), self)
        box.setObjectName("settings.controlSurfaces.gesture.thresholds")
        form = QFormLayout(box)
        self._spins: dict[str, QSpinBox] = {}
        specs = (
            ("minLengthPx", "settings.controlSurfaces.gesture.threshold.minLength", tr("Minimum length"), 1, 100_000, "px"),
            ("maxLengthPx", "settings.controlSurfaces.gesture.threshold.maxLength", tr("Maximum length"), 1, 100_000, "px"),
            ("slopPx", "settings.controlSurfaces.gesture.threshold.slop", tr("Slop"), 0, 10_000, "px"),
            ("minMs", "settings.controlSurfaces.gesture.threshold.minMs", tr("Minimum time"), 1, 60_000, "ms"),
            ("maxMs", "settings.controlSurfaces.gesture.threshold.maxMs", tr("Maximum time"), 1, 60_000, "ms"),
            ("angleMilliDeg", "settings.controlSurfaces.gesture.threshold.angle", tr("Angle"), 0, 180_000, "mdeg"),
            ("holdMs", "settings.controlSurfaces.gesture.threshold.holdMs", tr("Hold time"), 1, 60_000, "ms"),
        )
        for key, name, label, low, high, unit in specs:
            spin = QSpinBox(box)
            spin.setObjectName(name)
            spin.setRange(low, high)
            spin.setSuffix(" " + unit)
            spin.editingFinished.connect(self._thresholds_changed)
            self._spins[key] = spin
            form.addRow(label, spin)
        return box

    def _access(self) -> QGroupBox:
        box = QGroupBox(tr("Input access"), self)
        box.setObjectName("settings.controlSurfaces.gesture.access")
        layout = QVBoxLayout(box)
        self._qt_path = QLabel(tr("Qt pointer (this application)"), box)
        self._qt_path.setObjectName("settings.controlSurfaces.gesture.input.qt")
        layout.addWidget(self._qt_path)
        self._evdev = self._check(box, "settings.controlSurfaces.gesture.input.evdev", tr("Evdev device"), self._toggle_evdev)
        layout.addWidget(self._evdev)
        self._evdev_name = QLineEdit(box)
        self._evdev_name.setObjectName("settings.controlSurfaces.gesture.input.device")
        self._evdev_name.setPlaceholderText(tr("Evdev device"))
        self._evdev_name.editingFinished.connect(self._evdev_device)
        layout.addWidget(self._evdev_name)
        self._warning = self._note(box, tr("Evdev reads the selected device only, and only after a later phase turns it on. This version keeps the Qt path."))
        self._warning.setObjectName("settings.controlSurfaces.gesture.input.warning")
        layout.addWidget(self._warning)
        layout.addWidget(self._note(box, tr("Gestures stay off until you turn them on. Turning them on does not block the panel menu.")))
        return box

    def _filled(self, state: Any, _binary: Any) -> None:
        if not isinstance(state, dict):
            return
        self._state = state
        if self._snapshot is None and isinstance(state.get("state"), dict):
            self._snapshot = copy.deepcopy(state["state"])
            self._remember(state, dirty=False)
        self._fill(state)
        frame = state.get("hudFrame")
        window = self.window()
        layer = getattr(window, "gestures", None)
        show = getattr(layer, "show_frame", None)
        if callable(show) and isinstance(frame, dict):
            show(frame)

    def _fill(self, state: dict[str, Any]) -> None:
        self._filling = True
        try:
            self._enabled.setChecked(bool(state.get("enabled")))
            self._armed.setChecked(bool(state.get("armed")))
            self._mirror.setChecked(bool(state.get("mirror")))
            hand = self._hand.findData(state.get("hand") or "follow")
            if hand >= 0:
                self._hand.setCurrentIndex(hand)
            self._evdev.setChecked(bool(state.get("evdevRequested")))
            self._evdev_name.setText(str(state.get("evdevDevice") or ""))
            self._warning.setVisible(bool(state.get("evdevRequested")))
            raw_thresholds = state.get("thresholds")
            thresholds: dict[str, Any] = raw_thresholds if isinstance(raw_thresholds, dict) else {}
            for key, spin in self._spins.items():
                value = thresholds.get(key)
                if isinstance(value, int) and not isinstance(value, bool):
                    spin.setValue(value)
            raw_hud = state.get("hud")
            hud: dict[str, Any] = raw_hud if isinstance(raw_hud, dict) else {}
            for key, box in self._hud_boxes.items():
                box.setChecked(bool(hud.get(key, True if key != "reduced" and key != "contrast" else False)))
            self._templates.clear()
            for template in state.get("templates") or []:
                if isinstance(template, dict):
                    self._templates.addItem(f"{template.get('id')} ({len(template.get('points') or [])})")
            self._maps.clear()
            seen: set[tuple[str, str]] = set()
            for mapping in state.get("mappings") or []:
                if isinstance(mapping, dict):
                    ident = str(mapping.get("id") or "")
                    context = str(mapping.get("context") or "global")
                    seen.add((ident, context))
                    self._maps.addItem(f"{ident} → {mapping.get('command')} [{context}]")
            for profile in state.get("profiles") or []:
                if not isinstance(profile, dict):
                    continue
                context = str(profile.get("context") or "")
                for mapping in profile.get("mappings") or []:
                    if not isinstance(mapping, dict) or mapping.get("source") != "builtin":
                        continue
                    ident = str(mapping.get("id") or "")
                    if (ident, context) in seen:
                        continue
                    self._maps.addItem(f"{ident} → {mapping.get('command')} [{context}]")
            self._combo_list.clear()
            for combo in state.get("combinations") or []:
                if isinstance(combo, dict):
                    suffix = " · hold" if combo.get("trigger") == "hold" else ""
                    context = str(combo.get("context") or "global")
                    self._combo_list.addItem(f"{combo.get('key')} → {combo.get('command')}{suffix} [{context}]")
            self._button_list.clear()
            buttons = [row for row in state.get("buttons") or [] if isinstance(row, dict)]
            for button in buttons:
                self._button_list.addItem(f"{button.get('label') or button.get('role') or 'button'}  {button.get('code')}  {button.get('path')}")
            self._empty.setVisible(not buttons)
            status = str(state.get("status") or "idle")
            words = {"idle": tr("Idle"), "learning": tr("Learning"), "press": tr("Press"), "release": tr("Release"), "cancelled": tr("Cancelled"), "test": tr("Test")}
            self._test_status.setText(words.get(status, status))
            warning = state.get("warning")
            self._status.setText(str(warning or ""))
        finally:
            self._filling = False

    def _remember(self, state: dict[str, Any], dirty: bool) -> None:
        values = getattr(self._dialog, "values", None)
        if not isinstance(values, dict):
            return
        root = values.get("controlSurfaces")
        if not isinstance(root, dict) or "devices" not in root:
            return
        devices = [dict(row) for row in root.get("devices") or [] if isinstance(row, dict) and row.get("device") != POINTER]
        devices.append(_pointer_device(state))
        root["devices"] = devices
        if not dirty:
            saved = getattr(self._dialog, "saved", None)
            if isinstance(saved, dict) and isinstance(saved.get("controlSurfaces"), dict):
                saved["controlSurfaces"]["devices"] = copy.deepcopy(devices)

    def _revert(self) -> None:
        if self._snapshot is not None:
            self._ask("surface.gesture.restore", {"state": self._snapshot}, lambda _r, _b: None)

    def _configure(self, body: dict[str, Any]) -> None:
        if self._filling:
            return
        self._ask("surface.gesture.configure", body, self._edited)

    def _edited(self, state: Any, binary: Any) -> None:
        self._filled(state, binary)
        if isinstance(state, dict):
            self._remember(state, dirty=True)

    def _toggle_enabled(self, on: bool) -> None:
        self._configure({"enabled": bool(on)})

    def _toggle_armed(self, on: bool) -> None:
        self._configure({"armed": bool(on)})

    def _toggle_mirror(self, on: bool) -> None:
        self._configure({"mirror": bool(on)})

    def _toggle_evdev(self, on: bool) -> None:
        self._configure({"evdevRequested": bool(on), "evdevDevice": self._evdev_name.text().strip()})

    def _evdev_device(self) -> None:
        self._configure({"evdevRequested": self._evdev.isChecked(), "evdevDevice": self._evdev_name.text().strip()})

    def _hud_changed(self, _key: str) -> None:
        if self._filling:
            return
        self._configure({"hud": {key: box.isChecked() for key, box in self._hud_boxes.items()}})

    def _thresholds_changed(self) -> None:
        if self._filling:
            return
        self._configure({"thresholds": {key: spin.value() for key, spin in self._spins.items()}})

    def _learn(self, mode: str, phase: str) -> None:
        self._ask("surface.gesture.learn", {"mode": mode, "phase": phase}, self._edited)

    def _save_gesture(self) -> None:
        name = self._learn_name.text().strip() or "gesture"
        self._ask("surface.gesture.learn", {"mode": "gesture", "phase": "save", "id": name, "spatial": self._spatial.isChecked()}, self._edited)

    def _name_button(self) -> None:
        row = self._button_list.currentRow()
        buttons = [item for item in self._state.get("buttons") or [] if isinstance(item, dict)]
        if row < 0 or row >= len(buttons):
            return
        code = buttons[row].get("code")
        if isinstance(code, int):
            self._configure({"buttonCode": code, "buttonName": self._button_name.text().strip()})

    def _remove_template(self) -> None:
        text = self._templates.currentItem().text() if self._templates.currentItem() is not None else ""
        ident = text.split(" ", 1)[0]
        if ident:
            self._ask("surface.gesture.template", {"op": "remove", "id": ident}, self._edited)

    def _add_map(self) -> None:
        self._ask(
            "surface.gesture.map",
            {"op": "set", "id": self._map_id.text().strip(), "command": str(self._map_command.currentData() or ""), "context": self._map_context.text().strip() or "global"},
            self._edited,
        )

    def _remove_map(self) -> None:
        text = self._maps.currentItem().text() if self._maps.currentItem() is not None else ""
        ident = text.split(" ", 1)[0]
        context = "global"
        if "[" in text and text.endswith("]"):
            context = text[text.rfind("[") + 1 : -1] or "global"
        if ident:
            self._ask("surface.gesture.map", {"op": "remove", "id": ident, "context": context}, self._edited)

    def _add_combo(self) -> None:
        roles: list[int] = []
        for part in self._combo_roles.text().replace(" ", "").split(","):
            if part.isdigit():
                roles.append(int(part))
        trigger = "hold" if self._combo_hold.isChecked() else "chord"
        context = self._map_context.text().strip() or "global"
        self._ask(
            "surface.gesture.combo",
            {
                "op": "set",
                "roles": roles,
                "modifiers": 0,
                "command": str(self._combo_command.currentData() or ""),
                "context": context,
                "trigger": trigger,
            },
            self._edited,
        )

    def _remove_combo(self) -> None:
        text = self._combo_list.currentItem().text() if self._combo_list.currentItem() is not None else ""
        key = text.split(" ", 1)[0]
        roles: list[int] = []
        modifiers = 0
        trigger = "hold" if "· hold" in text else "chord"
        context = "global"
        if "[" in text and text.endswith("]"):
            context = text[text.rfind("[") + 1 : -1] or "global"
        if "+" in key:
            left, right = key.split("+", 1)
            roles = [int(part) for part in left.split(",") if part.isdigit()]
            if right.isdigit():
                modifiers = int(right)
        if (trigger == "hold" and len(roles) == 1) or len(roles) >= 2:
            self._ask(
                "surface.gesture.combo",
                {"op": "remove", "roles": roles, "modifiers": modifiers, "context": context, "trigger": trigger},
                self._edited,
            )

    def _profile(self, op: str) -> None:
        self._ask("surface.gesture.profile", {"op": op}, self._edited)

    def _load(self) -> None:
        self._ask("surface.gesture.profile", {"op": "load"}, self._edited)

    def _export(self) -> None:
        self._ask("surface.gesture.profile", {"op": "export"}, self._exported)

    def _exported(self, state: Any, _binary: Any) -> None:
        document = state.get("document") if isinstance(state, dict) else None
        self._export_document = document if isinstance(document, dict) else {}
        self._status.setText(tr("Export…"))

    def _import(self) -> None:
        document = getattr(self, "_export_document", None)
        if not isinstance(document, dict) or not document:
            self._status.setText(tr("Import…"))
            return
        self._ask("surface.gesture.profile", {"op": "import", "document": document}, self._edited)

    def _ask(self, command: str, params: dict[str, Any], reply: Any) -> None:
        engine = getattr(self._dialog, "engine", None)
        if engine is None:
            return
        engine.execute(command, params, on_reply=reply, on_error=self._failed)

    def _failed(self, message: str) -> None:
        self._status.setText(message)

    def _commands(self, parent: QWidget, name: str) -> QComboBox:
        combo = QComboBox(parent)
        combo.setObjectName(name)
        source = getattr(self._dialog, "commands", None)
        rows = source() if callable(source) else []
        allowed = [row for row in rows if isinstance(row, tuple) and len(row) >= 1 and str(row[0]).split(".", 1)[0] in {"playback", "playhead", "window"}]
        if not allowed:
            allowed = [(command, command) for command in _COMMANDS]
        for command, label in allowed:
            combo.addItem(str(label), str(command))
        return combo

    def _check(self, parent: QWidget, name: str, label: str, slot: Any) -> QCheckBox:
        box = QCheckBox(label, parent)
        box.setObjectName(name)
        if slot is not None:
            box.toggled.connect(slot)
        return box

    def _button(self, parent: QWidget, name: str, label: str, slot: Any) -> QPushButton:
        button = QPushButton(label, parent)
        button.setObjectName(name)
        button.clicked.connect(lambda _checked=False: slot())
        return button

    def _note(self, parent: QWidget, text: str) -> QLabel:
        label = QLabel(text, parent)
        label.setWordWrap(True)
        return label


def _pointer_device(state: dict[str, Any]) -> dict[str, Any]:
    bindings = []
    for mapping in state.get("mappings") or []:
        if not isinstance(mapping, dict):
            continue
        bindings.append({
            "control": {"gesture": {"id": mapping.get("id")}},
            "command": mapping.get("command") or "",
            "context": "" if mapping.get("context") in (None, "global") else mapping.get("context"),
            "params": mapping.get("params") if isinstance(mapping.get("params"), dict) else None,
        })
    return {
        "device": POINTER,
        "enabled": bool(state.get("enabled")),
        "profile": "",
        "bindings": bindings,
        "gestures": [row for row in state.get("templates") or [] if isinstance(row, dict)],
        "mirrorErgonomic": bool(state.get("mirror")),
    }


def gesture_page(dialog: Any, parent: QWidget) -> GesturePage:
    return GesturePage(dialog, parent)
