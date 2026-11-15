#!/usr/bin/env python3
"""CFA4-PY-TYPING-001 T0: the mypy exception list may only shrink.

``pyproject.toml`` lists the modules that still fail ``mypy --strict`` in one
``[[tool.mypy.overrides]]`` block with ``ignore_errors = true``. This check compares that list with
the base branch's: every module listed here must already be listed there, so a new or fixed file
cannot (re)join it. A module removed from the list is fine (that is the point).

Usage: check-mypy-ratchet.py HEAD_PYPROJECT BASE_PYPROJECT
Exit status: 0 = the list did not grow, 1 = it grew, 2 = a file could not be read.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path
from typing import Any


def exceptions(pyproject: dict[str, Any]) -> set[str]:
    """The modules of every ``ignore_errors = true`` override (none: an empty set)."""
    mypy = pyproject.get("tool", {}).get("mypy", {})
    out: set[str] = set()
    for override in mypy.get("overrides", []):
        if isinstance(override, dict) and override.get("ignore_errors") is True:
            module = override.get("module", [])
            out.update([module] if isinstance(module, str) else [str(m) for m in module])
    return out


def load(path: Path) -> dict[str, Any]:
    with path.open("rb") as f:
        return tomllib.load(f)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        head, base = load(Path(argv[0])), load(Path(argv[1]))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        print(f"mypy ratchet: cannot read pyproject.toml: {exc}", file=sys.stderr)
        return 2
    head_list, base_list = exceptions(head), exceptions(base)
    if not base_list:
        # T0 introduces the list; at T6 the emptied list and this check are removed together
        print(f"mypy ratchet: the base has no exception list yet; the list starts at {len(head_list)} modules")
        return 0
    added = sorted(head_list - base_list)
    if added:
        print("::error::mypy ratchet: these modules were added to the mypy exception list in pyproject.toml; "
              "fix their type errors instead (CFA4-PY-TYPING-001, IV.1): " + ", ".join(added))
        return 1
    removed = len(base_list - head_list)
    print(f"mypy ratchet: OK, {len(head_list)} modules on the list ({removed} removed in this change)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
