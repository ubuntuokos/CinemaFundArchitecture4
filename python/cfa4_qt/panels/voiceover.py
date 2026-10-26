"""Voice-over recording from the timeline, as egui's ``panels/voiceover.rs``: the audio track
header's microphone button, the Audio Track Mixer's record button and the Voice-Over Record
Settings dialog. The engine runs in another process, so the microphone is captured here with Qt
Multimedia and streamed to it (``audio.voiceover.hostInput`` + ``audio.voiceover.feed``); the take
itself is the engine's ``audio.voiceover.start`` / ``sync`` / ``stop``, tied to playback as egui
ties it: playback starts at the capture start (the pre-roll before the playhead, or the In
point), capture restarts when the playback clock starts, and the take ends with playback or after
the post-roll past the Out point. The countdown beeps come with the programme audio.

Automation ids (egui's): ``timeline.track.<A1…>.voiceover.settings`` (the header button's menu),
the dialog ``voiceover`` with ``voiceover.name``, ``voiceover.source``, ``voiceover.input``,
``voiceover.countdown``, ``voiceover.preroll``, ``voiceover.postroll``, ``voiceover.discard``,
``voiceover.ok``, ``voiceover.cancel``; the countdown overlay ``voiceover.countdown.overlay``.
UI commands ``voiceover.recordToggle`` and ``voiceover.settingsDialog``.
"""

from __future__ import annotations

import base64
import math
from array import array
from collections.abc import Callable
from typing import Any, Protocol

from PySide6.QtCore import QObject, Qt, QTimer, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import fmt, tr

try:  # Qt Multimedia is optional: without it there is no microphone, the rest still works
    from PySide6.QtMultimedia import QAudioFormat, QAudioSource, QMediaDevices

    HAVE_INPUT = True
except ImportError:  # pragma: no cover - depends on the PySide6 build
    HAVE_INPUT = False

TICKS_PER_SECOND = 254_016_000_000
FEED_MS = 50
# a block sent to the engine at most (one second at 192 kHz)
MAX_BLOCK = 192_000
RED = "#d8503f"


class Capture(Protocol):
    """A microphone being captured: ``open`` → (sample rate, channels), then interleaved
    float32 frames from ``read``."""

    def open(self) -> tuple[int, int]: ...
    def read(self) -> array: ...
    def discard(self) -> None: ...
    def stop(self) -> None: ...


def input_devices() -> list[tuple[str, int]]:
    """The input devices Qt sees: (name, channels)."""
    if not HAVE_INPUT:
        return []
    return [(d.description(), max(1, d.maximumChannelCount())) for d in QMediaDevices.audioInputs()]


def default_input() -> str:
    if not HAVE_INPUT:
        return ""
    d = QMediaDevices.defaultAudioInput()
    return "" if d.isNull() else d.description()


class QtCapture:
    """A Qt Multimedia input (``device`` "" = the default input), float32 or 16-bit."""

    def __init__(self, device: str) -> None:
        self.device = device
        self.source: Any = None
        self.io: Any = None
        self.int16 = False
        self.channels = 1

    def open(self) -> tuple[int, int]:
        if not HAVE_INPUT:
            raise RuntimeError(tr("Qt Multimedia is not available"))
        dev = next((d for d in QMediaDevices.audioInputs() if d.description() == self.device), None) if self.device else None
        if dev is None:
            dev = QMediaDevices.defaultAudioInput()
        if dev.isNull():
            raise RuntimeError(tr("There is no audio input device"))
        fmt_ = dev.preferredFormat()
        fmt_.setSampleFormat(QAudioFormat.SampleFormat.Float)
        if not dev.isFormatSupported(fmt_):
            fmt_.setSampleFormat(QAudioFormat.SampleFormat.Int16)
            self.int16 = True
        if not dev.isFormatSupported(fmt_):
            raise RuntimeError(tr("The audio input does not support a usable format"))
        self.channels = max(1, fmt_.channelCount())
        self.source = QAudioSource(dev, fmt_)
        self.io = self.source.start()
        if self.io is None:
            raise RuntimeError(tr("The audio input could not be started"))
        return max(1, fmt_.sampleRate()), self.channels

    def read(self) -> array:
        out = array("f")
        if self.io is None:
            return out
        data = bytes(self.io.readAll().data())
        if self.int16:
            raw = array("h")
            raw.frombytes(data[: len(data) // 2 * 2])
            out.extend(x / 32768.0 for x in raw)
        else:
            out.frombytes(data[: len(data) // 4 * 4])
        return out

    def discard(self) -> None:
        if self.io is not None:
            self.io.readAll()

    def stop(self) -> None:
        if self.source is not None:
            self.source.stop()
        self.source = self.io = None


class CountdownOverlay(QLabel):
    """The countdown / ● Recording / Post-roll badge over the window (egui's)."""

    def __init__(self, parent: QWidget) -> None:
        super().__init__(parent)
        self.setObjectName("voiceover.countdown.overlay")
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents)
        self.setFixedSize(220, 64)
        self.hide()

    def show_text(self, text: str) -> None:
        size = 36 if len(text) <= 2 else 20
        self.setStyleSheet(
            f"background: rgba(0,0,0,180); color: white; border: 2px solid {RED}; border-radius: 8px; font-size: {size}px; font-weight: 600;"
        )
        self.setText(text)
        parent = self.parentWidget()
        if parent is not None:
            self.move((parent.width() - self.width()) // 2, 90)
        self.show()
        self.raise_()


class VoiceOverRecorder(QObject):
    """One take at a time on the Program monitor's playback. ``capture_factory(device)`` and
    ``devices()`` default to Qt Multimedia; tests give their own."""

    changed = Signal(object)  # the record track's label (A1…) while recording, else None
    message = Signal(str, bool)  # status text, is an error
    refresh = Signal()  # a take was placed

    def __init__(self, engine: EngineClient, monitor: Any, window: QWidget) -> None:
        super().__init__(window)
        self.engine = engine
        self.monitor = monitor
        self.capture_factory: Callable[[str], Capture] = QtCapture
        self.devices: Callable[[], list[tuple[str, int]]] = input_devices
        self.default_device: Callable[[], str] = default_input
        self.rec: dict[str, Any] | None = None
        self.track: str | None = None
        self.capture: Capture | None = None
        self.channel = 0
        self.channels = 1
        self.level = 0.0  # the input's peak since the mixer last took it (linear)
        self._starting = False
        self._stopping = False
        self.timer = QTimer(self)
        self.timer.setInterval(FEED_MS)
        self.timer.timeout.connect(self._tick)
        self.overlay = CountdownOverlay(window)
        monitor.playback_stopped.connect(self._on_playback_stopped)

    @property
    def recording(self) -> bool:
        return self.rec is not None

    def announce(self, rate: int = 48_000) -> None:
        """Tell the engine which inputs Qt sees, so ``audio.voiceover.settings`` lists them (for
        the dialog and agents); a take sets the rate it captures at."""
        if self.rec is not None or self._starting:
            return
        devices = [{"name": n, "channels": c} for n, c in self.devices()]
        self.engine.execute("audio.voiceover.hostInput", {"devices": devices, "default": self.default_device(), "sampleRate": rate})

    # ----- start / stop -----
    def toggle(self, track: Any = None) -> None:
        """Record on ``track`` (None: the record-enabled, else a targeted track), or stop the take."""
        if self._starting:
            return
        if self.rec is not None:
            if self.monitor.playing:
                self.monitor.stop()  # finishes the take (playback_stopped)
            else:
                self.finish()
            return
        if self.monitor.playing:
            self.monitor.stop()
        self._starting = True
        self.engine.execute(
            "audio.voiceover.settings", {}, on_reply=lambda r, _b: self._open(r, track), on_error=self._failed_start
        )

    def _failed_start(self, error: str) -> None:
        self._starting = False
        if self.capture is not None:
            self.capture.stop()
            self.capture = None
        self.message.emit(error, True)

    def _open(self, settings: Any, track: Any) -> None:
        vo = settings.get("settings") if isinstance(settings, dict) else None
        vo = vo if isinstance(vo, dict) else {}
        source = str(vo.get("source") or "")
        try:
            capture = self.capture_factory(source)
            rate, channels = capture.open()
        except (RuntimeError, OSError, ValueError) as e:
            self._failed_start(fmt("Voice-over input: {error}", error=str(e)))
            return
        self.capture = capture
        self.channels = max(1, int(channels))
        self.channel = max(0, min(int(vo.get("inputChannel") or 0), self.channels - 1))
        devices = [{"name": n, "channels": c} for n, c in self.devices()]
        params = {"devices": devices, "default": self.default_device(), "sampleRate": int(rate)}
        self.engine.execute(
            "audio.voiceover.hostInput", params, on_reply=lambda _r, _b: self._start(track), on_error=self._failed_start
        )

    def _start(self, track: Any) -> None:
        params = {} if track is None else {"track": track}
        self.engine.execute("audio.voiceover.start", params, on_reply=lambda r, _b: self._started(r), on_error=self._failed_start)

    def _started(self, r: Any) -> None:
        self._starting = False
        if not isinstance(r, dict) or self.capture is None:
            return
        self.rec = r
        self.track = str(r.get("track") or "") or None
        start = int(r.get("captureStart") or 0)
        mon = self.monitor
        mon.free_run = True  # a take keeps playing (and recording) into the empty timeline past the end
        mon.set_playhead(start)
        mon.play(1.0)
        # the playback clock starts now: capture from here
        self.capture.discard()
        self.engine.execute("audio.voiceover.sync", {"time": int(mon.playhead)}, on_error=lambda m: self.message.emit(m, True))
        self.timer.start()
        self._tick()
        self.changed.emit(self.track)
        self.message.emit(tr("Recording voice-over…"), False)

    def finish(self) -> None:
        """Stop the take: the rest of the capture goes in, then the engine writes and places it."""
        if self.rec is None or self._stopping:
            return
        self._feed()
        self._close_capture()
        self._stopping = True
        t = int(self.monitor.playhead)
        self.engine.execute("audio.voiceover.stop", {"time": t}, on_reply=lambda r, _b: self._finished(r), on_error=self._failed_stop)

    def _finished(self, r: Any) -> None:
        self._stopping = False
        self._clear()
        path = r.get("path") if isinstance(r, dict) else None
        if path:
            self.message.emit(fmt("Voice-over recorded: {p}", p=str(path)), False)
        else:
            self.message.emit(tr("Voice-over recording stopped (nothing recorded)"), False)
        self.refresh.emit()

    def _failed_stop(self, error: str) -> None:
        # the engine keeps the take: stopping again retries the save
        self._stopping = False
        self.message.emit(error, True)

    def discard(self, done: Callable[[], None] | None = None) -> None:
        """Drop the take in progress (Voice-Over Record Settings ▸ Discard take)."""
        if self.rec is None:
            return

        def ok(_r: Any, _b: Any) -> None:
            self._clear()
            if self.monitor.playing:
                self.monitor.stop()
            self.message.emit(tr("Voice-over take discarded"), False)
            if done is not None:
                done()

        self._close_capture()
        self.engine.execute("audio.voiceover.stop", {"discard": True}, on_reply=ok, on_error=lambda m: self.message.emit(m, True))

    def _close_capture(self) -> None:
        self.timer.stop()
        if self.capture is not None:
            self.capture.stop()
            self.capture = None

    def _clear(self) -> None:
        self._close_capture()
        self.rec = None
        self.track = None
        self.monitor.free_run = False
        self.overlay.hide()
        self.changed.emit(None)

    def _on_playback_stopped(self) -> None:
        if self.rec is not None and not self._starting:
            self.finish()

    # ----- while recording -----
    def _feed(self) -> None:
        if self.capture is None:
            return
        frames = self.capture.read()
        if not frames:
            return
        mono = array("f", frames[self.channel :: self.channels])
        heard = [abs(x) for x in mono if math.isfinite(x)]
        if heard:
            self.level = max(self.level, max(heard))
        for i in range(0, len(mono), MAX_BLOCK):
            block = mono[i : i + MAX_BLOCK]
            data = base64.b64encode(block.tobytes()).decode("ascii")
            self.engine.execute("audio.voiceover.feed", {"samples": data}, on_reply=self._fed, on_error=lambda m: self.message.emit(m, True))

    def _fed(self, r: Any, _b: Any) -> None:
        if isinstance(r, dict) and r.get("full") and self.rec is not None:
            self.message.emit(tr("The voice-over input buffer is full: the take ends here"), True)
            if self.monitor.playing:
                self.monitor.stop()

    def take_level(self) -> float:
        lv, self.level = self.level, 0.0
        return lv

    def _tick(self) -> None:
        rec = self.rec
        if rec is None:
            return
        self._feed()
        mon = self.monitor
        if not mon.playing:
            return
        now = int(mon.playhead)
        record = int(rec.get("recordStart") or 0)
        punch = rec.get("punchOut")
        if isinstance(punch, int) and now >= punch + int(rec.get("postroll") or 0):
            mon.stop()
            return
        if now < record:
            text = str(max(1, math.ceil((record - now) / TICKS_PER_SECOND)))
        elif isinstance(punch, int) and now >= punch:
            text = tr("Post-roll")
        else:
            text = tr("● Recording")
        self.overlay.show_text(text)


class VoiceOverSettingsDialog(QDialog):
    """Voice-Over Record Settings (egui's): name, source, input channel, countdown cues, pre- and
    post-roll. OK sets them with ``audio.voiceover.settings``."""

    def __init__(self, parent: QWidget, engine: EngineClient, recorder: VoiceOverRecorder) -> None:
        super().__init__(parent)
        self.setObjectName("voiceover")
        self.setWindowTitle(tr("Voice-Over Record Settings"))
        self.engine = engine
        self.recorder = recorder
        self.ready = False
        self.devices = recorder.devices()
        self.default = recorder.default_device()
        lay = QVBoxLayout(self)
        form = QFormLayout()
        self.name = QLineEdit(self)
        self.name.setObjectName("voiceover.name")
        self.name.setMinimumWidth(220)
        form.addRow(tr("Name:"), self.name)
        self.source = QComboBox(self)
        self.source.setObjectName("voiceover.source")
        self.source.addItem(tr("Default Input"), "")
        for n, _c in self.devices:
            self.source.addItem(n, n)
        self.source.currentIndexChanged.connect(self._fill_channels)
        form.addRow(tr("Source:"), self.source)
        self.input = QComboBox(self)
        self.input.setObjectName("voiceover.input")
        form.addRow(tr("Input:"), self.input)
        self.countdown = QCheckBox(tr("Countdown Sound Cues"), self)
        self.countdown.setObjectName("voiceover.countdown")
        form.addRow("", self.countdown)
        self.preroll = self._seconds("voiceover.preroll")
        form.addRow(tr("Pre-roll:"), self.preroll)
        self.postroll = self._seconds("voiceover.postroll")
        form.addRow(tr("Post-roll:"), self.postroll)
        lay.addLayout(form)
        note = QLabel(tr("Playback starts the pre-roll before the playhead (or the In point); with In/Out marked, recording punches in and out there."), self)
        note.setWordWrap(True)
        note.setEnabled(False)
        lay.addWidget(note)
        self.error = QLabel("", self)
        self.error.setObjectName("voiceover.message")
        self.error.setStyleSheet(f"color: {RED};")
        self.error.hide()
        lay.addWidget(self.error)
        row = QHBoxLayout()
        row.addStretch(1)
        if recorder.recording:
            d = QPushButton(tr("Discard take"), self)
            d.setObjectName("voiceover.discard")
            d.setToolTip(tr("Discard the active take"))
            d.clicked.connect(lambda: recorder.discard(self.reject))
            row.addWidget(d)
        cancel = QPushButton(tr("Cancel"), self)
        cancel.setObjectName("voiceover.cancel")
        cancel.clicked.connect(self.reject)
        row.addWidget(cancel)
        self.ok = QPushButton(tr("OK"), self)
        self.ok.setObjectName("voiceover.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.apply)
        row.addWidget(self.ok)
        lay.addLayout(row)
        self._fill_channels()
        recorder.announce()
        engine.execute("audio.voiceover.settings", {}, on_reply=lambda r, _b: self._load(r), on_error=self._show_error)

    def _seconds(self, name: str) -> QDoubleSpinBox:
        w = QDoubleSpinBox(self)
        w.setObjectName(name)
        w.setRange(0.0, 60.0)
        w.setSingleStep(0.1)
        w.setDecimals(1)
        w.setSuffix(tr(" seconds"))
        return w

    def channel_count(self) -> int:
        device = str(self.source.currentData() or "") or self.default
        return next((c for n, c in self.devices if n == device), self.devices[0][1] if self.devices else 1)

    def _fill_channels(self, *_a: Any) -> None:
        cur = max(0, self.input.currentIndex())
        n = max(1, min(64, self.channel_count()))
        self.input.blockSignals(True)
        self.input.clear()
        for c in range(max(n, cur + 1) if self.ready else n):
            self.input.addItem(fmt("Channel {n}", n=c + 1), c)
        self.input.setCurrentIndex(min(cur, n - 1))
        self.input.blockSignals(False)

    def _load(self, r: Any) -> None:
        vo = r.get("settings") if isinstance(r, dict) else None
        if not isinstance(vo, dict):
            return
        self.name.setText(str(vo.get("name") or ""))
        source = str(vo.get("source") or "")
        i = self.source.findData(source)
        if i < 0 and source:  # a device that is not connected now stays chosen
            self.source.addItem(source, source)
            i = self.source.count() - 1
        self.source.setCurrentIndex(max(0, i))
        self._fill_channels()
        ch = max(0, min(63, int(vo.get("inputChannel") or 0)))
        while self.input.count() <= ch:
            self.input.addItem(fmt("Channel {n}", n=self.input.count() + 1), self.input.count())
        self.input.setCurrentIndex(ch)
        self.countdown.setChecked(bool(vo.get("countdownSoundCues")))
        self.preroll.setValue(float(vo.get("prerollSeconds") or 0.0))
        self.postroll.setValue(float(vo.get("postrollSeconds") or 0.0))
        self.ready = True

    def values(self) -> dict[str, Any]:
        return {
            "source": str(self.source.currentData() or ""),
            "inputChannel": int(self.input.currentData() or 0),
            "name": self.name.text(),
            "countdownSoundCues": self.countdown.isChecked(),
            "prerollSeconds": self.preroll.value(),
            "postrollSeconds": self.postroll.value(),
        }

    def apply(self) -> None:
        if not self.ready:
            return
        self.engine.execute("audio.voiceover.settings", self.values(), on_reply=lambda _r, _b: self.accept(), on_error=self._show_error)

    def _show_error(self, error: str) -> None:
        self.error.setText(error)
        self.error.show()
