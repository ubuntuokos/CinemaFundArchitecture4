"""Dialogs: a generic parameter form for any engine command, and native file dialogs for the
commands that take paths."""

from __future__ import annotations

import json
from typing import Any

from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QLabel,
    QLineEdit,
    QVBoxLayout,
    QWidget,
)

from .commands import Command, parse_value
from .i18n import tr

PROJECT_FILTER = "CFA4 project (*.fcproj);;All files (*)"


class ParamDialog(QDialog):
    """One line edit per parameter of ``command`` (values parse as JSON, else text)."""

    def __init__(
        self, command: Command, parent: QWidget | None = None, error: str = "", values: dict[str, Any] | None = None
    ) -> None:
        super().__init__(parent)
        self.setObjectName(f"paramDialog.{command.id}")
        self.setWindowTitle(tr(command.label))
        self.command = command
        self.fields: dict[str, QLineEdit] = {}
        layout = QVBoxLayout(self)
        if error:
            msg = QLabel(error, self)
            msg.setWordWrap(True)
            msg.setStyleSheet("color: #e57373;")
            layout.addWidget(msg)
        form = QFormLayout()
        for spec in command.param_specs:
            edit = QLineEdit(self)
            edit.setObjectName(f"paramDialog.{command.id}.{spec.name}")
            edit.setPlaceholderText(spec.hint + ("  (" + tr("optional") + ")" if spec.optional else ""))
            if values and spec.name in values:
                v = values[spec.name]
                edit.setText(v if isinstance(v, str) else json.dumps(v))
            form.addRow(spec.name + ("" if spec.optional else " *"), edit)
            self.fields[spec.name] = edit
        layout.addLayout(form)
        raw = QLabel(f"<small><code>{command.id}</code> {command.params}</small>", self)
        raw.setWordWrap(True)
        layout.addWidget(raw)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

    def params(self) -> dict[str, Any]:
        out: dict[str, Any] = {}
        for name, edit in self.fields.items():
            value = parse_value(edit.text())
            if value is not None:
                out[name] = value
        return out


def path_params(command: Command, parent: QWidget, project_path: str | None) -> dict[str, Any] | None:
    """Params from a native file dialog for commands that take paths. ``None``: not a path
    command (or a dialog not needed). ``{}`` with ``__cancel__``: the user cancelled."""
    cid = command.id
    names = {p.name for p in command.param_specs}
    start = project_path or ""
    if cid == "file.open":
        path, _ = QFileDialog.getOpenFileName(parent, tr("Open Project"), start, PROJECT_FILTER)
        return {"path": path} if path else {"__cancel__": True}
    if cid in ("file.saveAs", "file.saveCopy") or (cid == "file.save" and not project_path):
        path, _ = QFileDialog.getSaveFileName(parent, tr(command.label), start, PROJECT_FILTER)
        if not path:
            return {"__cancel__": True}
        if not path.endswith(".fcproj"):
            path += ".fcproj"
        return {"path": path}
    if cid.startswith("file.import") and "paths" in names:
        paths, _ = QFileDialog.getOpenFileNames(parent, tr(command.label), "", tr("All files") + " (*)")
        return {"paths": paths} if paths else {"__cancel__": True}
    if cid.startswith("file.import") and "path" in names:
        path, _ = QFileDialog.getOpenFileName(parent, tr(command.label), "", tr("All files") + " (*)")
        return {"path": path} if path else {"__cancel__": True}
    return None
