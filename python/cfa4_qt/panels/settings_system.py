"""Hardware & Devices ▸ **System Settings (root)** (``docs/plans/CFA4-SETTINGS-HW-003.md`` R3).

The page shows the components of ``cfa4-system-helper`` (ADR 0017): GPU access (the ``render`` /
``video`` group), VA-API drivers and the NPU check. Every change goes one way: **Check** (no root),
**Plan…** (no root; shows exactly what would change and how to undo it), **Apply…** (only the plan
the user looked at, then the system's password dialog through ``pkexec``), **Undo…** (only what the
helper did). The engine side is ``system.components`` / ``system.plan`` / ``system.apply`` /
``system.disable`` (``apps/cfa4-cli/src/serve_system.rs``).

Object names: ``settings.system.components``, ``settings.system.<id>.check``, ``.plan``,
``.apply``, ``.undo``, ``settings.system.<id>.status``, ``settings.system.log``,
``settings.system.availability``, ``settings.system.open`` (the link on the other pages).
"""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QFrame,
    QHBoxLayout,
    QLabel,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, fmt, tr

SYSTEM_PAGE = "systemComponents"
SYSTEM_TITLE = N_("System Settings (root)")
COMPONENT_TITLES = {
    "gpu-access": (N_("GPU access"), N_("Lets you open the GPU and NPU device nodes: adds you to the group (render, video) that owns them.")),
    "vaapi-drivers": (N_("VA-API drivers"), N_("Installs the Ubuntu package with the hardware video driver of an Intel or AMD GPU (NVIDIA uses NVDEC).")),
    "npu-check": (N_("NPU driver"), N_("Checks the NPU's kernel driver and firmware; it only gives guidance and changes nothing.")),
}
STATUS_TITLES = {
    "ok": N_("OK"),
    "needs_action": N_("Missing"),
    "not_needed": N_("Not needed"),
    "relogin": N_("Waiting for log out"),
    "unchecked": N_("Not checked"),
}
UNDO = {
    "add_to_group": N_("Undo: removes you from the group again."),
    "install_packages": N_("Undo: removes the installed packages again."),
}


def _obj(v: Any) -> dict[str, Any]:
    return v if isinstance(v, dict) else {}


@dataclass
class Component:
    id: str
    status: str
    details: list[str] = field(default_factory=list)
    can_apply: bool = False
    relogin_required: bool = False
    applied: int = 0


@dataclass
class SystemReport:
    user: str
    components: list[Component]
    installed: bool
    pkexec: bool
    has_helper: bool

    @staticmethod
    def parse(raw: Any) -> SystemReport:
        r = _obj(raw)
        avail = _obj(r.get("available"))
        recorded = _obj(_obj(r.get("state")).get("components"))
        comps = []
        for c in r.get("components") or []:
            if not isinstance(c, dict) or not isinstance(c.get("component"), str):
                continue
            cid = str(c["component"])
            rec = _obj(recorded.get(cid))
            applied = rec.get("applied")
            comps.append(Component(
                id=cid,
                status=str(c.get("status") or "unchecked"),
                details=[str(d) for d in c.get("details") or [] if isinstance(d, str)],
                can_apply=bool(c.get("can_apply")),
                relogin_required=bool(c.get("relogin_required")),
                applied=len(applied) if isinstance(applied, list) else 0,
            ))
        return SystemReport(user=str(r.get("user") or ""), components=comps, installed=bool(avail.get("installed")),
                            pkexec=bool(avail.get("pkexec")), has_helper=bool(avail.get("helper")))


def shown_status(c: Component) -> str:
    """The status word: a group change the helper made that still waits for a new login shows as
    *Waiting for log out*, not *Missing*."""
    if c.id == "gpu-access" and c.status == "needs_action" and c.applied > 0:
        return "relogin"
    return c.status if c.status in STATUS_TITLES else "unchecked"


def describe_plan(plan: dict[str, Any]) -> str:
    """The plan as plain lines for the confirmation: each change and how it is undone."""
    lines = []
    for a in plan.get("actions") or []:
        if not isinstance(a, dict):
            continue
        kind = a.get("kind")
        if kind == "add_to_group":
            lines.append(fmt("Add user {user} to the group {group}.", user=a.get("user", ""), group=a.get("group", "")))
        elif kind == "install_packages":
            lines.append(fmt("Install packages: {packages}", packages=", ".join(str(p) for p in a.get("packages") or [])))
        else:
            lines.append(str(kind))
        undo = UNDO.get(str(kind))
        if undo:
            lines.append("  " + tr(undo))
    if plan.get("relogin_required"):
        lines.append(tr("Log out and back in afterwards for the change to take effect."))
    return "\n".join(lines)


def _confirm(parent: QWidget, title: str, text: str) -> bool:
    box = QMessageBox(QMessageBox.Icon.Question, title, text, QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel, parent)
    box.setDefaultButton(QMessageBox.StandardButton.Cancel)
    return box.exec() == QMessageBox.StandardButton.Yes


class SystemPage:
    """The state behind the page. ``changed()`` runs when an answer arrived; ``confirm(title, text)``
    asks the user (a message box; tests replace it)."""

    def __init__(self, engine: Any, changed: Callable[[], None], confirm: Callable[[str, str], bool] | None = None) -> None:
        self.engine = engine
        self.changed = changed
        self.confirm = confirm
        self.report: SystemReport | None = None
        self.error = ""
        self.plans: dict[str, tuple[str, dict[str, Any]]] = {}
        self.busy = ""
        self.log: list[str] = []
        self._asked = False

    def _log(self, text: str) -> None:
        self.log.append(f"{time.strftime('%H:%M:%S')}  {text}")
        del self.log[:-200]

    def _call(self, method: str, params: dict[str, Any], ok: Callable[[Any], None], failed: Callable[[str], None]) -> None:
        call = getattr(self.engine, "call", None)
        if call is None:
            failed(tr("no answer"))
            return
        call(method, params, lambda r, _b=None: ok(r), lambda m: failed(str(m)))

    def needs_attention(self) -> bool:
        """GPU access or a VA-API driver is missing (the other pages show a link here)."""
        return self.report is not None and any(c.status == "needs_action" for c in self.report.components if c.id in ("gpu-access", "vaapi-drivers"))

    # ----- engine calls -----
    def load(self, refresh: bool = False) -> None:
        if self._asked and not refresh:
            return
        self._asked = True
        if refresh:
            self.report, self.error = None, ""
            self.changed()
        self._call("system.components", {}, self._loaded, self._failed)

    def _loaded(self, result: Any) -> None:
        self.report, self.error = SystemReport.parse(result), ""
        self.changed()

    def _failed(self, message: str) -> None:
        self.report, self.error = None, message or tr("no answer")
        self._log(fmt("Check failed: {error}", error=self.error))
        self.changed()

    def plan(self, component: str, parent: QWidget | None = None) -> None:
        self.busy = component
        self.changed()

        def ok(result: Any) -> None:
            self.busy = ""
            r = _obj(result)
            plan = _obj(r.get("plan"))
            digest = str(r.get("digest") or "")
            self.plans[component] = (digest, plan)
            self._log(fmt("Plan for {component}: {plan}", component=component, plan=describe_plan(plan).replace("\n", " ")))
            self.changed()
            if parent is not None:
                QMessageBox.information(parent, tr("Plan"), describe_plan(plan) or tr("Nothing to change."))

        def failed(message: str) -> None:
            self.busy = ""
            self._log(fmt("Plan for {component} failed: {error}", component=component, error=message))
            self.changed()

        self._call("system.plan", {"component": component}, ok, failed)

    def _ask(self, parent: QWidget | None, title: str, text: str) -> bool:
        if self.confirm is not None:
            return self.confirm(title, text)
        return parent is not None and _confirm(parent, title, text)

    def apply(self, component: str, parent: QWidget | None = None) -> None:
        """Applies the plan the user looked at (Plan… first); the system asks for the password."""
        shown = self.plans.get(component)
        if shown is None:
            return
        digest, plan = shown
        text = describe_plan(plan) + "\n\n" + tr("The system will ask for an administrator password.")
        if not self._ask(parent, tr("Apply the plan?"), text):
            return
        self._change("system.apply", component, {"component": component, "digest": digest, "plan": plan})

    def undo(self, component: str, parent: QWidget | None = None) -> None:
        if not self._ask(parent, tr("Undo the changes?"), tr("CFA4 undoes only what it changed itself. The system will ask for an administrator password.")):
            return
        self._change("system.disable", component, {"component": component})

    def _change(self, method: str, component: str, params: dict[str, Any]) -> None:
        self.busy = component
        self._log(fmt("{method} {component}: waiting for the password…", method=method, component=component))
        self.changed()

        def ok(result: Any) -> None:
            self.busy = ""
            self.plans.pop(component, None)
            if _obj(result).get("relogin_required"):
                self._log(tr("Done. Log out and back in for the change to take effect."))
            else:
                self._log(fmt("{method} {component}: done", method=method, component=component))
            self.load(refresh=True)

        def failed(message: str) -> None:
            self.busy = ""
            self.plans.pop(component, None)
            self._log(fmt("{method} {component} failed: {error}", method=method, component=component, error=message))
            self.load(refresh=True)

        self._call(method, params, ok, failed)

    # ----- the page -----
    def page(self, parent: QWidget) -> QWidget:
        tabs = QTabWidget(parent)
        tabs.setObjectName("settings.system.tabs")
        tabs.addTab(self._components(tabs), tr("Components"))
        log = QPlainTextEdit(tabs)
        log.setObjectName("settings.system.log")
        log.setReadOnly(True)
        log.setPlainText("\n".join(self.log) or tr("Nothing happened yet in this window."))
        tabs.addTab(log, tr("Log"))
        tabs.setMinimumHeight(420)
        return tabs

    def _components(self, parent: QWidget) -> QWidget:
        box = QWidget(parent)
        box.setObjectName("settings.system.components")
        col = QVBoxLayout(box)
        col.setSpacing(8)
        warn = QLabel(tr("The actions here ask for an administrator password and change the system outside CFA4."), box)
        warn.setWordWrap(True)
        warn.setStyleSheet("font-weight: bold;")
        col.addWidget(warn)
        r = self.report
        if r is None:
            text = fmt("Check failed: {error}", error=self.error) if self.error else tr("Checking…")
            status = QLabel(text, box)
            status.setObjectName("settings.system.availability")
            col.addWidget(status)
            col.addStretch(1)
            return box
        can_change = r.installed and r.pkexec
        if not can_change:
            why = tr("This installation has no system helper (it comes with the .deb package): you can see what is missing, but not change it here.") if not r.installed else tr("pkexec is not installed (Ubuntu package pkexec), so CFA4 cannot ask for the password.")
            note = QLabel(why, box)
            note.setObjectName("settings.system.availability")
            note.setWordWrap(True)
            note.setStyleSheet(f"color: {theme.current().text_dim};")
            col.addWidget(note)
        for c in r.components:
            col.addWidget(self._component(box, c, can_change, r.has_helper))
        col.addStretch(1)
        return box

    def _component(self, parent: QWidget, c: Component, can_change: bool, has_helper: bool) -> QWidget:
        frame = QFrame(parent)
        frame.setFrameShape(QFrame.Shape.StyledPanel)
        v = QVBoxLayout(frame)
        title, about = COMPONENT_TITLES.get(c.id, (c.id, ""))
        head = QHBoxLayout()
        name = QLabel(tr(title), frame)
        name.setStyleSheet("font-weight: bold;")
        head.addWidget(name)
        head.addStretch(1)
        status = QLabel(tr(STATUS_TITLES[shown_status(c)]), frame)
        status.setObjectName(f"settings.system.{c.id}.status")
        head.addWidget(status)
        v.addLayout(head)
        if about:
            a = QLabel(tr(about), frame)
            a.setWordWrap(True)
            a.setStyleSheet(f"color: {theme.current().text_dim}; font-size: 11.5px;")
            v.addWidget(a)
        if c.details:
            d = QLabel("\n".join(c.details), frame)
            d.setWordWrap(True)
            d.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
            v.addWidget(d)
        row = QHBoxLayout()
        busy = bool(self.busy)

        def button(key: str, text: str, enabled: bool, run: Callable[[], None]) -> None:
            b = QPushButton(text, frame)
            b.setObjectName(f"settings.system.{c.id}.{key}")
            b.setAutoDefault(False)
            b.setEnabled(enabled and not busy)
            b.clicked.connect(lambda _c=False: run())
            row.addWidget(b)

        button("check", tr("Check"), True, lambda: self.load(refresh=True))
        button("plan", tr("Plan…"), c.can_apply and has_helper, lambda: self.plan(c.id, frame))
        button("apply", tr("Apply…"), can_change and c.id in self.plans, lambda: self.apply(c.id, frame))
        button("undo", tr("Undo…"), can_change and c.applied > 0, lambda: self.undo(c.id, frame))
        row.addStretch(1)
        v.addLayout(row)
        return frame


def open_link(parent: QWidget, open_page: Callable[[str], None]) -> QPushButton:
    """The link from the Accelerators and Decoding & Encoding pages to this page."""
    b = QPushButton(tr("Missing access or driver: System Settings (root)…"), parent)
    b.setObjectName("settings.system.open")
    b.setAutoDefault(False)
    b.setFlat(True)
    b.setStyleSheet(f"color: {theme.current().accent}; text-align: left;")
    b.clicked.connect(lambda _c=False: open_page(SYSTEM_PAGE))
    return b
