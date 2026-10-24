"""Program and Source monitors, with playback.

Both monitors share one implementation (``Monitor``) and differ only in what they show:
the Program monitor shows the active sequence at the sequence playhead, the Source monitor the
clip opened with ``source.open`` at the Source playhead (``target: "source"`` in the engine
protocol). Frames come from the engine (``frame.render``, RGBA8 at the monitor's size); sound
from ``audio.render`` (interleaved float32) into a QAudioSink. Requests are coalesced: at most one
frame and one audio request are in flight, and a newer wanted time replaces an older one, so a
slow render drops frames instead of building a queue.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QElapsedTimer, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QImage, QMouseEvent, QPainter, QPaintEvent
from PySide6.QtWidgets import QHBoxLayout, QLabel, QSizePolicy, QToolButton, QVBoxLayout, QWidget

from ..engine import EngineClient
from ..i18n import tr
from ..timecode import TICKS_PER_SECOND, format_timecode, frame_ticks

try:  # QtMultimedia is optional: without it playback is silent
    from PySide6.QtMultimedia import QAudioFormat, QAudioSink, QMediaDevices

    HAVE_AUDIO = True
except ImportError:  # pragma: no cover - depends on the PySide6 build
    HAVE_AUDIO = False

AUDIO_CHUNK_SECONDS = 0.1
AUDIO_LEAD_SECONDS = 0.35
SOURCE_SAMPLE_RATE = 48_000


def ticks_per_frame_from_fps(fps: Any) -> int:
    """Frame length in ticks for a float frame rate (``source.inspect`` reports ``fps``)."""
    try:
        value = float(fps)
    except (TypeError, ValueError):
        value = 0.0
    if not 1.0 <= value <= 1000.0:
        value = 30.0
    return max(1, round(TICKS_PER_SECOND / value))


class FrameView(QWidget):
    def __init__(self, name: str = "program", parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"{name}.view")
        self.image: QImage | None = None
        self.message = ""
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setMinimumSize(160, 90)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)

    def set_image(self, image: QImage | None, message: str = "") -> None:
        self.image = image
        self.message = message
        self.update()

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.fillRect(self.rect(), QColor("#111111"))
        if self.image is not None and not self.image.isNull():
            iw, ih = self.image.width(), self.image.height()
            scale = min(self.width() / iw, self.height() / ih)
            w, h = iw * scale, ih * scale
            target = QRectF((self.width() - w) / 2, (self.height() - h) / 2, w, h)
            p.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            p.drawImage(target, self.image)
        if self.message:
            p.setPen(QColor("#cccccc"))
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter | Qt.TextFlag.TextWordWrap, self.message)


class ScrubBar(QWidget):
    """The monitor's time bar: range, In/Out shading, playhead. Click or drag to seek."""

    seek = Signal(int)

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
        p.fillRect(self.rect(), QColor("#1c1c1c"))
        if self.end <= self.start:
            return
        h = self.height()
        p.fillRect(QRectF(4, h / 2 - 2, self.width() - 8, 4), QColor("#3a3a3a"))
        if self.mark_in is not None or self.mark_out is not None:
            a = self._x(self.mark_in if self.mark_in is not None else self.start)
            b = self._x(self.mark_out if self.mark_out is not None else self.end)
            p.fillRect(QRectF(a, 2, max(1.0, b - a), h - 4), QColor(90, 140, 220, 110))
        x = self._x(self.playhead)
        p.fillRect(QRectF(x - 1, 0, 2, h), QColor("#e04040"))

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.seek.emit(self.time_at(e.position().x()))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if e.buttons() & Qt.MouseButton.LeftButton:
            self.seek.emit(self.time_at(e.position().x()))


class Monitor(QWidget):
    """Shared monitor: frame view, time bar, transport, marks, playback with sound."""

    target = "program"
    command = Signal(str, dict)
    playhead_moved = Signal(int)  # local playhead during playback

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"panel.{self.target.capitalize()}")
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.engine = engine
        self.playhead = 0
        self.revision = -1
        self._frame_busy = False
        self._frame_wanted: int | None = None
        self._shown: tuple[int, int, int, Any] | None = None  # (time, revision, width, item)
        self.view = FrameView(self.target, self)
        self.scrub = ScrubBar(self.target, self)
        self.scrub.seek.connect(self.seek)
        self.timecode = QLabel("00:00:00:00", self)
        self.timecode.setObjectName(f"{self.target}.timecode")
        self.duration_label = QLabel("", self)

        bar = QHBoxLayout()
        bar.setContentsMargins(4, 2, 4, 2)
        bar.addWidget(self.timecode)
        bar.addStretch(1)
        self.buttons: dict[str, QToolButton] = {}
        transport = (
            ("markIn", "{", "Mark In", lambda: self.command.emit("markers.markIn", {"target": self.target})),
            ("markOut", "}", "Mark Out", lambda: self.command.emit("markers.markOut", {"target": self.target})),
            ("start", "⏮", "Go to Start", lambda: self.seek(self.range()[0])),
            ("stepBack", "◀|", "Step Back 1 Frame", lambda: self.step(-1)),
            ("toggle", "▶", "Play/Stop", self.toggle_playback),
            ("stepForward", "|▶", "Step Forward 1 Frame", lambda: self.step(1)),
            ("end", "⏭", "Go to End", lambda: self.seek(self.range()[1] - self.frame())),
            *self.extra_buttons(),
        )
        for name, text, tip, slot in transport:
            b = QToolButton(self)
            b.setObjectName(f"{self.target}.transport.{name}")
            b.setText(text)
            b.setToolTip(tr(tip))
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            b.clicked.connect(slot)
            bar.addWidget(b)
            self.buttons[name] = b
        bar.addStretch(1)
        bar.addWidget(self.duration_label)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(self.view, 1)
        layout.addWidget(self.scrub)
        layout.addLayout(bar)

        # playback state
        self.playing = False
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

    # ----- what each monitor shows (overridden) -----
    def extra_buttons(self) -> tuple[tuple[str, str, str, Any], ...]:
        return ()

    def has_content(self) -> bool:
        raise NotImplementedError

    def range(self) -> tuple[int, int]:
        """(first tick, end tick) of what is shown."""
        raise NotImplementedError

    def marks(self) -> tuple[int | None, int | None]:
        return None, None

    def frame(self) -> int:
        raise NotImplementedError

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

    # ----- shared model -----
    def _content_changed(self) -> None:
        if not self.has_content():
            self.stop()
            self.view.set_image(None, self.empty_message())
            self.duration_label.setText("")
            self.scrub.set_range(0, 0, None, None, 1)
            return
        start, end = self.range()
        mark_in, mark_out = self.marks()
        self.duration_label.setText(format_timecode(max(0, end - start), self.frame()))
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

    def _update_timecode(self) -> None:
        self.timecode.setText(format_timecode(self.playhead, self.frame() if self.has_content() else 1))

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
        width = max(64, int(self.view.width() * self.view.devicePixelRatioF()))
        height = max(64, int(self.view.height() * self.view.devicePixelRatioF()))
        key = (t, self.revision, width, self.content_key())
        self._frame_wanted = None
        if key == self._shown:
            return
        self._frame_busy = True
        params: dict[str, Any] = {"time": t, "maxWidth": min(width, 8192), "maxHeight": min(height, 8192)}
        if self.target != "program":
            params["target"] = self.target
        self.engine.call(
            "frame.render",
            params,
            on_reply=lambda r, b, k=key: self._on_frame(k, r, b),
            on_error=self._on_frame_error,
        )

    def _on_frame(self, key: tuple[int, int, int, Any], result: Any, data: bytes | None) -> None:
        self._frame_busy = False
        if isinstance(result, dict) and data:
            w, h = int(result.get("w") or 0), int(result.get("h") or 0)
            if w > 0 and h > 0 and len(data) == w * h * 4:
                image = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
                offline = result.get("offline") or []
                self.view.set_image(image, tr("Media offline") if offline else "")
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

    def play(self) -> None:
        if not self.has_content() or self.playing:
            return
        start, end = self.range()
        if end and self.playhead >= end - self.frame():
            self.playhead = start
        self.playing = True
        self.buttons["toggle"].setText("■")
        self._play_from = self.playhead
        self._clock.start()
        self._start_audio()
        self._timer.start()

    def stop(self, notify: bool = True) -> None:
        if not self.playing:
            return
        self.playing = False
        self._timer.stop()
        self.buttons["toggle"].setText("▶")
        self._stop_audio()
        # snap to the frame grid and tell the engine where we stopped
        ft = self.frame()
        self.playhead = (self.playhead // ft) * ft
        if notify:
            command, params = self.seek_command(self.playhead)
            self.command.emit(command, params)

    def _tick(self) -> None:
        if not self.playing or not self.has_content():
            return
        elapsed = self._clock.nsecsElapsed()
        t = self._play_from + elapsed * TICKS_PER_SECOND // 1_000_000_000
        start, end = self.range()
        if end and t >= end:
            self.playhead = max(start, end - self.frame())
            self.stop()
            return
        ft = self.frame()
        frame_t = (t // ft) * ft
        if frame_t != self.playhead:
            self.playhead = frame_t
            self._update_timecode()
            self.scrub.set_playhead(frame_t)
            self.playhead_moved.emit(frame_t)
            self.request_frame(frame_t)
        self._feed_audio()

    # ----- audio -----
    def _start_audio(self) -> None:
        if not HAVE_AUDIO or not self.has_content():
            return
        rate = self.sample_rate()
        fmt = QAudioFormat()
        fmt.setSampleRate(rate)
        fmt.setChannelCount(2)
        fmt.setSampleFormat(QAudioFormat.SampleFormat.Float)
        device = QMediaDevices.defaultAudioOutput()
        if device.isNull() or not device.isFormatSupported(fmt):
            return
        self._audio_rate = rate
        self._audio_channels = 2
        self._sink = QAudioSink(device, fmt, self)
        self._sink.setBufferSize(int(rate * 2 * 4 * AUDIO_LEAD_SECONDS * 2))
        self._sink_io = self._sink.start()
        self._audio_next = self.playhead * rate // TICKS_PER_SECOND
        self._audio_pending = b""
        self._audio_busy = False
        self._feed_audio()

    def _stop_audio(self) -> None:
        if self._sink is not None:
            self._sink.stop()
            self._sink.deleteLater()
        self._sink = None
        self._sink_io = None
        self._audio_pending = b""

    def _feed_audio(self) -> None:
        if self._sink is None or self._sink_io is None:
            return
        if self._audio_pending:
            n = self._sink_io.write(self._audio_pending)
            self._audio_pending = self._audio_pending[max(0, n) :]
        frame_bytes = 4 * self._audio_channels
        queued = (self._sink.bufferSize() - self._sink.bytesFree()) // frame_bytes + len(
            self._audio_pending
        ) // frame_bytes
        if self._audio_busy or queued > self._audio_rate * AUDIO_LEAD_SECONDS:
            return
        frames = int(self._audio_rate * AUDIO_CHUNK_SECONDS)
        self._audio_busy = True
        start = self._audio_next
        self._audio_next += frames
        self.engine.call(
            "audio.render",
            self.audio_params(start, frames),
            on_reply=self._on_audio,
            on_error=lambda _m: setattr(self, "_audio_busy", False),
        )

    def _on_audio(self, result: Any, data: bytes | None) -> None:
        self._audio_busy = False
        if self._sink_io is None or not data or not isinstance(result, dict):
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

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(engine, parent)
        self.source: dict[str, Any] | None = None
        self.view.set_image(None, self.empty_message())

    def extra_buttons(self) -> tuple[tuple[str, str, str, Any], ...]:
        return (
            ("insert", "⤵", "Insert", lambda: self.command.emit("source.insert", {})),
            ("overwrite", "⇣", "Overwrite", lambda: self.command.emit("source.overwrite", {})),
        )

    def set_source(self, state: dict[str, Any] | None, revision: int) -> None:
        """``state`` is the ``source.inspect`` reply (``item`` null when nothing is open)."""
        previous = (self.source or {}).get("item")
        self.source = state if isinstance(state, dict) and state.get("item") is not None else None
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
