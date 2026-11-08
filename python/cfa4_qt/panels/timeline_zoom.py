"""egui's zoom scroll bar under the Timeline (``timeline.rs`` ``zoom_scrollbar`` / ``zoom_thumb``).

The thumb is the part of the Timeline in view, as wide as its share of the scroll range (the
sequence and ten minutes after it) and never narrower than ``MIN_THUMB_W``. Its middle drags the
view (the thumb staying under the pointer), a handle at either end zooms with the other end of the
view staying put, a press beside the thumb brings the thumb's middle there (and a drag goes on from
there), and Escape during a drag puts the view back. On the control channel the thumb is
``timeline.zoomBar`` and the bar ``timeline.zoomBar.track``.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QPointF, QRectF, QSize, Qt
from PySide6.QtGui import QKeyEvent, QMouseEvent, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QWidget

from .. import theme
from ..timecode import TICKS_PER_SECOND

if TYPE_CHECKING:
    from .timeline import TimelineCanvas

ZOOM_BAR_H = 17  # egui's SCROLLBAR_H; the bar itself 2 px in from top and bottom
MIN_THUMB_W = 34.0  # room for the two handles and a part between them to grab
HANDLE_GRAB = 9.0  # how near a handle a press grabs it
EXTENT_AFTER_S = 600.0  # egui's timeline_extent: the sequence and ten minutes after it


def timeline_extent(sequence_seconds: float) -> float:
    return max(0.0, sequence_seconds) + EXTENT_AFTER_S


def zoom_thumb(left: float, width: float, seq_seconds: float, scroll: float, pps: float) -> tuple[float, float, float]:
    """egui's ``zoom_thumb``: the thumb's left edge and width on a bar from ``left`` ``width``
    wide, and the seconds of scroll per pixel of thumb travel (0 when it cannot travel)."""
    vis = width / max(pps, 1e-9)
    total = max(timeline_extent(seq_seconds), vis)
    w = min(max((vis / total) * width, min(MIN_THUMB_W, width)), width)
    travel = max(width - w, 0.0)
    rng = max(total - vis, 0.0)
    at = min(max(scroll / rng, 0.0), 1.0) * travel if rng > 0 else 0.0
    return left + at, w, rng / travel if travel > 0.5 else 0.0


class TimelineZoomBar(QWidget):
    """The zoom scroll bar under a :class:`TimelineCanvas`, lined up with its tracks."""

    def __init__(self, canvas: TimelineCanvas, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.canvas = canvas
        self.setObjectName("timeline.zoomScroll")
        self.setMouseTracking(True)
        self.setFixedHeight(ZOOM_BAR_H)
        self.setFocusPolicy(Qt.FocusPolicy.NoFocus)  # the keyboard is grabbed only while dragging (Escape)
        self._drag: dict[str, Any] | None = None

    def sizeHint(self) -> QSize:
        return QSize(200, ZOOM_BAR_H)

    # ----- geometry -----
    def bar_rect(self) -> QRectF:
        from .timeline import HEADER_W

        right = float(self.canvas.width())
        return QRectF(float(HEADER_W), 2.0, max(0.0, right - HEADER_W), ZOOM_BAR_H - 4.0)

    def _view(self) -> tuple[float, float, float, float]:
        """(sequence seconds, scroll seconds, pixels per second, seconds in view)."""
        c = self.canvas
        bar = self.bar_rect()
        return c.duration / TICKS_PER_SECOND, c.scroll_ticks / TICKS_PER_SECOND, c.pps, bar.width() / max(c.pps, 1e-9)

    def thumb_rect(self) -> QRectF:
        bar = self.bar_rect()
        seq_s, scroll, pps, _vis = self._view()
        left, w, _per = zoom_thumb(bar.left(), bar.width(), seq_s, scroll, pps)
        return QRectF(left, bar.top(), w, bar.height())

    def automation_parts(self) -> list[tuple[str, QRectF, str]]:
        if self.canvas.sequence is None or self.bar_rect().width() <= 0:
            return []
        return [("timeline.zoomBar", self.thumb_rect(), "zoom scroll bar"), ("timeline.zoomBar.track", self.bar_rect(), "zoom scroll bar track")]

    # ----- painting -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        T = theme.current()
        p = QPainter(self)
        p.fillRect(self.rect(), T.c("panel_bg"))
        if self.canvas.sequence is None:
            return
        bar = self.bar_rect()
        if bar.width() <= 0:
            return
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(T.c("separator"))
        p.drawRoundedRect(bar, bar.height() / 2, bar.height() / 2)
        thumb = self.thumb_rect()
        hover = self._drag is not None or thumb.contains(self.mapFromGlobal(self.cursor().pos()).toPointF())
        p.setBrush(T.c("scroll_thumb_hover" if hover else "scroll_thumb"))
        p.drawRoundedRect(thumb, thumb.height() / 2, thumb.height() / 2)
        for x in (thumb.left() + thumb.height() / 2, thumb.right() - thumb.height() / 2):
            p.setPen(QPen(T.c("text"), 1.5))
            p.setBrush(T.c("panel_bg"))
            p.drawEllipse(QPointF(x, thumb.center().y()), 4.5, 4.5)

    # ----- view changes -----
    def _set_view(self, scroll_s: float, pps: float | None = None) -> None:
        c = self.canvas
        if pps is not None:
            c.pps = min(4000.0, max(0.5, pps))
        c.scroll_ticks = max(0, round(scroll_s * TICKS_PER_SECOND))
        c.update()
        self.update()

    def _jump(self, x: float) -> float:
        """Bring the thumb's middle under ``x``; the new scroll in seconds."""
        bar = self.bar_rect()
        seq_s, scroll, pps, vis = self._view()
        _left, w, per = zoom_thumb(bar.left(), bar.width(), seq_s, scroll, pps)
        if per > 0:
            rng = max(max(timeline_extent(seq_s), vis) - vis, 0.0)
            scroll = min(max((x - bar.left() - w / 2) * per, 0.0), rng)
            self._set_view(scroll)
        return scroll

    # ----- mouse -----
    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton or self.canvas.sequence is None:
            return
        x = e.position().x()
        thumb = self.thumb_rect()
        _seq, scroll, pps, _vis = self._view()
        start = (scroll, pps)
        if not thumb.adjusted(0, -4, 0, 4).contains(e.position()):
            scroll = self._jump(x)
            mode = 0
        elif abs(x - thumb.left()) < HANDLE_GRAB:
            mode = 1
        elif abs(x - thumb.right()) < HANDLE_GRAB:
            mode = 2
        else:
            mode = 0
        self._drag = {"mode": mode, "grab": x, "last": x, "scroll": scroll, "undo": start}
        self.grabKeyboard()
        self.update()

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        d = self._drag
        x = e.position().x()
        if d is None:
            thumb = self.thumb_rect()
            near = thumb.contains(e.position()) and (abs(x - thumb.left()) < HANDLE_GRAB or abs(x - thumb.right()) < HANDLE_GRAB)
            self.setCursor(Qt.CursorShape.SizeHorCursor if near else Qt.CursorShape.ArrowCursor)
            self.update()
            return
        bar = self.bar_rect()
        seq_s, scroll, pps, vis = self._view()
        total = max(timeline_extent(seq_s), vis)
        if d["mode"] == 0:
            # the thumb's middle: scroll, the thumb staying under the pointer
            _left, _w, per = zoom_thumb(bar.left(), bar.width(), seq_s, scroll, pps)
            self._set_view(min(max(d["scroll"] + (x - d["grab"]) * per, 0.0), max(total - vis, 0.0)))
        else:
            # a handle: zoom, the other end of the view staying where it is
            dx = (x - d["last"]) / max(bar.width(), 1.0) * total
            if d["mode"] == 1:
                end = scroll + vis
                ns = min(max(scroll + dx, 0.0), max(end - 0.05, 0.0))
                self._set_view(ns, bar.width() / max(end - ns, 0.05))
            else:
                ne = max(scroll + vis + dx, scroll + 0.05)
                self._set_view(scroll, bar.width() / (ne - scroll))
        d["last"] = x

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self._end_drag()

    def _end_drag(self) -> None:
        if self._drag is not None:
            self.releaseKeyboard()
        self._drag = None
        self.update()

    def keyPressEvent(self, e: QKeyEvent) -> None:
        d = self._drag
        if d is not None and e.key() == Qt.Key.Key_Escape:
            self._end_drag()
            scroll, pps = d["undo"]
            self._set_view(scroll, pps)
            return
        super().keyPressEvent(e)
