"""CFX expression editor (plan CFA4-AX-INTERPRETER-001, X4; N3-6′).

A non-modal window for the restricted spatial expression language of ``docs/expr-language.md``:
syntax highlighting, a function list, validation while typing, the compile state, the saved
expressions of the project with this machine's approval, a preview run and a bake.

The window owns no language knowledge beyond the highlighting lists: every check, save, approval
and run is the engine's (``expr.check``, ``expr.save``, ``expr.list``, ``expr.get``, ``expr.delete``,
``expr.approve``, ``expr.run_preview``, ``expr.bake``), a diagnostic keeps its stable code and its
line and column, and nothing runs until the user presses Preview or Bake. Opening a project, or this
window, never runs an expression.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QColor, QFont, QSyntaxHighlighter, QTextCharFormat, QTextCursor, QTextDocument
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPlainTextEdit,
    QPushButton,
    QSplitter,
    QTextEdit,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import TR, fmt, tr

CHECK_DELAY_MS = 350
KEYWORDS = ("reads", "writes", "param", "if", "else", "for", "in", "true", "false")
TYPES = ("float", "int", "bool", "vec3")
BUILTINS = ("P", "ijk", "voxelsize", "id")
# (name, signature) of every function of the language (docs/expr-language.md §7); signatures are language-neutral
FUNCTIONS: tuple[tuple[str, str], ...] = (
    ("abs", "abs(x)"),
    ("min", "min(a, b)"),
    ("max", "max(a, b)"),
    ("clamp", "clamp(x, lo, hi)"),
    ("lerp", "lerp(a, b, t)"),
    ("smoothstep", "smoothstep(e0, e1, x)"),
    ("step", "step(edge, x)"),
    ("fit", "fit(x, a, b, c, d)"),
    ("floor", "floor(x)"),
    ("ceil", "ceil(x)"),
    ("round", "round(x)"),
    ("sqrt", "sqrt(x)"),
    ("pow", "pow(x, y)"),
    ("exp", "exp(x)"),
    ("log", "log(x)"),
    ("sin", "sin(x)"),
    ("cos", "cos(x)"),
    ("tan", "tan(x)"),
    ("atan2", "atan2(y, x)"),
    ("length", "length(v)"),
    ("dot", "dot(a, b)"),
    ("cross", "cross(a, b)"),
    ("normalize", "normalize(v)"),
    ("noise", "noise(p)"),
    ("hash", "hash(a, …)"),
    ("rand", "rand(seed, id)"),
)
FUNCTION_NAMES = tuple(n for n, _ in FUNCTIONS)
LEAF_TYPES = ("float", "vec3")
OUTPUT = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(float|vec3)\s*$")


def parse_outputs(text: str) -> dict[str, str] | None:
    """``hot:float, tint:vec3`` as ``{name: type}``; ``None`` when an entry is not ``name:float|vec3``."""
    out: dict[str, str] = {}
    for part in text.split(","):
        if not part.strip():
            continue
        m = OUTPUT.match(part)
        if m is None:
            return None
        out[m.group(1)] = m.group(2)
    return out


def outputs_text(outputs: dict[str, str] | None) -> str:
    return ", ".join(f"{n}:{t}" for n, t in sorted((outputs or {}).items()))


def grid_attrs(inspection: dict[str, Any] | None) -> dict[str, str]:
    """The float and vec3 grids of a ``vdb.inspect`` answer as the attributes an expression may read."""
    attrs: dict[str, str] = {}
    for g in (inspection or {}).get("grids") or []:
        if not isinstance(g, dict) or not isinstance(g.get("name"), str):
            continue
        t = str(g.get("type") or "").lower()
        if t in ("float", "double"):
            attrs.setdefault(g["name"], "float")
        elif t in ("vec3f", "vec3s", "vec3d", "vec3"):
            attrs.setdefault(g["name"], "vec3")
    return attrs


def diagnostic_text(d: dict[str, Any]) -> str:
    """``E-ATTR-1 3:7: message`` in the interface language (the engine sends Hungarian and English)."""
    hu = str(d.get("messageHu") or "")
    en = str(d.get("messageEn") or "")
    message = (hu or en) if TR.language == "hu" else (en or hu)
    return f"{d.get('code', '')} {d.get('line', 0)}:{d.get('col', 0)}: {message}"


def error_range(source: str, d: dict[str, Any]) -> tuple[int, int] | None:
    """Character offset and length of a diagnostic (1-based line and column), clipped to the text."""
    nums = [d.get(k) for k in ("line", "col", "len")]
    if not all(isinstance(v, int) and not isinstance(v, bool) for v in nums):
        return None
    line, col, length = (v for v in nums if isinstance(v, int))
    if line < 1 or col < 1:
        return None
    lines = source.split("\n")
    if line > len(lines):
        return None
    start = sum(len(x) + 1 for x in lines[: line - 1]) + min(col - 1, len(lines[line - 1]))
    end = min(start + max(length, 1), sum(len(x) + 1 for x in lines[:line]) - 1 if line < len(lines) else len(source))
    return (start, max(end - start, 0)) if end > start else (start, 0)


class ExpressionHighlighter(QSyntaxHighlighter):
    """Colours keywords, types, built-ins, functions, ``@attributes``, numbers and comments.

    Block comments (``/* … */``) are tracked across lines with the block state.
    """

    def __init__(self, document: QTextDocument) -> None:
        super().__init__(document)

        def fmt_of(color: str, bold: bool = False, italic: bool = False) -> QTextCharFormat:
            f = QTextCharFormat()
            f.setForeground(QColor(color))
            if bold:
                f.setFontWeight(QFont.Weight.Bold)
            f.setFontItalic(italic)
            return f

        self.formats = {
            "keyword": fmt_of("#c678dd", bold=True),
            "type": fmt_of("#56b6c2"),
            "builtin": fmt_of("#d19a66"),
            "function": fmt_of("#61afef"),
            "attribute": fmt_of("#e5c07b", bold=True),
            "number": fmt_of("#98c379"),
            "comment": fmt_of("#7f848e", italic=True),
        }

        def word(names: tuple[str, ...]) -> re.Pattern[str]:
            return re.compile(r"\b(?:" + "|".join(names) + r")\b")

        self.rules = [
            (word(KEYWORDS), "keyword"),
            (word(TYPES), "type"),
            (word(BUILTINS), "builtin"),
            (re.compile(r"\b(?:" + "|".join(FUNCTION_NAMES) + r")(?=\s*\()"), "function"),
            (re.compile(r"@[A-Za-z_][A-Za-z0-9_]*"), "attribute"),
            (re.compile(r"\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b"), "number"),
        ]

    def spans(self, text: str, state: int) -> tuple[list[tuple[int, int, str]], int]:
        """``(start, length, kind)`` spans of one line and the block state after it (1: inside ``/* */``)."""
        out: list[tuple[int, int, str]] = []
        pos = 0
        n = len(text)
        while pos < n:
            if state == 1:
                end = text.find("*/", pos)
                if end < 0:
                    out.append((pos, n - pos, "comment"))
                    return out, 1
                out.append((pos, end + 2 - pos, "comment"))
                pos, state = end + 2, 0
                continue
            line_c, block_c = text.find("//", pos), text.find("/*", pos)
            starts = [i for i in (line_c, block_c) if i >= 0]
            stop = min(starts) if starts else n
            for rx, kind in self.rules:
                for m in rx.finditer(text, pos, stop):
                    out.append((m.start(), m.end() - m.start(), kind))
            if not starts:
                break
            if stop == line_c and (block_c < 0 or line_c < block_c):
                out.append((line_c, n - line_c, "comment"))
                return out, 0
            pos, state = block_c, 1
            out.append((block_c, 2, "comment"))
            pos = block_c + 2
        return out, state

    def highlightBlock(self, text: str) -> None:  # noqa: N802 (Qt override)
        spans, state = self.spans(text, max(self.previousBlockState(), 0))
        for start, length, kind in spans:
            self.setFormat(start, length, self.formats[kind])
        self.setCurrentBlockState(state)


class ExpressionEditor(QDialog):
    """Non-modal expression editor; the engine does all the work."""

    def __init__(
        self, engine: EngineClient, item_provider: Callable[[], int | None], parent: QWidget | None = None
    ) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.expression")
        self.setWindowTitle(tr("Expression Editor"))
        self.resize(900, 600)
        self.engine = engine
        self.item_provider = item_provider
        self.current_id: int | None = None
        self.attrs: dict[str, str] = {}
        self.diagnostic: dict[str, Any] | None = None
        self.compiled = False
        self._generation = 0

        self.saved = QListWidget(self)
        self.saved.setObjectName("expr.saved")
        self.saved.currentItemChanged.connect(self._load_saved)
        self.name = QLineEdit(self)
        self.name.setObjectName("expr.name")
        self.domain = QComboBox(self)
        self.domain.setObjectName("expr.domain")
        self.domain.setEditable(True)
        self.outputs = QLineEdit(self)
        self.outputs.setObjectName("expr.outputs")
        self.outputs.setPlaceholderText("hot:float, tint:vec3")
        self.source = QPlainTextEdit(self)
        self.source.setObjectName("expr.source")
        mono = QFont("monospace")
        mono.setStyleHint(QFont.StyleHint.Monospace)
        self.source.setFont(mono)
        self.highlighter = ExpressionHighlighter(self.source.document())
        self.status = QLabel(self)
        self.status.setObjectName("expr.status")
        self.status.setWordWrap(True)
        self.approval = QLabel(self)
        self.approval.setObjectName("expr.approval")
        self.message = QLabel(self)
        self.message.setObjectName("expr.message")
        self.message.setWordWrap(True)
        self.functions = QListWidget(self)
        self.functions.setObjectName("expr.functions")
        for name, signature in FUNCTIONS:
            item = QListWidgetItem(signature)
            item.setData(Qt.ItemDataRole.UserRole, name)
            self.functions.addItem(item)
        self.functions.itemActivated.connect(self._insert_function)

        def button(text: str, ident: str, slot: Callable[[], None]) -> QPushButton:
            b = QPushButton(text, self)
            b.setObjectName(ident)
            b.clicked.connect(lambda _c=False: slot())
            return b

        self.new_button = button(tr("New"), "expr.new", self.new_expression)
        self.save_button = button(tr("Save"), "expr.save", self.save)
        self.delete_button = button(tr("Delete"), "expr.delete", self.delete)
        self.approve_button = button(tr("Approve"), "expr.approve", self.approve)
        self.preview_button = button(tr("Preview"), "expr.preview", lambda: self.run("expr.run_preview"))
        self.bake_button = button(tr("Bake"), "expr.bake", lambda: self.run("expr.bake"))

        left = QWidget(self)
        lv = QVBoxLayout(left)
        lv.addWidget(QLabel(tr("Saved expressions"), left))
        lv.addWidget(self.saved, 1)
        lv.addWidget(self.new_button)
        centre = QWidget(self)
        cv = QVBoxLayout(centre)
        row = QHBoxLayout()
        for label, widget in ((tr("Name:"), self.name), (tr("Grid:"), self.domain), (tr("Outputs:"), self.outputs)):
            row.addWidget(QLabel(label, centre))
            row.addWidget(widget, 1)
        cv.addLayout(row)
        cv.addWidget(self.source, 1)
        cv.addWidget(self.status)
        cv.addWidget(self.approval)
        buttons = QHBoxLayout()
        for b in (self.save_button, self.delete_button, self.approve_button, self.preview_button, self.bake_button):
            buttons.addWidget(b)
        buttons.addStretch(1)
        cv.addLayout(buttons)
        cv.addWidget(self.message)
        right = QWidget(self)
        rv = QVBoxLayout(right)
        rv.addWidget(QLabel(tr("Functions"), right))
        rv.addWidget(self.functions, 1)
        split = QSplitter(self)
        split.addWidget(left)
        split.addWidget(centre)
        split.addWidget(right)
        split.setStretchFactor(1, 1)
        top = QVBoxLayout(self)
        top.addWidget(split)

        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(CHECK_DELAY_MS)
        self._timer.timeout.connect(self.check)
        self.source.textChanged.connect(self._timer.start)
        self.outputs.textChanged.connect(self._timer.start)
        self.domain.editTextChanged.connect(lambda _t: self._update_buttons())
        self._update_buttons()

    # ---- engine round trips ------------------------------------------------------------------------------

    def refresh(self) -> None:
        """Re-read the saved expressions (and the grids of the selected volume)."""
        if not self.engine.running:
            return
        self.engine.execute("expr.list", {}, on_reply=self._got_list, on_error=self._failed)
        item = self.item_provider()
        if item is None:
            self._got_grids({})
            return
        generation = self._generation

        def got_meta(result: Any, _b: bytes | None) -> None:
            volume = (result or {}).get("volume") if isinstance(result, dict) else None
            if generation != self._generation:
                return
            if not isinstance(volume, dict) or not volume.get("path"):
                self._got_grids({})
                return
            self.engine.execute(
                "vdb.inspect",
                {"path": volume["path"]},
                on_reply=lambda r, _b2: self._got_grids((r or {}).get("inspection") if isinstance(r, dict) else None),
                on_error=lambda _m: self._got_grids({}),
            )

        self.engine.execute("metadata.get", {"item": item}, on_reply=got_meta, on_error=lambda _m: self._got_grids({}))

    def _got_grids(self, inspection: dict[str, Any] | None) -> None:
        self.attrs = grid_attrs(inspection)
        keep = self.domain.currentText()
        self.domain.blockSignals(True)
        self.domain.clear()
        self.domain.addItems(list(self.attrs))
        self.domain.setEditText(keep)
        self.domain.blockSignals(False)
        self._update_buttons()
        self._timer.start()

    def _got_list(self, result: Any, _b: bytes | None) -> None:
        keep = self.current_id
        self.saved.blockSignals(True)
        self.saved.clear()
        for e in (result or {}).get("expressions") or [] if isinstance(result, dict) else []:
            if not isinstance(e, dict) or not isinstance(e.get("id"), int):
                continue
            item = QListWidgetItem(str(e.get("name") or ""))
            item.setData(Qt.ItemDataRole.UserRole, e["id"])
            item.setData(Qt.ItemDataRole.UserRole + 1, bool(e.get("approved")))
            self.saved.addItem(item)
            if e["id"] == keep:
                self.saved.setCurrentItem(item)
        self.saved.blockSignals(False)
        self._update_buttons()

    def _failed(self, message: str) -> None:
        self.message.setText(str(message))

    def _load_saved(self, current: QListWidgetItem | None, _previous: QListWidgetItem | None = None) -> None:
        if current is None:
            return
        ident = current.data(Qt.ItemDataRole.UserRole)
        self.engine.execute("expr.get", {"id": ident}, on_reply=self._got_expression, on_error=self._failed)

    def _got_expression(self, result: Any, _b: bytes | None) -> None:
        e = (result or {}).get("expression") if isinstance(result, dict) else None
        if not isinstance(e, dict):
            return
        self.current_id = e.get("id") if isinstance(e.get("id"), int) else None
        self.name.setText(str(e.get("name") or ""))
        self.domain.setEditText(str(e.get("domain") or ""))
        self.outputs.setText(outputs_text(e.get("outputs") if isinstance(e.get("outputs"), dict) else None))
        self.source.setPlainText(str(e.get("source") or ""))
        self._set_approved(bool(e.get("approved")))
        self.message.clear()
        self.check()

    def new_expression(self) -> None:
        self.current_id = None
        self.saved.blockSignals(True)
        self.saved.setCurrentRow(-1)
        self.saved.blockSignals(False)
        for w in (self.name, self.outputs):
            w.clear()
        self.source.clear()
        self.message.clear()
        self._set_approved(None)
        self._update_buttons()

    def check(self) -> None:
        """``expr.check`` of the text as it is now; the answer is shown, nothing runs."""
        text = self.source.toPlainText()
        outputs = parse_outputs(self.outputs.text())
        self._generation += 1
        generation = self._generation
        if not text.strip():
            self._show_check({"ok": False, "empty": True}, text, generation)
            return
        if outputs is None:
            self.diagnostic = None
            self.compiled = False
            self.status.setText(tr("Outputs are written name:float or name:vec3, separated by commas."))
            self.status.setProperty("state", "error")
            self.source.setExtraSelections([])
            self._update_buttons()
            return
        params: dict[str, Any] = {"source": text, "attrs": self.attrs, "outputs": outputs}
        self.engine.execute(
            "expr.check", params, on_reply=lambda r, _b: self._show_check(r, text, generation), on_error=self._failed
        )

    def _show_check(self, result: Any, text: str, generation: int) -> None:
        if generation != self._generation or not isinstance(result, dict):
            return
        selections: list[QTextEdit.ExtraSelection] = []
        self.diagnostic = None
        self.compiled = bool(result.get("ok"))
        if result.get("empty"):
            self.status.setText("")
            self.status.setProperty("state", "idle")
        elif self.compiled:
            reads = ", ".join(result.get("reads") or []) or "–"
            writes = ", ".join(result.get("writes") or []) or "–"
            warnings = len(result.get("warnings") or [])
            line = fmt("Compiles: reads {reads}; writes {writes}", reads=reads, writes=writes)
            self.status.setText(line + (f" ({warnings}⚠)" if warnings else ""))
            self.status.setProperty("state", "ok")
        else:
            d = result.get("error")
            self.diagnostic = d if isinstance(d, dict) else None
            self.status.setText(diagnostic_text(d) if isinstance(d, dict) else "")
            self.status.setProperty("state", "error")
            span = error_range(text, d) if isinstance(d, dict) else None
            if span is not None:
                sel = QTextEdit.ExtraSelection()
                cursor = QTextCursor(self.source.document())
                cursor.setPosition(span[0])
                cursor.setPosition(span[0] + max(span[1], 1), QTextCursor.MoveMode.KeepAnchor)
                sel.cursor = cursor
                f = QTextCharFormat()
                f.setUnderlineStyle(QTextCharFormat.UnderlineStyle.WaveUnderline)
                f.setUnderlineColor(QColor("#e06c75"))
                sel.format = f
                selections.append(sel)
        self.source.setExtraSelections(selections)
        self.status.style().unpolish(self.status)
        self.status.style().polish(self.status)
        self._update_buttons()

    def _fields(self) -> dict[str, Any] | None:
        outputs = parse_outputs(self.outputs.text())
        if outputs is None or not self.name.text().strip() or not self.domain.currentText().strip():
            return None
        return {
            "name": self.name.text().strip(),
            "source": self.source.toPlainText(),
            "domain": self.domain.currentText().strip(),
            "outputs": outputs,
        }

    def save(self) -> None:
        params = self._fields()
        if params is None:
            return
        if self.current_id is not None:
            params["id"] = self.current_id
        self.engine.execute("expr.save", params, on_reply=self._saved, on_error=self._failed)

    def _saved(self, result: Any, _b: bytes | None) -> None:
        e = (result or {}).get("expression") if isinstance(result, dict) else None
        if isinstance(e, dict) and isinstance(e.get("id"), int):
            self.current_id = e["id"]
            self._set_approved(bool(e.get("approved")))
        self.message.setText(tr("Saved"))
        self.refresh()

    def delete(self) -> None:
        if self.current_id is None:
            return
        ident = self.current_id
        self.engine.execute(
            "expr.delete",
            {"id": ident},
            on_reply=self._deleted,
            on_error=self._failed,
        )

    def _deleted(self, _result: Any, _b: bytes | None) -> None:
        self.new_expression()
        self.refresh()

    def _approved(self, _result: Any, _b: bytes | None) -> None:
        self._set_approved(True)
        self.refresh()

    def approve(self) -> None:
        if self.current_id is None:
            return
        self.engine.execute(
            "expr.approve",
            {"id": self.current_id},
            on_reply=self._approved,
            on_error=self._failed,
        )

    def run(self, command: str) -> None:
        """Preview runs the text as it is in the window; Bake runs the saved, approved expression."""
        item = self.item_provider()
        if item is None:
            self.message.setText(tr("Select a volume in the Project panel first."))
            return
        if command == "expr.bake":
            if self.current_id is None:
                return
            params: dict[str, Any] = {"id": self.current_id, "item": item}
        else:
            fields = self._fields()
            if fields is None:
                return
            fields.pop("name", None)
            params = {**fields, "item": item, "render": True}
        self.message.setText(tr("Running…"))
        self.engine.execute(command, params, on_reply=self._ran, on_error=self._failed)

    def _ran(self, result: Any, _b: bytes | None) -> None:
        report = (result or {}).get("report") if isinstance(result, dict) else None
        if isinstance(report, dict):
            self.message.setText(
                fmt(
                    "{voxels} voxels in {ms} ms",
                    voxels=f"{int(report.get('voxels') or 0):,}",
                    ms=int(report.get("elapsedMs") or 0),
                )
            )
        else:
            self.message.clear()

    # ---- state ---------------------------------------------------------------------------------------------

    def _set_approved(self, approved: bool | None) -> None:
        self.approval.setProperty("approved", approved)
        if approved is None:
            self.approval.setText("")
        else:
            self.approval.setText(tr("Approved on this machine") if approved else tr("Not approved on this machine"))

    def _update_buttons(self) -> None:
        saved = self.current_id is not None
        valid = self._fields() is not None and self.compiled
        approved = self.approval.property("approved") is True
        self.save_button.setEnabled(valid)
        self.delete_button.setEnabled(saved)
        self.approve_button.setEnabled(saved and not approved)
        self.preview_button.setEnabled(valid)
        self.bake_button.setEnabled(saved and approved)

    def _insert_function(self, item: QListWidgetItem) -> None:
        name = str(item.data(Qt.ItemDataRole.UserRole))
        self.source.insertPlainText(f"{name}(")
        self.source.setFocus()
