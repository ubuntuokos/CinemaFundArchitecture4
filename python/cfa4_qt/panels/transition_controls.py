"""Effect Controls for a selected transition, as egui's (``transition_controls.rs``): Duration,
Alignment, Reverse and the transition's own settings on the left, its own timeline on the right of
the divider (a time ruler over the cut, the outgoing clip A above, the transition, the incoming
clip B below, the cut and the playhead).

Duration reads and takes timecode, a frame count or a calculation in frames, and scrubs like a
hot number. Dragging the transition's middle in its timeline slides it over the cut, dragging one
of its ends changes its duration from that end (the Timeline's rules, applied by the engine);
clicking or dragging the ruler moves the playhead. Every change is one ``sequence.setTransition``
and a drag or a scrub is one undo step. The engine describes it with ``transitions.inspect``.
"""

from __future__ import annotations

import math
from typing import Any

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QFont, QMouseEvent, QPainter, QPaintEvent, QPen, QPolygonF
from PySide6.QtWidgets import (
    QCheckBox,
    QColorDialog,
    QComboBox,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..hot_number import HotSpin
from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND, format_timecode
from .effect_controls import (
    Divider,
    as_int,
    color_to_qcolor,
    finite,
    qcolor_to_value,
    ruler_steps,
    snap_frame,
)
from .timeline import duration_frames

RULER_H = 24
BAR_H = 18
EDGE_PX = 7.0  # how far into the transition from either end a press grabs that end
ALIGNMENTS = (("center", N_("Center at Cut")), ("start", N_("Start at Cut")), ("end", N_("End at Cut")))


class DurationSpin(HotSpin):
    """A duration in frames shown as timecode; typed: frames, timecode or a calculation."""

    frame = 1  # ticks a frame; class-wide first, as the base class formats while it is set up
    drop = False

    def __init__(self, parent: QWidget) -> None:
        super().__init__(parent, 1, 1e7, 0, 0.25)

    def textFromValue(self, v: float) -> str:
        return format_timecode(round(v) * self.frame, self.frame, self.drop)

    def _typed(self, text: str) -> float | None:
        v = duration_frames(text, round(self.value()), self.frame, self.drop)
        return None if v is None else float(max(self.minimum(), min(self.maximum(), v)))


class TransitionLane(QWidget):
    """The transition's own timeline: ruler, clip A, the transition, clip B, the cut, the playhead."""

    def __init__(self, view: TransitionControls) -> None:
        super().__init__(view)
        self.view = view
        self.setObjectName("effectControls.transition.timeline")
        self.setMouseTracking(True)
        self.setMinimumWidth(30)
        self._drag: dict[str, Any] | None = None
        self._seeking = False

    # ----- geometry -----
    def span_of_view(self) -> tuple[int, int]:
        """The cut with one and a half times the transition's duration either side; held still
        while a drag changes the transition under it."""
        if self._drag is not None:
            return self._drag["view"]
        d = self.view.data or {}
        frame = max(1, as_int(d.get("frame"), 1))
        half = max(as_int(d.get("duration")), frame, 1) * 3 // 2
        cut = as_int(d.get("cut"))
        return cut - half, cut + half

    def lane(self) -> QRectF:
        return QRectF(4, 4, max(1.0, self.width() - 10.0), max(1.0, self.height() - 10.0))

    def x_of(self, t: int) -> float:
        lane, (v0, v1) = self.lane(), self.span_of_view()
        f = max(-100.0, min(100.0, (t - v0) / max(1, v1 - v0)))
        return lane.left() + f * lane.width()

    def tick_at(self, x: float) -> int:
        lane, (v0, v1) = self.lane(), self.span_of_view()
        return v0 + round((x - lane.left()) / max(1.0, lane.width()) * (v1 - v0))

    def ruler_rect(self) -> QRectF:
        lane = self.lane()
        return QRectF(lane.left(), lane.top(), lane.width(), RULER_H)

    def row_rect(self, i: int) -> QRectF:
        lane = self.lane()
        return QRectF(lane.left(), lane.top() + RULER_H + 4 + i * (BAR_H + 2), lane.width(), BAR_H)

    def clip_rect(self, key: str) -> QRectF | None:
        """Clip ``a`` (outgoing) or ``b`` (incoming), inside the lane."""
        c = (self.view.data or {}).get(key)
        if not isinstance(c, dict):
            return None
        lane, r = self.lane(), self.row_rect(0 if key == "a" else 2)
        x0, x1 = max(lane.left(), self.x_of(as_int(c.get("start")))), min(lane.right(), self.x_of(as_int(c.get("end"))))
        return QRectF(x0, r.top(), x1 - x0, r.height()) if x1 > x0 else None

    def span_rect(self) -> QRectF:
        d = self.view.data or {}
        start = as_int(d.get("start"))
        x0, x1 = self.x_of(start), self.x_of(start + as_int(d.get("duration")))
        r = self.row_rect(1)
        return QRectF(x0, r.top(), max(1.0, x1 - x0), r.height())

    def grab(self, x: float) -> str:
        """The end a press at ``x`` grabs ("in", "out") or "" for the middle."""
        span = self.span_rect()
        zone = min(EDGE_PX, span.width() / 3)
        return "in" if x < span.left() + zone else "out" if x > span.right() - zone else ""

    # ----- painting -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        d = self.view.data
        if not d:
            return
        p = QPainter(self)
        t = theme.current()
        lane = self.lane()
        p.setClipRect(lane)
        p.fillRect(lane, t.c("tl_bg"))
        self._paint_ruler(p, d, t)
        small = QFont(self.font())
        small.setPointSizeF(max(6.0, small.pointSizeF() * 0.8))
        p.setFont(small)
        for key in ("a", "b"):
            r = self.clip_rect(key)
            if r is None:
                continue
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(t.c("clip_bar_bg"))
            p.drawRoundedRect(r, 2, 2)
            p.setPen(t.c("text"))
            p.drawText(r.adjusted(4, 0, -2, 0), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft,
                       str((d.get(key) or {}).get("name") or ""))
        span = self.span_rect()
        p.fillRect(span, QColor(0, 0, 0, 90))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.setPen(QPen(t.c("accent"), 2))
        p.drawRect(span.adjusted(1, 1, -1, -1))
        if span.width() > 30:
            p.setPen(t.c("text"))
            p.drawText(span.adjusted(5, 0, -2, 0), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft,
                       tr(str(d.get("name") or "")))
        cx = self.x_of(as_int(d.get("cut")))
        p.setPen(QPen(t.c("text_dim"), 1))
        p.drawLine(QPointF(cx, self.row_rect(0).top()), QPointF(cx, self.row_rect(2).bottom()))
        v0, v1 = self.span_of_view()
        ph = as_int(d.get("playhead"), v0 - 1)
        if v0 <= ph <= v1:
            x, ruler = self.x_of(ph), self.ruler_rect()
            top, tip = ruler.bottom() - 13, ruler.bottom()
            p.setRenderHint(QPainter.RenderHint.Antialiasing)
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(t.c("playhead"))
            p.drawPolygon(QPolygonF([QPointF(x - 5, top), QPointF(x + 5, top), QPointF(x + 5, tip - 5), QPointF(x, tip),
                                     QPointF(x - 5, tip - 5)]))
            p.setPen(QPen(t.c("playhead"), 1))
            p.drawLine(QPointF(x, tip), QPointF(x, lane.bottom()))

    def _paint_ruler(self, p: QPainter, d: dict[str, Any], t: Any) -> None:
        ruler = self.ruler_rect()
        v0, v1 = self.span_of_view()
        frame = max(1, as_int(d.get("frame"), 1))
        drop = bool(d.get("dropFrame"))
        frame_px = ruler.width() * frame / max(1, v1 - v0)
        steps = [s for s in ruler_steps(round(TICKS_PER_SECOND / frame)) if s > 0]
        label_step = next((s for s in steps if s * frame_px >= 80), None)
        minor = next((s for s in steps if s * frame_px >= 8 and (label_step is None or label_step % s == 0)), None)
        f0, f1 = v0 // frame, v1 // frame
        small = QFont(self.font())
        small.setPointSizeF(max(6.0, small.pointSizeF() * 0.8))
        p.setFont(small)
        base = ruler.bottom() - 1
        for step, h, labelled in ((minor, 3, False), (label_step, 7, True)):
            if step is None:
                continue
            f = f0 // step * step
            for _ in range(4096):  # a step is at least 8 px wide; the cap is for damaged numbers
                if f > f1:
                    break
                x = self.x_of(f * frame)
                p.setPen(t.c("tl_ruler_tick"))
                p.drawLine(QPointF(x, base - h), QPointF(x, base))
                if labelled and f >= 0:
                    p.setPen(t.c("tl_ruler_text"))
                    text = format_timecode(f * frame, frame, drop)
                    p.drawText(QPointF(x - p.fontMetrics().horizontalAdvance(text) / 2, ruler.top() + 11), text)
                f += step

    # ----- mouse -----
    def mousePressEvent(self, e: QMouseEvent) -> None:
        d = self.view.data
        if not d or e.button() != Qt.MouseButton.LeftButton:
            return
        pos = e.position()
        if self.ruler_rect().contains(pos):
            self._seeking = True
            self._seek(pos.x())
        elif self.span_rect().contains(pos):
            self._drag = {"edge": self.grab(pos.x()), "x": pos.x(), "view": self.span_of_view(), "delta": 0,
                          "start": as_int(d.get("start")), "duration": as_int(d.get("duration")), "begun": False}

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        pos = e.position()
        if self._seeking:
            self._seek(pos.x())
            return
        drag = self._drag
        if drag is None:
            if self.span_rect().contains(pos):
                self.setCursor(Qt.CursorShape.SizeHorCursor if self.grab(pos.x()) else Qt.CursorShape.OpenHandCursor)
            else:
                self.unsetCursor()
            return
        frame = max(1, as_int((self.view.data or {}).get("frame"), 1))
        v0, v1 = drag["view"]
        delta = round((pos.x() - drag["x"]) / max(1.0, self.lane().width()) * (v1 - v0) / frame) * frame
        if delta == drag["delta"]:
            return
        drag["delta"] = delta
        self.view.send({"drag": {"edge": drag["edge"] or None, "delta": delta, "start": drag["start"],
                                 "duration": drag["duration"]}, "merge": True, "begin": not drag["begun"]})
        drag["begun"] = True

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self._seeking = False
        self._drag = None
        self.update()

    def _seek(self, x: float) -> None:
        d = self.view.data or {}
        lane = self.lane()
        t = self.tick_at(max(lane.left(), min(x, lane.right())))
        t = max(0, snap_frame(t, max(1, as_int(d.get("frame"), 1))))
        if d.get("playhead") != t:
            d["playhead"] = t  # shown at once; the engine's playhead follows
            self.view.panel.seek.emit(t)
            self.update()


class TransitionControls(QWidget):
    """The transition view of Effect Controls: its settings, the divider, its timeline."""

    def __init__(self, panel: Any) -> None:
        super().__init__(panel)
        self.panel = panel
        self.setObjectName("effectControls.transition")
        self.data: dict[str, Any] | None = None
        self._shape: tuple[Any, ...] | None = None
        self.params: dict[str, Any] = {}  # param id -> its editor(s)
        self.left = QWidget(self)
        left = QVBoxLayout(self.left)
        left.setContentsMargins(10, 6, 10, 6)
        left.setSpacing(4)
        self.title = QLabel(self.left)
        self.title.setObjectName("effectControls.transition.title")
        self.title.setStyleSheet(f"color: {theme.current().text_dim};")
        left.addWidget(self.title)
        self.scroll = QScrollArea(self.left)
        self.scroll.setWidgetResizable(True)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        left.addWidget(self.scroll, 1)
        self.divider = Divider(panel, self)
        self.divider.setObjectName("effectControls.transition.divider")
        self.lane = TransitionLane(self)
        layout = QHBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(self.left)
        layout.addWidget(self.divider)
        layout.addWidget(self.lane, 1)
        self.duration = DurationSpin(self)
        self.duration.setObjectName("effectControls.transition.duration")
        self.duration.edited.connect(lambda v, begin: self.send({"frames": round(v), "merge": True, "begin": begin}))
        self.alignment = QComboBox(self)
        self.alignment.setObjectName("effectControls.transition.alignment")
        for key, label in ALIGNMENTS:
            self.alignment.addItem(tr(label), key)
        self.alignment.addItem(tr("Custom Start"), "custom")
        self.alignment.activated.connect(self._align)
        self.reverse = QCheckBox(self)
        self.reverse.setObjectName("effectControls.transition.reverse")
        self.reverse.clicked.connect(lambda on: self.send({"reverse": bool(on)}))
        self.reset = QPushButton(tr("Reset"), self)
        self.reset.setObjectName("effectControls.transition.reset")
        self.reset.clicked.connect(lambda: self.send({"reset": True}))

    def set_split(self, width: int) -> None:
        self.left.setFixedWidth(width)
        self.lane.update()

    def send(self, params: dict[str, Any]) -> None:
        if self.data is not None:
            self.panel.command.emit("sequence.setTransition", {**params, "transition": self.data.get("transition")})

    def _align(self, index: int) -> None:
        key = self.alignment.itemData(index)
        if key in ("center", "start", "end") and self.data is not None and key != self.data.get("align"):
            self.send({"align": key})

    # ----- showing -----
    def show_transition(self, data: dict[str, Any]) -> None:
        self.data = data
        name = tr(str(data.get("name") or ""))
        self.title.setText(f"{data.get('sequenceName') or ''} · {name}")
        params = [p for p in data.get("params") or [] if isinstance(p, dict)]
        shape = (data.get("transition"), data.get("effect"), tuple((p.get("id"), p.get("type")) for p in params))
        if shape != self._shape:
            self._shape = shape
            self._build(params)
        self.duration.frame = max(1, as_int(data.get("frame"), 1))
        self.duration.drop = bool(data.get("dropFrame"))
        self.duration.setMaximum(max(1, as_int(data.get("maxFrames"), 1)))
        self.duration.set_value(max(1, as_int(data.get("frames"), 1)))
        align = str(data.get("align") or "center")
        # Custom Start is shown, not chosen (it is where a dragged transition ends up)
        custom = self.alignment.model().item(self.alignment.findData("custom"))
        if custom is not None:
            custom.setEnabled(align == "custom")
        self.alignment.setCurrentIndex(max(0, self.alignment.findData(align)))
        self.alignment.setEnabled(not data.get("oneSided"))
        self.reverse.setChecked(bool(data.get("reverse")))
        for p in params:
            self._show_param(p)
        self.lane.update()

    def _build(self, params: list[dict[str, Any]]) -> None:
        old = self.scroll.takeWidget()
        for w in (self.duration, self.alignment, self.reverse, self.reset):
            w.setParent(self)
        if old is not None:
            old.deleteLater()
        self.params = {}
        column = QWidget()
        grid = QGridLayout(column)
        grid.setContentsMargins(0, 0, 0, 0)
        grid.setHorizontalSpacing(14)
        grid.setVerticalSpacing(4)
        row = 0

        def add(label: str, editor: QWidget) -> None:
            nonlocal row
            grid.addWidget(QLabel(label, column), row, 0)
            grid.addWidget(editor, row, 1, Qt.AlignmentFlag.AlignLeft)
            line = QFrame(column)
            line.setFrameShape(QFrame.Shape.HLine)
            line.setStyleSheet(f"color: {theme.current().separator};")
            grid.addWidget(line, row + 1, 0, 1, 2)
            row += 2

        add(tr("Duration"), self.duration)
        add(tr("Alignment"), self.alignment)
        add(tr("Reverse"), self.reverse)
        for p in params:
            editor = self._param_editor(p, column)
            if editor is not None:
                add(tr(str(p.get("label") or p.get("id") or "")), editor)
        grid.addWidget(self.reset, row, 0, 1, 2, Qt.AlignmentFlag.AlignLeft)
        grid.setRowStretch(row + 1, 1)
        grid.setColumnStretch(1, 1)
        self.scroll.setWidget(column)

    def _param_editor(self, p: dict[str, Any], parent: QWidget) -> QWidget | None:
        pid = str(p.get("id") or "")
        eid = f"effectControls.transition.param.{pid}"
        kind = p.get("type")

        def number(lo: float, hi: float, decimals: int, speed: float, suffix: str, name: str) -> HotSpin:
            spin = HotSpin(parent, lo, hi, decimals, speed, suffix)
            spin.setObjectName(name)
            return spin

        editor: QWidget
        if kind == "float":
            lo, hi = finite(p.get("min"), -1e6), finite(p.get("max"), 1e6)
            soft = (finite(p.get("softMax"), hi) - finite(p.get("softMin"), lo)) / 200
            unit = str(p.get("unit") or "")
            editor = number(lo, hi, max(0, min(6, as_int(p.get("decimals"), 1))), max(0.001, soft), unit, eid)
            editor.edited.connect(lambda v, begin, pid=pid: self.send({"params": {pid: v}, "merge": True, "begin": begin}))
        elif kind == "angle":
            editor = number(-1e6, 1e6, 1, 0.5, "°", eid)
            editor.edited.connect(lambda v, begin, pid=pid: self.send({"params": {pid: v}, "merge": True, "begin": begin}))
        elif kind == "bool":
            editor = QCheckBox(parent)
            editor.setObjectName(eid)
            editor.clicked.connect(lambda on, pid=pid: self.send({"params": {pid: bool(on)}}))
        elif kind == "choice":
            editor = QComboBox(parent)
            editor.setObjectName(eid)
            for o in p.get("options") or []:
                editor.addItem(tr(str(o)))
            editor.activated.connect(lambda i, pid=pid: self.send({"params": {pid: i}}))
        elif kind == "color":
            editor = QToolButton(parent)
            editor.setObjectName(eid)
            editor.setFixedSize(36, 18)
            editor.clicked.connect(lambda _c=False, pid=pid: self._pick_color(pid))
        elif kind == "point":
            editor = QWidget(parent)
            h = QHBoxLayout(editor)
            h.setContentsMargins(0, 0, 0, 0)
            xs, ys = number(-1e6, 1e6, 1, 1.0, "", f"{eid}.x"), number(-1e6, 1e6, 1, 1.0, "", f"{eid}.y")
            h.addWidget(QLabel("x", editor))
            h.addWidget(xs)
            h.addWidget(QLabel("y", editor))
            h.addWidget(ys)
            for spin in (xs, ys):
                spin.edited.connect(lambda _v, begin, pid=pid, xs=xs, ys=ys: self.send(
                    {"params": {pid: [xs.value(), ys.value()]}, "merge": True, "begin": begin}))
            editor.setObjectName(eid)
            self.params[pid] = (xs, ys)
            return editor
        else:
            editor = QLabel(parent)
            editor.setObjectName(eid)
            editor.setStyleSheet(f"color: {theme.current().text_faint};")
        self.params[pid] = editor
        return editor

    def _show_param(self, p: dict[str, Any]) -> None:
        pid = str(p.get("id") or "")
        editor = self.params.get(pid)
        value = p.get("value")
        kind = p.get("type")
        if editor is None:
            return
        if kind == "point" and isinstance(editor, tuple):
            size = (self.data or {}).get("frameSize") or [0, 0]
            v = value if isinstance(value, list) and len(value) == 2 else [None, None]
            # an automatic point (null) is the frame's centre until it is moved
            for spin, c, full in zip(editor, v, size, strict=False):
                fallback = finite(full) / 2
                spin.set_value(fallback if c is None or not math.isfinite(finite(c, math.nan)) else finite(c))
        elif isinstance(editor, HotSpin):
            editor.set_value(finite(value))
        elif isinstance(editor, QCheckBox):
            editor.setChecked(value is True)
        elif isinstance(editor, QComboBox):
            editor.setCurrentIndex(as_int(value, 0))
        elif isinstance(editor, QToolButton):
            c = color_to_qcolor(value)
            editor.setProperty("cfa4_color", value)
            editor.setStyleSheet(f"background: {c.name()}; border: 1px solid {theme.current().separator};")
        elif isinstance(editor, QLabel):
            editor.setText(str(value))

    def _pick_color(self, pid: str) -> None:
        editor = self.params.get(pid)
        if not isinstance(editor, QToolButton):
            return
        color = QColorDialog.getColor(color_to_qcolor(editor.property("cfa4_color")), self, tr("Color"),
                                      QColorDialog.ColorDialogOption.ShowAlphaChannel)
        if color.isValid():
            self.send({"params": {pid: qcolor_to_value(color)}})

