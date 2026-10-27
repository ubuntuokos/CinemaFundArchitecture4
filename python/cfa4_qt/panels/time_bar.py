"""The monitors' time bar, as egui's (crates/ui-egui/src/panels/monitor.rs ``mini_timeline`` and
``time_zoom_bar``, ``source_range.rs``):

* the mini timeline: frame ticks that fill in as it zooms, the marked range as a grey band with
  curly braces at In and Out, the markers in their colours, the playhead's blue head. A click or
  drag scrubs; the band's middle drags the marked range, its ends drag In or Out (a red trim cue
  shows), one ``project.setMarks`` on release; Esc lets go.
* the zoom scroll bar under it: its thumb is the part the timeline shows. Drag a handle at either
  end to zoom, the middle to scroll, press beside it to bring it there. While the monitor plays, a
  zoomed bar follows the playhead (page scrolling).
"""

from __future__ import annotations

import math
from typing import Any

from PySide6.QtCore import QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QColor, QKeyEvent, QMouseEvent, QPainter, QPaintEvent, QPen, QPolygonF
from PySide6.QtWidgets import QWidget

from .. import theme
from ..i18n import tr
from ..timecode import TICKS_PER_SECOND
from .info import marker_rgb

BAR_H = 22.0  # the mini timeline
ZOOM_H = 13.0  # the zoom scroll bar, 2 px under it
INSET = 4.0
MIN_ZOOM_THUMB_W = 34.0  # room for the two handles and a part between them to grab
MIN_ZOOM_FRAMES = 5  # the fewest frames the bar can zoom in to
MAX_TICKS = 512
EDGE_GRAB = 6.0
RANGE_COLOR = QColor(0x5C, 0x5C, 0x5C, 128)
TRIM_RED = QColor("#e0413a")


def zoom_range(zoom: Any) -> tuple[float, float]:
    """The (start, end) fractions a zoom stands for, made usable: anything empty, reversed or not
    a number is the whole length."""
    if isinstance(zoom, (list, tuple)) and len(zoom) == 2:
        try:
            s, e = float(zoom[0]), float(zoom[1])
        except (TypeError, ValueError):
            return 0.0, 1.0
        if math.isfinite(s) and math.isfinite(e):
            s, e = min(max(s, 0.0), 1.0), min(max(e, 0.0), 1.0)
            if s < e:
                return s, e
    return 0.0, 1.0


def shown_times(zoom: Any, origin: int, end: int) -> tuple[int, int]:
    """The times of ``origin..end`` the timeline shows at ``zoom`` (egui's ``ruler_shown``)."""
    s, e = zoom_range(zoom)
    length = max(0, end - origin)
    return (origin + int(s * length) if s > 0 else origin, origin + int(e * length) if e < 1 else end)


def tick_steps(frame: int, dur: int, width: float) -> tuple[int, int]:
    """(minor, major) tick steps in frames, as on the Timeline's ruler: minor ones at least 12 px
    apart, major ones at least 110 px, the minor step dividing the major one."""
    frame_px = frame / max(1, dur) * width
    base = max(1, round(TICKS_PER_SECOND / max(1, frame)))
    steps = [1, 2, 5, 10, max(1, base // 2), *(base * k for k in (1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600))]
    major = next((s for s in steps if s * frame_px >= 110.0), base * 3600)
    minor = next((s for s in steps if major % s == 0 and s * frame_px >= 12.0), major)
    return minor, major


class ScrubBar(QWidget):
    """A monitor's mini timeline and time zoom bar."""

    seek = Signal(object)  # ticks; object, not a 32-bit int
    set_marks = Signal(dict)  # {"in": ticks?, "out": ticks?}: the marked range dragged
    range_drag_started = Signal()  # the monitor stops playing

    def __init__(self, name: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.name = name
        self.setObjectName(f"{name}.scrub")
        self.setFixedHeight(int(BAR_H + 2 + ZOOM_H + 2))
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setMouseTracking(True)
        self.start = 0
        self.end = 0
        self.playhead = 0
        self.mark_in: int | None = None
        self.mark_out: int | None = None
        self.frame = 1
        self.markers: list[tuple[int, QColor]] = []
        self.range_editable = True  # a subclip's range is its own: no handles
        self.edges_need_marks = False  # Program: In / Out handles only where marked
        self.zoom: tuple[float, float] | None = None  # the part shown, as fractions; None: all
        self._drag: dict[str, Any] | None = None
        self._hot = ""  # "in" / "out" under the pointer: a trim cue shows

    # ----- model -----
    def set_range(self, start: int, end: int, mark_in: int | None, mark_out: int | None, frame: int) -> None:
        self.start, self.end = int(start), max(int(start), int(end))
        self.mark_in, self.mark_out = mark_in, mark_out
        self.frame = max(1, int(frame))
        self._drag = None if self._drag is not None and self._drag.get("kind") == "range" else self._drag
        self.update()

    def set_markers(self, markers: Any) -> None:
        """The sequence's (Program) or the clip's (Source) markers: their starts and colours."""
        out = []
        for m in markers if isinstance(markers, list) else []:
            if isinstance(m, dict) and isinstance(m.get("start"), int) and not isinstance(m.get("start"), bool):
                out.append((m["start"], marker_rgb(m.get("color"))))
        self.markers = out[:10_000]
        self.update()

    def set_playhead(self, t: int) -> None:
        self.playhead = int(t)
        self.update()

    def set_zoom(self, zoom: Any) -> None:
        s, e = zoom_range(zoom)
        self.zoom = (s, e) if s > 0 or e < 1 else None
        self.update()

    def follow(self, t: int) -> None:
        """While playing: a zoomed bar brings a playhead running off its right edge (or jumping
        before its left one) back near the left edge (page scrolling), keeping its width."""
        length = self.end - self.start
        if self.zoom is None or length <= 0:
            return
        s, e = self.zoom
        span = e - s
        f = (t - self.start) / length
        across = (f - s) / span if span > 0 else 0.0
        if not 0.0 <= across <= 0.95:
            ns = min(max(f - span * 0.05, 0.0), 1.0 - span)
            self.zoom = (ns, min(ns + span, 1.0))
            self.update()

    # ----- geometry -----
    def bar_rect(self) -> QRectF:
        return QRectF(INSET, 0, max(1.0, self.width() - 2 * INSET), BAR_H)

    def zoom_rect(self) -> QRectF:
        return QRectF(INSET, BAR_H + 2, max(1.0, self.width() - 2 * INSET), ZOOM_H)

    def shown(self) -> tuple[int, int]:
        return shown_times(self.zoom, self.start, self.end)

    def _x(self, t: int) -> float:
        a, b = self.shown()
        bar = self.bar_rect()
        f = (t - a) / max(1, b - a)
        return bar.left() + min(max(f, 0.0), 1.0) * bar.width()

    def shows(self, t: int) -> bool:
        a, b = self.shown()
        return (t >= a or a <= self.start) and (t <= b or b >= self.end)

    def time_at(self, x: float) -> int:
        a, b = self.shown()
        bar = self.bar_rect()
        f = min(1.0, max(0.0, (x - bar.left()) / max(1.0, bar.width())))
        t = a + int(max(0, b - a) * f)
        # snap to the frame grid, never past the last frame
        t = self.start + (t - self.start) // self.frame * self.frame
        return min(max(t, self.start), max(self.start, self.end - self.frame))

    def selected(self) -> tuple[int, int]:
        """The marked range: In to the end of Out (unset marks: the ends)."""
        a = min(max(self.mark_in if self.mark_in is not None else self.start, self.start), self.end)
        b = self.mark_out + self.frame if self.mark_out is not None else self.end
        return a, min(max(b, a), self.end)

    def range_part(self, x: float, y: float) -> str:
        """What a press at (x, y) grabs of the marked range: "in", "out", "move" or ""."""
        if not self.range_editable or self.end <= self.start or y > BAR_H:
            return ""
        a_t, b_t = self.selected()
        a, b = self._x(a_t), self._x(b_t)
        mid = (a + b) / 2
        need = self.edges_need_marks
        if not (need and self.mark_in is None) and self.shows(a_t) and a - EDGE_GRAB <= x < min(a + EDGE_GRAB, mid):
            return "in"
        if not (need and self.mark_out is None) and self.shows(b_t) and max(b - EDGE_GRAB, mid) <= x <= b + EDGE_GRAB:
            return "out"
        if not (need and (self.mark_in is None or self.mark_out is None)) and y >= 8 and min(a + 6, mid) <= x <= max(b - 6, mid) \
                and (self.mark_in is not None or self.mark_out is not None):
            return "move"
        return ""

    def thumb(self) -> tuple[QRectF, float, float]:
        """The zoom bar's thumb, how far it can travel, the fraction one pixel scrolls."""
        bar = self.zoom_rect()
        s, e = self.zoom or (0.0, 1.0)
        span = e - s
        w = min(max(span * bar.width(), min(MIN_ZOOM_THUMB_W, bar.width())), bar.width())
        travel = max(0.0, bar.width() - w)
        rng = max(0.0, 1.0 - span)
        left = bar.left() + (min(max(s / rng, 0.0), 1.0) * travel if rng > 0 else 0.0)
        per_px = rng / travel if travel > 0.5 else 0.0
        return QRectF(left, bar.top(), w, bar.height()), travel, per_px

    def min_span(self) -> float:
        length = self.end - self.start
        return min(self.frame * MIN_ZOOM_FRAMES / length, 1.0) if length > 0 else 1.0

    # ----- painting -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        T = theme.current()
        p.fillRect(self.rect(), T.c("panel_bg"))
        if self.end <= self.start:
            return
        self._paint_zoom_bar(p, T)
        bar = self.bar_rect()
        a, b = self.shown()
        dur = max(1, b - a)
        minor, major = tick_steps(self.frame, dur, bar.width())
        f = (a // self.frame) // minor * minor
        p.setPen(QPen(T.c("text_faint"), 1))
        for _ in range(MAX_TICKS):
            x = bar.left() + (f * self.frame - a) / dur * bar.width()
            if x > bar.right() + 0.5:
                break
            if x >= bar.left() - 0.5:
                h = 8.0 if f % major == 0 else 3.5
                p.drawLine(QPointF(x, bar.bottom() - h), QPointF(x, bar.bottom()))
            f += minor
        a_t, b_t = self._preview() or self.selected()
        xa, xb = self._x(a_t), self._x(b_t)
        marked = self.mark_in is not None or self.mark_out is not None
        if marked:
            p.fillRect(QRectF(xa, bar.top() + 8, max(0.0, xb - xa), bar.height() - 8), RANGE_COLOR)
            top, bottom = bar.top() + 8, bar.bottom()
            mid = (top + bottom) / 2
            p.setPen(QPen(T.c("playhead"), 1))
            p.setBrush(Qt.BrushStyle.NoBrush)
            for x, d, hot, edge, mark in ((xa, 1.0, self._hot == "in", a_t, self.mark_in), (xb, -1.0, self._hot == "out", b_t, self.mark_out)):
                if hot or mark is None or not self.shows(edge):
                    continue
                pts = [(4.0, top), (2.8, top), (2.0, top + 1), (2.0, mid - 2), (1.4, mid - 1), (0.0, mid), (1.4, mid + 1),
                       (2.0, mid + 2), (2.0, bottom - 1), (2.8, bottom), (4.0, bottom)]
                p.drawPolyline(QPolygonF([QPointF(x + d * o, y) for o, y in pts]))
        for t, c in self.markers:
            if self.shows(t):
                x, y = self._x(t), bar.top()
                p.setPen(Qt.PenStyle.NoPen)
                p.setBrush(c)
                p.drawPolygon(QPolygonF([QPointF(x - 3.5, y), QPointF(x + 3.5, y), QPointF(x + 3.5, y + 7), QPointF(x, y + 10),
                                         QPointF(x - 3.5, y + 7)]))
        if self.shows(self.playhead):
            x = self._x(self.playhead)
            hy = bar.bottom() - 11
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(T.c("playhead"))
            p.drawPolygon(QPolygonF([QPointF(x - 5.5, hy), QPointF(x + 5.5, hy), QPointF(x + 5.5, hy + 5), QPointF(x, hy + 9),
                                     QPointF(x - 5.5, hy + 5)]))
            p.setPen(QPen(T.c("playhead"), 1))
            p.drawLine(QPointF(x, hy + 8), QPointF(x, bar.bottom()))
        if self._hot in ("in", "out"):
            self._trim_cue(p, xa if self._hot == "in" else xb, 4.0 if self._hot == "in" else -4.0)

    def _trim_cue(self, p: QPainter, x: float, inward: float) -> None:
        bar = self.bar_rect()
        top, bottom = bar.top() + 8, bar.bottom()
        mid = (top + bottom) / 2
        p.setPen(QPen(TRIM_RED, 1.2))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawPolyline(QPolygonF([QPointF(x + inward, top), QPointF(x, top), QPointF(x, bottom), QPointF(x + inward, bottom)]))
        p.drawLine(QPointF(x - 9, mid), QPointF(x + 9, mid))
        for d in (-1.0, 1.0):
            p.drawPolyline(QPolygonF([QPointF(x + d * 6, mid - 2.5), QPointF(x + d * 9, mid), QPointF(x + d * 6, mid + 2.5)]))

    def _paint_zoom_bar(self, p: QPainter, T: Any) -> None:
        bar = self.zoom_rect()
        thumb, _travel, _per = self.thumb()
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(T.c("separator"))
        p.drawRoundedRect(bar, bar.height() / 2, bar.height() / 2)
        hover = self._drag is not None and self._drag.get("kind") == "zoom" or thumb.contains(self.mapFromGlobal(self.cursor().pos()).toPointF())
        p.setBrush(T.c("scroll_thumb_hover" if hover else "scroll_thumb"))
        p.drawRoundedRect(thumb, thumb.height() / 2, thumb.height() / 2)
        for x in (thumb.left() + thumb.height() / 2, thumb.right() - thumb.height() / 2):
            p.setPen(QPen(T.c("text"), 1.5))
            p.setBrush(T.c("panel_bg"))
            p.drawEllipse(QPointF(x, thumb.center().y()), 4.5, 4.5)

    # ----- mouse -----
    def _preview(self) -> tuple[int, int] | None:
        d = self._drag
        return d.get("preview") if d is not None and d.get("kind") == "range" else None

    def _range_preview(self, part: str, original: tuple[int, int], delta: int) -> tuple[int, int]:
        """The marked range with ``part`` dragged by ``delta`` ticks on whole frames, inside the
        clip or sequence and at least a frame long (egui's ``adjust_range``)."""
        fd = self.frame
        lo, hi = self.start, self.end
        a, b = original

        def snap(t: int) -> int:
            return lo + round((t - lo) / fd) * fd

        if part == "in":
            return min(max(snap(a + delta), lo), max(lo, b - fd)), b
        if part == "out":
            o = min(max(snap(b - fd + delta), a), max(a, hi - fd))
            return a, min(o + fd, hi)
        d = min(max(round(delta / fd) * fd, lo - a), hi - b)
        return a + d, b + d

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton or self.end <= self.start:
            return
        pos = e.position()
        if pos.y() >= BAR_H:  # the zoom bar
            thumb, _travel, per_px = self.thumb()
            s, end = self.zoom or (0.0, 1.0)
            span = end - s
            if not thumb.adjusted(0, -4, 0, 4).contains(pos):
                if per_px > 0:
                    s = min(max((pos.x() - self.zoom_rect().left() - thumb.width() / 2) * per_px, 0.0), 1.0 - span)
                    self.set_zoom((s, s + span))
                mode = "scroll"
            elif abs(pos.x() - thumb.left()) < 9:
                mode = "start"
            elif abs(pos.x() - thumb.right()) < 9:
                mode = "end"
            else:
                mode = "scroll"
            self._drag = {"kind": "zoom", "mode": mode, "x": pos.x(), "start": (self.zoom or (0.0, 1.0))[0],
                          "zoom": self.zoom or (0.0, 1.0)}
            return
        part = self.range_part(pos.x(), pos.y())
        playhead_x = self._x(self.playhead)
        if part and not (self.shows(self.playhead) and abs(pos.x() - playhead_x) <= 7 and pos.y() >= BAR_H - 12):
            # (a click on the band without a drag moves the playhead there)
            self._drag = {"kind": "range", "part": part, "x": pos.x(), "original": self.selected(),
                          "preview": self.selected(), "moved": False}
            self.range_drag_started.emit()
            return
        self._drag = {"kind": "scrub"}
        self.seek.emit(self.time_at(pos.x()))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        pos = e.position()
        d = self._drag
        if d is None:
            part = self.range_part(pos.x(), pos.y())
            hot = part if part in ("in", "out") else ""
            self.setCursor(Qt.CursorShape.SizeHorCursor if hot else Qt.CursorShape.OpenHandCursor if part == "move"
                           else Qt.CursorShape.ArrowCursor)
            self.setToolTip({"in": tr("Drag In point"), "out": tr("Drag Out point"), "move": tr("Move marked range")}.get(part, ""))
            if hot != self._hot:
                self._hot = hot
                self.update()
            elif pos.y() >= BAR_H:
                self.update()  # the thumb's hover
            return
        if not e.buttons() & Qt.MouseButton.LeftButton:
            return
        if d["kind"] == "scrub":
            self.seek.emit(self.time_at(pos.x()))
        elif d["kind"] == "zoom":
            s, end = d["zoom"]
            span = end - s
            if d["mode"] == "scroll":
                _thumb, _travel, per_px = self.thumb()
                ns = min(max(d["start"] + (pos.x() - d["x"]) * per_px, 0.0), max(0.0, 1.0 - span))
                self.set_zoom((ns, ns + span))
            else:
                dx = (pos.x() - d["x"]) / max(1.0, self.zoom_rect().width())
                ms = self.min_span()
                if d["mode"] == "start":
                    self.set_zoom((min(max(s + dx, 0.0), max(end - ms, 0.0)), end))
                else:
                    self.set_zoom((s, min(max(end + dx, min(s + ms, 1.0)), 1.0)))
        elif d["kind"] == "range":
            a, b = self.shown()
            delta = int((pos.x() - d["x"]) / max(1.0, self.bar_rect().width()) * max(1, b - a))
            d["moved"] = d["moved"] or abs(pos.x() - d["x"]) > 2
            d["preview"] = self._range_preview(d["part"], d["original"], delta)
            self._hot = d["part"] if d["part"] in ("in", "out") else ""
            self.update()

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        d, self._drag = self._drag, None
        if d is None or d["kind"] != "range":
            self.update()
            return
        if not d["moved"]:
            if d["part"] == "move":
                self.seek.emit(self.time_at(e.position().x()))
            self.update()
            return
        a, b = d["preview"]
        if (a, b) != d["original"]:
            params: dict[str, Any] = {}
            if d["part"] != "out":
                params["in"] = a
            if d["part"] != "in":
                params["out"] = max(self.start, b - self.frame)
            self.set_marks.emit(params)
        self.update()

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and self._drag is not None:
            self._drag = None  # the drag is abandoned
            self.update()
            return
        super().keyPressEvent(e)

    def leaveEvent(self, _e: Any) -> None:
        if self._hot:
            self._hot = ""
            self.update()
