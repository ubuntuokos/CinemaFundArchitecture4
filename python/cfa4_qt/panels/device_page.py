"""The shared device page of Settings ▸ Hardware & Devices (``docs/plans/CFA4-SETTINGS-HW-003.md``
R4, the menu plan's "common device page"): a device list with each device's status on the left,
and tabs on the right (Details, Capabilities, Settings, Drivers, Diagnostics). Accelerators is the first page
built on it; later device pages (displays, storage, cameras, tablets) reuse it with their own rows.

A page gives ``DeviceEntry`` rows and, per tab, a function that draws the tab for the selected
device. Object names: ``<name>.list`` (the device list), ``<name>.tabs``, ``<name>.tab.<key>``.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from PySide6.QtWidgets import QHBoxLayout, QListWidget, QListWidgetItem, QScrollArea, QTabWidget, QWidget

from ..i18n import N_, tr

# the standard tabs, in order (key, title)
TABS = (("details", N_("Details")), ("capabilities", N_("Capabilities")), ("settings", N_("Settings")), ("drivers", N_("Drivers")),
        ("diagnostics", N_("Diagnostics")))


@dataclass(frozen=True)
class DeviceEntry:
    """One device in the list: ``key`` is stable (the node), ``title`` the shown name, ``status``
    a short word after it (``OK``, ``No access``…)."""

    key: str
    title: str
    status: str = ""


TabBuilder = Callable[[QWidget, "DeviceEntry | None"], QWidget]


class DevicePage(QWidget):
    """``tabs`` maps a tab key of :data:`TABS` to its builder; tabs without a builder are left out.
    The selection survives a rebuild through ``selected`` (the entry key)."""

    def __init__(self, parent: QWidget, name: str, entries: list[DeviceEntry], tabs: dict[str, TabBuilder],
                 selected: str = "", on_select: Callable[[str], None] | None = None, tab: str = "",
                 on_tab: Callable[[str], None] | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(name)
        self.entries = entries
        self.builders = tabs
        self.on_select = on_select
        self.on_tab = on_tab
        row = QHBoxLayout(self)
        row.setContentsMargins(0, 0, 0, 0)
        self.list = QListWidget(self)
        self.list.setObjectName(f"{name}.list")
        self.list.setFixedWidth(220)
        for e in entries:
            item = QListWidgetItem(f"{e.title}\n{e.status}" if e.status else e.title)
            item.setToolTip(e.title)
            self.list.addItem(item)
        row.addWidget(self.list)
        self.tabs = QTabWidget(self)
        self.tabs.setObjectName(f"{name}.tabs")
        row.addWidget(self.tabs, 1)
        self.keys = [k for k, _t in TABS if k in tabs]
        index = next((i for i, e in enumerate(entries) if e.key == selected), 0)
        if entries:
            self.list.setCurrentRow(index)
        self._fill(entries[index] if entries else None)
        if tab in self.keys:
            self.tabs.setCurrentIndex(self.keys.index(tab))
        self.list.currentRowChanged.connect(self._selected)
        self.tabs.currentChanged.connect(self._tab_changed)

    def current(self) -> DeviceEntry | None:
        row = self.list.currentRow()
        return self.entries[row] if 0 <= row < len(self.entries) else None

    def _fill(self, entry: DeviceEntry | None) -> None:
        keep = self.tabs.currentIndex()
        self.tabs.blockSignals(True)
        while self.tabs.count():
            w = self.tabs.widget(0)
            self.tabs.removeTab(0)
            if w is not None:
                w.deleteLater()
        titles = dict(TABS)
        for key in self.keys:
            scroll = QScrollArea(self.tabs)
            scroll.setObjectName(f"{self.objectName()}.tab.{key}")
            scroll.setWidgetResizable(True)
            scroll.setWidget(self.builders[key](scroll, entry))
            self.tabs.addTab(scroll, tr(titles[key]))
        if 0 <= keep < self.tabs.count():
            self.tabs.setCurrentIndex(keep)
        self.tabs.blockSignals(False)

    def _selected(self, row: int) -> None:
        entry = self.entries[row] if 0 <= row < len(self.entries) else None
        self._fill(entry)
        if entry is not None and self.on_select is not None:
            self.on_select(entry.key)

    def _tab_changed(self, index: int) -> None:
        if self.on_tab is not None and 0 <= index < len(self.keys):
            self.on_tab(self.keys[index])
