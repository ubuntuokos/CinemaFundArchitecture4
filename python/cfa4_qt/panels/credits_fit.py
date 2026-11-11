"""Fit Credits to Music window: pick the music, see what the engine found in it (length, where
the sound ends, tempo) and the lengths it offers, choose one (or type a length), and fit the
selected credits to it.

The engine analyses (``graphics.credits.music``, changes nothing) and fits
(``graphics.credits.fit``, one undo step): cards keep their length, rolls stretch or squeeze the
gaps between their blocks. Nothing changes until the user presses Fit.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QRadioButton,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND
from .credits import format_seconds, local_number

SUGGESTIONS = {
    "musicLength": N_("The whole music ({time})"),
    "soundEnd": N_("Until the sound ends ({time})"),
    "lastBar": N_("To the last bar line ({time})"),
}


def audio_clips(sequence: dict[str, Any] | None) -> list[dict[str, Any]]:
    """Every clip on the sequence's audio tracks, in time order."""
    clips: list[dict[str, Any]] = []
    for track in (sequence or {}).get("audio") or []:
        for item in (track or {}).get("items") or []:
            if isinstance(item, dict) and isinstance(item.get("clip"), int):
                clips.append(item)
    clips.sort(key=lambda c: (c.get("start") or 0, c.get("clip")))
    return clips


class CreditsFitDialog(QDialog):
    fitted = Signal(object)  # the engine's reply

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.creditsFit")
        self.setWindowTitle(tr("Fit Credits to Music"))
        self.setModal(False)
        self.engine = engine
        self.credits: int | None = None
        self.info: dict[str, Any] | None = None
        self._loading = False

        self.music = QComboBox(self)
        self.music.setObjectName("creditsFit.music")
        self.music.currentIndexChanged.connect(self._music_changed)
        self.facts = QLabel(self)
        self.facts.setObjectName("creditsFit.facts")
        self.facts.setWordWrap(True)

        self.choices = QButtonGroup(self)
        self.choice_box = QVBoxLayout()
        self.custom = QRadioButton(tr("Length:"), self)
        self.custom.setObjectName("creditsFit.custom")
        self.seconds = QDoubleSpinBox(self)
        self.seconds.setObjectName("creditsFit.seconds")
        self.seconds.setRange(0.1, 86_400.0)
        self.seconds.setDecimals(2)
        self.seconds.setSuffix(" s")
        self.seconds.valueChanged.connect(lambda _v: self.custom.setChecked(True))
        custom_row = QHBoxLayout()
        custom_row.addWidget(self.custom)
        custom_row.addWidget(self.seconds)
        custom_row.addStretch(1)

        self.cards_on_beat = QCheckBox(tr("Cards last whole beats (cuts on the beat)"), self)
        self.cards_on_beat.setObjectName("creditsFit.cardsOnBeat")
        self.align = QCheckBox(tr("Start the credits where the music starts"), self)
        self.align.setObjectName("creditsFit.alignToAudio")

        form = QFormLayout()
        form.addRow(tr("Music"), self.music)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(self.facts)
        layout.addWidget(QLabel(tr("Length of the credits"), self))
        layout.addLayout(self.choice_box)
        layout.addLayout(custom_row)
        layout.addWidget(self.cards_on_beat)
        layout.addWidget(self.align)

        self.status = QLabel(self)
        self.status.setObjectName("creditsFit.status")
        self.status.setWordWrap(True)
        self.off_button = QPushButton(tr("Remove Fitting"), self)
        self.off_button.setObjectName("creditsFit.off")
        self.off_button.clicked.connect(self.remove)
        self.apply_button = QPushButton(tr("Fit to This Length"), self)
        self.apply_button.setObjectName("creditsFit.apply")
        self.apply_button.setDefault(True)
        self.apply_button.clicked.connect(self.apply)
        close = QPushButton(tr("Close"), self)
        close.setObjectName("creditsFit.close")
        close.clicked.connect(self.close)
        buttons = QHBoxLayout()
        buttons.addWidget(self.status, 1)
        buttons.addWidget(self.off_button)
        buttons.addWidget(self.apply_button)
        buttons.addWidget(close)
        layout.addLayout(buttons)
        self.resize(560, 360)

    # ----- music list and analysis -----
    def open_for(self, sequence: dict[str, Any] | None) -> None:
        self.credits = None
        self._loading = True
        try:
            self.music.clear()
            for c in audio_clips(sequence):
                self.music.addItem(f"{c.get('name') or ''} ({format_seconds(c.get('start'))})", c.get("clip"))
        finally:
            self._loading = False
        self.show()
        self.raise_()
        self.activateWindow()
        # the engine picks the music (selected audio clip, or the one under the credits)
        self.analyse(None)

    def _music_changed(self, _i: int) -> None:
        if not self._loading:
            self.analyse(self.music.currentData())

    def analyse(self, audio: int | None) -> None:
        params: dict[str, Any] = {}
        if audio is not None:
            params["audio"] = audio
        if self.credits is not None:
            params["clip"] = self.credits
        self.status.setText(tr("Listening to the music…"))
        self.apply_button.setEnabled(False)
        self.engine.execute("graphics.credits.music", params, on_reply=lambda r, _b: self._show(r), on_error=self._failed)

    def _failed(self, message: str) -> None:
        self.info = None
        self.status.setText(message)
        self.apply_button.setEnabled(False)

    def _show(self, info: Any) -> None:
        if not isinstance(info, dict):
            return
        self.info = info
        if isinstance(info.get("credits"), int):
            self.credits = info["credits"]
        self._loading = True
        try:
            idx = self.music.findData(info.get("audio"))
            if idx >= 0:
                self.music.setCurrentIndex(idx)
        finally:
            self._loading = False
        facts = [tr("Length: {time}").replace("{time}", format_seconds(info.get("duration")))]
        if info.get("soundEnd") is not None:
            facts.append(tr("the sound ends at {time}").replace("{time}", format_seconds(info.get("soundEnd"))))
        if info.get("bpm"):
            facts.append(tr("tempo {bpm} BPM").replace("{bpm}", local_number(info.get("bpm"))))
        if info.get("natural") is not None:
            facts.append(tr("the credits without fitting: {time}").replace("{time}", format_seconds(info.get("natural"))))
        self.facts.setText(" · ".join(facts))
        # one choice per offered length
        for b in self.choices.buttons():
            if b is self.custom:
                continue
            self.choices.removeButton(b)
            b.deleteLater()
        first = True
        for sug in info.get("suggestions") or []:
            sid = sug.get("id")
            if sid not in SUGGESTIONS:
                continue
            b = QRadioButton(tr(SUGGESTIONS[sid]).replace("{time}", format_seconds(sug.get("total"))), self)
            b.setObjectName(f"creditsFit.use.{sid}")
            b.setProperty("use", sid)
            self.choices.addButton(b)
            self.choice_box.addWidget(b)
            if first:
                b.setChecked(True)
                first = False
        if self.custom not in self.choices.buttons():
            self.choices.addButton(self.custom)
        if first:
            # no music to measure: a typed length is the only choice
            self.custom.setChecked(True)
        dur = info.get("duration") if info.get("duration") is not None else info.get("natural")
        if isinstance(dur, int):
            self.seconds.blockSignals(True)
            self.seconds.setValue(dur / TICKS_PER_SECOND)
            self.seconds.blockSignals(False)
        has_beat = bool(info.get("beat"))
        self.align.setEnabled(info.get("audio") is not None)
        if info.get("audio") is None:
            self.align.setChecked(False)
        self.cards_on_beat.setEnabled(has_beat)
        if not has_beat:
            self.cards_on_beat.setChecked(False)
        self.off_button.setVisible(bool(info.get("fit")))
        if self.credits is None:
            self.status.setText(tr("Select a credits page on the timeline."))
        elif info.get("audio") is None:
            self.status.setText(tr("No music to measure ({problem}); type the length.").replace("{problem}", str(info.get("problem") or "")))
        else:
            self.status.setText("")
        self.apply_button.setEnabled(self.credits is not None)

    # ----- fit -----
    def fit_params(self) -> dict[str, Any]:
        p: dict[str, Any] = {"cardsOnBeat": self.cards_on_beat.isChecked(), "alignToAudio": self.align.isChecked()}
        if self.credits is not None:
            p["clip"] = self.credits
        if self.info and isinstance(self.info.get("audio"), int):
            p["audio"] = self.info["audio"]
        chosen = self.choices.checkedButton()
        if chosen is None or chosen is self.custom:
            p["seconds"] = self.seconds.value()
            if not (self.cards_on_beat.isChecked() or self.align.isChecked()):
                p.pop("audio", None)
        else:
            p["use"] = chosen.property("use")
        return p

    def apply(self) -> None:
        self._run(self.fit_params())

    def remove(self) -> None:
        self._run({"clip": self.credits, "off": True} if self.credits is not None else {"off": True})

    def _run(self, params: dict[str, Any]) -> None:
        self.apply_button.setEnabled(False)
        self.status.setText(tr("Fitting…"))

        def done(result: Any, _b: bytes | None) -> None:
            self.apply_button.setEnabled(True)
            report = (result or {}).get("report") if isinstance(result, dict) else None
            clips = (result or {}).get("clips") if isinstance(result, dict) else None
            if clips and isinstance(clips[0], dict) and isinstance(clips[0].get("clip"), int):
                self.credits = clips[0]["clip"]
            if isinstance(report, dict):
                self.status.setText(tr("The credits now last {time}.").replace("{time}", format_seconds(report.get("result"))))
            else:
                self.status.setText(tr("Fitting removed."))
            self.off_button.setVisible(isinstance(report, dict))
            self.fitted.emit(result)

        def failed(message: str) -> None:
            self.apply_button.setEnabled(True)
            self.status.setText(message)

        self.engine.execute("graphics.credits.fit", params, on_reply=done, on_error=failed)
