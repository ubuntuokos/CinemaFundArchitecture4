"""Refresh Credits window: what changed in the spreadsheet since the credits were placed, row by
row, with a tick for each change, and the layers edited by hand.

The engine compares (``graphics.credits.diff``) and replaces the pages (``graphics.credits.refresh``,
one undo step). Nothing changes until the user applies; unticked changes stay as they were, and
hand edits are kept unless the user says otherwise.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QDialog,
    QFileDialog,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QPushButton,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, tr

KINDS = {"added": N_("Added"), "removed": N_("Removed"), "changed": N_("Changed")}
MAX_CELL_TEXT = 200


def row_text(rows: Any) -> str:
    """Rows of cells as readable text: cells joined by " | ", rows by line breaks."""
    if not isinstance(rows, list):
        return ""
    lines = []
    for r in rows:
        cells = [str(c) for c in r] if isinstance(r, list) else [str(r)]
        while cells and not cells[-1].strip():
            cells.pop()
        line = " | ".join(c.replace("\n", " ⏎ ") for c in cells)
        lines.append(line if line.strip() else tr("(empty row)"))
    text = "\n".join(lines)
    return text if len(text) <= MAX_CELL_TEXT else text[: MAX_CELL_TEXT - 1] + "…"


def rows_label(pair: Any) -> str:
    """1-based [first, last] rows as "12" or "12–14"; an empty range is "–"."""
    try:
        a, b = int(pair[0]), int(pair[1])
    except (TypeError, ValueError, IndexError):
        return ""
    if b < a:
        return "–"
    return str(a) if a == b else f"{a}–{b}"


class CreditsRefreshDialog(QDialog):
    refreshed = Signal(object)  # the engine's reply

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.creditsRefresh")
        self.setWindowTitle(tr("Refresh Credits from Spreadsheet"))
        self.setModal(False)
        self.engine = engine
        self.clip: int | None = None
        self.source: str | None = None
        self.result: dict[str, Any] | None = None

        self.heading = QLabel(self)
        self.heading.setObjectName("creditsRefresh.heading")
        self.heading.setWordWrap(True)
        self.changes = QTableWidget(0, 4, self)
        self.changes.setObjectName("creditsRefresh.changes")
        self.changes.setHorizontalHeaderLabels([tr("Apply"), tr("Change"), tr("Rows"), tr("Before → After")])
        self.changes.verticalHeader().setVisible(False)
        self.changes.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.changes.setSelectionMode(QAbstractItemView.SelectionMode.NoSelection)
        self.changes.setWordWrap(True)
        h = self.changes.horizontalHeader()
        for col in (0, 1, 2):
            h.setSectionResizeMode(col, QHeaderView.ResizeMode.ResizeToContents)
        h.setSectionResizeMode(3, QHeaderView.ResizeMode.Stretch)
        all_row = QHBoxLayout()
        tick_all = QPushButton(tr("Select All"), self)
        tick_all.setObjectName("creditsRefresh.all")
        tick_all.clicked.connect(lambda: self._tick_all(True))
        tick_none = QPushButton(tr("Select None"), self)
        tick_none.setObjectName("creditsRefresh.none")
        tick_none.clicked.connect(lambda: self._tick_all(False))
        all_row.addWidget(tick_all)
        all_row.addWidget(tick_none)
        all_row.addStretch(1)

        self.manual = QLabel(self)
        self.manual.setObjectName("creditsRefresh.manual")
        self.manual.setWordWrap(True)
        self.keep_manual = QCheckBox(tr("Keep my edits on the pages (where their block is unchanged)"), self)
        self.keep_manual.setObjectName("creditsRefresh.keepManual")
        self.keep_manual.setChecked(True)

        self.locate = QPushButton(tr("Locate Spreadsheet…"), self)
        self.locate.setObjectName("creditsRefresh.locate")
        self.locate.clicked.connect(self._locate)
        self.locate.hide()
        self.status = QLabel(self)
        self.status.setObjectName("creditsRefresh.status")
        self.status.setWordWrap(True)
        self.apply_button = QPushButton(tr("Apply"), self)
        self.apply_button.setObjectName("creditsRefresh.apply")
        self.apply_button.setDefault(True)
        self.apply_button.clicked.connect(self.apply)
        close = QPushButton(tr("Close"), self)
        close.setObjectName("creditsRefresh.close")
        close.clicked.connect(self.close)
        buttons = QHBoxLayout()
        buttons.addWidget(self.status, 1)
        buttons.addWidget(self.locate)
        buttons.addWidget(self.apply_button)
        buttons.addWidget(close)

        layout = QVBoxLayout(self)
        layout.addWidget(self.heading)
        layout.addWidget(self.changes, 1)
        layout.addLayout(all_row)
        layout.addWidget(self.manual)
        layout.addWidget(self.keep_manual)
        layout.addLayout(buttons)
        self.resize(860, 520)

    # ----- engine -----
    def _params(self) -> dict[str, Any]:
        p: dict[str, Any] = {}
        if self.clip is not None:
            p["clip"] = self.clip
        if self.source:
            p["source"] = self.source
        return p

    def open_for(self, clip: int | None = None) -> None:
        self.clip = clip
        self.source = None
        self.show()
        self.raise_()
        self.activateWindow()
        self.reload()

    def reload(self) -> None:
        self.status.setText(tr("Comparing…"))
        self.apply_button.setEnabled(False)
        self.engine.execute("graphics.credits.diff", self._params(), on_reply=lambda r, _b: self._show(r), on_error=self._failed)

    def _failed(self, message: str) -> None:
        self.result = None
        self.changes.setRowCount(0)
        self.manual.setText("")
        self.heading.setText("")
        self.status.setText(message)
        self.apply_button.setEnabled(False)
        # the spreadsheet moved: let the user point to it
        self.locate.setVisible("gone" in message)

    def _show(self, result: Any) -> None:
        if not isinstance(result, dict):
            return
        self.result = result
        self.locate.hide()
        changes = result.get("changes") or []
        manual = result.get("manual") or []
        src = result.get("source") or ""
        self.heading.setText(
            (tr("{n} changes in {file} since the credits were placed. Untick what you do not want.") if changes else tr("The spreadsheet {file} has not changed since the credits were placed."))
            .replace("{n}", str(len(changes)))
            .replace("{file}", str(src))
        )
        self.changes.setRowCount(len(changes))
        for row, c in enumerate(changes):
            box = QCheckBox(self.changes)
            box.setObjectName(f"creditsRefresh.change.{c.get('id')}")
            box.setChecked(True)
            box.setProperty("changeId", c.get("id"))
            holder = QWidget(self.changes)
            hl = QHBoxLayout(holder)
            hl.setContentsMargins(6, 0, 6, 0)
            hl.addWidget(box)
            self.changes.setCellWidget(row, 0, holder)
            kind = c.get("kind")
            self.changes.setItem(row, 1, QTableWidgetItem(tr(KINDS[kind]) if kind in KINDS else str(kind)))
            rows = rows_label(c.get("newRows") if kind != "removed" else c.get("oldRows"))
            self.changes.setItem(row, 2, QTableWidgetItem(rows))
            before, after = row_text(c.get("before")), row_text(c.get("after"))
            text = after if kind == "added" else before if kind == "removed" else f"{before}\n→ {after}"
            item = QTableWidgetItem(text)
            item.setToolTip(text)
            self.changes.setItem(row, 3, item)
        self.changes.resizeRowsToContents()
        if manual:
            lost = [m for m in manual if not m.get("survivesAll")]
            text = tr("{n} layers were edited by hand on the pages.").replace("{n}", str(len(manual)))
            if lost:
                names = ", ".join(str(m.get("text") or m.get("layer") or "") for m in lost[:5])
                text += " " + tr("Accepting every change would replace these, because their block changed: {names}").replace("{names}", names)
            self.manual.setText(text)
            self.keep_manual.show()
        else:
            self.manual.setText("")
            self.keep_manual.hide()
        self.status.setText("")
        self.apply_button.setEnabled(True)

    def accepted_ids(self) -> list[int]:
        ids = []
        for row in range(self.changes.rowCount()):
            holder = self.changes.cellWidget(row, 0)
            box = holder.findChild(QCheckBox) if holder is not None else None
            if box is not None and box.isChecked():
                cid = box.property("changeId")
                if isinstance(cid, int):
                    ids.append(cid)
        return ids

    def _tick_all(self, on: bool) -> None:
        for row in range(self.changes.rowCount()):
            holder = self.changes.cellWidget(row, 0)
            box = holder.findChild(QCheckBox) if holder is not None else None
            if box is not None:
                box.setChecked(on)

    def _locate(self) -> None:
        path, _ = QFileDialog.getOpenFileName(self, tr("Locate Spreadsheet…"), "", tr("Spreadsheets (*.ods *.xlsx *.csv *.tsv *.txt)"))
        if path:
            self.source = path
            self.reload()

    def apply(self) -> None:
        """Replace the pages with the accepted changes (one undo step)."""
        if self.result is None:
            return
        params = {**self._params(), "accept": self.accepted_ids(), "keepManual": self.keep_manual.isChecked()}
        self.apply_button.setEnabled(False)
        self.status.setText(tr("Refreshing…"))

        def done(result: Any, _b: bytes | None) -> None:
            kept = result.get("keptEdits", 0) if isinstance(result, dict) else 0
            n = result.get("accepted", 0) if isinstance(result, dict) else 0
            self.status.setText(tr("Refreshed: {n} changes applied, {kept} hand edits kept.").replace("{n}", str(n)).replace("{kept}", str(kept)))
            self.refreshed.emit(result)
            # the new pages are selected by the engine: compare again so the list shows what is left
            first = ((result or {}).get("clips") or [{}])[0] if isinstance(result, dict) else {}
            self.clip = first.get("clip") if isinstance(first, dict) else None
            self.reload()

        def failed(message: str) -> None:
            self.apply_button.setEnabled(True)
            self.status.setText(message)

        self.engine.execute("graphics.credits.refresh", params, on_reply=done, on_error=failed)
