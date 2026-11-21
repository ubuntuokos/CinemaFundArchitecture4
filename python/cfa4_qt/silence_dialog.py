"""Tools ▸ Audio Tools ▸ Remove Silence… (F6 / V1-A, plan CFA4-V1-SILENCE-REMOVAL-001).

A non-modal dialog (the timeline stays visible): **Analyze** runs the engine command
``audio.detectSpeech`` (it never deletes; the regions become range markers on the timeline and
rows of the list), every row has a check box (checked: remove, unchecked: keep, stored with
``silence.keep``), and **Remove selected** runs ``clip.removeSilence`` for what is still checked.
The dialog holds no editing logic of its own and has no one-key binding: the removal is always a
click on this button.

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
        form.addRow(tr("Detector:"), self.detector)
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

    def _auto_toggled(self, on: bool) -> None:
        self.threshold.setEnabled(not on)

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
        if not self.auto_threshold.isChecked():
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
        for key, box in (("scope", self.scope), ("mode", self.mode)):
            if key in patch:
                i = box.findData(patch[key])
                if i >= 0:
                    box.setCurrentIndex(i)
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
