"""The desktop control channel, as egui's (``docs/control-protocol.md``): ``--control PORT`` (or
``CFA4_CONTROL_PORT``) listens on 127.0.0.1 only; one JSON request per line, one JSON reply per
line. A line that is not a JSON object with a string ``method`` (not JSON, not UTF-8, longer than
4 MiB) gets one error reply and the connection is closed, so an HTTP request cannot smuggle a
command in; at most 16 connections at once.

Methods: ``engine.execute`` (the command runs as given, a UI command in the window),
``ui.menu.invoke`` (as the menu item would: a dialog asks for what is missing), ``engine.commands``
/ ``ui.menu.list``, ``ui.inspect``, ``ui.elements {prefix?}`` (every visible widget with an
automation id: id, label, rect in window coordinates, enabled), ``ui.click`` / ``ui.move``
(``{id}`` or ``{x, y}``, ``button``, ``count``, ``modifiers``), ``ui.drag {from, to, steps,
modifiers}``, ``ui.scroll {id|x,y, dx, dy, modifiers}``, ``ui.key {key}``, ``ui.type {text}``,
``ui.set {tool, workspace, timeline: {pps, scroll, fit}, program / source: {monitor view fields},
clipDialog / menuDialog: {param: value}}``, ``ui.timeline.hit {x, y, modifiers?}`` /
``ui.timeline.locate {clip, edge?}``, ``ui.panel.show`` / ``ui.panel.close
{panel}``, ``ui.playback {action, speed?}``, ``ui.focus {id?}``, ``ui.screenshot {path?, panel?}``,
``ui.resize {width, height}``, ``app.quit`` (no save prompt: unsaved changes stay in the recovery
journal) and ``perf.stats``. ``--control-input-only`` (unattended validation) ignores the native
keyboard, pointer and drops; ``ui.inspect.input.mode`` says which mode runs.
"""

from __future__ import annotations

import base64
import copy
import json
import math
import os
import warnings
from collections.abc import Callable
from dataclasses import asdict
from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QBuffer, QByteArray, QEvent, QIODevice, QObject, QPoint, QPointF, QRect, Qt
from PySide6.QtGui import QKeyEvent, QKeySequence, QWheelEvent
from PySide6.QtNetwork import QHostAddress, QTcpServer, QTcpSocket
from PySide6.QtTest import QTest
from PySide6.QtWidgets import (
    QAbstractButton,
    QApplication,
    QComboBox,
    QDialogButtonBox,
    QDockWidget,
    QDoubleSpinBox,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QSpinBox,
    QWidget,
)

from .panels.clip_dialogs import ClipDialog
from .panels.export_effects import SECTION_KEYS
from .panels.monitor_view import RESOLUTIONS, MonitorView
from .panels.preset_manager import PresetManager
from .panels.quick_export import quick_presets
from .panels.template_dialogs import ExportTemplateDialog, ReplaceFontsDialog
from .panels.timeline import HEADER_W, RULER_H, THUMBNAIL_MODES
from .panels.tools import TOOLS

# egui's ThemeKind::from_name → the appearance.colorTheme preference it sets
THEME_NAMES = {"dark": "darkest", "darkest": "darkest", "medium": "dark", "grey": "dark", "gray": "dark", "light": "light"}


def tool_by_name(name: Any) -> str | None:
    """egui's ``Tool::from_name``: the tool's id or label, any case, without spaces, ``_``, ``-``
    or the word "tool" (``razor``, ``Razor Tool``, ``track_select_forward``)."""
    if not isinstance(name, str):
        return None
    fold = lambda s: s.lower().replace(" ", "").replace("_", "").replace("-", "").replace("tool", "")
    key = fold(name)
    return next((t.id for t in TOOLS.values() if fold(t.id) == key or fold(t.label) == key), None)

if TYPE_CHECKING:
    from .main_window import MainWindow

MAX_LINE = 4 << 20
MAX_CONNECTIONS = 16
MAX_DRAG_STEPS = 200
Reply = Callable[[bool, Any], None]

BUTTONS = {"left": Qt.MouseButton.LeftButton, "right": Qt.MouseButton.RightButton, "middle": Qt.MouseButton.MiddleButton}
MODIFIERS = {
    "shift": Qt.KeyboardModifier.ShiftModifier,
    "ctrl": Qt.KeyboardModifier.ControlModifier,
    "control": Qt.KeyboardModifier.ControlModifier,
    "cmd": Qt.KeyboardModifier.ControlModifier,
    "command": Qt.KeyboardModifier.ControlModifier,
    "alt": Qt.KeyboardModifier.AltModifier,
    "meta": Qt.KeyboardModifier.MetaModifier,
}


EDGE_NAMES = {"in": "In", "out": "Out"}  # egui's Edge names



def _panel_key(name: Any) -> str | None:
    """egui's panel-name folding: lower case without spaces, ``_`` and ``-``."""
    if not isinstance(name, str):
        return None
    return name.lower().replace(" ", "").replace("_", "").replace("-", "")


def panel_dock(docks: dict[str, QDockWidget], name: Any) -> QDockWidget | None:
    """The dock for an egui panel name: ``EffectControls``, ``Effect Controls``, ``effect_controls``…"""
    key = _panel_key(name)
    if key is None:
        return None
    return docks.get(name) or next((d for title, d in docks.items() if _panel_key(title) == key), None)

class ControlError(Exception):
    pass



QUICK_FIELDS = ("quickOpen", "quickPath", "quickPreset")


def _camel(name: str) -> str:
    head, *rest = name.split("_")
    return head + "".join(w.title() for w in rest)


def _snake(name: str) -> str:
    return "".join(f"_{c.lower()}" if c.isupper() else c for c in name)


def _same(a: dict[str, Any], b: dict[str, Any]) -> bool:
    """Panel settings equal up to the order of the scopes listed (the panels keep their own)."""
    def norm(d: dict[str, Any]) -> dict[str, Any]:
        return {k: sorted(map(str, v)) if k in ("shown", "scopes") and isinstance(v, list) else v for k, v in d.items()}

    return norm(a) == norm(b)


def keyframe_refs(value: Any) -> list[tuple[Any, ...]]:
    """egui's ``KeyframeRef`` list ``[{clip, effect, param, mask?, time}]`` as the Effect Controls
    panel's keyframe references (keys it does not show are dropped by the panel)."""
    def whole(v: Any) -> bool:
        return isinstance(v, int) and not isinstance(v, bool) and 0 <= v < 2**63

    if not isinstance(value, list) or len(value) > 100_000:
        raise ControlError("keyframeSelection takes a list of {clip, effect, param, mask?, time}")
    out: list[tuple[Any, ...]] = []
    for k in value:
        if (not isinstance(k, dict) or not whole(k.get("clip")) or not whole(k.get("effect")) or not isinstance(k.get("param"), str)
                or not whole(k.get("time")) or not (k.get("mask") is None or whole(k.get("mask")))):
            raise ControlError("keyframeSelection: each key is {clip, effect, param, mask?, time}")
        out.append((k["clip"], k["effect"], k["param"], k.get("mask"), k["time"]))
    return out

def modifiers_of(value: Any) -> Qt.KeyboardModifier:
    """``["shift", "ctrl"]``, ``"Shift+Ctrl"`` or ``{"shift": true}`` (egui's)."""
    out = Qt.KeyboardModifier.NoModifier
    if isinstance(value, dict):
        names = [k for k, v in value.items() if v is True]
    elif isinstance(value, str):
        names = value.split("+")
    elif isinstance(value, list):
        names = [v for v in value if isinstance(v, str)]
    else:
        names = []
    for n in names:
        m = MODIFIERS.get(n.strip().lower())
        if m is None:
            raise ControlError(f"unknown modifier {n!r}")
        out |= m
    return out


def label_of(w: QWidget) -> str:
    if isinstance(w, (QAbstractButton, QLabel, QLineEdit)):
        text = w.text()
    elif isinstance(w, QComboBox):
        text = w.currentText()
    elif isinstance(w, QDockWidget) or w.isWindow():
        text = w.windowTitle()
    else:
        text = ""
    return text or w.toolTip() or w.accessibleName()


def _number(p: dict[str, Any], key: str, lo: float, hi: float) -> float:
    v = p.get(key)
    if not isinstance(v, (int, float)) or isinstance(v, bool) or not math.isfinite(v) or not lo <= v <= hi:
        raise ControlError(f"{key} must be a number from {lo:g} to {hi:g}")
    return float(v)


# what ``--control-input-only`` keeps from the user's own keyboard, pointer and drops
NATIVE_INPUT = frozenset(
    {
        QEvent.Type.KeyPress,
        QEvent.Type.KeyRelease,
        QEvent.Type.ShortcutOverride,
        QEvent.Type.MouseButtonPress,
        QEvent.Type.MouseButtonRelease,
        QEvent.Type.MouseButtonDblClick,
        QEvent.Type.MouseMove,
        QEvent.Type.Wheel,
        QEvent.Type.ContextMenu,
        QEvent.Type.DragEnter,
        QEvent.Type.DragMove,
        QEvent.Type.Drop,
        QEvent.Type.TabletPress,
        QEvent.Type.TouchBegin,
    }
)


def set_dialog_field(dialog: Any, name: str, value: Any) -> None:
    """egui's ``ui.set {clipDialog | menuDialog: {param: value}}`` on a Qt dialog: the control of
    ``<prefix>.<param>`` is set as a user would (so the dialog's own follow-ups run), a radio
    choice through ``<prefix>.<param>.<value>``; a parameter without a control is set directly."""
    pre = dialog.pre
    w = dialog.findChild(QWidget, f"{pre}.{name}")
    if w is None:
        radio = dialog.findChild(QAbstractButton, f"{pre}.{name}.{value}")
        if radio is not None:
            radio.setChecked(True)
            return
        if name not in dialog.params:
            raise ControlError(f"{pre} has no field {name}")
        dialog.params[name] = value
        return
    if isinstance(w, QAbstractButton) and w.isCheckable():
        if not isinstance(value, bool):
            raise ControlError(f"{name} takes true or false")
        w.setChecked(value)
    elif isinstance(w, QLineEdit):
        w.setText(str(value))
    elif isinstance(w, QPlainTextEdit):
        w.setPlainText(str(value))
    elif isinstance(w, QComboBox):
        i = w.findData(value)
        if i < 0:
            i = w.findText(str(value))
        if i < 0:
            raise ControlError(f"{name} has no choice {value!r}")
        w.setCurrentIndex(i)
        # the dialogs take a choice on `activated` (a user's pick), which this does not emit
        w.activated.emit(i)
    elif isinstance(w, (QSpinBox, QDoubleSpinBox)):
        if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value):
            raise ControlError(f"{name} takes a number")
        w.setValue(int(value) if isinstance(w, QSpinBox) else float(value))
    else:
        raise ControlError(f"{name} cannot be set")


class InputGate(QObject):
    """egui's ``--control-input-only``: for unattended validation, only the control channel's
    input reaches the widgets; the native keyboard, pointer, drops (and so the menus) are
    ignored. Close and screenshots still work. Installed only in that opt-in mode (an app-wide
    filter costs every event)."""

    def __init__(self, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self.own = 0  # > 0 while the channel runs a request: QTest marks its events spontaneous too

    def eventFilter(self, _obj: Any, event: Any) -> bool:
        return self.own == 0 and event.type() in NATIVE_INPUT and event.spontaneous()


class ControlServer(QObject):
    """Serves the control channel for ``window`` from the Qt event loop (no threads)."""

    def __init__(self, window: MainWindow) -> None:
        super().__init__(window)
        self.window = window
        self.server = QTcpServer(self)
        self.server.newConnection.connect(self._accept)
        self.buffers: dict[QTcpSocket, bytearray] = {}
        self.input_gate: InputGate | None = None

    def set_input_only(self, on: bool) -> None:
        """``--control-input-only``: ignore the native input (see :class:`InputGate`)."""
        app = QApplication.instance()
        if on and self.input_gate is None and app is not None:
            self.input_gate = InputGate(self)
            app.installEventFilter(self.input_gate)
        elif not on and self.input_gate is not None:
            if app is not None:
                app.removeEventFilter(self.input_gate)
            self.input_gate = None

    def listen(self, port: int) -> bool:
        """Listen on 127.0.0.1:``port`` (0: any free port, see :meth:`port`)."""
        return self.server.listen(QHostAddress(QHostAddress.SpecialAddress.LocalHost), max(0, min(65535, port)))

    def port(self) -> int:
        return self.server.serverPort()

    def close(self) -> None:
        for s in list(self.buffers):
            s.disconnectFromHost()
        self.server.close()

    # ----- transport -----
    def _accept(self) -> None:
        while self.server.hasPendingConnections():
            s = self.server.nextPendingConnection()
            if s is None:
                return
            if len(self.buffers) >= MAX_CONNECTIONS:
                self._write(s, {"ok": False, "error": f"too many connections (at most {MAX_CONNECTIONS}); closing the connection"})
                s.disconnectFromHost()
                continue
            self.buffers[s] = bytearray()
            s.readyRead.connect(lambda s=s: self._read(s))
            s.disconnected.connect(lambda s=s: self._gone(s))

    def _gone(self, s: QTcpSocket) -> None:
        self.buffers.pop(s, None)
        s.deleteLater()

    def _write(self, s: QTcpSocket, message: dict[str, Any]) -> None:
        if s.state() == QTcpSocket.SocketState.ConnectedState:
            s.write(QByteArray(json.dumps(message, ensure_ascii=False, allow_nan=False, default=str).encode() + b"\n"))

    def _refuse(self, s: QTcpSocket, why: str) -> None:
        self._write(s, {"ok": False, "error": f"{why}; closing the connection"})
        self.buffers.pop(s, None)
        s.readyRead.disconnect()
        s.disconnectFromHost()

    def _read(self, s: QTcpSocket) -> None:
        buf = self.buffers.get(s)
        if buf is None:
            return
        buf += bytes(s.readAll().data())
        while s in self.buffers:
            nl = buf.find(b"\n")
            if nl < 0:
                if len(buf) > MAX_LINE:
                    self._refuse(s, "line longer than 4 MiB")
                return
            line = bytes(buf[:nl]).rstrip(b"\r")
            del buf[: nl + 1]
            if len(line) > MAX_LINE:
                self._refuse(s, "line longer than 4 MiB")
                return
            if not line.strip():
                continue
            try:
                req = json.loads(line.decode("utf-8"))
            except (UnicodeDecodeError, ValueError):
                self._refuse(s, "not a JSON request")
                return
            if not isinstance(req, dict) or not isinstance(req.get("method"), str):
                self._refuse(s, "a request is a JSON object with a string method")
                return
            rid = req.get("id")
            params = req.get("params")

            def reply(ok: bool, value: Any, s: QTcpSocket = s, rid: Any = rid) -> None:
                out: dict[str, Any] = {"id": rid, "ok": ok}
                out["result" if ok else "error"] = value if ok else str(value)
                self._write(s, out)

            gate = self.input_gate
            if gate is not None:
                gate.own += 1
            try:
                self.dispatch(req["method"], params if isinstance(params, dict) else {}, reply)
            except ControlError as e:
                reply(False, str(e))
            except Exception as e:  # noqa: BLE001 - a request must never take the window down
                reply(False, f"{type(e).__name__}: {e}")
            finally:
                if gate is not None:
                    gate.own -= 1

    # ----- methods -----
    def dispatch(self, method: str, p: dict[str, Any], reply: Reply) -> None:
        win = self.window
        if method == "engine.execute":
            cid, params = self._command(p)
            if win.ui_command(cid, params):
                reply(True, {"ui": cid})
                return

            def done(r: Any, _b: bytes | None) -> None:
                win.schedule_refresh()
                reply(True, r)

            win.engine.execute(cid, params, on_reply=done, on_error=lambda m: reply(False, m))
        elif method == "ui.menu.invoke":
            cid, params = self._command(p)
            if params:
                win.run_preset(cid, params)
            else:
                win.invoke(cid)
            reply(True, {"invoked": cid})
        elif method in ("engine.commands", "ui.menu.list"):
            reply(True, [{"id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "enabled": c.enabled} for c in win.registry.commands.values()])
        elif method == "ui.inspect":
            reply(True, self.inspect())
        elif method == "ui.elements":
            prefix = p.get("prefix")
            reply(True, self.elements(prefix if isinstance(prefix, str) else ""))
        elif method in ("ui.click", "ui.move"):
            self.pointer(method == "ui.click", p)
            reply(True, {})
        elif method == "ui.drag":
            self.drag(p)
            reply(True, {})
        elif method == "ui.scroll":
            self.scroll(p)
            reply(True, {})
        elif method == "ui.key":
            self.key(p)
            reply(True, {})
        elif method == "ui.type":
            text = p.get("text")
            if not isinstance(text, str) or len(text) > 100_000:
                raise ControlError("text must be a string")
            self.type_text(self._keyboard_target(), text)
            reply(True, {})
        elif method == "ui.set":
            self.set(p)
            reply(True, self.inspect())
        elif method in ("ui.panel.show", "ui.panel.close"):
            if _panel_key(p.get("panel")) == "program":
                # the Program monitor is the window's centre in Qt: always shown, never closed
                if method == "ui.panel.close":
                    raise ControlError("the Program monitor is the window's centre and stays open")
            elif method == "ui.panel.show":
                dock = self._dock(p.get("panel"))
                dock.show()
                dock.raise_()
            else:
                self._dock(p.get("panel")).close()
            reply(True, {})
        elif method == "ui.timeline.hit":
            reply(True, self.timeline_hit(p))
        elif method == "ui.timeline.locate":
            reply(True, self.timeline_locate(p))
        elif method == "ui.playback":
            self.playback(p)
            reply(True, {})
        elif method == "ui.focus":
            if "id" in p:
                w = self._widget(p.get("id"))
                if not w.isEnabled():
                    raise ControlError(f"{p.get('id')} is disabled")
                w.setFocus(Qt.FocusReason.OtherFocusReason)
            else:
                win.raise_()
                win.activateWindow()
            reply(True, {})
        elif method == "ui.screenshot":
            reply(True, self.screenshot(p))
        elif method == "ui.resize":
            win.resize(int(_number(p, "width", 200, 20_000)), int(_number(p, "height", 150, 20_000)))
            reply(True, {"width": win.width(), "height": win.height()})
        elif method == "app.quit":
            reply(True, {})
            win._quit_confirmed = True
            win.close()
        elif method == "perf.stats":
            def stats(r: Any, _b: Any) -> None:
                out = dict(r) if isinstance(r, dict) else {}
                out["playback"] = win.monitor.playback_stats()  # the Program monitor's, as egui
                out["playback"]["source"] = win.source_monitor.playback_stats()

                def draft(pref: Any, _b2: Any = None) -> None:
                    # the preference itself: the monitors read it only when playback starts
                    on = pref.get("value") if isinstance(pref, dict) else pref
                    if isinstance(on, bool):
                        out["playback"]["draftDecode"] = out["playback"]["source"]["draftDecode"] = on
                    reply(True, out)

                win.engine.execute("prefs.get", {"key": "playback.draftDecode"}, on_reply=draft, on_error=lambda _m: reply(True, out))

            win.engine.execute("perf.stats", {}, on_reply=stats, on_error=lambda m: reply(False, m))
        else:
            raise ControlError(f"unknown method {method}")

    @staticmethod
    def _command(p: dict[str, Any]) -> tuple[str, dict[str, Any]]:
        cid = p.get("command")
        if not isinstance(cid, str) or not cid:
            raise ControlError("command must be a command id")
        params = p.get("params")
        if params is not None and not isinstance(params, dict):
            raise ControlError("params must be an object")
        return cid, dict(params or {})

    def inspect(self) -> dict[str, Any]:
        win = self.window
        canvas = win.timeline.canvas
        dialog = win.last_dialog
        try:
            open_dialog = dialog.objectName() if dialog is not None and dialog.isVisible() else None
        except RuntimeError:  # closed and deleted
            open_dialog = None

        def monitor(m: Any) -> dict[str, Any]:
            # audioClock: the device's played samples drive the picture (egui's); avOffset: how far the
            # frame shown is behind the sound heard, in ticks (under a frame), while playing with sound
            return {
                "playing": m.playing,
                "speed": m.speed,
                "playhead": int(m.playhead),
                "audioClock": m.audio_clock,
                "avOffset": m.av_offset(),
            }

        # the window system's focus (QWindow's), not Qt's active window, which ui.key may point at
        # a background window to route its shortcuts
        handle = win.windowHandle()
        native_focus = bool(handle is not None and handle.isActive())

        return {
            "tool": canvas.tool,
            "workspace": win.workspaces.current,
            "focused": win.active_monitor if win.active_monitor else None,
            "panels": {name: not dock.isHidden() for name, dock in win.docks.items()},
            "timeline": {"pps": canvas.pps, "scroll": int(canvas.scroll_ticks)},
            "playback": {"program": monitor(win.monitor), "source": monitor(win.source_monitor)},
            "dialog": open_dialog,
            "keyboardFocus": self._keyboard_target().objectName(),
            "window": {"width": win.width(), "height": win.height(), "active": native_focus},
            "input": {"mode": "controlOnly" if self.input_gate is not None else "native", "nativeWindowFocused": native_focus},
            "ui": self.ui_state(),
        }

    def _windows(self) -> list[QWidget]:
        """The main window and its own visible dialogs, popups and floating panels (not other
        top-level windows of the process)."""
        win = self.window

        def owned(w: QWidget) -> bool:
            p = w.parentWidget()
            while p is not None:
                if p is win:
                    return True
                p = p.parentWidget()
            return False

        tops = [w for w in QApplication.topLevelWidgets() if w is not win and w.isVisible() and owned(w)]
        for top in tops:
            name_standard_buttons(top)
        return [win, *tops]

    def elements(self, prefix: str) -> list[dict[str, Any]]:
        origin = self.window.mapToGlobal(QPoint(0, 0))
        out: list[dict[str, Any]] = []
        parts = self._parts()
        part_ids = {pid for _w, pid, _r, _label in parts}
        for top in self._windows():
            for w in [top, *top.findChildren(QWidget)]:
                name = w.objectName()
                if not name or not name.startswith(prefix) or name.startswith("qt_") or not w.isVisible() or w.visibleRegion().isEmpty():
                    continue
                if name in part_ids:  # the part's id names it (menu.File: the button, not the popup)
                    continue
                at = w.mapToGlobal(QPoint(0, 0)) - origin
                out.append({"id": name, "label": label_of(w), "rect": [at.x(), at.y(), w.width(), w.height()], "enabled": w.isEnabled()})
        for w, pid, r, label in parts:
            if pid.startswith(prefix):
                at = w.mapToGlobal(r.topLeft()) - origin
                out.append({"id": pid, "label": label, "rect": [at.x(), at.y(), r.width(), r.height()], "enabled": w.isEnabled()})
        return out

    def _parts(self) -> list[tuple[QWidget, str, QRect, str]]:
        """The painted parts of visible widgets that have ``automation_parts()`` (egui registers
        such regions with their own ids: the Audio Meters' readout, bars and scale, …)."""
        out: list[tuple[QWidget, str, QRect, str]] = []
        for top in self._windows():
            for w in [top, *top.findChildren(QWidget)]:
                parts = getattr(w, "automation_parts", None)
                if not callable(parts) or not w.isVisible() or w.visibleRegion().isEmpty():
                    continue
                for pid, rect, label in parts():
                    r = rect.toAlignedRect() if hasattr(rect, "toAlignedRect") else QRect(rect)
                    if r.isValid():
                        out.append((w, str(pid), r, str(label)))
        return out

    def _widget(self, wid: Any) -> QWidget:
        if not isinstance(wid, str) or not wid:
            raise ControlError("id must be an element id")
        for top in reversed(self._windows()):  # dialogs above the window first
            for w in [top, *top.findChildren(QWidget, wid)]:
                if w.objectName() == wid and w.isVisible():
                    return w
        raise ControlError(f"no visible element {wid}")

    def _dock(self, name: Any) -> QDockWidget:
        """The panel named as egui's ``PanelKind::from_name`` takes it: its id (``EffectControls``)
        or its title (``Effect Controls``), ignoring case, spaces, ``_`` and ``-``."""
        dock = panel_dock(self.window.docks, name)
        if dock is None:
            raise ControlError(f"unknown panel {name!r}")
        return dock

    def _at(self, p: Any) -> tuple[QWidget, QPoint]:
        """The widget and local point of ``{id}`` (its centre) or ``{x, y}`` (window coordinates)."""
        if isinstance(p, list) and len(p) == 2:
            p = {"x": p[0], "y": p[1]}
        if not isinstance(p, dict):
            raise ControlError("a point is {id} or {x, y}")
        if "id" in p:
            pid = p.get("id")
            # a painted part (egui's region ids) first: the menu bar's menu.File is the button, not
            # its open popup; the child under the part's centre takes the input
            part = next(((pw, r) for pw, i, r, _label in self._parts() if i == pid), None) if isinstance(pid, str) else None
            if part is not None:
                pw, at = part[0], part[1].center()
                child = pw.childAt(at)
                return (child, child.mapFrom(pw, at)) if child is not None else (pw, at)
            w = self._widget(pid)
            return w, w.rect().center()
        x, y = _number(p, "x", -1e5, 1e5), _number(p, "y", -1e5, 1e5)
        win = self.window
        g = win.mapToGlobal(QPoint(round(x), round(y)))
        for top in reversed(self._windows()):
            local = top.mapFromGlobal(g)
            if top.rect().contains(local):
                w = top.childAt(local) or top
                return w, w.mapFromGlobal(g)
        raise ControlError(f"({x:g}, {y:g}) is outside the window")

    def pointer(self, click: bool, p: dict[str, Any]) -> None:
        w, at = self._at(p)
        mods = modifiers_of(p.get("modifiers"))
        if not click:
            QTest.mouseMove(w, at)
            return
        button = BUTTONS.get(str(p.get("button") or "left"))
        if button is None:
            raise ControlError("button must be left, right or middle")
        # null is the default, as egui reads it (the MCP bridge sends every optional field)
        count = 1 if p.get("count") is None else p["count"]
        if count == 2:
            # a real double click: press and release, then the double-click press and release
            # (item views emit doubleClicked only for the row the first press landed on)
            QTest.mouseClick(w, button, mods, at)
            QTest.mouseDClick(w, button, mods, at)
        elif count == 1:
            QTest.mouseClick(w, button, mods, at)
        else:
            raise ControlError("count must be 1 or 2")

    def drag(self, p: dict[str, Any]) -> None:
        w, start = self._at(p.get("from"))
        target, end = self._at(p.get("to"))
        end = w.mapFromGlobal(target.mapToGlobal(end))
        steps = 8 if p.get("steps") is None else p["steps"]
        if not isinstance(steps, int) or isinstance(steps, bool) or not 1 <= steps <= MAX_DRAG_STEPS:
            raise ControlError(f"steps must be 1 to {MAX_DRAG_STEPS}")
        mods = modifiers_of(p.get("modifiers"))
        QTest.mousePress(w, Qt.MouseButton.LeftButton, mods, start)
        for i in range(1, steps + 1):
            f = i / steps
            QTest.mouseMove(w, QPoint(round(start.x() + (end.x() - start.x()) * f), round(start.y() + (end.y() - start.y()) * f)))
        QTest.mouseRelease(w, Qt.MouseButton.LeftButton, mods, end)

    def scroll(self, p: dict[str, Any]) -> None:
        w, at = self._at(p)
        dx = _number(p, "dx", -1e5, 1e5) if "dx" in p else 0.0
        dy = _number(p, "dy", -1e5, 1e5) if "dy" in p else 0.0
        mods = modifiers_of(p.get("modifiers"))
        pos = QPointF(at)
        delta = QPoint(round(dx), round(dy))
        ev = QWheelEvent(pos, QPointF(w.mapToGlobal(at)), delta, delta, Qt.MouseButton.NoButton, mods, Qt.ScrollPhase.NoScrollPhase, False)
        QApplication.sendEvent(w, ev)

    def _keyboard_target(self) -> QWidget:
        """The focused widget; in a window in the background (the agent never steals the user's
        keyboard), the one that has the window's focus."""
        windows = self._windows()
        focus = QApplication.focusWidget()
        if focus is None or focus.window() not in windows:  # another window of the process has it
            top = next((w for w in reversed(windows) if w.isModal()), self.window)
            focus = top.focusWidget()
        return focus if focus is not None else self.window

    @staticmethod
    def type_text(target: QWidget, text: str) -> None:
        """Type ``text``: ASCII as key clicks (so keys mean what they mean there), any other
        character as a key event carrying it (``QTest.keyClicks`` would pass it through Latin-1)."""
        for ch in text:
            if ord(ch) < 128:
                QTest.keyClicks(target, ch)
            else:
                for kind in (QEvent.Type.KeyPress, QEvent.Type.KeyRelease):
                    QApplication.sendEvent(target, QKeyEvent(kind, 0, Qt.KeyboardModifier.NoModifier, ch))

    def key(self, p: dict[str, Any]) -> None:
        name = p.get("key")
        if not isinstance(name, str) or not name or len(name) > 64:
            raise ControlError("key must be a key name such as Ctrl+K or Space")
        seq = QKeySequence(name.replace("Cmd+", "Ctrl+"))
        if seq.count() != 1 or not seq.toString() or seq[0].key() == Qt.Key.Key_unknown:
            raise ControlError(f"unknown key {name!r}")
        combo = seq[0]
        target = self._keyboard_target()
        if QApplication.activeWindow() is not target.window():
            # as egui's, the channel's keys reach the window without the native keyboard focus (a
            # window in the background, or Wayland, where a client cannot take the focus itself):
            # Qt matches shortcuts only in its active window, so make it that inside Qt alone (the
            # window system's focus is not asked for, and its next focus change takes it back).
            # Deprecated in Qt 6 for apps, but the only way without asking the window system.
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", DeprecationWarning)
                QApplication.setActiveWindow(target.window())
        QTest.keyClick(target, combo.key(), combo.keyboardModifiers())

    def set(self, p: dict[str, Any]) -> None:
        win = self.window
        tool = p.get("tool")
        if tool is not None:
            found = tool_by_name(tool)
            if found is None or not TOOLS[found].available:
                raise ControlError(f"unknown tool {tool!r}")
            win.tools.select(found)
        theme_name = p.get("theme")
        if theme_name is not None:
            pref = THEME_NAMES.get(theme_name.lower()) if isinstance(theme_name, str) else None
            if pref is None:
                raise ControlError("unknown theme (dark, medium, light)")
            win.set_theme(pref)
        res = p.get("playbackRes")
        if res is not None:
            # egui's: the Program monitor's Playback Resolution by its name or label; anything else keeps it
            found_res = next((k for k, label, _s in RESOLUTIONS if isinstance(res, str) and res.lower() in (k, label.lower())), None)
            if found_res is not None:
                win.monitor.set_view(res=found_res)
        search = p.get("effectsSearch")
        if search is not None:
            if not isinstance(search, str):
                raise ControlError("effectsSearch takes the search text")
            win.effects.search.setText(search)
        safe = p.get("safeMargins")
        if isinstance(safe, bool):
            win.monitor.set_view(safe_margins=safe)
        mode = p.get("mode")
        if mode is not None:
            if not isinstance(mode, str):
                raise ControlError("mode is Import, Edit or Export")
            win.set_mode(mode)  # egui's: anything but import/export is Edit
        space = p.get("workspace")
        if space is not None:
            if not isinstance(space, str) or space not in win.workspaces.all_names():
                raise ControlError(f"unknown workspace {space!r}")
            win.workspaces.switch(space, win.restore_layout)
        for name, mon in (("program", win.monitor), ("source", win.source_monitor)):
            fields = p.get(name)
            if fields is None:
                continue
            known = asdict(mon.mv)
            if not isinstance(fields, dict) or any(k not in known for k in fields):
                raise ControlError(f"{name} takes monitor view fields: {', '.join(known)}")
            merged = MonitorView.from_dict({**known, **fields})
            mon.set_view(**{k: getattr(merged, k) for k in fields})
        for key in ("clipDialog", "menuDialog"):
            fields = p.get(key)
            if fields is None:
                continue
            dialog = win.last_dialog
            try:
                shown = dialog is not None and dialog.isVisible()
            except RuntimeError:  # closed and deleted
                shown = False
            if not shown or not isinstance(dialog, ClipDialog):
                raise ControlError(f"{key}: no dialog is open")
            if not isinstance(fields, dict):
                raise ControlError(f"{key} takes {{param: value}}")
            for name, value in fields.items():
                set_dialog_field(dialog, str(name), value)
        if "settings" in p:
            # the open Preferences dialog: page and draft values, egui's
            dialog = getattr(win, "settings_dialog", None)
            try:
                shown = dialog is not None and dialog.isVisible() and dialog.ready
            except RuntimeError:  # closed and deleted
                shown = False
            if not shown:
                raise ControlError("the Settings dialog is not open")
            try:
                dialog.patch(p["settings"])
            except (ValueError, TypeError) as e:
                raise ControlError(str(e)) from None
        if "gfxTemplates" in p:
            self.set_gfx_templates(p["gfxTemplates"])
        if "panels" in p:
            self.set_panels(p["panels"])
        for key, panel in (("projectPanel", win.project_panel), ("mediaBrowser", win.media_browser), ("tts", win.tts)):
            if key not in p:
                continue
            if not isinstance(p[key], dict):
                raise ControlError(f"{key} takes {{field: value}}")
            try:
                panel.set_ui_state(p[key])
            except (ValueError, TypeError) as e:
                raise ControlError(f"{key}: {e}") from None
        if "keyframeSelection" in p:
            win.effect_controls.set_keyframe_selection(keyframe_refs(p["keyframeSelection"]))
        if "export" in p:
            self.set_export(p["export"])
        tl = p.get("timeline")
        if isinstance(tl, dict):
            canvas = win.timeline.canvas
            if "pps" in tl:
                canvas.pps = _number(tl, "pps", 0.5, 4000)
            if "scroll" in tl:
                canvas.scroll_ticks = int(_number(tl, "scroll", 0, 2**62))
            if tl.get("fit") is True:
                canvas.zoom_to_fit()
            heights = {k: _number(tl, f"{k}TrackHeight", 1, 10_000) for k in ("video", "audio") if f"{k}TrackHeight" in tl}
            if heights:
                canvas.set_track_heights(heights.get("video"), heights.get("audio"))
            thumbs = tl.get("thumbnails")
            if isinstance(thumbs, str):
                # Show Video Thumbnails: off or a mode (head, headAndTail, continuous), egui's
                mode = next((m for m, _l in THUMBNAIL_MODES if m.lower() == thumbs.lower()), None)
                if thumbs.lower() == "off":
                    canvas._set_display(show_thumbnails=False)
                elif mode is None:
                    raise ControlError(f"unknown thumbnail mode {thumbs!r} (off, head, headAndTail, continuous)")
                else:
                    canvas._set_display(show_thumbnails=True, thumbnail_mode=mode)
            canvas.update()

    # ----- panel state (the serde parts of egui's ``UiState``) -----
    def ui_state(self) -> dict[str, Any]:
        win = self.window
        keys = [{"clip": c, "effect": e, "param": prm, **({"mask": m} if m is not None else {}), "time": t}
                for c, e, prm, m, t in win.effect_controls.keyframe_selection]
        return {
            "mode": win.mode,
            "panels": self.panels_state(),
            "projectPanel": win.project_panel.ui_state(),
            "mediaBrowser": win.media_browser.ui_state(),
            "tts": win.tts.ui_state(),
            "gfxTemplates": self.gfx_templates_state(),
            "keyframeSelection": keys,
            "export": self.export_state(),
        }

    def _template_dialog(self, cls: type) -> Any:
        dialog = self.window.template_dialog
        try:
            return dialog if isinstance(dialog, cls) and dialog.isVisible() else None
        except RuntimeError:  # closed and deleted
            return None

    def gfx_templates_state(self) -> dict[str, Any]:
        """egui's ``GfxTemplatesState``: Essential Graphics ▸ Browse and the template / font
        dialogs (null while closed)."""
        export = self._template_dialog(ExportTemplateDialog)
        replace = self._template_dialog(ReplaceFontsDialog)
        return {**self.window.essential_graphics.ui_state(), "export": export.draft() if export else None,
                "replace": replace.draft() if replace else None}

    def set_gfx_templates(self, patch: Any) -> None:
        """egui's ``ui.set {gfxTemplates}``: merged in; ``export`` / ``replace`` merge into the open
        dialog's draft and null closes it (File ▸ Export ▸ Motion Graphics Template… and
        Replace Fonts open them)."""
        if not isinstance(patch, dict):
            raise ControlError("gfxTemplates takes {tab, query, category, selected, export, replace}")
        dialogs = {"export": self._template_dialog(ExportTemplateDialog), "replace": self._template_dialog(ReplaceFontsDialog)}
        for key, dialog in dialogs.items():
            v = patch.get(key)
            if key in patch and v is not None:
                if not isinstance(v, dict):
                    raise ControlError(f"gfxTemplates.{key} takes {{field: value}} or null")
                if dialog is None:
                    raise ControlError(f"gfxTemplates.{key}: the dialog is not open")
        panel = {k: v for k, v in patch.items() if k not in dialogs}
        try:  # check every part before changing any
            self.window.essential_graphics.check_ui_state(panel)
            for key, dialog in dialogs.items():
                if isinstance(patch.get(key), dict):
                    dialog.check_draft(patch[key])
        except (ValueError, TypeError) as e:
            raise ControlError(f"gfxTemplates: {e}") from None
        self.window.essential_graphics.set_ui_state(panel)
        for key, dialog in dialogs.items():
            if key in patch and patch[key] is None and dialog is not None:
                dialog.reject()
            elif isinstance(patch.get(key), dict):
                dialog.set_draft(patch[key])

    def panels_state(self) -> dict[str, Any]:
        win = self.window
        tc, ref = win.timecode_panel, win.reference
        return {
            "scopes": {_camel(k): v for k, v in asdict(win.scopes.view.st).items()},
            "timecode": {"rows": [dict(r) for r in tc.rows], "showName": tc.show_name},
            "reference": {"ganged": ref.ganged, "time": ref.time, "display": ref.display.currentData(), "scopes": list(ref.scope_kinds)},
            "progress": {"showFinished": win.progress.finished.isChecked()},
            "events": win.events.state(),
            "inspectorSource": win.inspector_source,
        }

    def set_panels(self, patch: Any) -> None:
        """egui's ``ui.set {panels}``: Lumetri Scopes, Timecode, Reference Monitor and Progress
        settings merged in, and which selection Info and Metadata follow. A value the panel would
        not take is refused and changes nothing."""
        win = self.window
        if not isinstance(patch, dict):
            raise ControlError("panels takes {scopes, timecode, reference, progress, events, inspectorSource}")
        known = ("scopes", "timecode", "reference", "progress", "events", "inspectorSource")
        bad = [k for k in patch if k not in known]
        if bad:
            raise ControlError(f"panels: no {bad[0]!r} in the Qt panels (known: {', '.join(known)})")
        for k in ("scopes", "timecode", "reference", "progress", "events"):
            if k in patch and not isinstance(patch[k], dict):
                raise ControlError(f"panels.{k} takes {{field: value}}")
        src = patch.get("inspectorSource")
        if src is not None and src not in ("project", "timeline"):
            raise ControlError("panels.inspectorSource is project or timeline")
        show = patch.get("progress", {}).get("showFinished")
        if show is not None and not isinstance(show, bool):
            raise ControlError("panels.progress.showFinished takes true or false")
        bad = [f for f in patch.get("progress", {}) if f != "showFinished"]
        if bad:
            raise ControlError(f"panels.progress: no field {bad[0]!r}")
        events = patch.get("events", {})
        bad = [f for f in events if f not in ("level", "selected")]
        if bad:
            raise ControlError(f"panels.events: no field {bad[0]!r} (known: level, selected)")
        if "level" in events and events["level"] not in ("info", "warning", "error"):
            raise ControlError("panels.events.level is info, warning or error")
        sel = events.get("selected")
        if sel is not None and (not isinstance(sel, int) or isinstance(sel, bool) or sel < 0):
            raise ControlError("panels.events.selected takes an entry id or null")
        cur = self.panels_state()
        # the panels load their saved settings with validation: write the merged state, reload, and
        # put the old one back when a value did not stick
        stored = {"scopes": "panels/scopes", "timecode": "panels/timecode", "reference": "panels/reference"}
        panels = {"scopes": win.scopes, "timecode": win.timecode_panel, "reference": win.reference}
        settings = win.settings
        for k, key in stored.items():
            if k not in patch:
                continue
            unknown = [f for f in patch[k] if f not in cur[k]]
            if unknown:
                raise ControlError(f"panels.{k}: no field {unknown[0]!r} (known: {', '.join(cur[k])})")
            merged = {**cur[k], **patch[k]}
            saved = {_snake(f): v for f, v in merged.items()} if k == "scopes" else merged
            old = settings.value(key)
            settings.setValue(key, json.dumps(saved))
            panels[k].attach_settings(settings)
            if not _same(self.panels_state()[k], merged):
                if old is None:
                    settings.remove(key)
                else:
                    settings.setValue(key, old)
                panels[k].attach_settings(settings)
                raise ControlError(f"panels.{k}: a value is not one the panel takes")
        if "timecode" in patch:
            win.timecode_panel.changed()
        if "reference" in patch:
            win.reference.request()
        if "scopes" in patch:
            win.scopes.view.request()
        if show is not None:
            win.progress.finished.setChecked(show)
        if events:
            win.events.set_state(events)
        if src is not None:
            win._follow(src)

    def export_state(self) -> dict[str, Any]:
        """egui's ``ExportUi``: the header's Quick Export popup (``quickOpen``, ``quickPath``,
        ``quickPreset``) and, while the Export window is open, its choices."""
        return {**(self.export_window_state() or {}), **self.quick_export_state()}

    def quick_export_state(self) -> dict[str, Any]:
        q = getattr(self.window, "quick_export", None)
        try:
            shown = q is not None and q.isVisible()
        except RuntimeError:
            shown, q = False, None
        return {"quickOpen": shown, "quickPath": q.path if q is not None else "", "quickPreset": q.preset if q is not None else ""}

    def set_quick_export(self, patch: dict[str, Any]) -> None:
        """``quickOpen`` opens (under the header button, as a click does) or closes the popup;
        ``quickPreset`` picks a preset of its list; ``quickPath`` sets the file, fitted to the
        preset's extension as typing it would be."""
        self.check_quick_export(patch)
        win = self.window
        q = getattr(win, "quick_export", None)
        name = patch.get("quickPreset")
        if patch.get("quickOpen") is True and not self.quick_export_state()["quickOpen"]:
            win.toggle_quick_export()
            q = win.quick_export
        elif patch.get("quickOpen") is False and q is not None and q.isVisible():
            q.close()
        if q is None:
            return
        if name is not None:
            q.choose(name)
        if "quickPath" in patch:
            q.path = os.path.expanduser(patch["quickPath"])
            q._fit_path()

    def check_quick_export(self, patch: dict[str, Any]) -> None:
        win = self.window
        if "quickOpen" in patch and not isinstance(patch["quickOpen"], bool):
            raise ControlError("export.quickOpen takes true or false")
        for k in ("quickPath", "quickPreset"):
            if k in patch and not isinstance(patch[k], str):
                raise ControlError(f"export.{k} takes a string")
        q = getattr(win, "quick_export", None)
        name = patch.get("quickPreset")
        if name is not None and q is not None and q.presets and name.lower() not in {n.lower() for n in quick_presets(q.presets)}:
            raise ControlError(f"export.quickPreset: {name!r} is not in the Quick Export list")
        opening = patch.get("quickOpen") is True and not self.quick_export_state()["quickOpen"]
        if opening and (win.sequence or {}).get("id") is None:
            raise ControlError("export.quickOpen: open a sequence first")
        if q is None and not opening and patch.keys() - {"quickOpen"}:
            raise ControlError("export: Quick Export has not been opened yet")

    def export_window_state(self) -> dict[str, Any] | None:
        """The Export window's choices (null while it is closed)."""
        d = getattr(self.window, "export_dialog", None)
        try:
            if d is None or not d.isVisible():
                return None
        except RuntimeError:
            return None
        manager = d.preset_manager
        try:
            shown = manager is not None and manager.isVisible()
        except RuntimeError:
            shown = False
        return {"preset": d.preset.currentData(), "format": d.format.currentData(), "path": d.path.text(), "range": d.range.currentData(),
                "customStart": d.start.value(), "customEnd": d.end.value(), "settings": copy.deepcopy(d.resolved_settings),
                "openSections": d.open_sections(), "manager": manager.state() if shown and manager is not None else None}

    def set_export(self, patch: Any) -> None:
        if not isinstance(patch, dict):
            raise ControlError("export takes {quickOpen, quickPath, quickPreset} and the open Export window's fields")
        quick = {k: v for k, v in patch.items() if k in QUICK_FIELDS}
        window = {k: v for k, v in patch.items() if k not in QUICK_FIELDS}
        if quick and window and self.export_window_state() is None:
            raise ControlError("export: the Export window is not open")
        self.check_quick_export(quick)
        if window:
            self.set_export_window(window)
        self.set_quick_export(quick)

    def set_export_window(self, patch: Any) -> None:
        cur = self.export_window_state()
        if cur is None:
            raise ControlError("export: the Export window is not open")
        if not isinstance(patch, dict):
            raise ControlError("export takes {preset, format, path, range, customStart, customEnd}")
        unknown = [f for f in patch if f not in cur]
        if unknown:
            raise ControlError(f"export: no field {unknown[0]!r} (known: {', '.join(cur)})")
        d = self.window.export_dialog
        combos = {"preset": d.preset, "format": d.format, "range": d.range}
        for f, box in combos.items():
            if f in patch and box.findData(patch[f]) < 0:
                raise ControlError(f"export.{f}: no {patch[f]!r}")
        for f in ("customStart", "customEnd"):
            if f in patch and (isinstance(patch[f], bool) or not isinstance(patch[f], (int, float)) or not math.isfinite(patch[f])):
                raise ControlError(f"export.{f} takes seconds")
        if "path" in patch and not isinstance(patch["path"], str):
            raise ControlError("export.path takes a file path")
        settings = patch.get("settings")
        if "settings" in patch:
            if not isinstance(settings, dict):
                raise ControlError("export.settings takes {field: value}, merged into the export settings")
            have = set(d.resolved_settings)
            unknown = [k for k in settings if have and k not in have and _snake(k) not in have and _camel(k) not in have]
            if unknown:
                raise ControlError(f"export.settings: no setting {unknown[0]!r}")
        sections = patch.get("openSections")
        if "openSections" in patch and (not isinstance(sections, list) or any(k not in SECTION_KEYS for k in sections)):
            raise ControlError(f"export.openSections takes a list of {', '.join(SECTION_KEYS)}")
        manager = patch.get("manager")
        if manager is not None and not isinstance(manager, dict):
            raise ControlError("export.manager takes {query, favoritesOnly, selected, saveName} or null")
        if isinstance(manager, dict):
            try:
                PresetManager.check_state(manager, [str(p.get("name")) for p in d.presets if isinstance(p, dict)])
            except ValueError as e:
                raise ControlError(f"export.{e}") from None
        # the preset first: it sets the format and the rest
        for f in ("preset", "format", "range"):
            if f in patch:
                i = combos[f].findData(patch[f])
                if f == "preset" and i == d.preset.currentIndex():
                    d._preset_changed()  # egui's: naming the preset applies its settings again
                combos[f].setCurrentIndex(i)
        if "path" in patch:
            d.path.setText(patch["path"])
        if "customStart" in patch:
            d.start.setValue(float(patch["customStart"]))
        if "customEnd" in patch:
            d.end.setValue(float(patch["customEnd"]))
        if isinstance(sections, list):
            d.set_open_sections(sections)
        if isinstance(settings, dict) and settings:
            d.set_agent_settings(settings)
        if "manager" in patch:
            self._set_manager(manager)

    def _set_manager(self, fields: dict[str, Any] | None) -> None:
        """The Preset Manager: null closes it, fields open it (on the current preset) and fill it."""
        d = self.window.export_dialog
        m = d.preset_manager
        try:
            shown = m is not None and m.isVisible()
        except RuntimeError:
            shown = False
        if fields is None:
            if shown and m is not None:
                m.reject()
            return
        if not shown:
            d.open_preset_manager()
        names = [str(p.get("name")) for p in d.presets if isinstance(p, dict)]
        d.preset_manager.set_state(fields, names)

    # ----- timeline hit-testing (egui's ``timeline::hit_json`` / ``locate``) -----
    def _canvas_point(self, p: dict[str, Any]) -> QPoint:
        canvas = self.window.timeline.canvas
        x, y = _number(p, "x", -1e5, 1e5), _number(p, "y", -1e5, 1e5)
        return canvas.mapFromGlobal(self.window.mapToGlobal(QPoint(round(x), round(y))))

    def timeline_hit(self, p: dict[str, Any]) -> dict[str, Any]:
        """What the timeline shows at window point ``{x, y}`` and ``kind``, the trim a press of the
        current tool with ``modifiers`` starts there (``trim``, ``ripple``, ``roll``,
        ``rateStretch``, ``remix``, ``volume`` on an audio clip's Volume line, or null)."""
        canvas = self.window.timeline.canvas
        pos = self._canvas_point(p)
        mods = modifiers_of(p.get("modifiers"))
        if not canvas.sequence or not canvas.rect().contains(pos):
            return {"hit": "none"}
        t = canvas.time_at(pos.x())
        if pos.y() < RULER_H and pos.x() >= HEADER_W:
            return {"hit": "ruler", "time": t}
        tool = canvas.tool
        if tool in ("selection", "pen"):
            line = canvas.volume_line_at(pos)
            if line is not None:
                row, clip, _r = line
                return {"hit": "clip", "track": row.track_id, "clip": clip.get("clip"), "edge": None, "kind": "volume", "time": t}
        th = canvas.transition_hit(pos)
        if th is not None:
            row, x, edge = th
            return {"hit": "transition", "track": row.track_id, "transition": x.get("id"), "edge": EDGE_NAMES.get(edge)}
        row, clip, edge = canvas.hit(pos)
        if row is None:
            return {"hit": "none"}
        if clip is None:
            return {"hit": "empty", "track": row.track_id, "time": t}
        kind = None
        if edge:
            additive = bool(mods & (Qt.KeyboardModifier.ShiftModifier | Qt.KeyboardModifier.ControlModifier))
            if tool == "selection" and not additive:
                kind = "trim"
            elif tool == "ripple":
                kind = "ripple"
            elif tool == "rolling":
                kind = "roll" if canvas.neighbour(row, clip, edge) is not None else "trim"
            elif tool == "rateStretch":
                kind = "rateStretch"
            elif tool == "remix" and edge == "out":
                kind = "remix"
        return {"hit": "clip", "track": row.track_id, "clip": clip.get("clip"), "edge": EDGE_NAMES.get(edge), "kind": kind, "time": t}

    def timeline_locate(self, p: dict[str, Any]) -> dict[str, Any]:
        """The window point of a clip's middle, or 2 px inside its ``edge`` ("in" / "out")."""
        canvas = self.window.timeline.canvas
        cid = p.get("clip")
        edge = p.get("edge")
        if not isinstance(cid, int) or isinstance(cid, bool):
            raise ControlError("need `clip`")
        if edge not in (None, "in", "out"):
            raise ControlError("edge must be in or out")
        for row in canvas.rows:
            for clip in row.track.get("items") or []:
                if isinstance(clip, dict) and clip.get("clip") == cid:
                    r = canvas.clip_rect(row, clip)
                    x = r.left() + 2 if edge == "in" else r.right() - 2 if edge == "out" else r.center().x()
                    at = canvas.mapToGlobal(QPoint(round(x), round(row.top + row.height / 2))) - self.window.mapToGlobal(QPoint(0, 0))
                    if not canvas.rect().contains(canvas.mapFromGlobal(self.window.mapToGlobal(at))):
                        break
                    return {"x": at.x(), "y": at.y()}
        raise ControlError("clip not visible in the timeline")

    def playback(self, p: dict[str, Any]) -> None:
        mon = self.window.focused_monitor() if p.get("monitor") is None else self.window.source_monitor if p.get("monitor") == "source" else self.window.monitor
        action = p.get("action")
        if action == "play":
            if not mon.playing:
                mon.toggle_playback()
        elif action == "stop":
            mon.stop()
        elif action == "toggle":
            mon.toggle_playback()
        else:
            raise ControlError("action must be play, stop or toggle")

    def screenshot(self, p: dict[str, Any]) -> dict[str, Any]:
        panel = p.get("panel")
        target: QWidget = self.window if panel is None else self.window.monitor if _panel_key(panel) == "program" else self._dock(panel)
        image = target.grab()
        path = p.get("path")
        if path is not None:
            if not isinstance(path, str) or not path.strip():
                raise ControlError("path must be a file path")
            if not image.save(path, "PNG"):
                raise ControlError(f"cannot write {path}")
            return {"path": path, "width": image.width(), "height": image.height()}
        data = QByteArray()
        buf = QBuffer(data)
        buf.open(QIODevice.OpenModeFlag.WriteOnly)
        image.save(buf, "PNG")
        return {"png": base64.b64encode(bytes(data.data())).decode(), "width": image.width(), "height": image.height()}


def name_standard_buttons(dialog: QWidget) -> None:
    """egui's ids for a dialog's standard buttons: an unnamed OK / Cancel / Close / … of a named
    dialog's button box becomes ``<dialog>.ok`` / ``.cancel`` / ``.close`` (egui's
    ``mediaBrowser.columns.ok``, ``savePreset.cancel``), so an agent clicks it by id. A name the
    dialog gave its button stays."""
    prefix = dialog.objectName()
    if not prefix:
        return
    for box in dialog.findChildren(QDialogButtonBox):
        if box.window() is not dialog:
            continue
        for button in box.buttons():
            if button.objectName():
                continue
            role = box.standardButton(button)
            if role == QDialogButtonBox.StandardButton.NoButton:
                continue
            key = role.name  # Ok, Cancel, RestoreDefaults, …
            button.setObjectName(f"{prefix}.{key[0].lower()}{key[1:]}")


__all__ = ["MAX_CONNECTIONS", "MAX_LINE", "ControlError", "ControlServer", "modifiers_of", "name_standard_buttons"]
