"""The Live Studio workspace (CFA4-LIVE-STUDIO-001 S1c, plan D5): scenes, sources and the program
picture of the live studio, in one window.

Everything shown here is the engine's: the scenes and their sources live in the project and are
read with ``live.studio.scenes``; every change is one engine command (``live.studio.scene.*``,
``live.studio.source.*``), so undo/redo, the control channel and agents see the same edits. The
picture is the S0 preview (``panels/live_studio.py``): the program frames of ``cfa4-studio-host``.
Selecting a scene puts it on the program (studio mode with a separate preview comes in S4).

Layout (D5): the program picture in the middle, the source's properties beside it, the scene list
and the source list of the current scene below.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QSignalBlocker, Qt, QTimer
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QCheckBox,
    QColorDialog,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QPushButton,
    QSpinBox,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import tr
from .live_studio import LiveStudioPreview

IMAGE_FILTER = "Pictures (*.png *.jpg *.jpeg *.webp *.tif *.tiff *.bmp *.gif *.exr *.dpx *.tga);;All files (*)"
VIDEO_FILTER = "Video (*.mp4 *.mov *.mkv *.webm *.avi *.mxf *.mpg *.mpeg *.ts *.m2ts);;All files (*)"
SOURCE_TYPES = ("color", "image", "text", "media", "liveGraphics")
FILE_TYPES = ("image", "media")  # sources that need a file before they can be added
REFRESH_DEBOUNCE_MS = 40
SCENE_ROLE = Qt.ItemDataRole.UserRole


def source_type_label(kind: str) -> str:
    """The user-facing name of a source type (``liveGraphics`` → "Live Graphics")."""
    return {
        "color": tr("Color"),
        "image": tr("Image"),
        "text": tr("Text"),
        "media": tr("Media File"),
        "liveGraphics": tr("Live Graphics"),
    }.get(kind, kind)


def as_dict(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def color_of(value: Any) -> QColor:
    """A source's ``[r, g, b, a]`` as a QColor (a bad value gives opaque white)."""
    if isinstance(value, list) and len(value) == 4 and all(isinstance(v, int) and 0 <= v <= 255 for v in value):
        return QColor(value[0], value[1], value[2], value[3])
    return QColor(255, 255, 255, 255)


def color_hex(color: QColor) -> str:
    """``#RRGGBBAA`` as the engine's colour parameter."""
    return f"#{color.red():02x}{color.green():02x}{color.blue():02x}{color.alpha():02x}"


def scenes_of(result: Any) -> tuple[list[dict[str, Any]], int | None]:
    """The scene list and the current scene id of a ``live.studio.scenes`` reply; a malformed reply
    gives no scenes (it is never shown half)."""
    if not isinstance(result, dict) or not isinstance(result.get("scenes"), list):
        return [], None
    scenes = [s for s in result["scenes"] if isinstance(s, dict) and isinstance(s.get("id"), int)]
    current = result.get("current")
    return scenes, current if isinstance(current, int) and not isinstance(current, bool) else None


def sources_of(scene: dict[str, Any] | None) -> list[dict[str, Any]]:
    """A scene's sources, top of the stack first (the engine lists the bottom first)."""
    if not scene or not isinstance(scene.get("sources"), list):
        return []
    return [s for s in reversed(scene["sources"]) if isinstance(s, dict) and isinstance(s.get("id"), int)]


class StudioWorkspace(QWidget):
    """Scenes, sources, properties and the program picture of the live studio."""

    def __init__(self, engine: EngineClient, parent: QWidget | None = None, auto_start: bool = True) -> None:
        super().__init__(parent)
        self.engine = engine
        self.auto_start = auto_start
        self.setObjectName("live_studio.workspace")
        self.setWindowTitle(tr("Live Studio"))
        self.scenes: list[dict[str, Any]] = []
        self.current_scene: int | None = None
        self.selected_source: int | None = None
        self._shown_source: tuple[int, str] | None = None  # (id, type) the property form was built for
        self._updating = False
        self._refresh_due = False
        self.error = ""

        self.preview = LiveStudioPreview(engine, self)
        self.preview.setObjectName("live_studio.preview")

        # ----- scenes -----
        self.scene_list = QListWidget(self)
        self.scene_list.setObjectName("live_studio.scenes")
        self.scene_list.currentItemChanged.connect(self._scene_chosen)
        self.scene_list.itemChanged.connect(self._scene_edited)
        self.scene_add = self._button("live_studio.scene.add", tr("Add"), self.add_scene)
        self.scene_remove = self._button("live_studio.scene.remove", tr("Remove"), self.remove_scene)
        self.scene_up = self._button("live_studio.scene.up", tr("Up"), lambda: self.move_scene(-1))
        self.scene_down = self._button("live_studio.scene.down", tr("Down"), lambda: self.move_scene(1))
        scenes_box = self._group(
            tr("Scenes"), self.scene_list, [self.scene_add, self.scene_remove, self.scene_up, self.scene_down]
        )

        # ----- sources -----
        self.source_list = QListWidget(self)
        self.source_list.setObjectName("live_studio.sources")
        self.source_list.currentItemChanged.connect(self._source_chosen)
        self.source_list.itemChanged.connect(self._source_edited)
        self.source_add = QPushButton(tr("Add"), self)
        self.source_add.setObjectName("live_studio.source.add")
        add_menu = QMenu(self.source_add)
        add_menu.setObjectName("live_studio.source.add.menu")
        for kind in SOURCE_TYPES:
            action = add_menu.addAction(source_type_label(kind) + ("…" if kind in FILE_TYPES else ""))
            action.setObjectName(f"live_studio.source.add.{kind}")
            action.triggered.connect(lambda _c=False, k=kind: self.add_source_dialog(k))
        self.source_add.setMenu(add_menu)
        self.source_remove = self._button("live_studio.source.remove", tr("Remove"), self.remove_source)
        self.source_up = self._button("live_studio.source.up", tr("Up"), lambda: self.move_source(1))
        self.source_down = self._button("live_studio.source.down", tr("Down"), lambda: self.move_source(-1))
        sources_box = self._group(
            tr("Sources"), self.source_list, [self.source_add, self.source_remove, self.source_up, self.source_down]
        )

        # ----- properties -----
        self.props = QGroupBox(tr("Properties"), self)
        self.props.setObjectName("live_studio.props")
        self.props_layout = QFormLayout(self.props)
        self.name_edit = QLineEdit(self)
        self.name_edit.setObjectName("live_studio.prop.name")
        self.name_edit.editingFinished.connect(self._name_edited)
        self.visible_box = QCheckBox(tr("Visible"), self)
        self.visible_box.setObjectName("live_studio.prop.visible")
        self.visible_box.toggled.connect(lambda on: self._set_source({"visible": bool(on)}))
        self.kind_form = QWidget(self)
        self.kind_form.setObjectName("live_studio.prop.kind")
        self.kind_layout = QFormLayout(self.kind_form)
        self.kind_layout.setContentsMargins(0, 0, 0, 0)
        self.kind_widgets: dict[str, QWidget] = {}
        self.transform_widgets: dict[str, Any] = {}
        self.props_layout.addRow(tr("Name"), self.name_edit)
        self.props_layout.addRow(self.visible_box)
        self.props_layout.addRow(self.kind_form)
        self._build_transform()
        self.hint = QLabel(tr("Add a scene, then a source to it."), self)
        self.hint.setObjectName("live_studio.hint")
        self.hint.setWordWrap(True)
        self.message = QLabel("", self)
        self.message.setObjectName("live_studio.message")
        self.message.setWordWrap(True)

        # ----- layout -----
        side = QWidget(self)
        side_layout = QVBoxLayout(side)
        side_layout.setContentsMargins(0, 0, 0, 0)
        side_layout.addWidget(self.hint)
        side_layout.addWidget(self.props, 1)
        top = QSplitter(Qt.Orientation.Horizontal, self)
        top.addWidget(self.preview)
        top.addWidget(side)
        top.setStretchFactor(0, 3)
        top.setStretchFactor(1, 1)
        bottom = QSplitter(Qt.Orientation.Horizontal, self)
        bottom.addWidget(scenes_box)
        bottom.addWidget(sources_box)
        main = QSplitter(Qt.Orientation.Vertical, self)
        main.addWidget(top)
        main.addWidget(bottom)
        main.setStretchFactor(0, 3)
        main.setStretchFactor(1, 1)
        layout = QVBoxLayout(self)
        layout.addWidget(main, 1)
        layout.addWidget(self.message)

        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(REFRESH_DEBOUNCE_MS)
        self._timer.timeout.connect(self.refresh)
        engine.event.connect(self._on_event)
        self._update_buttons()
        self._show_properties(None)

    # ----- construction helpers -----
    def _button(self, name: str, text: str, slot: Any) -> QPushButton:
        button = QPushButton(text, self)
        button.setObjectName(name)
        button.clicked.connect(lambda _c=False: slot())
        return button

    def _group(self, title: str, view: QListWidget, buttons: list[QPushButton]) -> QGroupBox:
        box = QGroupBox(title, self)
        column = QVBoxLayout(box)
        column.addWidget(view, 1)
        row = QHBoxLayout()
        for b in buttons:
            row.addWidget(b)
        row.addStretch(1)
        column.addLayout(row)
        return box

    def _spin(self, key: str, whole: bool = False, tip: str = "", prefix: str = "") -> Any:
        spin: Any = QSpinBox(self) if whole else QDoubleSpinBox(self)
        spin.setObjectName(f"live_studio.prop.{key}")
        if whole:
            spin.setRange(0, 65535)
        else:
            scaled = key.startswith("scale")
            spin.setRange(-1000.0 if scaled else -1e6, 1000.0 if scaled else 1e6)
            spin.setDecimals(3 if scaled else 1)
            spin.setSingleStep(0.1 if scaled else 1.0)
        if tip:
            spin.setToolTip(tip)
        if prefix:
            spin.setPrefix(prefix + " ")
        edited = "crop" if key.startswith("crop") else key
        spin.valueChanged.connect(lambda _v, k=edited: self._transform_edited(k))
        self.transform_widgets[key] = spin
        return spin

    def _pair(self, spins: list[Any]) -> QWidget:
        row = QWidget(self)
        layout = QHBoxLayout(row)
        layout.setContentsMargins(0, 0, 0, 0)
        for spin in spins:
            layout.addWidget(spin)
        return row

    def _build_transform(self) -> None:
        """Position, scale, rotation and crop (the source's transform). The letters in front of the
        boxes are axis names (X, Y) and the first letters of the sides (tooltips: the full words)."""
        self.props_layout.addRow(tr("Position"), self._pair([self._spin("x", prefix="X"), self._spin("y", prefix="Y")]))
        self.props_layout.addRow(
            tr("Scale"), self._pair([self._spin("scaleX", prefix="X"), self._spin("scaleY", prefix="Y")])
        )
        self.props_layout.addRow(tr("Rotation"), self._spin("rotation"))
        sides = (tr("Left"), tr("Top"), tr("Right"), tr("Bottom"))
        self.props_layout.addRow(
            tr("Crop"),
            self._pair([self._spin(f"crop{i}", True, side, side[:1]) for i, side in enumerate(sides)]),
        )

    # ----- engine -----
    def refresh(self) -> None:
        if not self.engine.running:
            return
        self.engine.execute("live.studio.scenes", {}, on_reply=lambda r, _b: self.apply(r), on_error=self._failed)

    def schedule_refresh(self) -> None:
        if not self._timer.isActive():
            self._timer.start()

    def _on_event(self, event: Any) -> None:
        if isinstance(event, dict) and event.get("event") == "projectChanged":
            self.schedule_refresh()

    def _failed(self, message: str) -> None:
        self.error = message
        self.message.setText(message)
        self.schedule_refresh()  # the lists show what the project really holds

    def run(self, command: str, params: dict[str, Any], on_done: Any = None) -> None:
        """One engine command, then the lists are read again (an error is shown, never raised)."""

        def done(result: Any, _b: bytes | None) -> None:
            self.error = ""
            self.message.setText("")
            if on_done is not None:
                on_done(result)
            self.schedule_refresh()

        if self.engine.running:
            self.engine.execute(command, params, on_reply=done, on_error=self._failed)

    # ----- showing the project -----
    def apply(self, result: Any) -> None:
        """Show a ``live.studio.scenes`` reply: the scene list, the current scene's sources, the
        selected source's properties."""
        scenes, current = scenes_of(result)
        self.scenes = scenes
        self.current_scene = current
        self._updating = True
        try:
            with QSignalBlocker(self.scene_list):
                self.scene_list.clear()
                for scene in scenes:
                    item = QListWidgetItem(str(scene.get("name", "")))
                    item.setData(SCENE_ROLE, scene["id"])
                    if scene["id"] == current:  # the scene on the program is bold
                        font = item.font()
                        font.setBold(True)
                        item.setFont(font)
                    item.setFlags(item.flags() | Qt.ItemFlag.ItemIsEditable)
                    self.scene_list.addItem(item)
                    if scene["id"] == current:
                        self.scene_list.setCurrentItem(item)
            sources = sources_of(self._scene(current))
            with QSignalBlocker(self.source_list):
                self.source_list.clear()
                for src in sources:
                    item = QListWidgetItem(str(src.get("name", "")))
                    item.setData(SCENE_ROLE, src["id"])
                    item.setFlags(item.flags() | Qt.ItemFlag.ItemIsUserCheckable | Qt.ItemFlag.ItemIsEditable)
                    item.setCheckState(Qt.CheckState.Checked if src.get("visible", True) else Qt.CheckState.Unchecked)
                    item.setToolTip(source_type_label(str(src.get("type", ""))))
                    self.source_list.addItem(item)
                    if src["id"] == self.selected_source:
                        self.source_list.setCurrentItem(item)
            ids = [s["id"] for s in sources]
            if self.selected_source not in ids:
                self.selected_source = None
            self._show_properties(next((s for s in sources if s["id"] == self.selected_source), None))
        finally:
            self._updating = False
        self.hint.setVisible(not scenes or not sources)
        self.hint.setText(tr("Add a scene, then a source to it."))
        self._update_buttons()

    def _scene(self, scene_id: int | None) -> dict[str, Any] | None:
        return next((s for s in self.scenes if s["id"] == scene_id), None)

    def _source(self, source_id: int | None) -> dict[str, Any] | None:
        return next((s for s in sources_of(self._scene(self.current_scene)) if s["id"] == source_id), None)

    def _update_buttons(self) -> None:
        scene_row = self.scene_list.currentRow()
        source_row = self.source_list.currentRow()
        have_scene = scene_row >= 0
        have_source = source_row >= 0
        self.scene_remove.setEnabled(have_scene)
        self.scene_up.setEnabled(have_scene and scene_row > 0)
        self.scene_down.setEnabled(have_scene and scene_row < self.scene_list.count() - 1)
        self.source_add.setEnabled(self.current_scene is not None)
        self.source_remove.setEnabled(have_source)
        self.source_up.setEnabled(have_source and source_row > 0)
        self.source_down.setEnabled(have_source and source_row < self.source_list.count() - 1)

    # ----- scenes -----
    def add_scene(self, name: str | None = None) -> None:
        self.run("live.studio.scene.add", {"name": name} if name else {})

    def remove_scene(self) -> None:
        item = self.scene_list.currentItem()
        if item is not None:
            self.run("live.studio.scene.remove", {"id": item.data(SCENE_ROLE)})

    def move_scene(self, step: int) -> None:
        row = self.scene_list.currentRow()
        item = self.scene_list.currentItem()
        if item is not None and 0 <= row + step < self.scene_list.count():
            self.run("live.studio.scene.move", {"id": item.data(SCENE_ROLE), "index": row + step})

    def _scene_chosen(self, item: QListWidgetItem | None, _previous: QListWidgetItem | None) -> None:
        self._update_buttons()
        if self._updating or item is None:
            return
        scene_id = item.data(SCENE_ROLE)
        if scene_id != self.current_scene:
            self.selected_source = None
            self.run("live.studio.scene.select", {"id": scene_id})

    def _scene_edited(self, item: QListWidgetItem) -> None:
        if self._updating:
            return
        scene = self._scene(item.data(SCENE_ROLE))
        name = item.text().strip()
        if scene is not None and name and name != scene.get("name"):
            self.run("live.studio.scene.rename", {"id": scene["id"], "name": name})
        else:
            self.schedule_refresh()

    # ----- sources -----
    def add_source_dialog(self, kind: str) -> None:
        """Add Source ▸ type: file types ask for the file first."""
        if kind in FILE_TYPES:
            title = source_type_label(kind)
            path, _ = QFileDialog.getOpenFileName(self, title, "", IMAGE_FILTER if kind == "image" else VIDEO_FILTER)
            if not path:
                return
            self.add_source(kind, {"path": path})
        else:
            self.add_source(kind)

    def add_source(self, kind: str, params: dict[str, Any] | None = None) -> None:
        def added(result: Any) -> None:
            src = result.get("source") if isinstance(result, dict) else None
            if isinstance(src, dict) and isinstance(src.get("id"), int):
                self.selected_source = src["id"]

        self.run("live.studio.source.add", {"type": kind, **(params or {})}, added)

    def remove_source(self) -> None:
        item = self.source_list.currentItem()
        if item is not None:
            self.selected_source = None
            self.run("live.studio.source.remove", {"id": item.data(SCENE_ROLE)})

    def move_source(self, step: int) -> None:
        """``step`` +1 raises the source in the stack (up in the list), -1 lowers it."""
        item = self.source_list.currentItem()
        src = self._source(item.data(SCENE_ROLE)) if item is not None else None
        scene = self._scene(self.current_scene)
        if src is None or scene is None:
            return
        bottom_first = [s["id"] for s in scene.get("sources", []) if isinstance(s, dict)]
        if src["id"] not in bottom_first:
            return
        target = bottom_first.index(src["id"]) + step
        if 0 <= target < len(bottom_first):
            self.run("live.studio.source.move", {"id": src["id"], "index": target})

    def _source_chosen(self, item: QListWidgetItem | None, _previous: QListWidgetItem | None) -> None:
        self._update_buttons()
        if self._updating:
            return
        self.selected_source = item.data(SCENE_ROLE) if item is not None else None
        self._show_properties(self._source(self.selected_source))

    def _source_edited(self, item: QListWidgetItem) -> None:
        if self._updating:
            return
        src = self._source(item.data(SCENE_ROLE))
        if src is None:
            return
        params: dict[str, Any] = {"id": src["id"]}
        visible = item.checkState() == Qt.CheckState.Checked
        if visible != src.get("visible", True):
            params["visible"] = visible
        name = item.text().strip()
        if name and name != src.get("name"):
            params["name"] = name
        if len(params) > 1:
            self.run("live.studio.source.set", params)
        else:
            self.schedule_refresh()

    # ----- properties -----
    def _set_source(self, fields: dict[str, Any], merge: str | None = None) -> None:
        """Change the selected source (``merge``: edits with this key share one undo step)."""
        if self._updating or self.selected_source is None:
            return
        params: dict[str, Any] = {"id": self.selected_source, **fields}
        if merge:
            params["merge"] = merge
        self.run("live.studio.source.set", params)

    def _name_edited(self) -> None:
        src = self._source(self.selected_source)
        name = self.name_edit.text().strip()
        if src is not None and name and name != src.get("name"):
            self._set_source({"name": name})

    def _transform_edited(self, key: str) -> None:
        if self._updating:
            return
        if key == "crop":
            crop = [int(self.transform_widgets[f"crop{i}"].value()) for i in range(4)]
            self._set_source({"transform": {"crop": crop}}, "transform.crop")
        else:
            self._set_source({"transform": {key: float(self.transform_widgets[key].value())}}, f"transform.{key}")

    def _clear_kind_form(self) -> None:
        while self.kind_layout.rowCount():
            self.kind_layout.removeRow(0)
        self.kind_widgets.clear()

    def _show_properties(self, src: dict[str, Any] | None) -> None:
        """Fill the property form with ``src`` (None: nothing selected, the form is disabled). The
        type-specific rows are rebuilt only when another source or type is chosen, so typing in a
        field is not interrupted by the refresh that follows each change."""
        self.props.setEnabled(src is not None)
        was = self._updating
        self._updating = True
        try:
            if src is None:
                self._shown_source = None
                self._clear_kind_form()
                self.name_edit.clear()
                return
            kind = str(src.get("type", ""))
            if self._shown_source != (src["id"], kind):
                self._shown_source = (src["id"], kind)
                self._clear_kind_form()
                self._build_kind_form(kind)
            if not self.name_edit.hasFocus():
                self.name_edit.setText(str(src.get("name", "")))
            self.visible_box.setChecked(bool(src.get("visible", True)))
            self._load_kind(src)
            transform = as_dict(src.get("transform"))
            for key in ("x", "y", "scaleX", "scaleY", "rotation"):
                spin = self.transform_widgets[key]
                if not spin.hasFocus():
                    spin.setValue(float(transform.get(key, 1.0 if key.startswith("scale") else 0.0)))
            crop_value = transform.get("crop")
            crop: list[Any] = crop_value if isinstance(crop_value, list) else [0, 0, 0, 0]
            for i in range(4):
                spin = self.transform_widgets[f"crop{i}"]
                if not spin.hasFocus():
                    spin.setValue(int(crop[i]) if i < len(crop) and isinstance(crop[i], int) else 0)
        finally:
            self._updating = was

    def _build_kind_form(self, kind: str) -> None:
        """The rows of one source type: colour and size, file, text, … ."""
        layout = self.kind_layout
        if kind in ("color", "text"):
            button = QPushButton(self)
            button.setObjectName("live_studio.prop.color")
            button.clicked.connect(lambda _c=False: self.choose_color())
            self.kind_widgets["color"] = button
            layout.addRow(tr("Color"), button)
        if kind == "color":
            for key, label in (("width", tr("Width")), ("height", tr("Height"))):
                side = QSpinBox(self)
                side.setObjectName(f"live_studio.prop.{key}")
                side.setRange(1, 16384)
                side.valueChanged.connect(lambda _v, k=key: self._set_source({k: self.kind_widgets[k].value()}, k))  # type: ignore[attr-defined]
                self.kind_widgets[key] = side
                layout.addRow(label, side)
        if kind in FILE_TYPES:
            row = QWidget(self)
            row_layout = QHBoxLayout(row)
            row_layout.setContentsMargins(0, 0, 0, 0)
            path = QLineEdit(row)
            path.setObjectName("live_studio.prop.path")
            path.editingFinished.connect(
                lambda: self._set_source({"path": path.text().strip()}) if path.text().strip() else None
            )
            browse = QPushButton(tr("Browse…"), row)
            browse.setObjectName("live_studio.prop.browse")
            browse.clicked.connect(lambda _c=False, k=kind: self._browse(k))
            row_layout.addWidget(path, 1)
            row_layout.addWidget(browse)
            self.kind_widgets["path"] = path
            layout.addRow(tr("File"), row)
        if kind == "media":
            loop = QCheckBox(tr("Loop"), self)
            loop.setObjectName("live_studio.prop.loop")
            loop.toggled.connect(lambda on: self._set_source({"loop": bool(on)}))
            self.kind_widgets["loop"] = loop
            layout.addRow(loop)
        if kind == "text":
            text = QLineEdit(self)
            text.setObjectName("live_studio.prop.text")
            text.editingFinished.connect(lambda: self._set_source({"text": text.text()}))
            size = QDoubleSpinBox(self)
            size.setObjectName("live_studio.prop.size")
            size.setRange(1.0, 4096.0)
            size.valueChanged.connect(lambda v: self._set_source({"size": float(v)}, "size"))
            self.kind_widgets["text"] = text
            self.kind_widgets["size"] = size
            layout.addRow(tr("Text"), text)
            layout.addRow(tr("Size"), size)

    def _load_kind(self, src: dict[str, Any]) -> None:
        kind_data = as_dict(src.get("kind"))
        for key, widget in self.kind_widgets.items():
            if widget.hasFocus():
                continue
            if key == "color" and isinstance(widget, QPushButton):
                color = color_of(kind_data.get("color"))
                widget.setText(color_hex(color).upper())
                widget.setStyleSheet(
                    f"background: {color.name()}; color: {'black' if color.lightness() > 128 else 'white'};"
                )
            elif isinstance(widget, QSpinBox):
                widget.setValue(int(kind_data.get(key, widget.minimum())))
            elif isinstance(widget, QDoubleSpinBox):
                widget.setValue(float(kind_data.get(key, widget.minimum())))
            elif isinstance(widget, QLineEdit):
                widget.setText(str(kind_data.get(key, "")))
            elif isinstance(widget, QCheckBox):
                widget.setChecked(bool(kind_data.get("loop", True)))

    def choose_color(self) -> None:
        src = self._source(self.selected_source)
        if src is None:
            return
        kind_data = as_dict(src.get("kind"))
        color = QColorDialog.getColor(
            color_of(kind_data.get("color")),
            self,
            tr("Color"),
            QColorDialog.ColorDialogOption.ShowAlphaChannel,
        )
        if color.isValid():
            self._set_source({"color": color_hex(color)})

    def _browse(self, kind: str) -> None:
        title = source_type_label(kind)
        path, _ = QFileDialog.getOpenFileName(self, title, "", IMAGE_FILTER if kind == "image" else VIDEO_FILTER)
        if path:
            self._set_source({"path": path})

    # ----- window -----
    def showEvent(self, event: Any) -> None:
        super().showEvent(event)
        self.refresh()
        if self.auto_start and self.engine.running and not self.preview.frame_timer.isActive():
            self.preview.start_host()

    def closeEvent(self, event: Any) -> None:
        self._timer.stop()
        self.preview.close()
        super().closeEvent(event)
