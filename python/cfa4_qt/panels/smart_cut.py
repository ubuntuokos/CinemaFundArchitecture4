"""Source monitor ▸ Settings (wrench) ▸ Smart Cut Source… (WP-08 of CFA4-SMART-CUT-RENDER-R1).

The clip of the Source monitor is cut into a new file without re-encoding what can be copied: the
engine analyses the clip (``export.analyze``), plans its marked range (``export.plan``) and writes
and verifies the cut (``export.start``). GOP Smart Cut is a Source monitor action, not an export
mode (SCE-001 v1.5). The dialog shows the plan before anything is written: the copied and
re-encoded frames and, when the cut cannot run, why. A plan that re-encodes its boundaries asks
for confirmation first.

Segments (CFA4-LOSSLESS-MEDIA-TOOLS-001 K1–K3): the clip's range markers are its segments. With
"Segments" chosen, the dialog lists them (checkable, in the order they are cut), plans and writes
one verified file per checked segment (``segments.plan`` / ``segments.export``, an output pattern
with ``{n}`` and ``{name}``), and makes subclips of them or places them on the timeline
(``segments.toSubclips`` / ``segments.toTimeline``). Nothing is written unless every checked
segment can run. "Find Segments…" proposes segments from scene changes, black or silence (K4–K5,
``segment_finder.py``).
"""

from __future__ import annotations

import os
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt, QTimer
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, fmt, tr

MODES = (("gop-boundary", N_("At GOP boundaries (no re-encoding)")), ("frame-accurate", N_("Frame accurate (re-encode the boundaries)")))
SOURCES = (("marked", N_("Marked range")), ("segments", N_("Segments (range markers)")))
GUARANTEES = (("bit-exact", N_("Bit-exact copy")), ("compatible", N_("Compatible copy")))

# the output extensions Smart Cut writes as they are (MP4 / MOV; MPEG program and transport streams)
KEPT_EXTENSIONS = ("mp4", "m4v", "mov", "mpg", "mpeg", "ts", "m2t", "m2ts", "mts")
OUTPUT_FILTER = "MP4 / MOV (*.mp4 *.m4v *.mov);;MPEG-PS / MPEG-TS (*.mpg *.mpeg *.ts *.m2ts *.mts)"
# typing in the output field plans again once the typing pauses
REPLAN_MS = 300


def default_output(path: str) -> str:
    """``clip.mp4`` → ``clip_cut.mp4``: MOV stays MOV, an MPEG program or transport stream stays one
    (WP-09c; a ``.vob`` becomes ``.mpg``)."""
    stem, dot, ext = path.rpartition(".")
    if not dot or "/" in ext:
        return f"{path}_cut.mp4"
    ext = ext.lower()
    if ext in ("vob", "m2p"):
        return f"{stem}_cut.mpg"
    return f"{stem}_cut.{ext if ext in KEPT_EXTENSIONS else 'mp4'}"


def default_pattern(path: str) -> str:
    """``clip.mp4`` → ``clip_{n}-{name}.mp4``: one file per segment, numbered and named after it."""
    out = default_output(path)
    stem, _dot, ext = out.rpartition(".")
    return f"{stem.removesuffix('_cut')}_{{n}}-{{name}}.{ext}"


def default_joined(path: str) -> str:
    """``clip.mp4`` → ``clip_joined.mp4``: the segments one after the other in one file (MP4 / MOV)."""
    out = default_output(path)
    stem, _dot, ext = out.rpartition(".")
    return f"{stem.removesuffix('_cut')}_joined.{ext if ext in ('mp4', 'm4v', 'mov') else 'mp4'}"


def segment_label(segment: dict[str, Any], rate: tuple[int, int] | None) -> str:
    """A segment's line in the list: its name (or "Segment"), and its length in frames when the
    frame rate is known."""
    name = str(segment.get("name") or "").strip() or tr("Segment")
    duration = segment.get("duration")
    if rate and isinstance(duration, int) and duration > 0 and rate[0] > 0 and rate[1] > 0:
        # ticks → frames: 254 016 000 000 ticks per second
        frames = duration * rate[0] // (254_016_000_000 * rate[1])
        return fmt("{name} ({frames} frames)", name=name, frames=frames)
    return name


def plan_text(result: dict[str, Any]) -> tuple[str, str | None]:
    """The plan summary and, when the plan cannot run, why (human readable; SCE-001 §10.4)."""
    if isinstance(result.get("ranges"), list):
        return ranges_text(result)
    summary = result.get("summary") or {}
    copied, reencoded = int(summary.get("copiedFrames") or 0), int(summary.get("reencodedFrames") or 0)
    text = fmt("{copied} frames copied, {reencoded} frames re-encoded", copied=copied, reencoded=reencoded)
    # the audio tracks copied with the video, and those that are not (WP-09b)
    audio = result.get("audio") or {}
    tracks = [t for t in audio.get("tracks") or [] if isinstance(t, dict)]
    if tracks:
        text += "\n" + fmt("Audio copied: {tracks}", tracks=", ".join(str(t.get("codec") or "?").upper() for t in tracks))
    for d in audio.get("dropped") or []:
        if isinstance(d, dict):
            text += "\n" + fmt("Audio track {track} is not copied: {why}", track=d.get("trackId", "?"), why=str(d.get("reason") or ""))
    plan = result.get("plan") or {}
    blockers = [str(b) for b in plan.get("blockers") or []]
    boundary = result.get("boundaryReencode") or {}
    if boundary.get("problem"):
        blockers.append(str(boundary["problem"]))
    if not plan.get("executable", True) or blockers:
        return text, "; ".join(blockers) or tr("The cut cannot run.")
    return text, None


def ranges_text(result: dict[str, Any]) -> tuple[str, str | None]:
    """The totals of a plan of several segments and, when one of them cannot run, why."""
    summary = result.get("summary") or {}
    copied, reencoded = int(summary.get("copiedFrames") or 0), int(summary.get("reencodedFrames") or 0)
    count = int(summary.get("ranges") or 0)
    if result.get("join"):
        text = fmt("{count} segments joined into one file: {copied} frames copied, {reencoded} frames re-encoded", count=count, copied=copied, reencoded=reencoded)
    else:
        text = fmt("{count} files: {copied} frames copied, {reencoded} frames re-encoded", count=count, copied=copied, reencoded=reencoded)
    problems = [p for p in result.get("problems") or [] if isinstance(p, dict)]
    if problems:
        return text, "; ".join(fmt("Segment {n}: {why}", n=p.get("range", "?"), why=str(p.get("problem") or "")) for p in problems)
    return text, None


class SmartCutDialog(QDialog):
    """Smart Cut of the Source monitor's clip (its marked range)."""

    def __init__(self, parent: QWidget, engine: EngineClient, item: int, said: Callable[[str], None]) -> None:
        super().__init__(parent)
        self.setObjectName("source.smartCut")
        self._chosen_output = ""
        self.setWindowTitle(tr("Smart Cut Source"))
        self.engine = engine
        self.item = item
        self.said = said
        self.analysis: dict[str, Any] | None = None
        self.plan: dict[str, Any] | None = None
        # the parameters `plan` was made for: Export runs only a plan of the current settings
        self.planned: dict[str, Any] | None = None
        self.finder: QDialog | None = None
        self._plan_request = 0
        self._replan_timer = QTimer(self)
        self._replan_timer.setSingleShot(True)
        self._replan_timer.setInterval(REPLAN_MS)
        self._replan_timer.timeout.connect(self.replan)
        # a marker's range or name can change while the dialog is open: the plan shown is then stale
        self.engine.event.connect(self._on_engine_event)
        lay = QVBoxLayout(self)
        self.info = QLabel(tr("Analyzing the clip…"), self)
        self.info.setObjectName("source.smartCut.info")
        self.info.setWordWrap(True)
        lay.addWidget(self.info)
        form = QFormLayout()
        self.source = QComboBox(self)
        self.source.setObjectName("source.smartCut.source")
        for key, label in SOURCES:
            self.source.addItem(tr(label), key)
        form.addRow(tr("Range:"), self.source)
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
        self.output.textChanged.connect(lambda _t: self._output_edited())
        out.addWidget(self.output, 1)
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName("source.smartCut.browse")
        browse.setAutoDefault(False)
        browse.clicked.connect(self.browse)
        out.addWidget(browse)
        form.addRow(tr("Output:"), out)
        lay.addLayout(form)
        # the clip's segments (range markers), checkable, in the order they are cut
        self.segments_box = QWidget(self)
        seg = QHBoxLayout(self.segments_box)
        seg.setContentsMargins(0, 0, 0, 0)
        self.segment_list = QListWidget(self.segments_box)
        self.segment_list.setObjectName("source.smartCut.segments")
        self.segment_list.itemChanged.connect(lambda _i: self.replan())
        moves = QVBoxLayout()
        for name, label, step in (("segmentUp", N_("Up"), -1), ("segmentDown", N_("Down"), 1)):
            b = QPushButton(tr(label), self.segments_box)
            b.setObjectName(f"source.smartCut.{name}")
            b.setAutoDefault(False)
            b.clicked.connect(lambda _c=False, d=step: self.move_segment(d))
            moves.addWidget(b)
        self.to_subclips = QPushButton(tr("Make Subclips"), self.segments_box)
        self.to_subclips.setObjectName("source.smartCut.toSubclips")
        self.to_subclips.setAutoDefault(False)
        self.to_subclips.clicked.connect(lambda _c=False: self.send("segments.toSubclips"))
        moves.addWidget(self.to_subclips)
        self.to_timeline = QPushButton(tr("To Timeline"), self.segments_box)
        self.to_timeline.setObjectName("source.smartCut.toTimeline")
        self.to_timeline.setAutoDefault(False)
        self.to_timeline.clicked.connect(lambda _c=False: self.send("segments.toTimeline"))
        moves.addWidget(self.to_timeline)
        self.find_button = QPushButton(tr("Find Segments…"), self.segments_box)
        self.find_button.setObjectName("source.smartCut.findSegments")
        self.find_button.setAutoDefault(False)
        self.find_button.clicked.connect(lambda _c=False: self.find_segments())
        moves.addWidget(self.find_button)
        moves.addStretch(1)
        seg.addLayout(moves)
        joins = QVBoxLayout()
        joins.addWidget(self.segment_list, 1)
        seg.insertLayout(0, joins, 1)
        # one file per segment, or the segments one after the other in one file (K1)
        self.join = QCheckBox(tr("Join the segments into one file"), self.segments_box)
        self.join.setObjectName("source.smartCut.join")
        self.join.toggled.connect(self._join_changed)
        joins.addWidget(self.join)
        self.segment_chapters = QCheckBox(tr("Segment names as chapters"), self.segments_box)
        self.segment_chapters.setObjectName("source.smartCut.segmentChapters")
        self.segment_chapters.setEnabled(False)
        self.segment_chapters.toggled.connect(lambda _on: self.replan())
        joins.addWidget(self.segment_chapters)
        self.segments_box.hide()
        lay.addWidget(self.segments_box)
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
        self.source.currentIndexChanged.connect(lambda _i: self._source_changed())
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

    # ----- segments -----
    def segments_mode(self) -> bool:
        return bool(self.source.currentData() == "segments")

    def joined(self) -> bool:
        return self.segments_mode() and self.join.isChecked()

    def _default_output(self, path: str) -> str:
        if not self.segments_mode():
            return default_output(path)
        return default_joined(path) if self.join.isChecked() else default_pattern(path)

    def _set_default_output(self) -> None:
        path = str((self.analysis or {}).get("path") or "")
        current = self.output.text().strip()
        if path and current in ("", default_output(path), default_pattern(path), default_joined(path)):
            self.output.setText(self._default_output(path))

    def _join_changed(self, on: bool) -> None:
        self.segment_chapters.setEnabled(on)
        self._set_default_output()
        self.replan()

    def _source_changed(self) -> None:
        segments = self.segments_mode()
        self.segments_box.setVisible(segments)
        self._set_default_output()
        if segments and self.segment_list.count() == 0:
            self.engine.execute("segments.list", {"item": self.item}, on_reply=lambda r, _b: self._listed(r), on_error=self._fail)
            return
        self.replan()

    def _listed(self, result: Any) -> None:
        segments = [m for m in (result or {}).get("segments") or [] if isinstance(m, dict)] if isinstance(result, dict) else []
        rate_json = (self.analysis or {}).get("frameRate") or {}
        rate = (int(rate_json.get("num") or 0), int(rate_json.get("den") or 0)) if isinstance(rate_json, dict) else None
        self.segment_list.blockSignals(True)
        self.segment_list.clear()
        for m in segments:
            it = QListWidgetItem(segment_label(m, rate))
            it.setData(Qt.ItemDataRole.UserRole, m.get("marker"))
            it.setFlags(it.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            it.setCheckState(Qt.CheckState.Checked)
            it.setToolTip(str(m.get("comment") or ""))
            self.segment_list.addItem(it)
        self.segment_list.blockSignals(False)
        if not segments:
            self.summary.setText("")
            self._fail(tr("This clip has no segments: add range markers in the Source monitor first."))
            return
        self.replan()

    def find_segments(self) -> QDialog:
        """Find Segments (K4–K5); the list is read again once segments were added."""
        from .segment_finder import SegmentFinderDialog

        rate = (self.analysis or {}).get("frameRate") or {}
        num, den = (int(rate.get("num") or 0), int(rate.get("den") or 0)) if isinstance(rate, dict) else (0, 0)
        d = SegmentFinderDialog(self, self.engine, self.item, num / den if num > 0 and den > 0 else 0.0, self.said)
        d.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        d.accepted.connect(self.reload_segments)
        self.finder = d
        d.open()
        return d

    def reload_segments(self) -> None:
        self.error.hide()
        self.engine.execute("segments.list", {"item": self.item}, on_reply=lambda r, _b: self._listed(r), on_error=self._fail)

    def chosen_segments(self) -> list[int]:
        out = []
        for i in range(self.segment_list.count()):
            it = self.segment_list.item(i)
            marker = it.data(Qt.ItemDataRole.UserRole)
            if it.checkState() == Qt.CheckState.Checked and isinstance(marker, int):
                out.append(marker)
        return out

    def move_segment(self, step: int) -> None:
        row = self.segment_list.currentRow()
        to = row + step
        if row < 0 or not 0 <= to < self.segment_list.count():
            return
        self.segment_list.blockSignals(True)
        it = self.segment_list.takeItem(row)
        self.segment_list.insertItem(to, it)
        self.segment_list.setCurrentRow(to)
        self.segment_list.blockSignals(False)
        self.replan()

    def send(self, command: str) -> None:
        chosen = self.chosen_segments()
        if not chosen:
            self._fail(tr("Check at least one segment."))
            return

        def done(result: Any) -> None:
            made = len((result or {}).get("subclips") or []) if isinstance(result, dict) else 0
            if command == "segments.toTimeline":
                self.said(fmt("{count} segments placed on the timeline", count=made))
            else:
                self.said(fmt("{count} subclips made from the segments", count=made))

        self.engine.execute(command, {"item": self.item, "segments": chosen}, on_reply=lambda r, _b: done(r), on_error=self._fail)

    def params(self) -> dict[str, Any]:
        if self.segments_mode():
            params: dict[str, Any] = {
                "item": self.item,
                "segments": self.chosen_segments(),
                "mode": self.mode.currentData(),
                "guarantee": self.guarantee.currentData(),
                "output": self.output.text().strip() or None,
            }
            if self.join.isChecked():
                params["join"] = True
                params["segmentChapters"] = self.segment_chapters.isChecked()
            return params
        return {"item": self.item, "mode": self.mode.currentData(), "guarantee": self.guarantee.currentData(), "output": self.output.text().strip() or None}

    def _invalidate(self) -> None:
        """The settings changed: the plan shown no longer applies, and an answer still on its way
        for the old settings is dropped."""
        self._replan_timer.stop()
        self._plan_request += 1
        self.plan = None
        self.planned = None
        self.run.setEnabled(False)

    def _on_engine_event(self, ev: Any) -> None:
        if not isinstance(ev, dict) or ev.get("event") != "projectChanged":
            return
        self._invalidate()
        if self.segments_mode():
            # the segments' ranges and names are read again, then planned again
            self.reload_segments()
        else:
            self._replan_timer.start()

    def _output_edited(self) -> None:
        self._invalidate()
        self._replan_timer.start()

    def plan_current(self) -> bool:
        """Whether the plan shown was made for the current settings."""
        return self.plan is not None and self.planned is not None and self.planned == self.params()

    def replan(self) -> None:
        self._invalidate()
        if self.analysis is None or not (self.analysis.get("smartCut") or {}).get("allowed"):
            return
        self.error.hide()
        segments = self.segments_mode()
        if segments and not self.chosen_segments():
            self.summary.setText("")
            if self.segment_list.count():
                self._fail(tr("Check at least one segment."))
            return
        request = self._plan_request
        params = self.params()

        def got(result: Any) -> None:
            if request != self._plan_request or not isinstance(result, dict):
                return
            self.plan = result
            self.planned = params
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

        self.engine.execute("segments.plan" if segments else "export.plan", params, on_reply=lambda r, _b: got(r), on_error=failed)

    # ----- actions -----
    def browse(self) -> None:
        path, _f = QFileDialog.getSaveFileName(self, tr("Smart Cut Source"), self.output.text(), OUTPUT_FILTER)
        if path:
            # the file dialog has already asked about replacing an existing file
            self._chosen_output = path
            self.output.setText(path)
            self.replan()

    def start(self) -> None:
        params = self.params()
        if not params["output"]:
            self._fail(tr("Choose the output file."))
            return
        if not self.plan_current():
            # the settings changed since the plan was made: plan them, then the user exports
            self.replan()
            return
        # the plan the user confirms: a change while a question is open (a renamed marker can
        # name another file) drops it, and nothing is written
        generation = self._plan_request

        def still_current() -> bool:
            return generation == self._plan_request and self.plan_current()

        # an existing file is replaced only once the user says so (plan 1.1 §7)
        if self.segments_mode():
            taken = [str(x) for x in (self.plan or {}).get("existing") or []]
        else:
            taken = [params["output"]] if params["output"] != self._chosen_output and os.path.exists(params["output"]) else []
        if taken:
            asked = QMessageBox.question(
                self,
                tr("Smart Cut Source"),
                fmt("These files already exist and will be replaced: {files}. Continue?", files=", ".join(taken)),
            )
            if asked != QMessageBox.StandardButton.Yes or not still_current():
                return
            if self.segments_mode():
                params["overwrite"] = True
        summary = (self.plan or {}).get("summary") or {}
        if summary.get("requiresConfirmation"):
            asked = QMessageBox.question(
                self,
                tr("Smart Cut Source"),
                fmt("{frames} frames at the boundaries are re-encoded. Continue?", frames=int(summary.get("reencodedFrames") or 0)),
            )
            if asked != QMessageBox.StandardButton.Yes or not still_current():
                return
            params["confirm"] = True
        self.run.setEnabled(False)
        self.summary.setText(tr("Writing and verifying the cut…"))

        segments = self.segments_mode()

        def done(result: Any) -> None:
            result = result if isinstance(result, dict) else {}
            if segments:
                count = len(result.get("ranges") or [])
                if result.get("join"):
                    self.said(fmt("{count} segments joined into {path} and verified", count=count, path=str(result.get("output") or params["output"])))
                else:
                    self.said(fmt("{count} segments written and verified", count=count))
                self.accept()
                return
            report = result.get("report") or {}
            path = str(result.get("output") or params["output"])
            self.said(fmt("Smart Cut written and verified: {path} ({frames} frames)", path=path, frames=int(report.get("samples") or 0)))
            self.accept()

        def failed(message: str) -> None:
            self.summary.setText("")
            self._fail(message)

        self.engine.execute("segments.export" if segments else "export.start", params, on_reply=lambda r, _b: done(r), on_error=failed)
