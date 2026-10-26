"""Credits window: a credits sequence from a spreadsheet (CSV, TSV, .xlsx, .ods).

Everything is the engine's (``crates/credits``, ``crates/engine/src/credits.rs``): the window
collects the file, worksheet, picture folder and styling, shows what the engine lays out
(``graphics.credits.preview``: pages, durations, warnings) and a picture of the chosen page at a
moment (``graphics.credits.thumbnail``), and places the credits at the playhead
(``graphics.credits.import``, one undo step). It never lays out credits itself.
"""

from __future__ import annotations

import base64
from pathlib import Path
from typing import Any

from PySide6.QtCore import Qt, QTimer, Signal
from PySide6.QtGui import QImage, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QComboBox,
    QDialog,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QSlider,
    QSplitter,
    QStyle,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND

SHEET_FILTER = N_("Spreadsheets and credits files (*.ods *.xlsx *.csv *.tsv *.txt *.cfa4credits)")
CREDITS_FILE_FILTER = N_("CFA4 credits file (*.cfa4credits)")
STYLING_FILTER = N_("Credits styling (*.json)")
BEHAVIOURS = {"scroll": N_("Roll"), "card": N_("Card")}
PREVIEW_WIDTH = 480
# The engine's checks (crates/credits/src/validate.rs), worded from their code and values.
ISSUE_TEXT = {
    "duplicate-name": N_("“{name}” appears twice in the same block"),
    "name-spelling": N_("“{name}” is written as “{other}” in row {otherRow}"),
    "missing-font": N_("The font “{font}” is not installed; another font is used"),
    "missing-glyphs": N_("The font “{font}” has no “{chars}”; another font draws them"),
    "outside-title-safe": N_("“{text}” is outside the title-safe area"),
    "low-contrast": N_("“{text}” is too dark on black (contrast {ratio}:1; at least 4.5:1 is needed)"),
    "small-text": N_("“{text}” is {px} px high; 24 px or more reads well"),
    "uneven-scroll": N_("The roll moves {speed} px per frame; a whole number does not shimmer"),
    "fast-scroll": N_("A line crosses the frame in {seconds} s; at least 6 s reads well"),
    "short-card": N_("The card holds for only {seconds} s after its fades"),
    "overlap": N_("This page starts {seconds} s before the previous one ends"),
    "upscaled-picture": N_("“{file}” is shown at {percent} % of its pixels; it will look soft"),
    "more-issues": N_("{more} more of the same"),
}


def issue_text(issue: dict[str, Any]) -> str:
    """An engine check in the user's language: its template filled with its values, after its
    place (row or page). Unknown codes fall back to the engine's English message."""
    template = ISSUE_TEXT.get(str(issue.get("code")))
    values = issue.get("values") if isinstance(issue.get("values"), dict) else {}
    if template:
        text = tr(template)
        for k, v in values.items():
            text = text.replace("{" + str(k) + "}", str(v))
    else:
        text = str(issue.get("message") or issue.get("code") or "")
    row, page = issue.get("row"), issue.get("page")
    if isinstance(row, int):
        return tr("Row {row}: {message}").replace("{row}", str(row)).replace("{message}", text)
    if isinstance(page, int):
        return tr("Page {page}: {message}").replace("{page}", str(page + 1)).replace("{message}", text)
    return text
# typing in a path field waits this long before asking the engine again
DEBOUNCE_MS = 400


def format_seconds(ticks: Any) -> str:
    """Engine ticks as m:ss.f (or h:mm:ss.f)."""
    try:
        s = max(0.0, float(ticks) / TICKS_PER_SECOND)
    except (TypeError, ValueError, OverflowError):
        return ""
    whole = int(s)
    tenth = int((s - whole) * 10)
    if whole >= 3600:
        return f"{whole // 3600}:{whole // 60 % 60:02d}:{whole % 60:02d}.{tenth}"
    return f"{whole // 60}:{whole % 60:02d}.{tenth}"


class CreditsDialog(QDialog):
    """Non-modal: the editor stays usable while the sheet is being written."""

    imported = Signal(object)  # number of pages placed (an object: integer signals are 32-bit in PySide6)

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.credits")
        self.setWindowTitle(tr("Credits from a Spreadsheet"))
        self.setModal(False)
        self.engine = engine
        self.preview: dict[str, Any] | None = None
        self._generation = 0  # replies to older requests are dropped
        self._thumb_generation = 0
        self._loading = False

        # ----- source -----
        self.path = QLineEdit(self)
        self.path.setObjectName("credits.path")
        self.path.setPlaceholderText(tr("A .ods, .xlsx or CSV credits sheet"))
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName("credits.browse")
        browse.clicked.connect(self._browse)
        path_row = QHBoxLayout()
        path_row.addWidget(self.path, 1)
        path_row.addWidget(browse)

        self.sheet = QComboBox(self)
        self.sheet.setObjectName("credits.sheet")

        self.pictures = QLineEdit(self)
        self.pictures.setObjectName("credits.pictureDir")
        self.pictures.setPlaceholderText(tr("The spreadsheet's folder"))
        pic_browse = QPushButton(tr("Browse…"), self)
        pic_browse.setObjectName("credits.pictureDir.browse")
        pic_browse.clicked.connect(self._browse_pictures)
        pic_row = QHBoxLayout()
        pic_row.addWidget(self.pictures, 1)
        pic_row.addWidget(pic_browse)

        self.styling = QLineEdit(self)
        self.styling.setObjectName("credits.styling")
        self.styling.setPlaceholderText(tr("Built-in styles"))
        sty_browse = QPushButton(tr("Browse…"), self)
        sty_browse.setObjectName("credits.styling.browse")
        sty_browse.clicked.connect(self._browse_styling)
        sty_row = QHBoxLayout()
        sty_row.addWidget(self.styling, 1)
        sty_row.addWidget(sty_browse)

        form = QFormLayout()
        form.addRow(tr("Spreadsheet"), path_row)
        form.addRow(tr("Sheet"), self.sheet)
        form.addRow(tr("Pictures"), pic_row)
        form.addRow(tr("Styling"), sty_row)

        # ----- pages and warnings -----
        self.pages = QTableWidget(0, 3, self)
        self.pages.setObjectName("credits.pages")
        self.pages.setHorizontalHeaderLabels([tr("Page"), tr("Type"), tr("Duration")])
        self.pages.verticalHeader().setVisible(False)
        self.pages.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.pages.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.pages.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        header = self.pages.horizontalHeader()
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        for col in (1, 2):
            header.setSectionResizeMode(col, QHeaderView.ResizeMode.ResizeToContents)
        self.pages.itemSelectionChanged.connect(self._request_thumbnail)
        self.total = QLabel(self)
        self.total.setObjectName("credits.total")
        self.warnings = QListWidget(self)
        self.warnings.setObjectName("credits.warnings")
        self.warnings.setMaximumHeight(140)
        self.warnings.setWordWrap(True)
        self.check_summary = QLabel(self)
        self.check_summary.setObjectName("credits.checkSummary")

        left = QWidget(self)
        lv = QVBoxLayout(left)
        lv.setContentsMargins(0, 0, 0, 0)
        lv.addWidget(self.pages, 1)
        lv.addWidget(self.total)
        lv.addWidget(self.check_summary)
        lv.addWidget(QLabel(tr("Checks and warnings"), self))
        lv.addWidget(self.warnings)

        # ----- page picture -----
        self.picture = QLabel(self)
        self.picture.setObjectName("credits.picture")
        self.picture.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.picture.setMinimumSize(PREVIEW_WIDTH // 2, PREVIEW_WIDTH * 9 // 32)
        self.picture.setStyleSheet("background: #000; color: #888;")
        self.position = QSlider(Qt.Orientation.Horizontal, self)
        self.position.setObjectName("credits.position")
        self.position.setRange(0, 100)
        self.position.setValue(50)
        self.position.setToolTip(tr("Moment in the page"))
        self.position.valueChanged.connect(self._request_thumbnail)
        right = QWidget(self)
        rv = QVBoxLayout(right)
        rv.setContentsMargins(0, 0, 0, 0)
        rv.addWidget(self.picture, 1)
        rv.addWidget(self.position)

        split = QSplitter(Qt.Orientation.Horizontal, self)
        split.addWidget(left)
        split.addWidget(right)
        split.setStretchFactor(0, 1)
        split.setStretchFactor(1, 1)

        # ----- buttons -----
        self.status = QLabel(self)
        self.status.setObjectName("credits.status")
        self.status.setWordWrap(True)
        self.save_button = QPushButton(tr("Save as Credits File…"), self)
        self.save_button.setObjectName("credits.save")
        self.save_button.setToolTip(tr("Spreadsheet, worksheet, styling and the table in one file, to open on any machine"))
        self.save_button.clicked.connect(self._save_file)
        self.import_button = QPushButton(tr("Place at Playhead"), self)
        self.import_button.setObjectName("credits.import")
        self.import_button.setDefault(True)
        self.import_button.clicked.connect(self.place)
        close = QPushButton(tr("Close"), self)
        close.setObjectName("credits.close")
        close.clicked.connect(self.close)
        buttons = QHBoxLayout()
        buttons.addWidget(self.status, 1)
        buttons.addWidget(self.save_button)
        buttons.addWidget(self.import_button)
        buttons.addWidget(close)

        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(split, 1)
        layout.addLayout(buttons)
        self.resize(980, 600)

        self._debounce = QTimer(self)
        self._debounce.setSingleShot(True)
        self._debounce.setInterval(DEBOUNCE_MS)
        self._debounce.timeout.connect(self.refresh)
        for field in (self.path, self.pictures, self.styling):
            field.textChanged.connect(self._changed)
        self.sheet.currentIndexChanged.connect(self._sheet_changed)
        self._show(None, "")

    # ----- parameters -----
    def params(self) -> dict[str, Any]:
        """The engine parameters the fields stand for."""
        p: dict[str, Any] = {"path": self.path.text().strip()}
        if self.sheet.isEnabled() and self.sheet.currentData() is not None:
            p["sheet"] = self.sheet.currentData()
        if self.pictures.text().strip():
            p["pictureDir"] = self.pictures.text().strip()
        if self.styling.text().strip():
            p["styling"] = self.styling.text().strip()
        return p

    def open_with(self, path: str | None = None) -> None:
        if path:
            self.path.setText(path)
        self.show()
        self.raise_()
        self.activateWindow()
        if self.path.text().strip():
            self.refresh()

    # ----- file pickers -----
    def _start_dir(self) -> str:
        text = self.path.text().strip()
        return str(Path(text).expanduser().parent) if text else str(Path.home())

    def _browse(self) -> None:
        path, _ = QFileDialog.getOpenFileName(self, tr("Credits Spreadsheet"), self._start_dir(), tr(SHEET_FILTER))
        if path:
            self.path.setText(path)

    def _browse_pictures(self) -> None:
        path = QFileDialog.getExistingDirectory(self, tr("Pictures"), self.pictures.text().strip() or self._start_dir())
        if path:
            self.pictures.setText(path)

    def _browse_styling(self) -> None:
        path, _ = QFileDialog.getOpenFileName(self, tr("Styling"), self._start_dir(), tr(STYLING_FILTER))
        if path:
            self.styling.setText(path)

    def _changed(self, _text: str) -> None:
        if not self._loading:
            self._debounce.start()

    def _sheet_changed(self, _index: int) -> None:
        if not self._loading:
            self.refresh()

    # ----- engine round trips -----
    def refresh(self) -> None:
        """Ask the engine to lay out the sheet as it stands."""
        self._debounce.stop()
        if not self.path.text().strip():
            self._show(None, "")
            return
        self._generation += 1
        gen = self._generation
        self.status.setText(tr("Reading…"))
        self.engine.execute(
            "graphics.credits.preview",
            self.params(),
            on_reply=lambda r, _b, g=gen: self._on_preview(g, r),
            on_error=lambda m, g=gen: self._on_preview_error(g, m),
        )

    def _on_preview(self, gen: int, result: Any) -> None:
        if gen != self._generation or not isinstance(result, dict):
            return
        self._show(result, "")

    def _on_preview_error(self, gen: int, message: str) -> None:
        if gen == self._generation:
            self._show(None, message)

    def _show(self, preview: dict[str, Any] | None, error: str) -> None:
        self.preview = preview
        pages = (preview or {}).get("pages") or []
        self._loading = True
        try:
            sheets = (preview or {}).get("sheets") or []
            chosen = (preview or {}).get("sheet") or ""
            if [self.sheet.itemText(i) for i in range(self.sheet.count())] != sheets:
                self.sheet.clear()
                for name in sheets:
                    self.sheet.addItem(name, name)
            if chosen:
                self.sheet.setCurrentIndex(max(0, self.sheet.findData(chosen)))
            self.sheet.setEnabled(len(sheets) > 1)
            if not sheets:
                self.sheet.clear()
                self.sheet.addItem(tr("(one table)"), None)
        finally:
            self._loading = False
        selected = self.pages.currentRow()
        self.pages.setRowCount(len(pages))
        for row, page in enumerate(pages):
            cells = (
                str(page.get("name") or ""),
                tr(BEHAVIOURS.get(page.get("behaviour"), "")) if page.get("behaviour") in BEHAVIOURS else "",
                format_seconds(page.get("duration")),
            )
            for col, text in enumerate(cells):
                item = QTableWidgetItem(text)
                if col == 2:
                    item.setTextAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
                self.pages.setItem(row, col, item)
        if pages:
            self.pages.selectRow(min(max(selected, 0), len(pages) - 1))
        self.total.setText(tr("Total: {time}").replace("{time}", format_seconds((preview or {}).get("duration"))) if pages else "")
        self.warnings.clear()
        style = self.style()
        icons = {
            "error": style.standardIcon(QStyle.StandardPixmap.SP_MessageBoxCritical),
            "warning": style.standardIcon(QStyle.StandardPixmap.SP_MessageBoxWarning),
            "info": style.standardIcon(QStyle.StandardPixmap.SP_MessageBoxInformation),
        }
        for issue in (preview or {}).get("issues") or []:
            if isinstance(issue, dict):
                item = QListWidgetItem(icons.get(str(issue.get("severity")), icons["info"]), issue_text(issue))
                item.setData(Qt.ItemDataRole.UserRole, issue.get("code"))
                self.warnings.addItem(item)
        for w in (preview or {}).get("warnings") or []:
            row = w.get("row") if isinstance(w, dict) else None
            text = str(w.get("message") if isinstance(w, dict) else w)
            item = QListWidgetItem(icons["warning"], tr("Row {row}: {message}").replace("{row}", str(row)).replace("{message}", text) if row else text)
            self.warnings.addItem(item)
        issues = [i for i in (preview or {}).get("issues") or [] if isinstance(i, dict)]
        errors = sum(1 for i in issues if i.get("severity") == "error")
        warns = sum(1 for i in issues if i.get("severity") == "warning") + len((preview or {}).get("warnings") or [])
        self.check_summary.setText(
            tr("Checks: {errors} errors, {warnings} warnings").replace("{errors}", str(errors)).replace("{warnings}", str(warns)) if pages else ""
        )
        self.import_button.setEnabled(bool(pages))
        self.save_button.setEnabled(bool(pages))
        if error:
            self.status.setText(error)
        elif pages:
            self.status.setText("")
        else:
            self.status.setText(tr("Choose a credits spreadsheet."))
        if not pages:
            self.picture.setPixmap(QPixmap())
            self.picture.setText(tr("No preview"))
        self._request_thumbnail()

    def _request_thumbnail(self) -> None:
        if not (self.preview or {}).get("pages"):
            return
        page = max(0, self.pages.currentRow())
        self._thumb_generation += 1
        gen = self._thumb_generation
        p = {**self.params(), "page": page, "at": self.position.value() / 100.0, "width": PREVIEW_WIDTH}
        self.engine.execute(
            "graphics.credits.thumbnail",
            p,
            on_reply=lambda r, _b, g=gen: self._on_thumbnail(g, r),
            on_error=lambda _m: None,
        )

    def _on_thumbnail(self, gen: int, result: Any) -> None:
        if gen != self._thumb_generation or not isinstance(result, dict):
            return
        try:
            png = base64.b64decode(result.get("pngBase64") or "", validate=True)
        except (ValueError, TypeError):
            return
        image = QImage.fromData(png, "PNG")
        if image.isNull():
            return
        self.picture.setText("")
        self.picture.setPixmap(QPixmap.fromImage(image))

    def _save_file(self) -> None:
        start = Path(self.path.text().strip() or str(Path.home() / "credits")).with_suffix(".cfa4credits")
        path, _ = QFileDialog.getSaveFileName(self, tr("Save as Credits File…"), str(start), tr(CREDITS_FILE_FILTER))
        if path:
            self.save_to(path)

    def save_to(self, path: str) -> None:
        """Write the credits as a `.cfa4credits` file (``graphics.credits.save``)."""

        def done(result: Any, _b: bytes | None) -> None:
            written = result.get("path") if isinstance(result, dict) else path
            self.status.setText(tr("Saved: {path}").replace("{path}", str(written)))

        self.engine.execute("graphics.credits.save", {"to": path, "from": self.params()}, on_reply=done, on_error=self.status.setText)

    def place(self) -> None:
        """Place the credits at the playhead (one undo step)."""
        if not (self.preview or {}).get("pages"):
            return
        self.import_button.setEnabled(False)
        self.status.setText(tr("Placing…"))

        def done(result: Any, _b: bytes | None) -> None:
            n = len((result or {}).get("clips") or []) if isinstance(result, dict) else 0
            self.import_button.setEnabled(True)
            self.status.setText(tr("Placed {n} pages on the timeline.").replace("{n}", str(n)))
            self.imported.emit(n)

        def failed(message: str) -> None:
            self.import_button.setEnabled(True)
            self.status.setText(message)

        self.engine.execute("graphics.credits.import", self.params(), on_reply=done, on_error=failed)
