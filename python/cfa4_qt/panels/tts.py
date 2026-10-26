"""The Text to Speech panel, as egui's (crates/ui-egui/src/panels/tts.rs): write what the narrator
says, pick a language, voice, vocal pitch and pace, hear it, and save it as a narration clip.
Selecting a narration clip on the timeline loads its script and settings here (and brings the
panel forward the first time), and Save regenerates it in place.

Every action is a ``tts.*`` command. Synthesis runs as a background job (``tts.render``); when
it finishes, the panel runs the action (preview, create or edit), which then uses the cached
audio, so the window never stalls. The natural voices need a one-time download that the user
confirms here (size, source, licence); it runs as a job with progress and Cancel. A preview
plays through the default audio output (``tts.previewAudio``).
"""

from __future__ import annotations

import re
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QBuffer, QByteArray, QIODevice, QSize, Qt, QTimer, Signal
from PySide6.QtGui import QStandardItem, QStandardItemModel
from PySide6.QtWidgets import (
    QComboBox,
    QFrame,
    QHBoxLayout,
    QLabel,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QScrollArea,
    QSlider,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, fmt, tr
from .essential_graphics import SectionHeader

try:  # QtMultimedia is optional: without it a preview is silent
    from PySide6.QtMultimedia import QAudio, QAudioFormat, QAudioSink, QMediaDevices

    HAVE_AUDIO = True
except ImportError:  # pragma: no cover - depends on the PySide6 build
    HAVE_AUDIO = False

POLL_MS = 150
PAUSE_RE = re.compile(r"\[\s*pause[^\]]*\]", re.IGNORECASE)
PITCH_LABELS = (N_("Extra low"), N_("Low"), N_("Default"), N_("High"), N_("Extra high"))


def says_something(text: str, max_bytes: int) -> bool:
    """``cfa4_tts::check_text``: within the size limit, and some speech besides the pauses."""
    return len(text.encode()) <= max_bytes and any(ch.isalnum() for ch in PAUSE_RE.sub(" ", text))


class TextToSpeechPanel(QWidget):
    command = Signal(str, dict)
    status = Signal(str)  # egui's status line: the main window's status bar
    changed = Signal()  # a narration was added or regenerated
    bring_forward = Signal()  # a narration clip was selected for the first time

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.TextToSpeech")
        self.engine = engine
        self.context: Callable[[], Any] = lambda: None  # the open project and sequence (the window sets it)
        self.catalog: dict[str, Any] = {}
        self.language = "en-US"
        self.voice = ""
        self.pitch = "default"
        self.pace = 1.0
        self.advanced_open = True
        self.loaded_from: tuple[int, int] | None = None  # clip, item
        self.pending: dict[str, Any] | None = None  # cmd, params, play, job
        self.confirm_download = False
        self.download_job: int | None = None
        self.download_state: tuple[float, str] | None = None
        self.download_error = ""
        self._asked = False
        self.has_sequence = True
        self._sink: Any = None
        self._buffer: QBuffer | None = None
        self._poll = QTimer(self, interval=POLL_MS)
        self._poll.timeout.connect(self._poll_jobs)

        t = theme.current()
        self.scroll = QScrollArea(self)
        self.scroll.setWidgetResizable(True)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        body = QWidget(self.scroll)
        self.lay = QVBoxLayout(body)
        self.lay.setContentsMargins(14, 8, 14, 8)
        self.lay.setSpacing(2)
        self.scroll.setWidget(body)

        mode = QHBoxLayout()
        self.mode = QLabel(body)
        self.mode.setObjectName("tts.mode")
        self.mode.setStyleSheet(f"color: {t.text_dim}; font-size: 11.5px;")
        self.new_button = QPushButton(tr("New"), body)
        self.new_button.setObjectName("tts.new")
        self.new_button.setToolTip(tr("New narration"))
        self.new_button.clicked.connect(self.new_narration)
        mode.addWidget(self.mode, 1)
        mode.addWidget(self.new_button)
        self.lay.addLayout(mode)
        self.lay.addSpacing(6)

        self.lay.addWidget(self._label(tr("Language")))
        self.language_combo = QComboBox(body)
        self.language_combo.setObjectName("tts.language")
        self.language_combo.activated.connect(self._language_picked)
        self.lay.addWidget(self.language_combo)
        self.lay.addSpacing(10)
        self.lay.addWidget(self._label(tr("Voice")))
        self.voice_combo = QComboBox(body)
        self.voice_combo.setObjectName("tts.voice")
        self.voice_combo.activated.connect(self._voice_picked)
        self.lay.addWidget(self.voice_combo)

        # the natural voices' one-time download
        self.download_box = QFrame(body)
        self.download_box.setObjectName("tts.downloadBox")
        self.download_box.setStyleSheet(
            f"QFrame#tts\\.downloadBox {{ background: {t.field_bg}; border: 1px solid {t.field_border}; border-radius: 4px; }}"
        )
        box = QVBoxLayout(self.download_box)
        box.setContentsMargins(8, 8, 8, 8)
        self.download_title = QLabel(self.download_box)
        self.download_title.setWordWrap(True)
        self.download_note = QLabel(self.download_box)
        self.download_note.setWordWrap(True)
        self.download_note.setStyleSheet(f"color: {t.text_dim}; font-size: 11px;")
        self.download_bar = QProgressBar(self.download_box)
        self.download_bar.setRange(0, 1000)
        buttons = QHBoxLayout()
        self.download_button = QPushButton(self.download_box)
        self.download_button.setObjectName("tts.download")
        self.download_button.clicked.connect(self._ask_download)
        self.download_ok = QPushButton(tr("Download"), self.download_box)
        self.download_ok.setObjectName("tts.download.confirm")
        self.download_ok.clicked.connect(self._download)
        self.download_cancel = QPushButton(tr("Cancel"), self.download_box)
        self.download_cancel.setObjectName("tts.download.cancel")
        self.download_cancel.clicked.connect(self._cancel_download)
        buttons.addWidget(self.download_button)
        buttons.addWidget(self.download_ok)
        buttons.addWidget(self.download_cancel)
        buttons.addStretch(1)
        box.addWidget(self.download_title)
        box.addWidget(self.download_note)
        box.addWidget(self.download_bar)
        box.addLayout(buttons)
        self.lay.addSpacing(6)
        self.lay.addWidget(self.download_box)
        self.lay.addSpacing(4)

        self.hear = QToolButton(body)
        self.hear.setObjectName("tts.hearVoice")
        self.hear.setText("▶  " + tr("Hear this voice"))
        self.hear.setAutoRaise(True)
        self.hear.setCursor(Qt.CursorShape.PointingHandCursor)
        self.hear.setStyleSheet(f"QToolButton {{ color: {t.accent}; font-weight: 600; }} QToolButton:hover {{ color: {t.accent_hover}; }}")
        self.hear.clicked.connect(self._hear_voice)
        self.lay.addWidget(self.hear, 0, Qt.AlignmentFlag.AlignLeft)
        self.lay.addSpacing(8)

        self.advanced = SectionHeader(N_("Advanced"), self.advanced_open, body)
        self.advanced.setObjectName("tts.advanced")
        self.advanced.twirled.connect(lambda _t: self._twirl())
        self.lay.addWidget(self.advanced)
        self.advanced_body = QWidget(body)
        adv = QVBoxLayout(self.advanced_body)
        adv.setContentsMargins(0, 4, 0, 0)
        adv.setSpacing(2)
        adv.addWidget(self._label(tr("Vocal pitch")))
        self.pitch_combo = QComboBox(self.advanced_body)
        self.pitch_combo.setObjectName("tts.pitch")
        self.pitch_combo.activated.connect(lambda _i: self._set_pitch(self.pitch_combo.currentData()))
        adv.addWidget(self.pitch_combo)
        adv.addSpacing(10)
        adv.addWidget(self._label(tr("Pace")))
        pace = QHBoxLayout()
        self.pace_slider = QSlider(Qt.Orientation.Horizontal, self.advanced_body)
        self.pace_slider.setObjectName("tts.pace")
        self.pace_slider.setRange(10, 40)  # steps of 0.05: 0.5× … 2×
        self.pace_slider.setValue(20)
        self.pace_slider.valueChanged.connect(lambda v: self._set_pace(v / 20))
        self.pace_value = QLabel(self.advanced_body)
        self.pace_value.setFixedWidth(46)
        pace.addWidget(self.pace_slider, 1)
        pace.addWidget(self.pace_value)
        adv.addLayout(pace)
        ticks = QHBoxLayout()
        ticks.setContentsMargins(0, 0, 50, 0)
        for text, stretch in (("0.5×", 1), ("1×", 2), ("2×", 0)):
            tick = QLabel(text, self.advanced_body)
            tick.setStyleSheet(f"color: {t.text_faint}; font-size: 10.5px;")
            ticks.addWidget(tick, stretch)
        adv.addLayout(ticks)
        self.lay.addWidget(self.advanced_body)
        self.lay.addSpacing(12)

        head = QHBoxLayout()
        text_label = QLabel(tr("Text"), body)
        text_label.setStyleSheet("font-weight: 600;")
        self.add_pause = QPushButton(tr("Add pause"), body)
        self.add_pause.setObjectName("tts.addPause")
        self.add_pause.clicked.connect(self._add_pause)
        info = QLabel("ⓘ", body)
        info.setStyleSheet(f"color: {t.text_faint};")
        info.setToolTip(
            tr("Add pause inserts [pause 1s]. Change the number for a longer or shorter pause, e.g. [pause 500ms] or [pause 2s].")
        )
        head.addWidget(text_label, 1)
        head.addWidget(self.add_pause)
        head.addWidget(info)
        self.lay.addLayout(head)
        self.text = QPlainTextEdit(body)
        self.text.setObjectName("tts.text")
        self.text.setPlaceholderText(tr("Type what you want the voice to say"))
        self.text.setMinimumHeight(150)
        self.text.textChanged.connect(self._text_changed)
        self.lay.addWidget(self.text, 1)
        self.lay.addStretch(1)

        foot = QHBoxLayout()
        foot.setContentsMargins(14, 10, 14, 10)
        foot.setSpacing(10)
        self.preview = QPushButton(body)
        self.preview.setObjectName("tts.preview")
        self.preview.clicked.connect(self._preview)
        self.save = QPushButton(body)
        self.save.setObjectName("tts.save")
        self.save.setProperty("primary", True)
        self.save.setStyleSheet(
            f"QPushButton {{ background: {t.accent}; color: white; border: none; border-radius: 4px; padding: 5px; font-weight: 600; }}"
            f" QPushButton:hover {{ background: {t.accent_hover}; }}"
            f" QPushButton:disabled {{ background: {t.field_bg}; color: {t.text_faint}; }}"
        )
        self.save.clicked.connect(self._save)
        foot.addWidget(self.preview, 1)
        foot.addWidget(self.save, 1)
        v = QVBoxLayout(self)
        v.setContentsMargins(0, 0, 0, 0)
        v.setSpacing(0)
        v.addWidget(self.scroll, 1)
        v.addLayout(foot)
        self._sync()

    def _label(self, text: str) -> QLabel:
        label = QLabel(text, self)
        label.setStyleSheet("font-weight: 600;")
        return label

    def sizeHint(self) -> QSize:
        return QSize(320, 560)

    # ----- the engine -----
    @property
    def running(self) -> bool:
        return self.engine is not None and getattr(self.engine, "running", False)

    def load_catalog(self) -> None:
        """Languages, voices, pitches and the natural-voice package (``tts.voices``)."""
        if not self.running:
            return
        self._asked = True
        self.engine.execute("tts.voices", {}, on_reply=lambda r, _b: self.set_catalog(r), on_error=lambda _m: None)

    def set_catalog(self, data: Any) -> None:
        self.catalog = data if isinstance(data, dict) else {}
        if not self.voice:
            natural_ready = bool((self.catalog.get("package") or {}).get("installed"))
            self.voice = "kokoro-heart" if natural_ready else str(self.catalog.get("defaultVoice") or "")
        self._sync()

    def follow_selection(self) -> None:
        """Load the selected narration clip into the draft when the selection moves to another
        one (egui's ``follow_selection``); a deselect keeps the text for a new narration."""
        if not self.running:
            return
        if not self._asked:
            self.load_catalog()

        def got(r: Any, _b: bytes | None) -> None:
            if not isinstance(r, dict) or r.get("clip") is None:
                return
            key = (int(r["clip"]), int(r.get("item") or 0))
            n = r.get("narration") or {}
            if key == self.loaded_from or not isinstance(n, dict):
                return
            first_time = self.loaded_from is None or self.loaded_from[0] != key[0]
            self.text.blockSignals(True)
            self.text.setPlainText(str(n.get("text") or ""))
            self.text.blockSignals(False)
            self.language = str(n.get("language") or self.language)
            self.voice = str(n.get("voice") or self.voice)
            self.pitch = str(n.get("pitch") or "default")
            self.pace = float(n.get("pace") or 1.0)
            self.loaded_from = key
            self._sync()
            if first_time:
                self.bring_forward.emit()

        def none(_m: str) -> None:
            if self.loaded_from is not None:
                self.loaded_from = None
                self._sync()

        self.engine.execute("tts.inspect", {}, on_reply=got, on_error=none)

    def params(self) -> dict[str, Any]:
        return {"text": self.text.toPlainText(), "voice": self.voice, "pitch": self.pitch, "pace": round(self.pace, 2)}

    def start(self, cmd: str, params: dict[str, Any], play: bool) -> None:
        """Synthesize in the background (or reuse the cache), then run ``cmd``."""
        if not self.running or self.pending is not None:
            return
        render = {k: v for k, v in params.items() if k not in ("dir", "time")}
        # the project and sequence it is for: a narration finished after another one was opened is not placed there
        self.pending = {"cmd": cmd, "params": params, "play": play, "job": None, "context": self.context()}
        self._sync()

        def rendered(r: Any, _b: bytes | None) -> None:
            job = r.get("job") if isinstance(r, dict) else None
            if job is None:
                self.pending = None
                self.run_now(cmd, params, play)
            else:
                assert self.pending is not None
                self.pending["job"] = int(job)
                self.status.emit(tr("Synthesizing narration…"))
                self._poll.start()
            self._sync()

        def failed(m: str) -> None:
            self.pending = None
            self.status.emit(m)
            self._sync()

        self.engine.execute("tts.render", render, on_reply=rendered, on_error=failed)

    def run_now(self, cmd: str, params: dict[str, Any], play: bool) -> None:
        def done(r: Any, _b: bytes | None) -> None:
            if play:
                self.play_preview()
                return
            if cmd == "tts.create":
                track = (r or {}).get("track") if isinstance(r, dict) else None
                self.status.emit(fmt("Narration added on {track}", track=str(track or tr("an audio track"))))
            else:
                self.status.emit(tr("Narration updated"))
            self.changed.emit()

        self.engine.execute(cmd, params, on_reply=done, on_error=lambda m: self.status.emit(m))

    def play_preview(self) -> None:
        """Play the last ``tts.preview`` (egui's ``play_tts_preview``)."""
        if not HAVE_AUDIO:
            self.status.emit(tr("no audio output (check Settings ▸ Audio Hardware)"))
            return
        device = QMediaDevices.defaultAudioOutput()
        fmt_ = QAudioFormat()
        fmt_.setSampleRate(48_000)
        fmt_.setChannelCount(2)
        fmt_.setSampleFormat(QAudioFormat.SampleFormat.Float)
        if device.isNull() or not device.isFormatSupported(fmt_):
            pref = device.preferredFormat() if not device.isNull() else fmt_
            fmt_.setSampleRate(pref.sampleRate() or 48_000)
            fmt_.setChannelCount(max(1, pref.channelCount()))

        def got(r: Any, data: bytes | None) -> None:
            self.stop_preview()
            if device.isNull() or not data:
                self.status.emit(tr("no audio output (check Settings ▸ Audio Hardware)"))
                return
            self._buffer = QBuffer(self)
            self._buffer.setData(QByteArray(data))
            self._buffer.open(QIODevice.OpenModeFlag.ReadOnly)
            self._sink = QAudioSink(device, fmt_, self)
            self._sink.stateChanged.connect(self._sink_state)
            self._sink.start(self._buffer)

        self.engine.call(
            "tts.previewAudio",
            {"sampleRate": fmt_.sampleRate(), "channels": fmt_.channelCount()},
            on_reply=got,
            on_error=lambda m: self.status.emit(m),
        )

    def _sink_state(self, state: Any) -> None:
        if HAVE_AUDIO and state == QAudio.State.IdleState:
            self.stop_preview()

    def stop_preview(self) -> None:
        if self._sink is not None:
            self._sink.stop()
            self._sink.deleteLater()
            self._sink = None
        if self._buffer is not None:
            self._buffer.close()
            self._buffer.deleteLater()
            self._buffer = None

    def _poll_jobs(self) -> None:
        if not self.running or (self.pending is None and self.download_job is None):
            self._poll.stop()
            return

        def got(r: Any, _b: bytes | None) -> None:
            jobs = {j.get("id"): j for j in r or [] if isinstance(j, dict)} if isinstance(r, list) else {}
            p = self.pending
            if p is not None and p.get("job") is not None:
                j = jobs.get(p["job"])
                if j is None or j.get("finished"):
                    self.pending = None
                    err = ((j or {}).get("result") or {}).get("error") if j else tr("the job disappeared")
                    if err:
                        self.status.emit(fmt("Text to Speech: {e}", e=str(err)))
                    elif p.get("context") != self.context():
                        self.status.emit(tr("Text to Speech: another sequence is open now; the narration was not placed"))
                    else:
                        self.run_now(p["cmd"], p["params"], p["play"])
            if self.download_job is not None:
                j = jobs.get(self.download_job)
                if j is None or j.get("finished"):
                    self.download_job = None
                    self.download_state = None
                    err = ((j or {}).get("result") or {}).get("error") if j else tr("the job disappeared")
                    if err:
                        self.download_error = str(err)
                        self.status.emit(fmt("Download of the natural voices failed: {e}", e=str(err)))
                    else:
                        self.status.emit(tr("Natural voices installed"))
                    self.load_catalog()
                else:
                    self.download_state = (float(j.get("progress") or 0), str(j.get("status") or ""))
            self._sync()

        self.engine.execute("jobs.list", {}, on_reply=got, on_error=lambda _m: None)

    # ----- actions -----
    def new_narration(self) -> None:
        """New: deselect the narration and start an empty script, keeping the voice settings."""
        self.command.emit("timeline.select", {"clips": []})
        self.loaded_from = None
        self.text.clear()
        self._sync()

    def _language_picked(self, _i: int) -> None:
        self.language = str(self.language_combo.currentData() or self.language)
        self._sync()

    def _voice_picked(self, _i: int) -> None:
        vid = self.voice_combo.currentData()
        if vid:
            self.voice = str(vid)
        self._sync()

    def _set_pitch(self, pid: Any) -> None:
        if pid:
            self.pitch = str(pid)

    def _set_pace(self, pace: float) -> None:
        self.pace = pace
        self.pace_value.setText(f"{pace:.2f}×")

    def _twirl(self) -> None:
        self.advanced_open = not self.advanced_open
        self._sync()

    def _add_pause(self) -> None:
        text = self.text.toPlainText()
        if text and not text.endswith(" "):
            text += " "
        text += str(self.catalog.get("pauseMarker") or "[pause 1s]") + " "
        self.text.setPlainText(text)
        self.text.moveCursor(self.text.textCursor().MoveOperation.End)

    def _text_changed(self) -> None:
        limit = int(self.catalog.get("maxTextBytes") or 65536)
        text = self.text.toPlainText()
        if len(text.encode()) > limit:  # egui's char limit
            cut = text.encode()[:limit].decode(errors="ignore")
            self.text.blockSignals(True)
            self.text.setPlainText(cut)
            self.text.moveCursor(self.text.textCursor().MoveOperation.End)
            self.text.blockSignals(False)
        self._sync_buttons()

    def _hear_voice(self) -> None:
        if self.voice_ready() and self.pending is None:
            self.start("tts.preview", {"sample": True, "voice": self.voice, "pitch": self.pitch, "pace": round(self.pace, 2)}, True)

    def _preview(self) -> None:
        if self.can_say():
            self.start("tts.preview", self.params(), True)

    def _save(self) -> None:
        if not self.can_say():
            return
        p = self.params()
        if self.loaded_from is not None:
            self.start("tts.edit", {**p, "clip": self.loaded_from[0]}, False)
        else:
            self.start("tts.create", p, False)

    def _ask_download(self) -> None:
        self.confirm_download = True
        self.download_error = ""
        self._sync()

    def _download(self) -> None:
        self.confirm_download = False

        def started(r: Any, _b: bytes | None) -> None:
            job = r.get("job") if isinstance(r, dict) else None
            if job is None:
                self.load_catalog()  # already installed
            else:
                self.download_job = int(job)
                self.download_state = (0.0, "")
                self._poll.start()
            self._sync()

        def failed(m: str) -> None:
            self.download_error = m
            self.status.emit(m)
            self._sync()

        self.engine.execute("tts.downloadVoices", {}, on_reply=started, on_error=failed)
        self._sync()

    def _cancel_download(self) -> None:
        if self.download_job is not None:
            self.engine.execute("jobs.cancel", {"job": self.download_job}, on_error=lambda _m: None)
        else:
            self.confirm_download = False
            self._sync()

    # ----- state -----
    def voices(self) -> list[dict[str, Any]]:
        return [v for v in self.catalog.get("voices") or [] if isinstance(v, dict)]

    def natural(self) -> bool:
        return any(v.get("id") == self.voice and v.get("engine") == "neural" for v in self.voices())

    def installed(self) -> bool:
        return bool((self.catalog.get("package") or {}).get("installed"))

    def voice_ready(self) -> bool:
        return not self.natural() or self.installed()

    def can_say(self) -> bool:
        limit = int(self.catalog.get("maxTextBytes") or 65536)
        return says_something(self.text.toPlainText(), limit) and self.voice_ready() and self.pending is None

    def _sync(self) -> None:
        t = theme.current()
        self.mode.setText(tr("Editing the selected narration") if self.loaded_from is not None else tr("New narration"))
        self.new_button.setVisible(self.loaded_from is not None)
        self.language_combo.blockSignals(True)
        self.language_combo.clear()
        for lang in self.catalog.get("languages") or []:
            self.language_combo.addItem(tr(str(lang.get("name"))), lang.get("id"))
        i = self.language_combo.findData(self.language)
        if i < 0 and self.language:
            self.language_combo.addItem(self.language, self.language)
            i = self.language_combo.count() - 1
        self.language_combo.setCurrentIndex(i)
        self.language_combo.blockSignals(False)
        # the voice list: Natural voices, then Basic voices (built in), with headings
        model = QStandardItemModel(self.voice_combo)
        current = -1
        for engine_, heading in (("neural", tr("Natural voices")), ("basic", tr("Basic voices (built in)"))):
            item = QStandardItem(heading)
            item.setFlags(Qt.ItemFlag.NoItemFlags)
            item.setForeground(theme.current().c("text_faint"))
            model.appendRow(item)
            for v in self.voices():
                if v.get("language") != self.language or v.get("engine") != engine_:
                    continue
                name = tr(str(v.get("name") or v.get("id")))
                if engine_ == "neural" and not v.get("installed"):
                    name = fmt("{name} (download)", name=name)
                item = QStandardItem("   " + name)
                item.setData(v.get("id"), Qt.ItemDataRole.UserRole)
                item.setToolTip(tr(str(v.get("description") or "")))
                model.appendRow(item)
                if v.get("id") == self.voice:
                    current = model.rowCount() - 1
        self.voice_combo.blockSignals(True)
        self.voice_combo.setModel(model)
        if current < 0 and self.voice:
            model.appendRow(QStandardItem(self.voice))
            model.item(model.rowCount() - 1).setData(self.voice, Qt.ItemDataRole.UserRole)
            current = model.rowCount() - 1
        self.voice_combo.setCurrentIndex(current)
        self.voice_combo.blockSignals(False)
        # the download box
        pkg = self.catalog.get("package") or {}
        mb = float(pkg.get("size") or 0) / 1e6
        show_box = self.natural() and not self.installed()
        self.download_box.setVisible(show_box)
        downloading = self.download_job is not None
        self.download_bar.setVisible(downloading)
        self.download_button.setVisible(not downloading and not self.confirm_download)
        self.download_ok.setVisible(not downloading and self.confirm_download)
        self.download_cancel.setVisible(downloading or self.confirm_download)
        self.download_button.setText(fmt("Download natural voices ({mb} MB)…", mb=f"{mb:.0f}"))
        if downloading:
            frac, status = self.download_state or (0.0, "")
            self.download_title.setText(status or tr("Downloading…"))
            self.download_title.setStyleSheet(f"color: {t.text};")
            self.download_bar.setValue(int(frac * 1000))
            self.download_note.setVisible(False)
        elif self.confirm_download:
            self.download_title.setText(fmt("Download the natural voices? {mb} MB, once.", mb=f"{mb:.0f}"))
            self.download_title.setStyleSheet(f"color: {t.text}; font-weight: 600;")
            self.download_note.setText(
                fmt(
                    "From {source} and github.com/cmusphinx/cmudict (pronunciations). Licence: {license}. Every file is checked against a pinned SHA-256.",
                    source=str(pkg.get("source") or ""),
                    license=str(pkg.get("license") or ""),
                )
            )
            self.download_note.setVisible(True)
        else:
            self.download_title.setText(tr("Natural voices need a one-time download."))
            self.download_title.setStyleSheet(f"color: {t.text};")
            self.download_note.setText(self.download_error)
            self.download_note.setVisible(bool(self.download_error))
        self.hear.setEnabled(self.voice_ready() and self.pending is None)
        # Advanced
        self.advanced.open = self.advanced_open
        self.advanced.update()
        self.advanced_body.setVisible(self.advanced_open)
        self.pitch_combo.blockSignals(True)
        self.pitch_combo.clear()
        pitches = self.catalog.get("pitches") or [{"id": p, "label": label} for p, label in zip(("extraLow", "low", "default", "high", "extraHigh"), PITCH_LABELS, strict=True)]
        for p in pitches:
            self.pitch_combo.addItem(tr(str(p.get("label"))), p.get("id"))
        self.pitch_combo.setCurrentIndex(max(0, self.pitch_combo.findData(self.pitch)))
        self.pitch_combo.blockSignals(False)
        self.pace_slider.blockSignals(True)
        self.pace_slider.setValue(round(self.pace * 20))
        self.pace_slider.blockSignals(False)
        self._set_pace(self.pace)
        self._sync_buttons()

    def _sync_buttons(self) -> None:
        busy = self.pending is not None
        self.preview.setText("▶  " + (tr("Synthesizing…") if busy else tr("Preview")))
        self.preview.setEnabled(self.can_say())
        self.save.setText("+  " + (tr("Save") if self.loaded_from is not None else tr("Add to timeline")))
        self.save.setEnabled(self.can_say() and self.has_sequence)
