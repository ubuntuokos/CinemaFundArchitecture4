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
# MPEG-2 containers (`Mpeg2Container` in crates/export): id, label, file extension
MPEG2_CONTAINERS = (
    ("ps", N_("Program Stream (.mpg, LPCM audio)"), "mpg"),
    ("ts", N_("Transport Stream (.ts, AAC audio)"), "ts"),
    ("m2ts", N_("BDAV Transport Stream (.m2ts, AAC audio)"), "m2ts"),
    ("es", N_("Elementary Stream (.m2v, no audio)"), "m2v"),
)
# DCP choices (`DcpResolution`, `DcpAspect`, `DCP_CONTENT_KINDS` in crates/export): id, label
DCP_RESOLUTIONS = (("2k", N_("2K")), ("4k", N_("4K")))
DCP_ASPECTS = (("flat", N_("Flat (1.85:1)")), ("scope", N_("Scope (2.39:1)")), ("full", N_("Full container (1.90:1)")))
DCP_KINDS = (
    ("feature", N_("Feature")),
    ("trailer", N_("Trailer")),
    ("teaser", N_("Teaser")),
    ("short", N_("Short")),
    ("advertisement", N_("Advertisement")),
    ("test", N_("Test")),
    ("transitional", N_("Transitional")),
    ("rating", N_("Rating")),
    ("policy", N_("Policy")),
    ("psa", N_("Public service announcement")),
)
# DCP immersive audio (`DcpImmersive` in crates/export): id, label
DCP_IMMERSIVE = (
    ("off", N_("Off (5.1 only)")),
    ("bed", N_("IAB 7.1 DS bed from the 5.1 mix")),
    ("file", N_("IAB from a file (IAB MXF, .iab, or Dolby Atmos TrueHD)")),
)
# what channels 1-6 of an immersive DCP's main sound carry; ST 429-19:2023 7.1.2 wants none (#348)
DCP_IAB_FALLBACK = (
    ("none", N_("None: FSK sync only (SMPTE ST 429-19)")),
    ("5.1", N_("5.1: the IAB rendered (not ST 429-19)")),
    ("mix", N_("5.1: the timeline mix (not ST 429-19)")),
)
AUDIO_CODECS = (("auto", N_("Automatic")), ("aac", N_("AAC")), ("pcm", N_("PCM (uncompressed)")), ("flac", N_("FLAC")), ("opus", N_("Opus")))
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
    """The files an export to ``out`` would replace: ``out`` itself (and for MXF OP-Atom its
    ``stem_A1.mxf``… audio files), or for an image sequence the numbered frames ``stem000.ext``,
    ``stem001.ext``… already there. Raises ``OSError`` when
    the folder cannot be listed: frames there may still be overwritten, so that is not "none"."""
    if fmt == "mxf-opatom":
        # one file per audio channel beside the picture: stem_A1.mxf… (`opatom_audio_paths`)
        audio = re.compile(re.escape(out.stem) + r"_A\d+\.mxf")
        tracks = sorted(p for p in out.parent.iterdir() if audio.fullmatch(p.name))
        return ([out] if out.exists() else []) + tracks
    if fmt not in IMAGE_SEQUENCES:
        return [out] if out.exists() else []
    frame = re.compile(re.escape(out.stem) + r"\d{3,}" + re.escape(out.suffix or ".png"))
    return sorted(p for p in out.parent.iterdir() if frame.fullmatch(p.name))


def sidecar_output(target: str, kind: Any) -> Path | None:
    """The caption sidecar an export to ``target`` writes (``captionSidecar``), named as the
    engine names it (``write_sidecar``): the output path up to its last dot, then ``vtt`` for WebVTT
    and otherwise ``kind`` exactly as given (``SRT`` writes ``cut.SRT``)."""
    if not isinstance(kind, str) or kind.lower() in ("", "none"):
        return None
    ext = "vtt" if kind.lstrip(".").lower() in ("vtt", "webvtt") else kind
    return Path(f"{target.rsplit('.', 1)[0] if '.' in target else target}.{ext}").expanduser()


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
        # existing files the user already chose to replace (the native save dialog asks only about
        # the file it returns, never about an image sequence's numbered frames)
        self._replace_confirmed: set[str] = set()
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
        self.container = QComboBox(self)
        self.container.setObjectName("export.mpeg2Container")
        for key, label, _ext in MPEG2_CONTAINERS:
            self.container.addItem(tr(label), key)
        self.container.setEnabled(False)  # MPEG-2 only
        # DCP only: resolution, container and content kind (the package is a folder)
        self.dcp_resolution = QComboBox(self)
        self.dcp_resolution.setObjectName("export.dcpResolution")
        for key, label in DCP_RESOLUTIONS:
            self.dcp_resolution.addItem(tr(label), key)
        self.dcp_aspect = QComboBox(self)
        self.dcp_aspect.setObjectName("export.dcpAspect")
        for key, label in DCP_ASPECTS:
            self.dcp_aspect.addItem(tr(label), key)
        self.dcp_kind = QComboBox(self)
        self.dcp_kind.setObjectName("export.dcpContentKind")
        for key, label in DCP_KINDS:
            self.dcp_kind.addItem(tr(label), key)
        for w in (self.dcp_resolution, self.dcp_aspect, self.dcp_kind):
            w.setEnabled(False)
        # DCP only: immersive audio, an IAB track file beside the 16-channel main sound (#343)
        self.dcp_immersive = QComboBox(self)
        self.dcp_immersive.setObjectName("export.dcpImmersive")
        for key, label in DCP_IMMERSIVE:
            self.dcp_immersive.addItem(tr(label), key)
        self.dcp_iab_source = QLineEdit(self)
        self.dcp_iab_source.setObjectName("export.dcpIabSource")
        self.dcp_iab_source.setPlaceholderText(tr("IAB MXF, .iab, or Dolby Atmos TrueHD (.thd, .mkv, .mp4, .m2ts)"))
        self.dcp_iab_browse = QPushButton(tr("Browse…"), self)
        self.dcp_iab_browse.setObjectName("export.dcpIabBrowse")
        self.dcp_iab_browse.clicked.connect(self._browse_iab)
        iab_row = QHBoxLayout()
        iab_row.addWidget(self.dcp_iab_source, 1)
        iab_row.addWidget(self.dcp_iab_browse)
        self.dcp_iab_info = QLabel(self)
        self.dcp_iab_info.setObjectName("export.dcpIabInfo")
        self.dcp_iab_info.setWordWrap(True)
        self.dcp_iab_fallback = QComboBox(self)
        self.dcp_iab_fallback.setObjectName("export.dcpIabFallback")
        for key, label in DCP_IAB_FALLBACK:
            self.dcp_iab_fallback.addItem(tr(label), key)
        self._iab_probed = ""
        for w in (self.dcp_immersive, self.dcp_iab_source, self.dcp_iab_browse, self.dcp_iab_fallback):
            w.setEnabled(False)
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
        # MPEG-2 only: GOP length, B pictures, constant quantiser, 4:2:2 Profile
        self.gop = QSpinBox(self)
        self.gop.setObjectName("export.keyframeDistance")
        self.gop.setRange(0, 1000)
        self.gop.setSpecialValueText(tr("From the preset"))
        self.gop.setSuffix(" " + tr("frames"))
        self.b_frames = QSpinBox(self)
        self.b_frames.setObjectName("export.mpeg2BFrames")
        self.b_frames.setRange(0, 7)
        self.b_frames.setValue(2)
        self.b_frames.setSpecialValueText(tr("None (I and P only)"))
        self.quantizer = QSpinBox(self)
        self.quantizer.setObjectName("export.mpeg2Quantizer")
        self.quantizer.setRange(0, 31)
        self.quantizer.setSpecialValueText(tr("Off (target bitrate)"))
        self.chroma = QComboBox(self)
        self.chroma.setObjectName("export.mpeg2Chroma")
        self.chroma.addItem(tr("4:2:0 (Main Profile)"), "420")
        self.chroma.addItem(tr("4:2:2 (4:2:2 Profile)"), "422")
        for w in (self.gop, self.b_frames, self.quantizer, self.chroma):
            w.setEnabled(False)
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
        # AAC and Opus: the engine clamps Opus to 6..510 kbps
        self.audio_bitrate = QSpinBox(self)
        self.audio_bitrate.setObjectName("export.audioBitrateKbps")
        self.audio_bitrate.setRange(0, 1_000)
        self.audio_bitrate.setSuffix(" kbps")
        self.audio_bitrate.setSpecialValueText(tr("From the preset"))

        out = QGroupBox(tr("Output"), self)
        f = QFormLayout(out)
        f.addRow(tr("Sequence"), self.sequence_label)
        f.addRow(tr("Preset"), self.preset)
        f.addRow(tr("Format"), self.format)
        f.addRow(tr("Container"), self.container)
        f.addRow(tr("DCP resolution"), self.dcp_resolution)
        f.addRow(tr("DCP container"), self.dcp_aspect)
        f.addRow(tr("Content kind"), self.dcp_kind)
        f.addRow(tr("Output file"), path_row)
        f.addRow(tr("Range"), range_row)
        video = QGroupBox(tr("Video"), self)
        fv = QFormLayout(video)
        fv.addRow(tr("Frame size"), size_row)
        fv.addRow(tr("Bitrate mode"), self.bitrate_mode)
        fv.addRow(tr("Target bitrate"), self.bitrate)
        fv.addRow(tr("Keyframe distance"), self.gop)
        fv.addRow(tr("B pictures"), self.b_frames)
        fv.addRow(tr("Constant quantiser"), self.quantizer)
        fv.addRow(tr("Chroma"), self.chroma)
        fv.addRow(self.hardware)
        fv.addRow(self.burn)
        audio = QGroupBox(tr("Audio"), self)
        fa = QFormLayout(audio)
        fa.addRow(self.audio)
        fa.addRow(tr("Audio codec"), self.audio_codec)
        fa.addRow(tr("Audio bitrate"), self.audio_bitrate)
        fa.addRow(tr("Immersive audio"), self.dcp_immersive)
        fa.addRow(tr("IAB source"), iab_row)
        fa.addRow("", self.dcp_iab_info)
        fa.addRow(tr("Main sound channels 1-6"), self.dcp_iab_fallback)

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
        self.container.currentIndexChanged.connect(self._container_changed)
        self.range.currentIndexChanged.connect(self._range_changed)
        for w in (self.start, self.end):
            w.valueChanged.connect(self._changed)
        for key, w in (("size", self.width_spin), ("size", self.height_spin), ("bitrate", self.bitrate), ("audioBitrate", self.audio_bitrate), ("gop", self.gop), ("bFrames", self.b_frames), ("quantizer", self.quantizer)):
            w.valueChanged.connect(lambda _v, k=key: self._touch(k))
        for c in (self.dcp_resolution, self.dcp_aspect, self.dcp_kind):
            c.currentIndexChanged.connect(self._changed)
        self.dcp_immersive.currentIndexChanged.connect(self._immersive_changed)
        self.dcp_iab_source.editingFinished.connect(self._probe_iab)
        self.dcp_iab_source.textChanged.connect(self._changed)
        self.dcp_iab_fallback.currentIndexChanged.connect(self._changed)
        for key, c in (("bitrateMode", self.bitrate_mode), ("audioCodec", self.audio_codec), ("chroma", self.chroma)):
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
        self.audio_bitrate.setValue(0)
        self.gop.setValue(0)
        self.b_frames.setValue(2)
        self.quantizer.setValue(0)
        self.chroma.setCurrentIndex(0)
        index = self.container.findData(preset.get("mpeg2Container") or "ps")
        self.container.setCurrentIndex(max(0, index))
        self.dcp_resolution.setCurrentIndex(max(0, self.dcp_resolution.findData(preset.get("dcpResolution") or "2k")))
        self.dcp_aspect.setCurrentIndex(max(0, self.dcp_aspect.findData(preset.get("dcpAspect") or "flat")))
        self.dcp_kind.setCurrentIndex(0)
        self.dcp_immersive.setCurrentIndex(max(0, self.dcp_immersive.findData(preset.get("dcpImmersive") or "off")))
        self.dcp_iab_source.setText(str(preset.get("dcpIabSource") or ""))
        self.dcp_iab_fallback.setCurrentIndex(max(0, self.dcp_iab_fallback.findData(preset.get("dcpIabFallback") or "none")))
        self.touched.clear()
        self._loading = False
        self._default_path()
        self._changed()

    def _format_changed(self) -> None:
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        self.audio.setEnabled(bool(fmt.get("audio", True)))
        self.audio_codec.setEnabled(bool(fmt.get("audio", True)))
        self.audio_bitrate.setEnabled(bool(fmt.get("audio", True)))
        self.bitrate_mode.setEnabled(bool(fmt.get("video", True)) and not fmt.get("imageSequence"))
        self.bitrate.setEnabled(bool(fmt.get("video", True)) and not fmt.get("imageSequence"))
        mpeg2 = self.format.currentData() == "mpeg2"
        for w in (self.container, self.gop, self.b_frames, self.quantizer, self.chroma):
            w.setEnabled(mpeg2)
        dcp = self._dcp()
        for w in (self.dcp_resolution, self.dcp_aspect, self.dcp_kind, self.dcp_immersive):
            w.setEnabled(dcp)
        self._immersive_changed()
        if dcp:
            # fixed by the DCI specifications: container size, J2K data rate, 5.1 24-bit PCM
            for w in (self.width_spin, self.height_spin, self.bitrate_mode, self.bitrate, self.audio_codec, self.audio_bitrate):
                w.setEnabled(False)
        else:
            self.width_spin.setEnabled(True)
            self.height_spin.setEnabled(True)
        self._container_changed()

    def _immersive_changed(self) -> None:
        """The IAB source row is for ``file`` only; the fallback for any immersive DCP."""
        mode = self.dcp_immersive.currentData() if self.dcp_immersive.isEnabled() else "off"
        for w in (self.dcp_iab_source, self.dcp_iab_browse):
            w.setEnabled(mode == "file")
        self.dcp_iab_fallback.setEnabled(mode in ("bed", "file"))
        if mode == "bed":
            self.dcp_iab_info.setText(tr("The 5.1 mix becomes a 7.1 DS bed (IAB Profile 1); channel 14 of the main sound carries the FSK sync signal."))
        elif mode == "file":
            self._probe_iab()
        else:
            self.dcp_iab_info.setText("")
        self._changed()

    def _browse_iab(self) -> None:
        path, _ = QFileDialog.getOpenFileName(
            self, tr("IAB source"), self.dcp_iab_source.text(), tr("Immersive audio") + " (*.mxf *.iab *.thd *.mlp *.mkv *.mka *.mp4 *.m4a *.mov *.m2ts *.mts *.ts);;" + tr("All files") + " (*)"
        )
        if path:
            self.dcp_iab_source.setText(path)
            self._probe_iab()

    def _probe_iab(self) -> None:
        """Ask the engine what the IAB source holds (``export.iabInfo``)."""
        path = os.path.expanduser(self.dcp_iab_source.text().strip())
        if not path:
            self._iab_probed = ""
            self.dcp_iab_info.setText(tr("Choose an IAB track file (IMF or DCP), a raw IAB stream, or Dolby Atmos TrueHD: a .thd stream or an MKV, MP4 or M2TS file with an Atmos track."))
            return
        if path == self._iab_probed:
            return
        self._iab_probed = path
        self.dcp_iab_info.setText(tr("Reading the IAB source…"))
        self.engine.execute(
            "export.iabInfo", {"path": path},
            on_reply=lambda r, _b, p=path: self._on_iab_info(p, r),
            on_error=lambda m, p=path: self._on_iab_error(p, m),
        )

    def _on_iab_info(self, path: str, result: Any) -> None:
        if path != self._iab_probed:
            return
        r = result if isinstance(result, dict) else {}
        if r.get("container") == "truehd":
            # a Dolby Atmos TrueHD stream: its objects become IAB objects at the DCP's frame rate (#347)
            text = tr("Dolby Atmos TrueHD, {seconds} s, {rate} Hz; up to {beds} bed channels and {objects} objects, made into IAB objects").format(
                seconds=round(float(r.get("seconds") or 0.0), 1), rate=r.get("sampleRate", 0), beds=r.get("maxBedChannels", 0), objects=r.get("maxObjects", 0)
            )
            notes = [str(n) for n in r.get("profileNotes") or []]
            if notes:
                text += "\n" + tr("Beyond IAB Profile 1: {notes}").format(notes="; ".join(notes))
            self.dcp_iab_info.setText(text)
            return
        rate = r.get("frameRate") or [0, 1]
        fps = f"{rate[0]}" if len(rate) == 2 and rate[1] == 1 else f"{rate[0]}/{rate[1] if len(rate) == 2 else 1}"
        text = tr("{frames} frames, {fps} fps, {rate} Hz; up to {beds} bed channels and {objects} objects").format(
            frames=r.get("frames", 0), fps=fps, rate=r.get("sampleRate", 0), beds=r.get("maxBedChannels", 0), objects=r.get("maxObjects", 0)
        )
        if r.get("needsConform"):
            text += "\n" + tr("Some frames are re-coded for the DCP (DLC, 24 bit).")
        notes = [str(n) for n in r.get("profileNotes") or []]
        if notes:
            text += "\n" + tr("Beyond IAB Profile 1: {notes}").format(notes="; ".join(notes))
        self.dcp_iab_info.setText(text)

    def _on_iab_error(self, path: str, message: Any) -> None:
        if path == self._iab_probed:
            self.dcp_iab_info.setText(tr("Not a usable IAB source: {error}").format(error=message))

    def _dcp(self) -> bool:
        """A DCP: the output is a folder."""
        return self.format.currentData() == "dcp"

    def _extension(self) -> str:
        """The output file's extension: the MPEG-2 container's, else the format's."""
        if self.container.isEnabled():
            return next(ext for key, _label, ext in MPEG2_CONTAINERS if key == self.container.currentData())
        return str(self.formats.get(str(self.format.currentData() or ""), {}).get("extension") or "")

    def _container_changed(self) -> None:
        mpeg2 = self.container.isEnabled()
        if mpeg2:
            # the elementary stream has no audio
            has_audio = self.container.currentData() != "es"
            self.audio.setEnabled(has_audio)
            self.audio_codec.setEnabled(False)  # LPCM in program streams, AAC in transport streams
        if self.path.text():
            if self._dcp():
                self.path.setText(str(Path(self.path.text()).with_suffix("")))
            else:
                self.path.setText(with_extension(self.path.text(), self._extension()))
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
        if self._dcp():
            self.path.setText(str(base))
            return
        self.path.setText(with_extension(str(base), self._extension() or str(fmt.get("extension") or "mp4")))

    def _browse(self) -> None:
        if self._dcp():
            # the package folder is made inside the chosen one
            parent = QFileDialog.getExistingDirectory(self, tr("Output folder"), str(Path(self.path.text()).parent) if self.path.text() else "")
            if parent:
                name = safe_file_name(str((self.sequence or {}).get("name") or "export"))
                self.path.setText(str(Path(parent) / name))
            return
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        ext = self._extension()
        pattern = f"{fmt.get('label') or ext} (*.{ext})" if ext else tr("All files") + " (*)"
        path, _ = QFileDialog.getSaveFileName(self, tr("Output file"), self.path.text(), pattern)
        if path:
            # the native dialog has asked about replacing exactly the file it returned; a
            # destination with another extension is asked about when exporting
            self._replace_confirmed = {str(Path(path).expanduser())}
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
        if self.width_spin.isEnabled() and self.width_spin.value() and self.height_spin.value():
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
            if self.audio_bitrate.value():
                p["audioBitrateKbps"] = self.audio_bitrate.value()
        if self.container.isEnabled():
            p["mpeg2Container"] = self.container.currentData()
            if self.gop.value():
                p["keyframeDistance"] = self.gop.value()
            if "bFrames" in self.touched:
                p["mpeg2BFrames"] = self.b_frames.value()
            if "quantizer" in self.touched:
                p["mpeg2Quantizer"] = self.quantizer.value()
            if "chroma" in self.touched:
                p["mpeg2Chroma"] = self.chroma.currentData()
        if self.dcp_resolution.isEnabled():
            p["dcpResolution"] = self.dcp_resolution.currentData()
            p["dcpAspect"] = self.dcp_aspect.currentData()
            p["dcpContentKind"] = self.dcp_kind.currentData()
            mode = self.dcp_immersive.currentData()
            p["dcpImmersive"] = mode
            if mode == "file":
                p["dcpIabSource"] = os.path.expanduser(self.dcp_iab_source.text().strip())
            if mode != "off":
                p["dcpIabFallback"] = self.dcp_iab_fallback.currentData()
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
        except OSError as e:
            self._summary_error(tr("Cannot create the output folder: {error}").replace("{error}", str(e)))
            return
        try:
            existing = existing_outputs(out, fmt)
            # the engine writes the caption sidecar before the encode starts
            sidecar = sidecar_output(str(target if isinstance(target, str) and target else params["path"]),
                                     settings.get("captionSidecar") if isinstance(settings, dict) else None)
            if sidecar is not None and sidecar.exists() and sidecar not in existing:
                existing.append(sidecar)
        except OSError as e:
            # unlisted files could still be overwritten without asking
            self._summary_error(tr("Cannot check the output folder for files to replace: {error}").replace("{error}", str(e)))
            return
        unconfirmed = [e for e in existing if str(e) not in self._replace_confirmed]
        if unconfirmed and not self._confirm_replace(out, unconfirmed[0], existing):
            return
        params["start"] = start
        self.engine.execute("export.queue.add", params, on_reply=lambda r, b: self._on_queue(r, b), on_error=self._summary_error)
        self._poll.start()

    def _confirm_replace(self, out: Path, existing: Path | None = None, replaced: list[Path] | None = None) -> bool:
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
        self._replace_confirmed = {str(p) for p in (replaced or [out])}
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
