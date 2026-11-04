"""The Export window's Captions and Effects sections (egui's ``export_mode`` Captions and Effects):
Export Options None / Burn Captions Into Video / Create Sidecar File with its SubRip or WebVTT
format; Image, Name and Timecode Overlay, Video Limiter and Loudness Normalization.

The values are the engine's ``ExportSettings`` (``burnCaptions``, ``captionSidecar`` and
``effects`` with its snake_case keys); the sections only show them and collect the changes.
Automation ids as egui's: ``export.captions.mode``, ``export.captions.format``,
``export.effects.<effect>`` (the switch) and ``export.effects.<effect>.<field>``.
"""

from __future__ import annotations

import copy
from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLineEdit,
    QPushButton,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, tr

CAPTION_MODES = (("none", N_("None")), ("burn", N_("Burn Captions Into Video")), ("sidecar", N_("Create Sidecar File")))
CAPTION_FORMATS = (("srt", N_("SubRip Subtitle (.srt)")), ("vtt", N_("WebVTT (.vtt)")))
PLACEMENTS = (
    ("topLeft", N_("Top Left")), ("topCenter", N_("Top Center")), ("topRight", N_("Top Right")),
    ("centerLeft", N_("Center Left")), ("center", N_("Center")), ("centerRight", N_("Center Right")),
    ("bottomLeft", N_("Bottom Left")), ("bottomCenter", N_("Bottom Center")), ("bottomRight", N_("Bottom Right")),
)
# the engine's defaults (crates/export/src/settings.rs), used until it answers
DEFAULT_EFFECTS: dict[str, Any] = {
    "image_overlay": {"enabled": False, "path": "", "placement": "topRight", "offset": [0.0, 0.0], "size_percent": 15.0, "opacity": 100.0},
    "name_overlay": {"enabled": False, "text": "", "placement": "bottomCenter", "offset": [0.0, 0.0], "size_percent": 5.0, "opacity": 100.0},
    "timecode_overlay": {"enabled": False, "text": "", "placement": "bottomCenter", "offset": [0.0, 0.0], "size_percent": 5.0, "opacity": 100.0},
    "video_limiter": {"enabled": False, "min_percent": 0.0, "max_percent": 100.0},
    "loudness": {"enabled": False, "target_lufs": -23.0, "true_peak_dbtp": -2.0},
}


def caption_mode(settings: dict[str, Any]) -> tuple[str, str]:
    """(mode, sidecar format) of resolved settings, as egui's Export Options shows them."""
    if settings.get("burnCaptions") is True:
        return "burn", "srt"
    side = settings.get("captionSidecar")
    if isinstance(side, str) and side:
        return "sidecar", "vtt" if side == "vtt" else "srt"
    return "none", "srt"


def _spin(parent: QWidget, name: str, lo: float, hi: float, step: float, suffix: str, decimals: int) -> QDoubleSpinBox:
    s = QDoubleSpinBox(parent)
    s.setObjectName(name)
    s.setRange(lo, hi)
    s.setSingleStep(step)
    s.setDecimals(decimals)
    s.setSuffix(suffix)
    return s


def _combo(parent: QWidget, name: str, choices: tuple[tuple[str, str], ...]) -> QComboBox:
    c = QComboBox(parent)
    c.setObjectName(name)
    for value, text in choices:
        c.addItem(tr(text), value)
    return c


def _set(w: QWidget, value: Any) -> None:
    w.blockSignals(True)
    if isinstance(w, QCheckBox):
        w.setChecked(bool(value))
    elif isinstance(w, QComboBox):
        i = w.findData(value)
        if i >= 0:
            w.setCurrentIndex(i)
    elif isinstance(w, QDoubleSpinBox) and isinstance(value, (int, float)) and not isinstance(value, bool):
        w.setValue(float(value))
    elif isinstance(w, QLineEdit) and isinstance(value, str):
        w.setText(value)
    w.blockSignals(False)


# egui's Export settings sections (`export_mode::section`), in their order; Video and Audio start open
SECTION_KEYS = ("video", "audio", "multiplexer", "captions", "effects", "metadata")
OPEN_SECTIONS = ("video", "audio")


class Section(QGroupBox):
    """A collapsible Export settings section as egui's: its title (``export.section.<key>``)
    opens and closes the body below it. The box itself shows while the format has the section."""

    opened = Signal(str, bool)

    def __init__(self, key: str, title: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.key = key
        self.head = QToolButton(self)
        self.head.setObjectName(f"export.section.{key}")
        self.head.setText(title)
        self.head.setCheckable(True)
        self.head.setAutoRaise(True)
        self.head.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        self.body = QWidget(self)
        outer = QVBoxLayout(self)
        outer.setContentsMargins(6, 2, 6, 6)
        outer.setSpacing(2)
        outer.addWidget(self.head)
        outer.addWidget(self.body)
        self.head.toggled.connect(self._toggled)
        self.head.setChecked(key in OPEN_SECTIONS)
        self._toggled(self.head.isChecked())

    def _toggled(self, on: bool) -> None:
        self.body.setVisible(on)
        self.head.setArrowType(Qt.ArrowType.DownArrow if on else Qt.ArrowType.RightArrow)
        self.opened.emit(self.key, on)

    def is_open(self) -> bool:
        return self.head.isChecked()

    def set_open(self, on: bool) -> None:
        self.head.setChecked(on)


class CaptionsSection(Section):
    changed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__("captions", tr("Captions"), parent)
        self.setObjectName("export.captions")
        f = QFormLayout(self.body)
        self.mode = _combo(self, "export.captions.mode", CAPTION_MODES)
        self.format = _combo(self, "export.captions.format", CAPTION_FORMATS)
        f.addRow(tr("Export Options"), self.mode)
        f.addRow(tr("File Format"), self.format)
        self.form = f
        self.mode.currentIndexChanged.connect(lambda _i: (self._sync(), self.changed.emit()))
        self.format.currentIndexChanged.connect(lambda _i: self.changed.emit())
        self._sync()

    def _sync(self) -> None:
        self.form.setRowVisible(self.format, self.mode.currentData() == "sidecar")

    def show_settings(self, settings: dict[str, Any]) -> None:
        mode, fmt = caption_mode(settings)
        _set(self.mode, mode)
        _set(self.format, fmt)
        self._sync()

    def params(self) -> dict[str, Any]:
        mode = self.mode.currentData()
        return {"burnCaptions": mode == "burn", "captionSidecar": self.format.currentData() if mode == "sidecar" else "none"}


class EffectsSection(Section):
    """Export ▸ Effects: each effect's switch, and its settings while it is on."""

    changed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__("effects", tr("Effects"), parent)
        self.setObjectName("export.effects")
        self.seen: dict[str, Any] = copy.deepcopy(DEFAULT_EFFECTS)  # the last resolved effects (offsets kept)
        # (settings key, field) -> widget
        self.fields: dict[tuple[str, str], QWidget] = {}
        self.switches: dict[str, QCheckBox] = {}
        self.bodies: dict[str, QWidget] = {}
        self.kinds = (True, True)  # (video, audio) of the chosen format
        lay = QVBoxLayout(self.body)
        # pick the image overlay's file (tests replace it): (title, filter, start) -> path or ""
        self.pick_image = lambda t, f, s: QFileDialog.getOpenFileName(self, t, s, f)[0]

        img = self._effect(lay, "image_overlay", "imageOverlay", N_("Image Overlay"))
        path = QLineEdit(img)
        path.setObjectName("export.effects.imageOverlay.path")
        path.setPlaceholderText(tr("PNG or JPEG file"))
        browse = QPushButton(tr("Browse…"), img)
        browse.setObjectName("export.effects.imageOverlay.browse")
        browse.clicked.connect(self.browse_image)
        row = QHBoxLayout()
        row.addWidget(browse)
        row.addWidget(path, 1)
        self.fields[("image_overlay", "path")] = path
        img.layout().addRow(tr("Image"), row)  # type: ignore[union-attr]
        self._placement(img, "image_overlay", "imageOverlay", 100.0, 0.5)
        name = self._effect(lay, "name_overlay", "nameOverlay", N_("Name Overlay"))
        self._text(name, "name_overlay", "nameOverlay", N_("Name"))
        self._placement(name, "name_overlay", "nameOverlay", 50.0, 0.2)
        tc = self._effect(lay, "timecode_overlay", "timecodeOverlay", N_("Timecode Overlay"))
        self._text(tc, "timecode_overlay", "timecodeOverlay", N_("Prefix"))
        self._placement(tc, "timecode_overlay", "timecodeOverlay", 50.0, 0.2)
        lim = self._effect(lay, "video_limiter", "videoLimiter", N_("Video Limiter"))
        lo = _spin(lim, "export.effects.videoLimiter.min", 0.0, 50.0, 0.5, " %", 1)
        hi = _spin(lim, "export.effects.videoLimiter.max", 50.0, 100.0, 0.5, " %", 1)
        self.fields[("video_limiter", "min_percent")] = lo
        self.fields[("video_limiter", "max_percent")] = hi
        pair = QHBoxLayout()
        pair.addWidget(lo)
        pair.addWidget(hi)
        lim.layout().addRow(tr("Luma Min / Max"), pair)  # type: ignore[union-attr]
        loud = self._effect(lay, "loudness", "loudness", N_("Loudness Normalization"))
        target = _spin(loud, "export.effects.loudness.target", -40.0, -5.0, 0.1, " LUFS", 1)
        peak = _spin(loud, "export.effects.loudness.truePeak", -9.0, 0.0, 0.1, " dBTP", 1)
        self.fields[("loudness", "target_lufs")] = target
        self.fields[("loudness", "true_peak_dbtp")] = peak
        loud.layout().addRow(tr("Target Loudness"), target)  # type: ignore[union-attr]
        loud.layout().addRow(tr("True Peak Limit"), peak)  # type: ignore[union-attr]

        for w in self.fields.values():
            if isinstance(w, QLineEdit):
                w.textEdited.connect(lambda _t: self.changed.emit())
            elif isinstance(w, QComboBox):
                w.currentIndexChanged.connect(lambda _i: self.changed.emit())
            elif isinstance(w, QDoubleSpinBox):
                w.valueChanged.connect(lambda _v: self.changed.emit())
        self.show_effects(self.seen)

    def _effect(self, lay: QVBoxLayout, key: str, auto: str, label: str) -> QWidget:
        on = QCheckBox(tr(label), self)
        on.setObjectName(f"export.effects.{auto}")
        body = QWidget(self)
        f = QFormLayout(body)
        f.setContentsMargins(18, 0, 0, 4)
        on.toggled.connect(lambda _v: (self._sync(), self.changed.emit()))
        lay.addWidget(on)
        lay.addWidget(body)
        self.switches[key] = on
        self.bodies[key] = body
        return body

    def _text(self, body: QWidget, key: str, auto: str, label: str) -> None:
        t = QLineEdit(body)
        t.setObjectName(f"export.effects.{auto}.text")
        self.fields[(key, "text")] = t
        body.layout().addRow(tr(label), t)  # type: ignore[union-attr]

    def _placement(self, body: QWidget, key: str, auto: str, max_size: float, step: float) -> None:
        pos = _combo(body, f"export.effects.{auto}.position", PLACEMENTS)
        size = _spin(body, f"export.effects.{auto}.size", 1.0, max_size, step, " %", 1)
        opacity = _spin(body, f"export.effects.{auto}.opacity", 0.0, 100.0, 1.0, " %", 0)
        self.fields[(key, "placement")] = pos
        self.fields[(key, "size_percent")] = size
        self.fields[(key, "opacity")] = opacity
        pair = QHBoxLayout()
        pair.addWidget(size)
        pair.addWidget(opacity)
        body.layout().addRow(tr("Position"), pos)  # type: ignore[union-attr]
        body.layout().addRow(tr("Size / Opacity"), pair)  # type: ignore[union-attr]

    def browse_image(self) -> None:
        path = self.pick_image(tr("Image Overlay"), tr("Images") + " (*.png *.jpg *.jpeg)", self.fields[("image_overlay", "path")].text())  # type: ignore[attr-defined]
        if path:
            _set(self.fields[("image_overlay", "path")], path)
            self.changed.emit()

    def set_kinds(self, video: bool, audio: bool) -> None:
        """The video effects for formats with picture, loudness for sound (egui's rule)."""
        self.kinds = (video, audio)
        self._sync()

    def _sync(self) -> None:
        video, audio = self.kinds
        for key, on in self.switches.items():
            shown = audio if key == "loudness" else video
            on.setVisible(shown)
            self.bodies[key].setVisible(shown and on.isChecked())

    def show_effects(self, effects: dict[str, Any]) -> None:
        for key, default in DEFAULT_EFFECTS.items():
            got = effects.get(key)
            self.seen[key] = {**default, **got} if isinstance(got, dict) else copy.deepcopy(default)
        for key, on in self.switches.items():
            on.blockSignals(True)
            on.setChecked(bool(self.seen[key].get("enabled")))
            on.blockSignals(False)
        for (key, field), w in self.fields.items():
            _set(w, self.seen[key].get(field))
        self._sync()

    def values(self) -> dict[str, Any]:
        """The effects as the widgets show them (``settings.effects``)."""
        out = copy.deepcopy(self.seen)
        for key, on in self.switches.items():
            out[key]["enabled"] = on.isChecked()
        for (key, field), w in self.fields.items():
            if isinstance(w, QLineEdit):
                out[key][field] = w.text()
            elif isinstance(w, QComboBox):
                out[key][field] = w.currentData()
            elif isinstance(w, QDoubleSpinBox):
                out[key][field] = w.value()
        return out



METADATA_FIELDS = (("title", N_("Title")), ("creator", N_("Creator")), ("copyright", N_("Copyright")),
                   ("description", N_("Description")), ("comment", N_("Comment")))
# formats whose files carry the metadata (egui's Metadata section)
METADATA_FORMATS = {"h264", "hevc", "av1", "prores", "dnxhr", "apv", "cineform", "mjpeg"}


class MetadataSection(Section):
    """Export ▸ Metadata: title, creator, copyright, description and comment written into the file."""

    changed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__("metadata", tr("Metadata"), parent)
        self.setObjectName("export.metadata")
        f = QFormLayout(self.body)
        self.fields: dict[str, QLineEdit] = {}
        for key, label in METADATA_FIELDS:
            e = QLineEdit(self)
            e.setObjectName(f"export.metadata.{key}")
            e.textEdited.connect(lambda _t: self.changed.emit())
            f.addRow(tr(label), e)
            self.fields[key] = e

    def show_metadata(self, metadata: dict[str, Any]) -> None:
        for key, e in self.fields.items():
            _set(e, str(metadata.get(key) or ""))

    def values(self) -> dict[str, str]:
        return {key: e.text() for key, e in self.fields.items()}
