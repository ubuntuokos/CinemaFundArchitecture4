"""Sequence ▸ Sequence Settings… and File ▸ New ▸ Sequence…, as egui's
``panels/sequence_settings.rs`` (Premiere Pro's dialog): General, Color Management and VR
Properties tabs, or General and Tracks for New Sequence; Cancel / OK.

OK sends one ``sequence.settings`` with what changed (nothing changed: no undo step), or
``file.newSequence`` (then ``sequence.settings`` for drop-frame and Maximum Render Quality, which
``file.newSequence`` has no parameter for). Premiere settings CFA4 has no equivalent for yet are
greyed out with their value and a "not supported yet" hint.

Automation ids (egui's, ``sequenceSettings.`` + …): ``tab.general``, ``tab.color``, ``tab.vr``,
``tab.tracks``; ``name``, ``editingMode``, ``timebase``, ``width``, ``height``, ``aspect``,
``scaleMotion``, ``par``, ``fields``, ``videoDisplay``, ``channelFormat``, ``channels``,
``sampleRate``, ``audioDisplay``, ``previewFormat``, ``previewCodec``, ``previewWidth``,
``previewHeight``, ``maxBitDepth``, ``maxRenderQuality``, ``linearColor``; ``workingSpace``,
``wideGamut``, ``autoToneMap``; ``vr.projection``, ``vr.layout``, ``vr.horizontal``,
``vr.vertical``; ``tracks.video``, ``tracks.audio``; ``ok``, ``cancel``.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QComboBox,
    QDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QStackedWidget,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr

NOT_YET = N_("Not supported in CFA4 yet")
LINEAR = N_("CFA4 always composites in linear light")
MAX_FRAME_SIDE = 32_768  # cfa4_project::MAX_FRAME_SIDE
MAX_TRACKS = 256
TIMEBASES = [
    (10, 1), (12, 1), (25, 2), (15, 1), (24_000, 1001), (24, 1), (25, 1), (30_000, 1001),
    (30, 1), (48, 1), (50, 1), (60_000, 1001), (60, 1), (120_000, 1001), (120, 1),
]  # fmt: skip
SAMPLE_RATES = [32_000, 44_100, 48_000, 88_200, 96_000]
MIXES = [N_("Stereo"), N_("Mono"), "5.1", N_("Adaptive")]
MIX_NAMES = {"Mono": "Mono", "Stereo": "Stereo", "Surround51": "5.1", "Adaptive": "Adaptive"}
WORKING_SPACES = [("rec709", N_("Rec. 709")), ("rec2100-pq", N_("Rec. 2100 PQ")), ("rec2100-hlg", N_("Rec. 2100 HLG"))]
WORKING_IDS = {"Rec709": "rec709", "Rec2100Pq": "rec2100-pq", "Rec2100Hlg": "rec2100-hlg"}


def _int(v: Any, default: int, lo: int = 1, hi: int = 2**31 - 1) -> int:
    return v if isinstance(v, int) and not isinstance(v, bool) and lo <= v <= hi else default


def _bool(v: Any, default: bool = False) -> bool:
    return v if isinstance(v, bool) else default


def sane_rate(num: int, den: int) -> tuple[int, int]:
    return (num, den) if num > 0 and den > 0 else (24_000, 1001)


def supports_drop_frame(num: int, den: int) -> bool:
    """cfa4_time's ``FrameRate::supports_drop_frame``: 29.97, 59.94, 119.88."""
    num, den = sane_rate(num, den)
    base = max(1, num // den + (1 if num % den else 0))
    return den == 1001 and base % 30 == 0


def rate_label(num: int, den: int) -> str:
    """cfa4_time's ``FrameRate::label``: "24", "23.976", "12.5"."""
    num, den = sane_rate(num, den)
    if den == 1:
        return str(num)
    return f"{num / den:.3f}".rstrip("0").rstrip(".")


def aspect(w: int, h: int) -> str:
    """"16:9", or "1.90:1" when the reduced ratio has no small integer form."""
    g = max(1, math.gcd(w, h))
    a, b = w // g, h // g
    if a <= 64 and b <= 64:
        return f"{a}:{b}"
    return f"{w / h:.2f}:1" if h > 0 else ""


def display_width(w: int, h: int, par: tuple[int, int]) -> int:
    """The width a ``w`` x ``h`` frame with ``par`` pixels shows at, in square pixels."""
    if par[0] <= 0 or par[1] <= 0:
        return w
    return min(2**32 - 1, max(0, round(w * par[0] / par[1])))


@dataclass
class Draft:
    """egui's ``SequenceSettingsDraft``."""

    tab: str = "general"
    name: str = ""
    new_sequence: bool = False
    video_tracks: int = 3
    audio_tracks: int = 3
    fps: tuple[int, int] = (24_000, 1001)
    width: int = 1920
    height: int = 1080
    scale_motion: bool = True
    drop_frame: bool = False
    mix: str = "Stereo"
    sample_rate: int = 48_000
    max_render_quality: bool = False
    working_space: str = "rec709"
    wide_gamut: bool = False
    auto_tone_map: bool = False
    # shown greyed: the sequence's own (the default for a new one)
    par: tuple[int, int] = (1, 1)
    preview_codec: str = "ProRes 422"
    max_bit_depth: bool = False


def draft_from(settings: Any, name: str = "") -> Draft:
    """A draft from ``sequence.inspect``'s ``settings`` (serde ``SequenceSettings``); damaged
    values read as the defaults."""
    st = settings if isinstance(settings, dict) else {}
    fr = st.get("frame_rate") if isinstance(st.get("frame_rate"), dict) else {}
    fps = sane_rate(_int(fr.get("num"), 24_000, 1, 10**9), _int(fr.get("den"), 1001, 1, 10**9))
    par = st.get("par") if isinstance(st.get("par"), list) and len(st["par"]) == 2 else [1, 1]
    color = st.get("color") if isinstance(st.get("color"), dict) else {}
    return Draft(
        name=name,
        fps=fps,
        width=_int(st.get("width"), 1920, 1, MAX_FRAME_SIDE),
        height=_int(st.get("height"), 1080, 1, MAX_FRAME_SIDE),
        drop_frame=_bool(st.get("drop_frame")) and supports_drop_frame(*fps),
        mix=MIX_NAMES.get(str(st.get("audio_master")), "Stereo"),
        sample_rate=_int(st.get("sample_rate"), 48_000, 1, 10**7),
        max_render_quality=_bool(st.get("max_render_quality")),
        working_space=WORKING_IDS.get(str(color.get("working")), "rec709"),
        wide_gamut=_bool(color.get("wide_gamut")),
        auto_tone_map=_bool(color.get("auto_tone_map")),
        par=(_int(par[0], 1, 0), _int(par[1], 1, 0)),
        preview_codec=str(st.get("preview_codec") or "ProRes 422"),
        max_bit_depth=_bool(st.get("max_bit_depth")),
    )


def changes(d: Draft, cur: Draft, cur_name: str) -> dict[str, Any]:
    """The ``sequence.settings`` parameters that differ from ``cur``; empty when nothing changed.
    A blank name keeps the current one."""
    p: dict[str, Any] = {}
    name = d.name.strip()
    if name and name != cur_name:
        p["name"] = name
    if d.fps != cur.fps:
        p["fps"] = d.fps[0] / d.fps[1]
    if (d.width, d.height) != (cur.width, cur.height):
        p.update(width=d.width, height=d.height, scaleMotion=d.scale_motion)
    df = d.drop_frame and supports_drop_frame(*d.fps)
    if df != (cur.drop_frame and supports_drop_frame(*cur.fps)):
        p["dropFrame"] = df
    if d.mix != cur.mix:
        p["mix"] = d.mix
    if d.sample_rate != cur.sample_rate:
        p["sampleRate"] = d.sample_rate
    if d.max_render_quality != cur.max_render_quality:
        p["maxRenderQuality"] = d.max_render_quality
    if d.working_space != cur.working_space:
        p["workingSpace"] = d.working_space
    if d.wide_gamut != cur.wide_gamut:
        p["wideGamut"] = d.wide_gamut
    if d.auto_tone_map != cur.auto_tone_map:
        p["autoToneMap"] = d.auto_tone_map
    return p


def new_sequence_params(d: Draft) -> tuple[dict[str, Any], dict[str, Any]]:
    """``file.newSequence``'s parameters, and the ``sequence.settings`` that follow on the new
    sequence for what it has no parameter for (drop-frame, Maximum Render Quality)."""
    p: dict[str, Any] = {}
    name = d.name.strip()
    if name:
        p["name"] = name
    p.update(
        width=d.width,
        height=d.height,
        fps=d.fps[0] / d.fps[1],
        sampleRate=d.sample_rate,
        mix=d.mix,
        video=min(d.video_tracks, MAX_TRACKS),
        audio=min(d.audio_tracks, MAX_TRACKS),
    )
    after = replace(Draft(), fps=d.fps, width=d.width, height=d.height, sample_rate=d.sample_rate)
    rest = {k: v for k, v in changes(d, after, name).items() if k in ("dropFrame", "maxRenderQuality")}
    return p, rest


def count_sequences(node: Any, depth: int = 0) -> int:
    """The sequences in ``project.inspect``'s bin tree."""
    if not isinstance(node, dict) or depth > 64:
        return 0
    n = 0
    for child in node.get("children") or [] if isinstance(node.get("children"), list) else []:
        if isinstance(child, dict):
            if "item" in child:
                n += int(str(child.get("type") or "").lower() == "sequence")
            else:
                n += count_sequences(child, depth + 1)
    return n


class SequenceSettingsDialog(QDialog):
    """``cur`` is the sequence's settings (or the defaults for New Sequence); ``run(command,
    params, then)`` sends a command and calls ``then(ok)``."""

    def __init__(
        self,
        parent: QWidget | None,
        draft: Draft,
        run: Callable[[str, dict[str, Any], Callable[[bool], None]], None],
        cur_name: str = "",
        audio_samples: bool = True,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("sequenceSettings")
        self.d = draft
        self.cur = replace(draft) if not draft.new_sequence else replace(Draft(), new_sequence=True)
        self.cur_name = cur_name
        self.run = run
        self.audio_samples = audio_samples
        self.sent: list[tuple[str, dict[str, Any]]] = []
        self.setWindowTitle(tr("New Sequence") if draft.new_sequence else tr("Sequence Settings"))
        self.setMinimumWidth(620)
        lay = QVBoxLayout(self)
        tabs = QHBoxLayout()
        self.stack = QStackedWidget(self)
        self.tabs: dict[str, QToolButton] = {}
        group = QButtonGroup(self)
        pages = [("general", N_("General"), self._general)]
        if draft.new_sequence:
            pages.append(("tracks", N_("Tracks"), self._tracks))
        else:
            pages += [("color", N_("Color Management"), self._color), ("vr", N_("VR Properties"), self._vr)]
        for key, label, build in pages:
            b = QToolButton(self)
            b.setObjectName(f"sequenceSettings.tab.{key}")
            b.setText(tr(label))
            b.setCheckable(True)
            b.setAutoRaise(True)
            group.addButton(b)
            b.clicked.connect(lambda _c=False, k=key: self.set_tab(k))
            tabs.addWidget(b)
            self.tabs[key] = b
            scroll = QScrollArea(self)
            scroll.setWidgetResizable(True)
            scroll.setFrameShape(QScrollArea.Shape.NoFrame)
            page = QWidget(scroll)
            build(page)
            scroll.setWidget(page)
            self.stack.addWidget(scroll)
        tabs.addStretch(1)
        lay.addLayout(tabs)
        lay.addWidget(self.stack, 1)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        self.cancel = QPushButton(tr("Cancel"), self)
        self.cancel.setObjectName("sequenceSettings.cancel")
        self.cancel.clicked.connect(self.reject)
        self.ok = QPushButton(tr("OK"), self)
        self.ok.setObjectName("sequenceSettings.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.apply)
        buttons.addWidget(self.cancel)
        buttons.addWidget(self.ok)
        lay.addLayout(buttons)
        self.set_tab(draft.tab if draft.tab in self.tabs else "general")
        self._sync()

    # ----- building -----
    def _named(self, w: QWidget, name: str, tip: str = "") -> QWidget:
        w.setObjectName(f"sequenceSettings.{name}")
        if tip:
            w.setEnabled(False)
            w.setToolTip(tr(tip))
        return w

    def _fixed_combo(self, name: str, value: str, tip: str = NOT_YET) -> QComboBox:
        c = QComboBox(self)
        c.addItem(tr(value))
        self._named(c, name, tip)
        return c

    def _fixed_value(self, name: str, value: str) -> QPushButton:
        b = QPushButton(value, self)
        b.setMinimumWidth(64)
        self._named(b, name, NOT_YET)
        return b

    def _combo(self, name: str, slot: Callable[[int], None]) -> QComboBox:
        c = QComboBox(self)
        c.setMinimumWidth(260)
        self._named(c, name)
        c.activated.connect(slot)
        return c

    def _general(self, page: QWidget) -> None:
        lay = QVBoxLayout(page)
        form = QFormLayout()
        form.setLabelAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
        self.name = QLineEdit(self.d.name, self)
        self._named(self.name, "name")
        self.name.textEdited.connect(lambda t: setattr(self.d, "name", t))
        form.addRow(tr("Sequence Name:"), self.name)
        form.addRow(tr("Editing Mode:"), self._fixed_combo("editingMode", N_("Custom")))
        self.timebase = self._combo("timebase", self._pick_timebase)
        form.addRow(tr("Timebase:"), self.timebase)
        lay.addLayout(form)

        video = QGroupBox(tr("Video"), self)
        vf = QFormLayout(video)
        size = QHBoxLayout()
        self.width_box = self._spin("width", 1, MAX_FRAME_SIDE, self.d.width, lambda v: self._set("width", v))
        self.height_box = self._spin("height", 1, MAX_FRAME_SIDE, self.d.height, lambda v: self._set("height", v))
        self.aspect = QLabel(self)
        self._named(self.aspect, "aspect")
        for w in (self.width_box, QLabel(tr("horizontal"), self), self.height_box, QLabel(tr("vertical"), self), self.aspect):
            size.addWidget(w)
        size.addStretch(1)
        vf.addRow(tr("Frame Size:"), size)
        self.scale_motion = QCheckBox(tr("Scale motion effects proportionally when changing frame size"), self)
        self._named(self.scale_motion, "scaleMotion")
        self.scale_motion.setChecked(self.d.scale_motion)
        self.scale_motion.toggled.connect(lambda on: self._set("scale_motion", on))
        vf.addRow(self.scale_motion)
        par = self.d.par
        par_label = (
            tr("Square Pixels (1.0)")
            if par[0] == par[1]
            else fmt("Custom ({ratio})", ratio=f"{par[0] / par[1]:.4f}")
            if par[1] > 0
            else tr("Custom")
        )
        vf.addRow(tr("Pixel Aspect Ratio:"), self._fixed_combo("par", par_label))
        vf.addRow(tr("Fields:"), self._fixed_combo("fields", N_("No Fields (Progressive Scan)")))
        self.video_display = self._combo("videoDisplay", self._pick_display)
        vf.addRow(tr("Display Format:"), self.video_display)
        lay.addWidget(video)

        audio = QGroupBox(tr("Audio"), self)
        af = QFormLayout(audio)
        mix = QHBoxLayout()
        self.mix = self._combo("channelFormat", lambda i: self._set("mix", MIXES[i] if 0 <= i < len(MIXES) else "Stereo"))
        self.mix.setMinimumWidth(120)
        for m in MIXES:
            self.mix.addItem(tr(m), m)
        self.channels = self._fixed_combo("channels", "2")
        channels_label = QLabel(tr("Number of Channels:"), self)
        channels_label.setEnabled(False)
        for w in (self.mix, channels_label, self.channels):
            mix.addWidget(w)
        mix.addStretch(1)
        af.addRow(tr("Channel Format:"), mix)
        self.sample_rate = self._combo("sampleRate", self._pick_rate)
        af.addRow(tr("Sample Rate:"), self.sample_rate)
        af.addRow(tr("Display Format:"), self._fixed_combo("audioDisplay", N_("Audio Samples") if self.audio_samples else N_("Milliseconds")))
        lay.addWidget(audio)

        previews = QGroupBox(tr("Video Previews"), self)
        pf = QFormLayout(previews)
        fmt_row = QHBoxLayout()
        fmt_row.addWidget(self._fixed_combo("previewFormat", "QuickTime"))
        configure = QPushButton(tr("Configure…"), self)
        configure.setEnabled(False)
        configure.setToolTip(tr(NOT_YET))
        fmt_row.addWidget(configure)
        fmt_row.addStretch(1)
        pf.addRow(tr("Preview File Format:"), fmt_row)
        pf.addRow(tr("Codec:"), self._fixed_combo("previewCodec", self.d.preview_codec))
        self.preview_w = self._fixed_value("previewWidth", "")
        pf.addRow(tr("Width:"), self.preview_w)
        h_row = QHBoxLayout()
        self.preview_h = self._fixed_value("previewHeight", "")
        reset = QPushButton(tr("Reset"), self)
        reset.setEnabled(False)
        reset.setToolTip(tr(NOT_YET))
        h_row.addWidget(self.preview_h)
        h_row.addWidget(reset)
        h_row.addStretch(1)
        pf.addRow(tr("Height:"), h_row)
        checks = QHBoxLayout()
        bit_depth = QCheckBox(tr("Maximum Bit Depth"), self)
        bit_depth.setChecked(self.d.max_bit_depth)
        self._named(bit_depth, "maxBitDepth", NOT_YET)
        self.max_quality = QCheckBox(tr("Maximum Render Quality"), self)
        self._named(self.max_quality, "maxRenderQuality")
        self.max_quality.setChecked(self.d.max_render_quality)
        self.max_quality.toggled.connect(lambda on: self._set("max_render_quality", on))
        checks.addWidget(bit_depth)
        checks.addWidget(self.max_quality)
        checks.addStretch(1)
        pf.addRow(checks)
        linear = QCheckBox(tr("Composite in Linear Color"), self)
        linear.setChecked(True)
        self._named(linear, "linearColor", LINEAR)
        pf.addRow(linear)
        lay.addWidget(previews)
        lay.addStretch(1)

    def _spin(self, name: str, lo: int, hi: int, value: int, slot: Callable[[int], None]) -> QSpinBox:
        s = QSpinBox(self)
        s.setRange(lo, hi)
        s.setValue(max(lo, min(hi, value)))
        s.setKeyboardTracking(False)
        self._named(s, name)
        s.valueChanged.connect(slot)
        return s

    def _color(self, page: QWidget) -> None:
        form = QFormLayout(page)
        self.working = self._combo("workingSpace", lambda i: self._set("working_space", WORKING_SPACES[i][0] if 0 <= i < len(WORKING_SPACES) else "rec709"))
        self.working.setMinimumWidth(220)
        for key, label in WORKING_SPACES:
            self.working.addItem(tr(label), key)
        form.addRow(tr("Working Color Space:"), self.working)
        self.wide = QCheckBox(tr("Wide gamut color (composite in BT.2020)"), self)
        self._named(self.wide, "wideGamut")
        self.wide.setChecked(self.d.wide_gamut)
        self.wide.toggled.connect(lambda on: self._set("wide_gamut", on))
        form.addRow(self.wide)
        self.tone_map = QCheckBox(tr("Auto Tone Map Media (HDR and log into SDR)"), self)
        self._named(self.tone_map, "autoToneMap")
        self.tone_map.setChecked(self.d.auto_tone_map)
        self.tone_map.toggled.connect(lambda on: self._set("auto_tone_map", on))
        form.addRow(self.tone_map)

    def _vr(self, page: QWidget) -> None:
        form = QFormLayout(page)
        form.addRow(tr("Projection:"), self._fixed_combo("vr.projection", N_("None")))
        form.addRow(tr("Layout:"), self._fixed_combo("vr.layout", N_("Monoscopic")))
        view = QHBoxLayout()
        for key, label in (("horizontal", N_("Horizontal")), ("vertical", N_("Vertical"))):
            view.addWidget(self._fixed_value(f"vr.{key}", "0°"))
            name = QLabel(tr(label), self)
            name.setEnabled(False)
            view.addWidget(name)
        view.addStretch(1)
        form.addRow(tr("Captured View:"), view)

    def _tracks(self, page: QWidget) -> None:
        form = QFormLayout(page)
        for key, label, value in (("video", N_("Video:"), self.d.video_tracks), ("audio", N_("Audio:"), self.d.audio_tracks)):
            row = QHBoxLayout()
            spin = self._spin(f"tracks.{key}", 0, MAX_TRACKS, value, lambda v, k=key: self._set(f"{k}_tracks", v))
            row.addWidget(spin)
            row.addWidget(QLabel(tr("tracks"), self))
            row.addStretch(1)
            form.addRow(tr(label), row)

    # ----- state -----
    def set_tab(self, key: str) -> None:
        keys = list(self.tabs)
        if key not in self.tabs:
            return
        self.d.tab = key
        self.tabs[key].setChecked(True)
        self.stack.setCurrentIndex(keys.index(key))

    def _set(self, attr: str, value: Any) -> None:
        setattr(self.d, attr, value)
        self._sync()

    def _pick_timebase(self, i: int) -> None:
        rate = self.timebase.itemData(i)
        if isinstance(rate, (list, tuple)) and len(rate) == 2 and tuple(rate) != self.d.fps:
            # a new timebase starts on its usual timecode (drop-frame for the NTSC rates)
            self.d.fps = (int(rate[0]), int(rate[1]))
            self.d.drop_frame = supports_drop_frame(*self.d.fps)
        self._sync()

    def _pick_display(self, i: int) -> None:
        key = self.video_display.itemData(i)
        if key in ("df", "ndf", "tc"):
            self.d.drop_frame = key == "df"
        self._sync()

    def _pick_rate(self, i: int) -> None:
        hz = self.sample_rate.itemData(i)
        if isinstance(hz, int):
            self.d.sample_rate = hz
        self._sync()

    def _sync(self) -> None:
        """Show the draft (lists follow the timebase, the aspect the frame size)."""
        d = self.d
        num, den = d.fps
        rates = list(TIMEBASES)
        if d.fps not in rates:
            rates.insert(0, d.fps)
        self.timebase.blockSignals(True)
        self.timebase.clear()
        for r in rates:
            self.timebase.addItem(fmt("{rate} frames/second", rate=rate_label(*r)), r)
        self.timebase.setCurrentIndex(rates.index(d.fps))
        self.timebase.blockSignals(False)
        label = rate_label(num, den)
        formats = (
            [("df", fmt("{rate} fps Drop-Frame Timecode", rate=label)), ("ndf", fmt("{rate} fps Non Drop-Frame Timecode", rate=label))]
            if supports_drop_frame(num, den)
            else [("tc", fmt("{rate} fps Timecode", rate=label))]
        )
        chosen = ("df" if d.drop_frame else "ndf") if supports_drop_frame(num, den) else "tc"
        self.video_display.blockSignals(True)
        self.video_display.clear()
        for key, text in formats:
            self.video_display.addItem(text, key)
        for text in (N_("Feet + Frames 16mm"), N_("Feet + Frames 35mm"), N_("Frames")):
            self.video_display.addItem(tr(text), None)
            item = self.video_display.model().item(self.video_display.count() - 1)
            if item is not None:
                item.setEnabled(False)
                item.setToolTip(tr(NOT_YET))
        self.video_display.setCurrentIndex(next((i for i, (k, _t) in enumerate(formats) if k == chosen), 0))
        self.video_display.blockSignals(False)
        self.aspect.setText(aspect(display_width(d.width, d.height, d.par), d.height))
        self.preview_w.setText(str(d.width))
        self.preview_h.setText(str(d.height))
        self.mix.setCurrentIndex(MIXES.index(d.mix) if d.mix in MIXES else 0)
        self.channels.setItemText(0, {"Mono": "1", "5.1": "6", "Adaptive": "—"}.get(d.mix, "2"))
        hz = list(SAMPLE_RATES)
        if d.sample_rate not in hz:
            hz.insert(0, d.sample_rate)
        self.sample_rate.blockSignals(True)
        self.sample_rate.clear()
        for r in hz:
            self.sample_rate.addItem(f"{r} Hz", r)
        self.sample_rate.setCurrentIndex(hz.index(d.sample_rate))
        self.sample_rate.blockSignals(False)
        if hasattr(self, "working"):
            keys = [k for k, _l in WORKING_SPACES]
            self.working.setCurrentIndex(keys.index(d.working_space) if d.working_space in keys else 0)

    # ----- OK -----
    def apply(self) -> None:
        d = self.d
        if d.new_sequence:
            p, rest = new_sequence_params(d)

            def made(ok: bool) -> None:
                if not ok:
                    return
                if rest:
                    self.sent.append(("sequence.settings", rest))
                    self.run("sequence.settings", rest, lambda _ok: None)
                self.accept()

            self.sent.append(("file.newSequence", p))
            self.run("file.newSequence", p, made)
            return
        p = changes(d, self.cur, self.cur_name)
        if not p:
            self.accept()  # nothing changed: no undo step
            return
        self.sent.append(("sequence.settings", p))
        self.run("sequence.settings", p, lambda ok: self.accept() if ok else None)


__all__ = ["Draft", "SequenceSettingsDialog", "changes", "draft_from", "new_sequence_params"]
