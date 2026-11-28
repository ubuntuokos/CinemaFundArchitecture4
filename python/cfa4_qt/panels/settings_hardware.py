"""The Qt-only Settings pages of CFA4-SETTINGS-HW-002 (``docs/plans/CFA4-SETTINGS-HW-002.md``).

``arrange`` lays the engine's categories out under the groups of ``prefs.schema``
(``crates/engine/src/settings.rs`` ``groups()``) and adds two pages the engine schema does not
carry: Hardware & Devices ▸ **Overview** first in its group (``settings_overview``), then
**Accelerators (GPU / NPU)** (on the shared device page, ``device_page``), Hardware & Devices ▸
**System Settings (root)**, last in it (``settings_system``, CFA4-SETTINGS-HW-003 R3), and Media &
Playback ▸ **Decoding & Encoding**, after Media. An engine that sends no ``groups`` keeps the flat list.

``HardwarePages`` asks the engine for ``hardware.inventory`` / ``codecs.capabilities`` once per
dialog (Detect Again asks with ``refresh``) and draws the read-only tables. CFA4 changes nothing
on the system here; the decoding page's two fields are the Playback category's own keys. When GPU
access or a driver is missing, both pages link to System Settings (root).

Object names: ``settings.group.<id>`` (the group headings), ``settings.accelerators.devices``,
``settings.accelerators.plans``, ``settings.accelerators.refresh``, ``settings.codecs.table``,
``settings.codecs.hardwareDecoder``, ``settings.codecs.gstreamer``, ``settings.codecs.refresh``,
``settings.system.open`` (the link to System Settings (root)).
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QAbstractItemView,
    QHeaderView,
    QLabel,
    QPushButton,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..hardware_report import WORKLOAD_TITLES, Codecs, Device, Hardware, ReportError, pci_name
from ..i18n import N_, fmt, tr
from .device_page import DeviceEntry, DevicePage
from .settings_overview import OVERVIEW_PAGE, OVERVIEW_TITLE
from .settings_system import SYSTEM_PAGE, SYSTEM_TITLE, SystemPage, open_link

ACCELERATORS_PAGE = "accelerators"
DECODING_PAGE = "decoding"
REPORT_PAGES = frozenset({ACCELERATORS_PAGE, DECODING_PAGE})
QT_ONLY_PAGES = frozenset({OVERVIEW_PAGE, ACCELERATORS_PAGE, DECODING_PAGE, SYSTEM_PAGE})
DECODING_FIELDS = ("playback.hardwareDecoding", "playback.draftDecode")
ACCELERATORS_TITLE = N_("Accelerators (GPU / NPU)")
DECODING_TITLE = N_("Decoding & Encoding")


def _field_rows(rows: Any, keys: tuple[str, ...]) -> list[dict[str, Any]]:
    """The field rows of ``keys`` anywhere in ``rows`` (groups included), in the order of ``keys``."""
    found: dict[str, dict[str, Any]] = {}

    def walk(rs: Any) -> None:
        for r in rs if isinstance(rs, list) else []:
            if not isinstance(r, dict):
                continue
            f = r.get("field")
            if r.get("type") == "field" and isinstance(f, dict) and f.get("key") in keys:
                found.setdefault(str(f["key"]), r)
            elif r.get("type") == "group":
                walk(r.get("rows"))

    walk(rows)
    return [found[k] for k in keys if k in found]


def _decoding_page(playback: dict[str, Any] | None) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    if playback is not None:
        rows += _field_rows(playback.get("rows"), DECODING_FIELDS)
        rows += [r for r in playback.get("rows") or [] if isinstance(r, dict) and r.get("type") == "note"
                 and str(r.get("text") or "").startswith("Hardware decoding")]
    rows.append({"type": "custom", "id": "codecs"})
    return {"id": DECODING_PAGE, "title": DECODING_TITLE, "group": "media", "qtOnly": True, "rows": rows}


def _accelerators_page() -> dict[str, Any]:
    return {"id": ACCELERATORS_PAGE, "title": ACCELERATORS_TITLE, "group": "hardware", "qtOnly": True,
            "rows": [{"type": "custom", "id": "accelerators"}]}


def _overview_page() -> dict[str, Any]:
    return {"id": OVERVIEW_PAGE, "title": OVERVIEW_TITLE, "group": "hardware", "qtOnly": True,
            "rows": [{"type": "custom", "id": OVERVIEW_PAGE}]}


def _system_page() -> dict[str, Any]:
    return {"id": SYSTEM_PAGE, "title": SYSTEM_TITLE, "group": "hardware", "qtOnly": True,
            "rows": [{"type": "custom", "id": SYSTEM_PAGE}]}


def arrange(schema: Any) -> list[tuple[dict[str, Any] | None, list[dict[str, Any]]]]:
    """``[(group, pages)]`` in display order; ``group`` is None for the flat list of an engine
    without groups. A page of a group the schema does not list goes under General."""
    raw = schema if isinstance(schema, dict) else {}
    cats = [c for c in raw.get("categories") or [] if isinstance(c, dict) and isinstance(c.get("id"), str)]
    groups = [g for g in raw.get("groups") or [] if isinstance(g, dict) and isinstance(g.get("id"), str)]
    if not groups:
        return [(None, cats)] if cats else []
    known = {str(g["id"]) for g in groups}
    fallback = "general" if "general" in known else str(groups[0]["id"])

    def group_of(c: dict[str, Any]) -> str:
        g = c.get("group")
        return g if isinstance(g, str) and g in known else fallback

    playback = next((c for c in cats if c.get("id") == "playback"), None)
    out: list[tuple[dict[str, Any] | None, list[dict[str, Any]]]] = []
    for g in groups:
        members = [c for c in cats if group_of(c) == g["id"]]
        if g["id"] == "hardware":
            members[:0] = [_overview_page(), _accelerators_page()]
            members.append(_system_page())
        if g["id"] == "media" and playback is not None:
            after = next((i + 1 for i, c in enumerate(members) if c.get("id") == "media"), len(members))
            members.insert(after, _decoding_page(playback))
        if members:
            out.append((g, members))
    return out


def _read_only(table: QTableWidget) -> None:
    table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
    table.setSelectionMode(QAbstractItemView.SelectionMode.NoSelection)
    table.verticalHeader().setVisible(False)
    table.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.ResizeToContents)
    table.horizontalHeader().setStretchLastSection(True)


def _label(parent: QWidget, text: str, name: str = "", dim: bool = False, bold: bool = False) -> QLabel:
    label = QLabel(text, parent)
    label.setWordWrap(True)
    if name:
        label.setObjectName(name)
    style = []
    if dim:
        style.append(f"color: {theme.current().text_dim}; font-size: 11.5px;")
    if bold:
        style.append("font-weight: bold;")
    if style:
        label.setStyleSheet(" ".join(style))
    return label


def _fit(table: QTableWidget) -> None:
    """Tall enough for every row: the page scrolls, not the table."""
    height = table.horizontalHeader().height() + 4 + sum(table.rowHeight(r) for r in range(table.rowCount()))
    table.setFixedHeight(max(height, 40))
    table.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)


class HardwarePages:
    """The reports behind the two pages. ``engine.call(method, params, on_reply, on_error)`` is the
    engine client's; ``changed(page)`` runs when a report for ``page`` arrived or failed."""

    def __init__(self, engine: Any, changed: Callable[[str], None], system: SystemPage | None = None,
                 open_page: Callable[[str], None] | None = None) -> None:
        self.engine = engine
        self.changed = changed
        self.system = system
        self.open_page = open_page
        # the Accelerators device page's selection, kept across rebuilds
        self.selected_device = ""
        self.selected_tab = ""
        self.hardware: Hardware | None = None
        self.hardware_error = ""
        self.codecs: Codecs | None = None
        self.codecs_error = ""
        self._asked: set[str] = set()

    # ----- the engine -----
    def load(self, page: str, refresh: bool = False) -> None:
        """Ask for the report ``page`` shows, once (again with ``refresh``)."""
        if page not in REPORT_PAGES or (page in self._asked and not refresh):
            return
        self._asked.add(page)
        if self.system is not None:
            # what the System Settings (root) page would fix decides the link on this page
            self.system.load()
        method = "hardware.inventory" if page == ACCELERATORS_PAGE else "codecs.capabilities"
        if refresh:
            if page == ACCELERATORS_PAGE:
                self.hardware, self.hardware_error = None, ""
            else:
                self.codecs, self.codecs_error = None, ""
            self.changed(page)
        call = getattr(self.engine, "call", None)
        if call is None:
            self._failed(page, tr("no answer"))
            return
        call(method, {"refresh": True} if refresh else {},
             lambda r, _b=None, p=page: self._loaded(p, r), lambda m, p=page: self._failed(p, str(m)))

    def _loaded(self, page: str, result: Any) -> None:
        try:
            if page == ACCELERATORS_PAGE:
                self.hardware, self.hardware_error = Hardware.parse(result), ""
            else:
                self.codecs, self.codecs_error = Codecs.parse(result), ""
        except ReportError as e:
            self._failed(page, str(e))
            return
        self.changed(page)

    def _failed(self, page: str, message: str) -> None:
        if page == ACCELERATORS_PAGE:
            self.hardware, self.hardware_error = None, message or tr("no answer")
        else:
            self.codecs, self.codecs_error = None, message or tr("no answer")
        self.changed(page)

    def _refresh_button(self, parent: QWidget, name: str, page: str) -> QPushButton:
        b = QPushButton(tr("Detect Again"), parent)
        b.setObjectName(name)
        b.setAutoDefault(False)
        b.clicked.connect(lambda _c=False: self.load(page, refresh=True))
        return b

    # ----- Hardware & Devices ▸ Accelerators (on the shared device page, HW-003 R4) -----
    @staticmethod
    def dev_path(node: str) -> str:
        """``drm:renderD128`` → ``/dev/dri/renderD128``; ``accel:accel0`` → ``/dev/accel/accel0``."""
        kind, _sep, name = node.partition(":")
        return {"drm": f"/dev/dri/{name}", "accel": f"/dev/accel/{name}"}.get(kind, node)

    def _access_line(self, d: Device) -> str:
        """The system helper's GPU-access fact for this device's node, when it was checked."""
        report = self.system.report if self.system is not None else None
        comp = next((c for c in report.components if c.id == "gpu-access"), None) if report else None
        path = self.dev_path(d.node)
        return next((line for line in comp.details if line.startswith(path + ":")), "") if comp else ""

    def _status(self, d: Device) -> str:
        line = self._access_line(d)
        if line.endswith(": accessible"):
            return tr("OK")
        return tr("No access") if line else ""

    def accelerators(self, parent: QWidget) -> QWidget:
        box = QWidget(parent)
        col = QVBoxLayout(box)
        col.setContentsMargins(0, 0, 0, 0)
        col.setSpacing(8)
        col.addWidget(_label(box, tr("Read-only: CFA4 lists the GPUs and NPUs the system reports and changes nothing here. Every job also runs on the CPU when no accelerator can take it."), dim=True))
        col.addWidget(self._refresh_button(box, "settings.accelerators.refresh", ACCELERATORS_PAGE), 0, Qt.AlignmentFlag.AlignLeft)
        hw = self.hardware
        if hw is None:
            text = fmt("Detection failed: {error}", error=self.hardware_error) if self.hardware_error else tr("Detecting…")
            col.addWidget(_label(box, text, "settings.accelerators.status"))
            return box
        col.addWidget(_label(box, tr("Devices") if hw.devices else tr("No GPU or NPU found: everything runs on the CPU."),
                             "settings.accelerators.status", bold=bool(hw.devices)))
        by_key = {d.node: d for d in hw.devices}
        entries = [DeviceEntry(d.node, pci_name(d.pci_vendor, d.pci_device) or d.name or tr("Unknown device"), self._status(d)) for d in hw.devices]

        def details(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            d = by_key.get(e.key) if e else None
            if d is None:
                v.addWidget(_label(w, tr("No GPU or NPU found: everything runs on the CPU.")))
            else:
                for title, value in ((tr("Type"), d.kind.upper()), (tr("Device"), e.title if e else ""), (tr("PCI ID"), d.pci_id),
                                     (tr("Vendor"), d.vendor), (tr("Node"), f"{d.node} ({self.dev_path(d.node)})")):
                    v.addWidget(_label(w, f"{title}: {value}"))
            v.addStretch(1)
            return w

        def capabilities(p: QWidget, _e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            if hw.devices:
                v.addWidget(self._devices(w, hw))
            v.addWidget(_label(w, tr("Acceleration per job"), bold=True))
            v.addWidget(self._plans(w, hw))
            v.addStretch(1)
            return w

        def drivers(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            d = by_key.get(e.key) if e else None
            if d is not None:
                v.addWidget(_label(w, fmt("Kernel driver: {driver}", driver=d.driver or "—")))
            report = self.system.report if self.system is not None else None
            vaapi = next((c for c in report.components if c.id == "vaapi-drivers"), None) if report else None
            for line in vaapi.details if vaapi else []:
                if d is None or not d.name or d.name in line or "NVDEC" in line:
                    v.addWidget(_label(w, line, dim=True))
            if self.codecs is not None:
                name = self.codecs.hardware_decoder.name
                v.addWidget(_label(w, fmt("Hardware decoder: {name}", name=name) if name else tr("No hardware decoder in use.")))
            v.addStretch(1)
            return w

        def diagnostics(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            d = by_key.get(e.key) if e else None
            line = self._access_line(d) if d is not None else ""
            v.addWidget(_label(w, line or tr("Access not checked (System Settings (root) checks it).")))
            for note in hw.notes:
                v.addWidget(_label(w, note, dim=True))
            v.addStretch(1)
            return w

        def remember(key: str) -> None:
            self.selected_device = key

        def remember_tab(key: str) -> None:
            self.selected_tab = key

        page = DevicePage(box, "settings.accelerators.page", entries,
                          {"details": details, "capabilities": capabilities, "drivers": drivers, "diagnostics": diagnostics},
                          self.selected_device, remember, self.selected_tab or ("details" if entries else "capabilities"), remember_tab)
        page.setMinimumHeight(420)
        col.addWidget(page)
        self._link(box, col, False)
        return box

    def _link(self, box: QWidget, col: QVBoxLayout, missing: bool) -> None:
        """The link to System Settings (root) when access or a driver is missing."""
        if self.open_page is not None and (missing or (self.system is not None and self.system.needs_attention())):
            col.addWidget(open_link(box, self.open_page), 0, Qt.AlignmentFlag.AlignLeft)

    @staticmethod
    def _plans(box: QWidget, hw: Hardware) -> QTableWidget:
        plans = QTableWidget(len(hw.plans), 3, box)
        plans.setObjectName("settings.accelerators.plans")
        plans.setHorizontalHeaderLabels([tr("Job"), tr("Tried in this order"), tr("Not used")])
        for row, p in enumerate(hw.plans):
            plans.setItem(row, 0, QTableWidgetItem(tr(WORKLOAD_TITLES.get(p.workload, p.workload))))
            plans.setItem(row, 1, QTableWidgetItem(" → ".join(p.chain)))
            skipped = QTableWidgetItem(", ".join(b for b, _ in p.skipped) or "—")
            skipped.setToolTip("\n".join(f"{b}: {tr(why)}" for b, why in p.skipped))
            plans.setItem(row, 2, skipped)
        _read_only(plans)
        _fit(plans)
        return plans

    @staticmethod
    def _devices(box: QWidget, hw: Hardware) -> QTableWidget:
        devices = QTableWidget(len(hw.devices), 6, box)
        devices.setObjectName("settings.accelerators.devices")
        devices.setHorizontalHeaderLabels([tr("Type"), tr("Device"), tr("PCI ID"), tr("Vendor"), tr("Driver"), tr("Node")])
        for row, d in enumerate(hw.devices):
            name = pci_name(d.pci_vendor, d.pci_device) or d.name or tr("Unknown device")
            for c, text in enumerate([d.kind.upper(), name, d.pci_id, d.vendor, d.driver or "—", d.node]):
                devices.setItem(row, c, QTableWidgetItem(text))
        _read_only(devices)
        _fit(devices)
        return devices

    # ----- Media & Playback ▸ Decoding & Encoding -----
    def codecs_section(self, parent: QWidget) -> QWidget:
        box = QWidget(parent)
        col = QVBoxLayout(box)
        col.setContentsMargins(0, 8, 0, 0)
        col.setSpacing(8)
        col.addWidget(_label(box, tr("On this machine"), bold=True))
        col.addWidget(_label(box, tr("Read-only: what CFA4 decodes and encodes here. Its own software codecs always work; a hardware decoder takes the streams it supports while Hardware decoding is on, and hands back to software on any error."), dim=True))
        col.addWidget(self._refresh_button(box, "settings.codecs.refresh", DECODING_PAGE), 0, Qt.AlignmentFlag.AlignLeft)
        c = self.codecs
        if c is None:
            text = fmt("Detection failed: {error}", error=self.codecs_error) if self.codecs_error else tr("Detecting…")
            col.addWidget(_label(box, text, "settings.codecs.status"))
            return box
        if c.hardware_decoder.name:
            hw_text = fmt("Hardware decoder: {name}", name=c.hardware_decoder.name)
        else:
            hw_text = fmt("Hardware decoder: none ({reason}). Install your GPU vendor's driver (NVIDIA: the proprietary driver; AMD / Intel: the VA-API driver) to decode H.264 and HEVC in hardware.",
                          reason=c.hardware_decoder.reason or tr("not found"))
        col.addWidget(_label(box, hw_text, "settings.codecs.hardwareDecoder"))
        if c.gstreamer.name:
            gst_text = fmt("GStreamer backend: {version} (fallback for files CFA4 does not open itself)", version=c.gstreamer.name)
        elif c.gstreamer_compiled_in:
            gst_text = fmt("GStreamer backend: not available ({reason})", reason=c.gstreamer.reason or tr("not installed"))
        else:
            gst_text = tr("GStreamer backend: not part of this build")
        col.addWidget(_label(box, gst_text, "settings.codecs.gstreamer"))
        table = QTableWidget(len(c.rows), 5, box)
        table.setObjectName("settings.codecs.table")
        table.setHorizontalHeaderLabels([tr("Codec"), tr("Decode"), tr("Hardware decode"), tr("Encode"), tr("Notes")])
        words = {"software": tr("Software"), "hardware": tr("Hardware only"), "none": "—"}
        for row, r in enumerate(c.rows):
            cells = [r.codec, words.get(r.decode, r.decode), r.hardware_decode or "—", words.get(r.encode, r.encode), r.decode_note]
            for col_i, text in enumerate(cells):
                item = QTableWidgetItem(text)
                if col_i == 3 and r.export_format and r.encode != "none":
                    item.setToolTip(fmt("Export ▸ {format}", format=r.export_format))
                if col_i in (0, 4) and text:
                    item.setToolTip(text)
                table.setItem(row, col_i, item)
        _read_only(table)
        header = table.horizontalHeader()
        # the page has a fixed width: the notes column takes what is left and elides (full text in the tooltip)
        table.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        table.setWordWrap(False)
        table.setTextElideMode(Qt.TextElideMode.ElideRight)
        header.setStretchLastSection(False)
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.Interactive)
        header.resizeSection(0, 170)
        for i in range(1, 4):
            header.setSectionResizeMode(i, QHeaderView.ResizeMode.ResizeToContents)
        header.setSectionResizeMode(4, QHeaderView.ResizeMode.Stretch)
        _fit(table)
        col.addWidget(table)
        self._link(box, col, not c.hardware_decoder.name)
        return box
