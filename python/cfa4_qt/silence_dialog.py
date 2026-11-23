"""Tools ▸ Audio Tools ▸ Remove Silence… (F6 / V1-A, plan CFA4-V1-SILENCE-REMOVAL-001).

A non-modal dialog (the timeline stays visible): **Analyze** runs the engine command
``audio.detectSpeech`` (it never deletes; the regions become range markers on the timeline and
rows of the list), every row has a check box (checked: remove, unchecked: keep, stored with
``silence.keep``), and **Remove selected** runs ``clip.removeSilence`` for what is still checked.
The dialog holds no editing logic of its own and has no one-key binding: the removal is always a
click on this button.

The detector list offers **Energy (level)** and, when the engine was built with it, **Speech model
(Silero VAD)** (V1-B). The model is never bundled: the dialog shows the engine's record
(``speech.vadModel``: source, licence, size), and only after the user ticks the licence acceptance
the **Download** button runs ``speech.downloadVadModel``. A missing model is a message that says
so, never a silent fall back to the energy detector.

Every widget has an automation id (``removeSilence.*``) and the state is readable and settable as
JSON (``ui_state`` / ``set_ui_state``).
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QTimer, Qt
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QProgressBar,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from .i18n import N_, fmt, tr
from .timecode import TICKS_PER_SECOND

SCOPES = (
    ("selection", N_("Selected clips")),
    ("track", N_("Track of the selected clips")),
    ("sequence", N_("Whole sequence")),
)
MODES = (("cut", N_("Delete")), ("mark", N_("Mark only")))
ID_ROLE = int(Qt.ItemDataRole.UserRole)
POLL_MS = 150


def seconds(ticks: Any) -> float:
    """Ticks as seconds (a value that is not a number is 0)."""
    if isinstance(ticks, bool) or not isinstance(ticks, (int, float)):
        return 0.0
    return float(ticks) / TICKS_PER_SECOND


class RemoveSilenceDialog(QDialog):
    def __init__(self, win: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.win = win
        self.engine = win.engine
        self.setObjectName("dialog.removeSilence")
        self.setWindowTitle(tr("Remove Silence"))
        self.setModal(False)
        self.resize(520, 560)
        # the regions as the engine reported them: (id, start ticks, duration ticks)
        self._regions: list[tuple[int, int, int]] = []
        self._job: int | None = None
        # the engine's record of the speech model (``speech.vadModel``), None until asked
        self._model: dict[str, Any] | None = None
        self._timer = QTimer(self)
        self._timer.setInterval(POLL_MS)
        self._timer.timeout.connect(self._poll)

        root = QVBoxLayout(self)
        form = QFormLayout()
        self.scope = QComboBox()
        self.scope.setObjectName("removeSilence.scope")
        for key, label in SCOPES:
            self.scope.addItem(tr(label), key)
        form.addRow(tr("Scope:"), self.scope)
        self.detector = QComboBox()
        self.detector.setObjectName("removeSilence.detector")
        self.detector.addItem(tr("Energy (level)"), "energy")
        self.detector.addItem(tr("Speech model (Silero VAD)"), "silero")
        self.detector.currentIndexChanged.connect(self._detector_changed)
        form.addRow(tr("Detector:"), self.detector)
        # the speech model: its record, the licence acceptance and the download (shown for it only)
        self.model_box = QWidget()
        self.model_box.setObjectName("removeSilence.modelBox")
        mb = QVBoxLayout(self.model_box)
        mb.setContentsMargins(0, 0, 0, 0)
        self.model_info = QLabel("")
        self.model_info.setObjectName("removeSilence.modelInfo")
        self.model_info.setWordWrap(True)
        self.model_info.setOpenExternalLinks(True)
        mb.addWidget(self.model_info)
        self.accept_licence = QCheckBox(tr("I have read the model's licence and accept it"))
        self.accept_licence.setObjectName("removeSilence.acceptLicence")
        self.accept_licence.toggled.connect(self._refresh_model_buttons)
        mb.addWidget(self.accept_licence)
        self.download = QPushButton(tr("Download the speech model"))
        self.download.setObjectName("removeSilence.downloadModel")
        self.download.clicked.connect(self.run_download)
        mb.addWidget(self.download)
        self.model_box.setVisible(False)
        form.addRow("", self.model_box)
        self.auto_threshold = QCheckBox(tr("Automatic threshold"))
        self.auto_threshold.setObjectName("removeSilence.autoThreshold")
        self.auto_threshold.setChecked(True)
        self.auto_threshold.toggled.connect(self._auto_toggled)
        form.addRow("", self.auto_threshold)
        self.threshold = QDoubleSpinBox()
        self.threshold.setObjectName("removeSilence.threshold")
        self.threshold.setRange(-120.0, 0.0)
        self.threshold.setValue(-40.0)
        self.threshold.setSuffix(" dB")
        self.threshold.setEnabled(False)
        form.addRow(tr("Threshold:"), self.threshold)
        self.min_silence = QDoubleSpinBox()
        self.min_silence.setObjectName("removeSilence.minSilence")
        self.min_silence.setRange(0.0, 3600.0)
        self.min_silence.setDecimals(2)
        self.min_silence.setSingleStep(0.1)
        self.min_silence.setValue(0.5)
        self.min_silence.setSuffix(" s")
        form.addRow(tr("Minimum silence:"), self.min_silence)
        self.padding = QDoubleSpinBox()
        self.padding.setObjectName("removeSilence.padding")
        self.padding.setRange(0.0, 60.0)
        self.padding.setDecimals(2)
        self.padding.setSingleStep(0.05)
        self.padding.setValue(0.1)
        self.padding.setSuffix(" s")
        form.addRow(tr("Padding:"), self.padding)
        root.addLayout(form)

        self.analyze = QPushButton(tr("Analyze"))
        self.analyze.setObjectName("removeSilence.analyze")
        self.analyze.clicked.connect(self.run_analyze)
        root.addWidget(self.analyze)
        self.progress = QProgressBar()
        self.progress.setObjectName("removeSilence.progress")
        self.progress.setRange(0, 1000)
        self.progress.setVisible(False)
        root.addWidget(self.progress)

        root.addWidget(QLabel(tr("Silent regions (checked ones are removed):")))
        self.regions = QListWidget()
        self.regions.setObjectName("removeSilence.regions")
        self.regions.itemChanged.connect(self._item_changed)
        self.regions.currentRowChanged.connect(self._row_changed)
        root.addWidget(self.regions, 1)
        row = QHBoxLayout()
        self.select_all = QPushButton(tr("Select all"))
        self.select_all.setObjectName("removeSilence.selectAll")
        self.select_all.clicked.connect(lambda _c=False: self._set_all(True))
        self.select_none = QPushButton(tr("Select none"))
        self.select_none.setObjectName("removeSilence.selectNone")
        self.select_none.clicked.connect(lambda _c=False: self._set_all(False))
        self.clear = QPushButton(tr("Clear silence markers"))
        self.clear.setObjectName("removeSilence.clear")
        self.clear.clicked.connect(self.run_clear)
        row.addWidget(self.select_all)
        row.addWidget(self.select_none)
        row.addStretch(1)
        row.addWidget(self.clear)
        root.addLayout(row)

        opts = QFormLayout()
        self.ripple = QCheckBox(tr("Ripple (close the gaps)"))
        self.ripple.setObjectName("removeSilence.ripple")
        self.ripple.setChecked(True)
        opts.addRow("", self.ripple)
        self.mode = QComboBox()
        self.mode.setObjectName("removeSilence.mode")
        for key, label in MODES:
            self.mode.addItem(tr(label), key)
        opts.addRow(tr("Mode:"), self.mode)
        root.addLayout(opts)

        self.status = QLabel("")
        self.status.setObjectName("removeSilence.status")
        self.status.setWordWrap(True)
        root.addWidget(self.status)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        self.remove = QPushButton(tr("Remove selected"))
        self.remove.setObjectName("removeSilence.remove")
        self.remove.setEnabled(False)
        self.remove.clicked.connect(self.run_remove)
        close = QPushButton(tr("Close"))
        close.setObjectName("removeSilence.close")
        close.clicked.connect(self.close)
        buttons.addWidget(self.remove)
        buttons.addWidget(close)
        root.addLayout(buttons)
        self.refresh_model()

    # ----- the speech model -----
    def uses_model(self) -> bool:
        return str(self.detector.currentData()) == "silero"

    def model_installed(self) -> bool:
        return bool(self._model and self._model.get("installed") is True)

    def refresh_model(self) -> None:
        """Ask the engine for the speech model's record and show what it says."""

        def got(result: Any, _blob: bytes | None) -> None:
            self._model = result if isinstance(result, dict) else None
            self._show_model()

        self.engine.execute("speech.vadModel", {}, on_reply=got, on_error=self._model_error)

    def _model_error(self, message: str) -> None:
        self._model = None
        self.model_info.setText(message)
        self._refresh_model_buttons()

    def _show_model(self) -> None:
        m = self._model or {}
        silero = self.detector.findData("silero")
        available = m.get("detectorAvailable") is True
        item = self.detector.model().item(silero) if silero >= 0 else None  # type: ignore[attr-defined]
        if item is not None:
            item.setEnabled(available)
        if not available and self.uses_model():
            self.detector.setCurrentIndex(self.detector.findData("energy"))
        size = m.get("size")
        mb = f"{size / 1_000_000:.1f}" if isinstance(size, int) and not isinstance(size, bool) else "?"
        if m.get("installed") is True:
            text = tr("The speech model is installed.")
        elif not available:
            text = tr("This build has no speech-model detector.")
        else:
            lic = str(m.get("license", ""))
            url = str(m.get("licenseUrl", ""))
            text = fmt(
                "The speech model ({mb} MB, {licence}) is not downloaded: read the licence, accept it, download it.",
                mb=mb,
                licence=f'<a href="{url}">{lic}</a>' if url.startswith("https://") else lic,
            )
        self.model_info.setText(text)
        self._refresh_model_buttons()

    def _refresh_model_buttons(self, *_args: Any) -> None:
        m = self._model or {}
        needs = not self.model_installed() and m.get("detectorAvailable") is True
        self.accept_licence.setVisible(needs)
        self.download.setVisible(needs)
        self.download.setEnabled(needs and m.get("downloadAvailable") is True and self.accept_licence.isChecked())

    def _detector_changed(self, _index: int = 0) -> None:
        model = self.uses_model()
        self.model_box.setVisible(model)
        # the model decides what is speech: the level threshold does not apply to it
        self.auto_threshold.setEnabled(not model)
        self.threshold.setEnabled(not model and not self.auto_threshold.isChecked())
        if model:
            self.refresh_model()

    def run_download(self) -> None:
        """Download the model after the licence was accepted (the engine refuses without it)."""
        if not self.accept_licence.isChecked():
            self.status.setText(tr("Accept the licence first."))
            return
        self.download.setEnabled(False)
        self.status.setText(tr("Downloading the speech model…"))

        def done(_result: Any, _blob: bytes | None) -> None:
            self.status.setText(tr("The speech model was downloaded."))
            self.refresh_model()

        def failed(message: str) -> None:
            self.status.setText(message)
            self._refresh_model_buttons()

        self.engine.execute("speech.downloadVadModel", {"licenseAccepted": True}, on_reply=done, on_error=failed)

    def _auto_toggled(self, on: bool) -> None:
        self.threshold.setEnabled(not on and not self.uses_model())

    # ----- state -----
    def params(self) -> dict[str, Any]:
        """The ``audio.detectSpeech`` parameters of the current settings (durations in ticks)."""
        p: dict[str, Any] = {
            "scope": str(self.scope.currentData()),
            "detector": str(self.detector.currentData()),
            "background": True,
            "minSilence": round(self.min_silence.value() * TICKS_PER_SECOND),
            "padding": round(self.padding.value() * TICKS_PER_SECOND),
        }
        if not self.auto_threshold.isChecked() and not self.uses_model():
            p["thresholdDb"] = self.threshold.value()
        return p

    def checked_ids(self) -> list[int]:
        out: list[int] = []
        for i in range(self.regions.count()):
            item = self.regions.item(i)
            if item is not None and item.checkState() == Qt.CheckState.Checked:
                out.append(int(item.data(ID_ROLE)))
        return out

    def ui_state(self) -> dict[str, Any]:
        return {
            "scope": self.scope.currentData(),
            "detector": self.detector.currentData(),
            "modelInstalled": self.model_installed(),
            "licenceAccepted": self.accept_licence.isChecked(),
            "autoThreshold": self.auto_threshold.isChecked(),
            "thresholdDb": self.threshold.value(),
            "minSilenceSeconds": self.min_silence.value(),
            "paddingSeconds": self.padding.value(),
            "ripple": self.ripple.isChecked(),
            "mode": self.mode.currentData(),
            "regions": [
                {"id": r[0], "start": r[1], "duration": r[2], "remove": r[0] in self.checked_ids()}
                for r in self._regions
            ],
            "status": self.status.text(),
        }

    def set_ui_state(self, patch: dict[str, Any]) -> None:
        for key, box in (("scope", self.scope), ("detector", self.detector), ("mode", self.mode)):
            if key in patch:
                i = box.findData(patch[key])
                if i >= 0:
                    box.setCurrentIndex(i)
        if "licenceAccepted" in patch:
            self.accept_licence.setChecked(bool(patch["licenceAccepted"]))
        if "autoThreshold" in patch:
            self.auto_threshold.setChecked(bool(patch["autoThreshold"]))
        if "thresholdDb" in patch:
            self.threshold.setValue(float(patch["thresholdDb"]))
        if "minSilenceSeconds" in patch:
            self.min_silence.setValue(float(patch["minSilenceSeconds"]))
        if "paddingSeconds" in patch:
            self.padding.setValue(float(patch["paddingSeconds"]))
        if "ripple" in patch:
            self.ripple.setChecked(bool(patch["ripple"]))

    # ----- the list -----
    def _fill(self, regions: list[Any]) -> None:
        """Rebuild the list from the engine's ``regions`` (a malformed entry is skipped)."""
        self._regions = []
        self.regions.blockSignals(True)
        self.regions.clear()
        for r in regions:
            if not isinstance(r, dict) or not isinstance(r.get("id"), int):
                continue
            rid, start, dur = int(r["id"]), r.get("start"), r.get("duration")
            if (
                isinstance(start, bool)
                or isinstance(dur, bool)
                or not isinstance(start, int)
                or not isinstance(dur, int)
            ):
                continue
            self._regions.append((rid, start, dur))
            text = fmt(
                "{n}: {start} s, {dur} s long",
                n=len(self._regions),
                start=f"{seconds(start):.2f}",
                dur=f"{seconds(dur):.2f}",
            )
            item = QListWidgetItem(text)
            item.setData(ID_ROLE, rid)
            item.setFlags(item.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            item.setCheckState(Qt.CheckState.Unchecked if r.get("keep") else Qt.CheckState.Checked)
            self.regions.addItem(item)
        self.regions.blockSignals(False)
        self.remove.setEnabled(bool(self._regions))

    def _set_all(self, on: bool) -> None:
        state = Qt.CheckState.Checked if on else Qt.CheckState.Unchecked
        for i in range(self.regions.count()):
            item = self.regions.item(i)
            if item is not None:
                item.setCheckState(state)

    def _item_changed(self, item: QListWidgetItem) -> None:
        rid = int(item.data(ID_ROLE))
        keep = item.checkState() != Qt.CheckState.Checked
        self.engine.execute(
            "silence.keep",
            {"ids": [rid], "keep": keep},
            on_reply=lambda _r, _b: self.win.schedule_refresh(),
            on_error=self._failed,
        )

    def _row_changed(self, row: int) -> None:
        """The playhead goes to the start of the region."""
        if 0 <= row < len(self._regions):
            self.win._seek_program(self._regions[row][1])

    def _failed(self, message: str) -> None:
        self.status.setText(message)

    # ----- the commands -----
    def running(self) -> bool:
        return self._job is not None

    def run_analyze(self) -> None:
        """Start the analysis as a background job; the Analyze button becomes Cancel."""
        if self.running():
            self._cancel()
            return

        def started(result: Any, _blob: bytes | None) -> None:
            job = result.get("job") if isinstance(result, dict) else None
            if not isinstance(job, int) or isinstance(job, bool):
                self._stop(tr("Analysis failed."))
                return
            self._job = job
            self.progress.setValue(0)
            self.progress.setVisible(True)
            self.analyze.setText(tr("Cancel"))
            self._timer.start()

        if self.uses_model() and not self.model_installed():
            self.status.setText(tr("The speech model is not downloaded: accept its licence and download it, or use the energy detector."))
            return
        self.status.setText(tr("Analyzing…"))
        self.engine.execute("audio.detectSpeech", self.params(), on_reply=started, on_error=self._stop)

    def _cancel(self) -> None:
        if self._job is not None:
            self.engine.execute("jobs.cancel", {"job": self._job}, on_error=lambda _m: None)
            self.status.setText(tr("Stopping…"))

    def _stop(self, text: str) -> None:
        self._timer.stop()
        self._job = None
        self.progress.setVisible(False)
        self.analyze.setText(tr("Analyze"))
        self.status.setText(text)

    def _poll(self) -> None:
        job = self._job
        if job is None:
            return

        def got(result: Any, _blob: bytes | None) -> None:
            if self._job != job or not isinstance(result, list):
                return
            mine = next((j for j in result if isinstance(j, dict) and j.get("id") == job), None)
            if mine is None:
                self._stop(tr("Analysis failed."))
                return
            fraction = mine.get("progress")
            if isinstance(fraction, (int, float)) and not isinstance(fraction, bool):
                self.progress.setValue(round(max(0.0, min(1.0, float(fraction))) * 1000))
            if not mine.get("finished"):
                return
            res = mine.get("result")
            if isinstance(res, dict) and res.get("error"):
                error = str(res["error"])
                self._stop(tr("Analysis cancelled.") if "cancel" in error else error)
                return
            self._stop(tr("Analyzing…"))
            self.engine.execute("silence.mark", {"job": job}, on_reply=self._marked, on_error=self._stop)

        self.engine.execute("jobs.list", {}, on_reply=got, on_error=lambda _m: None)

    def _marked(self, result: Any, _blob: bytes | None) -> None:
        regions = result.get("regions") if isinstance(result, dict) else None
        self._fill(regions if isinstance(regions, list) else [])
        total = seconds(result.get("ticks")) if isinstance(result, dict) else 0.0
        if self._regions:
            self.status.setText(fmt("{n} silent regions, {s} s in all", n=len(self._regions), s=f"{total:.2f}"))
        else:
            self.status.setText(tr("No silence found with these settings."))
        self.win.schedule_refresh()

    def run_remove(self) -> None:
        ids = self.checked_ids()
        if not ids:
            self.status.setText(tr("No region is checked."))
            return
        mode = str(self.mode.currentData())

        def done(result: Any, _blob: bytes | None) -> None:
            n = result.get("removed") if isinstance(result, dict) else 0
            if mode == "mark":
                self.status.setText(tr("Marked only: nothing was deleted."))
            else:
                self.status.setText(fmt("{n} regions removed", n=n if isinstance(n, int) else 0))
                self._fill([])
            self.win.schedule_refresh()

        self.engine.execute(
            "clip.removeSilence",
            {"regions": ids, "ripple": self.ripple.isChecked(), "mode": mode},
            on_reply=done,
            on_error=self._failed,
        )

    def run_clear(self) -> None:
        def done(_result: Any, _blob: bytes | None) -> None:
            self._fill([])
            self.status.setText(tr("Silence markers cleared."))
            self.win.schedule_refresh()

        self.engine.execute("silence.clear", {}, on_reply=done, on_error=self._failed)


def open_remove_silence(win: Any) -> RemoveSilenceDialog:
    """Shows the dialog (one per window)."""
    dlg = getattr(win, "remove_silence", None)
    if dlg is None:
        dlg = RemoveSilenceDialog(win, win)
        win.remove_silence = dlg
    dlg.show()
    dlg.raise_()
    dlg.activateWindow()
    return dlg
