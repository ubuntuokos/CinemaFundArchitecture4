"""More of the Export window's Video and Audio rows (egui's ``export_mode`` video and audio
sections): Frame Rate, Field Order, Aspect, the H.264 Profile and Level (H.265 and AV1: the
profile and the encoder, as text), Quality (CRF), Maximum Bitrate (Mbps), Key Frame Distance (off,
or every so many frames), GPU Rendering, Render at Maximum Depth, Use Maximum Render Quality;
Sample Rate, Channels, Sample Size (the sizes the codec and container cannot store greyed out,
saying why) and the FLAC Compression Level.

Each row shows the resolved setting (``export.resolve``'s camelCase ``settings``; ``audio`` keeps
its snake_case keys) until the user changes it, and only a changed row goes into the
``settings`` patch. Automation ids as egui's.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QObject, Qt, Signal
from PySide6.QtWidgets import QCheckBox, QComboBox, QDoubleSpinBox, QFormLayout, QHBoxLayout, QLabel, QSpinBox, QWidget

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
# H.265 and AV1 have one profile each, the level chosen by the encoder: (SDR text, HDR text)
ONE_PROFILE = {
    "hevc": (N_("Main (8-bit; Main 10 for HDR sequences)"), N_("Main 10 (10-bit, HDR)")),
    "av1": (N_("Main (8-bit; 10-bit for HDR sequences)"), N_("Main (10-bit, HDR)")),
}
SIZE_WHY_FLAC = N_("FLAC stores whole-number samples only")
SIZE_WHY_MXF = N_("MXF export writes integer PCM only")
SIZE_WHY = N_("Not supported by CFA4's encoders yet")
KEYFRAME_DEFAULT = 48  # egui's distance when Key Frame Distance is turned on
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
        # egui's Frame Rate row: Match Source, else the rate (29.97 when turned off)
        self.match_rate = QCheckBox(tr("Match Source"), parent)
        self.match_rate.setObjectName("export.video.matchRate")
        self.fps_row = QWidget(parent)
        fps_lay = QHBoxLayout(self.fps_row)
        fps_lay.setContentsMargins(0, 0, 0, 0)
        fps_lay.addWidget(self.match_rate)
        fps_lay.addWidget(self.fps)
        fps_lay.addStretch(1)
        self.match_rate.toggled.connect(self._match_rate_toggled)
        self.fps.currentIndexChanged.connect(lambda _i: self._sync_rate_shown())
        self.field_order = _combo(parent, "export.video.fieldOrder", FIELD_ORDERS)
        self.allow_interlaced(False)
        self.aspect = _combo(parent, "export.video.aspect", PARS)
        self.profile = _combo(parent, "export.video.profile", H264_PROFILES, translate=False)
        self.level = _combo(parent, "export.video.level", H264_LEVELS)
        self.crf = QSpinBox(parent)
        self.crf.setObjectName("export.video.crf")
        self.crf.setRange(0, 51)
        # in Mbps as egui's; 0 = the engine's 1.5 x target
        self.max_bitrate = QDoubleSpinBox(parent)
        self.max_bitrate.setObjectName("export.video.max")
        self.max_bitrate.setRange(0.0, 1000.0)
        self.max_bitrate.setDecimals(1)
        self.max_bitrate.setSingleStep(0.1)
        self.max_bitrate.setSuffix(" Mbps")
        self.max_bitrate.setSpecialValueText(tr("From the preset"))
        # Key Frame Distance as egui's: off (the encoder's two seconds) or every so many frames
        self.keyframe_box = QWidget(parent)
        self.keyframe_on = QCheckBox(self.keyframe_box)
        self.keyframe_on.setObjectName("export.video.keyframeOn")
        self.keyframe = QSpinBox(self.keyframe_box)
        self.keyframe.setObjectName("export.video.keyframe")
        self.keyframe.setRange(1, 600)
        self.keyframe.setValue(KEYFRAME_DEFAULT)
        self.keyframe.setSuffix(" " + tr("frames"))
        kf = QHBoxLayout(self.keyframe_box)
        kf.setContentsMargins(0, 0, 0, 0)
        kf.addWidget(self.keyframe_on)
        kf.addWidget(self.keyframe, 1)

        def show_keyframe(v: Any) -> None:
            on = isinstance(v, int) and not isinstance(v, bool) and v > 0
            self.keyframe_on.blockSignals(True)
            self.keyframe_on.setChecked(on)
            self.keyframe_on.blockSignals(False)
            self.keyframe.blockSignals(True)
            self.keyframe.setValue(v if on else KEYFRAME_DEFAULT)
            self.keyframe.blockSignals(False)
            self.keyframe.setVisible(on)
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
        # H.265 / AV1: egui's Profile and Encoder rows are text
        self.profile_info = QLabel(parent)
        self.profile_info.setObjectName("export.video.profileInfo")
        self.encoder_info = QLabel(tr("Software, or hardware where available"), parent)
        self.encoder_info.setObjectName("export.video.encoder")
        self.video_format = ""  # the encoder that writes the video (export.resolve's videoFormat)
        self.hdr = False

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
            Row("maxBitrate", self.max_bitrate, ("maxBitrateKbps",), lambda: round(self.max_bitrate.value() * 1000) or None,
                lambda v: self.max_bitrate.setValue(v / 1000 if isinstance(v, int) and not isinstance(v, bool) else 0.0)),
            Row("keyframe", self.keyframe_box, ("keyframeDistance",),
                lambda: self.keyframe.value() if self.keyframe_on.isChecked() else None, show_keyframe),
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

        video.addRow(tr("Frame Rate"), self.fps_row)
        self._sync_rate_shown()
        video.addRow(tr("Field Order"), self.field_order)
        video.addRow(tr("Aspect"), self.aspect)
        video.addRow(tr("Profile"), self.profile)
        video.addRow(tr("Level"), self.level)
        video.addRow(tr("Profile"), self.profile_info)
        video.addRow(tr("Encoder"), self.encoder_info)
        video.addRow(tr("Quality (CRF)"), self.crf)
        video.addRow(tr("Maximum Bitrate"), self.max_bitrate)
        video.addRow(tr("Key Frame Distance"), self.keyframe_box)
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
            elif isinstance(w, (QSpinBox, QDoubleSpinBox)):
                w.valueChanged.connect(lambda _v, k=key: self.changed.emit(k))
            elif isinstance(w, QCheckBox):
                w.toggled.connect(lambda _on, k=key: self.changed.emit(k))

        self.keyframe.valueChanged.connect(lambda _v: self.changed.emit("keyframe"))
        self.keyframe_on.toggled.connect(self._keyframe_toggled)

    def _keyframe_toggled(self, on: bool) -> None:
        self.keyframe.setVisible(on)
        self.changed.emit("keyframe")

    def allow_interlaced(self, on: bool) -> None:
        """The interlaced field orders only for MPEG-2, the one encoder that codes interlaced video
        (MPEG-2 files, and MPEG-2, XDCAM HD422 or D-10 in MXF); the others write progressive frames."""
        model = self.field_order.model()
        for i in range(1, self.field_order.count()):
            item = model.item(i)  # type: ignore[attr-defined]
            if item is not None:
                item.setEnabled(on)

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
        self._sync_rate_shown()

    def show_resolved(self, resolved: dict[str, Any]) -> None:
        """What ``export.resolve`` says beyond the settings: the H.265 / AV1 profile text (HDR or
        not) and which sample sizes the codec and container can store, with egui's reasons."""
        self.video_format = str(resolved.get("videoFormat") or "")
        self.hdr = resolved.get("hdr") is True
        texts = ONE_PROFILE.get(self.video_format)
        if texts is not None:
            self.profile_info.setText(tr(texts[1] if self.hdr else texts[0]))
        sizes = resolved.get("sampleSizes")
        if not isinstance(sizes, list):
            return
        settings = resolved.get("settings") if isinstance(resolved.get("settings"), dict) else {}
        fmt = str(settings.get("format") or "")
        why = SIZE_WHY_FLAC if resolved.get("audioCodec") == "flac" else SIZE_WHY_MXF if fmt.startswith("mxf") else SIZE_WHY
        model = self.bits.model()
        for size in sizes:
            if not isinstance(size, dict):
                continue
            key = f"{size.get('bits')}{'f' if size.get('float') is True else ''}"
            i = self.bits.findData(key)
            item = model.item(i) if i >= 0 else None  # type: ignore[attr-defined]
            if item is None:
                continue
            ok = size.get("supported") is not False
            item.setEnabled(ok)
            item.setData(None if ok else tr(why), Qt.ItemDataRole.ToolTipRole)

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
            "profileInfo": has_video and enc in ONE_PROFILE, "encoder": has_video and enc in ONE_PROFILE,
            "crf": mp4_video and bitrate_mode == "crf",
            "maxBitrate": mp4_video and bitrate_mode not in ("cbr", "crf"),
            "keyframe": mp4_video,
            "gpu": has_video, "maxDepth": has_video, "maxQuality": has_video,
            "sampleRate": has_audio, "channels": has_audio,
            "bits": has_audio and pcm_or_flac, "flacLevel": has_audio and flac,
        }

    def _match_rate_toggled(self, on: bool) -> None:
        _show_combo(self.fps, None if on else {"num": 30000, "den": 1001})
        self._sync_rate_shown()

    def _sync_rate_shown(self) -> None:
        """The rate shows only while it does not match the source."""
        match = self.fps.currentData() is None
        self.match_rate.blockSignals(True)
        self.match_rate.setChecked(match)
        self.match_rate.blockSignals(False)
        self.fps.setVisible(not match)

    def sync(self, shown: dict[str, bool]) -> None:
        for key, row in self.rows.items():
            form = self.audio_form if row.path[0] == "audio" else self.video_form
            form.setRowVisible(self.fps_row if row.widget is self.fps else row.widget, shown.get(key, True))
        for key, w in (("profileInfo", self.profile_info), ("encoder", self.encoder_info)):
            self.video_form.setRowVisible(w, shown.get(key, False))
