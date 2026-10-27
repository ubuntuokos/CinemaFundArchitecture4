"""The Text panel, as egui's (crates/ui-egui/src/panels/text.rs): Transcript, Captions and Graphics.

- Transcript: the sequence transcript as speaker paragraphs of clickable words (click, Shift+click
  extends); the selection marks In/Out (``transcript.select``) and can be extracted or lifted;
  search, Remove Filler Words, Create Captions, or Transcribe when there is none.
- Captions: the caption segments of a caption track with editable in/out timecodes and text,
  search, the track picker (with New … track), add / split / merge / delete / export, and the
  track's style (font, size, colour, box, alignment, position).
- Graphics: the text of every text layer in the sequence, searchable; the timecode goes to the
  clip and selects the layer, the text edits in place (``graphics.set``).

The engine reads it out (``transcript.inspect``, ``transcript.search``, ``captions.list``,
``text.inspect``, ``fonts.list``) and every edit is an engine command.
"""

from __future__ import annotations

import html
from typing import Any

from PySide6.QtCore import Qt, QTimer, QUrl, Signal
from PySide6.QtGui import QColor, QFont
from PySide6.QtWidgets import (
    QAbstractItemView,
    QApplication,
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDoubleSpinBox,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QMenu,
    QPushButton,
    QStackedWidget,
    QTabBar,
    QTableWidget,
    QTableWidgetItem,
    QTextBrowser,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, fmt, tr

TABS = (("Transcript", N_("Transcript")), ("Captions", N_("Captions")), ("Graphics", N_("Graphics")))
ID_ROLE = Qt.ItemDataRole.UserRole
FORMAT_LABELS = (N_("Subtitle"), N_("CEA-608"), N_("CEA-708"), N_("Teletext"))


def rgba_color(value: Any) -> QColor:
    """The engine's caption colour (``#rrggbbaa`` or ``#rrggbb``): QColor reads eight digits as
    ``#aarrggbb``, so the components are taken apart here."""
    text = str(value or "").strip().lstrip("#")
    try:
        if len(text) in (6, 8):
            return QColor(int(text[0:2], 16), int(text[2:4], 16), int(text[4:6], 16))
    except ValueError:
        pass
    return QColor("#ffffff")


def word_at(words: list[dict[str, Any]], t: int) -> int | None:
    """The word playing at ``t`` (engine ``transcript::word_at``): the last starting at or
    before it, if it has not ended."""
    best = None
    for w in words:
        start, end = w.get("start"), w.get("end")
        if not isinstance(start, int) or not isinstance(end, int) or start > t:
            continue
        if best is None or start >= best[0]:
            best = (start, end, w.get("i"))
    if best is None or t >= best[1]:
        return None
    return best[2] if isinstance(best[2], int) else None


def _tool(parent: QWidget, name: str, text: str, tip: str) -> QToolButton:
    b = QToolButton(parent)
    b.setObjectName(name)
    b.setText(text)
    b.setToolTip(tr(tip))
    b.setAutoRaise(True)
    return b


def _empty_state(parent: QWidget, title: str, note: str, buttons: list[tuple[str, str]]) -> tuple[QWidget, QLabel, list[QPushButton]]:
    """egui's empty states: a title, a note and rounded buttons (the first the accent one)."""
    w = QWidget(parent)
    v = QVBoxLayout(w)
    v.addStretch(1)
    head = QLabel(tr(title), w)
    font = head.font()
    font.setPixelSize(16)
    font.setWeight(QFont.Weight.DemiBold)
    head.setFont(font)
    head.setAlignment(Qt.AlignmentFlag.AlignCenter)
    v.addWidget(head)
    text = QLabel(tr(note), w)
    text.setAlignment(Qt.AlignmentFlag.AlignCenter)
    text.setWordWrap(True)
    text.setEnabled(False)
    v.addWidget(text)
    made = []
    for i, (name, label) in enumerate(buttons):
        b = QPushButton(tr(label), w)
        b.setObjectName(name)
        b.setFixedWidth(200)
        if i == 0:
            t = theme.current()
            b.setStyleSheet(f"QPushButton {{ background: {t.accent}; color: white; border-radius: 13px; padding: 5px; }}")
        v.addWidget(b, 0, Qt.AlignmentFlag.AlignHCenter)
        made.append(b)
    v.addStretch(2)
    return w, text, made


class TranscriptTab(QWidget):
    def __init__(self, panel: TextPanel) -> None:
        super().__init__(panel)
        self.panel = panel
        self.data: dict[str, Any] = {}
        self.hits: set[int] = set()
        self.sel: tuple[int, int] | None = None
        self.current: int | None = None
        self.empty, self.empty_note, (self.generate,) = _empty_state(
            self,
            N_("Transcribe sequence"),
            N_("Speech-to-text turns the dialogue into editable text."),
            [("text.transcript.generate", N_("Transcribe"))],
        )
        self.generate.clicked.connect(lambda: panel.send("transcript.generate", {}))
        self.body = QWidget(self)
        bar = QHBoxLayout()
        bar.setContentsMargins(10, 2, 10, 2)
        self.search = QLineEdit(self.body)
        self.search.setObjectName("text.transcript.search")
        self.search.setPlaceholderText(tr("Search"))
        self.search.setMaximumWidth(170)
        self.search.textChanged.connect(self._search)
        bar.addWidget(self.search)
        self.extract = _tool(self.body, "text.transcript.extract", "✂", N_("Extract selected text"))
        self.lift = _tool(self.body, "text.transcript.lift", "🗑", N_("Lift selected text"))
        fillers = _tool(self.body, "text.transcript.removeFillers", "⌫", N_("Remove filler words"))
        captions = _tool(self.body, "text.transcript.createCaptions", "CC", N_("Create captions"))
        self.extract.clicked.connect(lambda: self._range_cmd("transcript.extract"))
        self.lift.clicked.connect(lambda: self._range_cmd("transcript.lift"))
        fillers.clicked.connect(lambda: panel.send("transcript.removeFillers", {}))
        captions.clicked.connect(lambda: panel.send("transcript.createCaptions", {}))
        for b in (self.extract, self.lift, fillers, captions):
            bar.addWidget(b)
        bar.addStretch(1)
        self.view = QTextBrowser(self.body)
        self.view.setObjectName("text.transcript.words")
        self.view.setOpenLinks(False)
        self.view.anchorClicked.connect(self._clicked)
        v = QVBoxLayout(self.body)
        v.setContentsMargins(0, 0, 0, 0)
        v.addLayout(bar)
        v.addWidget(self.view, 1)
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.empty)
        self.stack.addWidget(self.body)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.addWidget(self.stack)

    @property
    def words(self) -> list[dict[str, Any]]:
        return [w for w in self.data.get("words") or [] if isinstance(w, dict)]

    def set_data(self, data: Any, speech: bool) -> None:
        old = self.data
        self.data = data if isinstance(data, dict) else {}
        n = len(self.words)
        self.empty_note.setText(
            tr("Speech-to-text turns the dialogue into editable text.")
            if speech
            else tr("This build has no speech-to-text; import a transcript with transcript.set.")
        )
        self.stack.setCurrentWidget(self.body if n else self.empty)
        if self.sel and not (self.sel[0] < n and self.sel[1] < n):
            self.sel = None
        cur = self.data.get("current")
        self.current = cur if isinstance(cur, int) else None
        if (old.get("words"), old.get("current")) != (self.data.get("words"), self.data.get("current")):
            self.render()
        self._enable()

    def set_current(self, index: Any) -> None:
        cur = index if isinstance(index, int) else None
        if cur != self.current:
            self.current = cur
            self.render()

    def _enable(self) -> None:
        for b in (self.extract, self.lift):
            b.setEnabled(self.sel is not None)

    def render(self) -> None:
        t = theme.current()
        words = self.words
        lo, hi = (min(self.sel), max(self.sel)) if self.sel else (-1, -2)
        out = []
        for para in self.data.get("paragraphs") or []:
            a, b = int(para.get("from") or 0), int(para.get("to") or 0)
            head = f"{html.escape(str(para.get('speaker') or tr('Speaker')))}&nbsp;&nbsp;{html.escape(self.panel.timecode(para.get('start')))}"
            spans = []
            for i in range(a, min(b, len(words) - 1) + 1):
                w = words[i]
                style = f"color:{t.hot_text if i == self.current else t.text}; text-decoration:none;"
                if lo <= i <= hi:
                    style += f" background-color:{t.row_selected};"
                elif i in self.hits:
                    style += f" background-color:{t.hover};"
                spans.append(f'<a href="w:{i}" style="{style}">{html.escape(str(w.get("text") or ""))}</a>')
            out.append(
                f'<p style="margin:0 0 2px 0; color:{t.text_dim}; font-size:11px; font-weight:600;">{head}</p>'
                f'<p style="margin:0 0 10px 0; font-size:13px;">{" ".join(spans)}</p>'
            )
        bar = self.view.verticalScrollBar().value()
        self.view.setHtml("".join(out))
        self.view.verticalScrollBar().setValue(bar)

    def _clicked(self, url: QUrl) -> None:
        text = url.toString()
        if not text.startswith("w:"):
            return
        try:
            i = int(text[2:])
        except ValueError:
            return
        shift = bool(QApplication.keyboardModifiers() & Qt.KeyboardModifier.ShiftModifier)
        self.select(i, extend=shift)

    def select(self, i: int, extend: bool = False) -> None:
        self.sel = (self.sel[0], i) if extend and self.sel else (i, i)
        a, b = min(self.sel), max(self.sel)
        self.panel.send("transcript.select", {"from": a, "to": b})
        self._enable()
        self.render()

    def _range_cmd(self, cmd: str) -> None:
        if self.sel is None:
            return
        a, b = min(self.sel), max(self.sel)
        self.sel = None
        self.panel.send(cmd, {"from": a, "to": b})
        self._enable()

    def _search(self, text: str) -> None:
        if not text.strip():
            self.hits = set()
            self.render()
            return
        engine = self.panel.engine

        def got(result: Any, _b: bytes | None, q: str = text) -> None:
            if q != self.search.text():
                return
            hits: set[int] = set()
            for m in (result or {}).get("matches") or []:
                hits.update(range(int(m.get("from") or 0), int(m.get("to") or 0) + 1))
            self.hits = hits
            self.render()

        if engine is not None and getattr(engine, "running", False):
            engine.execute("transcript.search", {"query": text}, on_reply=got)


class CaptionsTab(QWidget):
    COLUMNS = (N_("#"), N_("In"), N_("Out"), N_("Speaker"), N_("Text"))

    def __init__(self, panel: TextPanel) -> None:
        super().__init__(panel)
        self.panel = panel
        self.data: dict[str, Any] = {}
        self.track_index: int | None = None  # the track picked; None follows the selection
        self.fonts: list[str] = []
        self._filling = False
        self.empty, _note, (new_track, imp) = _empty_state(
            self,
            N_("Add captions"),
            N_("Create a caption track or import a caption file."),
            [("text.captions.newTrack", N_("Create new caption track")), ("text.captions.import", N_("Import captions file…"))],
        )
        new_track.clicked.connect(lambda: panel.send("captions.newTrack", {}))
        imp.clicked.connect(lambda: panel.run.emit("captions.import", {}))
        self.body = QWidget(self)
        bar = QHBoxLayout()
        bar.setContentsMargins(10, 2, 10, 2)
        self.search = QLineEdit(self.body)
        self.search.setObjectName("text.captions.search")
        self.search.setPlaceholderText(tr("Search"))
        self.search.setMaximumWidth(170)
        self.search.textChanged.connect(lambda _t: self._fill())
        bar.addWidget(self.search)
        self.picker = QToolButton(self.body)
        self.picker.setObjectName("text.captions.track")
        self.picker.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.picker.setStyleSheet("QToolButton { padding-right: 14px; }")
        self.picker.setMenu(QMenu(self.picker))
        self.picker.menu().aboutToShow.connect(self._picker_menu)
        bar.addWidget(self.picker)
        self.add = _tool(self.body, "text.captions.add", "+", N_("Add caption at playhead"))
        self.split = _tool(self.body, "text.captions.split", "✂", N_("Split caption at playhead"))
        self.merge = _tool(self.body, "text.captions.merge", "⛓", N_("Merge selected captions"))
        self.delete = _tool(self.body, "text.captions.delete", "🗑", N_("Delete selected captions"))
        self.export = _tool(self.body, "text.captions.export", "⇪", N_("Export captions…"))
        # at the shown playhead: while playing the engine's stays where playback started
        self.add.clicked.connect(lambda: panel.send("captions.add", {"track": self.track().get("id"), "time": panel.playhead()}))
        self.split.clicked.connect(self._split)
        self.merge.clicked.connect(lambda: panel.send("captions.merge", {}))
        self.delete.clicked.connect(lambda: panel.send("captions.delete", {}))
        self.export.clicked.connect(lambda: panel.run.emit("captions.export", {"track": self.track().get("id")}))
        for b in (self.add, self.split, self.merge, self.delete, self.export):
            bar.addWidget(b)
        bar.addStretch(1)
        self.table = QTableWidget(0, len(self.COLUMNS), self.body)
        self.table.setObjectName("text.captions.list")
        self.table.setHorizontalHeaderLabels([tr(c) for c in self.COLUMNS])
        self.table.verticalHeader().setVisible(False)
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setAlternatingRowColors(True)
        self.table.setWordWrap(True)
        head = self.table.horizontalHeader()
        for i, w in enumerate((28, 84, 84, 64)):
            self.table.setColumnWidth(i, w)
        head.setSectionResizeMode(4, QHeaderView.ResizeMode.Stretch)
        self.table.itemChanged.connect(self._edited)
        self.table.itemSelectionChanged.connect(self._selected)
        self.table.cellClicked.connect(self._cell_clicked)
        style = QHBoxLayout()
        style.setContentsMargins(10, 2, 10, 4)
        style.setSpacing(6)
        self.font_box = QComboBox(self.body)
        self.font_box.setObjectName("text.captions.style.font")
        self.font_box.setMinimumContentsLength(4)
        self.font_box.setSizeAdjustPolicy(QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)
        self.font_box.activated.connect(lambda _i: self._style({"font": self.font_box.currentText()}))
        self.size = QDoubleSpinBox(self.body)
        self.size.setObjectName("text.captions.style.size")
        self.size.setRange(8, 200)
        self.size.setSingleStep(0.5)
        self.size.setSuffix(" px")
        self.size.setKeyboardTracking(False)
        self.size.valueChanged.connect(lambda v: self._filling or self._style({"size": float(v)}))
        self.color = QPushButton(self.body)
        self.color.setObjectName("text.captions.style.color")
        self.color.setFixedWidth(40)
        self.color.setToolTip(tr("Text colour"))
        self.color.clicked.connect(self._pick_color)
        self.box = QCheckBox(tr("Box"), self.body)
        self.box.setObjectName("text.captions.style.background")
        self.box.toggled.connect(lambda on: self._filling or self._style({"background": on}))
        style.addWidget(self.font_box)
        style.addWidget(self.size)
        style.addWidget(self.color)
        style.addWidget(self.box)
        self.align: dict[str, QToolButton] = {}
        self.anchor: dict[str, QToolButton] = {}
        style.addStretch(1)
        # alignment and position on a second row, so the panel stays narrow (egui's strip wraps)
        place = QHBoxLayout()
        place.setContentsMargins(10, 0, 10, 4)
        place.setSpacing(6)
        label = QLabel(tr("Align"), self.body)
        label.setEnabled(False)
        place.addWidget(label)
        for name, text, tip in (("left", "L", N_("Align left")), ("center", "C", N_("Align center")), ("right", "R", N_("Align right"))):
            b = _tool(self.body, f"text.captions.style.align.{name}", text, tip)
            b.setCheckable(True)
            b.clicked.connect(lambda _c=False, n=name: self._style({"align": n}))
            place.addWidget(b)
            self.align[name] = b
        label = QLabel(tr("Position"), self.body)
        label.setEnabled(False)
        place.addWidget(label)
        for name, tip in (("top", N_("Position: top")), ("middle", N_("Position: middle")), ("bottom", N_("Position: bottom"))):
            b = _tool(self.body, f"text.captions.style.anchor.{name}", name[:1].upper(), tip)
            b.setCheckable(True)
            b.clicked.connect(lambda _c=False, n=name: self._style({"anchor": n}))
            place.addWidget(b)
            self.anchor[name] = b
        place.addStretch(1)
        self.note = QLabel(self.body)
        self.note.setObjectName("text.captions.note")
        self.note.setEnabled(False)
        v = QVBoxLayout(self.body)
        v.setContentsMargins(0, 0, 0, 0)
        v.addLayout(bar)
        v.addWidget(self.table, 1)
        v.addWidget(self.note)
        v.addLayout(style)
        v.addLayout(place)
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.empty)
        self.stack.addWidget(self.body)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.addWidget(self.stack)

    def tracks(self) -> list[dict[str, Any]]:
        return [t for t in self.data.get("tracks") or [] if isinstance(t, dict)]

    def track(self) -> dict[str, Any]:
        tracks = self.tracks()
        if not tracks:
            return {}
        if self.track_index is not None and self.track_index < len(tracks):
            return tracks[self.track_index]
        sel = set(self.data.get("selection") or [])
        for t in tracks:
            if any(c.get("id") in sel for c in t.get("captions") or []):
                return t
        return tracks[0]

    def set_data(self, data: Any) -> None:
        self.data = data if isinstance(data, dict) else {}
        self.stack.setCurrentWidget(self.body if self.tracks() else self.empty)
        self._fill()

    def set_fonts(self, fonts: list[str]) -> None:
        self.fonts = fonts
        self._fill_style()

    def _caption_at(self, ph: int) -> dict[str, Any] | None:
        for c in self.track().get("captions") or []:
            if int(c.get("start") or 0) <= ph < int(c.get("end") or 0):
                return c
        return None

    def _fill(self) -> None:
        track = self.track()
        self._filling = True
        try:
            self.picker.setText(f"{track.get('label', 'C1')} · {track.get('name', '')}" if track else "")
            ph = self.panel.playhead()
            sel = set(self.data.get("selection") or [])
            under = self._caption_at(ph)
            self.add.setEnabled(bool(track) and under is None)
            self.split.setEnabled(under is not None and int(under.get("start") or 0) < ph)
            self.merge.setEnabled(len(sel) > 1)
            self.delete.setEnabled(bool(sel))
            self.export.setEnabled(bool(track.get("captions")))
            q = self.search.text().lower()
            rows = [
                (n, c)
                for n, c in enumerate(track.get("captions") or [])
                if not q or q in str(c.get("text") or "").lower() or q in str(c.get("speaker") or "").lower()
            ]
            editing = self.table.state() == QAbstractItemView.State.EditingState
            if not editing:
                self.table.blockSignals(True)
                self.table.setRowCount(len(rows))
                bold = QFont(self.table.font())
                bold.setBold(True)
                for r, (n, c) in enumerate(rows):
                    cells = (str(n + 1), str(c.get("in") or ""), str(c.get("out") or ""), str(c.get("speaker") or ""), str(c.get("text") or ""))
                    for col, text in enumerate(cells):
                        item = QTableWidgetItem(text)
                        item.setData(ID_ROLE, c.get("id"))
                        flags = Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
                        if col in (1, 2, 4):
                            flags |= Qt.ItemFlag.ItemIsEditable
                        item.setFlags(flags)
                        if col == 1:
                            item.setForeground(theme.current().c("hot_text"))
                        if under is not None and c.get("id") == under.get("id"):
                            item.setFont(bold)
                        self.table.setItem(r, col, item)
                    self.table.item(r, 0).setToolTip(tr("Go to caption"))
                self.table.resizeRowsToContents()
                self.table.clearSelection()
                for r, (_n, c) in enumerate(rows):
                    if c.get("id") in sel:
                        self.table.selectRow(r) if len(sel) == 1 else self._add_row(r)
                self.table.blockSignals(False)
            if not rows:
                self.note.setText(
                    tr("No captions on this track. Press + to add one at the playhead.") if not q else tr("No matching captions.")
                )
            self.note.setVisible(not rows)
            self._fill_style()
        finally:
            self._filling = False

    def _add_row(self, r: int) -> None:
        for col in range(self.table.columnCount()):
            item = self.table.item(r, col)
            if item is not None:
                item.setSelected(True)

    def _fill_style(self) -> None:
        st = self.track().get("style") or {}
        was = self._filling
        self._filling = True
        try:
            font = str(st.get("font") or "")
            names = self.fonts if font in self.fonts or not font else [font, *self.fonts]
            if [self.font_box.itemText(i) for i in range(self.font_box.count())] != names:
                self.font_box.clear()
                self.font_box.addItems(names)
            self.font_box.setCurrentText(font)
            if not self.size.hasFocus():
                self.size.setValue(float(st.get("size") or 40))
            color = rgba_color(st.get("color"))
            self.color.setStyleSheet(f"background: {color.name()};")
            self.color.setProperty("cfa4_color", color.name())
            self.box.setChecked(bool(st.get("background")))
            for name, b in self.align.items():
                b.setChecked(st.get("align") == name)
            for name, b in self.anchor.items():
                b.setChecked(st.get("anchor") == name)
        finally:
            self._filling = was

    def _style(self, params: dict[str, Any]) -> None:
        track = self.track()
        if track.get("id") is not None and not self._filling:
            self.panel.send("captions.setStyle", {"track": track["id"], **params})

    def _pick_color(self) -> None:
        current = QColor(str(self.color.property("cfa4_color") or "#ffffff"))
        color = QColorDialog.getColor(current, self, tr("Text colour"))
        if color.isValid():
            self._style({"color": [color.red(), color.green(), color.blue(), 255]})

    def _picker_menu(self) -> None:
        menu = self.picker.menu()
        menu.clear()
        current = self.track().get("id")
        for i, t in enumerate(self.tracks()):
            a = menu.addAction(f"{t.get('label')} · {t.get('name')} ({tr(str(t.get('format') or ''))})")
            a.setCheckable(True)
            a.setChecked(t.get("id") == current)
            a.triggered.connect(lambda _c=False, n=i: self._pick(n))
        menu.addSeparator()
        for f in self.panel.formats or FORMAT_LABELS:
            a = menu.addAction(fmt("New {format} track", format=tr(str(f))))
            a.triggered.connect(lambda _c=False, ff=f: self.panel.send("captions.newTrack", {"format": ff}))

    def _pick(self, n: int) -> None:
        self.track_index = n
        self._fill()

    def _split(self) -> None:
        under = self._caption_at(self.panel.playhead())
        if under is not None:
            self.panel.send("captions.split", {"caption": under.get("id"), "time": self.panel.playhead()})

    def _cell_clicked(self, row: int, col: int) -> None:
        if col != 0:
            return
        item = self.table.item(row, 0)
        cid = item.data(ID_ROLE) if item is not None else None
        if cid is None:
            return
        mods = QApplication.keyboardModifiers()
        if mods & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.ShiftModifier):
            self.panel.send("captions.select", {"captions": [cid], "add": True})
        else:
            self.panel.send("captions.goTo", {"caption": cid})

    def _selected(self) -> None:
        if self._filling:
            return
        ids = sorted({i.data(ID_ROLE) for i in self.table.selectedItems() if i.data(ID_ROLE) is not None})
        if ids != sorted(self.data.get("selection") or []):
            self.panel.send("captions.select", {"captions": ids})

    def _edited(self, item: QTableWidgetItem) -> None:
        if self._filling:
            return
        cid = item.data(ID_ROLE)
        caption = next((c for c in self.track().get("captions") or [] if c.get("id") == cid), None)
        if caption is None:
            return
        text = item.text()
        col = item.column()
        if col == 4 and text != caption.get("text"):
            self.panel.send("captions.setText", {"caption": cid, "text": text})
        elif col in (1, 2) and text.strip() != caption.get("in" if col == 1 else "out"):
            self.panel.send("captions.setTimes", {"caption": cid, "startTimecode" if col == 1 else "endTimecode": text.strip()})


class GraphicsTab(QWidget):
    COLUMNS = (N_("Timecode"), N_("Track"), N_("Text"))

    def __init__(self, panel: TextPanel) -> None:
        super().__init__(panel)
        self.panel = panel
        self.rows: list[dict[str, Any]] = []
        self._filling = False
        self.search = QLineEdit(self)
        self.search.setObjectName("text.graphics.search")
        self.search.setPlaceholderText(tr("Search"))
        self.search.setMaximumWidth(220)
        self.search.textChanged.connect(lambda _t: self._fill())
        self.table = QTableWidget(0, len(self.COLUMNS), self)
        self.table.setObjectName("text.graphics.list")
        self.table.setHorizontalHeaderLabels([tr(c) for c in self.COLUMNS])
        self.table.verticalHeader().setVisible(False)
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setAlternatingRowColors(True)
        self.table.setColumnWidth(0, 96)
        self.table.setColumnWidth(1, 110)
        self.table.horizontalHeader().setSectionResizeMode(2, QHeaderView.ResizeMode.Stretch)
        self.table.cellClicked.connect(self._clicked)
        self.table.itemChanged.connect(self._edited)
        self.table.itemDoubleClicked.connect(lambda item: self._select_row(item.row()))
        self.note = QLabel(self)
        self.note.setObjectName("text.graphics.note")
        self.note.setEnabled(False)
        bar = QHBoxLayout()
        bar.setContentsMargins(10, 2, 10, 2)
        bar.addWidget(self.search)
        bar.addStretch(1)
        v = QVBoxLayout(self)
        v.setContentsMargins(0, 0, 0, 0)
        v.addLayout(bar)
        v.addWidget(self.table, 1)
        v.addWidget(self.note)

    def set_rows(self, rows: list[dict[str, Any]]) -> None:
        if rows != self.rows:
            self.rows = rows
            self._fill()

    def _shown(self) -> list[dict[str, Any]]:
        q = self.search.text().lower()
        return [g for g in self.rows if not q or q in str(g.get("text") or "").lower() or q in str(g.get("name") or "").lower()]

    def _fill(self) -> None:
        if self.table.state() == QAbstractItemView.State.EditingState:
            return
        shown = self._shown()
        self._filling = True
        self.table.setRowCount(len(shown))
        for r, g in enumerate(shown):
            label = f"{g.get('track')} · {g.get('name')}" if g.get("name") else str(g.get("track") or "")
            for col, text in enumerate((str(g.get("timecode") or ""), label, str(g.get("text") or ""))):
                item = QTableWidgetItem(text)
                item.setData(ID_ROLE, (g.get("clip"), g.get("layer")))
                flags = Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
                if col == 2:
                    flags |= Qt.ItemFlag.ItemIsEditable
                item.setFlags(flags)
                if col == 0:
                    item.setForeground(theme.current().c("hot_text"))
                    item.setToolTip(tr("Go to graphic"))
                self.table.setItem(r, col, item)
        self.table.resizeRowsToContents()
        self._filling = False
        if not shown:
            self.note.setText(tr("No text graphics in this sequence. Add text with the Type tool.") if not self.rows else tr("No matching text."))
        self.note.setVisible(not shown)

    def _row(self, r: int) -> dict[str, Any] | None:
        shown = self._shown()
        return shown[r] if 0 <= r < len(shown) else None

    def _select_row(self, r: int) -> None:
        g = self._row(r)
        if g is not None:
            self.panel.send("timeline.select", {"clips": [g.get("clip")]})
            self.panel.send("graphics.selectLayer", {"clip": g.get("clip"), "layers": [g.get("layer")]})

    def _clicked(self, r: int, col: int) -> None:
        g = self._row(r)
        if g is None:
            return
        if col == 0:
            self.panel.seek.emit(int(g.get("start") or 0))
        self._select_row(r)

    def _edited(self, item: QTableWidgetItem) -> None:
        if self._filling or item.column() != 2:
            return
        g = self._row(item.row())
        if g is not None and item.text() != g.get("text"):
            self.panel.send("graphics.set", {"clip": g.get("clip"), "layer": g.get("layer"), "props": {"text": item.text()}, "time": g.get("at")})


class TextPanel(QWidget):
    command = Signal(str, dict)
    run = Signal(str, dict)  # a command that needs a file (import / export captions)
    seek = Signal(object)  # ticks

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Text")
        self.engine = engine
        self.sequence: dict[str, Any] | None = None
        self.formats: list[str] = []
        self.speech = False
        self._playhead: int | None = None
        self._busy = False
        self._again = False
        self._fonts_asked = False
        self.tabs = QTabBar(self)
        self.tabs.setObjectName("text.tabs")
        self.tabs.setDrawBase(False)
        self.tabs.setExpanding(False)
        self.tabs.setStyleSheet("QTabBar { background: transparent; }")  # the app sheet paints dock tab bars
        for _key, label in TABS:
            self.tabs.addTab(tr(label))
        self.transcript = TranscriptTab(self)
        self.captions = CaptionsTab(self)
        self.graphics = GraphicsTab(self)
        self.stack = QStackedWidget(self)
        for w in (self.transcript, self.captions, self.graphics):
            self.stack.addWidget(w)
        self.no_sequence = QLabel(self)
        self.no_sequence.setObjectName("text.placeholder")
        self.no_sequence.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.no_sequence.setEnabled(False)
        self.stack.addWidget(self.no_sequence)
        self.tabs.currentChanged.connect(lambda _i: (self._show_tab(), self.refresh()))
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 4, 6, 0)
        tabs = QHBoxLayout()
        tabs.addWidget(self.tabs)
        tabs.addStretch(1)
        lay.addLayout(tabs)
        lay.addWidget(self.stack, 1)
        self._show_tab()

    def tab(self) -> str:
        return TABS[max(0, self.tabs.currentIndex())][0]

    def set_tab(self, key: str) -> None:
        self.tabs.setCurrentIndex(next((i for i, (k, _l) in enumerate(TABS) if k == key), 0))

    def _show_tab(self) -> None:
        if self.sequence is None:
            self.no_sequence.setText(
                {
                    "Transcript": tr("Open a sequence to see its transcript"),
                    "Captions": tr("Open a sequence to work with captions"),
                    "Graphics": tr("Open a sequence to search its graphics"),
                }[self.tab()]
            )
            self.stack.setCurrentWidget(self.no_sequence)
        else:
            self.stack.setCurrentWidget((self.transcript, self.captions, self.graphics)[max(0, self.tabs.currentIndex())])

    def playhead(self) -> int:
        if self._playhead is not None:
            return self._playhead
        return int((self.sequence or {}).get("playhead") or 0)

    def timecode(self, ticks: Any) -> str:
        from ..timecode import format_timecode, frame_ticks

        try:
            return format_timecode(int(ticks), frame_ticks((self.sequence or {}).get("settings")))
        except (TypeError, ValueError, ZeroDivisionError):
            return ""

    def set_sequence(self, sequence: dict[str, Any] | None, playhead: int | None = None) -> None:
        self.sequence = sequence
        self._playhead = playhead
        self._show_tab()
        self.refresh()

    def set_playhead(self, ticks: int) -> None:
        """The Program playhead moved: the current word and caption follow it."""
        if ticks != self._playhead:
            self._playhead = int(ticks)
            # the word under the shown playhead: the engine's own stays where playback started
            self.transcript.set_current(word_at(self.transcript.words, self._playhead))
            if self.tab() != "Graphics":
                self.refresh()

    def send(self, command: str, params: dict[str, Any]) -> None:
        self.command.emit(command, params)

    def refresh(self) -> None:
        """Ask the engine for what the current tab shows (only while shown)."""
        if self.engine is None or not getattr(self.engine, "running", False) or self.sequence is None:
            return
        if not self.isVisible() or self.visibleRegion().isEmpty():  # a tab behind another keeps "visible"
            return
        if self._busy:
            self._again = True
            return
        self._busy = True
        tab = self.tab()
        pending = [0]

        def finished() -> None:
            pending[0] -= 1
            if pending[0] > 0:
                return
            self._busy = False
            if self._again:
                self._again = False
                QTimer.singleShot(0, self.refresh)

        def call(method: str, params: dict[str, Any], on: Any, command: bool = True) -> None:
            pending[0] += 1

            def reply(r: Any, _b: bytes | None) -> None:
                on(r)
                finished()

            def failed(_m: str) -> None:
                finished()

            if command:
                self.engine.execute(method, params, on_reply=reply, on_error=failed)
            else:
                self.engine.call(method, params, on_reply=reply, on_error=failed)

        call("text.inspect", {"time": self.playhead()}, self._on_text, command=False)
        if tab == "Transcript":
            def transcript(r: Any) -> None:
                self.transcript.set_data(r, self.speech)
                self.transcript.set_current(word_at(self.transcript.words, self.playhead()))

            call("transcript.inspect", {}, transcript)
        elif tab == "Captions":
            call("captions.list", {}, self.captions.set_data)
            if not self._fonts_asked:
                self._fonts_asked = True
                call("fonts.list", {}, self._on_fonts)

    def _on_text(self, r: Any) -> None:
        r = r if isinstance(r, dict) else {}
        self.formats = [str(f) for f in r.get("captionFormats") or []]
        self.speech = bool(r.get("speech"))
        self.graphics.set_rows([g for g in r.get("graphics") or [] if isinstance(g, dict)])

    def _on_fonts(self, r: Any) -> None:
        names = sorted({str(f.get("family")) for f in r or [] if isinstance(f, dict) and f.get("family") and not str(f.get("family")).startswith(".")}, key=str.casefold)
        self.captions.set_fonts(names)

    def showEvent(self, e: Any) -> None:
        super().showEvent(e)
        self.refresh()
