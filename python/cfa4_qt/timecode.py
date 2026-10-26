"""Engine time helpers. All edit math stays in the engine's integer ticks."""

from __future__ import annotations

from typing import Any

TICKS_PER_SECOND = 254_016_000_000


def frame_rate(settings: Any) -> tuple[int, int]:
    """(numerator, denominator) of a sequence's ``frame_rate`` as the engine serialises it."""
    rate = settings.get("frame_rate") if isinstance(settings, dict) else None
    if isinstance(rate, dict):
        num = rate.get("num", rate.get("numerator"))
        den = rate.get("den", rate.get("denominator"))
    elif isinstance(rate, list | tuple) and len(rate) == 2:
        num, den = rate
    else:
        num, den = 30, 1
    try:
        num, den = int(num), int(den)
    except (TypeError, ValueError):
        num, den = 30, 1
    if num <= 0 or den <= 0:
        num, den = 30, 1
    return num, den


def frame_ticks(settings: Any) -> int:
    num, den = frame_rate(settings)
    return max(1, TICKS_PER_SECOND * den // num)


def format_timecode(ticks: int, frame: int) -> str:
    """HH:MM:SS:FF (non-drop) for ``ticks`` at a frame length of ``frame`` ticks."""
    frame = max(1, int(frame))
    ticks = max(0, int(ticks))
    fps = max(1, round(TICKS_PER_SECOND / frame))
    total = ticks // frame
    ff = total % fps
    secs = total // fps
    return f"{secs // 3600:02d}:{secs // 60 % 60:02d}:{secs % 60:02d}:{ff:02d}"
