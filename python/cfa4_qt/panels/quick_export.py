"""Quick Export (egui's header popup, ``panels::export_mode::quick_export``): File Name & Location,
a preset list (favourites, then the Match Source and Web & Social presets), the estimated file
size and Export, which runs ``export.quick`` in the background.

Automation ids as egui's: ``quickExport.path`` (the file name), ``quickExport.location`` (the
folder; opens the save dialog), ``quickExport.preset.<name>``, ``quickExport.go``,
``quickExport.close``; the header button is ``header.quickExport``.
"""

from __future__ import annotations

import os
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QPoint, Qt, Signal
from PySide6.QtWidgets import (
    QFileDialog,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import fmt, tr

DEFAULT_PRESET = "Match Source – Adaptive High Bitrate"  # the engine names its default in `export.presets.list`
QUICK_CATEGORIES = ("Match Source", "Web & Social")


def quick_presets(presets: list[dict[str, Any]]) -> list[str]:
    """Presets offered by Quick Export: favourites first, then the common delivery presets."""
    names = [str(p["name"]) for p in presets if isinstance(p.get("name"), str) and p.get("favorite") is True]
    for p in presets:
        name = p.get("name")
        if isinstance(name, str) and p.get("builtin") is True and p.get("category") in QUICK_CATEGORIES and name not in names:
            names.append(name)
    return names


def quick_file(path: str, seq_name: str, ext: str, known_exts: set[str], default_dir: str) -> str | None:
    """The Quick Export path for the preset's extension ``ext``, or None when ``path`` already
    fits (egui's ``quick_file``). No folder means the default one and no file name the
    sequence's; another format's extension is swapped and any other kept, so a typed or chosen
    name survives a preset change."""
    folder = path.endswith(("/", "\\"))
    head, tail = os.path.split(path)
    dir_ = (path if folder else head) or None
    name = None if folder or not tail.strip() else tail
    cur = os.path.splitext(name)[1][1:] if name else ""
    if dir_ is not None and name is not None and cur.lower() == ext.lower():
        return None
    if name is None:
        file = f"{seq_name}.{ext}"
    elif cur.lower() == ext.lower():
        file = name
    elif cur and cur.lower() in known_exts:
        file = f"{os.path.splitext(name)[0]}.{ext}"
    else:
        file = f"{name}.{ext}"
    return os.path.join(dir_ if dir_ is not None else default_dir, file)


class QuickExport(QFrame):
    """The popup under the header's Quick Export button."""

    status = Signal(str)

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent, Qt.WindowType.Popup)
        self.setObjectName("quickExport")
        self.setFrameShape(QFrame.Shape.StyledPanel)
        self.engine = engine
        self.sequence: dict[str, Any] = {}
        self.presets: list[dict[str, Any]] = []
        self.preset = ""
        self.path = ""
        self.default_dir = ""
        # the save dialog (tests replace it): (title, filter, start path) -> chosen path or ""
        self.pick_save: Callable[[str, str, str], str] = lambda title, filt, start: QFileDialog.getSaveFileName(self, title, start, filt)[0]
        self.setFixedWidth(340)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(10, 8, 10, 10)
        head = QHBoxLayout()
        title = QLabel(tr("Quick Export"), self)
        f = title.font()
        f.setBold(True)
        f.setPointSizeF(f.pointSizeF() + 1.5)
        title.setFont(f)
        head.addWidget(title, 1)
        close = QToolButton(self)
        close.setObjectName("quickExport.close")
        close.setText(tr("Close"))
        close.setAutoRaise(True)
        close.clicked.connect(self.close)
        head.addWidget(close)
        lay.addLayout(head)
        lay.addWidget(self._dim(tr("File Name & Location")))
        self.file = QLineEdit(self)
        self.file.setObjectName("quickExport.path")
        self.file.setPlaceholderText(tr("File Name"))
        self.file.editingFinished.connect(self._typed)
        lay.addWidget(self.file)
        self.location = QPushButton(self)
        self.location.setObjectName("quickExport.location")
        self.location.setFlat(True)
        self.location.setCursor(Qt.CursorShape.PointingHandCursor)
        self.location.setStyleSheet("QPushButton { text-align: left; text-decoration: underline; border: none; padding: 0 2px; }")
        self.location.clicked.connect(self.browse)
        lay.addWidget(self.location)
        lay.addWidget(self._dim(tr("Preset")))
        self.list = QWidget(self)
        self.list_lay = QVBoxLayout(self.list)
        self.list_lay.setContentsMargins(0, 0, 0, 0)
        self.list_lay.setSpacing(1)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.list)
        scroll.setMaximumHeight(220)
        self.scroll = scroll
        lay.addWidget(scroll)
        self.size = self._dim("")
        self.size.setObjectName("quickExport.size")
        lay.addWidget(self.size)
        foot = QHBoxLayout()
        foot.addStretch(1)
        self.go = QPushButton(tr("Export"), self)
        self.go.setObjectName("quickExport.go")
        self.go.setDefault(True)
        self.go.setMinimumSize(90, 26)
        self.go.clicked.connect(self.export)
        foot.addWidget(self.go)
        lay.addLayout(foot)
        self.rows: dict[str, QPushButton] = {}

    def _dim(self, text: str) -> QLabel:
        label = QLabel(text, self)
        label.setEnabled(False)  # the theme's dim text
        return label

    # ----- opening -----
    def open_at(self, at: QPoint, sequence: dict[str, Any] | None) -> bool:
        """Show the popup with its top-right corner at global ``at``; False (and a status line)
        without an open sequence."""
        if not sequence or sequence.get("id") is None:
            self.status.emit(tr("Quick Export: open a sequence first"))
            return False
        if sequence.get("id") != self.sequence.get("id"):
            self.path = ""  # another sequence: its own name
        self.sequence = sequence
        self.move(at - QPoint(self.width(), 0))
        self.show()
        self.file.setFocus()
        if self.engine is not None:
            self.engine.execute("export.presets.list", {}, on_reply=self._on_presets, on_error=lambda m: self.status.emit(str(m)))
        return True

    def _on_presets(self, result: Any, _b: Any = None) -> None:
        if not isinstance(result, dict):
            return
        self.presets = [p for p in result.get("presets") or [] if isinstance(p, dict)]
        if not self.preset:
            q = result.get("quickPreset")
            self.preset = q if isinstance(q, str) and q else str(result.get("default") or DEFAULT_PRESET)
        while self.list_lay.count():
            w = self.list_lay.takeAt(0).widget()
            if w is not None:
                w.deleteLater()
        self.rows = {}
        t = theme.current()
        # egui's list rows: the chosen one on the accent, the hovered one lighter
        row_style = (f"QPushButton {{ text-align: left; padding: 2px 6px; border: none; border-radius: 3px; min-height: 18px; background: transparent; }}"
                     f"QPushButton:hover {{ background: {t.c('hover').name()}; }}"
                     f"QPushButton:checked {{ background: {t.c('accent').name()}; color: white; }}")
        names = quick_presets(self.presets)
        self.scroll.setMinimumHeight(min(220, 22 * len(names) + 2))
        for name in names:
            b = QPushButton(tr(name), self.list)
            b.setObjectName(f"quickExport.preset.{name}")
            b.setCheckable(True)
            b.setFlat(True)
            b.setStyleSheet(row_style)
            b.clicked.connect(lambda _c=False, n=name: self.choose(n))
            self.list_lay.addWidget(b)
            self.rows[name] = b
        self.choose(self.preset)

    def _preset_info(self, name: str) -> dict[str, Any]:
        return next((p for p in self.presets if p.get("name") == name), {})

    def choose(self, name: str) -> None:
        """Pick preset ``name``: the file takes its extension and the size estimate follows."""
        self.preset = name
        for n, b in self.rows.items():
            b.setChecked(n.lower() == name.lower())
        if self.engine is None:
            return
        self.engine.execute("export.resolve", {"preset": name, "sequence": self.sequence.get("id")}, on_reply=self._on_resolved,
                            on_error=lambda _m: self.size.setText(fmt("Estimated file size: {size}", size="–")))

    def _on_resolved(self, result: Any, _b: Any = None) -> None:
        if not isinstance(result, dict):
            return
        default = (result.get("settings") or {}).get("path") if isinstance(result.get("settings"), dict) else None
        if isinstance(default, str) and default:
            self.default_dir = os.path.dirname(default)
        summary = result.get("summary") if isinstance(result.get("summary"), dict) else {}
        size = summary.get("estimated_size")
        self.size.setText(fmt("Estimated file size: {size}", size=size if isinstance(size, str) and size else "–"))
        self._fit_path()

    def _ext(self) -> str:
        ext = self._preset_info(self.preset).get("extension")
        return ext if isinstance(ext, str) and ext else "mp4"

    def _fit_path(self) -> None:
        known = {str(p.get("extension")).lower() for p in self.presets if isinstance(p.get("extension"), str)}
        name = str(self.sequence.get("name") or "").strip() or "Sequence"
        fitted = quick_file(self.path, name, self._ext(), known, self.default_dir)
        if fitted is not None:
            self.path = fitted
        self._show_path()

    def _show_path(self) -> None:
        head, tail = os.path.split(self.path)
        if not self.file.hasFocus() or self.file.text() != tail:
            self.file.setText(tail)
        self.location.setText(head)
        self.location.setToolTip(head)

    # ----- editing -----
    def _typed(self) -> None:
        v = self.file.text().strip()
        if not v:
            self._show_path()
            return
        # a file name goes in the shown folder; a typed or pasted path replaces both
        self.path = v if ("/" in v or "\\" in v) else os.path.join(os.path.dirname(self.path), v)
        self._fit_path()

    def browse(self) -> None:
        info = self._preset_info(self.preset)
        ext = self._ext()
        filt = f"{info.get('formatLabel') or ext} (*.{ext})"
        path = self.pick_save(tr("Quick Export"), filt, self.path)
        if path:
            self.path = path
            self._fit_path()
        if not self.isVisible():
            self.show()

    def export(self) -> None:
        self._typed()
        params = {"preset": self.preset, "path": self.path, "sequence": self.sequence.get("id")}
        if self.engine is not None:
            def done(result: Any, _b: Any = None) -> None:
                path = result.get("path") if isinstance(result, dict) else None
                self.status.emit(fmt("Quick Export: {path}", path=str(path or self.path)))

            self.engine.execute("export.quick", params, on_reply=done, on_error=lambda m: self.status.emit(str(m)))
        self.close()
