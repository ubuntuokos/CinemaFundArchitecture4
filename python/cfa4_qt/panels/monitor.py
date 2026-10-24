"""Program monitor and playback.

Frames come from the engine (``frame.render``, RGBA8 at the monitor's size); sound from
``audio.render`` (interleaved float32) into a QAudioSink. Requests are coalesced: at most one
frame and one audio request are in flight, and a newer wanted time replaces an older one, so a
slow render drops frames instead of building a queue.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QElapsedTimer, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QImage, QPainter, QPaintEvent
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


class FrameView(QWidget):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("program.view")
        self.image: QImage | None = None
        self.message = ""
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setMinimumSize(160, 90)

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
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter, self.message)


class ProgramMonitor(QWidget):
    command = Signal(str, dict)
    playhead_moved = Signal(int)  # local playhead during playback

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Program")
        self.engine = engine
        self.sequence: dict[str, Any] | None = None
        self.playhead = 0
        self.revision = -1
        self._frame_busy = False
        self._frame_wanted: int | None = None
        self._shown: tuple[int, int, int] | None = None  # (time, revision, width)
        self.view = FrameView(self)
        self.timecode = QLabel("00:00:00:00", self)
        self.timecode.setObjectName("program.timecode")
        self.duration_label = QLabel("", self)

        bar = QHBoxLayout()
        bar.setContentsMargins(4, 2, 4, 2)
        bar.addWidget(self.timecode)
        bar.addStretch(1)
        self.buttons: dict[str, QToolButton] = {}
        for name, text, tip, slot in (
            ("start", "⏮", "Go to Start", lambda: self.command.emit("playhead.start", {})),
            ("stepBack", "◀|", "Step Back 1 Frame", lambda: self.command.emit("playhead.stepBack", {})),
            ("toggle", "▶", "Play/Stop", self.toggle_playback),
            ("stepForward", "|▶", "Step Forward 1 Frame", lambda: self.command.emit("playhead.stepForward", {})),
            ("end", "⏭", "Go to End", lambda: self.command.emit("playhead.end", {})),
        ):
            b = QToolButton(self)
            b.setObjectName(f"program.transport.{name}")
            b.setText(text)
            b.setToolTip(tr(tip))
            b.clicked.connect(slot)
            bar.addWidget(b)
            self.buttons[name] = b
        bar.addStretch(1)
        bar.addWidget(self.duration_label)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.view, 1)
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

    # ----- model -----
    def set_sequence(self, seq: dict[str, Any] | None, revision: int) -> None:
        self.sequence = seq
        self.revision = revision
        if seq is None:
            self.stop()
            self.view.set_image(None, tr("No sequence"))
            self.duration_label.setText("")
            return
        if not self.playing and isinstance(seq.get("playhead"), int):
            self.playhead = seq["playhead"]
        self.duration_label.setText(format_timecode(int(seq.get("duration") or 0), self.frame()))
        self._update_timecode()
        self.request_frame()

    def frame(self) -> int:
        return frame_ticks((self.sequence or {}).get("settings"))

    def set_playhead(self, ticks: int) -> None:
        self.playhead = max(0, int(ticks))
        self._update_timecode()
        self.request_frame()

    def _update_timecode(self) -> None:
        self.timecode.setText(format_timecode(self.playhead, self.frame()))

    # ----- frames -----
    def request_frame(self, at: int | None = None) -> None:
        if self.sequence is None:
            return
        self._frame_wanted = self.playhead if at is None else at
        self._pump_frame()

    def _pump_frame(self) -> None:
        if self._frame_busy or self._frame_wanted is None or not self.engine.running:
            return
        t = self._frame_wanted
        width = max(64, int(self.view.width() * self.view.devicePixelRatioF()))
        height = max(64, int(self.view.height() * self.view.devicePixelRatioF()))
        key = (t, self.revision, width)
        self._frame_wanted = None
        if key == self._shown:
            return
        self._frame_busy = True
        self.engine.call(
            "frame.render",
            {"time": t, "maxWidth": min(width, 8192), "maxHeight": min(height, 8192)},
            on_reply=lambda r, b, k=key: self._on_frame(k, r, b),
            on_error=self._on_frame_error,
        )

    def _on_frame(self, key: tuple[int, int, int], result: Any, data: bytes | None) -> None:
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
        if self.sequence is None or self.playing:
            return
        duration = int(self.sequence.get("duration") or 0)
        if duration and self.playhead >= duration - self.frame():
            self.playhead = 0
        self.playing = True
        self.buttons["toggle"].setText("■")
        self._play_from = self.playhead
        self._clock.start()
        self._start_audio()
        self._timer.start()

    def stop(self) -> None:
        if not self.playing:
            return
        self.playing = False
        self._timer.stop()
        self.buttons["toggle"].setText("▶")
        self._stop_audio()
        # snap to the frame grid and tell the engine where we stopped
        ft = self.frame()
        self.playhead = (self.playhead // ft) * ft
        self.command.emit("playhead.set", {"time": self.playhead})

    def _tick(self) -> None:
        if not self.playing or self.sequence is None:
            return
        elapsed = self._clock.nsecsElapsed()
        t = self._play_from + elapsed * TICKS_PER_SECOND // 1_000_000_000
        duration = int(self.sequence.get("duration") or 0)
        if duration and t >= duration:
            self.playhead = max(0, duration - self.frame())
            self.stop()
            return
        ft = self.frame()
        frame_t = (t // ft) * ft
        if frame_t != self.playhead:
            self.playhead = frame_t
            self._update_timecode()
            self.playhead_moved.emit(frame_t)
            self.request_frame(frame_t)
        self._feed_audio()

    # ----- audio -----
    def _start_audio(self) -> None:
        if not HAVE_AUDIO or self.sequence is None:
            return
        rate = int((self.sequence.get("settings") or {}).get("sample_rate") or 48000)
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
            {"start": start, "frames": frames},
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
