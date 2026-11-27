"""Render job notices for the Qt GUI (C0 Qt6 sketch, step 1 and 3).

``render.list`` answers every job as ``{job, label, state, event}`` with the contract types of
``cfa4-tasks`` (``RenderState``, ``RenderEvent``). A *warning* or a *fallback* event must reach the
user (rule A4: nothing degrades quietly), not only the engine log. This module turns those events
into one-line notices, once per event. It has no Qt import, so it is tested without a display.
"""

from __future__ import annotations

from typing import Any

Notice = tuple[str, bool]  # (text, is_error)
Seen = set[tuple[str, str, str]]


def _event_notice(label: str, state: str, event: dict[str, Any]) -> Notice | None:
    kind = event.get("kind")
    if kind == "fallback":
        fb = event.get("fallback")
        if not isinstance(fb, dict):
            return None
        text = f"{label}: fell back from {fb.get('from', '?')} to {fb.get('to', '?')}"
        reason = fb.get("reason")
        return (f"{text} ({reason})" if reason else text, False)
    if kind == "warning":
        message = str(event.get("message") or "").strip()
        if not message:
            return None
        return (f"{label}: {message}", state == "failed")
    return None


def new_notices(entries: Any, seen: Seen) -> list[Notice]:
    """Notices for the warning/fallback events in a ``render.list`` reply not yet in ``seen``.

    ``seen`` is updated, so each event is announced once. Malformed entries are skipped.
    """
    out: list[Notice] = []
    if not isinstance(entries, list):
        return out
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        event = entry.get("event")
        if not isinstance(event, dict):
            continue
        job = str(entry.get("job", ""))
        label = str(entry.get("label") or f"Job {job}")
        notice = _event_notice(label, str(entry.get("state", "")), event)
        if notice is None:
            continue
        key = (job, str(event.get("kind")), notice[0])
        if key in seen:
            continue
        seen.add(key)
        out.append(notice)
    return out
