"""SC-4: the recent-effects list in the user preferences."""

from __future__ import annotations

from typing import Any

from cfa4_qt import recent_effects as r


class Store:
    def __init__(self) -> None:
        self.d: dict[str, Any] = {}

    def value(self, key: str, default: Any = None) -> Any:
        return self.d.get(key, default)

    def setValue(self, key: str, value: Any) -> None:  # noqa: N802 (Qt naming)
        self.d[key] = value


class Broken:
    def value(self, *_a: Any) -> Any:
        raise RuntimeError("no settings")

    def setValue(self, *_a: Any) -> None:  # noqa: N802
        raise RuntimeError("no settings")


def test_most_recent_first_without_duplicates() -> None:
    s = Store()
    for e in ("blur", "invert", "blur", "crop"):
        r.push(s, e)
    assert r.load(s) == ["crop", "blur", "invert"]


def test_limit() -> None:
    s = Store()
    for i in range(25):
        r.push(s, f"fx{i}")
    got = r.load(s)
    assert len(got) == r.LIMIT and got[0] == "fx24"


def test_hostile_stores_never_raise() -> None:
    assert r.load(None) == [] and r.push(None, "x") == ["x"]
    assert r.load(Broken()) == [] and r.push(Broken(), "x") == ["x"]
    s = Store()
    s.d[r.KEY] = [1, None, "a", "a", {"x": 1}]
    assert r.load(s) == ["a"]
    s.d[r.KEY] = "single"  # QSettings returns a bare string for a one-item list
    assert r.load(s) == ["single"]
    s.d[r.KEY] = 42
    assert r.load(s) == []
    assert r.push(s, "") == [] and r.push(s, 5) == []  # type: ignore[arg-type]


def test_clear() -> None:
    s = Store()
    r.push(s, "a")
    r.clear(s)
    assert r.load(s) == []
