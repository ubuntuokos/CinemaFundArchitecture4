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


class PreferencesDialog(QDialog):
    """``engine`` answers ``prefs.schema`` / ``prefs.get`` and runs edits; ``applied(changes)``
    after OK applied them. ``devices()`` gives the audio devices ``{"hosts", "inputs", "outputs",
    "outputChannels"}``; ``memory()`` the bytes of decoded pictures the window holds."""

    applied = Signal(dict)

    def __init__(self, parent: QWidget | None, engine: Any, page: str = "general",
                 devices: Callable[[], dict[str, Any]] | None = None, memory: Callable[[], int] | None = None) -> None:
        super().__init__(parent)
        self.engine = engine
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
