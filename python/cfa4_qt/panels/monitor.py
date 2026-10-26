"""Program and Source monitors, with playback.

Both monitors share one implementation (``Monitor``) and differ only in what they show:
the Program monitor shows the active sequence at the sequence playhead, the Source monitor the
clip opened with ``source.open`` at the Source playhead (``target: "source"`` in the engine
protocol). Frames come from the engine (``frame.render``, RGBA8) at the smaller of the
playback/paused resolution and the size on screen; sound from ``audio.render`` (interleaved
float32) into a QAudioSink. Requests are coalesced: at most one frame and one audio request are
in flight, and a newer wanted time replaces an older one, so a slow render drops frames (the
dropped-frame light turns yellow) instead of building a queue.

As egui's monitors (crates/ui-egui/src/panels/monitor.rs): a timecode field to type a time in,
the magnification (Fit, 10 %…1600 %; the wheel zooms about the pointer, a middle-button drag or
the Hand tool pans), Playback Resolution, the settings (wrench) menu with the display modes
(composite, alpha, red, green, blue), Paused Resolution, High Quality Playback, Safe Margins,
Show Transport Controls and Loop, the duration of the marked range, the time bar, and egui's
transport row with its Button Editor. J/K/L shuttle (doubling up to 8×) and Play In to Out work
on the focused monitor. What a monitor shows is kept per monitor (``MonitorView``).

Every ``audio.render`` asks for the engine's meter of the block too (``meter: true``: sample
peaks every 1/100 s and the loudness so far); the Audio Meters panel shows the block being heard.
Without an audio device the sound is still fetched at the playing pace, for the meters alone.
"""

from __future__ import annotations

import json
from collections import deque
from typing import Any

from PySide6.QtCore import QElapsedTimer, QEvent, QMimeData, QPointF, QRect, QRectF, QSettings, Qt, QTimer, Signal
from PySide6.QtGui import (
    QAction,
    QActionGroup,
    QColor,
    QDrag,
    QImage,
    QKeyEvent,
    QMouseEvent,
    QPainter,
    QPaintEvent,
    QPen,
    QWheelEvent,
)
from PySide6.QtWidgets import (
    QApplication,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMenu,
    QSizePolicy,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..engine import EngineClient
from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND, drop_frame, format_timecode, frame_ticks
from .monitor_view import (
    CHANNELS,
    DISPLAYS,
    MAX_ZOOM,
    MIN_ZOOM,
    RESOLUTIONS,
    SOURCE_DISPLAYS,
    SOURCE_MIME,
    ZOOMS,
    MonitorView,
    res_label,
)
from .timeline_media import TimelineMedia
from .transport import ACCENT, PROGRAM_BUTTONS, SOURCE_BUTTONS, TransportBar, TransportDef, icon_color, transport_icon

try:  # QtMultimedia is optional: without it playback is silent
    from PySide6.QtMultimedia import QAudioFormat, QAudioSink, QMediaDevices

    HAVE_AUDIO = True
except ImportError:  # pragma: no cover - depends on the PySide6 build
    HAVE_AUDIO = False

AUDIO_CHUNK_SECONDS = 0.1
AUDIO_LEAD_SECONDS = 0.35
METER_KEEP_SECONDS = 3.0
SOURCE_SAMPLE_RATE = 48_000
MAX_SPEED = 8.0
GREEN = QColor(0x4C, 0xC0, 0x5A)
YELLOW = QColor(0xF0, 0xC8, 0x40)


def ticks_per_frame_from_fps(fps: Any) -> int:
    """Frame length in ticks for a float frame rate (``source.inspect`` reports ``fps``)."""
    try:
        value = float(fps)
    except (TypeError, ValueError):
        value = 0.0
    if not 1.0 <= value <= 1000.0:
        value = 30.0
    return max(1, round(TICKS_PER_SECOND / value))


def channel_image(image: QImage, display: str) -> QImage:
    """One channel of a frame as greyscale (the Alpha, Red, Green and Blue display modes)."""
    if display not in CHANNELS or image.isNull():
        return image
    img = image.convertToFormat(QImage.Format.Format_RGBA8888)
    w, h = img.width(), img.height()
    data = bytes(img.constBits())
    bpl = img.bytesPerLine()
    index = {"red": 0, "green": 1, "blue": 2, "alpha": 3}[display]
    if bpl == w * 4:
        plane = data[index::4]
    else:
        plane = b"".join(data[y * bpl : y * bpl + w * 4][index::4] for y in range(h))
    return QImage(plane, w, h, w, QImage.Format.Format_Grayscale8).copy()


class FrameView(QWidget):
    """The picture: fitted or magnified and panned, with the display mode, safe margins and the
    dropped-frame light drawn over it."""

    view_changed = Signal()  # magnification or pan
    drag_out = Signal()  # a left-button drag on the picture (the Source monitor lifts its clip)

    def __init__(self, name: str = "program", mv: MonitorView | None = None, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"{name}.view")
        self.mv = mv or MonitorView()
        self.image: QImage | None = None  # as the engine rendered it
        self.shown: QImage | None = None  # in the display mode
        self.full = (0, 0)  # the frame's own size (pixels)
        self.par = 1.0
        self.message = ""
        self.hand = False  # the Hand tool is selected: a drag pans
        self.dropped: int | None = None  # dropped frames while playing (None: stopped)
        self._pan_from: QPointF | None = None
        self._press: QPointF | None = None
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setMinimumSize(160, 90)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)

    def set_image(self, image: QImage | None, message: str = "", full: tuple[int, int] | None = None, par: float | None = None) -> None:
        self.image = image
        self.message = message
        if full is not None and full[0] > 0 and full[1] > 0:
            self.full = (int(full[0]), int(full[1]))
        elif image is not None and not image.isNull() and self.full == (0, 0):
            self.full = (image.width(), image.height())
        if par is not None and par > 0:
            self.par = float(par)
        self.refresh_display()

    def refresh_display(self) -> None:
        self.shown = channel_image(self.image, self.mv.display) if self.image is not None else None
        self.update()

    # ----- geometry -----
    def display_size(self) -> tuple[float, float]:
        """The picture's size at 100 % on screen (its display aspect, in device-independent px)."""
        fw, fh = self.full
        if fw <= 0 or fh <= 0:
            if self.image is None or self.image.isNull():
                return 0.0, 0.0
            fw, fh = self.image.width(), self.image.height()
        dpr = max(0.1, self.devicePixelRatioF())
        return fw * self.par / dpr, fh / dpr

    def picture_rect(self) -> QRectF:
        """Fitted, or magnified about the centre plus the (clamped) pan (egui's picture_rect)."""
        w, h = self.display_size()
        area = QRectF(self.rect())
        if w <= 0 or h <= 0:
            return area
        if self.mv.zoom is None:
            s = min(area.width() / w, area.height() / h)
            size = (w * s, h * s)
        else:
            size = (w * self.mv.zoom, h * self.mv.zoom)
        maxx, maxy = max(0.0, (size[0] - area.width()) / 2), max(0.0, (size[1] - area.height()) / 2)
        px = min(maxx, max(-maxx, self.mv.pan[0])) if self.mv.zoom is not None else 0.0
        py = min(maxy, max(-maxy, self.mv.pan[1])) if self.mv.zoom is not None else 0.0
        c = area.center()
        return QRectF(c.x() + px - size[0] / 2, c.y() + py - size[1] / 2, size[0], size[1])

    def wanted_size(self) -> tuple[int, int]:
        """The largest frame (pixels) worth rendering for the picture as shown."""
        pic = self.picture_rect()
        dpr = max(0.1, self.devicePixelRatioF())
        w = int(pic.width() * dpr / max(0.01, self.par))
        h = int(pic.height() * dpr)
        return min(8192, max(16, w)), min(8192, max(16, h))

    def current_zoom(self) -> float:
        """The magnification shown (1.0: one frame pixel per screen pixel), Fit included."""
        w, _h = self.display_size()
        return self.picture_rect().width() / w if w > 0 else 1.0

    def zoom_about(self, factor: float, at: QPointF) -> None:
        """Zoom by ``factor`` keeping the point under ``at`` where it is (egui's zoom_pan_input)."""
        pic = self.picture_rect()
        if pic.width() <= 0:
            return
        cur = self.current_zoom()
        z = min(MAX_ZOOM, max(MIN_ZOOM, cur * factor))
        k = z / cur
        centre = at - (at - pic.center()) * k
        area = QRectF(self.rect())
        maxx = max(0.0, (pic.width() * k - area.width()) / 2)
        maxy = max(0.0, (pic.height() * k - area.height()) / 2)
        self.mv.zoom = z
        self.mv.pan = [min(maxx, max(-maxx, centre.x() - area.center().x())), min(maxy, max(-maxy, centre.y() - area.center().y()))]
        self.update()
        self.view_changed.emit()

    def pan_by(self, dx: float, dy: float) -> None:
        if self.mv.zoom is None:
            return
        pic = self.picture_rect()
        area = QRectF(self.rect())
        maxx, maxy = max(0.0, (pic.width() - area.width()) / 2), max(0.0, (pic.height() - area.height()) / 2)
        self.mv.pan = [min(maxx, max(-maxx, self.mv.pan[0] + dx)), min(maxy, max(-maxy, self.mv.pan[1] + dy))]
        self.update()
        self.view_changed.emit()

    # ----- input -----
    def wheelEvent(self, e: QWheelEvent) -> None:
        steps = e.angleDelta().y() / 120.0
        if not steps:
            return super().wheelEvent(e)
        # about 10 % a notch, so a notch out undoes a notch in exactly
        self.zoom_about(1.1**steps, e.position())
        e.accept()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        pans = e.button() == Qt.MouseButton.MiddleButton or (e.button() == Qt.MouseButton.LeftButton and self.hand)
        if pans and self.mv.zoom is not None:
            self._pan_from = e.position()
            self.setCursor(Qt.CursorShape.ClosedHandCursor)
            e.accept()
            return
        self._press = e.position() if e.button() == Qt.MouseButton.LeftButton and not self.hand else None
        super().mousePressEvent(e)

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._pan_from is not None:
            d = e.position() - self._pan_from
            self._pan_from = e.position()
            self.pan_by(d.x(), d.y())
            return
        if self._press is not None and (e.position() - self._press).manhattanLength() >= QApplication.startDragDistance():
            self._press = None
            self.drag_out.emit()
            return
        super().mouseMoveEvent(e)

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        self._press = None
        if self._pan_from is not None:
            self._pan_from = None
            self.set_hand(self.hand)
            return
        super().mouseReleaseEvent(e)

    def set_hand(self, on: bool) -> None:
        self.hand = on
        if on:
            self.setCursor(Qt.CursorShape.OpenHandCursor)
        else:
            self.unsetCursor()

    # ----- drawing -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        p.fillRect(self.rect(), T.c("panel_bg"))
        img = self.shown
        if img is not None and not img.isNull():
            pic = self.picture_rect()
            p.fillRect(pic, QColor("#000000"))
            p.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            p.drawImage(pic, img)
            if self.mv.safe_margins:
                for f, alpha in ((0.9, 120), (0.8, 90)):
                    r = QRectF(0, 0, pic.width() * f, pic.height() * f)
                    r.moveCenter(pic.center())
                    p.setPen(QPen(QColor(255, 255, 255, alpha), 1.0))
                    p.drawRect(r)
                c = pic.center()
                p.setPen(QPen(QColor(255, 255, 255, 120), 1.0))
                p.drawLine(QPointF(c.x() - 8, c.y()), QPointF(c.x() + 8, c.y()))
                p.drawLine(QPointF(c.x(), c.y() - 8), QPointF(c.x(), c.y() + 8))
            if self.mv.display != "composite":
                label = tr(dict(DISPLAYS).get(self.mv.display, ""))
                p.setPen(QColor(0x8C, 0x8C, 0x8C))
                f = p.font()
                f.setPixelSize(11)
                p.setFont(f)
                p.drawText(QRectF(0, 6, self.width() - 8, 16), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignTop, label)
        if self.dropped is not None:
            p.setRenderHint(QPainter.RenderHint.Antialiasing)
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(YELLOW if self.dropped > 0 else GREEN)
            p.drawEllipse(QPointF(10, 10), 4, 4)
        if self.message:
            p.setPen(T.c("text_dim"))
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter | Qt.TextFlag.TextWordWrap, self.message)


class TimecodeField(QLineEdit):
    """The monitor's timecode: shows the playhead, takes a typed time (Enter) as egui's
    timecode field does: ``01:02:03:04``, bare digits (``1000`` is 00:00:10:00), ``+15`` /
    ``-1.00`` relative. The engine reads it (``time.parse``). Escape puts the time back."""

    submitted = Signal(str)

    def __init__(self, name: str, parent: QWidget | None = None) -> None:
        super().__init__("00:00:00:00", parent)
        self.setObjectName(f"{name}.timecode")
        self.setFixedWidth(110)
        self.setFrame(False)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)  # never the window's first focus
        self._restyle()
        self.shown_text = "00:00:00:00"
        self.returnPressed.connect(lambda: self.submitted.emit(self.text()))

    def _restyle(self) -> None:
        c = theme.current().timecode
        sheet = f"QLineEdit {{ color: {c}; background: transparent; border: none; font-family: monospace; font-size: 14px; }}"
        if self.styleSheet() != sheet:  # setting it changes the palette again
            self.setStyleSheet(sheet)

    def changeEvent(self, e: QEvent) -> None:
        super().changeEvent(e)
        if e.type() == QEvent.Type.PaletteChange:
            self._restyle()

    def show_time(self, text: str) -> None:
        self.shown_text = text
        if not self.hasFocus():
            self.setText(text)

    def event(self, e: QEvent) -> bool:
        # while typing, the keys are the field's: Enter, digits and letters are also editor
        # shortcuts (Render Effects In to Out, Mark In, …)
        typing = isinstance(e, QKeyEvent) and not e.modifiers() & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.MetaModifier)
        if e.type() == QEvent.Type.ShortcutOverride and typing:
            e.accept()
            return True
        return super().event(e)

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape:
            self.setText(self.shown_text)
            self.clearFocus()
            return
        super().keyPressEvent(e)

    def focusInEvent(self, e) -> None:
        super().focusInEvent(e)
        QTimer.singleShot(0, self.selectAll)

    def focusOutEvent(self, e) -> None:
        super().focusOutEvent(e)
        self.setText(self.shown_text)


class ScrubBar(QWidget):
    """The monitor's time bar: range, In/Out shading, playhead. Click or drag to seek."""

    seek = Signal(object)  # ticks; object, not a 32-bit int

    def __init__(self, name: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"{name}.scrub")
        self.setFixedHeight(18)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.start = 0
        self.end = 0
        self.playhead = 0
        self.mark_in: int | None = None
        self.mark_out: int | None = None
        self.frame = 1

    def set_range(self, start: int, end: int, mark_in: int | None, mark_out: int | None, frame: int) -> None:
        self.start, self.end = int(start), max(int(start), int(end))
        self.mark_in, self.mark_out = mark_in, mark_out
        self.frame = max(1, int(frame))
        self.update()

    def set_playhead(self, t: int) -> None:
        self.playhead = int(t)
        self.update()

    def _x(self, t: int) -> float:
        span = max(1, self.end - self.start)
        return 4 + (self.width() - 8) * (t - self.start) / span

    def time_at(self, x: float) -> int:
        span = max(0, self.end - self.start)
        frac = min(1.0, max(0.0, (x - 4) / max(1.0, self.width() - 8)))
        t = self.start + int(span * frac)
        # snap to the frame grid, never past the last frame
        t = self.start + (t - self.start) // self.frame * self.frame
        return min(t, max(self.start, self.end - self.frame))

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        p.fillRect(self.rect(), T.c("panel_bg"))
        if self.end <= self.start:
            return
        h = self.height()
        p.fillRect(QRectF(4, h / 2 - 2, self.width() - 8, 4), T.c("separator"))
        if self.mark_in is not None or self.mark_out is not None:
            a = self._x(self.mark_in if self.mark_in is not None else self.start)
            b = self._x(self.mark_out if self.mark_out is not None else self.end)
            p.fillRect(QRectF(a, 2, max(1.0, b - a), h - 4), QColor(90, 140, 220, 110))
        x = self._x(self.playhead)
        p.fillRect(QRectF(x - 1, 0, 2, h), T.c("playhead"))

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.seek.emit(self.time_at(e.position().x()))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if e.buttons() & Qt.MouseButton.LeftButton:
            self.seek.emit(self.time_at(e.position().x()))


class Monitor(QWidget):
    """Shared monitor: picture, controls row, time bar, transport, playback with sound."""

    target = "program"
    BUTTONS: tuple[TransportDef, ...] = PROGRAM_BUTTONS
    command = Signal(str, dict)  # an engine command to execute as is
    run = Signal(str, dict)  # a command that may need the GUI first (Export Frame asks for a file)
    message = Signal(str)  # an error to show in Events
    # ticks exceed 32 bits after 8.5 ms, and an int-typed signal is a 32-bit C int: use object
    playhead_moved = Signal(object)  # local playhead during playback, ticks

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"panel.{self.target.capitalize()}")
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.engine = engine
        self.settings: QSettings | None = None
        self.mv = MonitorView()
        self.playhead = 0
        self.revision = -1
        self._frame_busy = False
        self._frame_wanted: int | None = None
        self._shown: tuple[Any, ...] | None = None
        self.view = FrameView(self.target, self.mv, self)
        self.view.view_changed.connect(self._view_changed)
        self.scrub = ScrubBar(self.target, self)
        self.scrub.seek.connect(self.seek)
        self.timecode = TimecodeField(self.target, self)
        self.timecode.submitted.connect(self._typed_time)
        self.duration_label = QLabel("", self)
        self.duration_label.setObjectName(f"{self.target}.duration")

        def dropdown(name: str, tip: str) -> QToolButton:
            b = QToolButton(self)
            b.setObjectName(f"{self.target}.{name}")
            b.setToolTip(tr(tip))
            b.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            b.setMenu(QMenu(b))
            return b

        self.zoom_button = dropdown("zoom", N_("Select Zoom Level"))
        self.zoom_button.menu().aboutToShow.connect(self._fill_zoom_menu)
        self.res_button = dropdown("resolution", N_("Select Playback Resolution"))
        self.res_button.menu().aboutToShow.connect(self._fill_res_menu)
        self.wrench = dropdown("settings", N_("Settings"))
        self.wrench.setIcon(transport_icon("wrench", icon_color(self)))
        self.wrench.setAutoRaise(True)
        self.wrench.setStyleSheet("QToolButton::menu-indicator { image: none; }")
        self.wrench.menu().aboutToShow.connect(self._fill_wrench_menu)

        controls = QHBoxLayout()
        self.controls = controls
        controls.setContentsMargins(10, 2, 10, 0)
        controls.setSpacing(6)
        controls.addWidget(self.timecode)
        controls.addWidget(self.zoom_button)
        controls.addStretch(1)
        controls.addWidget(self.res_button)
        controls.addWidget(self.wrench)
        controls.addSpacing(8)
        controls.addWidget(self.duration_label)

        self.transport = TransportBar(self.target, self.BUTTONS, self.mv.hidden, self)
        self.transport.triggered.connect(self.run_transport)
        self.transport.hidden_changed.connect(self._hidden_changed)
        self.transport.checked["playback.loop"] = lambda: self.mv.looping
        self.buttons = self.transport.buttons
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(2)
        layout.addWidget(self.view, 1)
        layout.addLayout(controls)
        layout.addWidget(self.scrub)
        layout.addWidget(self.transport)

        # playback state
        self.playing = False
        self._automation_pass = False  # a mixer automation pass is running (mixer.recordStart)
        self.speed = 1.0
        self.stop_at: int | None = None  # Play In to Out: stop here…
        self.rest_at: int | None = None  # …and rest the playhead here
        self.dropped = 0
        self._clock = QElapsedTimer()
        self._play_from = 0
        self._timer = QTimer(self)
        self._timer.setInterval(10)
        self._timer.timeout.connect(self._tick)
        self._sink: Any = None
        self._sink_io: Any = None
        self._audio_next = 0
        self._audio_busy = False
        self._audio_rate = 48000
        self._audio_channels = 2
        self._audio_pending = b""
        self.audio_generation = 0  # bumped when the played blocks restart (a new pass, a loop)
        # (first sample, samples per block, [peaks per block], [loudness per block]) as played
        self.meter_blocks: deque[tuple[int, int, list[Any], list[Any]]] = deque()
        # (first sample, frames, {strip id: peaks}) per played chunk, for the Audio Track Mixer
        self.strip_blocks: deque[tuple[int, int, dict[Any, list[float]]]] = deque()
        self._apply_view(request=False)
        self._restyle()

    def _restyle(self) -> None:
        sheet = f"QLabel {{ color: {theme.current().text_dim}; font-family: monospace; font-size: 14px; }}"
        if self.duration_label.styleSheet() != sheet:
            self.duration_label.setStyleSheet(sheet)
        self.wrench.setIcon(transport_icon("wrench", icon_color(self)))
        self.transport.set_icon("loop", "loop", ACCENT if self.mv.looping else None)
        self.transport.set_icon("toggle", "pause" if getattr(self, "playing", False) else "play")

    def changeEvent(self, e: QEvent) -> None:
        super().changeEvent(e)
        if e.type() == QEvent.Type.PaletteChange and hasattr(self, "transport"):
            self._restyle()

    # ----- what each monitor shows (overridden) -----
    def has_content(self) -> bool:
        raise NotImplementedError

    def range(self) -> tuple[int, int]:
        """(first tick, end tick) of what is shown."""
        raise NotImplementedError

    def marks(self) -> tuple[int | None, int | None]:
        return None, None

    def frame(self) -> int:
        raise NotImplementedError

    def drop_frame(self) -> bool:
        """Whether the timecode counts drop-frame (the engine parses typed timecode the same way)."""
        return False

    def content_key(self) -> Any:
        return None

    def empty_message(self) -> str:
        return ""

    def seek_command(self, t: int) -> tuple[str, dict[str, Any]]:
        raise NotImplementedError

    def audio_params(self, start: int, frames: int) -> dict[str, Any]:
        return {"start": start, "frames": frames}

    def sample_rate(self) -> int:
        return 48000

    # ----- view settings -----
    def attach_settings(self, settings: QSettings | None) -> None:
        """Remember this monitor's view in ``settings`` (and take the one saved there)."""
        self.settings = settings
        loaded = MonitorView.load(settings, self._settings_key())
        for name in vars(loaded):
            setattr(self.mv, name, getattr(loaded, name))
        self.transport.hidden = list(self.mv.hidden)
        self._apply_view(request=False)

    def _settings_key(self) -> str:
        return f"monitor/{self.target}"

    def _save_view(self) -> None:
        self.mv.save(self.settings, self._settings_key())

    def _apply_view(self, request: bool = True) -> None:
        self.zoom_button.setText(self.mv.zoom_label())
        self.res_button.setText(res_label(self.mv.res))
        self.transport.setVisible(self.mv.show_transport)
        self.transport.set_icon("loop", "loop", ACCENT if self.mv.looping else icon_color(self))
        self.transport._layout()
        self.view.refresh_display()
        if request:
            self.request_frame()

    def set_view(self, **changes: Any) -> None:
        for name, value in changes.items():
            setattr(self.mv, name, value)
        self._save_view()
        self._apply_view()

    def set_zoom(self, zoom: float | None) -> None:
        self.mv.set_zoom(zoom)
        self._save_view()
        self._apply_view()

    def set_resolution(self, res: str, paused: bool = False) -> None:
        self.set_view(**{"paused_res" if paused else "res": res})

    def _view_changed(self) -> None:
        self.zoom_button.setText(self.mv.zoom_label())
        self._save_view()
        self.request_frame()

    def _hidden_changed(self, hidden: list[str]) -> None:
        self.mv.hidden = list(hidden)
        self._save_view()

    def _fill_zoom_menu(self) -> None:
        m = self.zoom_button.menu()
        m.clear()
        group = QActionGroup(m)
        items = [("fit", tr("Fit"), None), *((k, f"{k}%", z) for k, z in ZOOMS)]
        for i, (key, label, z) in enumerate(items):
            a = QAction(label, m, checkable=True)
            a.setObjectName(f"{self.target}.zoom.{key}")
            a.setChecked(self.mv.zoom is None if z is None else self.mv.zoom is not None and abs(self.mv.zoom - z) < 1e-6)
            a.triggered.connect(lambda _c=False, z=z: self.set_zoom(z))
            group.addAction(a)
            m.addAction(a)
            if i == 0:
                m.addSeparator()

    def _res_actions(self, menu: QMenu, paused: bool) -> None:
        group = QActionGroup(menu)
        current = self.mv.paused_res if paused else self.mv.res
        for key, label, _scale in RESOLUTIONS:
            a = QAction(tr(label), menu, checkable=True)
            a.setObjectName(f"{self.target}.{'pausedRes' if paused else 'playbackRes'}.{key}")
            a.setChecked(key == current)
            a.triggered.connect(lambda _c=False, k=key: self.set_resolution(k, paused))
            group.addAction(a)
            menu.addAction(a)

    def _fill_res_menu(self) -> None:
        m = self.res_button.menu()
        m.clear()
        self._res_actions(m, paused=False)

    def _fill_wrench_menu(self) -> None:
        m = self.wrench.menu()
        m.clear()

        def display(key: str, label: str, menu: QMenu) -> None:
            a = menu.addAction(tr(label))
            a.setObjectName(f"{self.target}.settings.display.{key}")
            a.setCheckable(True)
            a.setChecked(self.mv.display == key)
            a.triggered.connect(lambda _c=False, k=key: self.set_view(display=k))

        display("composite", "Composite Video", m)
        channels = m.addMenu(tr("RGBA Channels"))
        for key, label in DISPLAYS[1:]:
            display(key, label, channels)
        if self.target == "source":
            for key, label in SOURCE_DISPLAYS:
                display(key, label, m)
        m.addSeparator()
        self._res_actions(m.addMenu(tr("Playback Resolution")), paused=False)
        self._res_actions(m.addMenu(tr("Paused Resolution")), paused=True)

        def toggle(name: str, label: str, attr: str) -> None:
            a = m.addAction(tr(label))
            a.setObjectName(f"{self.target}.settings.{name}")
            a.setCheckable(True)
            a.setChecked(bool(getattr(self.mv, attr)))
            a.triggered.connect(lambda on: self.set_view(**{attr: bool(on)}))

        toggle("highQualityPlayback", N_("High Quality Playback"), "high_quality")
        m.addSeparator()
        toggle("safeMargins", N_("Safe Margins"), "safe_margins")
        toggle("showTransport", N_("Show Transport Controls"), "show_transport")
        toggle("loop", N_("Loop") if self.target == "program" else N_("Loop marked range"), "looping")

    # ----- shared model -----
    def _range_marks(self) -> tuple[int, int]:
        """The marked range (In to the end of Out, or the whole), egui's duration and loop range."""
        start, end = self.range()
        mark_in, mark_out = self.marks()
        lo = mark_in if mark_in is not None else start
        hi = mark_out + self.frame() if mark_out is not None else end
        return lo, max(lo, hi)

    def _content_changed(self) -> None:
        if not self.has_content():
            self.stop()
            self.view.set_image(None, self.empty_message())
            self.duration_label.setText("")
            self.scrub.set_range(0, 0, None, None, 1)
            return
        start, end = self.range()
        mark_in, mark_out = self.marks()
        lo, hi = self._range_marks()
        self.duration_label.setText(format_timecode(max(0, hi - lo), self.frame(), self.drop_frame()))
        self.scrub.set_range(start, end, mark_in, mark_out, self.frame())
        self.scrub.set_playhead(self.playhead)
        self._update_timecode()
        self.request_frame()

    def set_playhead(self, ticks: int) -> None:
        """Show ``ticks`` (already the engine's playhead, or a local scrub)."""
        start = self.range()[0] if self.has_content() else 0
        self.playhead = max(start, int(ticks))
        self._update_timecode()
        self.scrub.set_playhead(self.playhead)
        self.request_frame()

    def seek(self, ticks: int) -> None:
        """Move the playhead here and tell the engine."""
        if not self.has_content():
            return
        if self.playing:
            self.stop(notify=False)
        start, end = self.range()
        t = min(max(start, int(ticks)), max(start, end - self.frame()))
        self.set_playhead(t)
        command, params = self.seek_command(t)
        self.command.emit(command, params)

    def step(self, frames: int) -> None:
        if self.has_content():
            ft = self.frame()
            self.seek((self.playhead // ft + frames) * ft)

    def go_to_in(self) -> None:
        if self.has_content():
            mark_in = self.marks()[0]
            self.seek(mark_in if mark_in is not None else self.range()[0])

    def go_to_out(self) -> None:
        if self.has_content():
            mark_out = self.marks()[1]
            self.seek(mark_out if mark_out is not None else self.range()[1] - self.frame())

    def _update_timecode(self) -> None:
        content = self.has_content()
        self.timecode.show_time(format_timecode(self.playhead, self.frame() if content else 1, content and self.drop_frame()))

    def _typed_time(self, text: str) -> None:
        if not self.has_content() or not self.engine.running:
            return
        params: dict[str, Any] = {"text": text, "time": int(self.playhead)}  # "+5" from the shown frame
        if self.target != "program":
            params["target"] = self.target

        def done(result: Any, _data: bytes | None) -> None:
            if isinstance(result, dict) and isinstance(result.get("time"), int):
                self.timecode.clearFocus()
                self.seek(result["time"])

        def failed(msg: str) -> None:
            self.timecode.selectAll()
            self.message.emit(f"{tr('Timecode')}: {msg}")

        self.engine.call("time.parse", params, on_reply=done, on_error=failed)

    def run_transport(self, command: str) -> None:
        """A transport button (or its entry in the overflow menu) was clicked."""
        target = {"target": self.target}
        # while playing, the engine's playhead is still where playback started: mark the shown frame
        at = {**target, "time": int(self.playhead)} if self.playing else target
        if command in ("markers.add", "markers.markIn", "markers.markOut"):
            self.command.emit(command, at)
        elif command in ("src.markIn", "src.markOut"):
            self.command.emit("markers." + command.split(".")[1], at)
        elif command in ("markers.goToIn", "src.goIn"):
            self.go_to_in()
        elif command in ("markers.goToOut", "src.goOut"):
            self.go_to_out()
        elif command in ("playhead.stepBack", "src.stepBack"):
            self.step(-1)
        elif command in ("playhead.stepForward", "src.stepFwd"):
            self.step(1)
        elif command in ("playback.toggle", "source.playback.toggle"):
            self.toggle_playback()
        elif command == "exportFrame":
            self.run.emit("file.exportFrame", {**target, "time": self.playhead})
        elif command == "playback.loop":
            self.set_view(looping=not self.mv.looping)
        else:  # sequence.lift / extract, source.insert / overwrite, media.toggleProxies
            self.command.emit(command, {})

    # ----- frames -----
    def request_frame(self, at: int | None = None) -> None:
        if not self.has_content():
            return
        self._frame_wanted = self.playhead if at is None else at
        self._pump_frame()

    def _pump_frame(self) -> None:
        if self._frame_busy or self._frame_wanted is None or not self.engine.running:
            return
        t = self._frame_wanted
        width, height = self.view.wanted_size()
        scale = self.mv.scale(self.playing)
        key = (t, self.revision, width, height, scale, self.mv.display, self.content_key())
        self._frame_wanted = None
        if key == self._shown:
            return
        self._frame_busy = True
        params: dict[str, Any] = {"time": t, "maxWidth": width, "maxHeight": height, "scale": scale}
        if self.target != "program":
            params["target"] = self.target
        if self.mv.display == "alpha":
            params["alpha"] = True  # the frame's own alpha, not the opaque monitor picture
        self.engine.call(
            "frame.render",
            params,
            on_reply=lambda r, b, k=key: self._on_frame(k, r, b),
            on_error=self._on_frame_error,
        )

    def _on_frame(self, key: tuple[Any, ...], result: Any, data: bytes | None) -> None:
        self._frame_busy = False
        if isinstance(result, dict) and data:
            w, h = int(result.get("w") or 0), int(result.get("h") or 0)
            if w > 0 and h > 0 and len(data) == w * h * 4:
                image = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
                offline = result.get("offline") or []
                full = (int(result.get("fullW") or 0), int(result.get("fullH") or 0))
                par = result.get("pixelAspect")
                self.view.set_image(image, tr("Media offline") if offline else "", full, float(par) if isinstance(par, (int, float)) else None)
                self._shown = key
        self._pump_frame()

    def _on_frame_error(self, message: str) -> None:
        self._frame_busy = False
        self.view.set_image(None, message)
        self._pump_frame()

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self.request_frame()

    # ----- playback -----
    def toggle_playback(self) -> None:
        if self.playing:
            self.stop()
        else:
            self.play()

    def play(self, speed: float = 1.0) -> None:
        """Play at ``speed`` (negative: backwards) from the playhead."""
        if not self.has_content():
            return
        if self.playing:
            self._halt()
        start, end = self.range()
        if speed > 0 and end and self.playhead >= end - self.frame():
            self.playhead = self._range_marks()[0] if self.mv.looping else start
        if speed < 0 and self.playhead <= start:
            return
        self.playing = True
        self.speed = float(speed)
        self.dropped = 0
        self.view.dropped = 0
        self.transport.set_icon("toggle", "pause")
        self._play_from = self.playhead
        self._clock.start()
        if self.speed == 1.0:
            self._start_audio()
            if self.target == "program" and not self._automation_pass:
                # Audio Track Mixer: an automation pass runs while playing forward in real time (egui's)
                self._automation_pass = True
                self.command.emit("mixer.recordStart", {"time": int(self.playhead)})
        self._timer.start()

    def shuttle(self, direction: int) -> None:
        """J / L: play backwards / forwards; again in the same direction doubles the speed, up
        to 8× (egui's playback.reverse / forward)."""
        if self.playing and self.speed * direction > 0:
            speed = min(MAX_SPEED, max(-MAX_SPEED, self.speed * 2))
        else:
            speed = float(direction)
        self.stop_at = self.rest_at = None
        self.play(speed)

    def play_in_to_out(self) -> None:
        """Play the marked range once and rest at Out (egui's playback.inToOut)."""
        if not self.has_content():
            return
        mark_in, mark_out = self.marks()
        if self.playing:
            self.stop(notify=False)
        if mark_in is not None:
            self.set_playhead(mark_in)
        self.play(1.0)
        self.stop_at = mark_out + self.frame() if mark_out is not None else None
        self.rest_at = mark_out

    def _halt(self) -> None:
        self.playing = False
        self._timer.stop()
        self._stop_audio()
        if self._automation_pass:  # writes the pass's fader moves (also on a loop or a speed change)
            self._automation_pass = False
            self.command.emit("mixer.recordStop", {"time": int(self.playhead)})

    def stop(self, notify: bool = True) -> None:
        if not self.playing:
            return
        self._halt()
        self.speed = 1.0
        self.view.dropped = None
        self.view.update()
        self.transport.set_icon("toggle", "play")
        if self.rest_at is not None:
            self.playhead = self.rest_at
        self.stop_at = self.rest_at = None
        # snap to the frame grid and tell the engine where we stopped
        ft = self.frame()
        self.playhead = (self.playhead // ft) * ft
        self._update_timecode()
        self.scrub.set_playhead(self.playhead)
        self.request_frame()  # at the paused resolution
        if notify:
            command, params = self.seek_command(self.playhead)
            self.command.emit(command, params)

    def _tick(self) -> None:
        if not self.playing or not self.has_content():
            return
        elapsed = self._clock.nsecsElapsed()
        t = self._play_from + int(elapsed * TICKS_PER_SECOND * self.speed) // 1_000_000_000
        start, end = self.range()
        ft = self.frame()
        if self.speed > 0:
            limit = self.stop_at if self.stop_at is not None else (self._range_marks()[1] if self.mv.looping else end)
            if limit and t >= limit:
                if self.mv.looping and self.stop_at is None:
                    lo = self._range_marks()[0]
                    self._halt()
                    self.playhead = lo
                    self.play(self.speed)
                    return
                self.playhead = max(start, limit - ft)
                self.stop()
                return
        elif t <= start:
            self.playhead = start
            self.stop()
            return
        frame_t = (t // ft) * ft
        if frame_t != self.playhead:
            if self._frame_busy:
                self.dropped += 1
                if self.view.dropped == 0:
                    self.view.dropped = self.dropped
                    self.view.update()
            self.playhead = frame_t
            self._update_timecode()
            self.scrub.set_playhead(frame_t)
            self.playhead_moved.emit(frame_t)
            self.request_frame(frame_t)
        self._feed_audio()

    # ----- audio -----
    def play_sample(self) -> int:
        """The sample being heard now (by the playback clock), at ``audio_rate``."""
        if not self.playing:
            return self.playhead * self._audio_rate // TICKS_PER_SECOND
        t = self._play_from + self._clock.nsecsElapsed() * TICKS_PER_SECOND // 1_000_000_000
        return t * self._audio_rate // TICKS_PER_SECOND

    @property
    def audio_rate(self) -> int:
        return self._audio_rate

    def _start_audio(self) -> None:
        if not self.has_content():
            return
        rate = self.sample_rate()
        self._audio_rate = rate
        self._audio_channels = 2
        self._audio_next = self.playhead * rate // TICKS_PER_SECOND
        self._audio_pending = b""
        self._audio_busy = False
        self.meter_blocks.clear()
        self.strip_blocks.clear()
        self.audio_generation += 1
        if HAVE_AUDIO:
            self._open_sink(rate)
        self._feed_audio()

    def _open_sink(self, rate: int) -> None:
        fmt = QAudioFormat()
        fmt.setSampleRate(rate)
        fmt.setChannelCount(2)
        fmt.setSampleFormat(QAudioFormat.SampleFormat.Float)
        device = QMediaDevices.defaultAudioOutput()
        if device.isNull() or not device.isFormatSupported(fmt):
            return  # silent: the meters still play
        self._sink = QAudioSink(device, fmt, self)
        self._sink.setBufferSize(int(rate * 2 * 4 * AUDIO_LEAD_SECONDS * 2))
        self._sink_io = self._sink.start()

    def _stop_audio(self) -> None:
        if self._sink is not None:
            self._sink.stop()
            self._sink.deleteLater()
        self._sink = None
        self._sink_io = None
        self._audio_pending = b""

    def _feed_audio(self) -> None:
        # Audio plays at normal speed only (play() starts it then); shuttle speeds stay silent.
        if not self.playing or self.speed != 1.0 or not self.engine.running:
            return
        if self._sink is not None and self._sink_io is not None:
            if self._audio_pending:
                n = self._sink_io.write(self._audio_pending)
                self._audio_pending = self._audio_pending[max(0, n) :]
            frame_bytes = 4 * self._audio_channels
            queued = (self._sink.bufferSize() - self._sink.bytesFree()) // frame_bytes + len(
                self._audio_pending
            ) // frame_bytes
        else:  # no device: keep just ahead of the clock, for the meters
            queued = self._audio_next - self.play_sample()
        if self._audio_busy or queued > self._audio_rate * AUDIO_LEAD_SECONDS:
            return
        frames = int(self._audio_rate * AUDIO_CHUNK_SECONDS)
        self._audio_busy = True
        start = self._audio_next
        self._audio_next += frames
        self.engine.call(
            "audio.render",
            {**self.audio_params(start, frames), "meter": True},
            on_reply=self._on_audio,
            on_error=lambda _m: setattr(self, "_audio_busy", False),
        )

    def _on_audio(self, result: Any, data: bytes | None) -> None:
        self._audio_busy = False
        if not isinstance(result, dict):
            return
        meter = result.get("meter")
        if self.playing and isinstance(meter, dict) and isinstance(result.get("start"), int):
            self.meter_blocks.append(
                (int(result["start"]), max(1, int(meter.get("block") or 1)), list(meter.get("peaks") or []), list(meter.get("loudness") or []))
            )
            while len(self.meter_blocks) > max(4, int(METER_KEEP_SECONDS / AUDIO_CHUNK_SECONDS)):
                self.meter_blocks.popleft()
            strips = meter.get("strips")
            if isinstance(strips, list):
                self.strip_blocks.append(
                    (int(result["start"]), int(result.get("frames") or 0), {s.get("id"): list(s.get("peaks") or []) for s in strips if isinstance(s, dict)})
                )
                while len(self.strip_blocks) > max(4, int(METER_KEEP_SECONDS / AUDIO_CHUNK_SECONDS)):
                    self.strip_blocks.popleft()
        if self._sink_io is None or not data:
            self._feed_audio()
            return
        if int(result.get("channels") or 0) != self._audio_channels:
            return
        self._audio_pending += data
        self._feed_audio()


class ProgramMonitor(Monitor):
    """The active sequence at the sequence playhead."""

    target = "program"

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(engine, parent)
        self.sequence: dict[str, Any] | None = None
        self.view.set_image(None, self.empty_message())

    def set_sequence(self, seq: dict[str, Any] | None, revision: int) -> None:
        self.sequence = seq
        self.revision = revision
        if seq is not None and not self.playing and isinstance(seq.get("playhead"), int):
            self.playhead = seq["playhead"]
        self._content_changed()

    def has_content(self) -> bool:
        return self.sequence is not None

    def range(self) -> tuple[int, int]:
        return 0, int((self.sequence or {}).get("duration") or 0)

    def marks(self) -> tuple[int | None, int | None]:
        seq = self.sequence or {}
        mark_in, mark_out = seq.get("in"), seq.get("out")
        return (mark_in if isinstance(mark_in, int) else None, mark_out if isinstance(mark_out, int) else None)

    def frame(self) -> int:
        return frame_ticks((self.sequence or {}).get("settings"))

    def drop_frame(self) -> bool:
        return drop_frame((self.sequence or {}).get("settings"))

    def content_key(self) -> Any:
        return (self.sequence or {}).get("id")

    def empty_message(self) -> str:
        return tr("No sequence")

    def seek_command(self, t: int) -> tuple[str, dict[str, Any]]:
        return "playhead.set", {"time": t}

    def sample_rate(self) -> int:
        return int(((self.sequence or {}).get("settings") or {}).get("sample_rate") or 48000)


class SourceMonitor(Monitor):
    """The clip opened in the Source monitor (``source.open``), at the Source playhead."""

    target = "source"
    BUTTONS = SOURCE_BUTTONS

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(engine, parent)
        self.source: dict[str, Any] | None = None
        self.view.set_image(None, self.empty_message())
        self.view.drag_out.connect(lambda: self.start_source_drag(self.view, True, True))
        # the audio waveform (egui's Audio Waveform / Video and Audio Waveform Split, and any
        # clip without video)
        self.media = TimelineMedia(self)
        self.media.attach(engine)
        self.wave = WaveformView(self)
        self.media.updated.connect(self.wave.update)
        layout = self.layout()
        if isinstance(layout, QVBoxLayout):
            layout.insertWidget(1, self.wave, 1)
        self.wave.hide()
        # Drag Video Only / Drag Audio Only / Drag Video and Audio (egui's source_drag::controls)
        self.drag_buttons: dict[str, SourceDragButton] = {}
        for i, (key, tip, video, audio) in enumerate(
            (
                ("video", N_("Drag Video Only"), True, False),
                ("audio", N_("Drag Audio Only"), False, True),
                ("both", N_("Drag Video and Audio"), True, True),
            )
        ):
            b = SourceDragButton(self, key, video, audio)
            b.setToolTip(tr(tip))
            self.controls.insertWidget(3 + i, b)
            self.drag_buttons[key] = b
        # the recently opened clips, then Close / Close All (egui's Source panel menu, #313)
        self.clips_button = QToolButton(self)
        self.clips_button.setObjectName("source.clips")
        self.clips_button.setToolTip(tr("Source clips"))
        self.clips_button.setText("≡")
        self.clips_button.setAutoRaise(True)
        self.clips_button.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.clips_button.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.clips_button.setStyleSheet("QToolButton::menu-indicator { image: none; }")
        self.clips_button.setMenu(QMenu(self.clips_button))
        self.clips_button.menu().aboutToShow.connect(self._fill_clips_menu)
        self.controls.insertWidget(0, self.clips_button)
        self._apply_wave()

    def _fill_clips_menu(self) -> None:
        m = self.clips_button.menu()
        m.clear()
        current = (self.source or {}).get("item")
        recent = [r for r in (self.source or self._recent_only or {}).get("recent") or [] if isinstance(r, dict) and isinstance(r.get("item"), int)]
        for r in recent:
            a = m.addAction(str(r.get("name") or ""))
            a.setObjectName(f"source.clips.recent.{r['item']}")
            a.setCheckable(True)
            a.setChecked(r["item"] == current)
            if r["item"] != current:
                a.triggered.connect(lambda _c=False, i=r["item"]: self.command.emit("source.open", {"item": i}))
        if recent:
            m.addSeparator()
        for cid, key, label in (("source.close", "close", N_("Close")), ("source.closeAll", "closeAll", N_("Close All"))):
            a = m.addAction(tr(label))
            a.setObjectName(f"source.clips.{key}")
            a.setEnabled(current is not None)
            a.triggered.connect(lambda _c=False, c=cid: self.command.emit(c, {}))

    def has_video(self) -> bool:
        return (self.source or {}).get("hasVideo") is not False

    def has_audio(self) -> bool:
        return (self.source or {}).get("hasAudio") is True

    def _apply_wave(self) -> None:
        """Picture, waveform or both, as egui's Source monitor shows them."""
        shown = self.source is not None
        wave_only = shown and (not self.has_video() or self.mv.display == "audioWaveform")
        split = shown and not wave_only and self.mv.display == "videoAndWaveform"
        self.view.setVisible(not wave_only)
        self.wave.setVisible(wave_only or split)
        layout = self.layout()
        if isinstance(layout, QVBoxLayout):
            layout.setStretch(0, 62 if split else 1)
            layout.setStretch(1, 38 if split else 1)
        for key, b in self.drag_buttons.items():
            b.setEnabled(shown and (self.has_video() if key != "audio" else True) and (self.has_audio() if key != "video" else True))
        self.wave.update()

    def _apply_view(self, request: bool = True) -> None:
        super()._apply_view(request)
        if hasattr(self, "wave"):
            self._apply_wave()

    def _update_timecode(self) -> None:
        super()._update_timecode()
        if hasattr(self, "wave") and self.wave.isVisible():
            self.wave.update()

    def selected_span(self) -> tuple[int, int] | None:
        """The marked span a drag carries (egui's ``selected_range``): (start, duration)."""
        sel = (self.source or {}).get("selected")
        if not isinstance(sel, dict):
            return None
        start, dur = sel.get("start"), sel.get("duration")
        ok = all(isinstance(v, int) and not isinstance(v, bool) for v in (start, dur))
        return (int(start), int(dur)) if ok and int(dur) > 0 and int(start) >= 0 else None

    def source_payload(self, video: bool, audio: bool) -> dict[str, Any] | None:
        src = self.source or {}
        media = src.get("media")
        span = self.selected_span()
        video, audio = video and self.has_video(), audio and self.has_audio()
        if not isinstance(media, int) or isinstance(media, bool) or span is None or not (video or audio):
            return None
        return {"item": media, "sourceIn": span[0], "duration": span[1], "video": video, "audio": audio}

    def start_source_drag(self, widget: QWidget, video: bool, audio: bool) -> bool:
        """Lift the marked span (video, audio or both) to drop on the Timeline."""
        payload = self.source_payload(video, audio)
        if payload is None:
            return False
        if self.playing:
            self.stop()
        mime = QMimeData()
        mime.setData(SOURCE_MIME, json.dumps(payload).encode())
        drag = QDrag(widget)
        drag.setMimeData(mime)
        drag.exec(Qt.DropAction.CopyAction)
        return True

    _recent_only: dict[str, Any] | None = None  # the reply with nothing open (its recent clips)

    def set_source(self, state: dict[str, Any] | None, revision: int) -> None:
        """``state`` is the ``source.inspect`` reply (``item`` null when nothing is open)."""
        previous = (self.source or {}).get("item")
        self.source = state if isinstance(state, dict) and state.get("item") is not None else None
        self._recent_only = state if isinstance(state, dict) and self.source is None else None
        self.revision = revision
        if self.source is not None and not self.playing:
            ph = self.source.get("playhead")
            if isinstance(ph, int):
                self.playhead = ph
            elif self.source.get("item") != previous:
                self.playhead = int(self.source.get("start") or 0)
        if self.source is None or self.source.get("item") != previous:
            self.stop(notify=False)
        self._content_changed()
        self._apply_wave()

    def has_content(self) -> bool:
        return self.source is not None

    def range(self) -> tuple[int, int]:
        src = self.source or {}
        start = int(src.get("start") or 0)
        return start, max(start, int(src.get("end") or 0))

    def marks(self) -> tuple[int | None, int | None]:
        src = self.source or {}
        mark_in, mark_out = src.get("markIn"), src.get("markOut")
        return (mark_in if isinstance(mark_in, int) else None, mark_out if isinstance(mark_out, int) else None)

    def frame(self) -> int:
        return ticks_per_frame_from_fps((self.source or {}).get("fps"))

    def content_key(self) -> Any:
        return (self.source or {}).get("item")

    def empty_message(self) -> str:
        return tr("Double-click a clip in the Project panel to open it here.")

    def seek_command(self, t: int) -> tuple[str, dict[str, Any]]:
        return "source.setPlayhead", {"time": t}

    def audio_params(self, start: int, frames: int) -> dict[str, Any]:
        return {"target": "source", "start": start, "frames": frames, "sampleRate": SOURCE_SAMPLE_RATE}

    def sample_rate(self) -> int:
        return SOURCE_SAMPLE_RATE


class SourceDragButton(QToolButton):
    """Drag Video Only / Audio Only / Video and Audio: pressed and dragged, it lifts the marked
    span of the Source clip with those streams."""

    def __init__(self, monitor: SourceMonitor, key: str, video: bool, audio: bool) -> None:
        super().__init__(monitor)
        self.monitor = monitor
        self.video, self.audio = video, audio
        self.setObjectName(f"source.drag.{key}")
        self.setAutoRaise(True)
        self.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.setCursor(Qt.CursorShape.OpenHandCursor)
        self.setFixedSize(30 if video and audio else 22, 22)
        self._press: QPointF | None = None

    def paintEvent(self, e: QPaintEvent) -> None:
        super().paintEvent(e)
        from .project import kind_icon  # the Project panel's film and audio glyphs

        p = QPainter(self)
        color = icon_color(self) if self.isEnabled() else theme.current().c("text_dim")
        kinds = [k for k, on in (("film", self.video), ("audio", self.audio)) if on]
        w = 14
        x = (self.width() - w * len(kinds)) // 2
        for k in kinds:
            kind_icon(k, color, 14).paint(p, QRect(x, (self.height() - 14) // 2, 14, 14))
            x += w
        p.end()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        self._press = e.position() if e.button() == Qt.MouseButton.LeftButton else None
        super().mousePressEvent(e)

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._press is not None and (e.position() - self._press).manhattanLength() >= QApplication.startDragDistance():
            self._press = None
            self.setDown(False)
            self.monitor.start_source_drag(self, self.video, self.audio)
            return
        super().mouseMoveEvent(e)

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        self._press = None
        super().mouseReleaseEvent(e)


class WaveformView(QWidget):
    """The Source clip's audio waveform (egui's ``monitor_view::waveform``): In / Out shading,
    the playhead; a click or a sideways drag scrubs, a downward drag lifts the marked audio."""

    COLOR = QColor(0x4F, 0xC3, 0x7A)

    def __init__(self, monitor: SourceMonitor) -> None:
        super().__init__(monitor)
        self.monitor = monitor
        self.setObjectName("source.waveform")
        self.setMinimumHeight(40)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self._press: QPointF | None = None

    def span(self) -> tuple[int, int]:
        start, end = self.monitor.range()
        return start, max(start + 1, end)

    def x_of(self, t: int) -> float:
        start, end = self.span()
        return min(1.0, max(0.0, (t - start) / (end - start))) * self.width()

    def time_at(self, x: float) -> int:
        start, end = self.span()
        f = min(1.0, max(0.0, x / max(1, self.width())))
        return start + int((end - start) * f)

    def peaks(self) -> list[tuple[float, float]] | None:
        src = self.monitor.source or {}
        start, end = self.span()
        return self.monitor.media.waveform(src.get("media"), start, end)

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        p.fillRect(self.rect(), T.c("monitor_bg"))
        if self.monitor.source is None:
            p.end()
            return
        w, h = self.width(), self.height()
        mark_in, mark_out = self.monitor.marks()
        start, end = self.span()
        if mark_in is not None or mark_out is not None:
            a, b = self.x_of(mark_in if mark_in is not None else start), self.x_of(mark_out if mark_out is not None else end)
            p.fillRect(QRectF(a, 0, max(0.0, b - a), h), QColor(255, 255, 255, 14))
        mid = h / 2
        p.setPen(QPen(QColor(255, 255, 255, 30), 1))
        p.drawLine(QPointF(0, mid), QPointF(w, mid))
        self.monitor.media.begin_paint()
        peaks = self.peaks()
        if peaks:
            half = h * 0.45
            n = len(peaks)
            for x in range(w):
                i0 = min(n - 1, x * n // max(1, w))
                i1 = max(i0 + 1, min(n, (x + 1) * n // max(1, w)))
                lo = min(pk[0] for pk in peaks[i0:i1])
                hi = max(pk[1] for pk in peaks[i0:i1])
                if hi - lo > 0.002:
                    p.fillRect(QRectF(x, mid - min(1.0, hi) * half, 1, (min(1.0, hi) - max(-1.0, lo)) * half + 1), self.COLOR)
        else:
            p.setPen(T.c("text_dim"))
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter, tr("(no audio)") if peaks is not None else tr("Generating waveform…"))
        x = self.x_of(self.monitor.playhead)
        p.setPen(QPen(T.c("playhead"), 1))
        p.drawLine(QPointF(x, 0), QPointF(x, h))
        p.end()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton and self.monitor.source is not None:
            self._press = e.position()
            self.monitor.seek(self.time_at(e.position().x()))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._press is None:
            return
        d = e.position() - self._press
        if abs(d.y()) > max(abs(d.x()), 6.0):
            # dragging down lifts the marked audio to the Timeline
            self._press = None
            self.monitor.start_source_drag(self, False, True)
            return
        self.monitor.seek(self.time_at(e.position().x()))

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self._press = None
