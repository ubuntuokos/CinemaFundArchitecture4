"""Hardware & Devices ▸ **Overview** (``docs/plans/CFA4-SETTINGS-HW-003.md`` R4): the group's first
page. One read-only list of the accelerators, hardware decoding, the system components, the control
surfaces, the audio devices and the memory, each with its warnings and a button to its own page.
It asks nothing new of the engine: the data comes from ``hardware.inventory``,
``codecs.capabilities``, ``system.components``, ``surface.devices``, Qt's audio device list and the
window's picture memory.

Object names: ``settings.overview`` (the list), ``settings.overview.<page>`` (a row),
``settings.overview.<page>.summary``, ``settings.overview.<page>.open``.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtWidgets import QFrame, QHBoxLayout, QLabel, QPushButton, QVBoxLayout, QWidget

from .. import theme
from ..i18n import N_, fmt, tr
from .settings_system import COMPONENT_TITLES, STATUS_TITLES, SystemPage, shown_status

OVERVIEW_PAGE = "hardwareOverview"
OVERVIEW_TITLE = N_("Overview")


class OverviewPage:
    """``hardware`` is the dialog's :class:`~.settings_hardware.HardwarePages`; ``devices()`` the
    audio devices, ``memory()`` the bytes of decoded pictures, ``budget_mb()`` the frame cache
    budget; ``has_page(id)`` tells which pages exist and ``open_page(id)`` opens one."""

    def __init__(self, engine: Any, hardware: Any, system: SystemPage, changed: Callable[[], None],
                 devices: Callable[[], dict[str, Any]], memory: Callable[[], int], budget_mb: Callable[[], int],
                 has_page: Callable[[str], bool], open_page: Callable[[str], None]) -> None:
        self.engine = engine
        self.hardware = hardware
        self.system = system
        self.changed = changed
        self.devices = devices
        self.memory = memory
        self.budget_mb = budget_mb
        self.has_page = has_page
        self.open_page = open_page
        self.surfaces: dict[str, Any] | None = None
        self._asked = False

    def load(self) -> None:
        """Asks for every report the list shows (each once; the other pages share them)."""
        self.hardware.load("accelerators")
        self.hardware.load("decoding")
        self.system.load()
        if self._asked:
            return
        self._asked = True
        call = getattr(self.engine, "call", None)
        if call is None:
            self.surfaces = {"devices": []}
            return

        def got(r: Any, _b: Any = None) -> None:
            self.surfaces = r if isinstance(r, dict) else {"devices": []}
            self.changed()

        call("surface.devices", {}, got, lambda m: got({"devices": [], "problems": [str(m)]}))

    # ----- the rows: (page, title, summary, warnings) -----
    def rows(self) -> list[tuple[str, str, str, list[str]]]:
        out: list[tuple[str, str, str, list[str]]] = []
        hw = self.hardware.hardware
        report = self.system.report
        gpu_access = next((c for c in report.components if c.id == "gpu-access"), None) if report else None
        if hw is None:
            acc = fmt("Detection failed: {error}", error=self.hardware.hardware_error) if self.hardware.hardware_error else tr("Detecting…")
            acc_warn: list[str] = []
        else:
            gpus = sum(1 for d in hw.devices if d.kind == "gpu")
            npus = sum(1 for d in hw.devices if d.kind == "npu")
            acc = fmt("{gpus} GPU, {npus} NPU", gpus=gpus, npus=npus) if hw.devices else tr("No GPU or NPU found: everything runs on the CPU.")
            acc_warn = []
            if gpu_access is not None and gpu_access.status == "needs_action":
                acc_warn.append(tr("A GPU or NPU cannot be opened: a group membership is missing (System Settings (root))."))
        out.append(("accelerators", tr("Accelerators (GPU / NPU)"), acc, acc_warn))

        c = self.hardware.codecs
        if c is None:
            dec = fmt("Detection failed: {error}", error=self.hardware.codecs_error) if self.hardware.codecs_error else tr("Detecting…")
            dec_warn: list[str] = []
        else:
            dec = fmt("Hardware decoder: {name}", name=c.hardware_decoder.name) if c.hardware_decoder.name else tr("Software decoding only")
            dec_warn = [] if c.hardware_decoder.name or hw is None or not hw.devices else [
                fmt("No hardware decoder ({reason}).", reason=c.hardware_decoder.reason or tr("not found"))]
        out.append(("decoding", tr("Decoding & Encoding"), dec, dec_warn))

        if report is None:
            sys_text = fmt("Check failed: {error}", error=self.system.error) if self.system.error else tr("Checking…")
            sys_warn: list[str] = []
        else:
            sys_text = ", ".join(f"{tr(COMPONENT_TITLES.get(x.id, (x.id, ''))[0])}: {tr(STATUS_TITLES[shown_status(x)])}" for x in report.components) or tr("No system helper in this installation.")
            sys_warn = [tr(COMPONENT_TITLES.get(x.id, (x.id, ""))[0]) + ": " + "; ".join(x.details) for x in report.components if x.status == "needs_action"]
        out.append(("systemComponents", tr("System Settings (root)"), sys_text, sys_warn))

        if self.surfaces is None:
            surf, surf_warn = tr("Checking…"), []
        else:
            found = [d for d in self.surfaces.get("devices") or [] if isinstance(d, dict)]
            surf = fmt("{count} connected", count=len(found)) if found else tr("None connected")
            surf_warn = [str(p) for p in self.surfaces.get("problems") or [] if isinstance(p, str)]
        out.append(("controlSurfaces", tr("Control Surfaces"), surf, surf_warn))

        audio = self.devices() or {}
        outputs: list[Any] = audio["outputs"] if isinstance(audio.get("outputs"), list) else []
        inputs: list[Any] = audio["inputs"] if isinstance(audio.get("inputs"), list) else []
        audio_text = fmt("{outputs} outputs, {inputs} inputs", outputs=len(outputs), inputs=len(inputs))
        out.append(("audioHardware", tr("Audio Hardware"), audio_text, [] if outputs else [tr("No audio output found: playback is silent.")]))

        used = self.memory() >> 20
        budget = self.budget_mb()
        mem = fmt("Decoded pictures: {used} MB of {budget} MB", used=used, budget=budget) if budget else fmt("Decoded pictures: {used} MB", used=used)
        out.append(("memory", tr("Memory"), mem, [tr("The picture cache is full.")] if budget and used >= budget else []))
        return [r for r in out if self.has_page(r[0])]

    def page(self, parent: QWidget) -> QWidget:
        box = QWidget(parent)
        box.setObjectName("settings.overview")
        col = QVBoxLayout(box)
        col.setContentsMargins(0, 0, 0, 0)
        col.setSpacing(6)
        intro = QLabel(tr("Read-only: the state of this machine's hardware at a glance. Each row opens its own page."), box)
        intro.setWordWrap(True)
        intro.setStyleSheet(f"color: {theme.current().text_dim}; font-size: 11.5px;")
        col.addWidget(intro)
        for page_id, title, summary, warnings in self.rows():
            col.addWidget(self._row(box, page_id, title, summary, warnings))
        col.addStretch(1)
        return box

    def _row(self, parent: QWidget, page_id: str, title: str, summary: str, warnings: list[str]) -> QWidget:
        frame = QFrame(parent)
        frame.setObjectName(f"settings.overview.{page_id}")
        frame.setFrameShape(QFrame.Shape.StyledPanel)
        h = QHBoxLayout(frame)
        v = QVBoxLayout()
        name = QLabel(title, frame)
        name.setStyleSheet("font-weight: bold;")
        v.addWidget(name)
        s = QLabel(summary, frame)
        s.setObjectName(f"settings.overview.{page_id}.summary")
        s.setWordWrap(True)
        v.addWidget(s)
        for w in warnings:
            warn = QLabel("⚠ " + w, frame)
            warn.setWordWrap(True)
            warn.setStyleSheet(f"color: {theme.current().hot_text};")
            v.addWidget(warn)
        h.addLayout(v, 1)
        b = QPushButton(tr("Open…"), frame)
        b.setObjectName(f"settings.overview.{page_id}.open")
        b.setAutoDefault(False)
        b.clicked.connect(lambda _c=False: self.open_page(page_id))
        h.addWidget(b)
        return frame
