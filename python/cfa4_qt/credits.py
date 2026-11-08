"""The About window's credits as egui's (``crates/ui-egui/src/credits.rs`` and its ``build.rs``):
every contributor (GitHub login always; display and real names only with recorded consent in
``people.toml``) and the AI models that helped, read from ``contributors/contributors.json``.

In a checkout the files are the repository's; the packages ship a copy next to this module
(``credits/``, packaging/linux/package.sh)."""

from __future__ import annotations

import json
import tomllib
from dataclasses import dataclass
from functools import cmp_to_key
from pathlib import Path
from typing import Any

from .i18n import N_

HERE = Path(__file__).resolve().parent
REPO_CREDITS = HERE.parent.parent / "contributors"
CREDITS_DIR = REPO_CREDITS if (REPO_CREDITS / "contributors.json").is_file() else HERE / "credits"


@dataclass(frozen=True)
class Contributor:
    login: str
    display_name: str | None = None
    real_name: str | None = None
    prs: int = 0
    commits: int = 0
    lines_added: int = 0
    lines_deleted: int = 0
    binary_added: int = 0
    binary_deleted: int = 0
    first_commit: str = ""  # ISO 8601 UTC; empty for someone credited only through a merged PR
    last_commit: str = ""

    def name(self, mode: str) -> str:
        """``username`` (``@login``), ``displayName`` or ``realName``, falling back to ``@login``."""
        chosen = {"displayName": self.display_name, "realName": self.real_name}.get(mode)
        return chosen if chosen else f"@{self.login}"

    @property
    def lines_delta(self) -> int:
        return self.lines_added - self.lines_deleted

    def summary(self) -> str:
        """One line with everything we know, for tooltips."""
        return (
            f"@{self.login}: {self.prs} PRs, {self.commits} commits, +{group(self.lines_added)} / −{group(self.lines_deleted)} lines "
            f"(Δ {signed(self.lines_delta)}), +{self.binary_added} / −{self.binary_deleted} binary assets, "
            f"{day(self.first_commit)} – {day(self.last_commit)}"
        )


@dataclass(frozen=True)
class Model:
    company: str
    model: str
    version: str
    commits: int = 0
    lines_added: int = 0
    lines_deleted: int = 0


@dataclass(frozen=True)
class Credits:
    total_commits: int
    contributors: tuple[Contributor, ...]
    models: tuple[Model, ...]


NAME_MODES = (("username", N_("Username")), ("displayName", N_("Display name")), ("realName", N_("Real name")))
# (key, menu label, table header), egui's SortKey::ALL
SORT_KEYS = (
    ("name", N_("Name (A–Z)"), "Name"),
    ("prs", N_("Merged PRs"), "PRs"),
    ("commits", N_("Commits"), "Commits"),
    ("linesAdded", N_("Lines added"), "+LOC"),
    ("linesDeleted", N_("Lines deleted"), "−LOC"),
    ("linesDelta", N_("Line delta"), "ΔLOC"),
    ("binaryAdded", N_("Binary assets added"), "+Bin"),
    ("binaryDeleted", N_("Binary assets removed"), "−Bin"),
    ("firstCommit", N_("First commit"), "First"),
    ("lastCommit", N_("Last commit"), "Last"),
)


def default_ascending(key: str) -> bool:
    """Names and dates read naturally oldest / A first; counts read biggest first."""
    return key in ("name", "firstCommit", "lastCommit")


def _name(v: Any) -> str | None:
    return v.strip() if isinstance(v, str) and v.strip() else None


def _n(v: dict[str, Any], k: str) -> int:
    x = v.get(k)
    return x if isinstance(x, int) and not isinstance(x, bool) and x >= 0 else 0


def _s(v: dict[str, Any], k: str) -> str:
    x = v.get(k)
    return x if isinstance(x, str) else ""


def parse(data: Any, people: Any) -> Credits:
    """build.rs's ``generate``: the contributors with their consented names, and the models."""
    data = data if isinstance(data, dict) else {}
    table = people.get("people") if isinstance(people, dict) else None
    table = table if isinstance(table, dict) else {}
    out: list[Contributor] = []
    for c in data.get("contributors") or []:
        if not isinstance(c, dict) or not _s(c, "login"):
            continue
        login = _s(c, "login")
        entry = table.get(login) if c.get("people_entry") is True else None
        entry = entry if isinstance(entry, dict) else {}
        display = _name(entry.get("display_name"))
        if display is None and entry.get("sync_github_name") is True:
            display = _name(c.get("github_name"))
        out.append(
            Contributor(
                login,
                display,
                _name(entry.get("real_name")),
                _n(c, "prs"),
                _n(c, "commits"),
                _n(c, "lines_added"),
                _n(c, "lines_deleted"),
                _n(c, "binary_added"),
                _n(c, "binary_deleted"),
                _s(c, "first_commit"),
                _s(c, "last_commit"),
            )
        )
    models = tuple(
        Model(_s(m, "company"), _s(m, "model"), _s(m, "version"), _n(m, "commits"), _n(m, "lines_added"), _n(m, "lines_deleted"))
        for m in data.get("models") or []
        if isinstance(m, dict)
    )
    return Credits(_n(data, "commits"), tuple(out), models)


def load(directory: Path = CREDITS_DIR) -> Credits:
    """The credits from ``directory``; none when they are missing or unreadable."""
    try:
        data = json.loads((directory / "contributors.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        data = None
    try:
        people = tomllib.loads((directory / "people.toml").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        people = {}
    return parse(data, people)


def _name_key(name: str) -> str:
    """Alphabetical key: case-insensitive, ignoring the ``@`` of usernames."""
    return name.lstrip("@").lower()


def _value(c: Contributor, mode: str, key: str) -> Any:
    if key == "name":
        return _name_key(c.name(mode))
    if key in ("firstCommit", "lastCommit"):
        d = c.first_commit if key == "firstCommit" else c.last_commit
        return (d == "", d)  # a PR-only contributor (no commit date) sorts after everyone dated
    return {
        "prs": c.prs,
        "commits": c.commits,
        "linesAdded": c.lines_added,
        "linesDeleted": c.lines_deleted,
        "linesDelta": c.lines_delta,
        "binaryAdded": c.binary_added,
        "binaryDeleted": c.binary_deleted,
    }[key]


def _cmp(a: Any, b: Any) -> int:
    return (a > b) - (a < b)


def sorted_contributors(contributors: tuple[Contributor, ...] | list[Contributor], mode: str, key: str, ascending: bool) -> list[Contributor]:
    """Contributors in display order; ties fall back to the name (A–Z), then the login."""

    def order(a: Contributor, b: Contributor) -> int:
        o = _cmp(_value(a, mode, key), _value(b, mode, key))
        o = o if ascending else -o
        return o or _cmp(_value(a, mode, "name"), _value(b, mode, "name")) or _cmp(a.login, b.login)

    return sorted(contributors, key=cmp_to_key(order))


def group(n: int) -> str:
    return f"{n:,}"


def signed(n: int) -> str:
    return f"−{group(abs(n))}" if n < 0 else f"+{group(n)}"


def day(iso: str) -> str:
    return iso[:10] if iso else "—"
