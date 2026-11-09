"""Dialogs: a generic parameter form for any engine command, and native file dialogs for the
commands that take paths."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPushButton,
    QRadioButton,
    QScrollArea,
    QVBoxLayout,
    QWidget,
)

from .commands import Command, parse_value
from .i18n import fmt, tr

PROJECT_FILTER = "CFA4 project (*.fcproj);;All files (*)"
FRAME_FILTER = "PNG (*.png);;JPEG (*.jpg *.jpeg);;TIFF (*.tif *.tiff);;BMP (*.bmp)"


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


def path_params(
    command: Command, parent: QWidget, project_path: str | None, given: dict[str, Any] | None = None
) -> dict[str, Any] | None:
    """Params from a native file dialog for commands that take paths. ``None``: not a path
    command (or a dialog not needed). ``{}`` with ``__cancel__``: the user cancelled. ``given``:
    the parameters the caller already has (a Text panel's ``track``, a ``format``)."""
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
            # the dialog asked about the name typed, not this one: ask before replacing it
            path += ".fcproj"
            if Path(path).exists() and not confirm_replace(parent, tr(command.label), path):
                return {"__cancel__": True}
        return {"path": path}
    if cid == "file.exportFrame" and "path" in names:
        path, chosen = QFileDialog.getSaveFileName(parent, tr("Export Frame"), "", FRAME_FILTER)
        if not path:
            return {"__cancel__": True}
        ext = Path(path).suffix.lower().lstrip(".")
        fmt = {"jpg": "jpeg", "jpeg": "jpeg", "tif": "tiff", "tiff": "tiff", "bmp": "bmp", "png": "png"}.get(ext)
        if fmt is None:
            fmt = next((f for f in ("jpeg", "tiff", "bmp") if f.upper() in chosen.upper()), "png")
            path += {"png": ".png", "jpeg": ".jpg", "tiff": ".tif", "bmp": ".bmp"}[fmt]
            if Path(path).exists() and not confirm_replace(parent, tr("Export Frame"), path):
                return {"__cancel__": True}
        return {"path": path, "format": fmt}
    if cid == "captions.export":
        return caption_export_params(parent, given or {})
    if cid == "graphics.template.install":
        filt = tr("Motion Graphics Templates") + " (*.fcgt)"
        path, _ = QFileDialog.getOpenFileName(parent, tr(command.label), "", filt)
        return {"path": path} if path else {"__cancel__": True}
    if cid in ("presets.export", "presets.import"):
        filt = tr("Effect presets") + " (*.json)"
        if cid == "presets.import":
            path, _ = QFileDialog.getOpenFileName(parent, tr(command.label), "", filt)
            return {"path": path} if path else {"__cancel__": True}
        path, _ = QFileDialog.getSaveFileName(parent, tr(command.label), "", filt)
        if not path:
            return {"__cancel__": True}
        return {"path": path if path.endswith(".json") else path + ".json"}
    if cid == "export.batchVariants":  # the spreadsheet; one sequence and export per row
        filt = tr("Spreadsheets") + " (*.csv *.tsv *.txt *.xlsx *.ods);;" + tr("All files") + " (*)"
        path, _ = QFileDialog.getOpenFileName(parent, tr(command.label), "", filt)
        return {"path": path} if path else {"__cancel__": True}
    if cid.startswith("file.import") and "paths" in names:
        paths, _ = QFileDialog.getOpenFileNames(parent, tr(command.label), "", tr("All files") + " (*)")
        return {"paths": paths} if paths else {"__cancel__": True}
    if cid.startswith("file.import") and "path" in names:
        path, _ = QFileDialog.getOpenFileName(parent, tr(command.label), "", tr("All files") + " (*)")
        return {"path": path} if path else {"__cancel__": True}
    return None


# The formats `captions.export` writes (cfa4_captions::Format::ALL): extension, name.
CAPTION_FORMATS = (
    ("srt", "SubRip (SRT)"),
    ("vtt", "WebVTT"),
    ("scc", "Scenarist SCC (CEA-608)"),
    ("mcc", "MacCaption MCC (CEA-608/708)"),
    ("stl", "EBU STL"),
    ("ttml", "TTML (IMSC1)"),
    ("dfxp", "DFXP (TTML1)"),
)
CAPTION_FORMAT_KEY = "captionExport/format"  # the format written last, offered next time


def caption_format(name: Any) -> str | None:
    """The extension of the caption format ``name`` names (an extension or the engine's names)."""
    key = str(name or "").strip().lstrip(".").lower()
    aliases = {"subrip": "srt", "webvtt": "vtt", "ebu": "stl", "ebu stl": "stl", "imsc1": "ttml", "xml": "ttml"}
    key = aliases.get(key, key)
    return key if any(key == ext for ext, _ in CAPTION_FORMATS) else None


def caption_destination(path: str, chosen: str) -> tuple[str, str]:
    """Where to write, and in which format, from the save dialog's answer and the chosen format
    (upstream #3523): a caption format's extension, in any case, is the user's choice (that file, in
    that format); any other name gets the chosen format's extension (``subs`` and ``subs.`` give
    ``subs.vtt``, ``subs.final`` gives ``subs.final.vtt``)."""
    typed = Path(path).suffix.lstrip(".").lower()
    if typed and any(typed == ext for ext, _ in CAPTION_FORMATS):
        return path, typed
    return f"{path.rstrip('.')}.{chosen}", chosen


class CaptionExportDialog(QDialog):
    """File ▸ Export ▸ Captions… and the Text panel's Export captions… (upstream #3523): the File
    Format of every format ``captions.export`` writes. Automation ids: ``captionExport.format``,
    ``captionExport.ok``, ``captionExport.cancel``."""

    def __init__(self, parent: QWidget | None, chosen: str) -> None:
        super().__init__(parent)
        self.setObjectName("captionExport")
        self.setWindowTitle(tr("Caption Export Settings"))
        form = QFormLayout(self)
        self.format = QComboBox(self)
        self.format.setObjectName("captionExport.format")
        for ext, name in CAPTION_FORMATS:
            self.format.addItem(name, ext)
        self.format.setCurrentIndex(max(0, self.format.findData(chosen)))
        self.format.setMinimumWidth(240)
        form.addRow(tr("File Format"), self.format)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.button(QDialogButtonBox.StandardButton.Ok).setObjectName("captionExport.ok")
        buttons.button(QDialogButtonBox.StandardButton.Cancel).setObjectName("captionExport.cancel")
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        form.addRow(buttons)

    def chosen(self) -> str:
        return str(self.format.currentData())


def caption_export_params(parent: QWidget, given: dict[str, Any]) -> dict[str, Any]:
    """Caption Export Settings, then a save panel offering only the chosen format. A ``format``
    in ``given`` is selected, else the one written last, else SRT. If the save panel's name gets a
    new extension and that file exists, nothing is written: the panel only asked about the name as
    typed."""
    settings = getattr(parent, "settings", None)
    last = settings.value(CAPTION_FORMAT_KEY, "") if settings is not None else ""
    chosen = caption_format(given.get("format")) or caption_format(last) or "srt"
    dlg = CaptionExportDialog(parent, chosen)
    if dlg.exec() != QDialog.DialogCode.Accepted:
        return {"__cancel__": True}
    chosen = dlg.chosen()
    name = dict(CAPTION_FORMATS)[chosen]
    seq = getattr(parent, "sequence", None)
    stem = str((seq or {}).get("name") or "").strip() if isinstance(seq, dict) else ""
    stem = "".join("_" if c in " /\\:" else c for c in stem) or "Captions"
    answered, _ = QFileDialog.getSaveFileName(parent, tr("Export Captions"), f"{stem}.{chosen}", f"{name} (*.{chosen})")
    if not answered:
        return {"__cancel__": True}
    path, written = caption_destination(answered, chosen)
    if path != answered and Path(path).exists():
        message = fmt("{name} already exists, so nothing was exported. To replace it, choose it in the save dialog.", name=Path(path).name)
        bar = getattr(parent, "statusBar", None)
        if callable(bar):
            bar().showMessage(message, 6000)
        return {"__cancel__": True}
    if settings is not None:
        settings.setValue(CAPTION_FORMAT_KEY, written)
    return {"path": path, "format": written}


def confirm_replace(parent: QWidget, title: str, path: str) -> bool:
    """Yes / Cancel (Cancel by default) before a save replaces an existing file."""
    box = QMessageBox(
        QMessageBox.Icon.Warning,
        title,
        tr("'{name}' already exists. Replace it?").replace("{name}", Path(path).name),
        parent=parent,
    )
    box.setObjectName("dialog.replace")
    box.setStandardButtons(QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel)
    box.setDefaultButton(QMessageBox.StandardButton.Cancel)
    return box.exec() == QMessageBox.StandardButton.Yes


class InterpretFootageDialog(QDialog):
    """egui's Interpret Footage (Color Management): the media colour space from the file's
    metadata, and an override (Use file metadata, or one of the engine's colour spaces). OK runs
    ``clip.interpretFootage`` with ``colorSpace`` through ``send``."""

    def __init__(self, parent: QWidget, engine: Any, items: list[int], send: Any) -> None:
        super().__init__(parent)
        self.setObjectName("interpretFootage")
        self.setWindowTitle(tr("Interpret Footage"))
        self.items, self.send = items, send
        layout = QVBoxLayout(self)
        n = len(items)
        heading = QLabel(fmt("Color Management · {n} item", n=n) if n == 1 else fmt("Color Management · {n} items", n=n), self)
        heading.setStyleSheet("font-weight: bold;")
        layout.addWidget(heading)
        self.detected = QLabel(fmt("Media colour space (from file metadata): {detected}", detected="—"), self)
        self.detected.setObjectName("interpretFootage.detected")
        layout.addWidget(self.detected)
        layout.addWidget(QLabel(tr("Override media colour space:"), self))
        self.group = QButtonGroup(self)
        self.choices: dict[str, QRadioButton] = {}
        self.list = QVBoxLayout()
        self._add_choice("auto", tr("Use file metadata"))
        self.choices["auto"].setChecked(True)
        box = QWidget(self)
        box.setLayout(self.list)
        scroll = QScrollArea(self)
        scroll.setWidget(box)
        scroll.setWidgetResizable(True)
        scroll.setMaximumHeight(300)
        layout.addWidget(scroll)
        # Auro-3D (#323): shown for a carrier only, on = the restored layout, off = its own channels
        self.auro = QCheckBox(tr("Decode Auro-3D height channels"), self)
        self.auro.setObjectName("interpretFootage.auro")
        self.auro_hint = QLabel(tr("Off: the file plays its own channels, as without an Auro-3D decoder."), self)
        self.auro_hint.setWordWrap(True)
        for w in (self.auro, self.auro_hint):
            w.setVisible(False)
            layout.addWidget(w)
        # each selected carrier's setting; mixed settings show the box partly checked and stay
        # as they are unless the user picks one value for all
        self._auro_states: list[bool] = []
        self.auro.clicked.connect(lambda _c=False: self.auro.setTristate(False))
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.button(QDialogButtonBox.StandardButton.Ok).setObjectName("interpretFootage.ok")
        buttons.button(QDialogButtonBox.StandardButton.Cancel).setObjectName("interpretFootage.cancel")
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)
        if engine is not None and getattr(engine, "running", False):
            engine.execute("color.spaces", {}, on_reply=lambda r, _b: self._spaces(r))
            if items:
                engine.execute("media.colorInfo", {"item": items[0]}, on_reply=lambda r, _b: self._info(r))
                for item in items:
                    engine.execute("media.auroInfo", {"item": item}, on_reply=lambda r, _b: self._auro_info(r))

    def _add_choice(self, cid: str, label: str) -> None:
        b = QRadioButton(label, self)
        b.setObjectName(f"interpretFootage.space.{cid}")
        self.group.addButton(b)
        self.choices[cid] = b
        self.list.addWidget(b)

    def _spaces(self, result: Any) -> None:
        for c in (result or {}).get("colorSpaces") or [] if isinstance(result, dict) else []:
            if isinstance(c, dict) and isinstance(c.get("id"), str) and c["id"] not in self.choices:
                self._add_choice(c["id"], tr(str(c.get("label") or c["id"])))
        self._select(self._override)

    _override: str | None = None

    def _info(self, result: Any) -> None:
        if not isinstance(result, dict):
            return
        self.detected.setText(fmt("Media colour space (from file metadata): {detected}",
                                  detected=str(result.get("detectedLabel") or "—")))
        self._override = result.get("override") if isinstance(result.get("override"), str) else None
        self._select(self._override)

    def _auro_info(self, result: Any) -> None:
        if not isinstance(result, dict) or result.get("carrier") is not True:
            return
        self._auro_states.append(result.get("decoding") is not False)
        mixed = len(set(self._auro_states)) > 1
        self.auro.setTristate(mixed)
        if mixed:
            self.auro.setCheckState(Qt.CheckState.PartiallyChecked)
        else:
            self.auro.setChecked(self._auro_states[0])
        for w in (self.auro, self.auro_hint):
            w.setVisible(True)

    def _select(self, cid: str | None) -> None:
        if cid is not None and cid in self.choices:
            self.choices[cid].setChecked(True)

    def chosen(self) -> str:
        return next((cid for cid, b in self.choices.items() if b.isChecked()), "auto")

    def accept(self) -> None:
        params: dict[str, Any] = {"colorSpace": self.chosen()}
        state = self.auro.checkState()
        if self._auro_states and state != Qt.CheckState.PartiallyChecked:
            on = state == Qt.CheckState.Checked
            if any(was != on for was in self._auro_states):
                params["auroDecoding"] = on
        if self.items:
            params["items"] = self.items
        self.send("clip.interpretFootage", params)
        super().accept()


class ErrorDialog(QDialog):
    """egui's error window: an exception escaped somewhere in the window; the session keeps
    running. Save Project (``error.save``) or Continue (``error.dismiss``)."""

    def __init__(self, parent: QWidget, save: Any) -> None:
        super().__init__(parent)
        self.setObjectName("error")
        self.setWindowTitle(tr("CFA4 encountered an error"))
        self.setModal(False)
        layout = QVBoxLayout(self)
        intro = QLabel(tr("Something went wrong while drawing the window. Your project is still open; save it to be safe."), self)
        intro.setWordWrap(True)
        intro.setMaximumWidth(460)
        layout.addWidget(intro)
        self.message = QLabel(self)
        self.message.setObjectName("error.message")
        self.message.setWordWrap(True)
        self.message.setMaximumWidth(460)
        self.message.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.message.setStyleSheet("font-family: monospace; font-size: 11px;")
        layout.addWidget(self.message)
        self.details = QLabel(self)
        self.details.setObjectName("error.details")
        self.details.setWordWrap(True)
        self.details.setStyleSheet("font-size: 11px;")
        layout.addWidget(self.details)
        row = QHBoxLayout()
        save_button = QPushButton(tr("Save Project"), self)
        save_button.setObjectName("error.save")
        save_button.setAutoDefault(False)
        save_button.clicked.connect(save)
        dismiss = QPushButton(tr("Continue"), self)
        dismiss.setObjectName("error.dismiss")
        dismiss.clicked.connect(self.hide)
        row.addWidget(save_button)
        row.addWidget(dismiss)
        row.addStretch(1)
        layout.addLayout(row)

    def show_error(self, summary: str, log: Any) -> None:
        self.message.setText(summary)
        self.details.setText(fmt("Details: {path}", path=str(log)) if log is not None else "")
        self.details.setVisible(log is not None)
        self.show()
        self.raise_()
