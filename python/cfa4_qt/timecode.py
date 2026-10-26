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


def drop_frame(settings: Any) -> bool:
    """Whether a sequence counts drop-frame timecode: asked for, and at 29.97 / 59.94 / 119.88
    (the engine's ``supports_drop_frame``)."""
    if not isinstance(settings, dict) or settings.get("drop_frame") is not True:
        return False
    num, den = frame_rate(settings)
    return den == 1001 and round(num / den) % 30 == 0


def format_timecode(ticks: int, frame: int, drop: bool = False) -> str:
    """HH:MM:SS:FF for ``ticks`` at a frame length of ``frame`` ticks; ``drop``: drop-frame
    HH;MM;SS;FF, counted as the engine does (so a shown value parses back to the same frame)."""
    frame = max(1, int(frame))
    ticks = max(0, int(ticks))
    fps = max(1, round(TICKS_PER_SECOND / frame))
    total = ticks // frame
    sep = ":"
    if drop and fps % 30 == 0:
        n_drop = fps // 15
        per_min = fps * 60 - n_drop
        per_10 = per_min * 10 + n_drop
        tens, rest = divmod(total, per_10)
        total += n_drop * 9 * tens
        if rest > n_drop:
            total += n_drop * ((rest - n_drop) // per_min)
        sep = ";"
    ff = total % fps
    secs = total // fps
    width = 3 if 100 <= fps < 1000 else 2  # as the engine pads 100+ fps (not a degenerate frame length)
    return f"{secs // 3600:02d}{sep}{secs // 60 % 60:02d}{sep}{secs % 60:02d}{sep}{ff:0{width}d}"
