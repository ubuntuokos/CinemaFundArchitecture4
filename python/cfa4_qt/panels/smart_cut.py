"""Source monitor ▸ Settings (wrench) ▸ Smart Cut Source… (WP-08 of CFA4-SMART-CUT-RENDER-R1).

The clip of the Source monitor is cut into a new file without re-encoding what can be copied: the
engine analyses the clip (``export.analyze``), plans its marked range (``export.plan``) and writes
and verifies the cut (``export.start``). GOP Smart Cut is a Source monitor action, not an export
mode (SCE-001 v1.5). The dialog shows the plan before anything is written: the copied and
re-encoded frames and, when the cut cannot run, why. A plan that re-encodes its boundaries asks
for confirmation first.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, fmt, tr

MODES = (("gop-boundary", N_("At GOP boundaries (no re-encoding)")), ("frame-accurate", N_("Frame accurate (re-encode the boundaries)")))
GUARANTEES = (("bit-exact", N_("Bit-exact copy")), ("compatible", N_("Compatible copy")))


def default_output(path: str) -> str:
    """``clip.mp4`` → ``clip_cut.mp4`` (MOV stays MOV)."""
    stem, dot, ext = path.rpartition(".")
    if not dot or "/" in ext:
        return f"{path}_cut.mp4"
    return f"{stem}_cut.{ext if ext.lower() in ('mp4', 'm4v', 'mov') else 'mp4'}"


def plan_text(result: dict[str, Any]) -> tuple[str, str | None]:
    """The plan summary and, when the plan cannot run, why (human readable; SCE-001 §10.4)."""
    summary = result.get("summary") or {}
    copied, reencoded = int(summary.get("copiedFrames") or 0), int(summary.get("reencodedFrames") or 0)
    text = fmt("{copied} frames copied, {reencoded} frames re-encoded", copied=copied, reencoded=reencoded)
    plan = result.get("plan") or {}
    blockers = [str(b) for b in plan.get("blockers") or []]
    boundary = result.get("boundaryReencode") or {}
    if boundary.get("problem"):
        blockers.append(str(boundary["problem"]))
    if not plan.get("executable", True) or blockers:
        return text, "; ".join(blockers) or tr("The cut cannot run.")
    return text, None


class SmartCutDialog(QDialog):
    """Smart Cut of the Source monitor's clip (its marked range)."""

    def __init__(self, parent: QWidget, engine: EngineClient, item: int, said: Callable[[str], None]) -> None:
        super().__init__(parent)
        self.setObjectName("source.smartCut")
        self.setWindowTitle(tr("Smart Cut Source"))
        self.engine = engine
        self.item = item
        self.said = said
        self.analysis: dict[str, Any] | None = None
        self.plan: dict[str, Any] | None = None
        self._plan_request = 0
        lay = QVBoxLayout(self)
        self.info = QLabel(tr("Analyzing the clip…"), self)
        self.info.setObjectName("source.smartCut.info")
        self.info.setWordWrap(True)
        lay.addWidget(self.info)
        form = QFormLayout()
        self.mode = QComboBox(self)
        self.mode.setObjectName("source.smartCut.mode")
        for key, label in MODES:
            self.mode.addItem(tr(label), key)
        form.addRow(tr("Cut:"), self.mode)
        self.guarantee = QComboBox(self)
        self.guarantee.setObjectName("source.smartCut.guarantee")
        for key, label in GUARANTEES:
            self.guarantee.addItem(tr(label), key)
        form.addRow(tr("Copy:"), self.guarantee)
        out = QHBoxLayout()
        self.output = QLineEdit(self)
        self.output.setObjectName("source.smartCut.output")
        out.addWidget(self.output, 1)
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName("source.smartCut.browse")
        browse.setAutoDefault(False)
        browse.clicked.connect(self.browse)
        out.addWidget(browse)
        form.addRow(tr("Output:"), out)
        lay.addLayout(form)
        self.summary = QLabel("", self)
        self.summary.setObjectName("source.smartCut.plan")
        self.summary.setWordWrap(True)
        lay.addWidget(self.summary)
        self.error = QLabel("", self)
        self.error.setObjectName("source.smartCut.error")
        self.error.setStyleSheet("color: #e08a6a;")
        self.error.setWordWrap(True)
        self.error.hide()
        lay.addWidget(self.error)
        row = QHBoxLayout()
        row.addStretch(1)
        cancel = QPushButton(tr("Cancel"), self)
        cancel.setObjectName("source.smartCut.cancel")
        cancel.setAutoDefault(False)
        cancel.clicked.connect(self.reject)
        row.addWidget(cancel)
        self.run = QPushButton(tr("Export"), self)
        self.run.setObjectName("source.smartCut.run")
        self.run.setDefault(True)
        self.run.setEnabled(False)
        self.run.clicked.connect(self.start)
        row.addWidget(self.run)
        lay.addLayout(row)
        self.mode.currentIndexChanged.connect(lambda _i: self.replan())
        self.guarantee.currentIndexChanged.connect(lambda _i: self.replan())
        self.engine.execute("export.analyze", {"item": item}, on_reply=lambda r, _b: self._analyzed(r), on_error=self._fail)

    # ----- engine replies -----
    def _fail(self, message: str) -> None:
        self.error.setText(message)
        self.error.show()
        self.run.setEnabled(False)

    def _analyzed(self, result: Any) -> None:
        if not isinstance(result, dict):
            self._fail(tr("The engine sent no analysis."))
            return
        self.analysis = result
        points = [p for p in result.get("accessPoints") or [] if isinstance(p, dict) and p.get("clean")]
        self.info.setText(
            fmt(
                "{codec} in {container}: {gops} GOPs, {points} clean cut points",
                codec=str(result.get("codec") or "?").upper(),
                container=str(result.get("container") or "?").upper(),
                gops=int(result.get("gops") or 0),
                points=len(points),
            )
        )
        if not self.output.text():
            self.output.setText(default_output(str(result.get("path") or "")))
        cut = result.get("smartCut") or {}
        if not cut.get("allowed"):
            self._fail(fmt("Smart Cut is not available for this clip: {why}", why="; ".join(str(b) for b in cut.get("blockers") or [])))
            return
        self.replan()

    def params(self) -> dict[str, Any]:
        return {"item": self.item, "mode": self.mode.currentData(), "guarantee": self.guarantee.currentData(), "output": self.output.text().strip() or None}

    def replan(self) -> None:
        if self.analysis is None or not (self.analysis.get("smartCut") or {}).get("allowed"):
            return
        self.error.hide()
        self.run.setEnabled(False)
        self._plan_request += 1
        request = self._plan_request

        def got(result: Any) -> None:
            if request != self._plan_request or not isinstance(result, dict):
                return
            self.plan = result
            text, problem = plan_text(result)
            self.summary.setText(text)
            if problem:
                self._fail(problem)
            else:
                self.run.setEnabled(True)

        def failed(message: str) -> None:
            if request == self._plan_request:
                self.summary.setText("")
                self._fail(message)

        self.engine.execute("export.plan", self.params(), on_reply=lambda r, _b: got(r), on_error=failed)

    # ----- actions -----
    def browse(self) -> None:
        path, _f = QFileDialog.getSaveFileName(self, tr("Smart Cut Source"), self.output.text(), "MP4 / MOV (*.mp4 *.m4v *.mov)")
        if path:
            self.output.setText(path)
            self.replan()

    def start(self) -> None:
        params = self.params()
        if not params["output"]:
            self._fail(tr("Choose the output file."))
            return
        summary = (self.plan or {}).get("summary") or {}
        if summary.get("requiresConfirmation"):
            asked = QMessageBox.question(
                self,
                tr("Smart Cut Source"),
                fmt("{frames} frames at the boundaries are re-encoded. Continue?", frames=int(summary.get("reencodedFrames") or 0)),
            )
            if asked != QMessageBox.StandardButton.Yes:
                return
            params["confirm"] = True
        self.run.setEnabled(False)
        self.summary.setText(tr("Writing and verifying the cut…"))

        def done(result: Any) -> None:
            result = result if isinstance(result, dict) else {}
            report = result.get("report") or {}
            path = str(result.get("output") or params["output"])
            self.said(fmt("Smart Cut written and verified: {path} ({frames} frames)", path=path, frames=int(report.get("samples") or 0)))
            self.accept()

        def failed(message: str) -> None:
            self.summary.setText("")
            self._fail(message)

        self.engine.execute("export.start", params, on_reply=lambda r, _b: done(r), on_error=failed)
