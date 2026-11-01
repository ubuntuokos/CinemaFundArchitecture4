"""Lumetri Color, as egui's (crates/ui-egui/src/panels/lumetri.rs): Basic Correction, Creative,
Curves (RGB and the hue curves), Color Wheels & Match, HSL Secondary and Vignette of the selected
video clip's Lumetri effect, each section with its on/off switch.

The engine reads the clip out (``lumetri.inspect``: values at the playhead, slider ranges, each
curve's drawn line, the LUT menus, HDR) and every change is an engine command:
``effects.setParam`` (a drag of one value is one undo step, as in Effect Controls),
``lumetri.setSection``, ``lumetri.setInputLut`` / ``lumetri.setLook`` (Browse… imports the file),
``lumetri.applyMatch``, ``effects.apply`` and ``effects.toggleEnabled``.
"""

from __future__ import annotations

import math
from typing import Any

from PySide6.QtCore import QElapsedTimer, QPointF, QRectF, QSettings, QSize, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QConicalGradient, QFont, QMouseEvent, QPainter, QPaintEvent, QPen, QPolygonF
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QFileDialog,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..hot_number import HotSpin
from ..i18n import N_, fmt, tr
from .effect_controls import GESTURE_GAP_MS, SEND_DELAY_MS

LABEL_W = 112  # egui's label column
VALUE_W = 58
CURVE_BG = QColor(0x16, 0x16, 0x16)
CURVE_GRID = QColor(0x2A, 0x2A, 0x2A)
TRACK = QColor(0x6E, 0x6E, 0x6E)
KNOB = QColor(0xB0, 0xB0, 0xB0)
SWITCH_OFF = QColor(0x55, 0x55, 0x55)
CHANNELS = (
    ("curve_luma", QColor(255, 255, 255)),
    ("curve_red", QColor(230, 70, 70)),
    ("curve_green", QColor(80, 200, 90)),
    ("curve_blue", QColor(80, 130, 250)),
)
HUE_CURVES = (
    ("hue_vs_sat", N_("Hue vs Sat")),
    ("hue_vs_hue", N_("Hue vs Hue")),
    ("hue_vs_luma", N_("Hue vs Luma")),
    ("luma_vs_sat", N_("Luma vs Sat")),
    ("sat_vs_sat", N_("Sat vs Sat")),
)
WHEELS = (
    ("wheel_shadows", "wheel_shadows_l", N_("Shadows")),
    ("wheel_midtones", "wheel_midtones_l", N_("Midtones")),
    ("wheel_highlights", "wheel_highlights_l", N_("Highlights")),
)
SHOW_MASK = (N_("Off"), N_("Color/Gray"), N_("Color/Black"), N_("White/Black"))
LOOKS = (
    N_("None"),
    N_("Teal & Orange"),
    N_("Warm Film"),
    N_("Cool Blue"),
    N_("Bleach Bypass"),
    N_("Faded Matte"),
    N_("Monochrome"),
    N_("Golden Hour"),
    N_("Night"),
)
# (section title, its switch param, the lumetri.setSection name); egui's order
SECTIONS = (
    (N_("Basic Correction"), "basic_on", "basic"),
    (N_("Creative"), "creative_on", "creative"),
    (N_("Curves"), "curves_on", "curves"),
    (N_("Color Wheels & Match"), "wheels_on", "wheels"),
    (N_("HSL Secondary"), "hsl_on", "hsl"),
    (N_("Vignette"), "vignette_on", "vignette"),
)


def finite(value: Any, fallback: float = 0.0) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return fallback
    return v if math.isfinite(v) else fallback


def lerp(a: QColor, b: QColor, f: float) -> QColor:
    return QColor(
        int(a.red() + (b.red() - a.red()) * f),
        int(a.green() + (b.green() - a.green()) * f),
        int(a.blue() + (b.blue() - a.blue()) * f),
    )


def gradient_color(kind: str, f: float) -> QColor:
    if kind == "temp":
        return lerp(QColor(70, 120, 230), QColor(240, 150, 50), f)
    if kind == "tint":
        return lerp(QColor(60, 190, 80), QColor(220, 70, 200), f)
    return QColor.fromHslF(f % 1.0, 0.8, 0.5)


def points_of(value: Any) -> list[list[float]]:
    out = []
    for q in value if isinstance(value, list) else []:
        if isinstance(q, list) and len(q) == 2:
            out.append([min(1.0, max(0.0, finite(q[0]))), min(1.0, max(0.0, finite(q[1])))])
    return out


class Slider(QWidget):
    """egui's spectrum slider: label, a 1 pt track (or a gradient), a hollow ring knob, and the
    value at the right, egui's hot number (drag it to scrub, click to type a value or a
    calculation). Click or drag the track sets the value; double-click restores the default."""

    changed = Signal(str, float, bool)  # param id, value, immediate
    gesture = Signal()  # a new undo step starts: a press on the track, a scrub, a typed value

    def __init__(self, pid: str, label: str, gradient: str | None, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.pid = pid
        self.label = label
        self.gradient = gradient
        self.setObjectName(f"lumetri.param.{pid}")
        self.setFixedHeight(26)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.info: dict[str, Any] = {}
        self.value = 0.0
        self.dragging = False
        self.spin = HotSpin(self)
        self.spin.setObjectName(f"lumetri.param.{pid}.value")
        self.spin.setAlignment(Qt.AlignmentFlag.AlignRight)
        self.spin.setFixedWidth(VALUE_W - 4)
        # a typed value applies at once; a scrub is sent like a drag of the track (one undo step)
        self.spin.edited.connect(self._edited)
        row = QHBoxLayout(self)
        row.setContentsMargins(0, 3, 0, 3)
        row.addStretch(1)
        row.addWidget(self.spin)

    def _edited(self, value: float, begin: bool) -> None:
        self.value = value
        self.update()
        if begin:
            self.gesture.emit()
        self.changed.emit(self.pid, value, begin and not self.spin._scrubbing)

    def set_info(self, info: dict[str, Any]) -> None:
        self.info = info
        self.spin.restyle()
        decimals = int(finite(info.get("decimals"), 1))
        self.spin.blockSignals(True)
        self.spin.setDecimals(max(0, min(6, decimals)))
        self.spin.setRange(finite(info.get("min"), -1e6), finite(info.get("max"), 1e6))
        span = self.soft()[1] - self.soft()[0]
        self.spin.setSingleStep(max(10 ** -decimals, round(span / 100.0, decimals)))
        self.spin.speed = span / 300.0  # egui's
        self.spin.blockSignals(False)
        if not self.dragging and not self.spin.busy:
            self.value = finite(info.get("value"))
            self.spin.set_value(self.value)
        self.update()

    def soft(self) -> tuple[float, float]:
        lo = finite(self.info.get("softMin"), 0.0)
        hi = finite(self.info.get("softMax"), 1.0)
        return (lo, hi) if hi > lo else (lo, lo + 1.0)

    def track(self) -> QRectF:
        mid = self.height() / 2
        return QRectF(LABEL_W, mid - 1, max(10.0, self.width() - LABEL_W - VALUE_W), 2)

    def knob_x(self) -> float:
        lo, hi = self.soft()
        t = self.track()
        return t.left() + max(0.0, min(1.0, (self.value - lo) / (hi - lo))) * t.width()

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(t.c("text_dim"))
        p.drawText(QRectF(4, 0, LABEL_W - 8, self.height()), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft, self.label)
        tr_ = self.track()
        if self.gradient:
            n = 24
            for i in range(n):
                f = i / n
                x0 = tr_.left() + tr_.width() * f
                p.fillRect(QRectF(x0, tr_.top() - 1, tr_.width() / n + 0.5, tr_.height() + 2), gradient_color(self.gradient, f))
        else:
            p.fillRect(tr_, TRACK)
        c = QPointF(self.knob_x(), tr_.center().y())
        p.setBrush(t.c("panel_bg"))
        p.setPen(QPen(t.c("hot_text") if self.dragging else KNOB, 1.5))
        p.drawEllipse(c, 5, 5)

    def _hit(self, e: QMouseEvent) -> bool:
        t = self.track()
        return t.left() - 6 <= e.position().x() <= t.right() + 6

    def _set_from(self, x: float) -> None:
        lo, hi = self.soft()
        t = self.track()
        f = max(0.0, min(1.0, (x - t.left()) / max(1.0, t.width())))
        decimals = self.spin.decimals()
        self.value = round(lo + f * (hi - lo), decimals)
        self.spin.blockSignals(True)
        self.spin.setValue(self.value)
        self.spin.blockSignals(False)
        self.update()
        self.changed.emit(self.pid, self.value, False)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton and self._hit(e):
            self.dragging = True
            self.gesture.emit()
            self._set_from(e.position().x())

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging:
            self._set_from(e.position().x())

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self.dragging = False
        self.update()

    def mouseDoubleClickEvent(self, e: QMouseEvent) -> None:
        if self._hit(e) and "default" in self.info:
            self.value = finite(self.info.get("default"))
            self.spin.blockSignals(True)
            self.spin.setValue(self.value)
            self.spin.blockSignals(False)
            self.update()
            self.gesture.emit()
            self.changed.emit(self.pid, self.value, True)


class ChannelButton(QToolButton):
    """egui's RGB curve channel dot; the shown channel is ringed."""

    def __init__(self, color: QColor, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.color = color
        self.setCheckable(True)
        self.setFixedSize(18, 18)

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        c = QPointF(self.width() / 2, self.height() / 2)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(self.color)
        p.drawEllipse(c, 6, 6)
        if self.isChecked():
            p.setPen(QPen(theme.current().c("hot_text"), 1.5))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawEllipse(c, 8.0, 8.0)


class CurveEditor(QWidget):
    """egui's curve editor: drag a point, press on the curve area to add one, double-click a point
    to remove it (not an end point of an RGB curve) or the empty area to reset an RGB curve. The
    drawn line is the engine's (``line`` of ``lumetri.inspect``)."""

    changed = Signal(str, list, bool)

    def __init__(self, pid: str, hue: bool, color: QColor, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.pid = pid
        self.hue = hue
        self.color = color
        self.setObjectName(f"lumetri.curve.{pid}")
        self.points: list[list[float]] = []
        self.line: list[float] = []
        self.dragging: int | None = None
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.setMinimumWidth(80)

    def set_curve(self, pid: str, color: QColor, info: dict[str, Any]) -> None:
        if pid != self.pid:
            self.dragging = None
        self.pid, self.color = pid, color
        self.setObjectName(f"lumetri.curve.{pid}")
        if self.dragging is None:
            self.points = points_of(info.get("value"))
        self.line = [min(1.0, max(0.0, finite(y))) for y in info.get("line") or []]
        self.update()

    def area(self) -> QRectF:
        w = min(float(self.width()), 260.0)
        h = w * 0.45 if self.hue else w
        return QRectF(0, 0, w, h)

    def sizeHint(self) -> QSize:
        return QSize(260, int(260 * 0.45) + 6 if self.hue else 266)

    def resizeEvent(self, _e: Any) -> None:
        self.setFixedHeight(int(self.area().height()) + 6)

    def to_screen(self, q: list[float]) -> QPointF:
        a = self.area()
        return QPointF(a.left() + q[0] * a.width(), a.bottom() - q[1] * a.height())

    def from_screen(self, s: QPointF) -> list[float]:
        a = self.area()
        x = (s.x() - a.left()) / max(1.0, a.width())
        y = (a.bottom() - s.y()) / max(1.0, a.height())
        return [round(min(1.0, max(0.0, x)), 4), round(min(1.0, max(0.0, y)), 4)]

    def near(self, pos: QPointF) -> int | None:
        best = None
        for i, q in enumerate(self.points):
            d = math.hypot(self.to_screen(q).x() - pos.x(), self.to_screen(q).y() - pos.y())
            if d < 9 and (best is None or d < best[1]):
                best = (i, d)
        return None if best is None else best[0]

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        a = self.area()
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(CURVE_BG)
        p.drawRoundedRect(a, 2, 2)
        p.setPen(QPen(CURVE_GRID, 1))
        for i in range(1, 4):
            f = i / 4
            p.drawLine(QPointF(a.left() + a.width() * f, a.top()), QPointF(a.left() + a.width() * f, a.bottom()))
            p.drawLine(QPointF(a.left(), a.top() + a.height() * f), QPointF(a.right(), a.top() + a.height() * f))
        if self.hue:
            n = 36
            for i in range(n):
                f = i / n
                p.fillRect(QRectF(a.left() + a.width() * f, a.bottom() - 6, a.width() / n + 0.5, 6), gradient_color("hue", f))
        if len(self.line) > 1:
            last = len(self.line) - 1
            poly = QPolygonF([self.to_screen([i / last, y]) for i, y in enumerate(self.line)])
            p.setPen(QPen(self.color, 1.5))
            p.drawPolyline(poly)
        p.setPen(Qt.PenStyle.NoPen)
        for i, q in enumerate(self.points):
            p.setBrush(t.c("hot_text") if self.dragging == i else self.color)
            p.drawEllipse(self.to_screen(q), 4, 4)

    def _send(self, immediate: bool) -> None:
        self.update()
        self.changed.emit(self.pid, [list(q) for q in self.points], immediate)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton or not self.area().contains(e.position()):
            return
        i = self.near(e.position())
        if i is None:
            self.points.append(self.from_screen(e.position()))
            i = len(self.points) - 1
            self._send(False)
        self.dragging = i

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging is not None and self.dragging < len(self.points):
            self.points[self.dragging] = self.from_screen(e.position())
            self._send(False)

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self.dragging = None
        self.update()

    def mouseDoubleClickEvent(self, e: QMouseEvent) -> None:
        if not self.area().contains(e.position()):
            return
        self.dragging = None
        i = self.near(e.position())
        if i is not None:
            # the end points are the leftmost and rightmost, not the first and last stored: an
            # added point is appended after them
            xs = [q[0] for q in self.points]
            if self.hue or (len(self.points) > 2 and min(xs) < xs[i] < max(xs)):
                del self.points[i]
                self._send(True)
        elif not self.hue:
            self.points = [[0.0, 0.0], [1.0, 1.0]]
            self._send(True)


class ColorWheel(QWidget):
    """egui's colour wheel: a hue ring with a puck (the offset, drag or click; double-click
    centres it) and the lightness slider at its left edge."""

    changed = Signal(str, object, bool)  # param id, [x, y] or the lightness, immediate

    def __init__(self, pid: str, lid: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.pid, self.lid = pid, lid
        self.setObjectName(f"lumetri.wheel.{pid}")
        self.offset = [0.0, 0.0]
        self.lightness = 0.0
        self.drag: str | None = None
        self.setMinimumSize(60, 60)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)

    def set_values(self, offset: Any, lightness: Any) -> None:
        if self.drag is None:
            xy = offset if isinstance(offset, list) and len(offset) == 2 else [0.0, 0.0]
            self.offset = [finite(xy[0]), finite(xy[1])]
            self.lightness = finite(lightness)
        self.update()

    def resizeEvent(self, _e: Any) -> None:
        self.setFixedHeight(min(self.width(), 120))

    def size_(self) -> float:
        return float(min(self.width() - 4, 120))

    def geometry_(self) -> tuple[QPointF, float, float, QRectF]:
        s = self.size_()
        c = QPointF(s / 2 + 8, s / 2)
        r_out = s / 2 - 4
        r_in = r_out - 10
        track = QRectF(3, 8, 4, max(1.0, s - 16))  # the knob (r 4) stays inside
        return c, r_out, r_in, track

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        c, r_out, r_in, track = self.geometry_()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        ring = QConicalGradient(c, 0)
        for i in range(37):
            f = i / 36
            ring.setColorAt(f, QColor.fromHslF((1.0 - f) % 1.0, 0.75, 0.5))
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(ring)
        p.drawEllipse(c, r_out, r_out)
        p.setBrush(CURVE_BG)
        p.drawEllipse(c, r_in - 1, r_in - 1)
        p.setPen(QPen(t.c("text_faint"), 1))
        p.drawLine(c - QPointF(6, 0), c + QPointF(6, 0))
        p.drawLine(c - QPointF(0, 6), c + QPointF(0, 6))
        puck = c + QPointF(self.offset[0], -self.offset[1]) * (r_in - 4)
        p.setPen(QPen(QColor("white"), 1.5))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawEllipse(puck, 5, 5)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(QColor(0x4B, 0x4B, 0x4B))
        p.drawRoundedRect(track, 2, 2)
        ky = track.center().y() - self.lightness / 100.0 * track.height() / 2
        p.setPen(QPen(KNOB, 1.5))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawEllipse(QPointF(track.center().x(), ky), 4, 4)

    def _apply(self, pos: QPointF, immediate: bool) -> None:
        c, _ro, r_in, track = self.geometry_()
        if self.drag == "lightness":
            v = (track.center().y() - pos.y()) / (track.height() / 2) * 100.0
            self.lightness = round(max(-100.0, min(100.0, v)), 1)
            self.changed.emit(self.lid, self.lightness, immediate)
        else:
            dx, dy = (pos.x() - c.x()) / (r_in - 4), (pos.y() - c.y()) / (r_in - 4)
            length = math.hypot(dx, dy)
            k = min(1.0, length) / length if length > 1e-6 else 1.0
            self.offset = [round(dx * k, 4), round(-dy * k, 4)]
            self.changed.emit(self.pid, list(self.offset), immediate)
        self.update()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton:
            return
        _c, _ro, _ri, track = self.geometry_()
        self.drag = "lightness" if track.adjusted(-6, 0, 6, 0).contains(e.position()) else "offset"
        self._apply(e.position(), False)

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.drag is not None:
            self._apply(e.position(), False)

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self.drag = None

    def mouseDoubleClickEvent(self, e: QMouseEvent) -> None:
        _c, _ro, _ri, track = self.geometry_()
        if not track.adjusted(-6, 0, 6, 0).contains(e.position()):
            self.drag = None
            self.offset = [0.0, 0.0]
            self.update()
            self.changed.emit(self.pid, [0.0, 0.0], True)


class SectionHeader(QWidget):
    """A section's title row: chevron and name (click: expand or collapse) and the section's
    on/off switch at the right (``lumetri.setSection``)."""

    toggled_open = Signal(str)
    switched = Signal(str)

    def __init__(self, title: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.title = title
        self.setObjectName(f"lumetri.section.{title}")
        self.setFixedHeight(40)
        self.expanded = False
        self.on = True

    def switch_rect(self) -> QRectF:
        return QRectF(self.width() - 31, self.height() / 2 - 7, 26, 14)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(t.c("separator"))
        p.drawLine(0, 0, self.width(), 0)
        mid = self.height() / 2
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("text_dim"))
        if self.expanded:
            chevron = [QPointF(2, mid - 3), QPointF(10, mid - 3), QPointF(6, mid + 3)]
        else:
            chevron = [QPointF(3, mid - 4), QPointF(9, mid), QPointF(3, mid + 4)]
        p.drawPolygon(QPolygonF(chevron))
        font = QFont(self.font())
        font.setPixelSize(13)
        font.setWeight(QFont.Weight.DemiBold)
        p.setFont(font)
        p.setPen(t.c("text"))
        p.drawText(QRectF(20, 0, self.width() - 60, self.height()), Qt.AlignmentFlag.AlignVCenter, tr(self.title))
        sw = self.switch_rect()
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("accent") if self.on else SWITCH_OFF)
        p.drawRoundedRect(sw, 7, 7)
        p.setBrush(QColor("white"))
        p.drawEllipse(QPointF(sw.right() - 7 if self.on else sw.left() + 7, sw.center().y()), 5, 5)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton:
            return
        if self.switch_rect().adjusted(-3, -3, 3, 3).contains(e.position()):
            self.switched.emit(self.title)
        else:
            self.toggled_open.emit(self.title)


class LumetriColorPanel(QWidget):
    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.LumetriColor")
        self.settings: QSettings | None = None
        self.data: dict[str, Any] = {}
        self.params: dict[str, dict[str, Any]] = {}
        self.open_sections: set[str] = {"Basic Correction"}
        self.channel = 0  # the RGB curve shown: luma, red, green, blue
        self.playhead: int | None = None
        self.program: Any = None  # the Program monitor (Comparison View)
        self._shape: tuple[Any, ...] | None = None
        self._pending: dict[tuple[int, int, str, int | None], Any] = {}
        self._send_timer = QTimer(self, singleShot=True, interval=SEND_DELAY_MS)
        self._send_timer.timeout.connect(self.flush)
        self._last_send = QElapsedTimer()

        self.header = QWidget(self)
        hl = QHBoxLayout(self.header)
        hl.setContentsMargins(12, 6, 12, 2)
        self.title = QLabel(self.header)
        self.title.setObjectName("lumetri.title")
        self.title.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        self.fx = QToolButton(self.header)
        self.fx.setObjectName("lumetri.fx")
        self.fx.setText("fx")
        self.fx.setAutoRaise(True)
        self.fx.setToolTip(tr("Toggle the effect on or off"))
        self.fx.clicked.connect(self._toggle_fx)
        hl.addWidget(self.title, 1)
        hl.addWidget(self.fx)
        self.placeholder = QLabel(tr("(no clip selected)"), self)
        self.placeholder.setObjectName("lumetri.placeholder")
        self.placeholder.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.add_button = QPushButton(tr("Add Lumetri Color to clip"), self)
        self.add_button.setObjectName("lumetri.add")
        self.add_button.clicked.connect(self._add)
        self.body = QWidget(self)
        self.body_layout = QVBoxLayout(self.body)
        self.body_layout.setContentsMargins(12, 0, 12, 20)
        self.body_layout.setSpacing(2)
        self.scroll = QScrollArea(self)
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setWidget(self.body)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.header)
        layout.addWidget(self.placeholder, 1)
        add_row = QHBoxLayout()
        add_row.setContentsMargins(12, 8, 12, 8)
        add_row.addWidget(self.add_button)
        add_row.addStretch(1)
        layout.addLayout(add_row)
        layout.addWidget(self.scroll, 1)
        self.sliders: dict[str, Slider] = {}
        self.headers: dict[str, SectionHeader] = {}
        self.bodies: dict[str, QWidget] = {}
        self.combos: dict[str, QComboBox] = {}
        self.curves: list[CurveEditor] = []
        self.wheels: list[ColorWheel] = []
        self.channel_buttons: list[ChannelButton] = []
        self._build()
        self.set_state(None)

    def attach_settings(self, settings: QSettings) -> None:
        self.settings = settings
        saved = settings.value("lumetri/openSections")
        if isinstance(saved, str):
            self.open_sections = {s for s in saved.split("|") if s}
        elif isinstance(saved, list):
            self.open_sections = {str(s) for s in saved if s}
        self._layout_sections()

    # ----- building -----
    def _section(self, title: str) -> QVBoxLayout:
        head = SectionHeader(title, self.body)
        head.toggled_open.connect(self._toggle_open)
        head.switched.connect(self._switch)
        box = QWidget(self.body)
        box.setObjectName(f"lumetri.body.{title}")
        v = QVBoxLayout(box)
        v.setContentsMargins(0, 0, 0, 6)
        v.setSpacing(0)
        self.body_layout.addWidget(head)
        self.body_layout.addWidget(box)
        self.headers[title] = head
        self.bodies[title] = box
        return v

    def _sub(self, v: QVBoxLayout, text: str, name: str = "") -> QLabel:
        label = QLabel(tr(text))
        font = label.font()
        font.setBold(True)
        font.setPixelSize(12)
        label.setFont(font)
        label.setContentsMargins(4, 4, 0, 0)
        if name:
            label.setObjectName(name)
        v.addWidget(label)
        return label

    def _slider(self, v: QVBoxLayout, pid: str, label: str, gradient: str | None = None) -> Slider:
        s = Slider(pid, tr(label), gradient)
        s.changed.connect(lambda p, val, now: self.set_param(p, val, now))
        s.gesture.connect(self.new_gesture)
        v.addWidget(s)
        self.sliders[pid] = s
        return s

    def _combo(self, v: QVBoxLayout, key: str, label: str) -> QComboBox:
        row = QWidget()
        h = QHBoxLayout(row)
        h.setContentsMargins(4, 2, 0, 2)
        text = QLabel(tr(label))
        text.setFixedWidth(LABEL_W - 4)
        text.setEnabled(False)
        combo = QComboBox(row)
        combo.setObjectName(f"lumetri.{key}")
        combo.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Fixed)
        h.addWidget(text)
        h.addWidget(combo, 1)
        h.addSpacing(VALUE_W - 18)
        v.addWidget(row)
        self.combos[key] = combo
        return combo

    def _build(self) -> None:
        # Basic Correction
        v = self._section("Basic Correction")
        self._combo(v, "input_lut", N_("Input LUT")).activated.connect(lambda i: self._lut("input_lut", i))
        self._sub(v, N_("Color"))
        self._slider(v, "temperature", N_("Temperature"), "temp")
        self._slider(v, "tint", N_("Tint"), "tint")
        self._slider(v, "saturation", N_("Saturation"))
        self._sub(v, N_("Light"))
        for pid, label in (
            ("exposure", N_("Exposure")),
            ("contrast", N_("Contrast")),
            ("highlights", N_("Highlights")),
            ("shadows", N_("Shadows")),
            ("whites", N_("Whites")),
            ("blacks", N_("Blacks")),
        ):
            self._slider(v, pid, label)
        self.hdr_widgets: list[QWidget] = [self._sub(v, N_("HDR"))]
        self.hdr_widgets += [self._slider(v, "hdr_white", N_("HDR White")), self._slider(v, "hdr_specular", N_("HDR Specular"))]
        # Creative
        v = self._section("Creative")
        look = self._combo(v, "look", N_("Look"))
        for i, name in enumerate(LOOKS):
            look.addItem(tr(name), i)
        look.activated.connect(lambda i: self.set_param("look", int(i), True))
        self._combo(v, "look_lut", N_("Look LUT")).activated.connect(lambda i: self._lut("look_lut", i))
        self._slider(v, "look_intensity", N_("Intensity"))
        self._sub(v, N_("Adjustments"))
        for pid, label in (
            ("faded_film", N_("Faded Film")),
            ("sharpen", N_("Sharpen")),
            ("vibrance", N_("Vibrance")),
            ("creative_sat", N_("Saturation")),
        ):
            self._slider(v, pid, label)
        # Curves
        v = self._section("Curves")
        self._sub(v, N_("RGB Curves"))
        row = QHBoxLayout()
        row.setContentsMargins(0, 2, 0, 2)
        for i, (pid, color) in enumerate(CHANNELS):
            b = ChannelButton(color)
            b.setObjectName(f"lumetri.channel.{pid}")
            b.clicked.connect(lambda _c=False, n=i: self._channel(n))
            row.addWidget(b)
            self.channel_buttons.append(b)
        row.addStretch(1)
        v.addLayout(row)
        self.rgb_curve = CurveEditor(CHANNELS[0][0], False, CHANNELS[0][1])
        self.rgb_curve.changed.connect(lambda p, pts, now: self.set_param(p, pts, now))
        v.addWidget(self.rgb_curve)
        self.curves.append(self.rgb_curve)
        self.hdr_widgets.append(self._slider(v, "curves_hdr_range", N_("HDR Range")))
        self._sub(v, N_("Hue Saturation Curves"))
        for pid, label in HUE_CURVES:
            text = QLabel(tr(label))
            text.setEnabled(False)
            v.addWidget(text)
            c = CurveEditor(pid, True, QColor("white"))
            c.changed.connect(lambda p, pts, now: self.set_param(p, pts, now))
            v.addWidget(c)
            self.curves.append(c)
        # Color Wheels & Match
        v = self._section("Color Wheels & Match")
        row = QHBoxLayout()
        row.setSpacing(8)
        for pid, lid, label in WHEELS:
            col = QVBoxLayout()
            text = QLabel(tr(label))
            text.setEnabled(False)
            col.addWidget(text)
            w = ColorWheel(pid, lid)
            w.changed.connect(lambda p, val, now: self.set_param(p, val, now))
            col.addWidget(w)
            row.addLayout(col, 1)
            self.wheels.append(w)
        v.addLayout(row)
        self._sub(v, N_("Color Match"))
        # Comparison View on / off (the Program monitor's display mode); Apply Match then uses its
        # reference frame, as egui's
        self.compare = QPushButton(tr("Comparison View"))
        self.compare.setObjectName("lumetri.match.comparisonView")
        self.compare.setCheckable(True)
        self.compare.clicked.connect(self._toggle_compare)
        v.addWidget(self.compare, 0, Qt.AlignmentFlag.AlignLeft)
        ref_row = QHBoxLayout()
        ref_row.setContentsMargins(4, 2, 0, 2)
        text = QLabel(tr("Reference"))
        text.setFixedWidth(LABEL_W - 4)
        text.setEnabled(False)
        self.reference = QLineEdit("00:00:00:00")
        self.reference.setObjectName("lumetri.match.reference")
        self.reference.setFixedWidth(96)
        ref_row.addWidget(text)
        ref_row.addWidget(self.reference)
        ref_row.addStretch(1)
        v.addLayout(ref_row)
        self.face = QCheckBox(tr("Face Detection (protect skin tones)"))
        self.face.setObjectName("lumetri.match.faceDetection")
        self.face.setChecked(True)
        v.addWidget(self.face)
        self.apply_match = QPushButton(tr("Apply Match"))
        self.apply_match.setObjectName("lumetri.match.apply")
        self.apply_match.clicked.connect(self._apply_match)
        v.addWidget(self.apply_match, 0, Qt.AlignmentFlag.AlignLeft)
        # HSL Secondary
        v = self._section("HSL Secondary")
        self._sub(v, N_("Key"))
        self._slider(v, "hsl_hue", N_("Hue"), "hue")
        for pid, label in (
            ("hsl_hue_range", N_("Hue Range")),
            ("hsl_sat_min", N_("Saturation Min")),
            ("hsl_luma_min", N_("Luma Min")),
            ("hsl_luma_max", N_("Luma Max")),
            ("hsl_soft", N_("Soften")),
        ):
            self._slider(v, pid, label)
        mask = self._combo(v, "hsl_show_mask", N_("Show Mask"))
        for i, name in enumerate(SHOW_MASK):
            mask.addItem(tr(name), i)
        mask.activated.connect(lambda i: self.set_param("hsl_show_mask", int(i), True))
        self._sub(v, N_("Refine"))
        self._slider(v, "hsl_denoise", N_("Denoise"))
        self._slider(v, "hsl_blur", N_("Blur"))
        self._sub(v, N_("Correction"))
        self._slider(v, "hsl_temp", N_("Temperature"), "temp")
        self._slider(v, "hsl_tint", N_("Tint"), "tint")
        self._slider(v, "hsl_sat", N_("Saturation"))
        self._slider(v, "hsl_hue_shift", N_("Hue Shift"), "hue")
        # Vignette
        v = self._section("Vignette")
        for pid, label in (
            ("vignette_amount", N_("Amount")),
            ("vignette_midpoint", N_("Midpoint")),
            ("vignette_roundness", N_("Roundness")),
            ("vignette_feather", N_("Feather")),
        ):
            self._slider(v, pid, label)
        self.body_layout.addStretch(1)
        self._channel(0)
        self._layout_sections()

    # ----- engine state -> widgets -----
    @property
    def clip_id(self) -> int | None:
        c = self.data.get("clip")
        return c if isinstance(c, int) else None

    @property
    def effect(self) -> int | None:
        e = self.data.get("effect")
        return e if isinstance(e, int) else None

    def set_state(self, data: Any) -> None:
        """Show what ``lumetri.inspect`` returned (or nothing)."""
        data = data if isinstance(data, dict) else {}
        if (data.get("clip"), data.get("effect")) != (self.data.get("clip"), self.data.get("effect")):
            self.flush()  # edits made so far go to the clip they were made on
        self.data = data
        raw = data.get("params")
        self.params = {str(k): v for k, v in raw.items() if isinstance(v, dict)} if isinstance(raw, dict) else {}
        has_clip = self.clip_id is not None
        has_fx = has_clip and self.effect is not None
        self.placeholder.setVisible(not has_clip)
        self.header.setVisible(has_clip)
        self.add_button.setVisible(has_clip and not has_fx)
        self.scroll.setVisible(has_fx)
        self.fx.setVisible(has_fx)
        self.title.setText(fmt("Master · {name}", name=str(data.get("name") or "")) if has_clip else "")
        if not has_fx:
            return
        t = theme.current()
        self.fx.setStyleSheet(f"font-style: italic; color: {t.text if data.get('enabled', True) else t.text_faint};")
        hdr = bool(data.get("hdr"))
        for w in self.hdr_widgets:
            w.setVisible(hdr)
        for pid, slider in self.sliders.items():
            info = self.params.get(pid)
            if info is not None:
                slider.set_info(info)
        for key in ("look", "hsl_show_mask"):
            combo = self.combos[key]
            if not combo.view().isVisible():
                combo.setCurrentIndex(max(0, min(combo.count() - 1, int(finite((self.params.get(key) or {}).get("value"))))))
        luts = data.get("luts") if isinstance(data.get("luts"), dict) else {}
        for key, menu in (("input_lut", "input"), ("look_lut", "look")):
            self._fill_lut(key, [e for e in luts.get(menu) or [] if isinstance(e, dict)])
        pid, color = CHANNELS[self.channel]
        self.rgb_curve.set_curve(pid, color, self.params.get(pid) or {})
        for c in self.curves[1:]:
            c.set_curve(c.pid, c.color, self.params.get(c.pid) or {})
        for w in self.wheels:
            w.set_values((self.params.get(w.pid) or {}).get("value"), (self.params.get(w.lid) or {}).get("value"))
        for title, param, _sec in SECTIONS:
            head = self.headers[title]
            head.on = bool((self.params.get(param) or {}).get("value", param != "hsl_on"))
            head.update()

    def _fill_lut(self, key: str, entries: list[dict[str, Any]]) -> None:
        combo = self.combos[key]
        if combo.view().isVisible():
            return
        current = str((self.params.get(key) or {}).get("value") or "")
        refs = [str(e.get("ref") or "") for e in entries]
        if current not in refs:
            entries = [*entries, {"ref": current, "name": (self.params.get(key) or {}).get("name") or current}]
        combo.blockSignals(True)
        combo.clear()
        for e in entries:
            ref = str(e.get("ref") or "")
            combo.addItem(tr("None") if ref == "" else str(e.get("name") or ref), ref)
        combo.insertSeparator(combo.count())
        combo.addItem(tr("Browse…"), "__browse__")
        combo.setCurrentIndex(max(0, combo.findData(current)))
        combo.blockSignals(False)

    def _layout_sections(self) -> None:
        for title, head in self.headers.items():
            head.expanded = title in self.open_sections
            self.bodies[title].setVisible(head.expanded)
            head.update()

    # ----- widgets -> engine -----
    def _toggle_open(self, title: str) -> None:
        self.open_sections ^= {title}
        if self.settings is not None:
            self.settings.setValue("lumetri/openSections", "|".join(sorted(self.open_sections)))
        self._layout_sections()

    def _switch(self, title: str) -> None:
        sec = next((s for t, _p, s in SECTIONS if t == title), None)
        if sec is not None and self.clip_id is not None:
            self.send("lumetri.setSection", {"clip": self.clip_id, "section": sec})

    def _channel(self, n: int) -> None:
        self.channel = n
        for i, b in enumerate(self.channel_buttons):
            b.setChecked(i == n)
        pid, color = CHANNELS[n]
        self.rgb_curve.set_curve(pid, color, self.params.get(pid) or {})

    def _toggle_fx(self) -> None:
        if self.clip_id is not None and self.effect is not None:
            self.send("effects.toggleEnabled", {"clip": self.clip_id, "index": self.effect})

    def _add(self) -> None:
        if self.clip_id is not None:
            self.send("effects.apply", {"clips": [self.clip_id], "effect": "lumetri"})

    def _lut(self, key: str, index: int) -> None:
        combo = self.combos[key]
        ref = combo.itemData(index)
        cmd = "lumetri.setInputLut" if key == "input_lut" else "lumetri.setLook"
        if self.clip_id is None:
            return
        if ref == "__browse__":
            path, _f = QFileDialog.getOpenFileName(self, tr("LUT"), "", "LUT (*.cube *.3dl)")
            if path:
                self.send(cmd, {"clip": self.clip_id, "path": path})
            else:
                self.set_state(self.data)
            return
        if isinstance(ref, str) and ref != str((self.params.get(key) or {}).get("value") or ""):
            self.send(cmd, {"clip": self.clip_id, "lut": ref})

    def _toggle_compare(self, on: bool) -> None:
        if self.program is not None:
            self.program.set_view(display="comparison" if on else "composite")
        self.show_comparison()

    def show_comparison(self) -> None:
        """The button follows the Program monitor; while comparing, Reference is its frame."""
        p = self.program
        on = p is not None and p.comparing()
        self.compare.setChecked(on)
        self.reference.setReadOnly(on)
        if on:
            from ..timecode import format_timecode

            self.reference.setText(format_timecode(p.reference(), p.frame(), p.drop_frame()))

    def _apply_match(self) -> None:
        if self.clip_id is None:
            return
        p = self.program
        if p is not None and p.comparing():
            ref: dict[str, Any] = {"referenceTime": p.reference()}
        else:
            ref = {"referenceTimecode": self.reference.text().strip()}
        self.send("lumetri.applyMatch", {"clip": self.clip_id, **ref, "faceDetection": self.face.isChecked()})

    def _edit_time(self, pid: str) -> int | None:
        """Where an edit of an animated value is keyed: the shown playhead, pinned to the clip."""
        if not (self.params.get(pid) or {}).get("animated"):
            return None
        start, end, ph = self.data.get("start"), self.data.get("end"), self.playhead
        if not all(isinstance(v, int) for v in (start, end, ph)):
            return None
        return max(start, min(ph, max(start, end - 1)))

    def set_param(self, pid: str, value: Any, immediate: bool = False) -> None:
        """A new value; drags are coalesced into one undo step (as Effect Controls)."""
        if self.clip_id is None or self.effect is None:
            return
        self._pending[(self.clip_id, self.effect, pid, self._edit_time(pid))] = value
        if pid in self.params:
            self.params[pid]["value"] = value
        if immediate:
            self.flush()
        else:
            self._send_timer.start()

    def new_gesture(self) -> None:
        """The next value starts its own undo step, however soon it follows the last one."""
        self.flush()
        self._new_gesture = True

    def send(self, command: str, params: dict[str, Any]) -> None:
        self.flush()
        self.command.emit(command, params)

    def flush(self) -> None:
        self._send_timer.stop()
        if not self._pending:
            return
        begin = getattr(self, "_new_gesture", False) or not self._last_send.isValid() or self._last_send.elapsed() > GESTURE_GAP_MS
        self._new_gesture = False
        for (clip, effect, param, at), value in self._pending.items():
            params: dict[str, Any] = {"clip": clip, "effect": effect, "param": param, "value": value, "merge": True, "begin": begin}
            if at is not None:
                params["time"] = at
            self.command.emit("effects.setParam", params)
        self._pending.clear()
        self._last_send.restart()
