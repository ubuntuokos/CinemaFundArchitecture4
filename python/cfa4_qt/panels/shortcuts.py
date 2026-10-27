"""Edit ▸ Keyboard Shortcuts… (Ctrl+Alt+K), as egui's ``panels/shortcuts_dialog.rs``: a keyboard
drawn with every assigned key coloured (application-wide / panel-specific / both), a searchable
command list with a Shortcut column (click it and press keys to assign; "+" adds another
shortcut), the "Key:" list, conflict warnings, preset load / Save As / Delete / Export / Import,
and Undo / Redo / Clear. Edits apply live (the menus follow at once); Cancel restores the set the
dialog opened with (``shortcuts.beginEditing`` / ``endEditing`` / ``cancelEditing``).

All state lives in the engine's shortcut set (``shortcuts.*`` commands), so agents do the same
over MCP. Automation ids (egui's): ``shortcuts.preset``, ``shortcuts.saveAs``,
``shortcuts.saveAs.name``, ``shortcuts.saveAs.ok``, ``shortcuts.saveAs.cancel``,
``shortcuts.delete``, ``shortcuts.export``, ``shortcuts.import``, ``shortcuts.context``,
``shortcuts.search``, ``shortcuts.key.<Key>``, ``shortcuts.mod.<Cmd|Ctrl|Alt|Shift>[.right]``,
``shortcuts.list`` (rows carry the command id; :meth:`ShortcutsDialog.row`), ``shortcuts.keyList``,
``shortcuts.assignKey``, ``shortcuts.conflict``, ``shortcuts.message``, ``shortcuts.undo``,
``shortcuts.redo``, ``shortcuts.clear``, ``shortcuts.cancel``, ``shortcuts.ok``.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QEvent, QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QColor, QFont, QKeyEvent, QKeySequence, QPainter, QPaintEvent, QPen, QPolygonF
from PySide6.QtWidgets import (
    QAbstractButton,
    QComboBox,
    QDialog,
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QSizePolicy,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, fmt, tr

# key colours (egui's palette): application-wide, panel-specific, unassigned, held modifier
APP_KEY = QColor(0x74, 0x57, 0xC8)
PANEL_KEY = QColor(0x4A, 0xA6, 0x84)
FREE_KEY = QColor(0x5A, 0x5A, 0x5E)
MOD_KEY = QColor(0x2F, 0x62, 0xC0)
WARN = "#e8a03c"
APPLICATION = "Application"
MOD_ORDER = ("Cmd", "Ctrl", "Alt", "Shift")

# (key name, width in key units); "" = gap; "#…" = a modifier or Caps Lock (egui's MAIN_ROWS)
MAIN_ROWS: tuple[tuple[tuple[str, float], ...], ...] = (
    (("Escape", 1), ("", 0.5), ("F1", 1), ("F2", 1), ("F3", 1), ("F4", 1), ("", 0.25), ("F5", 1), ("F6", 1), ("F7", 1), ("F8", 1),
     ("", 0.25), ("F9", 1), ("F10", 1), ("F11", 1), ("F12", 1)),
    (("`", 1), *((d, 1) for d in "1234567890"), ("-", 1), ("=", 1), ("Backspace", 2)),
    (("Tab", 1.5), *((c, 1) for c in "QWERTYUIOP"), ("[", 1), ("]", 1), ("\\", 1.5)),
    (("#Caps", 1.75), *((c, 1) for c in "ASDFGHJKL"), (";", 1), ("'", 1), ("Enter", 2.25)),
    (("#Shift", 2.25), *((c, 1) for c in "ZXCVBNM"), (",", 1), (".", 1), ("/", 1), ("#Shift", 2.75)),
    (("#Ctrl", 1.5), ("#Alt", 1.25), ("#Cmd", 1.5), ("Space", 6.5), ("#Cmd", 1.5), ("#Alt", 1.25), ("#Ctrl", 1.5)),
)
# the navigation cluster: (key, x offset in units) per row
NAV_ROWS: tuple[tuple[tuple[str, float], ...], ...] = (
    (),
    (("Insert", 0), ("Home", 1), ("PageUp", 2)),
    (("Delete", 0), ("End", 1), ("PageDown", 2)),
    (),
    (("Up", 1),),
    (("Left", 0), ("Down", 1), ("Right", 2)),
)
LEGENDS = {
    "#Caps": "Caps Lock",
    "#Shift": "⇧ Shift",
    "#Ctrl": "Win",
    "#Alt": "Alt",
    "#Cmd": "Ctrl",
    "Enter": "↩ Return",
    "Escape": "Esc",
    "PageUp": "Page Up",
    "PageDown": "Page Down",
    "Up": "↑",
    "Down": "↓",
    "Left": "←",
    "Right": "→",
}

_K = Qt.Key
# Qt keys -> the engine's key names (US layout; shifted symbols name their unshifted key)
QT_KEYS: dict[Any, str] = {
    _K.Key_Escape: "Escape",
    _K.Key_Tab: "Tab",
    _K.Key_Backtab: "Tab",
    _K.Key_Backspace: "Backspace",
    _K.Key_Return: "Enter",
    _K.Key_Enter: "Enter",
    _K.Key_Insert: "Insert",
    _K.Key_Delete: "Delete",
    _K.Key_Home: "Home",
    _K.Key_End: "End",
    _K.Key_PageUp: "PageUp",
    _K.Key_PageDown: "PageDown",
    _K.Key_Up: "Up",
    _K.Key_Down: "Down",
    _K.Key_Left: "Left",
    _K.Key_Right: "Right",
    _K.Key_Space: "Space",
    _K.Key_QuoteLeft: "`",
    _K.Key_AsciiTilde: "`",
    _K.Key_Minus: "-",
    _K.Key_Underscore: "-",
    _K.Key_Equal: "=",
    _K.Key_Plus: "=",
    _K.Key_BracketLeft: "[",
    _K.Key_BraceLeft: "[",
    _K.Key_BracketRight: "]",
    _K.Key_BraceRight: "]",
    _K.Key_Backslash: "\\",
    _K.Key_Bar: "\\",
    _K.Key_Semicolon: ";",
    _K.Key_Colon: ";",
    _K.Key_Apostrophe: "'",
    _K.Key_QuoteDbl: "'",
    _K.Key_Comma: ",",
    _K.Key_Less: ",",
    _K.Key_Period: ".",
    _K.Key_Greater: ".",
    _K.Key_Slash: "/",
    _K.Key_Question: "/",
    _K.Key_Exclam: "1",
    _K.Key_At: "2",
    _K.Key_NumberSign: "3",
    _K.Key_Dollar: "4",
    _K.Key_Percent: "5",
    _K.Key_AsciiCircum: "6",
    _K.Key_Ampersand: "7",
    _K.Key_Asterisk: "8",
    _K.Key_ParenLeft: "9",
    _K.Key_ParenRight: "0",
}
for _i in range(26):
    QT_KEYS[Qt.Key(_K.Key_A.value + _i)] = chr(ord("A") + _i)
for _i in range(10):
    QT_KEYS[Qt.Key(_K.Key_0.value + _i)] = str(_i)
for _i in range(12):
    QT_KEYS[Qt.Key(_K.Key_F1.value + _i)] = f"F{_i + 1}"
MODIFIER_KEYS = {_K.Key_Shift, _K.Key_Control, _K.Key_Alt, _K.Key_Meta, _K.Key_Super_L, _K.Key_Super_R, _K.Key_AltGr}


def wrap_lines(text: str, fm: Any, width: int, rows: int) -> list[str]:
    """``text`` word-wrapped into at most ``rows`` lines of ``width`` pixels, the last elided."""
    lines: list[str] = []
    words = text.split()
    while words and len(lines) < rows:
        line = words.pop(0)
        while words and fm.horizontalAdvance(line + " " + words[0]) <= width:
            line += " " + words.pop(0)
        if len(lines) == rows - 1 and words:
            line = line + " " + " ".join(words)
            words = []
        lines.append(fm.elidedText(line, Qt.TextElideMode.ElideRight, width))
    return lines


def key_legend(name: str) -> str:
    return LEGENDS.get(name, name)


def chord_of(key: Any, mods: Qt.KeyboardModifier) -> str | None:
    """The engine's chord text for a key press (Ctrl is the primary modifier, egui's ``Cmd``);
    None for a key that can't be a shortcut."""
    name = QT_KEYS.get(key)
    if name is None:
        return None
    m = Qt.KeyboardModifier
    prefix = "".join(p for on, p in ((mods & m.ControlModifier, "Cmd+"), (mods & m.AltModifier, "Alt+"), (mods & m.ShiftModifier, "Shift+")) if on)
    return prefix + name


def mods_text(mods: set[str]) -> str:
    return "+".join(m for m in MOD_ORDER if m in mods)


def _strings(v: Any) -> list[str]:
    return [x for x in v if isinstance(x, str)] if isinstance(v, list) else []


class KeyCap(QAbstractButton):
    """One key of the drawn keyboard."""

    def __init__(self, parent: QWidget, name: str, right: bool = False) -> None:
        super().__init__(parent)
        self.name = name
        self.is_mod = name.startswith("#")
        if self.is_mod:
            self.setObjectName(f"shortcuts.mod.{name[1:]}" + (".right" if right else ""))
        else:
            self.setObjectName(f"shortcuts.key.{name}")
        self.app_cmd: str | None = None
        self.panel_cmd: str | None = None
        self.caption = ""
        self.on = False  # a modifier in the filter
        self.current = False  # the key the "Key:" list shows
        self.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.setMouseTracking(True)

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        r = QRectF(self.rect()).adjusted(1.5, 1.5, -1.5, -1.5)
        if self.is_mod:
            base = MOD_KEY if self.on else FREE_KEY.darker(125)
        elif self.app_cmd:
            base = APP_KEY
        elif self.panel_cmd:
            base = PANEL_KEY
        else:
            base = FREE_KEY
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(base)
        p.drawRoundedRect(r, 3, 3)
        if self.app_cmd and self.panel_cmd:
            # both: the lower-right triangle in the panel colour
            p.setBrush(PANEL_KEY)
            p.drawPolygon(QPolygonF([r.topRight(), r.bottomRight(), r.bottomLeft()]))
        if self.underMouse():
            p.setPen(QPen(QColor("white"), 1.5))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRoundedRect(r, 3, 3)
        if self.current:
            p.setPen(QPen(self.palette().highlight().color(), 2))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRoundedRect(r.adjusted(1, 1, -1, -1), 3, 3)
        f = QFont(self.font())
        f.setPixelSize(max(8, min(12, int(self.height() * 0.2))))
        p.setFont(f)
        p.setPen(QColor("white"))
        inner = r.adjusted(4, 3, -4, -3)
        if self.caption:
            fm = p.fontMetrics()
            # as many lines as fit above the key's legend (egui: 2 on tall keys), the last elided
            rows = max(1, min(2, int((inner.height() - fm.height()) // fm.height())))
            for i, line in enumerate(wrap_lines(self.caption, fm, int(inner.width()), rows)):
                p.drawText(QPointF(inner.left(), inner.top() + fm.ascent() + i * fm.height()), line)
        p.setPen(QColor(255, 255, 255, 220))
        p.drawText(inner, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignBottom, key_legend(self.name))


class KeyboardView(QWidget):
    """egui's drawn US keyboard: keys coloured by what they run with the modifiers selected."""

    key_clicked = Signal(str)
    mod_clicked = Signal(str)

    UNITS_W = 15.0 + 0.5 + 3.0

    def __init__(self, parent: QWidget) -> None:
        super().__init__(parent)
        self.setObjectName("shortcuts.keyboard")
        self.setMinimumSize(560, 220)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        self.caps: list[tuple[KeyCap, int, float, float, float]] = []  # cap, row, x units, w units
        for ri, row in enumerate(MAIN_ROWS):
            x = 0.0
            for name, w in row:
                if name:
                    cap = KeyCap(self, name, right=name.startswith("#") and x > 7.5)
                    self._wire(cap)
                    self.caps.append((cap, ri, x, float(w), 0.0))
                x += w
        for ri, row in enumerate(NAV_ROWS):
            for name, off in row:
                cap = KeyCap(self, name)
                self._wire(cap)
                self.caps.append((cap, ri, 15.5 + off, 1.0, 0.0))

    def _wire(self, cap: KeyCap) -> None:
        if cap.is_mod:
            if cap.name != "#Caps":
                cap.clicked.connect(lambda _c=False, n=cap.name[1:]: self.mod_clicked.emit(n))
        else:
            cap.clicked.connect(lambda _c=False, n=cap.name: self.key_clicked.emit(n))

    def cap(self, name: str) -> KeyCap | None:
        return next((c for c, *_r in self.caps if c.name == name), None)

    def heightForWidth(self, w: int) -> int:
        return int(w / self.UNITS_W * 6.2)

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        u = min(self.width() / self.UNITS_W, self.height() / 6.2)
        row_h = u * 0.98
        for cap, ri, x, w, _ in self.caps:
            y = ri * row_h + (u * 0.15 if ri > 0 else 0.0)
            cap.setGeometry(round(x * u), round(y), max(1, round(w * u)), max(1, round(row_h)))

    def show_map(self, keys: dict[str, Any], label: Callable[[str], str], mods: set[str], current: str | None) -> None:
        for cap, *_r in self.caps:
            if cap.is_mod:
                cap.on = cap.name[1:] in mods
                cap.setToolTip(key_legend(cap.name))
            else:
                e = keys.get(cap.name) if isinstance(keys.get(cap.name), dict) else {}
                a, pn = e.get("application"), e.get("panel")
                cap.app_cmd = a if isinstance(a, str) else None
                cap.panel_cmd = pn if isinstance(pn, str) else None
                shown = cap.app_cmd or cap.panel_cmd
                cap.caption = label(shown) if shown else ""
                tips = [label(c) for c in (cap.app_cmd, cap.panel_cmd) if c]
                cap.setToolTip(" / ".join(tips) or key_legend(cap.name))
                cap.current = cap.name == current
            cap.update()


class Recorder(QWidget):
    """Takes the next key press while a Shortcut cell records (every key, Tab and Esc too)."""

    pressed = Signal(object, object)

    def __init__(self, parent: QWidget) -> None:
        super().__init__(parent)
        self.setObjectName("shortcuts.recorder")
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self.setFixedSize(1, 1)

    def event(self, e: QEvent) -> bool:
        if e.type() == QEvent.Type.ShortcutOverride:
            e.accept()
            return True
        if e.type() == QEvent.Type.KeyPress and isinstance(e, QKeyEvent):
            if e.key() not in MODIFIER_KEYS:
                self.pressed.emit(e.key(), e.modifiers())
            return True
        return super().event(e)


class ShortcutsDialog(QDialog):
    """``changed`` follows every edit; ``on_closed(ok)`` runs once the engine kept (OK) or
    restored (Cancel) the set, after the dialog may already be gone."""

    changed = Signal()

    def __init__(self, parent: QWidget, engine: EngineClient, on_closed: Callable[[bool], None] | None = None) -> None:
        super().__init__(parent)
        self.on_closed = on_closed
        self.setObjectName("shortcuts")
        self.setWindowTitle(tr("Keyboard Shortcuts"))
        self.engine = engine
        self.context = APPLICATION
        self.selected: str | None = None
        self.recording: tuple[str, bool] | None = None
        self.mods: set[str] = set()
        self.held: set[str] = set()
        self.key: str | None = None
        self.message = ""
        self.presets: dict[str, Any] = {}
        self.rows: list[dict[str, Any]] = []
        self.labels: dict[str, str] = {}
        self.conflicts: list[dict[str, Any]] = []
        self.ready = False
        self._closing = False
        self._build()
        self.engine.execute("shortcuts.beginEditing", {}, on_reply=lambda r, _b: self._started(r), on_error=self._failed)

    # ----- layout -----
    def _button(self, name: str, text: str, slot: Callable[[], None]) -> QPushButton:
        b = QPushButton(text, self)
        b.setObjectName(name)
        b.setAutoDefault(False)
        b.clicked.connect(lambda _c=False: slot())
        return b

    def _build(self) -> None:
        lay = QVBoxLayout(self)
        head = QHBoxLayout()
        head.addWidget(QLabel(tr("Preset:"), self))
        self.preset = QComboBox(self)
        self.preset.setObjectName("shortcuts.preset")
        self.preset.setMinimumWidth(260)
        self.preset.activated.connect(self._preset_chosen)
        head.addWidget(self.preset)
        head.addSpacing(8)
        self.save_as = self._button("shortcuts.saveAs", tr("Save As…"), self._open_save_as)
        self.delete = self._button("shortcuts.delete", tr("Delete"), self._delete_preset)
        self.export = self._button("shortcuts.export", tr("Export…"), self._export)
        self.import_ = self._button("shortcuts.import", tr("Import…"), self._import)
        for b in (self.save_as, self.delete, self.export, self.import_):
            head.addWidget(b)
        head.addStretch(1)
        kb = QLabel(tr("Keyboard: US"), self)
        kb.setEnabled(False)
        head.addWidget(kb)
        lay.addLayout(head)
        # Save As…: the name row
        self.save_row = QWidget(self)
        sr = QHBoxLayout(self.save_row)
        sr.setContentsMargins(0, 0, 0, 0)
        sr.addWidget(QLabel(tr("Save preset as:"), self.save_row))
        self.save_name = QLineEdit(self.save_row)
        self.save_name.setObjectName("shortcuts.saveAs.name")
        self.save_name.setFixedWidth(260)
        self.save_name.returnPressed.connect(self._save_preset)
        self.save_name.textChanged.connect(lambda t: self.save_ok.setEnabled(bool(t.strip())))
        sr.addWidget(self.save_name)
        self.save_ok = self._button("shortcuts.saveAs.ok", tr("Save"), self._save_preset)
        sr.addWidget(self.save_ok)
        sr.addWidget(self._button("shortcuts.saveAs.cancel", tr("Cancel"), self.save_row.hide))
        sr.addStretch(1)
        self.save_row.hide()
        lay.addWidget(self.save_row)
        ctx = QHBoxLayout()
        ctx.addWidget(QLabel(tr("Commands:"), self))
        self.context_box = QComboBox(self)
        self.context_box.setObjectName("shortcuts.context")
        self.context_box.setMinimumWidth(260)
        self.context_box.addItem(tr(APPLICATION), APPLICATION)
        self.context_box.activated.connect(self._context_chosen)
        ctx.addWidget(self.context_box)
        ctx.addStretch(1)
        lay.addLayout(ctx)
        self.keyboard = KeyboardView(self)
        self.keyboard.key_clicked.connect(self._key_clicked)
        self.keyboard.mod_clicked.connect(self._mod_clicked)
        lay.addWidget(self.keyboard, 3)
        legend = QHBoxLayout()
        for color, text in (
            (APP_KEY, N_("Application shortcuts work whatever panel has focus")),
            (PANEL_KEY, N_("Panel shortcuts override application shortcuts while that panel has focus")),
            (MOD_KEY, N_("Modifier held / selected")),
        ):
            sw = QLabel(self)
            sw.setFixedSize(14, 14)
            sw.setStyleSheet(f"background: {color.name()}; border-radius: 2px;")
            legend.addWidget(sw)
            t = QLabel(tr(text), self)
            t.setEnabled(False)
            legend.addWidget(t)
            legend.addSpacing(14)
        legend.addStretch(1)
        lay.addLayout(legend)
        body = QHBoxLayout()
        left = QVBoxLayout()
        top = QHBoxLayout()
        self.search = QLineEdit(self)
        self.search.setObjectName("shortcuts.search")
        self.search.setPlaceholderText(tr("Search commands or keys"))
        self.search.setFixedWidth(320)
        self.search.textChanged.connect(lambda _t: self._fill_list())
        top.addWidget(self.search)
        self.selected_label = QLabel("", self)
        self.selected_label.setEnabled(False)
        top.addWidget(self.selected_label, 1)
        left.addLayout(top)
        self.list = QTreeWidget(self)
        self.list.setObjectName("shortcuts.list")
        self.list.setColumnCount(3)
        self.list.setHeaderLabels([tr("Command"), tr("Shortcut"), ""])
        self.list.setRootIsDecorated(True)
        self.list.setAlternatingRowColors(True)
        self.list.setUniformRowHeights(True)
        self.list.header().setStretchLastSection(False)
        self.list.setColumnWidth(0, 330)
        self.list.setColumnWidth(1, 230)
        self.list.setColumnWidth(2, 28)
        self.list.itemClicked.connect(self._row_clicked)
        left.addWidget(self.list, 1)
        body.addLayout(left, 3)
        right = QVBoxLayout()
        kh = QHBoxLayout()
        k = QLabel(tr("Key:"), self)
        k.setStyleSheet("font-weight: 600;")
        kh.addWidget(k)
        self.key_label = QLabel("", self)
        self.key_label.setObjectName("shortcuts.keyName")
        kh.addWidget(self.key_label, 1)
        right.addLayout(kh)
        self.key_list = QTreeWidget(self)
        self.key_list.setObjectName("shortcuts.keyList")
        self.key_list.setColumnCount(2)
        self.key_list.setHeaderLabels([tr("Modifiers"), tr("Command")])
        self.key_list.setRootIsDecorated(False)
        self.key_list.setAlternatingRowColors(True)
        self.key_list.setColumnWidth(0, 110)
        right.addWidget(self.key_list, 1)
        self.key_hint = QLabel(tr("Click a key on the keyboard to see its shortcuts"), self)
        self.key_hint.setEnabled(False)
        self.key_hint.setWordWrap(True)
        right.addWidget(self.key_hint)
        self.assign = self._button("shortcuts.assignKey", "", self._assign_key)
        self.assign.hide()
        right.addWidget(self.assign)
        body.addLayout(right, 2)
        lay.addLayout(body, 3)
        foot = QHBoxLayout()
        msgs = QVBoxLayout()
        self.conflict_labels: list[QLabel] = []
        for _i in range(2):
            c = QLabel("", self)
            c.setObjectName("shortcuts.conflict")
            c.setStyleSheet(f"color: {WARN};")
            c.hide()
            msgs.addWidget(c)
            self.conflict_labels.append(c)
        self.message_label = QLabel("", self)
        self.message_label.setObjectName("shortcuts.message")
        self.message_label.setWordWrap(True)
        self.message_label.setEnabled(False)
        msgs.addWidget(self.message_label)
        foot.addLayout(msgs, 1)
        self.undo = self._button("shortcuts.undo", tr("Undo"), lambda: self._run("shortcuts.undo", {}))
        self.redo = self._button("shortcuts.redo", tr("Redo"), lambda: self._run("shortcuts.redo", {}))
        self.clear = self._button("shortcuts.clear", tr("Clear"), self._clear)
        for b in (self.undo, self.redo, self.clear):
            foot.addWidget(b)
        foot.addSpacing(16)
        self.cancel = self._button("shortcuts.cancel", tr("Cancel"), self.reject)
        self.ok = self._button("shortcuts.ok", tr("OK"), self.accept)
        self.ok.setDefault(True)
        foot.addWidget(self.cancel)
        foot.addWidget(self.ok)
        lay.addLayout(foot)
        self.recorder = Recorder(self)
        self.recorder.pressed.connect(self._recorded)
        self.resize(1180, 820)
        self._show_message()

    # ----- engine -----
    def _failed(self, message: str) -> None:
        self.message = message
        self._show_message()

    def _run(self, cmd: str, params: dict[str, Any], done: Callable[[Any], None] | None = None) -> None:
        def ok(result: Any, _b: bytes | None) -> None:
            if done is not None:
                done(result)
            self.refresh()
            self.changed.emit()

        self.engine.execute(cmd, params, on_reply=ok, on_error=self._failed)

    def _started(self, presets: Any) -> None:
        self._got_presets(presets)
        for name in _strings(self.presets.get("panels")):
            self.context_box.addItem(fmt("{panel} Panel", panel=tr(name)), name)
        self.refresh()

    def panel(self) -> str | None:
        return None if self.context == APPLICATION else self.context

    def refresh(self) -> None:
        """Re-read presets, the command list, the keyboard, conflicts and the "Key:" list."""
        e = self.engine
        e.execute("shortcuts.presets", {}, on_reply=lambda r, _b: self._got_presets(r), on_error=self._failed)
        e.execute("shortcuts.list", {}, on_reply=lambda r, _b: self._got_list(r), on_error=self._failed)
        self._refresh_keyboard()
        e.execute("shortcuts.conflicts", {}, on_reply=lambda r, _b: self._got_conflicts(r), on_error=self._failed)
        self._refresh_key_list()

    def _refresh_keyboard(self) -> None:
        params: dict[str, Any] = {"mods": mods_text(self.mods | self.held)}
        if self.panel():
            params["panel"] = self.panel()
        self.engine.execute("shortcuts.keyboard", params, on_reply=lambda r, _b: self._got_keyboard(r), on_error=self._failed)

    def _refresh_key_list(self) -> None:
        if self.key is None:
            self._got_key_list([])
            return
        self.engine.execute("shortcuts.forKey", {"key": self.key}, on_reply=lambda r, _b: self._got_key_list(r), on_error=self._failed)

    def label(self, cid: str) -> str:
        return tr(self.labels.get(cid, cid))

    def _got_presets(self, r: Any) -> None:
        if not isinstance(r, dict):
            return
        self.presets = r
        active = str(r.get("active") or "")
        modified = bool(r.get("modified"))
        builtin, custom = _strings(r.get("builtin")), _strings(r.get("custom"))
        self.preset.blockSignals(True)
        self.preset.clear()
        if modified:
            self.preset.addItem(fmt("{active} (modified)", active=active), None)
        for name in [*builtin, *custom]:
            self.preset.addItem(name, name)
        self.preset.setCurrentIndex(0 if modified else max(0, self.preset.findData(active)))
        self.preset.blockSignals(False)
        self.delete.setEnabled(active in custom)
        self.undo.setEnabled(bool(r.get("canUndo")))
        self.redo.setEnabled(bool(r.get("canRedo")))

    def _got_list(self, r: Any) -> None:
        self.rows = [row for row in r if isinstance(row, dict) and isinstance(row.get("id"), str)] if isinstance(r, list) else []
        self.labels = {row["id"]: str(row.get("label") or row["id"]) for row in self.rows}
        self._fill_list()
        self.ready = True

    def _got_keyboard(self, r: Any) -> None:
        keys = r.get("keys") if isinstance(r, dict) and isinstance(r.get("keys"), dict) else {}
        self.keyboard.show_map(keys, self.label, self.mods | self.held, self.key)

    def _got_conflicts(self, r: Any) -> None:
        self.conflicts = [c for c in (r.get("conflicts") if isinstance(r, dict) else None) or [] if isinstance(c, dict)]
        for i, lab in enumerate(self.conflict_labels):
            if i < len(self.conflicts):
                c = self.conflicts[i]
                names = [self.label(x) for x in _strings(c.get("commands"))]
                lab.setText(fmt("Conflict: {key} is assigned to {commands} ({ctx})", key=str(c.get("keys") or ""), commands=tr(" and ").join(names), ctx=tr(str(c.get("context") or ""))))
                lab.show()
            else:
                lab.hide()

    def _got_key_list(self, r: Any) -> None:
        self.key_list.clear()
        rows = [b for b in r if isinstance(b, dict)] if isinstance(r, list) else []
        for b in rows:
            keys = str(b.get("keys") or "")
            mods = keys[: -len(self.key)].rstrip("+") if self.key and keys.endswith(self.key) else ""
            shown_mods = _display(mods) if mods else ""
            cmd = str(b.get("command") or "")
            pn = b.get("panel")
            label = self.label(cmd) + (f" ({tr(pn)})" if isinstance(pn, str) else "")
            item = QTreeWidgetItem([shown_mods or tr("None"), label])
            item.setData(0, Qt.ItemDataRole.UserRole, cmd)
            self.key_list.addTopLevelItem(item)
        self.key_label.setText(key_legend(self.key) if self.key else "")
        if self.key is None:
            self.key_hint.setText(tr("Click a key on the keyboard to see its shortcuts"))
            self.key_hint.show()
        elif not rows:
            self.key_hint.setText(tr("Nothing is assigned to this key"))
            self.key_hint.show()
        else:
            self.key_hint.hide()
        self._update_assign()

    # ----- the command list -----
    def _matches(self, row: dict[str, Any], query: str) -> bool:
        if not query:
            return True
        texts = [row.get("id"), row.get("label"), tr(str(row.get("label") or "")), row.get("category"), tr(str(row.get("category") or ""))]
        for b in row.get("shortcuts") or []:
            if isinstance(b, dict):
                texts += [b.get("keys"), b.get("display"), b.get("panel")]
        return any(isinstance(t, str) and query in t.lower() for t in texts)

    def _cell_text(self, row: dict[str, Any]) -> str:
        if self.recording is not None and self.recording[0] == row["id"]:
            return tr("Type a shortcut… (Esc cancels)")
        parts = []
        for b in row.get("shortcuts") or []:
            if isinstance(b, dict):
                d = str(b.get("display") or b.get("keys") or "")
                parts.append(f"{d} ({tr(b['panel'])})" if isinstance(b.get("panel"), str) else d)
        return ",  ".join(parts)

    def _fill_list(self) -> None:
        query = self.search.text().strip().lower()
        self.list.setUpdatesEnabled(False)
        self.list.clear()
        cat_item: QTreeWidgetItem | None = None
        last = None
        accent = self.palette().highlight().color()
        for row in self.rows:
            if not self._matches(row, query):
                continue
            cat = str(row.get("category") or "")
            if cat != last or cat_item is None:
                cat_item = QTreeWidgetItem([tr(cat)])
                f = cat_item.font(0)
                f.setBold(True)
                cat_item.setFont(0, f)
                cat_item.setFirstColumnSpanned(False)
                self.list.addTopLevelItem(cat_item)
                cat_item.setExpanded(True)
                last = cat
            item = QTreeWidgetItem([tr(str(row.get("label") or row["id"])), self._cell_text(row), "+"])
            item.setData(0, Qt.ItemDataRole.UserRole, row["id"])
            item.setToolTip(0, row["id"])
            item.setToolTip(1, tr("Click and press keys to assign"))
            item.setToolTip(2, tr("Add another shortcut"))
            item.setTextAlignment(2, Qt.AlignmentFlag.AlignCenter)
            if self.recording is not None and self.recording[0] == row["id"]:
                item.setForeground(1, accent)
            cat_item.addChild(item)
            if row["id"] == self.selected:
                item.setSelected(True)
        if self.list.topLevelItemCount() == 0:
            self.list.addTopLevelItem(QTreeWidgetItem([tr("No matching commands")]))
        self.list.setUpdatesEnabled(True)
        self.selected_label.setText(fmt("Selected: {command}", command=self.label(self.selected)) if self.selected else "")

    def row(self, cid: str) -> QTreeWidgetItem | None:
        """The list row of command ``cid`` (egui's ``shortcuts.row.<id>``) while it is shown."""
        for i in range(self.list.topLevelItemCount()):
            cat = self.list.topLevelItem(i)
            for j in range(cat.childCount()):
                item = cat.child(j)
                if item.data(0, Qt.ItemDataRole.UserRole) == cid:
                    return item
        return None

    def _row_clicked(self, item: QTreeWidgetItem, column: int) -> None:
        cid = item.data(0, Qt.ItemDataRole.UserRole)
        if not isinstance(cid, str):
            return
        self.select(cid)
        if column == 1:
            self.record(cid, add=False)
        elif column == 2:
            self.record(cid, add=True)

    def select(self, cid: str) -> None:
        self.selected = cid
        self.selected_label.setText(fmt("Selected: {command}", command=self.label(cid)))
        self.clear.setEnabled(True)
        self._update_assign()

    def record(self, cid: str, add: bool) -> None:
        """Wait for a key press for ``cid``'s Shortcut cell ("+": add instead of replace)."""
        self.selected = cid
        self.recording = (cid, add)
        self._fill_list()
        self.recorder.setFocus(Qt.FocusReason.OtherFocusReason)

    def _recorded(self, key: Any, mods: Any) -> None:
        rec = self.recording
        if rec is None:
            return
        self.recording = None
        if key == Qt.Key.Key_Escape and not (mods & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.AltModifier | Qt.KeyboardModifier.ShiftModifier)):
            self.message = tr("Cancelled")
            self._show_message()
            self._fill_list()
            return
        chord = chord_of(key, mods)
        if chord is None:
            self.message = fmt("{key} can't be used as a shortcut", key=_key_name(key))
            self._show_message()
            self._fill_list()
            return
        self.assign_keys(rec[0], chord, rec[1])

    def assign_keys(self, cid: str, keys: str, add: bool = False) -> None:
        """``shortcuts.set`` in the chosen context; the message names any command it moved from."""

        def done(r: Any) -> None:
            moved = [self.label(b["command"]) for b in (r.get("reassigned") if isinstance(r, dict) else None) or [] if isinstance(b, dict) and isinstance(b.get("command"), str)]
            msg = f"{_display(keys)} → {self.label(cid)}"
            if moved:
                msg += fmt(" (removed from {list})", list=", ".join(moved))
            self.message = msg
            self._show_message()

        self._run("shortcuts.set", {"command": cid, "keys": keys, "panel": self.panel(), "add": add}, done)

    # ----- keyboard -----
    def _key_clicked(self, name: str) -> None:
        self.key = name
        self._refresh_keyboard()
        self._refresh_key_list()

    def _mod_clicked(self, name: str) -> None:
        self.mods ^= {name}
        self._refresh_keyboard()
        self._update_assign()

    def _update_assign(self) -> None:
        if self.key and self.selected:
            keys = "+".join([*(m for m in MOD_ORDER if m in self.mods), self.key])
            self.assign.setText(fmt("Assign {key} to {command}", key=_display(keys), command=self.label(self.selected)))
            self.assign.show()
        else:
            self.assign.hide()

    def _assign_key(self) -> None:
        if self.key and self.selected:
            self.assign_keys(self.selected, "+".join([*(m for m in MOD_ORDER if m in self.mods), self.key]))

    def _modifier_name(self, key: Any) -> str | None:
        return {Qt.Key.Key_Control: "Cmd", Qt.Key.Key_Alt: "Alt", Qt.Key.Key_Shift: "Shift"}.get(key)

    def keyPressEvent(self, e: QKeyEvent) -> None:
        # held modifiers filter the keyboard too (outside recording), as in egui
        m = self._modifier_name(e.key())
        if m is not None and self.recording is None:
            self.held.add(m)
            self._refresh_keyboard()
            return
        super().keyPressEvent(e)

    def keyReleaseEvent(self, e: QKeyEvent) -> None:
        m = self._modifier_name(e.key())
        if m is not None and m in self.held:
            self.held.discard(m)
            self._refresh_keyboard()
            return
        super().keyReleaseEvent(e)

    # ----- presets and context -----
    def _preset_chosen(self, index: int) -> None:
        name = self.preset.itemData(index)
        if not isinstance(name, str):
            return
        self._run("shortcuts.loadPreset", {"name": name}, lambda _r: self._say(fmt("Loaded “{name}”", name=name)))

    def _say(self, text: str) -> None:
        self.message = text
        self._show_message()

    def _open_save_as(self) -> None:
        active = str(self.presets.get("active") or "")
        self.save_name.setText(tr("My Shortcuts") if active in _strings(self.presets.get("builtin")) else active)
        self.save_row.show()
        self.save_name.setFocus()
        self.save_name.selectAll()

    def _save_preset(self) -> None:
        name = self.save_name.text().strip()
        if not name:
            return

        def done(_r: Any) -> None:
            self.save_row.hide()
            self._say(fmt("Saved preset “{name}”", name=name))

        self._run("shortcuts.savePreset", {"name": name}, done)

    def _delete_preset(self) -> None:
        active = str(self.presets.get("active") or "")
        self._run("shortcuts.deletePreset", {"name": active}, lambda _r: self._say(fmt("Deleted preset “{active}”", active=active)))

    def _export(self) -> None:
        active = str(self.presets.get("active") or "shortcuts")
        path, _f = QFileDialog.getSaveFileName(self, tr("Keyboard Shortcuts"), f"{active}.json", "JSON (*.json)")
        if not path:
            self._say(tr("Export: no file chosen (agents: shortcuts.export {path})"))
            return
        self._run("shortcuts.export", {"path": path}, lambda _r: self._say(fmt("Exported to {path}", path=path)))

    def _import(self) -> None:
        path, _f = QFileDialog.getOpenFileName(self, tr("Keyboard Shortcuts"), "", "JSON (*.json)")
        if not path:
            self._say(tr("Import: no file chosen (agents: shortcuts.import {path})"))
            return
        self._run("shortcuts.import", {"path": path}, lambda r: self._say(fmt("Imported “{name}”", name=str((r or {}).get("name") or ""))))

    def _context_chosen(self, index: int) -> None:
        name = self.context_box.itemData(index)
        self.context = name if isinstance(name, str) else APPLICATION
        self._refresh_keyboard()

    def _clear(self) -> None:
        cid = self.selected
        if cid is None:
            return
        self._run(
            "shortcuts.clear",
            {"command": cid, "panel": self.panel() or APPLICATION},
            lambda r: self._say(fmt("Cleared {n} shortcut(s) of {command}", n=(r or {}).get("removed", 0), command=self.label(cid))),
        )

    def _show_message(self) -> None:
        self.clear.setEnabled(self.selected is not None)
        self.message_label.setText(self.message or tr("Click a command's Shortcut cell and press keys to assign it; + adds another shortcut."))

    # ----- closing -----
    def done(self, result: int) -> None:
        if self._closing:
            return
        self._closing = True
        ok = getattr(result, "value", result) == QDialog.DialogCode.Accepted.value
        self.recording = None
        cb = self.on_closed or (lambda _ok: None)
        self.engine.execute(
            "shortcuts.endEditing" if ok else "shortcuts.cancelEditing", {}, on_reply=lambda _r, _b: cb(ok), on_error=lambda _m: cb(ok)
        )
        super().done(result)


def _key_name(key: Any) -> str:
    try:
        return QKeySequence(int(getattr(key, "value", key))).toString() or str(key)
    except (TypeError, ValueError):
        return str(key)


def _display(keys: str) -> str:
    """``Cmd+Alt+K`` as shown off macOS: ``Ctrl+Alt+K`` (the Control key of a macOS chord: Win)."""
    names = {"Cmd": "Ctrl", "Ctrl": "Win"}
    if keys.endswith("++"):
        return "+".join(names.get(p, p) for p in keys[:-2].split("+")) + "++"
    return "+".join(names.get(p, p) for p in keys.split("+"))
