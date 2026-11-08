"""File ▸ Export ▸ Motion Graphics Template… and Replace Fonts as egui's dialogs
(``crates/ui-egui/src/panels/graphics_templates.rs`` ``export_dialog`` / ``replace_dialog``).

Export As Motion Graphics Template: the template's name, category and description, and the
selected graphic's editable properties (each layer's text, font and size for text layers, then its
colour, position and visibility; only the text is on at first), exported with
``graphics.template.export``. Replace Fonts in Projects: the fonts the project uses (missing ones
marked), one family replaced by another with ``file.replaceFonts``. An engine error stays in the
dialog. Both open without blocking; their drafts are egui's ``ExportDraft`` / ``ReplaceDraft``
(``ui.inspect`` → ``ui.gfxTemplates``). Automation ids: ``exportTemplate.name`` / ``.category`` /
``.description`` / ``.control.<n>`` / ``.ok`` / ``.cancel``, ``replaceFonts.from`` / ``.to`` /
``.ok`` / ``.cancel``."""

from __future__ import annotations

import copy
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Signal
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QFormLayout,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import fmt, tr

DEFAULT_FAMILY = "Inter"  # cfa4_text::fonts::DEFAULT_FAMILY, the Replace Fonts "with" default
ERROR_COLOR = "#ff8080"  # egui's error label
MISSING_COLOR = "#ffa060"  # egui's missing font


def export_controls(layers: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """egui's editable properties for a graphic's layers (``graphics.inspect`` → ``layers``):
    text, font and size for a text layer, then colour, position and visibility; only the text is
    on at first."""
    out: list[dict[str, Any]] = []
    for idx, layer in enumerate(layers):
        i = layer.get("layer") if isinstance(layer.get("layer"), int) else idx
        n = str(layer.get("name") or "")
        if layer.get("kind") == "text":
            out.append({"layer": i, "param": "text", "name": n, "on": True})
            out.append({"layer": i, "param": "font", "name": fmt("{n} Font", n=n), "on": False})
            out.append({"layer": i, "param": "size", "name": fmt("{n} Size", n=n), "on": False})
        out.append({"layer": i, "param": "fill_color", "name": fmt("{n} Color", n=n), "on": False})
        out.append({"layer": i, "param": "position", "name": fmt("{n} Position", n=n), "on": False})
        out.append({"layer": i, "param": "enabled", "name": fmt("Show {n}", n=n), "on": False})
    return out


def _error_label(parent: QWidget) -> QLabel:
    label = QLabel(parent)
    label.setWordWrap(True)
    label.setStyleSheet(f"color: {ERROR_COLOR};")
    label.hide()
    return label


class ExportTemplateDialog(QDialog):
    """Export As Motion Graphics Template for the graphic ``inspected`` (``graphics.inspect``)."""

    finished_with = Signal(str)  # the status message after a successful export

    def __init__(self, inspected: dict[str, Any], run: Callable[..., None], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("exportTemplate")
        self.setWindowTitle(tr("Export As Motion Graphics Template"))
        self.setMinimumWidth(420)
        self.run = run
        self.clip = int(inspected.get("clip") or 0)
        form = QFormLayout()
        self.name = QLineEdit(str(inspected.get("name") or ""), self)
        self.name.setObjectName("exportTemplate.name")
        self.category = QLineEdit(tr("My Templates"), self)
        self.category.setObjectName("exportTemplate.category")
        self.description = QLineEdit(self)
        self.description.setObjectName("exportTemplate.description")
        for label, w in ((tr("Name:"), self.name), (tr("Category:"), self.category), (tr("Description:"), self.description)):
            w.setMinimumWidth(260)
            form.addRow(label, w)
        line = QFrame(self)
        line.setFrameShape(QFrame.Shape.HLine)
        self.controls_box = QWidget(self)
        self.controls_layout = QVBoxLayout(self.controls_box)
        self.controls_layout.setContentsMargins(0, 0, 0, 0)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setMaximumHeight(220)
        scroll.setWidget(self.controls_box)
        note = QLabel(tr("Saved as a CFA4 graphics template (.fcgt) in your templates folder."), self)
        note.setWordWrap(True)
        note.setStyleSheet(f"color: {theme.current().text_dim}; font-size: 11px;")
        self.error = _error_label(self)
        self.ok = QPushButton(tr("Export"), self)
        self.ok.setObjectName("exportTemplate.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.export)
        self.cancel = QPushButton(tr("Cancel"), self)
        self.cancel.setObjectName("exportTemplate.cancel")
        self.cancel.setAutoDefault(False)
        self.cancel.clicked.connect(self.reject)
        buttons = QHBoxLayout()
        buttons.addWidget(self.ok)
        buttons.addWidget(self.cancel)
        buttons.addStretch(1)
        lay = QVBoxLayout(self)
        lay.addLayout(form)
        lay.addWidget(line)
        lay.addWidget(QLabel(tr("Editable properties:"), self))
        lay.addWidget(scroll, 1)
        lay.addWidget(note)
        lay.addWidget(self.error)
        lay.addLayout(buttons)
        self.rows: list[tuple[QCheckBox, QLineEdit, dict[str, Any]]] = []
        self._set_controls(export_controls([x for x in inspected.get("layers") or [] if isinstance(x, dict)]))

    def _set_controls(self, controls: list[dict[str, Any]]) -> None:
        while self.controls_layout.count():
            item = self.controls_layout.takeAt(0)
            w = item.widget() if item is not None else None
            if w is not None:
                w.deleteLater()
        self.rows = []
        for n, c in enumerate(controls):
            row = QWidget(self.controls_box)
            h = QHBoxLayout(row)
            h.setContentsMargins(0, 0, 0, 0)
            on = QCheckBox(row)
            on.setObjectName(f"exportTemplate.control.{n}")
            on.setChecked(bool(c.get("on")))
            on.setAccessibleName(str(c.get("name") or ""))
            name = QLineEdit(str(c.get("name") or ""), row)
            name.setObjectName(f"exportTemplate.control.{n}.name")
            name.setMinimumWidth(220)
            param = QLabel(str(c.get("param") or "").replace("_", " "), row)
            param.setStyleSheet(f"color: {theme.current().text_dim};")
            h.addWidget(on)
            h.addWidget(name, 1)
            h.addWidget(param)
            self.controls_layout.addWidget(row)
            self.rows.append((on, name, {"layer": c.get("layer"), "param": c.get("param")}))
        self.controls_layout.addStretch(1)

    def draft(self) -> dict[str, Any]:
        """egui's ``ExportDraft``."""
        return {
            "clip": self.clip,
            "name": self.name.text(),
            "category": self.category.text(),
            "description": self.description.text(),
            "controls": [{**ref, "name": name.text(), "on": on.isChecked()} for on, name, ref in self.rows],
            "error": self.error.text(),
        }

    def set_draft(self, patch: dict[str, Any]) -> None:
        """Merge ``patch`` (egui's ``ui.set {gfxTemplates: {export}}``); ``controls`` replaces the
        list. A bad value raises ValueError or TypeError and changes nothing."""
        self.check_draft(patch)
        controls = patch.get("controls")
        for k, w in (("name", self.name), ("category", self.category), ("description", self.description)):
            if k in patch:
                w.setText(patch[k])
        if controls is not None:
            self._set_controls(copy.deepcopy(controls))
        if "error" in patch:
            self._show_error(patch["error"])

    def check_draft(self, patch: dict[str, Any]) -> None:
        """Raise ValueError or TypeError when ``set_draft`` would refuse ``patch``."""
        known = ("clip", "name", "category", "description", "controls", "error")
        for k, v in patch.items():
            if k not in known:
                raise ValueError(f"export: no field {k!r} (known: {', '.join(known)})")
            if k in ("name", "category", "description", "error") and not isinstance(v, str):
                raise TypeError(f"export.{k} takes a string")
            if k == "clip" and v != self.clip:
                raise ValueError("export.clip is the graphic the dialog was opened for")
        controls = patch.get("controls")
        if controls is not None:
            ok = isinstance(controls, list) and all(
                isinstance(c, dict) and isinstance(c.get("layer"), int) and isinstance(c.get("param"), str)
                and isinstance(c.get("name", ""), str) and isinstance(c.get("on", False), bool) for c in controls
            )
            if not ok:
                raise TypeError("export.controls takes [{layer, param, name, on}]")

    def _show_error(self, text: str) -> None:
        self.error.setText(text)
        self.error.setVisible(bool(text))

    def export(self) -> None:
        d = self.draft()
        controls = [{"layer": c["layer"], "param": c["param"], "name": c["name"]} for c in d["controls"] if c["on"]]
        params = {"clip": d["clip"], "name": d["name"], "category": d["category"], "description": d["description"], "controls": controls}

        def done(r: Any, _b: Any) -> None:
            path = r.get("path") if isinstance(r, dict) else ""
            self.finished_with.emit(fmt("Exported graphics template to {path}", path=str(path or "")))
            self.accept()

        self.run("graphics.template.export", params, done, self._show_error)


class ReplaceFontsDialog(QDialog):
    """Replace Fonts in Projects, from ``graphics.fonts.used`` and the ``fonts.list`` families."""

    finished_with = Signal(str)

    def __init__(self, used: list[dict[str, Any]], families: list[str], run: Callable[..., None], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("replaceFonts")
        self.setWindowTitle(tr("Replace Fonts in Projects"))
        self.setMinimumWidth(440)
        self.run = run
        self.used = QTreeWidget(self)
        self.used.setObjectName("replaceFonts.used")
        self.used.setHeaderHidden(True)
        self.used.setColumnCount(3)
        self.used.setRootIsDecorated(False)
        self.used.setAlternatingRowColors(True)
        for u in used:
            family = str(u.get("family") or "")
            item = QTreeWidgetItem([fmt("{f} (missing)", f=family) if u.get("missing") else family, str(u.get("style") or ""), str(u.get("uses") or 0)])
            if u.get("missing"):
                item.setForeground(0, QColor(MISSING_COLOR))
            self.used.addTopLevelItem(item)
        for col in range(3):
            self.used.resizeColumnToContents(col)
        self.from_combo = QComboBox(self)
        self.from_combo.setObjectName("replaceFonts.from")
        seen: list[str] = []
        for u in used:
            f = str(u.get("family") or "")
            if f and (not seen or seen[-1] != f):  # egui's dedup (of neighbours)
                seen.append(f)
        for f in seen:
            self.from_combo.addItem(f, f)
        self.to_combo = QComboBox(self)
        self.to_combo.setObjectName("replaceFonts.to")
        self.to_combo.setMaxVisibleItems(18)
        for f in families:
            self.to_combo.addItem(f, f)
        self._pick(self.to_combo, DEFAULT_FAMILY)
        self.error = _error_label(self)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Replace:"), self))
        row.addWidget(self.from_combo, 1)
        row.addWidget(QLabel(tr("with:"), self))
        row.addWidget(self.to_combo, 1)
        self.ok = QPushButton(tr("OK"), self)
        self.ok.setObjectName("replaceFonts.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.replace)
        self.cancel = QPushButton(tr("Cancel"), self)
        self.cancel.setObjectName("replaceFonts.cancel")
        self.cancel.setAutoDefault(False)
        self.cancel.clicked.connect(self.reject)
        buttons = QHBoxLayout()
        buttons.addWidget(self.ok)
        buttons.addWidget(self.cancel)
        buttons.addStretch(1)
        line = QFrame(self)
        line.setFrameShape(QFrame.Shape.HLine)
        lay = QVBoxLayout(self)
        lay.addWidget(QLabel(tr("Fonts used in the project:"), self))
        lay.addWidget(self.used, 1)
        lay.addWidget(line)
        lay.addLayout(row)
        lay.addWidget(self.error)
        lay.addLayout(buttons)

    @staticmethod
    def _pick(combo: QComboBox, family: str) -> None:
        """Choose ``family``, adding it when the list lacks it (egui's combo shows any text)."""
        i = combo.findData(family)
        if i < 0 and family:
            combo.addItem(family, family)
            i = combo.count() - 1
        combo.setCurrentIndex(i)

    def draft(self) -> dict[str, Any]:
        """egui's ``ReplaceDraft``."""
        return {"from": str(self.from_combo.currentData() or ""), "to": str(self.to_combo.currentData() or ""), "error": self.error.text()}

    def check_draft(self, patch: dict[str, Any]) -> None:
        """Raise ValueError or TypeError when ``set_draft`` would refuse ``patch``."""
        for k, v in patch.items():
            if k not in ("from", "to", "error"):
                raise ValueError(f"replace: no field {k!r} (known: from, to, error)")
            if not isinstance(v, str):
                raise TypeError(f"replace.{k} takes a string")

    def set_draft(self, patch: dict[str, Any]) -> None:
        """Merge ``patch`` (egui's ``ui.set {gfxTemplates: {replace}}``)."""
        self.check_draft(patch)
        if "from" in patch:
            self._pick(self.from_combo, patch["from"])
        if "to" in patch:
            self._pick(self.to_combo, patch["to"])
        if "error" in patch:
            self.error.setText(patch["error"])
            self.error.setVisible(bool(patch["error"]))

    def replace(self) -> None:
        d = self.draft()

        def done(r: Any, _b: Any) -> None:
            n = r.get("replaced") if isinstance(r, dict) else 0
            self.finished_with.emit(fmt("Replaced {n} font uses", n=n))
            self.accept()

        def failed(message: str) -> None:
            self.error.setText(message)
            self.error.setVisible(True)

        self.run("file.replaceFonts", {"from": d["from"], "to": d["to"]}, done, failed)


__all__ = ["ExportTemplateDialog", "ReplaceFontsDialog", "export_controls"]
