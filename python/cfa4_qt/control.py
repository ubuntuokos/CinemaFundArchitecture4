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
import json
import math
from collections.abc import Callable
from dataclasses import asdict
from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QBuffer, QByteArray, QEvent, QIODevice, QObject, QPoint, QPointF, Qt
from PySide6.QtGui import QKeyEvent, QKeySequence, QWheelEvent
from PySide6.QtNetwork import QHostAddress, QTcpServer, QTcpSocket
from PySide6.QtTest import QTest
from PySide6.QtWidgets import (
    QAbstractButton,
    QApplication,
    QComboBox,
    QDockWidget,
    QDoubleSpinBox,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QSpinBox,
    QWidget,
)

from .panels.clip_dialogs import ClipDialog
from .panels.monitor_view import MonitorView
from .panels.timeline import HEADER_W, RULER_H
from .panels.tools import TOOLS

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


class ControlError(Exception):
    pass


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

    def eventFilter(self, _obj: Any, event: Any) -> bool:
        # the control channel's events are posted by the app itself (not spontaneous)
        return event.type() in NATIVE_INPUT and event.spontaneous()


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

            try:
                self.dispatch(req["method"], params if isinstance(params, dict) else {}, reply)
            except ControlError as e:
                reply(False, str(e))
            except Exception as e:  # noqa: BLE001 - a request must never take the window down
                reply(False, f"{type(e).__name__}: {e}")

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
            dock = self._dock(p.get("panel"))
            if method == "ui.panel.show":
                dock.show()
                dock.raise_()
            else:
                dock.close()
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
            win.engine.execute("perf.stats", {}, on_reply=lambda r, _b: reply(True, r), on_error=lambda m: reply(False, m))
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
            return {"playing": m.playing, "speed": m.speed, "playhead": int(m.playhead)}

        return {
            "tool": canvas.tool,
            "workspace": win.workspaces.current,
            "focused": win.active_monitor if win.active_monitor else None,
            "panels": {name: not dock.isHidden() for name, dock in win.docks.items()},
            "timeline": {"pps": canvas.pps, "scroll": int(canvas.scroll_ticks)},
            "playback": {"program": monitor(win.monitor), "source": monitor(win.source_monitor)},
            "dialog": open_dialog,
            "keyboardFocus": self._keyboard_target().objectName(),
            "window": {"width": win.width(), "height": win.height(), "active": win.isActiveWindow()},
            "input": {"mode": "controlOnly" if self.input_gate is not None else "native", "nativeWindowFocused": win.isActiveWindow()},
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
        return [win, *tops]

    def elements(self, prefix: str) -> list[dict[str, Any]]:
        origin = self.window.mapToGlobal(QPoint(0, 0))
        out: list[dict[str, Any]] = []
        for top in self._windows():
            for w in [top, *top.findChildren(QWidget)]:
                name = w.objectName()
                if not name or not name.startswith(prefix) or name.startswith("qt_") or not w.isVisible() or w.visibleRegion().isEmpty():
                    continue
                at = w.mapToGlobal(QPoint(0, 0)) - origin
                out.append({"id": name, "label": label_of(w), "rect": [at.x(), at.y(), w.width(), w.height()], "enabled": w.isEnabled()})
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
        dock = self.window.docks.get(name) if isinstance(name, str) else None
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
            w = self._widget(p.get("id"))
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
        count = p.get("count", 1)
        if count == 2:
            QTest.mouseDClick(w, button, mods, at)
        elif count == 1:
            QTest.mouseClick(w, button, mods, at)
        else:
            raise ControlError("count must be 1 or 2")

    def drag(self, p: dict[str, Any]) -> None:
        w, start = self._at(p.get("from"))
        target, end = self._at(p.get("to"))
        end = w.mapFromGlobal(target.mapToGlobal(end))
        steps = p.get("steps", 8)
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
        QTest.keyClick(self._keyboard_target(), combo.key(), combo.keyboardModifiers())

    def set(self, p: dict[str, Any]) -> None:
        win = self.window
        tool = p.get("tool")
        if tool is not None:
            if not isinstance(tool, str) or tool not in TOOLS or not TOOLS[tool].available:
                raise ControlError(f"unknown tool {tool!r}")
            win.tools.select(tool)
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
        tl = p.get("timeline")
        if isinstance(tl, dict):
            canvas = win.timeline.canvas
            if "pps" in tl:
                canvas.pps = _number(tl, "pps", 0.5, 4000)
            if "scroll" in tl:
                canvas.scroll_ticks = int(_number(tl, "scroll", 0, 2**62))
            if tl.get("fit") is True:
                canvas.zoom_to_fit()
            canvas.update()

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
        target: QWidget = self._dock(panel) if panel is not None else self.window
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


__all__ = ["MAX_CONNECTIONS", "MAX_LINE", "ControlError", "ControlServer", "modifiers_of"]
