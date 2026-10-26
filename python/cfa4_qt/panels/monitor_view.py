"""How a monitor shows its picture: playback and paused resolution, magnification and pan,
display mode, safe margins and the transport buttons shown, as egui's ``MonitorView``
(crates/ui-egui/src/state.rs). Kept per monitor and remembered between sessions.

The View menu entries (Playback Resolution, Paused Resolution, Display Mode, Magnification,
High Quality Playback, Safe Margins) act on the focused monitor, as in egui.
"""

from __future__ import annotations

import json
import math
from collections.abc import Callable
from dataclasses import asdict, dataclass, field
from typing import Any

from PySide6.QtCore import QSettings
from PySide6.QtGui import QAction, QActionGroup
from PySide6.QtWidgets import QMenu

from ..i18n import N_, tr

# (id, label, scale): egui's PlaybackRes
RESOLUTIONS: tuple[tuple[str, str, float], ...] = (
    ("full", N_("Full"), 1.0),
    ("half", "1/2", 0.5),
    ("quarter", "1/4", 0.25),
    ("eighth", "1/8", 0.125),
    ("sixteenth", "1/16", 0.0625),
)
# egui's ZOOMS: the magnification menu, and the wheel's limits
ZOOMS: tuple[tuple[str, float], ...] = (
    ("10", 0.1),
    ("25", 0.25),
    ("50", 0.5),
    ("75", 0.75),
    ("100", 1.0),
    ("150", 1.5),
    ("200", 2.0),
    ("400", 4.0),
    ("800", 8.0),
    ("1600", 16.0),
)
MIN_ZOOM, MAX_ZOOM = ZOOMS[0][1], ZOOMS[-1][1]
# display modes the Qt monitors draw (egui's DisplayMode minus the ones not in Qt yet)
DISPLAYS: tuple[tuple[str, str], ...] = (
    ("composite", N_("Composite Video")),
    ("alpha", N_("Alpha")),
    ("red", N_("Red")),
    ("green", N_("Green")),
    ("blue", N_("Blue")),
)
# the Source monitor's audio display modes (egui's DisplayMode::AudioWaveform / VideoAndWaveform)
SOURCE_DISPLAYS: tuple[tuple[str, str], ...] = (
    ("audioWaveform", N_("Audio Waveform")),
    ("videoAndWaveform", N_("Video and Audio Waveform Split")),
)
CHANNELS = ("alpha", "red", "green", "blue")
# the marked span of the Source clip dragged to the Timeline: JSON {item, sourceIn, duration,
# video, audio} (egui's SourceDrag)
SOURCE_MIME = "application/x-cfa4-source"


def source_payload(raw: Any) -> dict[str, Any] | None:
    """A dragged Source span, checked: the media item, a span inside its media and the streams."""
    try:
        data = json.loads(bytes(raw).decode()) if raw is not None else None
    except (ValueError, UnicodeDecodeError, TypeError):
        return None
    if not isinstance(data, dict):
        return None

    def whole(k: str) -> int | None:
        v = data.get(k)
        return v if isinstance(v, int) and not isinstance(v, bool) and 0 <= v < 1 << 62 else None

    item, start, dur = whole("item"), whole("sourceIn"), whole("duration")
    video, audio = data.get("video") is True, data.get("audio") is True
    if item is None or start is None or not dur or not (video or audio):
        return None
    return {"item": item, "sourceIn": start, "duration": dur, "video": video, "audio": audio}
DEFAULT_HIDDEN = ("playback.loop",)  # egui: the Loop button is in the Button Editor, not shown


def res_scale(res: str) -> float:
    return next((s for k, _l, s in RESOLUTIONS if k == res), 0.5)


def res_label(res: str) -> str:
    return tr(next((lab for k, lab, _s in RESOLUTIONS if k == res), "1/2"))


@dataclass
class MonitorView:
    res: str = "half"  # Playback Resolution (while playing)
    paused_res: str = "full"  # Paused Resolution
    high_quality: bool = False  # play at the paused resolution when it is higher
    zoom: float | None = None  # None: Fit; 1.0: one frame pixel per screen pixel
    pan: list[float] = field(default_factory=lambda: [0.0, 0.0])  # from the centre, in pixels
    display: str = "composite"
    safe_margins: bool = False
    show_transport: bool = True
    hidden: list[str] = field(default_factory=lambda: list(DEFAULT_HIDDEN))
    looping: bool = False

    def scale(self, playing: bool) -> float:
        """egui's effective_res: Playback Resolution while playing (or the paused one with High
        Quality Playback when that is higher), Paused Resolution when stopped."""
        paused, play = res_scale(self.paused_res), res_scale(self.res)
        if not playing or (self.high_quality and paused > play):
            return paused
        return play

    def set_zoom(self, zoom: float | None) -> None:
        self.zoom = None if zoom is None else min(MAX_ZOOM, max(MIN_ZOOM, float(zoom)))
        self.pan = [0.0, 0.0]

    def zoom_label(self) -> str:
        return tr("Fit") if self.zoom is None else f"{round(self.zoom * 100)}%"

    # ----- persistence -----
    @classmethod
    def load(cls, settings: QSettings | None, key: str) -> MonitorView:
        v = cls()
        raw = settings.value(key) if settings is not None else None
        try:
            data = json.loads(raw) if isinstance(raw, str) else None
        except ValueError:
            data = None
        if isinstance(data, dict):
            for name, default in asdict(cls()).items():
                value = data.get(name, default)
                if name == "zoom":
                    v.zoom = None if value is None else min(MAX_ZOOM, max(MIN_ZOOM, float(value))) if isinstance(value, (int, float)) else None
                elif name in ("res", "paused_res"):
                    setattr(v, name, value if any(value == k for k, _l, _s in RESOLUTIONS) else default)
                elif name == "display":
                    v.display = value if any(value == k for k, _l in (*DISPLAYS, *SOURCE_DISPLAYS)) else default
                elif name == "hidden":
                    v.hidden = [str(x) for x in value] if isinstance(value, list) else list(default)
                elif name == "pan":
                    xy = value[:2] if isinstance(value, list) else []
                    ok = len(xy) == 2 and all(isinstance(x, (int, float)) and not isinstance(x, bool) and math.isfinite(x) for x in xy)
                    v.pan = [float(x) for x in xy] if ok else [0.0, 0.0]
                elif isinstance(default, bool):
                    setattr(v, name, bool(value))
        return v

    def save(self, settings: QSettings | None, key: str) -> None:
        if settings is not None:
            settings.setValue(key, json.dumps(asdict(self)))


def build_view_menu(menu: QMenu, monitor: Callable[[], Any]) -> list[QAction]:
    """View ▸ Playback Resolution / Paused Resolution / Display Mode / Magnification, High Quality
    Playback and Safe Margins, acting on ``monitor()`` (the focused monitor) and ticked from it
    each time the menu opens. Returns the top-level actions added."""
    added: list[QAction] = []

    def radio(title: str, name: str, items: tuple[tuple[str, str], ...], get: Callable[[MonitorView], Any], put: Callable[[Any, str], None]) -> None:
        sub = QMenu(tr(title), menu)
        sub.setObjectName(f"menu.View.{name}")
        group = QActionGroup(sub)
        actions: dict[str, QAction] = {}
        for key, label in items:
            a = QAction(tr(label), sub, checkable=True)
            a.setObjectName(f"view.{name}.{key}")
            a.triggered.connect(lambda _c=False, k=key: put(monitor(), k))
            group.addAction(a)
            sub.addAction(a)
            actions[key] = a

        def tick() -> None:
            current = get(monitor().mv)
            for key, a in actions.items():
                a.setChecked(key == current)

        sub.aboutToShow.connect(tick)
        added.append(menu.addMenu(sub))

    res_items = tuple((k, lab) for k, lab, _s in RESOLUTIONS)
    radio(N_("Playback Resolution"), "playbackRes", res_items, lambda v: v.res, lambda m, k: m.set_resolution(k))
    radio(N_("Paused Resolution"), "pausedRes", res_items, lambda v: v.paused_res, lambda m, k: m.set_resolution(k, paused=True))
    hq = QAction(tr("High Quality Playback"), menu, checkable=True)
    hq.setObjectName("view.highQualityPlayback")
    hq.triggered.connect(lambda on: monitor().set_view(high_quality=bool(on)))
    menu.addAction(hq)
    added.append(hq)
    radio(N_("Display Mode"), "display", DISPLAYS, lambda v: v.display, lambda m, k: m.set_view(display=k))
    zoom_items = (("fit", N_("Fit")), *((k, f"{k}%") for k, _z in ZOOMS))
    radio(
        N_("Magnification"),
        "magnification",
        zoom_items,
        lambda v: "fit" if v.zoom is None else next((k for k, z in ZOOMS if abs(z - v.zoom) < 1e-6), ""),
        lambda m, k: m.set_zoom(None if k == "fit" else dict(ZOOMS)[k]),
    )
    safe = QAction(tr("Safe Margins"), menu, checkable=True)
    safe.setObjectName("view.safeMargins")
    safe.triggered.connect(lambda on: monitor().set_view(safe_margins=bool(on)))
    menu.addAction(safe)
    added.append(safe)

    def tick_toggles() -> None:
        v = monitor().mv
        hq.setChecked(v.high_quality)
        safe.setChecked(v.safe_margins)

    menu.aboutToShow.connect(tick_toggles)
    return added
