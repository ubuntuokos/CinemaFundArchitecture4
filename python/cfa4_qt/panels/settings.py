"""The Preferences dialog, as egui's (``panels/settings.rs``): the category list on the left, the
category's page on the right, Help / Reset… / Cancel / OK at the bottom. The pages come from the
engine's settings schema (``prefs.schema``: fields, groups, notes, buttons and the sections a
frontend draws itself); edits go to a draft of the whole preferences document (``prefs.get``) and
OK applies what changed with one ``prefs.set``. Edit ▸ Preferences ▸ <category> opens it (General:
Ctrl+,).

Object names, egui's automation ids: ``settings.category.<id>``, ``settings.<key>`` for every
control (the preference key), ``settings.<key>.<value>`` (theme radio buttons),
``settings.<key>.browse`` (folders), ``settings.<key>.hex`` (colours),
``settings.favorite.<theme>`` (favourite stars), ``settings.labels.colors.<label>.name|color``,
``settings.labelPresets.<revert|import|export|folder|delete>``, ``settings.mediaCache.clean``,
``settings.controlSurfaces.refresh``, ``settings.controlSurfaces.<n>.enabled|profile`` (the n-th device),
``settings.page``, ``settings.help``, ``settings.reset``, ``settings.cancel``, ``settings.ok``.
"""

from __future__ import annotations

import copy
import math
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QPointF, QRectF, Qt, QUrl, Signal
from PySide6.QtGui import QColor, QDesktopServices, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFileDialog,
    QFrame,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QPushButton,
    QRadioButton,
    QScrollArea,
    QSizePolicy,
    QSpinBox,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import fmt, tr

LIST_W = 216
LABEL_W = 250
PAGE_W = 900 - LIST_W - 60
HELP_URL = "https://github.com/ubuntuokos/CinemaFundArchitecture4/blob/main/docs/project-files.md#settings"
LIGHT_THEMES = (("light", "Light"),)
DARK_THEMES = (("darkest", "Darkest"), ("dark", "Dark"))
APPEARANCE_MODES = (("auto", "Sync with system"), ("dark", "Dark"), ("light", "Light"))
DURATION_UNITS = (("frames", "Frames"), ("seconds", "Seconds"))
# drawn as the Appearance page's mode selector and theme cards, not as plain fields
APPEARANCE_KEYS = frozenset({"appearance.appearanceMode", "appearance.lightTheme", "appearance.darkTheme", "appearance.colorTheme", "appearance.favoriteThemes"})
# Edit ▸ Preferences (egui's menus.rs `app.settings.*`)
MENU = (
    ("general", "General…"),
    ("appearance", "Appearance…"),
    ("audio", "Audio…"),
    ("audioHardware", "Audio Hardware…"),
    ("autoSave", "Auto Save…"),
    ("color", "Color…"),
    ("graphics", "Graphics…"),
    ("labels", "Labels…"),
    ("media", "Media…"),
    ("mediaAnalysis", "Media Analysis & Transcription…"),
    ("mediaCache", "Media Cache…"),
    ("memory", "Memory…"),
    ("playback", "Playback…"),
    ("plugins", "Plugins…"),
    ("timeline", "Timeline…"),
    ("trim", "Trim…"),
)
LABELS = ("Violet", "Iris", "Caribbean", "Lavender", "Cerulean", "Forest", "Rose", "Mango", "Purple", "Blue", "Teal", "Magenta", "Tan", "Green",
          "Brown", "Yellow")


# ----- the draft document -----
def get(values: Any, key: str) -> Any:
    v = values
    for part in key.split("."):
        if not isinstance(v, dict) or part not in v:
            return None
        v = v[part]
    return v


def has(values: Any, key: str) -> bool:
    """``key`` (dotted) names a value in ``values``, as egui's JSON pointer finds it."""
    v = values
    for part in key.split("."):
        if not isinstance(v, dict) or part not in v:
            return False
        v = v[part]
    return True


def put(values: Any, key: str, x: Any) -> None:
    """Set an existing key (unknown keys are left alone, as egui's ``put``)."""
    parts = key.split(".")
    v = values
    for part in parts[:-1]:
        if not isinstance(v, dict) or part not in v:
            return
        v = v[part]
    if isinstance(v, dict) and parts[-1] in v:
        v[parts[-1]] = x


def diff(old: Any, new: Any, prefix: str = "") -> dict[str, Any]:
    """Leaf values of ``new`` that differ from ``old``, as dotted keys."""
    out: dict[str, Any] = {}
    if isinstance(old, dict) and isinstance(new, dict):
        for k, nv in new.items():
            out.update(diff(old.get(k), nv, f"{prefix}.{k}" if prefix else k))
    elif old != new and prefix:
        out[prefix] = new
    return out


def choice_text(v: Any) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float, str)):
        return str(v)
    return ""


def number(v: Any, fallback: float) -> float:
    try:
        x = float(v)
    except (TypeError, ValueError):
        return fallback
    return x if math.isfinite(x) else fallback


def parse_hex(s: Any) -> QColor | None:
    if not isinstance(s, str):
        return None
    t = s.strip()
    if len(t) == 7 and t.startswith("#") and all(c in "0123456789abcdefABCDEF" for c in t[1:]):
        return QColor(t)
    return None


def mb(n: int) -> str:
    return f"{n / (1 << 30):.1f} GB" if n >= 1 << 30 else f"{n / (1 << 20):.1f} MB"


# ----- small widgets -----
class ThemePreview(QWidget):
    """A small CFA4 editor painted with a theme's colours (egui's ``theme_preview``): header with
    the mode tabs, Source and Program monitors, Project panel, tools, Timeline, meters."""

    def __init__(self, tokens: theme.Tokens, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.t = tokens
        self.setFixedHeight(128)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = self.t
        p = QPainter(self)
        r = QRectF(self.rect())

        def fill(rect: QRectF, name_or_colour: Any, radius: float = 0.0) -> None:
            c = t.c(name_or_colour) if isinstance(name_or_colour, str) else name_or_colour
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(c)
            if radius:
                p.drawRoundedRect(rect, radius, radius)
            else:
                p.drawRect(rect)

        def bar(x: float, y: float, w: float, name: str) -> None:
            fill(QRectF(x, y, w, 2), name, 1)

        fill(r, "app_bg", 3)
        header = QRectF(r.left(), r.top(), r.width(), 12)
        fill(header, "header_bg", 3)
        fill(QRectF(header.left() + 5, header.center().y() - 2, 4, 4), "text_dim", 1)
        for i, w in enumerate((9.0, 6.0, 9.0)):
            x = header.left() + 14 + i * 13
            bar(x, header.center().y() - 1, w, "tab_text_active" if i == 1 else "tab_text")
            if i == 1:
                fill(QRectF(x, header.bottom() - 2.5, w, 1), "tab_text_active")
        bar(header.center().x() - 16, header.center().y() - 1, 32, "tab_text_active")
        for i in range(4):
            fill(QRectF(header.right() - 9 - i * 8, header.center().y() - 2, 4, 4), "icon", 1)
        body = QRectF(QPointF(r.left() + 2, header.bottom() + 2), QPointF(r.right() - 2, r.bottom() - 2))
        split = body.top() + body.height() * 0.5

        def panel(rect: QRectF, tab_w: float) -> QRectF:
            fill(rect, "panel_bg", 2)
            bar(rect.left() + 4, rect.top() + 3.5, tab_w, "tab_text_active")
            return QRectF(QPointF(rect.left() + 3, rect.top() + 9), QPointF(rect.right() - 3, rect.bottom() - 3))

        source = panel(QRectF(QPointF(body.left(), body.top()), QPointF(body.left() + body.width() * 0.36, split - 1)), 14)
        fill(source, "monitor_bg")
        fill(source.adjusted(6, 8, -6, -8), "field_bg")
        program = panel(QRectF(QPointF(body.left() + body.width() * 0.36 + 2, body.top()), QPointF(body.right(), split - 1)), 16)
        fill(program, "monitor_bg")
        fh = max(1.0, program.height() - 9)
        frame = QRectF(program.center().x() - fh * 8 / 9, program.center().y() - 3 - fh / 2, fh * 16 / 9, fh)
        fill(frame, QColor(0x2C, 0x4A, 0x2E))
        fill(QRectF(QPointF(frame.left(), frame.bottom() - frame.height() * 0.3), frame.bottomRight()), QColor(0x4A, 0x3C, 0x30))
        p.setBrush(QColor(0xC8, 0x2A, 0x26))
        rad = frame.height() * 0.26
        p.drawEllipse(QPointF(frame.left() + frame.width() * 0.62, frame.center().y() + 2), rad, rad)
        bar(program.left() + 2, program.bottom() - 3, 14, "timecode")
        low = split + 1
        project = panel(QRectF(QPointF(body.left(), low), QPointF(body.left() + body.width() * 0.24, body.bottom())), 12)
        for row in range(2):
            for col in range(2):
                cell = QRectF(project.left() + col * (project.width() / 2), project.top() + 2 + row * 18, project.width() / 2 - 3, 11)
                fill(cell, "field_bg" if (row + col) % 2 == 0 else "row_selected", 1)
                bar(cell.left(), cell.bottom() + 2, cell.width() * 0.7, "text_faint")
        tools = QRectF(QPointF(project.right() + 5, low), QPointF(project.right() + 15, body.bottom()))
        fill(tools, "panel_bg", 2)
        for i in range(5):
            fill(QRectF(tools.center().x() - 2.5, tools.top() + 4.5 + i * 9, 5, 5), "accent" if i == 0 else "icon", 1)
        meters = QRectF(QPointF(body.right() - 12, low), QPointF(body.right(), body.bottom()))
        fill(meters, "panel_bg", 2)
        for i, h in enumerate((0.55, 0.62)):
            x = meters.left() + 3 + i * 4
            fill(QRectF(QPointF(x, meters.bottom() - 3 - (meters.height() - 6) * h), QPointF(x + 2, meters.bottom() - 3)), QColor(0x4C, 0xAF, 0x50))
        tl = QRectF(QPointF(tools.right() + 2, low), QPointF(meters.left() - 2, body.bottom()))
        fill(tl, "tl_bg", 2)
        bar(tl.left() + 4, tl.top() + 3.5, 14, "tab_text_active")
        bar(tl.left() + 4, tl.top() + 9, 16, "timecode")
        heads_w = 22.0
        ruler = QRectF(QPointF(tl.left() + heads_w, tl.top() + 7), QPointF(tl.right() - 2, tl.top() + 13))
        fill(ruler, "tl_ruler_bg")
        p.setPen(QPen(t.c("tl_ruler_tick"), 1))
        x = ruler.left() + 2
        while x < ruler.right():
            p.drawLine(QPointF(x, ruler.bottom() - 2), QPointF(x, ruler.bottom()))
            x += 6
        video, audio, music = QColor(0x8A, 0x8A, 0xE6), QColor(0x2E, 0xBB, 0xB0), QColor(0x5E, 0xA2, 0x45)
        track_h = max(4.0, (tl.bottom() - ruler.bottom() - 3) / 4)
        for i in range(4):
            y = ruler.bottom() + 1 + i * track_h
            head = QRectF(tl.left() + 2, y, heads_w - 3, track_h - 1)
            fill(head, "tl_header_bg", 1)
            fill(QRectF(head.left() + 2, head.top() + 1.5, 5, track_h - 4), "accent", 1)
            lane = QRectF(QPointF(ruler.left(), y), QPointF(ruler.right(), y + track_h - 1))
            fill(lane, "tl_track_bg" if i % 2 == 0 else "tl_track_bg_alt")
            clips = ((0.55, 0.75),) if i == 0 else ((0.0, 0.22), (0.23, 0.48), (0.49, 0.7), (0.71, 0.9)) if i in (1, 2) else ((0.0, 0.9),)
            c = QColor(video if i < 2 else audio if i == 2 else music)
            c.setAlphaF(0.85)
            for a, b in clips:
                fill(QRectF(QPointF(lane.left() + lane.width() * a, lane.top() + 0.5), QPointF(lane.left() + lane.width() * b, lane.bottom() - 0.5)), c, 1)
        ph = ruler.left() + ruler.width() * 0.3
        p.setPen(QPen(t.c("playhead"), 1))
        p.drawLine(QPointF(ph, ruler.top()), QPointF(ph, tl.bottom() - 2))


class Swatch(QToolButton):
    """A colour button: shows ``#rrggbb``, a click opens the colour picker."""

    picked = Signal(str)

    def __init__(self, name: str, colour: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(name)
        self.setFixedSize(36, 20)
        self.colour = colour
        self.clicked.connect(self._pick)
        self.set_colour(colour)

    def set_colour(self, colour: str) -> None:
        self.colour = colour
        c = parse_hex(colour) or QColor(128, 128, 128)
        self.setStyleSheet(f"QToolButton {{ background: {c.name()}; border: 1px solid {theme.current().field_border}; border-radius: 3px; }}")

    def _pick(self) -> None:
        c = QColorDialog.getColor(parse_hex(self.colour) or QColor(128, 128, 128), self, tr("Color"))
        if c.isValid():
            self.set_colour(c.name())
            self.picked.emit(c.name())


def field_label(text: str, parent: QWidget, width: int = LABEL_W) -> QLabel:
    """A right-aligned label in the fixed label column; a long one wraps inside it."""
    label = QLabel(f"{tr(text)}:" if text else "", parent)
    label.setWordWrap(True)
    label.setFixedWidth(width)
    label.setAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
    return label


MIDI_KINDS = {"cc": "CC", "cc14": "CC", "note": "Note", "rpn": "RPN", "nrpn": "NRPN"}


def control_label(control: Any) -> str:
    """A control as the engine names it (``cfa4_surface_model::midi::control_label``): ``Ch 1 CC 16``."""
    if isinstance(control, dict):
        midi = control.get("midi")
        if isinstance(midi, dict):
            try:
                ch = int(midi.get("channel") or 0) + 1
            except (TypeError, ValueError):
                ch = 1
            kind = MIDI_KINDS.get(str(midi.get("kind")), str(midi.get("kind")))
            suffix = " (14-bit)" if midi.get("kind") == "cc14" else ""
            return f"Ch {ch} {kind} {midi.get('number')}{suffix}"
        hid = control.get("hid")
        if isinstance(hid, dict):
            name = str(hid.get("name") or "")
            if name.startswith("button") and name[6:].isdigit():
                return fmt("Button {n}", n=name[6:])
            return name
    return str(control)


class PreferencesDialog(QDialog):
    """``engine`` answers ``prefs.schema`` / ``prefs.get`` and runs edits; ``applied(changes)``
    after OK applied them. ``devices()`` gives the audio devices ``{"hosts", "inputs", "outputs",
    "outputChannels"}``; ``memory()`` the bytes of decoded pictures the window holds."""

    applied = Signal(dict)

    def __init__(self, parent: QWidget | None, engine: Any, page: str = "general",
                 devices: Callable[[], dict[str, Any]] | None = None, memory: Callable[[], int] | None = None,
                 commands: Callable[[], list[tuple[str, str]]] | None = None) -> None:
        super().__init__(parent)
        self.engine = engine
        # (id, label) of the commands a control surface button can run (Control Surfaces ▸ Assignments)
        self.commands = commands or list
        self.devices = devices or dict
        self.memory = memory or (lambda: 0)
        self.setObjectName("settings")
        self.setWindowTitle(tr("Preferences"))
        self.page = page
        self.categories: list[dict[str, Any]] = []
        self.values: dict[str, Any] = {}
        self.saved: dict[str, Any] = {}
        self.message = ""
        self.preset_message = ""
        self.cache_info: dict[str, Any] | None = None
        self.label_presets: dict[str, Any] | None = None
        self.auto_save_status: dict[str, Any] | None = None
        self.surfaces: dict[str, Any] | None = None  # surface.devices
        self.surface_profiles: list[dict[str, Any]] | None = None  # controlSurfaces.profiles
        self._surfaces_pending = self._surfaces_building = False
        self.monitor_lines: list[str] = []  # the MIDI Monitor's lines, kept across page rebuilds
        self.monitor_on = False
        self.monitor_error = ""
        self._monitor: dict[str, Any] = {}  # the monitor's widgets on the page shown now
        self.assign_device = ""  # the device whose assignments the page shows
        self.assign_command = ""  # the command Learn assigns
        self.learning = False
        self.learn_message = ""
        event = getattr(engine, "event", None)
        if event is not None:
            event.connect(self._engine_event)
        self.finished.connect(lambda _r: self._stop_reading())
        self.label_w = LABEL_W  # narrower beside Label Colors (egui: at most 45 % of the width)
        self._enablers: list[tuple[QWidget, str]] = []
        self._watchers: list[Callable[[], None]] = []
        outer = QVBoxLayout(self)
        outer.setContentsMargins(20, 14, 20, 14)
        body = QHBoxLayout()
        body.setSpacing(20)
        self.list_box = QFrame(self)
        self.list_box.setObjectName("settings.categories")
        self.list_box.setFixedWidth(LIST_W)
        self.list_box.setFrameShape(QFrame.Shape.StyledPanel)
        self.list_layout = QVBoxLayout(self.list_box)
        self.list_layout.setContentsMargins(1, 1, 1, 1)
        self.list_layout.setSpacing(0)
        self.list_layout.addStretch(1)
        self.category_group = QButtonGroup(self)
        self.category_group.setExclusive(True)
        body.addWidget(self.list_box)
        self.scroll = QScrollArea(self)
        self.scroll.setObjectName("settings.page")
        self.scroll.setWidgetResizable(True)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll.setFixedSize(PAGE_W, 600)
        body.addWidget(self.scroll)
        outer.addLayout(body)
        buttons = QHBoxLayout()
        self.help_button = self._button("settings.help", tr("Help"))
        self.help_button.clicked.connect(lambda: QDesktopServices.openUrl(QUrl(HELP_URL)))
        self.reset_button = self._button("settings.reset", tr("Reset…"))
        self.reset_button.clicked.connect(self.reset_page)
        self.cancel_button = self._button("settings.cancel", tr("Cancel"))
        self.cancel_button.clicked.connect(self.reject)
        self.ok_button = self._button("settings.ok", tr("OK"))
        self.ok_button.setDefault(True)
        self.ok_button.clicked.connect(self.apply)
        buttons.addWidget(self.help_button)
        buttons.addWidget(self.reset_button)
        buttons.addStretch(1)
        buttons.addWidget(self.cancel_button)
        buttons.addWidget(self.ok_button)
        outer.addSpacing(14)
        outer.addLayout(buttons)
        self.ready = False
        self._load()

    def _button(self, name: str, text: str) -> QPushButton:
        b = QPushButton(text, self)
        b.setObjectName(name)
        b.setAutoDefault(False)
        b.setMinimumSize(82, 30)
        return b

    # ----- engine -----
    def _load(self) -> None:
        got: dict[str, Any] = {}

        def done(key: str, result: Any) -> None:
            got[key] = result
            if "schema" in got and "values" in got:
                schema, values = got["schema"], got["values"]
                cats = schema.get("categories") if isinstance(schema, dict) else None
                vals = values.get("values") if isinstance(values, dict) else None
                if isinstance(cats, list) and isinstance(vals, dict):
                    self.set_data([c for c in cats if isinstance(c, dict)], vals)

        def failed(message: str) -> None:
            self.message = message
            self._build_page()

        self.engine.execute("prefs.schema", {}, on_reply=lambda r, _b: done("schema", r), on_error=failed)
        self.engine.execute("prefs.get", {}, on_reply=lambda r, _b: done("values", r), on_error=failed)

    def set_data(self, categories: list[dict[str, Any]], values: dict[str, Any]) -> None:
        self.categories = categories
        self.saved = copy.deepcopy(values)
        self.values = copy.deepcopy(values)
        while self.list_layout.count() > 1:
            item = self.list_layout.takeAt(0)
            w = item.widget() if item is not None else None
            if w is not None:
                w.deleteLater()
        for c in categories:
            cid = str(c.get("id") or "")
            b = QPushButton(tr(str(c.get("title") or cid)).replace("&", "&&"), self.list_box)
            b.setObjectName(f"settings.category.{cid}")
            b.setCheckable(True)
            b.setFlat(True)
            b.setAutoDefault(False)
            b.setFixedHeight(25)
            b.setToolTip(tr(str(c.get("title") or cid)))
            b.setStyleSheet("QPushButton { text-align: left; padding-left: 8px; border: none; }"
                            f" QPushButton:checked {{ background: {theme.current().row_alt}; color: {theme.current().text}; }}")
            b.clicked.connect(lambda _c=False, i=cid: self.select_page(i))
            self.category_group.addButton(b)
            self.list_layout.insertWidget(self.list_layout.count() - 1, b)
        if not any(c.get("id") == self.page for c in categories):
            self.page = str(categories[0].get("id")) if categories else ""
        self.ready = True
        self.select_page(self.page)

    def select_page(self, page: str) -> None:
        if not any(c.get("id") == page for c in self.categories):
            return
        self.page = page
        self.message = ""
        b = self.list_box.findChild(QPushButton, f"settings.category.{page}")
        if b is not None:
            b.setChecked(True)
        self._build_page()

    def patch(self, p: Any) -> None:
        """egui's ``settings::patch`` (``ui.set {settings}``): ``page`` picks a category,
        ``values`` sets draft values by their dotted keys (OK applies them, as typed). An unknown
        category or preference raises ValueError and changes nothing."""
        if not isinstance(p, dict):
            raise TypeError("settings takes {page, values}")
        page = p.get("page")
        if page is not None and not any(c.get("id") == page for c in self.categories):
            raise ValueError(f"unknown settings category {page!r}")
        values = p.get("values")
        if values is not None:
            if not isinstance(values, dict):
                raise TypeError("settings.values takes {key: value}")
            unknown = next((k for k in values if not has(self.values, str(k))), None)
            if unknown is not None:
                raise ValueError(f"unknown preference {unknown!r}")
        if page is not None:
            self.select_page(page)
        if values:
            for k, v in values.items():
                put(self.values, str(k), copy.deepcopy(v))
            self._build_page()

    def category(self) -> dict[str, Any]:
        return next((c for c in self.categories if c.get("id") == self.page), {})

    def set_value(self, key: str, value: Any, rebuild: bool = False) -> None:
        put(self.values, key, value)
        if rebuild:
            self._build_page()
            return
        for w, by in self._enablers:
            w.setEnabled(get(self.values, by) is not False)
        for f in self._watchers:
            f()

    def reset_page(self) -> None:
        """This category back to its defaults (OK applies it); the recent projects stay."""
        defaults = self.category().get("defaults")
        if isinstance(defaults, dict) and self.page in self.values:
            keep = copy.deepcopy(get(self.values, "general.recentProjects"))
            self.values[self.page] = copy.deepcopy(defaults)
            if keep is not None:
                put(self.values, "general.recentProjects", keep)
            self._build_page()

    def apply(self) -> None:
        changes = diff(self.saved, self.values)
        if not changes:
            self.accept()
            return

        def done(_r: Any, _b: Any) -> None:
            self.saved = copy.deepcopy(self.values)
            self.applied.emit(changes)
            self.accept()

        def failed(message: str) -> None:
            self.message = message
            self._build_page()

        self.engine.execute("prefs.set", {"values": changes}, on_reply=done, on_error=failed)

    # ----- the page -----
    def _build_page(self) -> None:
        old = self.scroll.takeWidget()
        if old is not None:
            old.deleteLater()
        self._enablers, self._watchers = [], []
        page = QWidget()
        page.setObjectName(f"settings.page.{self.page}")
        col = QVBoxLayout(page)
        col.setContentsMargins(0, 0, 8, 0)
        col.setSpacing(6)
        self._rows(page, col, [r for r in self.category().get("rows") or [] if isinstance(r, dict)])
        if self.message:
            col.addWidget(self._note(page, self.message, "settings.message"))
        col.addStretch(1)
        self.scroll.setWidget(page)
        for w, by in self._enablers:
            w.setEnabled(get(self.values, by) is not False)

    def _note(self, parent: QWidget, text: str, name: str = "") -> QLabel:
        label = QLabel(tr(text), parent)
        label.setWordWrap(True)
        label.setStyleSheet(f"color: {theme.current().text_dim}; font-size: 11.5px;")
        if name:
            label.setObjectName(name)
        return label

    def _rows(self, parent: QWidget, col: QVBoxLayout, rows: list[dict[str, Any]]) -> None:
        for i, r in enumerate(rows):
            kind = r.get("type")
            if kind == "custom" and r.get("id") == "labelColors":
                # Label Colors on the left, the rest of the page on the right
                row = QHBoxLayout()
                left = QVBoxLayout()
                left.addWidget(self._label_colors(parent))
                left.addStretch(1)
                row.addLayout(left)
                right = QVBoxLayout()
                self.label_w = 140
                self._rows(parent, right, rows[i + 1:])
                self.label_w = LABEL_W
                right.addStretch(1)
                row.addLayout(right, 1)
                col.addLayout(row)
                return
            if kind == "field":
                f = r.get("field") if isinstance(r.get("field"), dict) else {}
                if str(f.get("key")) in APPEARANCE_KEYS:
                    continue
                col.addWidget(self._field(parent, f))
            elif kind == "group":
                box = QGroupBox(tr(str(r.get("title") or "")), parent)
                inner = QVBoxLayout(box)
                inner.setContentsMargins(14, 16, 14, 10)
                inner.setSpacing(8)
                self._rows(box, inner, [x for x in r.get("rows") or [] if isinstance(x, dict)])
                col.addWidget(box)
            elif kind == "note":
                col.addWidget(self._note(parent, str(r.get("text") or "")))
            elif kind == "button":
                col.addWidget(self._command_button(parent, r))
            elif kind == "custom":
                w = self._custom(parent, str(r.get("id") or ""))
                if w is not None:
                    col.addWidget(w)

    def _field(self, parent: QWidget, f: dict[str, Any]) -> QWidget:
        key = str(f.get("key") or "")
        label = str(f.get("label") or "")
        k = f.get("kind") if isinstance(f.get("kind"), dict) else {}
        t = str(k.get("type") or "")
        cur = get(self.values, key)
        row = QWidget(parent)
        h = QHBoxLayout(row)
        h.setContentsMargins(22 if f.get("indent") else 0, 0, 0, 0)
        h.setSpacing(6)
        if t == "bool":
            w: QWidget = QCheckBox(tr(label), row)
            w.setObjectName(f"settings.{key}")
            w.setChecked(bool(cur))
            w.toggled.connect(lambda on, key=key: self.set_value(key, bool(on)))
            h.addWidget(w)
        elif t in ("int", "float"):
            h.addWidget(field_label(label, row, self.label_w))
            w = self._spin(row, key, number(cur, number(k.get("min"), 0.0)), number(k.get("min"), 0.0), number(k.get("max"), 1e9),
                           0 if t == "int" else int(number(k.get("decimals"), 2)), t == "int")
            h.addWidget(w)
            if k.get("unit"):
                h.addWidget(QLabel(tr(str(k.get("unit"))), row))
        elif t == "duration":
            h.addWidget(field_label(label, row, self.label_w))
            unit_key = str(k.get("unitKey") or "")
            frames = choice_text(get(self.values, unit_key)) == "frames"
            w = self._spin(row, key, number(cur, 1.0), 1.0 if frames else 0.01, 100_000.0 if frames else 3600.0, 0 if frames else 2, frames)
            h.addWidget(w)
            h.addWidget(self._combo(row, unit_key, choice_text(get(self.values, unit_key)), [(v, tr(lbl)) for v, lbl in DURATION_UNITS], 90, rebuild=True))
        elif t == "choice":
            h.addWidget(field_label(label, row, self.label_w))
            opts = [(str(c.get("value")), tr(str(c.get("label")))) for c in k.get("choices") or [] if isinstance(c, dict)]
            w = self._combo(row, key, choice_text(cur), opts, 300, numeric=isinstance(cur, (int, float)) and not isinstance(cur, bool))
            h.addWidget(w)
        elif t == "device":
            h.addWidget(field_label(label, row, self.label_w))
            dev = self.devices() or {}
            names = dev.get({"hosts": "hosts", "inputs": "inputs", "outputs": "outputs"}.get(str(k.get("list")), "outputs")) or []
            opts = [("", tr("System Default")), *((str(n), str(n)) for n in names)]
            w = self._combo(row, key, choice_text(cur), opts, 300)
            h.addWidget(w)
        elif t == "label":
            h.addWidget(field_label(label, row, self.label_w))
            swatch = QLabel(row)
            swatch.setFixedSize(14, 14)

            def paint_swatch(swatch: QLabel = swatch, key: str = key) -> None:
                name = choice_text(get(self.values, key))
                c = parse_hex(get(self.values, f"labels.colors.{name.lower()}.color")) or QColor(128, 128, 128)
                swatch.setStyleSheet(f"background: {c.name()}; border-radius: 2px;")

            paint_swatch()
            self._watchers.append(paint_swatch)
            h.addWidget(swatch)
            opts = [(n, str(get(self.values, f"labels.colors.{n.lower()}.name") or n)) for n in LABELS]
            w = self._combo(row, key, choice_text(cur), opts, 160)
            h.addWidget(w)
        elif t in ("text", "font", "path"):
            h.addWidget(field_label(label, row, self.label_w))
            w = QLineEdit(choice_text(cur), row)
            w.setObjectName(f"settings.{key}")
            w.setPlaceholderText(tr("Default location") if t == "path" else tr("Font family") if t == "font" else "")
            w.setMaximumWidth(300 if t == "path" else 220)
            w.textEdited.connect(lambda s, key=key: self.set_value(key, s))
            h.addWidget(w)
            if t == "path":
                b = QPushButton(tr("Browse…"), row)
                b.setObjectName(f"settings.{key}.browse")
                b.setAutoDefault(False)

                def browse(_c: bool = False, key: str = key, edit: QLineEdit = w) -> None:
                    d = QFileDialog.getExistingDirectory(self, tr("Browse…"), edit.text())
                    if d:
                        edit.setText(d)
                        self.set_value(key, d)

                b.clicked.connect(browse)
                h.addWidget(b)
        elif t == "color":
            h.addWidget(field_label(label, row, self.label_w))
            w = self._colour(row, key)
            h.addWidget(w)
        else:
            w = QLabel(tr(label), row)
            h.addWidget(w)
        h.addStretch(1)
        by = f.get("enabledBy")
        if isinstance(by, str) and by:
            self._enablers.append((row, by))
        return row

    def _spin(self, parent: QWidget, key: str, value: float, lo: float, hi: float, decimals: int, whole: bool) -> QWidget:
        if whole:
            s: QSpinBox | QDoubleSpinBox = QSpinBox(parent)
            s.setRange(int(max(lo, -2**31 + 1)), int(min(hi, 2**31 - 1)))
            s.setValue(round(max(lo, min(hi, value))))
            s.valueChanged.connect(lambda v, key=key: self.set_value(key, int(v)))
        else:
            s = QDoubleSpinBox(parent)
            s.setDecimals(max(0, min(6, decimals)))
            s.setRange(lo, hi)
            s.setValue(max(lo, min(hi, value)))
            s.valueChanged.connect(lambda v, key=key: self.set_value(key, float(v)))
        s.setObjectName(f"settings.{key}")
        s.setFixedWidth(84)
        s.setKeyboardTracking(False)
        return s

    def _combo(self, parent: QWidget, key: str, current: str, opts: list[tuple[str, str]], width: int,
               numeric: bool = False, rebuild: bool = False) -> QComboBox:
        c = QComboBox(parent)
        c.setObjectName(f"settings.{key}")
        for v, lbl in opts:
            c.addItem(lbl, v)
        i = c.findData(current)
        if i < 0 and current:
            c.addItem(current, current)
            i = c.count() - 1
        c.setCurrentIndex(max(0, i))
        c.setMaximumWidth(width)
        c.setMinimumContentsLength(min(30, max(8, width // 10)))
        c.setSizeAdjustPolicy(QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)

        def chosen(_i: int, c: QComboBox = c, key: str = key) -> None:
            v = str(c.currentData())
            x: Any = v
            if numeric:
                try:
                    x = int(v)
                except ValueError:
                    x = v
            self.set_value(key, x, rebuild=rebuild)

        c.currentIndexChanged.connect(chosen)
        return c

    def _colour(self, parent: QWidget, key: str) -> QWidget:
        box = QWidget(parent)
        h = QHBoxLayout(box)
        h.setContentsMargins(0, 0, 0, 0)
        swatch = Swatch(f"settings.{key}", choice_text(get(self.values, key)), box)
        hexe = QLineEdit(choice_text(get(self.values, key)), box)
        hexe.setObjectName(f"settings.{key}.hex")
        hexe.setFixedWidth(76)
        hexe.setStyleSheet("font-family: monospace;")

        def picked(s: str) -> None:
            hexe.setText(s)
            self.set_value(key, s)

        def typed(s: str) -> None:
            c = parse_hex(s)
            if c is not None:  # each valid colour is kept as it comes
                swatch.set_colour(c.name())
                self.set_value(key, c.name())

        def left() -> None:
            hexe.setText(choice_text(get(self.values, key)))

        swatch.picked.connect(picked)
        hexe.textEdited.connect(typed)
        hexe.editingFinished.connect(left)
        h.addWidget(swatch)
        h.addWidget(hexe)
        return box

    def _command_button(self, parent: QWidget, r: dict[str, Any]) -> QWidget:
        row = QWidget(parent)
        h = QHBoxLayout(row)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(field_label("Remove Media Cache Files", row))
        b = QPushButton(tr(str(r.get("label") or "")), row)
        b.setObjectName(f"settings.{r.get('id')}")
        b.setAutoDefault(False)
        command = str(r.get("command") or "")

        def run() -> None:
            def done(v: Any, _b: Any) -> None:
                v = v if isinstance(v, dict) else {}
                self.message = fmt("Deleted {n} file(s), {size}.", n=int(number(v.get("files"), 0)), size=mb(int(number(v.get("bytes"), 0))))
                self.cache_info = None
                self._build_page()

            def failed(m: str) -> None:
                self.message = m
                self._build_page()

            self.engine.execute(command, {}, on_reply=done, on_error=failed)

        b.clicked.connect(run)
        h.addWidget(b)
        h.addStretch(1)
        return row

    # ----- sections the dialog draws itself -----
    def _custom(self, parent: QWidget, name: str) -> QWidget | None:
        if name == "appearanceModes":
            return self._appearance(parent)
        if name == "labelPresets":
            return self._label_presets(parent)
        if name == "memoryInfo":
            used = self.memory()
            budget = int(number(get(self.values, "memory.frameCacheMb"), 0)) << 20
            box = QWidget(parent)
            v = QVBoxLayout(box)
            v.setContentsMargins(0, 0, 0, 0)
            v.addWidget(self._note(box, fmt("Decoded frames held for the monitors and thumbnails: {used} of {budget}.", used=mb(used), budget=mb(budget)),
                                   "settings.memoryInfo"))
            v.addWidget(self._note(box, tr("CFA4 keeps the rest of the RAM free for other applications.")))
            return box
        if name == "mediaCacheInfo":
            label = self._note(parent, "", "settings.mediaCacheInfo")

            def show(info: dict[str, Any]) -> None:
                loc = info.get("location") or tr("not available in this session")
                label.setText(fmt("Currently: {loc} — {files} file(s), {size}", loc=str(loc), files=int(number(info.get("files"), 0)),
                                  size=mb(int(number(info.get("bytes"), 0)))))

            if self.cache_info is None:
                def got(r: Any, _b: Any) -> None:
                    self.cache_info = r if isinstance(r, dict) else {}
                    try:
                        show(self.cache_info)
                    except RuntimeError:
                        pass  # the page was rebuilt meanwhile

                self.engine.execute("mediaCache.info", {}, on_reply=got)
            else:
                show(self.cache_info)
            return label
        if name == "controlSurfaces":
            return self._control_surfaces(parent)
        if name == "outputMapping":
            label = self._note(parent, "", "settings.outputMapping")

            def update() -> None:
                ch = int(number((self.devices() or {}).get("outputChannels"), 0))
                left = int(number(get(self.values, "audioHardware.mapLeft"), 0)) + 1
                right = int(number(get(self.values, "audioHardware.mapRight"), 1)) + 1
                dev = fmt(" ({ch} output channels)", ch=ch) if ch > 0 else ""
                label.setText(fmt("Programme left → device output {l}, right → device output {r}{dev}. Outputs beyond the device's channels fall back to 1 and 2.",
                                  l=left, r=right, dev=dev))

            update()
            self._watchers.append(update)
            return label
        if name == "autoSaveStatus":
            box = QWidget(parent)
            v = QVBoxLayout(box)
            v.setContentsMargins(0, 0, 0, 0)
            box.setObjectName("settings.autoSaveStatus")

            def got_status(r: Any, _b: Any) -> None:
                st = r if isinstance(r, dict) else {}
                lines = []
                if isinstance(st.get("autoSaveDir"), str):
                    lines.append(fmt("Auto-saves go to: {dir}", dir=st["autoSaveDir"]))
                if isinstance(st.get("sessionDir"), str):
                    lines.append(fmt("Recovery copy: {dir}", dir=st["sessionDir"]))
                a, j = st.get("lastAutoSaveAt"), st.get("lastJournalAt")
                if isinstance(a, str) and isinstance(j, str):
                    lines.append(fmt("Last auto-save {a} · last recovery copy {j}", a=a, j=j))
                elif isinstance(a, str):
                    lines.append(fmt("Last auto-save {a}", a=a))
                elif isinstance(j, str):
                    lines.append(fmt("Last recovery copy {j}", j=j))
                try:
                    for line in lines:
                        v.addWidget(self._note(box, line))
                except RuntimeError:
                    pass  # the page was rebuilt meanwhile

            self.engine.execute("file.autoSaveStatus", {}, on_reply=got_status)
            return box
        return None

    def _appearance(self, parent: QWidget) -> QWidget:
        """The Appearance Mode selector above the light and dark theme cards."""
        box = QWidget(parent)
        v = QVBoxLayout(box)
        v.setContentsMargins(0, 0, 0, 0)
        row = QHBoxLayout()
        row.addWidget(field_label("Appearance Mode", box))
        key = "appearance.appearanceMode"
        row.addWidget(self._combo(box, key, choice_text(get(self.values, key)), [(m, tr(lbl)) for m, lbl in APPEARANCE_MODES], 200, rebuild=True))
        row.addStretch(1)
        v.addLayout(row)
        mode = choice_text(get(self.values, key))
        light_active = mode == "light" or (mode == "auto" and theme.system_is_light() is True)
        cards = QHBoxLayout()
        cards.addWidget(self._theme_card(box, "appearance.lightTheme", "Light Theme", LIGHT_THEMES, light_active))
        cards.addWidget(self._theme_card(box, "appearance.darkTheme", "Dark Theme", DARK_THEMES, not light_active))
        v.addLayout(cards)
        return box

    def _theme_card(self, parent: QWidget, key: str, title: str, opts: tuple[tuple[str, str], ...], active: bool) -> QWidget:
        t = theme.current()
        card = QFrame(parent)
        card.setObjectName(f"settings.{key}")
        card.setStyleSheet(f"QFrame#settings\\.{key.replace('.', chr(92) + '.')} {{ background: {t.field_bg};"
                           f" border: 1px solid {t.accent if active else t.field_border}; border-radius: 6px; }}")
        v = QVBoxLayout(card)
        v.setContentsMargins(10, 10, 10, 10)
        head = QHBoxLayout()
        name = QLabel(tr(title), card)
        f = name.font()
        f.setBold(True)
        name.setFont(f)
        head.addWidget(name)
        if active:
            act = QLabel(tr("Active"), card)
            act.setObjectName(f"settings.{key}.active")
            act.setStyleSheet(f"color: {t.accent};")
            head.addWidget(act)
        head.addStretch(1)
        v.addLayout(head)
        current = choice_text(get(self.values, key))
        shown = next((o for o, _l in opts if o == current), opts[0][0])
        hl = get(self.values, "appearance.highlightColor")
        look = theme.with_appearance(theme.BY_KIND.get(shown, theme.DARKEST), hl if isinstance(hl, str) else None,
                                     bool(get(self.values, "appearance.accessibleContrast")))
        v.addWidget(ThemePreview(look, card))
        favourites = [str(x) for x in get(self.values, "appearance.favoriteThemes") or [] if isinstance(x, str)]
        for value, label in opts:
            row = QHBoxLayout()
            radio = QRadioButton(tr(label), card)
            radio.setObjectName(f"settings.{key}.{value}")
            radio.setChecked(current == value)
            radio.toggled.connect(lambda on, value=value, key=key: on and self.set_value(key, value, rebuild=True))
            row.addWidget(radio)
            row.addStretch(1)
            fav = value in favourites
            star = QToolButton(card)
            star.setObjectName(f"settings.favorite.{value}")
            star.setAutoRaise(True)
            star.setText("★" if fav else "☆")
            star.setStyleSheet(f"color: {t.accent if fav else t.text_faint};")
            star.setToolTip(f"{tr('Remove from favorites') if fav else tr('Add to favorites')}: {tr(label)}")
            star.clicked.connect(lambda _c=False, value=value: self.toggle_favourite(value))
            row.addWidget(star)
            v.addLayout(row)
        return card

    def toggle_favourite(self, value: str) -> None:
        favourites = [str(x) for x in get(self.values, "appearance.favoriteThemes") or [] if isinstance(x, str)]
        if value in favourites:
            favourites.remove(value)
        else:
            favourites.append(value)
        self.set_value("appearance.favoriteThemes", favourites, rebuild=True)

    def _label_colors(self, parent: QWidget) -> QWidget:
        box = QGroupBox(tr("Label Colors"), parent)
        box.setFixedWidth(250)
        grid = QGridLayout(box)
        grid.setHorizontalSpacing(10)
        grid.setVerticalSpacing(5)
        for i, name in enumerate(LABELS):
            lid = name.lower()
            nk, ck = f"labels.colors.{lid}.name", f"labels.colors.{lid}.color"
            edit = QLineEdit(choice_text(get(self.values, nk)), box)
            edit.setObjectName(f"settings.{nk}")
            edit.setFixedWidth(136)
            edit.textEdited.connect(lambda s, nk=nk: self.set_value(nk, s))
            grid.addWidget(edit, i, 0)
            sw = Swatch(f"settings.{ck}", choice_text(get(self.values, ck)), box)
            sw.picked.connect(lambda s, ck=ck: self.set_value(ck, s))
            grid.addWidget(sw, i, 1)
        return box

    # ----- Control Surfaces -----
    def _control_surfaces(self, parent: QWidget) -> QWidget:
        """Settings ▸ Control Surfaces: the MIDI controllers and jog/shuttle devices of this
        machine (``surface.devices``), each with Use and Profile; the choices go to the draft's
        ``controlSurfaces.devices`` (OK applies them). Refresh looks for devices again. Below them
        the MIDI Monitor."""
        page = QWidget(parent)
        col = QVBoxLayout(page)
        col.setContentsMargins(0, 0, 0, 0)
        col.addWidget(self._surface_devices(page))
        col.addWidget(self._assignments(page))
        col.addWidget(self._midi_monitor(page))
        return page

    def _surface_devices(self, parent: QWidget) -> QWidget:
        box = QGroupBox(tr("Devices"), parent)
        v = QVBoxLayout(box)
        if (self.surfaces is None or self.surface_profiles is None) and not self._surfaces_pending:
            self._surfaces_pending = self._surfaces_building = True
            try:
                self._load_surfaces()  # an engine that answers at once fills both before this returns
            finally:
                self._surfaces_building = False
        if self.surfaces is None or self.surface_profiles is None:
            v.addWidget(self._note(box, "Looking for devices…", "settings.controlSurfaces.status"))
            return box
        devices = [d for d in self.surfaces.get("devices") or [] if isinstance(d, dict)]
        grid = QGridLayout()
        grid.setHorizontalSpacing(12)
        grid.setVerticalSpacing(4)
        for c, head in enumerate((tr("Use"), tr("Device"), tr("Type"), tr("Status"), tr("Profile"))):
            h = QLabel(head, box)
            h.setStyleSheet(f"color: {theme.current().text_dim}; font-weight: 600;")
            grid.addWidget(h, 0, c)
        types = {"midi": tr("MIDI"), "hid": tr("HID jog/shuttle"), "osc": tr("OSC")}
        states = {"online": tr("Connected"), "offline": tr("Not connected"), "error": tr("Error"), "disabled": tr("Disabled")}
        for n, d in enumerate(devices, start=1):
            dev = str(d.get("device") or "")
            setting = self._surface_setting(dev)
            use = QCheckBox(box)
            use.setObjectName(f"settings.controlSurfaces.{n}.enabled")
            use.setChecked(bool(setting.get("enabled", d.get("enabled", True))))
            use.toggled.connect(lambda on, i=dev: self._set_surface(i, "enabled", bool(on)))
            grid.addWidget(use, n, 0)
            name = QLabel(str(d.get("name") or dev), box)
            name.setObjectName(f"settings.controlSurfaces.{n}.name")
            name.setToolTip(fmt("{device} ({port})", device=dev, port=str(d.get("port") or "")))
            grid.addWidget(name, n, 1)
            grid.addWidget(QLabel(types.get(str(d.get("protocol") or ""), str(d.get("protocol") or "")), box), n, 2)
            state = QLabel(states.get(str(d.get("state") or ""), str(d.get("state") or "")), box)
            state.setObjectName(f"settings.controlSurfaces.{n}.state")
            if d.get("error"):
                state.setToolTip(str(d.get("error")))
                state.setStyleSheet(f"color: {theme.current().text_dim};")
            grid.addWidget(state, n, 3)
            combo = QComboBox(box)
            combo.setObjectName(f"settings.controlSurfaces.{n}.profile")
            driver = str(d.get("driver") or "")
            for p in self.surface_profiles:
                if p.get("driver") == driver:
                    combo.addItem(tr(str(p.get("name") or p.get("id"))), str(p.get("id") or ""))
            current = str(setting.get("profile") or d.get("profile") or "")
            i = combo.findData(current)
            combo.setCurrentIndex(max(i, 0))
            combo.activated.connect(lambda _i, i=dev, c=combo: self._set_surface(i, "profile", str(c.currentData() or "")))
            grid.addWidget(combo, n, 4)
        grid.setColumnStretch(1, 1)
        v.addLayout(grid)
        if not devices:
            v.addWidget(self._note(box, "No MIDI controller or jog/shuttle device was found. Connect one and press Refresh.",
                                   "settings.controlSurfaces.empty"))
        for n, d in enumerate(devices, start=1):
            if d.get("error"):
                v.addWidget(self._note(box, fmt("{name}: {error}", name=str(d.get("name") or ""), error=str(d.get("error"))),
                                       f"settings.controlSurfaces.{n}.error"))
        for problem in self.surfaces.get("problems") or []:
            v.addWidget(self._note(box, str(problem)))
        row = QHBoxLayout()
        row.addStretch(1)
        refresh = self._button("settings.controlSurfaces.refresh", tr("Refresh"))
        refresh.clicked.connect(self._refresh_surfaces)
        row.addWidget(refresh)
        v.addLayout(row)
        return box

    # ----- Assignments and MIDI Learn -----
    def _midi_devices(self) -> list[dict[str, Any]]:
        """The devices whose buttons can be assigned: MIDI controllers and jog/shuttle devices."""
        return [d for d in (self.surfaces or {}).get("devices") or [] if isinstance(d, dict) and d.get("protocol") in ("midi", "hid")]

    def _bindings(self, device: str) -> list[dict[str, Any]]:
        return [b for b in self._surface_setting(device).get("bindings") or [] if isinstance(b, dict)]

    def _assignments(self, parent: QWidget) -> QWidget:
        """Assignments: what the buttons of a MIDI controller or a ShuttlePRO run. Pick a command,
        press Learn and press a button (or push a fader past half way) on the controller; OK saves
        it and the engine then sends the command whenever that control is pressed
        (``surfaceAction``). A ShuttlePRO's jog wheel and shuttle ring follow its profile."""
        box = QGroupBox(tr("Assignments"), parent)
        box.setObjectName("settings.controlSurfaces.assign")
        v = QVBoxLayout(box)
        devices = self._midi_devices()
        if not devices:
            v.addWidget(self._note(box, "Connect a MIDI controller or a ShuttlePRO to assign commands to its buttons.",
                                   "settings.controlSurfaces.assign.empty"))
            return box
        ids = [str(d.get("device") or "") for d in devices]
        if self.assign_device not in ids:
            self.assign_device = ids[0]
        top = QHBoxLayout()
        top.addWidget(QLabel(tr("Device"), box))
        device = QComboBox(box)
        device.setObjectName("settings.controlSurfaces.assign.device")
        for d in devices:
            device.addItem(str(d.get("name") or d.get("device")), str(d.get("device") or ""))
        device.setCurrentIndex(max(device.findData(self.assign_device), 0))
        device.activated.connect(lambda _i, c=device: self._pick_assign_device(str(c.currentData() or "")))
        top.addWidget(device, 1)
        v.addLayout(top)
        labels = dict(self.commands())
        grid = QGridLayout()
        grid.setHorizontalSpacing(12)
        bindings = self._bindings(self.assign_device)
        for c, head in enumerate((tr("Control"), tr("Command"))):
            h = QLabel(head, box)
            h.setStyleSheet(f"color: {theme.current().text_dim}; font-weight: 600;")
            grid.addWidget(h, 0, c)
        for n, b in enumerate(bindings, start=1):
            control = QLabel(control_label(b.get("control")), box)
            control.setObjectName(f"settings.controlSurfaces.assign.{n}.control")
            grid.addWidget(control, n, 0)
            cid = str(b.get("command") or "")
            command = QLabel(labels.get(cid) or cid, box)
            command.setObjectName(f"settings.controlSurfaces.assign.{n}.command")
            command.setToolTip(cid)
            grid.addWidget(command, n, 1)
            remove = QToolButton(box)
            remove.setObjectName(f"settings.controlSurfaces.assign.{n}.remove")
            remove.setText("✕")
            remove.setToolTip(tr("Remove this assignment"))
            remove.clicked.connect(lambda _c=False, b=b: self._remove_binding(b.get("control")))
            grid.addWidget(remove, n, 2)
        grid.setColumnStretch(1, 1)
        v.addLayout(grid)
        if not bindings:
            v.addWidget(self._note(box, "No assignments yet.", "settings.controlSurfaces.assign.none"))
        row = QHBoxLayout()
        command = QComboBox(box)
        command.setObjectName("settings.controlSurfaces.assign.command")
        for cid, label in sorted(labels.items(), key=lambda kv: kv[1].casefold()):
            command.addItem(label, cid)
            command.setItemData(command.count() - 1, cid, Qt.ItemDataRole.ToolTipRole)
        if self.assign_command and command.findData(self.assign_command) >= 0:
            command.setCurrentIndex(command.findData(self.assign_command))
        self.assign_command = str(command.currentData() or "")
        command.activated.connect(lambda _i, c=command: setattr(self, "assign_command", str(c.currentData() or "")))
        row.addWidget(command, 1)
        learn = self._button("settings.controlSurfaces.assign.learn", tr("Cancel Learn") if self.learning else tr("Learn"))
        learn.setCheckable(True)
        learn.setChecked(self.learning)
        learn.setEnabled(command.count() > 0)
        learn.clicked.connect(self._toggle_learn)
        row.addWidget(learn)
        v.addLayout(row)
        text = self.learn_message or ("Press or move a control on the controller…" if self.learning else
                                      "Pick a command, press Learn, then press a button on the controller. OK saves the assignments.")
        v.addWidget(self._note(box, text, "settings.controlSurfaces.assign.status"))
        return box

    def _pick_assign_device(self, device: str) -> None:
        if device != self.assign_device:
            self._cancel_learn()
            self.assign_device = device
            self.learn_message = ""
            self._build_page()

    def _set_bindings(self, device: str, bindings: list[dict[str, Any]]) -> None:
        self._set_surface(device, "bindings", bindings)

    def _remove_binding(self, control: Any) -> None:
        self._set_bindings(self.assign_device, [b for b in self._bindings(self.assign_device) if b.get("control") != control])
        self.learn_message = ""
        self._build_page()

    def _toggle_learn(self, on: bool) -> None:
        if not on:
            self._cancel_learn()
            self.learn_message = ""
            self._build_page()
            return

        def failed(m: str) -> None:
            self.learning = False
            self.learn_message = m
            self._rebuild_if_shown()

        self.learning = True
        self.learn_message = ""
        self._build_page()
        self.engine.call("surface.learn", {"device": self.assign_device}, on_error=failed)

    def _cancel_learn(self) -> None:
        if self.learning:
            self.learning = False
            self.engine.call("surface.learnCancel", {})

    def _learned(self, message: dict[str, Any]) -> None:
        if not self.learning or message.get("state") != "captured" or message.get("device") != self.assign_device:
            return
        self.learning = False
        control = message.get("control")
        if not isinstance(control, dict) or not self.assign_command:
            return
        kept = [b for b in self._bindings(self.assign_device) if b.get("control") != control]
        self._set_bindings(self.assign_device, [*kept, {"control": control, "command": self.assign_command}])
        labels = dict(self.commands())
        self.learn_message = fmt("{control} runs {command}.", control=control_label(control),
                                 command=labels.get(self.assign_command) or self.assign_command)
        self._rebuild_if_shown()

    def _rebuild_if_shown(self) -> None:
        if self.page == "controlSurfaces":
            try:
                self._build_page()
            except RuntimeError:
                pass  # the dialog is closing

    def _stop_reading(self) -> None:
        self._stop_monitor()
        self._cancel_learn()

    # ----- MIDI Monitor -----
    MONITOR_LINES = 500

    def _midi_monitor(self, parent: QWidget) -> QWidget:
        """Every MIDI message from the controllers as it arrives (``surface.monitor``, events
        ``surfaceMonitor``): time, device, bytes and what they mean. Reading stops with Stop or
        when the dialog closes."""
        box = QGroupBox(tr("MIDI Monitor"), parent)
        box.setObjectName("settings.controlSurfaces.monitor")
        v = QVBoxLayout(box)
        v.addWidget(self._note(box, "Shows every MIDI message from the controllers as it arrives, to check that a "
                                    "controller sends and what it sends. MIDI Clock and Active Sensing are left out."))
        row = QHBoxLayout()
        toggle = self._button("settings.controlSurfaces.monitor.toggle", tr("Stop Monitor") if self.monitor_on else tr("Start Monitor"))
        toggle.setCheckable(True)
        toggle.setChecked(self.monitor_on)
        toggle.clicked.connect(self._toggle_monitor)
        clear = self._button("settings.controlSurfaces.monitor.clear", tr("Clear"))
        clear.clicked.connect(self._clear_monitor)
        row.addWidget(toggle)
        row.addWidget(clear)
        row.addStretch(1)
        v.addLayout(row)
        log = QPlainTextEdit(box)
        log.setObjectName("settings.controlSurfaces.monitor.log")
        log.setReadOnly(True)
        log.setLineWrapMode(QPlainTextEdit.LineWrapMode.NoWrap)
        log.setMaximumBlockCount(self.MONITOR_LINES)
        log.setStyleSheet("font-family: monospace;")
        log.setMinimumHeight(180)
        log.setPlaceholderText(tr("Press Start Monitor, then move a control."))
        log.setPlainText("\n".join(self.monitor_lines))
        v.addWidget(log)
        error = self._note(box, self.monitor_error, "settings.controlSurfaces.monitor.error")
        error.setVisible(bool(self.monitor_error))
        v.addWidget(error)
        self._monitor = {"toggle": toggle, "log": log, "error": error}
        return box

    def _monitor_widget(self, key: str) -> Any:
        w = self._monitor.get(key)
        try:
            if w is not None:
                w.objectName()  # raises once Qt deleted it (the page was rebuilt or closed)
            return w
        except RuntimeError:
            return None

    def _show_monitor_state(self) -> None:
        toggle, error = self._monitor_widget("toggle"), self._monitor_widget("error")
        if toggle is not None:
            toggle.setChecked(self.monitor_on)
            toggle.setText(tr("Stop Monitor") if self.monitor_on else tr("Start Monitor"))
        if error is not None:
            error.setText(self.monitor_error)
            error.setVisible(bool(self.monitor_error))

    def _toggle_monitor(self, on: bool) -> None:
        def done(r: Any, _b: Any) -> None:
            self.monitor_on = bool(r.get("enabled")) if isinstance(r, dict) else False
            self._show_monitor_state()

        def failed(m: str) -> None:
            self.monitor_on = False
            self.monitor_error = m
            self._show_monitor_state()

        self.monitor_error = ""
        self.monitor_on = bool(on)
        self._show_monitor_state()
        self.engine.call("surface.monitor", {"enabled": bool(on)}, on_reply=done, on_error=failed)

    def _stop_monitor(self) -> None:
        if self.monitor_on:
            self.monitor_on = False
            self.engine.call("surface.monitor", {"enabled": False})

    def _clear_monitor(self) -> None:
        self.monitor_lines.clear()
        log = self._monitor_widget("log")
        if log is not None:
            log.clear()

    def _monitor_line(self, m: dict[str, Any]) -> str:
        device = str(m.get("device") or "")
        names = {str(d.get("device")): str(d.get("name") or "") for d in (self.surfaces or {}).get("devices") or [] if isinstance(d, dict)}
        name = names.get(device) or device.removeprefix("midi:")
        try:
            at = float(m.get("at") or 0) / 1e9
        except (TypeError, ValueError):
            at = 0.0
        return f"{at:9.3f}  {name[:24]:<24}  {str(m.get('raw') or ''):<11}  {m.get('decoded') or ''}"

    def _engine_event(self, message: Any) -> None:
        if not isinstance(message, dict):
            return
        kind = message.get("event")
        if kind == "surfaceLearn":
            self._learned(message)
            return
        if kind == "surfaceError":
            if self.learning:
                self.learning = False
                self.learn_message = str(message.get("error") or "")
                self._rebuild_if_shown()
            if self.monitor_on:
                self.monitor_on = False
                self.monitor_error = str(message.get("error") or "")
                self._show_monitor_state()
            return
        if not self.monitor_on:
            return
        if kind != "surfaceMonitor":
            return
        new = [self._monitor_line(m) for m in message.get("messages") or [] if isinstance(m, dict)]
        dropped = message.get("dropped")
        if isinstance(dropped, int) and dropped > 0:
            new.append(fmt("… {count} messages not shown (too many at once)", count=dropped))
        if not new:
            return
        self.monitor_lines = (self.monitor_lines + new)[-self.MONITOR_LINES:]
        log = self._monitor_widget("log")
        if log is not None:
            log.appendPlainText("\n".join(new))

    def _load_surfaces(self) -> None:
        def devices(r: Any, _b: Any) -> None:
            self.surfaces = r if isinstance(r, dict) else {"devices": []}
            self._surfaces_loaded()

        def profiles(r: Any, _b: Any) -> None:
            got = r.get("profiles") if isinstance(r, dict) else None
            self.surface_profiles = [p for p in got or [] if isinstance(p, dict)]
            self._surfaces_loaded()

        def failed(m: str) -> None:
            self.surfaces = {"devices": [], "problems": [m]}
            self._surfaces_loaded()

        self.engine.call("surface.devices", {}, on_reply=devices, on_error=failed)
        self.engine.execute("controlSurfaces.profiles", {}, on_reply=profiles,
                            on_error=lambda m: profiles({"profiles": []}, None))

    def _surfaces_loaded(self) -> None:
        if self.surfaces is None or self.surface_profiles is None or not self._surfaces_pending:
            return
        self._surfaces_pending = False
        if self._surfaces_building:
            return  # answered while the page was being built: that build shows it
        if self.page == "controlSurfaces":
            try:
                self._build_page()
            except RuntimeError:
                pass  # the dialog is closing

    def _refresh_surfaces(self) -> None:
        self.surfaces = None
        self._surfaces_pending = False
        self._build_page()

    def _surface_setting(self, device: str) -> dict[str, Any]:
        for d in get(self.values, "controlSurfaces.devices") or []:
            if isinstance(d, dict) and d.get("device") == device:
                return d
        return {}

    def _set_surface(self, device: str, key: str, value: Any) -> None:
        devices = [dict(d) for d in get(self.values, "controlSurfaces.devices") or [] if isinstance(d, dict)]
        entry = next((d for d in devices if d.get("device") == device), None)
        if entry is None:
            entry = {"device": device, "enabled": True, "profile": "", "bindings": []}
            devices.append(entry)
        entry[key] = value
        put(self.values, "controlSurfaces.devices", devices)

    def _label_presets(self, parent: QWidget) -> QWidget:
        """Label Color Presets: picking one or Revert edits the draft (OK applies it); Import,
        Export and Delete act on the files at once."""
        box = QGroupBox(tr("Label Color Presets"), parent)
        v = QVBoxLayout(box)
        if self.label_presets is None:
            def got(r: Any, _b: Any) -> None:
                self.label_presets = r if isinstance(r, dict) else {"presets": []}
                self._build_page()

            self.engine.execute("labels.presets", {}, on_reply=got, on_error=lambda m: got({"presets": []}, None))
            v.addWidget(self._note(box, "…"))
            return box
        presets = [p for p in self.label_presets.get("presets") or [] if isinstance(p, dict)]
        current = choice_text(get(self.values, "labels.preset"))
        selected = next((p for p in presets if p.get("name") == current), None)

        def matches(p: dict[str, Any]) -> bool:
            labels = [x for x in p.get("labels") or [] if isinstance(x, dict)]
            return len(labels) == len(LABELS) and all(
                get(self.values, f"labels.colors.{n.lower()}.name") == x.get("name")
                and str(get(self.values, f"labels.colors.{n.lower()}.color") or "").lower() == str(x.get("color") or "").lower()
                for n, x in zip(LABELS, labels, strict=False))

        modified = selected is not None and not matches(selected)
        row = QHBoxLayout()
        combo = QComboBox(box)
        combo.setObjectName("settings.labels.preset")
        for p in presets:
            combo.addItem(str(p.get("name")), str(p.get("name")))
        i = combo.findData(current)
        if i < 0:
            combo.addItem(current, current)
            i = combo.count() - 1
        combo.setCurrentIndex(i)
        if modified:
            combo.setItemText(i, fmt("{name} (Modified)", name=current))
        combo.setFixedWidth(150)

        def pick(p: dict[str, Any] | None) -> None:
            if p is None:
                return
            for n, x in zip(LABELS, [x for x in p.get("labels") or [] if isinstance(x, dict)], strict=False):
                put(self.values, f"labels.colors.{n.lower()}.name", str(x.get("name") or n))
                put(self.values, f"labels.colors.{n.lower()}.color", str(x.get("color") or "#808080"))
            put(self.values, "labels.preset", str(p.get("name") or ""))
            self._build_page()

        combo.activated.connect(lambda _i: pick(next((p for p in presets if p.get("name") == combo.currentData()), None)))
        row.addWidget(combo)

        def tool(name: str, text: str, tip: str, enabled: bool) -> QToolButton:
            b = QToolButton(box)
            b.setObjectName(f"settings.labelPresets.{name}")
            b.setText(text)
            b.setToolTip(tip)
            b.setEnabled(enabled)
            row.addWidget(b)
            return b

        tool("revert", "↺", tr("Revert to the preset's colors and names"), modified).clicked.connect(lambda: pick(selected))

        def failed(m: str) -> None:
            self.preset_message = m
            self._build_page()

        def import_file() -> None:
            path, _f = QFileDialog.getOpenFileName(self, tr("Label Color Presets"), "", "JSON (*.json)")
            if path:
                def done(r: Any, _b: Any) -> None:
                    name = str((r or {}).get("name") or "") if isinstance(r, dict) else ""
                    self.preset_message = fmt("Imported “{name}”", name=name)
                    presets_now = (r or {}).get("presets") if isinstance(r, dict) else None
                    self.label_presets = presets_now if isinstance(presets_now, dict) else None
                    p = next((x for x in (presets_now or {}).get("presets") or [] if isinstance(x, dict) and x.get("name") == name), None) \
                        if isinstance(presets_now, dict) else None
                    if p is not None:
                        pick(p)
                    else:
                        self._build_page()
                self.engine.execute("labels.importPreset", {"path": path}, on_reply=done, on_error=failed)

        def export_file() -> None:
            path, _f = QFileDialog.getSaveFileName(self, tr("Label Color Presets"), f"{current}.json", "JSON (*.json)")
            if path:
                self.engine.execute("labels.exportPreset", {"path": path},
                                    on_reply=lambda _r, _b: (setattr(self, "preset_message", fmt("Exported to {path}", path=path)), self._build_page()),
                                    on_error=failed)

        folder = self.label_presets.get("dir")
        tool("import", "⇩", tr("Import a preset file…"), True).clicked.connect(import_file)
        tool("export", "⇧", tr("Export the labels to a preset file…"), True).clicked.connect(export_file)
        tool("folder", "📁", tr("Show the presets folder"), isinstance(folder, str)).clicked.connect(
            lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(str(folder))))
        user = selected if selected is not None and not selected.get("builtin") else None

        def delete() -> None:
            if user is None:
                return
            name = str(user.get("name"))

            def done(_r: Any, _b: Any) -> None:
                put(self.values, "labels.preset", "Default")
                self.preset_message = fmt("Deleted preset “{active}”", active=name)
                self.label_presets = None
                self._build_page()

            self.engine.execute("labels.deletePreset", {"name": name}, on_reply=done, on_error=failed)

        tool("delete", "🗑", tr("Delete this preset"), user is not None).clicked.connect(delete)
        row.addStretch(1)
        v.addLayout(row)
        if self.preset_message:
            v.addWidget(self._note(box, self.preset_message, "settings.labelPresets.message"))
        return box
