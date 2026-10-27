"""The menu long tail's dialogs, as egui's ``panels/menu_dialogs.rs``: Find, Create Search Bin,
Project Settings (General / Scratch Disks / Ingest Settings), Properties (Get Media File
Properties), Edit Offline File, Source Settings, Update Metadata, Automate To Sequence, Scene Edit
Detection, Normalize Mix Track, Simplify Sequence, Transcribe Sequence, Save as Template, Flash
Cue Marker, New Color Matte and Color Matte Color.

As :mod:`clip_dialogs`: a menu item without parameters opens the dialog from the engine's
``dialog.defaults``; OK runs the command (Find stays open with "Match n of total"); with parameters
the command runs directly.

Automation ids (egui's, ``<p>`` from :data:`MENU_DIALOGS`): ``<p>.ok``, ``<p>.cancel``,
``<p>.<key>`` for each control, radio buttons ``<p>.<key>.<value>``; Find rows
``find.row.<n>.column|operator|text`` and ``find.match.<all|any>``; Project Settings tabs
``projectSettings.tab.<general|scratchDisks|ingest>``, scratch rows ``projectSettings.scratch.<key>``
with ``.browse`` / ``.same``, ``projectSettings.ingest.<enabled|action|destination>``,
``projectSettings.<titleSafe|actionSafe>.<horizontal|vertical>``.
"""

from __future__ import annotations

import copy
import math
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDoubleSpinBox,
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QRadioButton,
    QSlider,
    QStackedWidget,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr
from .clip_dialogs import ClipDialog, _num

# command -> (title, automation prefix, OK text)
MENU_DIALOGS: dict[str, tuple[str, str, str]] = {
    "edit.find": (N_("Find"), "find", N_("Find")),
    "file.newSearchBin": (N_("Create Search Bin"), "searchBin", N_("OK")),
    "project.settings": (N_("Project Settings"), "projectSettings", N_("OK")),
    "file.mediaProperties": (N_("Properties"), "properties", N_("Close")),
    "clip.editOffline": (N_("Edit Offline File"), "editOffline", N_("OK")),
    "clip.sourceSettings": (N_("Source Settings"), "sourceSettings", N_("OK")),
    "clip.updateMetadata": (N_("Update Metadata"), "updateMetadata", N_("OK")),
    "clip.automateToSequence": (N_("Automate To Sequence"), "automate", N_("OK")),
    "clip.sceneEditDetection": (N_("Scene Edit Detection"), "sceneDetect", N_("Analyze")),
    "sequence.normalizeMixTrack": (N_("Normalize Mix Track"), "normalizeMix", N_("OK")),
    "sequence.simplify": (N_("Simplify Sequence"), "simplify", N_("Simplify")),
    "sequence.transcribe": (N_("Transcribe Sequence"), "transcribe", N_("Transcribe")),
    "file.saveAsTemplate": (N_("Save as Template"), "saveTemplate", N_("Save")),
    "markers.addFlashCue": (N_("Flash Cue Marker"), "flashCue", N_("OK")),
    "file.newColorMatte": (N_("New Color Matte"), "colorMatte", N_("OK")),
    "project.matteColor": (N_("Color Matte Color"), "matteColor", N_("OK")),
}
# the menu items that open one of them (the Project Settings items share one)
MENU_COMMANDS = [*(c for c in MENU_DIALOGS if c != "project.settings"), "file.projectSettings.general", "file.projectSettings.scratchDisks", "project.ingestSettings"]
INFO_ONLY = {"file.mediaProperties", "clip.sourceSettings"}
LANGUAGES = ["auto", "en", "es", "fr", "de", "it", "pt", "ja", "ko", "zh", "nl", "ru"]
INGEST_ACTIONS = [("copy", N_("Copy")), ("transcode", N_("Transcode")), ("createProxies", N_("Create Proxies")), ("copyAndCreateProxies", N_("Copy and Create Proxies"))]
SCRATCH = [
    ("captured", N_("Captured and Generated:")),
    ("videoPreviews", N_("Video Previews:")),
    ("audioPreviews", N_("Audio Previews:")),
    ("autoSave", N_("Project Auto Save:")),
]


def _str(v: Any) -> str:
    if isinstance(v, str):
        return v
    return "" if v is None else str(v)


def _pairs(v: Any) -> list[tuple[str, str]]:
    """``[[name, label], …]`` or ``[name, …]`` from the engine, damaged entries left out."""
    out: list[tuple[str, str]] = []
    for x in v if isinstance(v, list) else []:
        if isinstance(x, str):
            out.append((x, x))
        elif isinstance(x, list) and len(x) == 2 and all(isinstance(y, str) for y in x):
            out.append((x[0], x[1]))
    return out


def finish(command: str, params: dict[str, Any]) -> dict[str, Any]:
    """The parameters sent on OK (egui's ``run``): Find in the Timeline searches its one row; an
    empty Flash Cue name and an automatic language are left to the command."""
    p = copy.deepcopy(params)
    if command == "edit.find" and p.get("scope") == "timeline":
        rows = p.get("rows")
        p["rows"] = rows[:1] if isinstance(rows, list) else []
    elif command == "markers.addFlashCue" and p.get("name") == "":
        p.pop("name", None)
    elif command == "sequence.transcribe" and p.get("language") == "auto":
        p.pop("language", None)
    return p


class MenuDialog(ClipDialog):
    """``state`` is ``dialog.defaults``' answer; ``run(command, params, then)`` as ClipDialog's."""

    def __init__(self, parent: QWidget | None, state: dict[str, Any], run: Any, status: Any = None) -> None:
        self.info_only = str(state.get("command") or "") in INFO_ONLY
        self.status = status  # where a finished command says what it did (the status bar)
        self.result: QLabel | None = None
        super().__init__(parent, state, run)
        self.setMinimumWidth(420)

    def meta(self) -> tuple[str, str, str]:
        return MENU_DIALOGS.get(self.command, (self.command, "menuDialog", N_("OK")))

    def cancel_text(self) -> str:
        return N_("Done") if self.command == "edit.find" else N_("Cancel")

    def builders(self) -> dict[str, Any]:
        return {
            "edit.find": self._find,
            "file.newSearchBin": self._search_bin,
            "project.settings": self._project_settings,
            "file.mediaProperties": self._media_properties,
            "clip.editOffline": self._edit_offline,
            "clip.sourceSettings": self._source_settings,
            "clip.updateMetadata": self._update_metadata,
            "clip.automateToSequence": self._automate,
            "clip.sceneEditDetection": self._scene_detect,
            "sequence.normalizeMixTrack": lambda: self.number("db", N_("Normalize Mix Track to:"), -96, 24, " dB"),
            "sequence.simplify": self._simplify,
            "sequence.transcribe": self._transcribe,
            "file.saveAsTemplate": lambda: self.text("name", N_("Template Name:"), 240),
            "markers.addFlashCue": self._flash_cue,
            "file.newColorMatte": self._color_matte,
            "project.matteColor": lambda: self.color("color", N_("Color:")),
        }

    # ----- helpers -----
    def _label(self, text: str, dim: bool = False, into: Any = None) -> QLabel:
        label = QLabel(text, self)
        label.setWordWrap(True)
        label.setEnabled(not dim)
        (into or self.body_lay).addWidget(label)
        return label

    def radio_row(self, key: str, label: str, choices: list[tuple[str, str]], target: dict[str, Any] | None = None) -> dict[str, QRadioButton]:
        row = QHBoxLayout()
        row.addWidget(QLabel(tr(label), self))
        group = QButtonGroup(self)
        out: dict[str, QRadioButton] = {}
        p = self.params if target is None else target
        for k, text in choices:
            r = QRadioButton(tr(text), self)
            r.setObjectName(f"{self.pre}.{key}.{k}")
            r.setChecked(p.get(key) == k)
            group.addButton(r)
            r.toggled.connect(lambda on, kk=k: on and self._set_in(p, key, kk))
            row.addWidget(r)
            out[k] = r
        row.addStretch(1)
        self.body_lay.addLayout(row)
        return out

    def _set_in(self, p: dict[str, Any], key: str, value: Any) -> None:
        p[key] = value
        self._changed(key)

    def combo(self, name: str, label: str, value: Any, choices: list[tuple[str, str]], on: Any, into: Any = None) -> QComboBox:
        c = QComboBox(self)
        c.setObjectName(name)
        cur = _str(value)
        if cur and cur not in [k for k, _l in choices]:
            choices = [(cur, cur), *choices]
        for k, text in choices:
            c.addItem(tr(text), k)
        c.setCurrentIndex(max(0, [k for k, _l in choices].index(cur) if cur in [k for k, _l in choices] else 0))
        c.activated.connect(lambda i: on(c.itemData(i)))
        if into is None:
            row = QHBoxLayout()
            if label:
                row.addWidget(QLabel(tr(label), self))
            row.addWidget(c)
            row.addStretch(1)
            self.body_lay.addLayout(row)
        else:
            into.addWidget(c)
        return c

    def color(self, key: str, label: str) -> QPushButton:
        row = self._row(label)
        b = QPushButton(self)
        b.setObjectName(f"{self.pre}.{key}")
        b.setFixedSize(48, 22)
        hex_label = QLabel(self)

        def show() -> None:
            c = QColor(_str(self.params.get(key)))
            c = c if c.isValid() else QColor("#000000")
            b.setStyleSheet(f"background: {c.name()}; border: 1px solid #888;")
            hex_label.setText(c.name())

        def pick() -> None:
            c = QColorDialog.getColor(QColor(_str(self.params.get(key)) or "#000000"), self, tr(label))
            if c.isValid():
                self.params[key] = c.name()
                show()

        b.clicked.connect(pick)
        show()
        row.addWidget(b)
        row.addWidget(hex_label)
        row.addStretch(1)
        return b

    # ----- the dialogs -----
    def _find(self) -> None:
        p = self.params
        rows = [r if isinstance(r, dict) else {} for r in p.get("rows") or []] if isinstance(p.get("rows"), list) else []
        while len(rows) < 2:
            rows.append({"column": "Name", "operator": "contains", "text": ""})
        p["rows"] = rows
        self.radio_row("scope", N_("Find in:"), [("project", N_("Project")), ("timeline", N_("Timeline"))])
        cols = _pairs(self.info.get("columns")) or [("Name", "Name")]
        ops = _pairs(self.info.get("operators")) or [("contains", "Contains")]
        self.timeline_box = QWidget(self)
        tl = QVBoxLayout(self.timeline_box)
        tl.setContentsMargins(0, 0, 0, 0)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Find:"), self))
        self.find_text = QLineEdit(_str(rows[0].get("text")), self)
        self.find_text.setObjectName("find.row.0.text")
        self.find_text.textChanged.connect(self._find_text)
        row.addWidget(self.find_text, 1)
        tl.addLayout(row)
        self.body_lay.addWidget(self.timeline_box)
        outer = self.body_lay
        self.body_lay = tl
        self.radio_row("in", N_("Search:"), [("all", N_("Clip Names and Markers")), ("clips", N_("Clip Names")), ("markers", N_("Markers"))])
        self.project_box = QWidget(self)
        pl = QVBoxLayout(self.project_box)
        pl.setContentsMargins(0, 0, 0, 0)
        self.row_texts: list[QLineEdit] = []
        for n in range(2):
            line = QHBoxLayout()
            r = rows[n]
            self.combo(f"find.row.{n}.column", "", r.get("column"), cols, lambda v, rr=r: rr.__setitem__("column", v), line)
            self.combo(f"find.row.{n}.operator", "", r.get("operator"), ops, lambda v, rr=r: rr.__setitem__("operator", v), line)
            t = QLineEdit(_str(r.get("text")), self)
            t.setObjectName(f"find.row.{n}.text.project")
            t.textChanged.connect(lambda v, rr=r: rr.__setitem__("text", v))
            line.addWidget(t, 1)
            self.row_texts.append(t)
            pl.addLayout(line)
        match = QHBoxLayout()
        match.addWidget(QLabel(tr("Match:"), self))
        group = QButtonGroup(self)
        for value, key, label in ((True, "all", N_("All")), (False, "any", N_("Any"))):
            rb = QRadioButton(tr(label), self)
            rb.setObjectName(f"find.match.{key}")
            rb.setChecked(p.get("matchAll") is value)
            group.addButton(rb)
            rb.toggled.connect(lambda on, v=value: on and p.__setitem__("matchAll", v))
            match.addWidget(rb)
        self.body_lay = pl
        self.check("caseSensitive", N_("Case Sensitive"), into=match)
        match.addStretch(1)
        pl.addLayout(match)
        self.body_lay = outer
        self.body_lay.addWidget(self.project_box)
        self.result = self._label("", dim=True)
        self.result.setObjectName("find.result")
        self.result.hide()
        self._changed("scope")

    def _find_text(self, t: str) -> None:
        rows = self.params["rows"]
        rows[0]["text"] = t
        if self.row_texts:
            self.row_texts[0].blockSignals(True)
            self.row_texts[0].setText(t)
            self.row_texts[0].blockSignals(False)

    def _search_bin(self) -> None:
        self.text("name", N_("Name:"), 220)
        cols = _pairs(self.info.get("columns")) or [("All", "All")]
        ops = _pairs(self.info.get("operators")) or [("contains", "Contains")]
        self.combo("searchBin.column", N_("Search:"), self.params.get("column"), cols, lambda v: self.params.__setitem__("column", v))
        self.combo("searchBin.operator", "", self.params.get("operator"), ops, lambda v: self.params.__setitem__("operator", v))
        self.text("text", N_("Find:"), 220)

    def _project_settings(self) -> None:
        p = self.params
        self._label(fmt("Project: {name}", name=_str(self.info.get("project"))))
        tabs = QHBoxLayout()
        self.stack = QStackedWidget(self)
        self.tabs: dict[str, QToolButton] = {}
        group = QButtonGroup(self)
        outer = self.body_lay
        for key, label, build in (
            ("general", N_("General"), self._ps_general),
            ("scratchDisks", N_("Scratch Disks"), self._ps_scratch),
            ("ingest", N_("Ingest Settings"), self._ps_ingest),
        ):
            b = QToolButton(self)
            b.setObjectName(f"projectSettings.tab.{key}")
            b.setText(tr(label))
            b.setCheckable(True)
            b.setAutoRaise(True)
            group.addButton(b)
            b.clicked.connect(lambda _c=False, k=key: self.set_tab(k))
            tabs.addWidget(b)
            self.tabs[key] = b
            page = QWidget(self)
            self.body_lay = QVBoxLayout(page)
            build()
            self.body_lay.addStretch(1)
            self.stack.addWidget(page)
        self.body_lay = outer
        tabs.addStretch(1)
        self.body_lay.addLayout(tabs)
        self.body_lay.addWidget(self.stack)
        self.set_tab(_str(p.get("tab")) or "general")

    def set_tab(self, key: str) -> None:
        keys = list(self.tabs)
        if key in keys:
            self.params["tab"] = key
            self.tabs[key].setChecked(True)
            self.stack.setCurrentIndex(keys.index(key))

    def _ps_general(self) -> None:
        p = self.params
        self._heading(N_("Video Rendering and Playback"))
        renderers = _pairs(self.info.get("renderers"))
        self.combo("projectSettings.renderer", N_("Renderer:"), p.get("renderer"), renderers, lambda v: p.__setitem__("renderer", v))
        self._heading(N_("Video"))
        displays = _pairs(self.info.get("videoDisplays"))
        self.combo("projectSettings.videoDisplay", N_("Display Format:"), p.get("videoDisplay"), displays, lambda v: p.__setitem__("videoDisplay", v))
        self._heading(N_("Audio"))
        audio = [("Audio Samples", N_("Audio Samples")), ("Milliseconds", N_("Milliseconds"))]
        self.combo("projectSettings.audioDisplay", N_("Display Format:"), p.get("audioDisplay"), audio, lambda v: p.__setitem__("audioDisplay", v))
        self._heading(N_("Capture"))
        self.combo("projectSettings.captureFormat", N_("Capture Format:"), p.get("captureFormat"), [("DV", "DV"), ("HDV", "HDV")], lambda v: p.__setitem__("captureFormat", v))
        self._heading(N_("Action and Title Safe Areas"))
        for key, label in (("titleSafe", N_("Title Safe Area")), ("actionSafe", N_("Action Safe Area"))):
            pair = p.get(key)
            p[key] = [_num(pair[i], 0.0) if isinstance(pair, list) and i < len(pair) else 0.0 for i in range(2)]
            row = self._row(label)
            for i, axis in enumerate(("horizontal", "vertical")):
                s = QDoubleSpinBox(self)
                s.setObjectName(f"projectSettings.{key}.{axis}")
                s.setRange(0.0, 50.0)
                s.setDecimals(1)
                s.setSuffix(" %")
                s.setValue(max(0.0, min(50.0, p[key][i])))
                s.valueChanged.connect(lambda v, k=key, ii=i: p[k].__setitem__(ii, v))
                row.addWidget(s)
                row.addWidget(QLabel(tr(axis), self))
            row.addStretch(1)

    def _ps_scratch(self) -> None:
        p = self.params
        scratch = self.info.get("scratch") if isinstance(self.info.get("scratch"), dict) else {}
        for key, label in SCRATCH:
            self._heading(label)
            row = QHBoxLayout()
            e = QLineEdit(_str(p.get(key)), self)
            e.setObjectName(f"projectSettings.scratch.{key}")
            e.setPlaceholderText(tr("Same as Project"))
            e.textChanged.connect(lambda t, k=key: p.__setitem__(k, t or None))
            browse = QPushButton(tr("Browse…"), self)
            browse.setObjectName(f"projectSettings.scratch.{key}.browse")
            browse.clicked.connect(lambda _c=False, ed=e: self._browse(ed))
            same = QPushButton(tr("Same as Project"), self)
            same.setObjectName(f"projectSettings.scratch.{key}.same")
            same.clicked.connect(lambda _c=False, ed=e: ed.clear())
            for w in (e, browse, same):
                row.addWidget(w)
            self.body_lay.addLayout(row)
            entry = scratch.get(key) if isinstance(scratch.get(key), dict) else {}
            self._label(fmt("Path: {path}", path=_str(entry.get("path"))), dim=True)

    def _browse(self, edit: QLineEdit) -> None:
        d = QFileDialog.getExistingDirectory(self, tr("Browse…"), edit.text())
        if d:
            edit.setText(d)

    def _ps_ingest(self) -> None:
        ing = self.params.get("ingest")
        ing = ing if isinstance(ing, dict) else {}
        self.params["ingest"] = ing
        b = QCheckBox(tr("Ingest"), self)
        b.setObjectName("projectSettings.ingest.enabled")
        b.setChecked(ing.get("enabled") is True)
        b.toggled.connect(lambda on: ing.__setitem__("enabled", on))
        self.body_lay.addWidget(b)
        self.combo("projectSettings.ingest.action", N_("Action:"), ing.get("action"), INGEST_ACTIONS, lambda v: ing.__setitem__("action", v))
        row = self._row(N_("Destination:"))
        e = QLineEdit(_str(ing.get("destination")), self)
        e.setObjectName("projectSettings.ingest.destination")
        e.setPlaceholderText(tr("Next to the media"))
        e.textChanged.connect(lambda t: ing.__setitem__("destination", t or None))
        row.addWidget(e, 1)

    def _media_properties(self) -> None:
        items = self.raw_info if isinstance(self.raw_info, list) else [self.raw_info] if isinstance(self.raw_info, dict) else []
        for v in items:
            if not isinstance(v, dict):
                continue
            title = self._label(_str(v.get("name")))
            font = title.font()
            font.setBold(True)
            title.setFont(font)

            def line(k: str, val: str) -> None:
                if val and val != "null":
                    self._label(f"{tr(k)}: {val}")

            line(N_("File Path"), _str(v.get("path")))
            line(N_("Type"), f"{_str(v.get('type'))} ({_str(v.get('container'))})")
            size = v.get("fileSize")
            line(N_("File Size"), f"{size / 1e6:.2f} MB" if isinstance(size, int) and not isinstance(size, bool) else "")
            dur = v.get("duration") if isinstance(v.get("duration"), dict) else {}
            line(N_("Total Duration"), _str(dur.get("timecode")))
            rate = v.get("dataRateKbps")
            line(N_("Average Data Rate"), f"{rate} kbit/s" if isinstance(rate, (int, float)) and not isinstance(rate, bool) and math.isfinite(rate) else "")
            x = v.get("video")
            if isinstance(x, dict):
                line(N_("Video"), f"{_str(x.get('codec'))} · {_str(x.get('width'))} x {_str(x.get('height'))} ({_str(x.get('pixelAspectRatio'))}) · {_str(x.get('frameRateLabel'))} fps · {_str(x.get('pixelFormat'))}")
                line(N_("Colour"), _str(x.get("color")))
            a = v.get("audio")
            if isinstance(a, dict):
                line(N_("Audio"), fmt("{codec} · {rate} Hz · {n} channel(s)", codec=_str(a.get("codec")), rate=_str(a.get("sampleRate")), n=_str(a.get("channels"))))

    def _edit_offline(self) -> None:
        for key, label in (
            ("mediaName", N_("Media Name:")),
            ("tapeName", N_("Tape Name:")),
            ("description", N_("Description:")),
            ("scene", N_("Scene:")),
            ("shot", N_("Shot/Take:")),
            ("logNote", N_("Log Note:")),
        ):
            self.text(key, label, 220)

    def _source_settings(self) -> None:
        self._label(fmt("Codec: {codec}", codec=_str(self.info.get("codec"))))
        self._label(_str(self.info.get("message")), dim=True)

    def _update_metadata(self) -> None:
        self._label(fmt("Write the metadata of the selected clip(s) ({n}) to XMP files next to their media?", n=_str(self.info.get("count"))))
        self._label(tr("Existing XMP files written by other applications are left unchanged."), dim=True)

    def _automate(self) -> None:
        self._label(fmt("To {name}", name=_str(self.info.get("sequence"))))
        self.radio_row("ordering", N_("Ordering:"), [("sort", N_("Sort Order")), ("selection", N_("Selection Order"))])
        self.radio_row("placement", N_("Placement:"), [("sequentially", N_("Sequentially")), ("unnumberedMarkers", N_("At Unnumbered Markers"))])
        self.radio_row("method", N_("Method:"), [("insert", N_("Insert Edit")), ("overwrite", N_("Overwrite Edit"))])
        self.number("overlapFrames", N_("Clip Overlap:"), 0, 600, " " + tr("frames"), 0)
        self.number("stillFrames", N_("Frames per Still:"), 1, 100_000, " " + tr("frames"), 0)
        self._heading(N_("Transitions"))
        self.check("audioTransition", N_("Apply Default Audio Transition"))
        self.check("videoTransition", N_("Apply Default Video Transition"))
        self._heading(N_("Ignore Options"))
        self.check("ignoreAudio", N_("Ignore Audio"))
        self.check("ignoreVideo", N_("Ignore Video"))

    def _scene_detect(self) -> None:
        self.check("applyCuts", N_("Apply a cut at each detected cut point"))
        self.check("createSubclips", N_("Create a bin of subclips from each detected cut point"))
        self.check("generateMarkers", N_("Create a clip marker at each detected cut point"))
        row = self._row(N_("Sensitivity:"))
        s = QSlider(Qt.Orientation.Horizontal, self)
        s.setObjectName("sceneDetect.sensitivity")
        s.setRange(0, 100)
        s.setValue(round(max(0.0, min(100.0, _num(self.params.get("sensitivity"), 50.0)))))
        s.valueChanged.connect(lambda v: self.params.__setitem__("sensitivity", float(v)))
        row.addWidget(s, 1)

    def _simplify(self) -> None:
        self.text("name", N_("Sequence Name:"), 240)
        self._heading(N_("Remove"))
        for key, label in (
            ("removeDisabled", N_("Disabled clips")),
            ("removeEmptyTracks", N_("Empty tracks")),
            ("closeGaps", N_("Gaps")),
            ("removeVideoEffects", N_("Video effects")),
            ("removeAudioEffects", N_("Audio effects")),
            ("removeText", N_("Text (graphics and captions)")),
        ):
            self.check(key, label)
        self._heading(N_("Clips"))
        self.check("moveClipsDown", N_("Move clips to the lowest track possible"))
        self.radio_row("keep", N_("Keep:"), [("both", N_("Video and Audio")), ("video", N_("Video Only")), ("audio", N_("Audio Only"))])

    def _transcribe(self) -> None:
        p = self.params
        self.combo("transcribe.language", N_("Language:"), p.get("language"), [(x, x) for x in LANGUAGES], lambda v: p.__setitem__("language", v))
        tracks = [("mix", N_("Mix"))] + [(t, fmt("Audio on track {t}", t=t)) for t in self.info.get("tracks") or [] if isinstance(t, str)]
        self.combo("transcribe.track", N_("Audio analysis:"), p.get("track"), tracks, lambda v: p.__setitem__("track", v))
        self.check("diarize", N_("Recognize when different speakers are talking"))

    def _flash_cue(self) -> None:
        self.text("name", N_("Name:"), 220)
        self.text("comment", N_("Comments:"), 220)

    def _color_matte(self) -> None:
        self.color("color", N_("Color:"))
        self.text("name", N_("Name:"), 220)
        self.number("width", N_("Width:"), 1, 16_384, " px", 0)
        self.number("height", N_("Height:"), 1, 16_384, " px", 0)
        self.number("seconds", N_("Duration:"), 0.04, 36_000, " s")

    def _changed(self, key: str) -> None:
        if self.command == "edit.find" and key == "scope":
            timeline = self.params.get("scope") == "timeline"
            self.timeline_box.setVisible(timeline)
            self.project_box.setVisible(not timeline)
            self.adjustSize()
        else:
            super()._changed(key)

    # ----- OK -----
    def apply(self) -> None:
        params = finish(self.command, self.params)
        self.sent.append(params)
        self.error.hide()
        if self.command == "project.settings":
            self._apply_project_settings(params)
            return
        self.ok.setEnabled(False)

        def done(error: str | None, result: Any = None) -> None:
            self.ok.setEnabled(True)
            if error is not None:
                self.fail(error)
                return
            if self.command == "edit.find" and self.result is not None:
                r = result if isinstance(result, dict) else {}
                index = r.get("index") if isinstance(r.get("index"), int) else 0
                total = r.get("matches") if isinstance(r.get("matches"), int) else 0
                self.result.setText(fmt("Match {n} of {total}", n=index + 1, total=total))
                self.result.show()
                return
            if self.status is not None:
                if self.command == "file.saveAsTemplate":
                    self.status(fmt("Saved template {path}", path=_str((result or {}).get("path") if isinstance(result, dict) else "")))
                elif self.command == "clip.sceneEditDetection":
                    self.status(tr("Scene Edit Detection: analysing in the background (see Progress)"))
            self.accept()

        self.run(self.command, params, done)

    def _apply_project_settings(self, p: dict[str, Any]) -> None:
        """General, then Scratch Disks, then Ingest Settings when they changed (egui's)."""
        steps: list[tuple[str, dict[str, Any]]] = [
            (
                "file.projectSettings.general",
                {
                    "renderer": p.get("renderer"),
                    "videoDisplay": p.get("videoDisplay"),
                    "audioDisplay": "milliseconds" if p.get("audioDisplay") == "Milliseconds" else "samples",
                    "captureFormat": p.get("captureFormat"),
                    "titleSafe": p.get("titleSafe"),
                    "actionSafe": p.get("actionSafe"),
                },
            ),
            ("file.projectSettings.scratchDisks", {k: p.get(k) for k, _l in SCRATCH}),
        ]
        if p.get("ingest") != self.info.get("ingest"):
            steps.append(("project.ingestSettings", p.get("ingest") if isinstance(p.get("ingest"), dict) else {}))
        self.ok.setEnabled(False)

        def next_step(error: str | None = None, _result: Any = None) -> None:
            if error is not None:
                self.ok.setEnabled(True)
                self.fail(error)
                return
            if not steps:
                self.ok.setEnabled(True)
                self.accept()
                return
            cid, params = steps.pop(0)
            self.run(cid, params, next_step)

        next_step()


__all__ = ["MENU_COMMANDS", "MENU_DIALOGS", "MenuDialog", "finish"]
