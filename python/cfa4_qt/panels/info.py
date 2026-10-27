"""Markers, Info, Metadata and Project Notes, as egui's panels (crates/ui-egui/src/panels/misc.rs,
metadata.rs, notes.rs). What they show comes from the engine: the sequence's markers
(``sequence.inspect``), the hidden marker colours (``state.inspect``), an item's clip and file
properties (``metadata.get``) and the project's notes (``project.notes``); every change is an
engine command (``markers.filterColors``, ``markers.edit``, ``metadata.set``, ``project.setNotes``).
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QRect, QRectF, QSize, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QFont, QIcon, QMouseEvent, QPainter, QPaintEvent, QPixmap
from PySide6.QtWidgets import (
    QComboBox,
    QFormLayout,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMenu,
    QPlainTextEdit,
    QScrollArea,
    QSizePolicy,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, fmt, tr
from ..timecode import format_timecode, frame_ticks

# cfa4_project::Label::ALL, and each label's marker colour (Label::marker_rgb)
LABELS = (
    "Violet", "Iris", "Caribbean", "Lavender", "Cerulean", "Forest", "Rose", "Mango",
    "Purple", "Blue", "Teal", "Magenta", "Tan", "Green", "Brown", "Yellow",
)  # fmt: skip
MARKER_RGB = {
    **dict.fromkeys(("Rose", "Magenta"), "#c13c3d"),
    **dict.fromkeys(("Purple", "Violet", "Lavender"), "#a98caf"),
    **dict.fromkeys(("Mango", "Brown", "Tan"), "#da7639"),
    "Yellow": "#c9a344",
    **dict.fromkeys(("Blue", "Iris", "Cerulean"), "#568bf4"),
    **dict.fromkeys(("Teal", "Caribbean"), "#72f0d7"),
}
DEFAULT_MARKER_RGB = "#758542"
# the Markers panel's colour filter (egui's MARKER_FILTER), one chip per marker colour
MARKER_FILTER = ("Green", "Rose", "Purple", "Mango", "Yellow", "Blue", "Teal")
# label colours of items (cfa4_project::Label::rgb), for the Metadata label menu swatches
LABEL_RGB = {
    "Violet": "#380ea7", "Iris": "#1d4a64", "Caribbean": "#355418", "Lavender": "#6b1c82",
    "Cerulean": "#23535a", "Forest": "#404a11", "Rose": "#801836", "Mango": "#803f17",
    "Purple": "#590db0", "Blue": "#192d94", "Teal": "#1f4d45", "Magenta": "#791c56",
    "Tan": "#6c5b47", "Green": "#295c2d", "Brown": "#583d14", "Yellow": "#6e6628",
}  # fmt: skip


def marker_rgb(label: Any) -> QColor:
    return QColor(MARKER_RGB.get(str(label), DEFAULT_MARKER_RGB))


def swatch(color: str | QColor, size: int = 12) -> QIcon:
    pm = QPixmap(size, size)
    pm.fill(QColor(color))
    return QIcon(pm)


# ----- Markers -----
class MarkerList(QWidget):
    """The sequence's markers, one 38 px row each: colour bar, name, start (and duration)."""

    go = Signal(object)  # marker start, ticks
    recolor = Signal(object, str)  # marker id, label

    ROW = 38

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("markers.list")
        self.setMouseTracking(True)
        self.markers: list[dict[str, Any]] = []
        self.frame = 1
        self.empty = ""
        self._hover = -1

    def set_markers(self, markers: list[dict[str, Any]], frame: int, empty: str) -> None:
        self.markers, self.frame, self.empty = markers, max(1, frame), empty
        self.setMinimumHeight(max(40, len(markers) * self.ROW))
        self.updateGeometry()
        self.update()

    def sizeHint(self) -> QSize:
        return QSize(200, max(40, len(self.markers) * self.ROW))

    def row_at(self, y: float) -> int:
        i = int(y // self.ROW)
        return i if 0 <= i < len(self.markers) else -1

    def row_rect(self, i: int) -> QRect:
        return QRect(0, i * self.ROW, self.width(), self.ROW)

    def label_text(self, m: dict[str, Any]) -> str:
        tc = format_timecode(int(m.get("start") or 0), self.frame)
        if int(m.get("duration") or 0) > 0:
            tc += f"  ({format_timecode(int(m['duration']), self.frame)})"
        if m.get("kind") == "Chapter":
            tc = fmt("{tc}  Chapter", tc=tc)
        return tc

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        p.fillRect(self.rect(), T.c("panel_bg"))
        if not self.markers:
            p.setPen(T.c("text_faint"))
            p.drawText(self.rect().adjusted(6, 4, -6, -4), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop | Qt.TextFlag.TextWordWrap, self.empty)
            return
        name_font = QFont(self.font())
        name_font.setPixelSize(12)
        mono = QFont("monospace")
        mono.setStyleHint(QFont.StyleHint.Monospace)
        mono.setPixelSize(11)
        for i, m in enumerate(self.markers):
            r = self.row_rect(i)
            if i == self._hover:
                p.setRenderHint(QPainter.RenderHint.Antialiasing)
                p.setPen(Qt.PenStyle.NoPen)
                p.setBrush(T.c("hover"))
                p.drawRoundedRect(QRectF(r), 3, 3)
            p.fillRect(QRect(r.left() + 4, r.top() + 6, 4, 26), marker_rgb(m.get("color")))
            p.setFont(name_font)
            p.setPen(T.c("text"))
            p.drawText(QRect(r.left() + 16, r.top() + 4, r.width() - 20, 16), Qt.AlignmentFlag.AlignVCenter, str(m.get("name") or tr("Marker")))
            p.setFont(mono)
            p.setPen(T.c("hot_text"))
            p.drawText(QRect(r.left() + 16, r.top() + 20, r.width() - 20, 14), Qt.AlignmentFlag.AlignVCenter, self.label_text(m))

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        i = self.row_at(e.position().y())
        if i != self._hover:
            self._hover = i
            self.update()

    def leaveEvent(self, _e) -> None:
        self._hover = -1
        self.update()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        i = self.row_at(e.position().y())
        if i < 0:
            return
        m = self.markers[i]
        if e.button() == Qt.MouseButton.LeftButton:
            self.go.emit(int(m.get("start") or 0))
        elif e.button() == Qt.MouseButton.RightButton:
            self.menu_for(i).exec(e.globalPosition().toPoint())

    def menu_for(self, i: int) -> QMenu:
        """egui's marker menu: Label ▸ the 16 colours."""
        m = self.markers[i]
        marker_id = m.get("id")
        menu = QMenu(self)
        sub = menu.addMenu(tr("Label"))
        sub.setObjectName("markers.menu.label")
        for name in LABELS:
            a = sub.addAction(swatch(marker_rgb(name)), tr(name))
            a.setObjectName(f"markers.menu.color.{name}")
            a.setCheckable(True)
            a.setChecked(m.get("color") == name)
            a.triggered.connect(lambda _c=False, mid=marker_id, n=name: self.recolor.emit(mid, n))
        return menu


class MarkersPanel(QWidget):
    """A colour filter row (a chip per marker colour: click to show or hide those markers), then
    the active sequence's markers; click one to go to it, right-click to change its colour."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Markers")
        self.hidden: set[str] = set()
        self.sequence: dict[str, Any] | None = None
        chips = QHBoxLayout()
        chips.setContentsMargins(6, 6, 6, 2)
        chips.setSpacing(2)
        self.chips: dict[str, QToolButton] = {}
        for name in MARKER_FILTER:
            b = QToolButton(self)
            b.setObjectName(f"markers.filter.{name}")
            b.setFixedSize(18, 18)
            b.setCheckable(True)
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            b.setToolTip(fmt("Show or hide {color} markers", color=tr(name)))
            b.clicked.connect(lambda on, n=name: self.command.emit("markers.filterColors", {"color": n, "visible": bool(on)}))
            chips.addWidget(b)
            self.chips[name] = b
        chips.addStretch(1)
        self.list = MarkerList(self)
        self.list.go.connect(lambda t: self.command.emit("playhead.set", {"time": t}))
        self.list.recolor.connect(lambda mid, n: self.command.emit("markers.edit", {"target": "program", "marker": mid, "color": n}))
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.list)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(2)
        layout.addLayout(chips)
        layout.addWidget(scroll, 1)
        self._paint_chips()

    def _paint_chips(self) -> None:
        for name, b in self.chips.items():
            c = marker_rgb(name).name()
            on = name not in self.hidden
            b.setChecked(on)
            b.setStyleSheet(
                f"QToolButton {{ background: {c if on else 'transparent'}; border: 1px solid {c}; border-radius: 3px; margin: 2px; }}"
            )

    def set_state(self, sequence: dict[str, Any] | None, hidden: list[Any] | None) -> None:
        self.sequence = sequence
        self.hidden = {str(h) for h in hidden or []}
        # a hidden colour hides every label drawn in it, as egui compares the marker colours
        hidden_rgb = {MARKER_RGB.get(h, DEFAULT_MARKER_RGB) for h in self.hidden}
        self._paint_chips()
        if sequence is None:
            self.list.set_markers([], 1, tr("(no sequence)"))
            return
        markers = [m for m in sequence.get("markers") or [] if isinstance(m, dict)]
        shown = [m for m in markers if MARKER_RGB.get(str(m.get("color")), DEFAULT_MARKER_RGB) not in hidden_rgb]
        empty = tr("No markers. Press M to add one.") if not markers else tr("All markers are hidden by the colour filter.")
        self.list.set_markers(shown, frame_ticks(sequence.get("settings")), empty)


# ----- Metadata -----
class MetadataPanel(QWidget):
    """The clip and file properties of the selected item (Project panel, or the clip selected in
    the Timeline). Name, Label and the log fields are editable; each committed edit is one
    ``metadata.set`` (undoable, saved in the project)."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Metadata")
        self.item: int | None = None
        self.fields: list[dict[str, Any]] = []
        self.edits: dict[str, QLineEdit] = {}
        self.label_box: QComboBox | None = None
        self.header = QLabel(self)
        self.header.setObjectName("metadata.header")
        f = self.header.font()
        f.setPixelSize(14)
        f.setBold(True)
        self.header.setFont(f)
        self.empty = QLabel(tr("Select a clip to view its metadata."), self)
        self.empty.setObjectName("metadata.empty")
        self.empty.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.body = QWidget(self)
        self.form = QFormLayout(self.body)
        self.form.setContentsMargins(0, 0, 0, 0)
        self.form.setLabelAlignment(Qt.AlignmentFlag.AlignLeft)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.body)
        self.scroll = scroll
        layout = QVBoxLayout(self)
        layout.setContentsMargins(10, 8, 10, 8)
        layout.addWidget(self.header)
        layout.addWidget(self.empty, 1)
        layout.addWidget(scroll, 1)
        self.set_fields(None, None)

    def editing(self) -> bool:
        w = self.focusWidget()
        return w is not None and (w in self.edits.values()) and w.hasFocus()

    def set_fields(self, item: int | None, result: dict[str, Any] | None) -> None:
        fields = [f for f in (result or {}).get("fields") or [] if isinstance(f, dict)]
        if item == self.item and fields == self.fields and item is not None:
            return
        if item == self.item and self.editing():
            return  # don't pull the field from under the typing
        self.item, self.fields = item, fields
        has = item is not None and bool(fields)
        self.empty.setVisible(not has)
        self.scroll.setVisible(has)
        self.header.setVisible(has)
        while self.form.rowCount():
            self.form.removeRow(0)
        self.edits.clear()
        self.label_box = None
        if not has:
            return
        self.header.setText(fmt("Clip: {name}", name=str((result or {}).get("name") or "")))
        section = ""
        for fd in fields:
            name, value = str(fd.get("name") or ""), str(fd.get("value") or "")
            if fd.get("section") != section:
                section = str(fd.get("section") or "")
                head = QLabel(tr(section), self.body)
                head.setObjectName(f"metadata.section.{section}")
                hf = head.font()
                hf.setBold(True)
                head.setFont(hf)
                head.setStyleSheet(f"color: {theme.current().text_dim}; margin-top: 4px;")
                self.form.addRow(head)
            label = QLabel(f"{tr(name)}:", self.body)
            label.setStyleSheet(f"color: {theme.current().text_dim};")
            oid = "metadata.field." + name.replace(" ", "")
            if name == "Label":
                box = QComboBox(self.body)
                box.setObjectName("metadata.label")
                for lab in LABELS:
                    box.addItem(swatch(LABEL_RGB[lab]), tr(lab), lab)
                box.setCurrentIndex(max(0, box.findData(value)))
                box.activated.connect(lambda _i, b=box: self._commit("Label", str(b.currentData())))
                self.label_box = box
                self.form.addRow(label, box)
            elif fd.get("editable"):
                edit = QLineEdit(value, self.body)
                edit.setObjectName(oid)
                edit.editingFinished.connect(lambda n=name, e=edit: self._commit(n, e.text()))
                self.edits[name] = edit
                self.form.addRow(label, edit)
            else:
                v = QLabel(value, self.body)
                v.setObjectName(oid)
                v.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
                v.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
                v.setToolTip(value)
                self.form.addRow(label, v)

    def _commit(self, field: str, value: str) -> None:
        if self.item is None:
            return
        old = next((str(f.get("value") or "") for f in self.fields if f.get("name") == field), None)
        if old is not None and value != old:
            self.command.emit("metadata.set", {"item": self.item, "field": field, "value": value})


# ----- Info -----
class InfoPanel(QWidget):
    """The selected item's media properties, then the active sequence: its settings, the playhead
    and the clip under the playhead on each track (egui's Info panel)."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Info")
        self.text = QLabel(self)
        self.text.setObjectName("info.text")
        self.text.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        self.text.setTextFormat(Qt.TextFormat.RichText)
        self.text.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.text.setWordWrap(True)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.text)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 8, 8, 8)
        layout.addWidget(scroll)
        self.lines: list[tuple[str, str]] = []  # (key, value), headers have an empty value

    @staticmethod
    def _fps(settings: Any) -> str:
        rate = settings.get("frame_rate") if isinstance(settings, dict) else None
        try:
            num, den = (rate.get("num"), rate.get("den")) if isinstance(rate, dict) else rate
            v = float(num) / float(den)
        except (TypeError, ValueError, ZeroDivisionError):
            return "?"
        return f"{v:.3f}".rstrip("0").rstrip(".")

    def update_info(self, item: dict[str, Any] | None, sequence: dict[str, Any] | None, playhead: int) -> None:
        lines: list[tuple[str, str]] = []
        fields = {str(f.get("name")): str(f.get("value") or "") for f in (item or {}).get("fields") or [] if isinstance(f, dict)}
        if item and fields:
            lines.append((str(item.get("name") or ""), ""))
            lines.append((tr("Type:"), tr(fields.get("Media Type", ""))))
            if fields.get("Video Info"):
                lines.append((tr("Video:"), f"{fields.get('Frame Rate', '')}, {fields['Video Info']}"))
                lines.append((tr("Codec:"), fields.get("Video Codec", "")))
            if fields.get("Audio Info"):
                lines.append((tr("Audio:"), " - ".join(x for x in (fields["Audio Info"], fields.get("Audio Codec", "")) if x)))
            lines.append((tr("Duration:"), fields.get("Media Duration", "")))
            lines.append(("—", ""))
        if sequence:
            s = sequence.get("settings") or {}
            ft = frame_ticks(s)
            lines.append((str(sequence.get("name") or ""), ""))
            lines.append((tr("Settings:"), f"{s.get('width')}x{s.get('height')} · {self._fps(s)} fps · {s.get('sample_rate')} Hz"))
            lines.append((tr("Playhead:"), format_timecode(playhead, ft)))

            def under(track: dict[str, Any]) -> str:
                for c in track.get("items") or []:
                    if int(c.get("start") or 0) <= playhead < int(c.get("end") or 0):
                        return str(c.get("name") or "")
                return ""

            video = list(sequence.get("video") or [])
            for i in range(len(video) - 1, -1, -1):
                lines.append((fmt("Video {n}:", n=i + 1), under(video[i])))
            for i, t in enumerate(sequence.get("audio") or []):
                lines.append((fmt("Audio {n}:", n=i + 1), under(t)))
        if lines == self.lines:
            return
        self.lines = lines
        T = theme.current()
        html = []
        for k, v in lines:
            if k == "—":
                html.append(f"<hr style='border-color:{T.separator}'>")
            elif not v and not k.endswith(":"):
                html.append(f"<p style='margin:2px 0'><b>{_esc(k)}</b></p>")
            else:
                html.append(
                    f"<table cellspacing=0 cellpadding=0><tr><td width=90 style='color:{T.text_dim}'>{_esc(k)}</td><td>{_esc(v)}</td></tr></table>"
                )
        if not lines:
            html.append(f"<span style='color:{T.text_faint}'>{_esc(tr('(no sequence)'))}</span>")
        self.text.setText("".join(html))


def _esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# ----- Project Notes -----
class NotesPanel(QWidget):
    """Free plain text kept in the project file. Every change is a ``project.setNotes``; one
    typing session (from focusing the editor until it loses focus) is one undo step."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.ProjectNotes")
        self.edit = NotesEdit(self)
        self.edit.setObjectName("notes.text")
        self.edit.setPlaceholderText(tr("Notes for this project: to-dos, instructions, frame numbers… Saved with the project."))
        self.edit.textChanged.connect(self._changed)
        self._loading = False
        self._timer = QTimer(self, singleShot=True, interval=300)
        self._timer.timeout.connect(self._send)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 6, 8, 6)
        layout.addWidget(self.edit)

    def set_notes(self, text: str) -> None:
        if self.edit.hasFocus() or self._timer.isActive():
            return
        if self.edit.toPlainText() != text:
            self._loading = True
            self.edit.setPlainText(text)
            self._loading = False

    def _changed(self) -> None:
        if not self._loading:
            self._timer.start()

    def _send(self) -> None:
        self.command.emit("project.setNotes", {"text": self.edit.toPlainText(), "merge": str(self.edit.session)})

    def flush(self) -> None:
        if self._timer.isActive():
            self._timer.stop()
            self._send()


class NotesEdit(QPlainTextEdit):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.session = 0

    def focusInEvent(self, e) -> None:
        self.session += 1  # a new undo step
        super().focusInEvent(e)

    def focusOutEvent(self, e) -> None:
        super().focusOutEvent(e)
        parent = self.parent()
        if isinstance(parent, NotesPanel):
            parent.flush()


TITLES = (N_("Markers"), N_("Info"), N_("Metadata"), N_("Project Notes"))
