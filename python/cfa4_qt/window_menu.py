"""Window menu: every panel of the egui editor, in its order and with its shortcuts, plus
Workspaces. A panel the Qt GUI does not have yet is listed disabled ("not yet in the Qt GUI"),
so the menu shows the parity gap instead of hiding it (docs/qt-parity.md).

Workspaces are saved dock layouts. "Editing" is the default layout; the user can save changes
to the current workspace, save the layout as a new named one, and reset to the saved layout.
egui's other workspaces (Assembly, Color, Effects, Audio, Captions and Graphics, All Panels)
start from egui's layouts of them (``crates/ui-egui/src/dock.rs`` ``workspace``).
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from PySide6.QtCore import QByteArray, QObject, QSettings, Qt
from PySide6.QtGui import QAction, QActionGroup, QKeySequence
from PySide6.QtWidgets import (
    QDialog,
    QDockWidget,
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMenu,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from .i18n import N_, tr


@dataclass(frozen=True)
class PanelEntry:
    title: str  # the egui panel title (crates/ui-egui/src/dock.rs PanelKind::title)
    shortcut: str = ""  # the egui Window-menu shortcut (PanelKind::window_shortcut)


# The egui Window menu, in its order (PanelKind::ALL).
PANELS = (
    PanelEntry(N_("Project"), "Shift+1"),
    PanelEntry(N_("Media Browser"), "Shift+8"),
    PanelEntry(N_("Libraries")),
    PanelEntry(N_("Info")),
    PanelEntry(N_("Effects"), "Shift+7"),
    PanelEntry(N_("Markers")),
    PanelEntry(N_("History")),
    PanelEntry(N_("Source"), "Shift+2"),
    PanelEntry(N_("Effect Controls"), "Shift+5"),
    PanelEntry(N_("Audio Clip Mixer"), "Shift+9"),
    PanelEntry(N_("Metadata")),
    PanelEntry(N_("Program"), "Shift+4"),
    PanelEntry(N_("Timeline"), "Shift+3"),
    PanelEntry(N_("Tools")),
    PanelEntry(N_("Audio Meters")),
    PanelEntry(N_("Audio Track Mixer"), "Shift+6"),
    PanelEntry(N_("Lumetri Color")),
    PanelEntry(N_("Lumetri Scopes")),
    PanelEntry(N_("Essential Graphics")),
    PanelEntry(N_("Essential Sound")),
    PanelEntry(N_("Text to Speech")),
    PanelEntry(N_("Properties")),
    PanelEntry(N_("Text")),
    PanelEntry(N_("Events")),
    PanelEntry(N_("Progress")),
    PanelEntry(N_("Reference Monitor")),
    PanelEntry(N_("Timecode")),
    PanelEntry(N_("Project Notes")),
)

# egui workspaces (crates/ui-egui/src/menus.rs window.workspace.*) and their shortcuts
DEFAULT_WORKSPACE = "Editing"
EGUI_WORKSPACES = (
    (N_("Editing"), "Alt+Shift+1"),
    (N_("Assembly"), "Alt+Shift+2"),
    (N_("Color"), "Alt+Shift+3"),
    (N_("Effects"), "Alt+Shift+4"),
    (N_("Audio"), "Alt+Shift+5"),
    (N_("Captions and Graphics"), "Alt+Shift+6"),
)
ALL_PANELS = N_("All Panels")
BUILTIN_NAMES = frozenset([*(n for n, _k in EGUI_WORKSPACES), ALL_PANELS])
NOT_YET = N_("Not yet in the Qt GUI (see docs/qt-parity.md)")


class Workspaces:
    """Named dock layouts in QSettings (``workspaces/<name>``); "Editing" falls back to the
    built-in default layout."""

    def __init__(
        self,
        window: QMainWindow,
        settings: QSettings,
        default_state: QByteArray,
        version: int,
        builtin: Callable[[str], QByteArray] | None = None,
    ) -> None:
        self.window = window
        self.settings = settings
        self.default_state = default_state
        self.version = version
        self.builtin = builtin  # egui's layout of a built-in workspace (Assembly, Color, …)

    def names(self) -> list[str]:
        """The user's own workspaces (a built-in one saved with changes is still built in)."""
        self.settings.beginGroup("workspaces")
        keys = [k for k in self.settings.childKeys() if k not in BUILTIN_NAMES]
        self.settings.endGroup()
        return sorted(keys, key=str.casefold)

    @property
    def current(self) -> str:
        name = str(self.settings.value("workspace/current") or DEFAULT_WORKSPACE)
        return name if name in BUILTIN_NAMES or name in self.names() else DEFAULT_WORKSPACE

    def saved_state(self, name: str) -> QByteArray:
        state = self.settings.value(f"workspaces/{name}")
        if isinstance(state, QByteArray) and not state.isEmpty():
            return state
        if name != DEFAULT_WORKSPACE and name in BUILTIN_NAMES and self.builtin is not None:
            return self.builtin(name)
        return self.default_state

    def switch(self, name: str, restore: Callable[[QByteArray], None]) -> None:
        self.settings.setValue("workspace/current", name)
        restore(self.saved_state(name))

    def save_current(self) -> None:
        self.settings.setValue(f"workspaces/{self.current}", self.window.saveState(self.version))

    def all_names(self) -> list[str]:
        return [*(n for n, _k in EGUI_WORKSPACES), ALL_PANELS, *self.names()]

    def changed(self, name: str) -> bool:
        """A workspace with a saved layout (a built-in one saved with changes)."""
        state = self.settings.value(f"workspaces/{name}")
        return isinstance(state, QByteArray) and not state.isEmpty()

    def rename(self, old: str, new: str) -> bool:
        new = new.strip().replace("/", "-")
        if old in BUILTIN_NAMES or old not in self.names() or not new or new in BUILTIN_NAMES or new in self.names():
            return False
        self.settings.setValue(f"workspaces/{new}", self.settings.value(f"workspaces/{old}"))
        self.settings.remove(f"workspaces/{old}")
        if str(self.settings.value("workspace/current") or "") == old:
            self.settings.setValue("workspace/current", new)
        return True

    def delete(self, name: str) -> None:
        """Delete an own workspace, or a built-in one's saved changes (Restore Original)."""
        self.settings.remove(f"workspaces/{name}")
        if name not in BUILTIN_NAMES and str(self.settings.value("workspace/current") or "") == name:
            self.settings.setValue("workspace/current", DEFAULT_WORKSPACE)

    def save_as(self, name: str) -> None:
        self.settings.setValue(f"workspaces/{name}", self.window.saveState(self.version))
        self.settings.setValue("workspace/current", name)


class EditWorkspacesDialog(QDialog):
    """Edit Workspaces… (egui's): rename or delete an own workspace; a built-in one that was
    saved with changes can go back to its original layout."""

    def __init__(self, workspaces: Workspaces, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.workspaces = workspaces
        self.setObjectName("workspaces.edit")
        self.setWindowTitle(tr("Edit Workspaces"))
        self.list = QListWidget(self)
        self.list.setObjectName("workspaces.edit.list")
        self.name = QLineEdit(self)
        self.name.setObjectName("workspaces.edit.name")
        self.rename_button = QPushButton(tr("Rename"), self)
        self.rename_button.setObjectName("workspaces.edit.rename")
        self.delete_button = QPushButton(tr("Delete"), self)
        self.delete_button.setObjectName("workspaces.edit.delete")
        close = QPushButton(tr("Close"), self)
        close.setObjectName("workspaces.edit.close")
        close.clicked.connect(self.accept)
        self.rename_button.clicked.connect(self._rename)
        self.delete_button.clicked.connect(self._delete)
        self.list.currentItemChanged.connect(lambda _c, _p: self._selected())
        self.name.textChanged.connect(lambda _t: self._selected(keep_name=True))
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Name:"), self))
        row.addWidget(self.name, 1)
        row.addWidget(self.rename_button)
        buttons = QHBoxLayout()
        buttons.addWidget(self.delete_button)
        buttons.addStretch(1)
        buttons.addWidget(close)
        layout = QVBoxLayout(self)
        layout.addWidget(self.list, 1)
        layout.addLayout(row)
        layout.addLayout(buttons)
        self._fill()

    def _fill(self, select: str | None = None) -> None:
        self.list.clear()
        for n in self.workspaces.all_names():
            it = QListWidgetItem(tr(n) if n in BUILTIN_NAMES else n)
            it.setData(Qt.ItemDataRole.UserRole, n)
            self.list.addItem(it)
            if n == select:
                self.list.setCurrentItem(it)
        self._selected()

    def current(self) -> str | None:
        it = self.list.currentItem()
        v = it.data(Qt.ItemDataRole.UserRole) if it is not None else None
        return v if isinstance(v, str) else None

    def _selected(self, keep_name: bool = False) -> None:
        sel = self.current()
        own = sel is not None and sel not in BUILTIN_NAMES
        restore = sel is not None and sel in BUILTIN_NAMES and self.workspaces.changed(sel)
        if not keep_name:
            self.name.blockSignals(True)
            self.name.setText(sel if own else "")
            self.name.blockSignals(False)
        self.name.setEnabled(own)
        self.rename_button.setEnabled(own and self.name.text().strip() not in ("", sel))
        self.delete_button.setText(tr("Restore Original") if restore else tr("Delete"))
        self.delete_button.setEnabled(own or restore)

    def _rename(self) -> None:
        sel, new = self.current(), self.name.text().strip()
        if sel is not None and self.workspaces.rename(sel, new):
            self._fill(new.replace("/", "-"))

    def _delete(self) -> None:
        sel = self.current()
        if sel is not None:
            self.workspaces.delete(sel)
            self._fill(sel if sel in BUILTIN_NAMES else None)


def panel_action(window: QMainWindow, entry: PanelEntry, dock: QDockWidget | None, central: QWidget | None) -> QAction:
    """A Window-menu entry: shows and brings forward the panel, or closes it when it is
    already open in front; disabled when the Qt GUI has no such panel yet."""
    action = QAction(tr(entry.title), window)
    action.setObjectName(f"window.panel.{entry.title.replace(' ', '')}")
    if dock is not None:
        action.setCheckable(True)
        action.setChecked(dock.isVisible())
        sync = dock.visibilityChanged.connect(lambda _v, a=action, d=dock: a.setChecked(not d.isHidden()))
        # the dock outlives the menu: a rebuild deletes this action, so its callback goes with it
        action.destroyed.connect(lambda *_a, c=sync: QObject.disconnect(c))

        def toggle(_checked: bool = False, d: QDockWidget = dock) -> None:
            if d.isHidden():
                d.show()
                d.raise_()
            elif d.visibleRegion().isEmpty():  # open, but behind another tab: bring it forward
                d.raise_()
            else:
                d.close()
            action.setChecked(not d.isHidden())

        action.triggered.connect(toggle)
    elif central is not None:  # the Program monitor is the central area: always open
        action.setCheckable(True)
        action.setChecked(True)
        action.triggered.connect(lambda _c=False: (action.setChecked(True), central.setFocus()))
    else:
        action.setEnabled(False)
        action.setToolTip(tr(NOT_YET))
        action.setStatusTip(tr(NOT_YET))
    return action


def build_window_menu(
    window: QMainWindow,
    menu: QMenu,
    docks: dict[str, QDockWidget],
    central: QWidget,
    workspaces: Workspaces,
    restore: Callable[[QByteArray], None],
    reset_action: QAction,
) -> list[tuple[QAction, str]]:
    """Fill ``menu``; returns (action, shortcut) pairs for the caller to bind where free."""
    menu.setToolTipsVisible(True)
    shortcuts: list[tuple[QAction, str]] = []

    ws = menu.addMenu(tr("Workspaces"))
    ws.setObjectName("menu.Window.Workspaces")
    ws.setToolTipsVisible(True)
    group = QActionGroup(window)
    for name, key in (*EGUI_WORKSPACES, (ALL_PANELS, "")):
        a = QAction(tr(name), window)
        a.setObjectName(f"window.workspace.{name.replace(' ', '')}")
        a.setCheckable(True)
        a.setChecked(workspaces.current == name)
        a.triggered.connect(lambda _c=False, n=name: workspaces.switch(n, restore))
        group.addAction(a)
        if key:
            shortcuts.append((a, key))
        ws.addAction(a)
    own = workspaces.names()
    if own:
        ws.addSeparator()
        for name in own:
            a = QAction(name, window)
            a.setObjectName(f"window.workspace.user.{name}")
            a.setCheckable(True)
            a.setChecked(workspaces.current == name)
            a.triggered.connect(lambda _c=False, n=name: workspaces.switch(n, restore))
            group.addAction(a)
            ws.addAction(a)
    ws.addSeparator()
    reset_action.setText(tr("Reset to Saved Layout"))
    ws.addAction(reset_action)
    shortcuts.append((reset_action, "Alt+Shift+0"))
    save = QAction(tr("Save Changes to this Workspace"), window)
    save.setObjectName("window.workspace.saveChanges")
    save.triggered.connect(workspaces.save_current)
    ws.addAction(save)
    save_as = QAction(tr("Save as New Workspace…"), window)
    save_as.setObjectName("window.workspace.saveAs")

    def ask_name() -> None:
        name, ok = QInputDialog.getText(window, tr("Save as New Workspace"), tr("Name"))
        name = name.strip().replace("/", "-")
        if ok and name and name not in BUILTIN_NAMES:
            workspaces.save_as(name)
            window.rebuild_window_menu()  # type: ignore[attr-defined]

    save_as.triggered.connect(ask_name)
    ws.addAction(save_as)
    edit = QAction(tr("Edit Workspaces…"), window)
    edit.setObjectName("window.workspace.edit")

    def edit_workspaces() -> None:
        EditWorkspacesDialog(workspaces, window).exec()
        window.rebuild_window_menu()  # type: ignore[attr-defined]

    edit.triggered.connect(edit_workspaces)
    ws.addAction(edit)

    new_project = QAction(tr("New Project Panel"), window)
    new_project.setEnabled(False)
    new_project.setToolTip(tr(NOT_YET))
    menu.addAction(new_project)
    for entry in PANELS:
        dock = docks.get(entry.title)
        a = panel_action(window, entry, dock, central if entry.title == "Program" else None)
        menu.addAction(a)
        if entry.shortcut and a.isEnabled():
            shortcuts.append((a, entry.shortcut))
    return shortcuts


def bind_free(window: QMainWindow, pairs: list[tuple[QAction, str]], taken: set[str]) -> list[QAction]:
    """Give each action its shortcut unless an engine command already uses the key. Returns the
    actions added to ``window``, so a rebuild can remove them (two live actions with one key
    would make the key ambiguous, and neither would fire)."""
    added = []
    for action, key in pairs:
        seq = QKeySequence(key)
        if seq.toString() not in taken:
            action.setShortcut(seq)
            window.addAction(action)
            added.append(action)
    return added
