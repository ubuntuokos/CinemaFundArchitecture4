"""Tools ▸ Video Tools ▸ Find Segments… (CFA4-LOSSLESS-MEDIA-TOOLS-001 K4–K5).

The clip of the Source monitor is analysed for scene changes, black pictures or silence
(``segments.detectScenes`` / ``segments.detectBlack`` / ``segments.detectSilence``) in a
background job; the dialog polls ``segments.proposal`` and lists what was found. Nothing changes
until the user accepts: every proposed segment can be unchecked, renamed and moved (its first and
last frame), and the checked ones become the clip's segments (range markers) in one undo step
(``segments.add``). The source file is never touched.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt, QTimer
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFormLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QPushButton,
    QSpinBox,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, fmt, tr

TICKS_PER_SECOND = 254_016_000_000
KINDS = (
    ("scenes", "segments.detectScenes", N_("Scene changes")),
    ("black", "segments.detectBlack", N_("Black pictures")),
    ("silence", "segments.detectSilence", N_("Silence")),
)
POLL_MS = 200


def to_frame(ticks: int, fps: float) -> int:
    """Media ticks → the nearest frame boundary (frames are what the user edits; the boundaries
    found lie on frame starts, which rounding keeps exact for NTSC rates too)."""
    if fps <= 0:
        return 0
    return round(ticks * fps / TICKS_PER_SECOND)


def kind_params(kind: str, values: dict[str, float]) -> dict[str, Any]:
    """The engine parameters of an analysis from the dialog's fields."""
    if kind == "scenes":
        return {"sensitivity": values["sensitivity"], "minShotFrames": int(values["minShotFrames"])}
    if kind == "black":
        return {"pixelThreshold": values["pixelThreshold"] / 100.0, "pictureThreshold": values["pictureThreshold"] / 100.0, "minSeconds": values["blackSeconds"]}
    return {"thresholdDb": values["thresholdDb"], "minSeconds": values["silenceSeconds"]}


class SegmentFinderDialog(QDialog):
    """Find segments of the Source monitor's clip and add the accepted ones as range markers."""

    def __init__(self, parent: QWidget, engine: EngineClient, item: int, fps: float, said: Callable[[str], None], has_audio: bool = True) -> None:
        super().__init__(parent)
        self.setObjectName("source.findSegments")
        self.setWindowTitle(tr("Find Segments"))
        self.engine = engine
        self.item = item
        self.fps = float(fps or 0.0)
        self.said = said
        self.job: int | None = None
        self.proposal: dict[str, Any] | None = None
        self.closed = False  # replies that arrive after the dialog closed change nothing
        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(POLL_MS)
        self._timer.timeout.connect(self._poll)
        lay = QVBoxLayout(self)
        form = QFormLayout()
        self.kind = QComboBox(self)
        self.kind.setObjectName("source.findSegments.kind")
        for key, _cmd, label in KINDS:
            if key == "silence" and not has_audio:
                continue
            self.kind.addItem(tr(label), key)
        form.addRow(tr("Find:"), self.kind)
        self.fields: dict[str, QSpinBox | QDoubleSpinBox] = {}
        self.rows: dict[str, list[str]] = {"scenes": [], "black": [], "silence": []}

        def field(kind: str, key: str, label: str, lo: float, hi: float, value: float, decimals: int, suffix: str = "") -> None:
            box: QSpinBox | QDoubleSpinBox
            if decimals == 0:
                box = QSpinBox(self)
                box.setRange(int(lo), int(hi))
                box.setValue(int(value))
            else:
                box = QDoubleSpinBox(self)
                box.setDecimals(decimals)
                box.setRange(lo, hi)
                box.setValue(value)
            box.setObjectName(f"source.findSegments.{key}")
            if suffix:
                box.setSuffix(suffix)
            self.fields[key] = box
            self.rows[kind].append(key)
            form.addRow(tr(label), box)

        field("scenes", "sensitivity", N_("Sensitivity:"), 0, 100, 50, 0)
        field("scenes", "minShotFrames", N_("Shortest shot (frames):"), 1, 10_000, 6, 0)
        field("black", "pixelThreshold", N_("Black level (%):"), 0, 100, 10, 1)
        field("black", "pictureThreshold", N_("Black share of the picture (%):"), 0, 100, 98, 1)
        field("black", "blackSeconds", N_("Shortest black (s):"), 0, 3600, 0.5, 2)
        field("silence", "thresholdDb", N_("Silence below (dBFS):"), -120, 0, -60, 1)
        field("silence", "silenceSeconds", N_("Shortest silence (s):"), 0, 3600, 2.0, 2)
        self.form = form
        lay.addLayout(form)
        self.analyse = QPushButton(tr("Analyze"), self)
        self.analyse.setObjectName("source.findSegments.analyze")
        self.analyse.setAutoDefault(False)
        self.analyse.clicked.connect(self.start)
        lay.addWidget(self.analyse)
        self.state = QLabel("", self)
        self.state.setObjectName("source.findSegments.state")
        self.state.setWordWrap(True)
        lay.addWidget(self.state)
        self.table = QTableWidget(0, 3, self)
        self.table.setObjectName("source.findSegments.segments")
        self.table.setHorizontalHeaderLabels([tr("Name"), tr("First frame"), tr("Last frame")])
        self.table.horizontalHeader().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        lay.addWidget(self.table, 1)
        self.error = QLabel("", self)
        self.error.setObjectName("source.findSegments.error")
        self.error.setStyleSheet("color: #e08a6a;")
        self.error.setWordWrap(True)
        self.error.hide()
        lay.addWidget(self.error)
        row = QHBoxLayout()
        row.addStretch(1)
        cancel = QPushButton(tr("Cancel"), self)
        cancel.setObjectName("source.findSegments.cancel")
        cancel.setAutoDefault(False)
        cancel.clicked.connect(self.reject)
        row.addWidget(cancel)
        self.accept_button = QPushButton(tr("Add as Segments"), self)
        self.accept_button.setObjectName("source.findSegments.add")
        self.accept_button.setEnabled(False)
        self.accept_button.clicked.connect(self.add)
        row.addWidget(self.accept_button)
        lay.addLayout(row)
        self.kind.currentIndexChanged.connect(lambda _i: self._kind_changed())
        self._kind_changed()
        self.resize(460, 520)

    # ----- settings -----
    def current_kind(self) -> str:
        return str(self.kind.currentData() or "scenes")

    def _kind_changed(self) -> None:
        kind = self.current_kind()
        for k, keys in self.rows.items():
            for key in keys:
                self.form.setRowVisible(self.fields[key], k == kind)

    def params(self) -> dict[str, Any]:
        values = {k: float(b.value()) for k, b in self.fields.items()}
        return {"item": self.item, **kind_params(self.current_kind(), values)}

    # ----- analysis -----
    def _fail(self, message: str) -> None:
        if self.closed:
            return
        self.error.setText(message)
        self.error.show()
        self.analyse.setEnabled(True)

    def start(self) -> None:
        self.error.hide()
        self.table.setRowCount(0)
        self.accept_button.setEnabled(False)
        self.analyse.setEnabled(False)
        self.state.setText(tr("Analyzing…"))
        command = next(cmd for key, cmd, _l in KINDS if key == self.current_kind())
        self.engine.execute(command, self.params(), on_reply=lambda r, _b: self._started(r), on_error=self._fail)

    def _started(self, result: Any) -> None:
        if self.closed:
            # closed while the analysis was starting: the job it started is not wanted
            job = result.get("job") if isinstance(result, dict) else None
            if isinstance(job, int) and not isinstance(job, bool):
                self._cancel(job)
            return
        if not isinstance(result, dict) or not isinstance(result.get("job"), int):
            self._fail(tr("The engine did not start the analysis."))
            return
        self.job = result["job"]
        self._got(result)

    def _poll(self) -> None:
        if self.job is not None and not self.closed:
            self.engine.execute("segments.proposal", {"job": self.job}, on_reply=lambda r, _b: self._got(r), on_error=self._fail)

    def _got(self, result: Any) -> None:
        if self.closed or not isinstance(result, dict) or result.get("job") != self.job:
            return
        state = result.get("state")
        if state == "running":
            progress = result.get("progress") or {}
            done, total = int(progress.get("done") or 0), int(progress.get("total") or 0)
            self.state.setText(fmt("Analyzing… {percent}%", percent=done * 100 // total if total > 0 else 0))
            self._timer.start()
            return
        self.analyse.setEnabled(True)
        if state != "done":
            self.state.setText("")
            self._fail(str(result.get("error") or tr("The analysis stopped.")))
            return
        self.proposal = result
        self.show_proposal([s for s in result.get("segments") or [] if isinstance(s, dict)], bool(result.get("truncated")))

    def show_proposal(self, segments: list[dict[str, Any]], truncated: bool = False) -> None:
        self.table.setRowCount(0)
        for seg in segments:
            r = self.table.rowCount()
            self.table.insertRow(r)
            name = QTableWidgetItem(str(seg.get("name") or ""))
            name.setFlags(name.flags() | Qt.ItemFlag.ItemIsUserCheckable | Qt.ItemFlag.ItemIsEditable)
            name.setCheckState(Qt.CheckState.Checked)
            name.setData(Qt.ItemDataRole.UserRole, seg)
            self.table.setItem(r, 0, name)
            first = to_frame(int(seg.get("in") or 0), self.fps)
            # the last frame shown is the one before `out` (Out is inclusive in the Source monitor)
            last = max(first, to_frame(int(seg.get("out") or 0), self.fps) - 1)
            self.table.setItem(r, 1, QTableWidgetItem(str(first)))
            self.table.setItem(r, 2, QTableWidgetItem(str(last)))
        text = fmt("{count} segments found", count=len(segments))
        if truncated:
            text += " " + tr("(only the first ones are listed)")
        self.state.setText(text)
        self.accept_button.setEnabled(bool(segments))

    # ----- accept -----
    def chosen(self) -> list[dict[str, Any]]:
        """The checked rows as `segments.add` entries: frames the user changed are sent as frames,
        unchanged ones as the exact ticks found."""
        out: list[dict[str, Any]] = []
        for r in range(self.table.rowCount()):
            name = self.table.item(r, 0)
            if name is None or name.checkState() != Qt.CheckState.Checked:
                continue
            seg = name.data(Qt.ItemDataRole.UserRole) or {}
            first_found = to_frame(int(seg.get("in") or 0), self.fps)
            last_found = max(first_found, to_frame(int(seg.get("out") or 0), self.fps) - 1)
            entry: dict[str, Any] = {"name": name.text().strip(), "color": seg.get("color")}
            try:
                first = int((self.table.item(r, 1) or QTableWidgetItem("")).text())
                last = int((self.table.item(r, 2) or QTableWidgetItem("")).text())
            except ValueError as e:
                raise ValueError(fmt("Row {n}: the first and last frame must be whole numbers", n=r + 1)) from e
            entry["in"] = seg.get("in") if first == first_found else {"frame": first}
            entry["out"] = seg.get("out") if last == last_found else {"frame": last + 1}
            out.append(entry)
        return out

    def add(self) -> None:
        try:
            chosen = self.chosen()
        except ValueError as e:
            self._fail(str(e))
            return
        if not chosen:
            self._fail(tr("Check at least one segment."))
            return

        def done(result: Any) -> None:
            if self.closed:
                return
            added = len((result or {}).get("markers") or []) if isinstance(result, dict) else 0
            self.said(fmt("{count} segments added to the clip", count=added))
            self.accept()

        self.engine.execute("segments.add", {"item": self.item, "segments": chosen}, on_reply=lambda r, _b: done(r), on_error=self._fail)

    def _cancel(self, job: int) -> None:
        self.engine.execute("jobs.cancel", {"job": job}, on_reply=lambda _r, _b: None, on_error=lambda _m: None)

    def reject(self) -> None:
        self._timer.stop()
        self.closed = True
        if self.job is not None and (self.proposal is None or self.proposal.get("job") != self.job):
            self._cancel(self.job)
        super().reject()
