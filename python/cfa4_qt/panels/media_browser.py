"""Media Browser, as egui's (crates/ui-egui/src/panels/media_browser.rs): the locations tree
(Favorites, Local Drives, Network, Recent Directories) beside the files of the current directory
in List or Thumbnail view (hover scrub), back / forward / up, the path field, File Types
Displayed, Ingest and Import as Image Sequence, Edit Columns…, the panel menu, Import / Open In
Source Monitor, and dragging files to the Project panel or the Timeline.

The engine lists, probes and renders (``mediaBrowser.inspect``, ``mediaBrowser.thumbnail``) and
every action is an engine command (``mediaBrowser.navigate``, ``.select``, ``.favorite``,
``.clearRecent``, ``.settings``, ``.import``, ``.openInSource``, ``project.ingestSettings``,
``prefs.set``); the panel keeps only view state (expanded folders, tree width).
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QEvent, QMimeData, QPoint, QSettings, QSize, Qt, QTimer, QUrl, Signal
from PySide6.QtGui import QAction, QColor, QDesktopServices, QIcon, QImage, QKeyEvent, QMouseEvent, QPainter, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QSlider,
    QSplitter,
    QStackedWidget,
    QStyle,
    QToolButton,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, fmt, tr

FILES_MIME = "application/x-cfa4-files"  # newline-separated paths (Media Browser drags)
PATH_ROLE = Qt.ItemDataRole.UserRole
ENTRY_ROLE = Qt.ItemDataRole.UserRole + 1
SECTION_ROLE = Qt.ItemDataRole.UserRole + 2
POLL_MS = 3000  # egui re-lists every 3 s
PROBE_AGAIN_MS = 150
FOLDER = QColor(237, 150, 58)
SECTIONS = (
    ("favorites", N_("Favorites")),
    ("localDrives", N_("Local Drives")),
    ("network", N_("Network")),
    ("recent", N_("Recent Directories")),
)
COLUMN_LABELS = (
    N_("Name"),
    N_("Media Type"),
    N_("Size"),
    N_("Date Modified"),
    N_("Frame Rate"),
    N_("Media Duration"),
    N_("Video Info"),
    N_("Audio Info"),
    N_("Video Codec"),
    N_("Audio Codec"),
)
FILE_TYPE_LABELS = (
    N_("All Supported Files"),
    N_("Video Files"),
    N_("Audio Files"),
    N_("Still Images"),
    N_("Projects and Interchange"),
    N_("Captions"),
)


def files_mime(paths: list[str]) -> QMimeData:
    """A drag of files: our own list of paths, and file URLs for other applications."""
    mime = QMimeData()
    mime.setData(FILES_MIME, "\n".join(paths).encode())
    mime.setUrls([QUrl.fromLocalFile(p) for p in paths])
    return mime


def dropped_files(mime: QMimeData) -> list[str]:
    """Paths dropped from the Media Browser or a file manager."""
    if mime.hasFormat(FILES_MIME):
        return [p for p in bytes(mime.data(FILES_MIME)).decode(errors="replace").split("\n") if p]
    return [u.toLocalFile() for u in mime.urls() if u.isLocalFile()]


def kind_icon(widget: QWidget, kind: str) -> QIcon:
    sp = {
        "folder": QStyle.StandardPixmap.SP_DirIcon,
        "audio": QStyle.StandardPixmap.SP_MediaVolume,
        "video": QStyle.StandardPixmap.SP_MediaPlay,
        "image": QStyle.StandardPixmap.SP_FileIcon,
    }.get(kind, QStyle.StandardPixmap.SP_FileIcon)
    return widget.style().standardIcon(sp)


class PathField(QLineEdit):
    """The path field: Enter goes to the typed directory (Enter is also an editor shortcut,
    Render Effects In to Out), Escape shows the current one again."""

    def event(self, e: QEvent) -> bool:
        if e.type() == QEvent.Type.ShortcutOverride and isinstance(e, QKeyEvent) and e.key() in (
            Qt.Key.Key_Return,
            Qt.Key.Key_Enter,
            Qt.Key.Key_Escape,
        ):
            e.accept()
            return True
        return super().event(e)

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape:
            self.clearFocus()
            return
        super().keyPressEvent(e)


class FileList(QTreeWidget):
    def mimeData(self, items: list[QTreeWidgetItem]) -> QMimeData:
        return files_mime([str(i.data(0, PATH_ROLE)) for i in items if not (i.data(0, ENTRY_ROLE) or {}).get("isDir")])

    def mimeTypes(self) -> list[str]:
        return [FILES_MIME]


class Thumbnails(QListWidget):
    """Thumbnail View; reports the pointer's fraction across the thumbnail under it (hover scrub)."""

    hovered = Signal(object, float)  # QListWidgetItem or None, fraction

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setMouseTracking(True)
        self.viewport().setMouseTracking(True)

    def mimeData(self, items: list[QListWidgetItem]) -> QMimeData:
        return files_mime([str(i.data(PATH_ROLE)) for i in items if not (i.data(ENTRY_ROLE) or {}).get("isDir")])

    def mimeTypes(self) -> list[str]:
        return [FILES_MIME]

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        super().mouseMoveEvent(e)
        item = self.itemAt(e.position().toPoint())
        if item is None:
            self.hovered.emit(None, 0.3)
            return
        r = self.visualItemRect(item)
        icon = self.iconSize()
        left = r.center().x() - icon.width() / 2
        self.hovered.emit(item, max(0.0, min(1.0, (e.position().x() - left) / max(1, icon.width()))))

    def leaveEvent(self, e: Any) -> None:
        super().leaveEvent(e)
        self.hovered.emit(None, 0.3)


class EditColumnsDialog(QDialog):
    """egui's Edit Columns: a checkbox per column (Name stays), ▲ / ▼ to order them."""

    def __init__(self, columns: list[str], all_columns: list[str], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("mediaBrowser.columns")
        self.setWindowTitle(tr("Edit Columns"))
        self.columns = list(columns)
        self.all_columns = all_columns
        self.rows = QVBoxLayout()
        v = QVBoxLayout(self)
        v.addLayout(self.rows)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        v.addWidget(buttons)
        self._fill()

    def _fill(self) -> None:
        while self.rows.count():
            item = self.rows.takeAt(0)
            w = item.widget() if item is not None else None
            if w is not None:
                w.deleteLater()
        for c in self.all_columns:
            row = QWidget(self)
            h = QHBoxLayout(row)
            h.setContentsMargins(0, 0, 0, 0)
            box = QCheckBox(tr(c), row)
            box.setObjectName(f"mediaBrowser.columns.{c}")
            box.setChecked(c in self.columns)
            box.setEnabled(c != "Name")
            box.toggled.connect(lambda on, col=c: self._toggle(col, on))
            h.addWidget(box, 1)
            i = self.columns.index(c) if c in self.columns else -1
            for text, d, ok in (("▲", -1, i > 1), ("▼", 1, 0 < i < len(self.columns) - 1)):
                b = QToolButton(row)
                b.setText(text)
                b.setEnabled(ok)
                b.setVisible(i > 0)
                b.clicked.connect(lambda _c=False, at=i, step=d: self._move(at, step))
                h.addWidget(b)
            self.rows.addWidget(row)

    def _toggle(self, column: str, on: bool) -> None:
        if on and column not in self.columns:
            self.columns.append(column)
        elif not on:
            self.columns = [c for c in self.columns if c != column]
        QTimer.singleShot(0, self._fill)

    def _move(self, i: int, d: int) -> None:
        j = i + d
        if 0 < i < len(self.columns) and 0 < j < len(self.columns):
            self.columns[i], self.columns[j] = self.columns[j], self.columns[i]
            QTimer.singleShot(0, self._fill)


class MediaBrowserPanel(QWidget):
    command = Signal(str, dict)
    invoke = Signal(str)  # a command run through the GUI's dialog path (Ingest Settings)

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.MediaBrowser")
        self.engine = engine
        self.settings: QSettings | None = None
        self.data: dict[str, Any] = {}
        self.roots: dict[str, Any] = {}
        self.expanded: set[str] = set()
        self.thumbs: dict[tuple[str, int, int], QPixmap] = {}
        self._thumbs_busy = 0
        self._wanted: list[tuple[str, float, int]] = []
        self._busy = False
        self._again = False
        self._hover: tuple[str, float] | None = None
        self._updating = False

        # ---- toolbar: back, forward, up, path, file types, view, panel menu
        bar = QHBoxLayout()
        bar.setContentsMargins(6, 4, 6, 0)
        bar.setSpacing(2)
        self.nav: dict[str, QToolButton] = {}
        for key, text, tip in (("back", "‹", N_("Go Back")), ("forward", "›", N_("Go Forward")), ("up", "↑", N_("Up One Level"))):
            b = QToolButton(self)
            b.setObjectName(f"mediaBrowser.{key}")
            b.setText(text)
            b.setToolTip(tr(tip))
            b.setAutoRaise(True)
            b.clicked.connect(lambda _c=False, k=key: self.navigate({k: True}))
            bar.addWidget(b)
            self.nav[key] = b
        self.path = PathField(self)
        self.path.setObjectName("mediaBrowser.path")
        self.path.returnPressed.connect(self._path_entered)
        bar.addWidget(self.path, 1)
        self.types = QToolButton(self)
        self.types.setObjectName("mediaBrowser.fileTypes")
        self.types.setToolTip(tr("File Types Displayed"))
        self.types.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.types.setMenu(QMenu(self.types))
        self.types.menu().aboutToShow.connect(self._types_menu)
        bar.addWidget(self.types)
        self.view_buttons: dict[str, QToolButton] = {}
        for key, text, tip in (("list", "☰", N_("List View")), ("thumbnails", "▦", N_("Thumbnail View"))):
            b = QToolButton(self)
            b.setObjectName(f"mediaBrowser.view.{key}")
            b.setText(text)
            b.setToolTip(tr(tip))
            b.setCheckable(True)
            b.setAutoRaise(True)
            b.clicked.connect(lambda _c=False, k=key: self.settings_cmd({"view": k}))
            bar.addWidget(b)
            self.view_buttons[key] = b
        self.menu_button = QToolButton(self)
        self.menu_button.setObjectName("mediaBrowser.menu")
        self.menu_button.setText("≡")
        self.menu_button.setAutoRaise(True)
        self.menu_button.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.menu_button.setMenu(QMenu(self.menu_button))
        self.menu_button.menu().aboutToShow.connect(self._panel_menu)
        bar.addWidget(self.menu_button)

        # ---- second row: Ingest and Import as Image Sequence
        row2 = QHBoxLayout()
        row2.setContentsMargins(6, 0, 6, 0)
        self.ingest = QCheckBox(tr("Ingest"), self)
        self.ingest.setObjectName("mediaBrowser.ingest")
        self.ingest.toggled.connect(lambda on: self._updating or self.send("project.ingestSettings", {"enabled": on}))
        gear = QToolButton(self)
        gear.setObjectName("mediaBrowser.ingestSettings")
        gear.setText("⚙")
        gear.setAutoRaise(True)
        gear.setToolTip(tr("Open Ingest Settings"))
        gear.clicked.connect(lambda: self.invoke.emit("project.ingestSettings"))
        self.image_sequence = QCheckBox(tr("Import as Image Sequence"), self)
        self.image_sequence.setObjectName("mediaBrowser.importAsImageSequence")
        self.image_sequence.toggled.connect(lambda on: self._updating or self.settings_cmd({"importAsImageSequence": on}))
        row2.addWidget(self.ingest)
        row2.addWidget(gear)
        row2.addSpacing(12)
        row2.addWidget(self.image_sequence)
        row2.addStretch(1)

        # ---- tree | files
        self.tree = QTreeWidget(self)
        self.tree.setObjectName("mediaBrowser.tree")
        self.tree.setHeaderHidden(True)
        self.tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(self._tree_menu)
        self.tree.itemClicked.connect(self._tree_clicked)
        self.tree.itemExpanded.connect(self._tree_expanded)
        self.tree.itemCollapsed.connect(lambda it: self.expanded.discard(str(it.data(0, PATH_ROLE))))
        self.list = FileList(self)
        self.list.setObjectName("mediaBrowser.list")
        self.list.setRootIsDecorated(False)
        self.list.setAlternatingRowColors(True)
        self.list.setUniformRowHeights(True)
        self.list.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.list.setDragEnabled(True)
        self.list.setDragDropMode(QAbstractItemView.DragDropMode.DragOnly)
        self.list.itemSelectionChanged.connect(self._selection_changed)
        self.list.itemDoubleClicked.connect(lambda it, _c: self.open_entry(it.data(0, ENTRY_ROLE) or {}))
        self.list.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.list.customContextMenuRequested.connect(lambda pos: self._entry_menu(self.list.itemAt(pos), self.list.viewport().mapToGlobal(pos)))
        self.list.header().setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.list.header().customContextMenuRequested.connect(self._header_menu)
        self.grid = Thumbnails(self)
        self.grid.setObjectName("mediaBrowser.thumbnails")
        self.grid.setViewMode(QListWidget.ViewMode.IconMode)
        self.grid.setResizeMode(QListWidget.ResizeMode.Adjust)
        self.grid.setMovement(QListWidget.Movement.Static)
        self.grid.setWordWrap(True)
        self.grid.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.grid.setDragEnabled(True)
        self.grid.setDragDropMode(QAbstractItemView.DragDropMode.DragOnly)
        self.grid.itemSelectionChanged.connect(self._selection_changed)
        self.grid.itemDoubleClicked.connect(lambda it: self.open_entry(it.data(ENTRY_ROLE) or {}))
        self.grid.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.grid.customContextMenuRequested.connect(lambda pos: self._entry_menu(self.grid.itemAt(pos), self.grid.viewport().mapToGlobal(pos)))
        self.grid.hovered.connect(self._hovered)
        self.grid.verticalScrollBar().valueChanged.connect(lambda _v: self._request_visible())
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.list)
        self.stack.addWidget(self.grid)
        self.message = QLabel(self)
        self.message.setObjectName("mediaBrowser.message")
        self.message.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.message.setWordWrap(True)
        self.stack.addWidget(self.message)
        self.split = QSplitter(Qt.Orientation.Horizontal, self)
        self.split.setObjectName("mediaBrowser.split")
        self.split.addWidget(self.tree)
        self.split.addWidget(self.stack)
        self.split.setStretchFactor(1, 1)
        self.split.setSizes([200, 500])

        # ---- footer: count, thumbnail size
        foot = QHBoxLayout()
        foot.setContentsMargins(6, 0, 6, 2)
        self.count = QLabel(self)
        self.count.setObjectName("mediaBrowser.count")
        self.count.setEnabled(False)
        self.size_slider = QSlider(Qt.Orientation.Horizontal, self)
        self.size_slider.setObjectName("mediaBrowser.thumbnailSize")
        self.size_slider.setRange(60, 320)
        self.size_slider.setFixedWidth(120)
        self.size_slider.valueChanged.connect(self._size_changed)
        self.size_slider.sliderReleased.connect(lambda: self.settings_cmd({"thumbnailSize": float(self.size_slider.value())}))
        foot.addWidget(self.count, 1)
        foot.addWidget(self.size_slider)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(4)
        layout.addLayout(bar)
        layout.addLayout(row2)
        layout.addWidget(self.split, 1)
        layout.addLayout(foot)

        self.poll = QTimer(self, interval=POLL_MS)
        self.poll.timeout.connect(self.refresh)
        self._probe_timer = QTimer(self, singleShot=True, interval=PROBE_AGAIN_MS)
        self._probe_timer.timeout.connect(self.refresh)

    def attach_settings(self, settings: QSettings) -> None:
        self.settings = settings
        saved = settings.value("mediaBrowser/expanded")
        if isinstance(saved, str):
            self.expanded = {p for p in saved.split("\n") if p}
        sizes = settings.value("mediaBrowser/split")
        if isinstance(sizes, str) and "," in sizes:
            try:
                self.split.setSizes([int(x) for x in sizes.split(",")])
            except ValueError:
                pass
        self.split.splitterMoved.connect(lambda _p, _i: settings.setValue("mediaBrowser/split", ",".join(str(s) for s in self.split.sizes())))

    # ----- engine -> panel -----
    def showEvent(self, e: Any) -> None:
        super().showEvent(e)
        self.poll.start()
        self.refresh()

    def hideEvent(self, e: Any) -> None:
        super().hideEvent(e)
        self.poll.stop()

    def refresh(self) -> None:
        """Ask the engine for the directory and the locations (only while shown)."""
        if self.engine is None or not getattr(self.engine, "running", False) or not self.isVisible():
            return
        if self._busy:
            self._again = True
            return
        self._busy = True

        def done(result: Any, _b: bytes | None) -> None:
            self._busy = False
            self.set_state(result if isinstance(result, dict) else {})
            if self._again:
                self._again = False
                self.refresh()

        def failed(m: str) -> None:
            self._busy = False
            self.set_state({"error": m, "entries": []})

        self.engine.call("mediaBrowser.inspect", {}, on_reply=done, on_error=failed)
        self.engine.execute("mediaBrowser.roots", {}, on_reply=lambda r, _b: self.set_roots(r))

    def set_state(self, data: dict[str, Any]) -> None:
        old = self.data
        self.data = data
        self._updating = True
        try:
            d = data.get("dir")
            if isinstance(d, str) and not self.path.hasFocus():
                self.path.setText(d)
            for key, flag in (("back", "canBack"), ("forward", "canForward"), ("up", "canUp")):
                self.nav[key].setEnabled(bool(data.get(flag)))
            self.types.setText(self._types_label())
            view = "thumbnails" if data.get("view") == "thumbnails" else "list"
            for key, b in self.view_buttons.items():
                b.setChecked(key == view)
            self.ingest.setChecked(bool(data.get("ingest")))
            self.image_sequence.setChecked(bool(data.get("importAsImageSequence")))
            self.size_slider.setVisible(view == "thumbnails")
            if not self.size_slider.isSliderDown():
                self.size_slider.setValue(int(float(data.get("thumbnailSize") or 120)))
            entries = [e for e in data.get("entries") or [] if isinstance(e, dict)]
            if data.get("error"):
                self.message.setText(fmt("Cannot open {dir}: {e}", dir=str(data.get("dir") or ""), e=str(data.get("error"))))
                self.stack.setCurrentWidget(self.message)
            else:
                self.stack.setCurrentWidget(self.grid if view == "thumbnails" else self.list)
            shape = (old.get("dir"), old.get("columns"), [(e.get("path"), e.get("cells")) for e in old.get("entries") or []])
            if shape != (data.get("dir"), data.get("columns"), [(e.get("path"), e.get("cells")) for e in entries]):
                self._fill_list(entries)
                self._fill_grid(entries)
            self._show_selection()
            files = sum(1 for e in entries if not e.get("isDir"))
            sel = len(data.get("selection") or [])
            self.count.setText(fmt("{sel} of {total} items selected", sel=sel, total=files) if sel else fmt("{total} items", total=files))
            self._mark_current()
        finally:
            self._updating = False
        if int(data.get("pending") or 0) > 0:
            self._probe_timer.start()
        self._request_visible()

    def _types_label(self) -> str:
        ft = str(self.data.get("fileTypes") or "all")
        for k, label in self.data.get("fileTypeChoices") or []:
            if k == ft:
                return tr(str(label))
        return f".{ft}"

    def _fill_list(self, entries: list[dict[str, Any]]) -> None:
        columns = [str(c) for c in self.data.get("columns") or ["Name"]]
        self.list.blockSignals(True)
        self.list.clear()
        self.list.setColumnCount(len(columns))
        self.list.setHeaderLabels([tr(c) for c in columns])
        for i, c in enumerate(columns):
            self.list.setColumnWidth(i, 220 if c == "Name" else 110)
        dim = theme.current().c("text_dim")
        for e in entries:
            cells = [str(c) for c in e.get("cells") or []]
            it = QTreeWidgetItem([*cells, *[""] * (len(columns) - len(cells))])
            it.setData(0, PATH_ROLE, e.get("path"))
            it.setData(0, ENTRY_ROLE, e)
            it.setIcon(0, kind_icon(self, "folder" if e.get("isDir") else str(e.get("kind"))))
            if e.get("isDir"):
                it.setForeground(0, FOLDER)
            for i in range(1, len(columns)):
                it.setForeground(i, dim)
            self.list.addTopLevelItem(it)
        self.list.blockSignals(False)

    def _fill_grid(self, entries: list[dict[str, Any]]) -> None:
        size = int(float(self.data.get("thumbnailSize") or 120))
        self.grid.blockSignals(True)
        self.grid.clear()
        self.grid.setIconSize(QSize(size, size * 9 // 16))
        self.grid.setGridSize(QSize(size + 16, size * 9 // 16 + 30))
        for e in entries:
            it = QListWidgetItem(str(e.get("name") or ""))
            it.setData(PATH_ROLE, e.get("path"))
            it.setData(ENTRY_ROLE, e)
            it.setIcon(self._placeholder(e))
            self.grid.addItem(it)
        self.grid.blockSignals(False)

    def _placeholder(self, e: dict[str, Any]) -> QIcon:
        size = self.grid.iconSize()
        pm = QPixmap(size)
        pm.fill(QColor(14, 14, 14))
        p = QPainter(pm)
        icon = kind_icon(self, "folder" if e.get("isDir") else str(e.get("kind")))
        edge = min(size.height() * 6 // 10, 48) if e.get("isDir") else 26
        icon.paint(p, (size.width() - edge) // 2, (size.height() - edge) // 2, edge, edge)
        p.end()
        return QIcon(pm)

    def _show_selection(self) -> None:
        sel = set(self.data.get("selection") or [])
        for view_items in (
            [self.list.topLevelItem(i) for i in range(self.list.topLevelItemCount())],
            [self.grid.item(i) for i in range(self.grid.count())],
        ):
            for it in view_items:
                if it is None:
                    continue
                path = it.data(0, PATH_ROLE) if isinstance(it, QTreeWidgetItem) else it.data(PATH_ROLE)
                want = path in sel
                if it.isSelected() != want:
                    blocker = self.list if isinstance(it, QTreeWidgetItem) else self.grid
                    blocker.blockSignals(True)
                    it.setSelected(want)
                    blocker.blockSignals(False)

    def set_roots(self, roots: Any) -> None:
        self.roots = roots if isinstance(roots, dict) else {}
        open_paths = set(self.expanded)
        self.tree.blockSignals(True)
        self.tree.clear()
        for key, title in SECTIONS:
            head = QTreeWidgetItem([tr(title)])
            head.setData(0, SECTION_ROLE, key)
            head.setFlags(Qt.ItemFlag.ItemIsEnabled)
            font = head.font(0)
            font.setBold(True)
            head.setFont(0, font)
            head.setForeground(0, theme.current().c("text_dim"))
            self.tree.addTopLevelItem(head)
            places = [r for r in self.roots.get(key) or [] if isinstance(r, dict) and r.get("path")]
            if not places:
                none = QTreeWidgetItem([tr("(none)")])
                none.setFlags(Qt.ItemFlag.NoItemFlags)
                head.addChild(none)
            for r in places:
                self._node(head, str(r.get("name") or r.get("path")), str(r["path"]), key, expandable=key != "recent")
            head.setExpanded(True)
        self.tree.blockSignals(False)
        for path in open_paths:
            for it in self._tree_items():
                if it.data(0, PATH_ROLE) == path and it.childCount():
                    it.setExpanded(True)
                    self._tree_expanded(it)
        self._mark_current()

    def _node(self, parent: QTreeWidgetItem, name: str, path: str, section: str, expandable: bool) -> QTreeWidgetItem:
        it = QTreeWidgetItem([name])
        it.setData(0, PATH_ROLE, path)
        it.setData(0, SECTION_ROLE, section)
        it.setToolTip(0, path)
        it.setIcon(0, kind_icon(self, "folder"))
        if expandable:
            it.setChildIndicatorPolicy(QTreeWidgetItem.ChildIndicatorPolicy.ShowIndicator)
        parent.addChild(it)
        return it

    def _tree_items(self) -> list[QTreeWidgetItem]:
        out: list[QTreeWidgetItem] = []

        def walk(it: QTreeWidgetItem) -> None:
            out.append(it)
            for i in range(it.childCount()):
                child = it.child(i)
                if child is not None:
                    walk(child)

        for i in range(self.tree.topLevelItemCount()):
            top = self.tree.topLevelItem(i)
            if top is not None:
                walk(top)
        return out

    def _mark_current(self) -> None:
        cur = self.data.get("dir")
        self.tree.blockSignals(True)
        for it in self._tree_items():
            it.setSelected(cur is not None and it.data(0, PATH_ROLE) == cur)
        self.tree.blockSignals(False)

    def _tree_expanded(self, it: QTreeWidgetItem) -> None:
        path = it.data(0, PATH_ROLE)
        if not isinstance(path, str):
            return
        self.expanded.add(path)
        self._save_expanded()
        section = str(it.data(0, SECTION_ROLE) or "")

        def got(result: Any, _b: bytes | None, item: QTreeWidgetItem = it) -> None:
            try:
                item.takeChildren()
            except RuntimeError:  # the tree was rebuilt meanwhile
                return
            dirs = [e for e in (result or {}).get("entries") or [] if isinstance(e, dict) and e.get("isDir")]
            if not dirs:
                item.setChildIndicatorPolicy(QTreeWidgetItem.ChildIndicatorPolicy.DontShowIndicator)
            for e in dirs:
                child = self._node(item, str(e.get("name")), str(e.get("path")), section, expandable=True)
                if child.data(0, PATH_ROLE) in self.expanded:
                    child.setExpanded(True)
            self._mark_current()

        if self.engine is not None and getattr(self.engine, "running", False):
            self.engine.execute("mediaBrowser.list", {"path": path, "fileTypes": "all"}, on_reply=got)

    def _save_expanded(self) -> None:
        if self.settings is not None:
            self.settings.setValue("mediaBrowser/expanded", "\n".join(sorted(self.expanded)))

    # ----- thumbnails -----
    def _bucket(self, frac: float) -> int:
        return round(frac * 40)

    def _request_visible(self) -> None:
        if self.stack.currentWidget() is not self.grid:
            return
        width = self.grid.iconSize().width()
        view = self.grid.viewport().rect()
        for i in range(self.grid.count()):
            it = self.grid.item(i)
            if it is None or not self.grid.visualItemRect(it).intersects(view):
                continue
            e = it.data(ENTRY_ROLE) or {}
            if e.get("isDir") or e.get("kind") not in ("video", "image"):
                continue
            frac = self._hover[1] if self._hover and self._hover[0] == e.get("path") else 0.3
            self._want(str(e.get("path")), frac, width)
        self._pump()

    def _want(self, path: str, frac: float, width: int) -> None:
        key = (path, self._bucket(frac), width)
        if key in self.thumbs:
            self._show_thumb(path, key)
        elif all((p, self._bucket(f), w) != key for p, f, w in self._wanted):
            self._wanted.append((path, frac, width))

    def _pump(self) -> None:
        while self._wanted and self._thumbs_busy < 2 and self.engine is not None and getattr(self.engine, "running", False):
            path, frac, width = self._wanted.pop()
            key = (path, self._bucket(frac), width)
            self._thumbs_busy += 1

            def done(result: Any, data: bytes | None, k: tuple[str, int, int] = key) -> None:
                self._thumbs_busy -= 1
                w, h = int((result or {}).get("w") or 0), int((result or {}).get("h") or 0)
                if data and w > 0 and h > 0 and len(data) >= w * h * 4:
                    img = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
                    self.thumbs[k] = QPixmap.fromImage(img)
                    self._show_thumb(k[0], k)
                self._pump()

            def failed(_m: str, k: tuple[str, int, int] = key) -> None:
                self._thumbs_busy -= 1
                self.thumbs[k] = QPixmap()  # not a picture: keep the placeholder, don't ask again
                self._pump()

            self.engine.call("mediaBrowser.thumbnail", {"path": path, "fraction": frac, "width": width}, on_reply=done, on_error=failed)

    def _show_thumb(self, path: str, key: tuple[str, int, int]) -> None:
        pm = self.thumbs.get(key)
        if pm is None or pm.isNull():
            return
        want = self._bucket(self._hover[1]) if self._hover and self._hover[0] == path else self._bucket(0.3)
        if key[1] != want:
            return
        size = self.grid.iconSize()
        canvas = QPixmap(size)
        canvas.fill(QColor(14, 14, 14))
        p = QPainter(canvas)
        fitted = pm.scaled(size, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation)
        p.drawPixmap((size.width() - fitted.width()) // 2, (size.height() - fitted.height()) // 2, fitted)
        if self._hover and self._hover[0] == path:
            x = int(size.width() * self._hover[1])
            p.setPen(QColor(0x5A, 0x9B, 0xF0))
            p.drawLine(x, 0, x, size.height())
        p.end()
        for i in range(self.grid.count()):
            it = self.grid.item(i)
            if it is not None and it.data(PATH_ROLE) == path:
                it.setIcon(QIcon(canvas))

    def _hovered(self, item: QListWidgetItem | None, frac: float) -> None:
        if not self.data.get("hoverScrub"):
            return
        old = self._hover
        e = item.data(ENTRY_ROLE) if item is not None else None
        if not isinstance(e, dict) or e.get("isDir") or e.get("kind") not in ("video", "image"):
            self._hover = None
        else:
            self._hover = (str(e.get("path")), frac)
        if old and (self._hover is None or old[0] != self._hover[0]):
            self._want(old[0], 0.3, self.grid.iconSize().width())
        if self._hover:
            self._wanted = [w for w in self._wanted if w[0] != self._hover[0]]
            self._want(self._hover[0], self._hover[1], self.grid.iconSize().width())
        self._pump()

    def _size_changed(self, v: int) -> None:
        if self._updating:
            return
        self.grid.setIconSize(QSize(v, v * 9 // 16))
        self.grid.setGridSize(QSize(v + 16, v * 9 // 16 + 30))
        self._fill_grid([e for e in self.data.get("entries") or [] if isinstance(e, dict)])
        self._request_visible()

    # ----- panel -> engine -----
    def send(self, command: str, params: dict[str, Any]) -> None:
        self.command.emit(command, params)

    def settings_cmd(self, params: dict[str, Any]) -> None:
        self.send("mediaBrowser.settings", params)

    def navigate(self, params: dict[str, Any]) -> None:
        self.send("mediaBrowser.navigate", params)

    def _path_entered(self) -> None:
        text = self.path.text().strip()
        if text and text != self.data.get("dir"):
            self.navigate({"path": text})
        self.path.clearFocus()

    def _tree_clicked(self, it: QTreeWidgetItem, _col: int) -> None:
        path = it.data(0, PATH_ROLE)
        if isinstance(path, str):
            self.navigate({"path": path})

    def _selected_paths(self) -> list[str]:
        if self.stack.currentWidget() is self.grid:
            return [str(i.data(PATH_ROLE)) for i in self.grid.selectedItems()]
        return [str(i.data(0, PATH_ROLE)) for i in self.list.selectedItems()]

    def _selection_changed(self) -> None:
        if not self._updating:
            self.send("mediaBrowser.select", {"paths": self._selected_paths()})

    def open_entry(self, e: dict[str, Any]) -> None:
        """Double-click: a folder opens, a project or caption file is imported, media opens in
        the Source monitor (egui's)."""
        path = e.get("path")
        if not isinstance(path, str):
            return
        if e.get("isDir"):
            self.navigate({"path": path})
        elif e.get("kind") in ("project", "caption"):
            self.send("mediaBrowser.import", {"paths": [path]})
        else:
            self.send("mediaBrowser.openInSource", {"path": path})

    def _entry_menu(self, it: Any, at: QPoint) -> None:
        if it is None:
            return
        e = (it.data(0, ENTRY_ROLE) if isinstance(it, QTreeWidgetItem) else it.data(ENTRY_ROLE)) or {}
        path = str(e.get("path") or "")
        sel = list(self.data.get("selection") or [])
        paths = sel if path in sel else [path]
        menu = QMenu(self)

        def add(name: str, label: str, slot: Any) -> QAction:
            a = menu.addAction(tr(label))
            a.setObjectName(f"mediaBrowser.entryMenu.{name}")
            a.triggered.connect(slot)
            return a

        add("import", N_("Import"), lambda: self.send("mediaBrowser.import", {"paths": paths, "imageSequence": False}))
        if not e.get("isDir"):
            add("openInSource", N_("Open In Source Monitor"), lambda: self.send("mediaBrowser.openInSource", {"path": path}))
        if e.get("numbered"):
            add("importSequence", N_("Import as Image Sequence"), lambda: self.send("mediaBrowser.import", {"paths": [path], "imageSequence": True}))
        if e.get("isDir"):
            fav = any(r.get("path") == path for r in self.roots.get("favorites") or [] if isinstance(r, dict))
            add("favorite", N_("Remove from Favorites") if fav else N_("Add to Favorites"),
                lambda: self.send("mediaBrowser.favorite", {"path": path, "remove": fav}))
        add("reveal", N_("Show in File Manager"), lambda: self.reveal(path))
        menu.exec(at)

    def reveal(self, path: str) -> None:
        e = next((x for x in self.data.get("entries") or [] if x.get("path") == path), {})
        folder = path if e.get("isDir") else (str(self.data.get("dir") or "") or path)
        QDesktopServices.openUrl(QUrl.fromLocalFile(folder))

    def _tree_menu(self, pos: QPoint) -> None:
        it = self.tree.itemAt(pos)
        path = it.data(0, PATH_ROLE) if it is not None else None
        if not isinstance(path, str):
            return
        section = str(it.data(0, SECTION_ROLE) or "")
        fav = any(r.get("path") == path for r in self.roots.get("favorites") or [] if isinstance(r, dict))
        menu = QMenu(self)
        a = menu.addAction(tr("Remove from Favorites") if fav else tr("Add to Favorites"))
        a.setObjectName("mediaBrowser.treeMenu.favorite")
        a.triggered.connect(lambda: self.send("mediaBrowser.favorite", {"path": path, "remove": fav}))
        menu.addAction(tr("Import")).triggered.connect(lambda: self.send("mediaBrowser.import", {"paths": [path]}))
        if section == "recent":
            menu.addAction(tr("Clear Recent Directories")).triggered.connect(lambda: self.send("mediaBrowser.clearRecent", {}))
        menu.exec(self.tree.viewport().mapToGlobal(pos))

    def _header_menu(self, pos: QPoint) -> None:
        menu = QMenu(self)
        menu.addAction(tr("Edit Columns…")).triggered.connect(self.edit_columns)
        menu.exec(self.list.header().mapToGlobal(pos))

    def edit_columns(self) -> None:
        dialog = EditColumnsDialog([str(c) for c in self.data.get("columns") or []], [str(c) for c in self.data.get("allColumns") or COLUMN_LABELS], self)
        self.columns_dialog = dialog
        if dialog.exec() == QDialog.DialogCode.Accepted:
            self.settings_cmd({"columns": dialog.columns})

    def _types_menu(self) -> None:
        menu = self.types.menu()
        menu.clear()
        current = str(self.data.get("fileTypes") or "all")
        for k, label in self.data.get("fileTypeChoices") or []:
            a = menu.addAction(tr(str(label)))
            a.setObjectName(f"mediaBrowser.fileTypes.{k}")
            a.setCheckable(True)
            a.setChecked(k == current)
            a.triggered.connect(lambda _c=False, key=k: self.settings_cmd({"fileTypes": key}))
        menu.addSeparator()
        ext = menu.addMenu(tr("File Extension"))
        for e in self.data.get("extensions") or []:
            a = ext.addAction(f".{e}")
            a.setCheckable(True)
            a.setChecked(e == current)
            a.triggered.connect(lambda _c=False, key=e: self.settings_cmd({"fileTypes": key}))

    def _panel_menu(self) -> None:
        """egui's ≡ menu of the Media Browser."""
        menu = self.menu_button.menu()
        menu.clear()
        d = self.data
        has_sel = bool(d.get("selection"))

        def add(name: str, label: str, slot: Any, checked: bool | None = None, enabled: bool = True) -> None:
            a = menu.addAction(tr(label))
            a.setObjectName(f"mediaBrowser.menu.{name}")
            if checked is not None:
                a.setCheckable(True)
                a.setChecked(checked)
            a.setEnabled(enabled)
            a.triggered.connect(slot)

        add("editColumns", N_("Edit Columns…"), self.edit_columns)
        add("importAsImageSequence", N_("Import as Image Sequence"),
            lambda: self.settings_cmd({"importAsImageSequence": not d.get("importAsImageSequence")}), bool(d.get("importAsImageSequence")))
        add("hoverScrub", N_("Hover Scrub"), lambda: self.settings_cmd({"hoverScrub": not d.get("hoverScrub")}), bool(d.get("hoverScrub")))
        add("newPanel", N_("New Media Browser Panel"), lambda: None, enabled=False)
        add("createFolderForImportedProjects", N_("Create Folder For Imported Projects"),
            lambda: self.send("prefs.set", {"key": "media.createFolderForImportedProjects", "value": not d.get("createFolderForImportedProjects")}),
            bool(d.get("createFolderForImportedProjects")))
        add("allowDuplicateMedia", N_("Allow Duplicate Media During Project Import"),
            lambda: self.send("prefs.set", {"key": "media.allowDuplicateMedia", "value": not d.get("allowDuplicateMedia")}),
            bool(d.get("allowDuplicateMedia")))
        add("refresh", N_("Refresh"), self._refresh_all)
        menu.addSeparator()
        fav = bool(d.get("favorite"))
        add("favorite", N_("Remove from Favorites") if fav else N_("Add to Favorites"), lambda: self.send("mediaBrowser.favorite", {"remove": fav}))
        add("clearRecent", N_("Clear Recent Directories"), lambda: self.send("mediaBrowser.clearRecent", {}), enabled=bool(d.get("recent")))
        add("import", N_("Import"), lambda: self.send("mediaBrowser.import", {}), enabled=has_sel)
        add("openInSource", N_("Open In Source Monitor"), lambda: self.send("mediaBrowser.openInSource", {}), enabled=has_sel)

    def _refresh_all(self) -> None:
        """Refresh: list again and probe the files again (egui's)."""
        self.thumbs.clear()
        self.data = {}
        if self.engine is not None and getattr(self.engine, "running", False):
            self.engine.call("mediaBrowser.inspect", {"refresh": True}, on_reply=lambda r, _b: self.set_state(r if isinstance(r, dict) else {}))
