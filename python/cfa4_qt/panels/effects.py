"""The Effects panel, as egui's (crates/ui-egui/src/panels/effects.rs and presets.rs).

A searchable folder tree in Premiere's order: Presets (built-in and user effect presets from
``presets.list``; right-click to delete a user preset, export one, import presets), Lumetri
Presets (sub-folders with thumbnails the engine renders, ``lumetri.presetThumbnail``), Audio
Effects, Audio Transitions, Video Effects, Video Transitions and Legacy (sub-folders, Video Effects
and Legacy sorted with Obsolete last; the accelerated / 32-bit / YUV badges). Double-click applies
to the selected clips; a row dragged onto a Timeline clip applies it to that clip (a transition to
the nearer edit). When the panel is wide, the Lumetri Presets folder clicked last shows as a
thumbnail grid beside the tree.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QMimeData, QModelIndex, QPoint, QRect, QSettings, QSize, Qt, Signal
from PySide6.QtGui import QColor, QDrag, QFont, QIcon, QImage, QPainter, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QHBoxLayout,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, tr

EFFECT_MIME = "application/x-cfa4-effect"  # an effect id, `preset:<name>` or `lumetri:<name>`
TOP_FOLDERS = [N_("Presets"), N_("Lumetri Presets"), N_("Audio Effects"), N_("Audio Transitions"), N_("Video Effects"), N_("Video Transitions"), N_("Legacy")]
WIDE_PX = 640  # wider than this: the Lumetri Presets grid beside the tree (egui's)
ROLE_ID = Qt.ItemDataRole.UserRole  # what a row applies (see EFFECT_MIME); a folder's key
ROLE_KIND = Qt.ItemDataRole.UserRole + 1  # "effect", "transition", "preset", "lumetri", "folder"
ROLE_BADGES = Qt.ItemDataRole.UserRole + 2


def dropped_effect(mime: QMimeData) -> str | None:
    if not mime.hasFormat(EFFECT_MIME):
        return None
    return bytes(mime.data(EFFECT_MIME).data()).decode() or None


def effect_mime(eid: str) -> QMimeData:
    m = QMimeData()
    m.setData(EFFECT_MIME, eid.encode())
    return m


def _icon(kind: str, accent: bool = False) -> QIcon:
    """egui's row icons: a box with "fx", a box with a diagonal (transition), a sparkle (preset)."""
    t = theme.current()
    pm = QPixmap(14, 14)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.setPen(QColor(t.accent if accent else t.text_dim))
    if kind == "preset":
        p.drawLine(7, 1, 7, 13)
        p.drawLine(1, 7, 13, 7)
        p.drawLine(3, 3, 11, 11)
        p.drawLine(11, 3, 3, 11)
    else:
        p.drawRoundedRect(1, 1, 12, 12, 2, 2)
        if kind == "transition":
            p.drawLine(1, 13, 13, 1)
        else:
            f = QFont(p.font())
            f.setPixelSize(7)
            p.setFont(f)
            p.drawText(QRect(1, 1, 12, 12), Qt.AlignmentFlag.AlignCenter, "fx")
    p.end()
    return QIcon(pm)


class BadgeDelegate(QStyledItemDelegate):
    """Paints an effect row's badges (accelerated / 32-bit / YUV) at the right, as egui's."""

    def paint(self, p: QPainter, opt: QStyleOptionViewItem, index: QModelIndex) -> None:
        super().paint(p, opt, index)
        badges = index.data(ROLE_BADGES)
        if not badges:
            return
        t = theme.current()
        p.save()
        f = QFont(opt.font)
        f.setPixelSize(9)
        p.setFont(f)
        x = opt.rect.right() - 8
        for label in badges:
            r = QRect(x - 20, opt.rect.top() + (opt.rect.height() - 14) // 2, 20, 14)
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(QColor(t.field_border))
            p.drawRoundedRect(r, 2, 2)
            p.setPen(QColor(t.text_dim))
            p.drawText(r, Qt.AlignmentFlag.AlignCenter, label)
            x -= 24
        p.restore()


class EffectTree(QTreeWidget):
    """The tree; a leaf row drags its effect (EFFECT_MIME)."""

    def startDrag(self, _actions: Any) -> None:
        it = self.currentItem()
        if it is None or it.data(0, ROLE_KIND) == "folder":
            return
        drag = QDrag(self)
        drag.setMimeData(effect_mime(str(it.data(0, ROLE_ID))))
        drag.exec(Qt.DropAction.CopyAction)


class PresetGrid(QListWidget):
    """The wide panel's Lumetri Presets thumbnail grid."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("effects.presetGrid")
        self.setViewMode(QListWidget.ViewMode.IconMode)
        self.setIconSize(QSize(150, 84))
        self.setGridSize(QSize(158, 112))
        self.setResizeMode(QListWidget.ResizeMode.Adjust)
        self.setMovement(QListWidget.Movement.Static)
        self.setWordWrap(True)
        self.setDragEnabled(True)
        self.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)

    def startDrag(self, _actions: Any) -> None:
        it = self.currentItem()
        if it is not None:
            drag = QDrag(self)
            drag.setMimeData(effect_mime(str(it.data(ROLE_ID))))
            drag.exec(Qt.DropAction.CopyAction)


class EffectsPanel(QWidget):
    command = Signal(str, dict)
    run = Signal(str, dict)  # a command that needs a file (Export Preset…, Import Presets…)

    def __init__(self, engine: Any = None, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Effects")
        self.engine = engine
        self.settings: QSettings | None = None
        self.effects: list[dict[str, Any]] = []
        self.presets: list[dict[str, Any]] = []
        self.lumetri: dict[str, Any] = {}
        self.expanded: set[str] = set()
        self.grid_folder: str | None = None
        self.thumbs: dict[tuple[str, int], QIcon] = {}
        self._asked: set[tuple[str, int]] = set()
        self.search = QLineEdit(self)
        self.search.setObjectName("effects.search")
        self.search.setPlaceholderText(tr("Search effects"))
        self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(lambda _t: self._rebuild())
        self.tree = EffectTree(self)
        self.tree.setObjectName("effects.tree")
        self.tree.setHeaderHidden(True)
        self.tree.setIconSize(QSize(32, 18))  # a Lumetri preset's thumbnail; the 14 px icons stay 14 px
        self.tree.setItemDelegate(BadgeDelegate(self.tree))
        self.tree.setDragEnabled(True)
        self.tree.setIndentation(16)
        self.tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(self._menu)
        self.tree.itemDoubleClicked.connect(lambda it, _c: self._apply(it.data(0, ROLE_ID), it.data(0, ROLE_KIND)))
        self.tree.itemExpanded.connect(lambda it: self._expanded(it, True))
        self.tree.itemCollapsed.connect(lambda it: self._expanded(it, False))
        self.grid = PresetGrid(self)
        self.grid.itemDoubleClicked.connect(lambda it: self._apply(it.data(ROLE_ID), "lumetri"))
        self.grid.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.grid.customContextMenuRequested.connect(self._grid_menu)
        self.grid.hide()
        body = QHBoxLayout()
        body.setContentsMargins(0, 0, 0, 0)
        body.addWidget(self.tree, 1)
        body.addWidget(self.grid, 1)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 6, 2, 2)
        layout.addWidget(self.search)
        layout.addLayout(body, 1)
        self._building = False

    def attach_settings(self, settings: QSettings) -> None:
        self.settings = settings
        raw = settings.value("effects/expanded", [])
        self.expanded = {str(s) for s in (raw if isinstance(raw, list) else [raw] if raw else [])}
        self._rebuild()

    # ----- data -----
    def set_effects(self, effects: Any) -> None:
        self.effects = []
        for e in effects if isinstance(effects, list) else []:
            if isinstance(e, dict):
                cat = e.get("category") or [x for x in str(e.get("path") or "").split("/") if x]
                self.effects.append({**e, "category": cat, "folder": e.get("folder", cat[1] if len(cat) > 1 else None)})
        self._rebuild()

    def refresh(self) -> None:
        """Effect presets and the Lumetri presets from the engine (presets change on import / delete)."""
        if self.engine is None or not getattr(self.engine, "running", False):
            return

        def got_presets(r: Any, _b: bytes | None) -> None:
            presets = [p for p in (r or {}).get("presets") or [] if isinstance(p, dict)] if isinstance(r, dict) else []
            if presets != self.presets:
                self.presets = presets
                self._rebuild()

        def got_lumetri(r: Any, _b: bytes | None) -> None:
            if isinstance(r, dict) and r != self.lumetri:
                self.lumetri = r
                self._rebuild()

        self.engine.execute("presets.list", {}, on_reply=got_presets, on_error=lambda _m: None)
        if not self.lumetri:
            self.engine.execute("lumetri.presets", {}, on_reply=got_lumetri, on_error=lambda _m: None)

    def thumb(self, name: str, width: int) -> QIcon | None:
        """A Lumetri preset's thumbnail; asked of the engine once, rows updated when it comes."""
        key = (name, width)
        if key in self.thumbs:
            return self.thumbs[key]
        if key in self._asked or self.engine is None or not getattr(self.engine, "running", False):
            return None
        self._asked.add(key)

        def got(r: Any, data: bytes | None) -> None:
            if not isinstance(r, dict) or not data:
                return
            w, h = int(r.get("width") or 0), int(r.get("height") or 0)
            if w * h * 4 != len(data):
                return
            img = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
            self.thumbs[key] = QIcon(QPixmap.fromImage(img))
            self._set_thumbs()

        self.engine.call("lumetri.presetThumbnail", {"name": name, "width": width}, on_reply=got, on_error=lambda _m: None)
        return None

    def _set_thumbs(self) -> None:
        def walk(it: QTreeWidgetItem) -> None:
            if it.data(0, ROLE_KIND) == "lumetri":
                icon = self.thumbs.get((str(it.data(0, ROLE_ID))[len("lumetri:"):], 64))
                if icon is not None:
                    it.setIcon(0, icon)
            for i in range(it.childCount()):
                walk(it.child(i))

        for i in range(self.tree.topLevelItemCount()):
            walk(self.tree.topLevelItem(i))
        for i in range(self.grid.count()):
            it = self.grid.item(i)
            icon = self.thumbs.get((str(it.data(ROLE_ID))[len("lumetri:"):], 160))
            if icon is not None:
                it.setIcon(icon)

    # ----- the tree -----
    def _folder(self, parent: QTreeWidgetItem | None, key: str, label: str) -> QTreeWidgetItem:
        it = QTreeWidgetItem([tr(label)])
        it.setData(0, ROLE_ID, key)
        it.setData(0, ROLE_KIND, "folder")
        it.setIcon(0, QIcon(self.style().standardIcon(self.style().StandardPixmap.SP_DirIcon).pixmap(14, 14)))  # not scaled up to a thumbnail
        if parent is None:
            self.tree.addTopLevelItem(it)
        else:
            parent.addChild(it)
        return it

    def _leaf(self, parent: QTreeWidgetItem, eid: str, kind: str, label: str, tip: str = "") -> QTreeWidgetItem:
        it = QTreeWidgetItem([tr(label)])
        it.setData(0, ROLE_ID, eid)
        it.setData(0, ROLE_KIND, kind)
        if tip:
            it.setToolTip(0, tr(tip))
        parent.addChild(it)
        return it

    def _rebuild(self) -> None:
        q = self.search.text().strip().lower()
        match = lambda name: not q or q in str(name).lower() or q in tr(str(name)).lower()
        self._building = True
        self.tree.setUpdatesEnabled(False)
        self.tree.clear()
        folders: list[QTreeWidgetItem] = []
        for top in TOP_FOLDERS:
            if top == "Presets":
                rows = [p for p in self.presets if match(p.get("name"))]
                if not rows:
                    continue
                f = self._folder(None, top, top)
                folders.append(f)
                for p in rows:
                    name = str(p.get("name"))
                    it = self._leaf(f, f"preset:{name}", "preset", name, str(p.get("description") or name))
                    it.setIcon(0, _icon("preset", accent=not p.get("builtin")))
                    it.setData(0, ROLE_BADGES, None)
                continue
            if top == "Lumetri Presets":
                presets = [p for p in self.lumetri.get("presets") or [] if isinstance(p, dict) and match(p.get("name"))]
                if not presets:
                    continue
                f = self._folder(None, top, top)
                folders.append(f)
                for sub in self.lumetri.get("folders") or []:
                    items = [p for p in presets if p.get("folder") == sub]
                    if not items:
                        continue
                    s = self._folder(f, f"{top}/{sub}", str(sub))
                    folders.append(s)
                    for p in items:
                        name = str(p.get("name"))
                        it = self._leaf(s, f"lumetri:{name}", "lumetri", name, str(p.get("description") or ""))
                        icon = self.thumb(name, 64)
                        if icon is not None:
                            it.setIcon(0, icon)
                        it.setSizeHint(0, QSize(0, 24))
                continue
            items = [e for e in self.effects if (e.get("category") or [None])[0] == top and match(e.get("name"))]
            if not items:
                continue
            if top in ("Video Effects", "Legacy"):  # Premiere's alphabetical order, Obsolete last
                items.sort(key=lambda e: ((e.get("folder") or "") == "Obsolete", str(e.get("folder") or ""), str(e.get("name") or "").lower()))
            f = self._folder(None, top, top)
            folders.append(f)
            subs: list[str] = []
            for e in items:
                s = str(e.get("folder") or "")
                if s not in subs:
                    subs.append(s)
            subs.sort(key=lambda s: s == "")  # loose items (Balance, Mute, Volume) last
            for s in subs:
                parent = f
                if s:
                    parent = self._folder(f, f"{top}/{s}", s)
                    folders.append(parent)
                for e in (e for e in items if str(e.get("folder") or "") == s):
                    kind = "transition" if "Transition" in str(e.get("kind") or "") else "effect"
                    it = self._leaf(parent, str(e.get("id")), kind, str(e.get("name") or e.get("id")))
                    it.setIcon(0, _icon(kind))
                    b = e.get("badges") or {}
                    it.setData(0, ROLE_BADGES, [label for on, label in ((b.get("yuv"), "YUV"), (b.get("float32"), "32"), (b.get("accelerated"), "⚡")) if on])
        for f in folders:
            f.setExpanded(bool(q) or str(f.data(0, ROLE_ID)) in self.expanded)
        self.tree.setUpdatesEnabled(True)
        self._building = False
        self._show_grid()

    def _expanded(self, it: QTreeWidgetItem, on: bool) -> None:
        if self._building or self.search.text().strip():
            return
        key = str(it.data(0, ROLE_ID))
        self.expanded = self.expanded | {key} if on else self.expanded - {key}
        if key.startswith("Lumetri Presets/"):
            self.grid_folder = key.split("/", 1)[1]  # the folder clicked last (egui's)
            self._show_grid()
        if self.settings is not None:
            self.settings.setValue("effects/expanded", sorted(self.expanded))

    # ----- the wide panel's grid -----
    def resizeEvent(self, e: Any) -> None:
        super().resizeEvent(e)
        self._show_grid()

    def _show_grid(self) -> None:
        wide = self.width() > WIDE_PX and self.grid_folder is not None
        self.grid.setVisible(wide)
        self.tree.setMaximumWidth(300 if wide else 16_777_215)
        if not wide:
            return
        names = [str(p.get("name")) for p in self.lumetri.get("presets") or [] if isinstance(p, dict) and p.get("folder") == self.grid_folder]
        if [self.grid.item(i).data(ROLE_ID) for i in range(self.grid.count())] == [f"lumetri:{n}" for n in names]:
            return
        self.grid.clear()
        tips = {str(p.get("name")): str(p.get("description") or "") for p in self.lumetri.get("presets") or [] if isinstance(p, dict)}
        for n in names:
            it = QListWidgetItem(tr(n))
            it.setData(ROLE_ID, f"lumetri:{n}")
            it.setToolTip(tr(tips.get(n, "")) if tips.get(n) else "")
            icon = self.thumb(n, 160)
            if icon is not None:
                it.setIcon(icon)
            self.grid.addItem(it)

    # ----- actions -----
    def _apply(self, eid: Any, kind: Any) -> None:
        if not eid or kind == "folder":
            return
        eid = str(eid)
        if kind == "preset":
            self.command.emit("presets.apply", {"preset": eid[len("preset:"):]})
        elif kind == "lumetri":
            self.command.emit("lumetri.applyPreset", {"name": eid[len("lumetri:"):]})
        else:
            self.command.emit("effects.apply", {"effect": eid})

    def _menu(self, pos: QPoint) -> None:
        it = self.tree.itemAt(pos)
        if it is None:
            return
        kind = it.data(0, ROLE_KIND)
        eid = str(it.data(0, ROLE_ID))
        menu = QMenu(self)
        if kind == "lumetri":
            menu.addAction(tr("Apply to Selected Clips"), lambda: self._apply(eid, kind)).setObjectName("effects.presetMenu.apply")
        elif kind == "preset":
            name = eid[len("preset:"):]
            builtin = next((bool(p.get("builtin")) for p in self.presets if p.get("name") == name), True)
            if not builtin:
                menu.addAction(tr("Delete Preset"), lambda: self.command.emit("presets.delete", {"name": name}))
            menu.addAction(tr("Export Preset…"), lambda: self.run.emit("presets.export", {"names": [name]}))
            menu.addAction(tr("Import Presets…"), lambda: self.run.emit("presets.import", {}))
        if not menu.isEmpty():
            menu.exec(self.tree.viewport().mapToGlobal(pos))

    def _grid_menu(self, pos: QPoint) -> None:
        it = self.grid.itemAt(pos)
        if it is not None:
            menu = QMenu(self)
            eid = str(it.data(ROLE_ID))
            menu.addAction(tr("Apply to Selected Clips"), lambda: self._apply(eid, "lumetri"))
            menu.exec(self.grid.viewport().mapToGlobal(pos))


__all__ = ["EFFECT_MIME", "EffectsPanel", "dropped_effect", "effect_mime"]
