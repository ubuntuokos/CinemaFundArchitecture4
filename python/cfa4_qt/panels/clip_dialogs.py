"""The Edit / Clip / File menu dialogs, as egui's ``panels/clip_dialogs.rs``: Paste Attributes,
Remove Attributes, New Offline File, Nest… (Nested Sequence Name), Make Subclip, Edit Subclip,
Modify ▸ Audio Channels, Modify ▸ Timecode, Frame Hold Options, Field Options and Clip Speed /
Duration.

A menu entry or shortcut without parameters opens the dialog, starting from the engine's
``dialog.defaults`` (the parameters the command takes, and what the dialog shows besides); OK runs
the command with the edited parameters (a failure stays in the dialog, in red); with parameters
the command runs directly.

Automation ids (egui's): the dialog ``<p>``, ``<p>.ok``, ``<p>.cancel`` and ``<p>.<key>`` for each
control, with ``<p>`` from :data:`DIALOGS`; Paste / Remove Attributes ``effect.<id>``; Audio
Channels ``format.<mono|stereo|5.1|adaptive>``, ``count``, ``clip.<n>.ch.<c>``; Frame Hold
``holdOn.<in|out|playhead|sourceTimecode|sequenceTime>``; Field Options
``processing.<none|alwaysDeinterlace|flickerRemoval>``; Speed / Duration
``interpolation.<frameSampling|frameBlending|opticalFlow>`` and ``duration`` (the readout).
"""

from __future__ import annotations

import copy
import math
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QFont, QFontDatabase
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDoubleSpinBox,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QRadioButton,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr

# command -> (title, automation prefix)
DIALOGS: dict[str, tuple[str, str]] = {
    "edit.pasteAttributes": (N_("Paste Attributes"), "pasteAttributes"),
    "edit.removeAttributes": (N_("Remove Attributes"), "removeAttributes"),
    "file.newOfflineFile": (N_("New Offline File"), "offlineFile"),
    "clip.nest": (N_("Nested Sequence Name"), "nest"),
    "clip.makeSubclip": (N_("Make Subclip"), "makeSubclip"),
    "clip.editSubclip": (N_("Edit Subclip"), "editSubclip"),
    "clip.audioChannels": (N_("Modify Clip: Audio Channels"), "audioChannels"),
    "clip.modifyTimecode": (N_("Modify Clip: Timecode"), "timecode"),
    "clip.frameHoldOptions": (N_("Frame Hold Options"), "frameHold"),
    "clip.fieldOptions": (N_("Field Options"), "fieldOptions"),
    "clip.speedDuration": (N_("Clip Speed / Duration"), "speedDuration"),
}
INTRINSIC_VIDEO = [("motion", N_("Motion")), ("opacity", N_("Opacity")), ("timeRemapping", N_("Time Remapping"))]
INTRINSIC_AUDIO = [("volume", N_("Volume")), ("channelVolume", N_("Channel Volume")), ("panner", N_("Panner"))]
FORMATS = [("mono", N_("Mono")), ("stereo", N_("Stereo")), ("5.1", "5.1"), ("adaptive", N_("Adaptive"))]
HOLD_ON = [
    ("in", N_("In Point")),
    ("out", N_("Out Point")),
    ("playhead", N_("Playhead")),
    ("sourceTimecode", N_("Source Timecode")),
    ("sequenceTime", N_("Sequence Time")),
]
PROCESSING = [("none", N_("None")), ("alwaysDeinterlace", N_("Always Deinterlace")), ("flickerRemoval", N_("Flicker Removal"))]
INTERPOLATION = [("frameSampling", N_("Frame Sampling")), ("frameBlending", N_("Frame Blending")), ("opticalFlow", N_("Optical Flow"))]
Runner = Callable[[str, dict[str, Any], Callable[..., None]], None]
MAX_CHANNELS = 64  # a matrix this wide is already unusable; more is damaged data
MAX_FRAME = 2**60


def _num(v: Any, default: float) -> float:
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        return default
    x = float(v)
    return x if math.isfinite(x) else default


def _int(v: Any, default: int) -> int:
    return v if isinstance(v, int) and not isinstance(v, bool) else default


def timecode_of_frames(frame: int, fps: float) -> str:
    """cfa4_time's ``format_timecode_frames`` (non-drop): HH:MM:SS:FF at the rate's timecode base."""
    base = max(1, math.ceil(fps - 1e-6)) if math.isfinite(fps) and 0 < fps < 10_000 else 24
    sign = "-" if frame < 0 else ""
    f = min(abs(frame), MAX_FRAME)
    ff = f % base
    s = f // base
    return f"{sign}{s // 3600:02}:{s // 60 % 60:02}:{s % 60:02}:{ff:02}"


def channel_map(fmt_name: str, channels: int) -> list[list[int]]:
    """cfa4_project's ``AudioChannelMap::for_format``: mono = one clip per channel, stereo =
    pairs, 5.1 / adaptive = one clip with every channel."""
    n = max(1, min(channels, MAX_CHANNELS))
    if fmt_name == "mono":
        return [[c] for c in range(n)]
    if fmt_name == "stereo":
        return [[c, c + 1] if c + 1 < n else [c] for c in range(0, n, 2)]
    return [list(range(n))]


def finish(command: str, params: dict[str, Any]) -> dict[str, Any]:
    """The parameters sent on OK (dialog-only fields dropped, as egui's ``finish``)."""
    p = dict(params)
    if command == "file.newOfflineFile" and p.get("name") == "":
        p.pop("name", None)
    elif command == "clip.modifyTimecode" and p.get("reset") is True or command == "clip.frameHoldOptions" and p.get("holdOn") not in ("sourceTimecode", "sequenceTime"):
        p.pop("timecode", None)
    return p


class ClipDialog(QDialog):
    """``state`` is ``dialog.defaults``' answer; ``run(command, params, then)`` sends the command
    and calls ``then(error, result)`` (error None on success)."""

    info_only = False  # a dialog that only shows something: one button, which closes it

    def __init__(self, parent: QWidget | None, state: dict[str, Any], run: Runner) -> None:
        super().__init__(parent)
        self.command = str(state.get("command") or "")
        title, self.pre, ok_text = self.meta()
        self.setObjectName(self.pre)
        self.setWindowTitle(tr(title))
        self.setMinimumWidth(360)
        params = state.get("params")
        info = state.get("info")
        self.params: dict[str, Any] = copy.deepcopy(params) if isinstance(params, dict) else {}
        self.raw_info: Any = info
        self.info: dict[str, Any] = info if isinstance(info, dict) else {}
        self.run = run
        self.sent: list[dict[str, Any]] = []
        self.lay = QVBoxLayout(self)
        self.mono = QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont)
        self.body = QWidget(self)
        self.body_lay = QVBoxLayout(self.body)
        self.body_lay.setContentsMargins(0, 0, 0, 0)
        self.lay.addWidget(self.body)
        build = self.builders().get(self.command)
        if build is not None:
            build()
        self.error = QLabel(self)
        self.error.setObjectName(f"{self.pre}.error")
        self.error.setWordWrap(True)
        self.error.setStyleSheet("color: #e06060;")
        self.error.hide()
        self.lay.addWidget(self.error)
        buttons = QHBoxLayout()
        self.cancel = QPushButton(tr(self.cancel_text()), self)
        self.cancel.setObjectName(f"{self.pre}.cancel")
        self.cancel.clicked.connect(self.reject)
        self.cancel.setVisible(not self.info_only)
        self.ok = QPushButton(tr(ok_text), self)
        self.ok.setObjectName(f"{self.pre}.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.accept if self.info_only else self.apply)
        buttons.addWidget(self.cancel)
        buttons.addWidget(self.ok)
        buttons.addStretch(1)
        self.lay.addLayout(buttons)

    def meta(self) -> tuple[str, str, str]:
        """(title, automation prefix, OK button text)."""
        title, pre = DIALOGS.get(self.command, (self.command, "clipDialog"))
        return title, pre, N_("OK")

    def cancel_text(self) -> str:
        return N_("Cancel")

    def builders(self) -> dict[str, Callable[[], None]]:
        return {
            "edit.pasteAttributes": self._attributes,
            "edit.removeAttributes": self._attributes,
            "file.newOfflineFile": self._offline,
            "clip.nest": self._nest,
            "clip.makeSubclip": self._subclip,
            "clip.editSubclip": self._subclip,
            "clip.audioChannels": self._audio_channels,
            "clip.modifyTimecode": self._timecode,
            "clip.frameHoldOptions": self._frame_hold,
            "clip.fieldOptions": self._field_options,
            "clip.speedDuration": self._speed,
        }

    # ----- controls bound to a parameter -----
    def _heading(self, text: str) -> None:
        label = QLabel(tr(text), self)
        font = QFont(label.font())
        font.setBold(True)
        label.setFont(font)
        self.body_lay.addWidget(label)

    def check(self, key: str, label: str, enabled: bool = True, into: Any = None) -> QCheckBox:
        b = QCheckBox(tr(label), self)
        b.setObjectName(f"{self.pre}.{key}")
        b.setChecked(self.params.get(key) is True)
        b.setEnabled(enabled)
        b.toggled.connect(lambda on, k=key: self._set(k, on))
        (into or self.body_lay).addWidget(b)
        return b

    def _row(self, label: str) -> QHBoxLayout:
        row = QHBoxLayout()
        row.addWidget(QLabel(tr(label), self))
        self.body_lay.addLayout(row)
        return row

    def text(self, key: str, label: str, width: int) -> QLineEdit:
        row = self._row(label)
        e = QLineEdit(str(self.params.get(key) or ""), self)
        e.setObjectName(f"{self.pre}.{key}")
        e.setMinimumWidth(width)
        e.textChanged.connect(lambda t, k=key: self._set(k, t))
        row.addWidget(e, 1)
        return e

    def number(self, key: str, label: str, lo: float, hi: float, suffix: str, decimals: int = 2) -> QDoubleSpinBox:
        row = self._row(label)
        s = QDoubleSpinBox(self)
        s.setObjectName(f"{self.pre}.{key}")
        s.setDecimals(decimals)
        s.setRange(lo, hi)
        s.setSuffix(suffix)
        s.setKeyboardTracking(False)
        s.setValue(max(lo, min(hi, _num(self.params.get(key), lo))))
        s.valueChanged.connect(lambda v, k=key: self._set(k, v))
        row.addWidget(s)
        row.addStretch(1)
        return s

    def frames(self, key: str, label: str, fps: float) -> QSpinBox:
        row = self._row(label)
        s = QSpinBox(self)
        s.setObjectName(f"{self.pre}.{key}")
        s.setRange(0, 2**31 - 1)
        s.setSuffix(" fr")
        s.setKeyboardTracking(False)
        s.setValue(max(0, min(2**31 - 1, _int(self.params.get(key), 0))))
        tc = QLabel(timecode_of_frames(s.value(), fps), self)
        tc.setObjectName(f"{self.pre}.{key}.timecode")
        tc.setFont(self.mono)
        tc.setEnabled(False)

        def changed(v: int, k: str = key) -> None:
            self._set(k, v)
            tc.setText(timecode_of_frames(v, fps))

        s.valueChanged.connect(changed)
        row.addWidget(s)
        row.addWidget(tc)
        row.addStretch(1)
        return s

    def radios(self, key: str, choices: list[tuple[str, str]], into: Any = None) -> dict[str, QRadioButton]:
        out: dict[str, QRadioButton] = {}
        for k, label in choices:
            r = QRadioButton(tr(label), self)
            r.setObjectName(f"{self.pre}.{key}.{k}")
            r.setChecked(self.params.get(key) == k)
            r.toggled.connect(lambda on, kk=k, key=key: on and self._set(key, kk))
            (into or self.body_lay).addWidget(r)
            out[k] = r
        return out

    def _set(self, key: str, value: Any) -> None:
        self.params[key] = value
        self._changed(key)

    def _changed(self, key: str) -> None:
        """Controls that follow another (enabled states, readouts)."""
        if self.command == "file.newOfflineFile" and key == "video":
            for w in self._video_only:
                w.setEnabled(self.params.get("video") is not False)
        elif self.command == "clip.frameHoldOptions":
            on = self.params.get("enabled") is not False
            for w in self._hold_widgets:
                w.setEnabled(on)
            self._hold_tc.setVisible(self.params.get("holdOn") in ("sourceTimecode", "sequenceTime"))
            self._hold_tc_label.setVisible(self._hold_tc.isVisible())
        elif self.command == "clip.speedDuration":
            self._update_duration()

    # ----- the dialogs -----
    def _attributes(self) -> None:
        v, a = self.info.get("video") is not False, self.info.get("audio") is not False
        self._heading(N_("Video Attributes"))
        for k, label in INTRINSIC_VIDEO:
            self.check(k, label, v)
        self._heading(N_("Audio Attributes"))
        for k, label in INTRINSIC_AUDIO:
            self.check(k, label, a)
        self._heading(N_("Effects"))
        effects = [e for e in self.info.get("effects") or [] if isinstance(e, list) and len(e) == 2 and all(isinstance(x, str) for x in e)]
        if not effects:
            none = QLabel(tr("No effects"), self)
            none.setEnabled(False)
            self.body_lay.addWidget(none)
        chosen = self.params.get("effects")
        self.params["effects"] = [e for e in chosen if isinstance(e, str)] if isinstance(chosen, list) else []
        for eid, name in effects:
            b = QCheckBox(tr(name), self)
            b.setObjectName(f"{self.pre}.effect.{eid}")
            b.setChecked(eid in self.params["effects"])
            b.toggled.connect(lambda on, e=eid: self._effect(e, on))
            self.body_lay.addWidget(b)
        if self.command == "edit.pasteAttributes":
            self.check("scaleTimes", N_("Scale Attribute Times"))

    def _effect(self, eid: str, on: bool) -> None:
        effects = [e for e in self.params.get("effects") or [] if e != eid]
        if on:
            effects.append(eid)
        self.params["effects"] = effects

    def _offline(self) -> None:
        self.text("fileName", N_("File Name:"), 220)
        self.text("name", N_("Clip Name:"), 220)
        self.text("tapeName", N_("Tape Name:"), 220)
        self.text("description", N_("Description:"), 220)
        row = self._row(N_("Contains:"))
        self.check("video", N_("Video"), into=row)
        self.check("audio", N_("Audio"), into=row)
        row.addStretch(1)
        self._video_only = [
            self.number("width", N_("Width:"), 16, 16_384, " px", 0),
            self.number("height", N_("Height:"), 16, 16_384, " px", 0),
            self.number("fps", N_("Frame Rate:"), 1, 240, " fps", 3),
        ]
        self.text("timecode", N_("Media Start:"), 120)
        self.number("seconds", N_("Duration:"), 0.05, 86_400, " s")
        self._changed("video")

    def _nest(self) -> None:
        # the name is ready to type over, and Enter accepts it
        e = self.text("name", N_("Name:"), 240)
        e.selectAll()
        e.setFocus()

    def _subclip(self) -> None:
        fps = _num(self.info.get("fps"), 24.0)
        if self.command == "clip.makeSubclip":
            self.text("name", N_("Name:"), 240)
        else:
            self.body_lay.addWidget(QLabel(fmt("Subclip: {name}", name=str(self.info.get("name") or "")), self))
        self.frames("startFrame", N_("Start:"), fps)
        self.frames("endFrame", N_("End:"), fps)
        self.check("restrictTrims", N_("Restrict Trims To Subclip Boundaries"))
        if self.command == "clip.editSubclip":
            self.check("convertToMaster", N_("Convert to Master Clip"))

    def _audio_channels(self) -> None:
        n = max(1, min(_int(self.info.get("channels"), 2), MAX_CHANNELS))
        self._n = n
        if self.info.get("items") is True:
            row = self._row(N_("Clip Channel Format:"))
            for k, r in self.radios("format", FORMATS, row).items():
                r.toggled.connect(lambda on, kk=k: on and self._format(kk))
            row.addStretch(1)
            row = self._row(N_("Number of Audio Clips:"))
            self.count = QSpinBox(self)
            self.count.setObjectName(f"{self.pre}.count")
            self.count.setRange(1, n)
            row.addWidget(self.count)
            row.addStretch(1)
            self._heading(N_("Media Source Channels"))
            self.matrix = QWidget(self)
            self.grid = QGridLayout(self.matrix)
            self.body_lay.addWidget(self.matrix)
            self.params["clips"] = self._clips()
            self.count.setValue(len(self.params["clips"]))
            self.count.valueChanged.connect(self._count)
            self._fill_matrix()
        else:
            self.body_lay.addWidget(QLabel(tr("Source channels the selected audio clips play:"), self))
            row = QHBoxLayout()
            chans = {c for c in self.params.get("channels") or [] if isinstance(c, int)} if isinstance(self.params.get("channels"), list) else set()
            self.params["channels"] = sorted(chans)
            for c in range(max(n, 2)):
                b = QCheckBox(fmt("Ch. {n}", n=c + 1), self)
                b.setObjectName(f"{self.pre}.clip.0.ch.{c}")
                b.setChecked(c in chans)
                b.toggled.connect(lambda on, cc=c: self._channel(cc, on))
                row.addWidget(b)
            row.addStretch(1)
            self.body_lay.addLayout(row)

    def _clips(self) -> list[list[int]]:
        raw = self.params.get("clips")
        clips = [sorted({c for c in x if isinstance(c, int) and 0 <= c < self._n}) for x in raw if isinstance(x, list)] if isinstance(raw, list) else []
        return clips[: self._n] or [[0]]

    def _format(self, k: str) -> None:
        self.params["clips"] = channel_map(k, self._n)
        self.count.blockSignals(True)
        self.count.setValue(len(self.params["clips"]))
        self.count.blockSignals(False)
        self._fill_matrix()

    def _count(self, count: int) -> None:
        clips = self._clips()
        while len(clips) < count:
            clips.append([min(len(clips), self._n - 1)])
        self.params["clips"] = clips[: max(1, count)]
        self._fill_matrix()

    def _fill_matrix(self) -> None:
        while self.grid.count():
            item = self.grid.takeAt(0)
            w = item.widget() if item is not None else None
            if w is not None:
                w.deleteLater()
        # the loudspeaker under each channel number, when the media knows its layout (#323)
        raw = self.info.get("speakers")
        speakers = [x if isinstance(x, str) else "" for x in raw] if isinstance(raw, list) else []
        for c in range(self._n):
            name = speakers[c] if c < len(speakers) else ""
            label = QLabel(fmt("Ch. {n}", n=c + 1) + (f"\n{name}" if name else ""), self.matrix)
            label.setObjectName(f"{self.pre}.ch.{c}")
            label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            self.grid.addWidget(label, 0, c + 1)
        for ci, chans in enumerate(self.params["clips"]):
            self.grid.addWidget(QLabel(fmt("Clip {n}", n=ci + 1), self.matrix), ci + 1, 0)
            for c in range(self._n):
                b = QCheckBox(self.matrix)
                b.setObjectName(f"{self.pre}.clip.{ci}.ch.{c}")
                b.setChecked(c in chans)
                b.toggled.connect(lambda on, i=ci, cc=c: self._matrix(i, cc, on))
                self.grid.addWidget(b, ci + 1, c + 1, Qt.AlignmentFlag.AlignCenter)

    def _matrix(self, ci: int, c: int, on: bool) -> None:
        clips = self.params["clips"]
        if 0 <= ci < len(clips):
            chans = {x for x in clips[ci] if x != c}
            if on:
                chans.add(c)
            clips[ci] = sorted(chans)

    def _channel(self, c: int, on: bool) -> None:
        chans = {x for x in self.params.get("channels") or [] if x != c}
        if on:
            chans.add(c)
        self.params["channels"] = sorted(chans)

    def _timecode(self) -> None:
        self.text("timecode", N_("Set Timecode:"), 120)
        self.text("tapeName", N_("Tape Name:"), 200)
        self.check("reset", N_("Use the file's timecode"))

    def _frame_hold(self) -> None:
        self.check("enabled", N_("Hold On"))
        self._hold_widgets: list[QWidget] = list(self.radios("holdOn", HOLD_ON).values())
        row = QHBoxLayout()
        self._hold_tc_label = QLabel(tr("Timecode:"), self)
        self._hold_tc = QLineEdit(str(self.params.get("timecode") or ""), self)
        self._hold_tc.setObjectName(f"{self.pre}.timecode")
        self._hold_tc.textChanged.connect(lambda t: self._set("timecode", t))
        row.addWidget(self._hold_tc_label)
        row.addWidget(self._hold_tc)
        row.addStretch(1)
        self.body_lay.addLayout(row)
        self._hold_widgets += [self._hold_tc, self._hold_tc_label, self.check("holdFilters", N_("Hold Filters"))]
        self._changed("enabled")

    def _field_options(self) -> None:
        self.check("reverseFieldDominance", N_("Reverse Field Dominance"))
        self._heading(N_("Processing Options"))
        self.radios("processing", PROCESSING)

    def _speed(self) -> None:
        self.number("speed", N_("Speed:"), 0.01, 100_000, " %")
        row = self._row(N_("Duration:"))
        self.duration = QLabel(self)
        self.duration.setObjectName(f"{self.pre}.duration")
        self.duration.setFont(self.mono)
        row.addWidget(self.duration)
        row.addStretch(1)
        self.check("reverse", N_("Reverse Speed"))
        self.check("ripple", N_("Ripple Edit, Shifting Trailing Clips"))
        self._heading(N_("Time Interpolation"))
        self.radios("interpolation", INTERPOLATION)
        self._update_duration()

    def _update_duration(self) -> None:
        """Duration follows the speed: the media shown stays the same."""
        dur = _int(self.info.get("duration"), 0)
        was = _num(self.info.get("speed"), 1.0)
        now = _num(self.params.get("speed"), 100.0) / 100.0
        num, den = _int(self.info.get("fpsNum"), 0), _int(self.info.get("fpsDen"), 0)
        fps = num / den if num > 0 and den > 0 else _num(self.info.get("fps"), 24.0)
        shown = dur > 0 and now > 0 and fps > 0
        if shown:
            ticks = dur * was / now
            frames = math.floor(ticks * fps / 254_016_000_000 + 1e-9) if math.isfinite(ticks) else 0
            self.duration.setText(timecode_of_frames(max(0, min(frames, MAX_FRAME)), fps))
        self.duration.setVisible(shown)

    # ----- OK -----
    def apply(self) -> None:
        params = finish(self.command, self.params)
        self.sent.append(params)
        self.ok.setEnabled(False)

        def done(error: str | None, _result: Any = None) -> None:
            self.ok.setEnabled(True)
            if error is None:
                self.accept()
                return
            self.fail(error)

        self.run(self.command, params, done)

    def fail(self, error: str) -> None:
        self.error.setText(error)
        self.error.show()


__all__ = ["DIALOGS", "ClipDialog", "channel_map", "finish", "timecode_of_frames"]
