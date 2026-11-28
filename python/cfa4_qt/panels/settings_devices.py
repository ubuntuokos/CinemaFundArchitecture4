"""Hardware & Devices ▸ **Displays** and **Storage Devices** (``docs/plans/CFA4-SETTINGS-HW-004.md``
R1–R2), on the shared device page (``device_page.py``). Both are read-only: the lists come from Qt
(``QGuiApplication.screens()``, ``QStorageInfo.mountedVolumes()``, plan decision D3), the CFA4
locations from the engine's ``mediaCache.info`` and ``file.autoSaveStatus``. CFA4 changes nothing in
the system here; the settings that belong to these devices stay on their pages (Color, Media Cache,
decision D4) and are linked.

Object names: ``settings.displays.page`` / ``settings.storage.page`` (with the device page's
``.list``, ``.tabs``, ``.tab.<key>``), ``settings.displays.open.color``,
``settings.storage.open.mediaCache``, ``settings.displays.status``, ``settings.storage.status``.
"""

from __future__ import annotations

import os
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from PySide6.QtCore import QStorageInfo, Qt
from PySide6.QtGui import QGuiApplication
from PySide6.QtWidgets import QLabel, QPushButton, QVBoxLayout, QWidget

from .. import theme
from ..i18n import N_, fmt, tr
from .device_page import DeviceEntry, DevicePage

DISPLAYS_PAGE = "displays"
STORAGE_PAGE = "storage"
DISPLAYS_TITLE = N_("Displays")
STORAGE_TITLE = N_("Storage Devices")
GB = 1 << 30
# file systems that are not storage a user picks for media (kernel, memory, containers, snaps)
PSEUDO_FS = frozenset({"proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "ramfs", "cgroup", "cgroup2", "securityfs", "pstore", "bpf",
                       "debugfs", "tracefs", "configfs", "fusectl", "mqueue", "hugetlbfs", "autofs", "binfmt_misc", "efivarfs",
                       "squashfs", "overlay", "nsfs", "rpc_pipefs", "fuse.portal", "fuse.gvfsd-fuse"})
LOG_LINES = 20


def _label(parent: QWidget, text: str, name: str = "", dim: bool = False, bold: bool = False) -> QLabel:
    label = QLabel(text, parent)
    label.setWordWrap(True)
    label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
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


def _link(parent: QWidget, text: str, name: str, page: str, open_page: Callable[[str], None] | None) -> QPushButton:
    b = QPushButton(text, parent)
    b.setObjectName(name)
    b.setAutoDefault(False)
    b.setFlat(True)
    b.setStyleSheet(f"color: {theme.current().accent}; text-align: left;")
    b.setEnabled(open_page is not None)
    if open_page is not None:
        b.clicked.connect(lambda _c=False: open_page(page))
    return b


def _gb(n: int) -> str:
    return fmt("{n} GB", n=f"{n / GB:.1f}")


def _stamp() -> str:
    return datetime.now().strftime("%H:%M:%S")


# ----- displays -----
@dataclass(frozen=True)
class Display:
    """One screen as Qt reports it; ``width`` × ``height`` in device pixels."""

    key: str
    name: str
    manufacturer: str = ""
    model: str = ""
    serial: str = ""
    width: int = 0
    height: int = 0
    refresh: float = 0.0
    scale: float = 1.0
    physical_mm: tuple[int, int] = (0, 0)
    dpi: float = 0.0
    x: int = 0
    y: int = 0
    primary: bool = False

    @property
    def title(self) -> str:
        product = " ".join(p for p in (self.manufacturer, self.model) if p).strip()
        return f"{product} ({self.name})" if product and self.name and product != self.name else (product or self.name or tr("Unknown device"))


def screens() -> list[Display]:
    """The screens Qt sees now, the primary one first."""
    app = QGuiApplication.instance()
    if not isinstance(app, QGuiApplication):
        return []
    primary = QGuiApplication.primaryScreen()
    out: list[Display] = []
    for i, s in enumerate(QGuiApplication.screens()):
        g = s.geometry()
        scale = float(s.devicePixelRatio() or 1.0)
        phys = s.physicalSize()
        out.append(Display(key=f"{i}:{s.name()}", name=s.name(), manufacturer=s.manufacturer(), model=s.model(),
                           serial=s.serialNumber(), width=round(g.width() * scale), height=round(g.height() * scale),
                           refresh=float(s.refreshRate()), scale=scale, physical_mm=(round(phys.width()), round(phys.height())),
                           dpi=float(s.physicalDotsPerInch()), x=g.x(), y=g.y(), primary=s is primary))
    out.sort(key=lambda d: not d.primary)
    return out


class DisplaysPage:
    """``source()`` gives the displays (Qt's by default); ``color_on()`` whether Color ▸ Display
    Color Management is on; ``open_page(id)`` opens a Settings page."""

    def __init__(self, open_page: Callable[[str], None] | None = None, color_on: Callable[[], bool] | None = None,
                 source: Callable[[], list[Display]] = screens) -> None:
        self.open_page = open_page
        self.color_on = color_on
        self.source = source
        self.log: list[str] = []
        self.selected = ""
        self.tab = ""
        self._last: tuple[str, ...] | None = None

    def displays(self) -> list[Display]:
        found = self.source()
        keys = tuple(d.key for d in found)
        if keys != self._last:
            # a detection that found something new goes into the Diagnostics log
            self._last = keys
            self.log = (self.log + [fmt("{time} Detected {count} display(s).", time=_stamp(), count=len(found))])[-LOG_LINES:]
        return found

    def summary(self) -> tuple[str, list[str]]:
        found = self.displays()
        if not found:
            return tr("No display found."), []
        primary = next((d for d in found if d.primary), found[0])
        return fmt("{count} display(s), primary: {name}", count=len(found), name=primary.title), []

    def page(self, parent: QWidget) -> QWidget:
        box = QWidget(parent)
        col = QVBoxLayout(box)
        col.setContentsMargins(0, 0, 0, 0)
        col.setSpacing(8)
        col.addWidget(_label(box, tr("Read-only: the displays the system reports. CFA4 changes nothing in the display settings here."), dim=True))
        found = self.displays()
        col.addWidget(_label(box, tr("Displays") if found else tr("No display found."), "settings.displays.status", bold=bool(found)))
        by_key = {d.key: d for d in found}
        entries = [DeviceEntry(d.key, d.title, tr("Primary") if d.primary else "") for d in found]

        def details(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            d = by_key.get(e.key) if e else None
            if d is None:
                v.addWidget(_label(w, tr("No display found.")))
            else:
                mm = fmt("{w} × {h} mm", w=d.physical_mm[0], h=d.physical_mm[1]) if all(d.physical_mm) else tr("not known")
                for title, value in ((tr("Name"), d.name), (tr("Vendor"), d.manufacturer or "—"), (tr("Model"), d.model or "—"),
                                     (tr("Serial number"), d.serial or "—"), (tr("Resolution"), f"{d.width} × {d.height}"),
                                     (tr("Refresh rate"), fmt("{hz} Hz", hz=f"{d.refresh:.2f}") if d.refresh else tr("not known")),
                                     (tr("Physical size"), mm), (tr("Pixel density"), fmt("{dpi} dpi", dpi=round(d.dpi)) if d.dpi else tr("not known")),
                                     (tr("Scaling"), f"{d.scale:g}×"), (tr("Position"), f"{d.x}, {d.y}")):
                    v.addWidget(_label(w, f"{title}: {value}"))
            v.addStretch(1)
            return w

        def capabilities(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            d = by_key.get(e.key) if e else None
            if d is not None:
                if d.width and d.height:
                    v.addWidget(_label(w, tr("Full-screen playback: yes")))
                v.addWidget(_label(w, fmt("One pixel of the Program Monitor at 100% covers {n} device pixel(s) here.", n=f"{d.scale:g}")))
            on = self.color_on() if self.color_on is not None else False
            v.addWidget(_label(w, tr("Display Color Management") + ": " + (tr("On") if on else tr("Off"))))
            v.addWidget(_label(w, tr("HDR: not known (the system does not report it to CFA4)."), dim=True))
            v.addStretch(1)
            return w

        def settings(p: QWidget, _e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            v.addWidget(_label(w, tr("CFA4's own display settings are on the Color page."), dim=True))
            v.addWidget(_link(w, tr("Color…"), "settings.displays.open.color", "color", self.open_page), 0, Qt.AlignmentFlag.AlignLeft)
            v.addStretch(1)
            return w

        def diagnostics(p: QWidget, _e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            for line in self.log:
                v.addWidget(_label(w, line, dim=True))
            v.addStretch(1)
            return w

        def remember(key: str) -> None:
            self.selected = key

        def remember_tab(key: str) -> None:
            self.tab = key

        page = DevicePage(box, "settings.displays.page", entries,
                          {"details": details, "capabilities": capabilities, "settings": settings, "diagnostics": diagnostics},
                          self.selected, remember, self.tab, remember_tab)
        page.setMinimumHeight(380)
        col.addWidget(page)
        return box


# ----- storage -----
@dataclass(frozen=True)
class Volume:
    """One mounted file system as Qt reports it (sizes in bytes)."""

    root: str
    device: str = ""
    fs: str = ""
    total: int = 0
    free: int = 0
    read_only: bool = False
    name: str = ""

    @property
    def title(self) -> str:
        return self.root if not self.name or self.name == self.root else f"{self.name} ({self.root})"


def volumes() -> list[Volume]:
    """The mounted storage, without kernel and memory file systems."""
    out: list[Volume] = []
    for s in QStorageInfo.mountedVolumes():
        if not s.isValid() or not s.isReady():
            continue
        fs = bytes(s.fileSystemType().data()).decode(errors="replace")
        if fs in PSEUDO_FS or s.bytesTotal() <= 0:
            continue
        out.append(Volume(root=s.rootPath(), device=bytes(s.device().data()).decode(errors="replace"), fs=fs, total=int(s.bytesTotal()),
                          free=int(s.bytesAvailable()), read_only=s.isReadOnly(), name=s.displayName()))
    return out


def low_space_limit(total: int, threshold_gb: float = 0.0) -> int:
    """The free bytes below which a volume warns: the setting when given, else 10 GB or 5%,
    whichever is larger (plan R2)."""
    if threshold_gb > 0:
        return int(threshold_gb * GB)
    return max(10 * GB, total // 20)


def volume_of(path: str, found: list[Volume]) -> Volume | None:
    """The volume ``path`` is on: the one with the longest matching mount point."""
    if not path:
        return None
    real = os.path.realpath(os.path.expanduser(path))
    best: Volume | None = None
    best_len = -1
    for v in found:
        root = v.root.rstrip("/") or "/"
        if (root == "/" or real == root or real.startswith(root + "/")) and len(root) > best_len:
            best, best_len = v, len(root)
    return best


class StoragePage:
    """``engine.execute(command, params, on_reply=…)`` answers ``mediaCache.info`` and
    ``file.autoSaveStatus``; ``changed()`` runs when they arrive; ``threshold_gb()`` the low-space
    warning setting (0 = the default)."""

    def __init__(self, engine: Any, changed: Callable[[], None], open_page: Callable[[str], None] | None = None,
                 threshold_gb: Callable[[], float] | None = None, source: Callable[[], list[Volume]] = volumes) -> None:
        self.engine = engine
        self.changed = changed
        self.open_page = open_page
        self.threshold_gb = threshold_gb
        self.source = source
        self.cache: dict[str, Any] | None = None
        self.auto_save: dict[str, Any] | None = None
        self.log: list[str] = []
        self.selected = ""
        self.tab = ""
        self._asked = False
        self._last: tuple[str, ...] | None = None

    def load(self) -> None:
        """Asks once where the media cache and the auto-saves are."""
        if self._asked:
            return
        self._asked = True
        execute = getattr(self.engine, "execute", None)
        if execute is None:
            self.cache, self.auto_save = {}, {}
            return

        def got_cache(r: Any, _b: Any = None) -> None:
            self.cache = r if isinstance(r, dict) else {}
            self.changed()

        def got_auto(r: Any, _b: Any = None) -> None:
            self.auto_save = r if isinstance(r, dict) else {}
            self.changed()

        execute("mediaCache.info", {}, on_reply=got_cache, on_error=lambda _m: got_cache({}))
        execute("file.autoSaveStatus", {}, on_reply=got_auto, on_error=lambda _m: got_auto({}))

    def locations(self) -> list[tuple[str, str]]:
        """(what, path) of the CFA4 locations known now."""
        out: list[tuple[str, str]] = []
        loc = (self.cache or {}).get("location")
        if isinstance(loc, str) and loc:
            out.append((tr("Media Cache"), loc))
        for key, title in (("autoSaveDir", N_("Auto Save")), ("sessionDir", N_("Recovery copy"))):
            p = (self.auto_save or {}).get(key)
            if isinstance(p, str) and p:
                out.append((tr(title), p))
        return out

    def volumes(self) -> list[Volume]:
        found = self.source()
        keys = tuple(v.root for v in found)
        if keys != self._last:
            self._last = keys
            self.log = (self.log + [fmt("{time} Detected {count} storage device(s).", time=_stamp(), count=len(found))])[-LOG_LINES:]
        return found

    def _limit(self, v: Volume) -> int:
        return low_space_limit(v.total, self.threshold_gb() if self.threshold_gb is not None else 0.0)

    def warnings(self, v: Volume, here: list[tuple[str, str]]) -> list[str]:
        """Warnings for a volume that holds a CFA4 location."""
        if not here:
            return []
        out = []
        if v.free < self._limit(v):
            out.append(fmt("Low on space: {free} free on {root}.", free=_gb(v.free), root=v.root))
        if v.read_only:
            out.append(fmt("{root} is read-only: CFA4 cannot write there.", root=v.root))
        return out

    def summary(self) -> tuple[str, list[str]]:
        found = self.volumes()
        locs = self.locations()
        loc = (self.cache or {}).get("location")
        v = volume_of(loc if isinstance(loc, str) else "", found)
        warn: list[str] = []
        for vol in found:
            warn += self.warnings(vol, [x for x in locs if volume_of(x[1], found) is vol])
        if v is not None:
            return fmt("{free} free on the media cache's disk ({root})", free=_gb(v.free), root=v.root), warn
        return fmt("{count} storage device(s)", count=len(found)), warn

    def page(self, parent: QWidget) -> QWidget:
        box = QWidget(parent)
        col = QVBoxLayout(box)
        col.setContentsMargins(0, 0, 0, 0)
        col.setSpacing(8)
        col.addWidget(_label(box, tr("Read-only: the mounted storage and where CFA4 keeps its files. CFA4 changes nothing on the disks here."), dim=True))
        found = self.volumes()
        locs = self.locations()
        col.addWidget(_label(box, tr("Storage Devices") if found else tr("No storage device found."), "settings.storage.status", bold=bool(found)))
        by_key = {v.root: v for v in found}
        here = {v.root: [x for x in locs if volume_of(x[1], found) is v] for v in found}

        def status(v: Volume) -> str:
            parts = [fmt("{free} free", free=_gb(v.free))]
            if v.read_only:
                parts.append(tr("read-only"))
            if here[v.root]:
                parts.append(tr("CFA4"))
            text = " · ".join(parts)
            return "⚠ " + text if self.warnings(v, here[v.root]) else text

        entries = [DeviceEntry(v.root, v.title, status(v)) for v in found]

        def details(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            vol = by_key.get(e.key) if e else None
            if vol is None:
                v.addWidget(_label(w, tr("No storage device found.")))
            else:
                for title, value in ((tr("Mount point"), vol.root), (tr("Device"), vol.device or "—"), (tr("File system"), vol.fs or "—"),
                                     (tr("Size"), _gb(vol.total)), (tr("Free"), _gb(vol.free)),
                                     (tr("Read-only"), tr("Yes") if vol.read_only else tr("No"))):
                    v.addWidget(_label(w, f"{title}: {value}"))
            v.addStretch(1)
            return w

        def capabilities(p: QWidget, e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            vol = by_key.get(e.key) if e else None
            mine = here.get(vol.root, []) if vol else []
            if not mine:
                v.addWidget(_label(w, tr("No CFA4 location on this disk.")))
            for title, path in mine:
                v.addWidget(_label(w, f"{title}: {path}"))
            if vol is not None:
                for line in self.warnings(vol, mine):
                    v.addWidget(_label(w, "⚠ " + line, "settings.storage.warning"))
                v.addWidget(_label(w, fmt("CFA4 warns below {limit} free.", limit=_gb(self._limit(vol))), dim=True))
            v.addStretch(1)
            return w

        def settings(p: QWidget, _e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            v.addWidget(_label(w, tr("Where CFA4 keeps its cache is set on the Media Cache page."), dim=True))
            v.addWidget(_link(w, tr("Media Cache…"), "settings.storage.open.mediaCache", "mediaCache", self.open_page), 0, Qt.AlignmentFlag.AlignLeft)
            v.addStretch(1)
            return w

        def diagnostics(p: QWidget, _e: DeviceEntry | None) -> QWidget:
            w = QWidget(p)
            v = QVBoxLayout(w)
            for line in self.log:
                v.addWidget(_label(w, line, dim=True))
            v.addStretch(1)
            return w

        def remember(key: str) -> None:
            self.selected = key

        def remember_tab(key: str) -> None:
            self.tab = key

        page = DevicePage(box, "settings.storage.page", entries,
                          {"details": details, "capabilities": capabilities, "settings": settings, "diagnostics": diagnostics},
                          self.selected, remember, self.tab, remember_tab)
        page.setMinimumHeight(380)
        col.addWidget(page)
        return box
