"""Mouse-gesture HUD (CFA4-CONTROL-SURFACE-GESTURE-001, MG3).

The overlay sits inside the CFA4 window and does not take mouse events. It draws what
``surface.gesture.sample`` already decided. Recognition stays in the engine.
"""

from __future__ import annotations

import time
from typing import Any

from PySide6.QtCore import QEvent, QObject, Qt
from PySide6.QtGui import QColor, QMouseEvent, QPainter, QPen
from PySide6.QtWidgets import (
    QAbstractSpinBox,
    QApplication,
    QDialog,
    QLineEdit,
    QMenu,
    QPlainTextEdit,
    QScrollBar,
    QSplitterHandle,
    QTextEdit,
    QWidget,
)

from .i18n import tr


def qt_int(value: object) -> int:
    number = getattr(value, "value", None)
    return number if isinstance(number, int) else 0


class GestureHud(QWidget):
    """An in-window layer. Mouse events pass through it."""

    def __init__(self, parent: QWidget) -> None:
        super().__init__(parent)
        self.setObjectName("gesture.hud")
        self.setAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents, True)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self._frame: dict[str, Any] = {}
        self.hide()

    def set_frame(self, frame: dict[str, Any] | None) -> None:
        self._frame = frame or {}
        show = bool(self._frame.get("show"))
        self.setVisible(show)
        parent = self.parentWidget()
        if show and parent is not None:
            self.setGeometry(parent.rect())
            self.raise_()
        self.update()

    def paintEvent(self, _event: QEvent) -> None:
        frame = self._frame
        if not frame.get("show"):
            return
        painter = QPainter(self)
        contrast = bool(frame.get("contrast"))
        ink = QColor(255, 255, 255) if contrast else QColor(80, 160, 255)
        painter.setPen(QPen(ink, 2))
        raw_trail = frame.get("trail")
        trail = raw_trail if isinstance(raw_trail, list) else []
        if not frame.get("reduced"):
            previous: tuple[int, int] | None = None
            for point in trail:
                if isinstance(point, (list, tuple)) and len(point) == 2:
                    try:
                        here = (int(point[0]), int(point[1]))
                    except (TypeError, ValueError):
                        continue
                    if previous is not None:
                        painter.drawLine(previous[0], previous[1], here[0], here[1])
                    previous = here
        lines = _lines(frame)
        if not lines:
            painter.end()
            return
        painter.fillRect(12, 12, 280, 18 * len(lines) + 12, QColor(0, 0, 0, 180 if contrast else 120))
        painter.setPen(ink)
        for index, line in enumerate(lines):
            painter.drawText(20, 30 + index * 18, line)
        painter.end()


def _panel_widget(name: str) -> bool:
    return name.startswith(("timeline.", "mediaBrowser.", "panel.menu.Timeline", "panel.menu.MediaBrowser")) or name in {
        "panel.Timeline",
        "panel.MediaBrowser",
    }


def _widget_name(watched: QObject) -> str:
    widget: QWidget | None = watched if isinstance(watched, QWidget) else None
    names: list[str] = []
    while widget is not None:
        name = widget.objectName()
        if name:
            names.append(name)
        widget = widget.parentWidget()
    for name in names:
        if _panel_widget(name):
            return name
    return names[0] if names else ""


def _conflicts(watched: QObject, event: QMouseEvent, code: int) -> bool:
    if event.type() == QEvent.Type.MouseButtonDblClick:
        return True
    if code in (qt_int(Qt.MouseButton.RightButton), qt_int(Qt.MouseButton.MiddleButton)):
        return True
    app = QApplication.instance()
    if isinstance(app, QApplication) and (app.activeModalWidget() is not None or app.activePopupWidget() is not None):
        return True
    widget: QWidget | None = watched if isinstance(watched, QWidget) else None
    protected = (QScrollBar, QSplitterHandle, QLineEdit, QTextEdit, QPlainTextEdit, QAbstractSpinBox, QDialog, QMenu)
    while widget is not None:
        if isinstance(widget, protected):
            return True
        widget = widget.parentWidget()
    return False


def _lines(frame: dict[str, Any]) -> list[str]:
    lines: list[str] = []
    if frame.get("cancelled"):
        lines.append(tr("Cancelled"))
    direction = frame.get("direction")
    if isinstance(direction, str) and direction:
        lines.append(direction)
    hint = frame.get("hint")
    if isinstance(hint, str) and hint:
        lines.append(hint)
    error = frame.get("error")
    if isinstance(error, str) and error:
        lines.append(error)
    return lines


class GestureLayer(QObject):
    """Follows the window size, and feeds the engine only while gestures are armed.

    Armed is off until the user turns it on. While it is off, mouse events are left
    alone, so the timeline and the monitors keep their own behaviour.
    """

    def __init__(self, window: QWidget, engine: Any) -> None:
        super().__init__(window)
        self._window = window
        self._engine = engine
        self._hud = GestureHud(window)
        self._armed = False
        self._down: set[int] = set()
        window.installEventFilter(self)
        app = QApplication.instance()
        if app is not None:
            app.installEventFilter(self)
        self.refresh()

    @property
    def hud(self) -> GestureHud:
        return self._hud

    def refresh(self) -> None:
        self._engine.execute("surface.gesture.inspect", {}, on_reply=self._inspected, on_error=lambda _m: None)

    def show_frame(self, frame: dict[str, Any] | None) -> None:
        self._hud.set_frame(frame)

    def eventFilter(self, watched: QObject, event: QEvent) -> bool:
        # Qt can deliver this while the object is being constructed or destroyed.
        window = getattr(self, "_window", None)
        hud = getattr(self, "_hud", None)
        if not isinstance(window, QWidget) or not isinstance(hud, GestureHud) or not hasattr(self, "_down"):
            return False
        if watched is window and event.type() == QEvent.Type.Resize:
            hud.setGeometry(window.rect())
            return False
        if not self._armed:
            return False
        if event.type() == QEvent.Type.WindowDeactivate and watched is window:
            self._send({"op": "focus", "from": "window", "tNs": time.monotonic_ns()})
            self._down.clear()
            return False
        if event.type() == QEvent.Type.KeyPress and watched is window and self._down:
            key = getattr(event, "key", lambda: None)()
            if key == Qt.Key.Key_Escape:
                self._send({"op": "cancel", "from": "window", "tNs": time.monotonic_ns()})
                self._down.clear()
            return False
        if not isinstance(event, QMouseEvent):
            return False
        self._observe(watched, event)
        return False

    def _observe(self, watched: QObject, event: QMouseEvent) -> None:
        kind = event.type()
        code = qt_int(event.button())
        # A double-click, the menu button and the middle button stay with the panels.
        # The sample is only a report: this filter never accepts the event.
        if kind == QEvent.Type.MouseButtonDblClick:
            self._send(self._sample("cancel", event, code, watched))
            self._down.clear()
            return
        primary = qt_int(Qt.MouseButton.LeftButton)
        if kind == QEvent.Type.MouseButtonPress and code not in (0, primary):
            self._down.add(code)
            self._send(self._sample("press", event, code, watched))
        elif kind == QEvent.Type.MouseButtonRelease and code in self._down:
            self._down.discard(code)
            self._send(self._sample("release", event, code, watched))
        elif kind == QEvent.Type.MouseMove and self._down:
            self._send(self._sample("move", event, 0, watched))

    def _sample(self, op: str, event: QMouseEvent, code: int, watched: QObject) -> dict[str, Any]:
        pos = event.position()
        # Qt's modifier flags do not fit the engine's u8. A modified click stays with the widget.
        modified = qt_int(event.modifiers()) != 0
        body: dict[str, Any] = {
            "op": op,
            "x": int(pos.x()),
            "y": int(pos.y()),
            "tNs": time.monotonic_ns(),
            "from": "window",
            "modifiers": 0,
            "widget": _widget_name(watched),
            "conflict": _conflicts(watched, event, code) or modified,
        }
        if op not in ("move", "cancel"):
            body["code"] = code
        return body

    def _send(self, body: dict[str, Any]) -> None:
        self._engine.execute("surface.gesture.sample", body, on_reply=self._sampled, on_error=lambda _m: None)

    def _inspected(self, state: Any, _binary: Any) -> None:
        if isinstance(state, dict):
            self._armed = bool(state.get("armed"))
            frame = state.get("hudFrame")
            self.show_frame(frame if isinstance(frame, dict) else None)

    def _sampled(self, state: Any, _binary: Any) -> None:
        if isinstance(state, dict):
            self._armed = bool(state.get("armed"))
            frame = state.get("hudFrame")
            self.show_frame(frame if isinstance(frame, dict) else None)
