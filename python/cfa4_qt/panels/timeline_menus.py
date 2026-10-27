"""The timeline's context menus, as egui's (``crates/ui-egui/src/panels/timeline.rs``): the clip
menu (``CLIP_MENU``, with the Label and Multi-Camera submenus), the edit point menu of a
right-clicked cut, the marker menu on the ruler and the menu of a right-clicked gap (Ripple
Delete). Object names are egui's automation ids: ``timeline.clipMenu.<command>``,
``timeline.clipMenu.label.<Label>``, ``timeline.clipMenu.multicam.camera.<angle>`` and
``timeline.editPointMenu.<id>`` and ``timeline.gapMenu.edit.rippleDelete``.

The canvas asks its :class:`MenuHost` (the main window) whether a command is enabled and runs
the chosen one through it, so the menu's commands open the same dialogs as the menu bar's.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any, Protocol

from PySide6.QtGui import QColor, QIcon, QPixmap
from PySide6.QtWidgets import QMenu, QWidget

from ..i18n import N_, tr

# egui's CLIP_MENU: groups of (label, command), a separator between groups
CLIP_MENU: tuple[tuple[tuple[str, str], ...], ...] = (
    (
        (N_("Cut"), "edit.cut"),
        (N_("Copy"), "edit.copy"),
        (N_("Paste Attributes…"), "edit.pasteAttributes"),
        (N_("Remove Attributes…"), "edit.removeAttributes"),
        (N_("Clear"), "edit.clear"),
        (N_("Ripple Delete"), "edit.rippleDelete"),
    ),
    ((N_("Edit Original"), "edit.editOriginal"), (N_("Replace With Clip From Source Monitor"), "clip.replaceFromSource")),
    (
        (N_("Enable"), "clip.enable"),
        (N_("Link"), "clip.link"),
        (N_("Group"), "clip.group"),
        (N_("Ungroup"), "clip.ungroup"),
        (N_("Synchronize…"), "clip.synchronize"),
        (N_("Merge Clips…"), "clip.mergeClips"),
        (N_("Nest…"), "clip.nest"),
        (N_("Make Subsequence"), "sequence.makeSubsequence"),
        (N_("Reveal Nested Sequence"), "sequence.revealNested"),
        (N_("Multi-Camera"), "clip.multicam"),
    ),
    ((N_("Label"), "edit.label"),),
    ((N_("Add Marker"), "markers.add"),),
    ((N_("Speed/Duration…"), "clip.speedDuration"), (N_("Scene Edit Detection…"), "clip.sceneEditDetection")),
    (
        (N_("Frame Hold Options…"), "clip.frameHoldOptions"),
        (N_("Add Frame Hold"), "clip.frameHold"),
        (N_("Insert Frame Hold Segment"), "clip.insertFrameHoldSegment"),
        (N_("Field Options…"), "clip.fieldOptions"),
        (N_("Scale to Frame Size"), "clip.scaleToFrameSize"),
        (N_("Fit to frame"), "clip.fitToFrame"),
        (N_("Fill frame"), "clip.fillFrame"),
    ),
    ((N_("Reveal in Project"), "clip.revealInProject"), (N_("Join Through Edits"), "sequence.joinThroughEdits")),
)


class MenuHost(Protocol):
    def command_enabled(self, cid: str) -> bool: ...

    def run_menu_command(self, cid: str, params: dict[str, Any]) -> None: ...

    def menu_request(self, cid: str, params: dict[str, Any], then: Callable[[bool], None]) -> None:
        """Run ``cid`` (None: nothing), bring the commands' enabled states up to date, then call
        ``then(ok)``: the menu opens on the state the right-click made."""
        ...

    def edit_point_kind(self, dist_px: float, neighbour: bool, then: Callable[[str], None]) -> None:
        """The trim type the Selection tool picks at a cut (egui's ``selection_trim_kind``)."""
        ...


def swatch(color: str) -> QIcon:
    pm = QPixmap(12, 12)
    pm.fill(QColor(color))
    return QIcon(pm)


def clip_menu(
    parent: QWidget,
    host: MenuHost,
    picked: list[dict[str, Any]],
    labels: dict[str, str],
    on_unlinked: Any = None,
) -> QMenu:
    """egui's clip menu for the selected clips ``picked`` (sequence.inspect items).
    ``labels``: label name -> colour. ``on_unlinked``: called after Unlink (egui keeps only the
    right-clicked clip selected, so it can be dragged away from the others)."""
    menu = QMenu(parent)
    menu.setObjectName("timeline.clipMenu")
    some = bool(picked)
    all_enabled = some and all(bool(c.get("enabled", True)) for c in picked)
    # the engine's clip.link unlinks only when every selected clip is linked, else it links them
    linked = some and all(c.get("link") is not None for c in picked)
    for i, group in enumerate(CLIP_MENU):
        if i:
            menu.addSeparator()
        for label, cid in group:
            if cid == "edit.label":
                sub = menu.addMenu(tr(label))
                sub.setObjectName("timeline.clipMenu.edit.label")
                sub.setEnabled(some)
                for name, color in labels.items():
                    title = name.capitalize()
                    a = sub.addAction(swatch(color), tr(title))
                    a.setObjectName(f"timeline.clipMenu.label.{title}")
                    a.triggered.connect(lambda _c=False, t=title: host.run_menu_command("edit.label", {"label": t}))
                continue
            if cid == "clip.multicam":
                multicam_menu(menu, host, picked)
                continue
            text = tr(label)
            if cid == "clip.link" and linked:
                text = tr("Unlink")
            a = menu.addAction(text)
            a.setObjectName(f"timeline.clipMenu.{cid}")
            if cid == "clip.enable":
                a.setCheckable(True)
                a.setChecked(all_enabled)
            a.setEnabled(some and host.command_enabled(cid))

            def run(_c: bool = False, c: str = cid, unlink: bool = cid == "clip.link" and linked) -> None:
                host.run_menu_command(c, {})
                if unlink and on_unlinked is not None:
                    on_unlinked()

            a.triggered.connect(run)
    return menu


def multicam_menu(menu: QMenu, host: MenuHost, picked: list[dict[str, Any]]) -> QMenu:
    """The Multi-Camera submenu: Enable, Flatten, and the cameras of the selected nested sequence
    clips (greyed out when none of them is a nested sequence)."""
    nests = [c for c in picked if c.get("nested")]
    enabled = bool(nests) and all(isinstance(c.get("multicam"), dict) and c["multicam"].get("enabled") for c in nests)
    sub = menu.addMenu(tr("Multi-Camera"))
    sub.setObjectName("timeline.clipMenu.clip.multicam")
    sub.setEnabled(bool(nests))
    a = sub.addAction(tr("Enable"))
    a.setObjectName("timeline.clipMenu.clip.multicamEnable")
    a.setCheckable(True)
    a.setChecked(enabled)
    a.setEnabled(host.command_enabled("clip.multicamEnable"))
    a.triggered.connect(lambda _c=False: host.run_menu_command("clip.multicamEnable", {}))
    a = sub.addAction(tr("Flatten"))
    a.setObjectName("timeline.clipMenu.clip.multicamFlatten")
    a.setEnabled(host.command_enabled("clip.multicamFlatten"))
    a.triggered.connect(lambda _c=False: host.run_menu_command("clip.multicamFlatten", {}))
    first = nests[0] if nests else {}
    cameras = [c for c in first.get("cameras") or [] if isinstance(c, dict) and isinstance(c.get("angle"), int)]
    if enabled and cameras:
        sub.addSeparator()
        mc = first.get("multicam") if isinstance(first.get("multicam"), dict) else {}
        clips = [c.get("clip") for c in nests]
        for cam in cameras:
            angle = cam["angle"]
            a = sub.addAction(str(cam.get("name") or ""))
            a.setObjectName(f"timeline.clipMenu.multicam.camera.{angle}")
            a.setCheckable(True)
            a.setChecked(mc.get("angle") == angle)
            a.triggered.connect(lambda _c=False, n=angle: host.run_menu_command("multicam.switchAngle", {"angle": n, "clips": clips}))
    return sub


# egui's EDIT_POINT_TYPES: label, id (``timeline.editPointMenu.<id>``), trim kind, and the side of
# the cut it selects ("in": the incoming clip's In, "out": the outgoing clip's Out)
EDIT_POINT_TYPES: tuple[tuple[str, str, str, str], ...] = (
    (N_("Ripple Trim In"), "rippleIn", "ripple", "in"),
    (N_("Ripple Trim Out"), "rippleOut", "ripple", "out"),
    (N_("Roll Edit"), "roll", "roll", "out"),
    (N_("Trim In"), "trimIn", "trim", "in"),
    (N_("Trim Out"), "trimOut", "trim", "out"),
)


def edit_point_menu(
    parent: QWidget, host: MenuHost, left: Any, right: Any, selected: dict[str, Any] | None, through: bool
) -> QMenu:
    """egui's edit point menu on a right-clicked cut (``left`` ends there, ``right`` starts there,
    either None at a gap): the trim types (the selected one checked), Apply Default Transitions and
    Join Through Edits (only for a through edit)."""
    menu = QMenu(parent)
    menu.setObjectName("timeline.editPointMenu")
    for label, mid, kind, edge in EDIT_POINT_TYPES:
        clip = right if edge == "in" else left
        if kind == "roll" and (left is None or right is None):
            clip = None  # a roll needs a clip on both sides of the cut
        a = menu.addAction(tr(label))
        a.setObjectName(f"timeline.editPointMenu.{mid}")
        a.setCheckable(True)
        a.setChecked(
            clip is not None and isinstance(selected, dict) and selected.get("clip") == clip
            and selected.get("out") is (edge == "out") and selected.get("kind") == kind
        )
        a.setEnabled(clip is not None)
        a.triggered.connect(
            lambda _c=False, c=clip, e=edge, k=kind: host.run_menu_command("trim.selectEditPoint", {"clip": c, "edge": e, "kind": k})
        )
    menu.addSeparator()
    a = menu.addAction(tr("Apply Default Transitions"))
    a.setObjectName("timeline.editPointMenu.applyDefaultTransitions")
    a.setEnabled(host.command_enabled("trim.applyDefaultTransition"))
    a.triggered.connect(lambda _c=False: host.run_menu_command("trim.applyDefaultTransition", {}))
    a = menu.addAction(tr("Join Through Edits"))
    a.setObjectName("timeline.editPointMenu.joinThroughEdits")
    a.setEnabled(through and left is not None and right is not None)
    a.triggered.connect(lambda _c=False: host.run_menu_command("sequence.joinThroughEdits", {"cut": [left, right]}))
    return menu


def marker_menu(parent: QWidget, host: MenuHost, marker: dict[str, Any]) -> QMenu:
    """egui's marker menu on the ruler: Label ▸ the marker colours, Clear Selected Marker, Clear
    Markers."""
    from .info import LABELS, marker_rgb

    menu = QMenu(parent)
    menu.setObjectName("timeline.markerMenu")
    sub = menu.addMenu(tr("Label"))
    sub.setObjectName("timeline.markerMenu.label")
    for name in LABELS:
        a = sub.addAction(swatch(marker_rgb(name).name()), tr(name))
        a.setObjectName(f"timeline.markerMenu.color.{name}")
        a.setCheckable(True)
        a.setChecked(marker.get("color") == name)
        a.triggered.connect(
            lambda _c=False, n=name: host.run_menu_command("markers.edit", {"target": "program", "marker": marker.get("id"), "color": n})
        )
    menu.addSeparator()
    for label, cid in ((N_("Clear Selected Marker"), "markers.clearCurrent"), (N_("Clear Markers"), "markers.clearAll")):
        a = menu.addAction(tr(label))
        a.setObjectName(f"timeline.markerMenu.{cid}")
        a.triggered.connect(lambda _c=False, c=cid: host.run_menu_command(c, {"target": "program"}))
    return menu


def gap_menu(parent: QWidget, host: MenuHost) -> QMenu:
    """The menu of a right-clicked gap: Ripple Delete closes it."""
    menu = QMenu(parent)
    menu.setObjectName("timeline.gapMenu")
    a = menu.addAction(tr("Ripple Delete"))
    a.setObjectName("timeline.gapMenu.edit.rippleDelete")
    a.setEnabled(host.command_enabled("edit.rippleDelete"))
    a.triggered.connect(lambda _c=False: host.run_menu_command("edit.rippleDelete", {}))
    return menu
