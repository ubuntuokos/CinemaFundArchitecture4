"""The Export window's Preset Manager (egui's ``export_mode::preset_manager``, Preset ▸ More
presets…): search, Favorites only, the presets by category with a favourite star, their format
and a User badge, the selected one's description, Save current settings as…, Import… /
Export… / Delete, Cancel and OK (applies the selected preset; a double click does too).

Automation ids as egui's: ``presetManager.search``, ``presetManager.favoritesOnly``,
``presetManager.item.<name>``, ``presetManager.favorite.<name>``, ``presetManager.saveName``,
``presetManager.save``, ``presetManager.delete``, ``presetManager.import``,
``presetManager.export``, ``presetManager.ok``, ``presetManager.cancel``.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
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
from ..i18n import tr

STAR = QColor(0xF5, 0xC2, 0x42)


def preset_key(name: str) -> str:
    """Preset names compare as the engine's ``preset_key``: letters and digits, any case."""
    return "".join(c for c in name if c.isalnum()).lower()


def matches(p: dict[str, Any], query: str) -> bool:
    q = query.casefold().strip()
    return not q or any(q in str(p.get(k) or "").casefold() or q in tr(str(p.get(k) or "")).casefold() for k in ("name", "description", "category"))


class _Row(QPushButton):
    """A preset row: a click selects it, a double click applies it."""

    double = Signal()

    def mouseDoubleClickEvent(self, e: Any) -> None:
        self.double.emit()
        e.accept()


class PresetManager(QDialog):
    applied = Signal(str)  # the preset OK (or a double click) chose
    status = Signal(str)

    def __init__(self, engine: Any, current: Callable[[], dict[str, Any]], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("presetManager")
        self.setWindowTitle(tr("Preset Manager"))
        self.resize(560, 520)
        self.engine = engine
        self.current = current  # the Export window's settings (`export.*` parameters with `preset`)
        self.presets: list[dict[str, Any]] = []
        self.selected = ""
        # file dialogs (tests replace them): (title, filter, start) -> path or ""
        self.pick_open: Callable[[str, str, str], str] = lambda t, f, s: QFileDialog.getOpenFileName(self, t, s, f)[0]
        self.pick_save: Callable[[str, str, str], str] = lambda t, f, s: QFileDialog.getSaveFileName(self, t, s, f)[0]
        lay = QVBoxLayout(self)
        top = QHBoxLayout()
        self.search = QLineEdit(self)
        self.search.setObjectName("presetManager.search")
        self.search.setPlaceholderText(tr("Search presets"))
        self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(lambda _t: self.rebuild())
        top.addWidget(self.search, 1)
        self.favorites_only = QCheckBox(tr("Favorites only"), self)
        self.favorites_only.setObjectName("presetManager.favoritesOnly")
        self.favorites_only.toggled.connect(lambda _on: self.rebuild())
        top.addWidget(self.favorites_only)
        lay.addLayout(top)
        self.list = QWidget(self)
        self.list_lay = QVBoxLayout(self.list)
        self.list_lay.setContentsMargins(0, 0, 0, 0)
        self.list_lay.setSpacing(1)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.list)
        lay.addWidget(scroll, 1)
        self.description = QLabel(self)
        self.description.setObjectName("presetManager.description")
        self.description.setWordWrap(True)
        self.description.setEnabled(False)  # the theme's dim text
        lay.addWidget(self.description)
        save = QHBoxLayout()
        save.addWidget(QLabel(tr("Save current settings as:"), self))
        self.save_name = QLineEdit(self)
        self.save_name.setObjectName("presetManager.saveName")
        self.save_name.setPlaceholderText(tr("Preset name"))
        save.addWidget(self.save_name, 1)
        self.save = QPushButton(tr("Save"), self)
        self.save.setObjectName("presetManager.save")
        self.save.setEnabled(False)
        self.save.clicked.connect(self.save_current)
        self.save_name.textChanged.connect(lambda t: self.save.setEnabled(bool(t.strip())))
        save.addWidget(self.save)
        lay.addLayout(save)
        foot = QHBoxLayout()
        self.import_ = self._button(foot, "presetManager.import", tr("Import…"), self.import_presets)
        self.export = self._button(foot, "presetManager.export", tr("Export…"), self.export_preset)
        self.delete = self._button(foot, "presetManager.delete", tr("Delete"), self.delete_preset)
        foot.addStretch(1)
        self._button(foot, "presetManager.cancel", tr("Cancel"), self.reject)
        self.ok = self._button(foot, "presetManager.ok", tr("OK"), self.accept_selected)
        lay.addLayout(foot)
        self.rows: dict[str, _Row] = {}
        self.stars: dict[str, QToolButton] = {}
        self._sync_buttons()

    def _button(self, row: QHBoxLayout, name: str, text: str, slot: Callable[[], None]) -> QPushButton:
        b = QPushButton(text, self)
        b.setObjectName(name)
        b.clicked.connect(lambda _c=False: slot())
        row.addWidget(b)
        return b

    # ----- data -----
    def open_on(self, selected: str) -> None:
        self.selected = selected
        self.show()
        self.raise_()
        self.refresh()

    def refresh(self) -> None:
        if self.engine is not None:
            self.engine.execute("export.presets.list", {}, on_reply=self._got, on_error=lambda m: self.status.emit(str(m)))

    def _got(self, result: Any, _b: Any = None) -> None:
        if isinstance(result, dict):
            self.presets = [p for p in result.get("presets") or [] if isinstance(p, dict) and isinstance(p.get("name"), str)]
            self.rebuild()

    def _info(self) -> dict[str, Any] | None:
        return next((p for p in self.presets if preset_key(p["name"]) == preset_key(self.selected)), None)

    def rebuild(self) -> None:
        while self.list_lay.count():
            item = self.list_lay.takeAt(0)
            w = item.widget()
            if w is not None:
                w.deleteLater()
        self.rows, self.stars = {}, {}
        t = theme.current()
        row_style = (f"QPushButton {{ text-align: left; padding: 2px 6px; border: none; border-radius: 3px; min-height: 18px; background: transparent; }}"
                     f"QPushButton:hover {{ background: {t.c('hover').name()}; }}"
                     f"QPushButton:checked {{ background: {t.c('accent').name()}; color: white; }}")
        category = None
        for p in self.presets:
            if not matches(p, self.search.text()) or (self.favorites_only.isChecked() and p.get("favorite") is not True):
                continue
            name = p["name"]
            if p.get("category") != category:
                category = p.get("category")
                head = QLabel(tr(str(category or "")), self.list)
                f = head.font()
                f.setBold(True)
                head.setFont(f)
                head.setEnabled(False)
                self.list_lay.addWidget(head)
            line = QWidget(self.list)
            h = QHBoxLayout(line)
            h.setContentsMargins(0, 0, 0, 0)
            h.setSpacing(4)
            fav = p.get("favorite") is True
            star = QToolButton(line)
            star.setObjectName(f"presetManager.favorite.{name}")
            star.setAutoRaise(True)
            star.setText("★" if fav else "☆")
            star.setStyleSheet(f"color: {STAR.name() if fav else t.c('text_faint').name()};")
            star.setToolTip(tr("Show in the Preset menu"))
            star.clicked.connect(lambda _c=False, n=name: self.toggle_favorite(n))
            h.addWidget(star)
            self.stars[name] = star
            row = _Row(tr(name), line)
            row.setObjectName(f"presetManager.item.{name}")
            row.setCheckable(True)
            row.setFlat(True)
            row.setStyleSheet(row_style)
            row.setToolTip(tr(str(p.get("description") or "")))
            row.setChecked(preset_key(name) == preset_key(self.selected))
            row.clicked.connect(lambda _c=False, n=name: self.select(n))
            row.double.connect(lambda n=name: (self.select(n), self.accept_selected()))
            h.addWidget(row, 1)
            fmt_label = QLabel(str(p.get("formatLabel") or ""), line)
            fmt_label.setEnabled(False)
            fmt_label.setFixedWidth(150)  # a column, as egui's rows of one width
            h.addWidget(fmt_label)
            if p.get("builtin") is not True:
                user = QLabel(tr("User"), line)
                user.setObjectName(f"presetManager.user.{name}")
                user.setStyleSheet(f"color: {t.c('accent').name()};")
                h.addWidget(user)
            self.list_lay.addWidget(line)
            self.rows[name] = row
        self.list_lay.addStretch(1)
        self._sync_buttons()

    def select(self, name: str) -> None:
        self.selected = name
        for n, r in self.rows.items():
            r.setChecked(preset_key(n) == preset_key(name))
        self._sync_buttons()

    def _sync_buttons(self) -> None:
        info = self._info()
        self.description.setText(tr(str(info.get("description") or "")) if info else "")
        self.ok.setEnabled(info is not None)
        self.export.setEnabled(info is not None)
        self.delete.setEnabled(info is not None and info.get("builtin") is not True)

    # ----- control channel (egui's ``ExportUi::manager``) -----
    def state(self) -> dict[str, Any]:
        return {"query": self.search.text(), "favoritesOnly": self.favorites_only.isChecked(), "selected": self.selected,
                "saveName": self.save_name.text()}

    def set_state(self, fields: dict[str, Any], names: list[str]) -> None:
        """Merge egui's ``PresetManager`` fields; ``selected`` must name one of ``names`` (or be
        empty). ValueError names a bad field, and nothing is changed then."""
        self.check_state(fields, names)
        if "query" in fields:
            self.search.setText(fields["query"])
        if "favoritesOnly" in fields:
            self.favorites_only.setChecked(fields["favoritesOnly"])
        if "saveName" in fields:
            self.save_name.setText(fields["saveName"])
        if isinstance(fields.get("selected"), str):
            self.select(fields["selected"])

    @staticmethod
    def check_state(fields: dict[str, Any], names: list[str]) -> None:
        known = ("query", "favoritesOnly", "selected", "saveName")
        bad = [k for k in fields if k not in known]
        if bad:
            raise ValueError(f"manager: unknown field {bad[0]!r} (known: {', '.join(known)})")
        for k in ("query", "selected", "saveName"):
            if k in fields and not isinstance(fields[k], str):
                raise ValueError(f"manager.{k} takes a string")
        if "favoritesOnly" in fields and not isinstance(fields["favoritesOnly"], bool):
            raise ValueError("manager.favoritesOnly takes true or false")
        sel = fields.get("selected")
        if sel and preset_key(sel) not in {preset_key(n) for n in names}:
            raise ValueError(f"manager.selected: no preset named {sel!r}")

    # ----- actions -----
    def _run(self, command: str, params: dict[str, Any], then: Callable[[Any], None] | None = None) -> None:
        if self.engine is None:
            return

        def done(result: Any, _b: Any = None) -> None:
            if then is not None:
                then(result)
            self.refresh()

        self.engine.execute(command, params, on_reply=done, on_error=lambda m: self.status.emit(str(m)))

    def toggle_favorite(self, name: str) -> None:
        self._run("export.presets.favorite", {"name": name})

    def save_current(self) -> None:
        name = self.save_name.text().strip()
        if not name:
            return
        params = {k: v for k, v in self.current().items() if k not in ("preset", "path", "range", "startSeconds", "endSeconds", "sequence")}
        from_ = self.current().get("preset")
        if isinstance(from_, str) and from_:
            params["from"] = from_
        params["name"] = name
        self.selected = name
        self._run("export.presets.save", params)

    def delete_preset(self) -> None:
        info = self._info()
        if info is None or info.get("builtin") is True:
            return
        self.selected = ""
        self._run("export.presets.delete", {"name": info["name"]})

    def import_presets(self) -> None:
        path = self.pick_open(tr("Export presets"), "JSON (*.json)", "")
        if path:
            self._run("export.presets.import", {"path": path})

    def export_preset(self) -> None:
        info = self._info()
        if info is None:
            return
        path = self.pick_save(tr("Export presets"), "JSON (*.json)", f"{info['name']}.json")
        if path:
            self._run("export.presets.export", {"path": path, "names": [info["name"]]})

    def accept_selected(self) -> None:
        info = self._info()
        if info is None:
            return
        self.applied.emit(info["name"])
        self.accept()

    def keyPressEvent(self, e: Any) -> None:
        # Enter in the name field saves; it does not apply the selected preset
        if e.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter) and self.save_name.hasFocus():
            self.save_current()
            return
        super().keyPressEvent(e)
