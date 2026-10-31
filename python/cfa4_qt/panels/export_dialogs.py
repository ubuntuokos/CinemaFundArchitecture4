"""Export settings dialogs, as egui's ``panels/frame_export.rs`` and
``panels/interchange_export.rs``: Export Frame (the Source or Program frame captured when it
opens: name, still format, depth, folder, Import into project, Replace / Keep existing file) and
File ▸ Export ▸ AAF… / OMF… (audio handling, file format, sample rate, bit depth, trimming and
handles, effects, breakout to mono; OK asks for the file name). Both start from the engine's
``dialog.defaults``; with parameters (``path`` …) the commands run directly.

Automation ids (egui's): ``exportFrame.name``, ``exportFrame.format``, ``exportFrame.depth``,
``exportFrame.path``, ``exportFrame.browse``, ``exportFrame.import``, ``exportFrame.export``
(OK), ``exportFrame.cancel``, ``exportFrame.replace``, ``exportFrame.keep``,
``exportFrame.timecode``; ``<p>.mixdownVideo`` (AAF), ``<p>.title`` (OMF), ``<p>.audio.<k>``,
``<p>.audioFormat.<k>``, ``<p>.sampleRate.<rate>``, ``<p>.bitDepth.<16|24>``, ``<p>.trimAudio``,
``<p>.handles``, ``<p>.renderAudioEffects``, ``<p>.breakoutToMono``, ``<p>.ok``, ``<p>.cancel``
with ``<p>`` ``aafExport`` or ``omfExport``.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from PySide6.QtWidgets import (
    QButtonGroup,
    QComboBox,
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QRadioButton,
    QSpinBox,
    QWidget,
)

from ..i18n import N_, fmt, tr
from .clip_dialogs import ClipDialog, _int

EXPORT_DIALOGS = {
    "file.exportFrame": (N_("Export Frame"), "exportFrame"),
    "file.exportAaf": (N_("AAF Export Settings"), "aafExport"),
    "file.exportOmf": (N_("OMF Export Settings"), "omfExport"),
}
STILL_SUFFIXES = (".png", ".jpg", ".jpeg", ".tif", ".tiff", ".bmp")


def clean_name(name: str) -> str:
    return "".join("_" if c in '/\\:*?"<>|' else c for c in name.strip())


def frame_path(folder: str, name: str, ext: str) -> str:
    """egui's: the name cleaned, a still extension it already has dropped, the format's added."""
    stem = clean_name(name)
    lower = stem.lower()
    for suffix in STILL_SUFFIXES:
        if lower.endswith(suffix):
            stem = stem[: len(stem) - len(suffix)]
            break
    file = stem if stem.lower().endswith(f".{ext}") else f"{stem}.{ext}"
    return str(Path(os.path.expanduser(folder.strip())) / file)


class ExportSettingsDialog(ClipDialog):
    """``state`` is ``dialog.defaults``' answer; ``run(command, params, then)`` as ClipDialog's;
    ``status(text)`` says what a finished export did, ``remember(folder)`` keeps Export Frame's
    folder for the next one (egui's)."""

    def __init__(self, parent: QWidget | None, state: dict[str, Any], run: Any, status: Any = None, remember: Any = None) -> None:
        self.status = status or (lambda _t: None)
        self.remember = remember or (lambda _folder: None)
        self.replace_path: str | None = None
        super().__init__(parent, state, run)
        if self.command == "file.exportFrame":
            self.ok.setObjectName("exportFrame.export")
            self._update_ok()

    def meta(self) -> tuple[str, str, str]:
        title, pre = EXPORT_DIALOGS.get(self.command, (self.command, "exportDialog"))
        return title, pre, N_("OK")

    def builders(self) -> dict[str, Any]:
        return {"file.exportFrame": self._frame, "file.exportAaf": self._interchange, "file.exportOmf": self._interchange}

    # ----- Export Frame -----
    def _frame(self) -> None:
        p = self.params
        self.name_edit = self.text("name", N_("Name:"), 300)
        self.name_edit.textChanged.connect(lambda _t: self._update_ok())
        formats = [f for f in self.info.get("formats") or [] if isinstance(f, list) and len(f) == 3 and isinstance(f[0], str)]
        self.formats = {f[0]: (str(f[1]), f[2] is True) for f in formats} or {"png": ("PNG", True)}
        row = self._row(N_("Format:"))
        self.format_box = QComboBox(self)
        self.format_box.setObjectName("exportFrame.format")
        for ext, (label, _d) in self.formats.items():
            self.format_box.addItem(label, ext)
        i = self.format_box.findData(p.get("format"))
        self.format_box.setCurrentIndex(max(0, i))
        p["format"] = self.format_box.currentData()
        self.format_box.currentIndexChanged.connect(lambda _i: self._set_format())
        row.addWidget(self.format_box)
        row.addStretch(1)
        row = self._row(N_("Depth:"))
        self.depth_box = QComboBox(self)
        self.depth_box.setObjectName("exportFrame.depth")
        for bits in (8, 16):
            self.depth_box.addItem(fmt("{bits} Bit", bits=bits), bits)
        self.depth_box.setCurrentIndex(1 if p.get("depth") == 16 else 0)
        self.depth_box.currentIndexChanged.connect(lambda _i: p.__setitem__("depth", self.depth_box.currentData()))
        row.addWidget(self.depth_box)
        row.addStretch(1)
        row = self._row(N_("Path:"))
        self.folder_edit = QLineEdit(str(p.get("folder") or ""), self)
        self.folder_edit.setObjectName("exportFrame.path")
        self.folder_edit.setMinimumWidth(300)
        self.folder_edit.textChanged.connect(self._folder_changed)
        row.addWidget(self.folder_edit, 1)
        row = QHBoxLayout()
        self.check("import", N_("Import into project"), into=row)
        row.addStretch(1)
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName("exportFrame.browse")
        browse.clicked.connect(self._browse)
        row.addWidget(browse)
        self.body_lay.addLayout(row)
        self.replace_box = QWidget(self)
        lay = QHBoxLayout(self.replace_box)
        lay.setContentsMargins(0, 0, 0, 0)
        self.replace_label = QLabel(self.replace_box)
        self.replace_label.setWordWrap(True)
        lay.addWidget(self.replace_label, 1)
        replace = QPushButton(tr("Replace"), self.replace_box)
        replace.setObjectName("exportFrame.replace")
        replace.clicked.connect(lambda: self._export(replace=True))
        keep = QPushButton(tr("Keep existing file"), self.replace_box)
        keep.setObjectName("exportFrame.keep")
        keep.clicked.connect(lambda: self._set_replace(None))
        lay.addWidget(replace)
        lay.addWidget(keep)
        self.replace_box.hide()
        self.body_lay.addWidget(self.replace_box)
        source = self.info.get("source") is True
        size = fmt("{w} × {h}", w=_int(self.info.get("width"), 0), h=_int(self.info.get("height"), 0))
        about = QLabel(f"{tr('Source') if source else tr('Program')} — {size}", self)
        about.setEnabled(False)
        self.body_lay.addWidget(about)
        item = QLabel(str(self.info.get("itemName") or ""), self)
        item.setEnabled(False)
        self.body_lay.addWidget(item)
        tc = QLabel(fmt("Frame time: {timecode}", timecode=str(self.info.get("timecode") or "")), self)
        tc.setObjectName("exportFrame.timecode")
        tc.setEnabled(False)
        self.body_lay.addWidget(tc)
        self._set_format()

    def _set_format(self) -> None:
        ext = self.format_box.currentData()
        self.params["format"] = ext
        deep = self.formats.get(ext, ("", False))[1]
        item = self.depth_box.model().item(1)
        if item is not None:
            item.setEnabled(deep)
        if not deep:
            self.depth_box.setCurrentIndex(0)
        self.params["depth"] = self.depth_box.currentData()
        self._set_replace(None)

    def _folder_changed(self, text: str) -> None:
        self.params["folder"] = text
        self._set_replace(None)
        self._update_ok()

    def _browse(self) -> None:
        d = QFileDialog.getExistingDirectory(self, tr("Browse…"), os.path.expanduser(self.folder_edit.text().strip()))
        if d:
            self.folder_edit.setText(d)

    def _update_ok(self) -> None:
        if self.command == "file.exportFrame" and hasattr(self, "ok"):
            self.ok.setEnabled(bool(self.name_edit.text().strip()) and bool(self.folder_edit.text().strip()))

    def _set_replace(self, path: str | None) -> None:
        self.replace_path = path
        if not hasattr(self, "replace_box"):
            return
        self.replace_box.setVisible(path is not None)
        self.replace_label.setText(fmt("This file already exists: {path}", path=path or ""))
        if hasattr(self, "ok"):
            self.ok.setVisible(path is None)
            self.cancel.setVisible(path is None)

    def _export(self, replace: bool = False) -> None:
        p = self.params
        path = frame_path(self.folder_edit.text(), self.name_edit.text(), str(p.get("format") or "png"))
        if Path(path).exists() and not (replace and self.replace_path == path):
            self._set_replace(path)
            return
        params = {"path": path, "format": p.get("format"), "depth": p.get("depth"), "import": p.get("import") is True, "target": p.get("target"), "time": p.get("time")}
        key = "item" if p.get("target") == "source" else "sequence"
        if key in p:
            params[key] = p[key]
        self.sent.append(params)

        def done(error: str | None, result: Any = None) -> None:
            if error is not None:
                self._set_replace(None)
                self.fail(error)
                return
            out = result.get("path") if isinstance(result, dict) else None
            self.status(fmt("Exported frame to {path}", path=str(out or path)))
            self.remember(self.folder_edit.text())
            self.accept()

        self.run("file.exportFrame", params, done)

    # ----- AAF / OMF -----
    def _interchange(self) -> None:
        omf = self.command == "file.exportOmf"
        p = self.params
        if omf:
            self.text("title", N_("OMF Title:"), 240)
        else:
            self.check("mixdownVideo", N_("Mixdown video"))
        modes = [("embedded", N_("Encapsulate")), ("separate", N_("Separate Audio"))] if omf else [("embedded", N_("Embed")), ("separate", N_("Separate Files")), ("linked", N_("Link to Media"))]
        self._radio_row("audio", N_("Audio:"), modes)
        formats = [("wav", "Broadcast Wave"), ("aiff", "AIFF")] + ([] if omf else [("mxf", "OP-Atom MXF")])
        self.format_radios = self._radio_row("audioFormat", N_("Audio File Format:"), formats)
        rates = [r for r in self.info.get("sampleRates") or [] if isinstance(r, int)] or [32_000, 44_100, 48_000, 96_000]
        self._radio_row("sampleRate", N_("Sample Rate:"), [(r, str(r)) for r in rates])
        self._radio_row("bitDepth", N_("Bits per Sample:"), [(16, "16"), (24, "24")])
        self.trim = self.check("trimAudio", N_("Trim audio files"))
        row = self._row(N_("Handle Frames:"))
        self.handles = QSpinBox(self)
        self.handles.setObjectName(f"{self.pre}.handles")
        self.handles.setRange(0, 10_000)
        self.handles.setValue(max(0, min(10_000, _int(p.get("handles"), 0))))
        self.handles.valueChanged.connect(lambda v: p.__setitem__("handles", v))
        row.addWidget(self.handles)
        row.addStretch(1)
        self.effects = self.check("renderAudioEffects", N_("Render audio clip effects"))
        self.check("breakoutToMono", N_("Breakout to mono"))
        self._interchange_enabled()

    def _radio_row(self, key: str, label: str, choices: list[tuple[Any, str]]) -> dict[Any, Any]:
        row = self._row(label)
        group = QButtonGroup(self)
        out = {}
        for value, text in choices:
            r = QRadioButton(tr(text), self)
            r.setObjectName(f"{self.pre}.{key}.{value}")
            r.setChecked(self.params.get(key) == value)
            r.toggled.connect(lambda on, v=value, k=key: on and self._set(k, v))
            group.addButton(r)
            row.addWidget(r)
            out[value] = r
        row.addStretch(1)
        return out

    def _changed(self, key: str) -> None:
        if self.command in ("file.exportAaf", "file.exportOmf"):
            self._interchange_enabled()
        else:
            super()._changed(key)

    def _interchange_enabled(self) -> None:
        if not hasattr(self, "effects"):
            return
        p = self.params
        linked = p.get("audio") == "linked"
        for r in self.format_radios.values():
            r.setEnabled(p.get("audio") == "separate")
        self.trim.setEnabled(not linked)
        self.handles.setEnabled(p.get("trimAudio") is True and not linked)
        self.effects.setEnabled(not linked)

    def _interchange_export(self) -> None:
        omf = self.command == "file.exportOmf"
        p = self.params
        ext, label = ("omf", "OMF") if omf else ("aaf", "AAF")
        title = str(p.get("title") or "")
        stem = title.replace("/", "_").replace("\\", "_").replace(":", "_") if title else "Sequence"
        path, _f = QFileDialog.getSaveFileName(self, tr(self.windowTitle()), f"{stem}.{ext}", f"{label} (*.{ext})")
        if not path:
            return
        if not path.lower().endswith(f".{ext}"):
            path += f".{ext}"
        linked = p.get("audio") == "linked"
        params: dict[str, Any] = {
            "path": path,
            "breakoutToMono": p.get("breakoutToMono") is True,
            "audio": p.get("audio"),
            "audioFormat": p.get("audioFormat"),
            "sampleRate": p.get("sampleRate"),
            "bitDepth": p.get("bitDepth"),
            "trimAudio": p.get("trimAudio") is True and not linked,
            "handles": p.get("handles"),
            "renderAudioEffects": p.get("renderAudioEffects") is True and not linked,
        }
        if omf:
            params["title"] = title
        else:
            params["mixdownVideo"] = p.get("mixdownVideo") is True
        self.sent.append(params)

        def done(error: str | None, result: Any = None) -> None:
            if error is not None:
                self.fail(error)
                return
            out = result.get("path") if isinstance(result, dict) else None
            self.status(fmt("Exported {path}", path=str(out or path)))
            self.accept()

        self.run(self.command, params, done)

    # ----- OK -----
    def apply(self) -> None:
        self.error.hide()
        if self.command == "file.exportFrame":
            self._export()
        else:
            self._interchange_export()


__all__ = ["EXPORT_DIALOGS", "ExportSettingsDialog", "clean_name", "frame_path"]
