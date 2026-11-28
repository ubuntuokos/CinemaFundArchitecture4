"""DCP Validation window: check a DCP package folder with a validation profile and read the
findings (docs/development/dcp-validation.md, plan DCP-VALIDATION-2026-10-27 §DCP-F19).

Everything is the engine's: the check runs there (``dcp.validate``, followed in ``jobs.list``), the
report is read back (``dcp.validate.report``) and written out by the engine
(``dcp.validate.save``, never into the package). The window never reads the package and never
talks to Ruby.
"""

from __future__ import annotations

import os
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QTimer
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QAbstractItemView,
    QComboBox,
    QDialog,
    QFileDialog,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QPushButton,
    QTableWidget,
    QTableWidgetItem,
    QTabWidget,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, fmt, tr

JOB_POLL_MS = 400
VERDICTS = {
    "PASS": N_("Passed"),
    "FAIL": N_("Failed"),
    "INCOMPLETE": N_("Incomplete"),
    "INTERRUPTED": N_("Interrupted"),
}
STATUSES = {
    "PASS": N_("Pass"),
    "NOT_APPLICABLE": N_("Not applicable"),
    "WARNING": N_("Warning"),
    "SKIPPED": N_("Skipped"),
    "UNCHECKED": N_("Not checked"),
    "INTERRUPTED": N_("Interrupted"),
    "FAIL": N_("Fail"),
}
COLORS = {
    "PASS": "#3a9a50",
    "FAIL": "#c83c3c",
    "WARNING": "#c8961e",
    "INCOMPLETE": "#7a7a7a",
    "INTERRUPTED": "#7a7a7a",
}
FILE_STATES = {
    "file": N_("OK"),
    "missing": N_("Missing"),
    "noPath": N_("Missing"),
    "notAFile": N_("Not a file"),
    "unsafe": N_("Unsafe path"),
}
# worst first: the findings table lists what needs attention before what passed
STATUS_ORDER = ("FAIL", "WARNING", "UNCHECKED", "INTERRUPTED", "SKIPPED", "NOT_APPLICABLE", "PASS")


def location_text(loc: Any) -> str:
    """``file › element (line:column)`` for a finding's location; "" when it has none."""
    if not isinstance(loc, dict):
        return ""
    text = str(loc.get("file") or "")
    if loc.get("element"):
        text = f"{text} › {loc['element']}" if text else str(loc["element"])
    if loc.get("line") is not None:
        text += f" ({loc['line']}:{loc.get('column', 0)})"
    if loc.get("asset") and not text:
        text = str(loc["asset"])
    return text


def sorted_findings(findings: Any) -> list[dict[str, Any]]:
    """The findings, worst status first, keeping the engine's order within a status."""
    items = [f for f in findings or [] if isinstance(f, dict)]
    rank = {s: i for i, s in enumerate(STATUS_ORDER)}
    return sorted(items, key=lambda f: rank.get(str(f.get("status")), len(rank)))


def _tree(name: str, headers: list[str], parent: QWidget) -> QTreeWidget:
    tree = QTreeWidget(parent)
    tree.setObjectName(name)
    tree.setColumnCount(len(headers))
    tree.setHeaderLabels(headers)
    tree.setUniformRowHeights(True)
    return tree


def _node(parent: QTreeWidget | QTreeWidgetItem, *cells: Any) -> QTreeWidgetItem:
    item = QTreeWidgetItem(parent, ["" if c is None else str(c) for c in cells])
    return item


class DcpValidationDialog(QDialog):
    """Non-modal: the editor stays usable while a package is checked."""

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.dcpValidation")
        self.setWindowTitle(tr("DCP Validation"))
        self.engine = engine
        self.root = ""
        self.report: dict[str, Any] = {}
        self.markdown = ""
        self._job: int | None = None
        # Bumped whenever a new check starts or the folder changes: replies to older requests are
        # dropped, so a slow answer never shows another package's report.
        self._generation = 0

        self.folder = QLineEdit(self)
        self.folder.setObjectName("dcpval.folder")
        self.folder.setPlaceholderText(tr("DCP folder (the one holding ASSETMAP)"))
        self.folder.returnPressed.connect(self.run_check)
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName("dcpval.browse")
        browse.clicked.connect(self._browse)
        self.profile = QComboBox(self)
        self.profile.setObjectName("dcpval.profile")
        self.run_button = QPushButton(tr("Validate"), self)
        self.run_button.setObjectName("dcpval.run")
        self.run_button.clicked.connect(self.run_check)
        self.cancel_button = QPushButton(tr("Cancel"), self)
        self.cancel_button.setObjectName("dcpval.cancel")
        self.cancel_button.setEnabled(False)
        self.cancel_button.clicked.connect(self.cancel_check)

        self.verdict = QLabel(self)
        self.verdict.setObjectName("dcpval.verdict")
        self.summary = QLabel(self)
        self.summary.setObjectName("dcpval.summary")
        self.summary.setWordWrap(True)

        self.findings = QTableWidget(0, 6, self)
        self.findings.setObjectName("dcpval.findings")
        self.findings.setHorizontalHeaderLabels(
            [tr("Status"), tr("Rule"), tr("Location"), tr("Message"), tr("Evidence"), tr("How to fix")]
        )
        self.findings.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.findings.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.findings.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.Interactive)
        self.findings.horizontalHeader().setStretchLastSection(True)
        self.findings.setWordWrap(True)

        self.rules = QTableWidget(0, 3, self)
        self.rules.setObjectName("dcpval.rules")
        self.rules.setHorizontalHeaderLabels([tr("Rule"), tr("Required"), tr("Result")])
        self.rules.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.rules.horizontalHeader().setStretchLastSection(True)

        # The structure views (plan DCP-F19) read ``dcp.package.info``; they never touch the package.
        self.package = _tree("dcpval.package", [tr("Name"), tr("Type"), tr("Size"), tr("State")], self)
        self.composition = _tree(
            "dcpval.composition", [tr("Name"), tr("Type"), tr("Duration"), tr("Offset"), tr("Language")], self
        )
        self.security = _tree("dcpval.security", [tr("Name"), tr("Type")], self)
        self._package_items: dict[str, QTreeWidgetItem] = {}
        self.findings.cellDoubleClicked.connect(self._jump_to_finding)

        self.text = QPlainTextEdit(self)
        self.text.setObjectName("dcpval.markdown")
        self.text.setReadOnly(True)

        self.tabs = QTabWidget(self)
        self.tabs.setObjectName("dcpval.tabs")
        self.tabs.addTab(self.findings, tr("Findings"))
        self.tabs.addTab(self.rules, tr("Rules"))
        self.tabs.addTab(self.package, tr("Package"))
        self.tabs.addTab(self.composition, tr("Composition"))
        self.tabs.addTab(self.security, tr("Security"))
        self.tabs.addTab(self.text, tr("Report"))

        self.save_json = QPushButton(tr("Save JSON…"), self)
        self.save_json.setObjectName("dcpval.saveJson")
        self.save_json.clicked.connect(lambda: self.save_report("json"))
        self.save_md = QPushButton(tr("Save Markdown…"), self)
        self.save_md.setObjectName("dcpval.saveMarkdown")
        self.save_md.clicked.connect(lambda: self.save_report("md"))
        self.save_html = QPushButton(tr("Save HTML…"), self)
        self.save_html.setObjectName("dcpval.saveHtml")
        self.save_html.clicked.connect(lambda: self.save_report("html"))
        self.message = QLabel(self)
        self.message.setObjectName("dcpval.message")
        self.message.setWordWrap(True)
        close = QPushButton(tr("Close"), self)
        close.setObjectName("dcpval.close")
        close.clicked.connect(self.hide)
        self._set_saveable(False)

        top = QHBoxLayout()
        top.addWidget(self.folder, 1)
        top.addWidget(browse)
        top.addWidget(self.profile)
        top.addWidget(self.run_button)
        top.addWidget(self.cancel_button)
        bottom = QHBoxLayout()
        bottom.addWidget(self.message, 1)
        bottom.addWidget(self.save_json)
        bottom.addWidget(self.save_md)
        bottom.addWidget(self.save_html)
        bottom.addWidget(close)
        layout = QVBoxLayout(self)
        layout.addLayout(top)
        layout.addWidget(self.verdict)
        layout.addWidget(self.summary)
        layout.addWidget(self.tabs, 1)
        layout.addLayout(bottom)
        self.resize(1000, 700)

        self._poll = QTimer(self, interval=JOB_POLL_MS)
        self._poll.timeout.connect(self._poll_job)
        self._profiles_loaded = False

    # ----- opening -----
    def open_for(self, root: str | None = None) -> None:
        """Show the window; ``root``: the package folder to put in the field."""
        if root:
            self.folder.setText(root)
        self.show()
        self.raise_()
        if not self._profiles_loaded:
            self.load_profiles()

    def load_profiles(self) -> None:
        if not self.engine.running:
            self.message.setText(tr("The engine is not running."))
            return
        self.engine.execute("dcp.validate.profiles", {}, on_reply=self._on_profiles, on_error=self._show_error)

    def _on_profiles(self, result: Any, _b: bytes | None) -> None:
        profiles = result.get("profiles") if isinstance(result, dict) else None
        if not isinstance(profiles, list):
            self._show_error(tr("the engine sent no profiles"))
            return
        self._profiles_loaded = True
        keep = self.profile.currentData()
        self.profile.clear()
        for p in profiles:
            if isinstance(p, dict):
                self.profile.addItem(f"{p.get('title', p.get('id'))} (v{p.get('version')})", p.get("id"))
        index = self.profile.findData(keep) if keep else -1
        self.profile.setCurrentIndex(max(index, 0))

    def _browse(self) -> None:
        path = QFileDialog.getExistingDirectory(self, tr("DCP folder"), self.folder.text() or os.path.expanduser("~"))
        if path:
            self.folder.setText(path)

    def _show_error(self, message: str) -> None:
        self.message.setText(str(message))

    def _set_saveable(self, on: bool) -> None:
        self.save_json.setEnabled(on)
        self.save_md.setEnabled(on)
        self.save_html.setEnabled(on)

    def _current(self, handler: Callable[..., None]) -> Callable[..., None]:
        """``handler``, called only while no newer check has replaced the one it was sent for."""
        generation = self._generation

        def call(*args: Any) -> None:
            if generation == self._generation:
                handler(*args)

        return call

    # ----- checking -----
    def run_check(self) -> None:
        folder = os.path.expanduser(self.folder.text().strip())
        if not folder:
            self.message.setText(tr("Choose the DCP folder."))
            return
        if not self.engine.running:
            self._run_failed(tr("The engine is not running."))
            return
        self._generation += 1
        self._poll.stop()
        self._job = None
        self.root = folder
        self._clear()
        self.run_button.setEnabled(False)
        self.verdict.setText(tr("Checking…"))
        params: dict[str, Any] = {"root": folder}
        profile = self.profile.currentData()
        if profile:
            params["profile"] = profile
        self.engine.execute(
            "dcp.validate", params, on_reply=self._current(self._on_started), on_error=self._current(self._run_failed)
        )

    def _clear(self) -> None:
        self.report = {}
        self.markdown = ""
        self.verdict.setText("")
        self.verdict.setStyleSheet("")
        self.summary.setText("")
        self.findings.setRowCount(0)
        self.rules.setRowCount(0)
        for tree in (self.package, self.composition, self.security):
            tree.clear()
        self._package_items = {}
        self.text.clear()
        self.message.setText("")
        self._set_saveable(False)

    def _on_started(self, result: Any, _b: bytes | None) -> None:
        job = result.get("job") if isinstance(result, dict) else None
        if not isinstance(job, int):
            self._run_failed(tr("the engine did not start the check"))
            return
        self._job = job
        self.cancel_button.setEnabled(True)
        self._poll.start()

    def _run_failed(self, message: str) -> None:
        self._poll.stop()
        self._job = None
        self.run_button.setEnabled(True)
        self.cancel_button.setEnabled(False)
        self.verdict.setText(tr("Not checked"))
        self.verdict.setStyleSheet("")
        self.message.setText(str(message))

    def cancel_check(self) -> None:
        if self._job is None or not self.engine.running:
            return
        self.cancel_button.setEnabled(False)
        self.engine.execute("jobs.cancel", {"job": self._job}, on_reply=lambda _r, _b: None, on_error=self._show_error)

    def _poll_job(self) -> None:
        if self._job is None or not self.engine.running:
            self._poll.stop()
            return
        self.engine.execute("jobs.list", {}, on_reply=self._current(self._on_jobs), on_error=lambda _m: None)

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
        self.cancel_button.setEnabled(False)
        error = (job.get("result") or {}).get("error") if isinstance(job.get("result"), dict) else None
        if error:
            self._run_failed(str(error))
            return
        self.engine.execute(
            "dcp.validate.report",
            {"root": self.root},
            on_reply=self._current(self._on_report),
            on_error=self._current(self._run_failed),
        )

    # ----- showing -----
    def _on_report(self, result: Any, _b: bytes | None) -> None:
        report = result.get("report") if isinstance(result, dict) else None
        if not isinstance(report, dict):
            self._run_failed(tr("the engine sent no report"))
            return
        self.report = report
        self.markdown = str(result.get("markdown") or "")
        verdict = str(report.get("verdict") or "")
        self.verdict.setText(tr(VERDICTS.get(verdict, verdict)))
        self.verdict.setStyleSheet(f"font-weight: bold; color: {COLORS.get(verdict, '#7a7a7a')};")
        profile = report.get("profile") or {}
        counts = report.get("counts") or {}
        parts = [fmt("Profile: {id} v{version}", id=profile.get("id", ""), version=profile.get("version", ""))]
        if report.get("standard"):
            parts.append(fmt("Standard: {standard}", standard=report["standard"]))
        parts.append(fmt("{n} files checked", n=report.get("assets_checked", 0)))
        parts.extend(f"{tr(STATUSES.get(k, k))}: {v}" for k, v in counts.items() if isinstance(v, int) and v)
        self.summary.setText(" · ".join(parts))
        self._show_findings(report.get("findings"))
        self._show_rules(profile, report.get("findings"))
        self.text.setPlainText(self.markdown)
        self._set_saveable(True)
        self.message.setText(self.root)
        self.engine.execute(
            "dcp.package.info",
            {"root": self.root},
            on_reply=self._current(self._on_package),
            on_error=self._current(self._package_failed),
        )

    # ----- structure views -----
    def _package_failed(self, message: str) -> None:
        # the findings already say what is wrong with the package; the views are just empty
        self.message.setText(fmt("Package structure unavailable: {error}", error=message))

    def _on_package(self, result: Any, _b: bytes | None) -> None:
        if not isinstance(result, dict):
            return
        for tree in (self.package, self.composition, self.security):
            tree.clear()
        self._package_items = {}
        self._show_package(result)
        self._show_compositions(result.get("compositions"))
        self._show_security(result.get("security"))

    def _show_package(self, info: dict[str, Any]) -> None:
        amap = info.get("assetMap") or {}
        cpl_files = {c.get("file") for c in info.get("compositions") or [] if isinstance(c, dict)}
        top = _node(self.package, amap.get("file"), tr("Asset map"))
        self._package_items[str(amap.get("file") or "")] = top
        for f in info.get("files") or []:
            if not isinstance(f, dict):
                continue
            path = f.get("path") or f.get("id") or ""
            if f.get("packingList"):
                kind = tr("Packing list")
            elif path in cpl_files:
                kind = tr("Composition")
            else:
                kind = tr("Track file")
            state = str(f.get("state") or "")
            item = _node(top, path, kind, f.get("size"), tr(FILE_STATES.get(state, state)))
            if state != "file":
                item.setForeground(3, QColor(COLORS["FAIL"]))
            if isinstance(path, str) and path:
                self._package_items[path] = item
        top.setExpanded(True)
        self.package.resizeColumnToContents(0)

    def _show_compositions(self, compositions: Any) -> None:
        for c in compositions or []:
            if not isinstance(c, dict):
                continue
            doc = c.get("document") or {}
            node = _node(self.composition, doc.get("content_title") or c.get("file"), tr("Composition"))
            for n, reel in enumerate(doc.get("reels") or [], 1):
                if not isinstance(reel, dict):
                    continue
                rnode = _node(node, fmt("Reel {n}", n=n))
                for a in reel.get("assets") or []:
                    if not isinstance(a, dict):
                        continue
                    anode = _node(
                        rnode, a.get("file") or a.get("id"), a.get("kind"), a.get("duration"), "", a.get("language")
                    )
                    for m in a.get("markers") or []:
                        if isinstance(m, dict):
                            _node(anode, m.get("label"), tr("Marker"), "", m.get("offset"))
            node.setExpanded(True)
        self.composition.expandAll()
        self.composition.resizeColumnToContents(0)

    def _show_security(self, security: Any) -> None:
        sec = security if isinstance(security, dict) else {}
        signed = _node(self.security, tr("Signed documents"))
        for d in sec.get("signedDocuments") or []:
            if isinstance(d, dict):
                kind = tr("Packing list") if d.get("kind") == "packingList" else tr("Composition")
                _node(signed, d.get("document"), kind)
        encrypted = _node(self.security, tr("Encrypted assets"))
        for e in sec.get("encryptedAssets") or []:
            if isinstance(e, dict):
                _node(encrypted, e.get("composition"), fmt("Key ID: {id}", id=e.get("keyId", "")))
        signed.setExpanded(True)
        encrypted.setExpanded(True)
        self.security.resizeColumnToContents(0)

    def _jump_to_finding(self, row: int, _column: int) -> None:
        """Double-click a finding: show the file it names in the package tree."""
        item = self.findings.item(row, 2)
        if item is None:
            return
        path = item.text().split(" › ")[0].split(" (")[0]
        target = self._package_items.get(path)
        if target is None:
            return
        self.tabs.setCurrentWidget(self.package)
        self.package.setCurrentItem(target)
        self.package.scrollToItem(target)

    def _show_findings(self, findings: Any) -> None:
        items = sorted_findings(findings)
        self.findings.setRowCount(len(items))
        for row, f in enumerate(items):
            status = str(f.get("status") or "")
            cells = [
                tr(STATUSES.get(status, status)),
                str(f.get("rule") or ""),
                location_text(f.get("location")),
                str(f.get("message") or ""),
                str(f.get("evidence") or ""),
                str(f.get("remedy") or ""),
            ]
            for col, text in enumerate(cells):
                item = QTableWidgetItem(text)
                if col == 0 and status in COLORS:
                    item.setForeground(QColor(COLORS[status]))
                self.findings.setItem(row, col, item)
        self.findings.resizeColumnsToContents()

    def _show_rules(self, profile: Any, findings: Any) -> None:
        """The rules of the profile with their worst result, so Unchecked and Skipped rules stay
        visible next to the passed ones (a quick scan must not look like a full check)."""
        worst: dict[str, str] = {}
        rank = {s: i for i, s in enumerate(STATUS_ORDER)}
        for f in findings or []:
            if isinstance(f, dict) and f.get("rule"):
                rule, status = str(f["rule"]), str(f.get("status") or "")
                if rule not in worst or rank.get(status, 99) < rank.get(worst[rule], 99):
                    worst[rule] = status
        self.rules.setRowCount(0)
        rules = profile.get("rules") if isinstance(profile, dict) else None
        names = [r for r in rules or [] if isinstance(r, dict)]
        if names:
            self.rules.setRowCount(len(names))
            for row, r in enumerate(names):
                status = worst.get(str(r.get("rule")), "")
                self.rules.setItem(row, 0, QTableWidgetItem(str(r.get("rule") or "")))
                self.rules.setItem(row, 1, QTableWidgetItem(tr("Yes") if r.get("required") else tr("No")))
                self.rules.setItem(row, 2, QTableWidgetItem(tr(STATUSES.get(status, status))))
        else:
            # the report names the profile, not its rules: list what the findings cover
            self.rules.setRowCount(len(worst))
            for row, (rule, status) in enumerate(sorted(worst.items())):
                self.rules.setItem(row, 0, QTableWidgetItem(rule))
                self.rules.setItem(row, 1, QTableWidgetItem(""))
                self.rules.setItem(row, 2, QTableWidgetItem(tr(STATUSES.get(status, status))))
        self.rules.resizeColumnsToContents()

    # ----- saving -----
    def save_report(self, fmt_id: str, path: str | None = None) -> None:
        """Write the latest report as JSON, Markdown or HTML; the engine refuses a path inside the DCP."""
        if not self.report:
            return
        if path is None:
            filt = {"json": "JSON (*.json)", "md": "Markdown (*.md)", "html": "HTML (*.html)"}.get(fmt_id, "*")
            path, _f = QFileDialog.getSaveFileName(
                self, tr("Save report"), os.path.expanduser(f"~/dcp-report.{fmt_id}"), filt
            )
        if not path:
            return
        self.engine.execute(
            "dcp.validate.save",
            {"root": self.root, "path": path, "format": fmt_id},
            on_reply=lambda r, _b: self.message.setText(fmt("Report saved: {path}", path=(r or {}).get("path", path))),
            on_error=self._show_error,
        )
