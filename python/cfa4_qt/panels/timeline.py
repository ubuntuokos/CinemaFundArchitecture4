"""Timeline panel: draws the active sequence from ``sequence.inspect`` and edits it through
engine commands (``timeline.select``, ``timeline.move``, ``timeline.trim``, ``timeline.razor``,
``timeline.place``, ``playhead.set``). The widget keeps no copy of the edit truth: after every
command the engine reports ``projectChanged`` and the panel redraws from a fresh inspection.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QPoint, QRect, QRectF, QSize, Qt, Signal
from PySide6.QtGui import (
    QBrush,
    QColor,
    QFont,
    QFontMetrics,
    QMouseEvent,
    QPainter,
    QPaintEvent,
    QPen,
    QWheelEvent,
)
from PySide6.QtWidgets import QHBoxLayout, QLabel, QScrollBar, QToolButton, QVBoxLayout, QWidget

from .. import theme
from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND, format_timecode, frame_ticks
from .media_browser import FILES_MIME, dropped_files
from .timeline_media import TimelineMedia

ITEM_MIME = "application/x-cfa4-item"

HEADER_W = 200
RULER_H = 26
VIDEO_H = 56
AUDIO_H = 52
LABEL_H = 14  # the clip name strip above a thumbnail or waveform
EDGE_PX = 6
SNAP_PX = 8

# label names (engine colour labels) -> display colour
LABEL_COLORS = {
    "violet": "#8f6fd8",
    "iris": "#6f7fd8",
    "caribbean": "#2bb6a5",
    "lavender": "#b39ddb",
    "cerulean": "#3a8fd9",
    "forest": "#4f9a4a",
    "rose": "#d9658f",
    "mango": "#e39a3b",
    "purple": "#9b59b6",
    "blue": "#4a7fd0",
    "teal": "#2f9e9e",
    "magenta": "#c04fc0",
    "tan": "#b89b72",
    "green": "#5aaa55",
    "brown": "#8d6748",
    "yellow": "#d4bf3a",
}


def ruler_step(frame: int, pps: float, min_px: float) -> int:
    """Ruler label spacing in ticks, on whole timecode units so labels read 00:00:05:00,
    00:00:10:00 ... (or whole frames when zoomed in). A timecode second is the nominal frame count
    (24 at 23.976 fps), so steps are counted in frames, not in wall-clock seconds."""
    frame = max(1, int(frame))
    fps = max(1, round(TICKS_PER_SECOND / frame))
    sec = fps  # frames per timecode second
    # below a second: divisors of the frame count, so every label still falls on whole seconds
    sub = [d for d in range(1, sec) if sec % d == 0]
    whole = [m * sec for m in (1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600)]
    for frames in (*sub, *whole):
        if frames * frame * pps / TICKS_PER_SECOND >= min_px:
            return frames * frame
    return 3600 * sec * frame


@dataclass
class Row:
    track_id: Any
    name: str
    kind: str  # "video" | "audio"
    top: int
    height: int
    track: dict[str, Any]
    short: str = ""  # V1, A2 ... (the header's targeting label)


@dataclass
class HeaderCell:
    key: str  # patch | target | locked | syncLock | enabled | muted | solo
    rect: QRect
    text: str
    on: bool
    tip: str


# track header switches: (key, label, tooltip) per kind, left to right after patch and target
HEADER_SWITCHES = {
    "video": (("locked", "", N_("Toggle Track Lock")), ("syncLock", "⇄", N_("Toggle Sync Lock")), ("enabled", "", N_("Toggle Track Output"))),
    "audio": (("locked", "", N_("Toggle Track Lock")), ("syncLock", "⇄", N_("Toggle Sync Lock")), ("muted", "M", N_("Mute Track")), ("solo", "S", N_("Solo Track"))),
}
CELL = 20


@dataclass
class Drag:
    kind: str  # "move" | "trim" | "scrub" | "roll" | "stretch" | "slip" | "slide" | "hand"
    clip: dict[str, Any] | None
    origin: QPoint
    origin_time: int
    edge: str = ""
    delta: int = 0
    row_delta: int = 0
    mode: str = "regular"  # trim: regular | ripple
    partner: dict[str, Any] | None = None  # roll: the clip on the other side of the cut
    scroll: tuple[int, int] = (0, 0)  # hand: scroll position at the press


class TimelineCanvas(QWidget):
    command = Signal(str, dict)  # engine command id, params
    seek = Signal(object)  # ticks, while scrubbing (local preview); object, not a 32-bit int
    files_dropped = Signal(list, object, object)  # paths (Media Browser, file manager), time, track id

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("timeline.canvas")
        self.setMouseTracking(True)
        self.setAcceptDrops(True)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setMinimumHeight(140)
        self.sequence: dict[str, Any] | None = None
        self.rows: list[Row] = []
        self.media = TimelineMedia(self)
        self.media.updated.connect(self.update)
        self.pps = 40.0  # pixels per second
        self.scroll_ticks = 0
        self.scroll_y = 0  # pixels the track rows are scrolled up under the ruler
        self.playhead = 0
        self.tool = "selection"
        self._drag: Drag | None = None
        self._drop_preview: tuple[int, Row] | None = None
        self.font_small = QFont(self.font())
        self.font_small.setPointSizeF(max(7.0, self.font().pointSizeF() * 0.85))

    # ----- model -----
    scrolled_y = Signal()  # the vertical offset changed (the panel's scroll bar follows)

    def set_sequence(self, seq: dict[str, Any] | None) -> None:
        self.sequence = seq if isinstance(seq, dict) else None
        if self.sequence and isinstance(self.sequence.get("playhead"), int) and self._drag is None:
            self.playhead = self.sequence["playhead"]
        self._layout_rows()
        self.updateGeometry()
        self.update()

    def _layout_rows(self) -> None:
        self.rows = []
        if not self.sequence:
            return
        y = RULER_H - self.scroll_y
        videos = list(self.sequence.get("video") or [])
        for i in reversed(range(len(videos))):  # highest video track on top
            t = videos[i]
            self.rows.append(Row(t.get("id"), str(t.get("name") or ""), "video", y, VIDEO_H, t, f"V{i + 1}"))
            y += VIDEO_H
        y += 4
        for i, t in enumerate(self.sequence.get("audio") or []):
            self.rows.append(Row(t.get("id"), str(t.get("name") or ""), "audio", y, AUDIO_H, t, f"A{i + 1}"))
            y += AUDIO_H

    def content_height(self) -> int:
        """Height of every track row together (below the ruler), unscrolled."""
        if not self.rows:
            return 0
        return self.rows[-1].top + self.rows[-1].height + self.scroll_y - RULER_H + 8

    def max_scroll_y(self) -> int:
        return max(0, self.content_height() - max(0, self.height() - RULER_H))

    def set_scroll_y(self, y: int) -> None:
        y = max(0, min(int(y), self.max_scroll_y()))
        if y != self.scroll_y:
            self.scroll_y = y
            self._layout_rows()
            self.update()
            self.scrolled_y.emit()

    def sizeHint(self) -> QSize:
        return QSize(800, RULER_H + min(self.content_height(), 400))

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        # a taller panel may show everything: keep the offset within range
        self.set_scroll_y(self.scroll_y)
        self.scrolled_y.emit()

    @property
    def duration(self) -> int:
        return int(self.sequence.get("duration") or 0) if self.sequence else 0

    @property
    def selection(self) -> list[Any]:
        return list(self.sequence.get("selection") or []) if self.sequence else []

    def frame_ticks(self) -> int:
        return frame_ticks((self.sequence or {}).get("settings"))

    # ----- geometry -----
    def x_of(self, ticks: int) -> float:
        return HEADER_W + (ticks - self.scroll_ticks) * self.pps / TICKS_PER_SECOND

    def time_at(self, x: float) -> int:
        return max(0, int(self.scroll_ticks + (x - HEADER_W) * TICKS_PER_SECOND / self.pps))

    def row_at(self, y: int) -> Row | None:
        if y < RULER_H:  # rows scrolled up under the ruler are hidden there
            return None
        for r in self.rows:
            if r.top <= y < r.top + r.height:
                return r
        return None

    def clip_rect(self, row: Row, clip: dict[str, Any]) -> QRectF:
        x0 = self.x_of(int(clip.get("start") or 0))
        x1 = self.x_of(int(clip.get("end") or 0))
        return QRectF(x0, row.top + 2, max(2.0, x1 - x0), row.height - 4)

    def hit(self, pos: QPoint) -> tuple[Row | None, dict[str, Any] | None, str]:
        row = self.row_at(pos.y())
        if row is None or pos.x() < HEADER_W:
            return row, None, ""
        for clip in row.track.get("items") or []:
            r = self.clip_rect(row, clip)
            if r.left() - 1 <= pos.x() <= r.right() + 1:
                edge_px = min(EDGE_PX, r.width() / 3)
                if pos.x() - r.left() <= edge_px:
                    return row, clip, "in"
                if r.right() - pos.x() <= edge_px:
                    return row, clip, "out"
                return row, clip, ""
        return row, None, ""

    def neighbour(self, row: Row, clip: dict[str, Any], edge: str) -> dict[str, Any] | None:
        """The clip touching ``clip`` at ``edge`` on the same track (a roll needs both sides)."""
        for c in row.track.get("items") or []:
            if edge == "out" and int(c.get("start") or 0) == int(clip.get("end") or 0):
                return c
            if edge == "in" and int(c.get("end") or 0) == int(clip.get("start") or 0):
                return c
        return None

    def track_select(self, row: Row, t: int, forward: bool, single_track: bool) -> list[Any]:
        """Track Select Forward/Backward: every clip ending after (starting before) ``t``, on
        all tracks or (Shift) only the clicked one, as the egui editor does."""
        ids = []
        for r in self.rows:
            if single_track and r is not row:
                continue
            for c in r.track.get("items") or []:
                if (int(c.get("end") or 0) > t) if forward else (int(c.get("start") or 0) < t):
                    ids.append(c.get("clip"))
        return ids

    def snap(self, ticks: int, exclude: set[Any]) -> int:
        """Snap to the playhead and clip edges within SNAP_PX."""
        best, best_px = ticks, SNAP_PX + 1.0
        candidates = [self.playhead, 0]
        for r in self.rows:
            for c in r.track.get("items") or []:
                if c.get("clip") in exclude:
                    continue
                candidates += [int(c.get("start") or 0), int(c.get("end") or 0)]
        for c in candidates:
            px = abs(c - ticks) * self.pps / TICKS_PER_SECOND
            if px < best_px:
                best, best_px = c, px
        return best

    def zoom(self, factor: float, anchor_x: float | None = None) -> None:
        anchor_x = HEADER_W + 200 if anchor_x is None else anchor_x
        t = self.time_at(anchor_x)
        self.pps = min(4000.0, max(0.5, self.pps * factor))
        self.scroll_ticks = max(0, int(t - (anchor_x - HEADER_W) * TICKS_PER_SECOND / self.pps))
        self.update()

    def zoom_to_fit(self) -> None:
        width = max(100, self.width() - HEADER_W - 20)
        seconds = max(1.0, self.duration / TICKS_PER_SECOND)
        self.pps = min(4000.0, max(0.5, width / seconds))
        self.scroll_ticks = 0
        self.update()

    # ----- painting -----
    def paintEvent(self, _event: QPaintEvent) -> None:
        p = QPainter(self)
        pal = self.palette()
        T = theme.current()
        p.fillRect(self.rect(), T.c("tl_bg"))
        if not self.sequence:
            p.setPen(pal.text().color())
            p.drawText(
                self.rect(),
                Qt.AlignmentFlag.AlignCenter,
                tr("No sequence open. Double-click a sequence in the Project panel."),
            )
            return
        p.save()
        p.setClipRect(QRect(0, RULER_H, self.width(), max(0, self.height() - RULER_H)))
        selected = set(self.selection)
        fm = QFontMetrics(self.font_small)
        p.setFont(self.font_small)
        for row in self.rows:
            band = QRect(HEADER_W, row.top, self.width() - HEADER_W, row.height)
            p.fillRect(band, T.c("tl_track_bg_alt") if row.kind == "video" else T.c("tl_track_bg"))
            p.setPen(QPen(T.c("separator")))
            p.drawLine(HEADER_W, row.top + row.height - 1, self.width(), row.top + row.height - 1)
            if row.track.get("locked"):
                p.fillRect(band, QBrush(QColor(128, 128, 128, 90), Qt.BrushStyle.BDiagPattern))
            dim = row.kind == "video" and not row.track.get("enabled", True) or row.kind == "audio" and row.track.get("muted")
            for clip in row.track.get("items") or []:
                r = self.clip_rect(row, clip)
                if r.right() < HEADER_W or r.left() > self.width():
                    continue
                base = QColor(
                    LABEL_COLORS.get(
                        str(clip.get("label") or "").lower(), "#5b7fb8" if row.kind == "video" else "#4f9a6a"
                    )
                )
                if not clip.get("enabled", True) or dim:
                    base = base.darker(180)
                p.fillRect(r, base)
                if self._drag and self._drag.kind == "move" and clip.get("clip") in selected:
                    ghost = r.translated(
                        self._drag.delta * self.pps / TICKS_PER_SECOND, self._drag.row_delta * row.height
                    )
                    p.setPen(QPen(QColor("#ffffff"), 1, Qt.PenStyle.DashLine))
                    p.drawRect(ghost)
                pen = QPen(
                    T.c("clip_selected_border") if clip.get("clip") in selected else base.darker(160),
                    2 if clip.get("clip") in selected else 1,
                )
                self._paint_clip_media(p, row, clip, r, base)
                p.setPen(pen)
                p.drawRect(r.adjusted(0.5, 0.5, -0.5, -0.5))
                text = str(clip.get("name") or "")
                if clip.get("effects"):
                    text = "fx " + text
                p.setPen(QColor("#ffffff"))
                clip_text = fm.elidedText(text, Qt.TextElideMode.ElideRight, int(r.width()) - 8)
                p.drawText(r.adjusted(4, 2, -4, -2), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop, clip_text)
        if self._drop_preview:
            t, row = self._drop_preview
            x = self.x_of(t)
            p.fillRect(QRectF(x, row.top, 3, row.height), QColor("#ffd54a"))
        if self._drag and self._drag.kind in ("trim", "roll", "stretch") and self._drag.clip is not None:
            x = self.x_of(self._drag.origin_time + self._drag.delta)
            p.setPen(QPen(QColor("#ffd54a"), 2))
            p.drawLine(int(x), RULER_H, int(x), self.height())
        self._paint_headers(p)
        p.restore()
        self._paint_ruler(p)
        self._paint_corner(p)
        # playhead
        x = self.x_of(self.playhead)
        if x >= HEADER_W:
            p.setPen(QPen(T.c("playhead"), 1))
            p.drawLine(int(x), 0, int(x), self.height())
            p.setBrush(T.c("playhead"))
            p.drawPolygon([QPoint(int(x) - 6, 0), QPoint(int(x) + 6, 0), QPoint(int(x), 9)])

    def _paint_ruler(self, p: QPainter) -> None:
        T = theme.current()
        p.fillRect(QRect(0, 0, self.width(), RULER_H), T.c("tl_ruler_bg"))
        p.setPen(T.c("tl_ruler_text"))
        p.setFont(self.font_small)
        ft = self.frame_ticks()
        label_w = QFontMetrics(self.font_small).horizontalAdvance("00:00:00:00") + 16
        step = ruler_step(ft, self.pps, label_w)
        first = self.scroll_ticks // step
        n = int((self.width() - HEADER_W) / max(1e-9, step * self.pps / TICKS_PER_SECOND)) + 2
        for i in range(first, first + n):
            t = i * step
            x = self.x_of(t)
            if x < HEADER_W:
                continue
            p.setPen(T.c("tl_ruler_tick"))
            p.drawLine(int(x), RULER_H - 8, int(x), RULER_H)
            p.setPen(T.c("tl_ruler_text"))
            p.drawText(int(x) + 3, RULER_H - 10, format_timecode(t, ft))
        for marker in self.sequence.get("markers") or [] if self.sequence else []:
            x = self.x_of(int(marker.get("start") or 0))
            if x >= HEADER_W:
                p.fillRect(QRectF(x - 3, RULER_H - 7, 7, 7), QColor("#4caf50"))
        # in / out
        if self.sequence:
            mi, mo = self.sequence.get("in"), self.sequence.get("out")
            if isinstance(mi, int) or isinstance(mo, int):
                x0 = self.x_of(mi if isinstance(mi, int) else 0)
                x1 = self.x_of(mo if isinstance(mo, int) else self.duration)
                p.fillRect(
                    QRectF(max(x0, HEADER_W), RULER_H - 4, max(0.0, x1 - max(x0, HEADER_W)), 4),
                    T.c("in_out_shade"),
                )

    def _paint_corner(self, p: QPainter) -> None:
        T = theme.current()
        p.fillRect(QRect(0, 0, HEADER_W, RULER_H), T.c("tl_header_bg"))
        p.setPen(T.c("timecode"))
        p.setFont(self.font_small)
        p.drawText(
            QRect(6, 0, HEADER_W - 6, RULER_H),
            Qt.AlignmentFlag.AlignVCenter,
            format_timecode(self.playhead, self.frame_ticks()),
        )

    def header_cells(self, row: Row) -> list[HeaderCell]:
        """The switches of one track header, left to right: source patch, targeting, then lock,
        sync lock and output (video) or mute and solo (audio)."""
        t = (self.sequence or {}).get("targeting") or {}
        dest = t.get("videoDest") if row.kind == "video" else t.get("audioDest")
        y = row.top + (row.height - CELL) // 2
        cells = [
            HeaderCell("patch", QRect(4, y, CELL + 4, CELL), row.short if dest == row.track_id else "", dest == row.track_id, tr("Source patch")),
            HeaderCell("target", QRect(30, y, CELL + 4, CELL), row.short, row.track_id in (t.get("targeted") or []), tr("Toggle track targeting")),
        ]
        x = 58
        for key, text, tip in HEADER_SWITCHES[row.kind]:
            value = row.track.get(key)
            on = bool(value) if key != "enabled" else value is not False
            cells.append(HeaderCell(key, QRect(x, y, CELL, CELL), text, on, tr(tip)))
            x += CELL + 2
        return cells

    def _paint_headers(self, p: QPainter) -> None:
        pal = self.palette()
        T = theme.current()
        p.fillRect(QRect(0, 0, HEADER_W, self.height()), T.c("tl_header_bg"))
        accent = T.c("track_target")
        for row in self.rows:
            r = QRect(0, row.top, HEADER_W - 1, row.height)
            p.fillRect(r.adjusted(2, 1, -2, -1), pal.button())
            cells = self.header_cells(row)
            for c in cells:
                if c.key in ("patch", "target"):
                    if c.on:
                        p.fillRect(c.rect, accent)
                        p.setPen(QColor("#ffffff"))
                    else:
                        p.setPen(QPen(pal.mid().color()))
                        p.drawRect(c.rect.adjusted(0, 0, -1, -1))
                        p.setPen(pal.buttonText().color())
                else:
                    warn = c.key in ("locked", "muted", "solo") and c.on or c.key == "enabled" and not c.on
                    if warn:
                        p.fillRect(c.rect, QColor("#c8892b") if c.key != "solo" else QColor("#c8b32b"))
                    p.setPen(QColor("#ffffff") if warn else (pal.buttonText().color() if c.on else pal.mid().color()))
                if c.key == "locked":
                    self._draw_lock(p, c.rect)
                elif c.key == "enabled":
                    self._draw_eye(p, c.rect, c.on)
                else:
                    p.drawText(c.rect, Qt.AlignmentFlag.AlignCenter, c.text)
            name_x = cells[-1].rect.right() + 6
            p.setPen(pal.buttonText().color())
            p.drawText(
                QRect(name_x, row.top, HEADER_W - name_x - 4, row.height),
                Qt.AlignmentFlag.AlignVCenter,
                QFontMetrics(p.font()).elidedText(row.name, Qt.TextElideMode.ElideRight, HEADER_W - name_x - 4),
            )

    @staticmethod
    def _draw_lock(p: QPainter, r: QRect) -> None:
        """A padlock in the current pen colour (no emoji: those keep their own colours)."""
        p.save()
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        c = r.center()
        body = QRectF(c.x() - 5, c.y() - 1, 10, 7)
        p.drawRect(body)
        p.drawArc(QRectF(c.x() - 3.5, c.y() - 7, 7, 9), 0, 180 * 16)
        p.restore()

    @staticmethod
    def _draw_eye(p: QPainter, r: QRect, on: bool) -> None:
        p.save()
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        c = r.center()
        p.drawEllipse(QRectF(c.x() - 7, c.y() - 4, 14, 8))
        p.drawEllipse(QRectF(c.x() - 2, c.y() - 2, 4, 4))
        if not on:
            p.drawLine(c.x() - 7, c.y() + 5, c.x() + 7, c.y() - 5)
        p.restore()

    def header_hit(self, pos: QPoint) -> tuple[Row | None, HeaderCell | None]:
        row = self.row_at(pos.y()) if pos.x() < HEADER_W else None
        if row is None:
            return None, None
        return row, next((c for c in self.header_cells(row) if c.rect.contains(pos)), None)

    def _press_header(self, row: Row, cell: HeaderCell) -> None:
        if cell.key == "patch":
            self.command.emit("timeline.setTargeting", {"track": row.track_id, "sourcePatch": not cell.on})
        elif cell.key == "target":
            self.command.emit("timeline.setTargeting", {"track": row.track_id, "targeted": not cell.on})
        else:
            self.command.emit("timeline.setTrack", {"track": row.track_id, cell.key: not cell.on})

    def _paint_clip_media(self, p: QPainter, row: Row, clip: dict[str, Any], r: QRectF, base: QColor) -> None:
        """A thumbnail at the head of a video clip, the waveform across an audio clip."""
        body = QRectF(r.left() + 1, r.top() + LABEL_H, r.width() - 2, r.height() - LABEL_H - 1)
        if body.width() < 6 or body.height() < 8:
            return
        item = clip.get("item")
        if row.kind == "video":
            img = self.media.thumbnail(item, int(clip.get("sourceIn") or 0), int(body.height()))
            if img is not None and not img.isNull():
                w = min(body.width(), body.height() * img.width() / max(1, img.height()))
                target = QRectF(body.left(), body.top(), w, body.height())
                p.save()
                p.setClipRect(body.intersected(QRectF(HEADER_W, 0, self.width(), self.height())))
                p.drawImage(target, img)
                p.restore()
            return
        start, end = int(clip.get("sourceIn") or 0), int(clip.get("sourceOut") or 0)
        peaks = self.media.waveform(item, min(start, end), max(start, end))
        if not peaks:
            return
        x0 = max(int(body.left()), HEADER_W)
        x1 = min(int(body.right()), self.width())
        if x1 <= x0:
            return
        mid = body.center().y()
        half = body.height() / 2 - 1
        n = len(peaks)
        reverse = bool(clip.get("reverse")) or end < start
        p.setPen(QPen(base.lighter(170), 1))
        for x in range(x0, x1):
            f = (x - body.left()) / max(1.0, body.width())
            i = min(n - 1, max(0, int((1.0 - f if reverse else f) * n)))
            lo, hi = peaks[i]
            p.drawLine(x, int(mid - max(0.0, min(1.0, hi)) * half), x, int(mid - max(-1.0, min(0.0, lo)) * half))

    # ----- mouse -----
    def mousePressEvent(self, e: QMouseEvent) -> None:
        if not self.sequence or e.button() != Qt.MouseButton.LeftButton:
            return
        pos = e.position().toPoint()
        if pos.x() < HEADER_W and pos.y() >= RULER_H:
            row, cell = self.header_hit(pos)
            if row is not None and cell is not None:
                self._press_header(row, cell)
            return
        if pos.y() < RULER_H and pos.x() >= HEADER_W:
            self._drag = Drag("scrub", None, pos, self.playhead)
            self._scrub_to(pos.x())
            return
        mods = e.modifiers()
        if self.tool == "hand":
            self._drag = Drag("hand", None, pos, 0, scroll=(self.scroll_ticks, self.scroll_y))
            self.setCursor(Qt.CursorShape.ClosedHandCursor)
            return
        if self.tool == "zoom":
            self.zoom(1 / 1.5 if mods & Qt.KeyboardModifier.AltModifier else 1.5, pos.x())
            return
        row, clip, edge = self.hit(pos)
        if self.tool in ("trackSelectForward", "trackSelectBackward") and row is not None:
            t = self.time_at(pos.x())
            ids = self.track_select(row, t, self.tool == "trackSelectForward", bool(mods & Qt.KeyboardModifier.ShiftModifier))
            self.command.emit("timeline.select", {"clips": ids})
            if self.sequence is not None:
                self.sequence["selection"] = ids
            if clip is not None and ids:  # as in Premiere, a drag moves what was just selected
                self._drag = Drag("move", clip, pos, int(clip.get("start") or 0))
            return
        if clip is None:
            if pos.x() >= HEADER_W:
                self.command.emit("edit.deselectAll", {})
            return
        cid = clip.get("clip")
        if self.tool in ("slip", "slide"):
            self.command.emit("timeline.select", {"clips": [cid]})
            self._drag = Drag(self.tool, clip, pos, int(clip.get("start") or 0))
            return
        if self.tool in ("ripple", "rolling", "rateStretch"):
            if not edge:  # these tools work on edges; a click on the body selects
                self.command.emit("timeline.select", {"clips": [cid]})
                return
            origin = int(clip.get("start") if edge == "in" else clip.get("end") or 0)
            if self.tool == "rateStretch":
                self._drag = Drag("stretch", clip, pos, origin, edge=edge)
            elif self.tool == "rolling" and row is not None and self.neighbour(row, clip, edge) is not None:
                self._drag = Drag("roll", clip, pos, origin, edge=edge, partner=self.neighbour(row, clip, edge))
            else:  # ripple, or a roll at a cut with nothing on the other side: a trim
                self._drag = Drag("trim", clip, pos, origin, edge=edge, mode="ripple" if self.tool == "ripple" else "regular")
            return
        if self.tool == "razor":
            t = self.time_at(pos.x())
            if not e.modifiers() & Qt.KeyboardModifier.ShiftModifier:
                t = self.snap(t, set())
            self.command.emit("timeline.razor", {"time": t, "clip": cid})
            return
        additive = bool(e.modifiers() & (Qt.KeyboardModifier.ShiftModifier | Qt.KeyboardModifier.ControlModifier))
        sel = self.selection
        if additive:
            new = [c for c in sel if c != cid] if cid in sel else [*sel, cid]
            self.command.emit("timeline.select", {"clips": new})
            return
        if cid not in sel:
            self.command.emit("timeline.select", {"clips": [cid]})
            if self.sequence is not None:
                self.sequence["selection"] = [cid]  # local echo until the engine's reply
        if edge:
            origin = int(clip.get("start") if edge == "in" else clip.get("end") or 0)
            self._drag = Drag("trim", clip, pos, origin, edge=edge)
        else:
            self._drag = Drag("move", clip, pos, int(clip.get("start") or 0))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        pos = e.position().toPoint()
        d = self._drag
        if d is None and pos.x() < HEADER_W:
            _row, cell = self.header_hit(pos)
            self.setToolTip(cell.tip if cell is not None else "")
            self.unsetCursor()
            return
        if d is None:
            _row, clip, edge = self.hit(pos)
            if self.tool == "hand":
                self.setCursor(Qt.CursorShape.OpenHandCursor)
            elif self.tool == "zoom":
                self.setCursor(Qt.CursorShape.CrossCursor)
            elif self.tool == "razor" and clip is not None:
                self.setCursor(Qt.CursorShape.IBeamCursor)
            elif self.tool in ("slip", "slide") and clip is not None:
                self.setCursor(Qt.CursorShape.SizeHorCursor)
            elif edge and self.tool in ("selection", "ripple", "rolling", "rateStretch"):
                self.setCursor(Qt.CursorShape.SplitHCursor if self.tool == "rolling" else Qt.CursorShape.SizeHorCursor)
            else:
                self.unsetCursor()
            return
        if d.kind == "scrub":
            self._scrub_to(pos.x())
            return
        if d.kind == "hand":
            dx, dy = pos.x() - d.origin.x(), pos.y() - d.origin.y()
            self.scroll_ticks = max(0, int(d.scroll[0] - dx * TICKS_PER_SECOND / self.pps))
            self.set_scroll_y(d.scroll[1] - dy)
            self.update()
            return
        if d.kind in ("slip", "slide"):
            ft = self.frame_ticks()
            raw = round((pos.x() - d.origin.x()) * TICKS_PER_SECOND / self.pps / ft) * ft
            d.delta = -raw if d.kind == "slip" else raw  # slip moves the media, not the clip
            self.update()
            return
        raw = int((pos.x() - d.origin.x()) * TICKS_PER_SECOND / self.pps)
        target = d.origin_time + raw
        if not e.modifiers() & Qt.KeyboardModifier.ShiftModifier:
            exclude = set(self.selection) if d.kind == "move" else {d.clip.get("clip") if d.clip else None}
            target = self.snap(target, exclude)
            if d.kind == "move" and d.clip is not None:
                # snap the clip's end as well
                length = int(d.clip.get("end") or 0) - int(d.clip.get("start") or 0)
                end_snap = self.snap(target + length, exclude) - length
                if abs(end_snap - (d.origin_time + raw)) < abs(target - (d.origin_time + raw)):
                    target = end_snap
        if d.kind == "move":
            target = max(0, target)
            start_row = self.row_at(d.origin.y())
            now_row = self.row_at(pos.y())
            d.row_delta = 0
            if start_row and now_row and start_row.kind == now_row.kind:
                d.row_delta = self.rows.index(now_row) - self.rows.index(start_row)
        d.delta = target - d.origin_time
        self.update()

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        d, self._drag = self._drag, None
        if d is None or e.button() != Qt.MouseButton.LeftButton:
            return
        if d.kind == "scrub":
            self.command.emit("playhead.set", {"time": self.playhead})
            return
        if d.clip is None or (d.delta == 0 and d.row_delta == 0):
            self.update()
            return
        if d.kind == "hand":
            self.setCursor(Qt.CursorShape.OpenHandCursor)
            return
        if d.kind == "trim":
            mode = "ripple" if d.mode == "ripple" or e.modifiers() & Qt.KeyboardModifier.ControlModifier else "regular"
            self.command.emit(
                "timeline.trim", {"clip": d.clip.get("clip"), "edge": d.edge, "mode": mode, "delta": d.delta}
            )
            return
        if d.kind == "roll" and d.partner is not None:
            left, right = (d.clip, d.partner) if d.edge == "out" else (d.partner, d.clip)
            self.command.emit("timeline.roll", {"left": left.get("clip"), "right": right.get("clip"), "delta": d.delta})
            return
        if d.kind == "stretch":
            self.command.emit("timeline.rateStretch", {"clip": d.clip.get("clip"), "edge": d.edge, "delta": d.delta})
            return
        if d.kind in ("slip", "slide"):
            self.command.emit(f"timeline.{d.kind}", {"clip": d.clip.get("clip"), "delta": d.delta})
            return
        moves = []
        by_clip = {c.get("clip"): (row, c) for row in self.rows for c in row.track.get("items") or []}
        for cid in self.selection or [d.clip.get("clip")]:
            found = by_clip.get(cid)
            if not found:
                continue
            row, clip = found
            idx = self.rows.index(row) + d.row_delta
            target_row = self.rows[idx] if 0 <= idx < len(self.rows) and self.rows[idx].kind == row.kind else row
            moves.append(
                {"clip": cid, "track": target_row.track_id, "time": max(0, int(clip.get("start") or 0) + d.delta)}
            )
        insert = bool(e.modifiers() & Qt.KeyboardModifier.ControlModifier)
        copy = bool(e.modifiers() & Qt.KeyboardModifier.AltModifier)
        params: dict[str, Any] = {"moves": moves, "linked": False}
        if insert:
            params["insert"] = True
        if copy:
            params["copy"] = True
        self.command.emit("timeline.move", params)

    def mouseDoubleClickEvent(self, e: QMouseEvent) -> None:
        _row, clip, _edge = self.hit(e.position().toPoint())
        if clip is not None and clip.get("item") is not None:
            self.command.emit("source.open", {"item": clip.get("item")})

    def _scrub_to(self, x: float) -> None:
        t = min(self.time_at(x), max(self.duration, 0)) if self.duration else self.time_at(x)
        ft = self.frame_ticks()
        t = (t // ft) * ft
        if t != self.playhead:
            self.playhead = t
            self.seek.emit(t)
            self.update()

    def wheelEvent(self, e: QWheelEvent) -> None:
        dy = e.angleDelta().y() or e.angleDelta().x()
        if e.modifiers() & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.AltModifier):
            self.zoom(1.25 if dy > 0 else 0.8, e.position().x())
        elif e.modifiers() & Qt.KeyboardModifier.ShiftModifier or (
            e.position().x() < HEADER_W and e.angleDelta().y()
        ):
            # Shift+wheel, or the wheel over the track headers: scroll the tracks up and down
            self.set_scroll_y(self.scroll_y - int(dy / 120 * 40))
        else:
            self.scroll_ticks = max(0, int(self.scroll_ticks - dy / 120 * 80 * TICKS_PER_SECOND / self.pps))
            self.update()
        e.accept()

    # ----- drag and drop from the Project panel -----
    def dragEnterEvent(self, e) -> None:
        if (e.mimeData().hasFormat(ITEM_MIME) or e.mimeData().hasFormat(FILES_MIME) or e.mimeData().hasUrls()) and self.sequence:
            e.acceptProposedAction()

    def dragMoveEvent(self, e) -> None:
        pos = e.position().toPoint()
        row = self.row_at(pos.y())
        if row is None or pos.x() < HEADER_W:
            self._drop_preview = None
            e.ignore()
        else:
            self._drop_preview = (self.snap(self.time_at(pos.x()), set()), row)
            e.acceptProposedAction()
        self.update()

    def dragLeaveEvent(self, _e) -> None:
        self._drop_preview = None
        self.update()

    def dropEvent(self, e) -> None:
        preview, self._drop_preview = self._drop_preview, None
        self.update()
        if preview is None:
            return
        try:
            items = [int(x) for x in bytes(e.mimeData().data(ITEM_MIME)).decode().split(",") if x] if e.mimeData().hasFormat(ITEM_MIME) else []
        except ValueError:
            return
        t, row = preview
        if not e.mimeData().hasFormat(ITEM_MIME):
            paths = dropped_files(e.mimeData())
            if paths:
                self.files_dropped.emit(paths, t, row.track_id)
                e.acceptProposedAction()
            return
        for item in items:
            self.command.emit("timeline.place", {"item": item, "time": t, "track": row.track_id})
        e.acceptProposedAction()


class TimelinePanel(QWidget):
    """Toolbar (tools, zoom) + canvas + horizontal scroll bar."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Timeline")
        self.canvas = TimelineCanvas(self)
        bar = QHBoxLayout()
        bar.setContentsMargins(4, 2, 4, 2)
        self.title = QLabel(tr("Timeline"))
        bar.addWidget(self.title)
        bar.addStretch(1)
        for text, slot, name in (
            ("−", lambda: self.canvas.zoom(0.8), "zoomOut"),
            ("+", lambda: self.canvas.zoom(1.25), "zoomIn"),
            (tr("Fit"), self.canvas.zoom_to_fit, "fit"),
        ):
            b = QToolButton(self)
            b.setObjectName(f"timeline.{name}")
            b.setText(text)
            b.clicked.connect(slot)
            bar.addWidget(b)
        self.hscroll = QScrollBar(Qt.Orientation.Horizontal, self)
        self.hscroll.valueChanged.connect(self._on_scroll)
        self.vscroll = QScrollBar(Qt.Orientation.Vertical, self)
        self.vscroll.setObjectName("timeline.vscroll")
        self.vscroll.valueChanged.connect(self.canvas.set_scroll_y)
        self.canvas.scrolled_y.connect(self._sync_vscroll)
        middle = QHBoxLayout()
        middle.setContentsMargins(0, 0, 0, 0)
        middle.setSpacing(0)
        middle.addWidget(self.canvas, 1)
        middle.addWidget(self.vscroll)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addLayout(bar)
        layout.addLayout(middle, 1)
        layout.addWidget(self.hscroll)

    def set_tool(self, tool: str) -> None:
        """The Tools panel picked ``tool`` (it decides what a drag on the timeline does)."""
        self.canvas.tool = tool
        self.canvas.unsetCursor()

    def set_sequence(self, seq: dict[str, Any] | None) -> None:
        self.canvas.set_sequence(seq)
        self.title.setText(f"{tr('Timeline')}: {seq.get('name')}" if seq else tr("Timeline"))
        self._sync_scroll()
        self.canvas.set_scroll_y(self.canvas.scroll_y)  # tracks added or removed
        self._sync_vscroll()

    def _sync_vscroll(self) -> None:
        top = self.canvas.max_scroll_y()
        self.vscroll.blockSignals(True)
        self.vscroll.setRange(0, top)
        self.vscroll.setPageStep(max(1, self.canvas.height() - RULER_H))
        self.vscroll.setSingleStep(20)
        self.vscroll.setValue(self.canvas.scroll_y)
        self.vscroll.blockSignals(False)
        self.vscroll.setVisible(top > 0)

    def _sync_scroll(self) -> None:
        seconds = max(10, int(self.canvas.duration / TICKS_PER_SECOND) + 10)
        self.hscroll.blockSignals(True)
        self.hscroll.setRange(0, seconds * 10)
        self.hscroll.setValue(int(self.canvas.scroll_ticks / TICKS_PER_SECOND * 10))
        self.hscroll.blockSignals(False)

    def _on_scroll(self, v: int) -> None:
        self.canvas.scroll_ticks = int(v / 10 * TICKS_PER_SECOND)
        self.canvas.update()
