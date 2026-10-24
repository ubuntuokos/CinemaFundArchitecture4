"""Timeline panel: draws the active sequence from ``sequence.inspect`` and edits it through
engine commands (``timeline.select``, ``timeline.move``, ``timeline.trim``, ``timeline.razor``,
``timeline.place``, ``playhead.set``). The widget keeps no copy of the edit truth: after every
command the engine reports ``projectChanged`` and the panel redraws from a fresh inspection.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QPoint, QRect, QRectF, QSize, Qt, Signal
from PySide6.QtGui import QColor, QFont, QFontMetrics, QMouseEvent, QPainter, QPaintEvent, QPen, QWheelEvent
from PySide6.QtWidgets import QHBoxLayout, QLabel, QScrollBar, QToolButton, QVBoxLayout, QWidget

from ..i18n import tr
from ..timecode import TICKS_PER_SECOND, format_timecode, frame_ticks

ITEM_MIME = "application/x-cfa4-item"

HEADER_W = 132
RULER_H = 26
VIDEO_H = 46
AUDIO_H = 40
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


@dataclass
class Row:
    track_id: Any
    name: str
    kind: str  # "video" | "audio"
    top: int
    height: int
    track: dict[str, Any]


@dataclass
class Drag:
    kind: str  # "move" | "trim" | "scrub"
    clip: dict[str, Any] | None
    origin: QPoint
    origin_time: int
    edge: str = ""
    delta: int = 0
    row_delta: int = 0


class TimelineCanvas(QWidget):
    command = Signal(str, dict)  # engine command id, params
    seek = Signal(object)  # ticks, while scrubbing (local preview); object, not a 32-bit int

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("timeline.canvas")
        self.setMouseTracking(True)
        self.setAcceptDrops(True)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setMinimumHeight(140)
        self.sequence: dict[str, Any] | None = None
        self.rows: list[Row] = []
        self.pps = 40.0  # pixels per second
        self.scroll_ticks = 0
        self.playhead = 0
        self.tool = "selection"
        self._drag: Drag | None = None
        self._drop_preview: tuple[int, Row] | None = None
        self.font_small = QFont(self.font())
        self.font_small.setPointSizeF(max(7.0, self.font().pointSizeF() * 0.85))

    # ----- model -----
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
        y = RULER_H
        for t in reversed(self.sequence.get("video") or []):  # highest video track on top
            self.rows.append(Row(t.get("id"), str(t.get("name") or ""), "video", y, VIDEO_H, t))
            y += VIDEO_H
        y += 4
        for t in self.sequence.get("audio") or []:
            self.rows.append(Row(t.get("id"), str(t.get("name") or ""), "audio", y, AUDIO_H, t))
            y += AUDIO_H

    def sizeHint(self) -> QSize:
        bottom = self.rows[-1].top + self.rows[-1].height if self.rows else 200
        return QSize(800, bottom + 8)

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
        p.fillRect(self.rect(), pal.base())
        if not self.sequence:
            p.setPen(pal.text().color())
            p.drawText(
                self.rect(),
                Qt.AlignmentFlag.AlignCenter,
                tr("No sequence open. Double-click a sequence in the Project panel."),
            )
            return
        self._paint_ruler(p)
        selected = set(self.selection)
        fm = QFontMetrics(self.font_small)
        p.setFont(self.font_small)
        for row in self.rows:
            band = QRect(HEADER_W, row.top, self.width() - HEADER_W, row.height)
            p.fillRect(band, pal.alternateBase() if row.kind == "video" else pal.base())
            p.setPen(QPen(pal.mid().color()))
            p.drawLine(HEADER_W, row.top + row.height - 1, self.width(), row.top + row.height - 1)
            for clip in row.track.get("items") or []:
                r = self.clip_rect(row, clip)
                if r.right() < HEADER_W or r.left() > self.width():
                    continue
                base = QColor(
                    LABEL_COLORS.get(
                        str(clip.get("label") or "").lower(), "#5b7fb8" if row.kind == "video" else "#4f9a6a"
                    )
                )
                if not clip.get("enabled", True):
                    base = base.darker(180)
                p.fillRect(r, base)
                if self._drag and self._drag.kind == "move" and clip.get("clip") in selected:
                    ghost = r.translated(
                        self._drag.delta * self.pps / TICKS_PER_SECOND, self._drag.row_delta * row.height
                    )
                    p.setPen(QPen(QColor("#ffffff"), 1, Qt.PenStyle.DashLine))
                    p.drawRect(ghost)
                pen = QPen(
                    QColor("#ffd54a") if clip.get("clip") in selected else base.darker(160),
                    2 if clip.get("clip") in selected else 1,
                )
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
        if self._drag and self._drag.kind == "trim" and self._drag.clip is not None:
            x = self.x_of(self._drag.origin_time + self._drag.delta)
            p.setPen(QPen(QColor("#ffd54a"), 2))
            p.drawLine(int(x), RULER_H, int(x), self.height())
        self._paint_headers(p)
        # playhead
        x = self.x_of(self.playhead)
        if x >= HEADER_W:
            p.setPen(QPen(QColor("#e8453c"), 1))
            p.drawLine(int(x), 0, int(x), self.height())
            p.setBrush(QColor("#e8453c"))
            p.drawPolygon([QPoint(int(x) - 6, 0), QPoint(int(x) + 6, 0), QPoint(int(x), 9)])

    def _paint_ruler(self, p: QPainter) -> None:
        pal = self.palette()
        p.fillRect(QRect(0, 0, self.width(), RULER_H), pal.window())
        p.setPen(pal.windowText().color())
        p.setFont(self.font_small)
        seconds_visible = (self.width() - HEADER_W) / self.pps
        steps = [1 / 30, 0.1, 0.5, 1, 2, 5, 10, 30, 60, 120, 300, 600, 1800, 3600]
        step = next((s for s in steps if s * self.pps >= 80), steps[-1])
        start = self.scroll_ticks / TICKS_PER_SECOND
        first = int(start / step)
        n = int(seconds_visible / step) + 2
        ft = self.frame_ticks()
        for i in range(first, first + n):
            t = int(i * step * TICKS_PER_SECOND)
            x = self.x_of(t)
            if x < HEADER_W:
                continue
            p.drawLine(int(x), RULER_H - 8, int(x), RULER_H)
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
                    QColor(90, 150, 255, 160),
                )

    def _paint_headers(self, p: QPainter) -> None:
        pal = self.palette()
        p.fillRect(QRect(0, 0, HEADER_W, self.height()), pal.window())
        p.setPen(pal.windowText().color())
        p.drawText(
            QRect(6, 0, HEADER_W - 6, RULER_H),
            Qt.AlignmentFlag.AlignVCenter,
            format_timecode(self.playhead, self.frame_ticks()),
        )
        for row in self.rows:
            r = QRect(0, row.top, HEADER_W - 1, row.height)
            p.fillRect(r.adjusted(2, 1, -2, -1), pal.button())
            flags = []
            if row.track.get("locked"):
                flags.append("🔒")
            if row.kind == "audio" and row.track.get("muted"):
                flags.append("M")
            if row.kind == "audio" and row.track.get("solo"):
                flags.append("S")
            if not row.track.get("enabled", True):
                flags.append("⊘")
            p.setPen(pal.buttonText().color())
            p.drawText(r.adjusted(8, 0, -4, 0), Qt.AlignmentFlag.AlignVCenter, f"{row.name}  {' '.join(flags)}")

    # ----- mouse -----
    def mousePressEvent(self, e: QMouseEvent) -> None:
        if not self.sequence or e.button() != Qt.MouseButton.LeftButton:
            return
        pos = e.position().toPoint()
        if pos.y() < RULER_H and pos.x() >= HEADER_W:
            self._drag = Drag("scrub", None, pos, self.playhead)
            self._scrub_to(pos.x())
            return
        _row, clip, edge = self.hit(pos)
        if clip is None:
            if pos.x() >= HEADER_W:
                self.command.emit("edit.deselectAll", {})
            return
        cid = clip.get("clip")
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
        if d is None:
            _row, clip, edge = self.hit(pos)
            if self.tool == "razor" and clip is not None:
                self.setCursor(Qt.CursorShape.IBeamCursor)
            elif edge:
                self.setCursor(Qt.CursorShape.SizeHorCursor)
            else:
                self.unsetCursor()
            return
        if d.kind == "scrub":
            self._scrub_to(pos.x())
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
        if d.kind == "trim":
            mode = "ripple" if e.modifiers() & Qt.KeyboardModifier.ControlModifier else "regular"
            self.command.emit(
                "timeline.trim", {"clip": d.clip.get("clip"), "edge": d.edge, "mode": mode, "delta": d.delta}
            )
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
        else:
            self.scroll_ticks = max(0, int(self.scroll_ticks - dy / 120 * 80 * TICKS_PER_SECOND / self.pps))
            self.update()
        e.accept()

    # ----- drag and drop from the Project panel -----
    def dragEnterEvent(self, e) -> None:
        if e.mimeData().hasFormat(ITEM_MIME) and self.sequence:
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
            items = [int(x) for x in bytes(e.mimeData().data(ITEM_MIME)).decode().split(",") if x]
        except ValueError:
            return
        t, row = preview
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
        self.tool_buttons: dict[str, QToolButton] = {}
        for tool, label, key in (("selection", "Selection Tool", "V"), ("razor", "Razor Tool", "C")):
            b = QToolButton(self)
            b.setObjectName(f"tools.{tool}")
            b.setText(f"{tr(label)} ({key})")
            b.setCheckable(True)
            b.setChecked(tool == "selection")
            b.clicked.connect(lambda _c=False, t=tool: self.set_tool(t))
            bar.addWidget(b)
            self.tool_buttons[tool] = b
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
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addLayout(bar)
        layout.addWidget(self.canvas, 1)
        layout.addWidget(self.hscroll)

    def set_tool(self, tool: str) -> None:
        self.canvas.tool = tool
        for t, b in self.tool_buttons.items():
            b.setChecked(t == tool)

    def set_sequence(self, seq: dict[str, Any] | None) -> None:
        self.canvas.set_sequence(seq)
        self.title.setText(f"{tr('Timeline')}: {seq.get('name')}" if seq else tr("Timeline"))
        self._sync_scroll()

    def _sync_scroll(self) -> None:
        seconds = max(10, int(self.canvas.duration / TICKS_PER_SECOND) + 10)
        self.hscroll.blockSignals(True)
        self.hscroll.setRange(0, seconds * 10)
        self.hscroll.setValue(int(self.canvas.scroll_ticks / TICKS_PER_SECOND * 10))
        self.hscroll.blockSignals(False)

    def _on_scroll(self, v: int) -> None:
        self.canvas.scroll_ticks = int(v / 10 * TICKS_PER_SECOND)
        self.canvas.update()
