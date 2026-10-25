"""Effect Controls: the selected clip's effects and their parameters, with keyframes.

All of it is the engine's: ``effects.inspect`` describes the clip (definitions, the value at the
playhead, keyframes in media and timeline time) and every change is an engine command
(``effects.setParam``, ``effects.toggleAnimation``, ``effects.addKeyframe``,
``effects.deleteKeyframe``, ``effects.setInterpolation``, ``effects.toggleEnabled``,
``effects.reset``, ``effects.resetParam``, ``effects.remove``). Editing an animated parameter
writes a keyframe at the playhead, as in egui. A drag of one value is one undo step: changes are
sent with ``merge`` and a new step is begun when the user pauses.
"""

from __future__ import annotations

import math
from typing import Any

from PySide6.QtCore import QElapsedTimer, QPointF, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QMouseEvent, QPainter, QPaintEvent, QPolygonF
from PySide6.QtWidgets import (
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDoubleSpinBox,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMenu,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, tr

# a pause longer than this starts a new undo step for the next change of the same value
GESTURE_GAP_MS = 800
SEND_DELAY_MS = 120
INTERPOLATIONS = (
    ("linear", N_("Linear")),
    ("bezier", N_("Bezier")),
    ("autoBezier", N_("Auto Bezier")),
    ("continuousBezier", N_("Continuous Bezier")),
    ("hold", N_("Hold")),
    ("easeIn", N_("Ease In")),
    ("easeOut", N_("Ease Out")),
)
UNEDITABLE = {"curve", "wheel", "path"}


def color_to_qcolor(value: Any) -> QColor:
    if isinstance(value, list) and len(value) >= 3:
        try:
            r, g, b = (max(0.0, min(1.0, float(v))) for v in value[:3])
            a = max(0.0, min(1.0, float(value[3]))) if len(value) > 3 else 1.0
            return QColor.fromRgbF(r, g, b, a)
        except (TypeError, ValueError):
            pass
    return QColor("#000000")


def qcolor_to_value(color: QColor) -> list[float]:
    return [round(color.redF(), 6), round(color.greenF(), 6), round(color.blueF(), 6), round(color.alphaF(), 6)]


def finite(value: Any, fallback: float = 0.0) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return fallback
    return v if math.isfinite(v) else fallback


class KeyframeStrip(QWidget):
    """The clip's span with the parameter's keyframes and the playhead. Click: go to a keyframe
    (or that time); right-click a keyframe: its interpolation, or delete it."""

    seek = Signal(object)  # ticks; object, not a 32-bit int
    interpolation = Signal(object, str)  # media time (ticks), interpolation id
    delete = Signal(object)  # media time (ticks)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setMinimumHeight(16)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.start = 0
        self.end = 1
        self.playhead = 0
        self.keyframes: list[dict[str, Any]] = []

    def set_data(self, start: int, end: int, playhead: int, keyframes: list[dict[str, Any]]) -> None:
        self.start, self.end, self.playhead = start, max(end, start + 1), playhead
        self.keyframes = keyframes
        self.update()

    def _x(self, t: int) -> float:
        return 6 + (t - self.start) / (self.end - self.start) * max(1, self.width() - 12)

    def _t(self, x: float) -> int:
        return int(self.start + (x - 6) / max(1, self.width() - 12) * (self.end - self.start))

    def _hit(self, x: float) -> dict[str, Any] | None:
        best = None
        for k in self.keyframes:
            t = k.get("time")
            if isinstance(t, int) and abs(self._x(t) - x) <= 6 and (best is None or abs(self._x(t) - x) < abs(self._x(best["time"]) - x)):
                best = k
        return best

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        mid = self.height() / 2
        p.setPen(self.palette().mid().color())
        p.drawLine(QPointF(6, mid), QPointF(self.width() - 6, mid))
        p.setPen(Qt.PenStyle.NoPen)
        for k in self.keyframes:
            t = k.get("time")
            if not isinstance(t, int):
                continue
            x = self._x(t)
            hold = k.get("interpolation") == "hold"
            p.setBrush(QColor("#d9b44a") if not hold else QColor("#c07a3a"))
            r = 5
            shape = QPolygonF([QPointF(x, mid - r), QPointF(x + r, mid), QPointF(x, mid + r), QPointF(x - r, mid)])
            if hold:
                p.drawRect(QRectF(x - r + 1, mid - r + 1, 2 * r - 2, 2 * r - 2))
            else:
                p.drawPolygon(shape)
        if self.start <= self.playhead <= self.end:
            p.setPen(QColor("#e04040"))
            x = self._x(self.playhead)
            p.drawLine(QPointF(x, 0), QPointF(x, self.height()))

    def mousePressEvent(self, e: QMouseEvent) -> None:
        x = e.position().x()
        k = self._hit(x)
        if e.button() == Qt.MouseButton.RightButton and k is not None:
            menu = QMenu(self)
            media = int(k.get("mediaTime") or 0)
            for key, label in INTERPOLATIONS:
                a = menu.addAction(tr(label))
                a.setCheckable(True)
                a.setChecked(k.get("interpolation") == key)
                a.triggered.connect(lambda _c=False, kk=key, m=media: self.interpolation.emit(m, kk))
            menu.addSeparator()
            d = menu.addAction(tr("Delete Keyframe"))
            d.triggered.connect(lambda _c=False, m=media: self.delete.emit(m))
            menu.exec(e.globalPosition().toPoint())
            return
        if e.button() == Qt.MouseButton.LeftButton:
            target = int(k["time"]) if k is not None else self._t(x)
            self.seek.emit(max(self.start, min(self.end - 1, target)))


# commands the engine runs at its playhead (animated values are evaluated or keyed there)
AT_PLAYHEAD = frozenset({"effects.toggleAnimation", "effects.resetParam", "effects.reset"})

class ParamRow:
    """Widgets of one parameter: stopwatch, label, editor, keyframe navigation, strip, reset."""

    def __init__(self, panel: EffectControlsPanel, grid: QGridLayout, row: int, effect: int, info: dict[str, Any]) -> None:
        self.panel = panel
        self.effect = effect
        self.info = info
        self.id = str(info.get("id"))
        self.kind = str(info.get("type") or "")
        parent = grid.parentWidget()
        name = f"effectControls.{effect}.{self.id}"
        self.stopwatch = QToolButton(parent)
        self.stopwatch.setObjectName(f"{name}.animate")
        self.stopwatch.setText("⏱")
        self.stopwatch.setCheckable(True)
        self.stopwatch.setToolTip(tr("Toggle Animation"))
        self.stopwatch.setEnabled(bool(info.get("animatable")) and self.kind not in UNEDITABLE)
        self.stopwatch.clicked.connect(self._toggle_animation)
        label = QLabel(tr(str(info.get("label") or self.id)), parent)
        label.setToolTip(label.text())
        label.setMinimumWidth(40)
        label.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        unit = str(info.get("unit") or "")
        self.editor: QWidget
        self.spins: list[QDoubleSpinBox] = []
        if self.kind in ("float", "angle"):
            spin = self._spin(parent, info, unit if self.kind == "float" else "°")
            self.spins = [spin]
            self.editor = spin
        elif self.kind == "point":
            box = QWidget(parent)
            h = QHBoxLayout(box)
            h.setContentsMargins(0, 0, 0, 0)
            self.spins = [self._spin(box, {}, ""), self._spin(box, {}, "")]
            for s in self.spins:
                h.addWidget(s)
            self.editor = box
        elif self.kind == "bool":
            check = QCheckBox(parent)
            check.toggled.connect(lambda on: self.panel.set_param(self, on, immediate=True))
            self.editor = check
        elif self.kind == "choice":
            combo = QComboBox(parent)
            for i, opt in enumerate(info.get("options") or []):
                combo.addItem(tr(str(opt)), i)
            combo.currentIndexChanged.connect(lambda i: self.panel.set_param(self, int(i), immediate=True))
            self.editor = combo
        elif self.kind == "color":
            button = QPushButton(parent)
            button.clicked.connect(self._pick_color)
            self.editor = button
        elif self.kind == "text":
            line = QLineEdit(parent)
            line.editingFinished.connect(lambda: self.panel.set_param(self, line.text(), immediate=True))
            self.editor = line
        else:
            note = QLabel(tr("Edited in the Lumetri and mask tools (not yet in the Qt GUI)"), parent)
            note.setEnabled(False)
            self.editor = note
        self.editor.setObjectName(f"{name}.value")
        if not self.spins:
            self.editor.setMinimumWidth(56)
            self.editor.setSizePolicy(QSizePolicy.Policy.Ignored, self.editor.sizePolicy().verticalPolicy())
        nav = QWidget(parent)
        nh = QHBoxLayout(nav)
        nh.setContentsMargins(0, 0, 0, 0)
        nh.setSpacing(0)
        self.prev_key = self._nav_button(nav, "◀", N_("Go to Previous Keyframe"), f"{name}.prevKeyframe", self._prev)
        self.add_key = self._nav_button(nav, "◆", N_("Add/Remove Keyframe"), f"{name}.keyframe", self._add_remove)
        self.next_key = self._nav_button(nav, "▶", N_("Go to Next Keyframe"), f"{name}.nextKeyframe", self._next)
        for b in (self.prev_key, self.add_key, self.next_key):
            nh.addWidget(b)
        self.strip = KeyframeStrip(parent)
        self.strip.setObjectName(f"{name}.keyframes")
        self.strip.seek.connect(self.panel.seek)
        self.strip.interpolation.connect(lambda m, i: self.panel.send(
            "effects.setInterpolation",
            {"clip": self.panel.clip_id, "effect": self.effect, "param": self.id, "mediaTime": m, "interpolation": i},
        ))
        self.strip.delete.connect(lambda m: self.panel.send(
            "effects.deleteKeyframe", {"clip": self.panel.clip_id, "effect": self.effect, "param": self.id, "mediaTime": m}
        ))
        reset = QToolButton(parent)
        reset.setObjectName(f"{name}.reset")
        reset.setText("↺")
        reset.setToolTip(tr("Reset Parameter"))
        reset.clicked.connect(lambda: self.panel.send(
            "effects.resetParam", {"clip": self.panel.clip_id, "effect": self.effect, "param": self.id}
        ))
        # two grid rows per parameter: the controls, then (when animated) the keyframe strip
        # across the panel, so it stays usable in a narrow dock
        grid.addWidget(self.stopwatch, 2 * row, 0)
        grid.addWidget(label, 2 * row, 1)
        grid.addWidget(self.editor, 2 * row, 2)
        grid.addWidget(nav, 2 * row, 3)
        grid.addWidget(reset, 2 * row, 4)
        grid.addWidget(self.strip, 2 * row + 1, 1, 1, 4)
        self.animated = False
        self.keyframes: list[dict[str, Any]] = []

    def _spin(self, parent: QWidget, info: dict[str, Any], suffix: str) -> QDoubleSpinBox:
        spin = QDoubleSpinBox(parent)
        lo = finite(info.get("min"), -1_000_000.0)
        hi = finite(info.get("max"), 1_000_000.0)
        spin.setRange(lo, hi)
        spin.setDecimals(2 if hi - lo > 10 else 3)
        spin.setKeyboardTracking(False)
        # narrow docks: let the value fields shrink instead of pushing the buttons out of view
        spin.setMinimumWidth(56)
        spin.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Fixed)
        spin.setAccelerated(True)
        if suffix:
            spin.setSuffix(f" {suffix}")
        spin.valueChanged.connect(lambda _v: self.panel.set_param(self, self._spin_value()))
        return spin

    def _spin_value(self) -> Any:
        if self.kind == "point":
            return [self.spins[0].value(), self.spins[1].value()]
        return self.spins[0].value()

    def _nav_button(self, parent: QWidget, text: str, tip: str, name: str, slot: Any) -> QToolButton:
        b = QToolButton(parent)
        b.setObjectName(name)
        b.setText(text)
        b.setToolTip(tr(tip))
        b.setAutoRaise(True)
        b.clicked.connect(slot)
        return b

    # ----- engine values -> widgets -----
    def show(self, info: dict[str, Any], clip: dict[str, Any]) -> None:
        self.info = info
        self.animated = bool(info.get("animated"))
        self.keyframes = [k for k in info.get("keyframes") or [] if isinstance(k, dict)]
        value = info.get("valueAtPlayhead") if self.animated else info.get("value")
        self.stopwatch.blockSignals(True)
        self.stopwatch.setChecked(self.animated)
        self.stopwatch.blockSignals(False)
        editing = self.editor.hasFocus() or any(s.hasFocus() for s in self.spins)
        if not editing:
            self._set_editor(value)
        for b in (self.prev_key, self.add_key, self.next_key):
            b.setVisible(self.animated)
        self.strip.setVisible(self.animated)
        here = self._keyframe_here(clip)
        self.add_key.setText("◆" if here is not None else "◇")
        self.strip.set_data(int(clip.get("start") or 0), int(clip.get("end") or 1), int(clip.get("playhead") or 0), self.keyframes)

    def _set_editor(self, value: Any) -> None:
        w = self.editor
        w.blockSignals(True)
        for s in self.spins:
            s.blockSignals(True)
        try:
            if self.kind in ("float", "angle"):
                self.spins[0].setValue(finite(value))
            elif self.kind == "point":
                xy = value if isinstance(value, list) and len(value) == 2 else [0.0, 0.0]
                for s, v in zip(self.spins, xy, strict=True):
                    s.setValue(finite(v))
                    s.setToolTip(tr("automatic") if v is None else "")
            elif isinstance(w, QCheckBox):
                w.setChecked(bool(value))
            elif isinstance(w, QComboBox):
                w.setCurrentIndex(max(0, int(finite(value))))
            elif isinstance(w, QPushButton):
                c = color_to_qcolor(value)
                w.setText(c.name().upper())
                w.setStyleSheet(f"background: {c.name()}; color: {'#000' if c.lightness() > 128 else '#fff'};")
                w.setProperty("cfa4_color", value)
            elif isinstance(w, QLineEdit):
                w.setText("" if value is None else str(value))
        finally:
            w.blockSignals(False)
            for s in self.spins:
                s.blockSignals(False)

    def _keyframe_here(self, clip: dict[str, Any]) -> dict[str, Any] | None:
        m = clip.get("mediaTimeAtPlayhead")
        return next((k for k in self.keyframes if k.get("mediaTime") == m), None)

    # ----- widgets -> engine -----
    def _toggle_animation(self) -> None:
        self.panel.send(
            "effects.toggleAnimation", {"clip": self.panel.clip_id, "effect": self.effect, "param": self.id}
        )

    def _pick_color(self) -> None:
        current = color_to_qcolor(self.editor.property("cfa4_color"))
        color = QColorDialog.getColor(current, self.editor, tr(str(self.info.get("label") or self.id)),
                                      QColorDialog.ColorDialogOption.ShowAlphaChannel)
        if color.isValid():
            self.panel.set_param(self, qcolor_to_value(color), immediate=True)

    def _add_remove(self) -> None:
        params: dict[str, Any] = {"clip": self.panel.clip_id, "effect": self.effect, "param": self.id}
        monitor = getattr(self.panel.window(), "monitor", None)
        if monitor is not None and getattr(monitor, "playing", False):
            # the snapshot and the engine playhead are still where playback started: stop on the
            # shown frame and toggle the keyframe there
            monitor.stop()
            at = self.panel._edit_time(monitor.playhead)
            half = max(1, int(monitor.frame()) // 2)
            here = None
            if at is not None:
                here = next((k for k in self.keyframes if isinstance(k.get("time"), int) and abs(k["time"] - at) < half), None)
            add = {**params, "time": at} if at is not None else params
        else:
            here = self._keyframe_here(self.panel.clip or {})
            add = params
        if here is not None:
            self.panel.send("effects.deleteKeyframe", {**params, "mediaTime": here["mediaTime"]})
        else:
            self.panel.send("effects.addKeyframe", add)

    def _times(self) -> list[int]:
        return sorted(int(k["time"]) for k in self.keyframes if isinstance(k.get("time"), int))

    def _prev(self) -> None:
        ph = int((self.panel.clip or {}).get("playhead") or 0)
        earlier = [t for t in self._times() if t < ph]
        if earlier:
            self.panel.seek.emit(earlier[-1])

    def _next(self) -> None:
        ph = int((self.panel.clip or {}).get("playhead") or 0)
        later = [t for t in self._times() if t > ph]
        if later:
            self.panel.seek.emit(later[0])


class EffectControlsPanel(QWidget):
    command = Signal(str, dict)
    seek = Signal(object)  # ticks; object, not a 32-bit int

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.EffectControls")
        self.engine: Any = None
        self.clip: dict[str, Any] | None = None
        self.clip_id: int | None = None
        self._shape: tuple[Any, ...] | None = None
        self.rows: dict[tuple[int, str], ParamRow] = {}
        self.effect_boxes: list[QWidget] = []
        # (clip, effect index, param id, timeline time or None) -> value: an edit stays bound to the
        # clip and, when animated, the time it was made at, whatever the panel shows by the flush
        self._pending: dict[tuple[int, int, str, int | None], Any] = {}
        self._send_timer = QTimer(self, singleShot=True, interval=SEND_DELAY_MS)
        self._send_timer.timeout.connect(self.flush)
        self._last_send = QElapsedTimer()
        self._busy = False
        self._again = False

        self.title = QLabel(tr("Select a clip in the Timeline."), self)
        self.title.setObjectName("effectControls.title")
        self.body = QWidget(self)
        self.body_layout = QVBoxLayout(self.body)
        self.body_layout.setContentsMargins(0, 0, 0, 0)
        self.body_layout.addStretch(1)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.body)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(4, 4, 4, 4)
        layout.addWidget(self.title)
        layout.addWidget(scroll, 1)

    # ----- loading -----
    def attach(self, engine: Any) -> None:
        self.engine = engine

    def refresh(self, selection: list[Any] | None) -> None:
        """Ask the engine about the first selected clip (or clear the panel)."""
        clips = [c for c in selection or [] if isinstance(c, int)]
        if not clips or self.engine is None or not self.engine.running:
            self._show(None)
            return
        if self._busy:
            self._again = True
            return
        self._busy = True
        self.engine.execute(
            "effects.inspect", {"clip": clips[0]}, on_reply=self._on_reply, on_error=lambda _m: self._on_reply(None, None)
        )

    def _on_reply(self, result: Any, _b: bytes | None) -> None:
        self._busy = False
        self._show(result if isinstance(result, dict) else None)
        if self._again:
            self._again = False
            parent = self.window()
            selection = getattr(parent, "sequence", None) or {}
            self.refresh(selection.get("selection") if isinstance(selection, dict) else None)

    def _show(self, clip: dict[str, Any] | None) -> None:
        self.flush()  # edits made on the clip shown so far go to that clip, at their own time
        self.clip = clip
        self.clip_id = int(clip["clip"]) if clip and isinstance(clip.get("clip"), int) else None
        if clip is None:
            self.title.setText(tr("Select a clip in the Timeline."))
            self._rebuild(None)
            return
        self.title.setText(str(clip.get("name") or ""))
        effects = [e for e in clip.get("effects") or [] if isinstance(e, dict)]
        shape = (self.clip_id, tuple((e.get("effect"), tuple(p.get("id") for p in e.get("params") or [])) for e in effects))
        if shape != self._shape:
            self._rebuild(clip)
            self._shape = shape
        for e in effects:
            box = self.effect_boxes[int(e.get("index") or 0)] if int(e.get("index") or 0) < len(self.effect_boxes) else None
            if box is not None:
                check = box.findChild(QCheckBox, f"effectControls.{e.get('index')}.enabled")
                if check is not None:
                    check.blockSignals(True)
                    check.setChecked(bool(e.get("enabled", True)))
                    check.blockSignals(False)
            for info in e.get("params") or []:
                row = self.rows.get((int(e.get("index") or 0), str(info.get("id"))))
                if row is not None:
                    row.show(info, clip)

    def _rebuild(self, clip: dict[str, Any] | None) -> None:
        for box in self.effect_boxes:
            box.setParent(None)
            box.deleteLater()
        self.effect_boxes = []
        self.rows = {}
        self._shape = None
        if clip is None:
            return
        for e in clip.get("effects") or []:
            if not isinstance(e, dict):
                continue
            index = int(e.get("index") or 0)
            box = QFrame(self.body)
            box.setObjectName(f"effectControls.{index}")
            box.setFrameShape(QFrame.Shape.StyledPanel)
            v = QVBoxLayout(box)
            v.setContentsMargins(4, 2, 4, 4)
            head = QHBoxLayout()
            enabled = QCheckBox(tr(str(e.get("name") or e.get("effect"))), box)
            enabled.setObjectName(f"effectControls.{index}.enabled")
            enabled.setChecked(bool(e.get("enabled", True)))
            enabled.setToolTip(tr("Toggle the effect on or off"))
            enabled.toggled.connect(lambda _on, i=index: self.send("effects.toggleEnabled", {"clip": self.clip_id, "index": i}))
            font = enabled.font()
            font.setBold(True)
            enabled.setFont(font)
            head.addWidget(enabled, 1)
            for text, tip, cmd, oname in (
                ("↺", N_("Reset Effect"), "effects.reset", "reset"),
                ("✕", N_("Remove Effect"), "effects.remove", "remove"),
            ):
                b = QToolButton(box)
                b.setObjectName(f"effectControls.{index}.{oname}")
                b.setText(text)
                b.setToolTip(tr(tip))
                b.clicked.connect(lambda _c=False, c=cmd, i=index: self.send(c, {"clip": self.clip_id, "index": i}))
                head.addWidget(b)
            v.addLayout(head)
            grid_host = QWidget(box)
            grid = QGridLayout(grid_host)
            grid.setContentsMargins(0, 0, 0, 0)
            grid.setColumnStretch(1, 2)
            grid.setColumnStretch(2, 3)
            for row, info in enumerate(e.get("params") or []):
                if isinstance(info, dict):
                    self.rows[(index, str(info.get("id")))] = ParamRow(self, grid, row, index, info)
            v.addWidget(grid_host)
            self.body_layout.insertWidget(self.body_layout.count() - 1, box)
            self.effect_boxes.append(box)

    # ----- edits -----
    def set_param(self, row: ParamRow, value: Any, immediate: bool = False) -> None:
        """Send a new value; spin-box drags are coalesced and become one undo step."""
        if self.clip_id is None:
            return
        self._pending[(self.clip_id, row.effect, row.id, self._edit_time() if row.animated else None)] = value
        if immediate:
            self.flush()
        else:
            self._send_timer.start()

    def _edit_time(self, playhead: int | None = None) -> int | None:
        """Where an animated edit is keyed: ``playhead`` when given, the Program monitor's frame while it plays (the engine
        playhead and this snapshot stay where playback started), else the shown snapshot's
        playhead; pinned to the clip as the engine pins its own."""
        clip = self.clip or {}
        ph, start, end = clip.get("playhead"), clip.get("start"), clip.get("end")
        monitor = getattr(self.window(), "monitor", None)
        if isinstance(playhead, int):
            ph = playhead
        elif monitor is not None and getattr(monitor, "playing", False) and isinstance(getattr(monitor, "playhead", None), int):
            ph = monitor.playhead
        if not all(isinstance(v, int) for v in (ph, start, end)):
            return None
        return max(start, min(ph, max(start, end - 1)))

    def send(self, command: str, params: dict[str, Any]) -> None:
        """Any other edit from the panel: the coalesced values go first, so they don't land on a
        reset, removed or reordered effect."""
        self.flush()
        if command in AT_PLAYHEAD:
            # the engine acts at its playhead, which stays where playback started: stop on the
            # shown frame first (the stop seeks the engine there)
            monitor = getattr(self.window(), "monitor", None)
            if monitor is not None and getattr(monitor, "playing", False):
                monitor.stop()
        self.command.emit(command, params)

    def flush(self) -> None:
        """Send the coalesced values now (also before a save or an unsaved-changes check)."""
        self._send_timer.stop()
        if not self._pending:
            return
        begin = not self._last_send.isValid() or self._last_send.elapsed() > GESTURE_GAP_MS
        for (clip, effect, param, at), value in self._pending.items():
            params = {"clip": clip, "effect": effect, "param": param, "value": value, "merge": True, "begin": begin}
            if at is not None:
                params["time"] = at
            self.command.emit("effects.setParam", params)
        self._pending.clear()
        self._last_send.restart()
