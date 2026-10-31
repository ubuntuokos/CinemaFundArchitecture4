"""Export window: preset, format, output file, range, video and audio settings, and the export
queue with progress.

Everything is the engine's: presets (``export.presets.list``), formats (``export.formats``),
the settings a choice resolves to (``export.resolve``) and the queue (``export.queue.*``, encoded
by the engine server between requests). The window only collects the choices and shows the
engine's answers; it never computes export settings itself.
"""

from __future__ import annotations

import copy
import os
import re
from pathlib import Path
from typing import Any

from PySide6.QtCore import QObject, QRect, Qt, QTimer, Signal
from PySide6.QtGui import QImage
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
    QFrame,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QMessageBox,
    QProgressBar,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSplitter,
    QTableWidget,
    QTableWidgetItem,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, decimal, fmt, tr
from .export_effects import METADATA_FORMATS, SECTION_KEYS, CaptionsSection, EffectsSection, MetadataSection, Section
from .export_preview import PreviewColumn
from .export_rows import ExportRows
from .preset_manager import PresetManager
from .smart_export import SmartExportBox

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
    ("bed", N_("IAB bed from the mix (7.1 DS, 9.1OH from 7.1.2)")),
    ("file", N_("IAB from a file (IAB MXF, .iab, or Dolby Atmos TrueHD)")),
)
# what channels 1-6 of an immersive DCP's main sound carry; ST 429-19:2023 7.1.2 wants none (#348)
DCP_IAB_FALLBACK = (
    ("none", N_("None: FSK sync only (SMPTE ST 429-19)")),
    ("5.1", N_("5.1: the IAB rendered (not ST 429-19)")),
    ("mix", N_("5.1: the timeline mix (not ST 429-19)")),
)
AUDIO_CODECS = (("auto", N_("Automatic")), ("aac", N_("AAC")), ("pcm", N_("PCM (uncompressed)")), ("flac", N_("FLAC")), ("opus", N_("Opus")))
# per-format choices (egui's export_mode per-format rows): the engine's flat parameter (and
# camelCase setting), the row label, the automation id and the choices (id, label)
FORMAT_CHOICES: tuple[tuple[str, str, str, tuple[tuple[str, str], ...]], ...] = (
    ("mxfVideoCodec", N_("Video Codec"), "export.video.mxfCodec", (("dnxhr", "Avid DNxHR"), ("proRes", "Apple ProRes"), ("h264", "H.264"))),
    ("proresProfile", N_("Profile"), "export.video.proresProfile", (
        ("proxy", "Apple ProRes 422 Proxy"), ("lt", "Apple ProRes 422 LT"), ("standard", "Apple ProRes 422"),
        ("hq", "Apple ProRes 422 HQ"), ("4444", "Apple ProRes 4444"), ("4444xq", "Apple ProRes 4444 XQ"))),
    ("dnxProfile", N_("Profile"), "export.video.dnxProfile", (("lb", "DNxHR LB"), ("sq", "DNxHR SQ"), ("hq", "DNxHR HQ"), ("hqx", "DNxHR HQX (10-bit)"))),
    ("apvProfile", N_("Profile"), "export.video.apvProfile", (("422-10", "APV 422-10"), ("422-12", "APV 422-12"), ("444-10", "APV 444-10"), ("444-12", "APV 444-12"))),
    ("cineformProfile", N_("Encoding"), "export.video.cineformProfile", (("422", "YUV 4:2:2 10-bit"), ("444", "RGB 4:4:4 12-bit"), ("4444", "RGBA 4:4:4:4 12-bit"))),
    ("cineformQuality", N_("Quality"), "export.video.cineformQuality", (
        ("filmscan3", N_("Film Scan 3")), ("filmscan2", N_("Film Scan 2")), ("filmscan1", N_("Film Scan 1")),
        ("high", N_("High")), ("medium", N_("Medium")), ("low", N_("Low")))),
    ("exrCompression", N_("Compression"), "export.video.exrCompression", (
        ("none", N_("None")), ("rle", "RLE"), ("zips", N_("ZIP (1 scan line)")), ("zip", N_("ZIP (16 scan lines)")))),
)
# what the engine encodes with when the setting is empty or unknown (`prores_profile()` & co.)
FORMAT_DEFAULTS = {"mxfVideoCodec": "dnxhr", "proresProfile": "hq", "dnxProfile": "hq", "apvProfile": "422-10", "cineformProfile": "422",
                   "cineformQuality": "filmscan1", "exrCompression": "none"}
MXF_FORMATS = {"mxf-op1a", "mxf-opatom"}
MXF_ENCODERS = {"dnxhr": "dnxhr", "proRes": "prores", "h264": "h264"}
# the encoder format each per-format row belongs to
FORMAT_ROW_ENCODER = {"proresProfile": "prores", "dnxProfile": "dnxhr", "apvProfile": "apv", "cineformProfile": "cineform",
                      "cineformQuality": "cineform", "exrCompression": "exr"}
QUALITY_FORMATS = {"mjpeg", "jpg"}  # JPEG quality 1..100
QUEUE_POLL_MS = 500
# engine QueueStatus (crates/engine/src/export_tools.rs): ready, encoding, done, failed, cancelled
FINISHED = {"done", "failed", "cancelled"}
STATUS_LABELS = {"ready": N_("Ready"), "encoding": N_("Encoding"), "done": N_("Done"), "failed": N_("Failed"), "cancelled": N_("Cancelled")}


def encoder_format(fmt: str, mxf_codec: str) -> str:
    """The format whose encoder writes the video (the engine's ``ExportSettings::video_format``):
    an MXF file's codec, HLS's H.264, else the format itself."""
    if fmt in MXF_FORMATS:
        return MXF_ENCODERS.get(mxf_codec, "dnxhr")
    return "h264" if fmt == "hls" else fmt


def supports_alpha(fmt: str, prores_profile: str, cineform_profile: str) -> bool:
    """Whether the output can keep an alpha channel (the engine's ``ExportSettings::supports_alpha``).

    GIF keeps binary transparency (alpha below 128 transparent, the rest opaque)."""
    if fmt in ("png", "tiff", "tga", "exr", "gif"):
        return True
    if fmt == "prores":
        return prores_profile in ("4444", "4444xq")
    return fmt == "cineform" and cineform_profile == "4444"


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


def safe_file_name(name: str) -> str:
    cleaned = "".join("_" if c in '/\\\0' else c for c in name).strip().strip(".")
    return cleaned or "export"


class OutputPath(QObject):
    """egui's Media File rows: the File Name (without its extension) and the Location folder.

    The output path is the folder, the name and the extension the settings write (``ext()``; none
    for a DCP's package folder), as egui's ``output_path``: an empty name is ``Export``. Reading it
    as one path (``text()`` / ``setText()``) keeps the rest of the window to a single value."""

    textChanged = Signal(str)

    def __init__(self, parent: QWidget, ext: Any) -> None:
        super().__init__(parent)
        self.ext = ext  # () -> the extension the settings write ("" for none, None while unknown)
        self.typed_ext = ""  # the extension a path came with, used while the format's is unknown
        self.file_name = QLineEdit(parent)
        self.file_name.setObjectName("export.fileName")
        self.file_name.setPlaceholderText(tr("Sequence name"))
        self.location = QLineEdit(parent)
        self.location.setObjectName("export.location")
        self.location.setPlaceholderText(tr("Folder"))
        self.file_name.textChanged.connect(lambda _t: self.textChanged.emit(self.text()))
        self.location.textChanged.connect(lambda _t: self.textChanged.emit(self.text()))

    def text(self) -> str:
        name, folder = self.file_name.text().strip(), self.location.text().strip()
        if not name and not folder:
            return ""
        ext = self.ext()
        ext = self.typed_ext if ext is None else ext
        file = f"{name or 'Export'}.{ext}" if ext else (name or "Export")
        return str(Path(folder) / file) if folder else file

    def setText(self, path: str) -> None:
        """Split a path into the folder and the name without its extension (the settings' own
        extension replaces it, as in egui)."""
        p = Path(path.strip()) if path.strip() else None
        name = (p.stem if p.suffix else p.name) if p is not None else ""
        self.typed_ext = p.suffix.lstrip(".") if p is not None else ""
        folder = str(p.parent) if p is not None and str(p.parent) != "." else ""
        if (name, folder) == (self.file_name.text(), self.location.text()):
            return
        self.blockSignals(True)
        try:
            self.file_name.setText(name)
            self.location.setText(folder)
        finally:
            self.blockSignals(False)
        self.textChanged.emit(self.text())


def format_eta(seconds: Any) -> str:
    try:
        s = max(0, round(float(seconds)))
    except (TypeError, ValueError, OverflowError):
        return ""
    return f"{s // 3600}:{s // 60 % 60:02d}:{s % 60:02d}" if s >= 3600 else f"{s // 60}:{s % 60:02d}"


def deep_merge(dst: dict[str, Any], src: dict[str, Any]) -> dict[str, Any]:
    """``src`` merged into ``dst`` (objects field by field, anything else replaced)."""
    for k, v in src.items():
        if isinstance(v, dict) and isinstance(dst.get(k), dict):
            deep_merge(dst[k], v)
        else:
            dst[k] = v
    return dst


QUEUE_COLUMN_W = 500
QUEUE_ACTIONS_COL = 6


class QueueTable(QTableWidget):
    """The export queue; each row is egui's ``export.queue.item.<id>`` part (its status as the label)."""

    def automation_parts(self) -> list[tuple[str, QRect, str]]:
        out = []
        offset = self.viewport().pos()
        for row in range(self.rowCount()):
            first = self.item(row, 0)
            if first is None or self.isRowHidden(row):
                continue
            rect = QRect()
            for col in range(self.columnCount()):
                rect = rect.united(self.visualRect(self.model().index(row, col)))
            rect = rect.intersected(self.viewport().rect())
            if rect.isValid():
                status = self.item(row, 3)
                out.append((f"export.queue.item.{first.data(Qt.ItemDataRole.UserRole)}", rect.translated(offset), status.text() if status else ""))
        return out


class ExportDialog(QDialog):
    """Non-modal: the editor stays usable while the queue encodes."""

    finished_export = Signal(str, bool)  # output path, success

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.export")
        self.setWindowTitle(tr("Export Media"))
        self.setModal(False)
        self.engine = engine
        # existing files the user already chose to replace when exporting to this output
        self._replace_confirmed: set[str] = set()
        self.sequence: dict[str, Any] | None = None
        self.project_path: str | None = None
        self.presets: list[dict[str, Any]] = []
        self.formats: dict[str, dict[str, Any]] = {}
        self._loading = False
        self._known_status: dict[Any, str] = {}
        self._queue_items: list[dict[str, Any]] = []  # the queue as last listed, in order
        self._follow: Any = None  # a moved item's id: the selection follows it
        # settings the user changed since the preset was chosen: only these override the preset
        self.touched: set[str] = set()
        self._want_preset: str | None = None
        self._shown_preset: str | None = None
        self.preset_manager: PresetManager | None = None
        # the settings the engine resolved last (egui's ``ExportUi::settings``), and the settings an
        # agent set through the control channel (deep-merged, sent under the choices made here)
        self.resolved_settings: dict[str, Any] = {}
        self.agent_settings: dict[str, Any] = {}
        self._agent_before: dict[str, Any] | None = None  # to undo a patch the engine refuses

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
        # egui's Media File rows: File Name, then Location with its folder picker
        self.path = OutputPath(self, lambda: "" if self._dcp() else (self._extension() or None))
        browse = QPushButton("…", self)
        browse.setObjectName("export.location.browse")
        browse.setToolTip(tr("Choose folder"))
        browse.clicked.connect(self._browse)
        path_row = QHBoxLayout()
        path_row.addWidget(self.path.location, 1)
        path_row.addWidget(browse)

        self.range = QComboBox(self)
        self.range.setObjectName("export.range")
        for key, label in RANGES:
            self.range.addItem(tr(label), key)
        self.start = QDoubleSpinBox(self)
        self.end = QDoubleSpinBox(self)
        for spin, name in ((self.start, "start"), (self.end, "end")):
            spin.setObjectName(f"export.range.{name}")
            spin.setDecimals(3)
            spin.setRange(0.0, 1e7)
            spin.setSuffix(" s")
        range_row = QHBoxLayout()
        range_row.addWidget(self.range, 1)
        range_row.addWidget(self.start)
        range_row.addWidget(QLabel("–", self))
        range_row.addWidget(self.end)
        # A one-frame Sequence In/Out is valid but easily left behind by accident: the engine's
        # `export.resolve` reports it in `warnings`, shown here until the range changes.
        self.range_warning = QLabel(tr("Sequence In/Out is only 1 frame. Choose Entire Sequence to export the whole sequence."), self)
        self.range_warning.setObjectName("export.range.warning")
        self.range_warning.setWordWrap(True)
        self.range_warning.setStyleSheet("color: #d04040")
        self.range_warning.setVisible(False)

        self.width_spin = QSpinBox(self)
        self.height_spin = QSpinBox(self)
        for spin, name in ((self.width_spin, "width"), (self.height_spin, "height")):
            spin.setObjectName(f"export.video.{name}")
            spin.setRange(0, 16384)
            spin.setSpecialValueText(tr("Match sequence"))
        # egui's Frame Size row: Match Source, else the width × height (the sequence's when turned off)
        self.match_size = QCheckBox(tr("Match Source"), self)
        self.match_size.setObjectName("export.video.matchSize")
        self.match_size.setChecked(True)
        self.size_times = QLabel("×", self)
        size_row = QHBoxLayout()
        size_row.addWidget(self.match_size)
        size_row.addWidget(self.width_spin)
        size_row.addWidget(self.size_times)
        size_row.addWidget(self.height_spin)
        size_row.addStretch(1)
        self.bitrate_mode = QComboBox(self)
        self.bitrate_mode.setObjectName("export.video.bitrateMode")
        for key, label in BITRATE_MODES:
            self.bitrate_mode.addItem(tr(label), key)
        # Target Bitrate as egui's: Mbps, the preset's value; an adaptive preset (Match Source) shows
        # its bits per pixel with Set, which turns it into a fixed bitrate
        self.bitrate = QDoubleSpinBox(self)
        self.bitrate.setObjectName("export.video.target")
        self.bitrate.setRange(0.1, 800.0)
        self.bitrate.setDecimals(1)
        self.bitrate.setSingleStep(0.1)
        self.bitrate.setSuffix(" Mbps")
        self.adaptive = QLabel(self)
        self.adaptive.setObjectName("export.video.adaptive")
        self.fixed_bitrate = QPushButton(tr("Set"), self)
        self.fixed_bitrate.setObjectName("export.video.fixedBitrate")
        self.fixed_bitrate.setToolTip(tr("Set a fixed bitrate"))
        self.fixed_bitrate.clicked.connect(self._set_fixed_bitrate)
        self.bitrate_box = QWidget(self)
        br = QHBoxLayout(self.bitrate_box)
        br.setContentsMargins(0, 0, 0, 0)
        br.addWidget(self.bitrate, 1)
        br.addWidget(self.adaptive, 1)
        br.addWidget(self.fixed_bitrate)
        self._target_kbps = 0  # the resolved target (an adaptive preset's computed bitrate)
        # MPEG-2 only: GOP length, B pictures, constant quantiser, 4:2:2 Profile
        self.gop = QSpinBox(self)
        self.gop.setObjectName("export.mpeg2Gop")
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
        # per-format settings: shown for the chosen format, sent only once changed here
        self.format_choices: dict[str, QComboBox] = {}
        for key, _label, name, choices in FORMAT_CHOICES:
            c = QComboBox(self)
            c.setObjectName(name)
            for value, text in choices:
                c.addItem(tr(text), value)
            c.setCurrentIndex(max(0, c.findData(FORMAT_DEFAULTS[key])))
            self.format_choices[key] = c
        self.alpha = QCheckBox(tr("Include Alpha Channel"), self)
        self.alpha.setObjectName("export.video.alpha")
        self.quality = QSpinBox(self)
        self.quality.setObjectName("export.video.quality")
        self.quality.setRange(1, 100)
        self.hardware = QCheckBox(tr("Use hardware encoding when available"), self)
        self.hardware.setObjectName("export.video.hardwareEncoding")
        self.captions = CaptionsSection(self)
        self.effects = EffectsSection(self)
        self.metadata = MetadataSection(self)
        self.preview = PreviewColumn(self)
        self._preview_busy = False
        # H.264 / H.265: MP4 or QuickTime (egui's Multiplexer section)
        self.multiplexer = QComboBox(self)
        self.multiplexer.setObjectName("export.multiplexer")
        self.multiplexer.addItem("MP4", "mp4")
        self.multiplexer.addItem("QuickTime", "mov")
        self.audio = QCheckBox(tr("Export Audio"), self)
        self.audio.setObjectName("export.audio.include")
        self.audio.setChecked(True)  # sound is part of an export unless the user leaves it out
        self.audio_codec = QComboBox(self)
        self.audio_codec.setObjectName("export.audio.codec")
        for key, label in AUDIO_CODECS:
            self.audio_codec.addItem(tr(label), key)
        # AAC and Opus: the engine clamps Opus to 6..510 kbps
        self.audio_bitrate = QSpinBox(self)
        self.audio_bitrate.setObjectName("export.audio.bitrate")
        self.audio_bitrate.setRange(0, 1_000)
        self.audio_bitrate.setSuffix(" kbps")
        self.audio_bitrate.setSpecialValueText(tr("From the preset"))

        out = QGroupBox(tr("Media File"), self)
        f = QFormLayout(out)
        f.addRow(tr("Sequence"), self.sequence_label)
        f.addRow(tr("File Name"), self.path.file_name)
        f.addRow(tr("Location"), path_row)
        preset_row = QHBoxLayout()
        preset_row.addWidget(self.preset, 1)
        more = QPushButton(tr("More presets…"), self)
        more.setObjectName("export.preset.more")
        more.clicked.connect(self.open_preset_manager)
        preset_row.addWidget(more)
        f.addRow(tr("Preset"), preset_row)
        f.addRow(tr("Format"), self.format)
        f.addRow(tr("Container"), self.container)
        f.addRow(tr("DCP resolution"), self.dcp_resolution)
        f.addRow(tr("DCP container"), self.dcp_aspect)
        f.addRow(tr("Content kind"), self.dcp_kind)
        f.addRow(tr("Range"), range_row)
        f.addRow("", self.range_warning)
        video = Section("video", tr("Video"), self)
        video.setObjectName("export.video")
        fv = QFormLayout(video.body)
        fv.addRow(tr("Frame Size"), size_row)
        fv.addRow(tr("Bitrate Encoding"), self.bitrate_mode)
        fv.addRow(tr("Target Bitrate"), self.bitrate_box)
        fv.addRow(tr("Key Frame Distance"), self.gop)  # MPEG-2's GOP length; H.26x: the rows' own
        fv.setRowVisible(self.gop, False)  # until MPEG-2 is chosen
        fv.addRow(tr("B pictures"), self.b_frames)
        fv.addRow(tr("Constant quantiser"), self.quantizer)
        fv.addRow(tr("Chroma"), self.chroma)
        for key, label, _name, _choices in FORMAT_CHOICES:
            fv.addRow(tr(label), self.format_choices[key])
        mux = Section("multiplexer", tr("Multiplexer"), self)
        mux.setObjectName("export.multiplexerSection")
        QFormLayout(mux.body).addRow(tr("Multiplexer"), self.multiplexer)
        fv.addRow(tr("Alpha"), self.alpha)
        fv.addRow(tr("Quality"), self.quality)
        self.video_form = fv
        fv.addRow(self.hardware)
        audio = Section("audio", tr("Audio"), self)
        audio.setObjectName("export.audio")
        fa = QFormLayout(audio.body)
        fa.addRow(self.audio)
        fa.addRow(tr("Audio Format"), self.audio_codec)
        fa.addRow(tr("Audio Bitrate"), self.audio_bitrate)
        fa.addRow(tr("Immersive audio"), self.dcp_immersive)
        fa.addRow(tr("IAB source"), iab_row)
        fa.addRow("", self.dcp_iab_info)
        fa.addRow(tr("Main sound channels 1-6"), self.dcp_iab_fallback)
        self.rows = ExportRows(self, fv, fa)
        self.mux_section = mux

        self.summary = QLabel(self)
        self.summary.setObjectName("export.summary")
        self.summary.setWordWrap(True)
        self.summary.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)

        self.export_button = QPushButton(tr("Export"), self)
        self.export_button.setObjectName("export.button")
        self.export_button.setDefault(True)
        self.export_button.clicked.connect(lambda: self._enqueue(start=True))
        self.queue_button = QPushButton(tr("Send to Queue"), self)
        self.queue_button.setObjectName("export.sendToQueue")
        self.queue_button.clicked.connect(lambda: self._enqueue(start=False))
        close = QPushButton(tr("Close"), self)
        close.clicked.connect(self.hide)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        buttons.addWidget(self.queue_button)
        buttons.addWidget(self.export_button)
        buttons.addWidget(close)

        # ----- queue -----
        self.table = QueueTable(0, 7, self)
        self.table.setObjectName("export.queue")
        self.table.setHorizontalHeaderLabels(
            [tr("Sequence"), tr("Output file"), tr("Format"), tr("Status"), tr("Progress"), tr("Remaining"), ""]
        )
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.table.verticalHeader().setVisible(False)
        header = self.table.horizontalHeader()
        header.setSectionResizeMode(QHeaderView.ResizeMode.ResizeToContents)
        # what egui's narrow column shows first: the export, its status and progress, its buttons
        header.moveSection(QUEUE_ACTIONS_COL, 5)
        header.moveSection(2, 6)
        header.setSectionResizeMode(4, QHeaderView.ResizeMode.Fixed)
        header.resizeSection(4, 64)
        # two rows of four: the column is narrow (egui's is 300 px)
        queue_buttons = QGridLayout()
        for n_button, (name, text, slot) in enumerate((
            ("start", N_("Start Queue"), lambda: self._queue_command("export.queue.start", {})),
            ("stop", N_("Stop Queue"), lambda: self._queue_command("export.queue.stop", {})),
            ("cancel", N_("Cancel Selected"), lambda: self._selected_command("export.queue.cancel")),
            ("retry", N_("Retry"), lambda: self._selected_command("export.queue.retry", {"start": True})),
            ("remove", N_("Remove"), lambda: self._selected_command("export.queue.remove")),
            ("clear", N_("Clear Finished"), lambda: self._queue_command("export.queue.clear", {})),
            # egui's Up / Down: the order the queue encodes in (not past the export encoding now)
            ("up", N_("Up"), lambda: self._move_selected(-1)),
            ("down", N_("Down"), lambda: self._move_selected(1)),
        )):
            b = QPushButton(tr(text), self)
            b.setObjectName(f"export.queue.{name}")
            b.clicked.connect(slot)
            queue_buttons.addWidget(b, n_button // 4, n_button % 4)
            if name in ("up", "down"):
                setattr(self, f"queue_{name}", b)
        self.table.itemSelectionChanged.connect(self._update_move_buttons)
        # egui's "Queue (n)" heading, and its hint while the queue is empty
        self.queue_box = queue_box = QGroupBox(tr("Queue ({n})").format(n=0), self)
        queue_box.setObjectName("export.queueBox")
        self.queue_empty = QLabel(tr("Send exports here with Send to Queue, then start the queue."), self)
        self.queue_empty.setObjectName("export.queue.empty")
        self.queue_empty.setWordWrap(True)
        self.queue_empty.setEnabled(False)
        qv = QVBoxLayout(queue_box)
        qv.addWidget(self.queue_empty)
        qv.addWidget(self.table)
        qv.addLayout(queue_buttons)

        # egui's left column: "Destinations" (Media File, the only one) over the queue
        self.destinations = QLabel(tr("Destinations"), self)
        self.destinations.setObjectName("export.destinations")
        font = self.destinations.font()
        font.setBold(True)
        self.destinations.setFont(font)
        self.media_file = QLabel(tr("Media File"), self)
        self.media_file.setObjectName("export.destination.mediaFile")
        self.media_file.setMinimumHeight(26)
        self.media_file.setContentsMargins(10, 0, 10, 0)
        # egui fills the row with the selected-row colour: Media File is always the chosen destination
        self.media_file.setAutoFillBackground(True)
        pal = self.media_file.palette()
        pal.setColor(self.media_file.backgroundRole(), pal.color(pal.ColorRole.Highlight))
        pal.setColor(self.media_file.foregroundRole(), pal.color(pal.ColorRole.HighlightedText))
        self.media_file.setPalette(pal)
        queue_column = QWidget(self)
        queue_column.setObjectName("export.queueColumn")
        qc = QVBoxLayout(queue_column)
        qc.setContentsMargins(0, 0, 0, 0)
        qc.addWidget(self.destinations)
        qc.addWidget(self.media_file)
        qc.addWidget(queue_box, 1)
        self.queue_column = queue_column

        top = QHBoxLayout()
        left = QVBoxLayout()
        left.addWidget(out)
        left.addWidget(video)
        left.addWidget(mux)
        left.addWidget(audio)
        left.addStretch(1)
        top.addLayout(left, 1)
        right = QVBoxLayout()
        right.addWidget(self.preview)
        # the export mode and the render plan preview (WP-08, Smart Export)
        self.smart = SmartExportBox(
            engine,
            lambda: (self.sequence or {}).get("id") if isinstance((self.sequence or {}).get("id"), int) else None,
            lambda: str(self.params().get("path") or ""),
            self._smart_said,
            self,
        )
        right.addWidget(self.smart)
        right.addWidget(self.captions)
        right.addWidget(self.effects)
        right.addWidget(self.metadata)
        right.addStretch(1)
        top.addLayout(right, 1)
        # the settings scroll (egui's settings column does); the queue stays in view
        settings_page = QWidget(self)
        settings_page.setLayout(top)
        scroll = QScrollArea(self)
        scroll.setObjectName("export.settingsScroll")
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(settings_page)
        self.settings_scroll = scroll
        # egui's placeholder in place of the settings while no sequence is open; the queue stays
        self.no_sequence = QLabel(tr("Open a sequence to export"), self)
        self.no_sequence.setObjectName("export.noSequence")
        self.no_sequence.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.no_sequence.setEnabled(False)
        self.no_sequence.hide()
        settings_column = QWidget(self)
        sc = QVBoxLayout(settings_column)
        sc.setContentsMargins(0, 0, 0, 0)
        sc.addWidget(scroll, 1)
        sc.addWidget(self.no_sequence, 1)
        sc.addWidget(self.summary)
        sc.addLayout(buttons)
        # egui's columns: the queue on the left (300 px there), the settings and preview beside it
        self.splitter = splitter = QSplitter(Qt.Orientation.Horizontal, self)
        splitter.setObjectName("export.columns")
        splitter.setChildrenCollapsible(False)
        splitter.addWidget(queue_column)
        splitter.addWidget(settings_column)
        splitter.setStretchFactor(1, 1)
        splitter.setSizes([QUEUE_COLUMN_W, 1080])
        layout = QVBoxLayout(self)
        layout.addWidget(splitter)
        self.resize(QUEUE_COLUMN_W + 1080, 860)

        # egui's collapsible settings sections (`ExportUi::open_sections`)
        self.sections: dict[str, Section] = {
            s.key: s for s in (video, mux, audio, self.captions, self.effects, self.metadata)
        }

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
        self.match_size.toggled.connect(self._match_size_toggled)
        for w in (self.width_spin, self.height_spin):
            w.valueChanged.connect(lambda _v: self._sync_size_shown())
        for c in (self.dcp_resolution, self.dcp_aspect, self.dcp_kind):
            c.currentIndexChanged.connect(self._changed)
        self.dcp_immersive.currentIndexChanged.connect(self._immersive_changed)
        self.dcp_iab_source.editingFinished.connect(self._probe_iab)
        self.dcp_iab_source.textChanged.connect(self._changed)
        self.dcp_iab_fallback.currentIndexChanged.connect(self._changed)
        for key, c in (("bitrateMode", self.bitrate_mode), ("audioCodec", self.audio_codec), ("chroma", self.chroma)):
            c.currentIndexChanged.connect(lambda _i, k=key: self._touch(k))
        for key, b in (("hardware", self.hardware), ("audio", self.audio), ("alpha", self.alpha)):
            b.toggled.connect(lambda _on, k=key: self._touch(k))
        for key, c in self.format_choices.items():
            c.currentIndexChanged.connect(lambda _i, k=key: (self._touch(k), self._sync_format_rows()))
        self.quality.valueChanged.connect(lambda _v: self._touch("quality"))
        self.captions.changed.connect(lambda: self._touch("captions"))
        self.effects.changed.connect(lambda: self._touch("effects"))
        self.metadata.changed.connect(lambda: self._touch("metadata"))
        self.preview.scaling.currentIndexChanged.connect(lambda _i: self._touch("scaling"))
        self.rows.changed.connect(lambda k: (self._touch(k), self._sync_rows()))
        self.bitrate_mode.currentIndexChanged.connect(lambda _i: self._sync_rows())
        self.audio_codec.currentIndexChanged.connect(lambda _i: self._sync_rows())
        self.audio.toggled.connect(lambda _on: self._sync_rows())
        self.multiplexer.currentIndexChanged.connect(lambda _i: (self._touch("multiplexer"), self._container_changed()))
        self.path.textChanged.connect(self._changed)
        self._range_changed()
        self._sync_size_shown()

    # ----- opening -----
    def open_for(self, sequence: dict[str, Any] | None, project_path: str | None) -> None:
        self.sequence = sequence
        self.project_path = project_path
        name = str((sequence or {}).get("name") or tr("No sequence"))
        self.sequence_label.setText(name)
        self.export_button.setEnabled(sequence is not None)
        self.queue_button.setEnabled(sequence is not None)
        self.sequence_updated(sequence)
        self.smart.refresh()
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
        self.refresh_preview()

    def sequence_updated(self, sequence: dict[str, Any] | None) -> None:
        """What the window shows of the open sequence: egui's placeholder while there is none, and
        the Captions hint while it has no caption track."""
        none = sequence is None
        self.settings_scroll.setVisible(not none)
        self.summary.setVisible(not none)
        self.no_sequence.setVisible(none)
        tracks = (sequence or {}).get("captionTracks")
        self.captions.set_has_track(isinstance(tracks, list) and bool(tracks))

    def refresh_preview(self) -> None:
        """The preview's frame: the sequence at the playhead (egui's export preview)."""
        if self.sequence is None or self._preview_busy:
            return
        self._preview_busy = True

        def got(result: Any, data: bytes | None) -> None:
            self._preview_busy = False
            if isinstance(result, dict) and data:
                w, h = int(result.get("w") or 0), int(result.get("h") or 0)
                if w > 0 and h > 0 and len(data) == w * h * 4:
                    image = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
                    self.preview.set_frame(image, (int(result.get("fullW") or 0), int(result.get("fullH") or 0)))

        def failed(_m: str) -> None:
            self._preview_busy = False
            self.preview.set_frame(None, (0, 0))

        self.engine.call("frame.render", {"maxWidth": 500, "maxHeight": 500}, on_reply=got, on_error=failed)

    def open_preset_manager(self) -> None:
        """Preset ▸ More presets…: the Preset Manager on the current preset; OK applies its choice."""
        if self.preset_manager is None:
            self.preset_manager = PresetManager(self.engine, self.params, self)
            self.preset_manager.applied.connect(self._preset_chosen)
            self.preset_manager.status.connect(self._error)
            self.preset_manager.finished.connect(lambda _r: self._reload_presets())
        self.preset_manager.open_on(str(self.preset.currentData() or ""))

    def _preset_chosen(self, name: str) -> None:
        self._want_preset = name
        self._reload_presets()

    def _reload_presets(self) -> None:
        # saved, imported or deleted presets show in the menu; the choice is kept
        self.engine.execute("export.presets.list", {}, on_reply=self._on_presets, on_error=self._error)

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
        self._shown_preset = self.preset.currentData() if self.preset.count() else None
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
        # after the Preset Manager: the preset it chose, else the one shown before the reload
        applied = self._want_preset is not None
        want = self._want_preset or self._shown_preset or default
        self._want_preset = None
        index = self.preset.findData(want)
        if index < 0:
            index = self.preset.findData(default)
        self.preset.setCurrentIndex(max(0, index))
        self.preset.blockSignals(False)
        self._loading = False
        # a reload that keeps the shown preset (Preset Manager ▸ Cancel) keeps the changes made here
        if applied or self.preset.currentData() != self._shown_preset:
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
        self.agent_settings = {}
        self._loading = False
        self._default_path()
        self._changed()

    def _format_changed(self) -> None:
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        self.audio.setEnabled(bool(fmt.get("audio", True)))
        self.audio_codec.setEnabled(bool(fmt.get("audio", True)))
        self.audio_bitrate.setEnabled(bool(fmt.get("audio", True)))
        self.bitrate_mode.setEnabled(bool(fmt.get("video", True)) and not fmt.get("imageSequence"))
        self.bitrate_box.setEnabled(bool(fmt.get("video", True)) and not fmt.get("imageSequence"))
        mpeg2 = self.format.currentData() == "mpeg2"
        for w in (self.container, self.gop, self.b_frames, self.quantizer, self.chroma):
            w.setEnabled(mpeg2)
        self.video_form.setRowVisible(self.gop, mpeg2)  # H.264 / H.265 / AV1 have egui's own row
        self._sync_format_rows()
        video = bool(fmt.get("video", True))
        self.captions.setVisible(video)
        self.metadata.setVisible(self.format.currentData() in METADATA_FORMATS)
        self.preview.set_video(video)
        self._sync_rows()
        self.sections["multiplexer"].setVisible(self._muxed())
        self.effects.set_kinds(video, bool(fmt.get("audio", True)) or not video)
        dcp = self._dcp()
        for w in (self.dcp_resolution, self.dcp_aspect, self.dcp_kind, self.dcp_immersive):
            w.setEnabled(dcp)
        self._immersive_changed()
        if dcp:
            # fixed by the DCI specifications: container size, J2K data rate, 5.1 24-bit PCM
            for w in (self.match_size, self.width_spin, self.height_spin, self.bitrate_mode, self.bitrate_box, self.audio_codec, self.audio_bitrate):
                w.setEnabled(False)
        else:
            self.match_size.setEnabled(True)
            self.width_spin.setEnabled(True)
            self.height_spin.setEnabled(True)
        self._container_changed()

    def _choice(self, key: str) -> str:
        return str(self.format_choices[key].currentData() or "")

    def _format_rows(self) -> dict[str, bool]:
        """Which per-format rows the chosen format shows (egui's ``export_mode`` rows)."""
        fmt = str(self.format.currentData() or "")
        enc = encoder_format(fmt, self._choice("mxfVideoCodec"))
        rows = {key: enc == FORMAT_ROW_ENCODER.get(key) for key in self.format_choices}
        rows["mxfVideoCodec"] = fmt in MXF_FORMATS
        rows["alpha"] = supports_alpha(fmt, self._choice("proresProfile"), self._choice("cineformProfile"))
        rows["quality"] = enc in QUALITY_FORMATS
        return rows

    def _sync_format_rows(self) -> None:
        widgets: dict[str, QWidget] = {**self.format_choices, "alpha": self.alpha, "quality": self.quality}
        for key, shown in self._format_rows().items():
            self.video_form.setRowVisible(widgets[key], shown)

    def _show_settings(self, settings: dict[str, Any]) -> None:
        """Show the resolved settings in the per-format rows the user has not changed here."""
        for key, c in self.format_choices.items():
            i = c.findData(settings.get(key))
            i = i if i >= 0 else c.findData(FORMAT_DEFAULTS[key])  # "" = the engine's default
            if key not in self.touched and i >= 0 and i != c.currentIndex():
                c.blockSignals(True)
                c.setCurrentIndex(i)
                c.blockSignals(False)
        if "alpha" not in self.touched and isinstance(settings.get("alpha"), bool):
            self.alpha.blockSignals(True)
            self.alpha.setChecked(settings["alpha"])
            self.alpha.blockSignals(False)
        self.rows.show_settings(settings, self.touched)
        self._show_bitrate(settings)
        if "bitrateMode" not in self.touched and isinstance(settings.get("bitrateMode"), str):
            # the preset's mode (CRF shows its Quality row)
            i = self.bitrate_mode.findData(settings["bitrateMode"])
            if i >= 0 and i != self.bitrate_mode.currentIndex():
                self.bitrate_mode.blockSignals(True)
                self.bitrate_mode.setCurrentIndex(i)
                self.bitrate_mode.blockSignals(False)
        self._sync_rows()
        if "captions" not in self.touched:
            self.captions.show_settings(settings)
        if "metadata" not in self.touched and isinstance(settings.get("metadata"), dict):
            self.metadata.show_metadata(settings["metadata"])
        if "multiplexer" not in self.touched and settings.get("multiplexer") != self.multiplexer.currentData():
            i = self.multiplexer.findData(settings.get("multiplexer"))
            if i >= 0:
                self.multiplexer.blockSignals(True)
                self.multiplexer.setCurrentIndex(i)
                self.multiplexer.blockSignals(False)
                self._container_changed()
        if "effects" not in self.touched and isinstance(settings.get("effects"), dict):
            self.effects.show_effects(settings["effects"])
        q = settings.get("quality")
        if "quality" not in self.touched and isinstance(q, int) and not isinstance(q, bool):
            self.quality.blockSignals(True)
            self.quality.setValue(q)
            self.quality.blockSignals(False)
        self._sync_format_rows()

    def _rows_shown(self) -> dict[str, bool]:
        fmt = str(self.format.currentData() or "")
        info = self.formats.get(fmt, {})
        video = bool(info.get("video", True))
        audio = bool(info.get("audio", True)) and (self.audio.isChecked() or not video)
        shown = self.rows.visible_rows(fmt, encoder_format(fmt, self._choice("mxfVideoCodec")), video, audio,
                                       str(self.bitrate_mode.currentData() or ""), str(self.audio_codec.currentData() or ""))
        if self._dcp():
            # the DCI specifications fix the J2K data rate and the sound (5.1 24-bit 48 kHz PCM)
            for key in ("crf", "maxBitrate", "sampleRate", "channels", "bits", "flacLevel"):
                shown[key] = False
        return shown

    def _match_size_toggled(self, on: bool) -> None:
        """egui's Match Source for the frame size: off starts from the sequence's size."""
        if on:
            w = h = 0
        else:
            st = (self.sequence or {}).get("settings")
            st = st if isinstance(st, dict) else {}
            w, h = st.get("width"), st.get("height")
            w = w if isinstance(w, int) and not isinstance(w, bool) and w > 0 else 1920
            h = h if isinstance(h, int) and not isinstance(h, bool) and h > 0 else 1080
        self.width_spin.setValue(w)
        self.height_spin.setValue(h)
        self._sync_size_shown()

    def _sync_size_shown(self) -> None:
        """The size fields show only while the frame size does not match the source."""
        match = not (self.width_spin.value() and self.height_spin.value())
        self.match_size.blockSignals(True)
        self.match_size.setChecked(match)
        self.match_size.blockSignals(False)
        for w in (self.width_spin, self.size_times, self.height_spin):
            w.setVisible(not match)

    def _sync_rows(self) -> None:
        self.rows.sync(self._rows_shown())
        # egui: Quality (CRF) takes the Target Bitrate's place
        self.video_form.setRowVisible(self.bitrate_box, self.bitrate_mode.currentData() != "crf")

    def _muxed(self) -> bool:
        return self.format.currentData() in ("h264", "hevc")

    def _immersive_changed(self) -> None:
        """The IAB source row is for ``file`` only; the fallback for any immersive DCP."""
        mode = self.dcp_immersive.currentData() if self.dcp_immersive.isEnabled() else "off"
        for w in (self.dcp_iab_source, self.dcp_iab_browse):
            w.setEnabled(mode == "file")
        self.dcp_iab_fallback.setEnabled(mode in ("bed", "file"))
        if mode == "bed":
            self.dcp_iab_info.setText(tr("A 7.1.2 mix becomes a 9.1OH bed, a 7.1 mix a 7.1 DS bed, any other mix a 7.1 DS bed from its 5.1 (IAB Profile 1); channel 14 of the main sound carries the FSK sync signal."))
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
        """The output file's extension: the MPEG-2 container's, QuickTime's for H.264 / H.265 in
        a QuickTime file, else the format's."""
        if self._muxed() and self.multiplexer.currentData() == "mov":
            return "mov"
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
        self._changed()  # the output path takes the new extension

    def _range_changed(self) -> None:
        custom = self.range.currentData() == "custom"
        self.start.setEnabled(custom)
        self.end.setEnabled(custom)
        self._changed()

    def _default_path(self) -> None:
        """egui's: the sequence's name in the default folder, while both rows are empty."""
        if self.path.text():
            return
        name = safe_file_name(str((self.sequence or {}).get("name") or "export"))
        self.path.file_name.setText(name)
        self.path.location.setText(str(default_output_dir(self.project_path)))

    def _browse(self) -> None:
        """Location's "…": choose the folder (a DCP's package folder is made inside it)."""
        start = self.path.location.text().strip()
        folder = QFileDialog.getExistingDirectory(self, tr("Choose folder"), os.path.expanduser(start) if start else "")
        if folder:
            self.path.location.setText(folder)

    def _changed(self) -> None:
        if not self._loading:
            self._resolve_timer.start()
            if hasattr(self, "smart"):
                self.smart.refresh()

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
            if "bitrate" in self.touched:
                p["bitrateKbps"] = round(self.bitrate.value() * 1000)
        if "hardware" in self.touched:
            p["hardwareEncoding"] = "auto" if self.hardware.isChecked() else "off"
        fmt = self.formats.get(str(self.format.currentData() or ""), {})
        if "captions" in self.touched and fmt.get("video", True):
            p.update(self.captions.params())
        patch: dict[str, Any] = copy.deepcopy(self.agent_settings)
        if "effects" in self.touched:
            patch["effects"] = self.effects.values()
        if "metadata" in self.touched and self.format.currentData() in METADATA_FORMATS:
            patch["metadata"] = self.metadata.values()
        deep_merge(patch, self.rows.patch(self.touched, self._rows_shown()))
        if "scaling" in self.touched:
            patch["scaling"] = self.preview.scaling.currentData()
        if "multiplexer" in self.touched and self._muxed():
            patch["multiplexer"] = self.multiplexer.currentData()
        if patch:
            p["settings"] = patch
        if self.audio.isEnabled():
            if "audio" in self.touched:
                p["audio"] = self.audio.isChecked()
            if "audioCodec" in self.touched:
                p["audioCodec"] = self.audio_codec.currentData()
            if self.audio_bitrate.value():
                p["audioBitrateKbps"] = self.audio_bitrate.value()
        rows = self._format_rows()
        for key in self.format_choices:
            if rows[key] and key in self.touched:
                p[key] = self._choice(key)
        if rows["alpha"] and "alpha" in self.touched:
            p["alpha"] = self.alpha.isChecked()
        if rows["quality"] and "quality" in self.touched:
            p["quality"] = self.quality.value()
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

    def _show_bitrate(self, settings: dict[str, Any]) -> None:
        """The preset's target in Mbps, or its adaptive bitrate (bits per pixel) with Set."""
        bpp = settings.get("adaptiveBitrate")
        adaptive = "bitrate" not in self.touched and isinstance(bpp, (int, float)) and not isinstance(bpp, bool)
        self.bitrate.setVisible(not adaptive)
        self.adaptive.setVisible(adaptive)
        self.fixed_bitrate.setVisible(adaptive)
        if adaptive:
            self.adaptive.setText(tr("Adaptive ({bpp} bits per pixel)").format(bpp=decimal(f"{float(bpp):g}")))
        kbps = settings.get("bitrateKbps")
        if "bitrate" not in self.touched and isinstance(kbps, int) and not isinstance(kbps, bool) and kbps > 0:
            self.bitrate.blockSignals(True)
            self.bitrate.setValue(kbps / 1000)
            self.bitrate.blockSignals(False)

    def _set_fixed_bitrate(self) -> None:
        """egui's Set: the adaptive preset's bitrate for this sequence becomes a fixed target."""
        if self._target_kbps > 0:
            self.bitrate.blockSignals(True)
            self.bitrate.setValue(self._target_kbps / 1000)
            self.bitrate.blockSignals(False)
        self._touch("bitrate")
        self._show_bitrate(self.resolved_settings)

    def _on_resolved(self, result: Any, _b: bytes | None) -> None:
        r = result if isinstance(result, dict) else {}
        self._agent_before = None
        target = (r.get("output") or {}).get("targetKbps")
        self._target_kbps = target if isinstance(target, int) and not isinstance(target, bool) else 0
        if isinstance(r.get("settings"), dict):
            self.resolved_settings = r["settings"]
            self.rows.show_resolved(r)
            self._show_settings(r["settings"])
        path = (r.get("settings") or {}).get("path") if isinstance(r.get("settings"), dict) else None
        self.preview.show_resolved(r, str(path or self.path.text()), self.range.currentText(), "scaling" in self.touched)
        out = r.get("output") or {}
        rng = r.get("range") or {}
        parts = []
        if out.get("width") and out.get("height"):
            parts.append(f"{out['width']}×{out['height']}")
        if out.get("fps"):
            parts.append(fmt("{fps} fps", fps=decimal(str(out["fps"]))))
        if out.get("targetKbps"):
            parts.append(f"{out['targetKbps']} kbps")
        if out.get("sampleRate"):
            parts.append(fmt("{n} ch {rate} Hz", n=out.get("channels", 2), rate=out["sampleRate"]))
        if isinstance(rng.get("seconds"), (int, float)):
            parts.append(fmt("{seconds} s", seconds=decimal(f"{float(rng['seconds']):.2f}")))
        if r.get("frames"):
            parts.append(f"{r['frames']} {tr('frames')}")
        self.summary.setText(" · ".join(parts))
        self.summary.setStyleSheet("")
        self.range_warning.setVisible(bool(r.get("warnings")))
        self.export_button.setEnabled(True)
        self.queue_button.setEnabled(True)

    def open_sections(self) -> list[str]:
        return [k for k in SECTION_KEYS if self.sections[k].is_open()]

    def set_open_sections(self, keys: list[str]) -> None:
        for k, section in self.sections.items():
            section.set_open(k in keys)

    def set_agent_settings(self, patch: dict[str, Any]) -> None:
        """egui's ``ui.set {export: {settings}}``: ``patch`` deep-merged into the export settings
        (under the choices made in the window); a patch the engine refuses is taken back."""
        self._agent_before = copy.deepcopy(self.agent_settings)
        deep_merge(self.agent_settings, copy.deepcopy(patch))
        self._resolve_timer.stop()
        self._resolve()

    def _summary_error(self, message: str) -> None:
        if self._agent_before is not None:
            self.agent_settings, self._agent_before = self._agent_before, None
            self._resolve_timer.start()
            message = f"{tr('Settings not taken')}: {message}"
        self.summary.setText(message)
        self.summary.setStyleSheet("color: #d04040")
        self.range_warning.setVisible(False)
        self.export_button.setEnabled(False)
        self.queue_button.setEnabled(False)

    def _error(self, message: str) -> None:
        self.summary.setText(message)
        self.summary.setStyleSheet("color: #d04040")

    # ----- queue -----
    def _smart_said(self, message: str, error: bool) -> None:
        if error:
            self._summary_error(message)
        else:
            self.summary.setText(message)
            self.summary.setStyleSheet("")

    def _enqueue(self, start: bool) -> None:
        if self.smart.smart():
            # Smart Export writes the file itself (no queue)
            self.smart.start()
            return
        params = self.params()
        if not params.get("path"):
            self._summary_error(tr("Choose a file name and location."))
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

    def _selected_row(self) -> int | None:
        rows = sorted({i.row() for i in self.table.selectedIndexes()})
        return rows[0] if len(rows) == 1 and rows[0] < len(self._queue_items) else None

    def _move_selected(self, by: int) -> None:
        """Move the selected export one place earlier (-1) or later (1) in the queue, as egui's
        Up / Down (``export.queue.move``); the selection follows it."""
        row = self._selected_row()
        if row is None or not self._can_move(row, by):
            return
        self._follow = self._queue_items[row].get("id")
        self._queue_command("export.queue.move", {"id": self._follow, "by": by})

    def _can_move(self, row: int, by: int) -> bool:
        items = self._queue_items
        if by < 0:  # egui: not up from the top, nor an export that is encoding
            return row > 0 and str(items[row].get("status") or "").lower() != "encoding"
        return row + 1 < len(items)

    def _update_move_buttons(self) -> None:
        row = self._selected_row()
        self.queue_up.setEnabled(row is not None and self._can_move(row, -1))
        self.queue_down.setEnabled(row is not None and self._can_move(row, 1))

    def _row_actions(self, row: int, it: dict[str, Any], status: str) -> None:
        """egui's buttons on each queued export: Up, Down, Cancel (ready or encoding) or Retry,
        and Remove, ids ``export.queue.item.<id>.<key>``; rebuilt only when what they offer changes,
        so a poll does not replace a button under the pointer."""
        qid = it.get("id")
        items = self._queue_items
        wanted = (
            qid,
            row > 0 and status != "encoding",
            row + 1 < len(items),
            status in ("ready", "encoding"),
            status != "encoding",
        )
        cell = self.table.cellWidget(row, QUEUE_ACTIONS_COL)
        if cell is not None and getattr(cell, "cfa4_actions", None) == wanted:
            return
        _id, up, down, cancel, remove = wanted
        cell = QWidget(self.table)
        cell.cfa4_actions = wanted  # type: ignore[attr-defined]
        h = QHBoxLayout(cell)
        h.setContentsMargins(2, 0, 2, 0)
        h.setSpacing(2)
        specs = [
            ("up", N_("Up"), up, "export.queue.move", {"id": qid, "by": -1}),
            ("down", N_("Down"), down, "export.queue.move", {"id": qid, "by": 1}),
            ("cancel", N_("Cancel"), True, "export.queue.cancel", {"id": qid})
            if cancel
            else ("retry", N_("Retry"), True, "export.queue.retry", {"id": qid, "start": True}),
            ("remove", N_("Remove"), remove, "export.queue.remove", {"id": qid}),
        ]
        small = cell.font()
        small.setPointSizeF(max(6.0, small.pointSizeF() - 1.5))
        for key, text, enabled, command, params in specs:
            b = QToolButton(cell)
            b.setText(tr(text))
            b.setFont(small)
            b.setObjectName(f"export.queue.item.{qid}.{key}")
            b.setEnabled(bool(enabled))
            b.clicked.connect(lambda _c=False, c=command, p=params: self._item_command(c, p))
            h.addWidget(b)
        self.table.setCellWidget(row, QUEUE_ACTIONS_COL, cell)
        self.table.resizeColumnToContents(QUEUE_ACTIONS_COL)

    def _item_command(self, command: str, params: dict[str, Any]) -> None:
        if command == "export.queue.move":
            self._follow = params.get("id")  # the selection follows the moved export, as Up / Down's
        self._queue_command(command, params)

    def refresh_queue(self) -> None:
        if self.engine.running:
            self.engine.execute("export.queue.list", {}, on_reply=self._on_queue, on_error=lambda _m: None)

    def _on_queue(self, result: Any, _b: bytes | None) -> None:
        data = result if isinstance(result, dict) else {}
        items = [i for i in data.get("items") or [] if isinstance(i, dict)]
        self._queue_items = items
        self.queue_box.setTitle(tr("Queue ({n})").format(n=len(items)))
        self.queue_empty.setVisible(not items)
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
                if col == 1:  # the file name, as egui's "sequence → file"; the whole path on hover
                    cell.setText(Path(text).name if text else "")
                    cell.setToolTip(text)
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
            self._row_actions(row, it, status)
            key = it.get("id")
            if self._known_status.get(key) != status and status in FINISHED and key in self._known_status:
                # it finished while we watched it
                self.finished_export.emit(str(it.get("path") or ""), status == "done")
            self._known_status[key] = status
        if self._follow is not None:
            row = next((r for r, it in enumerate(items) if it.get("id") == self._follow), None)
            self._follow = None
            if row is not None:
                self.table.selectRow(row)
        self._update_move_buttons()
        running = bool(data.get("running")) or any(str(i.get("status") or "").lower() == "encoding" for i in items)
        if not running and not self.isVisible():
            self._poll.stop()

    def hideEvent(self, e) -> None:
        super().hideEvent(e)
        # keep polling while something encodes, so the editor hears when it is done
        self.refresh_queue()
