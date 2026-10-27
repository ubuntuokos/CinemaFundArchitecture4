"""Timeline panel: draws the active sequence from ``sequence.inspect`` and edits it through
engine commands (``timeline.select``, ``timeline.move``, ``timeline.trim``, ``timeline.razor``,
``timeline.place``, ``playhead.set``). The widget keeps no copy of the edit truth: after every
command the engine reports ``projectChanged`` and the panel redraws from a fresh inspection.

Transitions are drawn as egui's: hatched over the lower part of the row, the name on a video one,
crossing fade curves on an audio one, outlined when selected. A click selects one (Shift adds or
removes), dragging an end changes its duration from that end and dragging its middle slides it
over the cut (``sequence.setTransition`` ``drag``, one undo step), a double-click opens Set
Transition Duration.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QPoint, QPointF, QRect, QRectF, QSize, Qt, Signal
from PySide6.QtGui import (
    QBrush,
    QColor,
    QFont,
    QFontMetrics,
    QMouseEvent,
    QPainter,
    QPainterPath,
    QPaintEvent,
    QPen,
    QWheelEvent,
)
from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMenu,
    QScrollBar,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..calc import evaluate
from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND, drop_frame, format_timecode, frame_ticks, parse_timecode
from . import timeline_menus
from .effects import EFFECT_MIME, dropped_effect
from .essential_graphics import TEMPLATE_MIME, dropped_template
from .info import marker_rgb
from .media_browser import FILES_MIME, dropped_files
from .monitor_view import SOURCE_MIME, source_payload
from .timeline_media import TimelineMedia

ITEM_MIME = "application/x-cfa4-item"
# egui's trim bracket colours (trim_monitor.rs)
RIPPLE_RED = QColor(0xE5, 0x3E, 0x3E)
ROLL_YELLOW = QColor(0xF2, 0xC4, 0x3D)

HEADER_W = 200
RULER_H = 26
VIDEO_H = 60  # egui's default track heights (state.rs)
AUDIO_H = 56
DRAG_MIN_H, DRAG_MAX_H = 22, 220  # dragging a header edge (timeline.rs)
KEY_MIN_H, KEY_MAX_H = 24, 240  # Increase / Decrease … Tracks Height (keyboard.rs)
HEIGHT_STEP = 12
EXPANDED_H = 120  # Expand All Tracks
LABEL_H = 14  # the clip name strip above a thumbnail or waveform
EDGE_PX = 6
SNAP_PX = 8
TRANSITION_TOP = 17  # a transition covers its row under the clips' name strip (egui's)
TRANSITION_EDGE_PX = 7  # how far into a transition from either end a press grabs that end

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
VOICEOVER_RED = "#d8503f"


@dataclass
class Drag:
    kind: str  # "move" | "trim" | "scrub" | "roll" | "stretch" | "remix" | "slip" | "slide" | "hand" | "height"
    clip: dict[str, Any] | None
    origin: QPoint
    origin_time: int
    edge: str = ""
    delta: int = 0
    row_delta: int = 0
    mode: str = "regular"  # trim: regular | ripple
    partner: dict[str, Any] | None = None  # roll: the clip on the other side of the cut
    scroll: tuple[int, int] = (0, 0)  # hand: scroll position at the press
    transition: dict[str, Any] | None = None  # transition: the one dragged, as it was at the press
    begun: bool = False  # transition: its first change went (the rest merge into that undo step)
    track_kind: str = ""  # height: "video" | "audio"


def as_int(v: Any, default: int = 0) -> int:
    return v if isinstance(v, int) and not isinstance(v, bool) else default


def duration_frames(text: str, current: int, frame: int, drop: bool) -> int | None:
    """A typed transition duration in frames, as egui's field reads one: a whole number is
    frames, otherwise timecode (``1.00`` is a second), otherwise a calculation in frames
    (``48/2``)."""
    t = text.strip()
    if t.isascii() and t.isdecimal():
        return int(t)
    tc = parse_timecode(t, frame, drop)
    if tc is not None:
        return tc
    v = evaluate(t, float(current))
    return None if v is None else round(v)


class TransitionDurationDialog(QDialog):
    """Set Transition Duration (double-clicking a transition): the duration as timecode; OK
    applies it (``sequence.setTransition`` ``frames``) when it changed."""

    def __init__(self, parent: QWidget, transition: dict[str, Any], frame: int, drop: bool, send: Any) -> None:
        super().__init__(parent)
        self.setObjectName("transitionDuration")
        self.setWindowTitle(tr("Set Transition Duration"))
        self.transition, self.frame, self.drop, self.send = transition, max(1, frame), drop, send
        self.frames = max(1, as_int(transition.get("duration")) // self.frame)
        layout = QVBoxLayout(self)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Duration:"), self))
        self.value = QLineEdit(format_timecode(self.frames * self.frame, self.frame, drop), self)
        self.value.setObjectName("transitionDuration.value")
        row.addWidget(self.value, 1)
        layout.addLayout(row)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.button(QDialogButtonBox.StandardButton.Ok).setObjectName("transitionDuration.ok")
        buttons.button(QDialogButtonBox.StandardButton.Cancel).setObjectName("transitionDuration.cancel")
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

    def accept(self) -> None:
        frames = duration_frames(self.value.text(), self.frames, self.frame, self.drop)
        if frames is not None and frames >= 1 and frames != self.frames:
            self.send("sequence.setTransition", {"transition": self.transition.get("id"), "frames": frames})
        super().accept()


class TimelineCanvas(QWidget):
    command = Signal(str, dict)  # engine command id, params
    seek = Signal(object)  # ticks, while scrubbing (local preview); object, not a 32-bit int
    files_dropped = Signal(list, object, object)  # paths (Media Browser, file manager), time, track id
    voiceover = Signal(object)  # an audio track header's microphone: record on that track (id) / stop
    voiceover_settings = Signal()  # …its menu: Voice-Over Record Settings…

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("timeline.canvas")
        self.setMouseTracking(True)
        self.setAcceptDrops(True)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setMinimumHeight(140)
        self.sequence: dict[str, Any] | None = None
        self.voiceover_track: str | None = None  # the track a voice-over records on (A1…)
        self.rows: list[Row] = []
        self.media = TimelineMedia(self)
        self.media.updated.connect(self.update)
        self.pps = 40.0  # pixels per second
        self.scroll_ticks = 0
        self.scroll_y = 0  # pixels the track rows are scrolled up under the ruler
        self.video_h = VIDEO_H  # every video track's height (egui's video_track_h)
        self.audio_h = AUDIO_H
        self.saved_heights: tuple[int, int] | None = None  # before Expand / Minimize All Tracks
        self.playhead = 0
        self.tool = "selection"
        self._drag: Drag | None = None
        self._drop_preview: tuple[int, Row] | None = None
        self._effect_target: tuple[Row, dict[str, Any], str] | None = None  # an effect dragged over a clip
        self.last_dialog: QDialog | None = None
        # the main window: runs the context menus' commands, knows which are enabled
        self.menu_host: timeline_menus.MenuHost | None = None
        self.last_menu: QMenu | None = None
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
            self.rows.append(Row(t.get("id"), str(t.get("name") or ""), "video", y, self.video_h, t, f"V{i + 1}"))
            y += self.video_h
        y += 4
        for i, t in enumerate(self.sequence.get("audio") or []):
            self.rows.append(Row(t.get("id"), str(t.get("name") or ""), "audio", y, self.audio_h, t, f"A{i + 1}"))
            y += self.audio_h

    # ----- track heights (egui's timeline.rs header edges, keyboard.rs commands) -----
    def set_track_heights(self, video: int | None = None, audio: int | None = None) -> None:
        if video is not None:
            self.video_h = max(DRAG_MIN_H, min(KEY_MAX_H, int(video)))
        if audio is not None:
            self.audio_h = max(DRAG_MIN_H, min(KEY_MAX_H, int(audio)))
        self._layout_rows()
        self.set_scroll_y(self.scroll_y)  # the content may have shrunk under the offset
        self.updateGeometry()
        self.update()
        self.scrolled_y.emit()  # the scroll bar's range follows the content

    def height_command(self, cid: str) -> dict[str, int]:
        """Expand / Minimize All Tracks (again: the heights from before), Increase / Decrease
        Video / Audio Tracks Height."""
        if cid in ("timeline.expandAllTracks", "timeline.minimizeAllTracks"):
            target = EXPANDED_H if cid == "timeline.expandAllTracks" else KEY_MIN_H
            if self.video_h == target and self.audio_h == target and self.saved_heights is not None:
                (v, a), self.saved_heights = self.saved_heights, None
                self.set_track_heights(v, a)
            else:
                self.saved_heights = (self.video_h, self.audio_h)
                self.set_track_heights(target, target)
        else:
            d = HEIGHT_STEP if "increase" in cid else -HEIGHT_STEP
            if "Video" in cid:
                self.set_track_heights(video=max(KEY_MIN_H, min(KEY_MAX_H, self.video_h + d)))
            else:
                self.set_track_heights(audio=max(KEY_MIN_H, min(KEY_MAX_H, self.audio_h + d)))
        return {"videoTrackHeight": self.video_h, "audioTrackHeight": self.audio_h}

    def height_edge_at(self, pos: QPoint) -> Row | None:
        """The track whose height a press at ``pos`` drags: a header's bottom edge, or the top
        edge of the top video track."""
        if pos.x() >= HEADER_W or pos.y() < RULER_H:
            return None
        y = pos.y()
        for i, r in enumerate(self.rows):
            bottom = r.top + r.height
            if bottom - 3 <= y < bottom + 2:
                return r
            if i == 0 and r.kind == "video" and r.top - 2 <= y < r.top + 3:
                return r
        return None

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

    @property
    def transition_selection(self) -> list[Any]:
        return list(self.sequence.get("transitionSelection") or []) if self.sequence else []

    def transition_rect(self, row: Row, x: dict[str, Any]) -> QRectF:
        start = as_int(x.get("start"))
        x0, x1 = self.x_of(start), self.x_of(start + max(0, as_int(x.get("duration"))))
        return QRectF(x0, row.top + TRANSITION_TOP, max(1.0, x1 - x0), max(1.0, row.height - TRANSITION_TOP - 1.0))

    def transition_hit(self, pos: QPoint) -> tuple[Row, dict[str, Any], str] | None:
        """The transition under ``pos`` and the end it is at ("in", "out" or "" for the middle);
        ahead of the clips under it, as in egui."""
        row = self.row_at(pos.y())
        if row is None or pos.x() < HEADER_W or pos.y() <= row.top + TRANSITION_TOP:
            return None
        for x in row.track.get("transitions") or []:
            if not isinstance(x, dict):
                continue
            r = self.transition_rect(row, x)
            if r.left() <= pos.x() <= r.right():
                # the ends are edge zones, narrower on a short transition so its middle stays grabbable
                zone = min(TRANSITION_EDGE_PX, r.width() / 3)
                edge = "in" if pos.x() - r.left() <= zone else "out" if r.right() - pos.x() <= zone else ""
                return row, x, edge
        return None

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
        self.media.begin_paint()
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
            chosen = set(self.transition_selection)
            for x in row.track.get("transitions") or []:
                if isinstance(x, dict):
                    r = self.transition_rect(row, x)
                    if r.right() >= HEADER_W and r.left() <= self.width():
                        self._paint_transition(p, r, x, row.kind == "audio", x.get("id") in chosen)
        self._paint_edit_points(p)
        if self._effect_target:  # egui's: the clip outlined, a transition's edge marked
            row, clip, edge = self._effect_target
            r = self.clip_rect(row, clip)
            accent = QColor(theme.current().accent)
            p.setPen(QPen(accent, 2))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRoundedRect(r.adjusted(1, 1, -1, -1), 3, 3)
            if edge:
                x = r.left() if edge == "in" else r.right()
                p.fillRect(QRectF(x - 1.5, r.top(), 3, r.height()), accent)
        if self._drop_preview:
            t, row = self._drop_preview
            x = self.x_of(t)
            p.fillRect(QRectF(x, row.top, 3, row.height), QColor("#ffd54a"))
        if self._drag and self._drag.kind in ("trim", "roll", "stretch", "remix") and self._drag.clip is not None:
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

    def _paint_edit_points(self, p: QPainter) -> None:
        """egui's trim brackets on the selected edit points: red for a ripple, yellow otherwise; a
        roll marks both sides of the cut."""
        for ep in (self.sequence or {}).get("editPoints") or []:
            if not isinstance(ep, dict):
                continue
            col = RIPPLE_RED if ep.get("kind") == "ripple" else ROLL_YELLOW
            for row in self.rows:
                clip = next((c for c in row.track.get("items") or [] if isinstance(c, dict) and c.get("clip") == ep.get("clip")), None)
                if clip is None:
                    continue
                out = ep.get("out") is True
                marks = [(clip, out)]
                partner = self.neighbour(row, clip, "out" if out else "in") if ep.get("kind") == "roll" else None
                if partner is not None:
                    marks.append((partner, not out))
                for c, o in marks:
                    self._paint_bracket(p, self.x_of(as_int(c.get("end") if o else c.get("start"))), row, o, col)

    @staticmethod
    def _paint_bracket(p: QPainter, x: float, row: Row, out: bool, col: QColor) -> None:
        """A trim bracket at ``x``, its arms pointing into the clip (left for an Out edge)."""
        y0, y1 = row.top + 2.0, row.top + row.height - 2.0
        d = -1.0 if out else 1.0
        xi = x + d * 1.5
        p.setPen(QPen(col, 3.0))
        p.drawLine(QPointF(xi, y0), QPointF(xi, y1))
        p.drawLine(QPointF(xi, y0 + 1.0), QPointF(xi + 6.0 * d, y0 + 1.0))
        p.drawLine(QPointF(xi, y1 - 1.0), QPointF(xi + 6.0 * d, y1 - 1.0))

    def _paint_transition(self, p: QPainter, r: QRectF, x: dict[str, Any], audio: bool, selected: bool) -> None:
        """egui's: darkened and hatched, crossing fade curves on an audio one, the name on a video
        one wide enough, a light outline, the accent when selected."""
        p.save()
        p.setClipRect(r.intersected(QRectF(HEADER_W, RULER_H, self.width() - HEADER_W, self.height())))
        p.fillRect(r, QColor(0, 0, 0, 90))
        p.setPen(QPen(QColor(0xD9, 0xD9, 0xD9, 110), 1))
        hx = r.left() - r.height()
        for _ in range(4096):  # 4 px apart; the cap is for damaged geometry
            if hx >= r.right():
                break
            p.drawLine(QPointF(hx, r.bottom()), QPointF(hx + r.height(), r.top()))
            hx += 4
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        if audio:
            p.setPen(QPen(QColor("#ffffff"), 1))
            for curve in (math.sin, math.cos):
                path = QPainterPath()
                for i in range(17):
                    f = i / 16
                    pt = QPointF(r.left() + r.width() * f, r.top() + r.height() * (1 - curve(f * math.pi / 2)))
                    if i == 0:
                        path.moveTo(pt)
                    else:
                        path.lineTo(pt)
                p.drawPath(path)
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.setPen(QPen(QColor(0xEB, 0xEB, 0xEB), 1))
        p.drawRect(r.adjusted(0.5, 0.5, -0.5, -0.5))
        if selected:
            p.setPen(QPen(theme.current().c("accent"), 2))
            p.drawRect(r.adjusted(1, 1, -1, -1))
        if r.width() > 34 and not audio:
            tag = QRectF(r.left() + 2, r.top() + 2, min(r.width() - 4, 80), 13)
            p.fillRect(tag, QColor(0, 0, 0, 170))
            p.setPen(QColor(0xD9, 0xD9, 0xD9))
            name = tr(str(x.get("name") or x.get("effect") or ""))
            p.drawText(tag.adjusted(3, 0, -2, 0), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft,
                       QFontMetrics(p.font()).elidedText(name, Qt.TextElideMode.ElideRight, int(tag.width()) - 5))
        p.restore()

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
        # markers in their colours (egui's), a marker with a duration with its range band
        for marker in self.sequence.get("markers") or [] if self.sequence else []:
            if not isinstance(marker, dict):
                continue
            col = marker_rgb(marker.get("color"))
            band = self.marker_band(marker)
            if band is not None:
                band = band.intersected(QRectF(HEADER_W, 0, max(0.0, self.width() - HEADER_W), RULER_H))
                c = QColor(col)
                c.setAlpha(110)
                p.fillRect(band, c)
            r = self.marker_rect(marker)
            if r.right() >= HEADER_W:
                path = QPainterPath()
                path.moveTo(r.left(), r.top())
                path.lineTo(r.right(), r.top())
                path.lineTo(r.right(), r.top() + r.height() * 0.6)
                path.lineTo(r.center().x(), r.bottom())
                path.lineTo(r.left(), r.top() + r.height() * 0.6)
                path.closeSubpath()
                p.fillPath(path, col)
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

    # ----- markers (egui's timeline_hit.rs marker_rect / marker_range_rect) -----
    def marker_rect(self, marker: dict[str, Any]) -> QRectF:
        x = self.x_of(as_int(marker.get("start")))
        return QRectF(x - 5.0, RULER_H - 15.0, 10.0, 13.0)

    def marker_band(self, marker: dict[str, Any]) -> QRectF | None:
        d = as_int(marker.get("duration"))
        if d <= 0:
            return None
        x0 = self.x_of(as_int(marker.get("start")))
        return QRectF(x0, 2.0, max(1.0, self.x_of(as_int(marker.get("start")) + d) - x0), 8.0)

    def marker_at(self, pos: QPoint) -> dict[str, Any] | None:
        """The topmost marker under ``pos`` on the ruler, its range band included."""
        if pos.y() >= RULER_H or pos.x() < HEADER_W or self.sequence is None:
            return None
        at = QPointF(pos)
        for m in reversed([m for m in self.sequence.get("markers") or [] if isinstance(m, dict)]):
            band = self.marker_band(m)
            if self.marker_rect(m).contains(at) or (band is not None and band.contains(at)):
                return m
        return None

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
        if row.kind == "audio":
            here = self.voiceover_track is not None and self.voiceover_track == row.short
            tip = tr("Stop Voice-over Recording") if here else tr("Voice-over record (right-click: Voice-Over Record Settings…)")
            cells.append(HeaderCell("voiceover", QRect(x, y, CELL, CELL), "", here, tip))
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
                if c.key == "voiceover":
                    p.setPen(QColor(VOICEOVER_RED) if c.on else pal.buttonText().color())
                    self._draw_mic(p, c.rect)
                elif c.key == "locked":
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
    def _draw_mic(p: QPainter, r: QRect) -> None:
        """A microphone in the current pen colour."""
        p.save()
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        c = r.center()
        p.drawRoundedRect(QRectF(c.x() - 2.5, c.y() - 7, 5, 9), 2.5, 2.5)
        p.drawArc(QRectF(c.x() - 5, c.y() - 4, 10, 9), 180 * 16, 180 * 16)
        p.drawLine(c.x(), c.y() + 5, c.x(), c.y() + 7)
        p.drawLine(c.x() - 3, c.y() + 7, c.x() + 3, c.y() + 7)
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
        if cell.key == "voiceover":
            self.voiceover.emit(row.track_id)
        elif cell.key == "patch":
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
        peaks = self.media.waveform(item, min(start, end), max(start, end), clip.get("audioStream", 0))
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
        width = max(1.0, body.width())
        for x in range(x0, x1):
            # every bucket under this pixel, so a short peak shows at any zoom
            fa, fb = (x - body.left()) / width, (x + 1 - body.left()) / width
            if reverse:
                fa, fb = 1.0 - fb, 1.0 - fa
            i0 = min(n - 1, max(0, int(fa * n)))
            i1 = min(n, max(i0 + 1, int(fb * n)))
            lo = min(q[0] for q in peaks[i0:i1])
            hi = max(q[1] for q in peaks[i0:i1])
            p.drawLine(x, int(mid - max(0.0, min(1.0, hi)) * half), x, int(mid - max(-1.0, min(0.0, lo)) * half))

    # ----- mouse -----
    def voiceover_menu(self, row: Row) -> QMenu:
        """The microphone's menu (egui's): Voice-Over Record Settings…"""
        menu = QMenu(self)
        a = menu.addAction(tr("Voice-Over Record Settings…"))
        a.setObjectName(f"timeline.track.{row.short}.voiceover.settings")
        a.triggered.connect(self.voiceover_settings.emit)
        return menu

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if self.sequence and e.button() == Qt.MouseButton.RightButton:
            row, cell = self.header_hit(e.position().toPoint())
            if row is not None and cell is not None and cell.key == "voiceover":
                self.voiceover_menu(row).popup(e.globalPosition().toPoint())
                return
            self.context_menu(e.position().toPoint(), e.globalPosition().toPoint())
            return
        if not self.sequence or e.button() != Qt.MouseButton.LeftButton:
            return
        pos = e.position().toPoint()
        if pos.x() < HEADER_W and pos.y() >= RULER_H:
            edge_row = self.height_edge_at(pos)
            if edge_row is not None:
                start = self.video_h if edge_row.kind == "video" else self.audio_h
                self._drag = Drag("height", None, pos, start, track_kind=edge_row.kind)
                return
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
        th = self.transition_hit(pos)
        if th is not None:
            if self.tool == "selection":
                self._press_transition(th, bool(mods & Qt.KeyboardModifier.ShiftModifier), pos)
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
        if self.tool == "remix":
            # egui's Remix tool: a clip's Out edge re-plans the music to the new length on release
            if edge != "out":
                self.command.emit("timeline.select", {"clips": [cid]})
                return
            self._drag = Drag("remix", clip, pos, int(clip.get("end") or 0), edge=edge)
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

    def context_menu(self, pos: QPoint, global_pos: QPoint) -> None:
        """egui's right-click on the tracks: a clip selects it (unless it is selected) and opens
        the clip menu; empty time between clips selects the gap and opens its menu."""
        host = self.menu_host
        if host is None or self.sequence is None or pos.x() < HEADER_W:
            return
        if pos.y() < RULER_H:
            marker = self.marker_at(pos)
            if marker is not None:
                # egui: the playhead goes to the marker, for Clear Selected Marker
                t = max(0, as_int(marker.get("start")))
                self.playhead = t
                self.seek.emit(t)
                self.command.emit("playhead.set", {"time": t})
                self.update()
                self._popup(timeline_menus.marker_menu(self, host, marker), global_pos)
            return
        row, clip, edge = self.hit(pos)
        if row is None:
            return
        if clip is not None and edge:
            self._edit_point_menu(row, clip, edge, pos, global_pos)
            return
        if clip is None:
            params = {"track": row.track.get("id"), "time": max(0, self.time_at(pos.x()))}

            def gap(ok: bool) -> None:
                if ok:
                    self._popup(timeline_menus.gap_menu(self, host), global_pos)

            host.menu_request("timeline.selectGap", params, gap)
            return
        cid = clip.get("clip")
        select = None
        chosen = set(self.selection)
        if cid not in chosen:
            select = {"clips": [cid]}
            chosen = {cid}
            self.sequence["selection"] = [cid]  # local echo until the engine's reply

        def clips(_ok: bool) -> None:
            # the selection as the engine made it (linked partners too), else what was clicked
            sel = set(self.selection) if cid in self.selection else chosen
            picked = [c for r in self.rows for c in r.track.get("items") or [] if isinstance(c, dict) and c.get("clip") in sel]

            def unlinked() -> None:
                # egui: only the right-clicked clip stays selected, to drag it away from the others
                self.command.emit("timeline.select", {"clips": [cid], "linked": False})

            self._popup(timeline_menus.clip_menu(self, host, picked, LABEL_COLORS, unlinked), global_pos)

        host.menu_request("timeline.select" if select else "", select or {}, clips)

    def _edit_point_menu(self, row: Row, clip: dict[str, Any], edge: str, pos: QPoint, global_pos: QPoint) -> None:
        """egui's right-click on a clip's edge: select that edit point (unless one of the cut's is
        selected) with the tool's trim type, then the edit point menu."""
        host = self.menu_host
        if host is None or self.sequence is None:
            return
        other = self.neighbour(row, clip, edge)
        cid = clip.get("clip")
        oid = other.get("clip") if other else None
        left, right = (cid, oid) if edge == "out" else (oid, cid)
        eps = [e for e in self.sequence.get("editPoints") or [] if isinstance(e, dict)]
        on_it = any((e.get("out") is True and e.get("clip") == left) or (e.get("out") is False and e.get("clip") == right) for e in eps)
        through = any(isinstance(t, dict) and t.get("left") == left and t.get("right") == right for t in self.sequence.get("throughEdits") or [])
        chosen: list[dict[str, Any] | None] = [eps[0] if eps else None]

        def show(_ok: bool) -> None:
            self._popup(timeline_menus.edit_point_menu(self, host, left, right, chosen[0], through), global_pos)

        if on_it:
            host.menu_request("", {}, show)
            return

        def select(kind: str) -> None:
            chosen[0] = {"clip": cid, "out": edge == "out", "kind": kind}
            host.menu_request("trim.selectEditPoint", {"clip": cid, "edge": edge, "kind": kind}, show)

        if self.tool == "rolling":
            select("roll")
        elif self.tool == "ripple":
            select("ripple")
        else:
            cut = int(clip.get("end") if edge == "out" else clip.get("start") or 0)
            host.edit_point_kind(abs(self.x_of(cut) - pos.x()), other is not None, select)

    def _popup(self, menu: QMenu, global_pos: QPoint) -> None:
        menu.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.last_menu = menu
        menu.destroyed.connect(lambda _o=None, m=menu: self._menu_gone(m))
        menu.popup(global_pos)

    def _menu_gone(self, menu: QMenu) -> None:
        if self.last_menu is menu:
            self.last_menu = None

    def _press_transition(self, hit: tuple[Row, dict[str, Any], str], shift: bool, pos: QPoint) -> None:
        """Select it (Shift adds or removes; a selected one is chosen on release, so dragging a
        selection keeps it) and get ready to drag it."""
        _row, x, edge = hit
        tid = x.get("id")
        if tid not in self.transition_selection:
            self.command.emit("timeline.select", {"transitions": [tid], "toggle": shift})
            if self.sequence is not None:  # local echo until the engine's reply
                self.sequence["transitionSelection"] = [*self.transition_selection, tid] if shift else [tid]
                if not shift:
                    self.sequence["selection"] = []
            shift = False
            self.update()
        self._drag = Drag("transition", None, pos, 0, edge=edge, mode="toggle" if shift else "pick", transition=dict(x))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        pos = e.position().toPoint()
        d = self._drag
        if d is None and pos.x() < HEADER_W:
            if self.height_edge_at(pos) is not None:
                self.setToolTip(tr("Drag to change the track height"))
                self.setCursor(Qt.CursorShape.SizeVerCursor)
                return
            _row, cell = self.header_hit(pos)
            self.setToolTip(cell.tip if cell is not None else "")
            self.unsetCursor()
            return
        if d is not None and d.kind == "height":
            # video tracks stack upwards: dragging up makes them taller
            dy = pos.y() - d.origin.y()
            h = d.origin_time + (-dy if d.track_kind == "video" else dy)
            h = max(DRAG_MIN_H, min(DRAG_MAX_H, h))
            self.set_track_heights(**{d.track_kind: h})
            return
        if d is None:
            th = self.transition_hit(pos) if self.tool == "selection" else None
            if th is not None:
                self.setCursor(Qt.CursorShape.SizeHorCursor if th[2] else Qt.CursorShape.OpenHandCursor)
                return
            _row, clip, edge = self.hit(pos)
            if self.tool == "hand":
                self.setCursor(Qt.CursorShape.OpenHandCursor)
            elif self.tool == "zoom":
                self.setCursor(Qt.CursorShape.CrossCursor)
            elif self.tool == "razor" and clip is not None:
                self.setCursor(Qt.CursorShape.IBeamCursor)
            elif self.tool in ("slip", "slide") and clip is not None:
                self.setCursor(Qt.CursorShape.SizeHorCursor)
            elif (edge and self.tool in ("selection", "ripple", "rolling", "rateStretch")) or (edge == "out" and self.tool == "remix"):
                self.setCursor(Qt.CursorShape.SplitHCursor if self.tool == "rolling" else Qt.CursorShape.SizeHorCursor)
            else:
                self.unsetCursor()
            return
        if d.kind == "scrub":
            self._scrub_to(pos.x())
            return
        if d.kind == "transition":
            self._drag_transition(d, pos)
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
        if d.kind == "remix" and d.clip is not None:
            # whole frames, at least one frame long
            ft = self.frame_ticks()
            target = max(int(d.clip.get("start") or 0) + ft, round(target / ft) * ft)
        if d.kind == "move":
            target = max(0, target)
            start_row = self.row_at(d.origin.y())
            now_row = self.row_at(pos.y())
            d.row_delta = 0
            if start_row and now_row and start_row.kind == now_row.kind:
                d.row_delta = self.rows.index(now_row) - self.rows.index(start_row)
        d.delta = target - d.origin_time
        self.update()

    def _drag_transition(self, d: Drag, pos: QPoint) -> None:
        """An end changes its duration from that end, the middle slides it over the cut: the
        engine applies the Timeline's rules to the span the drag began with; one undo step."""
        x = d.transition or {}
        ft = self.frame_ticks()
        delta = round((pos.x() - d.origin.x()) * TICKS_PER_SECOND / self.pps / ft) * ft
        if delta == d.delta:
            return
        d.delta = delta
        self.command.emit("sequence.setTransition", {
            "transition": x.get("id"),
            "drag": {"edge": d.edge or None, "delta": delta, "start": as_int(x.get("start")), "duration": as_int(x.get("duration"))},
            "merge": True,
            "begin": not d.begun,
        })
        d.begun = True

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        d, self._drag = self._drag, None
        if d is None or e.button() != Qt.MouseButton.LeftButton or d.kind == "height":
            return
        if d.kind == "scrub":
            self.command.emit("playhead.set", {"time": self.playhead})
            return
        if d.kind == "transition":
            if not d.begun:  # a click on a selected transition: it alone, or (Shift) out of the selection
                tid = (d.transition or {}).get("id")
                if d.mode == "toggle":
                    self.command.emit("timeline.select", {"transitions": [tid], "toggle": True})
                elif self.transition_selection != [tid] or self.selection:
                    self.command.emit("timeline.select", {"transitions": [tid]})
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
        if d.kind == "remix":
            length = int(d.clip.get("end") or 0) - int(d.clip.get("start") or 0) + d.delta
            self.command.emit("clip.remix", {"clip": d.clip.get("clip"), "duration": length})
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
        pos = e.position().toPoint()
        if pos.x() < HEADER_W and pos.y() >= RULER_H:
            # egui: a double-click on a header (off its buttons) toggles that kind's tracks
            # between tall and short
            row, cell = self.header_hit(pos)
            if row is not None and cell is None and self.height_edge_at(pos) is None:
                h = self.video_h if row.kind == "video" else self.audio_h
                self.set_track_heights(**{row.kind: 64 if h < 50 else 30})
            return
        th = self.transition_hit(e.position().toPoint())
        if th is not None:
            if self.tool == "selection":
                self.open_transition_duration(th[1])
            return
        _row, clip, _edge = self.hit(e.position().toPoint())
        if clip is not None and clip.get("item") is not None:
            self.command.emit("source.open", {"item": clip.get("item")})

    def open_transition_duration(self, x: dict[str, Any]) -> TransitionDurationDialog:
        """Set Transition Duration, as a double-click on a transition opens it in egui."""
        self.command.emit("timeline.select", {"transitions": [x.get("id")]})
        settings = (self.sequence or {}).get("settings")
        dialog = TransitionDurationDialog(self, x, self.frame_ticks(), drop_frame(settings),
                                          lambda c, prm: self.command.emit(c, prm))
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.last_dialog = dialog
        dialog.open()
        return dialog

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
        m = e.mimeData()
        ours = m.hasFormat(ITEM_MIME) or m.hasFormat(SOURCE_MIME) or m.hasFormat(FILES_MIME) or m.hasFormat(TEMPLATE_MIME) or m.hasFormat(EFFECT_MIME)
        if (ours or m.hasUrls()) and self.sequence:
            e.acceptProposedAction()

    def effect_target(self, pos: QPoint, effect: str) -> tuple[Row, dict[str, Any], str] | None:
        """The clip an effect dropped at ``pos`` goes on; a transition's edge is the nearer one."""
        row, clip, _edge = self.hit(pos)
        if row is None or clip is None:
            return None
        transition = next((str(x.get("kind") or "") for x in getattr(getattr(self.window(), "effects", None), "effects", []) if x.get("id") == effect), "").endswith("Transition")
        if not transition:
            return row, clip, ""
        r = self.clip_rect(row, clip)
        return row, clip, "in" if pos.x() - r.left() < r.right() - pos.x() else "out"

    def dragMoveEvent(self, e) -> None:
        pos = e.position().toPoint()
        effect = dropped_effect(e.mimeData())
        if effect is not None:
            self._effect_target = self.effect_target(pos, effect)
            self._drop_preview = None
            if self._effect_target is None:
                e.ignore()
            else:
                e.acceptProposedAction()
            self.update()
            return
        row = self.row_at(pos.y())
        source = source_payload(e.mimeData().data(SOURCE_MIME)) if e.mimeData().hasFormat(SOURCE_MIME) else None
        # a Source span lands only on tracks of a stream it carries (egui's)
        wrong_stream = source is not None and row is not None and not source["video" if row.kind == "video" else "audio"]
        if row is None or pos.x() < HEADER_W or wrong_stream or (e.mimeData().hasFormat(TEMPLATE_MIME) and row.kind != "video"):
            self._drop_preview = None
            e.ignore()
        else:
            self._drop_preview = (self.snap(self.time_at(pos.x()), set()), row)
            e.acceptProposedAction()
        self.update()

    def source_place_params(self, source: dict[str, Any], t: int, row: Row, insert: bool) -> dict[str, Any]:
        """``timeline.place`` for a Source span dropped on ``row`` at ``t``: the other stream goes
        on the track of the same number (egui's), Ctrl inserts."""
        seq = self.sequence or {}
        videos = [v.get("id") for v in seq.get("video") or [] if isinstance(v, dict)]
        audios = [a.get("id") for a in seq.get("audio") or [] if isinstance(a, dict)]
        mine, other = (videos, audios) if row.kind == "video" else (audios, videos)
        index = mine.index(row.track_id) if row.track_id in mine else 0
        partner = other[index] if index < len(other) else (other[0] if other else None)
        vt, at = (row.track_id, partner) if row.kind == "video" else (partner, row.track_id)
        return {
            "item": source["item"],
            "time": t,
            "track": vt,
            "audioTrack": at,
            "insert": insert,
            "sourceIn": source["sourceIn"],
            "duration": source["duration"],
            "video": source["video"],
            "audio": source["audio"],
        }

    def dragLeaveEvent(self, _e) -> None:
        self._drop_preview = None
        self._effect_target = None
        self.update()

    def dropEvent(self, e) -> None:
        preview, self._drop_preview = self._drop_preview, None
        target, self._effect_target = self._effect_target, None
        self.update()
        effect = dropped_effect(e.mimeData())
        if effect is not None:
            if target is not None:  # egui's: applied to that clip, which is then selected
                _row, clip, edge = target
                cid = clip.get("clip")
                if effect.startswith("preset:"):
                    self.command.emit("presets.apply", {"preset": effect[len("preset:"):], "clips": [cid]})
                elif effect.startswith("lumetri:"):
                    self.command.emit("lumetri.applyPreset", {"name": effect[len("lumetri:"):], "clips": [cid]})
                elif edge:
                    self.command.emit("effects.apply", {"effect": effect, "clip": cid, "edge": edge})
                else:
                    self.command.emit("effects.apply", {"effect": effect, "clips": [cid]})
                self.command.emit("timeline.select", {"clips": [cid]})
                e.acceptProposedAction()
            return
        if preview is None:
            return
        try:
            items = [int(x) for x in bytes(e.mimeData().data(ITEM_MIME)).decode().split(",") if x] if e.mimeData().hasFormat(ITEM_MIME) else []
        except ValueError:
            return
        t, row = preview
        template = dropped_template(e.mimeData())
        if template is not None:  # an Essential Graphics template card: egui places it on that video track
            videos = [v.get("id") for v in (self.sequence or {}).get("video") or []]
            if row.kind == "video" and row.track_id in videos:
                self.command.emit("graphics.template.apply", {"template": template, "time": t, "track": videos.index(row.track_id)})
                e.acceptProposedAction()
            return
        if e.mimeData().hasFormat(SOURCE_MIME):
            source = source_payload(e.mimeData().data(SOURCE_MIME))
            if source is not None:
                self.command.emit("timeline.place", self.source_place_params(source, t, row, bool(e.modifiers() & Qt.KeyboardModifier.ControlModifier)))
                e.acceptProposedAction()
            return
        if not e.mimeData().hasFormat(ITEM_MIME):
            paths = dropped_files(e.mimeData())
            if paths:
                self.files_dropped.emit(paths, t, row.track_id)
                e.acceptProposedAction()
            return
        if items:  # one after another, as one undo step (each at ``t`` would overwrite the last)
            self.command.emit("timeline.place", {"items": items, "time": t, "track": row.track_id})
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
