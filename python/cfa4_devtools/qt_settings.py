"""The Qt 6 Settings window (Edit ▸ Settings…, Ctrl+,).

Pages come from the Rust engine's settings schema (`cfa4-cli settings schema`), grouped as in the
CFA3 / FA3 Control Center design: General, Appearance, Editing, Media & Playback, Hardware &
Devices, Local AI. Hardware & Devices starts with the Accelerators page, a read-only view of the
GPUs and NPUs the engine found (`cfa4-cli hardware`).

Every CLI call runs on a QThreadPool worker and reports back through signals (ADR F8); the GUI
thread never waits on a process. Edits stay in the window until OK, which saves only the changed
keys through the engine's validated `prefs.set`.

Automation ids (QObject names): `settings.category.<page>`, `settings.<key>` for every control,
`settings.<unitKey>` for a duration's unit, `settings.ok`, `settings.cancel`, `settings.reset`,
`settings.accelerators.devices`, `settings.accelerators.plans`, `settings.accelerators.refresh`.
"""
from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Any

from PySide6.QtCore import QObject, QRunnable, Qt, QThreadPool, Signal
from PySide6.QtGui import QColor, QFont
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFileDialog,
    QGridLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QMessageBox,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSplitter,
    QStackedWidget,
    QTableWidget,
    QTableWidgetItem,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from cfa4_devtools.settings_backend import (
    ACCELERATORS_PAGE,
    WORKLOAD_TITLES,
    Field,
    Hardware,
    Page,
    Schema,
    SettingsBackendError,
    changed_values,
    duration_limits,
    pci_name,
    run_cli,
    run_hardware,
    run_load,
    run_save,
)

INT_LIMIT = 2_147_483_647
PAGE_ROLE = Qt.ItemDataRole.UserRole


class _Signals(QObject):
    done = Signal(object)
    failed = Signal(str)


class _Job(QRunnable):
    """One backend call off the GUI thread; the result or the error comes back as a signal."""

    def __init__(self, work: Callable[[], Any]):
        super().__init__()
        self.work = work
        self.signals = _Signals()

    def run(self) -> None:
        try:
            result = self.work()
        except SettingsBackendError as exc:
            self.signals.failed.emit(str(exc))
        except Exception as exc:  # noqa: BLE001 — never let a worker die silently (AGENTS.md §0)
            self.signals.failed.emit(f"unexpected error: {exc}")
        else:
            self.signals.done.emit(result)


class SettingsDialog(QDialog):
    def __init__(self, cli: Path | None, data_dir: Path | None = None, parent: QWidget | None = None):
        super().__init__(parent)
        self.setWindowTitle("Settings")
        self.setObjectName("settings.window")
        self.resize(940, 640)
        self.cli = cli
        self.data_dir = data_dir
        self.pool = QThreadPool(self)
        self.jobs: set[_Job] = set()
        self.schema: Schema | None = None
        self.saved: dict[str, Any] = {}
        self.edited: dict[str, Any] = {}
        self.hardware: Hardware | None = None
        self.hardware_error = ""
        self.page_widgets: dict[str, QWidget] = {}
        self.enablers: dict[str, list[QWidget]] = {}

        self.tree = QTreeWidget()
        self.tree.setObjectName("settings.categories")
        self.tree.setHeaderHidden(True)
        self.tree.setMinimumWidth(220)
        self.tree.currentItemChanged.connect(self._show_page)
        self.stack = QStackedWidget()
        self.stack.setObjectName("settings.page")
        splitter = QSplitter()
        splitter.addWidget(self.tree)
        splitter.addWidget(self.stack)
        splitter.setStretchFactor(1, 1)

        self.status = QLabel("Loading settings…")
        self.status.setObjectName("settings.status")
        self.status.setWordWrap(True)
        self.reset_button = self._button("settings.reset", "Reset Page…", self._reset_page)
        self.cancel_button = self._button("settings.cancel", "Cancel", self.reject)
        self.ok_button = self._button("settings.ok", "OK", self._save)
        self.ok_button.setDefault(True)
        self.ok_button.setEnabled(False)
        self.reset_button.setEnabled(False)
        buttons = QHBoxLayout()
        buttons.addWidget(self.reset_button)
        buttons.addWidget(self.status, 1)
        buttons.addWidget(self.cancel_button)
        buttons.addWidget(self.ok_button)

        layout = QVBoxLayout(self)
        layout.addWidget(splitter, 1)
        layout.addLayout(buttons)

        if cli is None:
            self.status.setText("The Rust engine CLI is not built (cargo build --release -p cfa4-cli).")
        else:
            self._load()

    # ------------------------------------------------------------------ background work

    def _run(self, work: Callable[[], Any], done: Callable[[Any], None], failed: Callable[[str], None]) -> None:
        job = _Job(work)
        self.jobs.add(job)
        job.setAutoDelete(False)

        def finish(handler: Callable[[Any], None], value: Any) -> None:
            self.jobs.discard(job)
            handler(value)

        job.signals.done.connect(lambda r: finish(done, r))
        job.signals.failed.connect(lambda e: finish(failed, e))
        self.pool.start(job)

    def _load(self) -> None:
        cli, data_dir = self.cli, self.data_dir
        self._run(lambda: run_load(cli, data_dir), self._loaded, self._load_failed)
        self._refresh_hardware()

    def _refresh_hardware(self) -> None:
        cli = self.cli
        if cli is None:
            return
        self._run(lambda: run_hardware(cli), self._hardware_loaded, self._hardware_failed)

    def _loaded(self, result: Any, pending: dict[str, Any] | None = None, page: str | None = None) -> None:
        """Show freshly loaded settings; `pending` are unsaved edits to keep, `page` the page to reopen."""
        schema, prefs = result
        self.schema = schema
        self.saved = schema.values(prefs)
        self.edited = dict(self.saved)
        self.edited.update({k: v for k, v in (pending or {}).items() if k in self.saved})
        self._build(page)
        self.status.setText("")
        self.ok_button.setEnabled(True)
        self.reset_button.setEnabled(True)

    def _load_failed(self, message: str) -> None:
        self.status.setText(f"Could not read the settings: {message}")

    def _hardware_loaded(self, hardware: Any) -> None:
        self.hardware, self.hardware_error = hardware, ""
        self._rebuild_accelerators()

    def _hardware_failed(self, message: str) -> None:
        self.hardware, self.hardware_error = None, message
        self._rebuild_accelerators()

    # ------------------------------------------------------------------ building pages

    def _button(self, name: str, text: str, slot: Callable[[], None]) -> QPushButton:
        b = QPushButton(text)
        b.setObjectName(name)
        b.clicked.connect(slot)
        return b

    def _build(self, select: str | None = None) -> None:
        if self.schema is None:
            return
        self.tree.clear()
        self.page_widgets.clear()
        self.enablers.clear()
        while self.stack.count():
            old = self.stack.widget(0)
            self.stack.removeWidget(old)
            old.deleteLater()
        first: QTreeWidgetItem | None = None
        for group in self.schema.groups:
            head = QTreeWidgetItem([group.title])
            head.setFlags(Qt.ItemFlag.ItemIsEnabled)
            font = QFont(head.font(0))
            font.setBold(True)
            head.setFont(0, font)
            self.tree.addTopLevelItem(head)
            for page in group.pages:
                item = QTreeWidgetItem([page.title])
                item.setData(0, PAGE_ROLE, page.id)
                head.addChild(item)
                widget = self._accelerators_page() if page.id == ACCELERATORS_PAGE else self._fields_page(page)
                widget.setObjectName(f"settings.category.{page.id}")
                self.page_widgets[page.id] = widget
                self.stack.addWidget(widget)
                if first is None or page.id == select:
                    first = item
            head.setExpanded(True)
        if first is not None:
            self.tree.setCurrentItem(first)

    def _show_page(self, item: QTreeWidgetItem | None, _previous: QTreeWidgetItem | None = None) -> None:
        page = item.data(0, PAGE_ROLE) if item is not None else None
        widget = self.page_widgets.get(page) if isinstance(page, str) else None
        if widget is not None:
            self.stack.setCurrentWidget(widget)
            self.reset_button.setEnabled(page != ACCELERATORS_PAGE and self.schema is not None)

    def current_page(self) -> str | None:
        item = self.tree.currentItem()
        page = item.data(0, PAGE_ROLE) if item is not None else None
        return page if isinstance(page, str) else None

    def _fields_page(self, page: Page) -> QWidget:
        body = QWidget()
        # a grid, not a QFormLayout: it gives wrapped labels their full height next to small controls
        grid = QGridLayout(body)
        grid.setColumnStretch(0, 3)
        grid.setColumnStretch(1, 2)
        grid.setVerticalSpacing(8)
        for row, f in enumerate(page.fields):
            control = self._control(f)
            control.setObjectName(f"settings.{f.key}")
            label = QLabel(f.label + ("" if f.wired else "  ·  no effect yet"))
            label.setWordWrap(True)
            label.setBuddy(control)
            if not f.wired:
                label.setToolTip("Saved for compatibility; CFA4 does not act on this setting yet.")
                label.setEnabled(False)
            grid.addWidget(label, row, 0)
            grid.addWidget(control, row, 1, Qt.AlignmentFlag.AlignVCenter)
            if f.enabled_by:
                self.enablers.setdefault(f.enabled_by, []).append(control)
                control.setEnabled(self.edited.get(f.enabled_by) is True)
        if not page.fields:
            grid.addWidget(QLabel("This page has no settings in the Qt window yet."), 0, 0)
        grid.setRowStretch(max(len(page.fields), 1), 1)
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setWidget(body)
        return scroll

    def _set(self, key: str, value: Any) -> None:
        self.edited[key] = value
        for w in self.enablers.get(key, []):
            w.setEnabled(value is True)

    def _control(self, f: Field) -> QWidget:
        value = self.edited.get(f.key)
        if f.kind == "bool":
            box = QCheckBox()
            box.setChecked(value is True)
            box.toggled.connect(lambda on, k=f.key: self._set(k, bool(on)))
            return box
        if f.kind == "int":
            spin = QSpinBox()
            lo = int(max(f.minimum if f.minimum is not None else 0, -INT_LIMIT))
            hi = int(min(f.maximum if f.maximum is not None else INT_LIMIT, INT_LIMIT))
            spin.setRange(lo, max(lo, hi))
            spin.setValue(int(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else lo)
            if f.unit:
                spin.setSuffix(f" {f.unit}")
            spin.valueChanged.connect(lambda v, k=f.key: self._set(k, int(v)))
            return spin
        if f.kind in ("float", "duration"):
            return self._number(f, value)
        if f.kind in ("choice", "label"):
            combo = QComboBox()
            for c in f.choices:
                combo.addItem(c.label, c.value)
            current = "" if value is None else str(value)
            index = combo.findData(current)
            if index < 0 and current:
                combo.addItem(current, current)  # a saved value the schema no longer lists
                index = combo.count() - 1
            combo.setCurrentIndex(max(index, 0))
            original_is_number = isinstance(value, (int, float)) and not isinstance(value, bool)

            def chosen(_i: int, k: str = f.key, box: QComboBox = combo, numeric: bool = original_is_number) -> None:
                data = box.currentData()
                self._set(k, int(data) if numeric and isinstance(data, str) and data.isdigit() else data)

            combo.currentIndexChanged.connect(chosen)
            return combo
        if f.kind == "path":
            return self._path(f, value)
        if f.kind == "color":
            return self._color(f, value)
        if f.kind in ("text", "font", "device"):
            line = QLineEdit("" if value is None else str(value))
            line.textChanged.connect(lambda t, k=f.key: self._set(k, t))
            return line
        shown = QLabel("" if value is None else str(value))
        shown.setToolTip("Edited in the editor; shown here read-only.")
        return shown

    def _number(self, f: Field, value: Any) -> QWidget:
        spin = QDoubleSpinBox()
        number = float(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else None
        if f.kind != "duration" or not f.unit_key:
            spin.setDecimals(f.decimals)
            lo = f.minimum if f.minimum is not None else 0.0
            hi = f.maximum if f.maximum is not None else 1_000_000.0
            spin.setRange(lo, max(lo, hi))
            spin.setValue(number if number is not None else lo)
            if f.unit:
                spin.setSuffix(f" {f.unit}")
            integral = isinstance(value, int) and not isinstance(value, bool)
            spin.valueChanged.connect(lambda v, k=f.key, i=integral: self._set(k, round(v) if i else float(v)))
            return spin
        unit = QComboBox()
        unit.setObjectName(f"settings.{f.unit_key}")
        unit.addItem("Frames", "frames")
        unit.addItem("Seconds", "seconds")
        unit.setCurrentIndex(max(unit.findData(self.edited.get(f.unit_key)), 0))

        def limit(unit_value: Any) -> None:
            """Range and precision of the engine's sanitize for this unit (whole frames)."""
            lo, hi, decimals = duration_limits(unit_value)
            spin.setDecimals(decimals)
            spin.setRange(lo, hi)  # clamps the shown value; valueChanged records it

        def changed(v: float, k: str = f.key, box: QComboBox = unit) -> None:
            self._set(k, round(v) if box.currentData() == "frames" else float(v))

        limit(unit.currentData())
        spin.setValue(number if number is not None else spin.minimum())
        spin.valueChanged.connect(changed)

        def unit_changed(_i: int, k: str = f.unit_key) -> None:
            self._set(k, unit.currentData())
            limit(unit.currentData())
            changed(spin.value())

        unit.currentIndexChanged.connect(unit_changed)
        row = QWidget()
        h = QHBoxLayout(row)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(spin, 1)
        h.addWidget(unit)
        return row

    def _path(self, f: Field, value: Any) -> QWidget:
        line = QLineEdit("" if value is None else str(value))
        line.setPlaceholderText("Default location")
        line.textChanged.connect(lambda t, k=f.key: self._set(k, t))
        browse = QPushButton("Browse…")
        browse.setObjectName(f"settings.{f.key}.browse")

        def pick() -> None:
            folder = QFileDialog.getExistingDirectory(self, f.label, line.text())
            if folder:
                line.setText(folder)

        browse.clicked.connect(pick)
        row = QWidget()
        h = QHBoxLayout(row)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(line, 1)
        h.addWidget(browse)
        return row

    def _color(self, f: Field, value: Any) -> QWidget:
        button = QPushButton()

        def paint(hex_color: str) -> None:
            button.setText(hex_color)
            button.setStyleSheet(f"background-color: {hex_color};" if QColor(hex_color).isValid() else "")

        paint(value if isinstance(value, str) else "")

        def pick() -> None:
            color = QColorDialog.getColor(QColor(self.edited.get(f.key) or "#000000"), self, f.label)
            if color.isValid():
                paint(color.name())
                self._set(f.key, color.name())

        button.clicked.connect(pick)
        return button

    # ------------------------------------------------------------------ accelerators

    def _accelerators_page(self) -> QWidget:
        page = QWidget()
        self.accel_layout = QVBoxLayout(page)
        self._fill_accelerators()
        return page

    def _rebuild_accelerators(self) -> None:
        if ACCELERATORS_PAGE not in self.page_widgets:
            return
        while self.accel_layout.count():
            item = self.accel_layout.takeAt(0)
            if item is not None and item.widget() is not None:
                item.widget().deleteLater()
        self._fill_accelerators()

    def _fill_accelerators(self) -> None:
        lay = self.accel_layout
        intro = QLabel(
            "Read-only: CFA4 lists the GPUs and NPUs the system reports and changes nothing here. "
            "Every job also runs on the CPU when no accelerator can take it."
        )
        intro.setWordWrap(True)
        lay.addWidget(intro)
        refresh = self._button("settings.accelerators.refresh", "Detect Again", self._refresh_hardware)
        lay.addWidget(refresh, 0, Qt.AlignmentFlag.AlignLeft)
        if self.hardware is None:
            lay.addWidget(QLabel(f"Detection failed: {self.hardware_error}" if self.hardware_error else "Detecting…"))
            lay.addStretch(1)
            return
        devices = QTableWidget(len(self.hardware.devices), 6)
        devices.setObjectName("settings.accelerators.devices")
        devices.setHorizontalHeaderLabels(["Type", "Device", "PCI ID", "Vendor", "Driver", "Node"])
        for row, d in enumerate(self.hardware.devices):
            name = pci_name(d.pci_vendor, d.pci_device) or d.name or "Unknown device"
            for col, text in enumerate([d.kind.upper(), name, d.pci_id, d.vendor, d.driver or "—", d.node]):
                devices.setItem(row, col, QTableWidgetItem(text))
        self._read_only(devices)
        lay.addWidget(QLabel("Devices" if self.hardware.devices else "No GPU or NPU found: everything runs on the CPU."))
        lay.addWidget(devices)
        plans = QTableWidget(len(self.hardware.plans), 3)
        plans.setObjectName("settings.accelerators.plans")
        plans.setHorizontalHeaderLabels(["Job", "Tried in this order", "Not used"])
        for row, p in enumerate(self.hardware.plans):
            plans.setItem(row, 0, QTableWidgetItem(WORKLOAD_TITLES.get(p.workload, p.workload)))
            plans.setItem(row, 1, QTableWidgetItem(" → ".join(p.chain)))
            skipped = QTableWidgetItem(", ".join(b for b, _ in p.skipped) or "—")
            skipped.setToolTip("\n".join(f"{b}: {why}" for b, why in p.skipped))
            plans.setItem(row, 2, skipped)
        self._read_only(plans)
        lay.addWidget(QLabel("Acceleration per job"))
        lay.addWidget(plans)
        for note in self.hardware.notes:
            lay.addWidget(QLabel(note))
        lay.addStretch(1)

    @staticmethod
    def _read_only(table: QTableWidget) -> None:
        table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        table.verticalHeader().setVisible(False)
        table.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.ResizeToContents)
        table.horizontalHeader().setStretchLastSection(True)

    # ------------------------------------------------------------------ OK / reset

    def changes(self) -> dict[str, Any]:
        return changed_values(self.saved, self.edited)

    def _busy(self, busy: bool, message: str = "") -> None:
        """While a save or reset runs, nothing can be edited, saved, reset or closed."""
        for w in (self.tree, self.stack, self.ok_button, self.cancel_button):
            w.setEnabled(not busy)
        self.reset_button.setEnabled(not busy and self.current_page() not in (None, ACCELERATORS_PAGE))
        self.status.setText(message)

    def reject(self) -> None:
        if not self.cancel_button.isEnabled() and self.schema is not None:
            return  # Escape / window close during a save or reset
        super().reject()

    def _save(self) -> None:
        changes = self.changes()
        if not changes or self.cli is None:
            self.accept()
            return
        cli, data_dir = self.cli, self.data_dir
        self._busy(True, "Saving…")
        self._run(lambda: run_save(cli, changes, data_dir), lambda _r: self.accept(), self._save_failed)

    def _save_failed(self, message: str) -> None:
        self._busy(False, f"Not saved: {message}")

    def _reset_page(self) -> None:
        page = self.current_page()
        if self.cli is None or self.schema is None or page is None or page == ACCELERATORS_PAGE:
            return
        answer = QMessageBox.question(
            self, "Reset Page", "Set every setting on this page back to its default now? Other unsaved edits are kept.",
        )
        if answer != QMessageBox.StandardButton.Yes:
            return
        cli, data_dir = self.cli, self.data_dir
        # editing is off until the reset finishes, so this snapshot stays complete
        page_keys = self.schema.page_keys(page)
        pending = {k: v for k, v in self.changes().items() if k not in page_keys}
        self._busy(True, "Resetting…")

        def work() -> Any:
            run_cli(cli, ["settings", "reset", page], data_dir=data_dir)
            return run_load(cli, data_dir)

        def done(result: Any) -> None:
            self._busy(False)
            self._loaded(result, pending, page)

        self._run(work, done, lambda e: self._busy(False, f"Not reset: {e}"))
