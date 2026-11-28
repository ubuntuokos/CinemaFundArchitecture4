"""Headphone monitoring (binaural, HRTF): ``docs/plans/CFA4-BINAURAL-MONITOR-001.md`` §3.4.

Settings ▸ Audio Hardware ▸ Headphones as ``audio.render``'s ``monitor`` parameter, and the reply's
``monitor`` as the line a monitor shows ("Atmos · 16 objects · binaural"). No Qt here: the plain
Python tests check it.
"""

from __future__ import annotations

import math
from typing import Any

from .i18n import N_, fmt, tr

# Settings ▸ Audio Hardware ▸ Headphones ▸ Room as ``monitor.room`` (0 dry, up to 0.5 Dolby's small
# room, above it the larger one)
ROOMS = {"off": 0.0, "little": 0.35, "medium": 0.75}
# ``monitor.layout`` (the channels heard as virtual loudspeakers) by name
LAYOUT_NAMES = {1: N_("Mono"), 2: N_("Stereo"), 6: "5.1", 8: "7.1", 10: "7.1.2", 12: "7.1.4"}
# ``monitor.format`` as the monitor shows it
FORMATS = {"Dolby TrueHD Atmos": "Atmos", "IAB": "IAB"}


def monitor_param(prefs: dict[str, Any]) -> dict[str, Any] | None:
    """``audio.render``'s ``monitor`` from the ``audioHardware`` preferences; None for the speakers."""
    if prefs.get("monitoring") != "headphones":
        return None
    hrtf = str(prefs.get("hrtfSet") or "").strip()
    lfe = prefs.get("headphoneLfeDb")
    lfe_db = -10.0
    if isinstance(lfe, (int, float)) and not isinstance(lfe, bool) and math.isfinite(lfe):
        lfe_db = max(-30.0, min(0.0, float(lfe)))
    room = ROOMS.get(str(prefs.get("headphoneRoom") or ""), ROOMS["little"])
    return {"mode": "binaural", "hrtf": hrtf or "builtin", "room": room, "lfe_db": lfe_db}


def heard_text(heard: dict[str, Any]) -> str:
    """What the headphones play, from a reply's ``monitor``: the objects of an Atmos or IAB clip, the
    layout of the mix, or that the HRTF set is loading; empty when they play nothing binaural."""
    if heard.get("pending"):
        return tr("Loading HRTF…")
    if heard.get("mode") != "binaural":
        return ""
    objects = heard.get("objects")
    name = str(heard.get("format") or "")
    if isinstance(objects, int) and name:
        return fmt("{source} · {n} objects · binaural", source=FORMATS.get(name, name), n=objects)
    layout = heard.get("layout")
    if isinstance(layout, int) and layout in LAYOUT_NAMES:
        return fmt("{layout} · binaural", layout=tr(LAYOUT_NAMES[layout]))
    return tr("Binaural")
