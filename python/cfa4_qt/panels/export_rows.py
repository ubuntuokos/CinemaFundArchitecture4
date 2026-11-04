"""More of the Export window's Video and Audio rows (egui's ``export_mode`` video and audio
sections): Frame Rate, Field Order, Aspect, the H.264 Profile and Level, Quality (CRF), Maximum
Bitrate, GPU Rendering, Render at Maximum Depth, Use Maximum Render Quality; Sample Rate,
Channels, Sample Size and the FLAC Compression Level.

Each row shows the resolved setting (``export.resolve``'s camelCase ``settings``; ``audio`` keeps
its snake_case keys) until the user changes it, and only a changed row goes into the
``settings`` patch. Automation ids as egui's.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QObject, Signal
from PySide6.QtWidgets import QCheckBox, QComboBox, QFormLayout, QSpinBox, QWidget

from ..i18n import N_, tr

# Frame Rate: (label, {num, den}); None = the sequence's (egui's RATES)
RATES: tuple[tuple[str, Any], ...] = (
    (N_("Match Source"), None),
    ("10", {"num": 10, "den": 1}), ("12", {"num": 12, "den": 1}), ("15", {"num": 15, "den": 1}),
    ("23.976", {"num": 24000, "den": 1001}), ("24", {"num": 24, "den": 1}), ("25", {"num": 25, "den": 1}),
    ("29.97", {"num": 30000, "den": 1001}), ("30", {"num": 30, "den": 1}), ("50", {"num": 50, "den": 1}),
    ("59.94", {"num": 60000, "den": 1001}), ("60", {"num": 60, "den": 1}),
)
FIELD_ORDERS = ((N_("Progressive"), "progressive"), (N_("Upper First"), "upperFirst"), (N_("Lower First"), "lowerFirst"))
# pixel aspect ratios (egui's PARS): None = square
PARS: tuple[tuple[str, Any], ...] = (
    (N_("Square Pixels (1.0)"), None), (N_("D1/DV NTSC (0.9091)"), [10, 11]), (N_("D1/DV PAL (1.0940)"), [59, 54]),
    (N_("HD Anamorphic 1080 (1.333)"), [4, 3]), (N_("DVCPRO HD (1.5)"), [3, 2]), (N_("Anamorphic 2:1 (2.0)"), [2, 1]),
)
H264_PROFILES = (("Baseline", "baseline"), ("Main", "main"), ("High", "high"))
H264_LEVELS: tuple[tuple[str, Any], ...] = ((N_("Auto"), None),) + tuple((f"{lv // 10}.{lv % 10}", lv) for lv in (30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62))
SAMPLE_RATES: tuple[tuple[str, Any], ...] = ((N_("Match Source"), None),) + tuple((f"{r} Hz", r) for r in (32_000, 44_100, 48_000, 96_000))
CHANNELS = ((N_("Mono"), 1), (N_("Stereo"), 2), ("5.1", 6))
# Sample Size: (label, (bits, float))
SAMPLE_SIZES: tuple[tuple[str, Any], ...] = ((N_("16 bit"), "16"), (N_("24 bit"), "24"), (N_("32 bit"), "32"), (N_("32 bit float"), "32f"))
MP4_VIDEO = {"h264", "hevc", "av1"}
AUDIO_ONLY_PCM = {"wav", "aiff"}


def _get(settings: dict[str, Any], path: tuple[str, ...]) -> Any:
    v: Any = settings
    for k in path:
        if not isinstance(v, dict):
            return None
        v = v.get(k)
    return v


@dataclass
class Row:
    key: str  # the touched key
    widget: QWidget
    path: tuple[str, ...]  # where the setting is in `settings`
    read: Callable[[], Any]  # the widget's value as the setting's JSON
    show: Callable[[Any], None]  # show a resolved value


def _combo(parent: QWidget, name: str, choices: tuple[tuple[str, Any], ...], translate: bool = True) -> QComboBox:
    c = QComboBox(parent)
    c.setObjectName(name)
    for label, value in choices:
        c.addItem(tr(label) if translate else label, value)
    return c


def _show_combo(c: QComboBox, value: Any) -> None:
    for i in range(c.count()):
        if c.itemData(i) == value:
            c.setCurrentIndex(i)
            return


class ExportRows(QObject):
    """The rows; ``changed(key)`` when the user changes one."""

    changed = Signal(str)

    def __init__(self, parent: QWidget, video: QFormLayout, audio: QFormLayout) -> None:
        super().__init__(parent)
        self.video_form, self.audio_form = video, audio
        self.rows: dict[str, Row] = {}
        self.fps = _combo(parent, "export.video.fps", RATES)
        self.field_order = _combo(parent, "export.video.fieldOrder", FIELD_ORDERS)
        # CFA4's encoders write progressive frames (egui greys the others out)
        for i in range(1, self.field_order.count()):
            item = self.field_order.model().item(i)  # type: ignore[attr-defined]
            if item is not None:
                item.setEnabled(False)
        self.aspect = _combo(parent, "export.video.aspect", PARS)
        self.profile = _combo(parent, "export.video.profile", H264_PROFILES, translate=False)
        self.level = _combo(parent, "export.video.level", H264_LEVELS)
        self.crf = QSpinBox(parent)
        self.crf.setObjectName("export.video.crf")
        self.crf.setRange(0, 51)
        self.max_bitrate = QSpinBox(parent)
        self.max_bitrate.setObjectName("export.video.max")
        self.max_bitrate.setRange(0, 1_000_000)
        self.max_bitrate.setSuffix(" kbps")
        self.max_bitrate.setSpecialValueText(tr("From the preset"))
        self.gpu = QCheckBox(tr("Composite on the GPU when available"), parent)
        self.gpu.setObjectName("export.gpuRendering")
        self.max_depth = QCheckBox(tr("Render at Maximum Depth"), parent)
        self.max_depth.setObjectName("export.video.maxDepth")
        self.max_quality = QCheckBox(tr("Use Maximum Render Quality"), parent)
        self.max_quality.setObjectName("export.video.maxQuality")
        self.sample_rate = _combo(parent, "export.audio.sampleRate", SAMPLE_RATES)
        self.channels = _combo(parent, "export.audio.channels", CHANNELS)
        self.bits = _combo(parent, "export.audio.bits", SAMPLE_SIZES)
        self.flac_level = QSpinBox(parent)
        self.flac_level.setObjectName("export.audio.flacLevel")
        self.flac_level.setRange(0, 8)
        self.flac_level.setToolTip(tr("0 fastest, 8 smallest"))

        def combo_row(key: str, c: QComboBox, path: tuple[str, ...]) -> Row:
            return Row(key, c, path, lambda: c.currentData(), lambda v: _show_combo(c, v))

        def spin_row(key: str, s: QSpinBox, path: tuple[str, ...]) -> Row:
            return Row(key, s, path, lambda: s.value(), lambda v: s.setValue(int(v)) if isinstance(v, (int, float)) and not isinstance(v, bool) else None)

        def check_row(key: str, b: QCheckBox, path: tuple[str, ...], on: Any = True, off: Any = False) -> Row:
            return Row(key, b, path, lambda: on if b.isChecked() else off, lambda v: b.setChecked(v == on))

        for row in (
            combo_row("fps", self.fps, ("frameRate",)),
            combo_row("fieldOrder", self.field_order, ("fieldOrder",)),
            combo_row("aspect", self.aspect, ("pixelAspect",)),
            combo_row("h264Profile", self.profile, ("h264Profile",)),
            combo_row("h264Level", self.level, ("h264Level",)),
            spin_row("crf", self.crf, ("crf",)),
            Row("maxBitrate", self.max_bitrate, ("maxBitrateKbps",), lambda: self.max_bitrate.value() or None,
                lambda v: self.max_bitrate.setValue(int(v) if isinstance(v, int) else 0)),
            check_row("gpu", self.gpu, ("gpuRendering",), "auto", "off"),
            check_row("maxDepth", self.max_depth, ("renderAtMaxDepth",)),
            check_row("maxQuality", self.max_quality, ("maxRenderQuality",)),
            combo_row("sampleRate", self.sample_rate, ("audio", "sample_rate")),
            combo_row("channels", self.channels, ("audio", "channels")),
            spin_row("flacLevel", self.flac_level, ("audio", "flac_level")),
        ):
            self.rows[row.key] = row
        self.rows["bits"] = Row("bits", self.bits, ("audio",), lambda: self.bits.currentData(),
                                lambda a: _show_combo(self.bits, f"{a.get('bits')}{'f' if a.get('float') is True else ''}") if isinstance(a, dict) else None)

        video.addRow(tr("Frame Rate"), self.fps)
        video.addRow(tr("Field Order"), self.field_order)
        video.addRow(tr("Aspect"), self.aspect)
        video.addRow(tr("Profile"), self.profile)
        video.addRow(tr("Level"), self.level)
        video.addRow(tr("Quality (CRF)"), self.crf)
        video.addRow(tr("Maximum Bitrate"), self.max_bitrate)
        video.addRow(tr("GPU Rendering"), self.gpu)
        video.addRow(self.max_depth)
        video.addRow(self.max_quality)
        audio.addRow(tr("Sample Rate"), self.sample_rate)
        audio.addRow(tr("Channels"), self.channels)
        audio.addRow(tr("Sample Size"), self.bits)
        audio.addRow(tr("Compression Level"), self.flac_level)

        for key, row in self.rows.items():
            w = row.widget
            if isinstance(w, QComboBox):
                w.currentIndexChanged.connect(lambda _i, k=key: self.changed.emit(k))
            elif isinstance(w, QSpinBox):
                w.valueChanged.connect(lambda _v, k=key: self.changed.emit(k))
            elif isinstance(w, QCheckBox):
                w.toggled.connect(lambda _on, k=key: self.changed.emit(k))

    def show_settings(self, settings: dict[str, Any], touched: set[str]) -> None:
        for key, row in self.rows.items():
            if key in touched:
                continue
            row.widget.blockSignals(True)
            row.show(_get(settings, row.path))
            row.widget.blockSignals(False)
        if "crf" not in touched and not isinstance(settings.get("crf"), (int, float)):
            self.crf.blockSignals(True)
            self.crf.setValue(23)  # the engine's DEFAULT_CRF where the preset has none
            self.crf.blockSignals(False)

    def patch(self, touched: set[str], shown: dict[str, bool]) -> dict[str, Any]:
        """The changed rows as a ``settings`` patch (rows the format does not show are left out)."""
        out: dict[str, Any] = {}
        for key, row in self.rows.items():
            if key not in touched or not shown.get(key, True):
                continue
            v = row.read()
            if key == "bits":
                size = str(v or "16")
                out.setdefault("audio", {}).update({"bits": int(size.rstrip("f")), "float": size.endswith("f")})
            elif row.path[0] == "audio":
                out.setdefault("audio", {})[row.path[1]] = v
            else:
                out[row.path[0]] = v
        return out

    def visible_rows(self, fmt: str, enc: str, has_video: bool, has_audio: bool, bitrate_mode: str, audio_codec: str) -> dict[str, bool]:
        """Which rows the format shows (egui's rules)."""
        mp4_video = enc in MP4_VIDEO
        pcm_or_flac = audio_codec in ("pcm", "flac") or fmt in AUDIO_ONLY_PCM | {"flac"} or fmt.startswith("mxf")
        flac = audio_codec == "flac" or fmt == "flac"
        return {
            "fps": has_video, "fieldOrder": has_video, "aspect": has_video,
            "h264Profile": enc == "h264", "h264Level": enc == "h264",
            "crf": mp4_video and bitrate_mode == "crf",
            "maxBitrate": mp4_video and bitrate_mode not in ("cbr", "crf"),
            "gpu": has_video, "maxDepth": has_video, "maxQuality": has_video,
            "sampleRate": has_audio, "channels": has_audio,
            "bits": has_audio and pcm_or_flac, "flacLevel": has_audio and flac,
        }

    def sync(self, shown: dict[str, bool]) -> None:
        for key, row in self.rows.items():
            form = self.audio_form if row.path[0] == "audio" else self.video_form
            form.setRowVisible(row.widget, shown.get(key, True))
