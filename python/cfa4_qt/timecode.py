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


def parse_timecode(text: str, frame: int, drop: bool = False) -> int | None:
    """Frames for a typed timecode, as the engine's ``parse_timecode`` reads one: fields split by
    ``:``, ``;``, ``.`` or ``,`` filled from the right (``1.00`` is a second), a bare number as
    right-aligned digit pairs (``130`` is 1 s 30 f); fields may overflow (90 frames). ``None``
    when it is not one."""
    s = text.strip()
    if not s or s[0] in "+-":
        return None
    parts = s.replace(";", ":").replace(".", ":").replace(",", ":").split(":")
    if len(parts) == 1:
        if not (s.isascii() and s.isdecimal()):
            return None
        nums = [int(s[max(0, end - 2):end]) for end in range(len(s), 0, -2)][::-1]
    else:
        if not all(p == "" or (p.isascii() and p.isdecimal()) for p in parts):
            return None
        nums = [int(p) if p else 0 for p in parts]
    if len(nums) > 4:
        return None
    h, m, sec, ff = [0] * (4 - len(nums)) + nums
    fps = max(1, round(TICKS_PER_SECOND / max(1, int(frame))))
    frames = ((h * 3600 + m * 60 + sec) * fps) + ff
    if drop and fps % 30 == 0 and m < 60 and sec < 60 and ff < fps:
        total_min = h * 60 + m
        frames -= fps // 15 * (total_min - total_min // 10)
    return frames
