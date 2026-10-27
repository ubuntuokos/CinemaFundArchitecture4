"""Media QC window: the automatic results of an export's quality check, its contact sheet, and
the guided human review (docs/media-qc.md).

Everything is the engine's: the check runs there (``qc.run``, followed in ``jobs.list``), the
report is read back verified (``qc.report.get``) and the review is stored into the report by the
engine (``qc.review.save``), which recomputes the verdict. The window never measures media and
never changes the project.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from PySide6.QtCore import Qt, QTimer, Signal
from PySide6.QtGui import QColor, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QComboBox,
    QDialog,
    QFormLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QListWidget,
    QPushButton,
    QScrollArea,
    QTableWidget,
    QTableWidgetItem,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, fmt, tr
from ..timecode import TICKS_PER_SECOND

JOB_POLL_MS = 400
VERDICTS = {"PASS": N_("Passed"), "WARN": N_("Passed with warnings"), "FAIL": N_("Failed"), "INCOMPLETE": N_("Incomplete")}
STATUSES = {"PASS": N_("Pass"), "WARN": N_("Warning"), "FAIL": N_("Fail"), "NOT_RUN": N_("Not run")}
ANSWERS = (("", N_("Not answered")), ("yes", N_("Yes")), ("no", N_("No")), ("unsure", N_("Unsure")))
SEVERITIES = (("info", N_("Info")), ("minor", N_("Minor")), ("major", N_("Major")), ("critical", N_("Critical")))
# the review questions the engine asks (crates/qc/src/report.rs QUESTIONS), for the catalog
QUESTION_TEXTS = (
    N_("Is there a visible picture defect?"),
    N_("Are colour and contrast acceptable?"),
    N_("Is there audible distortion?"),
    N_("Is there unwanted noise?"),
)
COLORS = {"PASS": "#3a9a50", "WARN": "#c8961e", "FAIL": "#c83c3c", "INCOMPLETE": "#7a7a7a", "NOT_RUN": "#7a7a7a"}


def value_text(check: dict[str, Any]) -> str:
    """The measured value with its unit, or "" for a findings check."""
    v = check.get("value")
    if not isinstance(v, (int, float)) or isinstance(v, bool):
        return ""
    unit = str(check.get("unit") or "")
    return f"{v:.2f} {unit}".strip()


def bounds_text(check: dict[str, Any]) -> str:
    """``pass`` / ``accept`` bounds as text: ``-15…-13 (accept -16…-12)``."""

    def one(b: Any) -> str:
        if not isinstance(b, dict):
            return ""
        lo, hi = b.get("min"), b.get("max")
        if lo is not None and hi is not None:
            return f"{lo:g}…{hi:g}"
        if lo is not None:
            return f"≥ {lo:g}"
        if hi is not None:
            return f"≤ {hi:g}"
        return ""

    p, a = one(check.get("pass")), one(check.get("accept"))
    return f"{p} ({tr('accept')} {a})" if p and a else p


def position_timecode(pos: Any) -> str:
    return str(pos.get("timecode") or "") if isinstance(pos, dict) else ""


def frame_rate_of(report: dict[str, Any]) -> tuple[int, int] | None:
    """The video frame rate the report's ``probe.video`` detail names (``"25/1"``)."""
    for c in report.get("checks") or []:
        if isinstance(c, dict) and c.get("id") == "probe.video":
            text = str((c.get("detail") or {}).get("frameRate") or "")
            num, _, den = text.partition("/")
            if num.isdecimal() and den.isdecimal() and int(num) > 0 and int(den) > 0:
                return int(num), int(den)
    return None


def parse_position(text: str, rate: tuple[int, int] | None) -> dict[str, Any] | None:
    """A typed ``HH:MM:SS:FF`` time code as a report position (frame and tick when the rate is
    known). ``None`` for an empty or unreadable entry."""
    parts = text.strip().replace(";", ":").split(":")
    if len(parts) != 4 or not all(p.isascii() and p.isdecimal() for p in parts):
        return None
    h, m, s, f = (int(p) for p in parts)
    pos: dict[str, Any] = {"timecode": text.strip()}
    if rate:
        fps = max(1, round(rate[0] / rate[1]))
        frame = (h * 3600 + m * 60 + s) * fps + f
        pos["frame"] = frame
        pos["tick"] = frame * TICKS_PER_SECOND * rate[1] // rate[0]
    return pos


class QcDialog(QDialog):
    """Non-modal: the editor stays usable while a check runs."""

    review_saved = Signal(str, str)  # media path, verdict

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.qc")
        self.setWindowTitle(tr("Quality Check"))
        self.engine = engine
        self.media = ""
        self.queue_item: int | None = None
        self.report: dict[str, Any] = {}
        self.events: list[dict[str, Any]] = []
        self._job: int | None = None

        self.file_label = QLabel(self)
        self.file_label.setObjectName("qc.file")
        self.file_label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.verdict = QLabel(self)
        self.verdict.setObjectName("qc.verdict")
        self.profile_label = QLabel(self)
        self.profile_label.setObjectName("qc.profile")
        self.reasons = QListWidget(self)
        self.reasons.setObjectName("qc.reasons")
        self.reasons.setMaximumHeight(110)
        self.run_button = QPushButton(tr("Check Now"), self)
        self.run_button.setObjectName("qc.run")
        self.run_button.clicked.connect(self.run_check)

        # automatic results
        self.checks = QTableWidget(0, 5, self)
        self.checks.setObjectName("qc.checks")
        self.checks.setHorizontalHeaderLabels([tr("Check"), tr("Status"), tr("Value"), tr("Bounds"), tr("Note")])
        self.events_table = QTableWidget(0, 3, self)
        self.events_table.setObjectName("qc.events")
        self.events_table.setHorizontalHeaderLabels([tr("Check"), tr("Time code"), tr("Message")])
        for t in (self.checks, self.events_table):
            t.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
            t.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
            t.verticalHeader().setVisible(False)
            t.horizontalHeader().setSectionResizeMode(t.columnCount() - 1, QHeaderView.ResizeMode.Stretch)
        to_finding = QPushButton(tr("Add as Finding"), self)
        to_finding.setObjectName("qc.eventToFinding")
        to_finding.clicked.connect(self.event_to_finding)
        auto = QWidget(self)
        av = QVBoxLayout(auto)
        av.addWidget(self.checks, 3)
        av.addWidget(QLabel(tr("Events"), auto))
        av.addWidget(self.events_table, 2)
        av.addWidget(to_finding, 0, Qt.AlignmentFlag.AlignLeft)

        # contact sheet
        self.sheet = QLabel(self)
        self.sheet.setObjectName("qc.contactSheet")
        self.sheet.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.sheet_frames = QLabel(self)
        self.sheet_frames.setObjectName("qc.contactSheetFrames")
        self.sheet_frames.setWordWrap(True)
        scroll = QScrollArea(self)
        scroll.setWidget(self.sheet)
        scroll.setWidgetResizable(True)
        sheet_page = QWidget(self)
        sv = QVBoxLayout(sheet_page)
        sv.addWidget(scroll, 1)
        sv.addWidget(self.sheet_frames)

        # human review
        review = QWidget(self)
        self.questions = QFormLayout()
        self.answers: dict[str, tuple[QComboBox, QLineEdit]] = {}
        self.findings = QTableWidget(0, 4, self)
        self.findings.setObjectName("qc.findings")
        self.findings.setHorizontalHeaderLabels([tr("Description"), tr("Severity"), tr("Time code"), tr("Note")])
        self.findings.verticalHeader().setVisible(False)
        self.findings.horizontalHeader().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        add = QPushButton(tr("Add Finding"), self)
        add.setObjectName("qc.addFinding")
        add.clicked.connect(lambda: self.add_finding())
        remove = QPushButton(tr("Remove Finding"), self)
        remove.setObjectName("qc.removeFinding")
        remove.clicked.connect(self.remove_finding)
        self.reviewer = QLineEdit(self)
        self.reviewer.setObjectName("qc.reviewer")
        self.review_done = QCheckBox(tr("Review complete"), self)
        self.review_done.setObjectName("qc.reviewDone")
        self.save_button = QPushButton(tr("Save Review"), self)
        self.save_button.setObjectName("qc.save")
        self.save_button.clicked.connect(self.save_review)
        rv = QVBoxLayout(review)
        rv.addLayout(self.questions)
        rv.addWidget(QLabel(tr("Findings"), review))
        rv.addWidget(self.findings, 1)
        fb = QHBoxLayout()
        fb.addWidget(add)
        fb.addWidget(remove)
        fb.addStretch(1)
        rv.addLayout(fb)
        form = QFormLayout()
        form.addRow(tr("Reviewer"), self.reviewer)
        rv.addLayout(form)
        sb = QHBoxLayout()
        sb.addWidget(self.review_done)
        sb.addStretch(1)
        sb.addWidget(self.save_button)
        rv.addLayout(sb)

        self.tabs = QTabWidget(self)
        self.tabs.setObjectName("qc.tabs")
        self.tabs.addTab(auto, tr("Automatic Checks"))
        self.tabs.addTab(sheet_page, tr("Contact Sheet"))
        self.tabs.addTab(review, tr("Human Review"))

        self.message = QLabel(self)
        self.message.setObjectName("qc.message")
        self.message.setWordWrap(True)
        close = QPushButton(tr("Close"), self)
        close.setObjectName("qc.close")
        close.clicked.connect(self.hide)

        head = QHBoxLayout()
        head.addWidget(self.verdict, 1)
        head.addWidget(self.run_button)
        layout = QVBoxLayout(self)
        layout.addWidget(self.file_label)
        layout.addLayout(head)
        layout.addWidget(self.profile_label)
        layout.addWidget(self.reasons)
        layout.addWidget(self.tabs, 1)
        bottom = QHBoxLayout()
        bottom.addWidget(self.message, 1)
        bottom.addWidget(close)
        layout.addLayout(bottom)
        self.resize(900, 760)

        self._poll = QTimer(self, interval=JOB_POLL_MS)
        self._poll.timeout.connect(self._poll_job)

    # ----- opening -----
    def open_for(self, media: str, queue_item: int | None = None, run: bool = False) -> None:
        """Show the report of ``media`` (an export); ``run``: check it first."""
        self.media = media
        self.queue_item = queue_item
        self.file_label.setText(media)
        self._clear()
        self.show()
        self.raise_()
        if run:
            self.run_check()
        else:
            self.load()

    def _clear(self) -> None:
        self.report = {}
        self.events = []
        self.verdict.setText("")
        self.profile_label.setText("")
        self.reasons.clear()
        self.checks.setRowCount(0)
        self.events_table.setRowCount(0)
        self.findings.setRowCount(0)
        self.sheet.clear()
        self.sheet_frames.setText("")
        self.message.setText("")

    def load(self) -> None:
        if self.media and self.engine.running:
            self.engine.execute("qc.report.get", {"path": self.media}, on_reply=self._on_report, on_error=self._no_report)

    def _no_report(self, message: str) -> None:
        self._clear()
        self.verdict.setText(tr("Not checked"))
        self.message.setText(message)
        self.save_button.setEnabled(False)

    # ----- checking -----
    def run_check(self) -> None:
        if not self.media:
            return
        params: dict[str, Any] = {"queueItem": self.queue_item} if self.queue_item is not None else {"path": self.media}
        self.run_button.setEnabled(False)
        self.verdict.setText(tr("Checking…"))
        self.engine.execute("qc.run", params, on_reply=self._on_started, on_error=self._run_failed)

    def _on_started(self, result: Any, _b: bytes | None) -> None:
        job = result.get("job") if isinstance(result, dict) else None
        if not isinstance(job, int):
            self._run_failed(tr("the engine did not start the check"))
            return
        self._job = job
        self._poll.start()

    def _run_failed(self, message: str) -> None:
        self._poll.stop()
        self._job = None
        self.run_button.setEnabled(True)
        self.verdict.setText(tr("Not checked"))
        self.message.setText(message)

    def _poll_job(self) -> None:
        if self._job is None or not self.engine.running:
            self._poll.stop()
            return
        self.engine.execute("jobs.list", {}, on_reply=self._on_jobs, on_error=lambda _m: None)

    def _on_jobs(self, result: Any, _b: bytes | None) -> None:
        jobs = result if isinstance(result, list) else (result.get("jobs") if isinstance(result, dict) else None)
        job = next((j for j in jobs or [] if isinstance(j, dict) and j.get("id") == self._job), None)
        if job is None or not job.get("finished"):
            if job is not None:
                self.message.setText(str(job.get("status") or ""))
            return
        self._poll.stop()
        self._job = None
        self.run_button.setEnabled(True)
        error = (job.get("result") or {}).get("error") if isinstance(job.get("result"), dict) else None
        if error:
            self._run_failed(str(error))
            return
        self.load()

    # ----- showing -----
    def _on_report(self, result: Any, _b: bytes | None) -> None:
        if not isinstance(result, dict) or not isinstance(result.get("full"), dict):
            self._no_report(tr("the engine sent no report"))
            return
        self.report = result["full"]
        self.save_button.setEnabled(True)
        verdict = str(result.get("verdict") or "")
        self.verdict.setText(tr(VERDICTS.get(verdict, verdict)))
        self.verdict.setStyleSheet(f"font-weight: bold; color: {COLORS.get(verdict, '#7a7a7a')};")
        profile = self.report.get("profile") or {}
        self.profile_label.setText(fmt("Profile: {id} v{version}", id=profile.get("id", ""), version=profile.get("version", "")))
        self.reasons.clear()
        for reason in (result.get("summary") or {}).get("reasons") or []:
            self.reasons.addItem(str(reason))
        self._show_checks()
        self._show_sheet(result.get("contactSheet"))
        self._show_review(result.get("questions") or [])
        self.message.setText(fmt("Report: {path}", path=result.get("report", "")))

    def _show_checks(self) -> None:
        checks = [c for c in self.report.get("checks") or [] if isinstance(c, dict)]
        self.checks.setRowCount(len(checks))
        self.events = []
        for row, c in enumerate(checks):
            status = str(c.get("status") or "")
            note = str(c.get("reason") or "")
            evs = [e for e in c.get("events") or [] if isinstance(e, dict)]
            if not note and evs:
                note = str(evs[0].get("message") or "")
            for col, text in enumerate((str(c.get("id") or ""), tr(STATUSES.get(status, status)), value_text(c), bounds_text(c), note)):
                cell = QTableWidgetItem(text)
                if col == 1:
                    cell.setForeground(Qt.GlobalColor.white)
                    cell.setBackground(QColor(COLORS.get(status, "#7a7a7a")))
                if col == 4:
                    cell.setToolTip(note)
                self.checks.setItem(row, col, cell)
            for e in evs:
                self.events.append({"check": c.get("id"), **e})
        self.events_table.setRowCount(len(self.events))
        for row, e in enumerate(self.events):
            for col, text in enumerate((str(e.get("check") or ""), position_timecode(e.get("at")), str(e.get("message") or ""))):
                self.events_table.setItem(row, col, QTableWidgetItem(text))

    def _show_sheet(self, path: Any) -> None:
        pix = QPixmap(str(path)) if isinstance(path, str) and Path(path).is_file() else QPixmap()
        if pix.isNull():
            self.sheet.setText(tr("No contact sheet"))
        else:
            self.sheet.setPixmap(pix)
        sheet = next((c for c in self.report.get("checks") or [] if isinstance(c, dict) and c.get("id") == "video.contactSheet"), {})
        frames = (sheet.get("detail") or {}).get("frames") or []
        self.sheet_frames.setText("  ".join(str(f.get("timecode") or f.get("frame")) for f in frames if isinstance(f, dict)))

    def _show_review(self, questions: list[Any]) -> None:
        while self.questions.rowCount():
            self.questions.removeRow(0)
        self.answers = {}
        human = self.report.get("human") or {}
        given = {a.get("question"): a for a in human.get("answers") or [] if isinstance(a, dict)}
        for q in questions:
            if not isinstance(q, dict) or not isinstance(q.get("id"), str):
                continue
            qid = q["id"]
            combo = QComboBox(self)
            combo.setObjectName(f"qc.answer.{qid}")
            for value, label in ANSWERS:
                combo.addItem(tr(label), value)
            note = QLineEdit(self)
            note.setObjectName(f"qc.answerNote.{qid}")
            if qid in given:
                combo.setCurrentIndex(max(0, combo.findData(given[qid].get("answer"))))
                note.setText(str(given[qid].get("note") or ""))
            row = QHBoxLayout()
            row.addWidget(combo)
            row.addWidget(note, 1)
            self.questions.addRow(tr(str(q.get("question") or qid)), row)
            self.answers[qid] = (combo, note)
        self.findings.setRowCount(0)
        for f in human.get("findings") or []:
            if isinstance(f, dict):
                self.add_finding(str(f.get("description") or ""), str(f.get("severity") or "minor"), position_timecode(f.get("at")), str(f.get("note") or ""))
        self.reviewer.setText(str(human.get("reviewer") or ""))
        self.review_done.setChecked(human.get("state") == "done")

    # ----- review -----
    def add_finding(self, description: str = "", severity: str = "minor", timecode: str = "", note: str = "") -> int:
        row = self.findings.rowCount()
        self.findings.insertRow(row)
        self.findings.setItem(row, 0, QTableWidgetItem(description))
        combo = QComboBox(self.findings)
        combo.setObjectName(f"qc.findingSeverity.{row}")
        for value, label in SEVERITIES:
            combo.addItem(tr(label), value)
        combo.setCurrentIndex(max(0, combo.findData(severity)))
        self.findings.setCellWidget(row, 1, combo)
        self.findings.setItem(row, 2, QTableWidgetItem(timecode))
        self.findings.setItem(row, 3, QTableWidgetItem(note))
        return row

    def remove_finding(self) -> None:
        rows = sorted({i.row() for i in self.findings.selectedIndexes()}, reverse=True)
        for row in rows:
            self.findings.removeRow(row)

    def event_to_finding(self) -> None:
        """Copy the selected automatic events into the findings, with their time code."""
        rows = sorted({i.row() for i in self.events_table.selectedIndexes()})
        for row in rows:
            if 0 <= row < len(self.events):
                e = self.events[row]
                self.add_finding(f"{e.get('check')}: {e.get('message')}", "minor", position_timecode(e.get("at")))
        if rows:
            self.tabs.setCurrentIndex(2)

    def review(self) -> dict[str, Any]:
        """The review as the engine stores it (``HumanReview`` in crates/qc)."""
        answers = []
        for qid, (combo, note) in self.answers.items():
            if combo.currentData():
                answers.append({"question": qid, "answer": combo.currentData(), "note": note.text().strip()})
        rate = frame_rate_of(self.report)
        findings = []
        def text(row: int, col: int) -> str:
            cell = self.findings.item(row, col)
            return cell.text().strip() if cell is not None else ""

        for row in range(self.findings.rowCount()):
            description = text(row, 0)
            if not description:
                continue
            combo = self.findings.cellWidget(row, 1)
            finding: dict[str, Any] = {
                "description": description,
                "severity": combo.currentData() if isinstance(combo, QComboBox) else "minor",
                "note": text(row, 3),
            }
            pos = parse_position(text(row, 2), rate)
            if pos:
                finding["at"] = pos
            findings.append(finding)
        anything = bool(answers or findings)
        state = "done" if self.review_done.isChecked() else ("inProgress" if anything else "notStarted")
        out: dict[str, Any] = {"state": state, "answers": answers, "findings": findings}
        if self.reviewer.text().strip():
            out["reviewer"] = self.reviewer.text().strip()
        return out

    def save_review(self) -> None:
        if not self.media:
            return
        self.save_button.setEnabled(False)
        self.engine.execute(
            "qc.review.save",
            {"path": self.media, "review": self.review()},
            on_reply=self._on_saved,
            on_error=self._save_failed,
        )

    def _on_saved(self, result: Any, _b: bytes | None) -> None:
        verdict = str(result.get("verdict") or "") if isinstance(result, dict) else ""
        self.review_saved.emit(self.media, verdict)
        self.load()

    def _save_failed(self, message: str) -> None:
        self.save_button.setEnabled(True)
        self.message.setText(message)
