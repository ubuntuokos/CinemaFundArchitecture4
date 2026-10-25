"""Export window: preset, format, output file, range, video and audio settings, and the export
queue with progress.

Everything is the engine's: presets (``export.presets.list``), formats (``export.formats``),
the settings a choice resolves to (``export.resolve``) and the queue (``export.queue.*``, encoded
by the engine server between requests). The window only collects the choices and shows the
engine's answers; it never computes export settings itself.
"""

from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any

from PySide6.QtCore import Qt, QTimer, Signal
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QMessageBox,
    QProgressBar,
    QPushButton,
    QSpinBox,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, tr

RANGES = (("entire", N_("Entire Sequence")), ("inOut", N_("Sequence In/Out")), ("workArea", N_("Work Area")), ("custom", N_("Custom")))
BITRATE_MODES = (("vbr1Pass", N_("VBR, 1 pass")), ("vbr2Pass", N_("VBR, 2 pass")), ("cbr", N_("CBR")), ("crf", N_("Constant quality (CRF)")))
AUDIO_CODECS = (("auto", N_("Automatic")), ("aac", N_("AAC")), ("pcm", N_("PCM (uncompressed)")), ("flac", N_("FLAC")))
QUEUE_POLL_MS = 500
# engine QueueStatus (crates/engine/src/export_tools.rs): ready, encoding, done, failed, cancelled
FINISHED = {"done", "failed", "cancelled"}
STATUS_LABELS = {"ready": N_("Ready"), "encoding": N_("Encoding"), "done": N_("Done"), "failed": N_("Failed"), "cancelled": N_("Cancelled")}


def default_output_dir(project_path: str | None) -> Path:
    """Next to the project file, else ~/Videos (XDG), else the home directory."""
    if project_path:
        parent = Path(project_path).expanduser().parent
        if parent.is_dir():
            return parent
    videos = Path(os.environ.get("XDG_VIDEOS_DIR") or Path.home() / "Videos").expanduser()
    return videos if videos.is_dir() else Path.home()


# export formats the engine writes as numbered files (`image_sequence_path` in crates/export)
IMAGE_SEQUENCES = frozenset({"png", "tiff", "bmp", "jpg", "tga", "dpx", "exr"})


def existing_outputs(out: Path, fmt: str) -> list[Path]:
    """The files an export to ``out`` would replace: ``out`` itself, or for an image sequence
    the numbered frames ``stem000.ext``, ``stem001.ext``… already there."""
    if fmt not in IMAGE_SEQUENCES:
        return [out] if out.exists() else []
    frame = re.compile(re.escape(out.stem) + r"\d{3,}" + re.escape(out.suffix or ".png"))
    try:
        return sorted(p for p in out.parent.iterdir() if frame.fullmatch(p.name))
    except OSError:
        return []


def with_extension(path: str, ext: str) -> str:
    """``path`` with its extension replaced by ``ext`` (kept when ``ext`` is empty)."""
    if not ext or not path:
        return path
    p = Path(path)
    return str(p.with_suffix("." + ext.lstrip(".")))


def safe_file_name(name: str) -> str:
    cleaned = "".join("_" if c in '/\\\0' else c for c in name).strip().strip(".")
    return cleaned or "export"


def format_eta(seconds: Any) -> str:
    try:
        s = max(0, round(float(seconds)))
    except (TypeError, ValueError, OverflowError):
        return ""
    return f"{s // 3600}:{s // 60 % 60:02d}:{s % 60:02d}" if s >= 3600 else f"{s // 60}:{s % 60:02d}"


class ExportDialog(QDialog):
    """Non-modal: the editor stays usable while the queue encodes."""

    finished_export = Signal(str, bool)  # output path, success

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.export")
        self.setWindowTitle(tr("Export Media"))
        self.setModal(False)
        self.engine = engine
        self._replace_confirmed = ""  # an existing output the user already chose to replace
        self.sequence: dict[str, Any] | None = None
        self.project_path: str | None = None
        self.presets: list[dict[str, Any]] = []
        self.formats: dict[str, dict[str, Any]] = {}
        self._loading = False
        self._known_status: dict[Any, str] = {}
        # settings the user changed since the preset was chosen: only these override the preset
        self.touched: set[str] = set()

        # ----- settings -----
        self.sequence_label = QLabel(self)
        self.preset = QComboBox(self)
        self.preset.setObjectName("export.preset")
        self.format = QComboBox(self)
        self.format.setObjectName("export.format")
        self.path = QLineEdit(self)
        self.path.setObjectName("export.path")
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName("export.browse")
        browse.clicked.connect(self._browse)
        path_row = QHBoxLayout()
        path_row.addWidget(self.path, 1)
        path_row.addWidget(browse)

        self.range = QComboBox(self)
        self.range.setObjectName("export.range")
        for key, label in RANGES:
            self.range.addItem(tr(label), key)
        self.start = QDoubleSpinBox(self)
        self.end = QDoubleSpinBox(self)
        for spin, name in ((self.start, "start"), (self.end, "end")):
            spin.setObjectName(f"export.{name}Seconds")
            spin.setDecimals(3)
            spin.setRange(0.0, 1e7)
            spin.setSuffix(" s")
        range_row = QHBoxLayout()
        range_row.addWidget(self.range, 1)
        range_row.addWidget(self.start)
        range_row.addWidget(QLabel("–", self))
        range_row.addWidget(self.end)

        self.width_spin = QSpinBox(self)
        self.height_spin = QSpinBox(self)
        for spin, name in ((self.width_spin, "width"), (self.height_spin, "height")):
            spin.setObjectName(f"export.{name}")
            spin.setRange(0, 16384)
            spin.setSpecialValueText(tr("Match sequence"))
        size_row = QHBoxLayout()
        size_row.addWidget(self.width_spin)
        size_row.addWidget(QLabel("×", self))
        size_row.addWidget(self.height_spin)
        self.bitrate_mode = QComboBox(self)
        self.bitrate_mode.setObjectName("export.bitrateMode")
        for key, label in BITRATE_MODES:
            self.bitrate_mode.addItem(tr(label), key)
        self.bitrate = QSpinBox(self)
        self.bitrate.setObjectName("export.bitrateKbps")
        self.bitrate.setRange(0, 1_000_000)
        self.bitrate.setSuffix(" kbps")
        self.bitrate.setSpecialValueText(tr("From the preset"))
        self.hardware = QCheckBox(tr("Use hardware encoding when available"), self)
        self.hardware.setObjectName("export.hardwareEncoding")
        self.burn = QCheckBox(tr("Burn in captions"), self)
        self.burn.setObjectName("export.burnCaptions")
        self.audio = QCheckBox(tr("Export audio"), self)
        self.audio.setObjectName("export.audio")
        self.audio.setChecked(True)  # sound is part of an export unless the user leaves it out
        self.audio_codec = QComboBox(self)
        self.audio_codec.setObjectName("export.audioCodec")
        for key, label in AUDIO_CODECS:
            self.audio_codec.addItem(tr(label), key)

        out = QGroupBox(tr("Output"), self)
        f = QFormLayout(out)
        f.addRow(tr("Sequence"), self.sequence_label)
        f.addRow(tr("Preset"), self.preset)
        f.addRow(tr("Format"), self.format)
        f.addRow(tr("Output file"), path_row)
        f.addRow(tr("Range"), range_row)
        video = QGroupBox(tr("Video"), self)
        fv = QFormLayout(video)
        fv.addRow(tr("Frame size"), size_row)
        fv.addRow(tr("Bitrate mode"), self.bitrate_mode)
        fv.addRow(tr("Target bitrate"), self.bitrate)
        fv.addRow(self.hardware)
        fv.addRow(self.burn)
        audio = QGroupBox(tr("Audio"), self)
        fa = QFormLayout(audio)
        fa.addRow(self.audio)
        fa.addRow(tr("Audio codec"), self.audio_codec)

        self.summary = QLabel(self)
        self.summary.setObjectName("export.summary")
        self.summary.setWordWrap(True)
        self.summary.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)

        self.export_button = QPushButton(tr("Export"), self)
        self.export_button.setObjectName("export.start")
        self.export_button.setDefault(True)
        self.export_button.clicked.connect(lambda: self._enqueue(start=True))
        self.queue_button = QPushButton(tr("Add to Queue"), self)
        self.queue_button.setObjectName("export.addToQueue")
        self.queue_button.clicked.connect(lambda: self._enqueue(start=False))
        close = QPushButton(tr("Close"), self)
        close.clicked.connect(self.hide)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        buttons.addWidget(self.queue_button)
        buttons.addWidget(self.export_button)
        buttons.addWidget(close)

        # ----- queue -----
        self.table = QTableWidget(0, 6, self)
        self.table.setObjectName("export.queue")
        self.table.setHorizontalHeaderLabels(
            [tr("Sequence"), tr("Output file"), tr("Format"), tr("Status"), tr("Progress"), tr("Remaining")]
        )
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.table.verticalHeader().setVisible(False)
        self.table.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        queue_buttons = QHBoxLayout()
        for name, text, slot in (
            ("startQueue", N_("Start Queue"), lambda: self._queue_command("export.queue.start", {})),
            ("stopQueue", N_("Stop Queue"), lambda: self._queue_command("export.queue.stop", {})),
            ("cancel", N_("Cancel Selected"), lambda: self._selected_command("export.queue.cancel")),
            ("retry", N_("Retry"), lambda: self._selected_command("export.queue.retry", {"start": True})),
            ("remove", N_("Remove"), lambda: self._selected_command("export.queue.remove")),
            ("clear", N_("Clear Finished"), lambda: self._queue_command("export.queue.clear", {})),
        ):
            b = QPushButton(tr(text), self)
            b.setObjectName(f"export.queue.{name}")
            b.clicked.connect(slot)
            queue_buttons.addWidget(b)
        queue_buttons.addStretch(1)
        queue_box = QGroupBox(tr("Export Queue"), self)
        qv = QVBoxLayout(queue_box)
        qv.addWidget(self.table)
        qv.addLayout(queue_buttons)

        top = QHBoxLayout()
        left = QVBoxLayout()
        left.addWidget(out)
        left.addWidget(video)
        left.addWidget(audio)
        left.addStretch(1)
        top.addLayout(left, 1)
        layout = QVBoxLayout(self)
        layout.addLayout(top)
        layout.addWidget(self.summary)
        layout.addLayout(buttons)
        layout.addWidget(queue_box, 1)
        self.resize(820, 760)

        self._resolve_timer = QTimer(self, singleShot=True, interval=150)
        self._resolve_timer.timeout.connect(self._resolve)
        self._poll = QTimer(self, interval=QUEUE_POLL_MS)
        self._poll.timeout.connect(self.refresh_queue)
        self.preset.currentIndexChanged.connect(self._preset_changed)
        self.format.currentIndexChanged.connect(self._format_changed)
        self.range.currentIndexChanged.connect(self._range_changed)
        for w in (self.start, self.end):
            w.valueChanged.connect(self._changed)
        for key, w in (("size", self.width_spin), ("size", self.height_spin), ("bitrate", self.bitrate)):
            w.valueChanged.connect(lambda _v, k=key: self._touch(k))
        for key, c in (("bitrateMode", self.bitrate_mode), ("audioCodec", self.audio_codec)):
            c.currentIndexChanged.connect(lambda _i, k=key: self._touch(k))
        for key, b in (("hardware", self.hardware), ("burn", self.burn), ("audio", self.audio)):
            b.toggled.connect(lambda _on, k=key: self._touch(k))
        self.path.textChanged.connect(self._changed)
        self._range_changed()

    # ----- opening -----
    def open_for(self, sequence: dict[str, Any] | None, project_path: str | None) -> None:
        self.sequence = sequence
        self.project_path = project_path
        name = str((sequence or {}).get("name") or tr("No sequence"))
        self.sequence_label.setText(name)
        self.export_button.setEnabled(sequence is not None)
        self.queue_button.setEnabled(sequence is not None)
        if sequence is not None:
            duration = int(sequence.get("duration") or 0) / 254_016_000_000
            self.end.setValue(duration)
        if not self.presets:
            self._load()
        elif not self.path.text():
            self._default_path()
        self.show()
        self.raise_()
        self.activateWindow()
        self.refresh_queue()
        self._poll.start()

    def _load(self) -> None:
        self._loading = True
        self.engine.execute("export.formats", {}, on_reply=self._on_formats, on_error=self._error)
        self.engine.execute("export.presets.list", {}, on_reply=self._on_presets, on_error=self._error)

    def _on_formats(self, result: Any, _b: bytes | None) -> None:
        self.formats = {}
        self.format.blockSignals(True)
        self.format.clear()
        for f in result if isinstance(result, list) else []:
            if not isinstance(f, dict) or not f.get("id"):
                continue
            self.formats[str(f["id"])] = f
            label = str(f.get("label") or f["id"])
            if not f.get("available", True):
                label += f"  ({tr('not available')})"
            self.format.addItem(label, str(f["id"]))
            item = self.format.model().item(self.format.count() - 1)  # type: ignore[attr-defined]
            if item is not None and not f.get("available", True):
                item.setEnabled(False)
        self.format.blockSignals(False)

    def _on_presets(self, result: Any, _b: bytes | None) -> None:
        data = result if isinstance(result, dict) else {}
        self.presets = [p for p in data.get("presets") or [] if isinstance(p, dict) and p.get("name")]
        default = data.get("default")
        self.preset.blockSignals(True)
        self.preset.clear()
        category = None
        for p in self.presets:
            if p.get("category") != category:
                category = p.get("category")
                if self.preset.count():
                    self.preset.insertSeparator(self.preset.count())
            self.preset.addItem(str(p["name"]), str(p["name"]))
            self.preset.setItemData(self.preset.count() - 1, str(p.get("description") or ""), Qt.ItemDataRole.ToolTipRole)
        index = self.preset.findData(default)
        self.preset.setCurrentIndex(max(0, index))
        self.preset.blockSignals(False)
        self._loading = False
        self._preset_changed()

    # ----- choices -----
    def _preset_changed(self) -> None:
        name = self.preset.currentData()
        preset = next((p for p in self.presets if p.get("name") == name), None)
        if preset is None:
            return
        self._loading = True
        index = self.format.findData(preset.get("format"))
        if index >= 0:
            self.format.setCurrentIndex(index)
        size = preset.get("frameSize")
        if isinstance(size, list) and len(size) == 2:
            self.width_spin.setValue(int(size[0]))
            self.height_spin.setValue(int(size[1]))
        else:
            self.width_spin.setValue(0)
            self.height_spin.setValue(0)
        self.bitrate.setValue(0)
        self.touched.clear()
        self._loading = False
        self._default_path()
        self._changed()

    def _format_changed(self) -> None:
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        self.audio.setEnabled(bool(fmt.get("audio", True)))
        self.audio_codec.setEnabled(bool(fmt.get("audio", True)))
        self.bitrate_mode.setEnabled(bool(fmt.get("video", True)) and not fmt.get("imageSequence"))
        self.bitrate.setEnabled(bool(fmt.get("video", True)) and not fmt.get("imageSequence"))
        if self.path.text():
            self.path.setText(with_extension(self.path.text(), str(fmt.get("extension") or "")))
        self._changed()

    def _range_changed(self) -> None:
        custom = self.range.currentData() == "custom"
        self.start.setEnabled(custom)
        self.end.setEnabled(custom)
        self._changed()

    def _default_path(self) -> None:
        if self.path.text():
            return
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        name = safe_file_name(str((self.sequence or {}).get("name") or "export"))
        base = default_output_dir(self.project_path) / name
        self.path.setText(with_extension(str(base), str(fmt.get("extension") or "mp4")))

    def _browse(self) -> None:
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        ext = str(fmt.get("extension") or "")
        pattern = f"{fmt.get('label') or ext} (*.{ext})" if ext else tr("All files") + " (*)"
        path, _ = QFileDialog.getSaveFileName(self, tr("Output file"), self.path.text(), pattern)
        if path:
            # the native dialog has asked about replacing an existing file
            self._replace_confirmed = path
            self.path.setText(with_extension(path, ext) if ext and not path.endswith("." + ext) else path)

    def _changed(self) -> None:
        if not self._loading:
            self._resolve_timer.start()

    def _touch(self, key: str) -> None:
        if not self._loading:
            self.touched.add(key)
        self._changed()

    def params(self) -> dict[str, Any]:
        """The export as engine parameters: the preset, overridden by the choices made here."""
        p: dict[str, Any] = {}
        if self.preset.currentData():
            p["preset"] = self.preset.currentData()
        if self.format.currentData():
            p["format"] = self.format.currentData()
        if self.path.text().strip():
            p["path"] = os.path.expanduser(self.path.text().strip())
        p["range"] = self.range.currentData()
        if p["range"] == "custom":
            p["startSeconds"] = self.start.value()
            p["endSeconds"] = self.end.value()
        # 0 × 0 = the preset's size (or the sequence's); the fields show the preset's own size
        if self.width_spin.value() and self.height_spin.value():
            p["width"] = self.width_spin.value()
            p["height"] = self.height_spin.value()
        if self.bitrate_mode.isEnabled():
            if "bitrateMode" in self.touched:
                p["bitrateMode"] = self.bitrate_mode.currentData()
            if self.bitrate.value():
                p["bitrateKbps"] = self.bitrate.value()
        if "hardware" in self.touched:
            p["hardwareEncoding"] = "auto" if self.hardware.isChecked() else "off"
        if "burn" in self.touched:
            p["burnCaptions"] = self.burn.isChecked()
        if self.audio.isEnabled():
            if "audio" in self.touched:
                p["audio"] = self.audio.isChecked()
            if "audioCodec" in self.touched:
                p["audioCodec"] = self.audio_codec.currentData()
        if self.sequence is not None and self.sequence.get("id") is not None:
            p["sequence"] = self.sequence["id"]
        return p

    # ----- engine answers -----
    def _resolve(self) -> None:
        if self.sequence is None:
            self.summary.setText(tr("No sequence"))
            return
        self.engine.execute("export.resolve", self.params(), on_reply=self._on_resolved, on_error=self._summary_error)

    def _on_resolved(self, result: Any, _b: bytes | None) -> None:
        r = result if isinstance(result, dict) else {}
        out = r.get("output") or {}
        rng = r.get("range") or {}
        parts = []
        if out.get("width") and out.get("height"):
            parts.append(f"{out['width']}×{out['height']}")
        if out.get("fps"):
            parts.append(f"{out['fps']} fps")
        if out.get("targetKbps"):
            parts.append(f"{out['targetKbps']} kbps")
        if out.get("sampleRate"):
            parts.append(f"{out.get('channels', 2)} ch {out['sampleRate']} Hz")
        if rng.get("seconds") is not None:
            parts.append(f"{float(rng['seconds']):.2f} s")
        if r.get("frames"):
            parts.append(f"{r['frames']} {tr('frames')}")
        self.summary.setText(" · ".join(parts))
        self.summary.setStyleSheet("")
        self.export_button.setEnabled(True)
        self.queue_button.setEnabled(True)

    def _summary_error(self, message: str) -> None:
        self.summary.setText(message)
        self.summary.setStyleSheet("color: #d04040")
        self.export_button.setEnabled(False)
        self.queue_button.setEnabled(False)

    def _error(self, message: str) -> None:
        self.summary.setText(message)
        self.summary.setStyleSheet("color: #d04040")

    # ----- queue -----
    def _enqueue(self, start: bool) -> None:
        params = self.params()
        if not params.get("path"):
            self._summary_error(tr("Choose an output file."))
            return
        # the engine completes the path (an extension, a folder's sequence-named file): ask it
        # which file the export will write before checking whether that one exists
        self.engine.execute(
            "export.resolve",
            params,
            on_reply=lambda r, _b: self._enqueue_resolved(params, start, r),
            on_error=self._summary_error,
        )

    def _enqueue_resolved(self, params: dict[str, Any], start: bool, resolved: Any) -> None:
        settings = resolved.get("settings") if isinstance(resolved, dict) else None
        target = settings.get("path") if isinstance(settings, dict) else None
        out = Path(target if isinstance(target, str) and target else params["path"]).expanduser()
        fmt = str(settings.get("format") or "") if isinstance(settings, dict) else ""
        try:
            out.parent.mkdir(parents=True, exist_ok=True)
            existing = existing_outputs(out, fmt)
        except OSError as e:
            self._summary_error(tr("Cannot create the output folder: {error}").replace("{error}", str(e)))
            return
        if existing and str(out) != str(Path(self._replace_confirmed).expanduser()) and not self._confirm_replace(out, existing[0]):
            return
        params["start"] = start
        self.engine.execute("export.queue.add", params, on_reply=lambda r, b: self._on_queue(r, b), on_error=self._summary_error)
        self._poll.start()

    def _confirm_replace(self, out: Path, existing: Path | None = None) -> bool:
        """The export truncates its output when it starts: ask before replacing a file."""
        box = QMessageBox(
            QMessageBox.Icon.Warning,
            tr("Export Media"),
            tr("'{name}' already exists. Replace it?").replace("{name}", (existing or out).name),
            parent=self,
        )
        box.setObjectName("dialog.export.replace")
        box.setStandardButtons(QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel)
        box.setDefaultButton(QMessageBox.StandardButton.Cancel)
        if box.exec() != QMessageBox.StandardButton.Yes:
            return False
        self._replace_confirmed = str(out)
        return True

    def _queue_command(self, command: str, params: dict[str, Any]) -> None:
        self.engine.execute(command, params, on_reply=lambda _r, _b: self.refresh_queue(), on_error=self._error)

    def _selected_command(self, command: str, extra: dict[str, Any] | None = None) -> None:
        for row in sorted({i.row() for i in self.table.selectedIndexes()}):
            item = self.table.item(row, 0)
            if item is not None and item.data(Qt.ItemDataRole.UserRole) is not None:
                self._queue_command(command, {"id": item.data(Qt.ItemDataRole.UserRole), **(extra or {})})

    def refresh_queue(self) -> None:
        if self.engine.running:
            self.engine.execute("export.queue.list", {}, on_reply=self._on_queue, on_error=lambda _m: None)

    def _on_queue(self, result: Any, _b: bytes | None) -> None:
        data = result if isinstance(result, dict) else {}
        items = [i for i in data.get("items") or [] if isinstance(i, dict)]
        self.table.setRowCount(len(items))
        for row, it in enumerate(items):
            status = str(it.get("status") or "").lower()
            texts = [
                str(it.get("sequenceName") or ""),
                str(it.get("path") or ""),
                str(it.get("format") or ""),
                tr(STATUS_LABELS.get(status, status)) + (f" — {it['error']}" if it.get("error") else ""),
                "",
                format_eta(it.get("etaSeconds")) if status not in FINISHED else "",
            ]
            for col, text in enumerate(texts):
                cell = self.table.item(row, col)
                if cell is None:  # reuse the row's cells: re-adding an owned item is refused
                    cell = QTableWidgetItem()
                    self.table.setItem(row, col, cell)
                cell.setText(text)
                if col == 0:
                    cell.setData(Qt.ItemDataRole.UserRole, it.get("id"))
                if col == 3:
                    cell.setToolTip(str(it.get("statusText") or ""))
            bar = self.table.cellWidget(row, 4)
            if not isinstance(bar, QProgressBar):
                bar = QProgressBar(self.table)
                bar.setRange(0, 1000)
                self.table.setCellWidget(row, 4, bar)
            try:
                bar.setValue(int(max(0.0, min(1.0, float(it.get("progress") or 0.0))) * 1000))
            except (TypeError, ValueError):
                bar.setValue(0)
            if status == "done":
                bar.setValue(1000)
            key = it.get("id")
            if self._known_status.get(key) != status and status in FINISHED and key in self._known_status:
                # it finished while we watched it
                self.finished_export.emit(str(it.get("path") or ""), status == "done")
            self._known_status[key] = status
        running = bool(data.get("running")) or any(str(i.get("status") or "").lower() == "encoding" for i in items)
        if not running and not self.isVisible():
            self._poll.stop()

    def hideEvent(self, e) -> None:
        super().hideEvent(e)
        # keep polling while something encodes, so the editor hears when it is done
        self.refresh_queue()
