"""Recently used effects (SC-4): a short most-recent-first list of effect ids kept in the user's
preferences (``QSettings`` key ``effects/recent``), never in the project. Any object with
``value(key, default)`` and ``setValue(key, value)`` works as the store. A missing, malformed or
unreadable store gives an empty list, never an error."""

from __future__ import annotations

from typing import Any

KEY = "effects/recent"
LIMIT = 10


def _clean(raw: Any) -> list[str]:
    if isinstance(raw, str):
        raw = [raw] if raw else []
    if not isinstance(raw, (list, tuple)):
        return []
    out: list[str] = []
    for item in raw:
        if isinstance(item, str) and item and item not in out:
            out.append(item)
    return out[:LIMIT]


def load(store: Any) -> list[str]:
    """The recent effect ids, most recent first."""
    if store is None:
        return []
    try:
        return _clean(store.value(KEY, []))
    except Exception:  # a broken settings backend must not break the panel
        return []


def push(store: Any, effect: str) -> list[str]:
    """Move ``effect`` to the front (adding it if new), keep at most ``LIMIT``, save and return the list."""
    if not isinstance(effect, str) or not effect:
        return load(store)
    items = [effect] + [e for e in load(store) if e != effect]
    items = items[:LIMIT]
    if store is not None:
        try:
            store.setValue(KEY, items)
        except Exception:
            pass
    return items


def clear(store: Any) -> None:
    if store is not None:
        try:
            store.setValue(KEY, [])
        except Exception:
            pass
