"""The Essential Graphics panel, as egui's (crates/ui-egui/src/panels/graphics_templates.rs and
the graphic editor in graphics.rs ``properties``).

Browse is the Graphics Templates library: search, category, Install…, cards with thumbnails
rendered by the engine (click selects, double-click or Apply places the template at the
playhead, a card dragged onto a video track places it there), Remove for a user template.
Edit is the graphic editor of the selected graphic clip: Layers (new text / rectangle /
ellipse, bring forward / send backward, delete, visibility, select), Template Properties,
Responsive Design – Time (with no layer selected) and – Position, Align and Transform, Text,
Shape and Appearance. The engine reads it out (``graphics.inspect``, ``graphics.template.list``,
``graphics.template.thumbnail``, ``fonts.list``); every change is an engine command, and a value
scrub is one undo step.
"""

from __future__ import annotations

import base64
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QMimeData, QPointF, QRectF, QSettings, QSize, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QDrag, QFont, QIcon, QImage, QPainter, QPaintEvent, QPen, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPlainTextEdit,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QSlider,
    QStackedWidget,
    QTabBar,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..hot_number import HotSpin
from ..i18n import N_, fmt, tr

TEMPLATE_MIME = "application/x-cfa4-template"
LABEL_W = 130  # egui's label column
THUMB_W = 192
SECTIONS = (
    N_("Layers"),
    N_("Template Properties"),
    N_("Responsive Design - Time"),
    N_("Responsive Design - Position"),
    N_("Align and Transform"),
    N_("Text"),
    N_("Shape"),
    N_("Appearance"),
)
CHOICE_NAMES = (
    N_("Rectangle"),
    N_("Ellipse"),
    N_("Polygon"),
    N_("Path"),
    N_("Solid"),
    N_("Linear Gradient"),
    N_("Outer"),
    N_("Center"),
    N_("Inner"),
    N_("Off"),
    N_("Roll"),
    N_("Crawl Left"),
    N_("Crawl Right"),
)


def template_mime(template_id: str) -> QMimeData:
    m = QMimeData()
    m.setData(TEMPLATE_MIME, template_id.encode())
    return m


def dropped_template(mime: QMimeData) -> str | None:
    if not mime.hasFormat(TEMPLATE_MIME):
        return None
    return bytes(mime.data(TEMPLATE_MIME)).decode() or None


def paint_glyph(p: QPainter, r: QRectF, how: str, c: QColor) -> None:
    """egui's code-drawn alignment glyphs: bars against an edge line, paragraph lines."""
    p.save()
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    x0, x1, y0, y1 = r.left() + 2, r.right() - 2, r.top() + 2, r.bottom() - 2
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    p.setPen(QPen(c, 1.2))

    def bar(ax: float, ay: float, bx: float, by: float) -> None:
        p.fillRect(QRectF(QPointF(min(ax, bx), min(ay, by)), QPointF(max(ax, bx), max(ay, by))), c)

    if how == "left":
        p.drawLine(QPointF(x0, y0), QPointF(x0, y1))
        bar(x0 + 2, y0 + 2, x1, y0 + 5)
        bar(x0 + 2, y1 - 5, cx + 1, y1 - 2)
    elif how == "right":
        p.drawLine(QPointF(x1, y0), QPointF(x1, y1))
        bar(x0, y0 + 2, x1 - 2, y0 + 5)
        bar(cx - 1, y1 - 5, x1 - 2, y1 - 2)
    elif how == "hcenter":
        p.drawLine(QPointF(cx, y0), QPointF(cx, y1))
        bar(x0 + 1, y0 + 2, x1 - 1, y0 + 5)
        bar(cx - 3, y1 - 5, cx + 3, y1 - 2)
    elif how == "top":
        p.drawLine(QPointF(x0, y0), QPointF(x1, y0))
        bar(x0 + 2, y0 + 2, x0 + 5, y1)
        bar(x1 - 5, y0 + 2, x1 - 2, cy + 1)
    elif how == "bottom":
        p.drawLine(QPointF(x0, y1), QPointF(x1, y1))
        bar(x0 + 2, y0, x0 + 5, y1 - 2)
        bar(x1 - 5, cy - 1, x1 - 2, y1 - 2)
    elif how == "vcenter":
        p.drawLine(QPointF(x0, cy), QPointF(x1, cy))
        bar(x0 + 2, y0 + 1, x0 + 5, y1 - 1)
        bar(x1 - 5, cy - 3, x1 - 2, cy + 3)
    elif how == "dh":
        for x in (x0 + 1, cx - 1.5, x1 - 4):
            bar(x, y0 + 3, x + 3, y1 - 3)
    elif how == "dv":
        for y in (y0 + 1, cy - 1.5, y1 - 4):
            bar(x0 + 3, y, x1 - 3, y + 3)
    elif how in ("pl", "pc", "pr", "pj"):
        for i, w in enumerate((1.0, 0.6, 0.85, 0.5)):
            w = (1.0 if how == "pj" and i < 3 else w) * (x1 - x0)
            y = y0 + 1 + i * 3.2
            xa = cx - w / 2 if how == "pc" else x1 - w if how == "pr" else x0
            p.drawLine(QPointF(xa, y), QPointF(xa + w, y))
    p.restore()


class GlyphButton(QToolButton):
    def __init__(self, how: str, tip: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.how = how
        self.setToolTip(tip)
        self.setFixedSize(24, 22)
        self.setAutoRaise(True)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        on = self.isChecked()
        hover = self.underMouse()
        if on:
            p.fillRect(self.rect(), QColor(0x4B, 0x4B, 0x4B))
        elif hover:
            p.fillRect(self.rect(), t.c("hover"))
        r = QRectF(self.width() / 2 - 8, self.height() / 2 - 7, 16, 14)
        paint_glyph(p, r, self.how, t.c("text") if on or hover else t.c("text_dim"))


class SectionHeader(QWidget):
    """A collapsible section's header row (egui's ``section_header``)."""

    twirled = Signal(str)

    def __init__(self, title: str, open_: bool, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.title = title
        self.open = open_
        self.setObjectName(f"graphics.section.{title.replace(' ', '')}")
        self.setFixedHeight(34)
        self.extra: QWidget | None = None

    def resizeEvent(self, _e: Any) -> None:
        if self.extra is not None:
            self.extra.move(self.width() - self.extra.width() - 4, 4 + (30 - self.extra.height()) // 2)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(t.c("separator"))
        p.drawLine(0, 4, self.width(), 4)
        mid = 4 + 15
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("text_dim"))
        if self.open:
            p.drawPolygon([QPointF(1, mid - 3), QPointF(11, mid - 3), QPointF(6, mid + 3)])
        else:
            p.drawPolygon([QPointF(3, mid - 5), QPointF(9, mid), QPointF(3, mid + 5)])
        font = QFont(self.font())
        font.setPixelSize(13)
        font.setWeight(QFont.Weight.DemiBold)
        p.setFont(font)
        p.setPen(t.c("text"))
        p.drawText(QRectF(18, 4, self.width() - 50, 30), Qt.AlignmentFlag.AlignVCenter, tr(self.title))

    def mousePressEvent(self, e: Any) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.twirled.emit(self.title)


class ColorButton(QPushButton):
    picked = Signal(str)

    def __init__(self, name: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(name)
        self.setFixedSize(34, 20)
        self.color = "#ffffff"
        self.clicked.connect(self._pick)

    def set_color(self, color: Any) -> None:
        c = QColor(str(color or "#ffffff"))
        if c.isValid() and c.name() != self.color:
            self.color = c.name()
            self.setStyleSheet(f"QPushButton {{ background: {self.color}; border: 1px solid {theme.current().field_border}; border-radius: 2px; }}")

    def _pick(self) -> None:
        c = QColorDialog.getColor(QColor(self.color), self, tr("Color"))
        if c.isValid():
            self.set_color(c.name())
            self.picked.emit(c.name())


class TextPropertiesDialog(QDialog):
    """Text Properties (the wrench in the Text section): Text Layer Type and Text Styling."""

    def __init__(self, paragraph: bool, vertical: bool, ligatures: bool, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("graphics.textProperties")
        self.setWindowTitle(tr("Text Properties"))
        v = QVBoxLayout(self)
        v.addWidget(QLabel(tr("Text Layer Type"), self))
        self.kind = QComboBox(self)
        self.kind.setObjectName("graphics.textProperties.type")
        self.kind.addItem(tr("Point Text"), "point")
        self.kind.addItem(tr("Paragraph Text"), "paragraph")
        if vertical:  # vertical text has no box to wrap in
            item = self.kind.model().item(1)  # type: ignore[attr-defined]
            item.setEnabled(False)
        self.kind.setCurrentIndex(1 if paragraph else 0)
        v.addWidget(self.kind)
        v.addSpacing(6)
        v.addWidget(QLabel(tr("Text Styling"), self))
        self.ligatures = QCheckBox(tr("Ligatures"), self)
        self.ligatures.setObjectName("graphics.textProperties.ligatures")
        self.ligatures.setChecked(ligatures)
        v.addWidget(self.ligatures)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.button(QDialogButtonBox.StandardButton.Ok).setText(tr("OK"))
        buttons.button(QDialogButtonBox.StandardButton.Cancel).setText(tr("Cancel"))
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        v.addWidget(buttons)


class TemplateList(QListWidget):
    """The template cards; a card dragged out carries its template id."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("gfxTemplates.list")
        self.setViewMode(QListWidget.ViewMode.IconMode)
        self.setResizeMode(QListWidget.ResizeMode.Adjust)
        self.setMovement(QListWidget.Movement.Static)
        self.setIconSize(QSize(150, 84))
        self.setGridSize(QSize(166, 136))
        self.setWordWrap(True)
        self.setSpacing(4)
        self.setUniformItemSizes(True)
        self.setDragEnabled(True)
        self.setDragDropMode(QAbstractItemView.DragDropMode.DragOnly)
        self.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)

    def startDrag(self, _actions: Any) -> None:
        item = self.currentItem()
        if item is None:
            return
        drag = QDrag(self)
        drag.setMimeData(template_mime(str(item.data(Qt.ItemDataRole.UserRole))))
        drag.setPixmap(item.icon().pixmap(120, 68))
        drag.exec(Qt.DropAction.CopyAction)


class TemplateBrowser(QWidget):
    """Essential Graphics ▸ Browse."""

    def __init__(self, panel: EssentialGraphicsPanel) -> None:
        super().__init__(panel)
        self.panel = panel
        self.entries: list[dict[str, Any]] = []
        self.thumbs: dict[str, QIcon] = {}
        self._asked: set[str] = set()
        bar = QHBoxLayout()
        bar.setContentsMargins(10, 6, 10, 2)
        self.search = QLineEdit(self)
        self.search.setObjectName("gfxTemplates.search")
        self.search.setPlaceholderText(tr("Search templates"))
        self.search.textChanged.connect(lambda _t: self._fill())
        self.category = QComboBox(self)
        self.category.setObjectName("gfxTemplates.category")
        self.category.setSizeAdjustPolicy(QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)
        self.category.setMinimumContentsLength(10)
        self.category.activated.connect(lambda _i: self._fill())
        install = QPushButton(tr("Install…"), self)
        install.setObjectName("gfxTemplates.install")
        install.setToolTip(tr("Install Motion Graphics Template (.fcgt)"))
        install.clicked.connect(lambda: panel.run.emit("graphics.template.install", {}))
        bar.addWidget(self.search, 1)
        bar.addWidget(self.category)
        bar.addWidget(install)
        self.list = TemplateList(self)
        self.list.itemSelectionChanged.connect(self._selected)
        self.list.itemDoubleClicked.connect(lambda item: self.apply(str(item.data(Qt.ItemDataRole.UserRole))))
        self.note = QLabel(tr("No templates match."), self)
        self.note.setEnabled(False)
        self.note.setContentsMargins(12, 4, 12, 4)
        foot = QHBoxLayout()
        foot.setContentsMargins(10, 2, 10, 8)
        self.apply_button = QPushButton(tr("Apply"), self)
        self.apply_button.setObjectName("gfxTemplates.apply")
        self.apply_button.clicked.connect(lambda: self.selected() and self.apply(str(self.selected().get("id"))))  # type: ignore[union-attr]
        self.remove = QPushButton(tr("Remove"), self)
        self.remove.setObjectName("gfxTemplates.remove")
        self.remove.clicked.connect(self._remove)
        self.count = QLabel(self)
        self.count.setEnabled(False)
        foot.addWidget(self.apply_button)
        foot.addWidget(self.remove)
        foot.addWidget(self.count, 1)
        v = QVBoxLayout(self)
        v.setContentsMargins(0, 0, 0, 0)
        v.addLayout(bar)
        v.addWidget(self.note)
        v.addWidget(self.list, 1)
        v.addLayout(foot)
        self._selected()

    def set_entries(self, entries: list[dict[str, Any]]) -> None:
        if entries == self.entries:
            return
        self.entries = entries
        cats = sorted({str(e.get("category") or "") for e in entries})
        cur = self.category.currentData()
        self.category.clear()
        self.category.addItem(tr("All Categories"), "")
        for c in cats:
            self.category.addItem(tr(c), c)
        self.category.setCurrentIndex(max(0, self.category.findData(cur)))
        self._fill()

    def shown(self) -> list[dict[str, Any]]:
        q = self.search.text().strip().lower()
        cat = self.category.currentData() or ""
        out = []
        for e in self.entries:
            hay = [str(e.get(k) or "") for k in ("name", "category", "description")] + [str(x) for x in e.get("tags") or []]
            hay += [tr(h) for h in hay]
            if (not cat or e.get("category") == cat) and (not q or any(q in h.lower() for h in hay)):
                out.append(e)
        return out

    def _fill(self) -> None:
        sel = (self.selected() or {}).get("id")
        self.list.blockSignals(True)
        self.list.clear()
        for e in self.shown():
            tid = str(e.get("id"))
            category = tr(str(e.get("category") or ""))
            sub = fmt("{category} · My Template", category=category) if e.get("source") == "user" else category
            item = QListWidgetItem(self.thumbs.get(tid, QIcon()), f"{tr(str(e.get('name') or ''))}\n{sub}")
            item.setData(Qt.ItemDataRole.UserRole, tid)
            item.setToolTip(tr(str(e.get("description") or "")))
            self.list.addItem(item)
            if tid == sel:
                item.setSelected(True)
            self._thumb(tid)
        self.list.blockSignals(False)
        self.note.setVisible(self.list.count() == 0)
        self._selected()

    def _thumb(self, tid: str) -> None:
        engine = self.panel.engine
        if tid in self._asked or engine is None or not getattr(engine, "running", False):
            return
        self._asked.add(tid)

        def got(r: Any, _b: bytes | None) -> None:
            data = (r or {}).get("pngBase64") if isinstance(r, dict) else None
            if not data:
                return
            img = QImage.fromData(base64.b64decode(data), "PNG")
            # a dark backdrop so white titles read (egui's)
            pm = QPixmap(150, 84)
            pm.fill(QColor(0x2A, 0x30, 0x3A))
            painter = QPainter(pm)
            scaled = img.scaled(150, 84, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation)
            painter.drawImage((150 - scaled.width()) // 2, (84 - scaled.height()) // 2, scaled)
            painter.end()
            self.thumbs[tid] = QIcon(pm)
            for i in range(self.list.count()):
                item = self.list.item(i)
                if item.data(Qt.ItemDataRole.UserRole) == tid:
                    item.setIcon(self.thumbs[tid])

        engine.execute("graphics.template.thumbnail", {"template": tid, "width": THUMB_W}, on_reply=got, on_error=lambda _m: None)

    def selected(self) -> dict[str, Any] | None:
        items = self.list.selectedItems()
        tid = items[0].data(Qt.ItemDataRole.UserRole) if items else None
        return next((e for e in self.entries if e.get("id") == tid), None)

    def _selected(self) -> None:
        e = self.selected()
        self.apply_button.setEnabled(e is not None)
        self.remove.setEnabled(e is not None and e.get("source") == "user")
        self.count.setText(fmt("{n} editable properties", n=len(e.get("controls") or [])) if e else "")

    def apply(self, tid: str) -> None:
        self.panel.send("graphics.template.apply", {"template": tid})
        self.panel.set_tab("edit")  # edit the new graphic's properties

    def _remove(self) -> None:
        e = self.selected()
        if e is not None:
            self.panel.send("graphics.template.remove", {"template": e.get("id")})
            self.panel.refresh_library()


class GraphicEditor(QWidget):
    """Essential Graphics ▸ Edit (and egui's Properties panel for a graphic clip)."""

    def __init__(self, panel: EssentialGraphicsPanel) -> None:
        super().__init__(panel)
        self.panel = panel
        self.data: dict[str, Any] = {}
        self.fonts: list[dict[str, Any]] = []
        self.collapsed: set[str] = set()
        self._shape: tuple[Any, ...] | None = None
        self._updaters: list[Callable[[dict[str, Any]], None]] = []
        self._labels: list[QLabel] = []
        self.placeholder = QWidget(self)
        pv = QVBoxLayout(self.placeholder)
        pv.addStretch(1)
        note = QLabel(tr("Select a graphic clip, or use the Type tool (T) on the Program monitor"), self.placeholder)
        note.setObjectName("graphics.placeholder")
        note.setWordWrap(True)
        note.setAlignment(Qt.AlignmentFlag.AlignCenter)
        note.setEnabled(False)
        pv.addWidget(note)
        create = QPushButton(tr("Create new graphic"), self.placeholder)
        create.setObjectName("graphics.createNew")
        create.clicked.connect(lambda: panel.send("graphics.newText", {"text": tr("New Text")}))
        row = QHBoxLayout()
        row.addStretch(1)
        row.addWidget(create)
        row.addStretch(1)
        pv.addLayout(row)
        pv.addStretch(1)
        self.body = QWidget(self)
        self.lay = QVBoxLayout(self.body)
        self.lay.setContentsMargins(8, 4, 6, 16)
        self.lay.setSpacing(0)
        self.scroll = QScrollArea(self)
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setWidget(self.body)
        v = QVBoxLayout(self)
        v.setContentsMargins(0, 0, 0, 0)
        v.addWidget(self.placeholder, 1)
        v.addWidget(self.scroll, 1)

    def _label_w(self) -> int:
        """egui's label column, narrower in a narrow dock so the controls still fit."""
        return max(60, min(LABEL_W - 20, int(self.scroll.viewport().width() * 0.3)))

    def resizeEvent(self, e: Any) -> None:
        super().resizeEvent(e)
        w = self._label_w()
        for label in self._labels:
            label.setFixedWidth(w)

    # ----- state -----
    @property
    def clip(self) -> Any:
        return self.data.get("clip")

    @property
    def layer(self) -> int | None:
        layer = self.data.get("layer")
        return layer if isinstance(layer, int) else None

    def param(self, pid: str) -> Any:
        return ((self.data.get("params") or {}).get(pid) or {}).get("value")

    def set_state(self, data: Any) -> None:
        data = data if isinstance(data, dict) else {}
        self.data = data
        has = data.get("clip") is not None
        self.placeholder.setVisible(not has)
        self.scroll.setVisible(has)
        if not has:
            self._shape = None
            return
        params = data.get("params") or {}
        flags = tuple(bool((params.get(k) or {}).get("value")) for k in ("fill", "background", "shadow"))
        shape = (
            data.get("clip"),
            data.get("layer"),
            data.get("kind"),
            repr(data.get("layers")),
            data.get("name"),
            frozenset(self.collapsed),
            (data.get("template") or {}).get("template") if data.get("template") else None,
            repr([(c.get("id"), c.get("kind"), "\n" in str(c.get("value"))) for c in (data.get("template") or {}).get("controls") or []]),
            (data.get("roll") or {}).get("mode"),
            data.get("pin") is not None,
            flags,
            (params.get("fill_kind") or {}).get("value"),
            len(self.fonts),
        )
        if shape != self._shape:
            self._shape = shape
            self._build()
            for combo in self.body.findChildren(QComboBox):  # narrow docks: combos shrink, not clip
                if combo.sizeAdjustPolicy() == QComboBox.SizeAdjustPolicy.AdjustToContentsOnFirstShow:
                    combo.setSizeAdjustPolicy(QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)
                    combo.setMinimumContentsLength(6)
        for update in self._updaters:
            update(data)

    def set_fonts(self, fonts: list[dict[str, Any]]) -> None:
        self.fonts = fonts
        self.set_state(self.data)

    # ----- sending -----
    def set(self, pid: str, value: Any, begin: bool = True) -> None:
        """A layer property; a scrub's later steps merge into its first undo step."""
        if self.layer is None:
            return
        params: dict[str, Any] = {"clip": self.clip, "layer": self.layer, "props": {pid: value}}
        if not begin:
            params.update(merge=True, begin=False)
        elif self.panel.scrubbing():
            params.update(merge=True, begin=True)
        monitor = getattr(self.window(), "monitor", None)
        if monitor is not None and getattr(monitor, "playing", False) and isinstance(getattr(monitor, "playhead", None), int):
            params["time"] = monitor.playhead  # the shown frame: the engine's playhead stays where playback started
        self.panel.send("graphics.set", params)

    # ----- building -----
    def _clear(self) -> None:
        self._updaters = []
        while self.lay.count():
            item = self.lay.takeAt(0)
            w = item.widget()
            if w is not None:
                w.hide()
                w.setParent(None)
                w.deleteLater()
            elif item.layout() is not None:
                self._clear_layout(item.layout())

    def _clear_layout(self, layout: Any) -> None:
        while layout.count():
            item = layout.takeAt(0)
            w = item.widget()
            if w is not None:
                w.hide()
                w.setParent(None)
                w.deleteLater()
            elif item.layout() is not None:
                self._clear_layout(item.layout())

    def _section(self, title: str) -> tuple[bool, SectionHeader]:
        head = SectionHeader(title, title not in self.collapsed, self.body)
        head.twirled.connect(self.panel.twirl)
        self.lay.addWidget(head)
        return head.open, head

    def _row(self, label: str, pid: str | None = None) -> QHBoxLayout:
        """egui's property row: a stopwatch for an animatable property, the label, the controls."""
        w = QWidget(self.body)
        w.setFixedHeight(28)
        h = QHBoxLayout(w)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(4)
        sw = QToolButton(w)
        sw.setFixedSize(16, 16)
        sw.setAutoRaise(True)
        info = ((self.data.get("params") or {}).get(pid) or {}) if pid else {}
        if info.get("animatable"):
            sw.setObjectName(f"graphics.prop.{pid}.stopwatch")
            sw.setText("⏱")
            sw.setCheckable(True)
            sw.setToolTip(tr("Toggle animation"))
            sw.clicked.connect(
                lambda _c=False, p=pid: self.panel.send(
                    "effects.toggleAnimation", {"clip": self.clip, "effect": self.data.get("effectIndex"), "param": p}
                )
            )
            self._updaters.append(lambda d, b=sw, p=pid: b.setChecked(bool(((d.get("params") or {}).get(p) or {}).get("animated"))))
        else:
            sw.setEnabled(False)
        h.addWidget(sw)
        text = QLabel(label, w)
        text.setToolTip(label)
        text.setFixedWidth(self._label_w())
        self._labels.append(text)
        text.setStyleSheet(f"color: {theme.current().text_dim};")
        h.addWidget(text)
        controls = QHBoxLayout()
        controls.setSpacing(4)
        h.addLayout(controls, 1)
        self.lay.addWidget(w)
        return controls

    def _spin(self, row: QHBoxLayout, pid: str, lo: float, hi: float, decimals: int, speed: float, unit: str, index: int | None = None) -> HotSpin:
        spin = HotSpin(self.body, lo, hi, decimals, speed, unit)
        spin.setObjectName(f"graphics.prop.{pid}" + ("" if index is None else (".x", ".y")[index]))

        def edited(v: float, begin: bool) -> None:
            if index is None:
                self.set(pid, v, begin)
            else:
                cur = list(self.param(pid) or [0.0, 0.0])
                cur[index] = v
                self.set(pid, cur, begin)

        spin.edited.connect(edited)
        row.addWidget(spin)

        def update(_d: dict[str, Any]) -> None:
            v = self.param(pid)
            spin.set_value(v[index] if index is not None and isinstance(v, list) and len(v) == 2 else v)

        self._updaters.append(update)
        return spin

    def number(self, label: str, pid: str, speed: float, lo: float, hi: float, decimals: int, unit: str) -> None:
        row = self._row(label, pid)
        self._spin(row, pid, lo, hi, decimals, speed, unit)
        row.addStretch(1)

    def point(self, label: str, pid: str) -> None:
        row = self._row(label, pid)
        self._spin(row, pid, -100_000, 100_000, 1, 1.0, " X", 0)
        self._spin(row, pid, -100_000, 100_000, 1, 1.0, " Y", 1)
        row.addStretch(1)

    def check(self, row: QHBoxLayout, pid: str, label: str = "") -> QCheckBox:
        box = QCheckBox(label, self.body)
        box.setObjectName(f"graphics.prop.{pid}")
        box.clicked.connect(lambda on: self.set(pid, bool(on)))
        row.addWidget(box)
        self._updaters.append(lambda _d: box.setChecked(bool(self.param(pid))))
        return box

    def color(self, row: QHBoxLayout, pid: str) -> None:
        b = ColorButton(f"graphics.prop.{pid}", self.body)
        b.picked.connect(lambda c: self.set(pid, c))
        row.addWidget(b)
        self._updaters.append(lambda _d: b.set_color(self.param(pid)))

    def choice(self, row: QHBoxLayout, pid: str, options: list[str], width: int) -> None:
        combo = QComboBox(self.body)
        combo.setObjectName(f"graphics.prop.{pid}")
        combo.setMaximumWidth(width)
        combo.setMinimumWidth(min(width, 52))
        for o in options:
            combo.addItem(tr(o))
        combo.activated.connect(lambda i: i != self.param(pid) and self.set(pid, i))
        row.addWidget(combo)

        def update(_d: dict[str, Any]) -> None:
            if not combo.view().isVisible():
                v = self.param(pid)
                combo.setCurrentIndex(v if isinstance(v, int) and 0 <= v < combo.count() else 0)

        self._updaters.append(update)

    def _build(self) -> None:
        self._clear()
        self._labels = []
        d = self.data
        body = self.body
        head = QHBoxLayout()
        head.setContentsMargins(0, 6, 0, 6)
        swatch = QLabel(body)
        swatch.setFixedSize(12, 12)
        swatch.setStyleSheet(f"background: {d.get('labelColor') or '#888'}; border-radius: 2px;")
        name = QLabel(str(d.get("name") or ""), body)
        name.setObjectName("graphics.name")
        name.setStyleSheet("font-weight: 600;")
        head.addWidget(swatch)
        head.addSpacing(6)
        head.addWidget(name, 1)
        self.lay.addLayout(head)
        self._layers()
        if d.get("template") and self._section("Template Properties")[0]:
            self._template()
        if self.layer is None:
            if self._section("Responsive Design - Time")[0]:
                self._time()
            note = QLabel(tr("Select a layer to edit its properties."), body)
            note.setEnabled(False)
            note.setContentsMargins(0, 12, 0, 0)
            self.lay.addWidget(note)
            self.lay.addStretch(1)
            return
        if self._section("Responsive Design - Position")[0]:
            self._position()
        if self._section("Align and Transform")[0]:
            self._transform()
        if d.get("kind") == "text":
            self._text()
        if d.get("kind") == "shape" and self._section("Shape")[0]:
            row = self._row(tr("Shape"))
            self.choice(row, "shape", [str(x) for x in d.get("shapes") or []], 120)
            row.addStretch(1)
            self.point(tr("Size"), "size")
            self.number(tr("Corner Radius"), "corner_radius", 0.5, 0, 10_000, 0, "")
            self.number(tr("Polygon Sides"), "sides", 0.05, 3, 64, 0, "")
        if self._section("Appearance")[0]:
            self._appearance()
        self.lay.addStretch(1)

    def _layers(self) -> None:
        d = self.data
        if not self._section("Layers")[0]:
            return
        bar = QHBoxLayout()
        bar.setContentsMargins(14, 2, 0, 2)
        bar.setSpacing(2)
        clip = self.clip

        def tool(name: str, text: str, tip: str, slot: Callable[[], None]) -> QToolButton:
            b = QToolButton(self.body)
            b.setObjectName(name)
            b.setText(text)
            b.setToolTip(tip)
            b.setAutoRaise(True)
            b.setFixedSize(26, 22)
            b.clicked.connect(slot)
            bar.addWidget(b)
            return b

        tool("graphics.NewTextLayer", "T", tr("New Text Layer"), lambda: self.panel.send("graphics.newText", {"clip": clip, "text": tr("New Text")}))
        tool("graphics.NewRectangle", "▭", tr("New Rectangle"), lambda: self.panel.send("graphics.newShape", {"clip": clip, "shape": "rectangle"}))
        tool(
            "graphics.NewEllipse",
            "◯",
            tr("New Ellipse"),
            lambda: self.panel.send("graphics.newShape", {"clip": clip, "shape": "ellipse", "size": [200, 200]}),
        )
        bar.addSpacing(8)
        layer = self.layer
        if layer is not None:
            for text, tip, to in (("↑", tr("Bring Forward"), "forward"), ("↓", tr("Send Backward"), "backward")):
                tool(
                    f"graphics.arrange.{to}",
                    text,
                    tip,
                    lambda t=to: self.panel.send("graphics.arrangeLayer", {"clip": clip, "layer": layer, "to": t}),
                )
            tool("graphics.deleteLayer", "🗑", tr("Delete Layer"), lambda: self.panel.send("graphics.deleteLayer", {"clip": clip, "layer": layer}))
        bar.addStretch(1)
        self.lay.addLayout(bar)
        t = theme.current()
        for entry in reversed([x for x in d.get("layers") or [] if isinstance(x, dict)]):  # front first
            li = entry.get("layer")
            row = QWidget(self.body)
            row.setObjectName(f"graphics.layers.{li}")
            row.setFixedHeight(26)
            on = li == layer
            row.setAutoFillBackground(False)
            if on:
                row.setStyleSheet("QWidget#{} {{ background: #2e3d55; border-radius: 2px; }}".format(row.objectName().replace(".", "\\.")))
            h = QHBoxLayout(row)
            h.setContentsMargins(14, 0, 4, 0)
            kind = QLabel("T" if entry.get("kind") == "text" else "◆", row)
            kind.setFixedWidth(16)
            kind.setStyleSheet(f"color: {t.text_dim}; font-weight: 600; background: transparent;")
            name = QLabel(str(entry.get("name") or ""), row)
            name.setStyleSheet("background: transparent;")
            eye = QToolButton(row)
            eye.setObjectName(f"graphics.layers.{li}.visible")
            eye.setText("👁" if entry.get("enabled") else "–")
            eye.setToolTip(tr("Toggle visibility"))
            eye.setAutoRaise(True)
            enabled = bool(entry.get("enabled"))
            eye.clicked.connect(
                lambda _c=False, n=li, en=enabled: self.panel.send("graphics.set", {"clip": clip, "layer": n, "props": {"enabled": not en}})
            )
            h.addWidget(kind)
            h.addWidget(name, 1)
            h.addWidget(eye)
            row.mousePressEvent = lambda _e, n=li: self.panel.send("graphics.selectLayer", {"clip": clip, "layers": [n]})  # type: ignore[method-assign]
            self.lay.addWidget(row)

    def _time(self) -> None:
        clip = self.clip
        for key, label in (("intro", tr("Intro Duration")), ("outro", tr("Outro Duration"))):
            row = self._row(label)
            spin = HotSpin(self.body, 0, 100_000, 0, 0.2, " fr")
            spin.setObjectName(f"graphics.time.{key}")
            spin.edited.connect(lambda v, _b, k=key: self.panel.send("graphics.setResponsiveTime", {"clip": clip, f"{k}Frames": round(v)}))
            row.addWidget(spin)
            row.addStretch(1)
            self._updaters.append(lambda d, s=spin, k=key: s.set_value((d.get("time") or {}).get(k)))
        row = self._row(tr("Roll"))
        combo = QComboBox(self.body)
        combo.setObjectName("graphics.roll.mode")
        for mode, label in self.data.get("rollModes") or []:
            combo.addItem(tr(str(label)), mode)
        combo.activated.connect(
            lambda i: combo.itemData(i) != (self.data.get("roll") or {}).get("mode")
            and self.panel.send("graphics.setRoll", {"clip": clip, "mode": combo.itemData(i)})
        )
        row.addWidget(combo)
        row.addStretch(1)
        self._updaters.append(lambda d: combo.view().isVisible() or combo.setCurrentIndex(max(0, combo.findData((d.get("roll") or {}).get("mode")))))
        if (self.data.get("roll") or {}).get("mode") in (None, "off"):
            return
        row = self._row("")
        for key, label in (("startOffScreen", tr("Start Off Screen")), ("endOffScreen", tr("End Off Screen"))):
            box = QCheckBox(label, self.body)
            box.setObjectName(f"graphics.roll.{key}")
            box.clicked.connect(lambda on, k=key: self.panel.send("graphics.setRoll", {"clip": clip, k: bool(on)}))
            row.addWidget(box)
            self._updaters.append(lambda d, b=box, k=key: b.setChecked(bool((d.get("roll") or {}).get(k))))
        row.addStretch(1)
        for key, label in (("preroll", tr("Preroll")), ("easeIn", tr("Ease In")), ("easeOut", tr("Ease Out")), ("postroll", tr("Postroll"))):
            row = self._row(label)
            spin = HotSpin(self.body, 0, 100_000, 0, 0.2, " fr")
            spin.setObjectName(f"graphics.roll.{key}")
            spin.edited.connect(lambda v, _b, k=key: self.panel.send("graphics.setRoll", {"clip": clip, f"{k}Frames": round(v)}))
            row.addWidget(spin)
            row.addStretch(1)
            self._updaters.append(lambda d, s=spin, k=key: s.set_value((d.get("roll") or {}).get(k)))

    def _position(self) -> None:
        clip, layer = self.clip, self.layer
        layers = [x for x in self.data.get("layers") or [] if isinstance(x, dict)]
        row = self._row(tr("Pin To"))
        combo = QComboBox(self.body)
        combo.setObjectName("graphics.pin.to")
        combo.addItem(tr("None"), "none")
        combo.addItem(tr("Video Frame"), "frame")
        for x in layers:
            if x.get("layer") != layer:
                combo.addItem(str(x.get("name") or ""), x.get("layer"))
        combo.activated.connect(lambda i: self.panel.send("graphics.pin", {"clip": clip, "layer": layer, "to": combo.itemData(i)}))
        row.addWidget(combo)
        row.addStretch(1)

        def update(d: dict[str, Any]) -> None:
            pin = d.get("pin")
            if not combo.view().isVisible():
                combo.setCurrentIndex(max(0, combo.findData("none" if pin is None else pin.get("to"))))

        self._updaters.append(update)
        if self.data.get("pin") is None:
            return
        row = self._row(tr("Pinned Edges"))
        edges: dict[str, QToolButton] = {}

        def send_edges() -> None:
            on = [n for n, b in edges.items() if b.isChecked()]
            to = (self.data.get("pin") or {}).get("to")
            if on:
                self.panel.send("graphics.pin", {"clip": clip, "layer": layer, "to": to, "edges": on})
            else:
                self.panel.send("graphics.pin", {"clip": clip, "layer": layer, "to": "none"})

        for name in ("left", "top", "right", "bottom"):
            b = QToolButton(self.body)
            b.setObjectName(f"graphics.pin.{name}")
            b.setText(name[:1].upper())
            b.setCheckable(True)
            b.setFixedSize(26, 22)
            b.clicked.connect(send_edges)
            edges[name] = b
            row.addWidget(b)
            self._updaters.append(lambda d, bb=b, n=name: bb.setChecked(bool((d.get("pin") or {}).get(n))))
        row.addStretch(1)

    def _transform(self) -> None:
        clip = self.clip
        bar = QHBoxLayout()
        bar.setContentsMargins(14, 2, 0, 2)
        bar.setSpacing(2)
        for how, tip in (
            ("left", tr("Align Left")),
            ("hcenter", tr("Align Center Horizontally")),
            ("right", tr("Align Right")),
            ("top", tr("Align Top")),
            ("vcenter", tr("Align Center Vertically")),
            ("bottom", tr("Align Bottom")),
        ):
            b = GlyphButton(how, tip, self.body)
            b.setObjectName(f"graphics.align.{how}")
            b.clicked.connect(lambda _c=False, h=how: self.panel.send("graphics.align", {"clip": clip, "align": h}))
            bar.addWidget(b)
        bar.addSpacing(6)
        n = len(self.data.get("layers") or [])
        for how, axis, tip in (("dh", "horizontal", tr("Distribute Horizontally")), ("dv", "vertical", tr("Distribute Vertically"))):
            b = GlyphButton(how, tip, self.body)
            b.setObjectName(f"graphics.distribute.{axis}")
            b.clicked.connect(lambda _c=False, a=axis: self.panel.send("graphics.distribute", {"clip": clip, "axis": a, "layers": list(range(n))}))
            bar.addWidget(b)
        bar.addStretch(1)
        self.lay.addLayout(bar)
        self.point(tr("Position"), "position")
        self.point(tr("Anchor point"), "anchor")
        self.number(tr("Scale"), "scale", 0.5, 0, 100_000, 0, " %")
        self.number(tr("Rotation"), "rotation", 0.5, -36_000, 36_000, 1, " °")
        self.number(tr("Opacity"), "opacity", 0.5, 0, 100, 0, " %")

    def _text(self) -> None:
        clip, layer = self.clip, self.layer
        opened, head = self._section("Text")
        wrench = QToolButton(head)
        wrench.setObjectName("graphics.textProperties")
        wrench.setText("🔧")
        wrench.setAutoRaise(True)
        wrench.setToolTip(tr("Text Properties"))
        wrench.setFixedSize(22, 22)
        wrench.clicked.connect(self._text_properties)
        head.extra = wrench
        if not opened:
            return
        source = QPlainTextEdit(self.body)
        source.setObjectName("graphics.sourceText")
        source.setFixedHeight(52)
        source.setContentsMargins(14, 0, 0, 0)

        def typed() -> None:
            if source.hasFocus() and source.toPlainText() != self.param("text"):
                self.panel.send("graphics.setText", {"clip": clip, "layer": layer, "text": source.toPlainText(), "merge": True})

        source.textChanged.connect(typed)
        wrap = QHBoxLayout()
        wrap.setContentsMargins(14, 2, 0, 4)
        wrap.addWidget(source)
        self.lay.addLayout(wrap)

        def update_source(_d: dict[str, Any]) -> None:
            text = str(self.param("text") or "")
            if not source.hasFocus() and source.toPlainText() != text:
                source.blockSignals(True)
                source.setPlainText(text)
                source.blockSignals(False)

        self._updaters.append(update_source)
        families = sorted({str(f.get("family")) for f in self.fonts if f.get("family") and not str(f.get("family")).startswith(".")}, key=str.casefold)
        row = self._row(tr("Font"), "font")
        font = QComboBox(self.body)
        font.setObjectName("graphics.prop.font")
        font.setMinimumWidth(90)
        font.addItems(families)
        font.activated.connect(lambda _i: font.currentText() != self.param("font") and self.set("font", font.currentText()))
        row.addWidget(font)
        row.addStretch(1)
        row = self._row(tr("Font Style"), "font_style")
        style = QComboBox(self.body)
        style.setObjectName("graphics.prop.font_style")
        style.setMinimumWidth(90)
        style.activated.connect(lambda _i: style.currentText() != self.param("font_style") and self.set("font_style", style.currentText()))
        row.addWidget(style)
        row.addStretch(1)

        def update_font(_d: dict[str, Any]) -> None:
            fam = str(self.param("font") or "")
            if not font.view().isVisible():
                if font.findText(fam) < 0 and fam:
                    font.insertItem(0, fam)
                font.setCurrentText(fam)
            styles: list[str] = []
            for f in self.fonts:
                if str(f.get("family") or "").lower() == fam.lower():
                    styles = [str(s) for s in f.get("styles") or []]
            seen: list[str] = []
            for s in [*styles, "Bold", "Italic", "Bold Italic"]:
                if s not in seen:
                    seen.append(s)
            if not style.view().isVisible():
                if [style.itemText(i) for i in range(style.count())] != seen:
                    style.clear()
                    style.addItems(seen)
                style.setCurrentText(str(self.param("font_style") or ""))

        self._updaters.append(update_font)
        self.number(tr("Font Size"), "size", 0.5, 1, 2000, 0, "")
        row = self._row(tr("Alignment"))
        aligns = []
        for i, (how, tip) in enumerate((("pl", tr("Left Align Text")), ("pc", tr("Center Align Text")), ("pr", tr("Right Align Text")), ("pj", tr("Justify")))):
            b = GlyphButton(how, tip, self.body)
            b.setObjectName(f"graphics.prop.align.{i}")
            b.setCheckable(True)
            b.clicked.connect(lambda _c=False, n=i: self.set("align", n))
            row.addWidget(b)
            aligns.append(b)
        row.addStretch(1)
        self._updaters.append(lambda _d: [b.setChecked(self.param("align") == i) for i, b in enumerate(aligns)])
        self.number(tr("Tracking"), "tracking", 1.0, -1000, 10_000, 0, "")
        self.number(tr("Leading"), "leading", 0.5, -5000, 5000, 0, "")
        self.number(tr("Baseline Shift"), "baseline_shift", 0.5, -5000, 5000, 0, "")
        row = self._row(tr("Orientation"))
        self.check(row, "vertical", tr("Vertical Text"))
        row.addStretch(1)
        self.number(tr("Text Box Width"), "box_width", 2.0, 0, 100_000, 0, "")
        self.number(tr("Text Box Height"), "box_height", 2.0, 0, 100_000, 0, "")
        row = self._row(tr("Style"))
        letters: list[tuple[QToolButton, Callable[[], bool]]] = []
        for pid, text, tip, css in (
            ("faux_bold", "T", tr("Faux Bold"), "font-weight: 700;"),
            ("faux_italic", "T", tr("Faux Italic"), "font-style: italic;"),
            ("all_caps", "TT", tr("All Caps"), ""),
            ("small_caps", "Tt", tr("Small Caps"), ""),
            ("underline", "U", tr("Underline"), "text-decoration: underline;"),
        ):
            b = QToolButton(self.body)
            b.setObjectName(f"graphics.prop.{pid}")
            b.setText(text)
            b.setToolTip(tip)
            b.setCheckable(True)
            b.setFixedSize(26, 22)
            if css:
                b.setStyleSheet(f"QToolButton {{ {css} }}")
            if pid == "all_caps":
                is_on: Callable[[], bool] = lambda: self.param("caps") == 1
                b.clicked.connect(lambda _c=False: self.set("caps", 0 if self.param("caps") == 1 else 1))
            elif pid == "small_caps":
                is_on = lambda: self.param("caps") == 2
                b.clicked.connect(lambda _c=False: self.set("caps", 0 if self.param("caps") == 2 else 2))
            else:
                is_on = lambda p=pid: bool(self.param(p))
                b.clicked.connect(lambda _c=False, p=pid: self.set(p, not bool(self.param(p))))
            row.addWidget(b)
            letters.append((b, is_on))
        row.addStretch(1)
        self._updaters.append(lambda _d: [b.setChecked(f()) for b, f in letters])
        row = self._row(tr("OpenType"))
        self.check(row, "kerning", tr("Kerning"))
        self.check(row, "ligatures", tr("Ligatures"))
        row.addStretch(1)

    def _text_properties(self) -> None:
        layer = self.layer
        if layer is None:
            return
        vertical = bool(self.param("vertical"))
        paragraph = float(self.param("box_width") or 0) > 0 and not vertical
        ligatures = bool(self.param("ligatures"))
        dlg = TextPropertiesDialog(paragraph, vertical, ligatures, self)
        self.panel.dialog = dlg
        if dlg.exec() == QDialog.DialogCode.Accepted:
            # a layer already of that type is left alone (no undo step)
            self.panel.send("graphics.setTextType", {"clip": self.clip, "layer": layer, "type": dlg.kind.currentData()})
            if dlg.ligatures.isChecked() != ligatures:
                self.panel.send("graphics.set", {"clip": self.clip, "layer": layer, "props": {"ligatures": dlg.ligatures.isChecked()}})
        self.panel.dialog = None

    def _appearance(self) -> None:
        d = self.data
        params = d.get("params") or {}
        flag = lambda k: bool((params.get(k) or {}).get("value"))
        row = self._row(tr("Fill"))
        self.check(row, "fill")
        self.color(row, "fill_color")
        if flag("fill"):
            self.choice(row, "fill_kind", [str(x) for x in d.get("fillKinds") or []], 130)
        row.addStretch(1)
        if flag("fill") and (params.get("fill_kind") or {}).get("value") == 1:
            row = self._row(tr("Gradient"))
            self.color(row, "gradient_start")
            self.color(row, "gradient_end")
            row.addStretch(1)
            self.number("   " + tr("Angle"), "gradient_angle", 0.5, -3600, 3600, 0, " °")
        strokes = [str(x) for x in d.get("strokes") or []]
        for on, col, width, kind, label in (
            ("stroke", "stroke_color", "stroke_width", "stroke_type", tr("Stroke")),
            ("stroke2", "stroke2_color", "stroke2_width", "stroke2_type", tr("Stroke 2")),
        ):
            row = self._row(label)
            self.check(row, on)
            self.color(row, col)
            self._spin(row, width, 0, 1000, 1, 0.2, "")
            self.choice(row, kind, strokes, 80)
            row.addStretch(1)
        row = self._row(tr("Background"))
        self.check(row, "background")
        self.color(row, "background_color")
        row.addStretch(1)
        if flag("background"):
            self.number("   " + tr("Opacity"), "background_opacity", 0.5, 0, 100, 0, " %")
            self.number("   " + tr("Size"), "background_size", 0.5, 0, 1000, 0, "")
            self.number("   " + tr("Corner Radius"), "background_radius", 0.5, 0, 1000, 0, "")
        row = self._row(tr("Shadow"))
        self.check(row, "shadow")
        self.color(row, "shadow_color")
        row.addStretch(1)
        if flag("shadow"):
            self.number("   " + tr("Opacity"), "shadow_opacity", 0.5, 0, 100, 0, " %")
            self.number("   " + tr("Angle"), "shadow_angle", 0.5, -36_000, 36_000, 0, " °")
            self.number("   " + tr("Distance"), "shadow_distance", 0.5, 0, 1000, 0, "")
            self.number("   " + tr("Size"), "shadow_size", 0.5, 0, 1000, 0, "")
            self.number("   " + tr("Blur"), "shadow_blur", 0.5, 0, 1000, 0, "")

    def _template(self) -> None:
        tpl = self.data.get("template") or {}
        clip = self.clip
        note = QLabel("   " + fmt("Template: {name}", name=str(tpl.get("name") or "")), self.body)
        note.setEnabled(False)
        self.lay.addWidget(note)

        def setter(cid: Any) -> Callable[[Any], None]:
            return lambda v: self.panel.send("graphics.template.set", {"clip": clip, "control": cid, "value": v})

        def value_of(d: dict[str, Any], cid: Any) -> Any:
            return next((c.get("value") for c in (d.get("template") or {}).get("controls") or [] if c.get("id") == cid), None)

        for c in [x for x in tpl.get("controls") or [] if isinstance(x, dict)]:
            cid, kind, name = c.get("id"), c.get("kind"), tr(str(c.get("name") or ""))
            set_ = setter(cid)
            oid = f"gfxTemplates.control.{cid}"
            if kind == "text" and c.get("param") != "enabled" and "\n" in str(c.get("value") or ""):
                label = QLabel(f"    {name}", self.body)
                label.setStyleSheet(f"color: {theme.current().text_dim};")
                self.lay.addWidget(label)
                edit = QPlainTextEdit(self.body)
                edit.setObjectName(oid)
                edit.setFixedHeight(18 * min(8, max(2, str(c.get("value")).count("\n") + 1)) + 10)
                edit.textChanged.connect(lambda e=edit, f=set_, i=cid: e.hasFocus() and e.toPlainText() != value_of(self.data, i) and f(e.toPlainText()))
                wrap = QHBoxLayout()
                wrap.setContentsMargins(14, 0, 4, 4)
                wrap.addWidget(edit)
                self.lay.addLayout(wrap)

                def upd_multi(d: dict[str, Any], e: QPlainTextEdit = edit, i: Any = cid) -> None:
                    v = str(value_of(d, i) or "")
                    if not e.hasFocus() and e.toPlainText() != v:
                        e.blockSignals(True)
                        e.setPlainText(v)
                        e.blockSignals(False)

                self._updaters.append(upd_multi)
                continue
            row = self._row(name)
            if c.get("param") == "enabled" or kind == "checkbox":
                box = QCheckBox(self.body)
                box.setObjectName(oid)
                box.clicked.connect(lambda on, f=set_: f(bool(on)))
                row.addWidget(box)
                self._updaters.append(lambda d, b=box, i=cid: b.setChecked(bool(value_of(d, i))))
            elif kind == "text":
                line = QLineEdit(self.body)
                line.setObjectName(oid)
                line.textEdited.connect(lambda s, f=set_: f(s))
                row.addWidget(line, 1)
                self._updaters.append(lambda d, w=line, i=cid: w.hasFocus() or w.setText(str(value_of(d, i) or "")))
            elif kind == "color":
                b = ColorButton(oid, self.body)
                b.picked.connect(lambda col, f=set_: f(col))
                row.addWidget(b)
                self._updaters.append(lambda d, w=b, i=cid: w.set_color(value_of(d, i)))
            elif kind == "slider":
                lo, hi = float(c.get("min") if c.get("min") is not None else 0), float(c.get("max") if c.get("max") is not None else 100)
                slider = QSlider(Qt.Orientation.Horizontal, self.body)
                slider.setObjectName(oid)
                slider.setRange(0, 1000)
                spin = HotSpin(self.body, lo, hi, 1, max(0.1, (hi - lo) / 200), "")
                spin.setObjectName(f"{oid}.value")
                to_pos = lambda v, a=lo, b=hi: round((float(v) - a) / (b - a) * 1000) if b > a else 0
                slider.valueChanged.connect(lambda p, f=set_, a=lo, b=hi, s=slider: s.isSliderDown() and f(round(a + p / 1000 * (b - a), 3)))
                slider.sliderReleased.connect(lambda f=set_, a=lo, b=hi, s=slider: f(round(a + s.value() / 1000 * (b - a), 3)))
                spin.edited.connect(lambda v, _b, f=set_: f(v))
                row.addWidget(slider, 1)
                row.addWidget(spin)

                def upd_slider(d: dict[str, Any], s: QSlider = slider, sp: HotSpin = spin, i: Any = cid, tp: Any = to_pos) -> None:
                    v = value_of(d, i)
                    if isinstance(v, int | float):
                        if not s.isSliderDown():
                            s.blockSignals(True)
                            s.setValue(tp(v))
                            s.blockSignals(False)
                        sp.set_value(v)

                self._updaters.append(upd_slider)
            elif kind == "font":
                families = sorted({str(f.get("family")) for f in self.fonts if f.get("family")}, key=str.casefold)
                combo = QComboBox(self.body)
                combo.setObjectName(oid)
                combo.addItems(families)
                combo.activated.connect(lambda _i, cb=combo, f=set_: f(cb.currentText()))
                row.addWidget(combo)

                def upd_font(d: dict[str, Any], cb: QComboBox = combo, i: Any = cid) -> None:
                    v = value_of(d, i)
                    fam = str(v.get("family") if isinstance(v, dict) else v or "")
                    if not cb.view().isVisible():
                        if fam and cb.findText(fam) < 0:
                            cb.insertItem(0, fam)
                        cb.setCurrentText(fam)

                self._updaters.append(upd_font)
            elif kind == "position":
                spins = []
                for k, axis in enumerate(("X", "Y")):
                    sp = HotSpin(self.body, -100_000, 100_000, 1, 1.0, f" {axis}")
                    sp.setObjectName(f"{oid}.{axis.lower()}")

                    def edited(v: float, _b: bool, k: int = k, i: Any = cid, f: Callable[[Any], None] = set_) -> None:
                        cur = value_of(self.data, i)
                        cur = list(cur) if isinstance(cur, list) and len(cur) == 2 else [0.0, 0.0]
                        cur[k] = v
                        f(cur)

                    sp.edited.connect(edited)
                    row.addWidget(sp)
                    spins.append(sp)
                self._updaters.append(
                    lambda d, ss=spins, i=cid: [s.set_value(v) for s, v in zip(ss, value_of(d, i) or [], strict=False)]
                )
            if not any(row.stretch(k) for k in range(row.count())):  # a text field or slider fills the row
                row.addStretch(1)


class EssentialGraphicsPanel(QWidget):
    command = Signal(str, dict)
    run = Signal(str, dict)  # a command that needs a file (Install…)

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.EssentialGraphics")
        self.engine = engine
        self.settings: QSettings | None = None
        self.dialog: QDialog | None = None
        self._tab_chosen = ""  # "browse" / "edit" once picked; "" follows the selection (egui's)
        self.tabs = QTabBar(self)
        self.tabs.setObjectName("essentialGraphics.tabs")
        self.tabs.setDrawBase(False)
        self.tabs.setExpanding(False)
        self.tabs.setStyleSheet("QTabBar { background: transparent; }")  # the app sheet paints dock tab bars
        self.tabs.addTab(tr("Browse"))
        self.tabs.addTab(tr("Edit"))
        self.tabs.tabBarClicked.connect(lambda i: self.set_tab("browse" if i == 0 else "edit"))
        self.browser = TemplateBrowser(self)
        self.editor = GraphicEditor(self)
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.browser)
        self.stack.addWidget(self.editor)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 4, 0, 0)
        row = QHBoxLayout()
        row.addWidget(self.tabs)
        row.addStretch(1)
        lay.addLayout(row)
        lay.addWidget(self.stack, 1)
        self._library_asked = False
        self._fonts_asked = False
        self._show_tab()

    def attach_settings(self, settings: QSettings) -> None:
        self.settings = settings
        raw = settings.value("essentialGraphics/collapsed", [])
        self.editor.collapsed = {str(s) for s in (raw if isinstance(raw, list) else [raw] if raw else [])}

    def scrubbing(self) -> bool:
        return any(s._scrubbing for s in self.editor.findChildren(HotSpin))

    def send(self, command: str, params: dict[str, Any]) -> None:
        self.command.emit(command, params)

    def tab(self) -> str:
        if self._tab_chosen:
            return self._tab_chosen
        return "edit" if self.editor.clip is not None else "browse"

    def set_tab(self, key: str) -> None:
        self._tab_chosen = key
        self._show_tab()

    def _show_tab(self) -> None:
        edit = self.tab() == "edit"
        self.tabs.blockSignals(True)
        self.tabs.setCurrentIndex(1 if edit else 0)
        self.tabs.blockSignals(False)
        self.stack.setCurrentWidget(self.editor if edit else self.browser)
        if not edit:
            self.refresh_library()

    def twirl(self, title: str) -> None:
        self.editor.collapsed ^= {title}
        if self.settings is not None:
            self.settings.setValue("essentialGraphics/collapsed", sorted(self.editor.collapsed))
        self.editor.set_state(self.editor.data)

    def set_state(self, data: Any) -> None:
        """Show what ``graphics.inspect`` returned."""
        self.editor.set_state(data)
        self._show_tab()
        if self.editor.clip is not None and not self._fonts_asked and self.engine is not None and getattr(self.engine, "running", False):
            self._fonts_asked = True
            self.engine.execute("fonts.list", {}, on_reply=lambda r, _b: self.editor.set_fonts([f for f in r or [] if isinstance(f, dict)]))

    def refresh_library(self) -> None:
        if self.engine is None or not getattr(self.engine, "running", False) or not self.isVisible():
            return

        def got(r: Any, _b: bytes | None) -> None:
            self.browser.set_entries([e for e in r or [] if isinstance(e, dict)])

        QTimer.singleShot(0, lambda: self.engine.execute("graphics.template.list", {}, on_reply=got, on_error=lambda _m: None))

    def showEvent(self, e: Any) -> None:
        super().showEvent(e)
        if self.tab() == "browse":
            self.refresh_library()

    def sizeHint(self) -> QSize:
        return QSize(320, 500)


__all__ = ["TEMPLATE_MIME", "EssentialGraphicsPanel", "dropped_template", "template_mime"]

_ = QSizePolicy  # kept for the layout helpers' callers
